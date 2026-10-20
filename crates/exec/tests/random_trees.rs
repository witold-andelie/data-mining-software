//! Random Trees training at **statistical parity** (`docs/statistical-parity.md`, rules 1, 2 and
//! 5): Modeler's bootstrap generator and the hash that seeds its predictor subsets are
//! undisclosed, so our model is compared by accuracy or RMSE and by the number of trees kept by
//! the accuracy stop rule, never tree by tree.
//!
//! For every `models_as/rt_*` and `models_next/rt_*` oracle, 30 seeds of our generator train the
//! stream's own settings. Modeler's accuracy (classification) or RMSE (regression) and its number
//! of trees must lie inside our seeds' range widened by 10 % of its width; the ranks of Modeler's
//! values among our 30 runs must average between 6 and 24 of 30. The saved nuggets are also scored
//! with the builder removed from the stream: their `$R-`/`$RC-` columns equal Modeler's exports.

#![cfg(feature = "corpus")]
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "oracle assertions fail loudly"
)]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{MiningFunction, Model, Pmml, Usage, Value as PValue};
use dms_stream::{NodeRef, Stream};
use dms_strfmt::ArchiveLimits;
use dms_strfmt::fastinfoset::{Element, Node as XmlNode, QualifiedName};
use dms_types::{Table, Value};
use serde_json::Value as Json;
use std::collections::HashMap;
use std::sync::Arc;

/// Seeds `1..=30`; the acceptance rules require at least ten.
const SEEDS: std::ops::RangeInclusive<u64> = 1..=30;
/// Every oracle that trains Random Trees; `models_next` has saved models but no score exports.
const ORACLES: &[(&str, &str)] = &[
    ("models_as", "rt_default"),
    ("models_as", "rt_fewer"),
    ("models_as", "rt_batch"),
    ("models_as", "rt_conf"),
    ("models_as", "rt_native"),
    ("models_as", "rt_noconf"),
    ("models_as", "rt_numeric"),
    ("models_as", "rt_numeric_batch"),
    ("models_as", "rt_shallow"),
    ("models_next", "rt_default"),
    ("models_next", "rt_fewer"),
    ("models_next", "rt_flag"),
];

/// The nine `models_as` streams whose apply nodes were exported as scored records.
const SCORED_ORACLES: &[&str] = &[
    "rt_default",
    "rt_fewer",
    "rt_batch",
    "rt_conf",
    "rt_native",
    "rt_noconf",
    "rt_numeric",
    "rt_numeric_batch",
    "rt_shallow",
];

type Loaded = (Stream, HashMap<String, Arc<[u8]>>, String, String);

/// The stream, its attachments, the Random Trees builder and the builder's upstream data node.
fn load(label: &str, case: &str) -> Loaded {
    let root = support::repo_root();
    let bytes = std::fs::read(root.join(format!("corpus/{label}/{case}.str"))).unwrap();
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|node| node.type_identifier() == "spss.randomTrees!RT")
        .unwrap()
        .uid
        .clone();
    let source = stream
        .diagram
        .links
        .iter()
        .find(|link| link.to == NodeRef::Uid(builder.clone()))
        .and_then(|link| match &link.from {
            NodeRef::Uid(uid) => Some(uid.clone()),
            NodeRef::Unresolved(_) => None,
        })
        .unwrap();
    let attachments = archive
        .attachments
        .into_iter()
        .map(|a| (a.name, Arc::<[u8]>::from(a.bytes)))
        .collect();
    (stream, attachments, builder, source)
}

fn context(label: &str, stream: &Stream, attachments: &HashMap<String, Arc<[u8]>>) -> ExecContext {
    ExecContext {
        files: FileResolver {
            data_dirs: vec![support::repo_root().join(format!("corpus/{label}"))],
        },
        attachments: Arc::new(attachments.clone()),
        stream_properties: stream.properties.clone().map(Arc::new),
        decimal_places: Some(15),
        ..ExecContext::default()
    }
}

/// The stream with `_seed` set to our seed; Modeler has no such setting.
fn with_seed(stream: &Stream, seed: u64) -> Stream {
    let mut seeded = stream.clone();
    let node = seeded
        .diagram
        .nodes
        .iter_mut()
        .find(|node| node.type_identifier() == "spss.randomTrees!RT")
        .unwrap();
    let settings = node.settings.as_mut().unwrap();
    let name = QualifiedName {
        prefix: None,
        namespace: None,
        local: Arc::from("_seed"),
    };
    let text = XmlNode::Text(Arc::from(seed.to_string()));
    if let Some(existing) = settings.children.iter_mut().find_map(|node| match node {
        XmlNode::Element(element) if &*element.name.local == "_seed" => Some(element),
        _ => None,
    }) {
        existing.children = vec![text];
    } else {
        settings.children.push(XmlNode::Element(Element {
            name,
            namespace_declarations: Vec::new(),
            attributes: Vec::new(),
            children: vec![text],
        }));
    }
    seeded
}

/// Trains one seed and returns the model and the stream's source records.
fn train(label: &str, case: &str, seed: u64) -> (Pmml, Arc<Table>) {
    let (stream, attachments, builder, source) = load(label, case);
    let seeded = with_seed(&stream, seed);
    let results = Plan::new(&seeded).evaluate(
        &[builder.as_str(), source.as_str()],
        &context(label, &seeded, &attachments),
    );
    let model = match results.get(builder.as_str()) {
        Some(Ok(NodeOutput::Model(model))) => (**model).clone(),
        other => panic!("{label}/{case} seed {seed}: builder {other:?}"),
    };
    let table = match results.get(source.as_str()) {
        Some(Ok(NodeOutput::Data(table))) => Arc::clone(table),
        other => panic!("{label}/{case}: source {other:?}"),
    };
    (model, table)
}

fn saved(label: &str, case: &str) -> Pmml {
    let dir = support::repo_root().join(format!("oracle/{label}/{case}/models"));
    let path = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|extension| extension == "xml"))
        .unwrap();
    Pmml::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn mining(pmml: &Pmml) -> &dms_pmml::MiningModel {
    match &pmml.model {
        Model::Mining(model) => model,
        other => panic!("expected a MiningModel, got {other:?}"),
    }
}

fn tree_count(pmml: &Pmml) -> f64 {
    mining(pmml).segments.len() as f64
}

fn target_name(pmml: &Pmml) -> String {
    mining(pmml)
        .header
        .mining_schema
        .iter()
        .find(|field| field.usage == Usage::Target)
        .map(|field| field.name.clone())
        .unwrap_or_else(|| panic!("no target field"))
}

fn function(pmml: &Pmml) -> MiningFunction {
    mining(pmml).header.function.clone()
}

fn pmml_value(value: &Value) -> PValue {
    match value {
        Value::Null => PValue::Missing,
        Value::String(text) if text.trim().is_empty() => PValue::Missing,
        Value::String(text) => PValue::Text(text.to_string()),
        other => other.as_f64().map_or(PValue::Missing, PValue::Number),
    }
}

/// Accuracy or RMSE of a PMML model on `table`'s records (the oracle's scored records).
fn metric(pmml: &Pmml, table: &Table) -> f64 {
    let model = mining(pmml);
    let target = target_name(pmml);
    let at = table.model().position(&target).unwrap();
    let names: Vec<&str> = table
        .model()
        .fields()
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    let scorer = pmml.bind(&names).unwrap();
    let mut correct = 0usize;
    let mut squared = 0.0;
    let mut n = 0usize;
    for row in table.rows() {
        let actual = &row[at];
        if table.model().fields()[at].is_missing(actual) {
            continue;
        }
        let record: Vec<PValue> = row
            .iter()
            .enumerate()
            .map(|(position, value)| {
                if table.model().fields()[position].is_missing(value) {
                    PValue::Missing
                } else {
                    pmml_value(value)
                }
            })
            .collect();
        let prediction = scorer
            .score(&record)
            .unwrap_or_else(|error| panic!("{error:?}"));
        n += 1;
        match model.header.function {
            MiningFunction::Regression => {
                let expected = actual.as_f64().unwrap_or(f64::NAN);
                let predicted = match prediction.value {
                    PValue::Number(value) => value,
                    _ => f64::NAN,
                };
                squared += (expected - predicted) * (expected - predicted);
            }
            _ => {
                let predicted = match prediction.value {
                    PValue::Text(text) => text,
                    PValue::Number(number) => number.to_string(),
                    PValue::Missing => String::new(),
                };
                if predicted == actual.to_string() {
                    correct += 1;
                }
            }
        }
    }
    match model.header.function {
        MiningFunction::Regression => (squared / n as f64).sqrt(),
        _ => correct as f64 / n as f64,
    }
}

/// Modeler's own Train scores export, when the manifest has one. Its rows are the records the
/// type node fed to the apply node, so the target and `$R-` columns give Modeler's metric.
fn modeler_csv_metric(label: &str, case: &str, pmml: &Pmml) -> Option<f64> {
    let root = support::repo_root();
    let manifest: Json = serde_json::from_str(
        &std::fs::read_to_string(root.join(format!("oracle/{label}/{case}/manifest.json")))
            .unwrap(),
    )
    .unwrap();
    let path = manifest["outputs"]
        .as_array()?
        .iter()
        .find(|entry| entry["label"] == "Train scores")?
        .get("file")?
        .as_str()?;
    let text = std::fs::read_to_string(support::local_path(path)).unwrap();
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    let header = support::parse_csv_line(lines.next()?);
    let target = target_name(pmml);
    let actual_at = header.iter().position(|name| *name == target)?;
    let predicted_at = header
        .iter()
        .position(|name| name.strip_prefix("$R-") == Some(target.as_str()))?;
    let regression = function(pmml) == MiningFunction::Regression;
    let mut correct = 0usize;
    let mut squared = 0.0;
    let mut n = 0usize;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let cells = support::parse_csv_line(line);
        let actual = cells.get(actual_at)?;
        let predicted = cells.get(predicted_at)?;
        if actual.is_empty() || actual == "$null$" {
            continue;
        }
        n += 1;
        if regression {
            let expected = actual.parse::<f64>().ok()?;
            let predicted = predicted.parse::<f64>().ok()?;
            squared += (expected - predicted) * (expected - predicted);
        } else if predicted.trim_matches('"') == actual.trim_matches('"') {
            correct += 1;
        }
    }
    (n > 0).then(|| {
        if regression {
            (squared / n as f64).sqrt()
        } else {
            correct as f64 / n as f64
        }
    })
}

fn range(values: &[f64]) -> (f64, f64) {
    values
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), &value| {
            (low.min(value), high.max(value))
        })
}

fn widened(range: (f64, f64)) -> (f64, f64) {
    let padding = 0.1 * (range.1 - range.0);
    (range.0 - padding, range.1 + padding)
}

fn inside_band(range: (f64, f64), modeler: f64) -> bool {
    let (low, high) = widened(range);
    if range.1 == range.0 {
        return modeler == range.0;
    }
    modeler >= low && modeler <= high
}

/// Midrank of `value` among `values`, 1-based; ties share the middle.
fn midrank(value: f64, values: &[f64]) -> f64 {
    let less = values
        .iter()
        .filter(|&&other| other < value - 1e-12)
        .count();
    let equal = values
        .iter()
        .filter(|&&other| (other - value).abs() <= 1e-12)
        .count();
    0.5 + less as f64 + equal as f64 / 2.0
}

#[test]
fn random_trees_statistical_parity_band() {
    let mut metric_ranks = Vec::new();
    let mut count_ranks = Vec::new();
    for (label, case) in ORACLES {
        let oracle = saved(label, case);
        let modeler_trees = tree_count(&oracle);
        let (_, table) = train(label, case, 1);
        let saved_metric = metric(&oracle, &table);
        let modeler_metric = match modeler_csv_metric(label, case, &oracle) {
            Some(exported) => {
                assert!(
                    (exported - saved_metric).abs() <= 1e-9 * exported.abs().max(1.0),
                    "{label}/{case}: Train scores metric {exported} != saved nugget {saved_metric}"
                );
                exported
            }
            None => saved_metric,
        };
        let mut counts = Vec::new();
        let mut metrics = Vec::new();
        for seed in SEEDS {
            let (pmml, _) = train(label, case, seed);
            counts.push(tree_count(&pmml));
            metrics.push(metric(&pmml, &table));
        }
        let count_range = range(&counts);
        let metric_range = range(&metrics);
        let count_rank = midrank(modeler_trees, &counts);
        let metric_rank = midrank(modeler_metric, &metrics);
        assert!(
            inside_band(count_range, modeler_trees),
            "{label}/{case}: Modeler keeps {modeler_trees} trees, outside our band {:?}",
            widened(count_range)
        );
        assert!(
            inside_band(metric_range, modeler_metric),
            "{label}/{case}: Modeler's metric {modeler_metric} is outside our band {:?}",
            widened(metric_range)
        );
        metric_ranks.push(metric_rank);
        count_ranks.push(count_rank);
        let (count_low, count_high) = widened(count_range);
        let (metric_low, metric_high) = widened(metric_range);
        println!(
            "{label}/{case}: {:?} Modeler trees {modeler_trees}, ours {:.0}..{:.0} \
             (accepted {count_low:.1}..{count_high:.1}) rank {count_rank:.1}; Modeler metric \
             {modeler_metric:.6}, ours {:.6}..{:.6} (accepted {metric_low:.6}..{metric_high:.6}) \
             rank {metric_rank:.1}",
            function(&oracle),
            count_range.0,
            count_range.1,
            metric_range.0,
            metric_range.1,
        );
    }
    let mean_metric_rank = metric_ranks.iter().sum::<f64>() / metric_ranks.len() as f64;
    let mean_count_rank = count_ranks.iter().sum::<f64>() / count_ranks.len() as f64;
    println!(
        "mean rank over {} oracles: metric {mean_metric_rank:.2}, trees {mean_count_rank:.2} of {}",
        metric_ranks.len(),
        SEEDS.end()
    );
    assert!(
        (6.0..=24.0).contains(&mean_metric_rank),
        "Modeler's metric rank is at an extreme: {mean_metric_rank:.2}"
    );
    assert!(
        (6.0..=24.0).contains(&mean_count_rank),
        "Modeler's tree count rank is at an extreme: {mean_count_rank:.2}"
    );
}

/// Exact saved nuggets: remove the builder, score the stream's apply nodes and compare every
/// `$R-`/`$RC-` cell with Modeler's export. The builder is only a trainer; the apply nodes carry
/// their own saved PMML, so their scores must not change.
#[test]
fn saved_random_trees_nuggets_score_exactly() {
    let root = support::repo_root();
    for case in SCORED_ORACLES {
        let (mut stream, attachments, builder, _) = load("models_as", case);
        stream.diagram.nodes.retain(|node| node.uid != builder);
        stream.diagram.links.retain(|link| {
            link.from != NodeRef::Uid(builder.clone()) && link.to != NodeRef::Uid(builder.clone())
        });
        let appliers: Vec<String> = stream
            .diagram
            .nodes
            .iter()
            .filter(|node| node.type_identifier() == "spss.randomTrees!applyRandomTrees")
            .map(|node| node.uid.clone())
            .collect();
        assert!(!appliers.is_empty(), "{case}: no apply node");
        let stream_context = context("models_as", &stream, &attachments);
        let ids: Vec<&str> = appliers.iter().map(String::as_str).collect();
        let results = Plan::new(&stream).evaluate(&ids, &stream_context);
        let data_dir = root.join(format!("oracle/models_as/{case}/data"));
        let mut scored = 0usize;
        for uid in &appliers {
            let Some(Ok(NodeOutput::Data(table))) = results.get(uid.as_str()) else {
                panic!("{case} {uid}: the saved nugget did not score");
            };
            let path = std::fs::read_dir(&data_dir)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| path.to_string_lossy().ends_with(&format!("_{uid}.csv")))
                .unwrap_or_else(|| panic!("{case} {uid}: no oracle export"));
            let text = std::fs::read_to_string(path).unwrap();
            let mut lines = text.lines().filter(|line| !line.is_empty());
            let header = support::parse_csv_line(lines.next().unwrap());
            let rows: Vec<_> = lines.map(support::parse_csv_line).collect();
            assert_eq!(table.rows().len(), rows.len(), "{case} {uid}: row count");
            let mut checked = 0usize;
            for (column, name) in header.iter().enumerate() {
                if !name.starts_with("$R") {
                    continue;
                }
                let at = table
                    .model()
                    .position(name)
                    .unwrap_or_else(|| panic!("{case} {uid}: no {name}"));
                for (row, expected) in table.rows().iter().zip(&rows) {
                    let actual = &row[at];
                    let expected = expected.get(column).map(String::as_str).unwrap_or("");
                    match actual {
                        Value::String(text) => {
                            assert_eq!(text.as_ref(), expected, "{case} {uid} {name}: text score")
                        }
                        Value::Integer(_) | Value::Real(_) => {
                            let actual = actual.as_f64().unwrap();
                            let expected = expected
                                .parse::<f64>()
                                .unwrap_or_else(|_| panic!("{case} {uid} {name}: {expected:?}"));
                            assert!(
                                (actual - expected).abs()
                                    <= 1e-12 * actual.abs().max(expected.abs()).max(1.0),
                                "{case} {uid} {name}: {actual} != {expected}"
                            );
                        }
                        other => panic!("{case} {uid} {name}: unexpected {other:?}"),
                    }
                    checked += 1;
                }
            }
            assert!(checked > 0, "{case} {uid}: no score cells compared");
            scored += 1;
            println!(
                "{case} {uid}: {} rows, {checked} score cells exact",
                table.rows().len()
            );
        }
        assert_eq!(
            scored,
            appliers.len(),
            "{case}: not every saved nugget scored"
        );
    }
}
