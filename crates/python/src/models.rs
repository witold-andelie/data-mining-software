//! Python model nodes: train with scikit-learn and keep the fitted model as bytes (a pickle
//! the stream stores), then score with it. The Python side is `models.py`.

use crate::{BridgeError, Mode, PythonCommand, ScriptRequest, run_script};
use dms_types::{CancelToken, Table};

const MODELS: &str = include_str!("models.py");

/// A fitted model and what training reported (records, importances, clusters, ...).
#[derive(Debug, Clone)]
pub struct Trained {
    pub model: Vec<u8>,
    pub summary: serde_json::Value,
}

/// Trains on `input`. `parameters` holds `kind`, `target`, `target_kind`, `inputs`, `name`
/// and `options` (see `models.py`).
pub fn train(
    python: &PythonCommand,
    input: &Table,
    parameters: &serde_json::Value,
    cancel: &CancelToken,
) -> Result<Trained, BridgeError> {
    let script = format!("{MODELS}\ntrain()\n");
    let mut result = run_script(
        &ScriptRequest {
            parameters: Some(parameters),
            files_out: &["model.pkl", "summary.json"],
            ..ScriptRequest::new(python, &script, Some(input), Mode::Consume)
        },
        cancel,
    )?;
    let summary = result
        .files
        .pop()
        .map(|(_, bytes)| bytes)
        .unwrap_or_default();
    let model = result
        .files
        .pop()
        .map(|(_, bytes)| bytes)
        .unwrap_or_default();
    let summary = serde_json::from_slice(&summary)
        .map_err(|e| BridgeError::Output(format!("unreadable training summary: {e}")))?;
    Ok(Trained { model, summary })
}

/// Scores `input` with a model `train` returned: a table of the score fields only, one row per
/// input record.
pub fn score(
    python: &PythonCommand,
    input: &Table,
    model: &[u8],
    cancel: &CancelToken,
) -> Result<Table, BridgeError> {
    let script = format!("{MODELS}\nscore()\n");
    let result = run_script(
        &ScriptRequest {
            files_in: &[("model.pkl", model)],
            ..ScriptRequest::new(python, &script, Some(input), Mode::Transform)
        },
        cancel,
    )?;
    result
        .table
        .ok_or_else(|| BridgeError::Output("scoring returned no data".into()))
}
