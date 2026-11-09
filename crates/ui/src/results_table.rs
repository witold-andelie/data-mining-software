//! Data tables in the results area. Rows are virtualized: only the rows on screen are laid out
//! and formatted, so a million rows scroll as smoothly as a hundred.

use dms_types::{Table, Value};
use egui::{Align, Layout, RichText, Ui};
use egui_extras::{Column, TableBuilder};
use std::hash::Hash;

/// More columns than this are not shown (the count is reported).
const MAX_COLUMNS: usize = 1000;
const ROW_HEIGHT: f32 = 20.0;

/// Draws `table`; `id_salt` keeps column widths and scroll position per result.
pub fn show_table(ui: &mut Ui, table: &Table, id_salt: impl Hash + std::fmt::Debug) {
    let fields = table.model().fields();
    let rows = table.rows();
    let shown = fields.len().min(MAX_COLUMNS);
    ui.horizontal(|ui| {
        ui.label(format!(
            "{} rows · {} fields",
            group_digits(rows.len()),
            fields.len()
        ));
        if shown < fields.len() {
            ui.weak(format!("(first {shown} fields shown)"));
        }
    });
    if fields.is_empty() {
        return;
    }
    let id = ui.make_persistent_id(id_salt);
    egui::ScrollArea::horizontal()
        .id_salt(id.with("h"))
        .show(ui, |ui| {
            let mut builder = TableBuilder::new(ui)
                .id_salt(id.with("table"))
                .striped(true)
                .resizable(true)
                .cell_layout(Layout::left_to_right(Align::Center))
                .column(Column::auto().at_least(36.0));
            for field in fields.iter().take(shown) {
                let width = (field.name.chars().count() as f32 * 7.5 + 24.0).clamp(60.0, 240.0);
                builder = builder.column(Column::initial(width).at_least(36.0).clip(true));
            }
            builder
                .header(ROW_HEIGHT + 2.0, |mut header| {
                    header.col(|ui| {
                        ui.weak("#");
                    });
                    for field in fields.iter().take(shown) {
                        header.col(|ui| {
                            ui.strong(&field.name).on_hover_text(format!(
                                "{}\nStorage: {:?}\nMeasurement: {:?}\nRole: {:?}",
                                field.label.as_deref().unwrap_or(&field.name),
                                field.storage,
                                field.measurement,
                                field.role
                            ));
                        });
                    }
                })
                .body(|body| {
                    body.rows(ROW_HEIGHT, rows.len(), |mut row| {
                        let index = row.index();
                        let Some(values) = rows.get(index) else {
                            return;
                        };
                        row.col(|ui| {
                            ui.weak(group_digits(index.saturating_add(1)));
                        });
                        for value in values.iter().take(shown) {
                            row.col(|ui| cell(ui, value));
                        }
                    });
                });
        });
}

fn cell(ui: &mut Ui, value: &Value) {
    match value {
        Value::Null => {
            ui.label(RichText::new("$null$").weak().italics());
        }
        Value::Integer(_) | Value::Real(_) => {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(value.to_string());
            });
        }
        _ => {
            ui.label(value.to_string());
        }
    }
}

/// `1234567` as `1 234 567` (thin spaces), readable in any locale.
pub fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{2009}');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::group_digits;

    #[test]
    fn groups_digits_by_three() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1\u{2009}000");
        assert_eq!(group_digits(1_234_567), "1\u{2009}234\u{2009}567");
    }
}
