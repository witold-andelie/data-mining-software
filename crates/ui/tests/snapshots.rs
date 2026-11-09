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
    workbench.state.canvas.selected = Some("synthetic00004".to_string());
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
