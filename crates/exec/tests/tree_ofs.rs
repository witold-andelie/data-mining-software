//! QUEST and C&R Tree with an overfit prevention set at statistical parity
//! (`docs/statistical-parity.md`, rules 1, 2 and 5): Modeler draws the held-out records with an
//! undisclosed generator, so we draw them with our own seedable generator. Each oracle stream is
//! trained with 30 seeds (`seed + k`); Modeler's accuracy or RMSE, root `recordCount`, depth and
//! leaf count must lie inside our seeds' range widened by 10 % of its width.
//!
//! The exact tier still holds without an overfit set (`crates/algo/tests/*_oracle.rs`,
//! `models_build.rs`, `quest.rs`) and Modeler's saved nuggets still score exactly.
#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{MiningFunction, Model, Node, Pmml, Usage, Value as PValue};
use dms_stream::{NodeRef, Stream};
use dms_strfmt::ArchiveLimits;
use dms_types::{Table, Value};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const SEEDS: u64 = 30;
const QUEST: &str = "Quest!spss.tree-extension";
const CART: &str = "Cart!spss.tree-extension";

struct Loaded {
    label: String,
    case: String,
    stream: Stream,
    attachments: HashMap<String, Arc<[u8]>>,
    builder: String,
    source: String,
}

impl Loaded {
    fn oracle_dir(&self) -> PathBuf {
        support::repo_root().join(format!("oracle/{}/{}", self.label, self.case))
    }

    fn context(&self, attachments: HashMap<String, Arc<[u8]>>) -> ExecContext {
        ExecContext {
            files: FileResolver {
                data_dirs: vec![support::repo_root().join(format!("corpus/{}", self.label))],
            },
            attachments: Arc::new(attachments),
            stream_properties: self.stream.properties.clone().map(Arc::new),
            decimal_places: Some(15),
            ..ExecContext::default()
        }
    }
}

fn load(label: &str, case: &str, builder_type: &str) -> Loaded {
    let root = support::repo_root();
    let bytes = std::fs::read(root.join(format!("corpus/{label}/{case}.str"))).unwrap();
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|node| node.type_identifier() == builder_type)
        .unwrap_or_else(|| panic!("{case}: no {builder_type}"))
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
        .unwrap_or_else(|| panic!("{case}: no builder input"));
    let attachments = archive
        .attachments
        .into_iter()
        .map(|attachment| (attachment.name, Arc::<[u8]>::from(attachment.bytes)))
        .collect();
    Loaded {
        label: label.to_string(),
        case: case.to_string(),
        stream,
        attachments,
        builder,
        source,
    }
}

/// Replace a `<Parameter name="key" value="…">` value in the extension attachment.
fn with_seed(loaded: &Loaded, key: &str, seed: u64) -> HashMap<String, Arc<[u8]>> {
    let marker = format!("name=\"{key}\"");
    let mut out = loaded.attachments.clone();
    for bytes in out.values_mut() {
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue;
        };
        let Some(name_at) = text.find(&marker) else {
            continue;
        };
        let Some(element_start) = text[..name_at].rfind("<Parameter") else {
            continue;
        };
        let Some(element_end) = text[name_at..].find('>').map(|at| at + name_at) else {
            continue;
        };
        let element = &text[element_start..=element_end];
        let Some(value_at) = element.find("value=\"") else {
            continue;
        };
        let value_start = element_start + value_at + "value=\"".len();
        let Some(value_end) = text[value_start..].find('"').map(|at| at + value_start) else {
            continue;
        };
        let replaced = format!("{}{seed}{}", &text[..value_start], &text[value_end..]);
        *bytes = Arc::from(replaced.into_bytes());
        return out;
    }
    panic!("{}: no {key} parameter", loaded.case);
}

fn train(loaded: &Loaded, key: &str, seed: u64) -> Result<(Pmml, Arc<Table>), String> {
    let context = loaded.context(with_seed(loaded, key, seed));
    let results = Plan::new(&loaded.stream)
        .evaluate(&[loaded.builder.as_str(), loaded.source.as_str()], &context);
    let model = match results.get(loaded.builder.as_str()) {
        Some(Ok(NodeOutput::Model(model))) => (**model).clone(),
        Some(Err(error)) => return Err(format!("{:?}", error.kind)),
        other => return Err(format!("builder: {other:?}")),
    };
    let table = match results.get(loaded.source.as_str()) {
        Some(Ok(NodeOutput::Data(table))) => Arc::clone(table),
        other => return Err(format!("source: {other:?}")),
    };
    Ok((model, table))
}

fn tree(pmml: &Pmml) -> &dms_pmml::TreeModel {
    match &pmml.model {
        Model::Tree(tree) => tree,
        other => panic!("not a tree: {other:?}"),
    }
}

fn depth(node: &Node) -> usize {
    1 + node.children.iter().map(depth).max().unwrap_or(0)
}

fn leaves(node: &Node) -> usize {
    if node.children.is_empty() {
        1
    } else {
        node.children.iter().map(leaves).sum()
    }
}

fn pmml_value(value: &Value) -> PValue {
    match value {
        Value::Null => PValue::Missing,
        // The CSV reader keeps an empty string field as `""`; Modeler scores it as missing and
        // the saved PMML does not list it as a category.
        Value::String(text) if text.trim().is_empty() => PValue::Missing,
        Value::String(text) => PValue::Text(text.to_string()),
        other => other.as_f64().map_or(PValue::Missing, PValue::Number),
    }
}

#[derive(Debug, Clone, Copy)]
struct Metrics {
    quality: f64,
    root: f64,
    depth: usize,
    leaves: usize,
}

/// Accuracy (classification) or RMSE (regression) over every record with a valid target, the root
/// `recordCount`, the number of levels and the number of leaves.
fn metrics(pmml: &Pmml, table: &Table) -> Metrics {
    let model = tree(pmml);
    let target = model
        .header
        .mining_schema
        .iter()
        .find(|field| field.usage == Usage::Target)
        .unwrap_or_else(|| panic!("no target"));
    let at = table
        .model()
        .position(&target.name)
        .unwrap_or_else(|| panic!("no target field {}", target.name));
    let names: Vec<&str> = table
        .model()
        .fields()
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    let scorer = pmml.bind(&names).unwrap();
    let mut correct = 0usize;
    let mut squared = 0.0;
    let mut scored = 0usize;
    for row in table.rows() {
        let actual = &row[at];
        if table.model().fields()[at].is_missing(actual) {
            continue;
        }
        let record: Vec<PValue> = row
            .iter()
            .enumerate()
            .map(|(field, value)| {
                if table.model().fields()[field].is_missing(value) {
                    PValue::Missing
                } else {
                    pmml_value(value)
                }
            })
            .collect();
        let prediction = scorer.score(&record).unwrap_or_else(|error| {
            panic!(
                "{}: row {scored} scores: {error:?}; row {row:?}",
                target.name
            )
        });
        scored += 1;
        match model.header.function {
            MiningFunction::Regression => {
                let expected = actual.as_f64().unwrap_or(f64::NAN);
                let predicted = match prediction.value {
                    PValue::Number(n) => n,
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
    let quality = match model.header.function {
        MiningFunction::Regression => (squared / scored as f64).sqrt(),
        _ => correct as f64 / scored as f64,
    };
    Metrics {
        quality,
        root: model.root.record_count.unwrap_or(0.0),
        depth: depth(&model.root),
        leaves: leaves(&model.root),
    }
}

fn saved_pmml(loaded: &Loaded) -> Pmml {
    let dir = loaded.oracle_dir().join("models");
    let path = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|extension| extension == "xml"))
        .unwrap_or_else(|| panic!("{}: no saved PMML", loaded.case));
    Pmml::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn seed_of(case: &str) -> u64 {
    case.strip_prefix("ofs_s")
        .and_then(|rest| rest.split('_').next())
        .and_then(|seed| seed.parse().ok())
        .unwrap_or_else(|| panic!("{case}: no seed in the name"))
}

fn band(case: &str, metric: &str, modeler: f64, values: &[f64]) {
    let low = values.iter().copied().fold(f64::INFINITY, f64::min);
    let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let margin = 0.1 * (high - low);
    println!(
        "{case} {metric}: Modeler {modeler:.6}, our {SEEDS} seeds {low:.6}..{high:.6} (band {:.6}..{:.6})",
        low - margin,
        high + margin
    );
    assert!(
        modeler >= low - margin && modeler <= high + margin,
        "{case} {metric}: Modeler {modeler} outside our band {low}..{high}"
    );
}

fn check_bands(loaded: &Loaded, key: &str, first_seed: u64) {
    let (_, table) = train(loaded, key, first_seed).unwrap();
    let theirs = metrics(&saved_pmml(loaded), &table);
    let mut runs = Vec::new();
    for k in 0..SEEDS {
        let (pmml, table) = match train(loaded, key, first_seed + k) {
            Ok(trained) => trained,
            Err(error) => panic!("{} seed {}: {error}", loaded.case, first_seed + k),
        };
        runs.push(metrics(&pmml, &table));
    }
    let metric = match tree(&saved_pmml(loaded)).header.function {
        MiningFunction::Regression => "RMSE",
        _ => "accuracy",
    };
    band(
        &loaded.case,
        metric,
        theirs.quality,
        &runs.iter().map(|run| run.quality).collect::<Vec<_>>(),
    );
    band(
        &loaded.case,
        "root recordCount",
        theirs.root,
        &runs.iter().map(|run| run.root).collect::<Vec<_>>(),
    );
    band(
        &loaded.case,
        "depth",
        theirs.depth as f64,
        &runs.iter().map(|run| run.depth as f64).collect::<Vec<_>>(),
    );
    band(
        &loaded.case,
        "leaves",
        theirs.leaves as f64,
        &runs.iter().map(|run| run.leaves as f64).collect::<Vec<_>>(),
    );
}

#[test]
fn quest_with_an_overfit_prevention_set_matches_modeler_in_the_seed_band() {
    for case in ["quest_default", "quest_depth3", "quest_unpruned"] {
        check_bands(&load("models_next", case, QUEST), "Quest_RandomSeed", 12345);
    }
}

#[test]
fn cart_with_an_overfit_prevention_set_matches_modeler_in_the_seed_band() {
    check_bands(
        &load("cart_ofs2", "ofs_cart_s12345_reg48_pct30", CART),
        "Cart_RandomSeed",
        12345,
    );
}

#[test]
fn four_record_cart_overfit_streams_match_modeler_in_the_seed_band() {
    let root = support::repo_root();
    let cases: Vec<String> = std::fs::read_dir(root.join("oracle/cart_ofs3"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|case| root.join(format!("corpus/cart_ofs3/{case}.str")).exists())
        .collect();
    assert!(!cases.is_empty(), "no cart_ofs3 oracles");
    for case in cases {
        check_bands(
            &load("cart_ofs3", &case, CART),
            "Cart_RandomSeed",
            seed_of(&case),
        );
    }
}

/// The held-out records drive pruning: with the same seed the pruned stream is smaller than the
/// unpruned one, so pruning is not just a no-op on the statistical path.
#[test]
fn overfit_set_pruning_cuts_the_grown_tree() {
    let (pruned, _) = train(
        &load("models_next", "quest_default", QUEST),
        "Quest_RandomSeed",
        12345,
    )
    .unwrap();
    let (unpruned, _) = train(
        &load("models_next", "quest_unpruned", QUEST),
        "Quest_RandomSeed",
        12345,
    )
    .unwrap();
    let (pruned, unpruned) = (tree(&pruned), tree(&unpruned));
    println!(
        "seed 12345: pruned {} nodes / depth {} / leaves {}, unpruned {} nodes / depth {} / leaves {}",
        count_nodes(&pruned.root),
        depth(&pruned.root),
        leaves(&pruned.root),
        count_nodes(&unpruned.root),
        depth(&unpruned.root),
        leaves(&unpruned.root)
    );
    assert!(
        leaves(&pruned.root) < leaves(&unpruned.root),
        "pruning did not cut the tree"
    );
}

fn count_nodes(node: &Node) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
}

/// Exact tier: with the builder removed from the stream, every saved QUEST nugget scores its
/// oracle records like Modeler (all `$` columns, numbers to 1e-9).
#[test]
fn saved_quest_nuggets_score_exactly() {
    let mut checked = 0;
    for case in ["quest_default", "quest_depth3", "quest_unpruned"] {
        let mut loaded = load("models_next", case, QUEST);
        let builder = loaded.builder.clone();
        loaded
            .stream
            .diagram
            .nodes
            .retain(|node| node.uid != builder);
        loaded.stream.diagram.links.retain(|link| {
            link.from != NodeRef::Uid(builder.clone()) && link.to != NodeRef::Uid(builder.clone())
        });
        let appliers: Vec<String> = loaded
            .stream
            .diagram
            .nodes
            .iter()
            .filter(|node| node.type_identifier() == "ApplyQuestRuleNode")
            .map(|node| node.uid.clone())
            .collect();
        assert!(!appliers.is_empty(), "{case}: no apply node");
        let context = loaded.context(loaded.attachments.clone());
        let ids: Vec<&str> = appliers.iter().map(String::as_str).collect();
        let results = Plan::new(&loaded.stream).evaluate(&ids, &context);
        let manifest_text =
            std::fs::read_to_string(loaded.oracle_dir().join("manifest.json")).unwrap();
        let manifest: JsonValue = serde_json::from_str(&manifest_text).unwrap();
        for entry in manifest["data"].as_array().unwrap() {
            if entry["type"].as_str() != Some("applyquest") {
                continue;
            }
            let uid = entry["id"].as_str().unwrap();
            let Some(Ok(NodeOutput::Data(table))) = results.get(uid) else {
                panic!("{case} {uid}: the saved nugget did not score");
            };
            let path = support::local_path(entry["file"].as_str().unwrap());
            let text = std::fs::read_to_string(path).unwrap();
            let mut lines = text.lines().filter(|line| !line.is_empty());
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
                for (row, expected) in table.rows().iter().zip(&rows) {
                    let expected = expected[column].trim_matches('"');
                    let actual = &row[at];
                    match (expected.parse::<f64>(), actual.as_f64()) {
                        (Ok(b), Some(a)) => assert!(
                            (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0),
                            "{case} {uid} {name}: {a} vs {b}"
                        ),
                        _ => assert_eq!(actual.to_string(), expected, "{case} {uid} {name}"),
                    }
                }
            }
            checked += 1;
        }
    }
    assert!(checked >= 6, "only {checked} nugget outputs checked");
    println!("{checked} saved QUEST nugget outputs score exactly");
}
