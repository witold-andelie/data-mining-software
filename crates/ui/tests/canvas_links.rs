//! Canvas geometry, vertical connection gestures, provenance and hidden command dispatch.
//! CPU-only: never render snapshots.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]

use dms_engine::command::{COMMANDS, Menu, by_id};
use dms_engine::session::{Arg, Document, Outcome, Session};
use dms_ui::canvas::link_curve;
use dms_ui::graph::{TILE, model_source};
use dms_ui::{Platform, ShellState, Workbench, synthetic};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};

#[test]
fn links_use_the_nearest_edge_pair_in_all_four_directions() {
    let a = pos2(100.0, 100.0);
    for zoom in [0.1, 1.0, 4.0] {
        let half = TILE.x * zoom / 2.0;
        for delta in [
            vec2(400.0, 0.0),
            vec2(-400.0, 0.0),
            vec2(0.0, 400.0),
            vec2(0.0, -400.0),
        ] {
            let b = a + delta;
            let [start, c1, c2, end] = link_curve(a, b, zoom).unwrap();
            let direction = delta.normalized();
            assert_eq!(start, a + direction * half);
            assert_eq!(end, b - direction * half);
            assert!((c1 - start).dot(direction) > 0.0);
            assert!((end - c2).dot(direction) > 0.0, "arrow enters the target");
        }
    }
    for b in [pos2(180.0, 220.0), pos2(-30.0, 180.0), pos2(105.0, 115.0)] {
        let [start, _, _, end] = link_curve(a, b, 1.0).unwrap();
        let offsets = [
            vec2(26.0, 0.0),
            vec2(-26.0, 0.0),
            vec2(0.0, 26.0),
            vec2(0.0, -26.0),
        ];
        assert!(offsets.contains(&(start - a)));
        assert!(offsets.contains(&(end - b)));
        for from in offsets {
            for to in offsets {
                assert!(start.distance_sq(end) <= (a + from).distance_sq(b + to));
            }
        }
    }
    assert!(link_curve(a, a, 1.0).is_none());
    assert!(link_curve(a, pos2(f32::NAN, 1.0), 1.0).is_none());
    assert!(link_curve(a, Pos2::ZERO, f32::INFINITY).is_none());
}

#[test]
fn hidden_commands_leave_menus_but_remain_executable() {
    for mac in [false, true] {
        let listed: Vec<_> = Menu::ALL
            .into_iter()
            .flat_map(|m| dms_ui::menu::menu_items(m, mac))
            .collect();
        for spec in COMMANDS {
            assert_eq!(listed.contains(&spec), spec.menu_visible);
        }
    }
    let mut stream = synthetic::stream(2);
    stream.diagram.links.clear();
    stream.diagram.nodes[0].class = "SelectNode".into();
    let a = stream.diagram.nodes[0].uid.clone();
    let b = stream.diagram.nodes[1].uid.clone();
    let mut session = Session::with_document(Document::from_stream("test.str".into(), stream));
    for (id, args) in [
        ("edit.select", vec![Arg::Nodes(vec![a.clone()])]),
        (
            "node.move",
            vec![Arg::Nodes(vec![a.clone()]), Arg::Delta(10, 20)],
        ),
        (
            "node.set_param",
            vec![
                Arg::Node(a.clone()),
                Arg::Param(
                    "condition".into(),
                    dms_exec::schema::ParamValue::Text("x > 0".into()),
                ),
            ],
        ),
        ("node.connect", vec![Arg::Link(a.clone(), b.clone())]),
        ("node.disconnect", vec![Arg::Link(a.clone(), b.clone())]),
        (
            "node.add",
            vec![Arg::Kind("select".into()), Arg::Position(100, 200)],
        ),
    ] {
        assert!(!by_id(id).unwrap().menu_visible);
        assert_eq!(session.execute(id, &args), Ok(Outcome::Done), "{id}");
    }
    assert!(!by_id("file.recover").unwrap().menu_visible);
    assert!(by_id("node.rename").unwrap().menu_visible);
    assert!(by_id("node.rename").unwrap().shortcuts.mac.is_some());
}

fn vertical_workbench() -> Harness<'static, Workbench> {
    let mut stream = synthetic::stream(2);
    stream.diagram.links.clear();
    for (index, node) in stream.diagram.nodes.iter_mut().enumerate() {
        node.label = Some(format!("Vertical {index}"));
        node.position = Some((100, 100 + index as i64 * 250));
    }
    let w = Workbench::with_document(
        Document::from_stream("vertical.str".into(), stream),
        ShellState::default(),
    );
    let mut h = Harness::builder()
        .with_size(vec2(1200.0, 800.0))
        .build_ui_state(
            |ui, w: &mut Workbench| {
                let commands = w.ui(
                    ui,
                    &Platform {
                        mac: true,
                        native_menu: true,
                        ..Default::default()
                    },
                );
                for id in commands {
                    w.dispatch(ui.ctx(), id, &mut |_| None);
                }
            },
            w,
        );
    h.run();
    h
}

fn tile(h: &Harness<'_, Workbench>, title: &str) -> Rect {
    let area = h
        .get_all_by_label(title)
        .map(|n| n.rect())
        .find(|r| r.height() >= r.width())
        .unwrap();
    Rect::from_min_size(area.min, vec2(area.width(), area.width()))
}

fn button(
    h: &mut Harness<'_, Workbench>,
    at: Pos2,
    kind: PointerButton,
    pressed: bool,
    modifiers: Modifiers,
) {
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(at));
    h.step();
    h.event(Event::PointerButton {
        pos: at,
        button: kind,
        pressed,
        modifiers,
    });
    h.step();
}

#[test]
fn all_three_gestures_connect_vertical_nodes_and_the_curve_can_be_deleted() {
    for gesture in 0..3 {
        let mut h = vertical_workbench();
        let from = tile(&h, "Vertical 0");
        let to = tile(&h, "Vertical 1");
        let (start, modifiers) = match gesture {
            0 => (from.right_center(), Modifiers::NONE),
            1 => (from.center(), Modifiers::ALT),
            _ => {
                button(
                    &mut h,
                    from.center(),
                    PointerButton::Secondary,
                    true,
                    Modifiers::NONE,
                );
                button(
                    &mut h,
                    from.center(),
                    PointerButton::Secondary,
                    false,
                    Modifiers::NONE,
                );
                h.run();
                h.get_by_label("Connect From Here").click();
                h.run();
                (to.center(), Modifiers::NONE)
            }
        };
        button(&mut h, start, PointerButton::Primary, true, modifiers);
        button(
            &mut h,
            to.center(),
            PointerButton::Primary,
            false,
            modifiers,
        );
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.run();
        assert_eq!(
            h.state()
                .session
                .document()
                .unwrap()
                .stream
                .diagram
                .links
                .len(),
            1
        );
        assert_eq!(tile(&h, "Vertical 0"), from);
        // Hit testing uses the new vertical curve, too.
        let midpoint = from.center().lerp(to.center(), 0.5);
        button(
            &mut h,
            midpoint,
            PointerButton::Primary,
            true,
            Modifiers::NONE,
        );
        button(
            &mut h,
            midpoint,
            PointerButton::Primary,
            false,
            Modifiers::NONE,
        );
        h.run();
        h.key_press(egui::Key::Backspace);
        h.run();
        assert!(
            h.state()
                .session
                .document()
                .unwrap()
                .stream
                .diagram
                .links
                .is_empty()
        );
    }
}

#[test]
fn model_provenance_resolves_unique_trainers_and_click_selects_them() {
    let mut stream = synthetic::stream(2);
    stream.diagram.nodes[0].class = "C50Node".into();
    stream.diagram.nodes[0].label = Some("My trainer".into());
    stream.diagram.nodes[1].model_builder = Some("C50Node".into());
    let trainer = stream.diagram.nodes[0].uid.clone();
    let nugget = stream.diagram.nodes[1].uid.clone();
    assert_eq!(
        model_source(&stream.diagram, &nugget),
        Some(("My trainer".into(), Some(trainer.clone())))
    );
    let mut w = Workbench::with_document(
        Document::from_stream("model.str".into(), stream.clone()),
        ShellState::default(),
    );
    w.focus_node(&nugget);
    let mut h = Harness::builder()
        .with_size(vec2(1200.0, 800.0))
        .build_ui_state(
            |ui, w: &mut Workbench| {
                w.ui(
                    ui,
                    &Platform {
                        mac: true,
                        native_menu: true,
                        ..Default::default()
                    },
                );
            },
            w,
        );
    h.run();
    h.get_by_label("Generated by My trainer").click();
    h.run();
    assert_eq!(
        h.state().session.document_state().unwrap().selection,
        [trainer]
    );
    let mut duplicate = stream.diagram.nodes[0].clone();
    duplicate.uid = "another trainer".into();
    stream.diagram.nodes.push(duplicate);
    assert_eq!(
        model_source(&stream.diagram, &nugget),
        Some(("C50Node".into(), None))
    );
    stream.diagram.nodes[1].model_builder = Some("MissingTrainer".into());
    assert_eq!(
        model_source(&stream.diagram, &nugget),
        Some(("MissingTrainer".into(), None))
    );
}
