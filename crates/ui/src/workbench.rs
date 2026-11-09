//! The main window: node palette, stream canvas, properties, status bar, and the dispatch of
//! commands by id. Lives in the UI library (not the app) so the whole window can be rendered
//! off-screen in tests. Layout: docs/gui/DESIGN.md §2.

use crate::canvas::{self, CanvasView, RunMark, ZOOM_STEP};
use crate::graph::StreamGraph;
use crate::menu;
use crate::results::{self, ResultsAction};
use dms_engine::catalog::{NODES, Palette, Scope};
use dms_engine::job::Waker;
use dms_engine::nodes;
use dms_engine::session::{
    Arg, CommandError, Document, Outcome, RunNotice, RunState, Session, ViewAction,
};
use dms_exec::ErrorKind;
use egui::{Context, Rect, RichText, Ui, ViewportCommand};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

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
}

impl Default for ShellState {
    fn default() -> Self {
        ShellState {
            show_palette: true,
            show_properties: true,
            show_results: true,
            canvas: CanvasView::default(),
            data_folders: Vec::new(),
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
    /// A folder (data folders).
    Folder,
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
    text: String,
    error: bool,
}

pub struct Workbench {
    pub session: Session,
    pub state: ShellState,
    graph: Option<StreamGraph>,
    status: Option<StatusMessage>,
    about_open: bool,
    fit_pending: bool,
    canvas_rect: Option<Rect>,
    /// The results tab shown (a node uid).
    results_tab: Option<String>,
    /// The window title last sent, so it is sent only when it changes.
    title: Option<String>,
}

impl Workbench {
    pub fn new(state: ShellState) -> Self {
        let mut session = Session::new();
        session.set_data_folders(state.data_folders.clone());
        Workbench {
            session,
            state,
            graph: None,
            status: None,
            about_open: false,
            fit_pending: false,
            canvas_rect: None,
            results_tab: None,
            title: None,
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

    pub fn graph(&self) -> Option<&StreamGraph> {
        self.graph.as_ref()
    }

    fn document_changed(&mut self, fit: bool) {
        self.graph = self
            .session
            .document()
            .map(|document| StreamGraph::from_diagram(&document.stream.diagram));
        self.state.canvas.selected = None;
        self.results_tab = None;
        self.fit_pending = fit && self.graph.is_some();
    }

    /// Selects a node, centres it on the canvas and shows its result.
    pub fn focus_node(&mut self, uid: &str) {
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
                } => {
                    let seconds = elapsed.as_secs_f64();
                    if root_causes == 0 {
                        StatusMessage {
                            text: format!("Finished in {seconds:.1} s"),
                            error: false,
                        }
                    } else {
                        StatusMessage {
                            text: format!(
                                "Finished in {seconds:.1} s · {root_causes} node{} failed",
                                if root_causes == 1 { "" } else { "s" }
                            ),
                            error: true,
                        }
                    }
                }
                RunNotice::Failed { message, .. } => StatusMessage {
                    text: format!("The run stopped unexpectedly: {message}"),
                    error: true,
                },
                RunNotice::Cancelled { .. } => StatusMessage {
                    text: "Stopped".to_string(),
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

    /// Opens a file directly (dropped on the window, or given on the command line).
    pub fn open_path(&mut self, path: PathBuf) {
        let result = self.session.execute("file.open", &[Arg::Path(path)]);
        self.finish("file.open", result, None);
    }

    /// Runs a command by id. `pick` is asked only when the command needs a file or folder the
    /// person has to choose (a native dialog in the app, a fixed answer in tests).
    pub fn dispatch(
        &mut self,
        ctx: &Context,
        id: &str,
        pick: &mut dyn FnMut(Pick) -> Option<PathBuf>,
    ) {
        let result = match self.session.execute(id, &[]) {
            Ok(Outcome::NeedsArgument("path")) => match pick(Pick::Stream) {
                Some(path) => self.session.execute(id, &[Arg::Path(path)]),
                None => return,
            },
            Ok(Outcome::NeedsArgument("folder")) => match pick(Pick::Folder) {
                Some(path) => self.session.execute(id, &[Arg::Path(path)]),
                None => return,
            },
            Ok(Outcome::NeedsArgument("node")) => match self.state.canvas.selected.clone() {
                Some(uid) => {
                    self.results_tab = Some(uid.clone());
                    self.session.execute(id, &[Arg::Node(uid)])
                }
                None => {
                    self.status = Some(StatusMessage {
                        text: "Select a node on the canvas first.".to_string(),
                        error: true,
                    });
                    return;
                }
            },
            other => other,
        };
        self.finish(id, result, Some(ctx));
    }

    fn finish(&mut self, id: &str, result: Result<Outcome, CommandError>, ctx: Option<&Context>) {
        match result {
            Ok(Outcome::Done) => {
                if id == "run.stream" || id == "run.node" {
                    self.status = None;
                }
                if id == "file.add_data_folder" || id == "file.clear_data_folders" {
                    self.state.data_folders = self.session.data_folders().to_vec();
                    self.status = Some(StatusMessage {
                        text: match self.state.data_folders.last() {
                            Some(folder) if id == "file.add_data_folder" => format!(
                                "Data folder added: {} (run again to use it)",
                                folder.display()
                            ),
                            _ => "Data folders cleared".to_string(),
                        },
                        error: false,
                    });
                }
                if id == "file.open" || id == "file.close" {
                    self.document_changed(id == "file.open");
                    self.status = self.session.document().map(|document| StatusMessage {
                        text: format!("Opened {}", document.path.display()),
                        error: false,
                    });
                }
            }
            Ok(Outcome::View(action)) => self.apply_view(action, ctx),
            Ok(Outcome::Quit) => {
                if let Some(ctx) = ctx {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
            Ok(Outcome::NeedsArgument(name)) => {
                self.status = Some(StatusMessage {
                    text: format!("{id} needs {name}"),
                    error: true,
                });
            }
            Err(error) => {
                self.status = Some(StatusMessage {
                    text: error.to_string(),
                    error: true,
                });
            }
        }
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
        self.poll_runs();
        let title = match self.session.document() {
            Some(document) => format!(
                "{} — Data Mining",
                document
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
            None => "Data Mining".to_string(),
        };
        if self.title.as_deref() != Some(title.as_str()) {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = Some(title);
        }
        if !platform.native_menu {
            requested.extend(menu::take_shortcuts(ui.ctx(), platform.mac));
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
                if ui.button("Open…").clicked() {
                    requested.push("file.open");
                }
                ui.separator();
                let has_document = self.session.document().is_some();
                ui.add_enabled_ui(has_document, |ui| {
                    if ui.button("−").on_hover_text("Zoom Out").clicked() {
                        requested.push("view.zoom_out");
                    }
                    if ui.button("Fit").on_hover_text("Zoom to Fit").clicked() {
                        requested.push("view.zoom_fit");
                    }
                    if ui.button("+").on_hover_text("Zoom In").clicked() {
                        requested.push("view.zoom_in");
                    }
                });
                ui.separator();
                for (id, label, hint) in [
                    ("run.stream", "Run", "Run Stream"),
                    (
                        "run.node",
                        "Run Selected",
                        "Run the selected node and what it needs",
                    ),
                    ("run.cancel", "Stop", "Stop the run"),
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
                ui.toggle_value(&mut self.state.show_palette, "Nodes");
                ui.toggle_value(&mut self.state.show_properties, "Properties");
                ui.toggle_value(&mut self.state.show_results, "Results");
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
                .show(ui, palette_panel);
        }
        if self.state.show_properties {
            egui::Panel::right("properties")
                .resizable(true)
                .default_size(230.0)
                .show(ui, |ui| self.properties_panel(ui));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.canvas_area(ui, &mut requested));

        if self.about_open {
            egui::Window::new("About")
                .open(&mut self.about_open)
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.heading("Data Mining");
                    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                    ui.label("Opens and runs SPSS Modeler streams (.str).");
                });
        }
        requested
    }

    fn canvas_area(&mut self, ui: &mut Ui, requested: &mut Vec<&'static str>) {
        let rect = ui.available_rect_before_wrap();
        self.canvas_rect = Some(rect);
        let Some(graph) = &self.graph else {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(rect.height() * 0.4);
                    ui.label(RichText::new("No stream open").heading());
                    ui.label("Open a stream, or drop a .str file on this window.");
                    if ui.button("Open…").clicked() {
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
        canvas::show(ui, graph, &mut self.state.canvas, &marks);
    }

    fn properties_panel(&self, ui: &mut Ui) {
        ui.heading("Properties");
        ui.separator();
        let node = self.state.canvas.selected.as_ref().and_then(|uid| {
            self.graph
                .as_ref()
                .and_then(|g| g.nodes.iter().find(|n| &n.uid == uid))
        });
        let Some(node) = node else {
            ui.weak("Select a node to see its settings.");
            return;
        };
        ui.label(RichText::new(&node.title).strong());
        egui::Grid::new("node_facts").num_columns(2).show(ui, |ui| {
            ui.weak("Type");
            ui.label(&node.type_id);
            ui.end_row();
            ui.weak("Status");
            ui.label(node.status.explanation());
            ui.end_row();
            ui.weak("Id");
            ui.label(&node.uid);
            ui.end_row();
        });
        if let Some(Err(error)) = self
            .session
            .results()
            .and_then(|r| r.report.results.get(&node.uid))
        {
            ui.separator();
            ui.colored_label(ui.visuals().error_fg_color, "Last run failed here");
            ui.label(results::error_text(error));
        }
        ui.separator();
        ui.weak("Settings forms arrive in a later version.");
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
                    Some(title) => format!("Running {title}"),
                    None => "Running".to_string(),
                });
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .desired_width(140.0)
                        .text(format!("{}/{}", progress.done, progress.total)),
                );
                if ui.small_button("Stop").clicked() {
                    command = Some("run.cancel");
                }
                ui.separator();
            }
            match (&self.status, self.session.document()) {
                (Some(message), _) if message.error => {
                    ui.colored_label(ui.visuals().error_fg_color, &message.text);
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
                        ui.label(&message.text);
                    }
                }
                _ => {
                    ui.weak("Ready");
                }
            }
            if let Some(graph) = &self.graph {
                ui.separator();
                ui.label(format!(
                    "{} nodes · {} links",
                    graph.nodes.len(),
                    graph.links.len()
                ));
                let unsupported = graph.unsupported();
                if unsupported > 0 {
                    ui.separator();
                    ui.label(format!("{unsupported} not supported yet"))
                        .on_hover_text(
                            "These nodes are kept as saved and shown with a dashed outline.",
                        );
                }
                ui.separator();
                ui.label(format!("{:.0}%", self.state.canvas.zoom * 100.0));
            }
            if let Some(ms) = platform.frame_cpu_ms {
                ui.separator();
                ui.monospace(format!("{ms:.1} ms"));
            }
        });
        command
    }
}

fn palette_panel(ui: &mut Ui) {
    ui.heading("Nodes");
    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| {
        for palette in Palette::ALL {
            let specs: Vec<_> = NODES.iter().filter(|s| s.palette == palette).collect();
            let ready = specs.iter().filter(|s| nodes::is_implemented(s.id)).count();
            egui::CollapsingHeader::new(format!("{} ({ready}/{})", palette.label(), specs.len()))
                .default_open(palette == Palette::Source)
                .show(ui, |ui| {
                    for spec in specs {
                        let implemented = nodes::is_implemented(spec.id);
                        let response = ui.add_enabled(
                            implemented,
                            egui::Label::new(spec.label).selectable(false),
                        );
                        if !implemented {
                            response.on_disabled_hover_text(unavailable_reason(spec.scope));
                        }
                    }
                });
        }
    });
}

fn unavailable_reason(scope: Scope) -> &'static str {
    match scope {
        Scope::Mvp => "Not implemented yet",
        Scope::Later => "Planned for a later release",
        Scope::LicenseBlocked => "Cannot be verified: blocked by the reference licence",
        Scope::OutOfScope => "Not planned",
    }
}
