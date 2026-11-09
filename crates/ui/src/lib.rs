//! Desktop interface (L6): the stream canvas, panels and menus, drawn with egui. Thin and
//! data-driven: every action is an engine command run by id; interface state is plain
//! serializable data. See docs/gui/DESIGN.md.

pub mod canvas;
pub mod document_view;
pub mod graph;
pub mod menu;
pub mod synthetic;
pub mod theme;
pub mod workbench;

pub use workbench::{Platform, ShellState, Workbench};
