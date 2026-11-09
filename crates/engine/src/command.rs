//! Command registry: every user-visible operation is a command with a stable id. The desktop
//! menus (native on macOS, in-window elsewhere), keyboard shortcuts, the CLI and a future MCP
//! server all name commands by id; none of them carries behaviour of its own.
//!
//! A command is pure data here ([`CommandSpec`]); [`crate::session::Session::execute`] runs it.
//! Commands that act on view state the engine does not own (zoom, full screen) return a
//! [`crate::session::Outcome`] that the interface applies. Commands that need an argument a
//! person would pick in a dialog (a file path) return `NeedsArgument` when called without it,
//! so a program calling the engine never gets a dialog.
//!
//! Phase A spike (docs/gui/DESIGN.md §5): the table covers the commands the spike shows. Labels
//! are English source strings that will become translation keys.

/// Menu a command appears in. `App` is the application menu on macOS; elsewhere its commands
/// move to `File` (quit, settings) and `Help` (about), see [`Menu::on_platform`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Menu {
    App,
    File,
    Edit,
    View,
    Run,
    Window,
    Help,
}

impl Menu {
    pub const ALL: [Menu; 7] = [
        Menu::App,
        Menu::File,
        Menu::Edit,
        Menu::View,
        Menu::Run,
        Menu::Window,
        Menu::Help,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Menu::App => "Data Mining",
            Menu::File => "File",
            Menu::Edit => "Edit",
            Menu::View => "View",
            Menu::Run => "Run",
            Menu::Window => "Window",
            Menu::Help => "Help",
        }
    }
}

/// Keys that commands bind. Platform-neutral: the interface maps them to its toolkit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    N,
    O,
    S,
    W,
    Q,
    Z,
    R,
    F,
    Comma,
    Minus,
    Equals,
    Num0,
    F11,
}

/// A key with modifiers. `command` is ⌘ on macOS and Ctrl elsewhere; `mac_control` is the
/// Control key on macOS (⌃), used only by macOS-specific bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub key: Key,
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
    pub mac_control: bool,
}

impl Shortcut {
    /// ⌘/Ctrl + key.
    pub const fn command(key: Key) -> Self {
        Shortcut {
            key,
            command: true,
            shift: false,
            alt: false,
            mac_control: false,
        }
    }

    /// ⌘⇧/Ctrl+Shift + key.
    pub const fn command_shift(key: Key) -> Self {
        Shortcut {
            shift: true,
            ..Shortcut::command(key)
        }
    }

    /// A key without modifiers.
    pub const fn plain(key: Key) -> Self {
        Shortcut {
            key,
            command: false,
            shift: false,
            alt: false,
            mac_control: false,
        }
    }
}

/// Shortcuts per platform family: macOS follows the Apple HIG, Windows its own conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcuts {
    pub mac: Option<Shortcut>,
    pub other: Option<Shortcut>,
}

impl Shortcuts {
    pub const NONE: Shortcuts = Shortcuts {
        mac: None,
        other: None,
    };

    /// The same binding on every platform (⌘ on macOS is Ctrl elsewhere).
    pub const fn both(shortcut: Shortcut) -> Self {
        Shortcuts {
            mac: Some(shortcut),
            other: Some(shortcut),
        }
    }

    pub fn for_platform(self, mac: bool) -> Option<Shortcut> {
        if mac { self.mac } else { self.other }
    }
}

/// What must hold for a command to be enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requires {
    Nothing,
    /// An open stream.
    Document,
    /// Not available yet; the reason is shown as a tooltip.
    Planned(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    /// Stable id, `<area>.<verb>`; never reused for another meaning.
    pub id: &'static str,
    pub label: &'static str,
    pub menu: Menu,
    /// Items of a menu with different groups are separated; groups are shown in ascending order.
    pub group: u8,
    pub shortcuts: Shortcuts,
    pub requires: Requires,
}

const PHASE_B: &str = "Arrives with stream editing (phase B)";
const PHASE_C: &str = "Arrives with stream execution in the interface";

macro_rules! command {
    ($id:literal, $label:literal, $menu:ident, $group:literal, $shortcuts:expr, $requires:expr) => {
        CommandSpec {
            id: $id,
            label: $label,
            menu: Menu::$menu,
            group: $group,
            shortcuts: $shortcuts,
            requires: $requires,
        }
    };
}

/// Every command, in menu order.
pub const COMMANDS: &[CommandSpec] = &[
    command!(
        "app.about",
        "About Data Mining",
        App,
        0,
        Shortcuts::NONE,
        Requires::Nothing
    ),
    command!(
        "app.settings",
        "Settings…",
        App,
        1,
        Shortcuts {
            mac: Some(Shortcut::command(Key::Comma)),
            other: None
        },
        Requires::Planned(PHASE_B)
    ),
    command!(
        "app.quit",
        "Quit Data Mining",
        App,
        9,
        Shortcuts {
            mac: Some(Shortcut::command(Key::Q)),
            other: None
        },
        Requires::Nothing
    ),
    command!(
        "file.new",
        "New Stream",
        File,
        0,
        Shortcuts::both(Shortcut::command(Key::N)),
        Requires::Planned(PHASE_B)
    ),
    command!(
        "file.open",
        "Open…",
        File,
        0,
        Shortcuts::both(Shortcut::command(Key::O)),
        Requires::Nothing
    ),
    command!(
        "file.close",
        "Close",
        File,
        1,
        Shortcuts::both(Shortcut::command(Key::W)),
        Requires::Document
    ),
    command!(
        "file.save",
        "Save",
        File,
        1,
        Shortcuts::both(Shortcut::command(Key::S)),
        Requires::Planned(PHASE_B)
    ),
    command!(
        "file.save_as",
        "Save As…",
        File,
        1,
        Shortcuts::both(Shortcut::command_shift(Key::S)),
        Requires::Planned(PHASE_B)
    ),
    command!(
        "edit.undo",
        "Undo",
        Edit,
        0,
        Shortcuts::both(Shortcut::command(Key::Z)),
        Requires::Planned(PHASE_B)
    ),
    command!(
        "edit.redo",
        "Redo",
        Edit,
        0,
        Shortcuts {
            mac: Some(Shortcut::command_shift(Key::Z)),
            other: Some(Shortcut::command_shift(Key::Z))
        },
        Requires::Planned(PHASE_B)
    ),
    command!(
        "view.zoom_in",
        "Zoom In",
        View,
        0,
        Shortcuts::both(Shortcut::command(Key::Equals)),
        Requires::Document
    ),
    command!(
        "view.zoom_out",
        "Zoom Out",
        View,
        0,
        Shortcuts::both(Shortcut::command(Key::Minus)),
        Requires::Document
    ),
    command!(
        "view.zoom_fit",
        "Zoom to Fit",
        View,
        0,
        Shortcuts::both(Shortcut::command(Key::Num0)),
        Requires::Document
    ),
    command!(
        "view.fullscreen",
        "Enter Full Screen",
        View,
        1,
        Shortcuts {
            mac: Some(Shortcut {
                mac_control: true,
                ..Shortcut::command(Key::F)
            }),
            other: Some(Shortcut::plain(Key::F11))
        },
        Requires::Nothing
    ),
    command!(
        "run.stream",
        "Run Stream",
        Run,
        0,
        Shortcuts::both(Shortcut::command(Key::R)),
        Requires::Planned(PHASE_C)
    ),
];

pub fn by_id(id: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|spec| spec.id == id)
}

impl Menu {
    /// Where a command of this menu appears on a platform without an application menu.
    pub fn on_platform(self, mac: bool, id: &str) -> Menu {
        match self {
            Menu::App if !mac && id == "app.about" => Menu::Help,
            Menu::App if !mac => Menu::File,
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_and_well_formed() {
        let mut seen = HashSet::new();
        for spec in COMMANDS {
            assert!(seen.insert(spec.id), "duplicate id {}", spec.id);
            let (area, verb) = spec.id.split_once('.').unwrap();
            assert!(!area.is_empty() && !verb.is_empty(), "{}", spec.id);
            assert!(
                spec.id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_'),
                "{}",
                spec.id
            );
            assert!(!spec.label.is_empty());
            assert_eq!(by_id(spec.id), Some(spec));
        }
        assert_eq!(by_id("no.such"), None);
    }

    #[test]
    fn shortcuts_do_not_collide_per_platform() {
        for mac in [true, false] {
            let mut seen = HashSet::new();
            for spec in COMMANDS {
                if let Some(shortcut) = spec.shortcuts.for_platform(mac) {
                    assert!(
                        seen.insert(shortcut),
                        "{} reuses {shortcut:?} (mac: {mac})",
                        spec.id
                    );
                }
            }
        }
    }

    #[test]
    fn mac_bindings_follow_the_hig() {
        let mac = |id: &str| by_id(id).unwrap().shortcuts.mac.unwrap();
        assert_eq!(mac("file.new"), Shortcut::command(Key::N));
        assert_eq!(mac("file.open"), Shortcut::command(Key::O));
        assert_eq!(mac("file.save"), Shortcut::command(Key::S));
        assert_eq!(mac("file.save_as"), Shortcut::command_shift(Key::S));
        assert_eq!(mac("edit.undo"), Shortcut::command(Key::Z));
        assert_eq!(mac("edit.redo"), Shortcut::command_shift(Key::Z));
        assert_eq!(mac("file.close"), Shortcut::command(Key::W));
        assert_eq!(mac("app.quit"), Shortcut::command(Key::Q));
        assert_eq!(mac("app.settings"), Shortcut::command(Key::Comma));
        assert_eq!(mac("run.stream"), Shortcut::command(Key::R));
    }

    #[test]
    fn app_menu_moves_off_macos() {
        assert_eq!(Menu::App.on_platform(true, "app.quit"), Menu::App);
        assert_eq!(Menu::App.on_platform(false, "app.quit"), Menu::File);
        assert_eq!(Menu::App.on_platform(false, "app.about"), Menu::Help);
        assert_eq!(Menu::View.on_platform(false, "view.zoom_in"), Menu::View);
    }
}
