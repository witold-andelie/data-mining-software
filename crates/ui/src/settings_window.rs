//! Application preferences. Folder and Python changes are returned to the command host.
use crate::i18n::{self, Language, tr, tr_fmt};
use crate::workbench::ShellState;
use dms_python::{BridgeError, PythonInfo};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

#[derive(Debug, Clone, Copy)]
pub(crate) enum Action {
    AddFolder,
    RemoveFolder(usize),
    PythonChanged,
}

/// The last "Check Python" result; detection runs on its own thread (it starts Python).
#[derive(Default)]
pub(crate) struct PythonCheck {
    running: Option<Receiver<Result<PythonInfo, BridgeError>>>,
    result: Option<Result<PythonInfo, BridgeError>>,
}

impl PythonCheck {
    fn start(&mut self, ctx: &egui::Context, configured: &str) {
        let (send, receive) = channel();
        let configured = PathBuf::from(configured.trim());
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("python-check".into())
            .spawn(move || {
                dms_python::forget_resolved();
                let found = (!configured.as_os_str().is_empty()).then_some(configured.as_path());
                let result = dms_python::detect(found, &dms_types::CancelToken::new());
                let _ = send.send(result);
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => {
                self.running = Some(receive);
                self.result = None;
            }
            Err(error) => self.result = Some(Err(BridgeError::Process(error.to_string()))),
        }
    }

    fn poll(&mut self) {
        if let Some(receive) = &self.running {
            match receive.try_recv() {
                Ok(result) => {
                    self.result = Some(result);
                    self.running = None;
                }
                Err(TryRecvError::Disconnected) => self.running = None,
                Err(TryRecvError::Empty) => {}
            }
        }
    }
}

fn python_section(
    ui: &mut egui::Ui,
    state: &mut ShellState,
    check: &mut PythonCheck,
    action: &mut Option<Action>,
) {
    ui.strong(tr("Python"));
    ui.label(tr(
        "Python nodes run in your own Python 3.9 or later with pyarrow and pandas installed. Leave the path empty to find Python automatically.",
    ));
    let response = ui.add(
        egui::TextEdit::singleline(&mut state.python)
            .hint_text(tr("Automatic"))
            .desired_width(f32::INFINITY),
    );
    let response = response.on_hover_text(tr(
        "The Python executable, for example /usr/local/bin/python3",
    ));
    if response.lost_focus() {
        *action = Some(Action::PythonChanged);
    }
    check.poll();
    ui.horizontal(|ui| {
        let running = check.running.is_some();
        if ui
            .add_enabled(!running, egui::Button::new(tr("Check Python")))
            .clicked()
        {
            *action = Some(Action::PythonChanged);
            check.start(ui.ctx(), &state.python);
        }
        if running {
            ui.spinner();
        }
    });
    match &check.result {
        Some(Ok(info)) => {
            ui.label(tr_fmt(
                "Python {version} at {path}",
                &[
                    ("version", &info.version_text()),
                    ("path", &info.executable),
                ],
            ));
            egui::Grid::new("python-packages")
                .num_columns(2)
                .show(ui, |ui| {
                    for (name, version) in &info.packages {
                        ui.monospace(name);
                        match version {
                            Some(version) => ui.label(version),
                            None => ui.weak(tr("not installed")),
                        };
                        ui.end_row();
                    }
                });
            if let Err(error) = info.check() {
                ui.colored_label(ui.visuals().error_fg_color, error.to_string());
            } else {
                ui.label(tr("Ready for Python nodes."));
            }
        }
        Some(Err(error)) => {
            ui.colored_label(ui.visuals().error_fg_color, error.to_string());
        }
        None => {}
    }
}

pub(crate) fn show(
    ctx: &egui::Context,
    open: &mut bool,
    state: &mut ShellState,
    check: &mut PythonCheck,
) -> Option<Action> {
    let mut action = None;
    egui::Window::new(tr("Settings"))
        .id(egui::Id::unique("app-settings"))
        .open(open)
        .collapsible(false)
        .default_width(460.0)
        .show(ctx, |ui| {
            let before = state.language;
            let label = |language| match language {
                Language::System => tr("System"),
                // Language names are autonyms, so they read the same in every language.
                Language::English => "English",
                Language::SimplifiedChinese => "简体中文",
            };
            egui::ComboBox::from_label(tr("Language"))
                .selected_text(label(state.language))
                .show_ui(ui, |ui| {
                    for language in [
                        Language::System,
                        Language::English,
                        Language::SimplifiedChinese,
                    ] {
                        ui.selectable_value(&mut state.language, language, label(language));
                    }
                });
            if state.language != before {
                i18n::set_language(state.language);
                ctx.request_repaint();
            }
            ui.separator();
            ui.strong(tr("Data Folders"));
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    for (index, folder) in state.data_folders.iter().enumerate() {
                        ui.push_id(index, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(folder.display().to_string());
                                if ui.button(tr("Remove")).clicked() {
                                    action = Some(Action::RemoveFolder(index));
                                }
                            });
                        });
                    }
                });
            if ui.button(tr("Add…")).clicked() {
                action = Some(Action::AddFolder);
            }
            ui.separator();
            python_section(ui, state, check, &mut action);
        });
    action
}
