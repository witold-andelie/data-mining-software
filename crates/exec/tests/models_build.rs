//! Build nodes end to end: run each `models/cart_*` and `models/chaid_*` probe stream's build node (settings from its
//! attachment, fields from the Type roles, types from the Type node) and compare the trained tree
//! with the PMML Modeler exported for that stream.

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

mod support;

use dms_exec::NodeOutput;
use dms_pmml::{Model, Node, Pmml};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::collections::HashMap;
use std::sync::Arc;

/// Cases whose tree must equal Modeler's (the algorithm tests list the GAPs).
const DONE: &[&str] = &[
    "chaid_default",
    "chaid_depth2",
    "chaid_exhaustive",
    "chaid_alpha",
    "chaid_regression",
    "cart_default",
    "cart_depth3",
    "cart_unpruned",
    "cart_min_records",
    "cart_no_surrogates",
    "cart_twoing",
    "cart_surrogates",
];

fn root(pmml: &Pmml) -> &Node {
    match &pmml.model {
        Model::Tree(tree) => &tree.root,
        other => panic!("not a tree: {other:?}"),
    }
}

/// The first node (depth first) whose id, record count, score or child count differ.
fn first_difference(ours: &Node, theirs: &Node) -> Option<String> {
    let same_score = match (ours.score.as_deref(), theirs.score.as_deref()) {
        (Some(a), Some(b)) => {
            a == b
                || matches!((a.parse::<f64>(), b.parse::<f64>()),
                    (Ok(x), Ok(y)) if (x - y).abs() <= 1e-9 * x.abs().max(1.0))
        }
        (a, b) => a == b,
    };
    if ours.id != theirs.id
        || ours.record_count != theirs.record_count
        || !same_score
        || ours.children.len() != theirs.children.len()
    {
        return Some(format!(
            "node {:?}: ours id {:?} records {:?} score {:?} children {}; Modeler records {:?} score {:?} children {}",
            theirs.id,
            ours.id,
            ours.record_count,
            ours.score,
            ours.children.len(),
            theirs.record_count,
            theirs.score,
            theirs.children.len()
        ));
    }
    ours.children
        .iter()
        .zip(&theirs.children)
        .find_map(|(a, b)| first_difference(a, b))
}

#[test]
fn tree_build_nodes_train_modelers_trees() {
    let root_dir = support::repo_root();
    let mut failures = Vec::new();
    for case in [
        "cart_default",
        "cart_depth3",
        "cart_unpruned",
        "cart_min_records",
        "cart_regression",
        "cart_surrogates",
        "cart_no_surrogates",
        "cart_twoing",
        "chaid_default",
        "chaid_depth2",
        "chaid_exhaustive",
        "chaid_alpha",
        "chaid_regression",
    ] {
        let bytes = std::fs::read(root_dir.join(format!("corpus/models/{case}.str"))).unwrap();
        let archive =
            dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
        let stream = Stream::from_document(&archive.document).unwrap();
        let builder = stream
            .diagram
            .nodes
            .iter()
            .find(|node| node.type_identifier().ends_with("!spss.tree-extension"))
            .map(|node| node.uid.clone())
            .unwrap();
        let attachments: HashMap<String, Arc<[u8]>> = archive
            .attachments
            .into_iter()
            .map(|a| (a.name, a.bytes.into()))
            .collect();
        let context = dms_exec::ExecContext {
            cancel: Default::default(),
            files: dms_exec::FileResolver {
                data_dirs: vec![root_dir.join("corpus/models")],
            },
            stream_properties: stream.properties.clone().map(Arc::new),
            attachments: Arc::new(attachments),
            decimal_places: Some(15),
            export_dir: None,
        };
        let results = dms_exec::Plan::new(&stream).evaluate(&[builder.as_str()], &context);
        let model = match results.get(builder.as_str()).unwrap() {
            Ok(NodeOutput::Model(model)) => Arc::clone(model),
            other => panic!("{case}: {other:?}"),
        };
        let xml_dir = root_dir.join(format!("oracle/models/{case}/models"));
        let xml = std::fs::read_dir(&xml_dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "xml"))
            .unwrap();
        let oracle = Pmml::parse(&std::fs::read_to_string(xml).unwrap()).unwrap();
        let difference = first_difference(root(&model), root(&oracle));
        println!("{case}: {}", difference.as_deref().unwrap_or("tree equal"));
        if DONE.contains(&case) && difference.is_some() {
            failures.push(case);
        }
    }
    assert!(
        failures.is_empty(),
        "build nodes differ from Modeler: {failures:?}"
    );
}
