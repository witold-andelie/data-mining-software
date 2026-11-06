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
//! ranked ascending by (support, number of frequent itemsets containing the item). The sort is
//! an insertion sort with a sentinel over field order (see [`sentinel_insertion_order`]), which
//! settles equal keys (`apriori_tie`, `apriori_tie2`, `tie_equal`, `tie_twonew`). Itemsets are grown
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
//! content value is the item; the antecedent and consequent field lists do not apply
//! (`antecons_diff_trans` mines the same rules as `trans_base`). The resulting rules match
//! `apriori_default`. With `contiguousID` false ids are grouped even when split
//! (`trans_noncontig` restores 128 transactions and the five rules).
//!
//! Evaluation measures: see [`Evaluation`] (probe `apriori_eval`, every measure at five
//! percent bounds).
//!
//! False flags (`trueflags_false`): `F` is its own item `field=F` (150 rules, 10 items).
//! Nominal `Set` fields (`nominal_set`): each value is an item `field=value` (4 rules).
//! Tabular differing sets (`antecons_diff_tab`): antecedents omit `jam`, so `jam=>butter`
//! and `jam+bread=>butter` are dropped (3 rules, 4 items).
//!
//! GAP: rules with an empty antecedent and events are rejected.
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
    /// `onlyTrueValues`. False makes `F` its own item (`trueflags_false`).
    pub only_true_values: bool,
    /// `transactionalData`.
    pub transactional: bool,
    /// `contiguousID`. False groups by id even when split (`trans_noncontig`).
    pub contiguous: bool,
    /// `idField`.
    pub id_field: Option<String>,
    /// `contentField`.
    pub content_field: Option<String>,
    /// `autoModelFieldSelection`. Transactional data then uses every content value.
    pub auto_fields: bool,
    /// `modelInputFieldSelection`, in listed order. Empty with transactional data: every value.
    pub antecedents: Vec<String>,
    /// `modelTargetFieldSelection`. Tabular lists may differ (`antecons_diff_tab`); both
    /// lists are ignored with transactional data (`antecons_diff_trans`).
    pub consequents: Vec<String>,
    /// `evaluationMeasure`: see [`Evaluation`].
    pub evaluation: Evaluation,
    /// `evaluationBound`, in whole percent (Modeler stores 0.3 as 0): a rule is kept when
    /// its evaluation measure reaches `bound / 100`. Stored even when the measure is rule
    /// confidence (`apriori_default`), which ignores it.
    pub evaluation_bound: f64,
    /// `modelName` (`models_next` in the three oracles).
    pub model_name: String,
}

/// Evaluation measures (`evaluationMeasure`), with `r`, `a`, `c` the rule, antecedent and
/// consequent counts and `n` the transactions. The Algorithms Guide names them (its formulas
/// are images); probe `apriori_eval` fixes each one: 20 candidate rules (10 % support, 40 %
/// confidence) filtered at bounds 5, 10, 20, 40 and 60 %, and only these definitions keep
/// exactly Modeler's rules at all five bounds of their measure. `>=` and `>` are not told
/// apart (no rule sits on a bound).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Evaluation {
    /// `rule`: no filter beyond the minimum confidence.
    #[default]
    RuleConfidence,
    /// `confidence` (absolute confidence difference to prior): `|r/a - c/n|`. The signed
    /// difference keeps too few rules at 5 and 10 %.
    DifferenceToPrior,
    /// `ratio` (difference of the confidence quotient to 1): `1 - min(q, 1/q)` with
    /// `q = (r/a) / (c/n)`. `|q - 1|` keeps too many rules at 40 and 60 %.
    ConfidenceRatio,
    /// `information` (information difference to prior): the mutual information of the
    /// antecedent and consequent indicators in bits. Nats keep too many at 5 and 10 %.
    InformationDifference,
    /// `normalised` (normalized chi-square): `(r n - a c)^2 / (a c (n - a) (n - c))`.
    NormalizedChiSquare,
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
            evaluation: Evaluation::RuleConfidence,
            evaluation_bound: 10.0,
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
    /// Nominal `Set` fields and `onlyTrueValues` false: one `field=value` item per
    /// field (`nominal_set`, `trueflags_false`). No flag true value is written.
    Valued(Vec<ValuedColumn>),
    Transactions {
        id: usize,
        content: usize,
    },
}

#[derive(Debug)]
struct ValuedColumn {
    position: usize,
    name: String,
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
    if !settings.evaluation_bound.is_finite() {
        return Err(AlgoError::Settings(
            "Apriori evaluationBound must be finite".into(),
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
    if antecedents.is_empty() || consequents.is_empty() {
        return Err(AlgoError::Unsupported(
            "Apriori antecedent and consequent sets are empty (GAP: no oracle for empty sets)"
                .into(),
        ));
    }
    // Tabular differing sets are supported when every antecedent is also a consequent
    // (`antecons_diff_tab`: antecedents omit `jam`, consequents list all five; `jam=>butter`
    // and `jam+bread=>butter` drop, 3 rules). Any antecedent-only field (in antecedents
    // but not consequents) has no oracle and stays rejected, as does valued differing.
    {
        let consequent_set: HashSet<&str> = consequents.iter().map(String::as_str).collect();
        if !antecedents
            .iter()
            .all(|name| consequent_set.contains(name.as_str()))
        {
            return Err(AlgoError::Unsupported(
                "Apriori antecedent and consequent sets differ (GAP: only the tabular subset omit has an oracle)".into(),
            ));
        }
    }
    let mut columns_flags = Vec::new();
    let mut columns_valued = Vec::new();
    let mut order = Vec::new();
    let mut only_flags = true;
    for name in &antecedents {
        let position = field_position(table, name)?;
        let field = table
            .model()
            .fields()
            .get(position)
            .ok_or_else(|| AlgoError::Settings(format!("Apriori field {name:?} is missing")))?;
        match &field.values {
            ValueSpec::Flag { .. } if settings.only_true_values => {
                // `onlyTrueValues` true: the flag name itself is the item.
            }
            ValueSpec::Flag { .. } | ValueSpec::Categories(_) => {
                only_flags = false;
            }
            _ => {
                return Err(AlgoError::Unsupported(format!(
                    "Apriori field {name:?} is not a flag or nominal Set (GAP: no oracle)"
                )));
            }
        }
    }
    // Consequent-only fields (tabular differing: `jam` is in consequents but not
    // antecedents) still need metadata for the dictionary when they are flags with
    // `onlyTrueValues` true; valued fields only mine antecedent fields (`antecons_diff_tab`
    // mines 4 items, no `jam`).
    if only_flags && settings.only_true_values {
        for name in &antecedents {
            let position = field_position(table, name)?;
            let field =
                table.model().fields().get(position).ok_or_else(|| {
                    AlgoError::Settings(format!("Apriori field {name:?} is missing"))
                })?;
            let ValueSpec::Flag { true_value, .. } = &field.values else {
                return Err(AlgoError::Settings(format!(
                    "Apriori field {name:?} is not a flag"
                )));
            };
            columns_flags.push(FlagColumn {
                position,
                name: name.clone(),
                true_value: true_value.clone(),
                true_text: true_value.to_string(),
            });
        }
        // Dictionary also lists consequent-only flag fields (`antecons_diff_tab` lists all
        // five fields with BOTH direction in the PMML even though `jam` is never mined).
        // `prepare` order for ranking is the antecedent field order.
        let mut transactions = Vec::with_capacity(table.rows().len());
        for row in table.rows() {
            let mut basket = Vec::new();
            for column in &columns_flags {
                if row.get(column.position) == Some(&column.true_value) {
                    basket.push(column.name.clone());
                }
            }
            transactions.push(basket);
        }
        let allowed = antecedents.iter().cloned().collect();
        return Ok(Prepared {
            transactions,
            order: antecedents.clone(),
            columns: Columns::Flags(columns_flags),
            antecedent: allowed,
            consequent: consequents.into_iter().collect(),
        });
    }
    // Valued items (`trueflags_false`, `nominal_set`, and tabular differing with valued
    // fields should it occur): `field=value` per field. Order is field-major: for each
    // antecedent field in listed order, each value in declared-then-seen order
    // (`nominal_set`: red,green,blue then circle,square; `trueflags_false`: T then F per
    // field as the dictionary writes).
    let field_order = antecedents.clone();
    // Include consequent-only fields in the dictionary order? No: mining only uses
    // antecedent fields (`antecons_diff_tab` mines 4 items). Keep antecedent order.
    for name in &field_order {
        let position = field_position(table, name)?;
        let field = table
            .model()
            .fields()
            .get(position)
            .ok_or_else(|| AlgoError::Settings(format!("Apriori field {name:?} is missing")))?;
        let values: Vec<String> = match &field.values {
            ValueSpec::Flag {
                true_value,
                false_value,
            } => {
                // `trueflags_false`: T then F, the dictionary order.
                [true_value, false_value]
                    .into_iter()
                    .map(ToString::to_string)
                    .collect()
            }
            ValueSpec::Categories(values) => {
                let mut seen: Vec<String> = Vec::new();
                for value in values {
                    let text = value.to_string();
                    if !seen.iter().any(|kept| kept == &text) {
                        seen.push(text);
                    }
                }
                // Nominal score grids may name unseen values (`nominal_grid` blue/circle);
                // mining only uses training values, first-seen order appended below.
                seen
            }
            _ => {
                return Err(AlgoError::Unsupported(format!(
                    "Apriori field {name:?} is not a flag or nominal Set (GAP: no oracle)"
                )));
            }
        };
        for value in values {
            let item = format!("{name}={value}");
            if !order.iter().any(|kept| kept == &item) {
                order.push(item);
            }
        }
        columns_valued.push(ValuedColumn {
            position,
            name: name.clone(),
        });
    }
    // Append first-seen values not in the declared set (transactional-style safety;
    // training files here declare all values, so this rarely adds).
    for row in table.rows() {
        for column in &columns_valued {
            let Some(value) = row.get(column.position) else {
                continue;
            };
            if *value == Value::Null {
                continue;
            }
            let item = format!("{}={}", column.name, value);
            if !order.iter().any(|kept| kept == &item) {
                order.push(item);
            }
        }
    }
    let mut transactions = Vec::with_capacity(table.rows().len());
    for row in table.rows() {
        let mut basket = Vec::new();
        for column in &columns_valued {
            let Some(value) = row.get(column.position) else {
                continue;
            };
            if *value == Value::Null {
                continue;
            }
            basket.push(format!("{}={}", column.name, value));
        }
        transactions.push(basket);
    }
    let allowed = antecedents.iter().cloned().collect();
    Ok(Prepared {
        transactions,
        order,
        columns: Columns::Valued(columns_valued),
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
    // The antecedent and consequent field lists do not apply to transactional data: with
    // antecedents eggs/jam/bread and consequents milk/butter (`antecons_diff_trans`) Modeler
    // mines the same 128 baskets and five rules as with none (`trans_base`); every other
    // transactional oracle saves empty lists. Every content value is an item.
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
    if settings.contiguous {
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
        return Ok(Prepared {
            transactions,
            order,
            columns: Columns::Transactions {
                id: id_at,
                content: content_at,
            },
            antecedent: HashSet::new(),
            consequent: HashSet::new(),
        });
    }
    // Non-contiguous ids (`trans_noncontig`): group every row with the same id even when
    // scattered, restoring 128 transactions from the split file. Order of first mention
    // follows row order.
    let mut grouped: HashMap<String, (Value, Vec<String>, HashSet<String>)> = HashMap::new();
    let mut ids: Vec<String> = Vec::new();
    for (index, row) in table.rows().iter().enumerate() {
        let id = row.get(id_at).cloned().unwrap_or(Value::Null);
        if id == Value::Null {
            return Err(AlgoError::Data(format!(
                "Apriori transaction id is null at row {}",
                index + 1
            )));
        }
        let key = id.to_string();
        if !grouped.contains_key(&key) {
            ids.push(key.clone());
            grouped.insert(key.clone(), (id, Vec::new(), HashSet::new()));
        }
        let Some(entry) = grouped.get_mut(&key) else {
            continue;
        };
        let content = row.get(content_at).cloned().unwrap_or(Value::Null);
        if content == Value::Null {
            continue;
        }
        let text = content.to_string();
        if !order.iter().any(|item| item == &text) {
            order.push(text.clone());
        }
        if entry.2.insert(text.clone()) {
            entry.1.push(text);
        }
    }
    if grouped.len() > MAX_TRANSACTIONS {
        return Err(AlgoError::Data(format!(
            "Apriori has {} transactions; the limit is {MAX_TRANSACTIONS}",
            grouped.len()
        )));
    }
    let transactions: Vec<Vec<String>> = ids
        .iter()
        .filter_map(|key| grouped.get(key).map(|(_, basket, _)| basket.clone()))
        .collect();
    Ok(Prepared {
        transactions,
        order,
        columns: Columns::Transactions {
            id: id_at,
            content: content_at,
        },
        antecedent: HashSet::new(),
        consequent: HashSet::new(),
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
    // Start from field (first-seen) order, then name.
    ranked.sort_by(|left, right| left.2.cmp(&right.2).then(left.0.cmp(&right.0)));
    let max_size = usize::try_from(settings.max_antecedents)
        .unwrap_or(32)
        .saturating_add(1);
    // The set of frequent itemsets does not depend on the rank, so a provisional rank
    // (ascending support, field order) yields the secondary key: how many frequent itemsets of
    // two or more items contain each item.
    let mut provisional: Vec<usize> = (0..ranked.len()).collect();
    provisional.sort_by_key(|&at| ranked.get(at).map_or(0, |(_, count, _)| *count));
    let provisional_names: Vec<String> = provisional
        .iter()
        .filter_map(|&at| ranked.get(at).map(|(name, _, _)| name.clone()))
        .collect();
    let (provisional_counts, _) =
        frequent_levels(prepared, &provisional_names, supp, conf, n, max_size)?;
    let mut in_sets = vec![0usize; ranked.len()];
    for items in provisional_counts.keys().filter(|items| items.len() >= 2) {
        for &rank in items {
            if let Some(slot) = provisional
                .get(usize::from(rank))
                .and_then(|&at| in_sets.get_mut(at))
            {
                *slot = slot.saturating_add(1);
            }
        }
    }
    let keys: Vec<(u64, usize)> = ranked
        .iter()
        .zip(&in_sets)
        .map(|((_, count, _), sets)| (*count, *sets))
        .collect();
    let order = sentinel_insertion_order(&keys);
    let names: Vec<String> = order
        .iter()
        .filter_map(|&at| ranked.get(at).map(|(name, _, _)| name.clone()))
        .collect();
    let (counts_of, levels) = frequent_levels(prepared, &names, supp, conf, n, max_size)?;
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
                if !evaluation_passes(
                    settings.evaluation,
                    settings.evaluation_bound,
                    union_count,
                    ant_count,
                    cons_count,
                    n,
                ) {
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

/// Itemset counts and the levels of nodes built under one item rank.
type Levels = (HashMap<Vec<u16>, u64>, Vec<Vec<Node>>);

/// Frequent itemsets under one item rank: their counts, and the levels of nodes (singletons
/// first) that rule generation walks.
fn frequent_levels(
    prepared: &Prepared,
    names: &[String],
    supp: u128,
    conf: u128,
    n: u64,
    max_size: usize,
) -> Result<Levels, AlgoError> {
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
    Ok((counts_of, levels))
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
    let Some(name) = names.get(usize::from(rank)) else {
        return false;
    };
    if set.contains(name) {
        return true;
    }
    // Valued items (`field=value`): the set holds field names (`nominal_set`,
    // `trueflags_false`), so check the field part.
    if let Some((field, _)) = name.split_once('=') {
        return set.contains(field);
    }
    false
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
            // Consequent-only flag fields (`antecons_diff_tab` `jam`): predicted direction,
            // no mining, but listed in the PMML.
            for name in prepared
                .consequent
                .iter()
                .filter(|name| !prepared.antecedent.contains(*name))
            {
                // Valued items contain `=`; flags here never do (onlyTrue True path).
                if name.contains('=') {
                    continue;
                }
                let Some(position) = table.model().position(name) else {
                    continue;
                };
                // Only list fields that exist; missing is a settings error elsewhere.
                // Direction predicted matches `antecons_diff_tab` (`jam` predicted).
                push(position, Usage::Active, Some("predicted".into()), None)?;
            }
        }
        Columns::Valued(columns) => {
            for column in columns {
                push(column.position, Usage::Active, Some("BOTH".into()), None)?;
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

/// Whether a rule passes its evaluation measure (`evaluationBound`, percent; see
/// [`Evaluation`]). Rule confidence already passed via `minRuleConfidence`.
fn evaluation_passes(
    evaluation: Evaluation,
    bound: f64,
    union_count: u64,
    ant_count: u64,
    cons_count: u64,
    n: u64,
) -> bool {
    if !bound.is_finite() {
        return false;
    }
    if evaluation == Evaluation::RuleConfidence {
        return true;
    }
    if ant_count == 0 || cons_count == 0 || n == 0 {
        return false;
    }
    let (r, a, c, n) = (
        union_count as f64,
        ant_count as f64,
        cons_count as f64,
        n as f64,
    );
    let posterior = r / a;
    let prior = c / n;
    let value = match evaluation {
        Evaluation::RuleConfidence => return true,
        Evaluation::DifferenceToPrior => (posterior - prior).abs(),
        Evaluation::ConfidenceRatio => {
            let q = posterior / prior;
            if q <= 0.0 { 1.0 } else { 1.0 - q.min(1.0 / q) }
        }
        Evaluation::InformationDifference => {
            // The four cells of the 2 x 2 table, each with its row and column totals.
            [
                (r, a, c),
                (a - r, a, n - c),
                (c - r, n - a, c),
                (n - a - c + r, n - a, n - c),
            ]
            .into_iter()
            .filter(|&(k, row, col)| k > 0.0 && row > 0.0 && col > 0.0)
            .map(|(k, row, col)| k / n * (k * n / (row * col)).log2())
            .sum()
        }
        Evaluation::NormalizedChiSquare => {
            let denominator = a * c * (n - a) * (n - c);
            if denominator <= 0.0 {
                return false;
            }
            (r * n - a * c).powi(2) / denominator
        }
    };
    value >= bound / 100.0
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

/// Modeler's item rank: an insertion sort with a sentinel, ascending by key. The first
/// minimum is swapped into position 0, then the rest is insertion-sorted (stable). The one
/// swap makes equal keys unstable in a specific way: the item first in field order moves to
/// where the minimum was.
///
/// The key is (support, number of frequent itemsets of two or more items containing the
/// item). It reproduces all 23 `apriori_tie`/`apriori_tie2` layouts, plus
/// `apriori_more/tie_equal`, `tie_twonew` and `trueflags_false`. Support alone fits only
/// 14 of the 23. The count of frequent pairs fits the 23 but breaks `trueflags_false`
/// (butter=T before butter=F). The size of the largest frequent itemset containing the item
/// fits everything too; no recorded case separates it from the count.
fn sentinel_insertion_order<K: Ord>(keys: &[K]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..keys.len()).collect();
    let key = |at: usize| keys.get(at);
    let mut min = 0usize;
    for (position, &at) in order.iter().enumerate().skip(1) {
        if key(at) < key(order.get(min).copied().unwrap_or(0)) {
            min = position;
        }
    }
    order.swap(0, min);
    for i in 2..order.len() {
        let mut j = i;
        while j > 1 {
            let (Some(&before), Some(&here)) = (order.get(j - 1), order.get(j)) else {
                break;
            };
            if key(before) <= key(here) {
                break;
            }
            order.swap(j - 1, j);
            j -= 1;
        }
    }
    order
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
    fn false_flags_supported_but_wide_antecedents_rejected() {
        let table = flags_table();
        let mut settings = settings(10.0, 80.0);
        // `onlyTrueValues` false is supported (`trueflags_false`): `F` is its own item.
        settings.only_true_values = false;
        assert!(train(&table, &settings).is_ok());
        settings.only_true_values = true;
        settings.max_antecedents = 33;
        assert!(train(&table, &settings).is_err());
        settings.max_antecedents = 5;
        // Antecedent-only differing (antecedents not a subset of consequents) stays rejected;
        // only the tabular subset omit (`antecons_diff_tab`) is supported.
        settings.consequents = vec!["bread".into()];
        assert!(train(&table, &settings).is_err());
    }
}
