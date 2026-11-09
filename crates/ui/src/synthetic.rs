//! Synthetic streams for screenshot tests, performance checks and demos. Only our own node
//! labels and layouts: no IBM or team data. Labels include Czech diacritics on purpose.

use dms_stream::{Diagram, Link, NodeRef, Stream, StreamNode};

/// Node types cycled through, as (`.str` class, label). The last one is a catalog node we have
/// not implemented, so its "not available" styling shows up.
const KINDS: &[(&str, &str)] = &[
    ("DelimitedFileImportNode", "zákazníci.csv"),
    ("TypeNode", "Typy polí"),
    ("SelectNode", "Výběr: věk ≥ 18"),
    ("DeriveNode", "Odvození příjmu"),
    ("BuildC5RuleNode", "Strom C5.0"),
    ("TableNode", "Tabulka"),
    ("PartitionNode", "Rozdělení"),
    ("HistogramNode", "Histogram"),
    ("BuildLogisticNode", "Logistic"),
    ("AnalysisNode", "Analysis"),
    ("TextFileExportNode", "export.txt"),
    ("ExtensionImportNode", "Extension import"),
];

const COLUMN_STEP: i64 = 130;
const ROW_STEP: i64 = 120;

/// `count` nodes in rows of six (see [`grid`]).
pub fn stream(count: usize) -> Stream {
    grid(count, 6)
}

/// `count` nodes in rows of `columns`: each row is a chain, and each row's first node is fed by
/// the previous row's first node.
pub fn grid(count: usize, columns: usize) -> Stream {
    let columns = columns.max(1);
    let mut diagram = Diagram::default();
    for i in 0..count {
        let (class, label) = KINDS[i % KINDS.len()];
        let column = (i % columns) as i64;
        let row = (i / columns) as i64;
        diagram.nodes.push(StreamNode {
            uid: uid(i),
            class: format!("com.example.{class}"),
            extension: None,
            model_builder: None,
            name: None,
            label: Some(label.to_string()),
            position: Some((60 + column * COLUMN_STEP, 60 + row * ROW_STEP)),
            properties_attachment: None,
            containers: Vec::new(),
            settings: None,
            binning_output: None,
            result_items: Vec::new(),
            subdiagram: None,
        });
        let from = if i % columns != 0 {
            Some(i - 1)
        } else if i >= columns {
            Some(i - columns)
        } else {
            None
        };
        if let Some(from) = from {
            diagram.links.push(Link {
                from: NodeRef::Uid(uid(from)),
                to: NodeRef::Uid(uid(i)),
            });
        }
    }
    Stream {
        file_version: None,
        save_version: None,
        properties: None,
        diagram,
    }
}

fn uid(i: usize) -> String {
    format!("synthetic{i:05}")
}
