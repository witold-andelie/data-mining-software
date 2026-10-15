//! Turns the raw dictionary into the public model: decodes strings with the file's encoding,
//! merges very long string segments, applies extension records to variables.

use crate::dict::{RawDict, RawVar};
use crate::error::{Error, ErrorKind};
use crate::model::{
    Alignment, Format, Header, Measure, MissingValues, SavFile, Value, ValueLabel, ValueLabelSet,
    Variable,
};
use encoding_rs::{Encoding, WINDOWS_1252};
use std::collections::HashMap;

/// Physical layout of one logical variable's data in a case.
pub(crate) struct Field {
    /// 0 = numeric, otherwise total string width in bytes.
    pub width: usize,
    /// Number of 8-byte elements occupied.
    pub records: usize,
    /// (bytes, records) of each string segment; one entry unless very long.
    pub segments: Vec<(usize, usize)>,
}

pub(crate) struct Assembled {
    pub file: SavFile,
    pub plan: Vec<Field>,
    pub sysmis: f64,
    pub encoding: &'static Encoding,
}

const NO_VAR: usize = usize::MAX;

pub(crate) fn trim(b: &[u8]) -> &[u8] {
    let end = b
        .iter()
        .rposition(|c| *c != b' ' && *c != 0)
        .map_or(0, |i| i + 1);
    b.get(..end).unwrap_or_default()
}

pub(crate) fn text(enc: &'static Encoding, b: &[u8]) -> String {
    enc.decode_without_bom_handling(b).0.into_owned()
}

pub(crate) fn trimmed(enc: &'static Encoding, b: &[u8]) -> String {
    text(enc, trim(b))
}

fn encoding_from_code(code: i32) -> Option<&'static Encoding> {
    let label = match code {
        2 | 3 => "windows-1252",
        65001 => "utf-8",
        1250..=1258 => return Encoding::for_label(format!("windows-{code}").as_bytes()),
        28591..=28599 => {
            return Encoding::for_label(format!("iso-8859-{}", code - 28590).as_bytes());
        }
        874 => "windows-874",
        932 => "shift_jis",
        936 => "gbk",
        949 => "euc-kr",
        950 => "big5",
        10000 => "macintosh",
        20866 => "koi8-r",
        21866 => "koi8-u",
        51932 => "euc-jp",
        54936 => "gb18030",
        _ => return None,
    };
    Encoding::for_label(label.as_bytes())
}

fn pick_encoding(raw: &RawDict, warnings: &mut Vec<String>) -> &'static Encoding {
    let declared = raw.encoding.as_ref().map(|b| trim(b));
    if let Some(name) = declared {
        match Encoding::for_label(name).filter(|e| e.is_ascii_compatible()) {
            Some(e) => return e,
            None => warnings.push(format!(
                "unknown character encoding {:?}",
                String::from_utf8_lossy(name)
            )),
        }
    }
    if let Some(info) = &raw.int_info {
        if let Some(e) = encoding_from_code(info.character_code) {
            return e;
        }
        warnings.push(format!(
            "unknown character code {}; assuming windows-1252",
            info.character_code
        ));
    }
    WINDOWS_1252
}

fn numeric(v: f64, sysmis: f64) -> Value {
    if v == sysmis {
        Value::SystemMissing
    } else {
        Value::Number(v)
    }
}

fn build_missing(
    pv: &RawVar,
    is_string: bool,
    ctx: &Ctx,
    warnings: &mut Vec<String>,
) -> MissingValues {
    if pv.missing_code == 0 {
        return MissingValues::None;
    }
    if is_string {
        if pv.missing_code < 0 {
            warnings.push(format!(
                "string variable {} has a numeric missing range; ignored",
                trimmed(ctx.enc, &pv.name)
            ));
            return MissingValues::None;
        }
        return MissingValues::Discrete(
            pv.missing
                .iter()
                .map(|m| Value::String(trimmed(ctx.enc, m)))
                .collect(),
        );
    }
    let nums: Vec<f64> = pv.missing.iter().map(|m| ctx.endian.f64(*m)).collect();
    match (pv.missing_code, nums.as_slice()) {
        (-2, [low, high]) => MissingValues::Range {
            low: *low,
            high: *high,
        },
        (-3, [low, high, value]) => MissingValues::RangeAndDiscrete {
            low: *low,
            high: *high,
            value: *value,
        },
        _ => MissingValues::Discrete(nums.into_iter().map(Value::Number).collect()),
    }
}

struct Ctx {
    enc: &'static Encoding,
    endian: crate::bytes::Endian,
}

pub(crate) fn assemble(raw: RawDict) -> Result<Assembled, Error> {
    let mut warnings = raw.warnings.clone();
    if let Some(info) = &raw.int_info
        && matches!(info.floating_point_rep, 2 | 3)
    {
        return Err(Error::new(
            0,
            ErrorKind::Unsupported("IBM/VAX floating point representation"),
        ));
    }
    let enc = pick_encoding(&raw, &mut warnings);
    let endian = raw.header.endian;
    let ctx = Ctx { enc, endian };
    let sysmis = raw.float_info.map_or(-f64::MAX, |f| f.0);

    let very_long: HashMap<&[u8], usize> =
        raw.very_long.iter().map(|(k, w)| (trim(k), *w)).collect();
    let long_names: HashMap<&[u8], &[u8]> = raw
        .long_names
        .iter()
        .map(|(k, v)| (trim(k), trim(v)))
        .collect();

    let mut variables: Vec<Variable> = Vec::new();
    let mut plan: Vec<Field> = Vec::new();
    let mut first_phys: Vec<usize> = Vec::new();
    let mut rec_map = vec![NO_VAR; raw.record_count.saturating_add(1)];
    let mut i = 0;
    while let Some(pv) = raw.vars.get(i) {
        let key = trim(&pv.name);
        let merged = very_long
            .get(key)
            .copied()
            .filter(|w| *w > 255)
            .and_then(|w| {
                let n = w.saturating_add(251) / 252;
                raw.vars.get(i..i.saturating_add(n)).map(|segs| (w, segs))
            });
        let (width, segs): (usize, &[RawVar]) = match merged {
            Some((w, segs)) => (w, segs),
            None => (pv.width, std::slice::from_ref(pv)),
        };
        if merged.is_none() && very_long.contains_key(key) && pv.width != 255 {
            warnings.push(format!(
                "very long string record for {} does not match the dictionary",
                String::from_utf8_lossy(key)
            ));
        }
        let li = variables.len();
        for seg in segs {
            for rec in seg.first_record..seg.first_record.saturating_add(seg.records) {
                if let Some(slot) = rec_map.get_mut(rec) {
                    *slot = li;
                }
            }
        }
        let short_name = trimmed(enc, key);
        let name = long_names
            .get(key)
            .map(|v| trimmed(enc, v))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| short_name.clone());
        let missing = build_missing(pv, width > 0, &ctx, &mut warnings);
        variables.push(Variable {
            name,
            short_name,
            label: pv.label.as_ref().map(|l| text(enc, l)),
            width,
            print_format: Format::from_raw(pv.print),
            write_format: Format::from_raw(pv.write),
            measure: Measure::Unknown,
            alignment: None,
            display_width: None,
            missing,
        });
        plan.push(Field {
            width,
            records: segs.iter().map(|s| s.records).sum(),
            segments: if width > 0 {
                segs.iter().map(|s| (s.width, s.records)).collect()
            } else {
                Vec::new()
            },
        });
        first_phys.push(i);
        i += segs.len();
    }

    apply_display(&raw, &mut variables, &first_phys, &mut warnings);

    // Value labels (types 3 and 4).
    let mut value_labels = Vec::new();
    for set in &raw.label_sets {
        let mut vars: Vec<usize> = Vec::new();
        for rec in &set.records {
            match rec_map.get(*rec).copied().filter(|v| *v != NO_VAR) {
                Some(v) if !vars.contains(&v) => vars.push(v),
                Some(_) => {}
                None => warnings.push(format!("value label refers to bad variable index {rec}")),
            }
        }
        let is_string = vars
            .first()
            .and_then(|v| variables.get(*v))
            .is_some_and(Variable::is_string);
        let labels = set
            .labels
            .iter()
            .map(|(value, label)| ValueLabel {
                value: if is_string {
                    Value::String(trimmed(enc, value))
                } else {
                    numeric(endian.f64(*value), sysmis)
                },
                label: text(enc, label),
            })
            .collect();
        value_labels.push(ValueLabelSet {
            variables: vars,
            labels,
        });
    }

    // Name lookup for long-string records (long name first, then short name).
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for (idx, v) in variables.iter().enumerate() {
        by_name.entry(v.name.clone()).or_insert(idx);
    }
    for (idx, v) in variables.iter().enumerate() {
        by_name.entry(v.short_name.clone()).or_insert(idx);
    }
    for ll in &raw.long_labels {
        let name = trimmed(enc, &ll.name);
        match by_name.get(&name) {
            Some(idx) => value_labels.push(ValueLabelSet {
                variables: vec![*idx],
                labels: ll
                    .labels
                    .iter()
                    .map(|(v, l)| ValueLabel {
                        value: Value::String(trimmed(enc, v)),
                        label: text(enc, l),
                    })
                    .collect(),
            }),
            None => warnings.push(format!(
                "long string value labels for unknown variable {name}"
            )),
        }
    }
    for lm in &raw.long_missing {
        let name = trimmed(enc, &lm.name);
        match by_name.get(&name).and_then(|i| variables.get_mut(*i)) {
            Some(var) => {
                var.missing = MissingValues::Discrete(
                    lm.values
                        .iter()
                        .map(|v| Value::String(trimmed(enc, v)))
                        .collect(),
                );
            }
            None => warnings.push(format!(
                "long string missing values for unknown variable {name}"
            )),
        }
    }

    let weight_variable = usize::try_from(raw.header.weight_index)
        .ok()
        .filter(|w| *w > 0)
        .and_then(|w| rec_map.get(w).copied())
        .filter(|v| *v != NO_VAR);
    let case_count = match raw.ncases64 {
        Some(n) if n >= 0 => u64::try_from(n).ok(),
        _ => u64::try_from(raw.header.ncases).ok(),
    };
    let h = &raw.header;
    let header = Header {
        product: trimmed(enc, &h.product),
        layout_code: h.layout_code,
        endianness: h.endian,
        compression: h.compression,
        nominal_case_size: h.nominal_case_size,
        weight_variable,
        case_count,
        bias: h.bias,
        creation_date: trimmed(enc, &h.creation_date),
        creation_time: trimmed(enc, &h.creation_time),
        file_label: trimmed(enc, &h.file_label),
    };
    let file = SavFile {
        header,
        variables,
        value_labels,
        documents: raw.documents.iter().map(|d| trimmed(enc, d)).collect(),
        encoding: raw
            .encoding
            .as_ref()
            .map(|e| String::from_utf8_lossy(trim(e)).into_owned()),
        encoding_used: enc.name().to_string(),
        unknown_extensions: raw.unknown,
        warnings,
        rows: Vec::new(),
    };
    Ok(Assembled {
        file,
        plan,
        sysmis,
        encoding: enc,
    })
}

/// Subtype 11: one entry per variable record (continuations excluded) times 2 or 3. Files with
/// very long strings are ambiguous (segments may or may not be counted), so both layouts are
/// tried.
fn apply_display(
    raw: &RawDict,
    variables: &mut [Variable],
    first_phys: &[usize],
    warnings: &mut Vec<String>,
) {
    let Some(display) = &raw.display else {
        return;
    };
    let len = display.len();
    let n_phys = raw.vars.len();
    let n_log = variables.len();
    let (stride, physical) = if len == n_phys.saturating_mul(3) {
        (3, true)
    } else if len == n_phys.saturating_mul(2) {
        (2, true)
    } else if len == n_log.saturating_mul(3) {
        (3, false)
    } else if len == n_log.saturating_mul(2) {
        (2, false)
    } else {
        warnings.push("variable display record does not match the variable count".into());
        return;
    };
    for (li, var) in variables.iter_mut().enumerate() {
        let idx = if physical {
            first_phys.get(li).copied().unwrap_or(li)
        } else {
            li
        };
        let start = idx.saturating_mul(stride);
        let Some(chunk) = display.get(start..start.saturating_add(stride)) else {
            continue;
        };
        var.measure = match chunk.first() {
            Some(1) => Measure::Nominal,
            Some(2) => Measure::Ordinal,
            Some(3) => Measure::Scale,
            _ => Measure::Unknown,
        };
        if stride == 3 {
            var.display_width = chunk.get(1).and_then(|w| u32::try_from(*w).ok());
        }
        var.alignment = match chunk.get(stride - 1) {
            Some(0) => Some(Alignment::Left),
            Some(1) => Some(Alignment::Right),
            Some(2) => Some(Alignment::Centre),
            _ => None,
        };
    }
}
