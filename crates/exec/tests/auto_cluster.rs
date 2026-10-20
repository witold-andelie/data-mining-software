//! Auto Cluster training and scoring against `oracle/models_next2/acluster_default`.
//!
//! (a) K-Means and TwoStep silhouettes and partition stats equal the oracle's exactly;
//! (b) Kohonen's silhouette over 30 seeds holds Modeler's single run inside our widened band;
//! (c) K-Means ranks first for every one of those seeds, and the trained stream reproduces
//! Modeler's scored records; (d) the saved nugget still scores its own component.
#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;

use dms_exec::nodes::auto_cluster::{self, ComponentAlgorithm, TrainedAutoCluster};
use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_stream::{Stream, StreamNode};
use dms_strfmt::ArchiveLimits;
use dms_types::{Table, Value};
use serde_json::Value as Json;
use std::path::Path;
use std::sync::Arc;

const STREAM: &str = "corpus/models_next2/acluster_default.str";
const BUILDER: &str = "id5FF1Z4LQI2MX";
const NUGGET: &str = "id3V511ZJ87MXR";
const ORACLE: &str = "oracle/models_next2/acluster_default";
const KMEANS_SILHOUETTE: f64 = 0.524_354_456_082_289;
const TWOSTEP_SILHOUETTE: f64 = 0.419_943_931_920_704;
const KOHONEN_SILHOUETTE: f64 = 0.400_983_163_795_6;

fn load() -> (Stream, ExecContext) {
    let root = support::repo_root();
    let bytes = std::fs::read(root.join(STREAM)).unwrap();
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let context = ExecContext {
        files: FileResolver {
            data_dirs: vec![
                root.join("corpus/models_next2"),
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
        decimal_places: Some(15),
        ..ExecContext::default()
    };
    (stream, context)
}

fn node<'a>(stream: &'a Stream, uid: &str) -> &'a StreamNode {
    stream
        .diagram
        .nodes
        .iter()
        .find(|node| node.uid == uid)
        .unwrap_or_else(|| panic!("no node {uid}"))
}

/// The records that reach the builder: evaluate its direct upstream node.
fn builder_input(stream: &Stream, context: &ExecContext) -> Arc<Table> {
    let plan = Plan::new(stream);
    let inputs = plan.inputs_of(BUILDER);
    assert_eq!(inputs.len(), 1, "the builder has one input");
    let target = inputs[0];
    let results = plan.evaluate(&[target], context);
    match results.get(target) {
        Some(Ok(NodeOutput::Data(table))) => Arc::clone(table),
        other => panic!("the builder input did not produce data: {other:?}"),
    }
}

fn trained(stream: &Stream, table: &Table) -> TrainedAutoCluster {
    auto_cluster::train(node(stream, BUILDER), table)
        .unwrap_or_else(|error| panic!("training failed: {error:?}"))
}

fn component(
    trained: &TrainedAutoCluster,
    algorithm: ComponentAlgorithm,
) -> &auto_cluster::TrainedComponent {
    trained
        .components
        .iter()
        .find(|component| component.algorithm == algorithm)
        .unwrap_or_else(|| panic!("no {algorithm:?} component"))
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= 1e-12 * actual.abs().max(expected.abs()).max(1.0)
}

fn table_matches_csv(table: &Table, path: &Path) {
    let text = std::fs::read_to_string(path).unwrap();
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    let names = support::parse_csv_line(lines.next().unwrap());
    let fields: Vec<&str> = table
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(fields, names, "{}", path.display());
    let rows: Vec<Vec<String>> = lines.map(support::parse_csv_line).collect();
    assert_eq!(table.rows().len(), rows.len(), "{}", path.display());
    for (r, (actual, expected)) in table.rows().iter().zip(&rows).enumerate() {
        assert_eq!(actual.len(), expected.len(), "{}", path.display());
        for (c, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            let matches = match actual {
                Value::Null => expected.is_empty() || expected == "$null$",
                Value::String(text) => text.as_ref() == expected,
                Value::Integer(_) | Value::Real(_) => {
                    let a = actual.as_f64().unwrap();
                    match expected.parse::<f64>() {
                        Ok(b) => {
                            a.is_finite()
                                && b.is_finite()
                                && (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
                        }
                        Err(_) => false,
                    }
                }
                other => panic!("unexpected value {other:?}"),
            };
            assert!(
                matches,
                "{} row {r} column {c}: {actual:?} != {expected:?}",
                path.display()
            );
        }
    }
}

#[test]
fn kmeans_and_twostep_stats_equal_the_oracle() {
    let (stream, context) = load();
    let input = builder_input(&stream, &context);
    let trained = trained(&stream, &input);

    for component in &trained.components {
        println!(
            "{}: silhouette {:.15} clusters {}",
            component.label,
            component.evaluation.silhouette,
            component.evaluation.num_clusters()
        );
    }
    assert_eq!(trained.components.len(), 3);
    assert!(
        trained
            .components
            .windows(2)
            .all(|pair| pair[0].evaluation.silhouette >= pair[1].evaluation.silhouette)
    );
    assert_eq!(trained.components.iter().filter(|c| c.use_model).count(), 1);
    assert_eq!(
        trained.selected().map(|component| component.algorithm),
        Some(ComponentAlgorithm::KMeans),
        "the best silhouette is K-Means"
    );
    assert_eq!(
        component(&trained, ComponentAlgorithm::KMeans).label,
        "K-means 1"
    );
    assert_eq!(
        component(&trained, ComponentAlgorithm::TwoStep).label,
        "TwoStep 1"
    );
    assert_eq!(
        component(&trained, ComponentAlgorithm::Kohonen).label,
        "Kohonen 1"
    );

    let kmeans = component(&trained, ComponentAlgorithm::KMeans);
    assert!(
        close(kmeans.evaluation.silhouette, KMEANS_SILHOUETTE),
        "{:?}",
        kmeans.evaluation
    );
    assert_eq!(kmeans.evaluation.num_clusters(), 5);
    let sizes: Vec<_> = kmeans
        .evaluation
        .clusters
        .iter()
        .map(|cluster| cluster.size)
        .collect();
    assert_eq!(sizes, [1, 13, 12, 11, 11]);
    let percents: Vec<_> = kmeans
        .evaluation
        .clusters
        .iter()
        .map(|cluster| cluster.percent)
        .collect();
    assert_eq!(percents, [2, 27, 25, 22, 22]);
    assert_eq!(kmeans.evaluation.smallest().map(|c| c.size), Some(1));
    assert_eq!(kmeans.evaluation.smallest().map(|c| c.percent), Some(2));
    assert_eq!(kmeans.evaluation.largest().map(|c| c.size), Some(13));
    assert_eq!(kmeans.evaluation.largest().map(|c| c.percent), Some(27));
    assert!(close(
        kmeans.evaluation.cluster_ratio(),
        0.076_923_076_923_076_93
    ));
    assert_eq!(kmeans.evaluation.num_overlays, 1);
    assert_eq!(kmeans.evaluation.importance, 0.0);

    let twostep = component(&trained, ComponentAlgorithm::TwoStep);
    assert!(
        close(twostep.evaluation.silhouette, TWOSTEP_SILHOUETTE),
        "{:?}",
        twostep.evaluation
    );
    assert_eq!(twostep.evaluation.num_clusters(), 2);
    let sizes: Vec<_> = twostep
        .evaluation
        .clusters
        .iter()
        .map(|cluster| cluster.size)
        .collect();
    assert_eq!(sizes, [31, 17]);
    let percents: Vec<_> = twostep
        .evaluation
        .clusters
        .iter()
        .map(|cluster| cluster.percent)
        .collect();
    assert_eq!(percents, [64, 35]);
    assert_eq!(twostep.evaluation.smallest().map(|c| c.size), Some(17));
    assert_eq!(twostep.evaluation.smallest().map(|c| c.percent), Some(35));
    assert_eq!(twostep.evaluation.largest().map(|c| c.size), Some(31));
    assert_eq!(twostep.evaluation.largest().map(|c| c.percent), Some(64));
    assert!(close(
        twostep.evaluation.cluster_ratio(),
        0.548_387_096_774_193_5
    ));
    assert_eq!(twostep.evaluation.num_overlays, 1);
    assert_eq!(twostep.evaluation.importance, 0.0);
}

#[test]
fn kohonen_seed_band_and_ranking() {
    let (stream, context) = load();
    let input = builder_input(&stream, &context);
    let builder = node(&stream, BUILDER);

    let seeds: Vec<u64> = (0..30).collect();
    let mut silhouettes = Vec::with_capacity(seeds.len());
    for &seed in &seeds {
        let trained = auto_cluster::train_with_kohonen_seed(builder, &input, seed)
            .unwrap_or_else(|error| panic!("training with seed {seed} failed: {error:?}"));
        assert_eq!(
            trained.selected().map(|component| component.algorithm),
            Some(ComponentAlgorithm::KMeans),
            "seed {seed} ranked {:?}",
            trained
                .components
                .iter()
                .map(|component| (component.algorithm, component.evaluation.silhouette))
                .collect::<Vec<_>>()
        );
        let kohonen = component(&trained, ComponentAlgorithm::Kohonen);
        println!(
            "seed {seed}: Kohonen silhouette {:.15}",
            kohonen.evaluation.silhouette
        );
        silhouettes.push(kohonen.evaluation.silhouette);
    }

    let low = silhouettes.iter().copied().fold(f64::INFINITY, f64::min);
    let high = silhouettes
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let widened = (high - low) * 0.10;
    let rank = 1 + silhouettes
        .iter()
        .filter(|&&silhouette| silhouette < KOHONEN_SILHOUETTE)
        .count();
    println!(
        "Modeler {KOHONEN_SILHOUETTE:.15}: ours {low:.15}..{high:.15}, widened {:.15}..{:.15}, rank {rank} of {}",
        low - widened,
        high + widened,
        silhouettes.len()
    );
    assert!(
        KOHONEN_SILHOUETTE >= low - widened && KOHONEN_SILHOUETTE <= high + widened,
        "Modeler's Kohonen silhouette is outside the widened band"
    );
}

#[test]
fn trained_stream_reproduces_the_oracle_scores() {
    let (stream, context) = load();
    let root = support::repo_root();
    let manifest: Json = serde_json::from_str(
        &std::fs::read_to_string(root.join(ORACLE).join("manifest.json")).unwrap(),
    )
    .unwrap();
    let outputs = manifest["outputs"].as_array().unwrap();
    assert_eq!(outputs.len(), 2);

    let plan = Plan::new(&stream);
    let targets: Vec<_> = outputs
        .iter()
        .map(|output| output["id"].as_str().unwrap())
        .collect();
    let results = plan.evaluate(&targets, &context);
    for output in outputs {
        let uid = output["id"].as_str().unwrap();
        let table = match results.get(uid) {
            Some(Ok(NodeOutput::Data(table))) => table,
            other => panic!("{uid}: {other:?}"),
        };
        let file = support::local_path(output["file"].as_str().unwrap());
        table_matches_csv(table, &file);
    }
}

#[test]
fn saved_nugget_still_scores_its_component() {
    let (stream, context) = load();
    let input = builder_input(&stream, &context);
    let operator = auto_cluster::apply(node(&stream, NUGGET)).unwrap();
    let scored = match operator.produce(&[Arc::clone(&input)], None, &context) {
        Ok(NodeOutput::Data(table)) => table,
        other => panic!("the saved nugget did not produce data: {other:?}"),
    };
    let manifest: Json = serde_json::from_str(
        &std::fs::read_to_string(support::repo_root().join(ORACLE).join("manifest.json")).unwrap(),
    )
    .unwrap();
    let file = support::local_path(manifest["outputs"][0]["file"].as_str().unwrap());
    table_matches_csv(&scored, &file);
}
