//! The main window: node palette, stream canvas, properties, status bar, and the dispatch of
//! commands by id. Lives in the UI library (not the app) so the whole window can be rendered
//! off-screen in tests. Layout: docs/gui/DESIGN.md §2.

use crate::i18n::{tr, tr_fmt};
#[path = "dialogs.rs"]
mod dialogs;
#[path = "edit_canvas.rs"]
mod edit_canvas;

use crate::canvas::{CanvasView, RunMark, ZOOM_STEP};
use crate::graph::StreamGraph;
use crate::menu;
use crate::results::{self, ResultsAction};
use dialogs::DiscardChoice;
use dms_engine::catalog::{NODES, Palette, Scope};
use dms_engine::job::Waker;
use dms_engine::nodes;
use dms_engine::session::{
    Arg, CommandError, Document, Outcome, RunNotice, RunState, Session, ViewAction,
};
use dms_exec::ErrorKind;
use edit_canvas::{Action, Editor};
use egui::{Context, Rect, RichText, Ui, ViewportCommand};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

/// The host owns the system clipboard; UI tests can supply an in-memory implementation.
pub trait Clipboard {
    fn get_text(&mut self) -> Result<String, String>;
    fn set_text(&mut self, text: &str) -> Result<(), String>;
}

const MAX_CLIPBOARD_BYTES: usize = 16 << 20;

#[derive(Clone)]
struct PendingCommand {
    id: &'static str,
    args: Vec<Arg>,
}

/// Interface state that survives restarts (stored by the app, see DESIGN.md §7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShellState {
    pub show_palette: bool,
    pub show_properties: bool,
    pub show_results: bool,
    pub canvas: CanvasView,
    /// Extra folders searched for the data streams reference (File > Add Data Folder…).
    pub data_folders: Vec<PathBuf>,
    pub language: crate::i18n::Language,
}

impl Default for ShellState {
    fn default() -> Self {
        ShellState {
            show_palette: true,
            show_properties: true,
            show_results: true,
            canvas: CanvasView::default(),
            data_folders: Vec::new(),
            language: crate::i18n::Language::System,
        }
    }
}

impl ShellState {
    pub fn sanitize(&mut self) {
        self.canvas.sanitize();
    }
}

/// What a command asks the person to choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// A stream file to open.
    Stream,
    /// A data file, filtered by the parameter schema except on macOS.
    DataFile { extensions: &'static [&'static str] },
    /// A folder (data folders).
    Folder,
    /// Native .dms save dialog (suggested filename is available on the workbench).
    Save,
}

/// What the host platform provides.
#[derive(Debug, Clone, Copy, Default)]
pub struct Platform {
    /// macOS conventions (application menu, ⌘ shortcuts).
    pub mac: bool,
    /// The menu bar is native (outside the window) and handles its own shortcuts.
    pub native_menu: bool,
    /// CPU time of the previous frame, shown in the status bar when set.
    pub frame_cpu_ms: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusMessage {
    text: StatusText,
    error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StatusText {
    Raw(String),
    Finished { seconds: String, failures: usize },
    Failed(String),
    Stopped,
    FolderAdded(PathBuf),
    FoldersCleared,
}

impl StatusText {
    fn display(&self) -> String {
        match self {
            Self::Raw(text) => text.clone(),
            Self::Finished {
                seconds,
                failures: 0,
            } => tr_fmt("Finished in {seconds} s", &[("seconds", seconds)]),
            Self::Finished {
                seconds,
                failures: 1,
            } => tr_fmt(
                "Finished in {seconds} s · {count} node failed",
                &[("seconds", seconds), ("count", "1")],
            ),
            Self::Finished { seconds, failures } => tr_fmt(
                "Finished in {seconds} s · {count} nodes failed",
                &[("seconds", seconds), ("count", &failures.to_string())],
            ),
            Self::Failed(message) => tr_fmt(
                "The run stopped unexpectedly: {message}",
                &[("message", message)],
            ),
            Self::Stopped => tr("Stopped").into(),
            Self::FolderAdded(folder) => tr_fmt(
                "Data folder added: {folder} (run again to use it)",
                &[("folder", &folder.display().to_string())],
            ),
            Self::FoldersCleared => tr("Data folders cleared").into(),
        }
    }
}

pub struct Workbench {
    pub session: Session,
    pub state: ShellState,
    graph: Option<StreamGraph>,
    status: Option<StatusMessage>,
    about_open: bool,
    settings_open: bool,
    fit_pending: bool,
    canvas_rect: Option<Rect>,
    /// The results tab shown (a node uid).
    results_tab: Option<String>,
    /// The window title last sent, so it is sent only when it changes.
    title: Option<String>,
    graph_revision: Option<u64>,
    editor: Editor,
    clipboard: Option<Box<dyn Clipboard>>,
    text_events: Vec<egui::Event>,
    pending: Option<PendingCommand>,
    force_next: Option<PendingCommand>,
    save_then_continue: bool,
    allow_close: bool,
    recovery: Vec<dms_engine::session::Recovery>,
    palette_drag: Option<String>,
    autosave_error: Option<String>,
    properties: crate::properties::Properties,
}

impl Workbench {
    pub fn new(mut state: ShellState) -> Self {
        let mut session = Session::new();
        session.set_data_folders(state.data_folders.clone());
        state.data_folders = session.data_folders().to_vec();
        Workbench {
            session,
            state,
            graph: None,
            status: None,
            about_open: false,
            settings_open: false,
            fit_pending: false,
            canvas_rect: None,
            results_tab: None,
            title: None,
            graph_revision: None,
            editor: Editor::default(),
            clipboard: None,
            text_events: Vec::new(),
            pending: None,
            force_next: None,
            save_then_continue: false,
            allow_close: false,
            recovery: Vec::new(),
            palette_drag: None,
            autosave_error: None,
            properties: crate::properties::Properties::default(),
        }
    }

    /// Repaints the window when a background run has news.
    pub fn connect_repaint(&mut self, ctx: &Context) {
        let ctx = ctx.clone();
        let waker: Waker = Arc::new(move || ctx.request_repaint());
        self.session.set_waker(Some(waker));
    }

    /// A workbench showing an already decoded stream, zoomed to fit on the first frame.
    pub fn with_document(document: Document, state: ShellState) -> Self {
        let mut workbench = Workbench::new(state);
        let folders = workbench.session.data_folders().to_vec();
        workbench.session = Session::with_document(document);
        workbench.session.set_data_folders(folders);
        workbench.document_changed(true);
        workbench
    }

    pub fn set_clipboard(&mut self, clipboard: Box<dyn Clipboard>) {
        self.clipboard = Some(clipboard);
    }

    /// Configure once at startup, before opening the recovery modal.
    pub fn set_autosave_dir(&mut self, dir: Option<PathBuf>) {
        self.session.set_autosave_dir(dir);
        self.recovery = self.session.recoverable();
    }

    pub fn save_filename(&self) -> String {
        let stem = self
            .session
            .document()
            .and_then(|d| d.path.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| tr("Untitled").into());
        format!("{stem}.dms")
    }

    pub fn graph(&self) -> Option<&StreamGraph> {
        self.graph.as_ref()
    }

    fn document_changed(&mut self, fit: bool) {
        self.graph = self
            .session
            .document()
            .map(|document| StreamGraph::from_diagram(&document.stream.diagram));
        self.graph_revision = self.session.document().map(|_| self.session.revision());
        self.editor = Editor::default();
        self.properties = crate::properties::Properties::default();
        self.state.canvas.selected = None;
        self.results_tab = None;
        self.fit_pending = fit && self.graph.is_some();
    }

    /// Selects a node, centres it on the canvas and shows its result.
    pub fn focus_node(&mut self, uid: &str) {
        let result = self
            .session
            .execute("edit.select", &[Arg::Nodes(vec![uid.into()])]);
        if let Err(error) = result {
            self.report_error(error.to_string());
        }
        self.state.canvas.selected = Some(uid.to_string());
        self.results_tab = Some(uid.to_string());
        let position = self
            .graph
            .as_ref()
            .and_then(|g| g.nodes.iter().find(|n| n.uid == uid))
            .map(|n| n.pos);
        if let (Some(position), Some(rect)) = (position, self.canvas_rect) {
            let view = &mut self.state.canvas;
            let offset = rect.size() / 2.0 - position.to_vec2() * view.zoom;
            view.pan = [offset.x, offset.y];
        }
    }

    /// Drains background-run events into the status line. Called every frame by `ui`.
    fn poll_runs(&mut self) {
        for notice in self.session.poll_jobs() {
            self.status = Some(match notice {
                RunNotice::Finished {
                    root_causes,
                    elapsed,
                    ..
                } => StatusMessage {
                    text: StatusText::Finished {
                        seconds: format!("{:.1}", elapsed.as_secs_f64()),
                        failures: root_causes,
                    },
                    error: root_causes != 0,
                },
                RunNotice::Failed { message, .. } => StatusMessage {
                    text: StatusText::Failed(message),
                    error: true,
                },
                RunNotice::Cancelled { .. } => StatusMessage {
                    text: StatusText::Stopped,
                    error: false,
                },
            });
            if let Some(first) = self
                .session
                .results()
                .and_then(|r| r.report.root_causes.first())
                .map(|e| e.uid.clone())
            {
                self.results_tab = Some(first);
            }
        }
    }

    /// Badges for the canvas: the running node, and every node of the latest run.
    fn run_marks(&self) -> HashMap<String, RunMark> {
        let mut marks = HashMap::new();
        if let Some(results) = self.session.results() {
            for (uid, result) in &results.report.results {
                let mark = match result {
                    Ok(_) => RunMark::Done,
                    Err(error) if matches!(error.kind, ErrorKind::Upstream(_)) => RunMark::Blocked,
                    Err(_) => RunMark::Failed,
                };
                marks.insert(uid.clone(), mark);
            }
        }
        if let RunState::Running { progress, .. } = self.session.run_state()
            && let Some(current) = progress.current
        {
            marks.insert(current, RunMark::Running);
        }
        marks
    }

    fn report_error(&mut self, text: String) {
        self.status = Some(StatusMessage {
            text: StatusText::Raw(text),
            error: true,
        });
    }

    fn sync_graph(&mut self) {
        let revision = self.session.document().map(|_| self.session.revision());
        if self.graph_revision != revision {
            self.graph = self
                .session
                .document()
                .map(|d| StreamGraph::from_diagram(&d.stream.diagram));
            self.graph_revision = revision;
            self.editor.selected_link = None;
        }
        self.state.canvas.selected = self
            .session
            .document_state()
            .and_then(|s| s.selection.first().cloned());
    }

    /// Opens from Finder, the CLI or a drop, preserving the requested path across confirmation.
    pub fn open_path(&mut self, path: PathBuf) {
        let args = vec![Arg::Path(path)];
        let result = self.session.execute("file.open", &args);
        self.finish("file.open", args, result, None);
    }

    /// Runs a command by id. Native pickers are used only in response to NeedsArgument.
    pub fn dispatch(
        &mut self,
        ctx: &Context,
        id: &str,
        pick: &mut dyn FnMut(Pick) -> Option<PathBuf>,
    ) {
        if self.pending.is_some()
            && !matches!(id, "file.save" | "file.save_as")
            && self.force_next.is_none()
        {
            return;
        }
        if id == "app.settings" {
            self.settings_open = true;
            ctx.request_repaint();
            return;
        }
        if id == "node.set_param" {
            if let Some(request) = self.properties.take_file_request()
                && let Some(path) = pick(Pick::DataFile {
                    extensions: request.extensions,
                })
            {
                self.properties
                    .picked_file(&mut self.session, request, path);
                ctx.request_repaint();
            }
            return;
        }
        if ctx.egui_wants_keyboard_input()
            && matches!(
                id,
                "edit.copy" | "edit.cut" | "edit.paste" | "edit.select_all"
            )
        {
            let event = match id {
                "edit.copy" => egui::Event::Copy,
                "edit.cut" => egui::Event::Cut,
                "edit.paste" => match self.clipboard_text() {
                    Ok(text) => egui::Event::Paste(text),
                    Err(error) => {
                        self.report_error(error);
                        return;
                    }
                },
                _ => egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
            };
            let queued_bytes = self.text_events.iter().fold(0usize, |size, event| {
                size.saturating_add(match event {
                    egui::Event::Paste(text) => text.len(),
                    _ => 0,
                })
            });
            let bytes = match &event {
                egui::Event::Paste(text) => text.len(),
                _ => 0,
            };
            if self.text_events.len() >= 64
                || queued_bytes.saturating_add(bytes) > MAX_CLIPBOARD_BYTES
            {
                self.report_error(tr("Too many pending clipboard operations.").into());
                return;
            }
            self.text_events.push(event);
            ctx.request_repaint();
            return;
        }
        if id == "node.rename" {
            if let (Some(uid), Some(graph)) = (&self.state.canvas.selected, &self.graph) {
                self.editor.begin_rename(uid, graph);
            }
            return;
        }
        if id == "edit.delete"
            && let Some((from, to)) = self.editor.selected_link.take()
        {
            self.execute_args(ctx, "node.disconnect", vec![Arg::Link(from, to)]);
            return;
        }
        let Some(spec) = dms_engine::command::by_id(id) else {
            self.report_error(tr_fmt("Unknown command {id}", &[("id", id)]));
            return;
        };
        let pending = self.force_next.take().filter(|p| p.id == spec.id);
        let args = pending.map_or_else(Vec::new, |mut p| {
            p.args.push(Arg::Force);
            p.args
        });
        self.dispatch_args(ctx, spec.id, args, pick);
    }

    fn clipboard_text(&mut self) -> Result<String, String> {
        let text = self
            .clipboard
            .as_mut()
            .ok_or(tr("System clipboard is unavailable."))?
            .get_text()?;
        if text.len() > MAX_CLIPBOARD_BYTES {
            return Err(tr("Clipboard text exceeds 16 MiB.").into());
        }
        Ok(text)
    }

    fn write_clipboard(&mut self, text: &str) -> Result<(), String> {
        if text.len() > MAX_CLIPBOARD_BYTES {
            return Err(tr("Clipboard text exceeds 16 MiB.").into());
        }
        self.clipboard
            .as_mut()
            .ok_or(tr("System clipboard is unavailable."))?
            .set_text(text)
    }

    fn dispatch_args(
        &mut self,
        ctx: &Context,
        id: &'static str,
        mut args: Vec<Arg>,
        pick: &mut dyn FnMut(Pick) -> Option<PathBuf>,
    ) {
        if id == "edit.paste" {
            match self.clipboard_text() {
                Ok(text) => args.push(Arg::Text(text)),
                Err(error) => {
                    self.report_error(error);
                    return;
                }
            }
        }
        // Write before cutting, so a failed system clipboard cannot destroy selected nodes.
        if id == "edit.cut" {
            match self.session.execute("edit.copy", &[]) {
                Ok(Outcome::Clipboard(text)) => {
                    if let Err(error) = self.write_clipboard(&text) {
                        self.report_error(error);
                        return;
                    }
                }
                Err(error) => {
                    self.report_error(error.to_string());
                    return;
                }
                _ => return,
            }
        }
        let mut actual_id = id;
        let mut result = self.session.execute(actual_id, &args);
        if let Ok(Outcome::NeedsArgument(name)) = &result {
            let kind = match *name {
                "path" => Some(Pick::Stream),
                "folder" => Some(Pick::Folder),
                "save_path" => Some(Pick::Save),
                _ => None,
            };
            if let Some(kind) = kind {
                let Some(path) = pick(kind) else {
                    self.save_then_continue = false;
                    return;
                };
                args.push(Arg::Path(path));
                // file.save on an imported/new document deliberately ignores Path; use Save As.
                if kind == Pick::Save {
                    actual_id = "file.save_as";
                }
                result = self.session.execute(actual_id, &args);
            } else if *name == "node"
                && let Some(uid) = &self.state.canvas.selected
            {
                args.push(Arg::Node(uid.clone()));
                result = self.session.execute(actual_id, &args);
            }
        }
        // Cut was already written successfully, before the destructive command.
        if id == "edit.cut" && matches!(result, Ok(Outcome::Clipboard(_))) {
            result = Ok(Outcome::Done);
        }
        let saved = matches!(actual_id, "file.save" | "file.save_as")
            && matches!(result, Ok(Outcome::Done))
            && !self.session.is_dirty();
        self.finish(actual_id, args, result, Some(ctx));
        if saved && self.save_then_continue {
            self.save_then_continue = false;
            if let Some(mut pending) = self.pending.take() {
                pending.args.push(Arg::Force);
                self.dispatch_args(ctx, pending.id, pending.args, pick);
            }
        }
    }

    fn execute_args(&mut self, ctx: &Context, id: &'static str, args: Vec<Arg>) {
        if matches!(id, "edit.copy" | "edit.cut" | "edit.paste") {
            self.dispatch_args(ctx, id, args, &mut |_| None);
        } else {
            let result = self.session.execute(id, &args);
            self.finish(id, args, result, Some(ctx));
        }
    }

    fn finish(
        &mut self,
        id: &'static str,
        args: Vec<Arg>,
        result: Result<Outcome, CommandError>,
        ctx: Option<&Context>,
    ) {
        match result {
            Ok(Outcome::Done) => {
                if id == "run.stream" || id == "run.node" {
                    self.status = None;
                }
                if matches!(id, "file.add_data_folder" | "file.clear_data_folders") {
                    self.state.data_folders = self.session.data_folders().to_vec();
                    self.status = Some(StatusMessage {
                        text: match self.state.data_folders.last() {
                            Some(folder) if id == "file.add_data_folder" => {
                                StatusText::FolderAdded(folder.clone())
                            }
                            _ => StatusText::FoldersCleared,
                        },
                        error: false,
                    });
                }
                if matches!(id, "file.open" | "file.close" | "file.new" | "file.recover") {
                    self.document_changed(id != "file.close");
                    self.status = None;
                    self.recovery.retain(|r| {
                        !args
                            .iter()
                            .any(|a| matches!(a, Arg::Path(p) if p == &r.path))
                    });
                }
            }
            Ok(Outcome::ConfirmDiscard) => {
                // Preserve the first pending action if more close requests arrive.
                if self.pending.is_none() {
                    self.pending = Some(PendingCommand { id, args });
                }
            }
            Ok(Outcome::Clipboard(text)) => {
                if let Err(error) = self.write_clipboard(&text) {
                    self.report_error(error);
                }
            }
            Ok(Outcome::View(action)) => self.apply_view(action, ctx),
            Ok(Outcome::Quit) => {
                self.allow_close = true;
                if let Some(ctx) = ctx {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
            Ok(Outcome::NeedsArgument(name)) => {
                self.report_error(tr_fmt("{id} needs {name}", &[("id", id), ("name", name)]))
            }
            Err(error) => {
                self.save_then_continue = false;
                self.report_error(error.to_string());
            }
        }
        self.sync_graph();
    }

    fn apply_view(&mut self, action: ViewAction, ctx: Option<&Context>) {
        let rect = self.canvas_rect.unwrap_or(Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        ));
        let view = &mut self.state.canvas;
        match action {
            ViewAction::ZoomIn => view.zoom_about(rect.min, rect.center(), ZOOM_STEP),
            ViewAction::ZoomOut => view.zoom_about(rect.min, rect.center(), 1.0 / ZOOM_STEP),
            ViewAction::ZoomFit => self.fit_pending = true,
            ViewAction::ToggleFullscreen => {
                if let Some(ctx) = ctx {
                    let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                    ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!full));
                }
            }
            ViewAction::ShowAbout => self.about_open = true,
        }
    }

    /// Draws the window. Returns the commands the person asked for this frame (menus, toolbar,
    /// shortcuts); the caller passes them to [`Self::dispatch`]. Dropped files open directly.
    pub fn ui(&mut self, ui: &mut Ui, platform: &Platform) -> Vec<&'static str> {
        let mut requested = Vec::new();
        let text_events = std::mem::take(&mut self.text_events);
        if !text_events.is_empty() {
            ui.ctx().input_mut(|i| i.events.extend(text_events));
        }
        self.poll_runs();
        self.sync_graph();
        if let Err(error) = self.session.autosave_if_due(Instant::now()) {
            if self.autosave_error.as_ref() != Some(&error) {
                self.report_error(error.clone());
                self.autosave_error = Some(error);
            }
        } else {
            self.autosave_error = None;
        }
        ui.ctx().request_repaint_after(Duration::from_secs(1));
        if ui.input(|i| i.viewport().close_requested()) && !self.allow_close {
            // All exits pass through the engine guard, even when the document is clean.
            ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose);
            if self.pending.is_none() {
                requested.push("app.quit");
            }
        }
        let title = match self.session.document() {
            Some(document) => format!(
                "{}{} — {}",
                document
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                if self.session.is_dirty() {
                    tr(" — Edited")
                } else {
                    ""
                },
                tr("Data Mining")
            ),
            None => tr("Data Mining").to_string(),
        };
        if self.title.as_deref() != Some(title.as_str()) {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = Some(title);
        }
        if self.pending.is_none() && self.recovery.is_empty() {
            requested.extend(menu::take_canvas_shortcuts(
                ui.ctx(),
                platform.mac,
                platform.native_menu,
                self.editor.focused,
            ));
            if self.editor.focused && !ui.ctx().egui_wants_keyboard_input() {
                ui.ctx().input_mut(|input| {
                    input.events.retain(|event| {
                        match event {
                            egui::Event::Copy => requested.push("edit.copy"),
                            egui::Event::Cut => requested.push("edit.cut"),
                            egui::Event::Paste(_) => requested.push("edit.paste"),
                            _ => return true,
                        }
                        false
                    })
                });
            }
        }
        let dropped: Vec<PathBuf> = ui.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if let Some(path) = dropped.into_iter().next() {
            self.open_path(path);
        }

        egui::Panel::top("toolbar").show(ui, |ui| {
            if !platform.native_menu {
                requested.extend(menu::menu_bar(ui, &self.session, platform.mac));
            }
            ui.horizontal(|ui| {
                if ui.button(tr("Open…")).clicked() {
                    requested.push("file.open");
                }
                ui.separator();
                let has_document = self.session.document().is_some();
                ui.add_enabled_ui(has_document, |ui| {
                    if ui.button("−").on_hover_text(tr("Zoom Out")).clicked() {
                        requested.push("view.zoom_out");
                    }
                    if ui
                        .button(tr("Fit"))
                        .on_hover_text(tr("Zoom to Fit"))
                        .clicked()
                    {
                        requested.push("view.zoom_fit");
                    }
                    if ui.button("+").on_hover_text(tr("Zoom In")).clicked() {
                        requested.push("view.zoom_in");
                    }
                });
                ui.separator();
                for (id, label, hint) in [
                    ("run.stream", tr("Run"), tr("Run Stream")),
                    (
                        "run.node",
                        tr("Run Selected"),
                        tr("Run the selected node and what it needs"),
                    ),
                    ("run.cancel", tr("Stop"), tr("Stop the run")),
                ] {
                    let availability = self.session.availability(id);
                    let response = ui.add_enabled(availability.is_ok(), egui::Button::new(label));
                    let response = match &availability {
                        Err(reason) => response.on_disabled_hover_text(reason),
                        Ok(()) => response.on_hover_text(hint),
                    };
                    if response.clicked() {
                        requested.push(id);
                    }
                }
                ui.separator();
                ui.toggle_value(&mut self.state.show_palette, tr("Nodes"));
                ui.toggle_value(&mut self.state.show_properties, tr("Properties"));
                ui.toggle_value(&mut self.state.show_results, tr("Results"));
            });
        });

        egui::Panel::bottom("status").show(ui, |ui| {
            if let Some(action) = self.status_bar(ui, platform) {
                requested.push(action);
            }
        });
        if self.state.show_results && self.session.results().is_some() {
            let mut action = None;
            egui::Panel::bottom("results")
                .resizable(true)
                .default_size(280.0)
                .min_size(120.0)
                .show(ui, |ui| {
                    if self.session.results_stale() {
                        ui.horizontal(|ui| {
                            ui.label(tr("The stream changed since this run."));
                            if ui
                                .add_enabled(
                                    self.session.is_enabled("run.stream"),
                                    egui::Button::new(tr("Run")),
                                )
                                .clicked()
                            {
                                requested.push("run.stream");
                            }
                        });
                    }
                    if let Some(results) = self.session.results() {
                        action = results::show_results(
                            ui,
                            &results.report,
                            self.graph.as_ref(),
                            self.state.canvas.selected.as_deref(),
                            &mut self.results_tab,
                            self.session
                                .document()
                                .map(|d| dms_engine::session::export_dir_for(&d.path))
                                .as_deref(),
                        );
                    }
                });
            match action {
                Some(ResultsAction::Focus(uid)) => self.focus_node(&uid),
                Some(ResultsAction::Command(id)) => requested.push(id),
                None => {}
            }
        }

        if self.state.show_palette {
            egui::Panel::left("palette")
                .resizable(true)
                .default_size(210.0)
                .show(ui, |ui| self.palette_panel(ui));
        }
        if self.state.show_properties {
            egui::Panel::right("properties")
                .resizable(true)
                .default_size(230.0)
                .show(ui, |ui| {
                    if self.properties_panel(ui) {
                        requested.push("node.set_param");
                    }
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.canvas_area(ui, &mut requested));

        if self.about_open {
            egui::Window::new(tr("About"))
                .open(&mut self.about_open)
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.heading(tr("Data Mining"));
                    ui.label(tr_fmt(
                        "Version {version}",
                        &[("version", env!("CARGO_PKG_VERSION"))],
                    ));
                    ui.label(tr("Opens and runs SPSS Modeler streams (.str)."));
                });
        }
        if let Some(action) =
            crate::settings_window::show(ui.ctx(), &mut self.settings_open, &mut self.state)
        {
            match action {
                crate::settings_window::Action::AddFolder => requested.push("file.add_data_folder"),
                crate::settings_window::Action::RemoveFolder(index) => {
                    if index < self.state.data_folders.len() {
                        self.state.data_folders.remove(index);
                        self.session
                            .set_data_folders(self.state.data_folders.clone());
                        ui.ctx().request_repaint();
                    }
                }
            }
        }
        self.show_dialogs(ui.ctx(), &mut requested);
        requested
    }

    fn show_dialogs(&mut self, ctx: &Context, requested: &mut Vec<&'static str>) {
        if self.pending.is_some() {
            let name = self
                .session
                .document()
                .and_then(|d| d.path.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| tr("Untitled").into());
            match dialogs::confirm_discard(ctx, &name) {
                Some(DiscardChoice::Save) => {
                    self.save_then_continue = true;
                    requested.push("file.save");
                }
                Some(DiscardChoice::Discard) => {
                    if let Some(pending) = self.pending.take() {
                        requested.push(pending.id);
                        self.force_next = Some(pending);
                    }
                }
                Some(DiscardChoice::Cancel) => {
                    self.pending = None;
                    self.save_then_continue = false;
                }
                None => {}
            }
        } else if !self.recovery.is_empty() {
            let mut recover = None;
            let mut discard = None;
            egui::Modal::new(egui::Id::unique("recover-streams")).show(ctx, |ui| {
                ui.heading(tr("Recover Streams"));
                ui.label(tr("Unsaved streams were found from a previous session."));
                for entry in &self.recovery {
                    ui.horizontal(|ui| {
                        let name = entry
                            .original
                            .as_ref()
                            .unwrap_or(&entry.path)
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let seconds = entry
                            .saved_at
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        ui.label(format!("{name} — {}", recovery_time(seconds)));
                        if ui.button(tr("Recover")).clicked() {
                            recover = Some(entry.path.clone());
                        }
                        if ui.button(tr("Discard")).clicked() {
                            discard = Some(entry.path.clone());
                        }
                    });
                }
            });
            if let Some(path) = recover {
                self.execute_args(ctx, "file.recover", vec![Arg::Path(path)]);
            }
            if let Some(path) = discard {
                // Paths come only from Session::recoverable (bounded app-owned regular files).
                match std::fs::remove_file(&path) {
                    Ok(()) => self.recovery.retain(|r| r.path != path),
                    Err(error) => self.report_error(tr_fmt(
                        "Cannot discard {path}: {error}",
                        &[
                            ("path", &path.display().to_string()),
                            ("error", &error.to_string()),
                        ],
                    )),
                }
            }
        }
    }

    fn canvas_area(&mut self, ui: &mut Ui, requested: &mut Vec<&'static str>) {
        let rect = ui.available_rect_before_wrap();
        self.canvas_rect = Some(rect);
        let Some(graph) = &self.graph else {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(rect.height() * 0.4);
                    ui.label(RichText::new(tr("No stream open")).heading());
                    ui.label(tr(
                        "Open or create a stream, or drop a .str or .dms file on this window.",
                    ));
                    if ui.button(tr("Open…")).clicked() {
                        requested.push("file.open");
                    }
                });
            });
            return;
        };
        if self.fit_pending {
            if let Some(bounds) = graph.bounds() {
                self.state.canvas.fit(bounds, rect.size());
            }
            self.fit_pending = false;
        }
        let marks = self.run_marks();
        let selection = self
            .session
            .document_state()
            .map(|s| s.selection.clone())
            .unwrap_or_default();
        let actions = edit_canvas::show(
            ui,
            graph,
            &mut self.state.canvas,
            &marks,
            &selection,
            &mut self.editor,
        );
        for action in actions {
            match action {
                Action::Command(id, args) => self.execute_args(ui.ctx(), id, args),
                Action::Rename(uid) => {
                    self.execute_args(ui.ctx(), "edit.select", vec![Arg::Nodes(vec![uid.clone()])]);
                    if let Some(graph) = &self.graph {
                        self.editor.begin_rename(&uid, graph);
                    }
                }
            }
        }
        if ui.input(|i| i.pointer.primary_released())
            && let Some(kind) = self.palette_drag.take()
            && let Some(p) = ui
                .input(|i| i.pointer.interact_pos())
                .filter(|p| rect.contains(*p))
        {
            self.add_node(ui.ctx(), kind, p);
        }
    }

    fn properties_panel(&mut self, ui: &mut Ui) -> bool {
        ui.heading(tr("Properties"));
        ui.separator();
        let node = self.state.canvas.selected.as_ref().and_then(|uid| {
            self.graph
                .as_ref()
                .and_then(|g| g.nodes.iter().find(|n| &n.uid == uid))
        });
        let Some(node) = node else {
            ui.weak(tr("Select a node to see its settings."));
            return false;
        };
        ui.label(RichText::new(&node.title).strong());
        egui::Grid::new("node_facts").num_columns(2).show(ui, |ui| {
            ui.weak(tr("Type"));
            ui.label(&node.type_id);
            ui.end_row();
            ui.weak(tr("Status"));
            ui.label(node.status.explanation());
            ui.end_row();
            ui.weak(tr("Id"));
            ui.label(&node.uid);
            ui.end_row();
        });
        if let Some(Err(error)) = self
            .session
            .results()
            .and_then(|r| r.report.results.get(&node.uid))
        {
            ui.separator();
            ui.colored_label(ui.visuals().error_fg_color, tr("Last run failed here"));
            ui.label(results::error_text(error));
        }
        ui.separator();
        self.properties.show(ui, &mut self.session, &node.uid)
    }

    /// The status line. Returns a command when its Stop button is pressed.
    fn status_bar(&self, ui: &mut Ui, platform: &Platform) -> Option<&'static str> {
        let mut command = None;
        ui.horizontal(|ui| {
            if let RunState::Running { progress, .. } = self.session.run_state() {
                let fraction = if progress.total == 0 {
                    0.0
                } else {
                    progress.done as f32 / progress.total as f32
                };
                let running = progress
                    .current
                    .as_deref()
                    .map(|uid| results::title_of(self.graph.as_ref(), uid));
                ui.spinner();
                ui.label(match running {
                    Some(title) => tr_fmt("Running {title}", &[("title", &title)]),
                    None => tr("Running").to_string(),
                });
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .desired_width(140.0)
                        .text(format!("{}/{}", progress.done, progress.total)),
                );
                if ui.small_button(tr("Stop")).clicked() {
                    command = Some("run.cancel");
                }
                ui.separator();
            }
            match (&self.status, self.session.document()) {
                (Some(message), _) if message.error => {
                    ui.colored_label(ui.visuals().error_fg_color, message.text.display());
                }
                (message, Some(document)) => {
                    let name = document
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    ui.label(name);
                    if let Some(message) = message {
                        ui.separator();
                        ui.label(message.text.display());
                    }
                }
                _ => {
                    ui.weak(tr("Ready"));
                }
            }
            if let Some(graph) = &self.graph {
                ui.separator();
                ui.label(tr_fmt(
                    "{nodes} nodes · {links} links",
                    &[
                        ("nodes", &graph.nodes.len().to_string()),
                        ("links", &graph.links.len().to_string()),
                    ],
                ));
                let unsupported = graph.unsupported();
                if unsupported > 0 {
                    ui.separator();
                    ui.label(tr_fmt(
                        "{unsupported} not supported yet",
                        &[("unsupported", &unsupported.to_string())],
                    ))
                    .on_hover_text(tr(
                        "These nodes are kept as saved and shown with a dashed outline.",
                    ));
                }
                ui.separator();
                ui.label(format!("{:.0}%", self.state.canvas.zoom * 100.0));
            }
            if let Some(ms) = platform.frame_cpu_ms {
                ui.separator();
                ui.monospace(tr_fmt("{ms} ms", &[("ms", &format!("{ms:.1}"))]));
            }
        });
        command
    }
}

impl Workbench {
    fn add_node(&mut self, ctx: &Context, kind: String, screen: egui::Pos2) {
        let Some(rect) = self.canvas_rect else {
            return;
        };
        let p = self.state.canvas.to_canvas(rect.min, screen);
        self.execute_args(
            ctx,
            "node.add",
            vec![
                Arg::Kind(kind),
                Arg::Position(edit_canvas::coordinate(p.x), edit_canvas::coordinate(p.y)),
            ],
        );
    }

    fn palette_panel(&mut self, ui: &mut Ui) {
        ui.heading(tr("Nodes"));
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for palette in Palette::ALL {
                let specs: Vec<_> = NODES.iter().filter(|s| s.palette == palette).collect();
                let ready = specs.iter().filter(|s| nodes::is_implemented(s.id)).count();
                egui::CollapsingHeader::new(format!(
                    "{} ({ready}/{})",
                    palette_label(palette),
                    specs.len()
                ))
                .default_open(palette == Palette::Source)
                .show(ui, |ui| {
                    for spec in specs {
                        let implemented = nodes::is_implemented(spec.id);
                        let response = ui.add_enabled(
                            implemented,
                            egui::Label::new(spec.label)
                                .selectable(false)
                                .sense(egui::Sense::click_and_drag()),
                        );
                        if !implemented {
                            response.on_disabled_hover_text(unavailable_reason(spec.scope));
                        } else {
                            if response.drag_started() {
                                self.palette_drag = Some(spec.id.into());
                            }
                            if response.double_clicked()
                                && let Some(rect) = self.canvas_rect
                            {
                                self.add_node(ui.ctx(), spec.id.into(), rect.center());
                            }
                        }
                    }
                });
            }
        });
    }
}

/// Gregorian UTC display without an additional date dependency. Input is capped before arithmetic.
fn recovery_time(seconds: u64) -> String {
    let seconds = seconds.min(253_402_300_799);
    let days = (seconds / 86_400) as i64;
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        seconds % 86_400 / 3600,
        seconds % 3600 / 60
    )
}

fn unavailable_reason(scope: Scope) -> &'static str {
    match scope {
        Scope::Mvp => tr("Not implemented yet"),
        Scope::Later => tr("Planned for a later release"),
        Scope::LicenseBlocked => tr("Cannot be verified: blocked by the reference licence"),
        Scope::OutOfScope => tr("Not planned"),
    }
}

fn palette_label(palette: Palette) -> &'static str {
    match palette {
        Palette::Source => tr("Sources"),
        Palette::RecordOps => tr("Record Ops"),
        Palette::FieldOps => tr("Field Ops"),
        Palette::Graphs => tr("Graphs"),
        Palette::Modeling => tr("Modeling"),
        Palette::Output => tr("Output"),
        Palette::Export => tr("Export"),
        Palette::Structure => tr("Structure"),
    }
}
