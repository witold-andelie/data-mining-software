//! Sort (Record Ops): orders records by one or more key fields.
//!
//! Settings: `sortKeys/rows/row[i]/{field,direction}` (direction `0`/`Ascending` or
//! `1`/`Descending`, per docs/formats/nodes/sort.md), `sortDefaultOrder` (direction for keys
//! without one; default ascending). Not implemented (display or performance hints only,
//! ignored): `useExistingSortOrder`/`existingSortKeys` (claim an existing order).
//!
//! Rules (settled by `oracle/records/sort_multi_keys`, `sort_strings_nulls`, `sort_desc_ties`):
//! - Nulls (`$null$`) always sort last, in both ascending and descending directions
//!   (`sort_multi_keys`: `iv` descending places null after `-10`; `sort_strings_nulls`: `rv`
//!   descending places null after `-3.5`; `sort_desc_ties`: both `iv` and `grp` descending place
//!   nulls at the bottom).
//! - Empty strings (`""`) are ordinary non-null strings and sort before non-empty strings in
//!   ascending order and after them in descending order (`sort_multi_keys`, `sort_strings_nulls`,
//!   `sort_desc_ties`).
//! - Strings compare case-insensitively first, then by exact code points as a tie-breaker so
//!   lowercase precedes uppercase for the same letter (`"apple"` < `"Apple"` < `"APPLE"` <
//!   `"banana"` < `"Banana"` in ascending order; `sort_strings_nulls`). Inferred: this is a
//!   locale collation (lowercase first at the tertiary level), unlike CLEM's `<`, which
//!   compares code points.
//! - Equal keys come out in reverse input order (`sort_multi_keys` and `sort_desc_ties`: records 1
//!   and 5 share `grp = "B"` and `iv = 10`, and Modeler emits record 5 before record 1). GAP:
//!   only this pair is observed; we reverse every tie.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{DataModel, Table, Value};
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

/// Compares two non-null values in ascending Sort order (strings compare case-insensitively first,
/// then by reversed byte order so lowercase precedes uppercase for equal letters).
pub(crate) fn compare_non_null(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::String(sa), Value::String(sb)) => {
            for (ca, cb) in sa.chars().zip(sb.chars()) {
                let ord = ca
                    .to_lowercase()
                    .cmp(cb.to_lowercase())
                    .then_with(|| cb.cmp(&ca));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            sa.chars().count().cmp(&sb.chars().count())
        }
        _ => a.total_cmp(b),
    }
}

/// Sort rows with resolved keys: nulls always sort last regardless of direction.
pub(crate) fn compare_rows(keys: &[(usize, bool)], a: &[Value], b: &[Value]) -> Ordering {
    for &(position, descending) in keys {
        let va = a.get(position).unwrap_or(&Value::Null);
        let vb = b.get(position).unwrap_or(&Value::Null);
        let ordering = match (va.is_null(), vb.is_null()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                let ord = compare_non_null(va, vb);
                if descending { ord.reverse() } else { ord }
            }
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
        if !keys.is_empty() {
            rows.reverse();
            rows.sort_by(|a, b| compare_rows(&keys, a, b));
        }
        Table::new(input.model().clone(), rows)
            .map_err(|error| OperatorError::Data(error.to_string()))
    }
}
