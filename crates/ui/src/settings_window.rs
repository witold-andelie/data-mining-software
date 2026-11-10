//! Application preferences. Folder changes are returned to the command host.
use crate::i18n::{self, Language, tr};
use crate::workbench::ShellState;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Action {
    AddFolder,
    RemoveFolder(usize),
}

pub(crate) fn show(ctx: &egui::Context, open: &mut bool, state: &mut ShellState) -> Option<Action> {
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
                Language::English => tr("English"),
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
        });
    action
}
