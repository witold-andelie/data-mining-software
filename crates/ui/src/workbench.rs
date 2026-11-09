//! The main window: node palette, stream canvas, properties, status bar, and the dispatch of
//! commands by id. Lives in the UI library (not the app) so the whole window can be rendered
//! off-screen in tests. Layout: docs/gui/DESIGN.md §2.

use crate::canvas::{self, CanvasView, ZOOM_STEP};
use crate::graph::StreamGraph;
use crate::menu;
use dms_engine::catalog::{NODES, Palette, Scope};
use dms_engine::nodes;
use dms_engine::session::{Arg, CommandError, Document, Outcome, Session, ViewAction};
use egui::{Context, Rect, RichText, Ui, ViewportCommand};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Interface state that survives restarts (stored by the app, see DESIGN.md §7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShellState {
    pub show_palette: bool,
    pub show_properties: bool,
    pub canvas: CanvasView,
}

impl Default for ShellState {
    fn default() -> Self {
        ShellState {
            show_palette: true,
            show_properties: true,
            canvas: CanvasView::default(),
        }
    }
}

impl ShellState {
    pub fn sanitize(&mut self) {
        self.canvas.sanitize();
    }
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
}

impl Workbench {
    pub fn new(state: ShellState) -> Self {
        Workbench {
            session: Session::new(),
            state,
            graph: None,
            status: None,
            about_open: false,
            fit_pending: false,
            canvas_rect: None,
        }
    }

    /// A workbench showing an already decoded stream, zoomed to fit on the first frame.
    pub fn with_document(document: Document, state: ShellState) -> Self {
        let mut workbench = Workbench::new(state);
        workbench.session = Session::with_document(document);
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
        self.fit_pending = fit && self.graph.is_some();
    }

    /// Opens a file directly (dropped on the window, or given on the command line).
    pub fn open_path(&mut self, path: PathBuf) {
        let result = self.session.execute("file.open", &[Arg::Path(path)]);
        self.finish("file.open", result, None);
    }

    /// Runs a command by id. `pick_path` is asked only when the command needs a path and the
    /// person has to choose one (a file dialog in the app, a fixed answer in tests).
    pub fn dispatch(
        &mut self,
        ctx: &Context,
        id: &str,
        pick_path: &mut dyn FnMut() -> Option<PathBuf>,
    ) {
        let result = match self.session.execute(id, &[]) {
            Ok(Outcome::NeedsArgument("path")) => match pick_path() {
                Some(path) => self.session.execute(id, &[Arg::Path(path)]),
                None => return,
            },
            other => other,
        };
        self.finish(id, result, Some(ctx));
    }

    fn finish(&mut self, id: &str, result: Result<Outcome, CommandError>, ctx: Option<&Context>) {
        match result {
            Ok(Outcome::Done) => {
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
                ui.toggle_value(&mut self.state.show_palette, "Nodes");
                ui.toggle_value(&mut self.state.show_properties, "Properties");
            });
        });

        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui, platform));

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
                    ui.label("Interface preview (phase A).");
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
        canvas::show(ui, graph, &mut self.state.canvas);
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
        ui.separator();
        ui.weak("Settings forms are generated from each node's settings schema (phase B).");
    }

    fn status_bar(&self, ui: &mut Ui, platform: &Platform) {
        ui.horizontal(|ui| {
            match (&self.status, self.session.document()) {
                (Some(message), _) if message.error => {
                    ui.colored_label(ui.visuals().error_fg_color, &message.text);
                }
                (_, Some(document)) => {
                    let name = document
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    ui.label(name);
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
