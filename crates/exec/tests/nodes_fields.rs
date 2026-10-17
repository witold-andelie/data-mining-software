//! Field Operations nodes (Derive, Filler, Reclassify, Filter, Partition) on synthetic tables.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_exec::nodes::{derive, filler, filter, partition, reclassify};
use dms_exec::{ExecContext, Factory, OperatorError};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
use dms_types::{
    DataModel, Field, Measurement, MissingSpec, Role, Storage, Table, Value, ValueSpec,
};
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

fn people() -> Table {
    table(
        vec![
            Field::new("age", Storage::Integer),
            Field::new("income", Storage::Real),
            Field::new("name", Storage::String),
        ],
        vec![
            vec![int(20), Value::Real(10.5), s("a")],
            vec![int(40), Value::Null, s("b")],
            vec![Value::Null, Value::Real(30.0), s("c")],
            vec![int(60), Value::Real(-99.0), s("")],
        ],
    )
}

// ---------------------------------------------------------------- derive

fn derive_settings(mut extra: Vec<Node>, name: &str) -> Vec<Node> {
    let mut all = vec![p("deriveSingleFieldName", name)];
    all.append(&mut extra);
    all
}

#[test]
fn derive_formula_appends_a_field_with_inferred_storage() {
    let out = run(
        derive::build,
        derive_settings(
            vec![
                p("deriveType", "formula"),
                p("deriveFormulaExpression", "age * 2"),
                p("deriveFormulaFieldMeasure", "range"),
            ],
            "double",
        ),
        people(),
    )
    .unwrap();
    assert_eq!(
        col(&out, "double"),
        vec![int(40), int(80), Value::Null, int(120)]
    );
    let field = out.model().field("double").unwrap();
    assert_eq!(field.storage, Storage::Integer);
    assert_eq!(field.measurement, Measurement::Continuous);
    assert_eq!(out.model().position("double"), Some(3));
}

#[test]
fn derive_sequence_functions_see_the_other_records() {
    let derive = |formula: &str| {
        let out = run(
            derive::build,
            derive_settings(
                vec![
                    p("deriveType", "formula"),
                    p("deriveFormulaExpression", formula),
                ],
                "seq",
            ),
            people(),
        )
        .unwrap();
        col(&out, "seq")
    };
    assert_eq!(
        derive("@OFFSET(age, 1)"),
        vec![Value::Null, int(20), int(40), Value::Null]
    );
    assert_eq!(
        derive("@SUM(age)"),
        vec![int(20), int(60), int(60), int(120)]
    );
    assert_eq!(
        derive("@SINCE0(age > 30)"),
        vec![int(2), int(0), int(1), int(0)]
    );
    let flags = run(
        derive::build,
        derive_settings(
            vec![
                p("deriveType", "flag"),
                p("deriveFlagFieldCondition", "@OFFSET(name, -1) = \"c\""),
            ],
            "before_c",
        ),
        people(),
    )
    .unwrap();
    assert_eq!(
        col(&flags, "before_c"),
        vec![s("F"), s("T"), s("F"), s("F")]
    );
}

#[test]
fn derive_formula_mixing_integers_and_reals_is_real_and_replaces_same_name() {
    let out = run(
        derive::build,
        derive_settings(
            vec![p(
                "deriveFormulaExpression",
                "if age > 30 then income else 1 endif",
            )],
            "income",
        ),
        people(),
    )
    .unwrap();
    assert_eq!(out.model().len(), 3, "replaced in place");
    assert_eq!(out.model().field("income").unwrap().storage, Storage::Real);
    assert_eq!(
        col(&out, "income"),
        vec![
            Value::Real(1.0),
            Value::Null,
            Value::Real(1.0),
            Value::Real(-99.0)
        ]
    );
}

#[test]
fn derive_multiple_uses_field_and_extension() {
    let out = run(
        derive::build,
        vec![
            p("deriveMode", "multiple"),
            list("deriveMultipleFieldList", &["age", "income"]),
            p("deriveMultipleExtensionName", "_neg"),
            p("deriveMultipleExtensionPosition", "suffix"),
            p("deriveFormulaExpression", "negate(@FIELD)"),
        ],
        people(),
    )
    .unwrap();
    assert_eq!(col(&out, "age_neg")[0], int(-20));
    assert_eq!(col(&out, "income_neg")[0], Value::Real(-10.5));
    let out = run(
        derive::build,
        vec![
            p("deriveMode", "multiple"),
            list("deriveMultipleFieldList", &["age"]),
            p("deriveMultipleExtensionName", "n_"),
            p("deriveMultipleExtensionPosition", "prefix"),
            p("deriveFormulaExpression", "@FIELD + 1"),
        ],
        people(),
    )
    .unwrap();
    assert_eq!(col(&out, "n_age")[1], int(41));
}

#[test]
fn derive_flag_sets_true_and_false_values_and_treats_null_as_false() {
    let out = run(
        derive::build,
        derive_settings(
            vec![
                p("deriveType", "flag"),
                p("deriveFlagFieldCondition", "age > 30"),
                p("deriveFlagTrueValue", "T"),
                p("deriveFlagFalseValue", "F"),
                p("deriveFlagFieldMeasure", "flag"),
            ],
            "old",
        ),
        people(),
    )
    .unwrap();
    assert_eq!(col(&out, "old"), vec![s("F"), s("T"), s("F"), s("T")]);
    let field = out.model().field("old").unwrap();
    assert_eq!(field.measurement, Measurement::Flag);
    assert_eq!(
        field.values,
        ValueSpec::Flag {
            true_value: s("T"),
            false_value: s("F")
        }
    );
}

#[test]
fn derive_flag_with_numeric_values_is_an_integer_field() {
    let out = run(
        derive::build,
        derive_settings(
            vec![
                p("deriveType", "flag"),
                p("deriveFlagFieldCondition", "age > 30"),
                p("deriveFlagTrueValue", "1"),
                p("deriveFlagFalseValue", "0"),
            ],
            "old",
        ),
        people(),
    )
    .unwrap();
    assert_eq!(col(&out, "old"), vec![int(0), int(1), int(0), int(1)]);
    assert_eq!(out.model().field("old").unwrap().storage, Storage::Integer);
}

fn set_rows(rows: &[(&str, &str)]) -> Node {
    g(
        "deriveSetTable",
        vec![g(
            "rows",
            rows.iter()
                .enumerate()
                .map(|(i, (name, condition))| {
                    g(
                        "row",
                        vec![
                            p("index", &i.to_string()),
                            p("name", name),
                            p("condition", condition),
                        ],
                    )
                })
                .collect(),
        )],
    )
}

#[test]
fn derive_set_takes_the_first_matching_row_else_the_default() {
    let out = run(
        derive::build,
        derive_settings(
            vec![
                p("deriveType", "set"),
                set_rows(&[("young", "age < 30"), ("mid", "age < 50")]),
                p("deriveSetDefaultValue", "old"),
            ],
            "group",
        ),
        people(),
    )
    .unwrap();
    assert_eq!(
        col(&out, "group"),
        vec![s("young"), s("mid"), s("old"), s("old")]
    );
}

#[test]
fn derive_set_with_numeric_values_is_an_integer_field() {
    let out = run(
        derive::build,
        derive_settings(
            vec![
                p("deriveType", "set"),
                set_rows(&[("1", "age < 30"), ("2", "age < 50")]),
                p("deriveSetDefaultValue", "-100"),
            ],
            "group",
        ),
        people(),
    )
    .unwrap();
    assert_eq!(
        col(&out, "group"),
        vec![int(1), int(2), int(-100), int(-100)]
    );
    assert_eq!(
        out.model().field("group").unwrap().storage,
        Storage::Integer
    );
    // A non-numeric default makes the whole field a string.
    let out = run(
        derive::build,
        derive_settings(
            vec![
                p("deriveType", "set"),
                set_rows(&[("1", "age < 30")]),
                p("deriveSetDefaultValue", "default"),
            ],
            "group",
        ),
        people(),
    )
    .unwrap();
    assert_eq!(col(&out, "group")[0], s("1"));
}

#[test]
fn derive_state_switches_on_and_off() {
    let input = table(
        vec![Field::new("x", Storage::Integer)],
        [0, 5, 6, 1, 7, 0].iter().map(|v| vec![int(*v)]).collect(),
    );
    let out = run(
        derive::build,
        derive_settings(
            vec![
                p("deriveType", "state"),
                p("deriveStateOnCondition", "x > 4"),
                p("deriveStateOffCondition", "x < 2"),
                p("deriveStateInitialState", "off"),
                p("deriveStateOnValue", "On"),
                p("deriveStateOffValue", "Off"),
            ],
            "st",
        ),
        input,
    )
    .unwrap();
    let expected = ["Off", "On", "On", "Off", "On", "Off"].map(s).to_vec();
    assert_eq!(col(&out, "st"), expected);
}

#[test]
fn derive_count_increments_and_resets() {
    let input = table(
        vec![Field::new("x", Storage::Integer)],
        [1, 1, 0, 1, 9, 1].iter().map(|v| vec![int(*v)]).collect(),
    );
    let out = run(
        derive::build,
        derive_settings(
            vec![
                p("deriveType", "count"),
                p("deriveCountInitialValue", "5"),
                p("deriveCountIncrementCondition", "x = 1"),
                p("deriveCountIncrementByExpression", "2"),
                p("deriveCountResetCondition", "x = 9"),
            ],
            "c",
        ),
        input,
    )
    .unwrap();
    assert_eq!(
        col(&out, "c"),
        vec![int(7), int(9), int(9), int(11), int(5), int(7)]
    );
}

#[test]
fn derive_conditional_without_else_gives_null() {
    let out = run(
        derive::build,
        derive_settings(
            vec![
                p("deriveType", "conditional"),
                p("deriveConditionalIfCondition", "age > 30"),
                p("deriveConditionalThenExpression", "\"big\""),
                p("deriveConditionalElseExpression", ""),
            ],
            "size",
        ),
        people(),
    )
    .unwrap();
    assert_eq!(
        col(&out, "size"),
        vec![Value::Null, s("big"), Value::Null, s("big")]
    );
}

#[test]
fn derive_rejects_bad_settings_and_unknown_fields_without_panicking() {
    assert!(matches!(
        derive::build(&node(derive_settings(
            vec![p("deriveFormulaExpression", "")],
            "a"
        )))
        .err()
        .unwrap(),
        OperatorError::Settings(_)
    ));
    assert!(matches!(
        derive::build(&node(derive_settings(
            vec![p("deriveFormulaExpression", "1 +")],
            "a"
        )))
        .err()
        .unwrap(),
        OperatorError::Settings(_)
    ));
    assert!(derive::build(&node(vec![p("deriveSingleFieldName", "")])).is_err());
    assert!(derive::build(&node(derive_settings(vec![p("deriveType", "bogus")], "a"))).is_err());
    let err = run(
        derive::build,
        derive_settings(vec![p("deriveFormulaExpression", "nosuch + 1")], "a"),
        people(),
    )
    .unwrap_err();
    assert!(matches!(err, OperatorError::Data(_)));
    // A type error at evaluation time is an error, not a panic.
    let err = run(
        derive::build,
        derive_settings(vec![p("deriveFormulaExpression", "name > 1")], "a"),
        people(),
    )
    .unwrap_err();
    assert!(matches!(err, OperatorError::Data(_)));
}

#[test]
fn derive_on_an_empty_table_adds_the_field() {
    let empty = table(vec![Field::new("age", Storage::Integer)], Vec::new());
    let out = run(
        derive::build,
        derive_settings(vec![p("deriveFormulaExpression", "age + 1")], "b"),
        empty,
    )
    .unwrap();
    assert_eq!(out.model().len(), 2);
    assert_eq!(out.row_count(), 0);
}

// ---------------------------------------------------------------- filler

#[test]
fn filler_nulls_mode_replaces_only_nulls_in_selected_fields() {
    let out = run(
        filler::build,
        vec![
            list("fillerSelectedFields", &["income"]),
            p("fillerReplaceMode", "nulls"),
            p("fillerReplaceExpression", "11.51"),
        ],
        people(),
    )
    .unwrap();
    assert_eq!(
        col(&out, "income"),
        vec![
            Value::Real(10.5),
            Value::Real(11.51),
            Value::Real(30.0),
            Value::Real(-99.0)
        ]
    );
    assert_eq!(col(&out, "age")[2], Value::Null, "other fields untouched");
}

#[test]
fn filler_blanks_use_the_missing_value_definition() {
    let mut input = people();
    let (model, rows) = {
        let mut fields = input.model().fields().to_vec();
        fields[1].missing = MissingSpec {
            values: vec![Value::Real(-99.0)],
            ..MissingSpec::default()
        };
        (DataModel::new(fields).unwrap(), input.rows().to_vec())
    };
    input = Table::new(model, rows).unwrap();
    let blanks = run(
        filler::build,
        vec![
            list("fillerSelectedFields", &["income"]),
            p("fillerReplaceMode", "blanks"),
            p("fillerReplaceExpression", "0"),
        ],
        input.clone(),
    )
    .unwrap();
    assert_eq!(
        col(&blanks, "income"),
        vec![
            Value::Real(10.5),
            Value::Null,
            Value::Real(30.0),
            Value::Real(0.0)
        ]
    );
    let both = run(
        filler::build,
        vec![
            list("fillerSelectedFields", &["income"]),
            p("fillerReplaceMode", "blanksAndNulls"),
            p("fillerReplaceExpression", "0"),
        ],
        input.clone(),
    )
    .unwrap();
    assert_eq!(
        col(&both, "income"),
        vec![
            Value::Real(10.5),
            Value::Real(0.0),
            Value::Real(30.0),
            Value::Real(0.0)
        ]
    );
    // The default conditional `@BLANK(@FIELD)` is the blanks of the field.
    let default = run(
        filler::build,
        vec![
            list("fillerSelectedFields", &["income"]),
            p("fillerReplaceExpression", "1"),
        ],
        input,
    )
    .unwrap();
    assert_eq!(col(&default, "income")[3], Value::Real(1.0));
}

#[test]
fn filler_conditional_and_always_use_field_in_the_replacement() {
    let out = run(
        filler::build,
        vec![
            list("fillerSelectedFields", &["age", "income"]),
            p("fillerReplaceMode", "conditional"),
            p("fillerCondition", "@FIELD > 25"),
            p("fillerReplaceExpression", "@FIELD * 2"),
        ],
        people(),
    )
    .unwrap();
    assert_eq!(
        col(&out, "age"),
        vec![int(20), int(80), Value::Null, int(120)]
    );
    assert_eq!(col(&out, "income")[2], Value::Real(60.0));
    let out = run(
        filler::build,
        vec![
            list("fillerSelectedFields", &["age"]),
            p("fillerReplaceMode", "always"),
            p("fillerReplaceExpression", "7"),
        ],
        people(),
    )
    .unwrap();
    assert_eq!(col(&out, "age"), vec![int(7); 4]);
}

#[test]
fn filler_changes_storage_with_the_new_values() {
    let out = run(
        filler::build,
        vec![
            list("fillerSelectedFields", &["age"]),
            p("fillerReplaceMode", "always"),
            p("fillerReplaceExpression", "to_string(@FIELD)"),
        ],
        people(),
    )
    .unwrap();
    assert_eq!(out.model().field("age").unwrap().storage, Storage::String);
    assert_eq!(col(&out, "age")[0], s("20"));
    // Integers filled into a real column keep it real.
    let out = run(
        filler::build,
        vec![
            list("fillerSelectedFields", &["income"]),
            p("fillerReplaceMode", "nulls"),
            p("fillerReplaceExpression", "1"),
        ],
        people(),
    )
    .unwrap();
    assert_eq!(out.model().field("income").unwrap().storage, Storage::Real);
    assert_eq!(col(&out, "income")[1], Value::Real(1.0));
}

#[test]
fn filler_errors_on_missing_fields_and_bad_settings() {
    assert!(filler::build(&node(vec![p("fillerReplaceExpression", "1")])).is_err());
    assert!(
        filler::build(&node(vec![
            list("fillerSelectedFields", &["a"]),
            p("fillerReplaceMode", "sometimes")
        ]))
        .is_err()
    );
    let err = run(
        filler::build,
        vec![list("fillerSelectedFields", &["nosuch"])],
        people(),
    )
    .unwrap_err();
    assert!(matches!(err, OperatorError::Data(_)));
    let empty = table(vec![Field::new("a", Storage::Integer)], Vec::new());
    let out = run(
        filler::build,
        vec![list("fillerSelectedFields", &["a"])],
        empty,
    )
    .unwrap();
    assert_eq!(out.row_count(), 0);
}

// ------------------------------------------------------------ reclassify

fn mapping(rows: &[(&str, &str)]) -> Node {
    g(
        "reclassifyTable",
        vec![g(
            "rows",
            rows.iter()
                .map(|(old, new)| g("row", vec![p("oldValue", old), p("newValue", new)]))
                .collect(),
        )],
    )
}

fn codes(storage: Storage, values: &[Value]) -> Table {
    table(
        vec![Field::new("code", storage)],
        values.iter().map(|v| vec![v.clone()]).collect(),
    )
}

#[test]
fn reclassify_integers_to_group_strings() {
    let out = run(
        reclassify::build,
        vec![
            p("singleFieldSelection", "code"),
            p("singleFieldName", "group"),
            mapping(&[("1", "Group1"), ("2", "Group1"), ("3", "Group2")]),
        ],
        codes(
            Storage::Integer,
            &[int(1), int(2), int(3), int(4), Value::Null],
        ),
    )
    .unwrap();
    assert_eq!(
        col(&out, "group"),
        vec![s("Group1"), s("Group1"), s("Group2"), s("4"), Value::Null]
    );
    assert_eq!(out.model().field("group").unwrap().storage, Storage::String);
    assert_eq!(out.model().len(), 2);
}

#[test]
fn reclassify_numeric_to_numeric_keeps_unmapped_values_and_real_storage() {
    let out = run(
        reclassify::build,
        vec![
            p("singleFieldSelection", "code"),
            p("singleFieldName", "four"),
            mapping(&[("5", "4")]),
        ],
        codes(Storage::Real, &[Value::Real(1.0), Value::Real(5.0)]),
    )
    .unwrap();
    assert_eq!(col(&out, "four"), vec![Value::Real(1.0), Value::Real(4.0)]);
    assert_eq!(out.model().field("four").unwrap().storage, Storage::Real);
}

#[test]
fn reclassify_default_replace_and_multiple_modes() {
    let out = run(
        reclassify::build,
        vec![
            p("singleFieldSelection", "code"),
            p("reclassifyReplaceField", "true"),
            mapping(&[("a", "x")]),
            p("useDefault", "true"),
            p("defaultValue", "other"),
        ],
        codes(Storage::String, &[s("a"), s("b"), Value::Null]),
    )
    .unwrap();
    assert_eq!(out.model().len(), 1);
    assert_eq!(col(&out, "code"), vec![s("x"), s("other"), Value::Null]);

    let out = run(
        reclassify::build,
        vec![
            p("reclassifyMode", "multiple"),
            list("multipleFieldList", &["code"]),
            p("multipleExtensionName", "_r"),
            p("multipleExtensionPosition", "prefix"),
            mapping(&[("a", "x")]),
            p("useDefault", "true"),
            p("defaultValue", "undef"),
        ],
        codes(Storage::String, &[s("a"), s("b")]),
    )
    .unwrap();
    assert_eq!(col(&out, "_rcode"), vec![s("x"), Value::Null]);
    assert_eq!(
        out.model().field("_rcode").unwrap().storage,
        Storage::String
    );
}

#[test]
fn reclassify_rejects_bad_settings() {
    assert!(reclassify::build(&node(vec![p("singleFieldName", "x")])).is_err());
    assert!(
        reclassify::build(&node(vec![p("singleFieldSelection", "a")])).is_err(),
        "new field needs a name"
    );
    assert!(reclassify::build(&node(vec![p("reclassifyMode", "multiple")])).is_err());
    let err = run(
        reclassify::build,
        vec![
            p("singleFieldSelection", "nosuch"),
            p("singleFieldName", "n"),
        ],
        codes(Storage::Integer, &[]),
    )
    .unwrap_err();
    assert!(matches!(err, OperatorError::Data(_)));
}

// ---------------------------------------------------------------- filter

fn filter_row(name: &str, drop: Option<bool>, rename: Option<&str>) -> Node {
    let mut children = vec![p("name", name)];
    if let Some(drop) = drop {
        children.push(p("drop", &drop.to_string()));
    }
    if let Some(rename) = rename {
        children.push(p("rename", rename));
    }
    g("row", children)
}

#[test]
fn filter_drops_and_renames() {
    let out = run(
        filter::build,
        vec![g(
            "filterTable",
            vec![
                p("defaultDrop", "false"),
                g(
                    "rows",
                    vec![
                        filter_row("age", Some(true), None),
                        filter_row("name", Some(false), Some("label")),
                        filter_row("ghost", Some(true), None),
                    ],
                ),
            ],
        )],
        people(),
    )
    .unwrap();
    let names: Vec<_> = out
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, vec!["income", "label"]);
    assert_eq!(out.rows()[0], vec![Value::Real(10.5), s("a")]);
}

#[test]
fn filter_default_drop_keeps_only_listed_fields() {
    let out = run(
        filter::build,
        vec![g(
            "filterTable",
            vec![
                p("defaultDrop", "true"),
                g("rows", vec![filter_row("income", Some(false), None)]),
            ],
        )],
        people(),
    )
    .unwrap();
    assert_eq!(out.model().len(), 1);
    assert_eq!(out.row_count(), 4);
}

#[test]
fn filter_without_settings_passes_everything_and_duplicate_names_fail() {
    let out = run(filter::build, Vec::new(), people()).unwrap();
    assert_eq!(out, people());
    let err = run(
        filter::build,
        vec![g(
            "filterTable",
            vec![g("rows", vec![filter_row("age", None, Some("name"))])],
        )],
        people(),
    )
    .unwrap_err();
    assert!(matches!(err, OperatorError::Data(_)));
    assert!(
        filter::build(&node(vec![g(
            "filterTable",
            vec![p("defaultDrop", "maybe")]
        )]))
        .is_err()
    );
}

// ------------------------------------------------------------- partition

fn many(n: usize) -> Table {
    table(
        vec![Field::new("x", Storage::Integer)],
        (0..n).map(|i| vec![int(i as i64)]).collect(),
    )
}

#[test]
fn partition_adds_a_partition_field_with_roles_and_values() {
    let out = run(
        partition::build,
        vec![
            p("partitionFieldName", "Partition"),
            p("trainingPartitionSize", "70"),
            p("testingPartitionSize", "30"),
            p("partitionValueMode", "system_label"),
            p("partitionRandomSeed", "1234567"),
        ],
        many(2000),
    )
    .unwrap();
    let field = out.model().field("Partition").unwrap();
    assert_eq!(field.role, Role::Partition);
    assert_eq!(
        field.values,
        ValueSpec::Categories(vec![s("1_Training"), s("2_Testing")])
    );
    let training = col(&out, "Partition")
        .iter()
        .filter(|v| **v == s("1_Training"))
        .count();
    assert!(
        (1300..=1500).contains(&training),
        "training count {training}"
    );
    // Deterministic for a given seed, different for another.
    let again = run(
        partition::build,
        vec![
            p("trainingPartitionSize", "70"),
            p("testingPartitionSize", "30"),
            p("partitionRandomSeed", "1234567"),
        ],
        many(2000),
    )
    .unwrap();
    assert_eq!(col(&out, "Partition"), col(&again, "Partition"));
    let other = run(
        partition::build,
        vec![
            p("trainingPartitionSize", "70"),
            p("testingPartitionSize", "30"),
            p("partitionRandomSeed", "42"),
        ],
        many(2000),
    )
    .unwrap();
    assert_ne!(col(&out, "Partition"), col(&other, "Partition"));
}

#[test]
fn partition_value_modes_validation_and_downstream_specials() {
    let out = run(
        partition::build,
        vec![
            p("partitionFieldName", "P"),
            p("createValidationPartition", "true"),
            p("trainingPartitionSize", "40"),
            p("testingPartitionSize", "30"),
            p("validationPartitionSize", "30"),
            p("partitionValueMode", "label"),
        ],
        many(300),
    )
    .unwrap();
    let field = out.model().field("P").unwrap();
    assert_eq!(
        field.values,
        ValueSpec::Categories(vec![s("Training"), s("Testing"), s("Validation")])
    );
    let validation = col(&out, "P")
        .iter()
        .filter(|v| **v == s("Validation"))
        .count();
    assert!(validation > 40, "validation count {validation}");

    let out = run(
        partition::build,
        vec![p("partitionValueMode", "system")],
        many(10),
    )
    .unwrap();
    // `system` values are the strings 1, 2 (`oracle/random/part_default`).
    assert_eq!(
        out.model().field("Partition").unwrap().storage,
        Storage::String
    );
    assert!(
        col(&out, "Partition")
            .iter()
            .all(|v| *v == s("1") || *v == s("2"))
    );

    // @PARTITION_FIELD = @TRAINING_PARTITION works on the output.
    let compiled = dms_clem::compile(
        &dms_clem::parse("@PARTITION_FIELD = @TRAINING_PARTITION").unwrap(),
        &dms_clem::Bindings {
            model: out.model(),
            current_field: None,
        },
    )
    .unwrap();
    let hits = out
        .rows()
        .iter()
        .filter(|row| compiled.eval(row, 1).unwrap().truth().unwrap() == Some(true))
        .count();
    assert_eq!(
        hits,
        col(&out, "Partition")
            .iter()
            .filter(|v| **v == s("1"))
            .count()
    );
}

#[test]
fn partition_handles_empty_input_and_rejects_bad_settings() {
    let out = run(partition::build, Vec::new(), many(0)).unwrap();
    assert_eq!(out.row_count(), 0);
    assert_eq!(out.model().len(), 2);
    assert!(
        partition::build(&node(vec![
            p("trainingPartitionSize", "0"),
            p("testingPartitionSize", "0")
        ]))
        .is_err()
    );
    assert!(partition::build(&node(vec![p("usePartitionHashColumn", "true")])).is_err());
    assert!(partition::build(&node(vec![p("partitionRandomSeed", "abc")])).is_err());
    // Out-of-range sizes are clamped, as Modeler does.
    assert!(partition::build(&node(vec![p("trainingPartitionSize", "150")])).is_ok());
}
