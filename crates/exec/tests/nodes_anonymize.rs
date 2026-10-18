//! Anonymize on synthetic tables.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_exec::nodes::anonymize;
use dms_exec::{ExecContext, OperatorError};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
use dms_types::{DataModel, Field, Storage, Table, Value};
use std::sync::Arc;

fn e(name: &str, children: Vec<Node>) -> Node {
    Node::Element(Element {
        name: QualifiedName {
            prefix: None,
            namespace: None,
            local: Arc::from(name),
        },
        namespace_declarations: Vec::new(),
        attributes: Vec::new(),
        children,
    })
}

fn p(name: &str, text: &str) -> Node {
    e(name, vec![Node::Text(Arc::from(text))])
}

fn run(fields: Vec<Node>, input: Table) -> Result<Table, OperatorError> {
    let Node::Element(settings) = e(
        "properties",
        vec![e("anonymizeValuesTable", vec![e("fields", fields)])],
    ) else {
        panic!("not an element")
    };
    let node = StreamNode {
        uid: "n1".into(),
        class: "x".into(),
        extension: None,
        model_builder: None,
        name: None,
        label: None,
        position: None,
        properties_attachment: None,
        containers: Vec::new(),
        settings: Some(settings),
        binning_output: None,
        result_items: Vec::new(),
        subdiagram: None,
    };
    anonymize::build(&node)?.run(&[Arc::new(input)], &ExecContext::default())
}

fn input() -> Table {
    let model = DataModel::new(vec![
        Field::new("a", Storage::String),
        Field::new("b", Storage::String),
        Field::new("n", Storage::Integer),
    ])
    .unwrap();
    let row = |a: &str, b: Option<&str>, n: i64| {
        vec![
            Value::string(a),
            b.map_or(Value::Null, Value::string),
            Value::Integer(n),
        ]
    };
    Table::new(
        model,
        vec![
            row("x", Some("y"), -4),
            row("y", None, 0),
            row("x", Some("z"), 2),
        ],
    )
    .unwrap()
}

fn col(table: &Table, name: &str) -> Vec<Value> {
    table.column(name).unwrap().cloned().collect()
}

#[test]
fn replaced_fields_share_one_dictionary_and_transforms_scale_after_translating() {
    let replace = |name: &str| {
        e(
            "field",
            vec![
                p("name", name),
                e(
                    "replace_operation",
                    vec![p("useCustomPrefix", "false"), p("prefix", "ignored_")],
                ),
            ],
        )
    };
    let transform = e(
        "field",
        vec![
            p("name", "n"),
            e(
                "transform_operation",
                vec![
                    p("transform_randomly", "true"),
                    p("scale_value", "6"),
                    p("translate_value", "39"),
                ],
            ),
        ],
    );
    let table = run(vec![replace("a"), replace("b"), transform], input()).unwrap();
    let s = |t: &str| Value::string(t);
    assert_eq!(col(&table, "a"), [s("anon_S0"), s("anon_S1"), s("anon_S0")]);
    assert_eq!(col(&table, "b"), [s("anon_S1"), Value::Null, s("anon_S2")]);
    assert_eq!(
        col(&table, "n"),
        [210, 234, 246].map(Value::Integer).to_vec()
    );

    let fractional = e(
        "field",
        vec![
            p("name", "n"),
            e(
                "transform_operation",
                vec![p("scale_value", "2.5"), p("translate_value", "0")],
            ),
        ],
    );
    assert!(run(vec![fractional], input()).is_err());
}
