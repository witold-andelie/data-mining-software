//! Oracle and unit tests for the Statistics, Means and Matrix output nodes.

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions and corpus setup fail loudly"
)]

mod support;

use dms_exec::nodes::{matrix, means, statistics};
use dms_exec::{Document, DocumentTable, ExecContext, Factory, NodeOutput, OperatorError};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
use dms_types::{DataModel, Field, Measurement, MissingSpec, Storage, Table, Value, ValueSpec};
use serde_json::Value as JsonValue;
use std::sync::Arc;

/// Parses a Modeler Statistics JSON export into `Document` tables.
fn parse_statistics_json(text: &str) -> Document {
    let root: JsonValue = serde_json::from_str(text).unwrap();
    let mut tables = Vec::new();
    let sections = root.get("data").and_then(JsonValue::as_array).unwrap();
    for section in sections {
        let title = section
            .get("title")
            .and_then(JsonValue::as_str)
            .map(str::to_owned);
        let inner_data = section.get("data").and_then(JsonValue::as_array).unwrap();
        for item in inner_data {
            if let Some(format) = item.get("format").and_then(JsonValue::as_str) {
                let raw_rows = item.get("table").and_then(JsonValue::as_array).unwrap();
                let parsed_rows: Vec<Vec<String>> = raw_rows
                    .iter()
                    .map(|r| {
                        r.as_array()
                            .unwrap()
                            .iter()
                            .map(|c| c.as_str().unwrap().to_owned())
                            .collect()
                    })
                    .collect();
                if format == "FORMAT_ROW_AND_COLUMN_HEADER" {
                    let (header, rows) = parsed_rows
                        .split_first()
                        .map(|(h, rest)| (h.clone(), rest.to_vec()))
                        .unwrap_or_default();
                    tables.push(DocumentTable {
                        title: title.clone(),
                        header,
                        rows,
                    });
                } else {
                    tables.push(DocumentTable {
                        title: title.clone(),
                        header: Vec::new(),
                        rows: parsed_rows,
                    });
                }
            } else if let Some(sub_data) = item.get("data").and_then(JsonValue::as_array) {
                for sub_item in sub_data {
                    // A title-only item is the multiple-modes footnote; the `*` on the Mode
                    // value carries the same fact, so documents do not model footnotes.
                    let Some(raw_rows) = sub_item.get("table").and_then(JsonValue::as_array) else {
                        continue;
                    };
                    let parsed_rows: Vec<Vec<String>> = raw_rows
                        .iter()
                        .map(|r| {
                            r.as_array()
                                .unwrap()
                                .iter()
                                .map(|c| c.as_str().unwrap().to_owned())
                                .collect()
                        })
                        .collect();
                    tables.push(DocumentTable {
                        title: title.clone(),
                        header: Vec::new(),
                        rows: parsed_rows,
                    });
                }
            }
        }
    }
    Document {
        title: String::new(),
        tables,
    }
}

#[test]
fn statistics_means_and_matrix_match_modeler_oracles() {
    support::check_oracle_cases(&["statistics", "means", "matrix"], |case, text| {
        if case.format == "json" {
            parse_statistics_json(text)
        } else {
            support::parse_csv_document(text)
        }
    });
}

// ------------------------------------------------------------------ Unit tests for uncovered rules

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
        subdiagram: None,
    }
}

fn produce_doc(
    factory: Factory,
    settings: Vec<Node>,
    input: Table,
) -> Result<Arc<Document>, OperatorError> {
    let op = factory(&make_node(settings))?;
    let ctx = ExecContext {
        decimal_places: Some(3),
        ..ExecContext::default()
    };
    match op.produce(&[Arc::new(input)], None, &ctx)? {
        NodeOutput::Document(d) => Ok(d),
        other => panic!("expected Document, got {other:?}"),
    }
}

#[test]
fn statistics_absolute_strength_mode_unique_mode_and_settings_validation() {
    let fields = vec![
        Field::new("x", Storage::Integer),
        Field::new("y", Storage::Real),
    ];
    let rows = vec![
        vec![Value::Integer(5), Value::Real(1.0)],
        vec![Value::Integer(5), Value::Real(2.0)],
        vec![Value::Integer(10), Value::Real(3.0)],
    ];
    let table = Table::new(DataModel::new(fields).unwrap(), rows).unwrap();

    // Unique mode (5 appears twice, 10 once -> "5" without '*'), absolute correlation mode with
    // showCorrelationLabels=false.
    let doc = produce_doc(
        statistics::build,
        vec![
            g("examineFields", vec![p("field", "x")]),
            g(
                "statistics",
                vec![p("setElement", "mode"), p("setElement", "median")],
            ),
            g("correlationFields", vec![p("field", "x"), p("field", "y")]),
            p("correlationStrengthMode", "absolute"),
            p("showCorrelationLabels", "false"),
        ],
        table,
    )
    .unwrap();
    assert_eq!(
        doc.tables[0].rows,
        vec![
            vec!["Median".to_string(), "5".to_string()],
            vec!["Mode".to_string(), "5".to_string()],
        ]
    );
    assert_eq!(doc.tables[1].rows[0][1], "1.000");

    // Rejects unsupported outputMode, unknown stat, unknown strength mode, and invalid thresholds.
    assert!(statistics::build(&make_node(vec![p("outputMode", "file")])).is_err());
    assert!(
        statistics::build(&make_node(vec![g(
            "statistics",
            vec![p("setElement", "skewness")]
        )]))
        .is_err()
    );
    assert!(statistics::build(&make_node(vec![p("correlationStrengthMode", "rank")])).is_err());
    assert!(
        statistics::build(&make_node(vec![
            p("weakProbabilityThreshold", "0.98"),
            p("mediumProbabilityThreshold", "0.90"),
        ]))
        .is_err()
    );
}

#[test]
fn means_rejects_within_records_and_handles_degenerate_groups() {
    assert!(means::build(&make_node(vec![p("method", "withinRecords")])).is_err());
    assert!(means::build(&make_node(vec![])).is_err());
    assert!(means::build(&make_node(vec![p("groupingField", "g")])).is_err());

    // Single group -> F-Test, df1, df2, Importance are all "--".
    let fields = vec![
        Field::new("g", Storage::String),
        Field::new("x", Storage::Real),
    ];
    let rows = vec![
        vec![Value::string("A"), Value::Real(2.0)],
        vec![Value::string("A"), Value::Real(4.0)],
    ];
    let table = Table::new(DataModel::new(fields).unwrap(), rows).unwrap();
    let doc = produce_doc(
        means::build,
        vec![
            p("groupingField", "g"),
            g("meansGroupedFields", vec![p("field", "x")]),
        ],
        table,
    )
    .unwrap();
    let row = &doc.tables[0].rows[0];
    assert_eq!(&row[row.len() - 4..], &["--", "--", "--", "--"]);
}

#[test]
fn matrix_supports_sum_sdev_min_max_and_excludes_missing_values_when_configured() {
    let mut g_field = Field::new("g", Storage::String);
    g_field.measurement = Measurement::Nominal;
    g_field.values = ValueSpec::Categories(vec![Value::string("A"), Value::string("B")]);
    g_field.missing = MissingSpec {
        values: vec![Value::string("B")],
        ..MissingSpec::default()
    };
    let mut c_field = Field::new("c", Storage::String);
    c_field.measurement = Measurement::Nominal;
    c_field.values = ValueSpec::Categories(vec![Value::string("X")]);
    let v_field = Field::new("v", Storage::Real);

    let rows = vec![
        vec![Value::string("A"), Value::string("X"), Value::Real(2.0)],
        vec![Value::string("A"), Value::string("X"), Value::Real(6.0)],
        vec![Value::string("B"), Value::string("X"), Value::Real(100.0)],
        vec![Value::Null, Value::string("X"), Value::Real(50.0)],
    ];
    let table = Table::new(
        DataModel::new(vec![g_field, c_field, v_field]).unwrap(),
        rows,
    )
    .unwrap();

    for (fn_name, expected) in [
        ("sum", "8.000"),
        ("sdev", "2.828"),
        ("min", "2.000"),
        ("max", "6.000"),
    ] {
        let doc = produce_doc(
            matrix::build,
            vec![
                p("rowField", "g"),
                p("columnField", "c"),
                p("cellContents", "parameter"),
                p("parameterField", "v"),
                p("parameterFunction", fn_name),
                p("includeMissingValues", "false"),
            ],
            table.clone(),
        )
        .unwrap();
        // With includeMissingValues=false, both $null$ and blank "B" are excluded, leaving only "A".
        assert_eq!(
            doc.tables[0].rows,
            vec![vec!["A".to_string(), expected.to_string()]]
        );
    }

    assert!(matrix::build(&make_node(vec![p("rowField", "g")])).is_err());
    assert!(
        matrix::build(&make_node(vec![
            p("rowField", "g"),
            p("columnField", "c"),
            p("cellContents", "parameter"),
        ]))
        .is_err()
    );
}
