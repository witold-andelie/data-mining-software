//! `cargo xtask compare <label> <path> [--strict]`: run a corpus stream with `dms-cli run` and
//! compare, node by node, the data reaching each terminal node with the Modeler oracle export
//! (`cargo xtask oracle` must have run for the stream).
//!
//! Cells: quoted text must match exactly; empty unquoted cells are nulls; unquoted numbers match
//! within the rounding of the oracle's printed decimals or a relative 1e-9; other unquoted text
//! (dates, times) must match exactly.

use crate::{Result, cargo, oracle, workspace_root};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

/// Mismatching cells reported per node.
const MAX_REPORTED: usize = 5;

pub fn run(args: &[String]) -> Result<()> {
    let strict = args.iter().any(|arg| arg == "--strict");
    let positional: Vec<&String> = args.iter().filter(|arg| !arg.starts_with("--")).collect();
    let [label, path] = positional.as_slice() else {
        return Err("usage: compare <label> <path> [--strict]".into());
    };
    let root = workspace_root()?;
    let oracle_dir = oracle::output_dir(label, path)?;
    let manifest = read_json(&oracle_dir.join("manifest.json"))
        .map_err(|e| format!("{e} (run `cargo xtask oracle {label} {path}` first)"))?;
    let stem = oracle_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("bad oracle directory")?
        .to_string();
    // Under the active target directory, so parallel agents do not overwrite each other.
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let ours = target.join("compare").join(label.as_str()).join(&stem);
    if ours.exists() {
        fs::remove_dir_all(&ours).map_err(|e| format!("cannot clear {}: {e}", ours.display()))?;
    }
    let stream = root.join("corpus").join(label.as_str()).join(path.as_str());
    let mut run_args: Vec<String> = vec![
        "run".into(),
        "--quiet".into(),
        "-p".into(),
        "dms-cli".into(),
        "--".into(),
        "run".into(),
        path_text(&stream)?,
        "--out".into(),
        path_text(&ours)?,
    ];
    for dir in data_dirs(&root, label) {
        run_args.push("--data-dir".into());
        run_args.push(dir);
    }
    let run_args: Vec<&str> = run_args.iter().map(String::as_str).collect();
    cargo(&run_args)?;
    let report = read_json(&ours.join("run.json"))?;

    let entries = manifest
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut rows = Vec::new();
    let mut matched = 0usize;
    for entry in &entries {
        let uid = entry.get("id").and_then(Value::as_str).unwrap_or("?");
        let kind = entry.get("type").and_then(Value::as_str).unwrap_or("?");
        let file = entry.get("file").and_then(Value::as_str).unwrap_or("");
        let expected = oracle_dir.join("data").join(
            Path::new(file)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(""),
        );
        let actual = ours.join(format!("{uid}.csv"));
        let status = if actual.is_file() {
            match compare_files(&expected, &actual) {
                Ok(problems) if problems.is_empty() => {
                    matched += 1;
                    json!({ "status": "match" })
                }
                Ok(problems) => json!({ "status": "mismatch", "problems": problems }),
                Err(error) => json!({ "status": "error", "problems": [error] }),
            }
        } else {
            json!({ "status": "not produced", "problems": [failure_reason(&report, uid)] })
        };
        println!(
            "  {:<14} {:<16} {:<14} {}",
            status.get("status").and_then(Value::as_str).unwrap_or("?"),
            uid,
            kind,
            status
                .get("problems")
                .and_then(Value::as_array)
                .and_then(|p| p.first())
                .and_then(Value::as_str)
                .unwrap_or("")
        );
        rows.push(json!({ "uid": uid, "type": kind, "result": status }));
    }
    let summary = json!({ "stream": format!("{label}/{path}"), "matched": matched, "total": entries.len(), "nodes": rows });
    let text = serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?;
    fs::write(ours.join("compare.json"), text + "\n")
        .map_err(|e| format!("cannot write compare.json: {e}"))?;
    println!(
        "xtask compare: {label}/{path}: {matched}/{} terminal inputs match Modeler",
        entries.len()
    );
    if strict && matched != entries.len() {
        return Err("mismatches (see compare.json)".into());
    }
    Ok(())
}

fn data_dirs(root: &Path, label: &str) -> Vec<String> {
    let mut dirs = vec![
        root.join("corpus").join(label),
        root.join("corpus").join("ibm-demo-data"),
    ];
    if let Some(home) = std::env::var_os("DMS_MODELER_HOME") {
        dirs.push(Path::new(&home).join("Demos"));
    }
    dirs.iter()
        .filter(|dir| dir.is_dir())
        .filter_map(|dir| dir.to_str().map(str::to_string))
        .collect()
}

fn path_text(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{} is not valid UTF-8", path.display()))
}

fn read_json(path: &Path) -> Result<Value> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("cannot parse {}: {e}", path.display()))
}

fn failure_reason(report: &Value, uid: &str) -> String {
    report
        .get("failed")
        .and_then(Value::as_array)
        .and_then(|failed| {
            failed
                .iter()
                .find(|f| f.get("uid").and_then(Value::as_str) == Some(uid))
        })
        .and_then(|f| f.get("error").and_then(Value::as_str))
        .unwrap_or("not exported by dms-cli")
        .to_string()
}

#[derive(Debug, Clone, PartialEq)]
struct Cell {
    text: String,
    quoted: bool,
}

fn compare_files(expected: &Path, actual: &Path) -> Result<Vec<String>> {
    let read = |path: &Path, dialect: Dialect| {
        fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))
            .map(|text| parse_csv(text.trim_start_matches('\u{feff}'), dialect))
    };
    let (expected, actual) = (
        read(expected, Dialect::Modeler)?,
        read(actual, Dialect::Rfc4180)?,
    );
    let mut problems = Vec::new();
    let header = |rows: &[Vec<Cell>]| -> Vec<String> {
        rows.first()
            .map(|row| row.iter().map(|cell| cell.text.clone()).collect())
            .unwrap_or_default()
    };
    let (expected_header, actual_header) = (header(&expected), header(&actual));
    if expected_header != actual_header {
        problems.push(format!(
            "fields differ: Modeler {expected_header:?}, ours {actual_header:?}"
        ));
        return Ok(problems);
    }
    if expected.len() != actual.len() {
        problems.push(format!(
            "row count: Modeler {}, ours {}",
            expected.len().saturating_sub(1),
            actual.len().saturating_sub(1)
        ));
    }
    for (index, (want, got)) in expected.iter().zip(&actual).enumerate().skip(1) {
        for (column, (w, g)) in want.iter().zip(got).enumerate() {
            if !cells_match(w, g) && problems.len() < MAX_REPORTED {
                let field = expected_header.get(column).map_or("?", String::as_str);
                problems.push(format!(
                    "row {index} field {field}: Modeler {w}, ours {g}",
                    w = display(w),
                    g = display(g)
                ));
            }
        }
    }
    Ok(problems)
}

fn display(cell: &Cell) -> String {
    if cell.quoted {
        format!("\"{}\"", cell.text)
    } else if cell.text.is_empty() {
        "$null$".into()
    } else {
        cell.text.clone()
    }
}

fn cells_match(expected: &Cell, actual: &Cell) -> bool {
    match (expected.quoted, actual.quoted) {
        (true, true) => expected.text == actual.text,
        (false, false) => {
            if expected.text == actual.text {
                return true;
            }
            match (expected.text.parse::<f64>(), actual.text.parse::<f64>()) {
                (Ok(a), Ok(b)) => {
                    let decimals = expected
                        .text
                        .split_once('.')
                        .map_or(0, |(_, fraction)| fraction.len());
                    let rounding = if decimals > 0 {
                        0.5 * 10f64.powi(-i32::try_from(decimals).unwrap_or(15))
                    } else {
                        0.0
                    };
                    // The printed rounding plus a relative error: a half-way value such as
                    // 40257.7955 printed as 40257.796 differs by slightly more than 0.0005 in
                    // binary.
                    let relative = 1e-9 * a.abs().max(b.abs()).max(1.0);
                    (a - b).abs() <= rounding + relative
                }
                _ => false,
            }
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    /// Our exports: quoted cells may contain separators, doubled quotes and newlines.
    Rfc4180,
    /// Modeler's Flat File export does not escape quotes inside quoted cells (oracle:
    /// `"a\"b"` is exported as `"a"b"`), so a quote closes a cell only before a separator, a
    /// line end or the end of the file.
    Modeler,
}

fn parse_csv(text: &str, dialect: Dialect) -> Vec<Vec<Cell>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            match (c, dialect) {
                ('"', Dialect::Rfc4180) if chars.peek() == Some(&'"') => {
                    chars.next();
                    cell.push('"');
                }
                ('"', Dialect::Modeler)
                    if !matches!(chars.peek(), None | Some(',' | '\r' | '\n')) =>
                {
                    cell.push('"');
                }
                ('"', _) => in_quotes = false,
                _ => cell.push(c),
            }
            continue;
        }
        match c {
            '"' => {
                in_quotes = true;
                quoted = true;
            }
            ',' => row.push(Cell {
                text: std::mem::take(&mut cell),
                quoted: std::mem::take(&mut quoted),
            }),
            '\r' => {}
            '\n' => {
                row.push(Cell {
                    text: std::mem::take(&mut cell),
                    quoted: std::mem::take(&mut quoted),
                });
                rows.push(std::mem::take(&mut row));
            }
            _ => cell.push(c),
        }
    }
    if !cell.is_empty() || quoted || !row.is_empty() {
        row.push(Cell { text: cell, quoted });
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(text: &str, quoted: bool) -> Cell {
        Cell {
            text: text.into(),
            quoted,
        }
    }

    #[test]
    fn parses_quotes_and_nulls() {
        let rows = parse_csv("a,b\r\n\"x,\"\"y\"\"\",,\n1.5,\"\"\n", Dialect::Rfc4180);
        assert_eq!(
            rows[1],
            vec![c("x,\"y\"", true), c("", false), c("", false)]
        );
        assert_eq!(rows[2], vec![c("1.5", false), c("", true)]);
    }

    #[test]
    fn reads_modeler_unescaped_quotes() {
        // Modeler exports the value a"b as "a"b" and a lone quote as """.
        let rows = parse_csv(
            r#"x,y,z
"a"b",""",3
"#,
            Dialect::Modeler,
        );
        assert_eq!(rows[1], vec![c("a\"b", true), c("\"", true), c("3", false)]);
    }

    #[test]
    fn half_way_rounding_matches() {
        assert!(cells_match(&c("40257.796", false), &c("40257.7955", false)));
        assert!(!cells_match(
            &c("40257.796", false),
            &c("40257.7944", false)
        ));
    }

    #[test]
    fn numbers_match_within_printed_rounding() {
        assert!(cells_match(&c("3.700", false), &c("3.7", false)));
        assert!(cells_match(&c("25.355", false), &c("25.3549999", false)));
        assert!(!cells_match(&c("25.355", false), &c("25.357", false)));
        assert!(cells_match(&c("2.000", false), &c("2", false)));
        assert!(!cells_match(&c("2", false), &c("2.4", false)));
        assert!(!cells_match(&c("", false), &c("0", false)));
        assert!(!cells_match(&c("MALE  ", true), &c("MALE", true)));
        assert!(cells_match(
            &c("1995-01-05", false),
            &c("1995-01-05", false)
        ));
    }
}
