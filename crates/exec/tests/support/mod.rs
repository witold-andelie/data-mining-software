//! Shared helpers for the output-node oracle tests: evaluate a stream's output node the way the
//! oracle ran it, read Modeler's exported document and compare the two cell by cell.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly; each test file uses a subset"
)]

use dms_exec::{Document, DocumentTable, ExecContext, FileResolver, NodeOutput, Plan};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

/// A path the oracle recorded (absolute, in the checkout that ran it) under this checkout: the
/// part from `corpus/` or `oracle/` on, so worktrees with data junctions work too.
pub fn local_path(recorded: &str) -> PathBuf {
    let normalized = recorded.replace('\\', "/");
    ["/corpus/", "/oracle/"]
        .iter()
        .find_map(|marker| {
            normalized
                .find(marker)
                .map(|at| repo_root().join(&normalized[at + 1..]))
        })
        .unwrap_or_else(|| PathBuf::from(recorded))
}

/// The node's document, or the evaluation error (a team stream may need a node we lack).
pub fn evaluate_stream_output(
    label: &str,
    stream_path: &Path,
    node_uid: &str,
) -> Result<Arc<Document>, String> {
    evaluate_stream_output_to(label, stream_path, node_uid, None)
}

/// [`evaluate_stream_output`], writing the files the node writes into `export_dir`.
pub fn evaluate_stream_output_to(
    label: &str,
    stream_path: &Path,
    node_uid: &str,
    export_dir: Option<PathBuf>,
) -> Result<Arc<Document>, String> {
    let root = repo_root();
    let bytes = std::fs::read(stream_path).unwrap();
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();

    let data_dirs = [
        root.join("corpus").join(label),
        root.join("corpus").join("ibm-demo-data"),
    ]
    .into_iter()
    .filter(|dir| dir.is_dir())
    .collect();
    let attachments: HashMap<String, Arc<[u8]>> = archive
        .attachments
        .into_iter()
        .map(|a| (a.name, a.bytes.into()))
        .collect();
    let context = ExecContext {
        files: FileResolver { data_dirs },
        stream_properties: stream.properties.clone().map(Arc::new),
        attachments: Arc::new(attachments),
        decimal_places: Some(15),
        export_dir,
    };

    let plan = Plan::new(&stream);
    let results = plan.evaluate(&[node_uid], &context);
    match results.get(node_uid).unwrap() {
        Ok(NodeOutput::Document(doc)) => Ok(Arc::clone(doc)),
        Ok(other) => panic!("expected NodeOutput::Document for {node_uid}, got {other:?}"),
        Err(error) => Err(format!("{error:?}")),
    }
}

/// Parses one line of Modeler's CSV dialect: quotes inside quoted cells are not escaped, so a
/// quoted cell ends at the first quote followed by a comma or the end of the line.
pub fn parse_csv_line(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let mut cells = Vec::new();
    let mut i = 0;
    loop {
        if i >= bytes.len() {
            cells.push(String::new());
            break;
        }
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut end = bytes.len();
            let mut next = bytes.len();
            let mut j = start;
            while j < bytes.len() {
                if bytes[j] == b'"' && (j + 1 == bytes.len() || bytes[j + 1] == b',') {
                    end = j;
                    next = j + 1;
                    break;
                }
                j += 1;
            }
            cells.push(line[start..end].to_string());
            if next < bytes.len() && bytes[next] == b',' {
                i = next + 1;
            } else {
                break;
            }
        } else {
            let start = i;
            while i < bytes.len() && bytes[i] != b',' {
                i += 1;
            }
            cells.push(line[start..i].to_string());
            if i < bytes.len() && bytes[i] == b',' {
                i += 1;
            } else {
                break;
            }
        }
    }
    cells
}

/// A CSV export as tables: blocks separated by blank lines, the first line of each block is its
/// header (Data Audit exports two tables this way; Matrix and Means one).
pub fn parse_csv_document(text: &str) -> Document {
    let mut tables = Vec::new();
    let mut block: Vec<Vec<String>> = Vec::new();
    let mut flush = |block: &mut Vec<Vec<String>>| {
        if !block.is_empty() {
            let rows = block.split_off(1);
            let header = block.pop().unwrap_or_default();
            tables.push(DocumentTable {
                title: None,
                header,
                rows,
            });
        }
    };
    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            flush(&mut block);
        } else {
            block.push(parse_csv_line(line));
        }
    }
    flush(&mut block);
    Document {
        title: String::new(),
        tables,
    }
}

fn cells_equal(actual: &str, expected: &str) -> bool {
    if actual == expected {
        return true;
    }
    if let (Some((a_num, a_label)), Some((e_num, e_label))) =
        (actual.split_once('/'), expected.split_once('/'))
    {
        return a_label == e_label && numbers_close(a_num, e_num);
    }
    numbers_close(actual, expected)
}

/// Numeric cells printed with the same number of decimals that agree to a relative 1e-12:
/// Modeler sums in an order we do not always reproduce, so the last printed digits can differ by
/// an ulp or two. This is stricter than `xtask compare` (printed rounding plus a relative 1e-9).
/// The report prints exact text matches separately so the gap stays visible.
fn numbers_close(a: &str, b: &str) -> bool {
    let (Ok(fa), Ok(fb)) = (a.parse::<f64>(), b.parse::<f64>()) else {
        return false;
    };
    if !fa.is_finite() || !fb.is_finite() {
        return false;
    }
    let a_decimals = a.split_once('.').map_or(0, |(_, frac)| frac.len());
    let b_decimals = b.split_once('.').map_or(0, |(_, frac)| frac.len());
    a_decimals == b_decimals && (fa - fb).abs() <= 1e-12 * fa.abs().max(fb.abs()).max(1.0)
}

/// The outcome of comparing a document with the oracle's.
pub struct Comparison {
    /// Cells equal as text or within [`numbers_close`].
    pub matched: usize,
    /// Cells equal as text.
    pub exact: usize,
    pub total: usize,
    /// The first cells that differ.
    pub diffs: Vec<String>,
    /// The first cells that only match within tolerance.
    pub near: Vec<String>,
}

fn compare_cells(
    table: usize,
    row: &str,
    actual: &[String],
    expected: &[String],
    out: &mut Comparison,
) {
    for c in 0..actual.len().max(expected.len()) {
        out.total += 1;
        let a = actual.get(c).map(String::as_str);
        let e = expected.get(c).map(String::as_str);
        match (a, e) {
            (Some(a), Some(e)) if a == e => {
                out.matched += 1;
                out.exact += 1;
            }
            (Some(a), Some(e)) if cells_equal(a, e) => {
                out.matched += 1;
                if out.near.len() < 6 {
                    out.near.push(format!(
                        "table {table} {row} col {c}: within 1e-12: actual={a:?}, expected={e:?}"
                    ));
                }
            }
            _ => {
                if out.diffs.len() < 12 {
                    out.diffs.push(format!(
                        "table {table} {row} col {c}: actual={a:?}, expected={e:?}"
                    ));
                }
            }
        }
    }
}

pub fn compare_documents(actual: &Document, expected: &Document) -> Comparison {
    let mut out = Comparison {
        matched: 0,
        exact: 0,
        total: 0,
        diffs: Vec::new(),
        near: Vec::new(),
    };
    let empty = DocumentTable::default();
    for t in 0..actual.tables.len().max(expected.tables.len()) {
        let a = actual.tables.get(t);
        let e = expected.tables.get(t);
        if a.is_none() || e.is_none() {
            out.diffs.push(format!(
                "table {t}: {}",
                if a.is_none() { "missing" } else { "extra" }
            ));
        }
        let (a, e) = (a.unwrap_or(&empty), e.unwrap_or(&empty));
        // A CSV export carries no table titles; compare them only when the oracle has them.
        if e.title.is_some() {
            out.total += 1;
            if a.title == e.title {
                out.matched += 1;
                out.exact += 1;
            } else if out.diffs.len() < 12 {
                out.diffs.push(format!(
                    "table {t} title: actual={:?}, expected={:?}",
                    a.title, e.title
                ));
            }
        }
        compare_cells(t, "header", &a.header, &e.header, &mut out);
        for r in 0..a.rows.len().max(e.rows.len()) {
            let no_row = Vec::new();
            compare_cells(
                t,
                &format!("row {r}"),
                a.rows.get(r).unwrap_or(&no_row),
                e.rows.get(r).unwrap_or(&no_row),
                &mut out,
            );
        }
    }
    out
}

#[derive(Debug)]
pub struct OracleCase {
    /// The corpus label (`outputs`, `team`).
    pub label: String,
    pub stream_name: String,
    pub stream_path: PathBuf,
    pub node_id: String,
    pub format: String,
    pub output_file: PathBuf,
    /// Probe streams must match fully; team streams are reported.
    pub is_probe: bool,
}

/// Every oracle output of the given node types (`manifest.json` `"outputs"` entries).
pub fn collect_oracle_cases(types: &[&str]) -> Vec<OracleCase> {
    let mut cases = Vec::new();
    for (label, is_probe) in [
        ("outputs", true),
        ("graphs", true),
        ("graphboard", true),
        // Not strict until the graphboard_more cases are implemented (1 of 12 matches).
        ("graphboard_more", false),
        ("misc", true),
        ("misc_next", true),
        ("outputs_next", true),
        ("team", false),
    ] {
        let dir = repo_root().join("oracle").join(label);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut subdirs: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.join("manifest.json").is_file())
            .collect();
        subdirs.sort();
        for sub in subdirs {
            let text = std::fs::read_to_string(sub.join("manifest.json")).unwrap();
            let manifest: JsonValue = serde_json::from_str(&text).unwrap();
            let stream = manifest["stream"].as_str().unwrap();
            let Some(outputs) = manifest["outputs"].as_array() else {
                continue;
            };
            for entry in outputs {
                if !types.contains(&entry["type"].as_str().unwrap_or_default()) {
                    continue;
                }
                cases.push(OracleCase {
                    label: label.to_string(),
                    stream_name: sub.file_name().unwrap().to_string_lossy().into_owned(),
                    stream_path: local_path(stream),
                    node_id: entry["id"].as_str().unwrap().to_string(),
                    format: entry["format"].as_str().unwrap().to_string(),
                    output_file: local_path(entry["file"].as_str().unwrap()),
                    is_probe,
                });
            }
        }
    }
    cases
}

/// Evaluates every case, prints `<case>/<file>: <matched>/<cells> cells (<exact> exact)` and the
/// differences, and fails when a probe case does not match (within tolerance) or does not run.
pub fn check_oracle_cases(types: &[&str], read_expected: impl Fn(&OracleCase, &str) -> Document) {
    let cases = collect_oracle_cases(types);
    assert!(!cases.is_empty(), "no oracle outputs of type {types:?}");
    let mut failures = Vec::new();
    for case in &cases {
        let file = case.output_file.file_name().unwrap().to_string_lossy();
        let name = format!("{}/{file}", case.stream_name);
        let actual = match evaluate_stream_output(&case.label, &case.stream_path, &case.node_id) {
            Ok(actual) => actual,
            Err(error) => {
                println!("{name}: not evaluated ({error})");
                if case.is_probe {
                    failures.push(format!("{name}: {error}"));
                }
                continue;
            }
        };
        let text = std::fs::read_to_string(&case.output_file).unwrap();
        let expected = read_expected(case, &text);
        let result = compare_documents(&actual, &expected);
        println!(
            "{name}: {}/{} cells ({} exact)",
            result.matched, result.total, result.exact
        );
        for diff in result.diffs.iter().chain(&result.near) {
            println!("    {diff}");
        }
        if result.matched != result.total && case.is_probe {
            failures.push(format!(
                "{name}: {}/{} cells; {:?}",
                result.matched, result.total, result.diffs
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "probe oracle mismatches:\n{}",
        failures.join("\n")
    );
}
