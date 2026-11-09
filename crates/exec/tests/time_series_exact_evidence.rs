//! Read-only corpus evidence for the Guide estimation investigation.
#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "corpus evidence assertions"
)]
mod support;
use dms_algo::{arima::ArimaSpec, time_series::FittedSpec};
use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_stream::Stream;
use dms_strfmt::{ArchiveLimits, read_length_prefixed_zip};
use serde_json::{Value, json};
use std::sync::Arc;

fn noise(a: &ArimaSpec) -> Value {
    json!({"constant":a.constant,"ar":a.ar,"ma":a.ma,"sar":a.seasonal_ar,
        "sma":a.seasonal_ma,"d":a.d,"sd":a.seasonal_d,"period":a.period,
        "transform":format!("{:?}",a.transform),"sigma2":a.sigma2,"df":a.df})
}
fn spec(a: &FittedSpec) -> Value {
    match a {
        FittedSpec::Arima(a) => json!({"noise":noise(a),"effects":[]}),
        FittedSpec::Transfer(a) => {
            json!({"noise":noise(&a.noise),"effects":a.effects.iter().map(|e|
            json!({"field":e.field,"num":e.numerator,"den":e.denominator,"delay":e.delay,
            "d":e.difference,"sd":e.seasonal_difference,"period":e.period})).collect::<Vec<_>>()})
        }
        _ => json!({"other":format!("{a:?}")}),
    }
}

fn collect_evidence(only_exact: bool) -> Vec<Value> {
    let root = support::repo_root();
    let cases = [
        "ts_airline",
        "ts_airline_noconst",
        "ts_ar",
        "ts_arima",
        "ts_arima_log",
        "ts_arima_sqrt",
        "ts_ma",
        "ts_sarlog",
        "ts_sarlog_noconst",
        "ts_seasonal",
        "ts_tf_delay",
        "ts_tf_den",
        "ts_tf_diff",
        "ts_tf_num",
        "ts_tf_static",
        "ts_tf_two",
    ];
    let mut output = Vec::new();
    for (label, case) in cases
        .iter()
        .map(|c| ("ts_more", *c))
        .chain([("models_next", "ts_arima")])
    {
        if only_exact && (label != "ts_more" || !EXACT_CASES.contains(&case)) {
            continue;
        }
        println!("export {label}/{case}");
        let archive = dms_strfmt::container::read_archive(
            &std::fs::read(root.join(format!("corpus/{label}/{case}.str"))).unwrap(),
            &ArchiveLimits::default(),
        )
        .unwrap();
        let stream = Stream::from_document(&archive.document).unwrap();
        let context = ExecContext {
            files: FileResolver {
                data_dirs: vec![
                    root.join(format!("corpus/{label}")),
                    root.join("corpus/ibm-demo-data"),
                ],
            },
            attachments: Arc::new(
                archive
                    .attachments
                    .into_iter()
                    .map(|a| (a.name, Arc::from(a.bytes)))
                    .collect(),
            ),
            stream_properties: stream.properties.clone().map(Arc::new),
            ..ExecContext::default()
        };
        let builder = stream
            .diagram
            .nodes
            .iter()
            .find(|n| n.type_identifier() == "spss.ts!ts_builder")
            .unwrap();
        let input = stream
            .diagram
            .nodes
            .iter()
            .find(|n| n.type_identifier() == "TypeNode")
            .unwrap();
        let result =
            Plan::new(&stream).evaluate(&[input.uid.as_str(), builder.uid.as_str()], &context);
        let Some(Ok(NodeOutput::Data(table))) = result.get(&input.uid) else {
            panic!("{case}: input")
        };
        let Some(Ok(NodeOutput::Model(model))) = result.get(&builder.uid) else {
            panic!("{case}: builder {:?}", result.get(&builder.uid))
        };
        let dms_pmml::Model::Mining(m) = &model.model else {
            panic!("mining")
        };
        let dms_pmml::Model::TimeSeries { xml, .. } = m.segments[0].model.as_ref() else {
            panic!("series")
        };
        let fitted = dms_exec::nodes::time_series::parse_saved_model(xml).unwrap();
        let target = if label == "ts_more" { "y" } else { "sales" };
        let at = table.model().position(target).unwrap();
        let y: Vec<_> = table
            .rows()
            .iter()
            .map(|r| r[at].as_f64().unwrap())
            .collect();
        let mut columns = serde_json::Map::new();
        for field in ["x1", "x2"] {
            if let Some(at) = table.model().position(field) {
                columns.insert(
                    field.into(),
                    json!(
                        table
                            .rows()
                            .iter()
                            .map(|r| r[at].as_f64().unwrap())
                            .collect::<Vec<_>>()
                    ),
                );
            }
        }
        let mut saved = Vec::new();
        for nugget in stream
            .diagram
            .nodes
            .iter()
            .filter(|n| n.type_identifier() == "spss.ts!ts_applier")
        {
            let member = &nugget
                .containers
                .iter()
                .find(|c| c.name.as_deref() == Some("PMML"))
                .unwrap()
                .member;
            let entries =
                read_length_prefixed_zip(&context.attachments[member], &ArchiveLimits::default())
                    .unwrap();
            let xml =
                std::str::from_utf8(&entries.iter().find(|e| e.name == "0.xml").unwrap().bytes)
                    .unwrap();
            saved.push(json!({"uid":nugget.uid,"spec":spec(&dms_exec::nodes::time_series::parse_saved_model(xml).unwrap())}));
        }
        output.push(json!({"label":label,"case":case,"y":y,"x":columns,"saved":saved,"fitted":spec(&fitted)}));
    }
    output
}

const EXACT_CASES: &[&str] = &[
    "ts_ar",
    "ts_ma",
    "ts_airline_noconst",
    "ts_tf_delay",
    "ts_tf_den",
    "ts_tf_num",
    "ts_tf_static",
];

fn assert_parameters(actual: &Value, expected: &Value, context: &str) {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            assert!(
                (a - b).abs() <= 1e-9 * b.abs().max(1.0),
                "{context}: {a} != {b}"
            );
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{context}");
            for (a, b) in a.iter().zip(b) {
                assert_parameters(a, b, context);
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            for (key, a) in a {
                // Saved predictor field IDs are ordinal; fitted fields use names.
                if key != "field" {
                    assert_parameters(a, &b[key], &format!("{context}/{key}"));
                }
            }
        }
        _ => assert_eq!(actual, expected, "{context}"),
    }
}

#[test]
fn guide_stopping_matches_recorded_coefficients() {
    let output = collect_evidence(true);
    assert_eq!(output.len(), EXACT_CASES.len());
    for case in output {
        assert_parameters(
            &case["fitted"],
            &case["saved"][0]["spec"],
            case["case"].as_str().unwrap(),
        );
    }
}

#[test]
#[ignore = "explicit local evidence export; writes only target/"]
fn export_exact_evidence() {
    std::fs::write(
        support::repo_root().join("target/arima-evidence.json"),
        serde_json::to_string_pretty(&collect_evidence(false)).unwrap() + "\n",
    )
    .unwrap();
}
