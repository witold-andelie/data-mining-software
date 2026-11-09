//! The three ways to connect two nodes, driven by real pointer events: drag from the output
//! port, Option/Alt-drag from the node, and right-click > Connect From Here then click.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use dms_engine::session::Document;
use dms_ui::{Platform, ShellState, Workbench, synthetic};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::path::PathBuf;

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
        .build_ui_state(
            move |ui, workbench: &mut Workbench| {
                workbench.ui(ui, &platform);
            },
            workbench,
        );
    harness.run();
    harness
}

/// The node's tile (its interaction area also covers the label below it).
fn tile(harness: &Harness<'_, Workbench>, title: &str) -> Rect {
    // The canvas node, not the properties heading with the same text (taller than wide).
    let area = harness
        .get_all_by_label(title)
        .map(|node| node.rect())
        .find(|rect| rect.height() >= rect.width())
        .unwrap();
    let side = area.width();
    Rect::from_min_size(area.min, vec2(side, side))
}

fn links(harness: &Harness<'_, Workbench>) -> Vec<(String, String)> {
    let stream = &harness.state().session.document().unwrap().stream;
    stream
        .diagram
        .links
        .iter()
        .map(|l| (format!("{:?}", l.from), format!("{:?}", l.to)))
        .collect()
}

fn press(harness: &mut Harness<'_, Workbench>, at: Pos2, modifiers: Modifiers) {
    // Held modifiers are input state, not part of the pointer event.
    harness.event(Event::ModifiersChanged(modifiers));
    harness.event(Event::PointerMoved(at));
    harness.step();
    harness.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers,
    });
    harness.step();
}

fn release(harness: &mut Harness<'_, Workbench>, at: Pos2, modifiers: Modifiers) {
    harness.event(Event::PointerMoved(at));
    harness.step();
    harness.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers,
    });
    for _ in 0..3 {
        harness.step();
    }
    harness.event(Event::ModifiersChanged(Modifiers::NONE));
    harness.step();
}

#[test]
fn dragging_from_the_port_connects() {
    let mut harness = harness();
    let before = links(&harness).len();
    let from = tile(&harness, "Tabulka");
    let to = tile(&harness, "Extension import");
    press(&mut harness, from.right_center(), Modifiers::NONE);
    release(&mut harness, to.center(), Modifiers::NONE);
    assert_eq!(links(&harness).len(), before + 1);
}

#[test]
fn option_dragging_a_node_connects_instead_of_moving_it() {
    let mut harness = harness();
    let before = links(&harness).len();
    let from = tile(&harness, "Strom C5.0");
    let to = tile(&harness, "export.txt");
    press(&mut harness, from.center(), Modifiers::ALT);
    release(&mut harness, to.center(), Modifiers::ALT);
    assert_eq!(links(&harness).len(), before + 1);
    // The node did not move.
    assert_eq!(tile(&harness, "Strom C5.0"), from);
}

#[test]
fn plain_dragging_a_node_moves_it_and_does_not_connect() {
    let mut harness = harness();
    let before = links(&harness).len();
    let from = tile(&harness, "Strom C5.0");
    let to = tile(&harness, "export.txt");
    press(&mut harness, from.center(), Modifiers::NONE);
    release(&mut harness, to.center(), Modifiers::NONE);
    assert_eq!(links(&harness).len(), before);
    assert_ne!(tile(&harness, "Strom C5.0"), from);
}

#[test]
fn connect_from_here_then_click_the_target() {
    let mut harness = harness();
    let before = links(&harness).len();
    let from = tile(&harness, "Odvození příjmu");
    let to = tile(&harness, "Analysis");
    harness.event(Event::PointerMoved(from.center()));
    harness.step();
    for pressed in [true, false] {
        harness.event(Event::PointerButton {
            pos: from.center(),
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        harness.step();
    }
    harness.run();
    harness.get_by_label("Connect From Here").click();
    harness.run();
    press(&mut harness, to.center(), Modifiers::NONE);
    release(&mut harness, to.center(), Modifiers::NONE);
    assert_eq!(links(&harness).len(), before + 1);
}
