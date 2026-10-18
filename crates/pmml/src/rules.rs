//! PMML 4.3 RuleSet: DMG RuleSet.html; Modeler Algorithms Guide p.51.
//! `models_ml/c50_rules_{voting,firsthit}` both export weightedSum, including overlap.

use crate::parse::{opt_f64_attr, parse_model_header, parse_predicate, req_attr};
use crate::score::{Env, empty_prediction, target_categories};
use crate::tree::{Truth, eval_predicate};
use crate::{
    BooleanOperator, Pmml, PmmlError, Predicate, Prediction, RuleSetModel, SimpleRule, Value,
};

pub(crate) fn parse(node: roxmltree::Node<'_, '_>) -> Result<RuleSetModel, PmmlError> {
    let set = node
        .children()
        .find(|n| n.has_tag_name("RuleSet"))
        .ok_or_else(|| PmmlError::Parse("RuleSetModel has no RuleSet".into()))?;
    let selection = set
        .children()
        .find(|n| n.has_tag_name("RuleSelectionMethod"))
        .ok_or_else(|| PmmlError::Parse("RuleSet has no RuleSelectionMethod".into()))?;
    let mut rules = Vec::new();
    for child in set.children().filter(roxmltree::Node::is_element) {
        if matches!(child.tag_name().name(), "SimpleRule" | "CompoundRule") {
            parse_rule(child, &[], &mut rules, 0)?;
        }
    }
    Ok(RuleSetModel {
        header: parse_model_header(node)?,
        selection: req_attr(selection, "criterion")?.into(),
        default_score: set.attribute("defaultScore").map(str::to_string),
        default_confidence: opt_f64_attr(set, "defaultConfidence")?,
        rules,
    })
}

fn parse_rule(
    node: roxmltree::Node<'_, '_>,
    ancestors: &[Predicate],
    rules: &mut Vec<SimpleRule>,
    depth: usize,
) -> Result<(), PmmlError> {
    if depth > 64 || rules.len() >= 100_000 {
        return Err(PmmlError::Parse("RuleSet depth/rule limit exceeded".into()));
    }
    let p = node
        .children()
        .find(|n| {
            matches!(
                n.tag_name().name(),
                "True" | "False" | "SimplePredicate" | "SimpleSetPredicate" | "CompoundPredicate"
            )
        })
        .ok_or_else(|| PmmlError::Parse("rule has no predicate".into()))?;
    let mut predicates = ancestors.to_vec();
    predicates.push(parse_predicate(p, 0)?);
    if node.has_tag_name("CompoundRule") {
        for c in node.children().filter(|n| {
            n.is_element() && matches!(n.tag_name().name(), "SimpleRule" | "CompoundRule")
        }) {
            parse_rule(c, &predicates, rules, depth + 1)?;
        }
    } else {
        let confidence = opt_f64_attr(node, "confidence")?.unwrap_or(1.0);
        let weight = opt_f64_attr(node, "weight")?.unwrap_or(1.0);
        if !(0.0..=1.0).contains(&confidence) || weight < 0.0 {
            return Err(PmmlError::Parse(
                "rule confidence/weight out of range".into(),
            ));
        }
        let predicate = if predicates.len() == 1 {
            predicates.remove(0)
        } else {
            Predicate::Compound {
                operator: BooleanOperator::And,
                predicates,
            }
        };
        rules.push(SimpleRule {
            id: node.attribute("id").map(str::to_string),
            predicate,
            score: req_attr(node, "score")?.into(),
            confidence,
            weight,
            record_count: opt_f64_attr(node, "recordCount")?,
            correct: opt_f64_attr(node, "nbCorrect")?,
            disabled: node.children().any(|n| {
                n.has_tag_name("Extension")
                    && n.attribute("name") == Some("isDisabled")
                    && n.attribute("value") == Some("true")
            }),
        });
    }
    Ok(())
}

/// Decision List (`algorithmName="DecisionList"`, `ibm-demos/pm_decisionlist`): the first rule
/// whose predicate holds, disabled rules included, as its index; `None` when none holds.
pub(crate) fn first_hit(model: &RuleSetModel, env: &Env<'_>) -> Result<Option<usize>, PmmlError> {
    for (index, rule) in model.rules.iter().enumerate() {
        if eval_predicate(&rule.predicate, env, 0)? == Truth::True {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

pub(crate) fn score(
    pmml: &Pmml,
    model: &RuleSetModel,
    env: &Env<'_>,
) -> Result<Prediction, PmmlError> {
    if !matches!(
        model.selection.as_str(),
        "firstHit" | "weightedMax" | "weightedSum"
    ) {
        return Err(PmmlError::Unsupported(format!(
            "RuleSelectionMethod {:?}",
            model.selection
        )));
    }
    let mut fired = Vec::new();
    for rule in &model.rules {
        if eval_predicate(&rule.predicate, env, 0)? == Truth::True {
            fired.push(rule);
        }
    }
    if fired.is_empty() {
        return Ok(Prediction {
            value: model
                .default_score
                .as_ref()
                .map_or(Value::Missing, |s| Value::Text(s.clone())),
            confidence: model.default_confidence,
            ..empty_prediction()
        });
    }
    if model.selection != "weightedSum" {
        let mut chosen = fired
            .first()
            .copied()
            .ok_or_else(|| PmmlError::Score("no firing rule".into()))?;
        if model.selection == "weightedMax" {
            for rule in &fired {
                if rule.weight > chosen.weight {
                    chosen = rule;
                }
            }
        }
        return Ok(Prediction {
            value: Value::Text(chosen.score.clone()),
            confidence: Some(chosen.confidence),
            entity_id: chosen.id.clone(),
            ..empty_prediction()
        });
    }
    let mut classes: Vec<String> = target_categories(pmml, &model.header)
        .into_iter()
        .map(str::to_string)
        .collect();
    for rule in &fired {
        if !classes.contains(&rule.score) {
            classes.push(rule.score.clone());
        }
    }
    let mut best: Option<(String, f64, f64)> = None;
    for class in classes {
        let weight: f64 = fired
            .iter()
            .filter(|r| r.score == class)
            .map(|r| r.weight)
            .sum();
        let confidence: f64 = fired
            .iter()
            .filter(|r| r.score == class)
            .map(|r| r.confidence)
            .sum();
        if best.as_ref().is_none_or(|(_, w, _)| weight > *w) {
            best = Some((class, weight, confidence));
        }
    }
    let Some((class, _, confidence)) = best else {
        return Ok(empty_prediction());
    };
    Ok(Prediction {
        value: Value::Text(class),
        confidence: Some(confidence / fired.len() as f64),
        ..empty_prediction()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(method: &str) -> String {
        format!(
            r#"<PMML version="4.3"><DataDictionary>
        <DataField name="x" dataType="double" optype="continuous"/>
        <DataField name="y" dataType="string" optype="categorical"><Value value="a"/><Value value="b"/></DataField>
        </DataDictionary><RuleSetModel functionName="classification"><MiningSchema><MiningField name="x"/><MiningField name="y" usageType="target"/></MiningSchema>
        <RuleSet defaultScore="b" defaultConfidence="0.25"><RuleSelectionMethod criterion="{method}"/>
        <SimpleRule id="one" score="a" confidence="0.8" weight="0.8"><SimplePredicate field="x" operator="greaterThan" value="0"/></SimpleRule>
        <CompoundRule><SimplePredicate field="x" operator="lessThan" value="10"/>
        <SimpleRule score="b" confidence="0.6" weight="0.6"><True/></SimpleRule>
        <SimpleRule score="b" confidence="0.5" weight="0.5"><True/></SimpleRule>
        </CompoundRule></RuleSet></RuleSetModel></PMML>"#
        )
    }

    #[test]
    fn rule_voting_first_hit_maximum_and_unknown() {
        for (method, class, confidence) in [
            ("weightedSum", "b", 1.1 / 3.0),
            ("firstHit", "a", 0.8),
            ("weightedMax", "a", 0.8),
        ] {
            let m = Pmml::parse(&document(method)).unwrap();
            let scorer = m.bind(&["x"]).unwrap();
            let p = scorer.score(&[Value::Number(1.0)]).unwrap();
            assert_eq!(p.value, Value::Text(class.into()));
            assert!((p.confidence.unwrap() - confidence).abs() < 1e-15);
            let missing = scorer.score(&[Value::Missing]).unwrap();
            assert_eq!(missing.value, Value::Text("b".into()));
            assert_eq!(missing.confidence, Some(0.25));
        }
    }

    #[test]
    fn malformed_and_deep_rules_and_models_do_not_panic() {
        let seed = document("weightedSum");
        for replacement in ["NaN", "inf", "-1", "2"] {
            assert!(
                Pmml::parse(&seed.replace(
                    "confidence=\"0.8\"",
                    &format!("confidence=\"{replacement}\"")
                ))
                .is_err()
            );
        }
        // Deterministic mutation fuzzing: truncations, deletion, replacement and insertion,
        // including Unicode; parse and score every mutation that remains well formed.
        let chars: Vec<char> = seed.chars().collect();
        for i in (0..chars.len()).step_by(3) {
            for operation in 0..4 {
                let mut text = chars.clone();
                match operation {
                    0 => text.truncate(i),
                    1 => {
                        text.remove(i);
                    }
                    2 => text[i] = 'λ',
                    _ => text.insert(i, '<'),
                }
                let text: String = text.into_iter().collect();
                if let Ok(m) = Pmml::parse(&text)
                    && let Ok(s) = m.bind(&["x"])
                {
                    let _ = s.score(&[Value::Number(f64::NAN)]);
                    let _ = s.score(&[Value::Missing]);
                }
            }
        }
        let rule = format!(
            "{}<SimpleRule score=\"a\"><True/></SimpleRule>{}",
            "<CompoundRule><True/>".repeat(70),
            "</CompoundRule>".repeat(70)
        );
        let deep = seed.replace(
            "<CompoundRule><SimplePredicate",
            &format!("{rule}<CompoundRule><SimplePredicate"),
        );
        assert!(Pmml::parse(&deep).is_err());
        let mut inner = "<RuleSetModel functionName=\"classification\"><MiningSchema/><RuleSet><RuleSelectionMethod criterion=\"firstHit\"/></RuleSet></RuleSetModel>".to_string();
        for _ in 0..80 {
            inner = format!(
                "<MiningModel functionName=\"classification\"><MiningSchema/><Segmentation multipleModelMethod=\"selectFirst\"><Segment><True/>{inner}</Segment></Segmentation></MiningModel>"
            );
        }
        assert!(
            Pmml::parse(&format!(
                "<PMML version=\"4.3\"><DataDictionary/>{inner}</PMML>"
            ))
            .is_err()
        );
    }
}
