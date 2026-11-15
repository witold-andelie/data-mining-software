//! Off-screen screenshot tests of the main window (egui_kittest + wgpu). Synthetic streams
//! only: no IBM or team data. Update the reference images with
//! `UPDATE_SNAPSHOTS=1 cargo test -p dms-ui --test snapshots` and review them before committing.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use dms_engine::session::Document;
use dms_ui::{Platform, ShellState, Workbench, synthetic};
use egui::{Theme, vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, SnapshotOptions};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

const SIZE: egui::Vec2 = vec2(1100.0, 680.0);

/// Creating wgpu devices from several test threads at once crashed the process on Windows
/// (NVIDIA RTX 4060, driver 560.70: STATUS_ACCESS_VIOLATION in about one run in four). Every
/// test that renders holds this lock for its whole life, so one renders at a time.
static GPU: Mutex<()> = Mutex::new(());

fn gpu() -> MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(PoisonError::into_inner)
}

fn workbench(nodes: usize) -> Workbench {
    Workbench::with_document(
        Document::from_stream(
            PathBuf::from("synthetic-stream.str"),
            synthetic::stream(nodes),
        ),
        ShellState::default(),
    )
}

fn harness(workbench: Workbench, theme: Theme, platform: Platform) -> Harness<'static, Workbench> {
    let mut harness = Harness::builder()
        .with_size(SIZE)
        .with_pixels_per_point(1.0)
        .build_ui_state(
            move |ui, workbench: &mut Workbench| {
                workbench.ui(ui, &platform);
            },
            workbench,
        );
    harness.ctx.set_theme(theme);
    harness.run();
    harness
}

/// Small rendering differences between GPUs and wgpu backends are tolerated; layout changes
/// are not (see DESIGN.md §7).
fn options() -> SnapshotOptions {
    SnapshotOptions::new().max_failed_pixels(64)
}

#[test]
fn workbench_light_in_window_menu() {
    let _gpu = gpu();
    let platform = Platform {
        mac: false,
        native_menu: false,
        frame_cpu_ms: None,
    };
    let mut harness = harness(workbench(24), Theme::Light, platform);
    harness.snapshot_options("workbench_light_in_window_menu", &options());
}

#[test]
fn canvas_dark_zoomed_with_selection() {
    let _gpu = gpu();
    let platform = Platform {
        mac: true,
        native_menu: true,
        frame_cpu_ms: None,
    };
    let mut workbench = workbench(14);
    workbench.state.show_palette = false;
    // Selection belongs to the session (v0.2); select through the command, as a click does.
    workbench
        .session
        .execute(
            "edit.select",
            &[dms_engine::session::Arg::Nodes(vec![
                "synthetic00004".to_string(),
            ])],
        )
        .unwrap();
    let mut harness = harness(workbench, Theme::Dark, platform);
    // Zoom in twice around the centre, as ⌘= would.
    let ctx = harness.ctx.clone();
    for _ in 0..2 {
        harness
            .state_mut()
            .dispatch(&ctx, "view.zoom_in", &mut |_| None);
        harness.run();
    }
    assert!(harness.state().state.canvas.zoom > 1.0);
    harness.snapshot_options("canvas_dark_zoomed_with_selection", &options());
}

#[test]
fn empty_window_offers_open() {
    let _gpu = gpu();
    let platform = Platform::default();
    let mut harness = harness(
        Workbench::new(ShellState::default()),
        Theme::Light,
        platform,
    );
    harness.get_by_label("No stream open");
    // The open command asks for a path; a cancelled dialog changes nothing.
    let ctx = harness.ctx.clone();
    harness
        .state_mut()
        .dispatch(&ctx, "file.open", &mut |_| None);
    assert!(harness.state().session.document().is_none());
    // A path that is not a stream reports an error instead of failing.
    harness.state_mut().dispatch(&ctx, "file.open", &mut |_| {
        Some(PathBuf::from("no-such-dir/missing.str"))
    });
    harness.run();
    harness.get_by_label_contains("cannot open");
}

/// The settings form generated from a node's schema (a Select node without saved settings
/// shows its defaults).
#[test]
fn properties_form_light() {
    let _gpu = gpu();
    let platform = Platform {
        mac: true,
        native_menu: true,
        frame_cpu_ms: None,
    };
    let mut workbench = workbench(14);
    workbench.state.show_palette = false;
    workbench
        .session
        .execute(
            "edit.select",
            &[dms_engine::session::Arg::Nodes(vec![
                "synthetic00002".to_string(),
            ])],
        )
        .unwrap();
    let mut harness = harness(workbench, Theme::Light, platform);
    harness.get_by_label_contains("Select");
    harness.snapshot_options("properties_form_light", &options());
}

/// The canvas tile of a node, by uid (not a heading or window title with the same text).
fn tile_center(harness: &Harness<'_, Workbench>, uid: &str) -> egui::Pos2 {
    let graph = dms_ui::graph::StreamGraph::from_diagram(
        &harness.state().session.document().unwrap().stream.diagram,
    );
    let title = &graph.nodes.iter().find(|n| n.uid == uid).unwrap().title;
    let area = harness
        .get_all_by_label(title)
        .map(|node| node.rect())
        .find(|rect| rect.height() >= rect.width())
        .unwrap();
    area.min + vec2(area.width() / 2.0, area.width() / 2.0)
}

fn right_click(harness: &mut Harness<'_, Workbench>, at: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
    }
    harness.run();
}

/// The node's right-click menu.
#[test]
fn node_menu_light() {
    let _gpu = gpu();
    let platform = Platform {
        mac: true,
        native_menu: true,
        frame_cpu_ms: None,
    };
    let mut harness = harness(workbench(14), Theme::Light, platform);
    let at = tile_center(&harness, "synthetic00002");
    right_click(&mut harness, at);
    harness.get_by_label("Edit…");
    harness.snapshot_options("node_menu_light", &options());
}

/// A node's settings in its own window, opened from the node menu at the pointer.
#[test]
fn node_window_light() {
    let _gpu = gpu();
    let platform = Platform {
        mac: true,
        native_menu: true,
        frame_cpu_ms: None,
    };
    let mut workbench = workbench(14);
    workbench.state.show_properties = false;
    let mut harness = harness(workbench, Theme::Light, platform);
    let at = tile_center(&harness, "synthetic00002");
    right_click(&mut harness, at);
    harness.get_by_label("Edit…").click();
    harness.run();
    harness.get_by_label("Close");
    harness.snapshot_options("node_window_light", &options());
}

/// An Extension node's Python script in the settings form's code editor (no wrapping).
#[test]
fn python_script_form_light() {
    use dms_engine::session::Arg;
    use dms_exec::schema::ParamValue;
    let _gpu = gpu();
    let platform = Platform {
        mac: true,
        native_menu: true,
        frame_cpu_ms: None,
    };
    let mut workbench = workbench(14);
    workbench.state.show_palette = false;
    // The synthetic stream's Extension Import node (a fixed uid, so the image is stable).
    let uid = dms_ui::graph::StreamGraph::from_diagram(
        &workbench.session.document().unwrap().stream.diagram,
    )
    .nodes
    .iter()
    .find(|n| n.type_id == "extension_import")
    .unwrap()
    .uid
    .clone();
    workbench
        .session
        .execute("edit.select", &[Arg::Nodes(vec![uid.clone()])])
        .unwrap();
    workbench
        .session
        .execute(
            "node.set_param",
            &[
                Arg::Node(uid),
                Arg::Param(
                    "python_syntax".into(),
                    ParamValue::Text(
                        "# modelerData is a pandas DataFrame\nimport pandas as pd\nmodelerData = pd.read_csv('customers.csv')\nmodelerData['ratio'] = (modelerData['income'] / modelerData['age']).round(2)\n".into(),
                    ),
                ),
            ],
        )
        .unwrap();
    let mut harness = harness(workbench, Theme::Light, platform);
    harness.get_by_label("Python script");
    harness.snapshot_options("python_script_form_light", &options());
}
