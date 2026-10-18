//! CARMA association rules (`BuildCarmaNode`; Algorithms Guide: "CARMA Algorithms"): every rule
//! X => Y with disjoint non-empty itemsets whose union meets the minimum rule support and whose
//! confidence meets the minimum rule confidence, consequents of several items included. The
//! model is the PMML `AssociationModel` Apriori writes (`crate::apriori`), with algorithm
//! `Carma`.
//!
//! Evidence (`oracle/models_next/carma_*`, the 16 baskets of `apriori_*`):
//! - Rule support is the support of the union, in percent of the transactions
//!   (`minRuleSupport`); confidence is the union count over the antecedent count
//!   (`minRuleConfidence`). `carma_default` (20%, 20%) has exactly the 16 such rules, among them
//!   `jam => butter & bread`; `carma_minsupp` (40%) the two between milk and bread.
//! - Rules are ordered by confidence, then union support, both descending, then by generation
//!   order: unions by size, each size in lexicographic order of the input fields, each union's
//!   antecedents by size, then lexicographically (`carma_default` rules 5/6, 8/9 and 13/14).
//!   GAP: every tie in these cases is also decided by a coarser key, so other generation orders
//!   that agree here are not excluded.
//! - Item and itemset ids follow Apriori's first-mention rule (antecedent items, then consequent
//!   items, each from the rarest; `carma_default`: eggs=1, milk=2, jam=3, butter=4, bread=5 and
//!   itemsets 1..8). `minimumSupport` is the smallest support of an emitted itemset and
//!   `minimumConfidence` the smallest rule confidence. Mining fields have no direction.
//!
//! Rejected (no oracle): transactional data, pruning, varying support, rules without
//! antecedents, excluding multiple consequents, weights and frequencies, and
//! `maxLengthSet` above 32.

use crate::AlgoError;
use crate::apriori::{
    self, AprioriSettings, Mined, at_least, at_least_ratio, percent_basis, prepare_flags,
};
use dms_pmml::Pmml;
use dms_types::Table;

const MAX_ITEMS: usize = 64;
const MAX_RULES: usize = 500_000;

#[derive(Debug, Clone, PartialEq)]
pub struct CarmaSettings {
    /// `minRuleSupport`, percent (default 20).
    pub min_support_percent: f64,
    /// `minRuleConfidence`, percent (default 20).
    pub min_confidence_percent: f64,
    /// `maxLengthSet`: the largest union (default 10).
    pub max_size: usize,
    /// `modelInputFieldSelection`; empty for the Type's inputs.
    pub inputs: Vec<String>,
    pub model_name: String,
}

/// Union counts of every item combination that meets the support, smallest first, each size in
/// lexicographic order of the input fields.
fn frequent(
    baskets: &[u64],
    items: usize,
    max_size: usize,
    supported: impl Fn(u64) -> bool,
) -> Result<Vec<(u64, u64)>, AlgoError> {
    let count = |mask: u64| baskets.iter().filter(|b| **b & mask == mask).count() as u64;
    let mut level: Vec<(u64, u64)> = (0..items)
        .map(|i| 1u64 << i)
        .map(|mask| (mask, count(mask)))
        .filter(|(_, c)| supported(*c))
        .collect();
    let mut all = level.clone();
    for _ in 2..=max_size {
        let mut next = Vec::new();
        for (mask, _) in &level {
            let highest = 63 - mask.leading_zeros() as usize;
            for item in highest + 1..items {
                let grown = mask | (1u64 << item);
                // Every subset one smaller must be frequent.
                let frequent_subsets = (0..items).filter(|i| grown & (1 << i) != 0).all(|i| {
                    let subset = grown & !(1u64 << i);
                    all.iter().any(|(m, _)| *m == subset)
                });
                if frequent_subsets {
                    let c = count(grown);
                    if supported(c) {
                        next.push((grown, c));
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        // Lexicographic order of the item lists.
        next.sort_by_key(|(mask, _)| {
            (0..items)
                .filter(|i| mask & (1 << i) != 0)
                .collect::<Vec<_>>()
        });
        if all.len() + next.len() > MAX_RULES {
            return Err(AlgoError::Unsupported("too many CARMA itemsets".into()));
        }
        all.extend(next.iter().copied());
        level = next;
    }
    Ok(all)
}

/// The item lists of every non-empty proper subset of `items`, by size, then lexicographically.
fn antecedents(items: &[usize]) -> Vec<Vec<usize>> {
    let mut subsets: Vec<Vec<usize>> = (1..(1u64 << items.len()) - 1)
        .map(|bits| {
            items
                .iter()
                .enumerate()
                .filter(|(i, _)| bits & (1 << i) != 0)
                .map(|(_, item)| *item)
                .collect()
        })
        .collect();
    subsets.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    subsets
}

pub fn train(table: &Table, settings: &CarmaSettings) -> Result<Pmml, AlgoError> {
    if !(1..=32).contains(&settings.max_size) {
        return Err(AlgoError::Unsupported(format!(
            "CARMA maxLengthSet {} (GAP: 1 to 32)",
            settings.max_size
        )));
    }
    let apriori_settings = AprioriSettings {
        min_support_percent: settings.min_support_percent,
        min_confidence_percent: settings.min_confidence_percent,
        auto_fields: settings.inputs.is_empty(),
        antecedents: settings.inputs.clone(),
        consequents: settings.inputs.clone(),
        model_name: settings.model_name.clone(),
        ..AprioriSettings::default()
    };
    let prepared = prepare_flags(table, &apriori_settings)?;
    let names = prepared.order.clone();
    if names.len() > MAX_ITEMS {
        return Err(AlgoError::Unsupported(format!(
            "CARMA with more than {MAX_ITEMS} items"
        )));
    }
    let n = prepared.transactions.len() as u64;
    if n == 0 {
        return Err(AlgoError::Data("CARMA has no transactions".into()));
    }
    let baskets: Vec<u64> = prepared
        .transactions
        .iter()
        .map(|basket| {
            basket
                .iter()
                .filter_map(|item| names.iter().position(|name| name == item))
                .fold(0u64, |mask, i| mask | (1 << i))
        })
        .collect();
    let supp = percent_basis(settings.min_support_percent)?;
    let conf = percent_basis(settings.min_confidence_percent)?;
    let count_of = |mask: u64| baskets.iter().filter(|b| **b & mask == mask).count() as u64;
    let unions = frequent(&baskets, names.len(), settings.max_size, |c| {
        at_least(c, supp, n)
    })?;
    let mut rules = Vec::new();
    for (mask, union_count) in unions.iter().filter(|(m, _)| m.count_ones() >= 2) {
        let items: Vec<usize> = (0..names.len()).filter(|i| mask & (1 << i) != 0).collect();
        for antecedent in antecedents(&items) {
            let ant_mask = antecedent.iter().fold(0u64, |m, i| m | (1 << i));
            let ant_count = count_of(ant_mask);
            if !at_least_ratio(*union_count, ant_count, conf) {
                continue;
            }
            let cons_mask = mask & !ant_mask;
            rules.push((
                ant_mask,
                cons_mask,
                *union_count,
                ant_count,
                count_of(cons_mask),
            ));
            if rules.len() > MAX_RULES {
                return Err(AlgoError::Unsupported("too many CARMA rules".into()));
            }
        }
    }
    // Confidence, then union support, descending; the sort is stable over generation order.
    rules.sort_by(|a, b| {
        let (ua, aa, ub, ab) = (
            u128::from(a.2),
            u128::from(a.3),
            u128::from(b.2),
            u128::from(b.3),
        );
        (ub * aa).cmp(&(ua * ab)).then(b.2.cmp(&a.2))
    });
    // Items within a side are mentioned from the rarest (then input order), as Apriori.
    let single: Vec<u64> = (0..names.len()).map(|i| count_of(1 << i)).collect();
    let ranks = |mask: u64| -> Vec<u16> {
        let mut items: Vec<usize> = (0..names.len()).filter(|i| mask & (1 << i) != 0).collect();
        items.sort_by_key(|i| (single[*i], *i));
        items
            .into_iter()
            .map(|i| u16::try_from(i).unwrap_or(u16::MAX))
            .collect()
    };
    let mined: Vec<Mined> = rules
        .iter()
        .map(|(ant, cons, union_count, ant_count, cons_count)| Mined {
            antecedent: ranks(*ant),
            consequent: ranks(*cons),
            union_count: *union_count,
            ant_count: *ant_count,
            cons_count: *cons_count,
        })
        .collect();
    let mut pmml = apriori::assemble_model(
        table,
        &apriori_settings,
        &prepared,
        &(names, mined, n),
        "Carma",
    )?;
    // CARMA's fields carry no direction (`carma_default`), unlike Apriori's `BOTH`.
    if let dms_pmml::Model::Association(model) = &mut pmml.model {
        for field in &mut model.header.mining_schema {
            field.direction = None;
        }
        let support: std::collections::HashMap<String, f64> = model
            .itemsets
            .iter()
            .map(|itemset| (itemset.id.clone(), itemset.support))
            .collect();
        for rule in &mut model.rules {
            rule.confidence = fifteen_digits(rule.confidence);
            if let Some(consequent) = support.get(&rule.consequent).filter(|s| **s > 0.0) {
                rule.lift = rule.confidence / consequent;
            }
        }
        if let Some(min) = model
            .rules
            .iter()
            .map(|rule| rule.confidence)
            .reduce(f64::min)
        {
            model.minimum_confidence = min;
        }
    }
    Ok(pmml)
}

/// CARMA saves a rule's confidence to 15 significant digits and derives its lift from that
/// value (`carma_default` rule 4: confidence 0.777777777777778, lift 1.0370370370370374 =
/// 0.777777777777778 / 0.75, where 7 × 16 / (9 × 12) is 1.037037037037037).
fn fifteen_digits(x: f64) -> f64 {
    format!("{x:.14e}").parse().unwrap_or(x)
}
