//! RFM Analysis on small synthetic tables. Oracle case names in module documentation establish
//! the rules; these fixtures deliberately use independent values and dates.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_exec::nodes::rfm_analysis;
use dms_exec::{ExecContext, Factory, OperatorError};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Attribute, Element, Node, QualifiedName};
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

/// A `BinningOutput` among `settings` becomes the node's sibling element, as streams save it.
fn node(settings: Vec<Node>) -> StreamNode {
    let (cache, settings): (Vec<Node>, Vec<Node>) = settings
        .into_iter()
        .partition(|n| matches!(n, Node::Element(e) if &*e.name.local == "BinningOutput"));
    let binning_output = cache.into_iter().find_map(|n| match n {
        Node::Element(e) => Some(e),
        _ => None,
    });
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
        binning_output,
        result_items: Vec::new(),
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
fn settings_error(result: Result<Table, OperatorError>) {
    assert!(
        matches!(result, Err(OperatorError::Settings(_))),
        "{result:?}"
    );
}

fn analysis_settings() -> Vec<Node> {
    vec![
        p("rfmAnalysisRecencyField", "r"),
        p("rfmAnalysisFrequencyField", "f"),
        p("rfmAnalysisMonetaryField", "m"),
    ]
}

fn measures(values: &[(Option<f64>, Option<i64>, Option<f64>)]) -> Table {
    table(
        &[
            ("r", Storage::Real),
            ("f", Storage::Integer),
            ("m", Storage::Real),
        ],
        values
            .iter()
            .map(|(r, f, m)| {
                vec![
                    r.map_or(Value::Null, Value::Real),
                    f.map_or(Value::Null, i),
                    m.map_or(Value::Null, Value::Real),
                ]
            })
            .collect(),
    )
}

#[test]
fn analysis_independent_bins_orientation_and_fractional_weights() {
    let mut settings = analysis_settings();
    settings.extend([
        p("rfmAnalysisRecencyNumBins", "2"),
        p("rfmAnalysisFrequencyNumBins", "3"),
        p("rfmAnalysisMonetaryNumBins", "2"),
        p("rfmAnalysisRecencyWeight", "0.5"),
        p("rfmAnalysisFrequencyWeight", "0"),
        p("rfmAnalysisMonetaryWeight", "2.25"),
    ]);
    let input = measures(&[
        (Some(0.0), Some(1), Some(1.0)),
        (Some(1.0), Some(2), Some(2.0)),
        (Some(2.0), Some(3), Some(3.0)),
        (Some(3.0), Some(4), Some(4.0)),
        (Some(4.0), Some(5), Some(5.0)),
        (Some(5.0), Some(6), Some(6.0)),
    ]);
    let output = run(rfm_analysis::build, settings, input.clone()).unwrap();
    assert_eq!(col(&output, "r"), col(&input, "r"));
    assert_eq!(
        col(&output, "Recency Score"),
        vec![i(2), i(2), i(1), i(1), i(1), i(1)]
    );
    assert_eq!(
        col(&output, "Frequency Score"),
        vec![i(1), i(1), i(1), i(2), i(2), i(3)]
    );
    assert_eq!(
        col(&output, "Monetary Score"),
        vec![i(1), i(1), i(1), i(1), i(2), i(2)]
    );
    assert_eq!(
        col(&output, "RFM Score"),
        vec![
            Value::Real(3.25),
            Value::Real(3.25),
            Value::Real(2.75),
            Value::Real(2.75),
            Value::Real(5.0),
            Value::Real(5.0)
        ]
    );
    assert_eq!(
        output.model().field("RFM Score").unwrap().storage,
        Storage::Real
    );
}

#[test]
fn analysis_ties_current_next_and_row_reversal() {
    let data = measures(&[
        (Some(0.0), Some(1), Some(1.0)),
        (Some(1.0), Some(2), Some(2.0)),
        (Some(1.0), Some(2), Some(2.0)),
        (Some(1.0), Some(2), Some(2.0)),
        (Some(2.0), Some(3), Some(3.0)),
        (Some(3.0), Some(4), Some(4.0)),
    ]);
    let mut settings = analysis_settings();
    settings.push(p("rfmAnalysisMonetaryNumBins", "2"));
    let current = run(rfm_analysis::build, settings.clone(), data.clone()).unwrap();
    assert_eq!(
        col(&current, "Monetary Score"),
        vec![i(1), i(1), i(1), i(1), i(2), i(2)]
    );
    let reversed = Table::new(
        data.model().clone(),
        data.rows().iter().cloned().rev().collect(),
    )
    .unwrap();
    let output = run(rfm_analysis::build, settings.clone(), reversed).unwrap();
    assert_eq!(
        output.rows().iter().cloned().rev().collect::<Vec<_>>(),
        current.rows()
    );
    settings.push(p("tiedValuesMethod", "high"));
    let next = run(rfm_analysis::build, settings, data).unwrap();
    assert_eq!(
        col(&next, "Monetary Score"),
        vec![i(1), i(2), i(2), i(2), i(2), i(2)]
    );
}

#[test]
fn analysis_nulls_and_negative_recency_propagate_even_with_zero_weight() {
    let mut settings = analysis_settings();
    settings.push(p("rfmAnalysisRecencyWeight", "0"));
    let output = run(
        rfm_analysis::build,
        settings,
        measures(&[
            (Some(-1.0), Some(2), Some(3.0)),
            (None, Some(3), Some(4.0)),
            (Some(4.0), None, Some(5.0)),
            (Some(5.0), Some(4), None),
            (None, None, None),
        ]),
    )
    .unwrap();
    assert_eq!(col(&output, "Recency Score").first(), Some(&Value::Null));
    assert_eq!(col(&output, "RFM Score"), vec![Value::Null; 5]);
}

#[test]
fn analysis_constant_singleton_and_empty() {
    for n in [0, 1, 7] {
        let rows = vec![(Some(7.0), Some(2), Some(10.0)); n];
        let output = run(rfm_analysis::build, analysis_settings(), measures(&rows)).unwrap();
        assert_eq!(output.model().len(), 7);
        assert_eq!(col(&output, "RFM Score"), vec![Value::Real(111.0); n]);
    }
}

#[test]
fn analysis_temporal_recency_keeps_timestamp_distinctions() {
    let mut settings = analysis_settings();
    settings.push(p("rfmAnalysisRecencyNumBins", "2"));
    let output = run(
        rfm_analysis::build,
        settings,
        table(
            &[
                ("r", Storage::Timestamp),
                ("f", Storage::Integer),
                ("m", Storage::Real),
            ],
            (0..6)
                .map(|v| vec![Value::Timestamp(v * 1_000_000), i(1), Value::Real(1.0)])
                .collect(),
        ),
    )
    .unwrap();
    assert_eq!(
        col(&output, "Recency Score"),
        vec![i(1), i(1), i(1), i(1), i(2), i(2)]
    );
}

#[test]
fn analysis_does_not_merge_large_integers_or_timestamp_microseconds() {
    for (storage, values) in [
        (
            Storage::Integer,
            (0..6).map(|v| i(i64::MAX - v)).collect::<Vec<_>>(),
        ),
        (
            Storage::Timestamp,
            (0..6)
                .map(|v| Value::Timestamp(1_700_000_000_000_000 + v))
                .collect(),
        ),
    ] {
        let mut settings = analysis_settings();
        settings.push(p("rfmAnalysisRecencyNumBins", "2"));
        let output = run(
            rfm_analysis::build,
            settings,
            table(
                &[
                    ("r", storage),
                    ("f", Storage::Integer),
                    ("m", Storage::Real),
                ],
                values
                    .into_iter()
                    .map(|v| vec![v, i(1), Value::Real(1.0)])
                    .collect(),
            ),
        )
        .unwrap();
        assert_eq!(
            col(&output, "Recency Score"),
            vec![i(1), i(1), i(1), i(1), i(2), i(2)]
        );
    }
}

fn cache() -> Node {
    let intervals = [(8.0, 30.0, false, false, 1), (2.0, 8.0, false, true, 2)];
    let tables = (0..3)
        .map(|dimension| {
            let bins = if dimension == 0 {
                intervals.to_vec()
            } else {
                vec![(0.0, 5.0, true, false, 1), (5.0, 30.0, true, true, 2)]
            };
            let rows = bins
                .into_iter()
                .map(|(low, high, lc, uc, id)| {
                    element(
                        "row",
                        vec![
                            p("lower", &low.to_string()),
                            p("upper", &high.to_string()),
                            p("lowerClosed", &lc.to_string()),
                            p("upperClosed", &uc.to_string()),
                            p("binID", &id.to_string()),
                        ],
                    )
                })
                .collect();
            let Node::Element(mut table) = element("IntervalTable", vec![element("rows", rows)])
            else {
                panic!("element")
            };
            for (name, value) in [("Row", dimension.to_string()), ("Col", "0".into())] {
                table.attributes.push(Attribute {
                    name: QualifiedName {
                        prefix: None,
                        namespace: None,
                        local: Arc::from(name),
                    },
                    value: Arc::from(value),
                });
            }
            Node::Element(table)
        })
        .collect();
    element("BinningOutput", vec![element("IntervalTables", tables)])
}

#[test]
fn analysis_saved_intervals_clamp_to_actual_bins_or_keep_nulls() {
    for clamp in [false, true] {
        let mut settings = analysis_settings();
        settings.extend([
            p("recalculateBins", "ifnecessary"),
            p(
                "rfmAnalysisAddOutliersToEndBins",
                if clamp { "true" } else { "false" },
            ),
            cache(),
        ]);
        let output = run(
            rfm_analysis::build,
            settings,
            measures(&[
                (Some(0.0), Some(0), Some(5.0)),
                (Some(2.0), Some(5), Some(30.0)),
                (Some(8.0), Some(30), Some(31.0)),
                (Some(30.0), Some(-1), Some(1.0)),
            ]),
        )
        .unwrap();
        assert_eq!(
            col(&output, "Recency Score"),
            if clamp {
                vec![i(2), i(2), i(2), i(1)]
            } else {
                vec![Value::Null, Value::Null, i(2), Value::Null]
            }
        );
        assert_eq!(
            col(&output, "Monetary Score"),
            if clamp {
                vec![i(2), i(2), i(2), i(1)]
            } else {
                vec![i(2), i(2), Value::Null, i(1)]
            }
        );
    }
}

#[test]
fn analysis_always_ignores_saved_intervals() {
    let mut settings = analysis_settings();
    settings.extend([p("recalculateBins", "always"), cache()]);
    let output = run(
        rfm_analysis::build,
        settings,
        measures(&[(Some(0.0), Some(3), Some(4.0))]),
    )
    .unwrap();
    assert_eq!(col(&output, "Recency Score"), vec![i(1)]);
}

#[test]
fn analysis_rejects_invalid_or_unsupported_settings_and_cache() {
    for (key, value) in [
        ("rfmAnalysisRecencyNumBins", "1"),
        ("rfmAnalysisFrequencyNumBins", "9223372036854775807"),
        ("rfmAnalysisMonetaryWeight", "NaN"),
        ("rfmAnalysisMonetaryWeight", "-1"),
        ("tiedValuesMethod", "random"),
        ("recalculateBins", "ifnecessary"),
        ("rfmAnalysisRecencyScoreOutputFieldName", "Monetary Score"),
        ("rfmAnalysisUnknown", "1"),
    ] {
        let mut settings = analysis_settings();
        settings.push(p(key, value));
        settings_error(run(rfm_analysis::build, settings, measures(&[])));
    }
    let mut settings = analysis_settings();
    settings.extend([
        p("recalculateBins", "ifnecessary"),
        element("BinningOutput", vec![]),
    ]);
    settings_error(run(rfm_analysis::build, settings, measures(&[])));
}

#[test]
fn analysis_rejects_bad_input_and_score_overflow() {
    assert!(matches!(
        run(
            rfm_analysis::build,
            analysis_settings(),
            measures(&[(Some(f64::INFINITY), Some(1), Some(1.0))])
        ),
        Err(OperatorError::Data(_))
    ));
    let mut settings = analysis_settings();
    settings.push(p("rfmAnalysisRecencyWeight", "1.7976931348623157e308"));
    assert!(matches!(
        run(
            rfm_analysis::build,
            settings,
            measures(&[
                (Some(0.0), Some(1), Some(1.0)),
                (Some(1.0), Some(2), Some(2.0))
            ])
        ),
        Err(OperatorError::Data(_))
    ));
    let mut settings = analysis_settings();
    settings.push(p("rfmAnalysisRFMScoreOutputFieldName", "r"));
    settings_error(run(rfm_analysis::build, settings, measures(&[])));
}
