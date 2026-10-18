//! Oracle and unit tests for the Analysis output node.

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions and corpus setup fail loudly"
)]

mod support;

use dms_exec::nodes::analysis;
use dms_exec::{Document, DocumentTable, ExecContext, NodeOutput, OperatorError};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
use dms_types::{DataModel, Field, Measurement, Role, Storage, Table, Value, ValueSpec};
use serde_json::Value as JsonValue;
use std::sync::Arc;

/// Walks a Modeler Analysis JSON tree. Each table's title is the ancestor title chain joined
/// by ` | `, which is the same join `analysis` uses. `FORMAT_ROW_AND_COLUMN_HEADER` lifts its
/// first row into `header`.
fn parse_analysis_json(text: &str) -> Document {
    let root: JsonValue = serde_json::from_str(text).unwrap();
    let mut tables = Vec::new();
    let mut chain = Vec::new();
    collect_json_tables(&root, &mut chain, &mut tables);
    Document {
        title: String::new(),
        tables,
    }
}

fn collect_json_tables(node: &JsonValue, chain: &mut Vec<String>, tables: &mut Vec<DocumentTable>) {
    let mut pushed = false;
    if let Some(title) = node.get("title").and_then(JsonValue::as_str) {
        chain.push(title.to_string());
        pushed = true;
    }
    if let (Some(format), Some(raw_rows)) = (
        node.get("format").and_then(JsonValue::as_str),
        node.get("table").and_then(JsonValue::as_array),
    ) {
        let parsed_rows: Vec<Vec<String>> = raw_rows
            .iter()
            .map(|row| {
                row.as_array()
                    .unwrap()
                    .iter()
                    .map(|cell| cell.as_str().unwrap().to_owned())
                    .collect()
            })
            .collect();
        let title = if chain.is_empty() {
            None
        } else {
            Some(chain.join(" | "))
        };
        if format == "FORMAT_ROW_AND_COLUMN_HEADER" {
            let (header, rows) = parsed_rows
                .split_first()
                .map(|(header, rest)| (header.clone(), rest.to_vec()))
                .unwrap_or_default();
            tables.push(DocumentTable {
                title,
                header,
                rows,
            });
        } else {
            tables.push(DocumentTable {
                title,
                header: Vec::new(),
                rows: parsed_rows,
            });
        }
    }
    if let Some(children) = node.get("data").and_then(JsonValue::as_array) {
        for child in children {
            collect_json_tables(child, chain, tables);
        }
    }
    if pushed {
        chain.pop();
    }
}

#[test]
fn analysis_matches_modeler_oracles() {
    support::check_oracle_cases(&["analysis"], |_, text| parse_analysis_json(text));
}

fn raw_element(name: &str, children: Vec<Node>) -> Element {
    Element {
        name: QualifiedName {
            prefix: None,
            namespace: None,
            local: Arc::from(name),
        },
        namespace_declarations: Vec::new(),
        attributes: Vec::new(),
        children,
    }
}

fn p(name: &str, text: &str) -> Node {
    Node::Element(raw_element(name, vec![Node::Text(Arc::from(text))]))
}

fn g(name: &str, children: Vec<Node>) -> Node {
    Node::Element(raw_element(name, children))
}

fn make_node(children: Vec<Node>) -> StreamNode {
    StreamNode {
        uid: "u1".into(),
        class: "c".into(),
        extension: None,
        model_builder: None,
        name: None,
        label: None,
        position: None,
        properties_attachment: None,
        containers: Vec::new(),
        settings: Some(raw_element("properties", children)),
        binning_output: None,
        result_items: Vec::new(),
        subdiagram: None,
    }
}

fn produce_doc(settings: Vec<Node>, input: Table) -> Result<Arc<Document>, OperatorError> {
    let op = analysis::build(&make_node(settings))?;
    let ctx = ExecContext {
        decimal_places: Some(15),
        ..ExecContext::default()
    };
    match op.produce(&[Arc::new(input)], None, &ctx)? {
        NodeOutput::Document(doc) => Ok(doc),
        other => panic!("expected Document, got {other:?}"),
    }
}

#[test]
fn analysis_rejects_unsupported_settings() {
    assert!(analysis::build(&make_node(vec![p("outputMode", "file")])).is_err());
    assert!(analysis::build(&make_node(vec![p("outputFileType", "pdf")])).is_err());
    assert!(analysis::build(&make_node(vec![p("appendTagField", "true")])).is_err());
    assert!(analysis::build(&make_node(vec![p("analysisUserDefined", "true")])).is_err());
    assert!(analysis::build(&make_node(vec![p("fieldDetectionMethod", "auto")])).is_err());
    assert!(analysis::build(&make_node(vec![p("analysisThreshold", "0")])).is_err());
    assert!(analysis::build(&make_node(vec![p("analysisThreshold", "105")])).is_err());
    assert!(analysis::build(&make_node(vec![p("analysisImproveAccuracy", "0.0")])).is_err());
    assert!(analysis::build(&make_node(vec![p("analysisImproveAccuracy", "-1")])).is_err());
    assert!(
        analysis::build(&make_node(vec![g(
            "analysisUserCompute",
            vec![p("setElement", "median")],
        )]))
        .is_err()
    );
    assert!(analysis::build(&make_node(vec![p("analysisFoo", "1")])).is_err());

    // The saved file stores the threshold as an integer (`90`) and the fold as `2.0`.
    assert!(analysis::build(&make_node(vec![p("analysisThreshold", "90")])).is_ok());
    assert!(analysis::build(&make_node(vec![p("analysisThreshold", "100")])).is_ok());
    assert!(analysis::build(&make_node(vec![p("analysisImproveAccuracy", "2.0")])).is_ok());
    assert!(analysis::build(&make_node(vec![p("splitByPartition", "false")])).is_ok());
}

#[test]
fn analysis_formats_large_counts_and_real_categories() {
    let mut rows = Vec::new();
    for _ in 0..1500 {
        rows.push(vec![Value::string("a"), Value::string("a")]);
    }
    for _ in 0..500 {
        rows.push(vec![Value::string("a"), Value::string("b")]);
    }
    let mut target = Field::new("y", Storage::String);
    target.role = Role::Target;
    target.measurement = Measurement::Nominal;
    target.values = ValueSpec::Categories(vec![Value::string("a"), Value::string("b")]);
    let mut pred = Field::new("$R-y", Storage::String);
    pred.measurement = Measurement::Nominal;
    let model = DataModel::new(vec![target, pred]).unwrap();
    let doc = produce_doc(Vec::new(), Table::new(model, rows).unwrap()).unwrap();
    assert_eq!(doc.tables.len(), 1);
    assert_eq!(
        doc.tables[0].rows,
        vec![
            vec![
                "Correct".to_string(),
                "1,500".to_string(),
                "75%".to_string()
            ],
            vec!["Wrong".to_string(), "500".to_string(), "25%".to_string()],
            vec!["Total".to_string(), "2,000".to_string(), String::new()],
        ]
    );

    let mut y = Field::new("y", Storage::Real);
    y.role = Role::Target;
    y.measurement = Measurement::Flag;
    y.values = ValueSpec::Flag {
        true_value: Value::real(1.0),
        false_value: Value::real(0.0),
    };
    let mut py = Field::new("$R-y", Storage::Real);
    py.measurement = Measurement::Flag;
    let model = DataModel::new(vec![y, py]).unwrap();
    let doc = produce_doc(
        vec![p("analysisCoincidenceMatrix", "true")],
        Table::new(
            model,
            vec![
                vec![Value::real(0.0), Value::real(0.0)],
                vec![Value::real(0.0), Value::real(1.0)],
                vec![Value::real(1.0), Value::real(1.0)],
            ],
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(doc.tables.len(), 2);
    assert!(doc.tables[1].header.is_empty());
    assert_eq!(
        doc.tables[1].rows,
        vec![
            vec![
                String::new(),
                "0.000000".to_string(),
                "1.000000".to_string()
            ],
            vec!["0.000000".to_string(), "1".to_string(), "1".to_string()],
            vec!["1.000000".to_string(), "0".to_string(), "1".to_string()],
        ]
    );
}

#[test]
fn analysis_binary_evaluation_without_a_partition() {
    let op = analysis::build(&make_node(vec![
        p("fieldDetectionMethod", "metadata"),
        p("analysisBinaryEvaluation", "true"),
    ]))
    .unwrap();
    let mut target = Field::new("y", Storage::Integer);
    target.role = Role::Target;
    target.measurement = Measurement::Flag;
    target.values = ValueSpec::Flag {
        true_value: Value::Integer(1),
        false_value: Value::Integer(0),
    };
    let mut pred = Field::new("$R-y", Storage::Integer);
    pred.measurement = Measurement::Flag;
    let mut conf = Field::new("$RC-y", Storage::Real);
    conf.measurement = Measurement::Continuous;
    let model = DataModel::new(vec![target, pred, conf]).unwrap();
    let table = Arc::new(
        Table::new(
            model,
            vec![
                vec![Value::Integer(0), Value::Integer(0), Value::real(0.8)],
                vec![Value::Integer(0), Value::Integer(1), Value::real(0.7)],
                vec![Value::Integer(1), Value::Integer(1), Value::real(0.7)],
                vec![Value::Integer(1), Value::Integer(1), Value::real(0.9)],
            ],
        )
        .unwrap(),
    );
    let ctx = ExecContext::default();
    assert!(matches!(
        op.run(std::slice::from_ref(&table), &ctx),
        Err(OperatorError::Inputs(_))
    ));
    let NodeOutput::Document(doc) = op.produce(&[table], None, &ctx).unwrap() else {
        panic!("expected Document");
    };
    assert_eq!(doc.tables.len(), 2);
    assert_eq!(
        doc.tables[1].header,
        vec!["Model".to_string(), "AUC".to_string(), "Gini".to_string()]
    );
    assert_eq!(
        doc.tables[1].rows,
        vec![vec![
            "$R-y".to_string(),
            "0.875".to_string(),
            "0.75".to_string()
        ]]
    );
    let eval_title = doc.tables[1].title.as_deref().unwrap();
    assert_eq!(
        eval_title,
        "Results for output field y | Evaluation Metrics"
    );
    assert!(!eval_title.contains("Individual Models"));
}

#[test]
fn analysis_name_detection_and_partition_switch() {
    let mut target = Field::new("y", Storage::String);
    target.role = Role::Input;
    target.measurement = Measurement::Nominal;
    let mut pred = Field::new("$R-y", Storage::String);
    pred.measurement = Measurement::Nominal;
    let model = DataModel::new(vec![target, pred]).unwrap();
    let rows = vec![vec![Value::string("a"), Value::string("a")]];
    let ignored = produce_doc(
        vec![p("fieldDetectionMethod", "metadata")],
        Table::new(model.clone(), rows.clone()).unwrap(),
    )
    .unwrap();
    assert!(ignored.tables.is_empty());
    let named = produce_doc(
        vec![p("fieldDetectionMethod", "name")],
        Table::new(model, rows).unwrap(),
    )
    .unwrap();
    assert_eq!(named.tables.len(), 1);
    assert_eq!(named.tables[0].rows[0][0], "Correct");

    let mut target = Field::new("y", Storage::String);
    target.role = Role::Target;
    target.measurement = Measurement::Nominal;
    let pred = Field::new("$R-y", Storage::String);
    let mut part = Field::new("Partition", Storage::String);
    part.role = Role::Partition;
    part.measurement = Measurement::Nominal;
    part.values = ValueSpec::Categories(vec![Value::string("2_Testing")]);
    let model = DataModel::new(vec![target, pred, part]).unwrap();
    let rows = vec![vec![
        Value::string("a"),
        Value::string("a"),
        Value::string("2_Testing"),
    ]];
    let off = produce_doc(
        vec![p("splitByPartition", "false")],
        Table::new(model.clone(), rows.clone()).unwrap(),
    )
    .unwrap();
    assert!(off.tables[0].header.is_empty());
    assert!(
        !off.tables[0]
            .title
            .as_deref()
            .unwrap()
            .contains("Individual Models")
    );
    let on = produce_doc(Vec::new(), Table::new(model, rows).unwrap()).unwrap();
    assert_eq!(
        on.tables[0].header,
        vec![
            "'Partition'".to_string(),
            "2_Testing".to_string(),
            String::new()
        ]
    );
    assert!(
        on.tables[0]
            .title
            .as_deref()
            .unwrap()
            .contains("Individual Models")
    );
}
