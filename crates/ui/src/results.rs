//! The results area: one tab per node that was run, plus a preview of the selected node when it
//! has a result. Data shows as a virtualized table, statistics and text outputs as documents,
//! trained models as their summary, failures with their cause and a way to the failing node.

use crate::chart::{Chart, chart_for, settings_are_bounded, show_chart};
use crate::document_view::show_document;
use crate::graph::NodeKind;
use crate::graph::StreamGraph;
use crate::i18n::{tr, tr_fmt};
use crate::model_view::show_model;
use crate::results_table::show_table;
use dms_engine::catalog::Palette;
use dms_engine::run::RunReport;
use dms_exec::{ErrorKind, NodeError, NodeOutput};
use dms_strfmt::fastinfoset::Element;
use egui::{RichText, Ui};
use std::path::Path;
use std::sync::Arc;

/// Cache only the currently displayed document, so large charts are not rebuilt every frame.
#[derive(Clone)]
struct ChartCache {
    document: Arc<dms_exec::Document>,
    settings: Option<Element>,
    node_type: String,
    chart: Option<Chart>,
}

fn show_graph_document(
    ui: &mut Ui,
    uid: &str,
    node_type: &str,
    settings: Option<&Element>,
    document: &Arc<dms_exec::Document>,
) {
    if !matches!(
        node_type,
        "histogram"
            | "distribution"
            | "plot"
            | "multiplot"
            | "time_plot"
            | "evaluation"
            | "collection"
    ) || !settings_are_bounded(settings)
    {
        show_document(ui, document);
        return;
    }
    let cache_id = ui.scope_id().with("result_chart_cache");
    let cached = ui.data(|data| data.get_temp::<Arc<ChartCache>>(cache_id));
    let cached = cached
        .filter(|cache| {
            Arc::ptr_eq(&cache.document, document)
                && cache.node_type == node_type
                && cache.settings.as_ref() == settings
        })
        .unwrap_or_else(|| {
            let cache = Arc::new(ChartCache {
                document: Arc::clone(document),
                settings: settings.cloned(),
                node_type: node_type.to_string(),
                chart: chart_for(node_type, settings, document),
            });
            ui.data_mut(|data| data.insert_temp(cache_id, Arc::clone(&cache)));
            cache
        });
    let Some(chart) = &cached.chart else {
        show_document(ui, document);
        return;
    };
    let view_id = ui.scope_id().with(("result_chart_view", uid));
    let mut data_view = ui
        .data(|data| data.get_temp::<bool>(view_id))
        .unwrap_or(false);
    ui.horizontal(|ui| {
        ui.selectable_value(&mut data_view, false, tr("Chart"));
        ui.selectable_value(&mut data_view, true, tr("Data"));
    });
    ui.data_mut(|data| data.insert_temp(view_id, data_view));
    if data_view {
        show_document(ui, document);
    } else {
        show_chart(ui, chart, ("result_chart", uid));
    }
}

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
    show_results_with_settings(ui, report, graph, selected, active, export_dir, &|_| None)
}

/// Results with settings looked up by uid in the currently open stream. The original entry
/// point remains available for callers that only have a report.
pub fn show_results_with_settings<'a>(
    ui: &mut Ui,
    report: &RunReport,
    graph: Option<&StreamGraph>,
    selected: Option<&str>,
    active: &mut Option<String>,
    export_dir: Option<&Path>,
    settings: &dyn Fn(&str) -> Option<&'a Element>,
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
                ui.label(tr_fmt(
                    "Exported files are in {folder}",
                    &[("folder", &dir.display().to_string())],
                ));
                if ui.button(tr("Show in Finder")).clicked() {
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
            if count == 1 {
                tr_fmt(
                    "{count} node failed; the other nodes ran:",
                    &[("count", &count.to_string())],
                )
            } else {
                tr_fmt(
                    "{count} nodes failed; the other nodes ran:",
                    &[("count", &count.to_string())],
                )
            },
        );
        for error in report.root_causes.iter().take(MAX_LISTED_FAILURES) {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .link(title_of(graph, &error.uid))
                    .on_hover_text(tr("Show this node"))
                    .clicked()
                {
                    action = Some(ResultsAction::Focus(error.uid.clone()));
                }
                ui.colored_label(error_color, error_text(error));
            });
        }
        if count > MAX_LISTED_FAILURES {
            ui.weak(tr_fmt(
                "and {count} more",
                &[(
                    "count",
                    &count.saturating_sub(MAX_LISTED_FAILURES).to_string(),
                )],
            ));
        }
        if missing_data(report) {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    tr("A data file the stream names was not found. Add the folder that holds it, then run again."),
                );
                if ui.button(tr("Add Data Folder…")).clicked() {
                    action = Some(ResultsAction::Command("file.add_data_folder"));
                }
            });
        }
    }
    if tabs.is_empty() {
        ui.weak(tr("This run produced no results."));
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
                label = tr_fmt("Preview: {label}", &[("label", &label)]);
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
        Some(Ok(NodeOutput::Document(document))) => {
            let node_type = graph
                .and_then(|g| g.nodes.iter().find(|n| n.uid == uid))
                .map(|n| n.type_id.as_str());
            if let Some(node_type) = node_type {
                show_graph_document(ui, &uid, node_type, settings(&uid), document);
            } else {
                show_document(ui, document);
            }
        }
        Some(Ok(NodeOutput::Model(model))) => show_model(ui, model),
        Some(Err(error)) => {
            ui.label(
                RichText::new(tr_fmt(
                    "{title} did not run",
                    &[("title", &title_of(graph, &uid))],
                ))
                .strong(),
            );
            ui.label(error_text(error));
            if let ErrorKind::Upstream(_) = &error.kind {
                let root = root_cause_of(report, &uid);
                if ui
                    .button(tr_fmt(
                        "Show {title}",
                        &[("title", &title_of(graph, &root))],
                    ))
                    .clicked()
                {
                    action = Some(ResultsAction::Focus(root));
                }
            } else if ui.button(tr("Show on canvas")).clicked() {
                action = Some(ResultsAction::Focus(uid.clone()));
            }
        }
        None => {
            ui.weak(tr("No result for this node."));
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
        ErrorKind::Cancelled => tr("Stopped before this node ran.").to_string(),
        ErrorKind::Unsupported(what) => tr_fmt(
            "This node type is not supported yet ({what}).",
            &[("what", what)],
        ),
        ErrorKind::Settings(message) => tr_fmt("Settings: {message}", &[("message", message)]),
        ErrorKind::Inputs(message) => tr_fmt("Inputs: {message}", &[("message", message)]),
        ErrorKind::Upstream(_) => tr("A node it depends on failed.").to_string(),
        ErrorKind::Io(message) => tr_fmt("File: {message}", &[("message", message)]),
        ErrorKind::Data(message) => tr_fmt("Data: {message}", &[("message", message)]),
        ErrorKind::Cycle => tr("The node is part of a cycle.").to_string(),
    }
}
