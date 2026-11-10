//! The native macOS menu bar (muda), built from the engine's command registry. Choosing an
//! item, or pressing its key equivalent, yields the command id; the window then dispatches it
//! like any other request. Elsewhere the window draws the same menus (`dms_ui::menu`).

use dms_engine::command::{CommandSpec, Key, Menu, Shortcut};
use dms_engine::session::Session;
use dms_ui::i18n::{self, Language, tr, tr_fmt};
use dms_ui::menu::{command_label, menu_items};
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use std::sync::mpsc;

type CommandItems = Vec<(&'static str, MenuItem)>;

pub struct NativeMenu {
    /// Kept alive for as long as the menu is installed.
    _menu: muda::Menu,
    items: CommandItems,
    enabled: Vec<bool>,
    labels: Vec<String>,
    events: mpsc::Receiver<MenuEvent>,
    language: Language,
}

impl NativeMenu {
    /// Builds the menu bar and installs it as the application's main menu. Menu events wake the
    /// event loop so they are handled at once.
    pub fn install(ctx: &eframe::egui::Context) -> Result<NativeMenu, muda::Error> {
        let (menu, items) = Self::build()?;

        let (sender, events) = mpsc::channel();
        let repaint = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            // The receiver lives as long as the app; a failed send only means it is closing.
            let _ = sender.send(event);
            repaint.request_repaint();
        }));
        let enabled = vec![false; items.len()];
        let labels = vec![String::new(); items.len()];
        Ok(NativeMenu {
            _menu: menu,
            items,
            enabled,
            labels,
            events,
            language: i18n::language(),
        })
    }

    /// Rebuild labels while retaining the original event channel and handler.
    pub fn rebuild_if_language_changed(&mut self) -> Result<(), muda::Error> {
        if self.language != i18n::language() {
            let (menu, items) = Self::build()?;
            self._menu = menu;
            self.enabled = vec![false; items.len()];
            self.labels = vec![String::new(); items.len()];
            self.items = items;
            self.language = i18n::language();
        }
        Ok(())
    }

    fn build() -> Result<(muda::Menu, CommandItems), muda::Error> {
        let menu = muda::Menu::new();
        let mut items = Vec::new();
        for kind in Menu::ALL {
            let submenu = Submenu::new(tr(kind.label()), true);
            let commands = menu_items(kind, true);
            let mut group = None;
            for spec in &commands {
                if group.is_some_and(|g| g != spec.group) {
                    submenu.append(&PredefinedMenuItem::separator())?;
                }
                group = Some(spec.group);
                // Standard application-menu items sit between Settings and Quit.
                if kind == Menu::App && spec.id == "app.quit" {
                    let hide = PredefinedMenuItem::hide(None);
                    let source = hide.text();
                    hide.set_text(match source.strip_prefix("Hide ") {
                        Some(app) => tr_fmt("Hide {app}", &[("app", app)]),
                        None => tr("Hide").into(),
                    });
                    submenu.append_items(&[
                        &PredefinedMenuItem::services(Some(tr("Services"))),
                        &PredefinedMenuItem::separator(),
                        &hide,
                        &PredefinedMenuItem::hide_others(Some(tr("Hide Others"))),
                        &PredefinedMenuItem::show_all(Some(tr("Show All"))),
                        &PredefinedMenuItem::separator(),
                    ])?;
                }
                let item = MenuItem::with_id(
                    spec.id,
                    display_label(spec, tr(spec.label)),
                    false,
                    accelerator(spec),
                );
                submenu.append(&item)?;
                items.push((spec.id, item));
            }
            match kind {
                Menu::Window => {
                    submenu.append_items(&[
                        &PredefinedMenuItem::minimize(Some(tr("Minimize"))),
                        &PredefinedMenuItem::maximize(Some(tr("Zoom"))),
                    ])?;
                    submenu.set_as_windows_menu_for_nsapp();
                }
                Menu::Help => submenu.set_as_help_menu_for_nsapp(),
                _ => {}
            }
            menu.append(&submenu)?;
        }
        menu.init_for_nsapp();

        Ok((menu, items))
    }

    /// Commands chosen since the last call.
    pub fn take_commands(&self) -> Vec<&'static str> {
        self.events
            .try_iter()
            .filter_map(|event| {
                self.items
                    .iter()
                    .find(|(id, _)| event.id() == *id)
                    .map(|(id, _)| *id)
            })
            .collect()
    }

    /// Enables exactly the items whose commands can run now.
    pub fn sync_enabled(&mut self, session: &Session, text_focus: bool) {
        for (((id, item), was), label) in self
            .items
            .iter()
            .zip(self.enabled.iter_mut())
            .zip(self.labels.iter_mut())
        {
            let now = session.is_enabled(id)
                || (text_focus
                    && matches!(
                        *id,
                        "edit.copy" | "edit.cut" | "edit.paste" | "edit.select_all"
                    ));
            if now != *was {
                item.set_enabled(now);
                *was = now;
            }
            if let Some(spec) = dms_engine::command::by_id(id) {
                let next = display_label(spec, &command_label(session, spec));
                if *label != next {
                    item.set_text(&next);
                    *label = next;
                }
            }
        }
    }
}

fn display_label(spec: &CommandSpec, source: &str) -> String {
    match spec.shortcuts.mac.filter(|s| !s.command).map(|s| s.key) {
        Some(key) => format!(
            "{source}\t{}",
            match key {
                Key::Backspace => "⌫",
                Key::Delete => "Delete",
                Key::Enter => "↩",
                Key::F2 => "F2",
                Key::Escape => "Esc",
                _ => "",
            }
        ),
        None => source.into(),
    }
}

fn accelerator(spec: &CommandSpec) -> Option<Accelerator> {
    let Shortcut {
        key,
        command,
        shift,
        alt,
        mac_control,
    } = spec.shortcuts.mac?;
    if !command {
        return None;
    }
    let mut modifiers = Modifiers::empty();
    if command {
        modifiers |= Modifiers::META;
    }
    if shift {
        modifiers |= Modifiers::SHIFT;
    }
    if alt {
        modifiers |= Modifiers::ALT;
    }
    if mac_control {
        modifiers |= Modifiers::CONTROL;
    }
    let code = match key {
        Key::N => Code::KeyN,
        Key::O => Code::KeyO,
        Key::S => Code::KeyS,
        Key::W => Code::KeyW,
        Key::Q => Code::KeyQ,
        Key::Z => Code::KeyZ,
        Key::R => Code::KeyR,
        Key::F => Code::KeyF,
        Key::Comma => Code::Comma,
        Key::Period => Code::Period,
        Key::Escape => Code::Escape,
        Key::Minus => Code::Minus,
        Key::Equals => Code::Equal,
        Key::Num0 => Code::Digit0,
        Key::F11 => Code::F11,
        Key::X => Code::KeyX,
        Key::C => Code::KeyC,
        Key::V => Code::KeyV,
        Key::D => Code::KeyD,
        Key::A => Code::KeyA,
        Key::Backspace => Code::Backspace,
        Key::Delete => Code::Delete,
        Key::Enter => Code::Enter,
        Key::F2 => Code::F2,
    };
    Some(Accelerator::new(modifiers, code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_keys_are_labels_without_native_accelerators() {
        for spec in dms_engine::command::COMMANDS {
            if spec.shortcuts.mac.is_some_and(|s| !s.command) {
                assert!(accelerator(spec).is_none());
                assert!(display_label(spec, spec.label).contains('\t'));
            }
        }
    }
}
