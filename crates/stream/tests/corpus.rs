//! Stream model over the real-file corpus (`--features corpus`). Expectations come from the
//! differential probes (`cargo xtask probe str_basics`) and the team streams.
#![cfg(feature = "corpus")]

use dms_stream::{NodeRef, Stream};
use dms_strfmt::{ArchiveLimits, read_archive};
use std::path::Path;

fn load(label: &str, path: &str) -> Stream {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus")
        .join(label)
        .join(path);
    let bytes = std::fs::read(&file)
        .unwrap_or_else(|e| panic!("{}: {e} (run `cargo xtask corpus sync`)", file.display()));
    let archive = read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    Stream::from_document(&archive.document).unwrap()
}

#[test]
fn every_corpus_stream_has_a_model() {
    let manifest = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../xtask/corpus.tsv"),
    )
    .unwrap();
    let mut count = 0;
    for line in manifest.lines().filter(|line| !line.starts_with('#')) {
        let fields: Vec<&str> = line.split('\t').collect();
        if let [label, path, _] = fields.as_slice()
            && path.ends_with(".str")
        {
            let stream = load(label, path);
            for node in stream.all_nodes() {
                assert!(
                    !node.class.is_empty(),
                    "{label}/{path}: node {} without class",
                    node.uid
                );
            }
            for link in &stream.diagram.links {
                assert!(
                    matches!((&link.from, &link.to), (NodeRef::Uid(_), NodeRef::Uid(_))),
                    "{label}/{path}: unresolved link {link:?}"
                );
            }
            count += 1;
        }
    }
    assert!(count >= 72, "only {count} streams");
}

/// Probe vf_17: Var. File (label "Custom Label" at 300,100) linked to a Table.
#[test]
fn probe_link_label_and_position() {
    let stream = load("probes", "vf_17.str");
    let nodes = &stream.diagram.nodes;
    assert_eq!(nodes.len(), 2);
    let source = nodes
        .iter()
        .find(|n| n.type_identifier() == "DelimitedFileImportNode")
        .unwrap();
    let table = nodes
        .iter()
        .find(|n| n.type_identifier() == "TableNode")
        .unwrap();
    assert_eq!(source.label.as_deref(), Some("Custom Label"));
    assert_eq!(source.position, Some((300, 100)));
    assert_eq!(stream.diagram.links.len(), 1);
    assert_eq!(
        stream.diagram.links[0].from,
        NodeRef::Uid(source.uid.clone())
    );
    assert_eq!(stream.diagram.links[0].to, NodeRef::Uid(table.uid.clone()));
    assert_eq!(
        stream.diagram.terminal_order,
        vec![NodeRef::Uid(table.uid.clone())]
    );
}

#[test]
fn team_streams_expose_extensions_nuggets_and_supernodes() {
    let linear = load("team", "linear_regression.str");
    let identifiers: Vec<String> = linear
        .all_nodes()
        .iter()
        .map(|n| n.type_identifier())
        .collect();
    assert!(
        identifiers
            .iter()
            .any(|id| id == "LinearRegression!spss.alm-extension")
    );
    assert!(
        linear
            .all_nodes()
            .iter()
            .any(|n| n.model_builder.as_deref() == Some("LinearRegression!spss.alm-extension"))
    );

    let cluster = load("team", "cluster_analysis.str");
    let supernode = cluster
        .all_nodes()
        .into_iter()
        .find(|n| n.type_identifier() == "MutableSuperNode")
        .unwrap();
    assert!(
        supernode
            .subdiagram
            .as_ref()
            .is_some_and(|d| !d.nodes.is_empty())
    );
}
