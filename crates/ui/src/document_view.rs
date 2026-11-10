//! Statistics and text outputs (`dms_exec::Document`): a title and titled tables.

use crate::i18n::tr;
use dms_exec::{Document, DocumentTable};
use egui::Ui;

/// Draws `document` into `ui`: the title as a heading (skipped when empty), then each table: its
/// title (when present) in strong text, a tr("Copy") button that puts `table_as_tsv(table)` on the
/// clipboard, and the table itself as a striped `egui::Grid` with the header row in strong text
/// and one row per `rows` entry. Rows may be shorter or longer than the header: only the cells
/// they have are shown, nothing is indexed out of range. Cells that parse as a number are
/// right-aligned, others left-aligned. Everything sits inside one `egui::ScrollArea::both()`, so
/// wide or long documents scroll. Each Grid gets a distinct id
/// (`ui.scope_id().with(("document_table", index))` — egui 0.37 renamed `Ui::id`).
pub fn show_document(ui: &mut Ui, document: &Document) {
    egui::ScrollArea::both().show(ui, |ui| {
        if !document.title.is_empty() {
            ui.heading(document.title.as_str());
        }
        for (index, table) in document.tables.iter().enumerate() {
            if let Some(title) = &table.title {
                ui.strong(title.as_str());
            }
            if ui.button(tr("Copy")).clicked() {
                ui.ctx().copy_text(table_as_tsv(table));
            }
            egui::Grid::new(ui.scope_id().with(("document_table", index)))
                .striped(true)
                .show(ui, |ui| {
                    for cell in &table.header {
                        show_cell(ui, cell, true);
                    }
                    ui.end_row();
                    for row in &table.rows {
                        for cell in row {
                            show_cell(ui, cell, false);
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(8.0);
        }
    });
}

/// One grid cell: strong for header cells, otherwise a plain label. Numeric text is
/// right-aligned, everything else left-aligned.
fn show_cell(ui: &mut Ui, text: &str, strong: bool) {
    let draw = |ui: &mut Ui| {
        if strong {
            ui.strong(text);
        } else {
            ui.label(text);
        }
    };
    if text.parse::<f64>().is_ok() {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), draw);
    } else {
        draw(ui);
    }
}

/// The table as tab-separated text: the header line, then one line per row, each line ending in
/// "\n". Tabs and newlines ("\n", "\r") inside a cell become single spaces.
pub fn table_as_tsv(table: &DocumentTable) -> String {
    let mut tsv = String::new();
    push_line(&mut tsv, table.header.iter());
    for row in &table.rows {
        push_line(&mut tsv, row.iter());
    }
    tsv
}

/// Appends one tab-separated line (the cells, then "\n").
fn push_line<'a>(out: &mut String, cells: impl Iterator<Item = &'a String>) {
    for (index, cell) in cells.enumerate() {
        if index > 0 {
            out.push('\t');
        }
        out.push_str(&cell.replace(['\t', '\n', '\r'], " "));
    }
    out.push('\n');
}
