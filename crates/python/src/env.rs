//! Finding the user's Python and what it has installed.

use crate::process::{self, PythonCommand};
use crate::{BridgeError, MIN_VERSION, REQUIRED_PACKAGES};
use dms_types::CancelToken;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

/// Packages whose versions are reported (the required ones and those Python nodes use).
const REPORTED: &[&str] = &[
    "pyarrow",
    "pandas",
    "numpy",
    "sklearn",
    "xgboost",
    "imblearn",
    "hdbscan",
    "matplotlib",
];

/// Prints one JSON line: the interpreter and the version of each reported package (null when it
/// cannot be imported). Never imports user code.
const PROBE: &str = r#"
import importlib, json, sys
packages = {}
for name in sys.argv[1:]:
    try:
        module = importlib.import_module(name)
        packages[name] = str(getattr(module, "__version__", "unknown"))
    except Exception:
        packages[name] = None
print(json.dumps({"executable": sys.executable, "version": list(sys.version_info[:3]), "packages": packages}))
"#;

const DETECT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonInfo {
    pub command: PythonCommand,
    /// The interpreter's own path (`sys.executable`).
    pub executable: String,
    pub version: (u32, u32, u32),
    /// Reported packages and their versions; `None` when not importable.
    pub packages: BTreeMap<String, Option<String>>,
}

impl PythonInfo {
    pub fn version_text(&self) -> String {
        let (a, b, c) = self.version;
        format!("{a}.{b}.{c}")
    }

    /// Required packages that are not importable.
    pub fn missing(&self) -> Vec<String> {
        REQUIRED_PACKAGES
            .iter()
            .filter(|name| !matches!(self.packages.get(**name), Some(Some(_))))
            .map(|name| name.to_string())
            .collect()
    }

    /// Ok when the bridge can run scripts with this Python.
    pub fn check(&self) -> Result<(), BridgeError> {
        let python = self.command.display();
        if self.version < MIN_VERSION {
            return Err(BridgeError::TooOld {
                python,
                version: self.version_text(),
            });
        }
        let packages = self.missing();
        if !packages.is_empty() {
            return Err(BridgeError::MissingPackages { python, packages });
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Probe {
    executable: String,
    version: Vec<u32>,
    packages: BTreeMap<String, Option<String>>,
}

/// Commands tried when no Python is configured, in order.
pub fn default_candidates() -> Vec<PythonCommand> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("DMS_PYTHON").filter(|p| !p.is_empty()) {
        candidates.push(PythonCommand::new(path));
    }
    if cfg!(windows) {
        candidates.push(PythonCommand {
            program: "py".into(),
            prefix: vec!["-3".into()],
        });
        candidates.push(PythonCommand::new("python"));
        candidates.push(PythonCommand::new("python3"));
    } else {
        // An app opened from the Finder gets a minimal PATH (/usr/bin first), so the usual
        // Homebrew and python.org locations come before the system's own python3.
        if cfg!(target_os = "macos") {
            for path in ["/opt/homebrew/bin/python3", "/usr/local/bin/python3"] {
                if Path::new(path).exists() {
                    candidates.push(PythonCommand::new(path));
                }
            }
        }
        candidates.push(PythonCommand::new("python3"));
        candidates.push(PythonCommand::new("python"));
    }
    candidates
}

/// Asks one interpreter about itself.
pub fn probe(command: &PythonCommand, cancel: &CancelToken) -> Result<PythonInfo, BridgeError> {
    let dir = std::env::temp_dir();
    let mut args: Vec<OsString> = vec!["-c".into(), PROBE.into()];
    args.extend(REPORTED.iter().map(OsString::from));
    let finished = process::run(command, &args, &dir, cancel, Some(DETECT_TIMEOUT), 1 << 16)?;
    if !finished.status.success() {
        return Err(BridgeError::Process(format!(
            "{} did not start: {}",
            command.display(),
            String::from_utf8_lossy(&finished.stderr).trim()
        )));
    }
    let text = String::from_utf8_lossy(&finished.stdout);
    let line = text.lines().last().unwrap_or_default();
    let probe: Probe = serde_json::from_str(line).map_err(|e| {
        BridgeError::Process(format!("{} is not a usable Python: {e}", command.display()))
    })?;
    let part = |i: usize| probe.version.get(i).copied().unwrap_or(0);
    Ok(PythonInfo {
        command: command.clone(),
        executable: probe.executable,
        version: (part(0), part(1), part(2)),
        packages: probe.packages,
    })
}

/// The configured interpreter, or else the first default candidate that runs. A configured
/// path that does not work is an error: it is not silently replaced by another Python.
pub fn detect(configured: Option<&Path>, cancel: &CancelToken) -> Result<PythonInfo, BridgeError> {
    if let Some(path) = configured.filter(|p| !p.as_os_str().is_empty()) {
        let command = PythonCommand::new(path);
        return probe(&command, cancel).map_err(|error| match error {
            BridgeError::Io(_) => BridgeError::NotFound {
                tried: vec![command.display()],
            },
            other => other,
        });
    }
    let candidates = default_candidates();
    let mut tried = Vec::new();
    for command in &candidates {
        cancel.check().map_err(|_| BridgeError::Cancelled)?;
        match probe(command, cancel) {
            Ok(info) => return Ok(info),
            Err(BridgeError::Cancelled) => return Err(BridgeError::Cancelled),
            Err(_) => tried.push(command.display()),
        }
    }
    Err(BridgeError::NotFound { tried })
}
