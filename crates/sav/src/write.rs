//! Writer for the system-file records [`crate::read`] parses.
//!
//! Little-endian, layout code 2, bytecode compression with bias 100. The machine-integer
//! record is the SPSS 31 / `spssio32.dll` profile observed in `oracle/exports/stat_default`
//! (those version numbers are not recovered by the reader, so the raw bytes have to be written
//! explicitly). Very long strings follow the public segment formula `n = (w + 251) / 252`;
//! the payload is packed the way [`crate::data`] concatenates segments (see [`string_segments`]).

use crate::error::{Error, ErrorKind};
use crate::model::{Alignment, Format, Measure, MissingValues, Value, ValueLabel};
use std::time::{SystemTime, UNIX_EPOCH};

/// One logical variable, already named and formatted.
///
/// `width` 0 is numeric. A string wider than 255 bytes is split into the segments the reader
/// merges. `role` is the Statistics attribute code: 0 input, 1 target, 2 both, 3 none,
/// 4 partition, 5 split.
#[derive(Debug, Clone)]
pub struct OutVariable {
    pub short_name: String,
    pub long_name: String,
    pub label: Option<String>,
    pub width: usize,
    pub print_format: Format,
    pub write_format: Format,
    pub measure: Measure,
    pub alignment: Alignment,
    pub display_width: u32,
    pub missing: MissingValues,
    pub value_labels: Vec<ValueLabel>,
    pub role: i32,
}

const PRODUCT: [u8; 60] = *b"@(#) SPSS DATA FILE MS Windows 31.0.1.0 spssio32.dll        ";
const SYSMIS: f64 = -f64::MAX;
/// Print/write format on a string continuation record (`stat_default`, the `s` continuations).
const CONTINUATION_FORMAT: i32 = 0x0001_1d01;
const MAX_STRING: usize = 32_767;
const MAX_CELLS: usize = 64_000_000;

/// Writes a system file. `rows[i]` has one [`Value`] per variable.
pub fn write_bytes(variables: &[OutVariable], rows: &[Vec<Value>]) -> Result<Vec<u8>, Error> {
    validate(variables, rows)?;
    let plans = plan_variables(variables)?;
    let slots: usize = plans.iter().map(|plan| plan.slots).sum();
    let cells = rows.len().saturating_mul(slots);
    if cells > MAX_CELLS {
        return Err(err("too many cells to write"));
    }
    let nominal = i32_of(slots, "case size")?;
    let ncases = i32::try_from(rows.len()).unwrap_or(-1);
    let ncases64 = i64::try_from(rows.len()).map_err(|_| err("case count does not fit in i64"))?;

    let mut out = Vec::new();
    let (date, time) = creation_stamp();
    push_header(&mut out, nominal, ncases, &date, &time)?;

    let mut rec_index: i32 = 1;
    let mut label_sets: Vec<LabelSet> = Vec::new();
    let mut very_long: Vec<(String, usize)> = Vec::new();
    let mut long_labels: Vec<Vec<u8>> = Vec::new();
    let mut long_missing: Vec<Vec<u8>> = Vec::new();
    let mut taken_names: Vec<String> = variables.iter().map(|var| var.short_name.clone()).collect();

    for (var, plan) in variables.iter().zip(&plans) {
        let start = rec_index;
        let (missing_code, missing_raw) = record_missing(var)?;
        for (segment_index, segment) in plan.segments.iter().enumerate() {
            let first = segment_index == 0;
            let name = if first {
                pad_name(&var.short_name)?
            } else {
                pad_name(&allocate_segment_name(&mut taken_names)?)?
            };
            let label = if first { var.label.as_deref() } else { None };
            let (code, raw) = if first {
                (missing_code, missing_raw.as_slice())
            } else {
                (0, &[][..])
            };
            let (print, write) = if first {
                (format_raw(var.print_format), format_raw(var.write_format))
            } else {
                let fmt = string_format(segment.declared)?;
                (format_raw(fmt), format_raw(fmt))
            };
            push_variable(
                &mut out,
                VariableRecord {
                    type_code: i32_of(segment.declared, "segment width")?,
                    name: &name,
                    label,
                    missing_code: code,
                    missing: raw,
                    print,
                    write,
                },
            )?;
            for _ in 1..segment.slots {
                push_continuation(&mut out)?;
            }
            rec_index = rec_index
                .checked_add(i32_of(segment.slots, "dictionary index")?)
                .ok_or_else(|| err("dictionary index overflow"))?;
        }
        if var.width > 255 {
            very_long.push((var.short_name.clone(), var.width));
        }
        if !var.value_labels.is_empty() {
            if var.width > 8 {
                long_labels.push(long_label_bytes(var)?);
            } else {
                label_sets.push(LabelSet {
                    index: start,
                    string: var.width > 0,
                    labels: var.value_labels.clone(),
                });
            }
        }
        if var.width > 8
            && let MissingValues::Discrete(values) = &var.missing
            && !values.is_empty()
        {
            long_missing.push(long_missing_bytes(var, values)?);
        }
    }

    for set in &label_sets {
        push_value_labels(&mut out, set)?;
    }
    push_ext(&mut out, 3, 4, 8, &machine_integer());
    push_ext(&mut out, 4, 8, 3, &machine_float());
    if !variables.is_empty() {
        let display = display_bytes(variables)?;
        let count = i32_of(variables.len().saturating_mul(3), "display count")?;
        push_ext(&mut out, 11, 4, count, &display);
        let names = long_name_bytes(variables)?;
        push_ext(&mut out, 13, 1, i32_of(names.len(), "long names")?, &names);
    }
    if !very_long.is_empty() {
        let bytes = very_long_bytes(&very_long)?;
        push_ext(
            &mut out,
            14,
            1,
            i32_of(bytes.len(), "very long strings")?,
            &bytes,
        );
    }
    let mut cases = Vec::new();
    push_i64(&mut cases, 1);
    push_i64(&mut cases, ncases64);
    push_ext(&mut out, 16, 8, 2, &cases);
    if !variables.is_empty() {
        let roles = role_bytes(variables)?;
        push_ext(&mut out, 18, 1, i32_of(roles.len(), "roles")?, &roles);
    }
    push_ext(&mut out, 20, 1, 5, b"UTF-8");
    for bytes in &long_labels {
        push_ext(
            &mut out,
            21,
            1,
            i32_of(bytes.len(), "long value labels")?,
            bytes,
        );
    }
    for bytes in &long_missing {
        push_ext(
            &mut out,
            22,
            1,
            i32_of(bytes.len(), "long missing values")?,
            bytes,
        );
    }
    push_i32(&mut out, 999);
    push_i32(&mut out, 0);

    {
        let mut compressed = Compressor::new(&mut out);
        for row in rows {
            for (var, plan) in variables.iter().zip(&plans) {
                let value = row
                    .get(plan.index)
                    .ok_or_else(|| err("row is shorter than the variable list"))?;
                emit_cell(&mut compressed, var, plan, value)?;
            }
        }
        compressed.finish();
    }
    Ok(out)
}

struct Segment {
    /// Declared byte width. 0 for a numeric variable.
    declared: usize,
    slots: usize,
}

struct VarPlan {
    index: usize,
    slots: usize,
    segments: Vec<Segment>,
    /// Sum of the segments' declared widths. The reader concatenates that many bytes.
    payload: usize,
}

struct LabelSet {
    index: i32,
    string: bool,
    labels: Vec<ValueLabel>,
}

fn plan_variables(variables: &[OutVariable]) -> Result<Vec<VarPlan>, Error> {
    let mut plans = Vec::with_capacity(variables.len());
    for (index, var) in variables.iter().enumerate() {
        let widths = if var.width == 0 {
            vec![0]
        } else {
            string_segments(var.width)?
        };
        let mut segments = Vec::with_capacity(widths.len());
        let mut slots = 0usize;
        let mut payload = 0usize;
        for declared in widths {
            let count = if declared == 0 {
                1
            } else {
                declared.div_ceil(8)
            };
            slots = slots.saturating_add(count);
            payload = payload.saturating_add(declared);
            segments.push(Segment {
                declared,
                slots: count,
            });
        }
        plans.push(VarPlan {
            index,
            slots,
            segments,
            payload,
        });
    }
    Ok(plans)
}

/// Segment declared widths. Non-last segments are 255; the last is `w - (n - 1) * 252`,
/// with `n = (w + 251) / 252`. The reader keeps every declared byte and then truncates the
/// concatenation to the logical width, so the extra 3 bytes per non-last segment sit at the
/// end of that concatenation (not as a gap inside the string).
fn string_segments(width: usize) -> Result<Vec<usize>, Error> {
    if width == 0 || width > MAX_STRING {
        return Err(err(format!("string width {width} is outside 1..=32767")));
    }
    if width <= 255 {
        return Ok(vec![width]);
    }
    let n = width.saturating_add(251) / 252;
    let last = width.saturating_sub(n.saturating_sub(1).saturating_mul(252));
    if n < 2 || last == 0 || last > 255 {
        return Err(err("very long string segment width is invalid"));
    }
    let mut segments = vec![255; n - 1];
    segments.push(last);
    Ok(segments)
}

fn validate(variables: &[OutVariable], rows: &[Vec<Value>]) -> Result<(), Error> {
    let mut seen = Vec::new();
    for var in variables {
        check_short(&var.short_name)?;
        check_long(&var.long_name)?;
        if var.role < 0 || var.role > 5 {
            return Err(err(format!("role {} is outside 0..=5", var.role)));
        }
        if var.width > MAX_STRING {
            return Err(err(format!(
                "{}: string width {} exceeds 32767",
                var.short_name, var.width
            )));
        }
        if seen.iter().any(|name| name == &var.short_name) {
            return Err(err(format!("duplicate short name {}", var.short_name)));
        }
        seen.push(var.short_name.clone());
        if let Some(label) = &var.label
            && label.len() > 255
        {
            return Err(err(format!(
                "{}: variable label is longer than 255 bytes",
                var.short_name
            )));
        }
        for entry in &var.value_labels {
            if entry.label.len() > 255 {
                return Err(err(format!(
                    "{}: a value label is longer than 255 bytes",
                    var.short_name
                )));
            }
        }
    }
    for (index, row) in rows.iter().enumerate() {
        if row.len() != variables.len() {
            return Err(err(format!(
                "row {index} has {} values, expected {}",
                row.len(),
                variables.len()
            )));
        }
    }
    Ok(())
}

fn check_short(name: &str) -> Result<(), Error> {
    let bytes = name.as_bytes();
    let Some(first) = bytes.first() else {
        return Err(err("short name is empty"));
    };
    if bytes.len() > 8 || !is_name_start(*first) || !bytes.iter().copied().all(is_name_char) {
        return Err(err(format!("short name {name:?} is not an SPSS name")));
    }
    Ok(())
}

fn check_long(name: &str) -> Result<(), Error> {
    if name.is_empty() || name.len() > 64 {
        return Err(err(format!(
            "long name {name:?} is empty or longer than 64 bytes"
        )));
    }
    if name
        .bytes()
        .any(|byte| matches!(byte, b'\t' | b'=' | 0 | b'/' | b':' | b'\'' | b'\n'))
    {
        return Err(err(format!(
            "long name {name:?} contains a character the name or role record uses as a separator"
        )));
    }
    Ok(())
}

fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_uppercase() || byte == b'@'
}

fn is_name_char(byte: u8) -> bool {
    byte.is_ascii_uppercase()
        || byte.is_ascii_digit()
        || matches!(byte, b'@' | b'#' | b'$' | b'_' | b'.')
}

fn allocate_segment_name(taken: &mut Vec<String>) -> Result<String, Error> {
    for n in 1..=9_999_999u32 {
        let name = format!("Z{n:07}");
        if !taken.iter().any(|existing| existing == &name) {
            taken.push(name.clone());
            return Ok(name);
        }
    }
    Err(err("could not allocate a segment name"))
}

fn pad_name(name: &str) -> Result<[u8; 8], Error> {
    let mut bytes = [b' '; 8];
    let raw = name.as_bytes();
    if raw.len() > 8 {
        return Err(err("short name is longer than 8 bytes"));
    }
    if let Some(slot) = bytes.get_mut(..raw.len()) {
        slot.copy_from_slice(raw);
    }
    Ok(bytes)
}

fn record_missing(var: &OutVariable) -> Result<(i32, Vec<[u8; 8]>), Error> {
    if var.width > 8 {
        if matches!(
            var.missing,
            MissingValues::Range { .. } | MissingValues::RangeAndDiscrete { .. }
        ) {
            return Err(err(format!(
                "{}: a string variable cannot store a missing-value range",
                var.short_name
            )));
        }
        return Ok((0, Vec::new()));
    }
    match &var.missing {
        MissingValues::None => Ok((0, Vec::new())),
        MissingValues::Discrete(values) if values.is_empty() => Ok((0, Vec::new())),
        MissingValues::Discrete(values) => {
            if values.len() > 3 {
                return Err(err(format!(
                    "{}: more than 3 discrete missing values",
                    var.short_name
                )));
            }
            let mut raw = Vec::with_capacity(values.len());
            for value in values {
                raw.push(discrete_bytes(var, value)?);
            }
            Ok((i32_of(values.len(), "missing values")?, raw))
        }
        MissingValues::Range { low, high } => {
            if var.width != 0 {
                return Err(err(format!(
                    "{}: a string variable cannot store a missing-value range",
                    var.short_name
                )));
            }
            Ok((-2, vec![low.to_le_bytes(), high.to_le_bytes()]))
        }
        MissingValues::RangeAndDiscrete { low, high, value } => {
            if var.width != 0 {
                return Err(err(format!(
                    "{}: a string variable cannot store a missing-value range",
                    var.short_name
                )));
            }
            Ok((
                -3,
                vec![low.to_le_bytes(), high.to_le_bytes(), value.to_le_bytes()],
            ))
        }
    }
}

fn discrete_bytes(var: &OutVariable, value: &Value) -> Result<[u8; 8], Error> {
    if var.width == 0 {
        return match value {
            Value::Number(number) => Ok(number.to_le_bytes()),
            Value::SystemMissing => Ok(SYSMIS.to_le_bytes()),
            Value::String(_) => Err(err(format!(
                "{}: a numeric missing value is a string",
                var.short_name
            ))),
        };
    }
    match value {
        Value::String(text) => pad8(text, &var.short_name),
        Value::SystemMissing => Ok([b' '; 8]),
        Value::Number(_) => Err(err(format!(
            "{}: a string missing value is a number",
            var.short_name
        ))),
    }
}

fn pad8(text: &str, name: &str) -> Result<[u8; 8], Error> {
    let raw = text.as_bytes();
    if raw.len() > 8 {
        return Err(err(format!(
            "{name}: missing value {text:?} is longer than 8 bytes"
        )));
    }
    let mut bytes = [b' '; 8];
    if let Some(slot) = bytes.get_mut(..raw.len()) {
        slot.copy_from_slice(raw);
    }
    Ok(bytes)
}

fn push_header(
    out: &mut Vec<u8>,
    nominal: i32,
    ncases: i32,
    date: &[u8; 9],
    time: &[u8; 8],
) -> Result<(), Error> {
    let start = out.len();
    out.extend_from_slice(b"$FL2");
    out.extend_from_slice(&PRODUCT);
    push_i32(out, 2);
    push_i32(out, nominal);
    push_i32(out, 1);
    push_i32(out, 0);
    push_i32(out, ncases);
    push_f64(out, 100.0);
    out.extend_from_slice(date);
    out.extend_from_slice(time);
    out.extend(std::iter::repeat_n(b' ', 64));
    out.extend_from_slice(&[0, 0, 0]);
    if out.len().saturating_sub(start) != 176 {
        return Err(err("file header is not 176 bytes"));
    }
    Ok(())
}

struct VariableRecord<'a> {
    type_code: i32,
    name: &'a [u8; 8],
    label: Option<&'a str>,
    missing_code: i32,
    missing: &'a [[u8; 8]],
    print: i32,
    write: i32,
}

fn push_variable(out: &mut Vec<u8>, record: VariableRecord<'_>) -> Result<(), Error> {
    push_i32(out, 2);
    push_i32(out, record.type_code);
    let label_bytes = record
        .label
        .map(str::as_bytes)
        .filter(|bytes| !bytes.is_empty());
    push_i32(out, i32::from(label_bytes.is_some()));
    push_i32(out, record.missing_code);
    push_i32(out, record.print);
    push_i32(out, record.write);
    out.extend_from_slice(record.name);
    if let Some(bytes) = label_bytes {
        push_i32(out, i32_of(bytes.len(), "variable label")?);
        let padded = bytes.len().div_ceil(4).saturating_mul(4);
        out.extend_from_slice(bytes);
        out.extend(std::iter::repeat_n(
            b' ',
            padded.saturating_sub(bytes.len()),
        ));
    }
    for value in record.missing {
        out.extend_from_slice(value);
    }
    Ok(())
}

fn push_continuation(out: &mut Vec<u8>) -> Result<(), Error> {
    push_variable(
        out,
        VariableRecord {
            type_code: -1,
            name: b"        ",
            label: None,
            missing_code: 0,
            missing: &[],
            print: CONTINUATION_FORMAT,
            write: CONTINUATION_FORMAT,
        },
    )
}

fn push_value_labels(out: &mut Vec<u8>, set: &LabelSet) -> Result<(), Error> {
    push_i32(out, 3);
    push_i32(out, i32_of(set.labels.len(), "value labels")?);
    for entry in &set.labels {
        out.extend_from_slice(&label_value(set.string, entry)?);
        let text = entry.label.as_bytes();
        let len = u8::try_from(text.len()).map_err(|_| err("value label longer than 255 bytes"))?;
        out.push(len);
        let padded = (text.len().saturating_add(1))
            .div_ceil(8)
            .saturating_mul(8)
            .saturating_sub(1);
        out.extend_from_slice(text);
        out.extend(std::iter::repeat_n(b' ', padded.saturating_sub(text.len())));
    }
    push_i32(out, 4);
    push_i32(out, 1);
    push_i32(out, set.index);
    Ok(())
}

fn label_value(string: bool, entry: &ValueLabel) -> Result<[u8; 8], Error> {
    if string {
        return match &entry.value {
            Value::String(text) => pad8(text, "value label"),
            Value::SystemMissing => Ok([b' '; 8]),
            Value::Number(_) => Err(err("a string value label is a number")),
        };
    }
    match &entry.value {
        Value::Number(number) => Ok(number.to_le_bytes()),
        Value::SystemMissing => Ok(SYSMIS.to_le_bytes()),
        Value::String(_) => Err(err("a numeric value label is a string")),
    }
}

fn long_label_bytes(var: &OutVariable) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    push_counted(&mut out, var.short_name.as_bytes())?;
    push_i32(&mut out, i32_of(var.width, "string width")?);
    push_i32(&mut out, i32_of(var.value_labels.len(), "value labels")?);
    for entry in &var.value_labels {
        let value: &[u8] = match &entry.value {
            Value::String(text) => text.as_bytes(),
            Value::SystemMissing => b"",
            Value::Number(_) => {
                return Err(err(format!(
                    "{}: a long-string value label is a number",
                    var.short_name
                )));
            }
        };
        push_counted(&mut out, value)?;
        push_counted(&mut out, entry.label.as_bytes())?;
    }
    Ok(out)
}

fn long_missing_bytes(var: &OutVariable, values: &[Value]) -> Result<Vec<u8>, Error> {
    if values.len() > 255 {
        return Err(err(format!(
            "{}: more than 255 long-string missing values",
            var.short_name
        )));
    }
    let mut out = Vec::new();
    push_counted(&mut out, var.short_name.as_bytes())?;
    let count = u8::try_from(values.len()).map_err(|_| err("missing-value count does not fit"))?;
    out.push(count);
    for value in values {
        let text = match value {
            Value::String(text) => text.as_str(),
            Value::SystemMissing => "",
            Value::Number(_) => {
                return Err(err(format!(
                    "{}: a long-string missing value is a number",
                    var.short_name
                )));
            }
        };
        if text.len() > var.width {
            return Err(err(format!(
                "{}: missing value is longer than the variable",
                var.short_name
            )));
        }
        push_counted(&mut out, text.as_bytes())?;
    }
    Ok(out)
}

fn push_counted(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), Error> {
    push_i32(out, i32_of(bytes.len(), "counted bytes")?);
    out.extend_from_slice(bytes);
    Ok(())
}

fn display_bytes(variables: &[OutVariable]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    for var in variables {
        let measure = match var.measure {
            Measure::Nominal => 1,
            Measure::Ordinal => 2,
            Measure::Scale => 3,
            Measure::Unknown => 0,
        };
        let align = match var.alignment {
            Alignment::Left => 0,
            Alignment::Right => 1,
            Alignment::Centre => 2,
        };
        push_i32(&mut out, measure);
        let width = i32::try_from(var.display_width)
            .map_err(|_| err("display width does not fit in i32"))?;
        push_i32(&mut out, width);
        push_i32(&mut out, align);
    }
    Ok(out)
}

fn long_name_bytes(variables: &[OutVariable]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    for (index, var) in variables.iter().enumerate() {
        if index > 0 {
            out.push(b'\t');
        }
        out.extend_from_slice(var.short_name.as_bytes());
        out.push(b'=');
        out.extend_from_slice(var.long_name.as_bytes());
    }
    Ok(out)
}

fn role_bytes(variables: &[OutVariable]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    for (index, var) in variables.iter().enumerate() {
        if index > 0 {
            out.push(b'/');
        }
        out.extend_from_slice(var.long_name.as_bytes());
        out.extend_from_slice(b":$@Role('");
        out.extend_from_slice(var.role.to_string().as_bytes());
        out.extend_from_slice(b"'\n)");
    }
    Ok(out)
}

fn very_long_bytes(pairs: &[(String, usize)]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    for (index, (name, width)) in pairs.iter().enumerate() {
        if index > 0 {
            out.push(0);
            out.push(b'\t');
        }
        out.extend_from_slice(name.as_bytes());
        out.push(b'=');
        if *width > 99_999 {
            return Err(err("very long string width does not fit in 5 digits"));
        }
        out.extend_from_slice(format!("{width:05}").as_bytes());
    }
    Ok(out)
}

fn machine_integer() -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    for value in [31, 0, 100, 720, 1, 1, 2, 65001] {
        push_i32(&mut out, value);
    }
    out
}

fn machine_float() -> Vec<u8> {
    let mut out = Vec::with_capacity(24);
    push_f64(&mut out, SYSMIS);
    push_f64(&mut out, f64::MAX);
    push_f64(&mut out, f64::from_bits(SYSMIS.to_bits().wrapping_sub(1)));
    out
}

fn string_format(width: usize) -> Result<Format, Error> {
    let stored = u8::try_from(width).map_err(|_| err("segment width does not fit in a format"))?;
    Ok(Format {
        type_code: 1,
        width: stored,
        decimals: 0,
    })
}

fn format_raw(format: Format) -> i32 {
    i32::from_le_bytes([format.decimals, format.width, format.type_code, 0])
}

fn emit_cell(
    out: &mut Compressor,
    var: &OutVariable,
    plan: &VarPlan,
    value: &Value,
) -> Result<(), Error> {
    if var.width == 0 {
        return emit_number(out, var, value);
    }
    let text = match value {
        Value::String(text) => text.as_str(),
        Value::SystemMissing => "",
        Value::Number(_) => {
            return Err(err(format!(
                "{}: a string cell is a number",
                var.short_name
            )));
        }
    };
    let mut payload = vec![b' '; plan.payload];
    let prefix = utf8_prefix(text, var.width);
    if let Some(slot) = payload.get_mut(..prefix.len()) {
        slot.copy_from_slice(prefix);
    }
    let mut offset = 0usize;
    for segment in &plan.segments {
        let end = offset.saturating_add(segment.declared);
        let declared = payload.get(offset..end).unwrap_or_default();
        let mut stored = vec![b' '; segment.slots.saturating_mul(8)];
        if let Some(slot) = stored.get_mut(..declared.len()) {
            slot.copy_from_slice(declared);
        }
        for chunk in stored.chunks(8) {
            emit_chars(out, chunk);
        }
        offset = end;
    }
    Ok(())
}

fn emit_number(out: &mut Compressor, var: &OutVariable, value: &Value) -> Result<(), Error> {
    match value {
        Value::SystemMissing => out.push(255, None),
        Value::Number(number) => {
            if !number.is_finite() || *number == SYSMIS {
                out.push(255, None);
            } else if let Some(code) = compressed_int(*number) {
                out.push(code, None);
            } else {
                out.push(253, Some(number.to_le_bytes()));
            }
        }
        Value::String(_) => {
            return Err(err(format!(
                "{}: a numeric cell is a string",
                var.short_name
            )));
        }
    }
    Ok(())
}

fn emit_chars(out: &mut Compressor, chunk: &[u8]) {
    let mut bytes = [b' '; 8];
    let n = chunk.len().min(8);
    if let Some(slot) = bytes.get_mut(..n)
        && let Some(src) = chunk.get(..n)
    {
        slot.copy_from_slice(src);
    }
    if bytes == [b' '; 8] {
        out.push(254, None);
    } else {
        out.push(253, Some(bytes));
    }
}

/// Bytecode for an integer in [-99, 151]. The code is `value + 100` (`stat_default`: 0, 9, 10
/// and -10 are single codes; -3.5 is not).
fn compressed_int(value: f64) -> Option<u8> {
    if !(-99.0..=151.0).contains(&value) {
        return None;
    }
    let rounded = value.round();
    if value != rounded {
        return None;
    }
    let code = i32::try_from(rounded as i64).ok()?.checked_add(100)?;
    u8::try_from(code)
        .ok()
        .filter(|code| (1..=251).contains(code))
}

fn utf8_prefix(text: &str, max_bytes: usize) -> &[u8] {
    if text.len() <= max_bytes {
        return text.as_bytes();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).map(str::as_bytes).unwrap_or_default()
}

/// Bytecode stream: groups of 8 codes, then the raw 8-byte payloads of code 253 in that group.
/// A short final group is padded with 0. Code 252 (end of file) is not written.
struct Compressor<'a> {
    out: &'a mut Vec<u8>,
    codes: Vec<u8>,
    payloads: Vec<u8>,
}

impl<'a> Compressor<'a> {
    fn new(out: &'a mut Vec<u8>) -> Self {
        Self {
            out,
            codes: Vec::with_capacity(8),
            payloads: Vec::new(),
        }
    }

    fn push(&mut self, code: u8, payload: Option<[u8; 8]>) {
        self.codes.push(code);
        if let Some(bytes) = payload {
            self.payloads.extend_from_slice(&bytes);
        }
        if self.codes.len() == 8 {
            self.flush();
        }
    }

    fn finish(&mut self) {
        if !self.codes.is_empty() {
            self.flush();
        }
    }

    fn flush(&mut self) {
        while self.codes.len() < 8 {
            self.codes.push(0);
        }
        self.out.extend_from_slice(&self.codes);
        self.out.extend_from_slice(&self.payloads);
        self.codes.clear();
        self.payloads.clear();
    }
}

fn push_ext(out: &mut Vec<u8>, subtype: i32, size: i32, count: i32, data: &[u8]) {
    push_i32(out, 7);
    push_i32(out, subtype);
    push_i32(out, size);
    push_i32(out, count);
    out.extend_from_slice(data);
}

fn push_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_f64(out: &mut Vec<u8>, value: f64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn i32_of(value: usize, what: &str) -> Result<i32, Error> {
    i32::try_from(value).map_err(|_| err(format!("{what} does not fit in i32")))
}

fn err(message: impl Into<String>) -> Error {
    Error::new(0, ErrorKind::Write(message.into()))
}

fn creation_stamp() -> ([u8; 9], [u8; 8]) {
    let fallback = (*b"01 Jan 70", *b"00:00:00");
    let Ok(elapsed) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return fallback;
    };
    let secs = elapsed.as_secs();
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let tod = secs % 86_400;
    let Some((year, month, day)) = civil_from_days(days) else {
        return fallback;
    };
    let mon = match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => return fallback,
    };
    let year2 = year.rem_euclid(100);
    let date = format!("{day:2} {mon} {year2:02}");
    let hh = tod / 3600;
    let mm = (tod % 3600) / 60;
    let ss = tod % 60;
    let time = format!("{hh:02}:{mm:02}:{ss:02}");
    match (exact_bytes::<9>(&date), exact_bytes::<8>(&time)) {
        (Some(date), Some(time)) => (date, time),
        _ => fallback,
    }
}

fn exact_bytes<const N: usize>(text: &str) -> Option<[u8; N]> {
    text.as_bytes().try_into().ok()
}

/// Howard Hinnant's civil-from-days, for a non-negative Unix day count.
fn civil_from_days(days: i64) -> Option<(i64, u32, u32)> {
    let z = days.checked_add(719_468)?;
    if z < 0 {
        return None;
    }
    let era = z.div_euclid(146_097);
    let doe = u64::try_from(z.checked_sub(era.checked_mul(146_097)?)?).ok()?;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = i64::try_from(yoe)
        .ok()?
        .checked_add(era.checked_mul(400)?)?;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y.checked_add(1)? } else { y };
    let month = u32::try_from(month).ok()?;
    let day = u32::try_from(day).ok()?;
    if (1..=12).contains(&month) && (1..=31).contains(&day) {
        Some((year, month, day))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Alignment, Measure, MissingValues, read};

    fn numeric(name: &str) -> OutVariable {
        OutVariable {
            short_name: name.to_string(),
            long_name: name.to_string(),
            label: None,
            width: 0,
            print_format: Format {
                type_code: 5,
                width: 10,
                decimals: 0,
            },
            write_format: Format {
                type_code: 5,
                width: 10,
                decimals: 0,
            },
            measure: Measure::Scale,
            alignment: Alignment::Right,
            display_width: 8,
            missing: MissingValues::None,
            value_labels: Vec::new(),
            role: 0,
        }
    }

    #[test]
    fn numbers_and_a_very_long_string_round_trip() {
        let mut wide = numeric("S");
        wide.short_name = "S".into();
        wide.long_name = "text".into();
        wide.width = 300;
        wide.print_format = Format {
            type_code: 1,
            width: 255,
            decimals: 0,
        };
        wide.write_format = wide.print_format;
        wide.measure = Measure::Nominal;
        wide.alignment = Alignment::Left;
        wide.display_width = 50;
        wide.missing = MissingValues::Discrete(vec![Value::String("NA".into())]);
        wide.value_labels = vec![ValueLabel {
            value: Value::String("A".into()),
            label: "alpha".into(),
        }];
        let mut number = numeric("ID");
        number.missing = MissingValues::Discrete(vec![Value::Number(-99.0)]);
        let rows = vec![
            vec![Value::String("B".repeat(300)), Value::Number(0.0)],
            vec![Value::String("C".repeat(10)), Value::SystemMissing],
            vec![Value::String(String::new()), Value::Number(-3.5)],
        ];
        let bytes = write_bytes(&[wide, number], &rows).unwrap();
        let file = read(&bytes, &crate::Limits::default()).unwrap();
        assert_eq!(file.header.compression, crate::Compression::Bytecode);
        assert_eq!(file.header.bias, 100.0);
        assert_eq!(file.header.case_count, Some(3));
        assert_eq!(file.variables.len(), 2);
        assert_eq!(file.variables[0].width, 300);
        assert_eq!(file.variables[0].name, "text");
        assert_eq!(file.rows[0][0], Value::String("B".repeat(300)));
        assert_eq!(file.rows[1][0], Value::String("C".repeat(10)));
        assert_eq!(file.rows[2][0], Value::String(String::new()));
        assert_eq!(file.rows[0][1], Value::Number(0.0));
        assert_eq!(file.rows[1][1], Value::SystemMissing);
        assert_eq!(file.rows[2][1], Value::Number(-3.5));
        assert_eq!(
            file.variables[1].missing,
            MissingValues::Discrete(vec![Value::Number(-99.0)])
        );
        assert_eq!(
            file.variables[0].missing,
            MissingValues::Discrete(vec![Value::String("NA".into())])
        );
        assert_eq!(file.value_labels.len(), 1);
        assert_eq!(file.value_labels[0].labels[0].label, "alpha");
        assert!(bytes.len() > 176);
        assert_eq!(&bytes[..4], b"$FL2");
    }
}
