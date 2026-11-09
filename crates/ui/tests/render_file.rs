//! Renders a stream file off-screen to `target/render/<name>.png`, for checking the canvas on
//! real streams without committing them (corpus streams are never committed, and neither are
//! their pictures). Ignored by default:
//! `DMS_RENDER_STREAM=corpus/team/x.str cargo test -p dms-ui --test render_file -- --ignored`
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
    let workbench = Workbench::with_document(document, ShellState::default());
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
    harness.run();
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
