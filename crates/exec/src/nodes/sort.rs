//! Sort (Record Ops): orders records by one or more key fields.
//!
//! Settings: `sortKeys/rows/row[i]/{field,direction}` (direction `0`/`Ascending` or
//! `1`/`Descending`, per docs/formats/nodes/sort.md), `sortDefaultOrder` (direction for keys
//! without one; default ascending). The sort is stable. Not implemented (display or performance
//! hints only, ignored): `useExistingSortOrder`/`existingSortKeys` (claim an existing order).
//!
//! GAP: null placement and string collation are not verified against Modeler. Nulls sort first in
//! ascending order and last in descending order; strings compare by code point (case-sensitive).

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{DataModel, Table};
use std::cmp::Ordering;
use std::sync::Arc;

/// A key field and direction by name, resolved against a model when the node runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeySpec {
    pub field: String,
    pub descending: bool,
}

impl KeySpec {
    /// Resolves to (position, descending).
    pub(crate) fn resolve(
        keys: &[KeySpec],
        model: &DataModel,
    ) -> Result<Vec<(usize, bool)>, OperatorError> {
        keys.iter()
            .map(|key| {
                model
                    .position(&key.field)
                    .map(|p| (p, key.descending))
                    .ok_or_else(|| {
                        OperatorError::Data(format!("unknown key field {:?}", key.field))
                    })
            })
            .collect()
    }
}

/// Sort rows with resolved keys.
pub(crate) fn compare_rows(
    keys: &[(usize, bool)],
    a: &[dms_types::Value],
    b: &[dms_types::Value],
) -> Ordering {
    for &(position, descending) in keys {
        let ordering = match (a.get(position), b.get(position)) {
            (Some(x), Some(y)) => x.total_cmp(y),
            _ => Ordering::Equal,
        };
        let ordering = if descending {
            ordering.reverse()
        } else {
            ordering
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

pub(crate) fn parse_keys(
    s: Option<&dms_strfmt::fastinfoset::Element>,
    path: &str,
    default_descending: bool,
) -> Result<Vec<KeySpec>, OperatorError> {
    let mut keys = Vec::new();
    for row in settings::children(s, path, "row") {
        let field = row.child("field").map(|e| e.text()).unwrap_or_default();
        if field.is_empty() {
            return Err(OperatorError::Settings(format!(
                "{path}: key without field"
            )));
        }
        let descending = match row
            .child("direction")
            .map(|e| e.text())
            .as_deref()
            .map(str::trim)
        {
            None | Some("") => default_descending,
            Some("0") | Some("Ascending") | Some("ascending") => false,
            Some("1") | Some("Descending") | Some("descending") => true,
            Some(other) => {
                return Err(OperatorError::Settings(format!(
                    "{path}: unknown direction {other:?}"
                )));
            }
        };
        keys.push(KeySpec { field, descending });
    }
    Ok(keys)
}

struct Sort {
    keys: Vec<KeySpec>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let default_descending = !settings::flag(s, "sortDefaultOrder")?.unwrap_or(true);
    let keys = parse_keys(s, "sortKeys/rows", default_descending)?;
    Ok(Box::new(Sort { keys }))
}

impl Operator for Sort {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Sort needs one input".into()))?;
        let keys = KeySpec::resolve(&self.keys, input.model())?;
        let mut rows = input.rows().to_vec();
        rows.sort_by(|a, b| compare_rows(&keys, a, b));
        Table::new(input.model().clone(), rows)
            .map_err(|error| OperatorError::Data(error.to_string()))
    }
}
