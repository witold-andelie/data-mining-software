//! The results area and a run in the main window, on synthetic data only.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use dms_engine::run::RunReport;
use dms_engine::session::{Document, RunState};
use dms_exec::{DocumentTable, ErrorKind, NodeError, NodeOutput};
use dms_types::{DataModel, Field, Storage, Table, Value};
use dms_ui::results::{ResultsAction, show_results};
use dms_ui::{Platform, ShellState, Workbench, synthetic};
use egui::{Theme, vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, SnapshotOptions};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// One test renders at a time: concurrent wgpu device creation crashed on Windows (see
/// tests/snapshots.rs). Held for each rendering test's whole life.
static GPU: Mutex<()> = Mutex::new(());

fn gpu() -> MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(PoisonError::into_inner)
}

fn table(rows: usize) -> Table {
    let model = DataModel::new(vec![
        Field::new("id", Storage::Integer),
        Field::new("město", Storage::String),
        Field::new("příjem", Storage::Real),
        Field::new("věk", Storage::Integer),
    ])
    .unwrap();
    let towns = ["Praha", "Brno", "Ostrava", "Plzeň", "Liberec"];
    let rows = (0..rows)
        .map(|i| {
            vec![
                Value::Integer(i as i64),
                Value::String(Arc::from(towns[i % towns.len()])),
                if i % 7 == 3 {
                    Value::Null
                } else {
                    Value::Real(1000.0 + i as f64 * 12.25)
                },
                Value::Integer(18 + (i % 60) as i64),
            ]
        })
        .collect();
    Table::new(model, rows).unwrap()
}

fn report() -> RunReport {
    let mut results = HashMap::new();
    results.insert("t".to_string(), Ok(NodeOutput::Data(Arc::new(table(1000)))));
    results.insert(
        "doc".to_string(),
        Ok(NodeOutput::Document(Arc::new(dms_exec::Document {
            title: "Statistics".to_string(),
            tables: vec![DocumentTable {
                title: Some("příjem".to_string()),
                header: vec!["Statistic".to_string(), "Value".to_string()],
                rows: vec![vec!["Mean".to_string(), "1612.5".to_string()]],
            }],
        }))),
    );
    let bad = NodeError::new("bad", ErrorKind::Settings("no file name".to_string()));
    results.insert("bad".to_string(), Err(bad.clone()));
    results.insert(
        "after".to_string(),
        Err(NodeError::new(
            "after",
            ErrorKind::Upstream("bad".to_string()),
        )),
    );
    RunReport {
        targets: vec!["t".to_string(), "doc".to_string(), "after".to_string()],
        results,
        root_causes: vec![bad],
        elapsed: Duration::from_millis(1234),
    }
}

#[test]
fn results_panel_shows_tables_documents_and_failures() {
    let _gpu = gpu();
    let report = report();
    let mut active = None;
    let mut actions = Vec::new();
    let mut harness = Harness::builder()
        .with_size(vec2(900.0, 360.0))
        .with_pixels_per_point(1.0)
        .build_ui(|ui| {
            if let Some(action) = show_results(ui, &report, None, None, &mut active, None) {
                actions.push(action);
            }
        });
    harness.ctx.set_theme(Theme::Light);
    harness.run();
    harness.get_by_label("1\u{2009}000 rows · 4 fields");
    assert!(harness.get_all_by_label("Plzeň").next().is_some());
    harness.snapshot_options(
        "results_table",
        &SnapshotOptions::new().max_failed_pixels(64),
    );
    harness.get_by_label("doc").click();
    harness.run();
    harness.get_by_label("Statistics");
    harness.get_by_label_contains("after").click();
    harness.run();
    harness.get_by_label("A node it depends on failed.");
    harness.get_by_label("Show bad").click();
    harness.run();
    drop(harness);
    assert_eq!(actions, [ResultsAction::Focus("bad".to_string())]);
}

#[test]
fn running_a_stream_marks_failures_and_reports_them() {
    let _gpu = gpu();
    let workbench = Workbench::with_document(
        Document::from_stream(PathBuf::from("synthetic.str"), synthetic::stream(6)),
        ShellState::default(),
    );
    let platform = Platform {
        mac: true,
        native_menu: true,
        frame_cpu_ms: None,
    };
    let mut harness = Harness::builder()
        .with_size(vec2(1100.0, 700.0))
        .with_pixels_per_point(1.0)
        .build_ui_state(
            move |ui, workbench: &mut Workbench| {
                workbench.ui(ui, &platform);
            },
            workbench,
        );
    harness.ctx.set_theme(Theme::Light);
    harness.run();
    let ctx = harness.ctx.clone();
    harness
        .state_mut()
        .dispatch(&ctx, "run.stream", &mut |_| None);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        harness.step();
        let done = harness.state().session.run_state() == RunState::Idle
            && harness.state().session.results().is_some();
        if done {
            break;
        }
        assert!(Instant::now() < deadline, "the run did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.run();
    let results = harness.state().session.results().unwrap();
    // Synthetic nodes have no settings: the source fails, everything downstream is blocked.
    assert!(!results.report.root_causes.is_empty());
    assert!(harness.get_all_by_label_contains("failed").next().is_some());
    harness.snapshot_options(
        "workbench_after_failed_run",
        &SnapshotOptions::new().max_failed_pixels(64),
    );
}

#[test]
#[ignore = "timing; run explicitly in release mode"]
fn million_row_table_frame_cost() {
    let big = table(1_000_000);
    let ctx = egui::Context::default();
    ctx.set_fonts(egui::FontDefinitions::default());
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1440.0, 900.0));
    let mut times = Vec::new();
    for frame in 0..300 {
        let mut input = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        input
            .events
            .push(egui::Event::PointerMoved(egui::pos2(700.0, 450.0)));
        input.events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(
                0.0,
                if (frame / 100) % 2 == 0 {
                    -400.0
                } else {
                    300.0
                },
            ),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Trackpad,
            modifiers: egui::Modifiers::NONE,
        });
        let start = Instant::now();
        let output = ctx.run_ui(input, |ui| {
            dms_ui::results_table::show_table(ui, &big, "big");
        });
        let primitives = ctx.tessellate(output.shapes, 2.0);
        assert!(!primitives.is_empty());
        if frame >= 10 {
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    times.sort_by(f64::total_cmp);
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    println!(
        "1 000 000 rows: mean {mean:.2} ms, p95 {:.2} ms, max {:.2} ms per frame (CPU)",
        times[times.len() * 95 / 100],
        times[times.len() - 1]
    );
}
