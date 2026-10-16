//! Reclassify (Field Operations): maps the values of a field to new values, into a new field or
//! in place.
//!
//! Implemented: single mode (`singleFieldSelection` into `singleFieldName`, or in place with
//! `reclassifyReplaceField`) and multiple mode (`multipleFieldList` with
//! `multipleExtensionName` as prefix or suffix; always new fields), the mapping table
//! `reclassifyTable/rows/row/{oldValue,newValue}`, and `useDefault`/`defaultValue` for values not in
//! the table (`undef` gives null). Without a default, unmapped values keep their value. Nulls stay
//! null.
//!
//! Old values are read according to the field's storage (numbers compare by value, so `5` matches
//! `5.0`; strings exactly; other storages by their plain text form). The output storage follows
//! the new values: when all of them (and the default) are numbers the output is numeric -- real
//! when the source is real, or any new value has a decimal point -- otherwise it is a string.
//! This matches the corpus (`ed` real -> `edu_4cat` real; integer -> string groups).
//!
//! GAP: `reclassifyReplaceField` is applied in single mode only; old values of date and time
//! fields are compared by their ISO text, not by Modeler's format; the exact output storage rule
//! for a numeric source with string new values and a pass-through of unmapped numbers (they are
//! converted to their text).

use super::field_util::{
    Column, extended_name, field_list, literal_values, position, with_columns,
};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Measurement, Storage, Table, Value};
use std::sync::Arc;

enum Targets {
    Single {
        field: String,
        new_name: String,
        replace: bool,
    },
    Multiple {
        fields: Vec<String>,
        extension: String,
        prefix: bool,
    },
}

struct Reclassify {
    targets: Targets,
    /// (old value text, new value text)
    table: Vec<(String, String)>,
    /// `Some(None)`: use the default `undef`; `Some(Some(text))`: replace with the text.
    default: Option<Option<String>>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let text = |path: &str| settings::text(s, path);
    let targets = match text("reclassifyMode").as_deref().map(str::trim) {
        None | Some("") | Some("single") => {
            let field = text("singleFieldSelection").unwrap_or_default();
            if field.trim().is_empty() {
                return Err(OperatorError::Settings(
                    "singleFieldSelection: no field selected".into(),
                ));
            }
            let replace = settings::flag(s, "reclassifyReplaceField")?.unwrap_or(false);
            let new_name = text("singleFieldName").unwrap_or_default();
            if !replace && new_name.trim().is_empty() {
                return Err(OperatorError::Settings(
                    "singleFieldName: the new field needs a name".into(),
                ));
            }
            Targets::Single {
                field,
                new_name,
                replace,
            }
        }
        Some("multiple") => {
            let fields = field_list(s, "multipleFieldList");
            if fields.is_empty() {
                return Err(OperatorError::Settings(
                    "multipleFieldList: no fields selected".into(),
                ));
            }
            Targets::Multiple {
                fields,
                extension: text("multipleExtensionName").unwrap_or_default(),
                prefix: text("multipleExtensionPosition").as_deref().map(str::trim)
                    == Some("prefix"),
            }
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "reclassifyMode: unknown mode {other:?}"
            )));
        }
    };
    let table = settings::children(s, "reclassifyTable/rows", "row")
        .into_iter()
        .map(|row| {
            (
                row.child("oldValue").map(|e| e.text()).unwrap_or_default(),
                row.child("newValue").map(|e| e.text()).unwrap_or_default(),
            )
        })
        .collect();
    let default = if settings::flag(s, "useDefault")?.unwrap_or(false) {
        let value = text("defaultValue").unwrap_or_else(|| "undef".into());
        Some((value.trim() != "undef").then_some(value))
    } else {
        None
    };
    Ok(Box::new(Reclassify {
        targets,
        table,
        default,
    }))
}

impl Operator for Reclassify {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Reclassify needs one input".into()))?;
        let columns = match &self.targets {
            Targets::Single {
                field,
                new_name,
                replace,
            } => {
                let name = if *replace { field } else { new_name };
                vec![self.column(input, field, name)?]
            }
            Targets::Multiple {
                fields,
                extension,
                prefix,
            } => fields
                .iter()
                .map(|field| self.column(input, field, &extended_name(field, extension, *prefix)))
                .collect::<Result<Vec<_>, _>>()?,
        };
        with_columns(input, columns)
    }
}

impl Reclassify {
    fn column(&self, input: &Table, source: &str, name: &str) -> Result<Column, OperatorError> {
        let current = position(input.model(), source)?;
        let source_field = input
            .model()
            .fields()
            .get(current)
            .ok_or_else(|| OperatorError::Data(format!("input has no field {source:?}")))?;
        let source_storage = source_field.storage;

        // New values (and the default) as typed values, with one storage for all of them.
        let mut texts: Vec<&str> = self.table.iter().map(|(_, new)| new.as_str()).collect();
        if let Some(Some(default)) = &self.default {
            texts.push(default);
        }
        let (literal_storage, mut typed) = literal_values(&texts);
        let all_numeric = !texts.is_empty() && literal_storage != Storage::String;
        let default_value = match &self.default {
            Some(Some(_)) => typed.pop(),
            Some(None) => Some(Value::Null),
            None => None,
        };
        let numeric_source = matches!(source_storage, Storage::Integer | Storage::Real);
        let output_storage = if all_numeric && (self.default.is_some() || numeric_source) {
            if literal_storage == Storage::Real || source_storage == Storage::Real {
                Storage::Real
            } else {
                Storage::Integer
            }
        } else if texts.is_empty() && self.default.is_none() {
            source_storage
        } else {
            Storage::String
        };
        let new_values: Vec<Value> = if output_storage == Storage::String {
            self.table
                .iter()
                .map(|(_, new)| Value::string(new.as_str()))
                .collect()
        } else {
            typed
        };
        let default_value = default_value.map(|value| match (output_storage, value) {
            (Storage::String, Value::Integer(_) | Value::Real(_)) => self
                .default
                .clone()
                .flatten()
                .map_or(Value::Null, Value::string),
            (_, value) => value,
        });

        let old_keys: Vec<(OldKey, &Value)> = self
            .table
            .iter()
            .zip(&new_values)
            .map(|((old, _), new)| (OldKey::parse(old, source_storage), new))
            .collect();

        let mut values = Vec::with_capacity(input.row_count());
        for row in input.rows() {
            let old = row.get(current).unwrap_or(&Value::Null);
            let mapped = if old.is_null() {
                Value::Null
            } else if let Some((_, new)) = old_keys.iter().find(|(key, _)| key.matches(old)) {
                (*new).clone()
            } else if let Some(default) = &default_value {
                default.clone()
            } else {
                old.clone()
            };
            values.push(finish(mapped, output_storage));
        }
        // The new field keeps the source's measurement level, not its values (team/stream_1_M:
        // `edu_4cat` from the ordinal `ed` is audited as Ordinal).
        let measurement = input
            .model()
            .fields()
            .get(current)
            .map_or(Measurement::Default, |field| field.measurement);
        let mut column = Column::inferred(name, measurement, values)?;
        if column.field.storage == Storage::Unknown {
            column.field.storage = output_storage;
        }
        Ok(column)
    }
}

/// Pass-through values take the output storage.
fn finish(value: Value, storage: Storage) -> Value {
    match (storage, value) {
        (Storage::String, value @ (Value::Integer(_) | Value::Real(_))) => {
            Value::string(value.to_string())
        }
        (Storage::Real, Value::Integer(i)) => Value::Real(i as f64),
        (_, value) => value,
    }
}

/// An old value of the mapping table, read for the source field's storage.
enum OldKey {
    Number(f64),
    Text(String),
}

impl OldKey {
    fn parse(text: &str, storage: Storage) -> OldKey {
        match storage {
            Storage::Integer | Storage::Real => text
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .map_or_else(|| OldKey::Text(text.to_string()), OldKey::Number),
            _ => OldKey::Text(text.to_string()),
        }
    }

    fn matches(&self, value: &Value) -> bool {
        match (self, value) {
            (OldKey::Number(n), Value::Integer(_) | Value::Real(_)) => {
                value.as_f64().is_some_and(|v| v == *n)
            }
            (OldKey::Text(t), Value::String(s)) => t.as_str() == &**s,
            (OldKey::Text(t), other @ (Value::Date(_) | Value::Time(_) | Value::Timestamp(_))) => {
                *t == other.to_string()
            }
            _ => false,
        }
    }
}
