//! Auto nugget combination, setting validation, and scoring-only registration.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_exec::ExecContext;
use dms_exec::nodes::apply;

use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Attribute, Element, Node, QualifiedName};
use dms_types::{DataModel, Field, Measurement, Storage, Table, Value, ValueSpec};
use std::collections::HashMap;
use std::sync::Arc;

fn name(local: &str) -> QualifiedName {
    QualifiedName {
        prefix: None,
        namespace: None,
        local: Arc::from(local),
    }
}

fn element(local: &str, attributes: Vec<(&str, &str)>, children: Vec<Node>) -> Element {
    Element {
        name: name(local),
        namespace_declarations: Vec::new(),
        attributes: attributes
            .into_iter()
            .map(|(n, v)| Attribute {
                name: name(n),
                value: Arc::from(v),
            })
            .collect(),
        children,
    }
}

fn p(local: &str, text: &str) -> Node {
    Node::Element(element(
        local,
        Vec::new(),
        vec![Node::Text(Arc::from(text))],
    ))
}

fn nugget(class: &str, settings: Vec<Node>) -> StreamNode {
    StreamNode {
        uid: "nugget".into(),
        class: format!("com.spss.clementine.component.node.{class}"),
        extension: None,
        model_builder: None,
        name: None,
        label: None,
        position: None,
        properties_attachment: None,
        containers: Vec::new(),
        settings: Some(element("properties", Vec::new(), settings)),
        binning_output: None,
        result_items: Vec::new(),
        subdiagram: None,
    }
}

fn auto(class: &str, kind: &str, labels: &[(&str, bool)]) -> (StreamNode, ExecContext) {
    let model = element(
        "model",
        vec![("useMultipleModels", "true")],
        vec![
            p("applicationVersion", "19"),
            Node::Element(element("targetField", vec![], vec![p("name", "y")])),
        ],
    );
    let mut node = nugget(
        class,
        vec![
            Node::Element(model),
            p("ensembleFlagsMethod", "confidenceWeightedVoting"),
            p("ensembleFlagTiesMethod", "random"),
            p("filterEnsembledFields", "true"),
        ],
    );
    let mut attachments = HashMap::new();
    for (i, (label, missing)) in labels.iter().enumerate() {
        let member = format!("m{i}");
        let predicate = if *missing {
            r#"<SimplePredicate field="x" operator="greaterThan" value="0"/>"#
        } else {
            "<True/>"
        };
        let xml = format!(
            r#"<PMML version="4.3"><DataDictionary><DataField name="x" dataType="double" optype="continuous"/><DataField name="y" dataType="string" optype="categorical"><Value value="N"/><Value value="Y"/></DataField></DataDictionary><TreeModel functionName="classification"><MiningSchema><MiningField name="x"/><MiningField name="y" usageType="target"/></MiningSchema><Node score="{label}" recordCount="10">{predicate}<ScoreDistribution value="{label}" recordCount="9" confidence="0.9"/></Node></TreeModel></PMML>"#
        );
        attachments.insert(member.clone(), Arc::from(xml.into_bytes()));
        let output = element(
            "modelOutput",
            vec![("class", kind)],
            vec![Node::Element(element(
                "properties",
                vec![],
                vec![Node::Element(element(
                    "model",
                    vec![("pmml", &member)],
                    vec![],
                ))],
            ))],
        );
        node.result_items.push(element(
            "resultItem",
            vec![("useModel", "true")],
            vec![Node::Element(output)],
        ));
    }
    (
        node,
        ExecContext {
            attachments: Arc::new(attachments),
            ..ExecContext::default()
        },
    )
}

fn input() -> Arc<Table> {
    let mut y = Field::new("y", Storage::String);
    y.measurement = Measurement::Flag;
    y.values = ValueSpec::Flag {
        true_value: Value::string("Y"),
        false_value: Value::string("N"),
    };
    Arc::new(
        Table::new(
            DataModel::new(vec![Field::new("x", Storage::Real), y]).unwrap(),
            vec![vec![Value::real(0.0), Value::string("N")]],
        )
        .unwrap(),
    )
}

#[test]
fn random_ties_fail_with_settings_gap() {
    let (node, ctx) = auto(
        "ApplyAutoClassifierNode",
        "CartRuleOutput",
        &[("N", false), ("Y", false)],
    );
    let Err(dms_exec::OperatorError::Settings(message)) =
        apply::build(&node).unwrap().run(&[input()], &ctx)
    else {
        panic!("tie must fail")
    };
    assert!(
        message.contains("GAP:") && message.contains("random"),
        "{message}"
    );
}

#[test]
fn binary_identifier_and_null_component_use_shared_ensemble_arithmetic() {
    let (node, ctx) = auto(
        "ApplyBinaryClassifierNode",
        "CartRuleOutput",
        &[("N", false), ("Y", true)],
    );
    let output = apply::build(&node).unwrap().run(&[input()], &ctx).unwrap();
    assert_eq!(output.rows()[0][2], Value::string("N"));
    assert_eq!(output.rows()[0][3], Value::real(0.9));
    assert_eq!(output.model().len(), 4);
}

#[test]
fn unsupported_settings_and_legacy_versions_are_gaps() {
    for (key, value) in [
        ("ensembleFlagsMethod", "highestConfidence"),
        ("ensembleFlagsMethod", "unknown"),
        ("filterEnsembledFields", "false"),
        ("useSmartScore", "false"),
        ("isEnsembleUpdate", "true"),
        ("newScoringOption", "true"),
        ("ensembleFlagTiesMethod", "unknown"),
    ] {
        let (mut node, _) = auto("ApplyAutoClassifierNode", "CartRuleOutput", &[("N", false)]);
        let settings = node.settings.as_mut().unwrap();
        settings
            .children
            .retain(|e| !matches!(e,Node::Element(e) if e.name.local.as_ref()==key));
        settings.children.push(p(key, value));
        let Err(dms_exec::OperatorError::Settings(message)) = apply::build(&node) else {
            panic!("{key}={value} must fail")
        };
        assert!(message.contains("GAP:"), "{message}");
    }
    let (mut node, _) = auto("ApplyAutoClassifierNode", "CartRuleOutput", &[("N", false)]);
    let model = node
        .settings
        .as_mut()
        .unwrap()
        .children
        .iter_mut()
        .find_map(|e| match e {
            Node::Element(e) if e.name.local.as_ref() == "model" => Some(e),
            _ => None,
        })
        .unwrap();
    model.children[0] = p("applicationVersion", "14");
    let Err(dms_exec::OperatorError::Settings(message)) = apply::build(&node) else {
        panic!("legacy version must fail")
    };
    assert!(message.contains("GAP:") && message.contains("legacy"));
}

#[test]
fn auto_builders_remain_scoring_only() {
    for (id, class) in [
        ("auto_classifier", "BuildBinaryClassifierNode"),
        ("auto_numeric", "BuildNumericClassifierNode"),
    ] {
        assert!(!dms_exec::registry::is_implemented(id));
        let node = nugget(class, vec![]);
        let Err(dms_exec::OperatorError::Settings(message)) =
            dms_exec::registry::factory(id).unwrap()(&node)
        else {
            panic!("builder must fail")
        };
        assert!(message.starts_with("GAP:"), "{message}");
    }
}
