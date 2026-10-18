//! Field Operations nodes Set to Flag, Transpose and History.
//!
//! The corpus probes cover the saved streams. These tests cover rejections, empty input and
//! nulls, plus rules the probes leave open (numeric categories, several aggregate keys).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_exec::nodes::{history, set_to_flag, transpose};
use dms_exec::{ExecContext, Factory, OperatorError};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
use dms_types::{DataModel, Field, Measurement, Storage, Table, Value, ValueSpec};
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

fn s(text: &str) -> Value {
    Value::string(text)
}

fn table(fields: Vec<Field>, rows: Vec<Vec<Value>>) -> Table {
    Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
}

fn col(table: &Table, name: &str) -> Vec<Value> {
    table.column(name).unwrap().cloned().collect()
}

fn names(table: &Table) -> Vec<String> {
    table
        .model()
        .fields()
        .iter()
        .map(|field| field.name.clone())
        .collect()
}

fn settings_error(err: OperatorError) -> String {
    match err {
        OperatorError::Settings(message) => message,
        other => panic!("expected a settings error, got {other:?}"),
    }
}

// ---------------------------------------------------------------- set to flag

fn flag_member(field: &str, value: &str) -> Node {
    g(
        "persistentObjectListMember",
        vec![
            p("srcFieldName", field),
            p("srcFieldValue", value),
            p("flagNameExtension", ""),
            p("prefixNotSuffix", "false"),
        ],
    )
}

fn flag_settings(extra: Vec<Node>) -> Vec<Node> {
    let mut children = vec![
        p("trueValue", "T"),
        p("falseValue", "F"),
        g(
            "createFlagFields",
            vec![flag_member("cat", "red"), flag_member("cat", "blue")],
        ),
    ];
    children.extend(extra);
    children
}

fn cats() -> Table {
    let mut cat = Field::new("cat", Storage::String);
    cat.measurement = Measurement::Nominal;
    let mut grp = Field::new("grp", Storage::String);
    grp.measurement = Measurement::Nominal;
    table(
        vec![grp, cat],
        vec![
            vec![s("A"), s("red")],
            vec![s("A"), Value::Null],
            vec![s("B"), s("")],
            vec![s("A"), s("blue")],
        ],
    )
}

#[test]
fn set_to_flag_null_and_blank_are_false_and_names_follow_members() {
    let out = run(set_to_flag::build, flag_settings(vec![]), cats()).unwrap();
    assert_eq!(names(&out), vec!["grp", "cat", "cat_red", "cat_blue"]);
    assert_eq!(col(&out, "cat_red"), vec![s("T"), s("F"), s("F"), s("F")]);
    assert_eq!(col(&out, "cat_blue"), vec![s("F"), s("F"), s("F"), s("T")]);
    let flag = out.model().field("cat_red").unwrap();
    assert_eq!(flag.storage, Storage::String);
    assert_eq!(flag.measurement, Measurement::Flag);
    assert_eq!(
        flag.values,
        ValueSpec::Flag {
            true_value: s("T"),
            false_value: s("F"),
        }
    );
}

#[test]
fn set_to_flag_empty_input_keeps_the_flag_fields() {
    let mut cat = Field::new("cat", Storage::String);
    cat.measurement = Measurement::Nominal;
    let out = run(
        set_to_flag::build,
        flag_settings(vec![]),
        table(vec![cat], vec![]),
    )
    .unwrap();
    assert_eq!(names(&out), vec!["cat", "cat_red", "cat_blue"]);
    assert!(out.rows().is_empty());
}

#[test]
fn set_to_flag_numeric_categories_use_integer_flag_values() {
    let mut code = Field::new("code", Storage::Integer);
    code.measurement = Measurement::Nominal;
    let settings = vec![
        p("trueValue", "1"),
        p("falseValue", "0"),
        g(
            "createFlagFields",
            vec![flag_member("code", "1"), flag_member("code", "2")],
        ),
    ];
    let out = run(
        set_to_flag::build,
        settings,
        table(
            vec![code],
            vec![
                vec![int(1)],
                vec![Value::Null],
                vec![int(2)],
                vec![Value::Real(1.0)],
            ],
        ),
    )
    .unwrap();
    assert_eq!(col(&out, "code_1"), vec![int(1), int(0), int(0), int(1)]);
    assert_eq!(col(&out, "code_2"), vec![int(0), int(0), int(1), int(0)]);
    assert_eq!(
        out.model().field("code_1").unwrap().storage,
        Storage::Integer
    );
}

#[test]
fn set_to_flag_aggregate_ors_flags_and_keeps_only_keys_in_first_seen_order() {
    let settings = flag_settings(vec![
        p("useAggregateKeyFields", "true"),
        list("aggregateKeyFields", &["grp", "extra"]),
    ]);
    let mut extra = Field::new("extra", Storage::String);
    extra.measurement = Measurement::Nominal;
    let base = cats();
    let (model, rows) = base.into_parts();
    let mut fields = model.fields().to_vec();
    fields.push(extra);
    let rows = rows
        .into_iter()
        .map(|mut row| {
            row.push(s("k"));
            row
        })
        .collect();
    // cats() is (A, red), (A, null), (B, ""), (A, blue). Every row gets extra "k".
    let input = table(fields, rows);
    let out = run(set_to_flag::build, settings, input).unwrap();
    assert_eq!(names(&out), vec!["grp", "extra", "cat_red", "cat_blue"]);
    assert_eq!(col(&out, "grp"), vec![s("A"), s("B")]);
    assert_eq!(col(&out, "cat_red"), vec![s("T"), s("F")]);
    assert_eq!(col(&out, "cat_blue"), vec![s("T"), s("F")]);
}

#[test]
fn set_to_flag_aggregate_groups_null_keys_and_empty_input() {
    let settings = flag_settings(vec![
        p("useAggregateKeyFields", "true"),
        list("aggregateKeyFields", &["grp"]),
    ]);
    let mut grp = Field::new("grp", Storage::String);
    grp.measurement = Measurement::Nominal;
    let mut cat = Field::new("cat", Storage::String);
    cat.measurement = Measurement::Nominal;
    let out = run(
        set_to_flag::build,
        settings.clone(),
        table(
            vec![grp.clone(), cat.clone()],
            vec![
                vec![Value::Null, s("red")],
                vec![Value::Null, s("blue")],
                vec![s(""), s("red")],
            ],
        ),
    )
    .unwrap();
    assert_eq!(col(&out, "grp"), vec![Value::Null, s("")]);
    assert_eq!(col(&out, "cat_red"), vec![s("T"), s("T")]);
    assert_eq!(col(&out, "cat_blue"), vec![s("T"), s("F")]);

    let empty = run(set_to_flag::build, settings, table(vec![grp, cat], vec![])).unwrap();
    assert_eq!(names(&empty), vec!["grp", "cat_red", "cat_blue"]);
    assert!(empty.rows().is_empty());
}

#[test]
fn set_to_flag_rejects_unknown_settings_empty_selection_and_member_extensions() {
    let unknown = settings_error(
        run(
            set_to_flag::build,
            flag_settings(vec![p("notARealSetting", "1")]),
            cats(),
        )
        .unwrap_err(),
    );
    assert!(unknown.contains("notARealSetting"), "{unknown}");

    let empty = settings_error(
        run(
            set_to_flag::build,
            vec![g("createFlagFields", vec![])],
            cats(),
        )
        .unwrap_err(),
    );
    assert!(empty.contains("no flag values"), "{empty}");

    let extended = g(
        "persistentObjectListMember",
        vec![
            p("srcFieldName", "cat"),
            p("srcFieldValue", "red"),
            p("flagNameExtension", "f_"),
            p("prefixNotSuffix", "true"),
        ],
    );
    let extension = settings_error(
        run(
            set_to_flag::build,
            vec![g("createFlagFields", vec![extended])],
            cats(),
        )
        .unwrap_err(),
    );
    assert!(extension.contains("flagNameExtension"), "{extension}");

    let set_field = settings_error(
        run(
            set_to_flag::build,
            flag_settings(vec![p("setField", "cat")]),
            cats(),
        )
        .unwrap_err(),
    );
    assert!(set_field.contains("setField"), "{set_field}");
}

#[test]
fn set_to_flag_missing_source_is_a_data_error_and_does_not_replace_a_field() {
    let missing = run(
        set_to_flag::build,
        flag_settings(vec![]),
        table(
            vec![Field::new("other", Storage::String)],
            vec![vec![s("red")]],
        ),
    )
    .unwrap_err();
    assert!(matches!(missing, OperatorError::Data(_)), "{missing:?}");

    let mut cat = Field::new("cat", Storage::String);
    cat.measurement = Measurement::Nominal;
    let clash = run(
        set_to_flag::build,
        flag_settings(vec![]),
        table(
            vec![cat, Field::new("cat_red", Storage::String)],
            vec![vec![s("red"), s("keep")]],
        ),
    )
    .unwrap_err();
    assert!(matches!(clash, OperatorError::Data(_)), "{clash:?}");
}

// ---------------------------------------------------------------- transpose

fn continuous(name: &str, storage: Storage) -> Field {
    let mut field = Field::new(name, storage);
    field.measurement = Measurement::Continuous;
    field
}

fn nominal(name: &str) -> Field {
    let mut field = Field::new(name, Storage::String);
    field.measurement = Measurement::Nominal;
    field
}

fn prefix_settings(prefix: &str, count: &str, fields: &[&str]) -> Vec<Node> {
    let mut children = vec![
        p("transposeMethod", "normal"),
        p("transposeFieldNamesMode", "use_prefix"),
        p("transposeFieldNamePrefix", prefix),
        p("transposeNumberOfNewFields", count),
        p("transposeRowIdFieldName", "src_field"),
    ];
    if fields.is_empty() {
        children.push(p("transposeStorageTypeMode", "numeric"));
    } else {
        children.push(p("transposeStorageTypeMode", "custom"));
        children.push(list("transposeCustomFieldList", fields));
    }
    children
}

#[test]
fn transpose_prefix_pads_nulls_drops_extra_records_and_promotes_integers() {
    let input = table(
        vec![
            continuous("id", Storage::Integer),
            nominal("note"),
            continuous("x", Storage::Real),
        ],
        vec![
            vec![int(1), s("a"), Value::Real(-1.5)],
            vec![int(2), s("b"), Value::Null],
            vec![int(3), s("c"), Value::Real(4.0)],
        ],
    );
    let out = run(transpose::build, prefix_settings("col", "2", &[]), input).unwrap();
    assert_eq!(names(&out), vec!["src_field", "col1", "col2"]);
    assert_eq!(col(&out, "src_field"), vec![s("id"), s("x")]);
    assert_eq!(col(&out, "col1"), vec![Value::Real(1.0), Value::Real(-1.5)]);
    assert_eq!(col(&out, "col2"), vec![Value::Real(2.0), Value::Null]);
    assert_eq!(out.model().field("col1").unwrap().storage, Storage::Real);
    assert_eq!(
        out.model().field("col1").unwrap().measurement,
        Measurement::Continuous
    );
    assert_eq!(
        out.model().field("src_field").unwrap().measurement,
        Measurement::Nominal
    );
}

#[test]
fn transpose_custom_order_and_string_mode() {
    let input = table(
        vec![
            continuous("x", Storage::Integer),
            continuous("y", Storage::Integer),
            nominal("note"),
        ],
        vec![vec![int(1), int(2), s("a")], vec![int(3), int(4), s("b")]],
    );
    let custom = run(
        transpose::build,
        prefix_settings("v", "2", &["y", "x"]),
        input.clone(),
    )
    .unwrap();
    assert_eq!(col(&custom, "src_field"), vec![s("y"), s("x")]);
    assert_eq!(col(&custom, "v1"), vec![int(2), int(1)]);
    assert_eq!(
        custom.model().field("v1").unwrap().storage,
        Storage::Integer
    );

    let strings = run(
        transpose::build,
        vec![
            p("transposeMethod", "normal"),
            p("transposeStorageTypeMode", "string"),
            p("transposeFieldNamesMode", "use_prefix"),
            p("transposeFieldNamePrefix", "s"),
            p("transposeNumberOfNewFields", "1"),
            p("transposeRowIdFieldName", "src_field"),
        ],
        input,
    )
    .unwrap();
    assert_eq!(col(&strings, "src_field"), vec![s("note")]);
    assert_eq!(col(&strings, "s1"), vec![s("a")]);
}

#[test]
fn transpose_read_names_keep_nulls_cap_and_do_not_pad() {
    let input = table(
        vec![continuous("x", Storage::Real), nominal("row_name")],
        vec![
            vec![Value::Real(1.0), s("r1")],
            vec![Value::Null, s("r2")],
            vec![Value::Real(3.0), s("r3")],
        ],
    );
    let out = run(
        transpose::build,
        vec![
            p("transposeMethod", "normal"),
            p("transposeStorageTypeMode", "numeric"),
            p("transposeFieldNamesMode", "read_from_field"),
            p("transposeReadFromField", "row_name"),
            p("transposeMaxNumberOfFields", "2"),
            p("transposeRowIdFieldName", "src_field"),
        ],
        input,
    )
    .unwrap();
    assert_eq!(names(&out), vec!["src_field", "r1", "r2"]);
    assert_eq!(col(&out, "r1"), vec![Value::Real(1.0)]);
    assert_eq!(col(&out, "r2"), vec![Value::Null]);
}

#[test]
fn transpose_empty_input_read_mode_has_only_the_id_field() {
    let input = table(
        vec![continuous("x", Storage::Real), nominal("row_name")],
        vec![],
    );
    let out = run(
        transpose::build,
        vec![
            p("transposeStorageTypeMode", "numeric"),
            p("transposeFieldNamesMode", "read_from_field"),
            p("transposeReadFromField", "row_name"),
            p("transposeMaxNumberOfFields", "5"),
            p("transposeRowIdFieldName", "src_field"),
        ],
        input,
    )
    .unwrap();
    assert_eq!(names(&out), vec!["src_field"]);
    assert_eq!(col(&out, "src_field"), vec![s("x")]);

    let no_fields = run(
        transpose::build,
        vec![
            p("transposeStorageTypeMode", "numeric"),
            p("transposeFieldNamesMode", "use_prefix"),
            p("transposeFieldNamePrefix", "col"),
            p("transposeNumberOfNewFields", "3"),
            p("transposeRowIdFieldName", "src_field"),
        ],
        table(vec![nominal("note")], vec![]),
    )
    .unwrap();
    assert_eq!(names(&no_fields), vec!["src_field", "col1", "col2", "col3"]);
    assert!(no_fields.rows().is_empty());
}

#[test]
fn transpose_rejects_other_methods_bad_counts_and_duplicate_or_null_names() {
    let input = table(
        vec![continuous("x", Storage::Real)],
        vec![vec![Value::Real(1.0)]],
    );
    let method = settings_error(
        run(
            transpose::build,
            vec![
                p("transposeMethod", "casetovar"),
                p("transposeRowIdFieldName", "src_field"),
            ],
            input.clone(),
        )
        .unwrap_err(),
    );
    assert!(method.contains("transposeMethod"), "{method}");

    let count = settings_error(
        run(
            transpose::build,
            prefix_settings("col", "0", &[]),
            input.clone(),
        )
        .unwrap_err(),
    );
    assert!(count.contains("transposeNumberOfNewFields"), "{count}");

    let unknown = settings_error(
        run(
            transpose::build,
            prefix_settings("col", "1", &[""]),
            input.clone(),
        )
        .unwrap_err(),
    );
    // empty custom name is rejected before the run; also an unknown child
    assert!(
        unknown.contains("empty field") || unknown.contains("not supported"),
        "{unknown}"
    );

    let odd = settings_error(
        run(
            transpose::build,
            {
                let mut settings = prefix_settings("col", "1", &[]);
                settings.push(p("notARealSetting", "1"));
                settings
            },
            input.clone(),
        )
        .unwrap_err(),
    );
    assert!(odd.contains("notARealSetting"), "{odd}");

    let names = table(
        vec![continuous("x", Storage::Real), nominal("row_name")],
        vec![
            vec![Value::Real(1.0), s("r")],
            vec![Value::Real(2.0), s("r")],
        ],
    );
    let duplicate = run(
        transpose::build,
        vec![
            p("transposeFieldNamesMode", "read_from_field"),
            p("transposeReadFromField", "row_name"),
            p("transposeMaxNumberOfFields", "5"),
            p("transposeRowIdFieldName", "src_field"),
        ],
        names,
    )
    .unwrap_err();
    assert!(matches!(duplicate, OperatorError::Data(_)), "{duplicate:?}");

    let null_name = run(
        transpose::build,
        vec![
            p("transposeFieldNamesMode", "read_from_field"),
            p("transposeReadFromField", "row_name"),
            p("transposeRowIdFieldName", "src_field"),
        ],
        table(
            vec![continuous("x", Storage::Real), nominal("row_name")],
            vec![vec![Value::Null, Value::Null]],
        ),
    )
    .unwrap_err();
    assert!(matches!(null_name, OperatorError::Data(_)), "{null_name:?}");
}

// ---------------------------------------------------------------- history

fn history_settings(
    fields: &[&str],
    offset: &str,
    span: &str,
    unavailable: &str,
    fill: &str,
) -> Vec<Node> {
    vec![
        list("historyFieldList", fields),
        p("historyOffset", offset),
        p("historySpan", span),
        p("historyKeepUndef", unavailable),
        p("historyUndefValue", fill),
    ]
}

fn series() -> Table {
    table(
        vec![
            continuous("x", Storage::Real),
            continuous("y", Storage::Integer),
        ],
        vec![
            vec![Value::Real(1.0), int(10)],
            vec![Value::Null, int(20)],
            vec![Value::Real(3.0), int(30)],
            vec![Value::Real(4.0), int(40)],
        ],
    )
}

#[test]
fn history_discard_drops_the_opening_records_and_keeps_nulls() {
    let out = run(
        history::build,
        history_settings(&["x", "y"], "1", "2", "discard", "undef"),
        series(),
    )
    .unwrap();
    assert_eq!(names(&out), vec!["x", "y", "x__1", "x__2", "y__1", "y__2"]);
    assert_eq!(col(&out, "x"), vec![Value::Real(3.0), Value::Real(4.0)]);
    assert_eq!(col(&out, "x__1"), vec![Value::Null, Value::Real(3.0)]);
    assert_eq!(col(&out, "x__2"), vec![Value::Real(1.0), Value::Null]);
    assert_eq!(col(&out, "y__1"), vec![int(20), int(30)]);
    assert_eq!(col(&out, "y__2"), vec![int(10), int(20)]);
    assert_eq!(out.model().field("x__1").unwrap().storage, Storage::Real);
    assert_eq!(
        out.model().field("x__1").unwrap().measurement,
        Measurement::Continuous
    );
    assert_eq!(out.model().field("y__1").unwrap().storage, Storage::Integer);
}

#[test]
fn history_fill_uses_the_literal_only_when_the_record_is_missing() {
    let out = run(
        history::build,
        history_settings(&["x"], "3", "2", "fill", "-999"),
        series(),
    )
    .unwrap();
    assert_eq!(names(&out), vec!["x", "y", "x__1", "x__2"]);
    assert_eq!(
        col(&out, "x__1"),
        vec![
            Value::Real(-999.0),
            Value::Real(-999.0),
            Value::Real(-999.0),
            Value::Real(1.0),
        ]
    );
    assert_eq!(
        col(&out, "x__2"),
        vec![
            Value::Real(-999.0),
            Value::Real(-999.0),
            Value::Real(-999.0),
            Value::Real(-999.0),
        ]
    );
}

#[test]
fn history_empty_input_keeps_the_new_fields() {
    let input = table(vec![continuous("x", Storage::Real)], vec![]);
    let out = run(
        history::build,
        history_settings(&["x"], "1", "2", "discard", "undef"),
        input,
    )
    .unwrap();
    assert_eq!(names(&out), vec!["x", "x__1", "x__2"]);
    assert!(out.rows().is_empty());
}

#[test]
fn history_string_fill_stays_text_and_a_bad_number_is_a_data_error() {
    let input = table(
        vec![nominal("note"), continuous("n", Storage::Integer)],
        vec![vec![s("a"), int(1)], vec![s("b"), int(2)]],
    );
    let out = run(
        history::build,
        history_settings(&["note"], "1", "1", "fill", "-999"),
        input.clone(),
    )
    .unwrap();
    assert_eq!(col(&out, "note__1"), vec![s("-999"), s("a")]);

    let bad = run(
        history::build,
        history_settings(&["n"], "1", "1", "fill", "abc"),
        input,
    )
    .unwrap_err();
    assert!(matches!(bad, OperatorError::Data(_)), "{bad:?}");
}

#[test]
fn history_rejects_unknown_settings_empty_fields_and_bad_windows() {
    let input = series();
    let unknown = settings_error(
        run(
            history::build,
            {
                let mut settings = history_settings(&["x"], "1", "1", "discard", "undef");
                settings.push(p("notARealSetting", "1"));
                settings
            },
            input.clone(),
        )
        .unwrap_err(),
    );
    assert!(unknown.contains("notARealSetting"), "{unknown}");

    let empty = settings_error(
        run(
            history::build,
            history_settings(&[], "1", "1", "discard", "undef"),
            input.clone(),
        )
        .unwrap_err(),
    );
    assert!(empty.contains("no fields"), "{empty}");

    let leave = settings_error(
        run(
            history::build,
            history_settings(&["x"], "1", "1", "leave", "undef"),
            input.clone(),
        )
        .unwrap_err(),
    );
    assert!(leave.contains("historyKeepUndef"), "{leave}");

    let span = settings_error(
        run(
            history::build,
            history_settings(&["x"], "1", "0", "fill", "0"),
            input,
        )
        .unwrap_err(),
    );
    assert!(span.contains("historySpan"), "{span}");
}
