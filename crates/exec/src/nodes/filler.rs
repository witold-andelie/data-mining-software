//! Filler (Field Operations): replaces values of the selected fields with the result of a CLEM
//! expression, in place.
//!
//! Implemented: `fillerSelectedFields`, `fillerReplaceMode` (`always`, `conditional` with
//! `fillerCondition`, `blanks`, `nulls`, `blanksAndNulls`) and `fillerReplaceExpression` with
//! `@FIELD` bound to each selected field in turn. Conditions and replacements see the record as
//! it arrived (not earlier fills of the same record). Blanks are the field's missing-value
//! definition (`@BLANK`); nulls the system-missing value.
//!
//! A field's storage follows its new values: replacing with another kind of value (for example
//! `to_string(@FIELD)`) changes the storage; integers in a real column become reals; a
//! replacement of a kind that cannot be combined with the remaining values is a data error.
//!
//! GAP: when the replacement changes a field's kind, Modeler's exact rules (it may instead null
//! the value) are not documented; the field's role and value labels are kept, but its known
//! values (`ValueSpec`) are reset since they may no longer hold.

use super::field_util::{
    bind, bind_condition, coerce_column, conditions, field_list, field_values, infer_storage,
    parse_required, position,
};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_clem::Expr;
use dms_stream::StreamNode;
use dms_types::{DataModel, Field, Table, Value, ValueSpec};
use std::sync::Arc;

#[derive(Clone)]
enum Mode {
    Always,
    Conditional(Expr),
    Blanks,
    Nulls,
    BlanksAndNulls,
}

struct Filler {
    fields: Vec<String>,
    mode: Mode,
    replacement: Expr,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let text = |path: &str| settings::text(s, path);
    let mode = match text("fillerReplaceMode").as_deref().map(str::trim) {
        None | Some("") | Some("conditional") => Mode::Conditional(parse_required(
            "fillerCondition",
            Some(text("fillerCondition").unwrap_or_else(|| "@BLANK(@FIELD)".into())),
        )?),
        Some("always") => Mode::Always,
        Some("blanks") => Mode::Blanks,
        Some("nulls") => Mode::Nulls,
        Some("blanksAndNulls") => Mode::BlanksAndNulls,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "fillerReplaceMode: unknown mode {other:?}"
            )));
        }
    };
    let fields = field_list(s, "fillerSelectedFields");
    if fields.is_empty() {
        return Err(OperatorError::Settings(
            "fillerSelectedFields: no fields selected".into(),
        ));
    }
    let replacement = parse_required(
        "fillerReplaceExpression",
        Some(text("fillerReplaceExpression").unwrap_or_else(|| "0".into())),
    )?;
    Ok(Box::new(Filler {
        fields,
        mode,
        replacement,
    }))
}

impl Operator for Filler {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Filler needs one input".into()))?;
        let model = input.model();
        let rows = input.rows();
        let mut replaced: Vec<(usize, Vec<Value>)> = Vec::new();
        for name in &self.fields {
            let current = position(model, name)?;
            let replacement = bind(
                "fillerReplaceExpression",
                &self.replacement,
                model,
                Some(current),
            )?;
            let condition = match &self.mode {
                Mode::Conditional(expr) => Some(bind_condition(
                    "fillerCondition",
                    expr,
                    model,
                    Some(current),
                )?),
                _ => None,
            };
            let missing = model
                .field(name)
                .map(|f| f.missing.clone())
                .unwrap_or_default();
            // Expressions run over all records first so that sequence functions see the others.
            // GAP: they see the input values, not the values this node has already replaced.
            let hits = condition
                .as_ref()
                .map(|c| conditions("fillerCondition", c, rows))
                .transpose()?;
            let replacements = field_values("fillerReplaceExpression", &replacement, rows)?;
            let mut column = Vec::with_capacity(rows.len());
            for (i, (row, replacement)) in rows.iter().zip(replacements).enumerate() {
                let old = row.get(current).cloned().unwrap_or(Value::Null);
                let hit = match &self.mode {
                    Mode::Always => true,
                    Mode::Conditional(_) => {
                        hits.as_ref().is_some_and(|hits| hits.get(i) == Some(&true))
                    }
                    Mode::Blanks => missing.is_blank(&old),
                    Mode::Nulls => old.is_null(),
                    Mode::BlanksAndNulls => old.is_null() || missing.is_blank(&old),
                };
                column.push(if hit { replacement } else { old });
            }
            replaced.push((current, column));
        }

        let mut fields: Vec<Field> = model.fields().to_vec();
        for (current, column) in &mut replaced {
            let name = fields
                .get(*current)
                .map(|f| f.name.clone())
                .unwrap_or_default();
            let storage = infer_storage(column).ok_or_else(|| {
                OperatorError::Data(format!("{name}: the filled values are of different kinds"))
            })?;
            coerce_column(column, storage);
            if let Some(field) = fields.get_mut(*current)
                && storage != dms_types::Storage::Unknown
                && storage != field.storage
            {
                field.storage = storage;
                field.values = ValueSpec::Unknown;
            }
        }
        let out_rows = rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let mut out = row.clone();
                for (current, column) in &replaced {
                    if let (Some(slot), Some(value)) = (out.get_mut(*current), column.get(i)) {
                        *slot = value.clone();
                    }
                }
                out
            })
            .collect();
        let model = DataModel::new(fields).map_err(|e| OperatorError::Data(e.to_string()))?;
        Table::new(model, out_rows).map_err(|e| OperatorError::Data(e.to_string()))
    }
}
