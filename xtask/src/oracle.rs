//! Reference results from IBM SPSS Modeler, used only as a test oracle.
//!
//! `cargo xtask oracle <label> <path>` runs one pinned corpus stream through Modeler's batch mode
//! (`clemb`) with `oracle/modeler_oracle.py`, which exports the records reaching every terminal
//! node as CSV and every built model as PMML. Results land in `oracle/<label>/<stream>/`
//! (gitignored) with a digest of each file and the provenance of the run. Windows only: Modeler is
//! never linked, bundled or shipped.

use crate::corpus::{sha256_file, verified_file};
use crate::{Result, workspace_root};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const SCRIPT_TEMPLATE: &str = include_str!("../oracle/modeler_oracle.py");
const MODELER_HOME_VAR: &str = "DMS_MODELER_HOME";
const DEFAULT_TIMEOUT_SECS: u64 = 600;
const DATA_DIRS_VAR: &str = "DMS_ORACLE_DATA_DIRS";
const OUTPUT_SUBDIRS: [&str; 3] = ["models", "data", "exports"];

pub fn run(args: &[String]) -> Result<()> {
    let (label, path, timeout) = match args {
        [label, path] => (label, path, DEFAULT_TIMEOUT_SECS),
        [label, path, flag, secs] if flag == "--timeout" => (
            label,
            path,
            secs.parse::<u64>()
                .map_err(|e| format!("invalid --timeout {secs:?}: {e}"))?,
        ),
        _ => return Err("usage: oracle <label> <path> [--timeout <secs>]".into()),
    };
    if !cfg!(windows) {
        return Err("the oracle runs only on the Windows machine with IBM SPSS Modeler".into());
    }
    let home = modeler_home()?;
    let (stream, stream_sha256) = verified_file(label, path)?;
    let out = output_dir(label, path)?;
    prepare_output_dir(&out)?;
    let data_dirs = data_dirs(&home, label)?;

    let script = out.join("oracle_script.py");
    fs::write(&script, render_script(&stream, &out, &data_dirs)?)
        .map_err(|e| format!("cannot write {}: {e}", script.display()))?;
    let log = out.join("clem_batch.log");
    let started = Instant::now();
    run_clemb(&home, &script, &log, &out, Duration::from_secs(timeout))?;
    let elapsed = started.elapsed();

    let manifest = read_json(&out.join("manifest.json"))?;
    let outputs = digest_outputs(&out)?;
    let provenance = json!({
        "stream": { "label": label, "path": path, "sha256": stream_sha256 },
        "modeler_version": modeler_version(&home),
        "data_dirs": data_dirs,
        "elapsed_secs": elapsed.as_secs_f64(),
        "outputs": outputs,
    });
    let text = serde_json::to_string_pretty(&provenance)
        .map_err(|e| format!("cannot serialize provenance: {e}"))?;
    fs::write(out.join("provenance.json"), text + "\n")
        .map_err(|e| format!("cannot write provenance.json: {e}"))?;

    let count = |key: &str| {
        manifest
            .get(key)
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    };
    println!(
        "xtask oracle: {label}/{path}: {} models, {} data exports, {} relocated, {} missing data files, {} errors in {:.1}s -> {}",
        count("models"),
        count("data"),
        count("relocated"),
        count("missing_data"),
        count("errors"),
        elapsed.as_secs_f64(),
        out.display()
    );
    let entries = |key: &str| -> Vec<Value> {
        manifest
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let missing = entries("missing_data");
    let errors = entries("errors");
    if missing.is_empty() && errors.is_empty() {
        return Ok(());
    }
    let mut report = Vec::new();
    for entry in &missing {
        let path = entry.get("path").and_then(Value::as_str).unwrap_or("?");
        report.push(format!("missing data file {path}"));
    }
    report.extend(errors.iter().map(Value::to_string));
    Err(format!(
        "{} missing data files, {} errors (partial results kept; add data directories with {DATA_DIRS_VAR}):\n  {}",
        missing.len(),
        errors.len(),
        report.join("\n  ")
    ))
}

/// Directories searched by file name for data a stream references on another machine:
/// Modeler's Demos, the stream's corpus label, then `DMS_ORACLE_DATA_DIRS` entries.
fn data_dirs(home: &Path, label: &str) -> Result<Vec<String>> {
    let mut dirs = vec![
        home.join("Demos"),
        workspace_root()?.join("corpus").join(label),
    ];
    if let Some(extra) = std::env::var_os(DATA_DIRS_VAR) {
        dirs.extend(std::env::split_paths(&extra).filter(|dir| !dir.as_os_str().is_empty()));
    }
    dirs.iter()
        .map(|dir| {
            dir.to_str()
                .map(|text| text.replace('\\', "/"))
                .ok_or_else(|| format!("{} is not valid UTF-8", dir.display()))
        })
        .collect()
}

fn modeler_home() -> Result<PathBuf> {
    let home = std::env::var_os(MODELER_HOME_VAR).map(PathBuf::from).ok_or_else(|| {
        format!("set {MODELER_HOME_VAR} to the Modeler installation directory (the one containing bin\\clemb.exe)")
    })?;
    if home.join("bin").join("clemb.exe").is_file() {
        Ok(home)
    } else {
        Err(format!("{}\\bin\\clemb.exe not found", home.display()))
    }
}

/// `oracle/<label>/<path without extension, '/' replaced by "__">`.
fn output_dir(label: &str, path: &str) -> Result<PathBuf> {
    let stem = path.strip_suffix(".str").unwrap_or(path).replace('/', "__");
    if stem.is_empty() || stem.contains("..") {
        return Err(format!("unusable corpus path {path:?}"));
    }
    Ok(workspace_root()?.join("oracle").join(label).join(stem))
}

fn prepare_output_dir(out: &Path) -> Result<()> {
    if out.exists() {
        fs::remove_dir_all(out).map_err(|e| format!("cannot clear {}: {e}", out.display()))?;
    }
    for sub in OUTPUT_SUBDIRS {
        let dir = out.join(sub);
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    Ok(())
}

/// Embed paths into the Jython template as ASCII-only unicode literals. Modeler compiles the
/// script from a Unicode string, so it rejects a coding declaration; non-ASCII characters are
/// written as `\uXXXX` / `\UXXXXXXXX` escapes. A JSON string literal escapes quotes, backslashes
/// and control characters the same way a Python 2 `u"..."` literal reads them.
fn render_script(stream: &Path, out: &Path, data_dirs: &[String]) -> Result<String> {
    let literal = |text: &str| -> Result<String> {
        let json = serde_json::to_string(&text.replace('\\', "/"))
            .map_err(|e| format!("cannot encode {text:?}: {e}"))?;
        let mut ascii = String::with_capacity(json.len() + 1);
        ascii.push('u');
        for c in json.chars() {
            match u32::from(c) {
                code if code < 0x80 => ascii.push(c),
                code if code <= 0xFFFF => ascii.push_str(&format!("\\u{code:04x}")),
                code => ascii.push_str(&format!("\\U{code:08x}")),
            }
        }
        Ok(ascii)
    };
    let path_literal = |path: &Path| -> Result<String> {
        literal(
            path.to_str()
                .ok_or_else(|| format!("{} is not valid UTF-8", path.display()))?,
        )
    };
    let dirs = data_dirs
        .iter()
        .map(|dir| literal(dir))
        .collect::<Result<Vec<_>>>()?;
    Ok(SCRIPT_TEMPLATE
        .replace("@STREAM_PATH@", &path_literal(stream)?)
        .replace("@OUT_DIR@", &path_literal(out)?)
        .replace("@DATA_DIRS@", &format!("[{}]", dirs.join(", "))))
}

fn run_clemb(home: &Path, script: &Path, log: &Path, out: &Path, timeout: Duration) -> Result<()> {
    let bin = home.join("bin");
    let mut child = Command::new(bin.join("clemb.exe"))
        .arg("-script")
        .arg(script)
        .arg("-execute")
        .arg("-log")
        .arg(log)
        .arg("-directory")
        .arg(out)
        .current_dir(&bin)
        .spawn()
        .map_err(|e| format!("cannot start clemb: {e}"))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return Ok(()),
            Ok(None) if Instant::now() >= deadline => {
                let killed = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "clemb exceeded {}s and was stopped ({}); see {}",
                    timeout.as_secs(),
                    killed.map_or_else(|e| e.to_string(), |()| "killed".to_string()),
                    log.display()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(format!("cannot wait for clemb: {e}")),
        }
    }
}

fn read_json(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path).map_err(|e| {
        format!(
            "cannot read {}: {e} (the Modeler script did not finish; see clem_batch.log)",
            path.display()
        )
    })?;
    serde_json::from_str(&text).map_err(|e| format!("cannot parse {}: {e}", path.display()))
}

fn digest_outputs(out: &Path) -> Result<Vec<Value>> {
    let mut outputs = Vec::new();
    for sub in OUTPUT_SUBDIRS {
        let dir = out.join(sub);
        let mut files: Vec<PathBuf> = fs::read_dir(&dir)
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_file())
            .collect();
        files.sort();
        for file in files {
            let name = file
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("?");
            let bytes = fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
            outputs.push(json!({
                "file": format!("{sub}/{name}"),
                "bytes": bytes,
                "sha256": sha256_file(&file)?,
            }));
        }
    }
    Ok(outputs)
}

/// First line of Modeler's `version.txt`, or null when unreadable.
fn modeler_version(home: &Path) -> Value {
    fs::read_to_string(home.join("version.txt"))
        .ok()
        .and_then(|text| text.lines().next().map(|line| Value::from(line.trim())))
        .unwrap_or(Value::Null)
}
