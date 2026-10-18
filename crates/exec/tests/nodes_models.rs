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
        binning_output: None,
        result_items: Vec::new(),
        subdiagram: None,
    }
}

fn analytic_nugget(extension: &str, kind: &str, settings: Vec<Node>) -> StreamNode {
    let mut node = nugget("ExtensionApplyModelNode", settings);
    node.extension = Some(dms_stream::Extension::Element {
        extension_id: extension.into(),
        element_id: kind.into(),
    });
    node
}

#[test]
fn analytic_nuggets_reject_uncovered_settings() {
    for (extension, kind) in [
        ("spss.randomTrees", "applyRandomTrees"),
        ("spss.treeEngine", "applyTree"),
        ("spss.linearEngine", "applyLinearEngine"),
        ("spss.linearSVM", "applyLinearSVM"),
    ] {
        for (name, value) in [
            ("calculateRawPropensities", "true"),
            ("calculateAdjustedPropensities", "true"),
            ("_calculateRawProp", "true"),
            ("_calculate_conf", "bogus"),
            ("_display_rule_id", "bogus"),
            ("_GenerateSQL", "udf"),
        ] {
            let node = analytic_nugget(extension, kind, vec![p(name, value)]);
            let Err(dms_exec::OperatorError::Settings(message)) = apply::build(&node) else {
                panic!("{extension}!{kind} must reject {name}={value}");
            };
            assert!(message.contains("GAP:"), "{message}");
        }
    }
}

#[test]
fn analytic_confidence_flags_and_replacement_link_schema() {
    // `rt_noconf`, `rt_batch`: a detached copy obeys false, the automatic
    // replacement-linked nugget has the confidence column in the oracle export.
    for (linked, confidence) in [(false, false), (true, true)] {
        let mut props = settings(false);
        props.push(p("_calculate_conf", "false"));
        if linked {
            props.push(p("refreshLinkState", "TurnOnReplaceLink"));
            props.push(p("repalcebuilderNodeId", "builder"));
        }
        let node = analytic_nugget("spss.randomTrees", "applyRandomTrees", props);
        let output = apply::build(&node)
            .unwrap()
            .run(&[Arc::new(input(false))], &context(&tree("40")))
            .unwrap();
        assert_eq!(output.model().position("$RC-risk").is_some(), confidence);
    }
    for enabled in [false, true] {
        let mut props = settings(false);
        props.push(p(
            "_display_rule_id",
            if enabled { "true" } else { "false" },
        ));
        let node = analytic_nugget("spss.treeEngine", "applyTree", props);
        let output = apply::build(&node)
            .unwrap()
            .run(&[Arc::new(input(false))], &context(&tree("40")))
            .unwrap();
        assert_eq!(output.model().position("$RI-risk").is_some(), enabled);
        if enabled {
            assert_eq!(
                column(&output, "$RI-risk"),
                [Value::Integer(1), Value::Integer(2), Value::Null]
            );
        }
    }
}

#[test]
fn analytic_builders_are_scoring_only_and_fail_with_gap() {
    for (id, extension, builder, applier) in [
        ("random_trees", "spss.randomTrees", "RT", "applyRandomTrees"),
        ("tree_as", "spss.treeEngine", "treeEngine", "applyTree"),
        (
            "linear_as",
            "spss.linearEngine",
            "linearEngine",
            "applyLinearEngine",
        ),
        ("lsvm", "spss.linearSVM", "linearSVM", "applyLinearSVM"),
    ] {
        assert!(!dms_exec::registry::is_implemented(id));
        let factory = dms_exec::registry::factory(id).unwrap();
        let mut node = analytic_nugget(extension, builder, vec![]);
        node.class = "com.spss.clementine.component.node.ExtensionBuildModelNode".into();
        let Err(dms_exec::OperatorError::Settings(message)) = factory(&node) else {
            panic!("{id} builder must fail");
        };
        assert!(message.starts_with("GAP:"));
        assert!(factory(&analytic_nugget(extension, applier, vec![])).is_ok());
    }
}

fn as_zip_pmml(member: &str, data: &[u8]) -> Vec<u8> {
    // Stored local ZIP record, with the four-byte big-endian AS length prefix.
    let mut zip = vec![0; 30];
    zip[..4].copy_from_slice(b"PK\x03\x04");
    zip[18..22].copy_from_slice(&(data.len() as u32).to_le_bytes());
    zip[22..26].copy_from_slice(&(data.len() as u32).to_le_bytes());
    zip[26..28].copy_from_slice(&(member.len() as u16).to_le_bytes());
    zip.extend_from_slice(member.as_bytes());
    zip.extend_from_slice(data);
    let mut wrapped = (zip.len() as u32).to_be_bytes().to_vec();
    wrapped.extend(zip);
    wrapped
}

#[test]
fn analytic_container_reads_pmml_xml_and_handles_corruption() {
    let mut node = analytic_nugget("spss.treeEngine", "applyTree", vec![]);
    node.containers.push(dms_stream::Container {
        name: Some("PMML".into()),
        member: "saved".into(),
    });
    let operator = apply::build(&node).unwrap();
    let pmml = tree("40");
    for (bytes, valid) in [
        (as_zip_pmml("PMML.xml", pmml.as_bytes()), true),
        (as_zip_pmml("other.xml", pmml.as_bytes()), false),
        (as_zip_pmml("PMML.xml", b"broken XML"), false),
        (as_zip_pmml("PMML.xml", &[0xff]), false),
        (b"\0\0\0\x04PK\x03\x04".to_vec(), false),
    ] {
        let ctx = ExecContext {
            attachments: Arc::new(HashMap::from([("saved".into(), Arc::from(bytes))])),
            ..ExecContext::default()
        };
        let result = operator.run(&[Arc::new(input(false))], &ctx);
        assert_eq!(result.is_ok(), valid, "{result:?}");
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

/// Logit on `x` with two target categories. `x = 0` is a tie (probability 1/2 each); a null
/// `x` cannot be scored.
fn logit_pmml(data_type: &str, first: &str, second: &str) -> String {
    format!(
        r#"<PMML xmlns="http://www.dmg.org/PMML-4_3" version="4.3">
  <DataDictionary numberOfFields="2">
    <DataField name="y" optype="categorical" dataType="{data_type}">
      <Value value="{first}"/><Value value="{second}"/>
    </DataField>
    <DataField name="x" optype="continuous" dataType="double"/>
  </DataDictionary>
  <RegressionModel functionName="classification" normalizationMethod="logit">
    <MiningSchema>
      <MiningField name="y" usageType="predicted"/>
      <MiningField name="x"/>
    </MiningSchema>
    <RegressionTable intercept="0" targetCategory="{first}">
      <NumericPredictor name="x" exponent="1" coefficient="1"/>
    </RegressionTable>
    <RegressionTable intercept="0" targetCategory="{second}"/>
  </RegressionModel>
</PMML>"#
    )
}

fn logistic_settings(procedure: &str, all_probabilities: bool) -> Vec<Node> {
    vec![
        p("calculateConfidences", "true"),
        p("calculateRawPropensities", "false"),
        p(
            "appendProbabilities",
            if all_probabilities { "true" } else { "false" },
        ),
        p("logisticProcedure", procedure),
        Node::Element(element(
            "model",
            vec![("pmml", "data/0001.dat")],
            Vec::new(),
        )),
    ]
}

fn logit_input(indexed: bool) -> Table {
    let mut fields = vec![Field::new("x", Storage::Real)];
    if indexed {
        fields.push(Field::new("$L-y", Storage::String));
    }
    let rows = [Some(0.0), None]
        .into_iter()
        .map(|x| {
            let mut row = vec![x.map_or(Value::Null, Value::Real)];
            if indexed {
                row.push(Value::string("old"));
            }
            row
        })
        .collect();
    Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
}

fn score_logit(procedure: &str, all_probabilities: bool, indexed: bool, pmml: &str) -> Table {
    let operator = apply::build(&nugget(
        "ApplyLogisticNode",
        logistic_settings(procedure, all_probabilities),
    ))
    .unwrap();
    operator
        .run(&[Arc::new(logit_input(indexed))], &context(pmml))
        .unwrap()
}

fn field_names(table: &Table) -> Vec<&str> {
    table
        .model()
        .fields()
        .iter()
        .map(|field| field.name.as_str())
        .collect()
}

#[test]
fn multinomial_appends_one_probability_field_per_category() {
    let out = score_logit("Multinomial", true, false, &logit_pmml("string", "A", "B"));
    assert_eq!(field_names(&out), ["x", "$L-y", "$LP-y", "$LP-A", "$LP-B"]);
    let probability = out.model().fields().iter().find(|f| f.name == "$LP-A");
    assert_eq!(probability.map(|f| f.storage), Some(Storage::Real));
    // A tie keeps the first category. A missing input scores nothing.
    assert_eq!(column(&out, "$L-y"), [Value::string("A"), Value::Null]);
    assert_eq!(column(&out, "$LP-y"), [Value::real(0.5), Value::Null]);
    assert_eq!(column(&out, "$LP-A"), [Value::real(0.5), Value::Null]);
    assert_eq!(column(&out, "$LP-B"), [Value::real(0.5), Value::Null]);
}

#[test]
fn binomial_appends_category_probabilities_when_the_flag_is_off() {
    let out = score_logit("Binomial", false, false, &logit_pmml("integer", "1", "2"));
    assert_eq!(field_names(&out), ["x", "$L-y", "$LP-y", "$LP-1", "$LP-2"]);
    assert_eq!(column(&out, "$L-y"), [Value::Integer(1), Value::Null]);
    assert_eq!(column(&out, "$LP-1"), [Value::real(0.5), Value::Null]);
    assert_eq!(column(&out, "$LP-2"), [Value::real(0.5), Value::Null]);
}

#[test]
fn multinomial_without_the_flag_omits_category_probabilities() {
    let out = score_logit("Multinomial", false, false, &logit_pmml("string", "A", "B"));
    assert_eq!(field_names(&out), ["x", "$L-y", "$LP-y"]);
}

#[test]
fn category_probabilities_use_the_prediction_fields_index() {
    let out = score_logit("Multinomial", true, true, &logit_pmml("string", "A", "B"));
    assert_eq!(
        field_names(&out),
        ["x", "$L-y", "$L1-y", "$LP1-y", "$LP1-A", "$LP1-B"]
    );
    assert_eq!(
        column(&out, "$L-y"),
        [Value::string("old"), Value::string("old")]
    );
    assert_eq!(column(&out, "$LP1-A"), [Value::real(0.5), Value::Null]);
}

#[test]
fn saved_alm_interval_boundaries_and_outside_domain_scores() {
    // PMML 4.3 valid-value intervals; the Auto Numeric oracle excludes this component
    // outside its input domain. Exercise all closure boundaries without external files.
    for (closure, low_valid, high_valid) in [
        ("closedClosed", true, true),
        ("closedOpen", true, false),
        ("openClosed", false, true),
        ("openOpen", false, false),
    ] {
        let xml = format!(
            r#"<PMML version="4.3"><DataDictionary><DataField name="x" dataType="double" optype="continuous"><Interval closure="{closure}" leftMargin="0" rightMargin="1"/></DataField><DataField name="y" dataType="double" optype="continuous"/></DataDictionary><GeneralRegressionModel algorithmName="ALM" functionName="regression" modelType="regression" targetVariableName="y"><MiningSchema><MiningField name="x"/><MiningField name="y" usageType="predicted"/></MiningSchema><ParameterList><Parameter name="p"/></ParameterList><CovariateList><Predictor name="x"/></CovariateList><PPMatrix><PPCell parameterName="p" predictorName="x" value="1"/></PPMatrix><ParamMatrix><PCell parameterName="p" beta="2"/></ParamMatrix></GeneralRegressionModel></PMML>"#
        );
        let node = nugget(
            "ApplyLinearRegressionSplitModelNode",
            vec![Node::Element(element("model", vec![("pmml", "m")], vec![]))],
        );
        let context = ExecContext {
            attachments: Arc::new(HashMap::from([(
                "m".to_string(),
                Arc::from(xml.into_bytes()),
            )])),
            ..ExecContext::default()
        };
        let input = Arc::new(
            Table::new(
                DataModel::new(vec![Field::new("x", Storage::Real)]).unwrap(),
                [-0.1, 0.0, 0.5, 1.0, 1.1]
                    .iter()
                    .map(|x| vec![Value::real(*x)])
                    .collect(),
            )
            .unwrap(),
        );
        let output = apply::build(&node)
            .unwrap()
            .run(&[input], &context)
            .unwrap();
        assert_eq!(
            column(&output, "$L-y"),
            vec![
                Value::Null,
                if low_valid {
                    Value::real(0.0)
                } else {
                    Value::Null
                },
                Value::real(1.0),
                if high_valid {
                    Value::real(2.0)
                } else {
                    Value::Null
                },
                Value::Null
            ],
            "{closure}"
        );
    }
}
