//! Neural Net training at statistical parity (`docs/statistical-parity.md`, rule 2): each
//! `models_ml/nn_*` builder trains with 30 seeds of our generator, and Modeler's training-record
//! accuracy (categorical targets) or RMSE (continuous targets) must lie inside our seeds' range
//! widened by 10 % of its width. Modeler's saved nuggets still score exactly
//! (`saved_neural_nuggets_score_exactly`).
#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{Pmml, Value as PValue};
use dms_stream::{NodeRef, Stream};
use dms_strfmt::ArchiveLimits;
use dms_types::{Table, Value};
use std::collections::HashMap;
use std::sync::Arc;

const BUILDER: &str = "neuralnet!spss.neuralnet-extension";
const SEEDS: u64 = 30;

/// Cases trained at statistical parity and their targets.
const CASES: &[(&str, &[&str])] = &[
    ("nn_fixed_a", &["y"]),
    ("nn_fixed_b", &["y"]),
    ("nn_seed2", &["y"]),
    ("nn_reverse", &["y"]),
    ("nn_one_cycle", &["y"]),
    ("nn_accuracy", &["y"]),
    ("nn_overfit30", &["y"]),
    ("nn_regression", &["r"]),
    ("nn_mixed_targets", &["y", "r"]),
    ("nn_missing_listwise", &["y", "r"]),
    ("nn_missing_impute", &["y", "r"]),
    ("nn_default_a", &["y"]),
    ("nn_default_b", &["y"]),
];

struct Case {
    stream: Stream,
    attachments: HashMap<String, Arc<[u8]>>,
    builder: String,
    source: String,
}

fn load(case: &str) -> Case {
    let root = support::repo_root();
    let bytes = std::fs::read(root.join(format!("corpus/models_ml/{case}.str"))).unwrap();
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == BUILDER)
        .unwrap()
        .uid
        .clone();
    let source = stream
        .diagram
        .links
        .iter()
        .find(|l| l.to == NodeRef::Uid(builder.clone()))
        .and_then(|l| match &l.from {
            NodeRef::Uid(uid) => Some(uid.clone()),
            NodeRef::Unresolved(_) => None,
        })
        .unwrap();
    let attachments = archive
        .attachments
        .into_iter()
        .map(|a| (a.name, Arc::<[u8]>::from(a.bytes)))
        .collect();
    Case {
        stream,
        attachments,
        builder,
        source,
    }
}

fn context(case: &Case, attachments: HashMap<String, Arc<[u8]>>) -> ExecContext {
    ExecContext {
        files: FileResolver {
            data_dirs: vec![support::repo_root().join("corpus/models_ml")],
        },
        attachments: Arc::new(attachments),
        stream_properties: case.stream.properties.clone().map(Arc::new),
        decimal_places: Some(15),
        ..ExecContext::default()
    }
}

/// The builder's parameters with `neuralnet.RandomSeed` replaced.
fn with_seed(case: &Case, seed: u64) -> HashMap<String, Arc<[u8]>> {
    let mut out = case.attachments.clone();
    for bytes in out.values_mut() {
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue;
        };
        if !text.contains("neuralnet.NeuralNetworkAlgorithm") {
            continue;
        }
        let marker = "name=\"neuralnet.RandomSeed\" value=\"";
        let start = text.find(marker).unwrap() + marker.len();
        let end = start + text[start..].find('"').unwrap();
        let replaced = format!("{}{seed}{}", &text[..start], &text[end..]);
        *bytes = Arc::from(replaced.into_bytes());
    }
    out
}

fn train(case: &Case, seed: u64) -> Result<(Pmml, Arc<Table>), String> {
    let context = context(case, with_seed(case, seed));
    let results =
        Plan::new(&case.stream).evaluate(&[case.builder.as_str(), case.source.as_str()], &context);
    let model = match results.get(&case.builder).unwrap() {
        Ok(NodeOutput::Model(m)) => (**m).clone(),
        Err(error) => return Err(format!("{error:?}")),
        other => panic!("builder: {other:?}"),
    };
    let Some(Ok(NodeOutput::Data(table))) = results.get(&case.source) else {
        panic!("training records failed");
    };
    Ok((model, Arc::clone(table)))
}

/// Accuracy or RMSE of each target over the training records (ids below 10000) that have a
/// target and a prediction.
fn metrics(pairs: &HashMap<&str, Vec<(String, String)>>, targets: &[&str]) -> Vec<f64> {
    targets
        .iter()
        .map(|t| {
            let rows = &pairs[t];
            if *t == "r" {
                let sq: Vec<f64> = rows
                    .iter()
                    .map(|(a, p)| {
                        let d = a.parse::<f64>().unwrap() - p.parse::<f64>().unwrap();
                        d * d
                    })
                    .collect();
                (sq.iter().sum::<f64>() / sq.len() as f64).sqrt()
            } else {
                rows.iter().filter(|(a, p)| a == p).count() as f64 / rows.len() as f64
            }
        })
        .collect()
}

fn ours(model: &Pmml, table: &Table, targets: &[&str]) -> Vec<f64> {
    let names: Vec<&str> = table
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    let scorer = model.bind(&names).unwrap();
    let id = table.model().position("id").unwrap();
    let mut pairs: HashMap<&str, Vec<(String, String)>> = HashMap::new();
    for row in table.rows() {
        if row[id].as_f64().is_none_or(|v| v >= 10000.0) {
            continue;
        }
        let record: Vec<PValue> = row
            .iter()
            .map(|v| match v {
                Value::Null => PValue::Missing,
                Value::String(s) => PValue::Text(s.to_string()),
                other => other.as_f64().map_or(PValue::Missing, PValue::Number),
            })
            .collect();
        let predictions = scorer.score_neural(&record).unwrap();
        for t in targets {
            let at = table.model().position(t).unwrap();
            let field = &table.model().fields()[at];
            if field.is_missing(&row[at]) {
                continue;
            }
            let Some(prediction) = predictions.iter().find(|p| p.name == *t) else {
                continue;
            };
            let predicted = match &prediction.value {
                PValue::Missing => continue,
                PValue::Number(n) => n.to_string(),
                PValue::Text(s) => s.clone(),
            };
            pairs
                .entry(t)
                .or_default()
                .push((row[at].to_string(), predicted));
        }
    }
    metrics(&pairs, targets)
}

fn hidden_units(model: &Pmml) -> usize {
    match &model.model {
        dms_pmml::Model::Neural(network) => network.layers.first().map_or(0, |l| l.neurons.len()),
        _ => 0,
    }
}

/// Modeler's metrics from its scored training records.
fn modeler(case: &str, targets: &[&str]) -> Vec<f64> {
    let dir = support::repo_root().join(format!("oracle/models_ml/{case}/data"));
    for path in std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()) {
        let text = std::fs::read_to_string(&path).unwrap();
        let mut lines = text.lines().filter(|l| !l.is_empty());
        let header = support::parse_csv_line(lines.next().unwrap());
        if !targets.iter().all(|t| header.contains(&format!("$N-{t}"))) {
            continue;
        }
        let rows: Vec<Vec<String>> = lines.map(support::parse_csv_line).collect();
        let id = header.iter().position(|h| h == "id").unwrap();
        if !rows
            .iter()
            .any(|r| r[id].parse::<f64>().is_ok_and(|v| v < 10000.0))
        {
            continue;
        }
        let mut pairs: HashMap<&str, Vec<(String, String)>> = HashMap::new();
        for row in &rows {
            if row[id].parse::<f64>().is_ok_and(|v| v >= 10000.0) {
                continue;
            }
            for t in targets {
                let actual = &row[header.iter().position(|h| h == t).unwrap()];
                let predicted = &row[header.iter().position(|h| *h == format!("$N-{t}")).unwrap()];
                let missing = actual.is_empty()
                    || actual == "\"MISSING\""
                    || actual == "-99"
                    || actual.starts_with("-99.");
                if missing || predicted.is_empty() {
                    continue;
                }
                let clean = |s: &str| {
                    let s = s.trim_matches('"').to_string();
                    s.parse::<f64>().map_or(s, |v| v.to_string())
                };
                pairs
                    .entry(t)
                    .or_default()
                    .push((clean(actual), clean(predicted)));
            }
        }
        return metrics(&pairs, targets);
    }
    panic!("{case}: no scored training records");
}

#[test]
fn neural_training_matches_modeler_within_the_seed_band() {
    for (case, targets) in CASES {
        let loaded = load(case);
        let theirs = modeler(case, targets);
        let mut runs: Vec<Vec<f64>> = Vec::new();
        let mut units: Vec<usize> = Vec::new();
        for k in 0..SEEDS {
            let (model, table) = train(&loaded, 12345 + k).unwrap();
            runs.push(ours(&model, &table, targets));
            units.push(hidden_units(&model));
        }
        // The automatic architecture: Modeler's hidden-unit count lies in our seeds' range.
        if case.starts_with("nn_default") {
            let path = support::repo_root()
                .join(format!("oracle/models_ml/{case}/models/01_models_ml.xml"));
            let theirs =
                hidden_units(&Pmml::parse(&std::fs::read_to_string(path).unwrap()).unwrap());
            let (low, high) = (
                units.iter().min().copied().unwrap(),
                units.iter().max().copied().unwrap(),
            );
            println!("{case}: Modeler {theirs} hidden units, our seeds {low}..{high}");
            assert!(
                (low..=high).contains(&theirs),
                "{case}: {theirs} hidden units outside {low}..{high}"
            );
        }
        for (i, target) in targets.iter().enumerate() {
            let values: Vec<f64> = runs.iter().map(|r| r[i]).collect();
            let low = values.iter().copied().fold(f64::INFINITY, f64::min);
            let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let margin = 0.1 * (high - low);
            let metric = if *target == "r" { "RMSE" } else { "accuracy" };
            println!(
                "{case} {target}: Modeler {metric} {:.4}, our {SEEDS} seeds {low:.4}..{high:.4}",
                theirs[i]
            );
            assert!(
                theirs[i] >= low - margin && theirs[i] <= high + margin,
                "{case} {target}: Modeler {metric} {} outside our band {low}..{high}",
                theirs[i]
            );
        }
    }
}

#[test]
fn neural_settings_without_an_oracle_are_gaps() {
    for case in ["nn_rbf", "nn_partition"] {
        let error = train(&load(case), 12345).unwrap_err();
        assert!(error.contains("GAP"), "{case}: {error}");
        println!("{case}: refused as a GAP");
    }
}

/// Exact tier: with the builder removed, every saved Neural Net nugget scores its oracle
/// records like Modeler (all `$` columns, numbers to 1e-9).
#[test]
fn saved_neural_nuggets_score_exactly() {
    let root = support::repo_root();
    let mut checked = 0;
    for entry in std::fs::read_dir(root.join("oracle/models_ml")).unwrap() {
        let case = entry.unwrap().file_name().to_string_lossy().into_owned();
        if !case.starts_with("nn_") || !root.join(format!("corpus/models_ml/{case}.str")).exists() {
            continue;
        }
        let mut loaded = load(&case);
        let builder = loaded.builder.clone();
        loaded.stream.diagram.nodes.retain(|n| n.uid != builder);
        loaded.stream.diagram.links.retain(|l| {
            l.from != NodeRef::Uid(builder.clone()) && l.to != NodeRef::Uid(builder.clone())
        });
        let appliers: Vec<String> = loaded
            .stream
            .diagram
            .nodes
            .iter()
            .filter(|n| n.type_identifier() == "neuralnetmodel!spss.neuralnet-extension")
            .map(|n| n.uid.clone())
            .collect();
        let context = context(&loaded, loaded.attachments.clone());
        let ids: Vec<&str> = appliers.iter().map(String::as_str).collect();
        let results = Plan::new(&loaded.stream).evaluate(&ids, &context);
        let dir = root.join(format!("oracle/models_ml/{case}/data"));
        for uid in &appliers {
            let Some(path) = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| p.to_string_lossy().ends_with(&format!("_{uid}.csv")))
            else {
                continue;
            };
            let Some(Ok(NodeOutput::Data(table))) = results.get(uid) else {
                panic!(
                    "{case} {uid}: the saved nugget did not score: {:?}",
                    results.get(uid)
                );
            };
            let text = std::fs::read_to_string(path).unwrap();
            let mut lines = text.lines().filter(|l| !l.is_empty());
            let header = support::parse_csv_line(lines.next().unwrap());
            let rows: Vec<Vec<String>> = lines.map(support::parse_csv_line).collect();
            assert_eq!(table.rows().len(), rows.len(), "{case} {uid}");
            for (column, name) in header.iter().enumerate() {
                if !name.starts_with('$') {
                    continue;
                }
                let at = table
                    .model()
                    .position(name)
                    .unwrap_or_else(|| panic!("{case} {uid}: no {name}"));
                for (r, (row, expected)) in table.rows().iter().zip(&rows).enumerate() {
                    let expected = expected[column].trim_matches('"');
                    let actual = &row[at];
                    match (expected.parse::<f64>(), actual.as_f64()) {
                        (Ok(b), Some(a)) => assert!(
                            (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0),
                            "{case} {uid} row {r} {name}: {a} vs {b}"
                        ),
                        _ => {
                            let actual = if actual.is_null() {
                                String::new()
                            } else {
                                actual.to_string()
                            };
                            assert_eq!(actual, expected, "{case} {uid} row {r} {name}");
                        }
                    }
                }
            }
            checked += 1;
        }
    }
    assert!(checked >= 15, "only {checked} nugget outputs checked");
    println!("{checked} saved Neural Net nugget outputs score exactly");
}
