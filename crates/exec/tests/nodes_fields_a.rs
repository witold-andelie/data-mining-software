//! Field Operations nodes from the fields-a work: Field Reorder, then Binning.
//! Rules the corpus probes do not cover: rejected settings, empty input, nulls, and orders
//! the four reorder streams do not show.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_exec::nodes::{binning, field_reorder};
use dms_exec::{ExecContext, Factory, OperatorError};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
use dms_types::{DataModel, Field, Measurement, Role, Storage, Table, Value, ValueSpec};
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

fn field(name: &str, storage: Storage, measurement: Measurement) -> Field {
    let mut field = Field::new(name, storage);
    field.measurement = measurement;
    field
}

fn table(fields: Vec<Field>, rows: Vec<Vec<Value>>) -> Table {
    Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
}

fn names(table: &Table) -> Vec<&str> {
    table
        .model()
        .fields()
        .iter()
        .map(|field| field.name.as_str())
        .collect()
}

fn col(table: &Table, name: &str) -> Vec<Value> {
    table.column(name).unwrap().cloned().collect()
}

fn reorder_input() -> Table {
    let mut age = field("age", Storage::Integer, Measurement::Continuous);
    age.role = Role::Target;
    age.values = ValueSpec::Range {
        low: Value::Integer(1),
        high: Value::Integer(9),
    };
    age.label = Some("Age".into());
    table(
        vec![
            age,
            field("sex", Storage::String, Measurement::Flag),
            field("drug", Storage::String, Measurement::Nominal),
            field("na", Storage::Real, Measurement::Continuous),
        ],
        vec![
            vec![
                Value::Integer(1),
                Value::string("T"),
                Value::string("drugA"),
                Value::Real(0.5),
            ],
            vec![
                Value::Null,
                Value::string("F"),
                Value::Null,
                Value::Real(-1.5),
            ],
        ],
    )
}

fn custom(start: &[&str], end: &[&str]) -> Vec<Node> {
    let mut children = vec![p("dividerIndex", &start.len().to_string())];
    children.extend(start.iter().map(|name| p("fieldName", name)));
    children.extend(end.iter().map(|name| p("fieldName", name)));
    vec![
        p("autoOrderType", "custom"),
        g("fieldReorderTable", children),
    ]
}

#[test]
fn reorder_custom_puts_unlisted_fields_between_the_lists() {
    let out = run(
        field_reorder::build,
        custom(&["drug", "na"], &["age"]),
        reorder_input(),
    )
    .unwrap();
    assert_eq!(names(&out), vec!["drug", "na", "sex", "age"]);
    assert_eq!(
        out.column("age").unwrap().cloned().collect::<Vec<_>>(),
        vec![Value::Integer(1), Value::Null]
    );
    let age = out.model().field("age").unwrap();
    assert_eq!(age.storage, Storage::Integer);
    assert_eq!(age.measurement, Measurement::Continuous);
    assert_eq!(age.role, Role::Target);
    assert_eq!(
        age.values,
        ValueSpec::Range {
            low: Value::Integer(1),
            high: Value::Integer(9),
        }
    );
    assert_eq!(age.label.as_deref(), Some("Age"));
    assert_eq!(
        out.column("drug").unwrap().cloned().collect::<Vec<_>>(),
        vec![Value::string("drugA"), Value::Null]
    );
}

#[test]
fn reorder_custom_skips_missing_names_and_repeats() {
    let out = run(
        field_reorder::build,
        custom(&["missing", "sex", "sex"], &["gone", "drug"]),
        reorder_input(),
    )
    .unwrap();
    assert_eq!(names(&out), vec!["sex", "age", "na", "drug"]);
}

#[test]
fn reorder_custom_divider_edges() {
    let all_end = run(
        field_reorder::build,
        vec![
            p("autoOrderType", "custom"),
            g(
                "fieldReorderTable",
                vec![
                    p("dividerIndex", "0"),
                    p("fieldName", "na"),
                    p("fieldName", "age"),
                ],
            ),
        ],
        reorder_input(),
    )
    .unwrap();
    assert_eq!(names(&all_end), vec!["sex", "drug", "na", "age"]);

    let all_start = run(
        field_reorder::build,
        vec![
            p("autoOrderType", "custom"),
            g(
                "fieldReorderTable",
                vec![
                    p("dividerIndex", "9"),
                    p("fieldName", "drug"),
                    p("fieldName", "age"),
                ],
            ),
        ],
        reorder_input(),
    )
    .unwrap();
    assert_eq!(names(&all_start), vec!["drug", "age", "sex", "na"]);

    let negative = run(
        field_reorder::build,
        vec![
            p("autoOrderType", "custom"),
            g(
                "fieldReorderTable",
                vec![
                    p("dividerIndex", "-1"),
                    p("fieldName", "na"),
                    p("fieldName", "sex"),
                ],
            ),
        ],
        reorder_input(),
    )
    .unwrap();
    assert_eq!(names(&negative), vec!["age", "drug", "na", "sex"]);
}

#[test]
fn reorder_by_name_is_code_point_order() {
    let asc = run(
        field_reorder::build,
        vec![
            p("autoOrderType", "auto"),
            p("nameTypeStorage", "Name"),
            p("ascendingDescending", "true"),
        ],
        reorder_input(),
    )
    .unwrap();
    assert_eq!(names(&asc), vec!["age", "drug", "na", "sex"]);

    let desc = run(
        field_reorder::build,
        vec![
            p("autoOrderType", "auto"),
            p("nameTypeStorage", "Name"),
            p("ascendingDescending", "false"),
        ],
        reorder_input(),
    )
    .unwrap();
    assert_eq!(names(&desc), vec!["sex", "na", "drug", "age"]);

    let mixed = table(
        vec![
            field("b", Storage::String, Measurement::Nominal),
            field("A", Storage::String, Measurement::Nominal),
            field("a", Storage::String, Measurement::Nominal),
        ],
        vec![vec![
            Value::string("b"),
            Value::string("A"),
            Value::string("a"),
        ]],
    );
    let out = run(
        field_reorder::build,
        vec![p("autoOrderType", "auto"), p("nameTypeStorage", "Name")],
        mixed,
    )
    .unwrap();
    // 'A' (U+0041) precedes 'a' and 'b'. The probes use lowercase names only.
    assert_eq!(names(&out), vec!["A", "a", "b"]);
}

#[test]
fn reorder_by_type_groups_measurement_then_name() {
    let asc = run(
        field_reorder::build,
        vec![
            p("autoOrderType", "auto"),
            p("nameTypeStorage", "Type"),
            p("ascendingDescending", "true"),
        ],
        reorder_input(),
    )
    .unwrap();
    assert_eq!(names(&asc), vec!["age", "na", "sex", "drug"]);

    let desc = run(
        field_reorder::build,
        vec![
            p("autoOrderType", "auto"),
            p("nameTypeStorage", "Type"),
            p("ascendingDescending", "false"),
        ],
        reorder_input(),
    )
    .unwrap();
    assert_eq!(names(&desc), vec!["drug", "sex", "na", "age"]);
}

#[test]
fn reorder_empty_input_keeps_the_new_order() {
    let input = table(
        vec![
            field("b", Storage::Integer, Measurement::Continuous),
            field("a", Storage::Integer, Measurement::Continuous),
        ],
        Vec::new(),
    );
    let out = run(
        field_reorder::build,
        vec![p("autoOrderType", "auto"), p("ascendingDescending", "true")],
        input,
    )
    .unwrap();
    assert_eq!(names(&out), vec!["a", "b"]);
    assert_eq!(out.row_count(), 0);

    let bare = run(
        field_reorder::build,
        vec![p("autoOrderType", "custom")],
        table(Vec::new(), Vec::new()),
    )
    .unwrap();
    assert!(bare.model().is_empty());
    assert_eq!(bare.row_count(), 0);
}

#[test]
fn reorder_rejects_unknown_settings() {
    let storage = run(
        field_reorder::build,
        vec![p("autoOrderType", "auto"), p("nameTypeStorage", "Storage")],
        reorder_input(),
    );
    assert!(
        matches!(storage, Err(OperatorError::Settings(message)) if message.contains("Storage"))
    );

    let criterion = run(
        field_reorder::build,
        vec![p("autoOrderType", "auto"), p("nameTypeStorage", "Label")],
        reorder_input(),
    );
    assert!(matches!(criterion, Err(OperatorError::Settings(_))));

    let mode = run(
        field_reorder::build,
        vec![p("autoOrderType", "Custom")],
        reorder_input(),
    );
    assert!(
        matches!(mode, Err(OperatorError::Settings(message)) if message.contains("autoOrderType"))
    );

    let flag = run(
        field_reorder::build,
        vec![p("autoOrderType", "auto"), p("ascendingDescending", "yes")],
        reorder_input(),
    );
    assert!(matches!(flag, Err(OperatorError::Settings(_))));

    let divider = run(
        field_reorder::build,
        vec![
            p("autoOrderType", "custom"),
            g("fieldReorderTable", vec![p("dividerIndex", "1.5")]),
        ],
        reorder_input(),
    );
    assert!(matches!(divider, Err(OperatorError::Settings(_))));
}

fn real(value: f64) -> Value {
    Value::Real(value)
}

/// `x` from `fields_main.csv`, including the null on row 8.
fn probe_x() -> Vec<Value> {
    [
        -10.0,
        -10.0,
        -2.5,
        0.0,
        0.0,
        0.0,
        2.5,
        f64::NAN,
        5.0,
        5.0,
        7.5,
        10.0,
        10.0,
        10.0,
        15.0,
        20.0,
    ]
    .into_iter()
    .map(|value| {
        if value.is_nan() {
            Value::Null
        } else {
            real(value)
        }
    })
    .collect()
}

fn probe_y() -> Vec<Value> {
    [
        -3.5,
        0.0,
        1.25,
        -2.5,
        4.5,
        10.0,
        -1.5,
        2.5,
        8.0,
        0.0,
        -3.5,
        f64::NAN,
        3.5,
        1.25,
        9.0,
        -0.5,
    ]
    .into_iter()
    .map(|value| {
        if value.is_nan() {
            Value::Null
        } else {
            real(value)
        }
    })
    .collect()
}

fn column_table(name: &str, storage: Storage, values: Vec<Value>) -> Table {
    table(
        vec![field(name, storage, Measurement::Continuous)],
        vec_rows(&values),
    )
}

fn vec_rows(values: &[Value]) -> Vec<Vec<Value>> {
    values.iter().cloned().map(|value| vec![value]).collect()
}

fn ints(values: &[Value]) -> Vec<Option<i64>> {
    values
        .iter()
        .map(|value| match value {
            Value::Integer(integer) => Some(*integer),
            Value::Null => None,
            other => panic!("expected an integer bin, found {other:?}"),
        })
        .collect()
}

fn bin_fields(names: &[&str]) -> Node {
    list_fields("binningFields", names)
}

fn list_fields(name: &str, names: &[&str]) -> Node {
    g(name, names.iter().map(|field| p("field", field)).collect())
}

// ---------------------------------------------------------------- binning

#[test]
fn bin_width_matches_the_probe_and_centers_the_first_bin_on_the_minimum() {
    let out = run(
        binning::build,
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "fixedWidth"),
            p("fixedBinMethod", "width"),
            p("fixedBinWidth", "5"),
        ],
        column_table("x", Storage::Real, probe_x()),
    )
    .unwrap();
    assert_eq!(names(&out), vec!["x", "x_BIN"]);
    assert_eq!(
        ints(&col(&out, "x_BIN")),
        vec![
            Some(1),
            Some(1),
            Some(3),
            Some(3),
            Some(3),
            Some(3),
            Some(4),
            None,
            Some(4),
            Some(4),
            Some(5),
            Some(5),
            Some(5),
            Some(5),
            Some(6),
            Some(7)
        ]
    );
    let bin = out.model().field("x_BIN").unwrap();
    assert_eq!(bin.storage, Storage::Integer);
    assert_eq!(bin.measurement, Measurement::Ordinal);

    // Minimum 1 is not a multiple of width 5. The first bin is [1 - 2.5, 1 + 2.5).
    let shifted = run(
        binning::build,
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "fixedWidth"),
            p("fixedBinMethod", "width"),
            p("fixedBinWidth", "5"),
            p("fixedBinExtensionPosition", "prefix"),
            p("fixedBinExtensionName", "b_"),
        ],
        column_table("x", Storage::Real, vec![real(1.0), real(3.0), real(8.0)]),
    )
    .unwrap();
    assert_eq!(names(&shifted), vec!["x", "b_x"]);
    assert_eq!(ints(&col(&shifted, "b_x")), vec![Some(1), Some(1), Some(2)]);
}

#[test]
fn bin_count_matches_the_probe() {
    let rows = probe_x()
        .into_iter()
        .zip(probe_y())
        .map(|(x, y)| vec![x, y])
        .collect();
    let input = table(
        vec![
            field("x", Storage::Real, Measurement::Continuous),
            field("y", Storage::Real, Measurement::Continuous),
        ],
        rows,
    );
    let out = run(
        binning::build,
        vec![
            bin_fields(&["x", "y"]),
            p("binningMethod", "fixedWidth"),
            p("fixedBinMethod", "count"),
            p("fixedBinCount", "4"),
        ],
        input,
    )
    .unwrap();
    assert_eq!(names(&out), vec!["x", "y", "x_BIN", "y_BIN"]);
    assert_eq!(
        ints(&col(&out, "x_BIN")),
        vec![
            Some(1),
            Some(1),
            Some(2),
            Some(2),
            Some(2),
            Some(2),
            Some(2),
            None,
            Some(3),
            Some(3),
            Some(3),
            Some(3),
            Some(3),
            Some(3),
            Some(4),
            Some(4)
        ]
    );
    assert_eq!(
        ints(&col(&out, "y_BIN")),
        vec![
            Some(1),
            Some(2),
            Some(2),
            Some(1),
            Some(3),
            Some(4),
            Some(1),
            Some(2),
            Some(4),
            Some(2),
            Some(1),
            None,
            Some(3),
            Some(2),
            Some(4),
            Some(1)
        ]
    );
}

#[test]
fn bin_tiles_match_the_quartile_and_decile_probes() {
    let quartile = run(
        binning::build,
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "equalFreq"),
            p("binTile4", "true"),
            p("binTile10", "false"),
            p("tiedValuesMethod", "high"),
            p("tilingMethod", "count"),
        ],
        column_table("x", Storage::Real, probe_x()),
    )
    .unwrap();
    assert_eq!(
        ints(&col(&quartile, "x_TILE4")),
        vec![
            Some(1),
            Some(1),
            Some(1),
            Some(2),
            Some(2),
            Some(2),
            Some(2),
            None,
            Some(3),
            Some(3),
            Some(3),
            Some(4),
            Some(4),
            Some(4),
            Some(4),
            Some(4)
        ]
    );

    let decile = run(
        binning::build,
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "equalFreq"),
            p("binTile4", "false"),
            p("binTile10", "true"),
            p("tiedValuesMethod", "low"),
        ],
        column_table("x", Storage::Real, probe_x()),
    )
    .unwrap();
    assert_eq!(
        ints(&col(&decile, "x_TILE10")),
        vec![
            Some(1),
            Some(1),
            Some(2),
            Some(2),
            Some(2),
            Some(2),
            Some(3),
            None,
            Some(4),
            Some(4),
            Some(5),
            Some(6),
            Some(6),
            Some(6),
            Some(7),
            Some(8)
        ]
    );
}

#[test]
fn bin_ranks_match_the_probes() {
    let rank = run(
        binning::build,
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "rank"),
            p("binByRank", "true"),
            p("binByFractionalRank", "false"),
            p("binByPcFractionalRank", "false"),
        ],
        column_table("x", Storage::Real, probe_x()),
    )
    .unwrap();
    assert_eq!(
        col(&rank, "x_RANK"),
        vec![
            real(1.5),
            real(1.5),
            real(3.0),
            real(5.0),
            real(5.0),
            real(5.0),
            real(7.0),
            Value::Null,
            real(8.5),
            real(8.5),
            real(10.0),
            real(12.0),
            real(12.0),
            real(12.0),
            real(14.0),
            real(15.0)
        ]
    );
    let ranked = rank.model().field("x_RANK").unwrap();
    assert_eq!(ranked.storage, Storage::Real);
    assert_eq!(ranked.measurement, Measurement::Continuous);

    let fractional = run(
        binning::build,
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "rank"),
            p("binByRank", "false"),
            p("binByFractionalRank", "true"),
            p("binByPcFractionalRank", "false"),
        ],
        column_table("x", Storage::Real, probe_x()),
    )
    .unwrap();
    let expected: Vec<Value> = [
        1.5,
        1.5,
        3.0,
        5.0,
        5.0,
        5.0,
        7.0,
        f64::NAN,
        8.5,
        8.5,
        10.0,
        12.0,
        12.0,
        12.0,
        14.0,
        15.0,
    ]
    .into_iter()
    .map(|rank| {
        if rank.is_nan() {
            Value::Null
        } else {
            real(rank / 15.0)
        }
    })
    .collect();
    assert_eq!(col(&fractional, "x_F_RANK"), expected);

    let reversed = run(
        binning::build,
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "rank"),
            p("binByRank", "true"),
            p("binByFractionalRank", "false"),
            p("binByPcFractionalRank", "false"),
            p("reverseRank", "true"),
        ],
        column_table(
            "x",
            Storage::Integer,
            vec![Value::Integer(1), Value::Integer(1), Value::Integer(5)],
        ),
    )
    .unwrap();
    // Ascending average ranks would be 1.5, 1.5, 3. Descending is n+1 minus that.
    assert_eq!(
        col(&reversed, "x_RANK"),
        vec![real(2.5), real(2.5), real(1.0)]
    );
}

#[test]
fn bin_sdev_uses_the_closed_central_interval() {
    let out = run(
        binning::build,
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "aroundMean"),
            p("binMeanStdDevCount", "1"),
        ],
        column_table("x", Storage::Real, probe_x()),
    )
    .unwrap();
    assert_eq!(
        ints(&col(&out, "x_SDBIN")),
        vec![
            Some(-1),
            Some(-1),
            Some(0),
            Some(0),
            Some(0),
            Some(0),
            Some(0),
            None,
            Some(0),
            Some(0),
            Some(0),
            Some(0),
            Some(0),
            Some(0),
            Some(1),
            Some(1)
        ]
    );

    // mean 0, sample sd 2, so -2 and 2 sit on the closed ends of bin 0.
    let boundary = run(
        binning::build,
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "aroundMean"),
            p("binMeanStdDevCount", "1"),
        ],
        column_table(
            "x",
            Storage::Integer,
            vec![Value::Integer(-2), Value::Integer(0), Value::Integer(2)],
        ),
    )
    .unwrap();
    assert_eq!(
        ints(&col(&boundary, "x_SDBIN")),
        vec![Some(0), Some(0), Some(0)]
    );
}

#[test]
fn bin_empty_input_and_all_nulls_keep_a_null_column() {
    let empty = run(
        binning::build,
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "fixedWidth"),
            p("fixedBinWidth", "5"),
        ],
        table(
            vec![field("x", Storage::Real, Measurement::Continuous)],
            Vec::new(),
        ),
    )
    .unwrap();
    assert_eq!(names(&empty), vec!["x", "x_BIN"]);
    assert_eq!(empty.row_count(), 0);

    let nulls = run(
        binning::build,
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "rank"),
            p("binByRank", "true"),
            p("binByFractionalRank", "false"),
            p("binByPcFractionalRank", "false"),
        ],
        column_table("x", Storage::Real, vec![Value::Null, Value::Null]),
    )
    .unwrap();
    assert_eq!(col(&nulls, "x_RANK"), vec![Value::Null, Value::Null]);
}

#[test]
fn bin_rejects_settings_and_non_numeric_fields() {
    let cases = [
        vec![p("binningMethod", "fixedWidth")],
        vec![bin_fields(&["x", "x"]), p("binningMethod", "fixedWidth")],
        vec![bin_fields(&["x"]), p("binningMethod", "optimal")],
        vec![bin_fields(&["x"]), p("binningMethod", "sideways")],
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "fixedWidth"),
            p("fixedBinWidth", "0"),
        ],
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "fixedWidth"),
            p("fixedBinMethod", "count"),
            p("fixedBinCount", "0"),
        ],
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "fixedWidth"),
            p("fixedBinSameBins", "true"),
        ],
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "equalFreq"),
            p("binTile4", "false"),
            p("binTile10", "false"),
        ],
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "equalFreq"),
            p("tilingMethod", "sum"),
        ],
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "equalFreq"),
            p("tiedValuesMethod", "random"),
        ],
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "equalFreq"),
            p("useBinTileN", "true"),
        ],
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "rank"),
            p("binByRank", "false"),
            p("binByFractionalRank", "false"),
            p("binByPcFractionalRank", "false"),
        ],
        vec![
            bin_fields(&["x"]),
            p("binningMethod", "aroundMean"),
            p("binMeanStdDevCount", "4"),
        ],
    ];
    for settings in cases {
        let error = run(
            binning::build,
            settings,
            column_table("x", Storage::Real, vec![real(1.0)]),
        );
        assert!(
            matches!(error, Err(OperatorError::Settings(_))),
            "{error:?}"
        );
    }

    let text = run(
        binning::build,
        vec![bin_fields(&["name"]), p("binningMethod", "fixedWidth")],
        column_table("name", Storage::String, vec![Value::string("a")]),
    );
    assert!(matches!(text, Err(OperatorError::Data(_))));
}

#[test]
fn reorder_needs_one_input() {
    let operator = field_reorder::build(&node(vec![p("autoOrderType", "auto")])).unwrap();
    assert_eq!(operator.arity(), dms_exec::Arity::Exactly(1));
    let error = operator.run(&[], &ExecContext::default()).unwrap_err();
    assert!(matches!(error, OperatorError::Inputs(_)));
}
