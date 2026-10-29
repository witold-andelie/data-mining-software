//! PMML `AssociationModel` (Apriori). Items, itemsets and rules are kept by id.
//! [`score_rules`] returns every rule whose antecedent items are in the basket.
//!
//! The nugget settings ([`ScoringOptions`]) were fixed on unlinked nuggets scoring the 128
//! training baskets of probe `apriori_eval` (`score_matrix`, 20 rules; every setting matches
//! all rows): `checkPredictions` `notInBasket` (default: the consequent must be absent),
//! `inBasket` (it must be present) or `noCheck`; `ignoreUnmatchedItems` false makes the
//! antecedent equal the whole basket (`score_trans_unmatched`: `{bread, jam}` fires only
//! rule 17, a basket with the unknown `cheese` fires nothing). Hits are ordered by the
//! [`RuleCriterion`] descending (equal confidences by antecedent length and item ids), then
//! rule id ascending (`apriori_thresholds` rows 9–10 keep
//! rule 5 over rule 4). With `allow_repeats` false a hit whose consequent items were all
//! predicted by earlier hits is skipped: one rule per single consequent (`apriori_default`
//! row 14 keeps rule 2, jam ⇒ butter, over rule 4), and `jam & butter` after `butter` and
//! `jam` (`carma_default` grid row 2). With `allow_repeats` true both are kept.
//!
//! A consequent of several items (CARMA, `carma_default` rules 11, 14, 16) predicts their
//! values joined by ` & ` in `ItemRef` order (`butter & bread`); with `consequent_absent` it is
//! dropped only when every one of its items is in the basket (`carma_default` training row 3,
//! butter and jam: rule 11, jam => butter & bread, is predicted).

use crate::parse::{self, parse_model_header};
use crate::{ModelHeader, PmmlError};
use std::collections::HashSet;

/// One scored rule: the consequent item, its confidence and the PMML rule id.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredAssociation {
    pub item: String,
    pub confidence: f64,
    pub rule_id: i64,
}

/// An `Item`.
#[derive(Debug, Clone, PartialEq)]
pub struct AssociationItem {
    pub id: String,
    pub value: String,
}

/// An `Itemset`. `items` are `Item@id` values in document order.
#[derive(Debug, Clone, PartialEq)]
pub struct AssociationItemset {
    pub id: String,
    pub support: f64,
    pub items: Vec<String>,
}

/// An `AssociationRule`. `antecedent` and `consequent` are itemset ids.
#[derive(Debug, Clone, PartialEq)]
pub struct AssociationRule {
    pub id: String,
    pub antecedent: String,
    pub consequent: String,
    pub support: f64,
    pub confidence: f64,
    pub lift: f64,
}

/// PMML 4.2 `AssociationModel` as Modeler writes it for Apriori.
#[derive(Debug, Clone, PartialEq)]
pub struct AssociationModel {
    pub header: ModelHeader,
    pub number_of_transactions: u64,
    pub minimum_support: f64,
    pub minimum_confidence: f64,
    pub items: Vec<AssociationItem>,
    pub itemsets: Vec<AssociationItemset>,
    pub rules: Vec<AssociationRule>,
}

impl AssociationModel {
    /// The support of itemset `id`.
    pub fn itemset_support(&self, id: &str) -> Result<f64, PmmlError> {
        self.itemsets
            .iter()
            .find(|itemset| itemset.id == id)
            .map(|itemset| itemset.support)
            .ok_or_else(|| PmmlError::Score(format!("association model has no itemset {id:?}")))
    }

    /// The numeric item ids of itemset `id`, ascending.
    pub fn itemset_item_ids(&self, id: &str) -> Result<Vec<u64>, PmmlError> {
        let itemset = self
            .itemsets
            .iter()
            .find(|itemset| itemset.id == id)
            .ok_or_else(|| PmmlError::Score(format!("association model has no itemset {id:?}")))?;
        let mut ids = itemset
            .items
            .iter()
            .map(|item| {
                item.parse::<u64>().map_err(|_| {
                    PmmlError::Score(format!("association item id {item:?} is not an integer"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        ids.sort_unstable();
        Ok(ids)
    }

    /// Item values of itemset `id`, in `ItemRef` order.
    pub fn itemset_values(&self, id: &str) -> Result<Vec<&str>, PmmlError> {
        let itemset = self
            .itemsets
            .iter()
            .find(|itemset| itemset.id == id)
            .ok_or_else(|| PmmlError::Score(format!("association model has no itemset {id:?}")))?;
        let mut values = Vec::with_capacity(itemset.items.len());
        for item_id in &itemset.items {
            let item = self
                .items
                .iter()
                .find(|item| item.id == *item_id)
                .ok_or_else(|| {
                    PmmlError::Score(format!("association itemset {id:?} cites item {item_id:?}"))
                })?;
            values.push(item.value.as_str());
        }
        Ok(values)
    }
}

pub(crate) fn parse(node: roxmltree::Node<'_, '_>) -> Result<AssociationModel, PmmlError> {
    let header = parse_model_header(node)?;
    let number_of_transactions = req_u64(node, "numberOfTransactions")?;
    let minimum_support = parse::req_f64_attr(node, "minimumSupport")?;
    let minimum_confidence = parse::req_f64_attr(node, "minimumConfidence")?;
    let mut items = Vec::new();
    let mut itemsets = Vec::new();
    let mut rules = Vec::new();
    let mut item_ids = HashSet::new();
    let mut itemset_ids = HashSet::new();
    let mut rule_ids = HashSet::new();
    for child in node.children().filter(roxmltree::Node::is_element) {
        match child.tag_name().name() {
            "Item" => {
                let id = parse::req_attr(child, "id")?.to_string();
                let value = parse::req_attr(child, "value")?.to_string();
                if !item_ids.insert(id.clone()) {
                    return Err(PmmlError::Parse(format!("duplicate Item id {id:?}")));
                }
                items.push(AssociationItem { id, value });
            }
            "Itemset" => {
                let id = parse::req_attr(child, "id")?.to_string();
                let support = parse::req_f64_attr(child, "support")?;
                let mut members = Vec::new();
                for item_ref in child.children().filter(roxmltree::Node::is_element) {
                    if item_ref.tag_name().name() != "ItemRef" {
                        continue;
                    }
                    members.push(parse::req_attr(item_ref, "itemRef")?.to_string());
                }
                if members.is_empty() {
                    return Err(PmmlError::Parse(format!("Itemset {id:?} has no items")));
                }
                if let Some(text) = child.attribute("numberOfItems") {
                    let declared = text.parse::<usize>().map_err(|_| {
                        PmmlError::Parse(format!("Itemset {id:?} has numberOfItems {text:?}"))
                    })?;
                    if declared != members.len() {
                        return Err(PmmlError::Parse(format!(
                            "Itemset {id:?} declares {declared} items and lists {}",
                            members.len()
                        )));
                    }
                }
                if !itemset_ids.insert(id.clone()) {
                    return Err(PmmlError::Parse(format!("duplicate Itemset id {id:?}")));
                }
                itemsets.push(AssociationItemset {
                    id,
                    support,
                    items: members,
                });
            }
            "AssociationRule" => {
                let id = parse::req_attr(child, "id")?.to_string();
                if !rule_ids.insert(id.clone()) {
                    return Err(PmmlError::Parse(format!(
                        "duplicate AssociationRule id {id:?}"
                    )));
                }
                rules.push(AssociationRule {
                    id,
                    antecedent: parse::req_attr(child, "antecedent")?.to_string(),
                    consequent: parse::req_attr(child, "consequent")?.to_string(),
                    support: parse::req_f64_attr(child, "support")?,
                    confidence: parse::req_f64_attr(child, "confidence")?,
                    lift: parse::req_f64_attr(child, "lift")?,
                });
            }
            _ => {}
        }
    }
    check_count(node, "numberOfItems", items.len())?;
    check_count(node, "numberOfItemsets", itemsets.len())?;
    check_count(node, "numberOfRules", rules.len())?;
    for itemset in &itemsets {
        for item_id in &itemset.items {
            if !item_ids.contains(item_id) {
                return Err(PmmlError::Parse(format!(
                    "Itemset {:?} cites missing item {item_id:?}",
                    itemset.id
                )));
            }
        }
    }
    for rule in &rules {
        if !itemset_ids.contains(&rule.antecedent) || !itemset_ids.contains(&rule.consequent) {
            return Err(PmmlError::Parse(format!(
                "AssociationRule {:?} cites a missing itemset",
                rule.id
            )));
        }
    }
    Ok(AssociationModel {
        header,
        number_of_transactions,
        minimum_support,
        minimum_confidence,
        items,
        itemsets,
        rules,
    })
}

fn req_u64(node: roxmltree::Node<'_, '_>, attr: &str) -> Result<u64, PmmlError> {
    let text = parse::req_attr(node, attr)?;
    text.parse::<u64>()
        .map_err(|_| PmmlError::Parse(format!("invalid integer {text:?} for {attr}")))
}

fn check_count(node: roxmltree::Node<'_, '_>, attr: &str, found: usize) -> Result<(), PmmlError> {
    let Some(text) = node.attribute(attr) else {
        return Ok(());
    };
    let declared = text
        .parse::<usize>()
        .map_err(|_| PmmlError::Parse(format!("invalid integer {text:?} for {attr}")))?;
    if declared != found {
        return Err(PmmlError::Parse(format!(
            "{attr} is {declared} but {found} elements were read"
        )));
    }
    Ok(())
}

/// Nugget `ruleCriteria`: how fired rules are ranked. Modeler ranks from the rule values as
/// its PMML stores them, so confidence enters with 15 significant digits: rule support is
/// the antecedent support times that confidence, which puts `bread => butter`
/// (0.75 x 0.5 = 0.375) ahead of `jam => butter` (0.4375 x 0.857142857142857 =
/// 0.37499999999999994) in `score_matrix`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleCriterion {
    Confidence,
    /// `support`: the antecedent's support.
    Support,
    /// `ruleSupport`: antecedent support times confidence.
    RuleSupport,
    /// `lift`: confidence over the consequent's support.
    Lift,
    /// `deployability`: antecedent support minus rule support (antecedent without the
    /// consequent).
    Deployability,
}

/// Nugget `checkPredictions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasketCheck {
    /// `notInBasket`: the consequent must be absent from the basket.
    NotInBasket,
    /// `inBasket`: the consequent must be present.
    InBasket,
    /// `noCheck`.
    NoCheck,
}

/// The Apriori nugget's scoring settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoringOptions {
    pub max_predictions: usize,
    pub allow_repeats: bool,
    pub check: BasketCheck,
    pub criterion: RuleCriterion,
    /// `ignoreUnmatchedItems` false: the antecedent must be the whole basket.
    pub exact_antecedent: bool,
}

/// Rules that fire on `present` item values, with the default criterion and no exact match.
/// `consequent_absent` is nugget `checkPredictions=notInBasket`.
pub fn score_rules(
    model: &AssociationModel,
    present: &HashSet<String>,
    max_predictions: usize,
    allow_repeats: bool,
    consequent_absent: bool,
) -> Result<Vec<ScoredAssociation>, PmmlError> {
    score_rules_by(
        model,
        present,
        &ScoringOptions {
            max_predictions,
            allow_repeats,
            check: if consequent_absent {
                BasketCheck::NotInBasket
            } else {
                BasketCheck::NoCheck
            },
            criterion: RuleCriterion::Confidence,
            exact_antecedent: false,
        },
    )
}

/// A value with 15 significant digits, as Modeler's PMML stores rule statistics.
fn stored(value: f64) -> f64 {
    format!("{value:.14e}").parse().unwrap_or(value)
}

/// [`score_rules`] with every nugget setting.
pub fn score_rules_by(
    model: &AssociationModel,
    present: &HashSet<String>,
    options: &ScoringOptions,
) -> Result<Vec<ScoredAssociation>, PmmlError> {
    let ScoringOptions {
        max_predictions,
        allow_repeats,
        check,
        criterion,
        exact_antecedent,
    } = *options;
    if max_predictions == 0 {
        return Ok(Vec::new());
    }
    let mut hits = Vec::new();
    for rule in &model.rules {
        let antecedent = model.itemset_values(&rule.antecedent)?;
        let consequent = model.itemset_values(&rule.consequent)?;
        if consequent.is_empty() {
            return Err(PmmlError::Score(format!(
                "association rule {:?} has no consequent items",
                rule.id
            )));
        }
        if !antecedent.iter().all(|name| present.contains(*name))
            || (exact_antecedent && antecedent.len() != present.len())
        {
            continue;
        }
        let in_basket = consequent.iter().all(|name| present.contains(*name));
        match check {
            BasketCheck::NotInBasket if in_basket => continue,
            BasketCheck::InBasket if !in_basket => continue,
            _ => {}
        }
        let rule_id = rule.id.parse::<i64>().map_err(|_| {
            PmmlError::Score(format!(
                "association rule id {:?} is not an integer",
                rule.id
            ))
        })?;
        let items: Vec<String> = consequent.iter().map(|name| (*name).to_string()).collect();
        let rank = match criterion {
            RuleCriterion::Confidence => rule.confidence,
            RuleCriterion::Support => model.itemset_support(&rule.antecedent)?,
            RuleCriterion::RuleSupport => {
                model.itemset_support(&rule.antecedent)? * stored(rule.confidence)
            }
            RuleCriterion::Lift => {
                stored(rule.confidence) / model.itemset_support(&rule.consequent)?
            }
            RuleCriterion::Deployability => model.itemset_support(&rule.antecedent)? - rule.support,
        };
        // Equal confidences follow the antecedents, shorter first, then by item ids, then the
        // rule id (`trueflags_false` row 6, an all-false basket with 17 rules at confidence 1:
        // Modeler predicts rules 71 {2,4}, 75 {2,8} and 17 {2,10}, where rule ids alone give
        // 17, 21, 22; `score_matrix` `noCheck` row 6 keeps rule 5 before rule 8 on the same
        // antecedent). Every other confidence-scored oracle row fits both orders. The other
        // criteria break ties by rule id alone (this order changes 16 `score_matrix` rows
        // under support).
        let tie = if criterion == RuleCriterion::Confidence {
            Some(model.itemset_item_ids(&rule.antecedent)?)
        } else {
            None
        };
        // `$AC-` holds the criterion's value (`score_matrix`: support writes 0.75, the
        // antecedent support of `bread => jam`, not its confidence 0.4167).
        hits.push((
            ScoredAssociation {
                item: items.join(" & "),
                confidence: rank,
                rule_id,
            },
            items,
            (rank, tie),
        ));
    }
    hits.sort_by(
        |(left, _, (left_rank, left_tie)), (right, _, (right_rank, right_tie))| {
            right_rank
                .total_cmp(left_rank)
                .then_with(|| match (left_tie, right_tie) {
                    (Some(left_ante), Some(right_ante)) => left_ante
                        .len()
                        .cmp(&right_ante.len())
                        .then_with(|| left_ante.cmp(right_ante)),
                    _ => std::cmp::Ordering::Equal,
                })
                .then(left.rule_id.cmp(&right.rule_id))
        },
    );
    let mut predicted: HashSet<String> = HashSet::new();
    let mut kept = Vec::new();
    for (hit, items, _) in hits {
        if !allow_repeats && items.iter().all(|item| predicted.contains(item)) {
            continue;
        }
        predicted.extend(items);
        kept.push(hit);
        if kept.len() == max_predictions {
            break;
        }
    }
    Ok(kept)
}
