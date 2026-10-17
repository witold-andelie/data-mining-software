//! RFM Aggregate on small synthetic tables. Oracle case names in module documentation establish
//! the rules; these fixtures deliberately use independent values and dates.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_exec::nodes::rfm_aggregate;
use dms_exec::{ExecContext, Factory, OperatorError};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
use dms_types::calendar::{MICROS_PER_DAY, days_from_civil};
use dms_types::{DataModel, Field, Storage, Table, Value};
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
        uid: "rfm".into(),
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
        subdiagram: None,
    }
}

fn table(fields: &[(&str, Storage)], rows: Vec<Vec<Value>>) -> Table {
    Table::new(
        DataModel::new(fields.iter().map(|(n, s)| Field::new(*n, *s)).collect()).unwrap(),
        rows,
    )
    .unwrap()
}

fn context() -> ExecContext {
    let Node::Element(properties) = element("properties", vec![p("timeZone", "UTC")]) else {
        panic!("element")
    };
    ExecContext {
        stream_properties: Some(Arc::new(properties)),
        ..ExecContext::default()
    }
}

fn run(factory: Factory, settings: Vec<Node>, input: Table) -> Result<Table, OperatorError> {
    factory(&node(settings))?.run(&[Arc::new(input)], &context())
}

fn col(table: &Table, name: &str) -> Vec<Value> {
    table.column(name).unwrap().cloned().collect()
}
fn i(value: i64) -> Value {
    Value::Integer(value)
}
fn date(day: u32) -> Value {
    Value::Date(days_from_civil(2024, 3, day).unwrap())
}
fn settings_error(result: Result<Table, OperatorError>) {
    assert!(
        matches!(result, Err(OperatorError::Settings(_))),
        "{result:?}"
    );
}

fn aggregate_settings() -> Vec<Node> {
    vec![
        p("rfmAggregateIdField", "customer"),
        p("rfmAggregateDateField", "day"),
        p("rfmAggregateValueField", "amount"),
        p("rfmAggregateRecencyDateMode", "Fixed"),
        // 2024-03-31 midnight UTC.
        p("rfmAggregateRecencyDate", "1711843200000"),
    ]
}

fn transactions(rows: Vec<Vec<Value>>) -> Table {
    table(
        &[
            ("customer", Storage::String),
            ("day", Storage::Date),
            ("amount", Storage::Real),
        ],
        rows,
    )
}

#[test]
fn aggregation_preserves_first_order_nulls_and_duplicate_top_dates() {
    let mut settings = aggregate_settings();
    settings.extend([
        p("rfmAggregateSaveDate2", "true"),
        p("rfmAggregateSaveDate3", "true"),
    ]);
    let output = run(
        rfm_aggregate::build,
        settings,
        transactions(vec![
            vec![Value::string("b"), date(30), Value::Real(2.5)],
            vec![Value::string("a"), date(28), Value::Null],
            vec![Value::string("b"), date(30), Value::Real(4.0)],
            vec![Value::string("b"), date(29), Value::Null],
            vec![Value::string("b"), Value::Null, Value::Real(1.5)],
            vec![Value::string("z"), Value::Null, Value::Null],
        ]),
    )
    .unwrap();
    assert_eq!(
        col(&output, "customer"),
        vec![Value::string("b"), Value::string("a"), Value::string("z")]
    );
    assert_eq!(col(&output, "Recency"), vec![i(1), i(3), Value::Null]);
    assert_eq!(col(&output, "Frequency"), vec![i(4), i(1), i(1)]);
    assert_eq!(
        col(&output, "Monetary"),
        vec![Value::Real(8.0), Value::Null, Value::Null]
    );
    assert_eq!(col(&output, "R2"), vec![i(1), Value::Null, Value::Null]);
    assert_eq!(col(&output, "R3"), vec![i(2), Value::Null, Value::Null]);
}

#[test]
fn aggregation_filters_before_grouping_with_inclusive_amount_threshold() {
    let mut settings = aggregate_settings();
    settings.extend([
        p("rfmAggregateDiscardRecordsBelow", "true"),
        p("rfmAggregateDiscardRecordsBelowValue", "5"),
    ]);
    let output = run(
        rfm_aggregate::build,
        settings,
        transactions(vec![
            vec![Value::string("gone"), date(31), Value::Real(4.99)],
            vec![Value::string("a"), date(28), Value::Real(5.0)],
            vec![Value::string("a"), date(31), Value::Null],
        ]),
    )
    .unwrap();
    assert_eq!(
        output.rows(),
        &[vec![Value::string("a"), i(3), i(1), Value::Real(5.0)]]
    );
}

#[test]
fn aggregation_relative_window_excludes_lower_bound_and_future() {
    let mut settings = aggregate_settings();
    settings.extend([
        p("rfmAggregateIncludeRecentTransactions", "true"),
        p("rfmAggregateSpecifyTransactionDate", "false"),
        p("rfmAggregateTransactionWithinLast", "2"),
        p("rfmAggregateTransactionsScale", "Days"),
    ]);
    let output = run(
        rfm_aggregate::build,
        settings,
        transactions(vec![
            vec![Value::string("x"), date(29), Value::Real(100.0)],
            vec![Value::string("x"), date(30), Value::Real(2.0)],
            vec![Value::string("x"), date(31), Value::Real(3.0)],
            vec![
                Value::string("x"),
                Value::Date(days_from_civil(2024, 4, 1).unwrap()),
                Value::Real(100.0),
            ],
        ]),
    )
    .unwrap();
    assert_eq!(
        output.rows(),
        &[vec![Value::string("x"), i(0), i(2), Value::Real(5.0)]]
    );
}

#[test]
fn aggregation_absolute_cutoff_does_not_exclude_future() {
    let mut settings = aggregate_settings();
    settings.extend([
        p("rfmAggregateIncludeRecentTransactions", "true"),
        p("rfmAggregateSpecifyTransactionDate", "true"),
        p("rfmAggregateTransactionDateAfter", "1711756800000"),
    ]);
    let output = run(
        rfm_aggregate::build,
        settings,
        transactions(vec![
            vec![Value::string("x"), date(30), Value::Real(100.0)],
            vec![
                Value::string("x"),
                Value::Date(days_from_civil(2024, 4, 1).unwrap()),
                Value::Real(7.0),
            ],
        ]),
    )
    .unwrap();
    assert_eq!(
        output.rows(),
        &[vec![Value::string("x"), i(-1), i(1), Value::Real(7.0)]]
    );
}

#[test]
fn aggregation_invalid_calendar_month_window_returns_empty_schema() {
    let mut settings = aggregate_settings();
    settings.extend([
        p("rfmAggregateIncludeRecentTransactions", "true"),
        p("rfmAggregateSpecifyTransactionDate", "false"),
        p("rfmAggregateTransactionWithinLast", "1"),
        p("rfmAggregateTransactionsScale", "Months"),
    ]);
    let output = run(
        rfm_aggregate::build,
        settings,
        transactions(vec![vec![Value::string("x"), date(31), Value::Real(3.0)]]),
    )
    .unwrap();
    assert!(output.rows().is_empty());
    assert_eq!(output.model().len(), 4);
}

#[test]
fn aggregation_timestamp_recency_uses_calendar_days_and_custom_names() {
    let mut settings = aggregate_settings();
    settings.extend([
        p("rfmAggregateNameExtension", "pre_"),
        p("rfmAggregateExtensionPosition", "Prefix"),
        p("rfmAggregateContiguousIds", "true"),
        p("rfmAggregateRecencyOutputFieldName", "R"),
    ]);
    let tick = days_from_civil(2024, 3, 30).unwrap() * MICROS_PER_DAY + MICROS_PER_DAY - 1;
    let output = run(
        rfm_aggregate::build,
        settings,
        table(
            &[
                ("customer", Storage::String),
                ("day", Storage::Timestamp),
                ("amount", Storage::Integer),
            ],
            vec![vec![Value::string("x"), Value::Timestamp(tick), i(2)]],
        ),
    )
    .unwrap();
    assert_eq!(col(&output, "pre_R"), vec![i(1)]);
    assert_eq!(col(&output, "pre_Monetary"), vec![Value::Real(2.0)]);
}

#[test]
fn aggregation_rejects_unimplemented_and_malformed_settings() {
    for (key, value) in [
        ("rfmAggregateRecencyDateMode", "Today"),
        ("rfmAggregateSaveDate3", "true"),
        ("rfmAggregateRecencyDate", "9223372036854775807"),
        ("rfmAggregateContiguousIds", "yes"),
        ("rfmAggregateIdOutputFieldName", "renamed"),
        ("rfmAggregateUnknown", "true"),
    ] {
        let mut settings = aggregate_settings();
        settings.retain(|e| !matches!(e, Node::Element(e) if e.name.local.as_ref() == key));
        settings.push(p(key, value));
        settings_error(run(rfm_aggregate::build, settings, transactions(vec![])));
    }
}

#[test]
fn aggregation_rejects_nonfinite_data_and_monetary_overflow() {
    for values in [vec![f64::INFINITY], vec![f64::MAX, f64::MAX]] {
        let input = transactions(
            values
                .into_iter()
                .map(|v| vec![Value::string("x"), date(31), Value::Real(v)])
                .collect(),
        );
        assert!(matches!(
            run(rfm_aggregate::build, aggregate_settings(), input),
            Err(OperatorError::Data(_))
        ));
    }
}

#[test]
fn aggregation_rejects_noncontiguous_ids_in_contiguous_mode() {
    let mut settings = aggregate_settings();
    settings.push(p("rfmAggregateContiguousIds", "true"));
    settings_error(run(
        rfm_aggregate::build,
        settings,
        transactions(
            ["a", "b", "a"]
                .into_iter()
                .map(|id| vec![Value::string(id), date(31), Value::Real(1.0)])
                .collect(),
        ),
    ));
}

#[test]
fn saved_instants_are_days_in_the_stream_time_zone() {
    let zone = |name: &str| {
        let Node::Element(properties) = element("properties", vec![p("timeZone", name)]) else {
            panic!("element")
        };
        ExecContext {
            stream_properties: Some(Arc::new(properties)),
            ..ExecContext::default()
        }
    };
    // 2020-03-30T22:00:00Z, saved by a date picker at midnight in UTC+2.
    let saved = 1_585_605_600_000;
    let day = |y, m, d| days_from_civil(y, m, d).unwrap();
    assert_eq!(zone("UTC").day_of_instant(saved).unwrap(), day(2020, 3, 30));
    assert_eq!(
        zone("Europe/Berlin").day_of_instant(saved).unwrap(),
        day(2020, 3, 31)
    );
    assert!(zone("Not/AZone").day_of_instant(saved).is_err());
}
