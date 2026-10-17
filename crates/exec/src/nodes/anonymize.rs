//! Anonymize (Field Ops, `AnonymizeNode`): replaces the values of chosen fields
//! (`oracle/misc/misc_an_*`).
//!
//! Settings: `anonymizeValuesTable/fields/field` with `name`, `enabled` (default true) and
//! either `replace_operation` (`useCustomPrefix`, `prefix`) or `transform_operation`
//! (`scale_value`, `translate_value`, `transform_randomly`, `seed_value`,
//! `seed_value_specified`). An embedded `filterTable` with rows is rejected.
//!
//! Rules:
//! - Without fields every value passes unchanged (`misc_an_default`).
//! - A replaced field becomes a string field of `<prefix>S<n>`; the prefix is `anon_` unless
//!   `useCustomPrefix` (`misc_an_prefix`: `cust_S0`; `misc_an_prefix_off`: `anon_S0`).
//! - Numbers `n` count the distinct values in the order they are met, record by record and,
//!   within a record, field by field in the table's order; all replaced fields share one
//!   dictionary, so a value has the same number in every field (`misc_an_multi_symbols`: flags
//!   `T` `anon_S0`, `grp` `B` `anon_S1`, `text_edge` `banana` `anon_S2`, and the empty string is
//!   `anon_S8` in all three; `misc_an_reverse`: numbering follows the input order). Null stays
//!   null.
//! - A transformed field is `scale * (value + translate)` (`misc_an_scale`: `rv` doubled;
//!   `misc_an_integer_set`: `iv` -4 becomes 6 * (-4 + 39) = 210). With `transform_randomly`
//!   Modeler draws the two numbers when the node is set up and saves them, so the saved ones
//!   apply (`misc_an_integer_set`, seed 1: scale 6, translate 39). Null stays null.
//!
//! Rejected: transforming a field that is not numeric, and an integer field with a scale or
//! translation that is not whole (no probe shows the storage then).
//!
//! GAPs: dictionaries of fields with different prefixes are assumed shared as well; user-missing
//! values are replaced like any other (no probe).

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{DataModel, Field, Measurement, Storage, Table, Value, ValueSpec};
use std::collections::HashMap;
use std::sync::Arc;

enum Operation {
    Replace { prefix: String },
    Transform { scale: f64, translate: f64 },
}

struct AnonymizeField {
    name: String,
    operation: Operation,
}

struct Anonymize {
    fields: Vec<AnonymizeField>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let bad = |what: String| OperatorError::Settings(format!("Anonymize: {what}"));
    let mut fields = Vec::new();
    for field in settings::children(s, "anonymizeValuesTable/fields", "field") {
        let field = Some(field);
        let name = settings::text(field, "name").unwrap_or_default();
        if name.is_empty() {
            return Err(bad("a field without a name".into()));
        }
        if !settings::flag(field, "enabled")?.unwrap_or(true) {
            continue;
        }
        let replace = settings::element(field, "replace_operation");
        let transform = settings::element(field, "transform_operation");
        let operation = match (replace, transform) {
            (Some(op), None) => {
                let op = Some(op);
                let prefix = if settings::flag(op, "useCustomPrefix")?.unwrap_or(false) {
                    settings::text(op, "prefix").unwrap_or_default()
                } else {
                    "anon_".to_string()
                };
                Operation::Replace { prefix }
            }
            (None, Some(op)) => {
                let op = Some(op);
                let number = |key: &str| -> Result<f64, OperatorError> {
                    settings::text(op, key)
                        .and_then(|t| t.trim().parse::<f64>().ok())
                        .filter(|v| v.is_finite())
                        .ok_or_else(|| bad(format!("field {name:?}: {key} is not a number")))
                };
                Operation::Transform {
                    scale: number("scale_value")?,
                    translate: number("translate_value")?,
                }
            }
            _ => {
                return Err(bad(format!(
                    "field {name:?} needs one replace or transform operation"
                )));
            }
        };
        fields.push(AnonymizeField { name, operation });
    }
    if !settings::children(s, "filterTable/rows", "row").is_empty() {
        return Err(bad(
            "renaming or dropping fields in the node is not supported yet".into(),
        ));
    }
    Ok(Box::new(Anonymize { fields }))
}

impl Operator for Anonymize {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Anonymize needs one input".into()))?;
        let model = input.model();
        let mut fields: Vec<Field> = model.fields().to_vec();
        let mut targets = Vec::with_capacity(self.fields.len());
        for field in &self.fields {
            let at = model.position(&field.name).ok_or_else(|| {
                OperatorError::Data(format!("Anonymize: no field {}", field.name))
            })?;
            let source = &model.fields()[at];
            match &field.operation {
                Operation::Replace { .. } => {
                    let mut replaced = Field::new(source.name.clone(), Storage::String);
                    replaced.measurement = Measurement::Categorical;
                    replaced.role = source.role;
                    fields[at] = replaced;
                }
                Operation::Transform { scale, translate } => match source.storage {
                    Storage::Real => {}
                    Storage::Integer if scale.fract() == 0.0 && translate.fract() == 0.0 => {}
                    Storage::Integer => {
                        return Err(OperatorError::Settings(format!(
                            "Anonymize: field {}: a scale or translation that is not whole on \
                             an integer field is not supported yet",
                            field.name
                        )));
                    }
                    other => {
                        return Err(OperatorError::Settings(format!(
                            "Anonymize: field {} has {other:?} storage; only numbers transform",
                            field.name
                        )));
                    }
                },
            }
            if matches!(field.operation, Operation::Transform { .. }) {
                fields[at].values = ValueSpec::default();
            }
            targets.push((at, &field.operation));
        }

        let mut dictionary: HashMap<String, usize> = HashMap::new();
        let mut rows = Vec::with_capacity(input.row_count());
        for row in input.rows() {
            let mut row = row.clone();
            for &(at, operation) in &targets {
                let Some(cell) = row.get_mut(at) else {
                    continue;
                };
                if cell.is_null() {
                    continue;
                }
                let replacement = match operation {
                    Operation::Replace { prefix } => {
                        let next = dictionary.len();
                        let number = *dictionary.entry(cell.to_string()).or_insert(next);
                        Value::string(format!("{prefix}S{number}"))
                    }
                    Operation::Transform { scale, translate } => match &*cell {
                        Value::Integer(i) => {
                            Value::Integer((scale * (*i as f64 + translate)) as i64)
                        }
                        other => match other.as_f64() {
                            Some(x) => Value::real(scale * (x + translate)),
                            None => Value::Null,
                        },
                    },
                };
                *cell = replacement;
            }
            rows.push(row);
        }
        let model = DataModel::new(fields).map_err(|e| OperatorError::Data(e.to_string()))?;
        Table::new(model, rows).map_err(|e| OperatorError::Data(e.to_string()))
    }
}
