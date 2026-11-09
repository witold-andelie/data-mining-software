//! Renders a stream file off-screen to `target/render/<name>.png`, for checking the canvas on
//! real streams without committing them (corpus streams are never committed, and neither are
//! their pictures). Ignored by default:
//! `DMS_RENDER_STREAM=corpus/team/x.str cargo test -p dms-ui --test render_file -- --ignored`
//! (`DMS_RENDER_RUN=1` also runs the stream and renders its results).
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test tool")]

use dms_engine::session::Session;
use dms_ui::{Platform, ShellState, Workbench};
use egui::{Theme, vec2};
use egui_kittest::Harness;
use std::path::{Path, PathBuf};

#[test]
#[ignore = "needs DMS_RENDER_STREAM; writes only target/render/"]
fn render_stream_file() {
    let Some(file) = std::env::var_os("DMS_RENDER_STREAM") else {
        return;
    };
    // Paths are relative to the workspace root, not this crate.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let file = root.join(PathBuf::from(file));
    let mut session = Session::new();
    session
        .execute("file.open", &[dms_engine::session::Arg::Path(file.clone())])
        .unwrap();
    let document = session.document().unwrap().clone();
    let mut state = ShellState::default();
    // Extra data folders, separated like PATH (as `dms --data-dir`).
    if let Some(dirs) = std::env::var_os("DMS_RENDER_DATA_DIRS") {
        state.data_folders = std::env::split_paths(&dirs).collect();
    }
    let workbench = Workbench::with_document(document, state);
    let platform = Platform {
        mac: true,
        native_menu: true,
        frame_cpu_ms: None,
    };
    let mut harness = Harness::builder()
        .with_size(vec2(1280.0, 760.0))
        .with_pixels_per_point(2.0)
        .build_ui_state(
            move |ui, workbench: &mut Workbench| {
                workbench.ui(ui, &platform);
            },
            workbench,
        );
    harness.ctx.set_theme(Theme::Light);
    // Real data may hold characters the bundled test fonts lack: draw boxes, do not fail.
    harness
        .ctx
        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
    harness.run();
    if std::env::var_os("DMS_RENDER_RUN").is_some() {
        // Run the stream, then render the window with its results.
        let ctx = harness.ctx.clone();
        harness
            .state_mut()
            .dispatch(&ctx, "run.stream", &mut |_| None);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
        while harness.state().session.run_state() != dms_engine::session::RunState::Idle {
            assert!(std::time::Instant::now() < deadline, "run did not finish");
            harness.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        harness.run();
        if let Some(results) = harness.state().session.results() {
            let report = &results.report;
            println!(
                "run: {} targets, {} nodes evaluated, {} root causes, {:.2} s",
                report.targets.len(),
                report.results.len(),
                report.root_causes.len(),
                report.elapsed.as_secs_f64()
            );
            for uid in &report.targets {
                let kind = report
                    .results
                    .get(uid)
                    .map_or("missing", dms_ui::results::result_kind);
                println!("  target {uid}: {kind}");
            }
            for error in &report.root_causes {
                println!("  root cause: {error}");
            }
        }
    }
    // Select a node by its title and show its result (`DMS_RENDER_FOCUS=CHAID`).
    if let Some(title) = std::env::var_os("DMS_RENDER_FOCUS") {
        let title = title.to_string_lossy().into_owned();
        let state = harness.state();
        let targets: Vec<String> = state
            .session
            .results()
            .map(|r| r.report.targets.clone())
            .unwrap_or_default();
        let matching: Vec<String> = state
            .graph()
            .map(|g| {
                g.nodes
                    .iter()
                    .filter(|n| n.title == title)
                    .map(|n| n.uid.clone())
                    .collect()
            })
            .unwrap_or_default();
        // Prefer a node that was run (it has a results tab).
        let uid = matching
            .iter()
            .find(|uid| targets.contains(uid))
            .or(matching.first())
            .cloned();
        if let Some(uid) = uid {
            harness.state_mut().focus_node(&uid);
            harness.run();
        }
    }
    let graph = harness.state().graph().unwrap();
    println!(
        "{}: {} nodes, {} links, {} dangling, {} not supported",
        file.display(),
        graph.nodes.len(),
        graph.links.len(),
        graph.dangling_links,
        graph.unsupported()
    );
    let out = root.join("target/render");
    std::fs::create_dir_all(&out).unwrap();
    let name = file.file_stem().unwrap().to_string_lossy().into_owned();
    let path = out.join(format!("{name}.png"));
    harness.render().unwrap().save(&path).unwrap();
    println!("wrote {}", path.display());
}
