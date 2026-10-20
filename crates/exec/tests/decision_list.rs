//! Compare actual builder output with saved Modeler PMML, independently of nugget fallback.
#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan, registry};
use dms_pmml::{BooleanOperator, Model, Operator, Pmml, Predicate, RuleSetModel};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::sync::Arc;

fn rules(pmml: &Pmml) -> &RuleSetModel {
    match &pmml.model {
        Model::RuleSet(m) => m,
        other => panic!("expected RuleSet: {other:?}"),
    }
}

fn train(label: &str, case: &str) -> Result<(Pmml, Pmml), String> {
    let root = support::repo_root();
    let bytes = std::fs::read(root.join(format!("corpus/{label}/{case}.str"))).unwrap();
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let auto = label == "auto_train";
    let kind = if auto {
        "BuildBinaryClassifierNode"
    } else {
        "BuildDecisionListNode"
    };
    let node = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == kind)
        .unwrap();
    let mut builder = node.clone();
    let oracle = if auto {
        // Exercise the actual Decision List factory with ac_lift's saved component
        // settings, not an imitation of Auto Classifier defaults or a saved-nugget fallback.
        let saved = stream
            .diagram
            .nodes
            .iter()
            .flat_map(|n| &n.result_items)
            .filter_map(|item| item.child("buildModelNode"))
            .find(|n| {
                n.attribute("class")
                    .is_some_and(|c| c.ends_with(".BuildDecisionListNode"))
            })
            .unwrap();
        builder.class = saved.attribute("class").unwrap().into();
        builder.settings = saved.child("properties").cloned();
        archive
            .attachments
            .iter()
            .filter_map(|a| std::str::from_utf8(&a.bytes).ok())
            .filter_map(|xml| Pmml::parse(xml).ok())
            .find(|p| matches!(p.model, Model::RuleSet(_)))
            .unwrap()
    } else {
        let path = std::fs::read_dir(root.join(format!("oracle/{label}/{case}/models")))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "xml"))
            .unwrap();
        Pmml::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
    };
    builder.result_items.clear();
    let context = ExecContext {
        files: FileResolver {
            data_dirs: vec![
                root.join(format!("corpus/{label}")),
                root.join("corpus/ibm-demo-data"),
            ],
        },
        stream_properties: stream.properties.clone().map(Arc::new),
        decimal_places: Some(15),
        ..ExecContext::default()
    };
    let plan = Plan::new(&stream);
    let upstream = plan.inputs_of(&node.uid)[0];
    let result = plan.evaluate(&[upstream], &context);
    let input = match result.get(upstream).unwrap().as_ref().unwrap() {
        NodeOutput::Data(table) => Arc::clone(table),
        other => panic!("{label}/{case}: {other:?}"),
    };
    if auto {
        assert_eq!(input.row_count(), 360);
    }
    let operator = registry::factory("decision_list").unwrap()(&builder).unwrap();
    let ours = match operator.produce(&[input], None, &context) {
        Ok(NodeOutput::Model(pmml)) => (*pmml).clone(),
        Err(error) => return Err(format!("{error:?}")),
        other => panic!("{label}/{case}: {other:?}"),
    };
    Ok((ours, oracle))
}

#[test]
fn trained_lists_match_twenty_oracles_in_full_ir() {
    let mut models = 0;
    let mut rule_count = 0;
    for (label, cases) in [
        (
            "dl_discriminate",
            &[
                "dl01_duplicates",
                "dl02_final_prefix",
                "dl03_canonical",
                "dl04_coverage",
                "dl06_preprune",
                "dl07_interaction",
                "dl08_completed",
                "dl09_prefix_canonical",
                "dl10_tie_sizes",
                "dl11_tie_equal",
                "dl12_tie_reorder",
            ][..],
        ),
        (
            "train_next",
            &[
                "dl_default",
                "dl_max_rules2",
                "dl_max_rules4",
                "dl_search",
                "dl_min_group",
                "dl_confidence80",
                "dl_numeric",
            ][..],
        ),
        ("models_next2", &["dl_min_group"][..]),
        ("auto_train", &["ac_lift"][..]),
    ] {
        for case in cases {
            let (ours, oracle) = train(label, case).unwrap();
            assert_eq!(rules(&ours), rules(&oracle), "{label}/{case}");
            models += 1;
            rule_count += rules(&ours).rules.len();
            println!(
                "{label}/{case}: {} rules equal (including remainder)",
                rules(&ours).rules.len()
            );
        }
    }
    assert_eq!(models, 20);
    println!("{models} exact RuleSet models, {rule_count} rules including remainders");
}

#[test]
fn completed_support_tie_is_refused_but_capped_tie_is_resolved() {
    let error = train("models_next2", "dl_default").unwrap_err();
    assert!(
        error.contains("GAP: Decision List unresolved final list tie"),
        "{error}"
    );
    let (ours, oracle) = train("models_next2", "dl_max_rules").unwrap();
    assert_eq!(rules(&ours), rules(&oracle));
}

#[test]
fn dl05_retains_the_documented_partition_counterexample() {
    let (ours, oracle) = train("dl_discriminate", "dl05_carry").unwrap();
    let conjunction = |fields: &[(&str, &str)]| Predicate::Compound {
        operator: BooleanOperator::And,
        predicates: fields
            .iter()
            .map(|(field, value)| Predicate::Simple {
                field: (*field).into(),
                operator: Operator::Equal,
                value: Some((*value).into()),
            })
            .collect(),
    };
    let actual: Vec<_> = rules(&ours)
        .rules
        .iter()
        .map(|r| (r.predicate.clone(), r.record_count, r.correct, r.disabled))
        .collect();
    assert_eq!(
        actual,
        vec![
            (
                conjunction(&[("b", "v0"), ("c", "v0")]),
                Some(140.0),
                Some(118.0),
                false
            ),
            (
                conjunction(&[("a", "v1"), ("b", "v1")]),
                Some(180.0),
                Some(120.0),
                false
            ),
            (
                conjunction(&[("b", "v0"), ("a", "v0")]),
                Some(40.0),
                Some(36.0),
                false
            ),
            (Predicate::True, Some(300.0), Some(10.0), true),
        ]
    );
    assert_eq!(
        rules(&oracle).rules[0].predicate,
        conjunction(&[("a", "v1"), ("c", "v0")])
    );
    assert_ne!(rules(&ours), rules(&oracle));
}

#[test]
fn unverified_search_and_weight_settings_are_rejected() {
    use dms_strfmt::fastinfoset::Node;
    let bytes =
        std::fs::read(support::repo_root().join("corpus/train_next/dl_default.str")).unwrap();
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "BuildDecisionListNode")
        .unwrap();
    let factory = dms_exec::registry::factory("decision_list").unwrap();
    assert!(dms_exec::registry::is_implemented("decision_list"));
    for (key, value) in [
        ("searchDirection", "down"),
        ("reuseFields", "false"),
        ("useFrequency", "true"),
        ("useWeight", "true"),
        ("useEvents", "true"),
        ("useInteractiveState", "true"),
    ] {
        let mut node = builder.clone();
        let element = node
            .settings
            .as_mut()
            .unwrap()
            .children
            .iter_mut()
            .find_map(|c| match c {
                Node::Element(e) if &*e.name.local == key => Some(e),
                _ => None,
            })
            .unwrap();
        element.children = vec![Node::Text(value.into())];
        assert!(factory(&node).is_err(), "{key}={value}");
    }
}
