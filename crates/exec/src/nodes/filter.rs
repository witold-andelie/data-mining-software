//! Filter (Field Operations): drops and renames fields; record data is untouched.
//!
//! Implemented: `filterTable/defaultDrop` (what happens to fields without a row: kept unless
//! `true`), and per-field rows `filterTable/rows/row/{name,drop,rename}`. Rows for fields that
//! are not in the input are ignored (Modeler does not validate them either); a row without `drop` keeps its field; an empty `rename`
//! keeps the name. Field order is the input order. Two fields ending up with the same name are a
//! data error.
//!
//! Not implemented: nothing else is saved for this node; the Filter's field groups of the user
//! interface are display-only.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{DataModel, Table, Value};
use std::collections::HashMap;
use std::sync::Arc;

struct Row {
    drop: bool,
    rename: Option<String>,
}

struct Filter {
    default_drop: bool,
    rows: HashMap<String, Row>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let default_drop = settings::flag(s, "filterTable/defaultDrop")?.unwrap_or(false);
    let mut rows = HashMap::new();
    for row in settings::children(s, "filterTable/rows", "row") {
        let name = row.child("name").map(|e| e.text()).unwrap_or_default();
        let drop = settings::flag(Some(row), "drop")?.unwrap_or(false);
        let rename = settings::text(Some(row), "rename").filter(|r| !r.trim().is_empty());
        rows.insert(name, Row { drop, rename });
    }
    Ok(Box::new(Filter { default_drop, rows }))
}

impl Operator for Filter {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Filter needs one input".into()))?;
        let mut kept = Vec::new();
        let mut fields = Vec::new();
        for (position, field) in input.model().fields().iter().enumerate() {
            let row = self.rows.get(&field.name);
            let drop = row.map_or(self.default_drop, |r| r.drop);
            if drop {
                continue;
            }
            let mut field = field.clone();
            if let Some(rename) = row.and_then(|r| r.rename.as_ref()) {
                field.name = rename.clone();
            }
            kept.push(position);
            fields.push(field);
        }
        let model = DataModel::new(fields).map_err(|e| OperatorError::Data(e.to_string()))?;
        let rows = input
            .rows()
            .iter()
            .map(|row| {
                kept.iter()
                    .map(|p| row.get(*p).cloned().unwrap_or(Value::Null))
                    .collect()
            })
            .collect();
        Table::new(model, rows).map_err(|e| OperatorError::Data(e.to_string()))
    }
}
