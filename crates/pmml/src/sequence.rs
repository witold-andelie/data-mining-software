//! PMML `SequenceModel` (Modeler's Sequence node) and the rules container the nugget saves.
//!
//! A sequential rule is an ordered list of *itemsets* (the antecedent) followed by one itemset
//! (the consequent); an itemset is a set of `field = value` conditions that hold together in one
//! time window of one id. [`SequenceRule`] is the single representation both channels share:
//!
//! * the saved attachment (`model/rules@containerData`, a `data/0002.dat` member holding
//!   `<rule>` elements) — read by [`parse_rules`]; it carries the confidence only, so `support`
//!   and `occurrence` read as 0;
//! * the exported PMML (`<SequenceModel>` with `Item` / `Itemset` / `Sequence` /
//!   `SequenceRule`) — read by [`parse`]; it also carries `support` (the antecedent's share of
//!   the transactions) and `occurrence` (the transactions holding antecedent and consequent in
//!   order).
//!
//! Scoring a record walks each id's history and is the executor's job (`dms_exec::nodes`);
//! nothing here scores, because a sequence model writes one prediction per rank, not one
//! [`crate::Prediction`].
//!
//! Modeler's `Item` elements carry the value only, so the content field of an item comes from
//! the model's single active `MiningField`; a model with several content fields cannot be read
//! back from PMML (`GAP:` no oracle trains one).

use crate::parse::{self, parse_model_header};
use crate::{ModelHeader, PmmlError, Usage};
use std::collections::HashMap;

/// One itemset: the `field = value` conditions that hold together in one time window.
pub type Itemset = Vec<(String, String)>;

/// One sequential rule, in the model's order.
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceRule {
    /// The antecedent itemsets, earliest first.
    pub antecedent: Vec<Itemset>,
    /// The consequent itemsets: the last itemset of the mined sequence.
    pub consequent: Vec<Itemset>,
    /// `P(consequent | antecedent)`: transactions holding the whole sequence over those
    /// holding the antecedent.
    pub confidence: f64,
    /// PMML `support`: the antecedent's share of the transactions. The rules container does
    /// not store it, so a rule read from there carries 0.0.
    pub support: f64,
    /// PMML `occurrence`: the transactions holding the antecedent and the consequent in
    /// order. The rules container does not store it, so a rule read from there carries 0.
    pub occurrence: u64,
}

/// PMML `SequenceModel`, flattened: the rules in the model's order.
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceModel {
    pub header: ModelHeader,
    /// The `MiningField@usageType="group"`: the id field.
    pub id_field: String,
    /// The `MiningField@usageType="order"`: the time field, absent without one.
    pub time_field: Option<String>,
    /// The `MiningField@usageType="active"` fields: the content fields, in schema order.
    pub content_fields: Vec<String>,
    /// `numberOfTransactions`: the id groups the model mined.
    pub number_of_transactions: u64,
    /// The rules, in the model's order.
    pub rules: Vec<SequenceRule>,
}

pub(crate) fn parse(node: roxmltree::Node<'_, '_>) -> Result<SequenceModel, PmmlError> {
    let header = parse_model_header(node)?;
    let number_of_transactions = match node.attribute("numberOfTransactions") {
        Some(text) => text
            .parse::<u64>()
            .map_err(|_| PmmlError::Parse(format!("invalid numberOfTransactions {text:?}")))?,
        None => 0,
    };
    let (id_field, time_field, content_fields) = schema_fields(&header)?;
    let content = content_fields
        .first()
        .cloned()
        .ok_or_else(|| PmmlError::Parse("a Sequence model names no content field".to_string()))?;
    let mut items: HashMap<String, String> = HashMap::new();
    let mut itemsets: HashMap<String, Vec<String>> = HashMap::new();
    let mut sequences: HashMap<String, Vec<String>> = HashMap::new();
    let mut rules = Vec::new();
    for child in node.children().filter(roxmltree::Node::is_element) {
        match child.tag_name().name() {
            "Item" => {
                let id = parse::req_attr(child, "id")?.to_string();
                let value = parse::req_attr(child, "value")?.to_string();
                items.insert(id, value);
            }
            "Itemset" => {
                let id = parse::req_attr(child, "id")?.to_string();
                let mut members = Vec::new();
                for reference in child.children().filter(roxmltree::Node::is_element) {
                    if reference.tag_name().name() == "ItemRef" {
                        members.push(parse::req_attr(reference, "itemRef")?.to_string());
                    }
                }
                itemsets.insert(id, members);
            }
            "Sequence" => {
                let id = parse::req_attr(child, "id")?.to_string();
                let mut sets = Vec::new();
                for reference in child.children().filter(roxmltree::Node::is_element) {
                    if reference.tag_name().name() == "SetReference" {
                        sets.push(parse::req_attr(reference, "setId")?.to_string());
                    }
                }
                sequences.insert(id, sets);
            }
            "SequenceRule" => {
                rules.push(parse_rule(child, &content, &items, &itemsets, &sequences)?);
            }
            _ => {}
        }
    }
    Ok(SequenceModel {
        header,
        id_field,
        time_field,
        content_fields,
        number_of_transactions,
        rules,
    })
}

/// The id, time and content fields of the mining schema. `usageType="order"` is not a PMML
/// usage the parser names, so it reads as [`Usage::Other`].
fn schema_fields(header: &ModelHeader) -> Result<(String, Option<String>, Vec<String>), PmmlError> {
    let mut id_field = None;
    let mut time_field = None;
    let mut content_fields = Vec::new();
    for field in &header.mining_schema {
        match field.usage {
            Usage::Group => {
                if id_field.is_none() {
                    id_field = Some(field.name.clone());
                }
            }
            Usage::Active => content_fields.push(field.name.clone()),
            // The time field: the only other usage a sequence model has.
            _ => {
                if time_field.is_none() {
                    time_field = Some(field.name.clone());
                }
            }
        }
    }
    Ok((id_field.unwrap_or_default(), time_field, content_fields))
}

/// One `SequenceRule`: the antecedent and consequent `Sequence`s it cites, resolved to
/// `field = value` itemsets, with its confidence, support and occurrence.
fn parse_rule(
    node: roxmltree::Node<'_, '_>,
    content: &str,
    items: &HashMap<String, String>,
    itemsets: &HashMap<String, Vec<String>>,
    sequences: &HashMap<String, Vec<String>>,
) -> Result<SequenceRule, PmmlError> {
    let id = node.attribute("id").unwrap_or("?");
    let cited = |tag: &str| -> Result<&str, PmmlError> {
        node.children()
            .find(|child| child.has_tag_name(tag))
            .and_then(|child| {
                child
                    .children()
                    .find(|c| c.has_tag_name("SequenceReference"))
            })
            .and_then(|reference| reference.attribute("seqId"))
            .ok_or_else(|| PmmlError::Parse(format!("SequenceRule {id:?} has no {tag}")))
    };
    let antecedent = resolve(
        cited("AntecedentSequence")?,
        content,
        items,
        itemsets,
        sequences,
    )?;
    let consequent = resolve(
        cited("ConsequentSequence")?,
        content,
        items,
        itemsets,
        sequences,
    )?;
    Ok(SequenceRule {
        antecedent,
        consequent,
        confidence: parse::req_f64_attr(node, "confidence")?,
        support: parse::req_f64_attr(node, "support")?,
        occurrence: match node.attribute("occurrence") {
            Some(text) => text.parse::<u64>().map_err(|_| {
                PmmlError::Parse(format!("SequenceRule {id:?} has occurrence {text:?}"))
            })?,
            None => 0,
        },
    })
}

/// The itemsets of the `Sequence` with this id, as `(field, value)` pairs.
fn resolve(
    seq_id: &str,
    content: &str,
    items: &HashMap<String, String>,
    itemsets: &HashMap<String, Vec<String>>,
    sequences: &HashMap<String, Vec<String>>,
) -> Result<Vec<Itemset>, PmmlError> {
    let sets = sequences
        .get(seq_id)
        .ok_or_else(|| PmmlError::Parse(format!("a Sequence rule cites no sequence {seq_id:?}")))?;
    let mut out = Vec::with_capacity(sets.len());
    for set_id in sets {
        let members = itemsets.get(set_id).ok_or_else(|| {
            PmmlError::Parse(format!("Sequence {seq_id:?} cites no itemset {set_id:?}"))
        })?;
        let mut set = Vec::with_capacity(members.len());
        for item_id in members {
            let value = items.get(item_id).ok_or_else(|| {
                PmmlError::Parse(format!("Itemset {set_id:?} cites no item {item_id:?}"))
            })?;
            set.push((content.to_string(), value.clone()));
        }
        out.push(set);
    }
    Ok(out)
}

/// The rules of a saved rules container (`model/rules@containerData`).
///
/// The member holds a sequence of `<rule>` elements; each has an `<antecedent>` and a
/// `<consequent>` `SequenceCondition` and a `<confidence>`. A condition's `then` list is its
/// itemsets, each an `and` of `field = value` conditions.
pub fn parse_rules(text: &str) -> Result<Vec<SequenceRule>, PmmlError> {
    let wrapped = format!("<rules>{text}</rules>");
    let document = roxmltree::Document::parse(&wrapped)
        .map_err(|error| PmmlError::Parse(format!("sequence rules: {error}")))?;
    let mut rules = Vec::new();
    for rule in document
        .root_element()
        .children()
        .filter(|node| node.has_tag_name("rule"))
    {
        let part = |tag: &str| -> Result<Vec<Itemset>, PmmlError> {
            rule.children()
                .find(|node| node.has_tag_name(tag))
                .ok_or_else(|| PmmlError::Parse(format!("a Sequence rule has no {tag}")))
                .and_then(condition_itemsets)
        };
        let consequent = part("consequent")?;
        let confidence = rule
            .children()
            .find(|node| node.has_tag_name("confidence"))
            .and_then(|node| node.text())
            .and_then(|text| text.trim().parse::<f64>().ok())
            .ok_or_else(|| PmmlError::Parse("a Sequence rule has no confidence".into()))?;
        rules.push(SequenceRule {
            antecedent: part("antecedent")?,
            consequent,
            confidence,
            support: 0.0,
            occurrence: 0,
        });
    }
    Ok(rules)
}

/// The itemsets of a `SequenceCondition`: its `then` list, each an `and` of `equal`
/// conditions. Any other shape has no oracle and is an error.
fn condition_itemsets(condition: roxmltree::Node<'_, '_>) -> Result<Vec<Itemset>, PmmlError> {
    let bad = |what: &str| PmmlError::Parse(format!("sequence rules: {what}"));
    let children = condition
        .children()
        .find(|node| node.has_tag_name("children"))
        .ok_or_else(|| bad("a condition without children"))?;
    let mut sets = Vec::new();
    for logical in children
        .children()
        .filter(|node| node.has_tag_name("child"))
    {
        let items = logical
            .children()
            .find(|node| node.has_tag_name("children"))
            .ok_or_else(|| bad("an itemset without conditions"))?;
        let mut set = Vec::new();
        for item in items.children().filter(|node| node.has_tag_name("child")) {
            let text = |tag: &str| {
                item.children()
                    .find(|node| node.has_tag_name(tag))
                    .and_then(|node| node.text())
                    .map(str::to_string)
            };
            if text("op").as_deref() != Some("equal") {
                return Err(bad("a condition other than equal"));
            }
            set.push((
                text("field").ok_or_else(|| bad("a condition without a field"))?,
                text("value").unwrap_or_default(),
            ));
        }
        sets.push(set);
    }
    Ok(sets)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: &str = r#"<rule>
<antecedent><children><child><children>
<child><op>equal</op><field>item</field><value>zeta</value></child>
</children></child></children></antecedent>
<consequent><children><child><children>
<child><op>equal</op><field>item</field><value>amber</value></child>
</children></child></children></consequent>
<confidence>0.780487804878049</confidence>
</rule>"#;

    #[test]
    fn reads_the_rules_container() {
        let rules = parse_rules(RULES).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].antecedent,
            vec![vec![("item".to_string(), "zeta".to_string())]]
        );
        assert_eq!(
            rules[0].consequent,
            vec![vec![("item".to_string(), "amber".to_string())]]
        );
        assert!((rules[0].confidence - 0.780487804878049).abs() < 1e-15);
    }

    #[test]
    fn rejects_a_condition_other_than_equal() {
        let text = RULES.replacen("<op>equal</op>", "<op>greater</op>", 1);
        assert!(parse_rules(&text).is_err());
    }

    #[test]
    fn rejects_a_container_without_confidence() {
        let text = RULES.replace("<confidence>0.780487804878049</confidence>", "");
        assert!(parse_rules(&text).is_err());
    }
}
