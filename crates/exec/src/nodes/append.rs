//! Append (Record Ops): stacks the records of several inputs.
//!
//! Settings: `appendMode` (`name` default | `position`), `appendMatchCase` (field names compare
//! case-sensitively; default false), `appendIncludeAllFields` (true = fields of all inputs, false
//! = only the main (first) input's fields), `appendIncludeTag` and `appendTagField` (default
//! `Input`). Encodings from `docs/formats/nodes/append.md`.
//!
//! Rules (settled by `oracle/records/append_name_main`, `append_name_all_case`,
//! `append_position_all`, `append_position_main`):
//! - Fields missing from an input are null (`append_name_main`, `append_name_all_case`,
//!   `append_position_all`).
//! - In position mode, fields are named by the first input that has that position
//!   (`append_position_all`, `append_position_main`).
//! - In name mode, `appendMatchCase=false` (default) matches field names case-insensitively
//!   (`Name` and `name` collapse into `Name` in `append_name_main`), whereas `appendMatchCase=true`
//!   keeps distinct case variants as separate columns (`append_name_all_case`).
//! - A field whose storage differs between inputs is widened (`Integer` + `Real` = `Real`, as on
//!   `num` across `append_1.csv` and `append_2.csv`); other conflicts are a data error.
//! - When `appendIncludeTag` is enabled, the tag field has `Storage::String` and holds the input's
//!   1-based index as a string (`"1"`, `"2"`, ...; `append_name_main`, `append_name_all_case`,
//!   `append_position_main`).
//!
//! GAP: metadata (measurement, roles, value labels) comes from the first input with the field.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{DataModel, Field, Storage, Table, Value};
use std::collections::HashMap;
use std::sync::Arc;

struct Append {
    by_position: bool,
    match_case: bool,
    all_fields: bool,
    tag: Option<String>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let by_position = match settings::text(s, "appendMode").as_deref().map(str::trim) {
        None | Some("") | Some("name") => false,
        Some("position") => true,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "appendMode: unknown value {other:?}"
            )));
        }
    };
    let tag = if settings::flag(s, "appendIncludeTag")?.unwrap_or(false) {
        Some(
            settings::text(s, "appendTagField")
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "Input".to_string()),
        )
    } else {
        None
    };
    Ok(Box::new(Append {
        by_position,
        match_case: settings::flag(s, "appendMatchCase")?.unwrap_or(false),
        all_fields: settings::flag(s, "appendIncludeAllFields")?.unwrap_or(false),
        tag,
    }))
}

fn unify(a: Storage, b: Storage) -> Option<Storage> {
    match (a, b) {
        (x, y) if x == y => Some(x),
        (Storage::Unknown, y) => Some(y),
        (x, Storage::Unknown) => Some(x),
        (Storage::Integer, Storage::Real) | (Storage::Real, Storage::Integer) => {
            Some(Storage::Real)
        }
        _ => None,
    }
}

impl Operator for Append {
    fn arity(&self) -> Arity {
        Arity::AtLeast(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let main = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Append needs at least one input".into()))?;
        let normalize = |name: &str| -> String {
            if self.match_case {
                name.to_string()
            } else {
                name.to_lowercase()
            }
        };
        let mut fields: Vec<Field> = main.model().fields().to_vec();
        // For every input, the output position of each of its fields (None = not included).
        let mut mapping: Vec<Vec<Option<usize>>> = Vec::new();
        let mut by_name: HashMap<String, usize> = HashMap::new();
        for (position, field) in fields.iter().enumerate() {
            by_name.entry(normalize(&field.name)).or_insert(position);
        }
        mapping.push((0..fields.len()).map(Some).collect());
        for input in inputs.iter().skip(1) {
            let mut map = Vec::new();
            for (position, field) in input.model().fields().iter().enumerate() {
                let target = if self.by_position {
                    if position < fields.len() {
                        Some(position)
                    } else if self.all_fields {
                        fields.push(field.clone());
                        Some(fields.len() - 1)
                    } else {
                        None
                    }
                } else {
                    match by_name.get(&normalize(&field.name)) {
                        Some(&target) => Some(target),
                        None if self.all_fields => {
                            by_name.insert(normalize(&field.name), fields.len());
                            fields.push(field.clone());
                            Some(fields.len() - 1)
                        }
                        None => None,
                    }
                };
                if let Some(target) = target
                    && let Some(existing) = fields.get_mut(target)
                {
                    existing.storage = unify(existing.storage, field.storage).ok_or_else(|| {
                        OperatorError::Data(format!(
                            "field {:?} has incompatible storage in the inputs",
                            existing.name
                        ))
                    })?;
                }
                map.push(target);
            }
            mapping.push(map);
        }
        if let Some(name) = &self.tag {
            fields.push(Field::new(name.clone(), Storage::String));
        }
        let model =
            DataModel::new(fields).map_err(|error| OperatorError::Data(error.to_string()))?;
        let width = model.len();
        let tag_position = self.tag.as_ref().map(|_| width.saturating_sub(1));
        let mut rows = Vec::new();
        for (n, input) in inputs.iter().enumerate() {
            let map = mapping.get(n).map(Vec::as_slice).unwrap_or_default();
            let tag_value = Value::string((n + 1).to_string());
            for row in input.rows() {
                let mut out = vec![Value::Null; width];
                for (position, value) in row.iter().enumerate() {
                    if let Some(Some(target)) = map.get(position)
                        && let Some(slot) = out.get_mut(*target)
                    {
                        let widen = model
                            .fields()
                            .get(*target)
                            .is_some_and(|f| f.storage == Storage::Real);
                        *slot = match value {
                            Value::Integer(i) if widen => Value::Real(*i as f64),
                            other => other.clone(),
                        };
                    }
                }
                if let Some(slot) = tag_position.and_then(|p| out.get_mut(p)) {
                    *slot = tag_value.clone();
                }
                rows.push(out);
            }
        }
        Table::new(model, rows).map_err(|error| OperatorError::Data(error.to_string()))
    }
}
