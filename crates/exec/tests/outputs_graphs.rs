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

/// Every `embeddedSource` of a VIZML document as a table (`;`-separated names and rows).
fn parse_vizml(text: &str) -> Document {
    let doc = roxmltree::Document::parse(text).unwrap();
    let tables = doc
        .descendants()
        .filter(|n| n.has_tag_name("embeddedSource"))
        .map(|source| {
            let child_text = |tag: &str| -> Vec<String> {
                source
                    .children()
                    .filter(|n| n.has_tag_name(tag))
                    .map(|n| n.text().unwrap_or_default().to_string())
                    .collect()
            };
            DocumentTable {
                title: None,
                header: child_text("names")[0]
                    .split(';')
                    .map(str::to_string)
                    .collect(),
                rows: child_text("row")
                    .iter()
                    .map(|r| r.split(';').map(str::to_string).collect())
                    .collect(),
            }
        })
        .collect();
    Document {
        title: String::new(),
        tables,
    }
}

/// Web links come in an order we do not reproduce (`nodes::web`): put Modeler's in ours, by
/// the position of their FROM and TO nodes in the node table.
fn web_links_in_node_order(mut doc: Document) -> Document {
    if doc.tables.len() == 2 {
        let nodes: Vec<String> = doc.tables[0].rows.iter().map(|r| r[0].clone()).collect();
        let index = |name: &str| nodes.iter().position(|n| n == name).unwrap_or(usize::MAX);
        doc.tables[1]
            .rows
            .sort_by_key(|r| (index(&r[0]), index(&r[1])));
    }
    doc
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
            "web",
            "evaluation",
        ],
        |_, text| web_links_in_node_order(parse_vizml(text)),
    );
}
