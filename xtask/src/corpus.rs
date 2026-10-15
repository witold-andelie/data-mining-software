//! Real-file corpus: IBM demo streams and the team's own streams.
//!
//! The files never enter git. `xtask/corpus.tsv` pins each file by label, relative path and
//! SHA-256; `corpus sync` copies them from a local source directory into `corpus/<label>/`.

use crate::{Result, workspace_root};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const MANIFEST: &str = "xtask/corpus.tsv";
const MAX_DEPTH: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    label: String,
    path: String,
    sha256: String,
}

pub fn run(args: &[String]) -> Result<()> {
    match args {
        [cmd, label, dir, rest @ ..] if cmd == "record" => {
            let recursive = !rest.iter().any(|arg| arg == "--flat");
            let extensions = rest
                .iter()
                .find(|arg| !arg.starts_with("--"))
                .map(|list| list.split(',').map(str::to_lowercase).collect())
                .unwrap_or_else(|| vec!["str".to_string()]);
            record(label, Path::new(dir), &extensions, recursive)
        }
        [cmd, label, dir] if cmd == "sync" => sync(label, Path::new(dir)),
        [cmd] if cmd == "verify" => verify(),
        _ => {
            Err("usage: corpus record <label> <dir> [ext,...|none] [--flat] | sync <label> <dir> | verify".into())
        }
    }
}

/// `extensions` may include `none` for files without an extension; `recursive: false` records only
/// the top level of `dir`.
fn record(label: &str, dir: &Path, extensions: &[String], recursive: bool) -> Result<()> {
    if label.is_empty() || label.contains(['\t', '/', '\\']) {
        return Err(format!("invalid label {label:?}"));
    }
    let mut files = Vec::new();
    collect(dir, dir, extensions, recursive, 0, &mut files)?;
    files.sort();
    let mut entries: Vec<Entry> = read_manifest()?
        .into_iter()
        .filter(|entry| entry.label != label)
        .collect();
    for relative in &files {
        entries.push(Entry {
            label: label.to_string(),
            path: relative.clone(),
            sha256: sha256_file(&dir.join(relative))?,
        });
    }
    entries.sort_by(|a, b| (&a.label, &a.path).cmp(&(&b.label, &b.path)));
    write_manifest(&entries)?;
    println!("xtask corpus: recorded {} files for {label}", files.len());
    Ok(())
}

fn sync(label: &str, dir: &Path) -> Result<()> {
    let entries: Vec<Entry> = read_manifest()?
        .into_iter()
        .filter(|entry| entry.label == label)
        .collect();
    if entries.is_empty() {
        return Err(format!("no manifest entries for label {label}"));
    }
    let target = workspace_root()?.join("corpus").join(label);
    let mut problems = Vec::new();
    for entry in &entries {
        let source = dir.join(&entry.path);
        let digest = match sha256_file(&source) {
            Ok(digest) => digest,
            Err(e) => {
                problems.push(e);
                continue;
            }
        };
        if digest != entry.sha256 {
            problems.push(format!(
                "{}: sha256 {digest} != pinned {}",
                entry.path, entry.sha256
            ));
            continue;
        }
        let destination = target.join(&entry.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        fs::copy(&source, &destination)
            .map_err(|e| format!("cannot copy {}: {e}", source.display()))?;
    }
    report(label, entries.len(), problems)
}

fn verify() -> Result<()> {
    let root = workspace_root()?.join("corpus");
    let entries = read_manifest()?;
    let mut problems = Vec::new();
    for entry in &entries {
        let path = root.join(&entry.label).join(&entry.path);
        match sha256_file(&path) {
            Ok(digest) if digest == entry.sha256 => {}
            Ok(digest) => problems.push(format!("{}: sha256 {digest} != pinned", path.display())),
            Err(e) => problems.push(e),
        }
    }
    report("all labels", entries.len(), problems)
}

fn report(scope: &str, total: usize, problems: Vec<String>) -> Result<()> {
    if problems.is_empty() {
        println!("xtask corpus: {total} files ok ({scope})");
        Ok(())
    } else {
        Err(format!(
            "{} of {total} files failed ({scope}):\n  {}",
            problems.len(),
            problems.join("\n  ")
        ))
    }
}

fn collect(
    root: &Path,
    dir: &Path,
    extensions: &[String],
    recursive: bool,
    depth: usize,
    out: &mut Vec<String>,
) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(format!(
            "{} is nested deeper than {MAX_DEPTH}",
            dir.display()
        ));
    }
    let entries = fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
        if kind.is_dir() {
            if recursive {
                collect(root, &path, extensions, recursive, depth + 1, out)?;
            }
        } else if kind.is_file() && has_extension(&path, extensions) {
            out.push(relative_path(root, &path)?);
        }
    }
    Ok(())
}

fn has_extension(path: &Path, extensions: &[String]) -> bool {
    match path.extension() {
        None => extensions.iter().any(|wanted| wanted == "none"),
        Some(ext) => ext.to_str().is_some_and(|ext| {
            extensions
                .iter()
                .any(|wanted| wanted.eq_ignore_ascii_case(ext))
        }),
    }
}

/// Manifest paths use `/` so they are identical on Windows and macOS.
fn relative_path(root: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|e| format!("{} is outside {}: {e}", path.display(), root.display()))?;
    let parts: Option<Vec<&str>> = relative
        .components()
        .map(|c| c.as_os_str().to_str())
        .collect();
    let parts = parts.ok_or_else(|| format!("{} is not valid UTF-8", path.display()))?;
    let joined = parts.join("/");
    if joined.contains('\t') {
        return Err(format!("{joined:?} contains a tab"));
    }
    Ok(joined)
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// The synced copy of a pinned corpus file, verified against its manifest digest.
pub fn verified_file(label: &str, path: &str) -> Result<(PathBuf, String)> {
    let entry = read_manifest()?
        .into_iter()
        .find(|entry| entry.label == label && entry.path == path)
        .ok_or_else(|| format!("{label}/{path} is not in {MANIFEST}"))?;
    let file = workspace_root()?.join("corpus").join(label).join(path);
    let digest = sha256_file(&file)
        .map_err(|e| format!("{e} (run `cargo xtask corpus sync {label} <dir>`)"))?;
    if digest != entry.sha256 {
        return Err(format!(
            "{}: sha256 {digest} != pinned {}",
            file.display(),
            entry.sha256
        ));
    }
    Ok((file, digest))
}

fn manifest_path() -> Result<PathBuf> {
    Ok(workspace_root()?.join(MANIFEST))
}

fn read_manifest() -> Result<Vec<Entry>> {
    let path = manifest_path()?;
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read {MANIFEST}: {e}")),
    };
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(
            |line| match line.split('\t').collect::<Vec<_>>().as_slice() {
                [label, path, sha256] => Ok(Entry {
                    label: (*label).to_string(),
                    path: (*path).to_string(),
                    sha256: (*sha256).to_string(),
                }),
                _ => Err(format!("malformed {MANIFEST} line: {line:?}")),
            },
        )
        .collect()
}

fn write_manifest(entries: &[Entry]) -> Result<()> {
    let mut text =
        String::from("# label\tpath\tsha256 (files live in corpus/<label>/, never in git)\n");
    for entry in entries {
        text.push_str(&format!(
            "{}\t{}\t{}\n",
            entry.label, entry.path, entry.sha256
        ));
    }
    fs::write(manifest_path()?, text).map_err(|e| format!("cannot write {MANIFEST}: {e}"))
}
