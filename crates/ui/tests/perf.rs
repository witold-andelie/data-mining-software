//! CPU cost of one frame of the main window with a 200-node stream: egui layout plus
//! tessellation, at Retina scale, while the canvas pans and zooms. GPU time is measured in the
//! app (`dms --bench`). Run with
//! `cargo test --release -p dms-ui --test perf -- --ignored --nocapture`.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]

use dms_engine::session::Document;
use dms_ui::{Platform, ShellState, Workbench, synthetic};
use egui::{Event, MouseWheelUnit, Pos2, RawInput, Rect, TouchPhase, vec2};
use std::path::PathBuf;
use std::time::Instant;

const FRAMES: usize = 600;

#[test]
#[ignore = "timing; run explicitly in release mode"]
fn two_hundred_nodes_frame_cost() {
    for (columns, label) in [
        (20, "200 nodes, labels visible"),
        (6, "200 nodes, zoomed out"),
    ] {
        let mut workbench = Workbench::with_document(
            Document::from_stream(
                PathBuf::from("synthetic.str"),
                synthetic::grid(200, columns),
            ),
            ShellState::default(),
        );
        let ctx = egui::Context::default();
        egui_extras_fonts(&ctx);
        let platform = Platform {
            mac: true,
            native_menu: true,
            frame_cpu_ms: None,
        };
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0));
        let ppp = 2.0;
        let mut times = Vec::with_capacity(FRAMES);
        for frame in 0..FRAMES {
            let mut input = RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            };
            input
                .events
                .push(Event::PointerMoved(Pos2::new(700.0, 450.0)));
            // Alternate trackpad pans and pinches, as a person exploring the stream would.
            if frame % 2 == 0 {
                let dx = if (frame / 60) % 2 == 0 { 4.0 } else { -4.0 };
                input.events.push(Event::MouseWheel {
                    unit: MouseWheelUnit::Point,
                    delta: vec2(dx, 1.0),
                    phase: TouchPhase::Move,
                    source: egui::MouseWheelSource::Trackpad,
                    modifiers: egui::Modifiers::NONE,
                });
            } else {
                let zoom = if (frame / 100) % 2 == 0 { 1.004 } else { 0.996 };
                input.events.push(Event::Zoom(zoom));
            }
            let start = Instant::now();
            let output = ctx.run_ui(input, |ui| {
                workbench.ui(ui, &platform);
            });
            let primitives = ctx.tessellate(output.shapes, ppp);
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            assert!(!primitives.is_empty());
            if frame >= 10 {
                times.push(elapsed);
            }
        }
        times.sort_by(f64::total_cmp);
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        let p95 = times[times.len() * 95 / 100];
        let max = times[times.len() - 1];
        println!(
            "{label}: zoom {:.2}, mean {mean:.2} ms, p95 {p95:.2} ms, max {max:.2} ms per frame (CPU)",
            workbench.state.canvas.zoom
        );
        assert!(workbench.graph().unwrap().nodes.len() == 200);
    }
}

fn egui_extras_fonts(ctx: &egui::Context) {
    ctx.set_fonts(egui::FontDefinitions::default());
}
