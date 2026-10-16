//! Distinct (Record Ops): finds or removes duplicate records by key fields.
//!
//! Implemented: `distinctMode` (`Include`/`include` default: emit the first record of each key
//! group | `Discard`/`discard`: emit only the duplicate records after the first of each key group;
//! `Composite`/`composite` is rejected), `distinctFieldSelection/field[n]` (also accepted under
//! `distinctKeyFields/field[n]`; non-empty list of key field names),
//! `sortKeys/rows/row[i]/{field,direction}` (also accepted under `distinctSortWithinGroups/rows/row`;
//! optional within-group sort keys; direction `0`/`Ascending` or `1`/`Descending`), and
//! `sortDefaultOrder` (also accepted under `distinctDefaultOrder`; default ascending = `true`).
//! Performance hints `distinctLowDistinctKeyCount` and `disableSQLGeneration` are accepted and do
//! not change output data.
//! Evidence: `corpus/records/distinct_include.str`, `distinct_discard.str`,
//! `oracle/records/distinct_include`, `oracle/records/distinct_discard`, and the IBM SPSS Modeler
//! 19 Scripting and Automation Guide (`distinctnode` properties).
//!
//! Rules (settled by `oracle/records/distinct_include` and `distinct_discard`):
//! - Two records belong to the same group when all their key values are equal (null equals null;
//!   in `rec_main.csv`, rows 8 and 13 both have `grp = ""` and form one group).
//! - Groups are emitted in ascending key order as the Sort node orders values (`""`, `"A"`,
//!   `"B"`, `"C"`, `"D"`; inferred: case-insensitive collation and nulls last, as Sort). Within
//!   each key group, records are ordered by `sortKeys` if non-empty, and ties (or groups without
//!   within-group sort keys) are ordered in reverse input order, as Sort orders ties (the last
//!   input record of each tie comes first: in `distinct_include`, group `""` emits id 13; `"A"` emits id 14; `"B"` emits id 15; `"C"` emits id 11; `"D"` emits id 16; in
//!   `distinct_discard`, the remaining records of each group follow in reverse input order: `"A"`
//!   emits ids 10, 6, 4, 3; `"B"` emits ids 12, 9, 5, 2, 1).
//! - In `Include` mode, the first record of each ordered group is emitted; in `Discard` mode, all
//!   records after the first in each ordered group are emitted.
//! - `distinctIncludeCount`, `distinctCountFieldName`, and `compositeValuesTable` are saved with
//!   defaults even in `Include`/`Discard` modes and only take effect in `Composite` mode.
//!
//! GAP: `distinctPreSortedInput=true` / `existingSortKeys` has no oracle stream; when enabled,
//! adjacent records with equal keys form a group in stream order (sorting within each group if
//! `sortKeys` is set).
//! Not supported (`build` fails): `distinctMode=Composite` (composite record aggregation across
//! duplicates).

use super::sort::compare_rows;
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{DataModel, Table, Value};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Include,
    Discard,
}

struct SortKey {
    field: String,
    descending: bool,
}

struct Distinct {
    mode: Mode,
    keys: Vec<String>,
    within_group_sort: Vec<SortKey>,
    pre_sorted: bool,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    if settings::flag(s, "distinctCreateCompositeRecord")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "distinctCreateCompositeRecord=true is not supported yet".into(),
        ));
    }
    let mode = match settings::text(s, "distinctMode").as_deref().map(str::trim) {
        None | Some("") | Some("Include" | "include") => Mode::Include,
        Some("Discard" | "discard") => Mode::Discard,
        Some("Composite" | "composite") => {
            return Err(OperatorError::Settings(
                "distinctMode=Composite (composite record) is not supported yet".into(),
            ));
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "distinctMode: expected Include or Discard, found {other:?}"
            )));
        }
    };
    let mut keys: Vec<String> = settings::children(s, "distinctFieldSelection", "field")
        .into_iter()
        .map(|e| e.text())
        .filter(|t| !t.trim().is_empty())
        .collect();
    if keys.is_empty() {
        keys = settings::children(s, "distinctKeyFields", "field")
            .into_iter()
            .map(|e| e.text())
            .filter(|t| !t.trim().is_empty())
            .collect();
    }
    if keys.is_empty() {
        return Err(OperatorError::Settings(
            "distinctFieldSelection: at least one key field is required".into(),
        ));
    }
    let default_ascending = match settings::flag(s, "sortDefaultOrder")? {
        Some(v) => v,
        None => settings::flag(s, "distinctDefaultOrder")?.unwrap_or(true),
    };
    let default_descending = !default_ascending;
    let mut sort_rows = settings::children(s, "sortKeys/rows", "row");
    if sort_rows.is_empty() {
        sort_rows = settings::children(s, "distinctSortWithinGroups/rows", "row");
    }
    let mut within_group_sort = Vec::new();
    for row in sort_rows {
        let field = row
            .child("field")
            .map(|e| e.text())
            .filter(|f| !f.trim().is_empty())
            .ok_or_else(|| OperatorError::Settings("sortKeys: row without field".into()))?;
        let descending = match row.child("direction").map(|e| e.text()).as_deref() {
            None | Some("") => default_descending,
            Some("0" | "Ascending" | "ascending") => false,
            Some("1" | "Descending" | "descending") => true,
            Some(other) => {
                return Err(OperatorError::Settings(format!(
                    "sortKeys direction: unknown value {other:?}"
                )));
            }
        };
        within_group_sort.push(SortKey { field, descending });
    }
    let pre_sorted = match settings::flag(s, "distinctPreSortedInput")? {
        Some(v) => v,
        None => settings::flag(s, "distinctContiguousKeys")?.unwrap_or(false),
    };
    Ok(Box::new(Distinct {
        mode,
        keys,
        within_group_sort,
        pre_sorted,
    }))
}

#[derive(Debug, Clone, PartialEq)]
struct KeyTuple(Vec<Value>);

impl Eq for KeyTuple {}

impl PartialOrd for KeyTuple {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for KeyTuple {
    fn cmp(&self, other: &Self) -> Ordering {
        for (a, b) in self.0.iter().zip(&other.0) {
            let ord = super::sort::compare_ascending(a, b);
            if ord != Ordering::Equal {
                return ord;
            }
        }
        self.0.len().cmp(&other.0.len())
    }
}

impl Distinct {
    fn resolve_positions(model: &DataModel, names: &[String]) -> Result<Vec<usize>, OperatorError> {
        names
            .iter()
            .map(|name| {
                model
                    .position(name)
                    .ok_or_else(|| OperatorError::Data(format!("input has no field {name:?}")))
            })
            .collect()
    }

    fn resolve_sort_keys(&self, model: &DataModel) -> Result<Vec<(usize, bool)>, OperatorError> {
        self.within_group_sort
            .iter()
            .map(|key| {
                let pos = model.position(&key.field).ok_or_else(|| {
                    OperatorError::Data(format!("input has no sort field {:?}", key.field))
                })?;
                Ok((pos, key.descending))
            })
            .collect()
    }
}

impl Operator for Distinct {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Distinct needs one input".into()))?;
        let model = input.model();
        let key_positions = Self::resolve_positions(model, &self.keys)?;
        let sort_keys = self.resolve_sort_keys(model)?;

        let groups: Vec<Vec<Vec<Value>>> = if self.pre_sorted {
            let mut groups: Vec<Vec<Vec<Value>>> = Vec::new();
            let mut current_key: Option<KeyTuple> = None;
            for row in input.rows() {
                let key = KeyTuple(
                    key_positions
                        .iter()
                        .map(|&p| row.get(p).cloned().unwrap_or(Value::Null))
                        .collect(),
                );
                if current_key.as_ref() != Some(&key) {
                    current_key = Some(key);
                    groups.push(Vec::new());
                }
                if let Some(group) = groups.last_mut() {
                    group.push(row.clone());
                }
            }
            for group in &mut groups {
                if !sort_keys.is_empty() {
                    group.sort_by(|a, b| compare_rows(&sort_keys, a, b));
                }
            }
            groups
        } else {
            let mut map: BTreeMap<KeyTuple, Vec<Vec<Value>>> = BTreeMap::new();
            for row in input.rows() {
                let key = KeyTuple(
                    key_positions
                        .iter()
                        .map(|&p| row.get(p).cloned().unwrap_or(Value::Null))
                        .collect(),
                );
                map.entry(key).or_default().push(row.clone());
            }
            let mut groups = Vec::with_capacity(map.len());
            for (_, mut group) in map {
                group.reverse();
                if !sort_keys.is_empty() {
                    group.sort_by(|a, b| compare_rows(&sort_keys, a, b));
                }
                groups.push(group);
            }
            groups
        };

        let mut out = Vec::new();
        for group in groups {
            match self.mode {
                Mode::Include => {
                    if let Some(first) = group.into_iter().next() {
                        out.push(first);
                    }
                }
                Mode::Discard => {
                    out.extend(group.into_iter().skip(1));
                }
            }
        }

        Table::new(model.clone(), out).map_err(|error| OperatorError::Data(error.to_string()))
    }
}
