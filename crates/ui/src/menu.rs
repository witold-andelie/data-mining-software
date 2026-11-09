//! Menus and shortcuts generated from the engine's command registry. On macOS the application
//! builds the native menu bar from the same registry (apps/dms) and the window shows no menu;
//! elsewhere [`menu_bar`] draws it in the window and [`take_shortcuts`] handles the keys.

use dms_engine::command::{COMMANDS, CommandSpec, Key, Menu, Shortcut};
use dms_engine::session::Session;
use egui::{Button, KeyboardShortcut, Modifiers, Ui};

/// The egui form of a registry shortcut.
pub fn keyboard_shortcut(shortcut: Shortcut, mac: bool) -> KeyboardShortcut {
    let mut modifiers = Modifiers::NONE;
    if shortcut.command {
        modifiers |= Modifiers::COMMAND;
    }
    if shortcut.shift {
        modifiers |= Modifiers::SHIFT;
    }
    if shortcut.alt {
        modifiers |= Modifiers::ALT;
    }
    if shortcut.mac_control && mac {
        modifiers |= Modifiers::CTRL;
    }
    KeyboardShortcut::new(modifiers, egui_key(shortcut.key))
}

fn egui_key(key: Key) -> egui::Key {
    match key {
        Key::N => egui::Key::N,
        Key::O => egui::Key::O,
        Key::S => egui::Key::S,
        Key::W => egui::Key::W,
        Key::Q => egui::Key::Q,
        Key::Z => egui::Key::Z,
        Key::R => egui::Key::R,
        Key::F => egui::Key::F,
        Key::Comma => egui::Key::Comma,
        Key::Minus => egui::Key::Minus,
        Key::Equals => egui::Key::Equals,
        Key::Num0 => egui::Key::Num0,
        Key::F11 => egui::Key::F11,
    }
}

/// Commands whose shortcut was pressed this frame (the shortcut is consumed). Shortcuts with
/// more modifiers are tried first, so ⇧⌘S is not taken for ⌘S.
pub fn take_shortcuts(ctx: &egui::Context, mac: bool) -> Vec<&'static str> {
    let mut bound: Vec<(&'static str, KeyboardShortcut)> = COMMANDS
        .iter()
        .filter_map(|spec| {
            spec.shortcuts
                .for_platform(mac)
                .map(|s| (spec.id, keyboard_shortcut(s, mac)))
        })
        .collect();
    bound.sort_by_key(|(_, s)| {
        std::cmp::Reverse(
            u8::from(s.modifiers.shift) + u8::from(s.modifiers.alt) + u8::from(s.modifiers.ctrl),
        )
    });
    let mut fired = Vec::new();
    ctx.input_mut(|input| {
        for (id, shortcut) in &bound {
            if input.consume_shortcut(shortcut) {
                fired.push(*id);
            }
        }
    });
    fired
}

/// Commands of `menu` on this platform, grouped in display order.
pub fn menu_items(menu: Menu, mac: bool) -> Vec<&'static CommandSpec> {
    let mut items: Vec<&'static CommandSpec> = COMMANDS
        .iter()
        .filter(|spec| spec.menu.on_platform(mac, spec.id) == menu)
        .collect();
    items.sort_by_key(|spec| spec.group);
    items
}

/// The in-window menu bar (all platforms but macOS). Returns the commands chosen.
pub fn menu_bar(ui: &mut Ui, session: &Session, mac: bool) -> Vec<&'static str> {
    let mut chosen = Vec::new();
    egui::MenuBar::new().ui(ui, |ui| {
        for menu in Menu::ALL {
            let items = menu_items(menu, mac);
            if items.is_empty() {
                continue;
            }
            ui.menu_button(menu.label(), |ui| {
                let mut group = None;
                for spec in items {
                    if group.is_some_and(|g| g != spec.group) {
                        ui.separator();
                    }
                    group = Some(spec.group);
                    let mut button = Button::new(spec.label);
                    if let Some(shortcut) = spec.shortcuts.for_platform(mac) {
                        button = button.shortcut_text(
                            ui.ctx().format_shortcut(&keyboard_shortcut(shortcut, mac)),
                        );
                    }
                    let availability = session.availability(spec.id);
                    let response = ui.add_enabled(availability.is_ok(), button);
                    let response = match &availability {
                        Err(reason) => response.on_disabled_hover_text(reason),
                        Ok(()) => response,
                    };
                    if response.clicked() {
                        chosen.push(spec.id);
                        ui.close();
                    }
                }
            });
        }
    });
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_a_menu_on_both_platforms() {
        for mac in [true, false] {
            let listed: usize = Menu::ALL.iter().map(|m| menu_items(*m, mac).len()).sum();
            assert_eq!(listed, COMMANDS.len());
        }
        assert!(menu_items(Menu::App, false).is_empty());
    }

    #[test]
    fn command_maps_to_cmd_on_mac_and_ctrl_elsewhere() {
        let save_as = Shortcut::command_shift(Key::S);
        let s = keyboard_shortcut(save_as, true);
        assert_eq!(s.modifiers, Modifiers::COMMAND | Modifiers::SHIFT);
        assert_eq!(s.logical_key, egui::Key::S);
    }
}
