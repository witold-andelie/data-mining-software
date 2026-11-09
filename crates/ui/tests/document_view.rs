//! Acceptance tests for `dms_ui::document_view::show_document` (statistics and text outputs).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use dms_exec::{Document, DocumentTable};
use dms_ui::document_view::{show_document, table_as_tsv};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn sample() -> Document {
    Document {
        title: "Statistics of příjem".to_string(),
        tables: vec![
            DocumentTable {
                title: Some("Summary".to_string()),
                header: vec!["Statistic".to_string(), "Value".to_string()],
                rows: vec![
                    vec!["Mean".to_string(), "1485.566997852143".to_string()],
                    vec!["Count".to_string(), "200".to_string()],
                ],
            },
            DocumentTable {
                title: None,
                header: vec!["Field".to_string(), "Note".to_string()],
                rows: vec![vec!["věk".to_string(), "ragged".to_string()], vec![]],
            },
        ],
    }
}

#[test]
fn shows_title_section_headers_and_cells() {
    let document = sample();
    let mut harness = Harness::builder()
        .with_size(vec2(600.0, 400.0))
        .build_ui(|ui| show_document(ui, &document));
    harness.run();
    harness.get_by_label("Statistics of příjem");
    harness.get_by_label("Summary");
    harness.get_by_label("Statistic");
    harness.get_by_label("1485.566997852143");
    harness.get_by_label("věk");
}

#[test]
fn empty_document_does_not_fail() {
    let document = Document {
        title: String::new(),
        tables: Vec::new(),
    };
    let mut harness = Harness::builder()
        .with_size(vec2(300.0, 200.0))
        .build_ui(|ui| show_document(ui, &document));
    harness.run();
}

#[test]
fn tsv_has_header_then_rows_and_escapes_tabs_and_newlines() {
    let table = DocumentTable {
        title: Some("T".to_string()),
        header: vec!["a".to_string(), "b".to_string()],
        rows: vec![
            vec!["1".to_string(), "x\ty".to_string()],
            vec!["2".to_string(), "line\nbreak".to_string()],
        ],
    };
    assert_eq!(table_as_tsv(&table), "a\tb\n1\tx y\n2\tline break\n");
}
