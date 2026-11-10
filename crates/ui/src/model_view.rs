//! A trained model in the results area: its kind, target and inputs, then the sections of
//! `dms_pmml::summary` (facts, the tree as indented text, rule and coefficient tables).

use crate::i18n::{tr, tr_fmt};
use dms_pmml::Pmml;
use dms_pmml::summary::{ModelSummary, SectionBody, summarize};
use egui::{RichText, TextStyle, Ui};

/// Inputs listed by name before the rest is summarized as a count.
const MAX_LISTED_INPUTS: usize = 40;

pub fn show_model(ui: &mut Ui, model: &Pmml) {
    show_summary(ui, &summarize(model));
}

pub fn show_summary(ui: &mut Ui, summary: &ModelSummary) {
    egui::ScrollArea::both()
        .id_salt("model_view")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.heading(&summary.kind);
            egui::Grid::new("model_header")
                .num_columns(2)
                .show(ui, |ui| {
                    if let Some(name) = &summary.name {
                        ui.weak(tr("Name"));
                        ui.label(name);
                        ui.end_row();
                    }
                    if let Some(function) = &summary.function {
                        ui.weak(tr("Function"));
                        ui.label(function);
                        ui.end_row();
                    }
                    if !summary.targets.is_empty() {
                        ui.weak(tr("Target"));
                        ui.label(summary.targets.join(", "));
                        ui.end_row();
                    }
                    if !summary.inputs.is_empty() {
                        ui.weak(tr("Inputs"));
                        let listed: Vec<&str> = summary
                            .inputs
                            .iter()
                            .take(MAX_LISTED_INPUTS)
                            .map(String::as_str)
                            .collect();
                        let more = summary.inputs.len().saturating_sub(listed.len());
                        let mut text = listed.join(", ");
                        if more > 0 {
                            text.push_str(&tr_fmt(
                                " and {more} more",
                                &[("more", &more.to_string())],
                            ));
                        }
                        ui.add(egui::Label::new(text).wrap());
                        ui.end_row();
                    }
                });
            for (index, section) in summary.sections.iter().enumerate() {
                ui.add_space(10.0);
                ui.label(RichText::new(&section.heading).strong());
                match &section.body {
                    SectionBody::Facts(facts) => {
                        egui::Grid::new(("model_facts", index))
                            .num_columns(2)
                            .show(ui, |ui| {
                                for (label, value) in facts {
                                    ui.weak(label);
                                    ui.label(value);
                                    ui.end_row();
                                }
                            });
                    }
                    SectionBody::Lines(lines) => {
                        let row_height = ui.text_style_height(&TextStyle::Monospace);
                        egui::ScrollArea::vertical()
                            .id_salt(("model_lines", index))
                            .max_height(420.0)
                            .auto_shrink([false, true])
                            .show_rows(ui, row_height, lines.len(), |ui, range| {
                                for line in lines.get(range).unwrap_or_default() {
                                    ui.monospace(line);
                                }
                            });
                    }
                    SectionBody::Table { header, rows } => {
                        egui::Grid::new(("model_table", index))
                            .striped(true)
                            .show(ui, |ui| {
                                for cell in header {
                                    ui.strong(cell);
                                }
                                ui.end_row();
                                for row in rows {
                                    for cell in row {
                                        ui.label(cell);
                                    }
                                    ui.end_row();
                                }
                            });
                    }
                    SectionBody::Truncated { shown, total } => {
                        ui.weak(tr_fmt(
                            "Showing {shown} of {total}.",
                            &[("shown", &shown.to_string()), ("total", &total.to_string())],
                        ));
                    }
                }
            }
        });
}
