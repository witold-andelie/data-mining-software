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
        &ScriptRequest {
            python,
            script,
            input: Some(&table),
            mode: Mode::Transform,
        },
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
        &ScriptRequest {
            python: &python,
            script,
            input: None,
            mode: Mode::Import,
        },
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
        &ScriptRequest {
            python: &python,
            script: "import time\ntime.sleep(60)\n",
            input: None,
            mode: Mode::Consume,
        },
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
