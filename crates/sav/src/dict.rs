//! Dictionary parsing: header, variable, value-label, document and extension records.
//! Strings stay raw bytes here; they are decoded once the encoding is known.

use crate::bytes::{Endian, Reader};
use crate::error::{Error, ErrorKind};
use crate::model::{Compression, Limits, RawExtension};

pub(crate) struct RawHeader {
    pub product: Vec<u8>,
    pub layout_code: i32,
    pub endian: Endian,
    pub compression: Compression,
    pub nominal_case_size: i32,
    pub weight_index: i32,
    pub ncases: i32,
    pub bias: f64,
    pub creation_date: Vec<u8>,
    pub creation_time: Vec<u8>,
    pub file_label: Vec<u8>,
}

pub(crate) struct RawVar {
    pub width: usize,
    pub name: Vec<u8>,
    pub label: Option<Vec<u8>>,
    pub missing_code: i32,
    pub missing: Vec<[u8; 8]>,
    pub print: i32,
    pub write: i32,
    /// 1-based dictionary index of this record.
    pub first_record: usize,
    /// Records occupied (1 + continuation records).
    pub records: usize,
}

pub(crate) struct RawLabelSet {
    pub labels: Vec<([u8; 8], Vec<u8>)>,
    /// 1-based dictionary (record) indices.
    pub records: Vec<usize>,
}

pub(crate) struct RawLongLabels {
    pub name: Vec<u8>,
    pub labels: Vec<(Vec<u8>, Vec<u8>)>,
}

pub(crate) struct RawLongMissing {
    pub name: Vec<u8>,
    pub values: Vec<Vec<u8>>,
}

pub(crate) struct IntInfo {
    pub floating_point_rep: i32,
    pub character_code: i32,
}

pub(crate) struct RawDict {
    pub header: RawHeader,
    pub vars: Vec<RawVar>,
    pub record_count: usize,
    pub label_sets: Vec<RawLabelSet>,
    pub documents: Vec<Vec<u8>>,
    pub int_info: Option<IntInfo>,
    /// (sysmis, highest, lowest)
    pub float_info: Option<(f64, f64, f64)>,
    pub display: Option<Vec<i32>>,
    pub long_names: Vec<(Vec<u8>, Vec<u8>)>,
    pub very_long: Vec<(Vec<u8>, usize)>,
    pub encoding: Option<Vec<u8>>,
    pub long_labels: Vec<RawLongLabels>,
    pub long_missing: Vec<RawLongMissing>,
    pub ncases64: Option<i64>,
    pub unknown: Vec<RawExtension>,
    pub warnings: Vec<String>,
    /// Offset of the first data byte (after the termination record).
    pub data_offset: usize,
}

const HEADER_LEN: usize = 176;

fn parse_header(bytes: &[u8]) -> Result<RawHeader, Error> {
    let head = bytes
        .get(..HEADER_LEN)
        .ok_or_else(|| Error::new(bytes.len(), ErrorKind::Truncated("file header")))?;
    match head.get(..4) {
        Some(b"$FL2" | b"$FL3") => {}
        Some([0x5b, 0xc6, 0xd3, 0xf2 | 0xf3]) => {
            return Err(Error::new(0, ErrorKind::Unsupported("EBCDIC system file")));
        }
        _ => return Err(Error::new(0, ErrorKind::BadMagic)),
    }
    let layout_bytes: [u8; 4] = head
        .get(64..68)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| Error::new(64, ErrorKind::Truncated("layout code")))?;
    let le = i32::from_le_bytes(layout_bytes);
    let be = i32::from_be_bytes(layout_bytes);
    let (endian, layout_code) = if matches!(le, 2 | 3) {
        (Endian::Little, le)
    } else if matches!(be, 2 | 3) {
        (Endian::Big, be)
    } else {
        return Err(Error::new(64, ErrorKind::BadLayoutCode(le)));
    };
    let mut r = Reader::new(head, 0, endian);
    r.take(4, "file header")?;
    let product = r.take(60, "file header")?.to_vec();
    r.take(4, "file header")?; // layout code, already decoded
    let nominal_case_size = r.i32("file header")?;
    let compression = match r.i32("file header")? {
        0 => Compression::None,
        1 => Compression::Bytecode,
        2 => Compression::Zlib,
        _ => {
            return Err(Error::new(
                72,
                ErrorKind::Unsupported("unknown compression code"),
            ));
        }
    };
    let weight_index = r.i32("file header")?;
    let ncases = r.i32("file header")?;
    let bias = r.f64("file header")?;
    let creation_date = r.take(9, "file header")?.to_vec();
    let creation_time = r.take(8, "file header")?.to_vec();
    let file_label = r.take(64, "file header")?.to_vec();
    Ok(RawHeader {
        product,
        layout_code,
        endian,
        compression,
        nominal_case_size,
        weight_index,
        ncases,
        bias,
        creation_date,
        creation_time,
        file_label,
    })
}

pub(crate) fn parse(bytes: &[u8], limits: &Limits) -> Result<RawDict, Error> {
    let header = parse_header(bytes)?;
    let endian = header.endian;
    let rest = bytes.get(HEADER_LEN..).unwrap_or_default();
    let mut r = Reader::new(rest, HEADER_LEN, endian);
    let mut dict = RawDict {
        header,
        vars: Vec::new(),
        record_count: 0,
        label_sets: Vec::new(),
        documents: Vec::new(),
        int_info: None,
        float_info: None,
        display: None,
        long_names: Vec::new(),
        very_long: Vec::new(),
        encoding: None,
        long_labels: Vec::new(),
        long_missing: Vec::new(),
        ncases64: None,
        unknown: Vec::new(),
        warnings: Vec::new(),
        data_offset: 0,
    };
    loop {
        let at = r.offset();
        match r.i32("record type")? {
            2 => parse_variable(&mut r, &mut dict, limits)?,
            3 => parse_value_labels(&mut r, &mut dict)?,
            4 => {
                return Err(Error::new(
                    at,
                    ErrorKind::Malformed("value label variables record without labels"),
                ));
            }
            6 => {
                let n = r.len("document line count")?;
                let total = n
                    .checked_mul(80)
                    .ok_or_else(|| r.err(ErrorKind::Malformed("document size")))?;
                let lines = r.take(total, "document record")?;
                dict.documents.extend(lines.chunks(80).map(<[u8]>::to_vec));
            }
            7 => parse_extension(&mut r, &mut dict)?,
            999 => {
                r.take(4, "dictionary termination")?;
                dict.data_offset = r.offset();
                return Ok(dict);
            }
            _ => {
                return Err(Error::new(
                    at,
                    ErrorKind::Malformed("unknown record type in dictionary"),
                ));
            }
        }
    }
}

fn parse_variable(r: &mut Reader<'_>, dict: &mut RawDict, limits: &Limits) -> Result<(), Error> {
    let at = r.offset();
    if dict.record_count >= limits.max_variables {
        return Err(Error::new(at, ErrorKind::LimitExceeded("variable records")));
    }
    let type_code = r.i32("variable record")?;
    let has_label = r.i32("variable record")?;
    let missing_code = r.i32("variable record")?;
    let print = r.i32("variable record")?;
    let write = r.i32("variable record")?;
    let name = r.take(8, "variable name")?.to_vec();
    let label = match has_label {
        0 => None,
        1 => {
            let n = r.len("variable label length")?;
            let padded = n.div_ceil(4).saturating_mul(4);
            let raw = r.take(padded, "variable label")?;
            Some(raw.get(..n).unwrap_or_default().to_vec())
        }
        _ => return Err(Error::new(at, ErrorKind::Malformed("has_var_label"))),
    };
    let n_missing = match missing_code {
        0 => 0,
        1..=3 => usize::try_from(missing_code).unwrap_or(0),
        -2 => 2,
        -3 => 3,
        _ => return Err(Error::new(at, ErrorKind::Malformed("missing value count"))),
    };
    let mut missing = Vec::with_capacity(n_missing);
    for _ in 0..n_missing {
        missing.push(r.array::<8>("missing values")?);
    }
    dict.record_count += 1;
    if type_code == -1 {
        let var = dict
            .vars
            .last_mut()
            .ok_or_else(|| Error::new(at, ErrorKind::Malformed("continuation without string")))?;
        var.records += 1;
        return Ok(());
    }
    let width = usize::try_from(type_code)
        .ok()
        .filter(|w| *w <= 255)
        .ok_or_else(|| Error::new(at, ErrorKind::Malformed("variable type")))?;
    dict.vars.push(RawVar {
        width,
        name,
        label,
        missing_code,
        missing,
        print,
        write,
        first_record: dict.record_count,
        records: 1,
    });
    Ok(())
}

fn parse_value_labels(r: &mut Reader<'_>, dict: &mut RawDict) -> Result<(), Error> {
    let count = r.len("value label count")?;
    let mut labels = Vec::new();
    for _ in 0..count {
        let value = r.array::<8>("value label")?;
        let len = usize::from(r.u8("value label")?);
        let padded = (len + 1).div_ceil(8) * 8 - 1;
        let raw = r.take(padded, "value label")?;
        labels.push((value, raw.get(..len).unwrap_or_default().to_vec()));
    }
    let at = r.offset();
    if r.i32("value label variables record")? != 4 {
        return Err(Error::new(
            at,
            ErrorKind::Malformed("value labels not followed by a type 4 record"),
        ));
    }
    let n = r.len("value label variable count")?;
    if n.saturating_mul(4) > r.remaining() {
        return Err(r.err(ErrorKind::Truncated("value label variables")));
    }
    let mut records = Vec::with_capacity(n);
    for _ in 0..n {
        let idx = r.i32("value label variables")?;
        records.push(usize::try_from(idx).unwrap_or(0));
    }
    dict.label_sets.push(RawLabelSet { labels, records });
    Ok(())
}

fn parse_extension(r: &mut Reader<'_>, dict: &mut RawDict) -> Result<(), Error> {
    let subtype = r.i32("extension header")?;
    let size = r.i32("extension header")?;
    let count = r.i32("extension header")?;
    let at = r.offset();
    let len = u64::try_from(size)
        .ok()
        .zip(u64::try_from(count).ok())
        .and_then(|(s, c)| s.checked_mul(c))
        .and_then(|l| usize::try_from(l).ok())
        .ok_or_else(|| Error::new(at, ErrorKind::Malformed("extension size")))?;
    let data = r.take(len, "extension record")?;
    let endian = dict.header.endian;
    match parse_known(dict, subtype, size, count, data, endian) {
        Ok(true) => {}
        Ok(false) => dict.unknown.push(raw_ext(subtype, size, count, data)),
        Err(msg) => {
            dict.warnings
                .push(format!("extension subtype {subtype} ignored: {msg}"));
            dict.unknown.push(raw_ext(subtype, size, count, data));
        }
    }
    Ok(())
}

fn raw_ext(subtype: i32, size: i32, count: i32, data: &[u8]) -> RawExtension {
    RawExtension {
        subtype,
        size,
        count,
        data: data.to_vec(),
    }
}

const BAD: &str = "malformed or truncated";

fn parse_known(
    dict: &mut RawDict,
    subtype: i32,
    size: i32,
    count: i32,
    data: &[u8],
    endian: Endian,
) -> Result<bool, &'static str> {
    let mut r = Reader::new(data, 0, endian);
    match subtype {
        3 => {
            if size != 4 || count != 8 {
                return Err("unexpected size/count");
            }
            let mut v = [0i32; 8];
            for slot in &mut v {
                *slot = r.i32("").map_err(|_| BAD)?;
            }
            dict.int_info = Some(IntInfo {
                floating_point_rep: v[4],
                character_code: v[7],
            });
        }
        4 => {
            if size != 8 || count != 3 {
                return Err("unexpected size/count");
            }
            let a = r.f64("").map_err(|_| BAD)?;
            let b = r.f64("").map_err(|_| BAD)?;
            let c = r.f64("").map_err(|_| BAD)?;
            dict.float_info = Some((a, b, c));
        }
        11 => {
            if size != 4 {
                return Err("unexpected size");
            }
            let n = usize::try_from(count).map_err(|_| BAD)?;
            let mut v = Vec::with_capacity(n.min(data.len() / 4));
            for _ in 0..n {
                v.push(r.i32("").map_err(|_| BAD)?);
            }
            dict.display = Some(v);
        }
        13 => {
            for tuple in data.split(|b| *b == 9).filter(|t| !t.is_empty()) {
                let eq = tuple.iter().position(|b| *b == b'=').ok_or(BAD)?;
                let (k, v) = tuple.split_at(eq);
                dict.long_names
                    .push((k.to_vec(), v.get(1..).unwrap_or_default().to_vec()));
            }
        }
        14 => {
            for tuple in data.split(|b| *b == 9) {
                let tuple: Vec<u8> = tuple.iter().copied().filter(|b| *b != 0).collect();
                if tuple.is_empty() {
                    continue;
                }
                let eq = tuple.iter().position(|b| *b == b'=').ok_or(BAD)?;
                let (k, v) = tuple.split_at(eq);
                let digits =
                    std::str::from_utf8(v.get(1..).unwrap_or_default()).map_err(|_| BAD)?;
                let width: usize = digits.trim().parse().map_err(|_| BAD)?;
                dict.very_long.push((k.to_vec(), width));
            }
        }
        16 => {
            if size != 8 || count != 2 {
                return Err("unexpected size/count");
            }
            r.i64("").map_err(|_| BAD)?;
            dict.ncases64 = Some(r.i64("").map_err(|_| BAD)?);
        }
        20 => dict.encoding = Some(data.to_vec()),
        21 => {
            let mut out = Vec::new();
            while r.remaining() > 0 {
                let n = r.len("").map_err(|_| BAD)?;
                let name = r.take(n, "").map_err(|_| BAD)?.to_vec();
                r.i32("").map_err(|_| BAD)?; // width
                let n_labels = r.len("").map_err(|_| BAD)?;
                let mut labels = Vec::new();
                for _ in 0..n_labels {
                    let vl = r.len("").map_err(|_| BAD)?;
                    let value = r.take(vl, "").map_err(|_| BAD)?.to_vec();
                    let ll = r.len("").map_err(|_| BAD)?;
                    let label = r.take(ll, "").map_err(|_| BAD)?.to_vec();
                    labels.push((value, label));
                }
                out.push(RawLongLabels { name, labels });
            }
            dict.long_labels.extend(out);
        }
        22 => {
            let mut out = Vec::new();
            while r.remaining() > 0 {
                let n = r.len("").map_err(|_| BAD)?;
                let name = r.take(n, "").map_err(|_| BAD)?.to_vec();
                let n_values = r.u8("").map_err(|_| BAD)?;
                let mut values = Vec::new();
                for _ in 0..n_values {
                    let vl = r.len("").map_err(|_| BAD)?;
                    values.push(r.take(vl, "").map_err(|_| BAD)?.to_vec());
                }
                out.push(RawLongMissing { name, values });
            }
            dict.long_missing.extend(out);
        }
        _ => return Ok(false),
    }
    Ok(true)
}
