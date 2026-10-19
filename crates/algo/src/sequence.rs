//! Sequence (`BuildSequenceNode`): sequential pattern mining, Algorithms Guide "Sequence
//! Algorithm" (Agrawal and Srikant 1995; Hidber 1999 for the lattice).
//!
//! **Statistical parity (tie order free)** (`docs/statistical-parity.md`, acceptance rule 4):
//! the rules and their statistics are exactly Modeler's; only the order of rules tied on
//! confidence, antecedent length and antecedent support is free, because Modeler orders those
//! by no recoverable key (`target/sequence/rules.py`, `disc.py`).
//!
//! Evidence: `crates/exec/tests/sequence.rs` reproduces all eight oracles
//! (`train_next/sequence_*`, `models_next/sequence_*`: 95 rules, 64 or 12 transactions) at 15
//! significant digits, the precision Modeler stores, and all 1608 scored records. Rule order
//! differs only inside 12 tie groups (29 rules); predictions differ only inside
//! equal-confidence groups.
//!
//! # Data
//!
//! Records of one id (`SequenceIdField`) form a transaction; the rows sharing a time value
//! (`SequenceTimeField`) form one itemset, holding the non-blank values of the content fields
//! (`SequenceContentField`). Itemsets are ordered by the first appearance of their time. Blanks
//! are ignored: a record with a blank content value simply lacks that item, so it matches no
//! rule requiring it (Algorithms Guide, "Blank Handling"). A blank time is a time key of its own
//! (GAP: no oracle has one).
//!
//! # Mining
//!
//! A sequence is frequent when at least `SequenceSupp` percent of the transactions contain it:
//! each of its itemsets is a subset of one transaction itemset, each in a strictly later one
//! than the previous. The lattice is grown level by level, one level per item of the sequence,
//! by appending an itemset `{x}` or by adding a frequent item `x` to the last itemset. Both
//! moves add one item, so a level is exactly the sequences of that many items, and
//! `SequenceMaxSize` caps the number of items of a mined sequence (the guide's "size").
//!
//! Modeler mines the same lattice with a two-pass Carma, which the guide notes can miss a
//! sequence whose last subsequence enters the lattice late (`{crackers} > {wine} > {beer}`).
//! This implementation enumerates every frequent sequence exactly, so it keeps those; the six
//! `train_next/sequence_*` oracles contain none.
//!
//! # Rules
//!
//! Every frequent sequence of at least two itemsets yields one rule: the leading itemsets are
//! the antecedent, the last itemset is the consequent (Algorithms Guide, "Sequential Patterns").
//! A rule is kept when its support and its confidence both pass:
//!
//! | quantity | meaning |
//! |---|---|
//! | `occurrence` | transactions containing the antecedent and the consequent, in order |
//! | `support` (PMML) | the antecedent's share of the transactions |
//! | rule support | `occurrence / transactions` |
//! | `confidence` | `occurrence / antecedent transactions` |
//!
//! The support bound is the rule support, not the antecedent's: `sequence_default` keeps no
//! `teal ⇒ copper` rule although `{teal}` covers 25 % of the transactions, because the pair
//! covers 12.5 %. Since the antecedent of a kept rule always covers at least the rule support,
//! it is frequent too.
//!
//! # Order
//!
//! Confidence descending, then antecedent length (itemsets) ascending, then antecedent support
//! descending, then the antecedent's itemsets and the consequent in item order — a
//! deterministic tie break of our own, since Modeler's order inside such a group is not
//! recoverable (`target/sequence/rules.py`). Item ids are numbered by first mention in rule
//! order.
//!
//! # GAPs
//!
//! `SequenceUseGaps` (gap statistics), `SequenceUseTime` false (no time field),
//! `SequenceUseTolerance` (timestamp tolerance), `SequenceUseMaxDuration`, `SequenceUsePruning`,
//! `useEvents` / `eventFields`, frequency and weight fields, a partition field, and
//! non-contiguous ids are rejected: no oracle covers them. More than one content field is
//! mined, but PMML re-import names only the first (no oracle has several). A consequent of
//! several items (or several itemsets) is mined but scored by no nugget
//! (`crates/exec/src/nodes/sequence.rs`).

use crate::AlgoError;
use crate::common::data_field;
use dms_pmml::{
    DataField, InvalidTreatment, MiningField, MiningFunction, Model, ModelHeader, Outliers, Pmml,
    SequenceItemset, SequenceModel, SequenceRule, Usage,
};
use dms_types::{Table, Value};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

/// Percent scaled by one million, as Apriori does, so 10, 20 and 75 percent of 64 transactions
/// do not depend on binary fractions.
const PERCENT_SCALE: u128 = 1_000_000;
const PERCENT_CROSS: u128 = PERCENT_SCALE * 100;

const MAX_TRANSACTIONS: usize = 5_000_000;
const MAX_ITEMS: usize = 100_000;
/// Candidate sequences generated for one level before giving up with a readable error.
const MAX_CANDIDATES: usize = 2_000_000;
/// `SequenceMaxSize` above this is rejected instead of explored.
const MAX_SIZE: usize = 10_000;

/// Settings of a `BuildSequenceNode`. Defaults match the property descriptors of
/// `corpus/train_next/sequence_default.str`.
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceSettings {
    /// `SequenceSupp`, percent of the transactions a rule must cover. Default 20.
    pub min_support_percent: f64,
    /// `SequenceConf`, percent. Default 20.
    pub min_confidence_percent: f64,
    /// `SequenceMaxSize`: the most items a mined sequence may have. Default 10.
    pub max_size: usize,
    /// `SequenceIdField`.
    pub id_field: String,
    /// `SequenceTimeField`.
    pub time_field: String,
    /// `SequenceContentField`, in listed order.
    pub content_fields: Vec<String>,
    /// `SequenceMaxSequences`: the most frequent sequences the lattice may hold.
    pub max_sequences: usize,
    /// `modelName` (`train_next` in the oracles).
    pub model_name: String,
}

impl Default for SequenceSettings {
    fn default() -> Self {
        Self {
            min_support_percent: 20.0,
            min_confidence_percent: 20.0,
            max_size: 10,
            id_field: String::new(),
            time_field: String::new(),
            content_fields: Vec::new(),
            max_sequences: 1_000_000,
            model_name: String::new(),
        }
    }
}

/// Trains a Sequence model on `table` (already restricted to the training partition).
pub fn train(table: &Table, settings: &SequenceSettings) -> Result<Pmml, AlgoError> {
    validate(settings)?;
    let data = Data::build(table, settings)?;
    if data.groups.is_empty() {
        return Err(AlgoError::Data("Sequence has no transactions".into()));
    }
    let lattice = mine(&data, settings)?;
    let rules = rules(settings, &lattice)?;
    assemble(table, settings, &data, &rules)
}

fn validate(settings: &SequenceSettings) -> Result<(), AlgoError> {
    let percent = |name: &str, value: f64| -> Result<(), AlgoError> {
        if !value.is_finite() || !(0.0..=100.0).contains(&value) {
            return Err(AlgoError::Settings(format!(
                "Sequence {name} must be from 0 to 100, found {value}"
            )));
        }
        Ok(())
    };
    percent("SequenceSupp", settings.min_support_percent)?;
    percent("SequenceConf", settings.min_confidence_percent)?;
    if !(1..=MAX_SIZE).contains(&settings.max_size) {
        return Err(AlgoError::Settings(format!(
            "Sequence SequenceMaxSize must be from 1 to {MAX_SIZE}, found {}",
            settings.max_size
        )));
    }
    if settings.id_field.trim().is_empty() {
        return Err(AlgoError::Settings("Sequence needs SequenceIdField".into()));
    }
    if settings.time_field.trim().is_empty() {
        return Err(AlgoError::Settings(
            "Sequence needs SequenceTimeField (GAP: no oracle mines without a time field)".into(),
        ));
    }
    if settings.content_fields.is_empty() {
        return Err(AlgoError::Settings(
            "Sequence needs SequenceContentField".into(),
        ));
    }
    if settings.max_sequences == 0 {
        return Err(AlgoError::Settings(
            "Sequence SequenceMaxSequences must be positive".into(),
        ));
    }
    Ok(())
}

/// The transactions, with items numbered by first mention while reading the records.
struct Data {
    /// One entry per id: its itemsets in time order, as item indices.
    groups: Vec<Vec<Vec<u32>>>,
    /// The items of each group, for singleton support.
    present: Vec<HashSet<u32>>,
    /// `(field, value)` of each item index.
    values: Vec<(String, String)>,
    /// The number of transactions.
    n: u64,
}

impl Data {
    fn build(table: &Table, settings: &SequenceSettings) -> Result<Data, AlgoError> {
        let id_at = field_position(table, &settings.id_field)?;
        let time_at = field_position(table, &settings.time_field)?;
        let mut content_at: Vec<usize> = Vec::new();
        for name in &settings.content_fields {
            let at = field_position(table, name)?;
            if !content_at.contains(&at) {
                content_at.push(at);
            }
        }
        if content_at.is_empty() {
            return Err(AlgoError::Settings(
                "Sequence needs SequenceContentField".into(),
            ));
        }
        let fields = table.model();
        let content_names: Vec<&str> = content_at
            .iter()
            .filter_map(|&at| fields.fields().get(at).map(|field| field.name.as_str()))
            .collect();
        if content_names.len() != content_at.len() {
            return Err(AlgoError::Settings(
                "Sequence content field is out of range".into(),
            ));
        }
        if table.rows().len() > MAX_TRANSACTIONS {
            return Err(AlgoError::Data(format!(
                "Sequence has {} records; the limit is {MAX_TRANSACTIONS}",
                table.rows().len()
            )));
        }
        let mut index: HashMap<(String, String), u32> = HashMap::new();
        let mut values: Vec<(String, String)> = Vec::new();
        let mut groups: Vec<Vec<Vec<u32>>> = Vec::new();
        let mut present: Vec<HashSet<u32>> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut current: Option<Value> = None;
        let mut slots: HashMap<String, usize> = HashMap::new();
        let mut sets: Vec<Vec<u32>> = Vec::new();
        for (row_index, row) in table.rows().iter().enumerate() {
            let id = row.get(id_at).cloned().unwrap_or(Value::Null);
            if id.is_null() {
                return Err(AlgoError::Data(format!(
                    "Sequence id is null at row {}",
                    row_index + 1
                )));
            }
            if current.as_ref() != Some(&id) {
                if current.is_some() {
                    let done = std::mem::take(&mut sets);
                    present.push(done.iter().flatten().copied().collect());
                    groups.push(done);
                    slots.clear();
                }
                // Modeler requires the records of one id to be adjacent (`SequenceContiguous`).
                if !seen.insert(format!("{id:?}")) {
                    return Err(AlgoError::Data(format!(
                        "Sequence id {id} is not contiguous at row {}",
                        row_index + 1
                    )));
                }
                current = Some(id);
            }
            let time = row.get(time_at).cloned().unwrap_or(Value::Null);
            let slot = match slots.entry(format!("{time:?}")) {
                Entry::Occupied(entry) => *entry.get(),
                Entry::Vacant(entry) => {
                    let next = sets.len();
                    entry.insert(next);
                    sets.push(Vec::new());
                    next
                }
            };
            for (position, &at) in content_at.iter().enumerate() {
                let Some(value) = row.get(at).filter(|value| !value.is_null()) else {
                    continue;
                };
                let key = (content_names[position].to_string(), value.to_string());
                let item = match index.get(&key) {
                    Some(&item) => item,
                    None => {
                        if values.len() >= MAX_ITEMS {
                            return Err(AlgoError::Data(format!(
                                "Sequence has {MAX_ITEMS} items; the limit is reached"
                            )));
                        }
                        let item = u32::try_from(values.len()).map_err(|_| {
                            AlgoError::Data("Sequence has too many items".to_string())
                        })?;
                        values.push(key.clone());
                        index.insert(key, item);
                        item
                    }
                };
                if let Some(set) = sets.get_mut(slot)
                    && !set.contains(&item)
                {
                    set.push(item);
                }
            }
        }
        if current.is_some() {
            let done = std::mem::take(&mut sets);
            present.push(done.iter().flatten().copied().collect());
            groups.push(done);
        }
        let n = u64::try_from(groups.len())
            .map_err(|_| AlgoError::Data("Sequence has too many transactions".into()))?;
        Ok(Data {
            groups,
            present,
            values,
            n,
        })
    }
}

/// A frequent sequence and the number of transactions containing it.
struct Frequent {
    sets: Vec<Vec<u32>>,
    count: u64,
}

/// Every frequent sequence, one level per item of the sequence.
fn mine(data: &Data, settings: &SequenceSettings) -> Result<Vec<Vec<Frequent>>, AlgoError> {
    let support = percent_basis(settings.min_support_percent)?;
    let items = u32::try_from(data.values.len())
        .map_err(|_| AlgoError::Data("Sequence has too many items".into()))?;
    let mut first: Vec<Frequent> = Vec::new();
    for item in 0..items {
        let count = data
            .present
            .iter()
            .filter(|present| present.contains(&item))
            .count();
        let count = u64::try_from(count)
            .map_err(|_| AlgoError::Data("Sequence has too many transactions".into()))?;
        if at_least(count, support, data.n) {
            first.push(Frequent {
                sets: vec![vec![item]],
                count,
            });
        }
    }
    if first.is_empty() {
        return Ok(Vec::new());
    }
    let frequent_items: Vec<u32> = first.iter().map(|f| f.sets[0][0]).collect();
    let mut levels = vec![first];
    for _ in 2..=settings.max_size {
        let Some(previous) = levels.last() else { break };
        let mut candidates: Vec<Vec<Vec<u32>>> = Vec::new();
        let mut generated: HashSet<Vec<Vec<u32>>> = HashSet::new();
        for sequence in previous {
            // A new itemset after the last one.
            for &item in &frequent_items {
                let mut next = sequence.sets.clone();
                next.push(vec![item]);
                if generated.insert(next.clone()) {
                    candidates.push(next);
                }
            }
            // One more item in the last itemset.
            let Some(last) = sequence.sets.last() else {
                continue;
            };
            for &item in &frequent_items {
                if last.contains(&item) {
                    continue;
                }
                let mut merged = last.clone();
                merged.push(item);
                merged.sort_unstable();
                let mut next = sequence.sets.clone();
                if let Some(slot) = next.last_mut() {
                    *slot = merged;
                }
                if generated.insert(next.clone()) {
                    candidates.push(next);
                }
            }
        }
        if candidates.len() > MAX_CANDIDATES {
            return Err(AlgoError::Data(format!(
                "Sequence generated {candidates_len} candidate sequences at one level; the \
                 limit is {MAX_CANDIDATES} (raise SequenceSupp)",
                candidates_len = candidates.len()
            )));
        }
        let mut level = Vec::new();
        for candidate in &candidates {
            let count = support_of(data, candidate);
            if at_least(count, support, data.n) {
                level.push(Frequent {
                    sets: candidate.clone(),
                    count,
                });
            }
        }
        if level.is_empty() {
            break;
        }
        let total: usize = levels
            .iter()
            .map(|level| level.len())
            .sum::<usize>()
            .saturating_add(level.len());
        if total > settings.max_sequences {
            return Err(AlgoError::Data(format!(
                "Sequence mined {total} frequent sequences; SequenceMaxSequences is {}",
                settings.max_sequences
            )));
        }
        levels.push(level);
    }
    Ok(levels)
}

/// Transactions containing `sequence`: each itemset a subset of one transaction itemset, each in
/// a strictly later one than the previous.
fn occurs(sequence: &[Vec<u32>], group: &[Vec<u32>]) -> bool {
    let mut next = 0;
    for set in sequence {
        let found = group.get(next..).and_then(|rest| {
            rest.iter()
                .position(|items| set.iter().all(|item| items.contains(item)))
        });
        match found {
            Some(offset) => next = next.saturating_add(offset).saturating_add(1),
            None => return false,
        }
    }
    true
}

fn support_of(data: &Data, sequence: &[Vec<u32>]) -> u64 {
    let count = data
        .groups
        .iter()
        .filter(|group| occurs(sequence, group))
        .count();
    u64::try_from(count).unwrap_or(u64::MAX)
}

/// A rule over item indices, with the statistics Modeler's PMML carries.
struct Mined {
    antecedent: Vec<Vec<u32>>,
    consequent: Vec<u32>,
    confidence: f64,
    /// Transactions containing the antecedent.
    antecedent_count: u64,
    /// Transactions containing the antecedent and the consequent, in order.
    occurrence: u64,
}

/// The rules of every frequent sequence of at least two itemsets, in Modeler's order.
fn rules(settings: &SequenceSettings, levels: &[Vec<Frequent>]) -> Result<Vec<Mined>, AlgoError> {
    let confidence_basis = percent_basis(settings.min_confidence_percent)?;
    let mut counts: HashMap<Vec<Vec<u32>>, u64> = HashMap::new();
    for level in levels {
        for sequence in level {
            counts.insert(sequence.sets.clone(), sequence.count);
        }
    }
    let mut mined: Vec<Mined> = Vec::new();
    for level in levels {
        for sequence in level {
            if sequence.sets.len() < 2 {
                continue;
            }
            let Some((consequent, antecedent)) = sequence.sets.split_last() else {
                continue;
            };
            let antecedent_count = counts
                .get(antecedent)
                .copied()
                .ok_or_else(|| AlgoError::Data("a frequent antecedent was not mined".into()))?;
            if !at_least_ratio(sequence.count, antecedent_count, confidence_basis) {
                continue;
            }
            mined.push(Mined {
                antecedent: antecedent.to_vec(),
                consequent: consequent.clone(),
                confidence: ratio(sequence.count, antecedent_count),
                antecedent_count,
                occurrence: sequence.count,
            });
        }
    }
    mined.sort_by(|left, right| {
        right
            .confidence
            .total_cmp(&left.confidence)
            .then(left.antecedent.len().cmp(&right.antecedent.len()))
            .then(right.antecedent_count.cmp(&left.antecedent_count))
            .then(left.antecedent.cmp(&right.antecedent))
            .then(left.consequent.cmp(&right.consequent))
    });
    Ok(mined)
}

fn assemble(
    table: &Table,
    settings: &SequenceSettings,
    data: &Data,
    rules: &[Mined],
) -> Result<Pmml, AlgoError> {
    let id_at = field_position(table, &settings.id_field)?;
    let time_at = field_position(table, &settings.time_field)?;
    let mut content_at: Vec<usize> = Vec::new();
    for name in &settings.content_fields {
        let at = field_position(table, name)?;
        if !content_at.contains(&at) {
            content_at.push(at);
        }
    }
    let mut dictionary: Vec<DataField> = Vec::new();
    for at in std::iter::once(&id_at)
        .chain(std::iter::once(&time_at))
        .chain(content_at.iter())
    {
        if let Some(field) = data_field(table, *at)
            && !dictionary.iter().any(|seen| seen.name == field.name)
        {
            dictionary.push(field);
        }
    }
    // Modeler's `DataDictionary` lists the fields by name (`sequence_default`: customer, item,
    // time), not in mining-schema order.
    dictionary.sort_by(|left, right| left.name.cmp(&right.name));
    let mut schema = Vec::new();
    schema.push(usage_field(table, id_at, Usage::Group)?);
    schema.push(usage_field(table, time_at, Usage::Other)?);
    for &at in &content_at {
        schema.push(usage_field(table, at, Usage::Active)?);
    }
    let model_rules = rules
        .iter()
        .map(|rule| SequenceRule {
            antecedent: resolve(data, &rule.antecedent),
            consequent: resolve(data, std::slice::from_ref(&rule.consequent)),
            confidence: rule.confidence,
            support: ratio(rule.antecedent_count, data.n),
            occurrence: rule.occurrence,
        })
        .collect();
    Ok(Pmml {
        version: "4.2".into(),
        data_dictionary: dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Sequence(SequenceModel {
            header: ModelHeader {
                name: (!settings.model_name.is_empty()).then(|| settings.model_name.clone()),
                function: MiningFunction::Other("sequences".into()),
                algorithm: Some("Sequence".into()),
                mining_schema: schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            id_field: settings.id_field.clone(),
            time_field: Some(settings.time_field.clone()),
            content_fields: settings.content_fields.clone(),
            number_of_transactions: data.n,
            rules: model_rules,
        }),
    })
}

/// Itemsets of item indices as `(field, value)` itemsets.
fn resolve(data: &Data, sets: &[Vec<u32>]) -> Vec<SequenceItemset> {
    sets.iter()
        .map(|set| {
            set.iter()
                .filter_map(|&item| data.values.get(usize::try_from(item).unwrap_or(usize::MAX)))
                .cloned()
                .collect()
        })
        .collect()
}

fn usage_field(table: &Table, at: usize, usage: Usage) -> Result<MiningField, AlgoError> {
    let field = table
        .model()
        .fields()
        .get(at)
        .ok_or_else(|| AlgoError::Settings("Sequence field is out of range".into()))?;
    Ok(MiningField {
        name: field.name.clone(),
        usage,
        missing_replacement: None,
        outliers: Outliers::AsIs,
        low_value: None,
        high_value: None,
        invalid_treatment: InvalidTreatment::AsIs,
        direction: None,
    })
}

/// `common::mining_field`, for the positions this module resolves itself.
fn field_position(table: &Table, name: &str) -> Result<usize, AlgoError> {
    table
        .model()
        .position(name)
        .ok_or_else(|| AlgoError::Settings(format!("Sequence needs field {name:?}")))
}

/// `percent / 100` as a basis of one millionth of a percent.
fn percent_basis(percent: f64) -> Result<u128, AlgoError> {
    let scaled = (percent * PERCENT_SCALE as f64).round();
    if !scaled.is_finite() || !(0.0..=100.0 * PERCENT_SCALE as f64).contains(&scaled) {
        return Err(AlgoError::Settings(
            "Sequence percent is out of range".into(),
        ));
    }
    Ok(scaled as u128)
}

/// `count / total >= basis / 1e8`.
fn at_least(count: u64, basis: u128, total: u64) -> bool {
    (count as u128).saturating_mul(PERCENT_CROSS) >= basis.saturating_mul(total as u128)
}

/// `numer / denom >= basis / 1e8`.
fn at_least_ratio(numer: u64, denom: u64, basis: u128) -> bool {
    if denom == 0 {
        return false;
    }
    (numer as u128).saturating_mul(PERCENT_CROSS) >= basis.saturating_mul(denom as u128)
}

fn ratio(numer: u64, denom: u64) -> f64 {
    if denom == 0 {
        0.0
    } else {
        numer as f64 / denom as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Dataset;
    use crate::common::{data_dictionary, mining_schema};
    use dms_types::{DataModel, Field, Measurement, Role, Storage};

    fn table(rows: Vec<Vec<Value>>) -> Table {
        let model = DataModel::new(vec![
            Field::new("customer", Storage::Integer),
            Field::new("time", Storage::Integer),
            Field::new("item", Storage::String),
        ])
        .unwrap();
        Table::new(model, rows).unwrap()
    }

    fn settings() -> SequenceSettings {
        SequenceSettings {
            id_field: "customer".into(),
            time_field: "time".into(),
            content_fields: vec!["item".into()],
            model_name: "train_next".into(),
            ..SequenceSettings::default()
        }
    }

    fn rows(spec: &[(i64, i64, &str)]) -> Vec<Vec<Value>> {
        spec.iter()
            .map(|(customer, time, item)| {
                vec![
                    Value::Integer(*customer),
                    Value::Integer(*time),
                    Value::string(*item),
                ]
            })
            .collect()
    }

    /// The guide's six gourmet customers, one item per time.
    fn gourmet() -> Vec<Vec<Value>> {
        rows(&[
            (1, 1, "cheese"),
            (1, 1, "crackers"),
            (1, 2, "wine"),
            (1, 3, "beer"),
            (2, 1, "wine"),
            (2, 2, "beer"),
            (2, 3, "cheese"),
            (3, 1, "bread"),
            (3, 2, "wine"),
            (3, 3, "cheese"),
            (3, 3, "beer"),
            (4, 1, "crackers"),
            (4, 2, "wine"),
            (4, 3, "beer"),
            (4, 4, "cheese"),
            (5, 1, "beer"),
            (5, 2, "cheese"),
            (5, 2, "crackers"),
            (5, 3, "bread"),
            (6, 1, "crackers"),
            (6, 2, "bread"),
        ])
    }

    fn model(table: &Table, settings: &SequenceSettings) -> SequenceModel {
        match train(table, settings).unwrap().model {
            Model::Sequence(model) => model,
            other => panic!("not a sequence model: {other:?}"),
        }
    }

    #[test]
    fn the_guides_gourmet_supports_are_reproduced() {
        let data = Data::build(&table(gourmet()), &settings()).unwrap();
        assert_eq!(data.n, 6);
        let support = |sequence: &[Vec<&str>]| -> f64 {
            let sets: Vec<Vec<u32>> = sequence
                .iter()
                .map(|set| {
                    set.iter()
                        .filter_map(|value| {
                            data.values
                                .iter()
                                .position(|(_, v)| v == *value)
                                .map(|i| u32::try_from(i).unwrap())
                        })
                        .collect()
                })
                .collect();
            support_of(&data, &sets) as f64 / 6.0
        };
        // Algorithms Guide, table 32-2.
        for (sequence, want) in [
            (vec![vec!["crackers"], vec!["cheese"]], 0.17),
            (vec![vec!["wine"], vec!["beer"]], 0.67),
            (vec![vec!["cheese"], vec!["wine"]], 0.17),
            (vec![vec!["bread"], vec!["wine"]], 0.17),
            (vec![vec!["cheese", "crackers"]], 0.33),
            (vec![vec!["cheese", "beer"]], 0.17),
            (vec![vec!["crackers"], vec!["wine"], vec!["beer"]], 0.33),
            (vec![vec!["wine"], vec!["beer"], vec!["cheese"]], 0.33),
            (
                vec![vec!["crackers"], vec!["wine"], vec!["beer"], vec!["cheese"]],
                0.17,
            ),
        ] {
            let got = support(&sequence);
            assert!((got - want).abs() < 0.01, "{sequence:?}: {got} vs {want}");
        }
    }

    #[test]
    fn rules_are_the_last_itemset_of_every_frequent_sequence() {
        // zeta > amber > mint > bravo in three of four transactions, amber > mint elsewhere.
        let table = table(rows(&[
            (1, 1, "zeta"),
            (1, 2, "amber"),
            (1, 3, "mint"),
            (1, 4, "bravo"),
            (2, 1, "zeta"),
            (2, 2, "amber"),
            (2, 3, "mint"),
            (2, 4, "bravo"),
            (3, 1, "zeta"),
            (3, 2, "amber"),
            (3, 3, "mint"),
            (3, 4, "bravo"),
            (4, 1, "amber"),
            (4, 2, "mint"),
        ]));
        let model = model(&table, &settings());
        let text = |rule: &SequenceRule| {
            format!(
                "{} => {} ({})",
                rule.antecedent
                    .iter()
                    .map(|set| set
                        .iter()
                        .map(|(_, value)| value.as_str())
                        .collect::<Vec<_>>()
                        .join("&"))
                    .collect::<Vec<_>>()
                    .join(" > "),
                rule.consequent[0][0].1.as_str(),
                rule.confidence
            )
        };
        let printed: Vec<String> = model.rules.iter().map(text).collect();
        // Every sequence of at least two itemsets yields a rule; confidence descending, then
        // antecedent length ascending, then antecedent support descending.
        assert_eq!(
            printed,
            vec![
                "amber => mint (1)".to_string(),
                "zeta => amber (1)".to_string(),
                "zeta => mint (1)".to_string(),
                "zeta => bravo (1)".to_string(),
                "zeta > amber => mint (1)".to_string(),
                "zeta > amber => bravo (1)".to_string(),
                "zeta > mint => bravo (1)".to_string(),
                "zeta > amber > mint => bravo (1)".to_string(),
                "amber => bravo (0.75)".to_string(),
                "mint => bravo (0.75)".to_string(),
                "amber > mint => bravo (0.75)".to_string(),
            ]
        );
        assert_eq!(model.number_of_transactions, 4);
    }

    #[test]
    fn support_is_the_antecedents_share_and_occurrence_the_instance_count() {
        let table = table(rows(&[
            (1, 1, "zeta"),
            (1, 2, "amber"),
            (1, 3, "mint"),
            (2, 1, "zeta"),
            (2, 2, "amber"),
            (3, 1, "zeta"),
            (3, 2, "copper"),
            (4, 1, "amber"),
            (4, 2, "mint"),
        ]));
        let model = model(&table, &settings());
        let rule = model
            .rules
            .iter()
            .find(|rule| rule.consequent[0][0].1 == "amber")
            .unwrap();
        assert_eq!(rule.antecedent.len(), 1);
        assert!((rule.confidence - 2.0 / 3.0).abs() < 1e-12);
        // `support` is the antecedent's share (3 of 4 transactions hold zeta).
        assert!((rule.support - 0.75).abs() < 1e-12);
        assert_eq!(rule.occurrence, 2);
    }

    #[test]
    fn max_size_caps_the_number_of_items() {
        let table = table(rows(&[
            (1, 1, "zeta"),
            (1, 2, "amber"),
            (1, 3, "mint"),
            (1, 4, "bravo"),
            (2, 1, "zeta"),
            (2, 2, "amber"),
            (2, 3, "mint"),
            (2, 4, "bravo"),
        ]));
        let mut settings = settings();
        settings.max_size = 2;
        let model = model(&table, &settings);
        assert!(
            model
                .rules
                .iter()
                .all(|rule| rule.antecedent.iter().flatten().count()
                    + rule.consequent.iter().flatten().count()
                    <= 2)
        );
        // Six two-item sequences: zeta and the three items after it, amber and the two after it,
        // and mint > bravo. With two items neither a larger antecedent nor a larger consequent
        // fits, so these are all the rules `SequenceMaxSize` 2 can keep.
        let text = |rule: &SequenceRule| rule.consequent[0][0].1.clone();
        assert_eq!(
            model.rules.iter().map(text).collect::<Vec<_>>(),
            vec!["amber", "mint", "bravo", "mint", "bravo", "bravo"]
        );
    }

    #[test]
    fn same_time_items_share_one_itemset() {
        // copper and violet at one time: {copper} alone never occurs, so no copper rule.
        let table = table(rows(&[
            (1, 1, "zeta"),
            (1, 2, "copper"),
            (1, 2, "violet"),
            (2, 1, "zeta"),
            (2, 2, "copper"),
            (2, 2, "violet"),
            (3, 1, "zeta"),
            (3, 2, "amber"),
        ]));
        let model = model(&table, &settings());
        assert!(
            model
                .rules
                .iter()
                .all(|rule| rule.consequent[0][0].1 != "violet" || rule.antecedent.len() == 1)
        );
        // zeta => violet fires twice, over the three transactions holding zeta.
        let rule = model
            .rules
            .iter()
            .find(|rule| rule.consequent[0][0].1 == "violet")
            .unwrap();
        assert_eq!(rule.occurrence, 2);
        assert!((rule.confidence - 2.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn blanks_are_ignored() {
        let table = table(rows(&[
            (1, 1, "zeta"),
            (1, 2, "amber"),
            (2, 1, "zeta"),
            (3, 1, "amber"),
            (3, 2, "mint"),
        ]));
        let model = model(&table, &settings());
        // The blank item at (2,2) contributes nothing, so zeta => amber occurs once.
        let rule = model
            .rules
            .iter()
            .find(|rule| rule.consequent[0][0].1 == "amber")
            .unwrap();
        assert_eq!(rule.occurrence, 1);
    }

    #[test]
    fn non_contiguous_ids_are_rejected() {
        let table = table(rows(&[(1, 1, "zeta"), (2, 1, "amber"), (1, 2, "mint")]));
        let error = train(&table, &settings()).unwrap_err();
        assert!(error.to_string().contains("not contiguous"), "{error}");
    }

    #[test]
    fn a_null_id_is_rejected() {
        let table = table(rows(&[(1, 1, "zeta")]));
        let mut rows = table.rows().to_vec();
        rows.push(vec![Value::Null, Value::Integer(2), Value::string("amber")]);
        let model = DataModel::new(vec![
            Field::new("customer", Storage::Integer),
            Field::new("time", Storage::Integer),
            Field::new("item", Storage::String),
        ])
        .unwrap();
        let error = train(&Table::new(model, rows).unwrap(), &settings()).unwrap_err();
        assert!(error.to_string().contains("null"), "{error}");
    }

    #[test]
    fn settings_without_a_content_field_are_rejected() {
        let mut settings = settings();
        settings.content_fields = Vec::new();
        let error = train(&table(rows(&[(1, 1, "zeta")])), &settings).unwrap_err();
        assert!(
            error.to_string().contains("SequenceContentField"),
            "{error}"
        );
    }

    #[test]
    fn the_dictionary_and_schema_name_the_three_kinds_of_field() {
        let table = table(rows(&[(1, 1, "zeta"), (1, 2, "amber")]));
        let pmml = train(&table, &settings()).unwrap();
        let names: Vec<&str> = pmml
            .data_dictionary
            .iter()
            .map(|field| field.name.as_str())
            .collect();
        assert_eq!(names, vec!["customer", "item", "time"]);
        let Model::Sequence(model) = &pmml.model else {
            panic!("not a sequence model");
        };
        assert_eq!(model.id_field, "customer");
        assert_eq!(model.time_field.as_deref(), Some("time"));
        assert_eq!(model.content_fields, vec!["item".to_string()]);
        let usages: Vec<Usage> = model
            .header
            .mining_schema
            .iter()
            .map(|field| field.usage)
            .collect();
        assert_eq!(usages, vec![Usage::Group, Usage::Other, Usage::Active]);
        // The shared helpers stay consistent with the schema this module writes.
        let dataset = Dataset {
            table: &table,
            target: 0,
            inputs: &[1, 2],
            frequency: None,
        };
        assert_eq!(data_dictionary(&dataset).len(), 3);
        assert_eq!(mining_schema(&dataset).len(), 3);
    }

    #[test]
    fn the_mining_schema_field_of_a_table_round_trips() {
        // `common::mining_field` is the reference for the fields this module writes itself.
        let table = table(rows(&[(1, 1, "zeta")]));
        let field = crate::common::mining_field(&table, 2, Usage::Active).unwrap();
        assert_eq!(field.name, "item");
        assert_eq!(field.usage, Usage::Active);
    }

    #[test]
    fn a_field_role_does_not_change_the_mining() {
        let mut fields = vec![
            Field::new("customer", Storage::Integer),
            Field::new("time", Storage::Integer),
            Field::new("item", Storage::String),
        ];
        fields[2].role = Role::Input;
        fields[2].measurement = Measurement::Nominal;
        let table = Table::new(
            DataModel::new(fields).unwrap(),
            rows(&[(1, 1, "zeta"), (1, 2, "amber")]),
        )
        .unwrap();
        let trained = model_of(&table);
        assert_eq!(trained.rules.len(), 1);
        assert_eq!(trained.rules[0].consequent[0][0].1, "amber");
    }

    fn model_of(table: &Table) -> SequenceModel {
        model(table, &settings())
    }
}
