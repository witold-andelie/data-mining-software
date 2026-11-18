//! Python bridge (L2): runs a script in the user's own Python as a subprocess. Data goes in and
//! comes back as Arrow IPC files in a private temporary folder; the script sees a pandas
//! DataFrame. No libpython is linked and no Python is bundled; a missing Python or package is
//! an error that says how to install it.
//!
//! Protocol: the folder holds `runner.py` (this crate's `src/runner.py`), `request.json`
//! (script text, input and output file names), `input0.arrow`; the runner writes
//! `result.json` and, when data is expected, `output.arrow`. The script's printed output is
//! its text output.

pub mod env;
mod error;
pub mod exchange;
pub mod models;
pub mod process;

pub use env::{PythonInfo, detect};
pub use error::BridgeError;
pub use process::PythonCommand;

use dms_types::{CancelToken, Table};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

/// The oldest Python the runner supports.
pub const MIN_VERSION: (u32, u32, u32) = (3, 9, 0);
/// Packages every script needs (data exchange).
pub const REQUIRED_PACKAGES: &[&str] = &["pyarrow", "pandas"];
/// Longest script accepted (bytes).
pub const MAX_SCRIPT: usize = 4 << 20;
/// Most printed output kept from a script (bytes, each of stdout and stderr).
pub const MAX_TEXT: usize = 16 << 20;

const RUNNER: &str = include_str!("runner.py");

/// What the script is for: whether it receives data and whether it must return data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// No input; returns data (Extension Import).
    Import,
    /// Input; returns data (Extension Transform).
    Transform,
    /// Input; returns text only (Extension Output, Extension Export).
    Consume,
}

/// Largest file a script may hand back (a trained model), bytes.
pub const MAX_FILE: u64 = 1 << 30;

pub struct ScriptRequest<'a> {
    pub python: &'a PythonCommand,
    pub script: &'a str,
    pub input: Option<&'a Table>,
    pub mode: Mode,
    /// Given to the script as the dict `dmsParameters`.
    pub parameters: Option<&'a serde_json::Value>,
    /// Files written into the script's working folder first (a saved model), by plain name.
    pub files_in: &'a [(&'a str, &'a [u8])],
    /// Files the script must leave in its working folder, read back by plain name.
    pub files_out: &'a [&'a str],
}

impl<'a> ScriptRequest<'a> {
    /// A plain script: no parameters or files.
    pub fn new(
        python: &'a PythonCommand,
        script: &'a str,
        input: Option<&'a Table>,
        mode: Mode,
    ) -> Self {
        Self {
            python,
            script,
            input,
            mode,
            parameters: None,
            files_in: &[],
            files_out: &[],
        }
    }
}

/// A file name without folders, so scripts cannot be pointed outside their working folder.
fn plain_name(name: &str) -> Result<&str, BridgeError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    ok.then_some(name)
        .ok_or_else(|| BridgeError::Input(format!("bad bridge file name {name:?}")))
}

#[derive(Debug)]
pub struct ScriptResult {
    /// The returned data (Import and Transform).
    pub table: Option<Table>,
    /// What the script printed.
    pub text: String,
    /// Warnings and other messages the script wrote to stderr.
    pub messages: String,
    pub truncated: bool,
    /// The requested `files_out`, in order.
    pub files: Vec<(String, Vec<u8>)>,
}

#[derive(Deserialize)]
struct RunnerResult {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    traceback: Option<String>,
    #[serde(default)]
    package: Option<String>,
}

static NONCE: AtomicU64 = AtomicU64::new(0);

/// A private temporary folder, removed when dropped.
struct WorkDir(PathBuf);

impl WorkDir {
    fn new() -> Result<Self, BridgeError> {
        let path = std::env::temp_dir().join(format!(
            "dms-python-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tail(bytes: &[u8], lines: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let all: Vec<&str> = text.trim_end().lines().collect();
    all.get(all.len().saturating_sub(lines)..)
        .unwrap_or_default()
        .join("\n")
}

/// Runs a script. Cancelling kills the Python process.
pub fn run_script(
    request: &ScriptRequest<'_>,
    cancel: &CancelToken,
) -> Result<ScriptResult, BridgeError> {
    if request.script.len() > MAX_SCRIPT {
        return Err(BridgeError::Input(format!(
            "the script is longer than {} MB",
            MAX_SCRIPT >> 20
        )));
    }
    let dir = WorkDir::new()?;
    let path = |name: &str| dir.0.join(name);
    std::fs::write(path("runner.py"), RUNNER)?;
    let mut inputs = Vec::new();
    if let Some(table) = request.input {
        exchange::write_table(table, &path("input0.arrow"))?;
        inputs.push(path("input0.arrow"));
    }
    for (name, bytes) in request.files_in {
        std::fs::write(path(plain_name(name)?), bytes)?;
    }
    for name in request.files_out {
        plain_name(name)?;
    }
    let output =
        matches!(request.mode, Mode::Import | Mode::Transform).then(|| path("output.arrow"));
    let json = serde_json::json!({
        "script": request.script,
        "inputs": inputs,
        "output": output,
        "result": path("result.json"),
        "parameters": request.parameters,
    });
    std::fs::write(path("request.json"), json.to_string())?;
    cancel.check().map_err(|_| BridgeError::Cancelled)?;
    let finished = process::run(
        request.python,
        &[path("runner.py").into(), path("request.json").into()],
        &dir.0,
        cancel,
        None,
        MAX_TEXT,
    )?;
    let text = String::from_utf8_lossy(&finished.stdout).into_owned();
    let messages = String::from_utf8_lossy(&finished.stderr).into_owned();
    let result: RunnerResult = match std::fs::read(path("result.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| BridgeError::Process(format!("unreadable result: {e}")))?,
        Err(_) => {
            return Err(BridgeError::Process(format!(
                "{} stopped ({}) before finishing: {}",
                request.python.display(),
                finished.status,
                tail(&finished.stderr, 8)
            )));
        }
    };
    match result.error.as_deref() {
        None => {}
        Some("script") => {
            return Err(BridgeError::Script {
                message: result.message.unwrap_or_default(),
                line: result.line,
                traceback: result.traceback.unwrap_or_default(),
            });
        }
        Some("missing_package") => {
            return Err(BridgeError::MissingPackages {
                python: request.python.display(),
                packages: vec![result.package.unwrap_or_else(|| "pyarrow".into())],
            });
        }
        Some(_) => return Err(BridgeError::Output(result.message.unwrap_or_default())),
    }
    let table = match &output {
        Some(file) => Some(exchange::read_table(file, request.input.map(Table::model))?),
        None => None,
    };
    let mut files = Vec::with_capacity(request.files_out.len());
    for name in request.files_out {
        let file = path(name);
        let size = std::fs::metadata(&file)
            .map_err(|_| BridgeError::Output(format!("the script did not write {name}")))?
            .len();
        if size > MAX_FILE {
            return Err(BridgeError::Output(format!(
                "{name} is larger than {} MB",
                MAX_FILE >> 20
            )));
        }
        files.push((name.to_string(), std::fs::read(&file)?));
    }
    Ok(ScriptResult {
        table,
        text,
        messages,
        truncated: finished.truncated,
        files,
    })
}

/// Interpreters that worked, by configured path (empty: found automatically), so detection
/// runs once per configuration rather than once per node.
static RESOLVED: Mutex<Vec<(PathBuf, PythonCommand)>> = Mutex::new(Vec::new());

/// The Python to run scripts with: the configured one, or else the first one found. Checks
/// the version and required packages the first time.
pub fn resolve(
    configured: Option<&Path>,
    cancel: &CancelToken,
) -> Result<PythonCommand, BridgeError> {
    let key = configured.map(Path::to_path_buf).unwrap_or_default();
    if let Some((_, command)) = RESOLVED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|(k, _)| *k == key)
    {
        return Ok(command.clone());
    }
    let info = detect(configured, cancel)?;
    info.check()?;
    let mut resolved = RESOLVED.lock().unwrap_or_else(PoisonError::into_inner);
    if resolved.len() >= 16 {
        resolved.clear();
    }
    resolved.push((key, info.command.clone()));
    Ok(info.command)
}

/// Forgets resolved interpreters (after the user changes the Python setting or installs
/// packages).
pub fn forget_resolved() {
    RESOLVED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}
