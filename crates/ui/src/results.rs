//! The results area: one tab per node that was run, plus a preview of the selected node when it
//! has a result. Data shows as a virtualized table, statistics and text outputs as documents,
//! trained models as their summary, failures with their cause and a way to the failing node.

use crate::document_view::show_document;
use crate::graph::NodeKind;
use crate::graph::StreamGraph;
use crate::model_view::show_model;
use crate::results_table::show_table;
use dms_engine::catalog::Palette;
use dms_engine::run::RunReport;
use dms_exec::{ErrorKind, NodeError, NodeOutput};
use egui::{RichText, Ui};
use std::path::Path;

/// Failures listed by name before the rest are counted.
const MAX_LISTED_FAILURES: usize = 20;

/// What the person asked for in the results area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultsAction {
    /// Select this node on the canvas and centre it.
    Focus(String),
    /// Run this command (for example "file.add_data_folder").
    Command(&'static str),
}

/// Whether the run failed because a data file the stream names was not found.
fn missing_data(report: &RunReport) -> bool {
    report.root_causes.iter().any(
        |error| matches!(&error.kind, ErrorKind::Io(message) if message.contains("cannot find")),
    )
}

/// The tab to show: `active` if it still has a result, else the first target that has one.
fn tabs(report: &RunReport, selected: Option<&str>) -> Vec<String> {
    let mut tabs: Vec<String> = Vec::new();
    for uid in &report.targets {
        if report.results.contains_key(uid) && !tabs.contains(uid) {
            tabs.push(uid.clone());
        }
    }
    if let Some(selected) = selected
        && report.results.contains_key(selected)
        && !tabs.iter().any(|t| t == selected)
    {
        tabs.push(selected.to_string());
    }
    tabs
}

pub fn title_of(graph: Option<&StreamGraph>, uid: &str) -> String {
    graph
        .and_then(|g| g.nodes.iter().find(|n| n.uid == uid))
        .map_or_else(|| uid.to_string(), |n| n.title.clone())
}

pub fn show_results(
    ui: &mut Ui,
    report: &RunReport,
    graph: Option<&StreamGraph>,
    selected: Option<&str>,
    active: &mut Option<String>,
    export_dir: Option<&Path>,
) -> Option<ResultsAction> {
    let mut action = None;
    if let (Some(dir), Some(graph)) = (export_dir, graph) {
        let exported = report.targets.iter().any(|uid| {
            matches!(report.results.get(uid), Some(Ok(_)))
                && graph
                    .nodes
                    .iter()
                    .any(|n| &n.uid == uid && n.kind == NodeKind::Palette(Palette::Export))
        });
        if exported {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("Exported files are in {}", dir.display()));
                if ui.button("Show in Finder").clicked() {
                    ui.ctx()
                        .open_url(egui::OpenUrl::same_tab(format!("file://{}", dir.display())));
                }
            });
        }
    }
    let tabs = tabs(report, selected);
    if !report.root_causes.is_empty() {
        let error_color = ui.visuals().error_fg_color;
        let count = report.root_causes.len();
        ui.colored_label(
            error_color,
            format!(
                "{count} node{} failed; the other nodes ran:",
                if count == 1 { "" } else { "s" }
            ),
        );
        for error in report.root_causes.iter().take(MAX_LISTED_FAILURES) {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .link(title_of(graph, &error.uid))
                    .on_hover_text("Show this node")
                    .clicked()
                {
                    action = Some(ResultsAction::Focus(error.uid.clone()));
                }
                ui.colored_label(error_color, error_text(error));
            });
        }
        if count > MAX_LISTED_FAILURES {
            ui.weak(format!("and {} more", count - MAX_LISTED_FAILURES));
        }
        if missing_data(report) {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    "A data file the stream names was not found. Add the folder that holds it, \
                     then run again.",
                );
                if ui.button("Add Data Folder…").clicked() {
                    action = Some(ResultsAction::Command("file.add_data_folder"));
                }
            });
        }
    }
    if tabs.is_empty() {
        ui.weak("This run produced no results.");
        return action;
    }
    if !active.as_ref().is_some_and(|a| tabs.contains(a)) {
        *active = tabs.first().cloned();
    }
    ui.horizontal_wrapped(|ui| {
        for uid in &tabs {
            let preview = !report.targets.contains(uid);
            let mut label = title_of(graph, uid);
            if preview {
                label = format!("Preview: {label}");
            }
            let failed = matches!(report.results.get(uid), Some(Err(_)));
            let text = if failed {
                RichText::new(label).color(ui.visuals().error_fg_color)
            } else {
                RichText::new(label)
            };
            if ui
                .selectable_label(active.as_deref() == Some(uid), text)
                .clicked()
            {
                *active = Some(uid.clone());
            }
        }
    });
    ui.separator();
    let Some(uid) = active.clone() else {
        return action;
    };
    match report.results.get(&uid) {
        Some(Ok(NodeOutput::Data(table))) => show_table(ui, table, ("result", &uid)),
        Some(Ok(NodeOutput::Document(document))) => show_document(ui, document),
        Some(Ok(NodeOutput::Model(model))) => show_model(ui, model),
        Some(Err(error)) => {
            ui.label(RichText::new(format!("{} did not run", title_of(graph, &uid))).strong());
            ui.label(error_text(error));
            if let ErrorKind::Upstream(_) = &error.kind {
                let root = root_cause_of(report, &uid);
                if ui
                    .button(format!("Show {}", title_of(graph, &root)))
                    .clicked()
                {
                    action = Some(ResultsAction::Focus(root));
                }
            } else if ui.button("Show on canvas").clicked() {
                action = Some(ResultsAction::Focus(uid.clone()));
            }
        }
        None => {
            ui.weak("No result for this node.");
        }
    }
    action
}

/// The node where the failure that blocked `uid` started: follows `Upstream` errors (bounded).
pub fn root_cause_of(report: &RunReport, uid: &str) -> String {
    let mut current = uid.to_string();
    for _ in 0..1024 {
        match report.results.get(&current) {
            Some(Err(NodeError {
                kind: ErrorKind::Upstream(next),
                ..
            })) if next != &current => current = next.clone(),
            _ => break,
        }
    }
    current
}

/// "data", "document", "model" or "error": what a node produced.
pub fn result_kind(result: &dms_exec::NodeResult) -> &'static str {
    match result {
        Ok(NodeOutput::Data(_)) => "data",
        Ok(NodeOutput::Document(_)) => "document",
        Ok(NodeOutput::Model(_)) => "model",
        Err(_) => "error",
    }
}

/// The error without the node id prefix (the tab already names the node).
pub fn error_text(error: &NodeError) -> String {
    match &error.kind {
        ErrorKind::Unsupported(what) => format!("This node type is not supported yet ({what})."),
        ErrorKind::Settings(message) => format!("Settings: {message}"),
        ErrorKind::Inputs(message) => format!("Inputs: {message}"),
        ErrorKind::Upstream(_) => "A node it depends on failed.".to_string(),
        ErrorKind::Io(message) => format!("File: {message}"),
        ErrorKind::Data(message) => format!("Data: {message}"),
        ErrorKind::Cycle => "The node is part of a cycle.".to_string(),
    }
}
