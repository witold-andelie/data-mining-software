//! Time Intervals rules on synthetic tables. Oracle fixtures remain outside this test file.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use dms_exec::nodes::time_intervals;
use dms_exec::{ExecContext, OperatorError};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
use dms_types::calendar::{MICROS_PER_DAY, MICROS_PER_SECOND, days_from_civil};
use dms_types::{DataModel, Field, Measurement, Storage, Table, Value, ValueSpec};
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

fn node(properties: Vec<Node>) -> StreamNode {
    let Node::Element(properties) = element("properties", properties) else {
        panic!("element helper")
    };
    StreamNode {
        uid: "ti".into(),
        class: "TimeIntervalsNode".into(),
        extension: None,
        model_builder: None,
        name: None,
        label: None,
        position: None,
        properties_attachment: None,
        containers: vec![],
        settings: Some(properties),
        binning_output: None,
        subdiagram: None,
    }
}

fn run(interval: &str, mut properties: Vec<Node>, input: Table) -> Result<Table, OperatorError> {
    properties.push(p("timeIntervalType", interval));
    time_intervals::build(&node(properties))?.run(&[Arc::new(input)], &ExecContext::default())
}

fn table(fields: Vec<Field>, rows: Vec<Vec<Value>>) -> Table {
    Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
}

fn integers(count: usize) -> Table {
    table(
        vec![Field::new("id", Storage::Integer)],
        (0..count).map(|i| vec![Value::Integer(i as i64)]).collect(),
    )
}

fn column(t: &Table, name: &str) -> Vec<Value> {
    t.column(name).unwrap().cloned().collect()
}
fn i(value: i64) -> Value {
    Value::Integer(value)
}
fn s(value: &str) -> Value {
    Value::string(value)
}
fn date(year: i64, month: u32, day: u32) -> Value {
    Value::Date(days_from_civil(year, month, day).unwrap())
}
fn tick(day: i64, second: i64) -> Value {
    Value::Timestamp(day * MICROS_PER_DAY + second * MICROS_PER_SECOND)
}

fn create(mut extras: Vec<Node>) -> Vec<Node> {
    let mut properties = vec![
        p("timeIntervalMode", "CREATE"),
        p("timeIntervalField", "tick"),
    ];
    properties.append(&mut extras);
    properties
}

fn numbers(rows: Vec<(Value, Value)>) -> Table {
    let mut x = Field::new("x", Storage::Real);
    x.measurement = Measurement::Continuous;
    table(
        vec![Field::new("tick", Storage::Timestamp), x],
        rows.into_iter().map(|(t, x)| vec![t, x]).collect(),
    )
}

fn rule(field: &str, aggregate: &str, padding: &str) -> Node {
    element(
        "row",
        vec![
            p("field", field),
            p("aggr", aggregate),
            p("padding", padding),
        ],
    )
}

fn rules(rows: Vec<Node>) -> Node {
    element("timeAggrAndPaddingFields", vec![element("rows", rows)])
}

#[test]
fn periods_default_and_custom_start_preserve_original_records() {
    for (properties, first) in [(vec![], 1), (vec![p("timeLabelPeriodStart", "7")], 7)] {
        let output = run("PERIODS", properties, integers(3)).unwrap();
        assert_eq!(column(&output, "id"), vec![i(0), i(1), i(2)]);
        assert_eq!(column(&output, "$TI_TimeIndex"), vec![i(1), i(2), i(3)]);
        assert_eq!(
            column(&output, "$TI_Period"),
            vec![i(first), i(first + 1), i(first + 2)]
        );
        assert_eq!(
            column(&output, "$TI_TimeLabel")[0],
            s(&format!("Period {first}"))
        );
    }
}

#[test]
fn cycles_wrap_from_partial_cycle() {
    let output = run(
        "C_PERIODS",
        vec![
            p("timeLabelCycleStart", "3"),
            p("timeLabelPeriodStart", "2"),
            p("timeLabelPeriodsPerCycle", "3"),
        ],
        integers(4),
    )
    .unwrap();
    assert_eq!(column(&output, "$TI_Cycle"), vec![i(3), i(3), i(4), i(4)]);
    assert_eq!(column(&output, "$TI_Period"), vec![i(2), i(3), i(1), i(2)]);
    assert_eq!(column(&output, "$TI_TimeLabel")[2], s("Cycle 4, Period 1"));
}

#[test]
fn years_and_fiscal_quarters_roll_over_and_use_relative_quarter_year() {
    let years = run("YEARS", vec![p("timeLabelYearStart", "1999")], integers(2)).unwrap();
    assert_eq!(column(&years, "$TI_TimeLabel"), vec![s("1999"), s("2000")]);
    let quarters = run(
        "QUARTERS",
        vec![
            p("timeLabelYearStart", "2019"),
            p("timeLabelQuarterStart", "4"),
            p("timeFiscalYearBegins", "April"),
            p("timeFieldNameExtension", "Q_"),
        ],
        integers(3),
    )
    .unwrap();
    assert_eq!(
        column(&quarters, "Q_TimeLabel"),
        vec![s("Q4 2019/2020"), s("Q1 2020/2021"), s("Q2 2020/2021")]
    );
    assert_eq!(column(&quarters, "Q_Year"), vec![i(1), i(2), i(2)]);
    assert_eq!(column(&quarters, "Q_Quarter"), vec![i(4), i(1), i(2)]);
}

#[test]
fn months_are_dates_with_suffix_and_correct_year_rollover() {
    let output = run(
        "MONTHS",
        vec![
            p("timeLabelYearStart", "2019"),
            p("timeLabelMonthStart", "December"),
            p("timeFieldNameExtension", "_m"),
            p("timeFieldNameExtensionAsPrefix", "false"),
            p("dateFormat", "format6"),
        ],
        integers(3),
    )
    .unwrap();
    assert_eq!(
        column(&output, "TimeLabel_m"),
        vec![date(2019, 12, 1), date(2020, 1, 1), date(2020, 2, 1)]
    );
    assert_eq!(column(&output, "Month_m"), vec![i(12), i(1), i(2)]);
    assert_eq!(
        output.model().field("TimeLabel_m").unwrap().storage,
        Storage::Date
    );
}

fn february() -> Vec<Node> {
    vec![
        p("timeLabelYearStart", "2020"),
        p("timeLabelMonthStart", "February"),
        p("timeLabelDayStart", "28"),
    ]
}

#[test]
fn daily_labels_cross_leap_day_without_reading_input_dates() {
    let output = run("DAYS_NON_PERIODIC", february(), integers(3)).unwrap();
    assert_eq!(
        column(&output, "$TI_TimeLabel"),
        vec![date(2020, 2, 28), date(2020, 2, 29), date(2020, 3, 1)]
    );
    assert_eq!(output.model().len(), 3);
}

#[test]
fn workdays_skip_inactive_days_and_count_relative_weeks() {
    let mut properties = february();
    properties.extend([
        p("timeNumDaysPerWeek", "4"),
        p("timeWeekBeginsOn", "Tuesday"),
    ]);
    let output = run("DAYS_PER_WEEK", properties, integers(3)).unwrap();
    assert_eq!(
        column(&output, "$TI_TimeLabel"),
        vec![date(2020, 2, 28), date(2020, 3, 3), date(2020, 3, 4)]
    );
    assert_eq!(column(&output, "$TI_Week"), vec![i(1), i(2), i(2)]);
    assert_eq!(column(&output, "$TI_Day"), vec![i(4), i(1), i(2)]);
}

#[test]
fn working_hours_roll_to_opening_after_weekend() {
    let mut properties = february();
    properties.extend([
        p("timeNumDaysPerWeek", "5"),
        p("timeNumHoursPerDay", "2"),
        p("timeDayBeginsAt", "28800"),
        p("timeLabelTimeStart", "32400"),
    ]);
    let output = run("HOURS_PER_DAY", properties, integers(3)).unwrap();
    let friday = days_from_civil(2020, 2, 28).unwrap();
    assert_eq!(
        column(&output, "$TI_TimeLabel"),
        vec![
            tick(friday, 32400),
            tick(friday + 3, 28800),
            tick(friday + 3, 32400)
        ]
    );
    assert_eq!(column(&output, "$TI_Day"), vec![i(5), i(1), i(1)]);
    assert_eq!(column(&output, "$TI_Hour"), vec![i(9), i(8), i(9)]);
}

#[test]
fn working_minutes_and_seconds_use_exact_business_start() {
    for (interval, start, opening, increment, second_start) in [
        ("MINUTES_PER_DAY", "31800", "29100", "15", 31800),
        ("SECONDS_PER_DAY", "32687", "29117", "30", 32687),
    ] {
        let mut properties = february();
        properties.extend([
            p("timeNumDaysPerWeek", "5"),
            p("timeNumHoursPerDay", "1"),
            p("timeDayBeginsAt", opening),
            p("timeLabelTimeStart", start),
            p("timeIncrementBy", increment),
        ]);
        let output = run(interval, properties, integers(3)).unwrap();
        let friday = days_from_civil(2020, 2, 28).unwrap();
        let opening: i64 = opening.parse().unwrap();
        let increment: i64 = increment.parse().unwrap();
        let step = if interval == "MINUTES_PER_DAY" {
            increment * 60
        } else {
            increment
        };
        assert_eq!(
            column(&output, "$TI_TimeLabel"),
            vec![
                tick(friday, second_start),
                tick(friday + 3, opening),
                tick(friday + 3, opening + step)
            ]
        );
    }
}

#[test]
fn create_daily_sorts_floors_aggregates_and_pads_leap_day() {
    let first = days_from_civil(2020, 2, 28).unwrap();
    let input = numbers(vec![
        (tick(first + 2, 0), Value::Real(9.0)),
        (tick(first, 50), Value::Real(2.0)),
        (tick(first, 80), Value::Real(5.0)),
        (Value::Null, Value::Real(100.0)),
    ]);
    let output = run("DAYS_NON_PERIODIC", create(vec![]), input).unwrap();
    assert_eq!(
        column(&output, "x"),
        vec![Value::Real(7.0), Value::Null, Value::Real(9.0)]
    );
    assert_eq!(
        column(&output, "$TI_TimeLabel"),
        vec![date(2020, 2, 28), date(2020, 2, 29), date(2020, 3, 1)]
    );
    assert_eq!(column(&output, "$TI_Count"), vec![i(2), i(0), i(1)]);
    assert!(output.model().field("tick").is_none());
    assert!(output.model().field("$TI_Future").is_none());
}

#[test]
fn all_keyed_aggregates_ignore_nulls_and_follow_duplicate_time_order() {
    let functions = ["0", "1", "2", "3", "4", "5", "6"];
    let names = ["mean", "sum", "mode", "min", "max", "first", "last"];
    let mut fields = vec![
        Field::new("tick", Storage::Timestamp),
        Field::new("drop", Storage::Integer),
    ];
    fields.extend(names.iter().map(|name| Field::new(*name, Storage::Real)));
    let mut rows = Vec::new();
    for (time, value) in [
        (4, Value::Real(2.0)),
        (4, Value::Real(8.0)),
        (5, Value::Null),
    ] {
        let mut row = vec![tick(0, time), i(7)];
        row.extend(names.iter().map(|_| value.clone()));
        rows.push(row);
    }
    let properties = create(vec![
        p("timeCreateFieldsAndFunctionsMode", "Specify"),
        rules(
            names
                .iter()
                .zip(functions)
                .rev()
                .map(|(name, function)| rule(name, function, "0"))
                .collect(),
        ),
    ]);
    let output = run("DAYS_NON_PERIODIC", properties, table(fields, rows)).unwrap();
    assert_eq!(
        output
            .model()
            .fields()
            .iter()
            .take(7)
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        names
    );
    for (name, expected) in names.iter().zip([5.0, 10.0, 2.0, 2.0, 8.0, 8.0, 2.0]) {
        assert_eq!(column(&output, name), vec![Value::Real(expected)], "{name}");
    }
    assert_eq!(column(&output, "$TI_Count"), vec![i(3)]);
    assert!(output.model().field("drop").is_none());
}

#[test]
fn mean_padding_uses_three_raw_nonnull_points_and_never_padded_values() {
    let input = numbers(vec![
        (tick(0, 58), Value::Real(2.0)),
        (tick(0, 58), Value::Real(8.0)),
        (tick(0, 59), Value::Real(8.0)),
        (tick(0, 62), Value::Real(20.0)),
        (tick(0, 65), Value::Null),
        (tick(0, 240), Value::Real(50.0)),
    ]);
    let output = run(
        "MINUTES_NON_PERIODIC",
        create(vec![
            p("timeAggrFunctionRangeDefault", "Min"),
            p("timePaddingFunctionRangeDefault", "MeanOfRecentPoints"),
        ]),
        input,
    )
    .unwrap();
    assert_eq!(
        column(&output, "x"),
        vec![
            Value::Real(2.0),
            Value::Real(20.0),
            Value::Real(10.0),
            Value::Real(10.0),
            Value::Real(50.0)
        ]
    );
    assert_eq!(
        column(&output, "$TI_Count"),
        vec![i(3), i(2), i(0), i(0), i(1)]
    );
}

#[test]
fn mean_padding_without_three_points_is_null() {
    let output = run(
        "SECONDS_NON_PERIODIC",
        create(vec![p(
            "timePaddingFunctionRangeDefault",
            "MeanOfRecentPoints",
        )]),
        numbers(vec![
            (tick(0, 0), Value::Real(5.0)),
            (tick(0, 2), Value::Real(9.0)),
        ]),
    )
    .unwrap();
    assert_eq!(column(&output, "x")[1], Value::Null);
}

fn flag_input() -> Table {
    let mut flag = Field::new("f", Storage::String);
    flag.measurement = Measurement::Flag;
    flag.values = ValueSpec::Flag {
        true_value: s("yes"),
        false_value: s("no"),
    };
    let mut cat = Field::new("cat", Storage::String);
    cat.measurement = Measurement::Nominal;
    table(
        vec![Field::new("tick", Storage::Timestamp), flag, cat],
        vec![
            vec![tick(0, 0), s("no"), s("A")],
            vec![tick(0, 0), s("yes"), s("B")],
            vec![tick(0, 2), Value::Null, s("")],
        ],
    )
}

#[test]
fn flag_values_come_from_metadata_and_empty_strings_are_values() {
    let output = run(
        "SECONDS_NON_PERIODIC",
        create(vec![p("timePaddingFunctionFlagDefault", "True")]),
        flag_input(),
    )
    .unwrap();
    assert_eq!(column(&output, "f"), vec![s("yes"), s("yes"), Value::Null]);
    assert_eq!(column(&output, "cat"), vec![s("A"), Value::Null, s("")]);
    assert_eq!(column(&output, "$TI_Count"), vec![i(2), i(0), i(1)]);
}

#[test]
fn categorical_and_flag_first_last_mode_use_timestamp_order() {
    for (function, expected) in [("First", "B"), ("Last", "A"), ("Mode", "A")] {
        let output = run(
            "SECONDS_NON_PERIODIC",
            create(vec![
                p("timeAggrFunctionSetDefault", function),
                p("timeAggrFunctionFlagDefault", function),
                p("timePaddingFunctionFlagDefault", "False"),
            ]),
            flag_input(),
        )
        .unwrap();
        assert_eq!(column(&output, "cat")[0], s(expected));
        assert_eq!(
            column(&output, "f")[0],
            s(if expected == "B" { "yes" } else { "no" })
        );
        assert_eq!(column(&output, "f")[1], s("no"));
    }
}

#[test]
fn future_rows_are_null_and_indicator_is_conditional() {
    for make_flag in [false, true] {
        let output = run(
            "SECONDS_NON_PERIODIC",
            create(vec![
                p("timeIntervalsCreateFutureRecords", "true"),
                p("timeIntervalsNumFutureRecords", "2"),
                p(
                    "timeIntervalsCreateFutureField",
                    if make_flag { "true" } else { "false" },
                ),
                p("timeIntervalsFutureFieldName", "future"),
                p("timePaddingFunctionFlagDefault", "True"),
            ]),
            flag_input(),
        )
        .unwrap();
        assert_eq!(output.row_count(), 5);
        assert_eq!(&column(&output, "f")[3..], &[Value::Null, Value::Null]);
        assert_eq!(&column(&output, "$TI_Count")[3..], &[i(0), i(0)]);
        if make_flag {
            assert_eq!(
                column(&output, "future"),
                vec![i(0), i(0), i(0), i(1), i(1)]
            );
        } else {
            assert!(output.model().field("future").is_none());
        }
    }
}

#[test]
fn estimation_settings_do_not_remove_records() {
    let output = run(
        "HOURS_NON_PERIODIC",
        create(vec![
            p("timeSeriesBeginEstimationMode", "false"),
            p("timeSeriesEndEstimationMode", "false"),
            p("timeSeriesEstimationPeriodStartOffset", "1"),
            p("timeSeriesEstimationPeriodNumHoldouts", "1"),
        ]),
        numbers(vec![
            (tick(0, 3599), Value::Real(1.0)),
            (tick(0, 7200), Value::Real(3.0)),
        ]),
    )
    .unwrap();
    assert_eq!(
        column(&output, "$TI_TimeLabel"),
        vec![s("0:00"), s("1:00"), s("2:00")]
    );
    assert_eq!(column(&output, "$TI_Hour"), vec![i(0), i(1), i(2)]);
}

#[test]
fn cap_at_exact_result_size_is_a_controlled_error() {
    let error = run(
        "SECONDS_NON_PERIODIC",
        create(vec![p("timePaddingMaxRecordsCreated", "8")]),
        numbers(vec![
            (tick(0, 0), Value::Real(1.0)),
            (tick(0, 7), Value::Real(2.0)),
        ]),
    )
    .unwrap_err();
    assert_eq!(
        error,
        OperatorError::Data("The maximum number of records to create has been exceeded".into())
    );
}

#[test]
fn empty_and_all_missing_sources_have_the_output_schema_without_rows() {
    for rows in [vec![], vec![(Value::Null, Value::Real(4.0))]] {
        let output = run("DAYS_NON_PERIODIC", create(vec![]), numbers(rows)).unwrap();
        assert_eq!(output.row_count(), 0);
        assert_eq!(output.model().len(), 4);
    }
}

#[test]
fn rejects_unknown_and_unsettled_settings_explicitly() {
    let cases = [
        ("NONE", vec![]),
        ("WEEKS", vec![]),
        ("PERIODS", vec![p("newFeature", "true")]),
        ("PERIODS", vec![p("timeLabelPeriodStart", "0")]),
        (
            "PERIODS",
            vec![
                p("timeLabelPeriodStart", "1"),
                p("timeLabelPeriodStart", "2"),
            ],
        ),
        (
            "PERIODS",
            vec![element("timeLabelPeriodStart", vec![p("nested", "1")])],
        ),
        (
            "PERIODS",
            vec![p("timeIntervalsNumFutureRecords", "9223372036854775807")],
        ),
        ("PERIODS", vec![p("timePaddingMaxRecordsCreated", "-1")]),
        ("PERIODS", vec![p("timeSeriesBeginEstimationMode", "yes")]),
        (
            "PERIODS",
            vec![p("timeIntervalsCreateFutureRecords", "true")],
        ),
        ("MONTHS", create(vec![])),
        ("DAYS_PER_WEEK", create(vec![])),
        ("DAYS_NON_PERIODIC", vec![p("dateFormat", "format6")]),
        (
            "PERIODS",
            vec![
                p("timeSeriesBeginEstimationMode", "false"),
                p("timeSeriesEstimationPeriodStartOffset", "2"),
            ],
        ),
        (
            "PERIODS",
            vec![element(
                "timeIntervalsFutureValuesFields",
                vec![element("rows", vec![element("row", vec![])])],
            )],
        ),
        (
            "PERIODS",
            vec![element(
                "timeIntervalsCustomFutureValues",
                vec![element("future_values_lists", vec![p("value", "3")])],
            )],
        ),
        (
            "DAYS_NON_PERIODIC",
            create(vec![p("timeCreateFieldsAndFunctionsMode", "Specify")]),
        ),
        ("PERIODS", vec![p("timeIncrementBy", "0")]),
    ];
    for (interval, settings) in cases {
        assert!(
            matches!(
                run(interval, settings, integers(1)),
                Err(OperatorError::Settings(_))
            ),
            "{interval}"
        );
    }
}

#[test]
fn rejects_invalid_business_start_and_overnight_schedule() {
    let invalid_day = vec![
        p("timeLabelYearStart", "2020"),
        p("timeLabelMonthStart", "February"),
        p("timeLabelDayStart", "29"),
        p("timeNumDaysPerWeek", "5"),
    ];
    assert!(matches!(
        run("DAYS_PER_WEEK", invalid_day, integers(1)),
        Err(OperatorError::Settings(_))
    ));
    for properties in [
        vec![p("timeNumHoursPerDay", "2"), p("timeDayBeginsAt", "82800")],
        vec![p("timeDayBeginsAt", "28800"), p("timeLabelTimeStart", "0")],
    ] {
        assert!(matches!(
            run("HOURS_PER_DAY", properties, integers(1)),
            Err(OperatorError::Settings(_))
        ));
    }
}

#[test]
fn rejects_unknown_nested_functions_duplicates_and_unproved_recent_padding() {
    for extra in [
        rules(vec![rule("x", "99", "0")]),
        rules(vec![rule("x", "1", "99")]),
        rules(vec![rule("x", "1", "0"), rule("x", "1", "0")]),
        rules(vec![element(
            "row",
            vec![
                p("field", "x"),
                p("aggr", "1"),
                p("padding", "0"),
                p("hidden", "1"),
            ],
        )]),
        rules(vec![rule("x", "1", "2")]),
        rules(vec![rule("absent", "1", "0")]),
        rules(vec![rule("tick", "1", "0")]),
    ] {
        assert!(matches!(
            run(
                "DAYS_NON_PERIODIC",
                create(vec![
                    p("timeCreateFieldsAndFunctionsMode", "Specify"),
                    extra
                ]),
                numbers(vec![(tick(0, 0), Value::Real(1.0))])
            ),
            Err(OperatorError::Settings(_))
        ));
    }
}

#[test]
fn bad_data_collisions_and_extreme_numbers_return_errors() {
    let huge = run(
        "SECONDS_NON_PERIODIC",
        create(vec![]),
        numbers(vec![(Value::Date(i64::MAX), Value::Real(1.0))]),
    );
    assert!(matches!(huge, Err(OperatorError::Data(_))));
    let span = run(
        "SECONDS_NON_PERIODIC",
        create(vec![]),
        numbers(vec![
            (tick(-1_000_000, 0), Value::Real(1.0)),
            (tick(1_000_000, 0), Value::Real(1.0)),
        ]),
    );
    assert!(matches!(span, Err(OperatorError::Data(_))));
    assert!(matches!(
        run(
            "PERIODS",
            vec![p("timeLabelPeriodStart", &i64::MAX.to_string())],
            integers(2)
        ),
        Err(OperatorError::Data(_))
    ));
    let collision = table(
        vec![Field::new("$TI_TimeLabel", Storage::Integer)],
        vec![vec![i(1)]],
    );
    assert!(matches!(
        run("PERIODS", vec![], collision),
        Err(OperatorError::Data(_))
    ));
    assert!(matches!(
        run(
            "DAYS_NON_PERIODIC",
            create(vec![]),
            numbers(vec![(tick(0, 0), Value::Real(f64::INFINITY))])
        ),
        Err(OperatorError::Data(_))
    ));
}

#[test]
fn negative_epoch_flooring_and_source_storage_are_checked() {
    let output = run(
        "MINUTES_NON_PERIODIC",
        create(vec![]),
        numbers(vec![
            (Value::Timestamp(-1), Value::Real(1.0)),
            (Value::Timestamp(0), Value::Real(2.0)),
        ]),
    )
    .unwrap();
    assert_eq!(
        column(&output, "$TI_TimeLabel"),
        vec![s("23:59"), s("0:00")]
    );
    assert_eq!(column(&output, "$TI_Minute"), vec![i(59), i(0)]);
    assert!(matches!(
        run(
            "DAYS_NON_PERIODIC",
            create(vec![]),
            numbers(vec![(s("bad"), Value::Real(1.0))])
        ),
        Err(OperatorError::Data(_))
    ));
}

#[test]
fn missing_input_and_undeclared_flag_values_are_handled() {
    let operator = time_intervals::build(&node(vec![p("timeIntervalType", "PERIODS")])).unwrap();
    assert!(matches!(
        operator.run(&[], &ExecContext::default()),
        Err(OperatorError::Inputs(_))
    ));
    let mut flag = Field::new("f", Storage::Integer);
    flag.measurement = Measurement::Flag;
    let input = table(
        vec![Field::new("tick", Storage::Timestamp), flag],
        vec![vec![tick(0, 0), i(1)]],
    );
    assert!(matches!(
        run("SECONDS_NON_PERIODIC", create(vec![]), input),
        Err(OperatorError::Settings(_))
    ));
}

#[test]
fn integer_sum_overflow_is_reported_without_wrapping() {
    let mut x = Field::new("x", Storage::Integer);
    x.measurement = Measurement::Continuous;
    let input = table(
        vec![Field::new("tick", Storage::Timestamp), x],
        vec![vec![tick(0, 0), i(i64::MAX)], vec![tick(0, 1), i(1)]],
    );
    assert!(matches!(
        run("DAYS_NON_PERIODIC", create(vec![]), input),
        Err(OperatorError::Data(_))
    ));
}

#[test]
fn date_and_time_sources_use_the_runtime_value_types() {
    let dates = run(
        "DAYS_NON_PERIODIC",
        create(vec![]),
        numbers(vec![
            (Value::Date(0), Value::Real(2.0)),
            (Value::Date(2), Value::Real(3.0)),
        ]),
    )
    .unwrap();
    assert_eq!(
        column(&dates, "$TI_TimeLabel"),
        vec![Value::Date(0), Value::Date(1), Value::Date(2)]
    );
    let times = run(
        "SECONDS_NON_PERIODIC",
        create(vec![]),
        numbers(vec![
            (Value::Time(0), Value::Real(2.0)),
            (Value::Time(2 * MICROS_PER_SECOND), Value::Real(3.0)),
        ]),
    )
    .unwrap();
    assert_eq!(
        column(&times, "$TI_TimeLabel"),
        vec![s("0:00:00"), s("0:00:01"), s("0:00:02")]
    );
    assert!(matches!(
        run(
            "SECONDS_NON_PERIODIC",
            create(vec![]),
            numbers(vec![(Value::Time(MICROS_PER_DAY), Value::Real(1.0))])
        ),
        Err(OperatorError::Data(_))
    ));
}
