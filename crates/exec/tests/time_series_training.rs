//! Statistical parity (optimizer), acceptance rules 1, 3, 5.
#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;

use dms_algo::time_series::{self, FittedSpec};
use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{Model, Pmml};
use dms_stream::{NodeRef, Stream};
use dms_strfmt::{ArchiveLimits, read_length_prefixed_zip};
use dms_types::Table;
use std::sync::Arc;

const CASES: [(&str, &str); 4] = [
    ("models_next", "ts_arima"),
    ("models_next", "ts_expert"),
    ("models_next2", "ts_simple"),
    ("models_next2", "ts_holt"),
];

fn load(label: &str, case: &str) -> (Stream, ExecContext) {
    let root = support::repo_root();
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(root.join(format!("corpus/{label}/{case}.str"))).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let context = ExecContext {
        files: FileResolver {
            data_dirs: vec![root.join(format!("corpus/{label}"))],
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
    (stream, context)
}

fn series_specs(pmml: &Pmml) -> Vec<(String, FittedSpec)> {
    let Model::Mining(m) = &pmml.model else {
        panic!("expected time series collection")
    };
    assert_eq!(m.method, "selectAll");
    m.segments
        .iter()
        .map(|segment| {
            let Model::TimeSeries { header, xml } = segment.model.as_ref() else {
                panic!("not TimeSeriesModel")
            };
            let target = &header
                .mining_schema
                .iter()
                .find(|f| f.usage == dms_pmml::Usage::Target)
                .unwrap()
                .name;
            (
                target.clone(),
                dms_exec::nodes::time_series::parse_saved_model(xml).unwrap(),
            )
        })
        .collect()
}

fn saved_specs(node: &dms_stream::StreamNode, context: &ExecContext) -> Vec<FittedSpec> {
    let member = &node
        .containers
        .iter()
        .find(|c| c.name.as_deref() == Some("PMML"))
        .unwrap()
        .member;
    let entries =
        read_length_prefixed_zip(&context.attachments[member], &ArchiveLimits::default()).unwrap();
    (0..2)
        .map(|i| {
            let bytes = &entries
                .iter()
                .find(|e| e.name == format!("{i}.xml"))
                .unwrap()
                .bytes;
            dms_exec::nodes::time_series::parse_saved_model(std::str::from_utf8(bytes).unwrap())
                .unwrap()
        })
        .collect()
}

fn compare(
    table: &Table,
    label: &str,
    case: &str,
    uid: &str,
    tolerances: Option<&[(String, f64)]>,
) -> (usize, f64) {
    let directory = support::repo_root().join(format!("oracle/{label}/{case}/data"));
    let path = std::fs::read_dir(directory)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(&format!("_{uid}.csv")))
        .unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    let mut lines = text.lines().filter(|l| !l.is_empty());
    let header = support::parse_csv_line(lines.next().unwrap());
    let rows: Vec<_> = lines.map(support::parse_csv_line).collect();
    let names: Vec<_> = table
        .model()
        .fields()
        .iter()
        .map(|f| f.name.clone())
        .collect();
    assert_eq!(names, header, "{case} schema names/order");
    assert_eq!(rows.len(), table.row_count(), "{case} row count");
    let mut maximum: f64 = 0.0;
    let mut checked = 0;
    for (actual, expected) in table.rows().iter().zip(rows) {
        for ((a, b), name) in actual.iter().zip(expected).zip(&header) {
            match (a.as_f64(), b.parse::<f64>()) {
                (Some(a), Ok(b)) => {
                    let prediction = name.starts_with("$TS");
                    let tolerance = if prediction {
                        tolerances
                            .and_then(|ts| {
                                ts.iter()
                                    .find(|(target, _)| name.ends_with(&format!("-{target}")))
                                    .map(|(_, t)| *t)
                            })
                            .unwrap_or(1e-9 * b.abs().max(1.0))
                    } else {
                        1e-12 * b.abs().max(1.0)
                    };
                    if prediction {
                        maximum = maximum.max((a - b).abs());
                        checked += 1;
                    }
                    assert!(
                        (a - b).abs() <= tolerance,
                        "{case} {uid} {name}: ours={a:.15} Modeler={b:.15} diff={} bound={tolerance}",
                        (a - b).abs()
                    );
                }
                _ => assert_eq!(
                    if a.is_null() {
                        String::new()
                    } else {
                        a.to_string()
                    },
                    b,
                    "{case} {name}"
                ),
            }
        }
    }
    (checked, maximum)
}

#[test]
fn time_series_optimizer_parity_all_four_oracles() {
    let mut compared = 0;
    for (label, case) in CASES {
        let (stream, context) = load(label, case);
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
        let plan = Plan::new(&stream);
        let results = plan.evaluate(&[&builder.uid, &input.uid], &context);
        // Explicitly require a trained model: Plan otherwise falls back to a saved nugget.
        let Some(Ok(NodeOutput::Model(pmml))) = results.get(&builder.uid) else {
            panic!("{case} builder: {:?}", results.get(&builder.uid));
        };
        let Some(Ok(NodeOutput::Data(table))) = results.get(&input.uid) else {
            panic!("{case} input")
        };
        let ours = series_specs(pmml);
        let nugget = stream
            .diagram
            .nodes
            .iter()
            .find(|n| n.type_identifier() == "spss.ts!ts_applier")
            .unwrap();
        let saved = saved_specs(nugget, &context);
        let mut tolerances = Vec::new();
        for ((name, estimated), modeler) in ours.iter().zip(&saved) {
            let at = table.model().position(name).unwrap();
            let y: Vec<_> = table
                .rows()
                .iter()
                .map(|r| r[at].as_f64().unwrap())
                .collect();
            let n = y.len() as f64;
            let mean = y.iter().sum::<f64>() / n;
            let sd = (y.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
            tolerances.push((name.clone(), 0.01 * sd));
            let a = time_series::objective(estimated, &y).unwrap();
            let b = time_series::objective(modeler, &y).unwrap();
            println!(
                "{case}/{name}: objective ours={a:.15} Modeler={b:.15}; prediction tolerance={:.15}",
                0.01 * sd
            );
            println!("  coefficients ours={estimated:?}\n  coefficients Modeler={modeler:?}");
            assert!(
                a <= b + 1e-9 * b.abs().max(f64::MIN_POSITIVE),
                "{case}/{name} objective {a} > {b}"
            );
            match (estimated, modeler) {
                (FittedSpec::Arima(a), FittedSpec::Arima(b)) => {
                    assert_eq!(a.df, b.df);
                    assert_eq!(a.transform, b.transform);
                    assert_eq!((a.ar.len(), a.d, a.ma.len()), (b.ar.len(), b.d, b.ma.len()));
                }
                (FittedSpec::Smooth(_), FittedSpec::Smooth(_)) => {}
                _ => panic!("model family changed"),
            }
        }
        // Exercise the unchanged nugget's scoring/assembly path with new PMML.
        // Remove saved PMML attachments so this cannot pass by using saved coefficients.
        let mut scoring = context.clone();
        scoring.attachments = Arc::new(Default::default());
        for node in stream
            .diagram
            .nodes
            .iter()
            .filter(|n| n.type_identifier() == "spss.ts!ts_applier")
        {
            let op = dms_exec::nodes::time_series::apply(node).unwrap();
            let result = op
                .produce(&[Arc::clone(table)], Some(Arc::clone(pmml)), &scoring)
                .unwrap();
            let NodeOutput::Data(scored) = result else {
                panic!("expected scores")
            };
            let NodeOutput::Data(saved) = op.produce(&[Arc::clone(table)], None, &context).unwrap()
            else {
                panic!("saved scores")
            };
            assert_eq!(
                scored.model(),
                saved.model(),
                "{case}: storage, measurement and output order"
            );
            let (count, max) = compare(&scored, label, case, &node.uid, Some(&tolerances));
            println!(
                "{case}/{}: {count} fitted/forecast/interval cells; largest difference={max:.15}",
                node.uid
            );
            compared += count;
        }
    }
    println!("optimizer parity compared {compared} prediction cells");
}

#[test]
fn saved_time_series_nuggets_still_score_exactly_without_builder() {
    let mut compared = 0;
    for (label, case) in CASES {
        let (mut stream, context) = load(label, case);
        let builder = stream
            .diagram
            .nodes
            .iter()
            .find(|n| n.type_identifier() == "spss.ts!ts_builder")
            .unwrap()
            .uid
            .clone();
        stream.diagram.nodes.retain(|n| n.uid != builder);
        stream.diagram.links.retain(|l| {
            l.from != NodeRef::Uid(builder.clone()) && l.to != NodeRef::Uid(builder.clone())
        });
        let nodes: Vec<_> = stream
            .diagram
            .nodes
            .iter()
            .filter(|n| n.type_identifier() == "spss.ts!ts_applier")
            .map(|n| n.uid.as_str())
            .collect();
        let results = Plan::new(&stream).evaluate(&nodes, &context);
        for uid in nodes {
            let Some(Ok(NodeOutput::Data(table))) = results.get(uid) else {
                panic!("{case}/{uid}: {:?}", results.get(uid))
            };
            let (n, max) = compare(table, label, case, uid, None);
            compared += n;
            println!("saved {case}/{uid}: {n} exact cells; largest difference={max:.15}");
        }
    }
    println!("saved-nugget exact parity compared {compared} prediction cells");
}

#[test]
fn unsupported_time_series_settings_are_gaps() {
    use dms_strfmt::fastinfoset::Node;
    let (stream, _) = load("models_next", "ts_arima");
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "spss.ts!ts_builder")
        .unwrap();
    for (key, value) in [
        ("_method", "Unknown"),
        ("_useWeight", "true"),
        ("_specifyEstimationPeriod", "true"),
        ("_arimaDetectOutliersAutomatically", "true"),
        ("_continueEstimation", "true"),
        ("_dataStructure", "panel"),
    ] {
        let mut node = builder.clone();
        let property = node
            .settings
            .as_mut()
            .unwrap()
            .children
            .iter_mut()
            .find_map(|n| match n {
                Node::Element(e) if &*e.name.local == key => Some(e),
                _ => None,
            })
            .unwrap();
        property.children = vec![Node::Text(Arc::from(value))];
        assert!(
            matches!(dms_exec::nodes::time_series::build(&node),Err(dms_exec::OperatorError::Settings(s)) if s.starts_with("GAP:")),
            "{key}"
        );
    }
    let (stream, _) = load("models_next", "ts_expert");
    let mut node = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "spss.ts!ts_builder")
        .unwrap()
        .clone();
    node.containers.clear();
    // Without a recorded specification, Expert AllModels now selects automatically
    // (`time_series_expert.rs`), so the builder is accepted.
    assert!(dms_exec::nodes::time_series::build(&node).is_ok());
    assert!(dms_exec::registry::is_statistical("time_series"));
}

#[test]
fn estimation_never_uses_saved_parameters_and_responds_to_new_data() {
    use std::io::{Cursor, Write};
    for (label, case) in CASES {
        let (stream, context) = load(label, case);
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
        let results = Plan::new(&stream).evaluate(&[&builder.uid, &input.uid], &context);
        let Some(Ok(NodeOutput::Model(original))) = results.get(&builder.uid) else {
            panic!("{case} model")
        };
        let Some(Ok(NodeOutput::Data(table))) = results.get(&input.uid) else {
            panic!("{case} input")
        };
        let mut bare = context.clone();
        let mut attachments = std::collections::HashMap::new();
        if case == "ts_expert" {
            let member = &builder
                .containers
                .iter()
                .find(|c| c.name.as_deref() == Some("PMML"))
                .unwrap()
                .member;
            let entries =
                read_length_prefixed_zip(&context.attachments[member], &ArchiveLimits::default())
                    .unwrap();
            let datamodel = &entries
                .iter()
                .find(|e| e.name == "datamodel.xml")
                .unwrap()
                .bytes;
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            zip.start_file("datamodel.xml", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(datamodel).unwrap();
            for index in 0..2 {
                zip.start_file(
                    format!("{index}.xml"),
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
                // No constant, variance, initial state, historical values or coefficients.
                zip.write_all(b"<PMML><TimeSeriesModel><ARIMA transformation=\"logarithmic\" seasonLength=\"1\"><NonseasonalComponent p=\"0\" d=\"1\" q=\"0\"/></ARIMA></TimeSeriesModel></PMML>").unwrap();
            }
            attachments.insert(
                member.clone(),
                Arc::<[u8]>::from(zip.finish().unwrap().into_inner()),
            );
        }
        bare.attachments = Arc::new(attachments);
        let operator = dms_exec::nodes::build::build(builder).unwrap();
        let NodeOutput::Model(refit) = operator.produce(&[Arc::clone(table)], None, &bare).unwrap()
        else {
            panic!("refit model")
        };
        assert_eq!(
            original, &refit,
            "{case}: no saved parameter may affect estimation"
        );
        let mut rows = table.rows().to_vec();
        for row in rows.iter_mut().step_by(2) {
            for target in ["sales", "cost"] {
                let at = table.model().position(target).unwrap();
                row[at] = dms_types::Value::Real(row[at].as_f64().unwrap() + 10.0);
            }
        }
        let changed = Arc::new(Table::new(table.model().clone(), rows).unwrap());
        let NodeOutput::Model(new) = operator.produce(&[changed], None, &bare).unwrap() else {
            panic!("changed model")
        };
        assert_ne!(
            refit, new,
            "{case}: estimation must respond to new observations"
        );
    }
}
