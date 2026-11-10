//! Editable parameters actually consumed by the reference executor.
//!
//! Keys and encodings follow `docs/formats/nodes/`. Values use file encodings (including
//! inverted flags). Reading never adds defaults to the document; writing an unchanged value
//! preserves even the original spelling, whitespace, attributes and unknown children.
mod attachment;
mod properties;
mod specs;
#[cfg(test)]
mod tests;

pub use attachment::{read_attachment, write_attachment};
pub use properties::{read, write};
pub use specs::schema;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamSource {
    Properties,
    Attachment,
}

#[derive(Debug)]
pub struct ParamSpec {
    pub key: &'static str,
    pub source: ParamSource,
    pub path: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub group: &'static str,
    pub kind: ParamKind,
    pub default: Option<&'static str>,
    pub visible_if: Option<(&'static str, &'static [&'static str])>,
}

#[derive(Debug)]
pub enum ParamKind {
    Bool,
    Int {
        min: i64,
        max: i64,
    },
    Real {
        min: f64,
        max: f64,
    },
    Text,
    Expr,
    Choice(&'static [(&'static str, &'static str)]),
    Field {
        measures: &'static [&'static str],
    },
    Fields {
        measures: &'static [&'static str],
    },
    FilePath {
        extensions: &'static [&'static str],
    },
    Table {
        row_path: &'static str,
        key_child: &'static str,
        columns: &'static [ColumnSpec],
    },
}

#[derive(Debug)]
pub struct ColumnSpec {
    pub child: &'static str,
    pub label: &'static str,
    pub kind: ParamKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    Unset,
    Bool(bool),
    Int(i64),
    Real(f64),
    Text(String),
    Fields(Vec<String>),
    Rows(Vec<(String, Vec<Option<String>>)>),
}

pub(super) const MAX_TEXT: usize = 1 << 20;
pub(super) const MAX_ITEMS: usize = 100_000;
pub(super) const MAX_ROWS: usize = 10_000;
const MAX_DEPTH: usize = 512;

fn check_text(text: &str) -> Result<(), String> {
    if text.len() > MAX_TEXT {
        return Err("Parameter text exceeds 1 MiB".into());
    }
    if text.chars().any(|c| !matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')) {
        return Err("Parameter text contains a character unavailable in XML".into());
    }
    Ok(())
}

fn scalar(kind: &ParamKind, text: &str) -> Result<ParamValue, String> {
    check_text(text)?;
    let trimmed = text.trim();
    match kind {
        ParamKind::Bool => match trimmed {
            "" => Ok(ParamValue::Unset),
            "true" => Ok(ParamValue::Bool(true)),
            "false" => Ok(ParamValue::Bool(false)),
            _ => Err(format!("Expected true or false, found {text:?}")),
        },
        ParamKind::Int { .. } => {
            if trimmed.is_empty() {
                Ok(ParamValue::Unset)
            } else {
                trimmed
                    .parse()
                    .map(ParamValue::Int)
                    .map_err(|_| format!("Expected an integer, found {text:?}"))
            }
        }
        ParamKind::Real { .. } => {
            if trimmed.is_empty() {
                Ok(ParamValue::Unset)
            } else {
                trimmed
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .map(ParamValue::Real)
                    .ok_or_else(|| format!("Expected a finite number, found {text:?}"))
            }
        }
        ParamKind::Choice(choices) => {
            if trimmed.is_empty() {
                return Ok(ParamValue::Unset);
            }
            if !choices.iter().any(|(v, _)| *v == trimmed) {
                return Err(format!("Unknown choice {text:?}"));
            }
            Ok(ParamValue::Text(text.to_string()))
        }
        ParamKind::Text
        | ParamKind::Expr
        | ParamKind::Field { .. }
        | ParamKind::FilePath { .. } => Ok(ParamValue::Text(text.into())),
        _ => Err("Expected a scalar parameter".into()),
    }
}

pub fn validate(spec: &ParamSpec, value: &ParamValue) -> Result<(), String> {
    validate_kind(&spec.kind, value).map_err(|e| format!("{}: {e}", spec.label))
}

fn validate_kind(kind: &ParamKind, value: &ParamValue) -> Result<(), String> {
    if matches!(value, ParamValue::Unset) {
        return Ok(());
    }
    match (kind, value) {
        (ParamKind::Bool, ParamValue::Bool(_)) => Ok(()),
        (ParamKind::Int { min, max }, ParamValue::Int(n)) if (min..=max).contains(&n) => Ok(()),
        (ParamKind::Real { min, max }, ParamValue::Real(n))
            if n.is_finite() && (min..=max).contains(&n) =>
        {
            Ok(())
        }
        (ParamKind::Choice(choices), ParamValue::Text(s))
            if choices.iter().any(|(v, _)| *v == s) =>
        {
            check_text(s)
        }
        (
            ParamKind::Text
            | ParamKind::Expr
            | ParamKind::Field { .. }
            | ParamKind::FilePath { .. },
            ParamValue::Text(s),
        ) => check_text(s),
        (ParamKind::Fields { .. }, ParamValue::Fields(fields)) => {
            if fields.len() > MAX_ROWS {
                return Err("List exceeds 10000 entries".into());
            }
            let mut size = 0usize;
            for field in fields {
                check_text(field)?;
                size = size.saturating_add(field.len());
            }
            if size > MAX_TEXT {
                return Err("List text exceeds 1 MiB".into());
            }
            Ok(())
        }
        (ParamKind::Table { columns, .. }, ParamValue::Rows(rows)) => {
            if rows.len() > MAX_ROWS {
                return Err("Table exceeds 10000 rows".into());
            }
            let mut seen = std::collections::HashSet::new();
            let mut size = 0usize;
            for (key, values) in rows {
                check_text(key)?;
                size = size.saturating_add(key.len());
                if !seen.insert(key) {
                    return Err(format!("Duplicate table key {key:?}"));
                }
                if values.len() != columns.len() {
                    return Err("Wrong table column count".into());
                }
                for (column, value) in columns.iter().zip(values) {
                    if let Some(text) = value {
                        size = size.saturating_add(text.len());
                        let parsed = scalar(&column.kind, text)?;
                        validate_kind(&column.kind, &parsed)?;
                    }
                }
            }
            if size > MAX_TEXT {
                return Err("Table text exceeds 1 MiB".into());
            }
            Ok(())
        }
        _ => Err(
            "Value has the wrong type, an unknown choice, or is outside the permitted range".into(),
        ),
    }
}

// IEEE signed zero must survive an explicit change even though f64 PartialEq equates it.
fn same_value(a: &ParamValue, b: &ParamValue) -> bool {
    match (a, b) {
        (ParamValue::Real(a), ParamValue::Real(b)) => a.to_bits() == b.to_bits(),
        _ => a == b,
    }
}

fn file_text(value: &ParamValue) -> Result<String, String> {
    match value {
        ParamValue::Bool(v) => Ok(v.to_string()),
        ParamValue::Int(v) => Ok(v.to_string()),
        ParamValue::Real(v) if v.is_finite() => {
            let mut text = v.to_string();
            if !text.contains(['.', 'e', 'E']) {
                text.push_str(".0");
            }
            Ok(text)
        }
        ParamValue::Text(v) => Ok(v.clone()),
        _ => Err("Value is not scalar".into()),
    }
}

/// Measurement names shared by field constraints and engine field queries.
pub fn measurement_name(value: dms_types::Measurement) -> &'static str {
    use dms_types::Measurement;
    match value {
        Measurement::Default => "auto",
        Measurement::Continuous => "range",
        Measurement::Categorical => "discrete",
        Measurement::Flag => "flag",
        Measurement::Nominal => "set",
        Measurement::Ordinal => "orderedSet",
        Measurement::Typeless => "typeless",
        Measurement::Collection => "collection",
        Measurement::Geospatial => "geospatial",
    }
}
