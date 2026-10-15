//! Case data: uncompressed, bytecode-compressed and ZLIB-compressed.

use crate::assemble::{Field, trim};
use crate::bytes::{Endian, Reader};
use crate::error::{Error, ErrorKind};
use crate::model::{Compression, Limits, Value};
use encoding_rs::Encoding;
use flate2::read::ZlibDecoder;
use std::io::Read;

pub(crate) struct DataParams<'a> {
    pub compression: Compression,
    pub endian: Endian,
    pub bias: f64,
    pub sysmis: f64,
    pub plan: &'a [Field],
    pub encoding: &'static Encoding,
    pub case_count: Option<u64>,
    /// Offset of the data record within the file.
    pub offset: usize,
}

/// A stream of 8-byte elements.
struct Elements<'a> {
    data: &'a [u8],
    pos: usize,
    base: usize,
    compressed: bool,
    cmds: [u8; 8],
    next_cmd: usize,
    done: bool,
    bias: f64,
    endian: Endian,
    sysmis: [u8; 8],
}

fn take8(data: &[u8], pos: usize) -> Option<[u8; 8]> {
    let end = pos.checked_add(8)?;
    data.get(pos..end)?.try_into().ok()
}

impl Elements<'_> {
    fn offset(&self) -> usize {
        self.base.saturating_add(self.pos)
    }

    fn truncated(&self) -> Error {
        Error::new(self.offset(), ErrorKind::Truncated("case data"))
    }

    /// `Ok(None)` at a clean end of data.
    fn next(&mut self) -> Result<Option<[u8; 8]>, Error> {
        if !self.compressed {
            if self.pos >= self.data.len() {
                return Ok(None);
            }
            let e = take8(self.data, self.pos).ok_or_else(|| self.truncated())?;
            self.pos += 8;
            return Ok(Some(e));
        }
        loop {
            if self.done {
                return Ok(None);
            }
            if self.next_cmd >= 8 {
                match take8(self.data, self.pos) {
                    Some(c) => {
                        self.cmds = c;
                        self.pos += 8;
                        self.next_cmd = 0;
                    }
                    None => {
                        self.done = true;
                        return Ok(None);
                    }
                }
            }
            let code = self.cmds.get(self.next_cmd).copied().unwrap_or(0);
            self.next_cmd += 1;
            match code {
                0 => {}
                1..=251 => {
                    let v = f64::from(code) - self.bias;
                    return Ok(Some(self.endian.f64_bytes(v)));
                }
                252 => {
                    self.done = true;
                    return Ok(None);
                }
                253 => {
                    let e = take8(self.data, self.pos).ok_or_else(|| self.truncated())?;
                    self.pos += 8;
                    return Ok(Some(e));
                }
                254 => return Ok(Some([b' '; 8])),
                255 => return Ok(Some(self.sysmis)),
            }
        }
    }
}

pub(crate) fn read_rows(
    file: &[u8],
    p: &DataParams<'_>,
    limits: &Limits,
) -> Result<Vec<Vec<Value>>, Error> {
    let tail = file.get(p.offset..).unwrap_or_default();
    let inflated;
    let (data, base, compressed) = match p.compression {
        Compression::None => (tail, p.offset, false),
        Compression::Bytecode => (tail, p.offset, true),
        Compression::Zlib => {
            inflated = inflate(file, p.offset, p.endian, limits)?;
            (inflated.as_slice(), p.offset, true)
        }
    };
    let record_total: usize = p.plan.iter().map(|f| f.records).sum();
    if p.plan.is_empty() || record_total == 0 {
        return Ok(Vec::new());
    }
    let n_vars = p.plan.len() as u64;
    if let Some(n) = p.case_count
        && n.checked_mul(n_vars)
            .is_none_or(|cells| cells > limits.max_cells)
    {
        return Err(Error::new(p.offset, ErrorKind::LimitExceeded("cells")));
    }
    let max_rows = if compressed {
        data.len() / record_total
    } else {
        data.len() / record_total.saturating_mul(8)
    };
    let mut rows: Vec<Vec<Value>> = Vec::new();
    if let Some(n) = p.case_count {
        rows.reserve(usize::try_from(n).unwrap_or(usize::MAX).min(max_rows));
    }
    let mut src = Elements {
        data,
        pos: 0,
        base,
        compressed,
        cmds: [0; 8],
        next_cmd: 8,
        done: false,
        bias: p.bias,
        endian: p.endian,
        sysmis: p.endian.f64_bytes(p.sysmis),
    };
    let mut buf: Vec<u8> = Vec::new();
    loop {
        if p.case_count.is_some_and(|n| rows.len() as u64 >= n) {
            break;
        }
        if (rows.len() as u64).saturating_add(1).saturating_mul(n_vars) > limits.max_cells {
            return Err(Error::new(src.offset(), ErrorKind::LimitExceeded("cells")));
        }
        let mut row = Vec::with_capacity(p.plan.len());
        for (fi, field) in p.plan.iter().enumerate() {
            buf.clear();
            for ri in 0..field.records {
                match src.next()? {
                    Some(e) => {
                        if field.width > 0 || ri == 0 {
                            buf.extend_from_slice(&e);
                        }
                    }
                    None if fi == 0 && ri == 0 => {
                        return finish(rows, p, src.offset());
                    }
                    None => return Err(src.truncated()),
                }
            }
            row.push(decode_field(field, &buf, p));
        }
        rows.push(row);
    }
    Ok(rows)
}

fn finish(rows: Vec<Vec<Value>>, p: &DataParams<'_>, at: usize) -> Result<Vec<Vec<Value>>, Error> {
    match p.case_count {
        Some(n) if (rows.len() as u64) < n => Err(Error::new(
            at,
            ErrorKind::Truncated("case data (fewer cases than the header declares)"),
        )),
        _ => Ok(rows),
    }
}

fn decode_field(field: &Field, buf: &[u8], p: &DataParams<'_>) -> Value {
    if field.width == 0 {
        let v = buf
            .get(..8)
            .and_then(|b| <[u8; 8]>::try_from(b).ok())
            .map_or(p.sysmis, |b| p.endian.f64(b));
        return if v == p.sysmis {
            Value::SystemMissing
        } else {
            Value::Number(v)
        };
    }
    let mut bytes: Vec<u8> = Vec::with_capacity(field.width.min(buf.len()));
    let mut off = 0usize;
    for (seg_bytes, seg_records) in &field.segments {
        let take = (*seg_bytes).min(seg_records.saturating_mul(8));
        if let Some(s) = buf.get(off..off.saturating_add(take)) {
            bytes.extend_from_slice(s);
        }
        off = off.saturating_add(seg_records.saturating_mul(8));
    }
    bytes.truncate(field.width);
    Value::String(crate::assemble::text(p.encoding, trim(&bytes)))
}

/// Concatenates the ZLIB blocks described by the data header and trailer.
fn inflate(file: &[u8], start: usize, endian: Endian, limits: &Limits) -> Result<Vec<u8>, Error> {
    let malformed = |what: &'static str| Error::new(start, ErrorKind::Malformed(what));
    let to_usize = |v: i64, what: &'static str| usize::try_from(v).map_err(|_| malformed(what));
    let header = file.get(start..).unwrap_or_default();
    let mut h = Reader::new(header, start, endian);
    h.i64("zlib data header")?;
    let trailer_ofs = to_usize(h.i64("zlib data header")?, "zlib trailer offset")?;
    let trailer_len = to_usize(h.i64("zlib data header")?, "zlib trailer length")?;
    if trailer_len < 24 {
        return Err(malformed("zlib trailer too short"));
    }
    let trailer_end = trailer_ofs
        .checked_add(trailer_len)
        .ok_or_else(|| malformed("zlib trailer offset"))?;
    let trailer = file
        .get(trailer_ofs..trailer_end)
        .ok_or_else(|| Error::new(file.len(), ErrorKind::Truncated("zlib trailer")))?;
    let mut t = Reader::new(trailer, trailer_ofs, endian);
    t.i64("zlib trailer")?;
    t.i64("zlib trailer")?;
    t.i32("zlib trailer")?;
    let n_blocks = t.len("zlib block count")?;
    if n_blocks != (trailer_len - 24) / 24 {
        return Err(malformed("zlib block count does not match trailer length"));
    }
    let mut out: Vec<u8> = Vec::new();
    for _ in 0..n_blocks {
        t.i64("zlib block descriptor")?;
        let c_ofs = to_usize(t.i64("zlib block descriptor")?, "zlib block offset")?;
        let u_size = t.len("zlib block size")?;
        let c_size = t.len("zlib block size")?;
        let block = c_ofs
            .checked_add(c_size)
            .and_then(|end| file.get(c_ofs..end))
            .ok_or_else(|| Error::new(c_ofs, ErrorKind::Truncated("zlib block")))?;
        let total = (out.len() as u64).saturating_add(u_size as u64);
        if total > limits.max_decompressed_bytes {
            return Err(Error::new(
                c_ofs,
                ErrorKind::LimitExceeded("decompressed size"),
            ));
        }
        let before = out.len();
        ZlibDecoder::new(block)
            .take((u_size as u64).saturating_add(1))
            .read_to_end(&mut out)
            .map_err(|e| Error::new(c_ofs, ErrorKind::Decompress(e.to_string())))?;
        if out.len() - before != u_size {
            return Err(Error::new(
                c_ofs,
                ErrorKind::Decompress("block size differs from trailer".into()),
            ));
        }
    }
    Ok(out)
}
