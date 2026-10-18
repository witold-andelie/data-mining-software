//! Apriori build nodes train the rule set Modeler exported.
//!
//! `cargo xtask compare` scores the nugget. A failed builder would leave the nugget its saved
//! PMML, so this test compares the model this process trained with Modeler's file: rule id,
//! antecedent and consequent items, support, confidence and lift.

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

mod support;

use dms_exec::NodeOutput;
use dms_pmml::{AssociationModel, AssociationRule, Model, Pmml};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::collections::HashMap;
use std::sync::Arc;

#[test]
fn trained_rules_match_modeler() {
    for case in [
        "apriori_default",
        "apriori_thresholds",
        "apriori_transactional",
    ] {
        let ours = train(case);
        let oracle = oracle_model(case);
        let diff = difference(&ours, &oracle);
        assert!(diff.is_none(), "{case}: {}", diff.unwrap_or_default());
    }
}

fn train(case: &str) -> AssociationModel {
    let root = support::repo_root();
    let bytes = std::fs::read(root.join(format!("corpus/models_next/{case}.str"))).unwrap();
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|node| node.type_identifier() == "BuildAprioriNode")
        .map(|node| node.uid.clone())
        .unwrap();
    let attachments: HashMap<String, Arc<[u8]>> = archive
        .attachments
        .into_iter()
        .map(|attachment| (attachment.name, attachment.bytes.into()))
        .collect();
    let context = dms_exec::ExecContext {
        files: dms_exec::FileResolver {
            data_dirs: vec![root.join("corpus/models_next")],
        },
        stream_properties: stream.properties.clone().map(Arc::new),
        attachments: Arc::new(attachments),
        decimal_places: Some(15),
        export_dir: None,
    };
    let results = dms_exec::Plan::new(&stream).evaluate(&[builder.as_str()], &context);
    match results.get(builder.as_str()).unwrap() {
        Ok(NodeOutput::Model(model)) => association(model),
        other => panic!("{case}: {other:?}"),
    }
}

fn oracle_model(case: &str) -> AssociationModel {
    let dir = support::repo_root().join(format!("oracle/models_next/{case}/models"));
    let xml = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "xml"))
        .unwrap();
    let pmml = Pmml::parse(&std::fs::read_to_string(xml).unwrap()).unwrap();
    association(&pmml)
}

fn association(pmml: &Pmml) -> AssociationModel {
    match &pmml.model {
        Model::Association(model) => model.clone(),
        other => panic!("not an association model: {other:?}"),
    }
}

fn difference(ours: &AssociationModel, oracle: &AssociationModel) -> Option<String> {
    if ours.number_of_transactions != oracle.number_of_transactions {
        return Some(format!(
            "transactions {} vs {}",
            ours.number_of_transactions, oracle.number_of_transactions
        ));
    }
    if (ours.minimum_support - oracle.minimum_support).abs() > 1e-12 {
        return Some(format!(
            "minimumSupport {} vs {}",
            ours.minimum_support, oracle.minimum_support
        ));
    }
    // Modeler truncates the model attribute (5/7 is `0.714285714285714` on the model and
    // `0.7142857142857143` on rule 4 of `apriori_thresholds`).
    if (ours.minimum_confidence - oracle.minimum_confidence).abs() > 1e-15 {
        return Some(format!(
            "minimumConfidence {} vs {}",
            ours.minimum_confidence, oracle.minimum_confidence
        ));
    }
    if ours.rules.len() != oracle.rules.len() {
        return Some(format!(
            "{} rules vs {}",
            ours.rules.len(),
            oracle.rules.len()
        ));
    }
    let schema = |model: &AssociationModel| {
        model
            .header
            .mining_schema
            .iter()
            .map(|field| (field.name.clone(), field.usage, field.direction.clone()))
            .collect::<Vec<_>>()
    };
    if schema(ours) != schema(oracle) {
        return Some(format!(
            "mining schema {:?} vs {:?}",
            schema(ours),
            schema(oracle)
        ));
    }
    for (index, oracle_rule) in oracle.rules.iter().enumerate() {
        let Some(rule) = ours.rules.get(index) else {
            return Some(format!("missing rule {}", oracle_rule.id));
        };
        if let Some(diff) = rule_diff(ours, rule, oracle, oracle_rule) {
            return Some(diff);
        }
    }
    for itemset in &oracle.itemsets {
        let Some(ours_set) = ours.itemsets.iter().find(|set| set.id == itemset.id) else {
            return Some(format!("missing itemset {}", itemset.id));
        };
        if ours_set.items != itemset.items || (ours_set.support - itemset.support).abs() > 1e-12 {
            return Some(format!(
                "itemset {} {:?} support {} vs {:?} {}",
                itemset.id, ours_set.items, ours_set.support, itemset.items, itemset.support
            ));
        }
    }
    Ok::<(), String>(()).err()
}

fn rule_diff(
    ours: &AssociationModel,
    rule: &AssociationRule,
    oracle: &AssociationModel,
    oracle_rule: &AssociationRule,
) -> Option<String> {
    if rule.id != oracle_rule.id {
        return Some(format!("rule id {} vs {}", rule.id, oracle_rule.id));
    }
    let antecedent = ours.itemset_values(&rule.antecedent).ok()?;
    let consequent = ours.itemset_values(&rule.consequent).ok()?;
    let oracle_antecedent = oracle.itemset_values(&oracle_rule.antecedent).ok()?;
    let oracle_consequent = oracle.itemset_values(&oracle_rule.consequent).ok()?;
    if antecedent != oracle_antecedent || consequent != oracle_consequent {
        return Some(format!(
            "rule {} {:?} => {:?} vs {:?} => {:?}",
            rule.id, antecedent, consequent, oracle_antecedent, oracle_consequent
        ));
    }
    if !close(rule.support, oracle_rule.support)
        || !close(rule.confidence, oracle_rule.confidence)
        || !close(rule.lift, oracle_rule.lift)
    {
        return Some(format!(
            "rule {} support {} confidence {} lift {} vs {} {} {}",
            rule.id,
            rule.support,
            rule.confidence,
            rule.lift,
            oracle_rule.support,
            oracle_rule.confidence,
            oracle_rule.lift
        ));
    }
    None
}

/// Exact, or one ulp. `apriori_thresholds` rule 6 prints confidence `0.7777777777777779`.
fn close(left: f64, right: f64) -> bool {
    left.to_bits().abs_diff(right.to_bits()) <= 1 && left.signum() == right.signum()
}
