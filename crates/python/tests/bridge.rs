//! End-to-end bridge tests against the Python on this machine. They are skipped (and say so)
//! when no Python with pyarrow and pandas is found, so CI machines without Python still pass.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use dms_python::{BridgeError, Mode, PythonCommand, ScriptRequest, resolve, run_script};
use dms_types::{CancelToken, DataModel, Field, Measurement, Role, Storage, Table, Value};
use std::path::Path;
use std::time::{Duration, Instant};

fn python() -> Option<PythonCommand> {
    match resolve(None, &CancelToken::new()) {
        Ok(command) => Some(command),
        Err(error) => {
            eprintln!("skipped: {error}");
            None
        }
    }
}

fn input() -> Table {
    let mut age = Field::new("age", Storage::Integer);
    age.measurement = Measurement::Continuous;
    age.role = Role::Input;
    let model = DataModel::new(vec![
        age,
        Field::new("income", Storage::Real),
        Field::new("name", Storage::String),
        Field::new("day", Storage::Date),
    ])
    .unwrap();
    Table::new(
        model,
        vec![
            vec![
                Value::Integer(30),
                Value::Real(1000.5),
                Value::string("Ann"),
                Value::Date(19_000),
            ],
            vec![Value::Null, Value::Null, Value::Null, Value::Null],
            vec![
                Value::Integer(41),
                Value::Real(-2.25),
                Value::string("Bő"),
                Value::Date(-365),
            ],
        ],
    )
    .unwrap()
}

fn transform(
    python: &PythonCommand,
    script: &str,
) -> Result<dms_python::ScriptResult, BridgeError> {
    let table = input();
    run_script(
        &ScriptRequest::new(python, script, Some(&table), Mode::Transform),
        &CancelToken::new(),
    )
}

#[test]
fn a_transform_adds_a_column_and_keeps_types_nulls_and_metadata() {
    let Some(python) = python() else { return };
    let result = transform(
        &python,
        "modelerData['older'] = modelerData['age'] + 1\nprint(len(modelerDataModel), 'fields')\n",
    )
    .unwrap();
    let table = result.table.unwrap();
    let model = table.model();
    assert_eq!(
        model
            .fields()
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        ["age", "income", "name", "day", "older"]
    );
    assert_eq!(model.fields()[0].measurement, Measurement::Continuous);
    assert_eq!(model.fields()[4].storage, Storage::Integer);
    assert_eq!(table.rows()[0][4], Value::Integer(31));
    assert_eq!(table.rows()[1], vec![Value::Null; 5]);
    assert_eq!(table.rows()[2][2], Value::string("Bő"));
    assert_eq!(table.rows()[2][3], Value::Date(-365));
    assert_eq!(result.text.trim(), "4 fields");
}

#[test]
fn an_import_returns_booleans_times_and_zoned_timestamps() {
    let Some(python) = python() else { return };
    let script = r#"
import pandas as pd, datetime as dt
modelerData = pd.DataFrame({
    "flag": [True, False],
    "when": pd.to_datetime(["2024-01-01T12:00:00+02:00", "2024-01-02T00:00:00+02:00"]),
    "t": [dt.time(1, 2, 3), None],
    "small": pd.Series([1, 2], dtype="int8"),
    "cat": pd.Categorical(["a", "b"]),
})
"#;
    let result = run_script(
        &ScriptRequest::new(&python, script, None, Mode::Import),
        &CancelToken::new(),
    )
    .unwrap();
    let table = result.table.unwrap();
    let storages: Vec<_> = table.model().fields().iter().map(|f| f.storage).collect();
    assert_eq!(
        storages,
        [
            Storage::Integer,
            Storage::Timestamp,
            Storage::Time,
            Storage::Integer,
            Storage::String
        ]
    );
    // 2024-01-01T10:00:00Z
    assert_eq!(table.rows()[0][1], Value::Timestamp(1_704_103_200_000_000));
    assert_eq!(table.rows()[0][2], Value::Time(3_723_000_000));
    assert_eq!(table.rows()[1][2], Value::Null);
    assert_eq!(table.rows()[1][4], Value::string("b"));
}

#[test]
fn script_errors_name_the_line() {
    let Some(python) = python() else { return };
    match transform(&python, "x = 1\ny = modelerData['nope']\n") {
        Err(BridgeError::Script { line, message, .. }) => {
            assert_eq!(line, Some(2));
            assert!(message.contains("nope"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    match transform(&python, "def f(:\n") {
        Err(BridgeError::Script { line, .. }) => assert_eq!(line, Some(1)),
        other => panic!("{other:?}"),
    }
    match transform(&python, "modelerData = 5\n") {
        Err(BridgeError::Output(message)) => assert!(message.contains("DataFrame"), "{message}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn cancelling_kills_a_running_script() {
    let Some(python) = python() else { return };
    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    let started = Instant::now();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        trigger.cancel();
    });
    let result = run_script(
        &ScriptRequest::new(
            &python,
            "import time\ntime.sleep(60)\n",
            None,
            Mode::Consume,
        ),
        &cancel,
    );
    canceller.join().unwrap();
    assert!(matches!(result, Err(BridgeError::Cancelled)), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[test]
fn a_configured_python_that_does_not_exist_is_not_found() {
    let error = resolve(
        Some(Path::new("/no/such/python-for-dms-tests")),
        &CancelToken::new(),
    )
    .unwrap_err();
    assert!(matches!(error, BridgeError::NotFound { .. }), "{error:?}");
    assert!(error.to_string().contains("Settings"));
}

#[test]
fn parameters_and_files_go_in_and_come_back() {
    let Some(python) = python() else { return };
    let parameters = serde_json::json!({"scale": 3});
    let model: &[u8] = b"saved";
    let result = run_script(
        &ScriptRequest {
            parameters: Some(&parameters),
            files_in: &[("model.bin", model)],
            files_out: &["out.bin"],
            ..ScriptRequest::new(&python, "data = open('model.bin', 'rb').read()\nopen('out.bin', 'wb').write(data * dmsParameters['scale'])\n", None, Mode::Consume)
        },
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(
        result.files,
        [("out.bin".to_string(), b"savedsavedsaved".to_vec())]
    );
    // A file outside the working folder is refused before Python starts.
    let refused = run_script(
        &ScriptRequest {
            files_out: &["../escape"],
            ..ScriptRequest::new(&python, "pass\n", None, Mode::Consume)
        },
        &CancelToken::new(),
    );
    assert!(matches!(refused, Err(BridgeError::Input(_))), "{refused:?}");
}

#[test]
fn a_missing_module_is_reported_as_a_package_to_install() {
    let Some(python) = python() else { return };
    let result = run_script(
        &ScriptRequest::new(
            &python,
            "import dms_no_such_module_xyz\n",
            None,
            Mode::Consume,
        ),
        &CancelToken::new(),
    );
    match result {
        Err(BridgeError::MissingPackages { packages, .. }) => {
            assert_eq!(packages, ["dms_no_such_module_xyz"]);
        }
        other => panic!("{other:?}"),
    }
}

fn churn() -> Table {
    let mut target = Field::new("churn", Storage::String);
    target.measurement = Measurement::Flag;
    let model = DataModel::new(vec![
        Field::new("age", Storage::Integer),
        Field::new("plan", Storage::String),
        target,
    ])
    .unwrap();
    let rows = (0..200)
        .map(|i| {
            let age = 20 + (i % 50);
            let plan = if i % 3 == 0 { "basic" } else { "premium" };
            let churn = if age > 45 && plan == "basic" {
                "T"
            } else {
                "F"
            };
            vec![
                if i == 7 {
                    Value::Null
                } else {
                    Value::Integer(age)
                },
                Value::string(plan),
                Value::string(churn),
            ]
        })
        .collect();
    Table::new(model, rows).unwrap()
}

#[test]
fn a_random_forest_trains_saves_and_scores() {
    let Some(python) = python() else { return };
    let data = churn();
    let parameters = serde_json::json!({
        "kind": "random_forest",
        "target": "churn",
        "target_kind": "categorical",
        "inputs": ["age", "plan"],
        "name": "churn",
        "options": {"n_estimators": 20, "random_seed": 3},
    });
    let trained =
        dms_python::models::train(&python, &data, &parameters, &CancelToken::new()).unwrap();
    assert!(trained.model.len() > 100);
    assert!(trained.summary["training_accuracy"].as_f64().unwrap() > 0.9);
    let scored =
        dms_python::models::score(&python, &data, &trained.model, &CancelToken::new()).unwrap();
    let names: Vec<_> = scored
        .model()
        .fields()
        .iter()
        .map(|f| f.name.clone())
        .collect();
    // Only the score fields come back.
    assert_eq!(names, ["$R-churn", "$RP-churn"]);
    let correct = data
        .rows()
        .iter()
        .zip(scored.rows())
        .filter(|(row, score)| row[2] == score[0])
        .count();
    assert!(correct > 180, "{correct}");
    // Data that already has the fields gets the next free index.
    let mut fields = data.model().fields().to_vec();
    fields.push(Field::new("$R-churn", Storage::String));
    let rows = data
        .rows()
        .iter()
        .map(|r| r.iter().cloned().chain([Value::Null]).collect())
        .collect();
    let scored_once = Table::new(DataModel::new(fields).unwrap(), rows).unwrap();
    let again =
        dms_python::models::score(&python, &scored_once, &trained.model, &CancelToken::new())
            .unwrap();
    assert!(again.model().field("$R1-churn").is_some());
}

#[test]
fn clustering_and_anomaly_models_score_every_record() {
    let Some(python) = python() else { return };
    let data = churn();
    for (kind, fields) in [
        ("gaussian_mixture", ["$GMM-m", "$GMMP-m"]),
        ("one_class_svm", ["$O-m", "$OP-m"]),
    ] {
        let parameters = serde_json::json!({
            "kind": kind, "inputs": ["age", "plan"], "name": "m", "options": {},
        });
        let trained =
            dms_python::models::train(&python, &data, &parameters, &CancelToken::new()).unwrap();
        let scored =
            dms_python::models::score(&python, &data, &trained.model, &CancelToken::new()).unwrap();
        for field in fields {
            let position = scored.model().position(field).unwrap();
            assert!(
                scored.rows().iter().all(|row| !row[position].is_null()),
                "{kind} {field}"
            );
        }
    }
}
