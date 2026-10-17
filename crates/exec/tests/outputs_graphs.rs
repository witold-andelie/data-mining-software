//! Oracle tests for the graph nodes: the data behind each graph against the `embeddedSource` of
//! Modeler's VIZML export (`oracle/graphs/*`, and the team streams' graphs).

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions and corpus setup fail loudly"
)]

mod support;

use dms_exec::{Document, DocumentTable};

/// The first `embeddedSource` of a VIZML document as one table (`;`-separated names and rows).
fn parse_vizml(text: &str) -> Document {
    let doc = roxmltree::Document::parse(text).unwrap();
    let source = doc
        .descendants()
        .find(|n| n.has_tag_name("embeddedSource"))
        .unwrap();
    let child_text = |tag: &str| -> Vec<String> {
        source
            .children()
            .filter(|n| n.has_tag_name(tag))
            .map(|n| n.text().unwrap_or_default().to_string())
            .collect()
    };
    let header = child_text("names")[0]
        .split(';')
        .map(str::to_string)
        .collect();
    let rows = child_text("row")
        .iter()
        .map(|r| r.split(';').map(str::to_string).collect())
        .collect();
    Document {
        title: String::new(),
        tables: vec![DocumentTable {
            title: None,
            header,
            rows,
        }],
    }
}

#[test]
fn graph_nodes_match_modelers_graph_data() {
    support::check_oracle_cases(
        &[
            "distribution",
            "histogram",
            "plot",
            "multiplot",
            "timeplot",
            "collection",
        ],
        |_, text| parse_vizml(text),
    );
}
