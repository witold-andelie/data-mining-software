//! Restructure and Distinct nodes on synthetic tables.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_exec::nodes::{distinct, restructure};
use dms_exec::{ExecContext, Factory, OperatorError};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
use dms_types::{DataModel, Field, Measurement, MissingSpec, Storage, Table, Value, ValueSpec};
use std::sync::Arc;

fn element(name: &str, children: Vec<Node>) -> Node {
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
    element(name, vec![Node::Text(Arc::from(text))])
}

fn g(name: &str, children: Vec<Node>) -> Node {
    element(name, children)
}

fn list(name: &str, names: &[&str]) -> Node {
    g(name, names.iter().map(|n| p("field", n)).collect())
}

fn node(children: Vec<Node>) -> StreamNode {
    let Node::Element(settings) = element("properties", children) else {
        panic!("element() builds an element")
    };
    StreamNode {
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
    }
}

fn run(factory: Factory, settings: Vec<Node>, input: Table) -> Result<Table, OperatorError> {
    let operator = factory(&node(settings))?;
    operator.run(&[Arc::new(input)], &ExecContext::default())
}

fn int(i: i64) -> Value {
    Value::Integer(i)
}

fn real(r: f64) -> Value {
    Value::Real(r)
}

fn s(text: &str) -> Value {
    Value::string(text)
}

fn table(fields: Vec<Field>, rows: Vec<Vec<Value>>) -> Table {
    Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
}

fn col(table: &Table, name: &str) -> Vec<Value> {
    table.column(name).unwrap().cloned().collect()
}

fn member(src_field: &str, src_value: &str, include_field_names: Option<bool>) -> Node {
    let mut children = vec![p("srcFieldName", src_field), p("srcFieldValue", src_value)];
    if let Some(inc) = include_field_names {
        children.push(p("includeFieldNames", if inc { "true" } else { "false" }));
    }
    g("persistentObjectListMember", children)
}

fn sample_table() -> Table {
    let mut grp = Field::new("grp", Storage::String);
    grp.measurement = Measurement::Nominal;
    grp.values = ValueSpec::Categories(vec![s("A"), s("B")]);
    let mut iv = Field::new("iv", Storage::Integer);
    iv.measurement = Measurement::Continuous;
    let mut rv = Field::new("rv", Storage::Real);
    rv.measurement = Measurement::Continuous;
    table(
        vec![Field::new("id", Storage::Integer), grp, iv, rv],
        vec![
            vec![int(1), s("B"), int(10), real(-3.5)],
            vec![int(2), s("A"), int(20), Value::Null],
            vec![int(3), Value::Null, int(30), real(1.25)],
            vec![int(4), s("C"), int(40), real(2.5)],
            vec![int(5), s("A"), Value::Null, real(5.0)],
        ],
    )
}

// ------------------------------------------------------------ restructure

#[test]
fn restructure_flags_creates_integer_flag_columns_and_handles_nulls() {
    let out = run(
        restructure::build,
        vec![
            p("restructureValuesMode", "Flags"),
            g(
                "createRestructuredFields",
                vec![
                    member("grp", "A", Some(true)),
                    member("grp", "B", Some(true)),
                ],
            ),
        ],
        sample_table(),
    )
    .unwrap();

    let names: Vec<_> = out
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, vec!["id", "grp", "iv", "rv", "grp_A", "grp_B"]);

    let fa = out.model().field("grp_A").unwrap();
    assert_eq!(fa.storage, Storage::Integer);
    assert_eq!(fa.measurement, Measurement::Flag);
    assert_eq!(
        fa.values,
        ValueSpec::Flag {
            true_value: int(1),
            false_value: int(0),
        }
    );

    assert_eq!(
        col(&out, "grp_A"),
        vec![int(0), int(1), int(0), int(0), int(1)]
    );
    assert_eq!(
        col(&out, "grp_B"),
        vec![int(1), int(0), int(0), int(0), int(0)]
    );
}

#[test]
fn restructure_all_values_uses_type_categories_and_dollar_suffix_for_flags() {
    let out = run(
        restructure::build,
        vec![
            p("restructureValuesMode", "Flags"),
            g(
                "createRestructuredFields",
                vec![member("grp", "all", Some(true))],
            ),
        ],
        sample_table(),
    )
    .unwrap();

    let names: Vec<_> = out
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, vec!["id", "grp", "iv", "rv", "grp_A", "grp_B"]);
    assert_eq!(
        col(&out, "grp_A"),
        vec![int(0), int(1), int(0), int(0), int(1)]
    );
    assert_eq!(
        col(&out, "grp_B"),
        vec![int(1), int(0), int(0), int(0), int(0)]
    );
}

#[test]
fn restructure_all_values_without_instantiated_type_scans_non_missing_values() {
    let mut grp = Field::new("grp", Storage::String);
    grp.missing = MissingSpec {
        values: vec![s("SKIP")],
        ..MissingSpec::default()
    };
    let input = table(
        vec![grp],
        vec![
            vec![s("Y")],
            vec![Value::Null],
            vec![s("SKIP")],
            vec![s("X")],
            vec![s("Y")],
        ],
    );
    let out = run(
        restructure::build,
        vec![
            p("restructureValuesMode", "Flags"),
            p("restructureIncludeFieldNames", "false"),
            g("createRestructuredFields", vec![member("grp", "all", None)]),
        ],
        input,
    )
    .unwrap();

    let names: Vec<_> = out
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, vec!["grp", "Y", "X"]);
    assert_eq!(col(&out, "Y"), vec![int(1), int(0), int(0), int(0), int(1)]);
    assert_eq!(col(&out, "X"), vec![int(0), int(0), int(0), int(1), int(0)]);
}

#[test]
fn restructure_other_fields_orders_by_value_field_then_member_and_preserves_storage() {
    let out = run(
        restructure::build,
        vec![
            p("restructureValuesMode", "OtherFields"),
            g(
                "createRestructuredFields",
                vec![
                    member("grp", "A", Some(true)),
                    member("grp", "B", Some(true)),
                ],
            ),
            list("restructureValueFields", &["iv", "rv"]),
        ],
        sample_table(),
    )
    .unwrap();

    let names: Vec<_> = out
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec![
            "id", "grp", "iv", "rv", "grp_A_iv", "grp_B_iv", "grp_A_rv", "grp_B_rv"
        ]
    );

    let f_iv = out.model().field("grp_A_iv").unwrap();
    assert_eq!(f_iv.storage, Storage::Integer);
    assert_eq!(f_iv.measurement, Measurement::Continuous);

    let f_rv = out.model().field("grp_A_rv").unwrap();
    assert_eq!(f_rv.storage, Storage::Real);
    assert_eq!(f_rv.measurement, Measurement::Continuous);

    assert_eq!(
        col(&out, "grp_A_iv"),
        vec![Value::Null, int(20), Value::Null, Value::Null, Value::Null]
    );
    assert_eq!(
        col(&out, "grp_B_iv"),
        vec![int(10), Value::Null, Value::Null, Value::Null, Value::Null]
    );
    assert_eq!(
        col(&out, "grp_A_rv"),
        vec![
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            real(5.0)
        ]
    );
    assert_eq!(
        col(&out, "grp_B_rv"),
        vec![
            real(-3.5),
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null
        ]
    );
}

#[test]
fn restructure_without_field_names_and_empty_input() {
    let empty = table(sample_table().model().fields().to_vec(), Vec::new());
    let out = run(
        restructure::build,
        vec![
            p("restructureValuesMode", "OtherFields"),
            p("restructureIncludeFieldNames", "false"),
            g(
                "createRestructuredFields",
                vec![member("grp", "A", Some(false))],
            ),
            list("restructureValueFields", &["iv"]),
        ],
        empty,
    )
    .unwrap();
    assert_eq!(out.row_count(), 0);
    let names: Vec<_> = out
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, vec!["id", "grp", "iv", "rv", "A_iv"]);
}

#[test]
fn restructure_rejects_bad_settings_and_missing_fields() {
    assert!(restructure::build(&node(Vec::new())).is_err());
    assert!(
        restructure::build(&node(vec![
            p("restructureValuesMode", "InvalidMode"),
            g("createRestructuredFields", vec![member("grp", "A", None)]),
        ]))
        .is_err()
    );
    assert!(
        restructure::build(&node(vec![
            p("restructureValuesMode", "OtherFields"),
            g("createRestructuredFields", vec![member("grp", "A", None)]),
        ]))
        .is_err(),
        "OtherFields without restructureValueFields must fail"
    );
    assert!(
        restructure::build(&node(vec![
            p("restructureValuesMode", "Flags"),
            g("createRestructuredFields", vec![member("", "A", None)]),
        ]))
        .is_err()
    );
    assert!(
        restructure::build(&node(vec![
            p("restructureValuesMode", "Flags"),
            g("createRestructuredFields", vec![member("grp", "", None)]),
        ]))
        .is_err()
    );

    let err = run(
        restructure::build,
        vec![
            p("restructureValuesMode", "Flags"),
            g(
                "createRestructuredFields",
                vec![member("nosuch", "A", Some(true))],
            ),
        ],
        sample_table(),
    )
    .unwrap_err();
    assert!(matches!(err, OperatorError::Data(_)));

    let err = run(
        restructure::build,
        vec![
            p("restructureValuesMode", "OtherFields"),
            g("createRestructuredFields", vec![member("grp", "A", None)]),
            list("restructureValueFields", &["nosuch"]),
        ],
        sample_table(),
    )
    .unwrap_err();
    assert!(matches!(err, OperatorError::Data(_)));
}

// ------------------------------------------------------------ distinct

fn distinct_input() -> Table {
    table(
        vec![
            Field::new("id", Storage::Integer),
            Field::new("grp", Storage::String),
            Field::new("iv", Storage::Integer),
        ],
        vec![
            vec![int(1), s("B"), int(10)],
            vec![int(2), s("A"), int(20)],
            vec![int(3), Value::Null, int(5)],
            vec![int(4), s("A"), int(15)],
            vec![int(5), Value::Null, int(30)],
            vec![int(6), s("B"), int(10)],
            vec![int(7), s("C"), int(99)],
        ],
    )
}

#[test]
fn distinct_include_and_discard_group_nulls_and_order_groups_like_modeler() {
    let inc = run(
        distinct::build,
        vec![
            p("distinctMode", "Include"),
            list("distinctFieldSelection", &["grp"]),
        ],
        distinct_input(),
    )
    .unwrap();
    // Groups in Sort order (nulls last); the last record of each group in input order.
    assert_eq!(col(&inc, "id"), vec![int(4), int(6), int(7), int(5)]);

    let disc = run(
        distinct::build,
        vec![
            p("distinctMode", "Discard"),
            list("distinctFieldSelection", &["grp"]),
        ],
        distinct_input(),
    )
    .unwrap();
    assert_eq!(col(&disc, "id"), vec![int(2), int(1), int(3)]);
}

#[test]
fn distinct_sorts_within_groups_and_supports_pre_sorted_keys() {
    let sort_row = |field: &str, dir: &str| g("row", vec![p("field", field), p("direction", dir)]);
    let inc = run(
        distinct::build,
        vec![
            p("distinctMode", "Include"),
            list("distinctFieldSelection", &["grp"]),
            g("sortKeys", vec![g("rows", vec![sort_row("iv", "0")])]),
        ],
        distinct_input(),
    )
    .unwrap();
    // A group: iv 15 (id 4) < iv 20 (id 2) -> id 4
    // B group: iv 10 tie -> reverse input order keeps id 6 first
    // C group: id 7
    // Null group last: iv 5 (id 3) < iv 30 (id 5) -> id 3
    assert_eq!(col(&inc, "id"), vec![int(4), int(6), int(7), int(3)]);

    let contig_input = table(
        vec![
            Field::new("id", Storage::Integer),
            Field::new("grp", Storage::String),
        ],
        vec![
            vec![int(1), s("B")],
            vec![int(2), s("B")],
            vec![int(3), s("A")],
            vec![int(4), s("B")],
        ],
    );
    let contig = run(
        distinct::build,
        vec![
            p("distinctMode", "Include"),
            list("distinctFieldSelection", &["grp"]),
            p("distinctPreSortedInput", "true"),
        ],
        contig_input,
    )
    .unwrap();
    assert_eq!(col(&contig, "id"), vec![int(1), int(3), int(4)]);
}

#[test]
fn distinct_handles_empty_input_and_rejects_bad_settings() {
    let empty = table(distinct_input().model().fields().to_vec(), Vec::new());
    let out = run(
        distinct::build,
        vec![list("distinctFieldSelection", &["grp"])],
        empty,
    )
    .unwrap();
    assert_eq!(out.row_count(), 0);

    assert!(distinct::build(&node(Vec::new())).is_err(), "no key fields");
    assert!(
        distinct::build(&node(vec![
            p("distinctMode", "bogus"),
            list("distinctFieldSelection", &["grp"]),
        ]))
        .is_err()
    );
    assert!(
        distinct::build(&node(vec![
            p("distinctMode", "Composite"),
            list("distinctFieldSelection", &["grp"]),
        ]))
        .is_err(),
        "Composite mode must be rejected"
    );
    assert!(
        distinct::build(&node(vec![
            p("distinctCreateCompositeRecord", "true"),
            list("distinctFieldSelection", &["grp"]),
        ]))
        .is_err(),
        "composite records must be rejected"
    );
    assert!(
        distinct::build(&node(vec![
            list("distinctFieldSelection", &["grp"]),
            g(
                "sortKeys",
                vec![g(
                    "rows",
                    vec![g("row", vec![p("field", "iv"), p("direction", "9")])]
                )],
            ),
        ]))
        .is_err()
    );

    let err = run(
        distinct::build,
        vec![list("distinctFieldSelection", &["nosuch"])],
        distinct_input(),
    )
    .unwrap_err();
    assert!(matches!(err, OperatorError::Data(_)));
}
