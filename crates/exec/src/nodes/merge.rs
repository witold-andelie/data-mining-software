//! Merge (Record Ops): combines the fields of several inputs, by order, by key, or by condition.
//!
//! Settings: `mergeMode` (`order` default | `key` | `condition`), `mergeCondition` (CLEM
//! condition for condition merge), `keyFields/field[n]`, `mergeCommonKeys` (must be true when
//! merging by key), `joinType` (`inner` default | `fullOuter` | `partialOuter` | `anti`),
//! `includeIncompleteRecords/includeIncompleteRecordsList/includeIncompleteRecords[i]/
//! inputNodeTagName` (the inputs whose unmatched records a partial outer join keeps),
//! `tagTable/tagNames/tag[@id]/name` (maps those tag names to inputs), `mergePrimaryDataset` (the
//! input an anti join reports; default 1), `mergeRenameDuplicateFields` (default true), and
//! `filterTableGroup/{defaultDrop,rows/row}` (drops/renames fields in the merged output).
//! Encodings from `docs/formats/nodes/merge.md`.
//!
//! Rules (settled by `oracle/records/merge_order`, `merge_key_inner`, `merge_key_full_outer`,
//! `merge_key_partial_outer`, `merge_key_anti`, `merge_condition`, `oracle/team/stream_2_M`, and
//! `oracle/team/cluster_analysis`):
//! - Output fields: the first input's fields, then each later input's fields in order. In key
//!   merge, a key field in a later input is coalesced into the first input's key column; any
//!   other non-key field name that appears in more than one input is renamed to `<tag>_<field>`
//!   on **every** input that has it (including input 1: `1_k`, `1_val`, `2_k`, `2_val` in
//!   `merge_condition`) when `mergeRenameDuplicateFields` is true, unless dropped or explicitly
//!   renamed by `filterTableGroup`. In `anti` join, only the primary dataset's fields are emitted
//!   with their original names (`merge_key_anti`: `k,l_id,val,l_num`).
//! - Order merge pairs records by position and stops at the shortest input (`merge_order`).
//! - Key merge: records with equal non-null keys are combined (`""` is an empty string, not null,
//!   so `""` matches `""`); null keys (`$null$`) never match any record (`merge_key_inner`,
//!   `merge_key_full_outer`, `merge_key_partial_outer`, `merge_key_anti`).
//! - Key merge row order: by ascending key in the Sort node's collation, matched and unmatched
//!   keys alike (`merge_key_full_outer`: `""`, `A`, `B`, then the unmatched `C`, `L1`, `R1`;
//!   `team/stream_2_M` interleaves them by `ID`); within one key each input's records come in
//!   reverse input order, as Sort orders ties (`merge_key_inner`: key `""` emits `l_id=7` then
//!   `l_id=4`; key `"B"` emits `(5, 105)`, `(5, 101)`, `(1, 105)`, `(1, 101)`).
//! - Anti join returns the primary input's records whose key is null or absent from the other
//!   inputs, ordered by ascending key with nulls last (`merge_key_anti`: `"C"` then `"L1"`).
//! - Condition merge evaluates `mergeCondition` on the cross product of input rows in a combined
//!   model where duplicate field names are disambiguated as `<tag>_<field>` on every input (and
//!   also accessible by bare name on input 1).
//!
//! Ignored (performance hints only): `mergeSingleLargeInput`/`mergeSingleLargeInputTag`,
//! `mergePreSortedInput`/`existingSortKeys`.
//! Not supported (`build` fails): `mergeCommonKeys=false`, non-empty `mergeRankConditions`.

use super::select::Condition;
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{DataModel, Field, Table, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Join {
    Inner,
    FullOuter,
    PartialOuter,
    Anti,
}

enum Mode {
    Order,
    Key { keys: Vec<String>, join: Join },
    Condition { condition: Condition },
}

struct FilterRow {
    drop: bool,
    rename: Option<String>,
}

struct Merge {
    mode: Mode,
    /// Zero-based inputs whose incomplete records a partial outer join keeps.
    outer_inputs: Vec<usize>,
    primary: usize,
    rename_duplicates: bool,
    /// Zero-based input index -> tag name (default `"1"`, `"2"`, ...).
    input_tags: HashMap<usize, String>,
    default_drop: bool,
    filter_rows: HashMap<String, FilterRow>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    if !settings::children(s, "mergeRankConditions/rows", "row").is_empty() {
        return Err(OperatorError::Settings(
            "mergeRankConditions not supported yet".into(),
        ));
    }
    let parse_join = || -> Result<Join, OperatorError> {
        match settings::text(s, "joinType").as_deref().map(str::trim) {
            None | Some("") | Some("inner") => Ok(Join::Inner),
            Some("fullOuter") => Ok(Join::FullOuter),
            Some("partialOuter") => Ok(Join::PartialOuter),
            Some("anti") => Ok(Join::Anti),
            Some(other) => Err(OperatorError::Settings(format!(
                "joinType: unknown value {other:?}"
            ))),
        }
    };
    let mode = match settings::text(s, "mergeMode").as_deref().map(str::trim) {
        None | Some("") | Some("order") => Mode::Order,
        Some("key") => {
            if !settings::flag(s, "mergeCommonKeys")?.unwrap_or(true) {
                return Err(OperatorError::Settings(
                    "mergeCommonKeys=false not supported yet".into(),
                ));
            }
            let keys: Vec<String> = settings::children(s, "keyFields", "field")
                .into_iter()
                .map(|e| e.text())
                .collect();
            if keys.is_empty() {
                return Err(OperatorError::Settings(
                    "merge by key without key fields".into(),
                ));
            }
            Mode::Key {
                keys,
                join: parse_join()?,
            }
        }
        Some("condition") => {
            let condition =
                Condition::parse(settings::text(s, "mergeCondition"), "mergeCondition")?;
            if parse_join()? != Join::Inner {
                return Err(OperatorError::Settings(
                    "merge by condition only supports inner join".into(),
                ));
            }
            Mode::Condition { condition }
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "mergeMode: unknown value {other:?}"
            )));
        }
    };
    // tag name -> tag id, and zero-based input index -> tag name
    let mut tag_ids: HashMap<String, usize> = HashMap::new();
    let mut input_tags: HashMap<usize, String> = HashMap::new();
    for tag in settings::children(s, "tagTable/tagNames", "tag") {
        let id: Option<usize> = tag.attribute("id").and_then(|id| id.trim().parse().ok());
        let name = tag.child("name").map(|e| e.text());
        if let (Some(id), Some(name)) = (id, name) {
            if let Some(idx) = id.checked_sub(1) {
                input_tags.insert(idx, name.clone());
            }
            tag_ids.insert(name, id);
        }
    }
    let mut outer_inputs = Vec::new();
    for entry in settings::children(
        s,
        "includeIncompleteRecords/includeIncompleteRecordsList",
        "includeIncompleteRecords",
    ) {
        let name = entry
            .child("inputNodeTagName")
            .map(|e| e.text())
            .unwrap_or_default();
        let id = tag_ids
            .get(&name)
            .copied()
            .or_else(|| name.trim().parse().ok())
            .ok_or_else(|| OperatorError::Settings(format!("unknown input tag {name:?}")))?;
        outer_inputs.push(id.checked_sub(1).ok_or_else(|| {
            OperatorError::Settings(format!("input tag {name:?} is not a valid input"))
        })?);
    }
    let primary = settings::integer(s, "mergePrimaryDataset")?
        .unwrap_or(1)
        .checked_sub(1)
        .and_then(|p| usize::try_from(p).ok())
        .ok_or_else(|| OperatorError::Settings("mergePrimaryDataset must be at least 1".into()))?;
    let rename_duplicates = settings::flag(s, "mergeRenameDuplicateFields")?.unwrap_or(true);
    let default_drop = settings::flag(s, "filterTableGroup/defaultDrop")?.unwrap_or(false);
    let mut filter_rows = HashMap::new();
    for row in settings::children(s, "filterTableGroup/rows", "row") {
        let name = row.child("name").map(|e| e.text()).unwrap_or_default();
        let drop = settings::flag(Some(row), "drop")?.unwrap_or(false);
        let rename = settings::text(Some(row), "rename").filter(|r| !r.trim().is_empty());
        filter_rows.insert(name, FilterRow { drop, rename });
    }
    Ok(Box::new(Merge {
        mode,
        outer_inputs,
        primary,
        rename_duplicates,
        input_tags,
        default_drop,
        filter_rows,
    }))
}

/// Key values ordered with `Value::total_cmp`.
#[derive(Debug, Clone, PartialEq)]
struct KeyTuple(Vec<Value>);

impl Eq for KeyTuple {}

impl PartialOrd for KeyTuple {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Keys order as the Sort node orders values (Modeler sorts each input by key before joining,
/// inferred from the shared tie and collation behaviour).
impl Ord for KeyTuple {
    fn cmp(&self, other: &Self) -> Ordering {
        for (a, b) in self.0.iter().zip(&other.0) {
            let ordering = super::sort::compare_non_null(a, b);
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        self.0.len().cmp(&other.0.len())
    }
}

/// Where one field of one input lands in the output row.
enum FieldAction {
    /// Excluded by `filterTableGroup`.
    Drop,
    /// Writes into output column `usize`; if `coalesce` is true (a shared key field), only writes
    /// when the slot is still `Value::Null`.
    Write { target: usize, coalesce: bool },
}

impl Operator for Merge {
    fn arity(&self) -> Arity {
        Arity::AtLeast(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let key_names: &[String] = match &self.mode {
            Mode::Order | Mode::Condition { .. } => &[],
            Mode::Key { keys, .. } => keys,
        };
        // Key positions per input.
        let mut key_positions: Vec<Vec<usize>> = Vec::new();
        if matches!(self.mode, Mode::Key { .. }) {
            for (n, input) in inputs.iter().enumerate() {
                key_positions.push(
                    key_names
                        .iter()
                        .map(|k| {
                            input.model().position(k).ok_or_else(|| {
                                OperatorError::Data(format!(
                                    "input {} has no key field {k:?}",
                                    n + 1
                                ))
                            })
                        })
                        .collect::<Result<_, _>>()?,
                );
            }
        }

        if let Mode::Key {
            join: Join::Anti, ..
        } = &self.mode
        {
            return self.anti(inputs, &key_positions);
        }

        // Count how many inputs contain each field name (for disambiguating duplicates).
        let mut name_counts: HashMap<&str, usize> = HashMap::new();
        for input in inputs {
            for field in input.model().fields() {
                *name_counts.entry(field.name.as_str()).or_insert(0) += 1;
            }
        }

        // Output fields and where each input's fields go.
        let mut fields: Vec<Field> = Vec::new();
        let mut placement: Vec<Vec<FieldAction>> = Vec::new();
        let mut key_targets: HashMap<String, usize> = HashMap::new();
        let mut seen_raw: HashSet<String> = HashSet::new();

        for (n, input) in inputs.iter().enumerate() {
            let tag = self
                .input_tags
                .get(&n)
                .cloned()
                .unwrap_or_else(|| (n + 1).to_string());
            let mut map = Vec::new();
            for field in input.model().fields() {
                let is_later_dup = !seen_raw.insert(field.name.clone());
                let is_shared = name_counts.get(field.name.as_str()).copied().unwrap_or(0) > 1;
                if is_later_dup
                    && key_names.contains(&field.name)
                    && let Some(&target) = key_targets.get(&field.name)
                {
                    map.push(FieldAction::Write {
                        target,
                        coalesce: true,
                    });
                    continue;
                }
                let is_dup_non_key = is_shared && !key_names.contains(&field.name);
                let tagged = format!("{tag}_{}", field.name);
                let filter_row = if is_later_dup || is_dup_non_key {
                    self.filter_rows.get(&tagged).or_else(|| {
                        (!is_later_dup)
                            .then(|| self.filter_rows.get(&field.name))
                            .flatten()
                    })
                } else {
                    self.filter_rows
                        .get(&field.name)
                        .or_else(|| self.filter_rows.get(&tagged))
                };
                let drop = filter_row.map_or(self.default_drop, |r| r.drop);
                if drop {
                    map.push(FieldAction::Drop);
                    continue;
                }
                let mut out_field = field.clone();
                if let Some(rename) = filter_row.and_then(|r| r.rename.as_ref()) {
                    out_field.name = rename.clone();
                } else if is_dup_non_key {
                    if self.rename_duplicates {
                        out_field.name = tagged;
                    } else if is_later_dup {
                        return Err(OperatorError::Data(format!(
                            "field {:?} appears in several inputs; duplicate fields are not supported when mergeRenameDuplicateFields is false",
                            field.name
                        )));
                    }
                }
                let target = fields.len();
                if key_names.contains(&field.name) {
                    key_targets.entry(field.name.clone()).or_insert(target);
                }
                map.push(FieldAction::Write {
                    target,
                    coalesce: false,
                });
                fields.push(out_field);
            }
            placement.push(map);
        }
        let model =
            DataModel::new(fields).map_err(|error| OperatorError::Data(error.to_string()))?;
        let width = model.len();

        // Row combinations: one optional row index per input.
        let combos: Vec<Vec<Option<usize>>> = match &self.mode {
            Mode::Order => {
                let shortest = inputs.iter().map(|t| t.row_count()).min().unwrap_or(0);
                (0..shortest).map(|i| vec![Some(i); inputs.len()]).collect()
            }
            Mode::Key { join, .. } => self.key_combos(inputs, &key_positions, *join),
            Mode::Condition { condition } => {
                self.condition_combos(inputs, &name_counts, condition)?
            }
        };

        let mut rows = Vec::with_capacity(combos.len());
        for combo in &combos {
            let mut out = vec![Value::Null; width];
            for (n, choice) in combo.iter().enumerate() {
                let Some(row) = choice.and_then(|i| inputs.get(n).and_then(|t| t.rows().get(i)))
                else {
                    continue;
                };
                for (position, action) in placement.get(n).into_iter().flatten().enumerate() {
                    if let FieldAction::Write { target, coalesce } = *action
                        && let (Some(slot), Some(value)) = (out.get_mut(target), row.get(position))
                        && (!coalesce || slot.is_null())
                    {
                        *slot = value.clone();
                    }
                }
            }
            rows.push(out);
        }
        Table::new(model, rows).map_err(|error| OperatorError::Data(error.to_string()))
    }
}

/// A condition merge tests every combination of records; beyond this it fails instead of
/// running for hours.
const MAX_CONDITION_COMBINATIONS: u64 = 100_000_000;

impl Merge {
    fn condition_combos(
        &self,
        inputs: &[Arc<Table>],
        name_counts: &HashMap<&str, usize>,
        condition: &Condition,
    ) -> Result<Vec<Vec<Option<usize>>>, OperatorError> {
        // Build an evaluation model containing every input field: unique fields keep their name;
        // fields present in multiple inputs are exposed both under `<tag>_<name>` on every input
        // and under the bare `<name>` on the first input that has it.
        let mut eval_fields: Vec<Field> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        for (n, input) in inputs.iter().enumerate() {
            let tag = self
                .input_tags
                .get(&n)
                .cloned()
                .unwrap_or_else(|| (n + 1).to_string());
            for field in input.model().fields() {
                let is_shared = name_counts.get(field.name.as_str()).copied().unwrap_or(0) > 1;
                let is_first = seen.insert(field.name.as_str());
                if is_shared {
                    let mut tagged = field.clone();
                    tagged.name = format!("{tag}_{}", field.name);
                    eval_fields.push(tagged);
                    if is_first {
                        eval_fields.push(field.clone());
                    }
                } else {
                    eval_fields.push(field.clone());
                }
            }
        }
        let eval_model =
            DataModel::new(eval_fields).map_err(|error| OperatorError::Data(error.to_string()))?;
        let bound = condition.bind(&eval_model)?;

        // Every combination of records, the last input varying fastest, visited one at a time.
        let total = inputs
            .iter()
            .try_fold(1u64, |acc, input| {
                acc.checked_mul(u64::try_from(input.row_count()).unwrap_or(u64::MAX))
            })
            .unwrap_or(u64::MAX);
        if total > MAX_CONDITION_COMBINATIONS {
            return Err(OperatorError::Data(format!(
                "merge by condition would test {total} record combinations (limit                  {MAX_CONDITION_COMBINATIONS})"
            )));
        }
        let mut combos = Vec::new();
        if total == 0 {
            return Ok(combos);
        }
        let mut indices = vec![0usize; inputs.len()];
        let mut combined = Vec::with_capacity(eval_model.len());
        let mut seen_row = HashSet::new();
        for ordinal in 0..total {
            combined.clear();
            seen_row.clear();
            for (input, &row_idx) in inputs.iter().zip(&indices) {
                if let Some(row) = input.rows().get(row_idx) {
                    for (field, val) in input.model().fields().iter().zip(row) {
                        let is_shared =
                            name_counts.get(field.name.as_str()).copied().unwrap_or(0) > 1;
                        let is_first = seen_row.insert(field.name.as_str());
                        if is_shared {
                            combined.push(val.clone());
                            if is_first {
                                combined.push(val.clone());
                            }
                        } else {
                            combined.push(val.clone());
                        }
                    }
                }
            }
            if bound.truth_of(&combined, ordinal.saturating_add(1))? == Some(true) {
                combos.push(indices.iter().copied().map(Some).collect());
            }
            for (n, index) in indices.iter_mut().enumerate().rev() {
                *index += 1;
                if *index < inputs.get(n).map_or(0, |input| input.row_count()) {
                    break;
                }
                *index = 0;
            }
        }
        Ok(combos)
    }

    fn key_combos(
        &self,
        inputs: &[Arc<Table>],
        key_positions: &[Vec<usize>],
        join: Join,
    ) -> Vec<Vec<Option<usize>>> {
        let count = inputs.len();
        let mut groups: BTreeMap<KeyTuple, Vec<Vec<usize>>> = BTreeMap::new();
        let mut null_keyed: Vec<(usize, usize)> = Vec::new();
        for (n, input) in inputs.iter().enumerate() {
            let positions = key_positions.get(n).map(Vec::as_slice).unwrap_or_default();
            for (i, row) in input.rows().iter().enumerate() {
                let key: Vec<Value> = positions
                    .iter()
                    .map(|&p| row.get(p).cloned().unwrap_or(Value::Null))
                    .collect();
                if key.iter().any(Value::is_null) {
                    null_keyed.push((n, i));
                } else {
                    let entry = groups
                        .entry(KeyTuple(key))
                        .or_insert_with(|| vec![Vec::new(); count]);
                    if let Some(list) = entry.get_mut(n) {
                        list.push(i);
                    }
                }
            }
        }
        let keeps = |present: &[bool]| -> bool {
            let all = present.iter().all(|p| *p);
            match join {
                Join::Inner | Join::Anti => all,
                Join::FullOuter => present.iter().any(|p| *p),
                Join::PartialOuter => {
                    all || self
                        .outer_inputs
                        .iter()
                        .any(|&n| present.get(n).copied().unwrap_or(false))
                }
            }
        };
        let mut combos = Vec::new();
        for lists in groups.values() {
            let present: Vec<bool> = lists.iter().map(|l| !l.is_empty()).collect();
            if !keeps(&present) {
                continue;
            }
            let mut product: Vec<Vec<Option<usize>>> = vec![Vec::new()];
            for list in lists {
                let options: Vec<Option<usize>> = if list.is_empty() {
                    vec![None]
                } else {
                    list.iter().rev().map(|&i| Some(i)).collect()
                };
                product = product
                    .into_iter()
                    .flat_map(|prefix| {
                        options.iter().map(move |option| {
                            let mut next = prefix.clone();
                            next.push(*option);
                            next
                        })
                    })
                    .collect();
            }
            combos.extend(product);
        }
        // Null keys never match; inferred from the Sort node: after the other keys, in reverse
        // input order (no oracle case has a null key).
        for &(n, i) in null_keyed.iter().rev() {
            let mut present = vec![false; count];
            if let Some(p) = present.get_mut(n) {
                *p = true;
            }
            if keeps(&present) {
                let mut combo = vec![None; count];
                if let Some(slot) = combo.get_mut(n) {
                    *slot = Some(i);
                }
                combos.push(combo);
            }
        }
        combos
    }

    /// Anti join: the primary input's records with no key match in any other input.
    fn anti(
        &self,
        inputs: &[Arc<Table>],
        key_positions: &[Vec<usize>],
    ) -> Result<Table, OperatorError> {
        let primary = inputs.get(self.primary).ok_or_else(|| {
            OperatorError::Settings(format!(
                "mergePrimaryDataset {} but only {} inputs",
                self.primary + 1,
                inputs.len()
            ))
        })?;
        let mut seen: BTreeSet<KeyTuple> = BTreeSet::new();
        for (n, input) in inputs.iter().enumerate() {
            if n == self.primary {
                continue;
            }
            let positions = key_positions.get(n).map(Vec::as_slice).unwrap_or_default();
            for row in input.rows() {
                let key: Vec<Value> = positions
                    .iter()
                    .map(|&p| row.get(p).cloned().unwrap_or(Value::Null))
                    .collect();
                if !key.iter().any(Value::is_null) {
                    seen.insert(KeyTuple(key));
                }
            }
        }
        let positions = key_positions
            .get(self.primary)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let sort_keys: Vec<(usize, bool)> = positions.iter().map(|&p| (p, false)).collect();
        let mut rows: Vec<Vec<Value>> = primary
            .rows()
            .iter()
            .filter(|row| {
                let key: Vec<Value> = positions
                    .iter()
                    .map(|&p| row.get(p).cloned().unwrap_or(Value::Null))
                    .collect();
                key.iter().any(Value::is_null) || !seen.contains(&KeyTuple(key))
            })
            .cloned()
            .collect();
        rows.reverse();
        rows.sort_by(|a, b| super::sort::compare_rows(&sort_keys, a, b));
        Table::new(primary.model().clone(), rows)
            .map_err(|error| OperatorError::Data(error.to_string()))
    }
}
