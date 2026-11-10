//! Headless interaction coverage: no GPU, rendering, or snapshot baselines.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use dms_engine::session::Document;
use dms_ui::workbench::{Clipboard, Pick};
use dms_ui::{Platform, ShellState, Workbench, synthetic};
use egui::{Key, Modifiers, Pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Default)]
struct Clip {
    text: String,
    writes: usize,
    fail: bool,
}
struct FakeClipboard(Rc<RefCell<Clip>>);
impl Clipboard for FakeClipboard {
    fn get_text(&mut self) -> Result<String, String> {
        Ok(self.0.borrow().text.clone())
    }
    fn set_text(&mut self, text: &str) -> Result<(), String> {
        let mut clip = self.0.borrow_mut();
        if clip.fail {
            return Err("Test clipboard unavailable".into());
        }
        clip.text = text.into();
        clip.writes += 1;
        Ok(())
    }
}
struct State {
    w: Workbench,
    clip: Rc<RefCell<Clip>>,
    save: Option<PathBuf>,
}
fn harness(linked: bool) -> Harness<'static, State> {
    let mut stream = synthetic::stream(2);
    stream.diagram.nodes[0].label = Some("Alpha".into());
    stream.diagram.nodes[1].label = Some("Beta".into());
    if !linked {
        stream.diagram.links.clear();
    }
    let mut w = Workbench::with_document(
        Document::from_stream("example.str".into(), stream),
        ShellState {
            show_properties: false,
            ..Default::default()
        },
    );
    let clip = Rc::new(RefCell::new(Clip::default()));
    w.set_clipboard(Box::new(FakeClipboard(clip.clone())));
    let mut h = Harness::builder()
        .with_size(vec2(1100.0, 680.0))
        .with_step_dt(0.05)
        .with_max_steps(20)
        .with_os(egui::os::OperatingSystem::Mac)
        .build_ui_state(
            |ui, s: &mut State| {
                let commands = s.w.ui(
                    ui,
                    &Platform {
                        mac: true,
                        native_menu: true,
                        ..Default::default()
                    },
                );
                let path = s.save.clone();
                for id in commands {
                    s.w.dispatch(ui.ctx(), id, &mut |kind| {
                        if kind == Pick::Save {
                            path.clone()
                        } else {
                            None
                        }
                    });
                }
            },
            State {
                w,
                clip,
                save: None,
            },
        );
    h.run();
    h
}
fn dispatch(h: &mut Harness<'_, State>, id: &str) {
    let ctx = h.ctx.clone();
    let path = h.state().save.clone();
    h.state_mut().w.dispatch(&ctx, id, &mut |kind| {
        if kind == Pick::Save {
            path.clone()
        } else {
            None
        }
    });
    h.run();
}
fn double_click(h: &mut Harness<'_, State>, label: &str) {
    h.get_by_label(label).click();
    h.run();
    h.get_by_label(label).click();
    h.run();
}
fn drag(h: &mut Harness<'_, State>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.drag_at(from);
    h.run();
    h.hover_at(to);
    h.run();
    h.drop_at(to);
    h.run();
}
fn document_text(h: &Harness<'_, State>) -> String {
    dms_stream::native::to_toml(&h.state().w.session.document_state().unwrap().doc).unwrap()
}
fn dirty(h: &mut Harness<'_, State>) {
    h.get_by_label("Alpha").click();
    h.run();
    dispatch(h, "edit.duplicate");
    assert!(h.state().w.session.is_dirty());
}

#[test]
fn double_click_palette_adds_at_visible_center() {
    let mut h = harness(false);
    let center = h.get_by_label("Stream canvas").rect().center();
    double_click(&mut h, "Var. File");
    let state = h.state().w.session.document_state().unwrap();
    assert_eq!(state.doc.stream.diagram.nodes.len(), 3);
    let node = state.doc.stream.diagram.nodes.last().unwrap();
    let graph_node = h
        .state()
        .w
        .graph()
        .unwrap()
        .nodes
        .iter()
        .find(|n| n.uid == node.uid)
        .unwrap();
    let canvas = h.get_by_label("Stream canvas").rect();
    assert!(
        h.state()
            .w
            .state
            .canvas
            .to_screen(canvas.min, graph_node.pos)
            .distance(center)
            < 2.0
    );
}

#[test]
fn palette_drag_adds_at_drop_position() {
    let mut h = harness(false);
    let from = h.get_by_label("Var. File").rect().center();
    let to = h.get_by_label("Stream canvas").rect().center() + vec2(30.0, 90.0);
    drag(&mut h, from, to);
    assert_eq!(
        h.state()
            .w
            .session
            .document()
            .unwrap()
            .stream
            .diagram
            .nodes
            .len(),
        3
    );
}

#[test]
fn move_is_one_undo_step_and_restores_positions() {
    let mut h = harness(false);
    let before = document_text(&h);
    let from = h.get_by_label("Alpha").rect().center();
    h.hover_at(from);
    h.drag_at(from);
    h.run();
    h.hover_at(from + vec2(20.0, 20.0));
    h.run();
    h.hover_at(from + vec2(75.0, 40.0));
    h.run();
    assert_eq!(
        h.state().w.session.revision(),
        0,
        "preview must not mutate the document"
    );
    h.drop_at(from + vec2(75.0, 40.0));
    h.run();
    assert_eq!(h.state().w.session.undo_label(), Some("Move"));
    dispatch(&mut h, "edit.undo");
    assert_eq!(document_text(&h), before);
    assert!(h.state().w.session.undo_label().is_none());
}

#[test]
fn connect_from_port_rejects_cycle_duplicate_and_self() {
    let mut h = harness(false);
    let from = h.get_by_label("Connect from Alpha").rect().center();
    let to = h.get_by_label("Beta").rect().center();
    drag(&mut h, from, to);
    assert_eq!(
        h.state()
            .w
            .session
            .document()
            .unwrap()
            .stream
            .diagram
            .links
            .len(),
        1
    );
    let revision = h.state().w.session.revision();
    let from = h.get_by_label("Connect from Beta").rect().center();
    let to = h.get_by_label("Alpha").rect().center();
    drag(&mut h, from, to);
    h.get_by_label("This connection would create a cycle.");
    assert_eq!(h.state().w.session.revision(), revision);
    let from = h.get_by_label("Connect from Alpha").rect().center();
    let to = h.get_by_label("Beta").rect().center();
    drag(&mut h, from, to);
    h.get_by_label("This connection already exists.");
    let from = h.get_by_label("Connect from Alpha").rect().center();
    let to = h.get_by_label("Alpha").rect().center();
    drag(&mut h, from, to);
    h.get_by_label("A node cannot connect to itself.");
    assert_eq!(h.state().w.session.revision(), revision);
}

#[test]
fn box_select_delete_and_undo_restores_nodes_and_link() {
    let mut h = harness(true);
    let before = document_text(&h);
    let a = h.get_by_label("Alpha").rect();
    let b = h.get_by_label("Beta").rect();
    let area = a.union(b).expand(18.0);
    drag(&mut h, area.min, area.max);
    assert_eq!(
        h.state()
            .w
            .session
            .document_state()
            .unwrap()
            .selection
            .len(),
        2
    );
    h.key_press(Key::Backspace);
    h.run();
    assert!(
        h.state()
            .w
            .session
            .document()
            .unwrap()
            .stream
            .diagram
            .nodes
            .is_empty()
    );
    dispatch(&mut h, "edit.undo");
    assert_eq!(document_text(&h), before);
}

#[test]
fn shift_click_toggles_selection_and_copy_paste_uses_host_clipboard() {
    let mut h = harness(true);
    h.get_by_label("Alpha").click();
    h.run();
    h.get_by_label("Beta").click_modifiers(Modifiers::SHIFT);
    h.run();
    assert_eq!(
        h.state()
            .w
            .session
            .document_state()
            .unwrap()
            .selection
            .len(),
        2
    );
    h.get_by_label("Beta").click_modifiers(Modifiers::SHIFT);
    h.run();
    assert_eq!(
        h.state()
            .w
            .session
            .document_state()
            .unwrap()
            .selection
            .len(),
        1
    );
    dispatch(&mut h, "edit.copy");
    assert_eq!(h.state().clip.borrow().writes, 1);
    assert!(h.state().clip.borrow().text.contains("dms-clipboard"));
    dispatch(&mut h, "edit.paste");
    assert_eq!(
        h.state()
            .w
            .session
            .document()
            .unwrap()
            .stream
            .diagram
            .nodes
            .len(),
        3
    );
    dispatch(&mut h, "edit.undo");
    assert_eq!(
        h.state()
            .w
            .session
            .document()
            .unwrap()
            .stream
            .diagram
            .nodes
            .len(),
        2
    );
}

#[test]
fn failed_clipboard_write_does_not_cut_nodes() {
    let mut h = harness(false);
    h.get_by_label("Alpha").click();
    h.run();
    h.state().clip.borrow_mut().fail = true;
    dispatch(&mut h, "edit.cut");
    assert_eq!(
        h.state()
            .w
            .session
            .document()
            .unwrap()
            .stream
            .diagram
            .nodes
            .len(),
        2
    );
    assert!(!h.state().w.session.is_dirty());
    h.get_by_label("Test clipboard unavailable");
}

#[test]
fn rename_double_click_type_enter_and_native_copy_stays_in_text() {
    let mut h = harness(false);
    double_click(&mut h, "Alpha");
    assert!(h.ctx.egui_wants_keyboard_input());
    dispatch(&mut h, "edit.select_all");
    h.event(egui::Event::Text("Renamed".into()));
    h.run();
    let writes = h.state().clip.borrow().writes;
    dispatch(&mut h, "edit.copy");
    assert_eq!(
        h.state().clip.borrow().writes,
        writes,
        "native Copy must not serialize nodes in a text field"
    );
    assert_eq!(h.state().w.session.revision(), 0);
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(
        h.state().w.session.document().unwrap().stream.diagram.nodes[0]
            .label
            .as_deref(),
        Some("Renamed")
    );
    h.get_by_label("Renamed");
}

#[test]
fn close_cancel_and_escape_preserve_document() {
    let mut h = harness(false);
    dirty(&mut h);
    dispatch(&mut h, "file.close");
    h.get_by_label("Do you want to save the changes you made to example.str?");
    h.get_by_label("Cancel").click();
    h.run();
    assert!(h.state().w.session.is_dirty());
    dispatch(&mut h, "file.close");
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().w.session.document().is_some());
    assert!(h.query_by_label("Don't Save").is_none());
}

#[test]
fn close_dont_save_closes_without_writing() {
    let mut h = harness(false);
    dirty(&mut h);
    dispatch(&mut h, "file.close");
    h.get_by_label("Don't Save").click();
    h.run();
    assert!(h.state().w.session.document().is_none());
}

static NEXT: AtomicU64 = AtomicU64::new(0);
fn temp_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "dms-wp3-editing-{}-{}.dms",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn close_save_saves_import_as_dms_then_closes() {
    let path = temp_path();
    let mut h = harness(false);
    dirty(&mut h);
    h.state_mut().save = Some(path.clone());
    dispatch(&mut h, "file.close");
    h.get_by_label("Save").click();
    h.run();
    assert!(h.state().w.session.document().is_none());
    let doc = dms_stream::native::load(&path).unwrap();
    assert_eq!(doc.stream.diagram.nodes.len(), 3);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn canceled_or_failed_save_keeps_pending_close_and_document() {
    let mut h = harness(false);
    dirty(&mut h);
    dispatch(&mut h, "file.close");
    h.get_by_label("Save").click();
    h.run();
    assert!(h.state().w.session.is_dirty());
    h.get_by_label("Don't Save");
    h.state_mut().save = Some(temp_path().join("missing/stream.dms"));
    h.get_by_label("Save").click();
    h.run();
    assert!(h.state().w.session.is_dirty());
    h.get_by_label("Cancel");
}

#[test]
fn pending_open_preserves_requested_path_when_canceled_or_discarded() {
    let mut h = harness(false);
    dirty(&mut h);
    let path = temp_path();
    dms_stream::native::save_atomic(&dms_stream::StreamDoc::new(), &path).unwrap();
    h.state_mut().w.open_path(path.clone());
    h.run();
    h.get_by_label("Cancel").click();
    h.run();
    assert_eq!(
        h.state()
            .w
            .session
            .document()
            .unwrap()
            .stream
            .diagram
            .nodes
            .len(),
        3
    );
    h.state_mut().w.open_path(path.clone());
    h.run();
    h.get_by_label("Don't Save").click();
    h.run();
    assert_eq!(h.state().w.session.document().unwrap().path, path);
    assert!(
        h.state()
            .w
            .session
            .document()
            .unwrap()
            .stream
            .diagram
            .nodes
            .is_empty()
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn selected_connection_delete_is_reversible() {
    let mut h = harness(true);
    let a = h.get_by_label("Connect from Alpha").rect().center();
    let b = h.get_by_label("Beta").rect();
    let midpoint = Pos2::new((a.x + b.left()) / 2.0, a.y);
    h.hover_at(midpoint);
    h.drag_at(midpoint);
    h.drop_at(midpoint);
    h.run();
    h.key_press(Key::Delete);
    h.run();
    assert!(
        h.state()
            .w
            .session
            .document()
            .unwrap()
            .stream
            .diagram
            .links
            .is_empty()
    );
    dispatch(&mut h, "edit.undo");
    assert_eq!(
        h.state()
            .w
            .session
            .document()
            .unwrap()
            .stream
            .diagram
            .links
            .len(),
        1
    );
}

#[test]
fn native_plain_keys_rename_and_text_backspace_does_not_delete_nodes() {
    let mut h = harness(false);
    h.get_by_label("Alpha").click();
    h.run();
    h.key_press(Key::F2);
    h.run();
    assert!(h.ctx.egui_wants_keyboard_input());
    dispatch(&mut h, "edit.select_all");
    h.key_press(Key::Backspace);
    h.run();
    h.event(egui::Event::Text("New name".into()));
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::C);
    h.run();
    assert_eq!(h.state().clip.borrow().writes, 0);
    assert_eq!(
        h.state()
            .w
            .session
            .document()
            .unwrap()
            .stream
            .diagram
            .nodes
            .len(),
        2
    );
    assert_eq!(h.state().w.session.revision(), 0);
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(
        h.state().w.session.document().unwrap().stream.diagram.nodes[0]
            .label
            .as_deref(),
        Some("New name")
    );
}

#[test]
fn viewport_close_is_canceled_until_discard_and_emits_close_afterward() {
    let mut h = harness(false);
    dirty(&mut h);
    h.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    h.run();
    assert!(h.state().w.session.document().is_some());
    h.get_by_label("Don't Save");
    h.get_by_label("Don't Save").click();
    h.run();
    assert!(h.state().w.session.document().is_none());
}

#[test]
fn save_is_default_confirmation_action() {
    let path = temp_path();
    let mut h = harness(false);
    dirty(&mut h);
    h.state_mut().save = Some(path.clone());
    dispatch(&mut h, "file.close");
    h.key_press(Key::Enter);
    h.run();
    assert!(path.exists());
    assert!(h.state().w.session.document().is_none());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn recovery_dialog_lists_timestamp_and_recovers_or_discards_only_snapshot() {
    use std::time::{Duration, Instant};
    let dir = temp_path().with_extension("autosave");
    let mut session = dms_engine::session::Session::new();
    session.set_autosave_dir(Some(dir.clone()));
    session.execute("file.new", &[]).unwrap();
    session
        .autosave_if_due(Instant::now() + Duration::from_secs(31))
        .unwrap();
    let path = session.recoverable(); // active snapshots are deliberately not offered
    assert!(path.is_empty());
    let mut h = harness(false);
    h.state_mut().w.set_autosave_dir(Some(dir.clone()));
    h.run();
    h.get_by_label("Recover Streams");
    h.get_by_label("Recover").click();
    h.run();
    assert!(h.state().w.session.is_dirty());
    assert!(
        h.state()
            .w
            .session
            .document()
            .unwrap()
            .stream
            .diagram
            .nodes
            .is_empty()
    );
    assert!(h.query_by_label("Recover Streams").is_none());
    // An independent workbench can discard the same orphan without modifying its document.
    let mut second = harness(false);
    second.state_mut().w.set_autosave_dir(Some(dir.clone()));
    second.run();
    second.get_by_label("Discard").click();
    second.run();
    assert_eq!(
        second
            .state()
            .w
            .session
            .document()
            .unwrap()
            .stream
            .diagram
            .nodes
            .len(),
        2
    );
    assert!(second.state().w.session.recoverable().is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

/// `dms --run` quits on its own after writing the report; an unsaved (imported) stream must
/// not leave it waiting on the save prompt (it did, in the v0.3 bundle smoke test).
#[test]
fn unattended_quit_does_not_wait_for_the_save_prompt() {
    let mut workbench = Workbench::with_document(
        Document::from_stream(PathBuf::from("imported.str"), synthetic::stream(3)),
        ShellState::default(),
    );
    workbench
        .session
        .execute(
            "node.move",
            &[
                dms_engine::session::Arg::Nodes(vec!["synthetic00000".into()]),
                dms_engine::session::Arg::Delta(10, 0),
            ],
        )
        .unwrap();
    assert!(workbench.session.is_dirty());
    workbench.quit_without_saving(&egui::Context::default());
    assert!(workbench.session.document().is_none());
}
