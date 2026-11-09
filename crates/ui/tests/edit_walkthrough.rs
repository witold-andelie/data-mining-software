//! The v0.2 editing round trip on a real stream, through the same Session and Workbench the
//! app uses: open a `.str`, move a node, add a Table, connect it, rename it, undo and redo,
//! save as `.dms`, close, reopen, run, and render the window. Corpus streams are never
//! committed, so this is ignored unless asked for:
//! `DMS_WALK_STREAM=corpus/team/x.str [DMS_WALK_DATA_DIRS=dir:dir] cargo test -p dms-ui --test edit_walkthrough -- --ignored --nocapture`
//! Writes only `target/render/` and a temporary `.dms`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test tool"
)]

use dms_engine::session::{Arg, Outcome, RunState, Session};
use dms_ui::{Platform, ShellState, Workbench};
use egui::{Theme, vec2};
use egui_kittest::Harness;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn run(session: &mut Session, id: &str, args: &[Arg]) -> Outcome {
    let outcome = session.execute(id, args).unwrap();
    println!("{id}: {outcome:?}");
    outcome
}

#[test]
#[ignore = "needs DMS_WALK_STREAM; writes only target/render/ and a temporary .dms"]
fn edit_save_reopen_run() {
    let Some(file) = std::env::var_os("DMS_WALK_STREAM") else {
        return;
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let file = root.join(PathBuf::from(file));
    let folders: Vec<PathBuf> = std::env::var_os("DMS_WALK_DATA_DIRS")
        .map(|d| std::env::split_paths(&d).collect())
        .unwrap_or_default();

    let mut session = Session::new();
    session.set_data_folders(folders.clone());
    run(&mut session, "file.open", &[Arg::Path(file.clone())]);
    let stream = &session.document().unwrap().stream;
    let original = stream.diagram.nodes.len();
    let type_node = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.class.ends_with("TypeNode"))
        .map(|n| (n.uid.clone(), n.position.unwrap_or((0, 0))))
        .expect("a Type node");
    println!("opened {original} nodes; Type node {}", type_node.0);

    run(
        &mut session,
        "node.move",
        &[Arg::Nodes(vec![type_node.0.clone()]), Arg::Delta(0, 20)],
    );
    let (x, y) = type_node.1;
    run(
        &mut session,
        "node.add",
        &[Arg::Kind("table".into()), Arg::Position(x + 60, y + 160)],
    );
    let added = session
        .document_state()
        .unwrap()
        .selection
        .first()
        .cloned()
        .unwrap();
    run(
        &mut session,
        "node.connect",
        &[Arg::Link(type_node.0.clone(), added.clone())],
    );
    run(
        &mut session,
        "node.rename",
        &[
            Arg::Node(added.clone()),
            Arg::Text("Walkthrough table".into()),
        ],
    );
    println!("undo label: {:?}", session.undo_label());
    run(&mut session, "edit.undo", &[]);
    run(&mut session, "edit.redo", &[]);
    assert!(session.is_dirty());
    assert_eq!(
        run(&mut session, "file.close", &[]),
        Outcome::ConfirmDiscard
    );

    let saved = std::env::temp_dir().join(format!("dms-walkthrough-{}.dms", std::process::id()));
    run(&mut session, "file.save_as", &[Arg::Path(saved.clone())]);
    assert!(!session.is_dirty());
    run(&mut session, "file.close", &[]);
    assert!(session.document().is_none());

    run(&mut session, "file.open", &[Arg::Path(saved.clone())]);
    let reopened = &session.document().unwrap().stream;
    assert_eq!(reopened.diagram.nodes.len(), original + 1);
    let label = reopened
        .diagram
        .nodes
        .iter()
        .find(|n| n.uid == added)
        .and_then(|n| n.label.clone());
    assert_eq!(label.as_deref(), Some("Walkthrough table"));
    println!(
        "reopened {} nodes from {}",
        reopened.diagram.nodes.len(),
        saved.display()
    );

    // Run it in the window, as the app does, and render.
    let document = session.document().unwrap().clone();
    let state = ShellState {
        data_folders: folders,
        ..ShellState::default()
    };
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
    harness
        .ctx
        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
    harness.run();
    let ctx = harness.ctx.clone();
    harness
        .state_mut()
        .dispatch(&ctx, "run.stream", &mut |_| None);
    let deadline = Instant::now() + Duration::from_secs(300);
    while harness.state().session.run_state() != RunState::Idle {
        assert!(Instant::now() < deadline, "run did not finish");
        harness.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    harness.run();
    harness.state_mut().focus_node(&added);
    harness.run();
    let results = harness.state().session.results().unwrap();
    let new_result = results
        .report
        .results
        .get(&added)
        .map(dms_ui::results::result_kind);
    println!(
        "run: {} targets, {} root causes; new Table node: {new_result:?}",
        results.report.targets.len(),
        results.report.root_causes.len()
    );
    for error in &results.report.root_causes {
        println!("  root cause: {error}");
    }
    let out = root.join("target/render");
    std::fs::create_dir_all(&out).unwrap();
    let image = out.join("walkthrough.png");
    harness.render().unwrap().save(&image).unwrap();
    println!("wrote {} and kept {}", image.display(), saved.display());
}
