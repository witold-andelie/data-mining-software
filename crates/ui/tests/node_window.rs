//! Double-clicking a node opens its settings in a window at the pointer; right-clicking it
//! opens the node menu (edit, run, connect, disconnect, rename, clipboard).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use dms_engine::session::Document;
use dms_ui::{Platform, ShellState, Workbench, synthetic};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn harness() -> Harness<'static, Workbench> {
    let workbench = Workbench::with_document(
        Document::from_stream(PathBuf::from("synthetic.str"), synthetic::stream(12)),
        ShellState::default(),
    );
    let platform = Platform {
        mac: true,
        native_menu: true,
        frame_cpu_ms: None,
    };
    let mut harness = Harness::builder()
        .with_size(vec2(1280.0, 800.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            move |ui, workbench: &mut Workbench| {
                workbench.ui(ui, &platform);
            },
            workbench,
        );
    harness.run();
    harness
}

/// The node's tile on the canvas (not a heading or window title with the same text).
fn tile(harness: &Harness<'_, Workbench>, title: &str) -> Rect {
    let area = harness
        .get_all_by_label(title)
        .map(|node| node.rect())
        .find(|rect| rect.height() >= rect.width())
        .unwrap();
    Rect::from_min_size(area.min, vec2(area.width(), area.width()))
}

fn click(harness: &mut Harness<'_, Workbench>, at: Pos2, button: PointerButton) {
    harness.event(Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.event(Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        });
        harness.step();
    }
}

fn double_click(harness: &mut Harness<'_, Workbench>, at: Pos2) {
    click(harness, at, PointerButton::Primary);
    click(harness, at, PointerButton::Primary);
    harness.run();
}

fn right_click(harness: &mut Harness<'_, Workbench>, at: Pos2) {
    click(harness, at, PointerButton::Secondary);
    harness.run();
}

/// Uids linked to or from `uid`, and the terminals reached downstream of it.
fn neighbourhood(harness: &Harness<'_, Workbench>, uid: &str) -> (usize, BTreeSet<String>) {
    let diagram = &harness.state().session.document().unwrap().stream.diagram;
    let edges: Vec<(String, String)> = diagram
        .links
        .iter()
        .map(|l| (format!("{:?}", l.from), format!("{:?}", l.to)))
        .collect();
    let key = |uid: &str| format!("{:?}", dms_stream::NodeRef::Uid(uid.into()));
    let me = key(uid);
    let touching = edges.iter().filter(|(a, b)| *a == me || *b == me).count();
    let mut terminals = BTreeSet::new();
    let mut stack = vec![me.clone()];
    let mut seen = BTreeSet::new();
    while let Some(at) = stack.pop() {
        let next: Vec<_> = edges.iter().filter(|(a, _)| *a == at).collect();
        if next.is_empty() && at != me {
            terminals.insert(at.clone());
        }
        for (_, b) in next {
            if seen.insert(b.clone()) {
                stack.push(b.clone());
            }
        }
    }
    let terminals = diagram
        .nodes
        .iter()
        .filter(|n| terminals.contains(&key(&n.uid)))
        .map(|n| n.uid.clone())
        .collect();
    (touching, terminals)
}

/// A top-level node with outgoing links, by its canvas title.
fn upstream_node(harness: &Harness<'_, Workbench>) -> (String, String) {
    let document = harness.state().session.document().unwrap();
    let diagram = &document.stream.diagram;
    let graph = dms_ui::graph::StreamGraph::from_diagram(diagram);
    let node = graph
        .nodes
        .iter()
        .enumerate()
        .find(|(i, _)| graph.links.iter().any(|l| l.from == *i))
        .map(|(_, n)| n)
        .unwrap();
    (node.uid.clone(), node.title.clone())
}

#[test]
fn double_click_opens_the_settings_window_at_the_pointer() {
    let mut harness = harness();
    let (uid, title) = upstream_node(&harness);
    let at = tile(&harness, &title).center();
    double_click(&mut harness, at);
    assert_eq!(
        harness.state().session.document_state().unwrap().selection,
        [uid]
    );
    let close = harness.get_by_label("Close").rect();
    // The window opens with its corner at the pointer, so its buttons are just below-right.
    assert!(
        close.min.x >= at.x - 1.0 && close.min.x < at.x + 400.0,
        "{close:?} {at:?}"
    );
    assert!(close.min.y >= at.y - 1.0, "{close:?} {at:?}");
    harness.get_by_label("Close").click();
    harness.run();
    assert!(harness.query_by_label("Close").is_none());
    // Escape closes it as well (after a pause, so the clicks are not read as one gesture).
    for _ in 0..30 {
        harness.step();
    }
    double_click(&mut harness, at);
    assert!(harness.query_by_label("Close").is_some());
    harness.key_press(Key::Escape);
    harness.run();
    assert!(harness.query_by_label("Close").is_none());
}

#[test]
fn the_node_menu_offers_edit_run_connect_rename_and_clipboard() {
    let mut harness = harness();
    let (_, title) = upstream_node(&harness);
    let at = tile(&harness, &title).center();
    right_click(&mut harness, at);
    for label in [
        "Edit…",
        "Run From Here",
        "Run to Here",
        "Connect From Here",
        "Disconnect",
        "Rename…",
        "Cut",
        "Copy",
        "Paste",
        "Duplicate",
        "Delete",
    ] {
        harness.get_by_label(label);
    }
    harness.get_by_label("Edit…").click();
    harness.run();
    assert!(harness.query_by_label("Close").is_some());
}

#[test]
fn disconnect_removes_every_link_of_the_node_in_one_undo_step() {
    let mut harness = harness();
    let (uid, title) = upstream_node(&harness);
    let before = harness
        .state()
        .session
        .document()
        .unwrap()
        .stream
        .diagram
        .links
        .len();
    let (touching, _) = neighbourhood(&harness, &uid);
    assert!(touching > 0);
    let at = tile(&harness, &title).center();
    right_click(&mut harness, at);
    harness.get_by_label("Disconnect").click();
    harness.run();
    assert_eq!(neighbourhood(&harness, &uid).0, 0);
    let ctx = harness.ctx.clone();
    harness
        .state_mut()
        .dispatch(&ctx, "edit.undo", &mut |_| None);
    harness.run();
    let after_undo = harness
        .state()
        .session
        .document()
        .unwrap()
        .stream
        .diagram
        .links
        .len();
    assert_eq!(after_undo, before);
}

#[test]
fn run_from_here_runs_the_terminals_downstream() {
    let mut harness = harness();
    let (uid, title) = upstream_node(&harness);
    let (_, expected) = neighbourhood(&harness, &uid);
    assert!(!expected.is_empty());
    let at = tile(&harness, &title).center();
    right_click(&mut harness, at);
    harness.get_by_label("Run From Here").click();
    let deadline = Instant::now() + Duration::from_secs(30);
    while harness.state().session.results().is_none() {
        assert!(Instant::now() < deadline, "the run did not finish");
        std::thread::sleep(Duration::from_millis(10));
        harness.step();
    }
    let targets: BTreeSet<String> = harness
        .state()
        .session
        .results()
        .unwrap()
        .report
        .targets
        .iter()
        .cloned()
        .collect();
    assert_eq!(targets, expected);
}
