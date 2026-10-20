//! Auto Data Prep on synthetic tables. Oracle case names in the node module docs establish the
//! rules; these fixtures use independent values.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_exec::nodes::auto_data_prep;
use dms_exec::{ExecContext, OperatorError};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
use dms_types::calendar::days_from_civil;
use dms_types::{DataModel, Field, Measurement, Role, Storage, Table, Value};
use std::sync::Arc;

fn element(name: &str, children: Vec<Node>) -> Node {
    Node::Element(Element {
        name: QualifiedName {
            prefix: None,
            namespace: None,
            local: Arc::from(name),
        },
        namespace_declarations: vec![],
        attributes: vec![],
        children,
    })
}

fn p(name: &str, value: &str) -> Node {
    element(name, vec![Node::Text(Arc::from(value))])
}

fn node(settings: Vec<Node>) -> StreamNode {
    let Node::Element(settings) = element("properties", settings) else {
        panic!("element")
    };
    StreamNode {
        uid: "adp".into(),
        class: "test".into(),
        extension: None,
        model_builder: None,
        name: None,
        label: None,
        position: None,
        properties_attachment: None,
        containers: vec![],
        settings: Some(settings),
        binning_output: None,
        result_items: Vec::new(),
        subdiagram: None,
    }
}

fn field(name: &str, storage: Storage, measurement: Measurement, role: Role) -> Field {
    let mut field = Field::new(name, storage);
    field.measurement = measurement;
    field.role = role;
    field
}

fn table(fields: Vec<Field>, rows: Vec<Vec<Value>>) -> Table {
    Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
}

fn run(settings: Vec<Node>, input: Table) -> Result<Table, OperatorError> {
    auto_data_prep::build(&node(settings))?.run(&[Arc::new(input)], &ExecContext::default())
}

fn names(table: &Table) -> Vec<String> {
    table
        .model()
        .fields()
        .iter()
        .map(|field| field.name.clone())
        .collect()
}

fn col(table: &Table, name: &str) -> Vec<Value> {
    table.column(name).unwrap().cloned().collect()
}

fn settings_error(result: Result<Table, OperatorError>, needle: &str) {
    match result {
        Err(OperatorError::Settings(message)) => assert!(
            message.contains(needle),
            "message {message:?} does not name {needle}"
        ),
        other => panic!("expected a settings error naming {needle}, got {other:?}"),
    }
}

fn real_close(actual: &[Value], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (got, want) in actual.iter().zip(expected) {
        match got {
            Value::Real(number) => assert!((number - want).abs() < 1e-12, "{number} != {want}"),
            other => panic!("expected real {want}, got {other:?}"),
        }
    }
}

fn date(year: i64, month: u32, day: u32) -> Value {
    Value::Date(days_from_civil(year, month, day).unwrap())
}

/// `1, 2, null` with imputation: mean 1.5, ss 0.5, divisor n-1 = 2, sd 0.5 → -1, 1, 0.
#[test]
fn z_score_imputes_missing_with_divisor_n_minus_one() {
    let input = table(
        vec![field(
            "x",
            Storage::Real,
            Measurement::Continuous,
            Role::Input,
        )],
        vec![
            vec![Value::Real(1.0)],
            vec![Value::Real(2.0)],
            vec![Value::Null],
        ],
    );
    let output = run(vec![p("scaleToOrdinalValuesNumThreshold", "0")], input).unwrap();
    assert_eq!(names(&output), vec!["x_transformed".to_string()]);
    assert_eq!(
        output.model().field("x_transformed").unwrap().storage,
        Storage::Real
    );
    assert_eq!(
        output.model().field("x_transformed").unwrap().measurement,
        Measurement::Continuous
    );
    real_close(&col(&output, "x_transformed"), &[-1.0, 1.0, 0.0]);
}

#[test]
fn z_score_honors_final_mean_and_sd() {
    let input = table(
        vec![field(
            "x",
            Storage::Integer,
            Measurement::Continuous,
            Role::Input,
        )],
        vec![
            vec![Value::Integer(1)],
            vec![Value::Integer(2)],
            vec![Value::Null],
        ],
    );
    let output = run(
        vec![
            p("scaleToOrdinalValuesNumThreshold", "0"),
            p("zScore_mean", "10"),
            p("zScore_SD", "2"),
        ],
        input,
    )
    .unwrap();
    real_close(&col(&output, "x_transformed"), &[8.0, 12.0, 10.0]);
}

/// Without imputation the divisor is the valid count minus one, and the null stays null.
#[test]
fn z_score_without_imputation_uses_valid_count() {
    let input = table(
        vec![field(
            "x",
            Storage::Real,
            Measurement::Continuous,
            Role::Input,
        )],
        vec![
            vec![Value::Real(1.0)],
            vec![Value::Real(2.0)],
            vec![Value::Null],
        ],
    );
    let output = run(
        vec![
            p("scaleToOrdinalValuesNumThreshold", "0"),
            p("uiReplaceScaleFeaturesMissingValue", "false"),
        ],
        input,
    )
    .unwrap();
    let sd = 0.5_f64.sqrt();
    real_close(&col(&output, "x_transformed")[..2], &[-0.5 / sd, 0.5 / sd]);
    assert_eq!(col(&output, "x_transformed")[2], Value::Null);
}

#[test]
fn zero_standard_deviation_writes_final_mean_without_nan() {
    let input = table(
        vec![field(
            "x",
            Storage::Real,
            Measurement::Continuous,
            Role::Input,
        )],
        vec![vec![Value::Real(5.0)], vec![Value::Null]],
    );
    let output = run(
        vec![
            p("excludePoorQualityFeatures", "false"),
            p("scaleToOrdinalValuesNumThreshold", "0"),
            p("zScore_mean", "4"),
        ],
        input,
    )
    .unwrap();
    real_close(&col(&output, "x_transformed"), &[4.0, 4.0]);
}

/// Fewer than 5 distinct values stay ordinal and are not rescaled (`scaleToOrdinalValuesNumThreshold`).
#[test]
fn few_distinct_continuous_values_stay_ordinal() {
    let input = table(
        vec![field(
            "x",
            Storage::Integer,
            Measurement::Continuous,
            Role::Input,
        )],
        vec![
            vec![Value::Integer(1)],
            vec![Value::Integer(2)],
            vec![Value::Integer(3)],
            vec![Value::Integer(1)],
        ],
    );
    let output = run(vec![], input).unwrap();
    assert_eq!(names(&output), vec!["x".to_string()]);
    assert_eq!(
        output.model().field("x").unwrap().measurement,
        Measurement::Ordinal
    );
    assert_eq!(
        col(&output, "x"),
        vec![
            Value::Integer(1),
            Value::Integer(2),
            Value::Integer(3),
            Value::Integer(1)
        ]
    );
}

/// Guide reorder example: A,A,A,B,C,C → B=0, C=1, A=2, stored as strings.
#[test]
fn categories_are_frequency_codes_with_null_and_ties() {
    let input = table(
        vec![field(
            "g",
            Storage::String,
            Measurement::Nominal,
            Role::Input,
        )],
        ["A", "A", "A", "B", "C", "C"]
            .into_iter()
            .map(|value| vec![Value::string(value)])
            .collect(),
    );
    let output = run(vec![], input).unwrap();
    assert_eq!(
        col(&output, "g_transformed"),
        ["2", "2", "2", "0", "1", "1"]
            .into_iter()
            .map(Value::string)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        output.model().field("g_transformed").unwrap().storage,
        Storage::String
    );
    assert_eq!(
        output.model().field("g_transformed").unwrap().measurement,
        Measurement::Nominal
    );

    let tied = table(
        vec![field(
            "g",
            Storage::String,
            Measurement::Nominal,
            Role::Input,
        )],
        ["B", "A", "B", "A"]
            .into_iter()
            .map(|value| vec![Value::string(value)])
            .collect(),
    );
    let tied = run(vec![], tied).unwrap();
    assert_eq!(
        col(&tied, "g_transformed"),
        ["1", "0", "1", "0"]
            .into_iter()
            .map(Value::string)
            .collect::<Vec<_>>()
    );

    // Null is its own category when another value exists (`adp_balanced` flg). One non-missing
    // category is a constant and is screened out, so this fixture keeps B as well.
    let missing = table(
        vec![field("g", Storage::String, Measurement::Flag, Role::Input)],
        vec![
            vec![Value::Null],
            vec![Value::string("A")],
            vec![Value::string("B")],
            vec![Value::string("A")],
        ],
    );
    let missing = run(vec![], missing).unwrap();
    assert_eq!(
        col(&missing, "g_transformed"),
        ["0", "2", "1", "2"]
            .into_iter()
            .map(Value::string)
            .collect::<Vec<_>>()
    );
}

#[test]
fn trim_blanks_merges_category_identity_and_leaves_passthrough_text() {
    let input = table(
        vec![
            field("note", Storage::String, Measurement::Typeless, Role::None),
            field("g", Storage::String, Measurement::Nominal, Role::Input),
        ],
        vec![
            vec![Value::string("  "), Value::string("  ")],
            vec![Value::string("  "), Value::string("")],
            vec![Value::string("  "), Value::string("A")],
            vec![Value::string("  "), Value::string("A")],
        ],
    );
    let output = run(vec![], input).unwrap();
    assert_eq!(col(&output, "note"), vec![Value::string("  "); 4]);
    assert_eq!(
        col(&output, "g_transformed"),
        ["0", "0", "1", "1"]
            .into_iter()
            .map(Value::string)
            .collect::<Vec<_>>()
    );
}

#[test]
fn field_order_groups_and_continuous_missing_count() {
    let input = table(
        vec![
            field("id", Storage::Integer, Measurement::Continuous, Role::None),
            field("iv", Storage::Real, Measurement::Continuous, Role::Input),
            field("pos", Storage::Real, Measurement::Continuous, Role::Input),
            field("txt", Storage::String, Measurement::Typeless, Role::None),
            field("g", Storage::String, Measurement::Nominal, Role::Input),
            field("dt", Storage::Date, Measurement::Continuous, Role::Input),
            field(
                "ts",
                Storage::Timestamp,
                Measurement::Continuous,
                Role::Input,
            ),
            field("y", Storage::String, Measurement::Nominal, Role::Target),
        ],
        (0..6)
            .map(|row| {
                vec![
                    Value::Integer(row),
                    if row == 0 {
                        Value::Null
                    } else {
                        Value::Real(row as f64)
                    },
                    Value::Real((row + 1) as f64),
                    Value::string("keep"),
                    Value::string(if row < 3 { "A" } else { "B" }),
                    date(2020, 1, (row + 1) as u32),
                    Value::Timestamp(1_000_000 * (row + 1)),
                    Value::string(if row % 2 == 0 { "A" } else { "B" }),
                ]
            })
            .collect(),
    );
    let output = run(vec![], input).unwrap();
    assert_eq!(
        names(&output),
        [
            "id",
            "txt",
            "ts",
            "y",
            "pos_transformed",
            "iv_transformed",
            "dt_months",
            "g_transformed"
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>()
    );
    assert_eq!(col(&output, "y")[0], Value::string("A"));
    assert_eq!(col(&output, "ts")[1], Value::Timestamp(2_000_000));
    assert!(output.model().field("iv").is_none());
    assert!(output.model().field("dt").is_none());
}

#[test]
fn date_duration_months_match_days_and_year_stays_ordinal() {
    let rows: Vec<Vec<Value>> = (1..=11)
        .map(|month| vec![date(2020, month, 1)])
        .chain(std::iter::once(vec![Value::Null]))
        .collect();
    let input = || {
        table(
            vec![field(
                "dt",
                Storage::Date,
                Measurement::Continuous,
                Role::Input,
            )],
            rows.clone(),
        )
    };
    let months = run(vec![p("uiExtractYearFromDate", "false")], input()).unwrap();
    let days = run(vec![p("dateUnit", "day")], input()).unwrap();
    real_close(
        &col(&months, "dt_months"),
        &col(&days, "dt_days")
            .iter()
            .map(|value| match value {
                Value::Real(number) => *number,
                other => panic!("days {other:?}"),
            })
            .collect::<Vec<_>>(),
    );

    let extracted = run(
        vec![
            p("dateUnit", "day"),
            p("uiExtractYearFromDate", "true"),
            p("uiExtractMonthFromDate", "true"),
            p("uiExtractDayFromDate", "true"),
        ],
        input(),
    )
    .unwrap();
    // 11 months are recast (threshold 10); the single day-of-month and the single year stay ordinal.
    assert_eq!(
        names(&extracted),
        ["dt_year", "dt_day", "dt_days", "dt_month"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        extracted.model().field("dt_year").unwrap().storage,
        Storage::Integer
    );
    assert_eq!(
        extracted.model().field("dt_year").unwrap().measurement,
        Measurement::Ordinal
    );
    assert_eq!(col(&extracted, "dt_year")[0], Value::Integer(2020));
    assert_eq!(col(&extracted, "dt_year")[11], Value::Null);
    assert_eq!(col(&extracted, "dt_day")[11], Value::Null);
    assert_eq!(col(&extracted, "dt_days")[11], Value::Real(0.0));
    assert_eq!(col(&extracted, "dt_month")[11], Value::Real(0.0));
    assert_eq!(
        extracted.model().field("dt_month").unwrap().storage,
        Storage::Real
    );
}

#[test]
fn ten_distinct_months_stay_ordinal() {
    let input = table(
        vec![field(
            "dt",
            Storage::Date,
            Measurement::Continuous,
            Role::Input,
        )],
        (1..=10).map(|month| vec![date(2020, month, 1)]).collect(),
    );
    let output = run(
        vec![
            p("uiComputeDateDuration", "false"),
            p("uiExtractMonthFromDate", "true"),
            p("uiExtractYearFromDate", "false"),
        ],
        input,
    )
    .unwrap();
    assert_eq!(names(&output), vec!["dt_month".to_string()]);
    assert_eq!(col(&output, "dt_month")[0], Value::Integer(1));
    assert_eq!(col(&output, "dt_month")[9], Value::Integer(10));
    assert_eq!(
        output.model().field("dt_month").unwrap().storage,
        Storage::Integer
    );
}

#[test]
fn screening_drops_missing_constant_and_too_many_categories() {
    let mostly_missing = table(
        vec![field(
            "g",
            Storage::String,
            Measurement::Nominal,
            Role::Input,
        )],
        vec![
            vec![Value::Null],
            vec![Value::Null],
            vec![Value::Null],
            vec![Value::Null],
            vec![Value::string("A")],
            vec![Value::string("B")],
        ],
    );
    let dropped = run(vec![], mostly_missing).unwrap();
    assert!(names(&dropped).is_empty());

    let half_missing = table(
        vec![field(
            "g",
            Storage::String,
            Measurement::Nominal,
            Role::Input,
        )],
        vec![
            vec![Value::Null],
            vec![Value::Null],
            vec![Value::Null],
            vec![Value::string("A")],
            vec![Value::string("B")],
            vec![Value::string("C")],
        ],
    );
    let kept = run(vec![], half_missing).unwrap();
    assert_eq!(names(&kept), vec!["g_transformed".to_string()]);

    let constant = table(
        vec![field(
            "x",
            Storage::Real,
            Measurement::Continuous,
            Role::Input,
        )],
        vec![vec![Value::Real(3.0)], vec![Value::Real(3.0)]],
    );
    let constant = run(vec![], constant).unwrap();
    assert!(names(&constant).is_empty());

    let three = table(
        vec![field(
            "g",
            Storage::String,
            Measurement::Nominal,
            Role::Input,
        )],
        ["A", "B", "C"]
            .into_iter()
            .map(|value| vec![Value::string(value)])
            .collect(),
    );
    let too_many = run(vec![p("maxNOfUniqueCategories", "2")], three).unwrap();
    assert!(names(&too_many).is_empty());
    let two = table(
        vec![field(
            "g",
            Storage::String,
            Measurement::Nominal,
            Role::Input,
        )],
        ["A", "B", "A"]
            .into_iter()
            .map(|value| vec![Value::string(value)])
            .collect(),
    );
    let at_limit = run(vec![p("maxNOfUniqueCategories", "2")], two).unwrap();
    assert_eq!(names(&at_limit), vec!["g_transformed".to_string()]);
}

#[test]
fn unsupported_settings_name_the_property() {
    let input = table(
        vec![field(
            "x",
            Storage::Real,
            Measurement::Continuous,
            Role::Input,
        )],
        vec![vec![Value::Real(1.0)]],
    );
    let cases = [
        (vec![p("notARealSetting", "1")], "notARealSetting"),
        (
            vec![p("scaleContinuousMode", "minMax")],
            "scaleContinuousMode",
        ),
        (
            vec![p("uiDetectTargetOutliers", "true")],
            "uiDetectTargetOutliers",
        ),
        (vec![p("useCustomSettings", "true")], "useCustomSettings"),
        (vec![p("useFrequency", "true")], "useFrequency"),
        (vec![p("targetField", "y")], "targetField"),
        (vec![p("analysisObjective", "nope")], "analysisObjective"),
        (vec![p("excludeNoUsedField", "keep")], "excludeNoUsedField"),
        (vec![p("dateUnit", "year")], "dateUnit"),
        (
            vec![p("uiReplaceOrdinalFeaturesMissingValue", "true")],
            "uiReplaceOrdinalFeaturesMissingValue",
        ),
        (
            vec![p("uiReplaceNominalFeaturesMissingValue", "false")],
            "uiReplaceNominalFeaturesMissingValue",
        ),
        (
            vec![p("uiFeatureConstruction", "true")],
            "uiFeatureConstruction",
        ),
        (
            vec![p("uiRescaleScaleTargetUsingBoxCox", "true")],
            "uiRescaleScaleTargetUsingBoxCox",
        ),
        (
            vec![element("inputFields", vec![p("field", "x")])],
            "inputFields",
        ),
    ];
    for (settings, needle) in cases {
        settings_error(run(settings, input.clone()), needle);
    }
}

#[test]
fn time_field_with_duration_flag_is_rejected_and_timestamp_passes() {
    let time = table(
        vec![field(
            "t",
            Storage::Time,
            Measurement::Continuous,
            Role::Input,
        )],
        vec![vec![Value::Time(0)]],
    );
    settings_error(run(vec![], time), "uiComputeTimeDuration");
    let time_off = table(
        vec![field(
            "t",
            Storage::Time,
            Measurement::Continuous,
            Role::Input,
        )],
        vec![vec![Value::Time(5)]],
    );
    let output = run(vec![p("uiComputeTimeDuration", "false")], time_off).unwrap();
    assert_eq!(col(&output, "t"), vec![Value::Time(5)]);
}

#[test]
fn duplicate_derived_name_is_a_data_error() {
    let input = table(
        vec![
            field(
                "x_transformed",
                Storage::String,
                Measurement::Typeless,
                Role::None,
            ),
            field("x", Storage::Real, Measurement::Continuous, Role::Input),
        ],
        (1..=5)
            .map(|value| vec![Value::string("keep"), Value::Real(value as f64)])
            .collect(),
    );
    let error = run(vec![], input).unwrap_err();
    assert!(
        matches!(error, OperatorError::Data(ref message) if message.contains("x_transformed")),
        "{error:?}"
    );
}

#[test]
fn empty_input_is_copied_and_single_row_does_not_panic() {
    let fields = vec![field(
        "x",
        Storage::Real,
        Measurement::Continuous,
        Role::Input,
    )];
    let empty = run(vec![], table(fields.clone(), vec![])).unwrap();
    assert_eq!(empty.row_count(), 0);
    assert_eq!(names(&empty), vec!["x".to_string()]);

    let one = run(vec![], table(fields, vec![vec![Value::Real(1.0)]])).unwrap();
    assert_eq!(one.row_count(), 1);
    for row in one.rows() {
        for value in row {
            if let Value::Real(number) = value {
                assert!(number.is_finite());
            }
        }
    }
}

#[test]
fn reorder_and_scale_flags_off_keep_original_values() {
    let input = table(
        vec![
            field("x", Storage::Real, Measurement::Continuous, Role::Input),
            field("g", Storage::String, Measurement::Nominal, Role::Input),
        ],
        (1..=5)
            .map(|value| vec![Value::Real(value as f64), Value::string("A")])
            .collect(),
    );
    let output = run(
        vec![
            p("uiScaleContinuousFeatures", "false"),
            p("uiReorderNominalCatsPriorToMerging", "false"),
            p("excludePoorQualityFeatures", "false"),
        ],
        input,
    )
    .unwrap();
    assert_eq!(names(&output), vec!["x".to_string(), "g".to_string()]);
    assert_eq!(col(&output, "g")[0], Value::string("A"));
    assert_eq!(col(&output, "x")[4], Value::Real(5.0));
}
