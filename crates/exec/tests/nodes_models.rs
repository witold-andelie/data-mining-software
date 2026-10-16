//! Model nuggets scoring with a saved PMML attachment or with the model their build node trained.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_exec::nodes::apply;
use dms_exec::{ExecContext, NodeOutput};
use dms_pmml::Pmml;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Attribute, Element, Node, QualifiedName};
use dms_types::{DataModel, Field, Storage, Table, Value};
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
        subdiagram: None,
    }
}

/// A one-split tree on `age` predicting the flag `risk` (N/Y).
fn tree(threshold: &str) -> String {
    format!(
        r#"<PMML xmlns="http://www.dmg.org/PMML-4_3" version="4.3">
  <DataDictionary numberOfFields="2">
    <DataField name="risk" optype="categorical" dataType="string"><Value value="N"/><Value value="Y"/></DataField>
    <DataField name="age" optype="continuous" dataType="integer"/>
  </DataDictionary>
  <TreeModel functionName="classification" algorithmName="C&amp;RT">
    <MiningSchema><MiningField name="risk" usageType="predicted"/><MiningField name="age"/></MiningSchema>
    <Node id="0" score="N" recordCount="10"><True/>
      <ScoreDistribution value="N" recordCount="6" confidence="0.6"/>
      <ScoreDistribution value="Y" recordCount="4" confidence="0.4"/>
      <Node id="1" score="N" recordCount="6"><SimplePredicate field="age" operator="lessOrEqual" value="{threshold}"/>
        <ScoreDistribution value="N" recordCount="5" confidence="0.8"/>
        <ScoreDistribution value="Y" recordCount="1" confidence="0.2"/>
      </Node>
      <Node id="2" score="Y" recordCount="4"><SimplePredicate field="age" operator="greaterThan" value="{threshold}"/>
        <ScoreDistribution value="N" recordCount="1" confidence="0.25"/>
        <ScoreDistribution value="Y" recordCount="3" confidence="0.75"/>
      </Node>
    </Node>
  </TreeModel>
</PMML>"#
    )
}

fn input(with_previous_score: bool) -> Table {
    let mut fields = vec![Field::new("age", Storage::Integer)];
    if with_previous_score {
        fields.push(Field::new("$R-risk", Storage::String));
    }
    let rows = [30, 60, 0]
        .iter()
        .map(|age| {
            let mut row = vec![if *age == 0 {
                Value::Null
            } else {
                Value::Integer(*age)
            }];
            if with_previous_score {
                row.push(Value::string("x"));
            }
            row
        })
        .collect();
    Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
}

fn context(pmml: &str) -> ExecContext {
    let mut attachments: HashMap<String, Arc<[u8]>> = HashMap::new();
    attachments.insert("data/0001.dat".into(), pmml.as_bytes().into());
    ExecContext {
        attachments: Arc::new(attachments),
        ..ExecContext::default()
    }
}

fn settings(raw: bool) -> Vec<Node> {
    vec![
        p("calculateConfidences", "true"),
        p(
            "calculateRawPropensities",
            if raw { "true" } else { "false" },
        ),
        Node::Element(element(
            "model",
            vec![("pmml", "data/0001.dat")],
            Vec::new(),
        )),
    ]
}

fn column(table: &Table, name: &str) -> Vec<Value> {
    table.column(name).unwrap().cloned().collect()
}

#[test]
fn nugget_scores_with_its_saved_pmml() {
    let operator = apply::build(&nugget("ApplyChaidRuleNode", settings(true))).unwrap();
    let out = operator
        .run(&[Arc::new(input(false))], &context(&tree("40")))
        .unwrap();
    let names: Vec<&str> = out
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, ["age", "$R-risk", "$RC-risk", "$RRP-risk"]);
    assert_eq!(
        column(&out, "$R-risk"),
        // A missing age satisfies no split; the default strategy predicts nothing.
        [Value::string("N"), Value::string("Y"), Value::Null]
    );
}

#[test]
fn a_second_model_on_the_same_target_gets_an_index() {
    let operator = apply::build(&nugget("ApplyCartRuleNode", settings(false))).unwrap();
    let out = operator
        .run(&[Arc::new(input(true))], &context(&tree("40")))
        .unwrap();
    let names: Vec<&str> = out
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, ["age", "$R-risk", "$R1-risk", "$RC1-risk"]);
}

#[test]
fn the_build_nodes_model_replaces_the_saved_one() {
    let operator = apply::build(&nugget("ApplyCartRuleNode", settings(false))).unwrap();
    let trained = Arc::new(Pmml::parse(&tree("70")).unwrap());
    let out = operator
        .produce(
            &[Arc::new(input(false))],
            Some(trained),
            &context(&tree("40")),
        )
        .unwrap();
    let NodeOutput::Data(out) = out else {
        panic!("a nugget produces data")
    };
    // age 60 is <= 70 with the trained model, > 40 with the saved one.
    assert_eq!(column(&out, "$R-risk")[1], Value::string("N"));
}

#[test]
fn unsupported_nuggets_and_missing_models_are_errors() {
    assert!(apply::build(&nugget("ApplyNeuralNetNode", Vec::new())).is_err());
    let operator = apply::build(&nugget("ApplyCartRuleNode", Vec::new())).unwrap();
    assert!(
        operator
            .run(&[Arc::new(input(false))], &ExecContext::default())
            .is_err()
    );
}
