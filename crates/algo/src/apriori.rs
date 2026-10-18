//! Apriori association rules (Algorithms Guide, Apriori chapter).
//!
//! Itemset support is the count divided by the number of transactions. Rule support is the
//! support of the antecedent union the consequent. Confidence is that union divided by the
//! antecedent count. Lift is confidence divided by the consequent's support.
//!
//! `minRuleSupport` is the antecedent's support, in percent (`apriori_default` 10, so 2 of 16;
//! `apriori_thresholds` 30, so 5 of 16). A rule whose union is below that support is still kept
//! when the antecedent passes and the confidence passes (`apriori_thresholds` rule 7: union
//! 0.25, antecedent support 0.3125, confidence 0.8). Itemsets are therefore mined at
//! support × confidence, which is low enough to contain every such union and high enough that
//! a passing rule cannot hide below it.
//!
//! Counts use integer cross-multiplication (`count * scale >= percent * n`), so 10, 30, 70 and
//! 80 percent of 16 do not depend on binary fractions.
//!
//! Generation order (`apriori_default` rules 1–5 and `apriori_thresholds` rules 1–7): items are
//! ranked by ascending support, then by their original index, then by name. Itemsets are grown
//! level by level in lexicographic order of that rank. For each itemset of size at least 2,
//! each member is tried as the single consequent, most frequent first. The antecedent must
//! meet the minimum support, fit in `maxRuleAntecedents`, and every antecedent item must be an
//! allowed antecedent; the consequent must be an allowed consequent; confidence must pass.
//!
//! Item and itemset ids are assigned when a rule is emitted, not by frequency. Antecedent items
//! are mentioned from rarest to most frequent, then the consequent. The first mention of an
//! item takes the next item id. The first mention of an antecedent itemset or a consequent
//! singleton takes the next itemset id (`apriori_default`: eggs=1 … bread=5, itemsets 1–8;
//! `apriori_thresholds`: the same item ids, itemsets 1–6). `ItemRef`s are stored in ascending
//! item-id order.
//!
//! `minimumSupport` is the smallest support of an emitted itemset (0.125 and 0.3125).
//! `minimumConfidence` is the smallest emitted confidence. The model attribute Modeler writes
//! for 5/7 is truncated; the rule attribute is the full ratio (`apriori_thresholds` rule 4).
//!
//! Transactional input (`apriori_transactional`) groups contiguous rows of the same id. The
//! content value is the item. Empty antecedent and consequent lists mean every content value.
//! The resulting rules match `apriori_default`.
//!
//! GAP: equal support, so the original-index tie break, does not occur in these baskets.
//! GAP: two new items in one antecedent, so mention order among new items, does not occur.
//! GAP: `onlyTrueValues` false, rules with an empty antecedent, a non-flag field, evaluation
//! measures other than rule confidence, and non-contiguous ids are rejected.
//! GAP: antecedent and consequent sets that differ are rejected (no oracle for the direction
//! extension or for which items are mined).
//! GAP: `Item` / `Itemset` element order in Modeler's file is not id order and is not
//! reproduced. Comparison is by id.

use crate::AlgoError;
use crate::common::{data_field, mining_field};
use dms_pmml::{
    AssociationItem, AssociationItemset, AssociationModel, AssociationRule, MiningFunction, Model,
    ModelHeader, Pmml, Usage,
};
use dms_types::{Role, Table, Value, ValueSpec};
use std::collections::{HashMap, HashSet};

const MAX_TRANSACTIONS: usize = 5_000_000;
const MAX_ITEMS: usize = 10_000;
const MAX_ITEMSETS: usize = 500_000;
const MAX_RULES: usize = 500_000;
/// Percent scaled by one million. 10% is 10_000_000. Integers up to 100 × this fit in `f64`.
const PERCENT_SCALE: u128 = 1_000_000;

/// Settings of a `BuildAprioriNode`. Defaults match the property descriptors in
/// `apriori_default`.
#[derive(Debug, Clone, PartialEq)]
pub struct AprioriSettings {
    /// `minRuleSupport`, percent of transactions the antecedent must cover. Default 10.
    pub min_support_percent: f64,
    /// `minRuleConfidence`, percent. Default 80. Minimum stored by Modeler is 0.0001.
    pub min_confidence_percent: f64,
    /// `maxRuleAntecedents` (1..=32, default 5).
    pub max_antecedents: u32,
    /// `onlyTrueValues`. False is rejected for tabular data.
    pub only_true_values: bool,
    /// `transactionalData`.
    pub transactional: bool,
    /// `contiguousID`. Required when transactional; ignored otherwise (`apriori_default` is false).
    pub contiguous: bool,
    /// `idField`.
    pub id_field: Option<String>,
    /// `contentField`.
    pub content_field: Option<String>,
    /// `autoModelFieldSelection`. Transactional data then uses every content value.
    pub auto_fields: bool,
    /// `modelInputFieldSelection`, in listed order. Empty with transactional data: every value.
    pub antecedents: Vec<String>,
    /// `modelTargetFieldSelection`. Must name the same set as `antecedents` when either is set.
    pub consequents: Vec<String>,
    /// `modelName` (`models_next` in the three oracles).
    pub model_name: String,
}

impl Default for AprioriSettings {
    fn default() -> Self {
        Self {
            min_support_percent: 10.0,
            min_confidence_percent: 80.0,
            max_antecedents: 5,
            only_true_values: true,
            transactional: false,
            contiguous: false,
            id_field: None,
            content_field: None,
            auto_fields: true,
            antecedents: Vec::new(),
            consequents: Vec::new(),
            model_name: String::new(),
        }
    }
}

#[derive(Debug)]
struct FlagColumn {
    position: usize,
    name: String,
    true_value: Value,
    true_text: String,
}

#[derive(Debug)]
enum Columns {
    Flags(Vec<FlagColumn>),
    Transactions { id: usize, content: usize },
}

pub(crate) struct Prepared {
    pub(crate) transactions: Vec<Vec<String>>,
    /// Tie-break order: field order, or declared categories then first seen.
    pub(crate) order: Vec<String>,
    columns: Columns,
    antecedent: HashSet<String>,
    consequent: HashSet<String>,
}

struct Node {
    items: Vec<u16>,
    tids: Vec<u32>,
}

/// A rule over item ranks (indices into the mined names): antecedent and consequent items in
/// the order their ids are assigned.
pub(crate) struct Mined {
    pub(crate) antecedent: Vec<u16>,
    pub(crate) consequent: Vec<u16>,
    pub(crate) union_count: u64,
    pub(crate) ant_count: u64,
    pub(crate) cons_count: u64,
}

struct Assigned {
    items: Vec<AssociationItem>,
    itemsets: Vec<(u32, Vec<u32>, u64)>,
    rules: Vec<(u32, u32, u64, u64, u64)>,
}

/// Trains an Apriori model. `table` is already restricted to the training partition.
pub fn train(table: &Table, settings: &AprioriSettings) -> Result<Pmml, AlgoError> {
    validate(settings)?;
    let prepared = prepare(table, settings)?;
    if prepared.transactions.is_empty() {
        return Err(AlgoError::Data("Apriori has no transactions".into()));
    }
    if prepared.transactions.len() > MAX_TRANSACTIONS {
        return Err(AlgoError::Data(format!(
            "Apriori has {} transactions; the limit is {MAX_TRANSACTIONS}",
            prepared.transactions.len()
        )));
    }
    let mined = mine(&prepared, settings)?;
    assemble(table, settings, &prepared, &mined)
}

fn validate(settings: &AprioriSettings) -> Result<(), AlgoError> {
    if !settings.min_support_percent.is_finite()
        || !(0.0..=100.0).contains(&settings.min_support_percent)
    {
        return Err(AlgoError::Settings(
            "Apriori minRuleSupport must be from 0 to 100".into(),
        ));
    }
    if !settings.min_confidence_percent.is_finite()
        || !(1.0e-4..=100.0).contains(&settings.min_confidence_percent)
    {
        return Err(AlgoError::Settings(
            "Apriori minRuleConfidence must be from 0.0001 to 100".into(),
        ));
    }
    if !(1..=32).contains(&settings.max_antecedents) {
        return Err(AlgoError::Settings(format!(
            "Apriori maxRuleAntecedents must be from 1 to 32, found {}",
            settings.max_antecedents
        )));
    }
    if !settings.transactional && !settings.only_true_values {
        return Err(AlgoError::Unsupported(
            "Apriori onlyTrueValues false (GAP: no oracle for a false flag as its own item)".into(),
        ));
    }
    if settings.transactional && !settings.contiguous {
        return Err(AlgoError::Unsupported(
            "Apriori transactional data with contiguousID false (GAP: no oracle for non-contiguous ids)".into(),
        ));
    }
    Ok(())
}

fn prepare(table: &Table, settings: &AprioriSettings) -> Result<Prepared, AlgoError> {
    if settings.transactional {
        prepare_transactions(table, settings)
    } else {
        prepare_flags(table, settings)
    }
}

pub(crate) fn prepare_flags(
    table: &Table,
    settings: &AprioriSettings,
) -> Result<Prepared, AlgoError> {
    let (antecedents, consequents) = if settings.auto_fields {
        (
            role_names(table, |role| matches!(role, Role::Input | Role::Both)),
            role_names(table, |role| matches!(role, Role::Target | Role::Both)),
        )
    } else {
        (unique(&settings.antecedents), unique(&settings.consequents))
    };
    if antecedents.is_empty() || !same_names(&antecedents, &consequents) {
        return Err(AlgoError::Unsupported(
            "Apriori antecedent and consequent sets differ or are empty (GAP: no oracle for a direction other than BOTH)".into(),
        ));
    }
    let mut columns = Vec::new();
    for name in &antecedents {
        let position = field_position(table, name)?;
        let field = table
            .model()
            .fields()
            .get(position)
            .ok_or_else(|| AlgoError::Settings(format!("Apriori field {name:?} is missing")))?;
        let ValueSpec::Flag { true_value, .. } = &field.values else {
            return Err(AlgoError::Unsupported(format!(
                "Apriori field {name:?} is not a flag (GAP: no oracle for nominal items)"
            )));
        };
        columns.push(FlagColumn {
            position,
            name: name.clone(),
            true_value: true_value.clone(),
            true_text: true_value.to_string(),
        });
    }
    let mut transactions = Vec::with_capacity(table.rows().len());
    for row in table.rows() {
        let mut basket = Vec::new();
        for column in &columns {
            if row.get(column.position) == Some(&column.true_value) {
                basket.push(column.name.clone());
            }
        }
        transactions.push(basket);
    }
    let allowed = antecedents.iter().cloned().collect();
    Ok(Prepared {
        transactions,
        order: antecedents,
        columns: Columns::Flags(columns),
        antecedent: allowed,
        consequent: consequents.into_iter().collect(),
    })
}

fn prepare_transactions(table: &Table, settings: &AprioriSettings) -> Result<Prepared, AlgoError> {
    let id_name = settings
        .id_field
        .as_deref()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| AlgoError::Settings("Apriori transactional data needs idField".into()))?;
    let content_name = settings
        .content_field
        .as_deref()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            AlgoError::Settings("Apriori transactional data needs contentField".into())
        })?;
    if id_name == content_name {
        return Err(AlgoError::Settings(
            "Apriori idField and contentField are the same field".into(),
        ));
    }
    let id_at = field_position(table, id_name)?;
    let content_at = field_position(table, content_name)?;
    let (antecedents, consequents) = if settings.auto_fields {
        (Vec::new(), Vec::new())
    } else {
        (unique(&settings.antecedents), unique(&settings.consequents))
    };
    if antecedents.is_empty() != consequents.is_empty() || !same_names(&antecedents, &consequents) {
        return Err(AlgoError::Unsupported(
            "Apriori transactional antecedent and consequent lists differ (GAP: no oracle)".into(),
        ));
    }
    let restrict: HashSet<String> = antecedents.iter().cloned().collect();
    let declared = match table
        .model()
        .fields()
        .get(content_at)
        .map(|field| &field.values)
    {
        Some(ValueSpec::Categories(values)) => values.iter().map(ToString::to_string).collect(),
        _ => Vec::new(),
    };
    let mut order = declared;
    let mut transactions = Vec::new();
    let mut basket = Vec::new();
    let mut seen = HashSet::new();
    let mut current: Option<Value> = None;
    for (index, row) in table.rows().iter().enumerate() {
        let id = row.get(id_at).cloned().unwrap_or(Value::Null);
        if id == Value::Null {
            return Err(AlgoError::Data(format!(
                "Apriori transaction id is null at row {}",
                index + 1
            )));
        }
        if current.as_ref() != Some(&id) {
            if current.is_some() {
                transactions.push(std::mem::take(&mut basket));
                seen.clear();
            }
            current = Some(id);
        }
        let content = row.get(content_at).cloned().unwrap_or(Value::Null);
        if content == Value::Null {
            continue;
        }
        let text = content.to_string();
        if !restrict.is_empty() && !restrict.contains(&text) {
            continue;
        }
        if !order.iter().any(|item| item == &text) {
            order.push(text.clone());
        }
        if seen.insert(text.clone()) {
            basket.push(text);
        }
    }
    if current.is_some() {
        transactions.push(basket);
    }
    Ok(Prepared {
        transactions,
        order,
        columns: Columns::Transactions {
            id: id_at,
            content: content_at,
        },
        antecedent: restrict.clone(),
        consequent: if consequents.is_empty() {
            HashSet::new()
        } else {
            consequents.into_iter().collect()
        },
    })
}

fn mine(
    prepared: &Prepared,
    settings: &AprioriSettings,
) -> Result<(Vec<String>, Vec<Mined>, u64), AlgoError> {
    let n = u64::try_from(prepared.transactions.len())
        .map_err(|_| AlgoError::Data("too many transactions".into()))?;
    let supp = percent_basis(settings.min_support_percent)?;
    let conf = percent_basis(settings.min_confidence_percent)?;
    let mut counts: HashMap<String, u64> = HashMap::new();
    for basket in &prepared.transactions {
        for item in basket {
            let count = counts.entry(item.clone()).or_insert(0);
            *count = count.saturating_add(1);
        }
    }
    if counts.len() > MAX_ITEMS {
        return Err(AlgoError::Data(format!(
            "Apriori has {} items; the limit is {MAX_ITEMS}",
            counts.len()
        )));
    }
    let mut ranked: Vec<(String, u64, usize)> = counts
        .into_iter()
        .filter(|(_, count)| *count > 0 && floor_ok(*count, supp, conf, n))
        .map(|(name, count)| {
            let index = prepared
                .order
                .iter()
                .position(|item| item == &name)
                .unwrap_or(usize::MAX);
            (name, count, index)
        })
        .collect();
    ranked.sort_by(|left, right| {
        left.1
            .cmp(&right.1)
            .then(left.2.cmp(&right.2))
            .then(left.0.cmp(&right.0))
    });
    let names: Vec<String> = ranked.iter().map(|(name, _, _)| name.clone()).collect();
    let rank_of: HashMap<&str, u16> = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let rank = u16::try_from(index).unwrap_or(u16::MAX);
            (name.as_str(), rank)
        })
        .collect();
    let mut baskets = Vec::with_capacity(prepared.transactions.len());
    for basket in &prepared.transactions {
        let mut ranks = Vec::new();
        for name in basket {
            if let Some(rank) = rank_of.get(name.as_str()) {
                ranks.push(*rank);
            }
        }
        ranks.sort_unstable();
        ranks.dedup();
        baskets.push(ranks);
    }
    let mut tids: Vec<Vec<u32>> = vec![Vec::new(); names.len()];
    for (row, basket) in baskets.iter().enumerate() {
        let Some(row) = u32::try_from(row).ok() else {
            return Err(AlgoError::Data("too many transactions".into()));
        };
        for &rank in basket {
            if let Some(list) = tids.get_mut(usize::from(rank)) {
                list.push(row);
            }
        }
    }
    let mut counts_of: HashMap<Vec<u16>, u64> = HashMap::new();
    let mut frequent: HashSet<Vec<u16>> = HashSet::new();
    let mut level = Vec::new();
    for (rank, list) in tids.into_iter().enumerate() {
        let Some(rank) = u16::try_from(rank).ok() else {
            return Err(AlgoError::Data("too many items".into()));
        };
        let key = vec![rank];
        let count = u64::try_from(list.len()).unwrap_or(u64::MAX);
        counts_of.insert(key.clone(), count);
        frequent.insert(key.clone());
        level.push(Node {
            items: key,
            tids: list,
        });
    }
    let max_size = usize::try_from(settings.max_antecedents)
        .unwrap_or(32)
        .saturating_add(1);
    let mut levels = vec![level];
    while levels.len() < max_size {
        let next = join(levels.last(), &frequent, supp, conf, n, &mut counts_of)?;
        if next.is_empty() {
            break;
        }
        for node in &next {
            frequent.insert(node.items.clone());
        }
        levels.push(next);
    }
    let mut rules = Vec::new();
    for level in levels.iter().skip(1) {
        for node in level {
            // Most frequent consequent first. Rank increases with support, and `items` is
            // sorted by rank, so a higher index is tried sooner.
            let mut order: Vec<usize> = (0..node.items.len()).collect();
            order.sort_by(|&left, &right| node.items.get(right).cmp(&node.items.get(left)));
            for at in order {
                let Some(&cons) = node.items.get(at) else {
                    continue;
                };
                let antecedent: Vec<u16> = node
                    .items
                    .iter()
                    .copied()
                    .filter(|item| *item != cons)
                    .collect();
                let Some(&ant_count) = counts_of.get(&antecedent) else {
                    continue;
                };
                let Some(&union_count) = counts_of.get(&node.items) else {
                    continue;
                };
                if ant_count == 0
                    || !at_least(ant_count, supp, n)
                    || !at_least_ratio(union_count, ant_count, conf)
                {
                    continue;
                }
                if !antecedent
                    .iter()
                    .all(|rank| allowed(&prepared.antecedent, &names, *rank))
                {
                    continue;
                }
                if !allowed(&prepared.consequent, &names, cons) {
                    continue;
                }
                let cons_count = counts_of.get(&vec![cons]).copied().unwrap_or(0);
                if cons_count == 0 {
                    continue;
                }
                if rules.len() >= MAX_RULES {
                    return Err(AlgoError::Data(format!(
                        "Apriori found more than {MAX_RULES} rules"
                    )));
                }
                rules.push(Mined {
                    antecedent,
                    consequent: vec![cons],
                    union_count,
                    ant_count,
                    cons_count,
                });
            }
        }
    }
    Ok((names, rules, n))
}

fn join(
    previous: Option<&Vec<Node>>,
    frequent: &HashSet<Vec<u16>>,
    supp: u128,
    conf: u128,
    n: u64,
    counts_of: &mut HashMap<Vec<u16>, u64>,
) -> Result<Vec<Node>, AlgoError> {
    let Some(previous) = previous else {
        return Ok(Vec::new());
    };
    let Some(width) = previous.first().map(|node| node.items.len()) else {
        return Ok(Vec::new());
    };
    let prefix = width.saturating_sub(1);
    let mut next = Vec::new();
    for i in 0..previous.len() {
        for j in (i + 1)..previous.len() {
            let Some(left) = previous.get(i) else {
                continue;
            };
            let Some(right) = previous.get(j) else {
                continue;
            };
            if left.items.get(..prefix) != right.items.get(..prefix) {
                break;
            }
            let (Some(&left_last), Some(&added)) = (left.items.last(), right.items.last()) else {
                continue;
            };
            if left_last >= added {
                continue;
            }
            let mut items = left.items.clone();
            items.push(added);
            if !subsets_frequent(&items, frequent) {
                continue;
            }
            let tids = intersect(&left.tids, &right.tids);
            let count = u64::try_from(tids.len()).unwrap_or(u64::MAX);
            if !floor_ok(count, supp, conf, n) {
                continue;
            }
            if counts_of.len() >= MAX_ITEMSETS {
                return Err(AlgoError::Data(format!(
                    "Apriori kept more than {MAX_ITEMSETS} itemsets"
                )));
            }
            counts_of.insert(items.clone(), count);
            next.push(Node { items, tids });
        }
    }
    Ok(next)
}

fn subsets_frequent(items: &[u16], frequent: &HashSet<Vec<u16>>) -> bool {
    for skip in 0..items.len() {
        let mut subset = Vec::with_capacity(items.len().saturating_sub(1));
        for (index, item) in items.iter().enumerate() {
            if index != skip {
                subset.push(*item);
            }
        }
        if !frequent.contains(&subset) {
            return false;
        }
    }
    true
}

fn intersect(left: &[u32], right: &[u32]) -> Vec<u32> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < left.len() && j < right.len() {
        match left.get(i).cmp(&right.get(j)) {
            std::cmp::Ordering::Equal => {
                if let Some(value) = left.get(i) {
                    out.push(*value);
                }
                i = i.saturating_add(1);
                j = j.saturating_add(1);
            }
            std::cmp::Ordering::Less => i = i.saturating_add(1),
            std::cmp::Ordering::Greater => j = j.saturating_add(1),
        }
    }
    out
}

fn allowed(set: &HashSet<String>, names: &[String], rank: u16) -> bool {
    if set.is_empty() {
        return true;
    }
    names
        .get(usize::from(rank))
        .is_some_and(|name| set.contains(name))
}

fn assemble(
    table: &Table,
    settings: &AprioriSettings,
    prepared: &Prepared,
    mined: &(Vec<String>, Vec<Mined>, u64),
) -> Result<Pmml, AlgoError> {
    assemble_model(table, settings, prepared, mined, "Apriori")
}

/// The `AssociationModel` of mined rules, written as Modeler writes Apriori and CARMA models.
pub(crate) fn assemble_model(
    table: &Table,
    settings: &AprioriSettings,
    prepared: &Prepared,
    mined: &(Vec<String>, Vec<Mined>, u64),
    algorithm: &str,
) -> Result<Pmml, AlgoError> {
    let (names, rules, n) = mined;
    let assigned = assign_ids(names, rules)?;
    let min_support = assigned
        .itemsets
        .iter()
        .map(|(_, _, count)| ratio(*count, *n))
        .reduce(f64::min)
        .filter(|value| value.is_finite())
        .unwrap_or(settings.min_support_percent / 100.0);
    let confidences: Vec<f64> = assigned
        .rules
        .iter()
        .map(|(_, _, union_count, ant_count, _)| ratio(*union_count, *ant_count))
        .collect();
    let min_confidence = confidences
        .iter()
        .copied()
        .reduce(f64::min)
        .filter(|value| value.is_finite())
        .unwrap_or(settings.min_confidence_percent / 100.0);
    let model_rules = assigned
        .rules
        .iter()
        .enumerate()
        .map(
            |(index, (antecedent, consequent, union_count, ant_count, cons_count))| {
                AssociationRule {
                    id: (index + 1).to_string(),
                    antecedent: antecedent.to_string(),
                    consequent: consequent.to_string(),
                    support: ratio(*union_count, *n),
                    confidence: ratio(*union_count, *ant_count),
                    lift: lift(*union_count, *ant_count, *cons_count, *n),
                }
            },
        )
        .collect();
    let itemsets = assigned
        .itemsets
        .iter()
        .map(|(id, items, count)| AssociationItemset {
            id: id.to_string(),
            support: ratio(*count, *n),
            items: items.iter().map(u32::to_string).collect(),
        })
        .collect();
    let (dictionary, schema) = dictionary(table, prepared)?;
    let name = if settings.model_name.is_empty() {
        None
    } else {
        Some(settings.model_name.clone())
    };
    Ok(Pmml {
        version: "4.2".into(),
        data_dictionary: dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Association(AssociationModel {
            header: ModelHeader {
                name,
                function: MiningFunction::Other("associationRules".into()),
                algorithm: Some(algorithm.into()),
                mining_schema: schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            number_of_transactions: *n,
            minimum_support: min_support,
            minimum_confidence: min_confidence,
            items: assigned.items,
            itemsets,
            rules: model_rules,
        }),
    })
}

fn assign_ids(names: &[String], rules: &[Mined]) -> Result<Assigned, AlgoError> {
    let mut item_of: HashMap<String, u32> = HashMap::new();
    let mut items = Vec::new();
    let mut itemset_of: HashMap<Vec<u32>, (u32, u64)> = HashMap::new();
    let mut itemsets = Vec::new();
    let mut stored = Vec::new();
    let mut next_item: u32 = 0;
    let mut next_itemset: u32 = 0;
    for rule in rules {
        let mut antecedent_ids = Vec::new();
        for rank in &rule.antecedent {
            let name = names
                .get(usize::from(*rank))
                .ok_or_else(|| AlgoError::Data("Apriori antecedent rank is out of range".into()))?;
            antecedent_ids.push(item_id(&mut item_of, &mut items, &mut next_item, name)?);
        }
        let mut consequent_ids = Vec::new();
        for rank in &rule.consequent {
            let name = names
                .get(usize::from(*rank))
                .ok_or_else(|| AlgoError::Data("Apriori consequent rank is out of range".into()))?;
            consequent_ids.push(item_id(&mut item_of, &mut items, &mut next_item, name)?);
        }
        let antecedent = itemset_id(
            &mut itemset_of,
            &mut itemsets,
            &mut next_itemset,
            &antecedent_ids,
            rule.ant_count,
        )?;
        let consequent = itemset_id(
            &mut itemset_of,
            &mut itemsets,
            &mut next_itemset,
            &consequent_ids,
            rule.cons_count,
        )?;
        stored.push((
            antecedent,
            consequent,
            rule.union_count,
            rule.ant_count,
            rule.cons_count,
        ));
    }
    Ok(Assigned {
        items,
        itemsets,
        rules: stored,
    })
}

fn item_id(
    item_of: &mut HashMap<String, u32>,
    items: &mut Vec<AssociationItem>,
    next: &mut u32,
    name: &str,
) -> Result<u32, AlgoError> {
    if let Some(id) = item_of.get(name) {
        return Ok(*id);
    }
    *next = next
        .checked_add(1)
        .ok_or_else(|| AlgoError::Data("too many Apriori items".into()))?;
    item_of.insert(name.to_string(), *next);
    items.push(AssociationItem {
        id: next.to_string(),
        value: name.to_string(),
    });
    Ok(*next)
}

fn itemset_id(
    itemset_of: &mut HashMap<Vec<u32>, (u32, u64)>,
    itemsets: &mut Vec<(u32, Vec<u32>, u64)>,
    next: &mut u32,
    item_ids: &[u32],
    count: u64,
) -> Result<u32, AlgoError> {
    let mut key = item_ids.to_vec();
    key.sort_unstable();
    if let Some((id, _)) = itemset_of.get(&key) {
        return Ok(*id);
    }
    *next = next
        .checked_add(1)
        .ok_or_else(|| AlgoError::Data("too many Apriori itemsets".into()))?;
    itemset_of.insert(key.clone(), (*next, count));
    itemsets.push((*next, key, count));
    Ok(*next)
}

fn dictionary(
    table: &Table,
    prepared: &Prepared,
) -> Result<(Vec<dms_pmml::DataField>, Vec<dms_pmml::MiningField>), AlgoError> {
    let mut data = Vec::new();
    let mut schema = Vec::new();
    let mut push = |position: usize,
                    usage: Usage,
                    direction: Option<String>,
                    true_text: Option<String>|
     -> Result<(), AlgoError> {
        let mut field = data_field(table, position).ok_or_else(|| {
            AlgoError::Data("Apriori field disappeared while building the dictionary".into())
        })?;
        field.flag_true_value = true_text;
        data.push(field);
        let mut mining = mining_field(table, position, usage).ok_or_else(|| {
            AlgoError::Data("Apriori field disappeared while building the mining schema".into())
        })?;
        mining.direction = direction;
        schema.push(mining);
        Ok(())
    };
    match &prepared.columns {
        Columns::Flags(columns) => {
            for column in columns {
                push(
                    column.position,
                    Usage::Active,
                    Some("BOTH".into()),
                    Some(column.true_text.clone()),
                )?;
            }
        }
        Columns::Transactions { id, content } => {
            push(*id, Usage::Group, None, None)?;
            push(*content, Usage::Active, None, None)?;
        }
    }
    Ok((data, schema))
}

pub(crate) fn percent_basis(percent: f64) -> Result<u128, AlgoError> {
    let scaled = (percent * PERCENT_SCALE as f64).round();
    if !scaled.is_finite() || !(0.0..=100.0 * PERCENT_SCALE as f64).contains(&scaled) {
        return Err(AlgoError::Settings(
            "Apriori percent is out of range".into(),
        ));
    }
    Ok(scaled as u128)
}

/// `count / total >= basis / 1e8`, with `basis = round(percent * 1e6)`.
pub(crate) fn at_least(count: u64, basis: u128, total: u64) -> bool {
    (count as u128).saturating_mul(PERCENT_SCALE * 100) >= basis.saturating_mul(total as u128)
}

/// `numer / denom >= basis / 1e8`.
pub(crate) fn at_least_ratio(numer: u64, denom: u64, basis: u128) -> bool {
    if denom == 0 {
        return false;
    }
    (numer as u128).saturating_mul(PERCENT_SCALE * 100) >= basis.saturating_mul(denom as u128)
}

/// Union count is at least support × confidence of `n`.
fn floor_ok(count: u64, supp: u128, conf: u128, n: u64) -> bool {
    let Some(right) = supp
        .checked_mul(conf)
        .and_then(|value| value.checked_mul(n as u128))
    else {
        return false;
    };
    let Some(left) = (count as u128)
        .checked_mul(PERCENT_SCALE * 100)
        .and_then(|value| value.checked_mul(PERCENT_SCALE * 100))
    else {
        return false;
    };
    left >= right
}

fn ratio(numer: u64, denom: u64) -> f64 {
    if denom == 0 {
        0.0
    } else {
        numer as f64 / denom as f64
    }
}

fn lift(union_count: u64, antecedent: u64, consequent: u64, n: u64) -> f64 {
    let denominator = (antecedent as f64) * (consequent as f64);
    if denominator == 0.0 {
        0.0
    } else {
        (union_count as f64) * (n as f64) / denominator
    }
}

fn role_names(table: &Table, keep: impl Fn(Role) -> bool) -> Vec<String> {
    table
        .model()
        .fields()
        .iter()
        .filter(|field| keep(field.role))
        .map(|field| field.name.clone())
        .collect()
}

fn unique(names: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for name in names {
        if !name.is_empty() && !out.iter().any(|kept: &String| kept == name) {
            out.push(name.clone());
        }
    }
    out
}

fn same_names(left: &[String], right: &[String]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .all(|name| right.iter().any(|other| other == name))
}

fn field_position(table: &Table, name: &str) -> Result<usize, AlgoError> {
    table
        .model()
        .position(name)
        .ok_or_else(|| AlgoError::Settings(format!("Apriori field {name:?} is not in the data")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dms_pmml::score_rules;
    use dms_types::{DataModel, Field, Measurement, Storage};

    const ITEMS: [&str; 5] = ["bread", "milk", "eggs", "butter", "jam"];
    const BASKETS: [&[&str]; 16] = [
        &["bread", "milk"],
        &["bread", "milk", "eggs"],
        &["bread", "milk"],
        &["bread", "milk", "eggs"],
        &["bread", "milk", "butter"],
        &["bread", "butter"],
        &["bread", "butter", "jam"],
        &["bread", "butter", "jam"],
        &["butter", "jam"],
        &["butter", "jam"],
        &["bread", "butter", "jam"],
        &["milk", "eggs"],
        &["bread", "milk", "eggs"],
        &["bread", "jam"],
        &["bread", "milk", "butter", "jam"],
        &["eggs", "milk"],
    ];

    fn flag(name: &str) -> Field {
        let mut field = Field::new(name, Storage::String);
        field.measurement = Measurement::Flag;
        field.role = Role::Both;
        field.values = ValueSpec::Flag {
            true_value: Value::string("T"),
            false_value: Value::string("F"),
        };
        field
    }

    fn flags_table() -> Table {
        let fields: Vec<Field> = ITEMS.into_iter().map(flag).collect();
        let rows = BASKETS
            .into_iter()
            .map(|basket| {
                ITEMS
                    .into_iter()
                    .map(|item| {
                        if basket.contains(&item) {
                            Value::string("T")
                        } else {
                            Value::string("F")
                        }
                    })
                    .collect()
            })
            .collect();
        Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
    }

    fn trans_table() -> Table {
        let mut id = Field::new("basket", Storage::Integer);
        id.measurement = Measurement::Continuous;
        id.role = Role::RecordId;
        let mut item = Field::new("item", Storage::String);
        item.measurement = Measurement::Nominal;
        item.role = Role::Both;
        item.values = ValueSpec::Categories(ITEMS.into_iter().map(Value::string).collect());
        let mut rows = Vec::new();
        for (index, basket) in BASKETS.into_iter().enumerate() {
            for name in basket {
                rows.push(vec![
                    Value::Integer(i64::try_from(index + 1).unwrap()),
                    Value::string(*name),
                ]);
            }
        }
        Table::new(DataModel::new(vec![id, item]).unwrap(), rows).unwrap()
    }

    fn settings(support: f64, confidence: f64) -> AprioriSettings {
        AprioriSettings {
            min_support_percent: support,
            min_confidence_percent: confidence,
            auto_fields: false,
            antecedents: ITEMS.into_iter().map(str::to_string).collect(),
            consequents: ITEMS.into_iter().map(str::to_string).collect(),
            model_name: "models_next".into(),
            ..AprioriSettings::default()
        }
    }

    fn model_of(pmml: &Pmml) -> &AssociationModel {
        match &pmml.model {
            Model::Association(model) => model,
            other => panic!("not an association model: {other:?}"),
        }
    }

    fn sides(model: &AssociationModel, rule: &AssociationRule) -> (Vec<String>, Vec<String>) {
        let map = |id: &str| {
            model
                .itemset_values(id)
                .unwrap()
                .into_iter()
                .map(str::to_string)
                .collect()
        };
        (map(&rule.antecedent), map(&rule.consequent))
    }

    #[test]
    fn default_rules_match_modeler_ids() {
        let trained = train(&flags_table(), &settings(10.0, 80.0)).unwrap();
        let model = model_of(&trained);
        assert_eq!(model.number_of_transactions, 16);
        assert_eq!(model.minimum_support, 0.125);
        assert_eq!(model.minimum_confidence, 0.8);
        let id = |name: &str| {
            model
                .items
                .iter()
                .find(|item| item.value == name)
                .unwrap()
                .id
                .as_str()
        };
        assert_eq!(
            (id("eggs"), id("milk"), id("jam"), id("butter"), id("bread")),
            ("1", "2", "3", "4", "5")
        );
        let expected = [
            ("1", vec!["eggs"], vec!["milk"], 0.3125, 1.0),
            ("2", vec!["jam"], vec!["butter"], 0.375, 6.0 / 7.0),
            ("3", vec!["eggs", "bread"], vec!["milk"], 0.1875, 1.0),
            ("4", vec!["jam", "bread"], vec!["butter"], 0.25, 0.8),
            ("5", vec!["milk", "butter"], vec!["bread"], 0.125, 1.0),
        ];
        assert_eq!(model.rules.len(), expected.len());
        for (rule, (id, antecedent, consequent, support, confidence)) in
            model.rules.iter().zip(expected)
        {
            let (got_ant, got_cons) = sides(model, rule);
            assert_eq!(rule.id, id);
            assert_eq!(got_ant, antecedent);
            assert_eq!(got_cons, consequent);
            assert_eq!(rule.support, support);
            assert_eq!(rule.confidence, confidence);
        }
        assert_eq!(model.rules[1].lift, 6.0 * 16.0 / (7.0 * 8.0));
        assert_eq!(model.rules[4].lift, 2.0 * 16.0 / (2.0 * 12.0));
        let mut present = HashSet::new();
        present.insert("bread".to_string());
        present.insert("jam".to_string());
        let hits = score_rules(model, &present, 3, false, true).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item, "butter");
        assert_eq!(hits[0].rule_id, 2);
        assert_eq!(hits[0].confidence, 6.0 / 7.0);
    }

    #[test]
    fn threshold_rules_include_the_union_below_min_support() {
        let trained = train(&flags_table(), &settings(30.0, 70.0)).unwrap();
        let model = model_of(&trained);
        assert_eq!(model.minimum_support, 0.3125);
        assert_eq!(model.rules.len(), 7);
        assert_eq!(model.rules[6].support, 0.25);
        assert_eq!(model.rules[6].confidence, 0.8);
        let (antecedent, consequent) = sides(model, &model.rules[6]);
        assert_eq!(antecedent, vec!["jam".to_string(), "bread".to_string()]);
        assert_eq!(consequent, vec!["butter".to_string()]);
        assert_eq!(model.rules[3].confidence, 5.0 / 7.0);
        assert_eq!(model.rules[5].confidence, 7.0 / 9.0);
        assert_eq!(model.rules[5].lift, 7.0 * 16.0 / (9.0 * 12.0));
        let mut present = HashSet::new();
        present.insert("butter".to_string());
        present.insert("jam".to_string());
        let hits = score_rules(model, &present, 3, false, true).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].item.as_str(), hits[0].rule_id), ("bread", 5));
    }

    #[test]
    fn transactional_baskets_match_the_default_rules() {
        let mut settings = settings(10.0, 80.0);
        settings.transactional = true;
        settings.contiguous = true;
        settings.id_field = Some("basket".into());
        settings.content_field = Some("item".into());
        settings.antecedents.clear();
        settings.consequents.clear();
        let trained = train(&trans_table(), &settings).unwrap();
        let model = model_of(&trained);
        assert_eq!(model.rules.len(), 5);
        assert_eq!(model.rules[0].confidence, 1.0);
        assert_eq!(model.number_of_transactions, 16);
        assert!(
            model
                .header
                .mining_schema
                .iter()
                .any(|field| field.name == "basket" && field.usage == Usage::Group)
        );
    }

    #[test]
    fn false_flags_and_wide_antecedents_are_rejected() {
        let table = flags_table();
        let mut settings = settings(10.0, 80.0);
        settings.only_true_values = false;
        assert!(train(&table, &settings).is_err());
        settings.only_true_values = true;
        settings.max_antecedents = 33;
        assert!(train(&table, &settings).is_err());
        settings.max_antecedents = 5;
        settings.consequents = vec!["bread".into()];
        assert!(train(&table, &settings).is_err());
    }
}
