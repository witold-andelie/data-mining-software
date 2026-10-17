//! JSON export (`json_export!pasw.json`): writes the input table as one JSON value and passes
//! that table through unchanged.
//!
//! The node's file settings are not on the stream element. They are `Parameter` entries in the
//! extension attachment (`node@properties`, `data/0001.dat`), read when the node runs.
//! `build` rejects an `executionMode` other than `export_data`. A parameter this node does not
//! implement is rejected with [`OperatorError::Settings`] at run time, which is the first moment
//! the attachment bytes are available.
//!
//! Implemented parameters:
//!
//! | parameter | scripting property | accepted values |
//! |---|---|---|
//! | `full_filename` | `full_filename` | path; the file written is [`ExecContext::export_path`] |
//! | `stringFormat` | `string_format` | `records` (default when absent), `values` |
//! | `pythonexport.generate_import` | `generate_import` | `true` or `false`; does not change the file |
//! | `pythonexport.use_virtual_env` | — | `true` or `false`; the file is written here, not by a Python process |
//! | `pythonexport.export_syntax` | — | the default `modelerJson.write_json` script whose `'orient'` matches `stringFormat` |
//!
//! `pasw.annotation_*` parameters are labels and are ignored. Publish settings on the node
//! element (`autoGenerateImportNode`, `publishMetadata`, `outputFileName`, `publishParameters`,
//! `publishUseParameters`) do not change the file.
//!
//! Rules, each one visible in `oracle/exports/json_records/exports/json_records.json` unless named
//! otherwise:
//!
//! - The file is UTF-8 with no byte-order mark, no spaces and no trailing newline (`json_records`,
//!   `json_values`).
//! - `records` is a JSON array of objects, keys in field order (`json_records`). `values` is an
//!   array of arrays with no header (`json_values`).
//! - An integer column with no missing value stays a JSON integer (`id` is `1`). An integer column
//!   with any missing value is written as a JSON number with a decimal point (`iv` is `10.0`,
//!   and the missing cell is `null`). This is the promotion pandas applies before `to_json`.
//! - Reals are rounded to 10 decimal places, half to even, then trailing zeros are removed and one
//!   fractional digit is kept (`rv` `-3.5`, `0.0`, `10.0`, and `0.123456789` from
//!   `0.123456789012345`). The stream's decimal places are not used: the oracle stream saves 15
//!   and the export still has 10.
//! - A missing numeric or date value is JSON `null`. An empty string is `""` (`s` on the row with
//!   `id` 6, and `grp` on the row with `id` 8).
//! - Dates are UTC ISO-8601 with millisecond zeros (`"2020-01-15T00:00:00.000Z"`). A missing date
//!   is `null` (the row with `id` 7).
//! - Strings use JSON escapes: `"` and `\` are backslash-escaped, tab is `\t` (the row with `id`
//!   9), other controls and non-ASCII characters are `\uXXXX` (lowercase, `ensure_ascii`). `/` is
//!   not escaped.
//! - A `"` inside a string is escaped, not removed. The probe source cells `say ""hi""` and
//!   `q ""and"" semi;colon,comma` are `say hi` and `q and semi;colon,comma` in the oracle, because
//!   Modeler's variable-file import dropped those quote characters before the export ran.

use super::extension_params::{Param, flag, load_parameters, reject_lists, reject_unknown, text};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::format_real;
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Field, Storage, Table, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

const JSON_REAL_PLACES: usize = 10;

const KNOWN_PARAMETERS: &[&str] = &[
    "full_filename",
    "stringFormat",
    "pythonexport.generate_import",
    "pythonexport.use_virtual_env",
    "pythonexport.export_syntax",
];

#[derive(Clone, Copy)]
enum JsonFormat {
    Records,
    Values,
}

#[derive(Clone, Copy)]
enum JsonKind {
    Int,
    Float,
    Date,
    Text,
}

struct JsonExport {
    attachment: Option<String>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    match settings::text(node.settings.as_ref(), "executionMode")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("export_data") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "JSON export: executionMode {other:?} is not implemented"
            )));
        }
    }
    Ok(Box::new(JsonExport {
        attachment: node.properties_attachment.clone(),
    }))
}

impl Operator for JsonExport {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("JSON export needs one input".into()))?;
        let params = load_parameters(self.attachment.as_deref(), context, "JSON export")?;
        let settings = json_settings(&params)?;
        let path = context.export_path(&settings.path);
        let bytes = render(input, settings.format).into_bytes();
        write_bytes(&path, &bytes)?;
        Ok(input.as_ref().clone())
    }
}

struct JsonSettings {
    path: String,
    format: JsonFormat,
}

fn json_settings(params: &HashMap<String, Param>) -> Result<JsonSettings, OperatorError> {
    reject_unknown(params, KNOWN_PARAMETERS, "JSON export")?;
    reject_lists(params, "JSON export")?;
    let path = text(params, "full_filename")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OperatorError::Settings("JSON export: full_filename is missing".into()))?;
    let format = match text(params, "stringFormat").as_deref().map(str::trim) {
        None | Some("records") => JsonFormat::Records,
        Some("values") => JsonFormat::Values,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "JSON export: unknown stringFormat {other:?}"
            )));
        }
    };
    // Both flags are saved by the probes. Neither changes the bytes of the export.
    let _ = flag(params, "pythonexport.generate_import")?;
    let _ = flag(params, "pythonexport.use_virtual_env")?;
    if let Some(script) = text(params, "pythonexport.export_syntax")
        && !script.is_empty()
    {
        if !script.contains("write_json") {
            return Err(OperatorError::Settings(
                "JSON export: a custom export script is not implemented".into(),
            ));
        }
        let orient = match format {
            JsonFormat::Records => "'orient' : 'records'",
            JsonFormat::Values => "'orient' : 'values'",
        };
        if !script.contains(orient) {
            return Err(OperatorError::Settings(format!(
                "JSON export: export_syntax orient does not match stringFormat ({orient})"
            )));
        }
    }
    Ok(JsonSettings { path, format })
}

fn render(table: &Table, format: JsonFormat) -> String {
    let fields = table.model().fields();
    let kinds: Vec<JsonKind> = fields
        .iter()
        .enumerate()
        .map(|(index, field)| json_kind(field, table.rows(), index))
        .collect();
    let mut out = String::from("[");
    for (row_index, row) in table.rows().iter().enumerate() {
        if row_index > 0 {
            out.push(',');
        }
        match format {
            JsonFormat::Records => write_record(&mut out, fields, &kinds, row),
            JsonFormat::Values => write_values(&mut out, &kinds, row),
        }
    }
    out.push(']');
    out
}

fn write_record(out: &mut String, fields: &[Field], kinds: &[JsonKind], row: &[Value]) {
    out.push('{');
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&json_string(&field.name));
        out.push(':');
        let value = row.get(index).unwrap_or(&Value::Null);
        let kind = kinds.get(index).copied().unwrap_or(JsonKind::Text);
        out.push_str(&json_value(value, kind));
    }
    out.push('}');
}

fn write_values(out: &mut String, kinds: &[JsonKind], row: &[Value]) {
    out.push('[');
    for (index, kind) in kinds.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let value = row.get(index).unwrap_or(&Value::Null);
        out.push_str(&json_value(value, *kind));
    }
    out.push(']');
}

/// Integer columns become JSON numbers with a decimal point when any cell is missing, which is
/// how pandas stores an integer series that contains `NaN` (`json_records`, field `iv`).
fn json_kind(field: &Field, rows: &[Vec<Value>], index: usize) -> JsonKind {
    match field.storage {
        Storage::Real => JsonKind::Float,
        Storage::Integer => {
            let all_integers = rows
                .iter()
                .all(|row| matches!(row.get(index), Some(Value::Integer(_))));
            if all_integers {
                JsonKind::Int
            } else {
                JsonKind::Float
            }
        }
        Storage::Date => JsonKind::Date,
        _ => JsonKind::Text,
    }
}

fn json_value(value: &Value, kind: JsonKind) -> String {
    if matches!(value, Value::Null) {
        return "null".to_string();
    }
    match kind {
        JsonKind::Int => match value {
            Value::Integer(number) => number.to_string(),
            other => json_value_loose(other),
        },
        JsonKind::Float => match value {
            Value::Integer(number) => json_float(*number as f64),
            Value::Real(number) => json_float(*number),
            other => json_value_loose(other),
        },
        JsonKind::Date => match value {
            Value::Date(days) => json_date(*days),
            other => json_value_loose(other),
        },
        JsonKind::Text => json_value_loose(value),
    }
}

fn json_value_loose(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Integer(number) => number.to_string(),
        Value::Real(number) => json_float(*number),
        Value::String(text) => json_string(text),
        Value::Date(days) => json_date(*days),
        Value::Time(_) | Value::Timestamp(_) => json_string(&value.to_string()),
        Value::List(items) => {
            let mut out = String::from("[");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&json_value_loose(item));
            }
            out.push(']');
            out
        }
    }
}

/// `format_real` to 10 places, then the trailing zeros pandas drops. A fractional digit remains
/// so a whole number stays a JSON float (`10.0`, not `10`).
fn json_float(value: f64) -> String {
    if !value.is_finite() {
        return "null".to_string();
    }
    let text = format_real(value, JSON_REAL_PLACES);
    let Some((whole, fraction)) = text.split_once('.') else {
        return format!("{text}.0");
    };
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        format!("{whole}.0")
    } else {
        format!("{whole}.{fraction}")
    }
}

/// A date as pandas writes a UTC midnight timestamp (`date_format='iso'`).
fn json_date(days: i64) -> String {
    let day = Value::Date(days).to_string();
    if day.starts_with('<') {
        return json_string(&day);
    }
    let mut text = String::with_capacity(day.len().saturating_add(14));
    text.push_str(&day);
    text.push_str("T00:00:00.000Z");
    json_string(&text)
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || !c.is_ascii() => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    push_hex(&mut out, u32::from(*unit));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn push_hex(out: &mut String, unit: u32) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push_str("\\u");
    let unit = unit & 0xffff;
    for shift in [12, 8, 4, 0] {
        let nibble = ((unit >> shift) & 0xf) as usize;
        out.push(char::from(HEX[nibble]));
    }
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), OperatorError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| OperatorError::Io(format!("{}: {error}", parent.display())))?;
    }
    std::fs::write(path, bytes)
        .map_err(|error| OperatorError::Io(format!("{}: {error}", path.display())))
}
