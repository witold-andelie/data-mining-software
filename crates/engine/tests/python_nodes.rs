//! Python Extension nodes end to end through engine commands: add, set the script, connect,
//! save as .dms, reopen, run. Each test is skipped (with a message) when no Python with pyarrow
//! and pandas is found, or when a library the test's nodes use (scikit-learn, imbalanced-learn)
//! is not installed.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use dms_engine::session::{Arg, RunNotice, Session};
use dms_exec::schema::ParamValue;
use dms_exec::{ErrorKind, NodeOutput};
use std::time::{Duration, Instant};

/// Whether Python is usable and has these importable modules (`sklearn`, `imblearn`, ...).
/// Says why on standard error when not, so a skipped test is visible in `--nocapture` runs.
fn python_has(modules: &[&str]) -> bool {
    let info = match dms_python::detect(None, &Default::default()) {
        Ok(info) => info,
        Err(error) => {
            eprintln!("skipped: {error}");
            return false;
        }
    };
    if let Err(error) = info.check() {
        eprintln!("skipped: {error}");
        return false;
    }
    let missing: Vec<&str> = modules
        .iter()
        .copied()
        .filter(|name| !matches!(info.packages.get(*name), Some(Some(_))))
        .collect();
    if !missing.is_empty() {
        eprintln!(
            "skipped: {} is missing the Python modules {}",
            info.command.display(),
            missing.join(", ")
        );
        return false;
    }
    true
}

fn add(session: &mut Session, kind: &str, x: i64) -> String {
    session
        .execute("node.add", &[Arg::Kind(kind.into()), Arg::Position(x, 100)])
        .unwrap();
    session.document_state().unwrap().selection[0].clone()
}

fn script(session: &mut Session, uid: &str, text: &str) {
    session
        .execute(
            "node.set_param",
            &[
                Arg::Node(uid.into()),
                Arg::Param("python_syntax".into(), ParamValue::Text(text.into())),
            ],
        )
        .unwrap();
}

fn run(session: &mut Session, uid: &str) {
    session
        .execute("run.node", &[Arg::Node(uid.into())])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let notices = session.poll_jobs();
        if notices
            .iter()
            .any(|n| matches!(n, RunNotice::Finished { .. } | RunNotice::Cancelled { .. }))
        {
            return;
        }
        assert!(Instant::now() < deadline, "the run did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn import_transform_and_output_scripts_run_after_a_save_and_reopen() {
    if let Err(error) = dms_python::resolve(None, &Default::default()) {
        eprintln!("skipped: {error}");
        return;
    }
    let mut session = Session::new();
    session.execute("file.new", &[]).unwrap();
    let import = add(&mut session, "extension_import", 0);
    let transform = add(&mut session, "extension_transform", 100);
    let output = add(&mut session, "extension_output", 200);
    script(
        &mut session,
        &import,
        "import pandas as pd\nmodelerData = pd.DataFrame({'x': [1, 2, 3]})\n",
    );
    script(
        &mut session,
        &transform,
        "# indented code survives saving\nfor _ in range(1):\n    modelerData['y'] = modelerData['x'] * 10\n",
    );
    script(
        &mut session,
        &output,
        "print(int(modelerData['y'].sum()))\nprint([f['name'] for f in modelerDataModel])\n",
    );
    for (a, b) in [(&import, &transform), (&transform, &output)] {
        session
            .execute("node.connect", &[Arg::Link(a.clone(), b.clone())])
            .unwrap();
    }
    let dir = std::env::temp_dir().join(format!("dms-python-nodes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("python.dms");
    session
        .execute("file.save_as", &[Arg::Path(path.clone())])
        .unwrap();
    let mut session = Session::new();
    session.execute("file.open", &[Arg::Path(path)]).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    run(&mut session, &output);
    let report = &session.results().unwrap().report;
    let document = match report.results.get(&output).unwrap() {
        Ok(NodeOutput::Document(document)) => document.clone(),
        other => panic!("{other:?}"),
    };
    let lines: Vec<&str> = document.tables[0]
        .rows
        .iter()
        .map(|row| row[0].as_str())
        .collect();
    assert_eq!(lines, ["60", "['x', 'y']"]);
}

#[test]
fn a_failing_script_names_its_node_and_line() {
    if let Err(error) = dms_python::resolve(None, &Default::default()) {
        eprintln!("skipped: {error}");
        return;
    }
    let mut session = Session::new();
    session.execute("file.new", &[]).unwrap();
    let import = add(&mut session, "extension_import", 0);
    script(
        &mut session,
        &import,
        "x = 1\nraise ValueError('bad data')\n",
    );
    run(&mut session, &import);
    let report = &session.results().unwrap().report;
    let error = report.results.get(&import).unwrap().as_ref().unwrap_err();
    assert_eq!(error.uid, import);
    match &error.kind {
        ErrorKind::Data(message) => {
            assert!(message.contains("line 2"), "{message}");
            assert!(message.contains("bad data"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

fn param(session: &mut Session, uid: &str, key: &str, value: ParamValue) {
    session
        .execute(
            "node.set_param",
            &[Arg::Node(uid.into()), Arg::Param(key.into(), value)],
        )
        .unwrap();
}

fn nuggets(session: &Session, builder: &str) -> Vec<String> {
    session
        .document()
        .unwrap()
        .stream
        .diagram
        .nodes
        .iter()
        .filter(|n| n.model_builder.as_deref() == Some(builder))
        .map(|n| n.uid.clone())
        .collect()
}

#[test]
fn a_random_forest_generates_a_nugget_that_scores_and_survives_saving() {
    if !python_has(&["sklearn"]) {
        return;
    }
    let mut session = Session::new();
    session.execute("file.new", &[]).unwrap();
    let source = add(&mut session, "extension_import", 0);
    script(
        &mut session,
        &source,
        "import pandas as pd\nrows = [(20 + i % 50, 'basic' if i % 3 == 0 else 'premium') for i in range(300)]\nmodelerData = pd.DataFrame(rows, columns=['age', 'plan'])\nmodelerData['churn'] = ['T' if a > 45 and p == 'basic' else 'F' for a, p in rows]\n",
    );
    let forest = add(&mut session, "random_forest", 100);
    param(
        &mut session,
        &forest,
        "custom_fields",
        ParamValue::Bool(true),
    );
    param(
        &mut session,
        &forest,
        "target",
        ParamValue::Text("churn".into()),
    );
    param(
        &mut session,
        &forest,
        "inputs",
        ParamValue::Fields(vec!["age".into(), "plan".into()]),
    );
    param(&mut session, &forest, "n_estimators", ParamValue::Int(25));
    session
        .execute("node.connect", &[Arg::Link(source.clone(), forest.clone())])
        .unwrap();
    session
        .execute("edit.select", &[Arg::Nodes(vec![forest.clone()])])
        .unwrap();
    let mut notices = Vec::new();
    session
        .execute("run.node", &[Arg::Node(forest.clone())])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    while notices.is_empty() {
        notices = session.poll_jobs();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    match &notices[0] {
        RunNotice::Finished { models, .. } => assert_eq!(models, &Ok(1)),
        other => panic!("{other:?}"),
    }
    let nugget = nuggets(&session, &forest);
    assert_eq!(nugget.len(), 1);
    let nugget = nugget[0].clone();
    // Fed from the build node's input, and one undo step removes it again.
    let links = &session.document().unwrap().stream.diagram.links;
    assert!(
        links.iter().any(|l| format!("{:?}", l.to).contains(&nugget)
            && format!("{:?}", l.from).contains(&source))
    );
    assert_eq!(session.undo_label(), Some("Generate Model"));
    run(&mut session, &nugget);
    let check = |session: &Session| {
        let table = match session
            .results()
            .unwrap()
            .report
            .results
            .get(&nugget)
            .unwrap()
        {
            Ok(NodeOutput::Data(table)) => table.clone(),
            other => panic!("{other:?}"),
        };
        let model = table.model();
        let (truth, predicted) = (
            model.position("churn").unwrap(),
            model.position("$R-churn").unwrap(),
        );
        assert!(model.field("$RP-churn").is_some());
        let correct = table
            .rows()
            .iter()
            .filter(|r| r[truth] == r[predicted])
            .count();
        assert!(correct * 10 > table.row_count() * 9, "{correct}");
    };
    check(&session);
    // Saved with the stream and scored again after reopening.
    let dir = std::env::temp_dir().join(format!("dms-python-models-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("forest.dms");
    session
        .execute("file.save_as", &[Arg::Path(path.clone())])
        .unwrap();
    let mut session = Session::new();
    session.execute("file.open", &[Arg::Path(path)]).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    run(&mut session, &nugget);
    check(&session);
    // Training again updates the same nugget.
    run(&mut session, &forest);
    assert_eq!(nuggets(&session, &forest), [nugget]);
}

fn output(session: &Session, uid: &str) -> NodeOutput {
    session
        .results()
        .unwrap()
        .report
        .results
        .get(uid)
        .unwrap()
        .clone()
        .unwrap()
}

#[test]
fn extension_model_tsne_smote_and_kde_simulation_run_as_nodes() {
    if !python_has(&["sklearn", "imblearn"]) {
        return;
    }
    let mut session = Session::new();
    session.execute("file.new", &[]).unwrap();
    let source = add(&mut session, "extension_import", 0);
    script(
        &mut session,
        &source,
        "import pandas as pd\nrows = [(20 + i % 50, 'basic' if i % 3 == 0 else 'premium') for i in range(120)]\nmodelerData = pd.DataFrame(rows, columns=['age', 'plan'])\nmodelerData['churn'] = ['T' if a > 60 else 'F' for a, p in rows]\n",
    );
    let model = add(&mut session, "extension_model", 100);
    param(
        &mut session,
        &model,
        "python_build_syntax",
        ParamValue::Text("modelerModel = float(modelerData['age'].mean())\n".into()),
    );
    param(
        &mut session,
        &model,
        "python_score_syntax",
        ParamValue::Text("modelerData['$E-old'] = modelerData['age'] > modelerModel\n".into()),
    );
    let mut nodes = vec![model.clone()];
    for (kind, x) in [("tsne", 200), ("smote", 300), ("kde_simulation", 400)] {
        nodes.push(add(&mut session, kind, x));
    }
    for uid in &nodes {
        session
            .execute("node.connect", &[Arg::Link(source.clone(), uid.clone())])
            .unwrap();
    }
    // Explicit fields (no Type node in this stream).
    let tsne = &nodes[1];
    let smote = &nodes[2];
    let simulation = &nodes[3];
    for uid in [tsne, smote, simulation] {
        param(&mut session, uid, "custom_fields", ParamValue::Bool(true));
    }
    param(
        &mut session,
        tsne,
        "inputs",
        ParamValue::Fields(vec!["age".into(), "plan".into()]),
    );
    param(
        &mut session,
        tsne,
        "color_field",
        ParamValue::Text("churn".into()),
    );
    param(
        &mut session,
        smote,
        "target",
        ParamValue::Text("churn".into()),
    );
    param(
        &mut session,
        smote,
        "inputs",
        ParamValue::Fields(vec!["age".into(), "plan".into()]),
    );
    param(
        &mut session,
        simulation,
        "inputs",
        ParamValue::Fields(vec!["age".into()]),
    );
    param(&mut session, simulation, "records", ParamValue::Int(50));

    run(&mut session, &model);
    let nugget = nuggets(&session, &model);
    assert_eq!(nugget.len(), 1);
    run(&mut session, &nugget[0]);
    match output(&session, &nugget[0]) {
        NodeOutput::Data(table) => assert!(table.model().field("$E-old").is_some()),
        other => panic!("{other:?}"),
    }
    run(&mut session, tsne);
    match output(&session, tsne) {
        NodeOutput::Document(document) => {
            assert_eq!(document.tables[0].header, ["tSNE-1", "tSNE-2", "churn"]);
            assert_eq!(document.tables[0].rows.len(), 120);
        }
        other => panic!("{other:?}"),
    }
    run(&mut session, smote);
    match output(&session, smote) {
        NodeOutput::Data(table) => assert!(table.row_count() > 120),
        other => panic!("{other:?}"),
    }
    run(&mut session, simulation);
    match output(&session, simulation) {
        NodeOutput::Data(table) => assert_eq!(table.row_count(), 50),
        other => panic!("{other:?}"),
    }
}
