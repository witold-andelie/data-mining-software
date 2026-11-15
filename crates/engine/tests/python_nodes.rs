//! Python Extension nodes end to end through engine commands: add, set the script, connect,
//! save as .dms, reopen, run. Skipped (with a message) when no Python with pyarrow and pandas
//! is found.
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
