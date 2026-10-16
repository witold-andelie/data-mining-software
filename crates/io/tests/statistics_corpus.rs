//! Every corpus `.sav` read as a Statistics File node: values, storage, measurement, role and
//! labels against the Modeler exports in `oracle/probes/sav_to_csv/` (`cargo test -p dms-io
//! --features corpus` after `cargo xtask corpus sync`).
#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_io::statistics::{StatisticsOptions, read_table};
use dms_types::{Measurement, Role, Storage, Table};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Columns of a `*.fields.json` dump (Jython `json.dump(indent=1)`), as key to raw text.
fn oracle_columns(text: &str) -> Vec<BTreeMap<String, String>> {
    let mut columns = Vec::new();
    let mut current = BTreeMap::new();
    for line in text.lines().map(str::trim_end) {
        if line == "  {" {
            current = BTreeMap::new();
        } else if line == "  }" || line == "  }," {
            columns.push(std::mem::take(&mut current));
        } else if line.starts_with("   \"")
            && let Some((key, value)) = line.trim().trim_end_matches(',').split_once(": ")
        {
            current.insert(
                key.trim_matches('"').into(),
                value
                    .trim_matches('"')
                    .replace("\\\"", "\"")
                    .replace("\\\\", "\\"),
            );
        }
    }
    columns
}

#[derive(Debug, PartialEq)]
struct Cell {
    text: String,
    quoted: bool,
}

fn parse_csv(text: &str) -> Vec<Vec<Cell>> {
    let (mut rows, mut row, mut cell) = (Vec::new(), Vec::new(), String::new());
    let (mut quoted, mut in_quotes) = (false, false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    cell.push('"');
                }
                '"' => in_quotes = false,
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
    rows
}

fn same(expected: &Cell, actual: &Cell) -> bool {
    if expected.quoted != actual.quoted {
        return false;
    }
    if expected.quoted || expected.text == actual.text {
        return expected.text == actual.text;
    }
    match (expected.text.parse::<f64>(), actual.text.parse::<f64>()) {
        (Ok(a), Ok(b)) => (a - b).abs() <= 1e-12 * a.abs().max(b.abs()).max(1.0),
        _ => false,
    }
}

fn storage_name(storage: Storage) -> &'static str {
    match storage {
        Storage::String => "String",
        Storage::Integer => "Integer",
        Storage::Real => "Real",
        Storage::Date => "Date",
        Storage::Time => "Time",
        Storage::Timestamp => "Timestamp",
        Storage::List => "List",
        Storage::Unknown => "Unknown",
    }
}

fn measure_name(measurement: Measurement) -> &'static str {
    match measurement {
        Measurement::Continuous => "Range",
        Measurement::Nominal => "Set",
        Measurement::Ordinal => "OrderedSet",
        _ => "other",
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Input => "Input",
        Role::Target => "Target",
        Role::Both => "Both",
        Role::None => "None",
        Role::Partition => "Partition",
        Role::Split => "Split",
        _ => "other",
    }
}

fn check(base: &str, path: &Path, problems: &mut Vec<String>) {
    let table: Table = read_table(&std::fs::read(path).unwrap(), StatisticsOptions::default())
        .unwrap_or_else(|error| panic!("{base}: {error}"));
    let dir = root().join("oracle/probes/sav_to_csv");
    let columns =
        oracle_columns(&std::fs::read_to_string(dir.join(format!("{base}.fields.json"))).unwrap());
    let fields = table.model().fields();
    if fields.len() != columns.len() {
        problems.push(format!(
            "{base}: {} fields, Modeler {}",
            fields.len(),
            columns.len()
        ));
        return;
    }
    for (field, column) in fields.iter().zip(&columns) {
        let name = column.get("name").map_or("", String::as_str);
        let mut differ = |what: &str, ours: String, theirs: Option<&String>| {
            if theirs.is_some_and(|theirs| *theirs != ours) && problems.len() < 40 {
                problems.push(format!(
                    "{base}.{name}: {what} ours {ours}, Modeler {theirs:?}"
                ));
            }
        };
        differ("name", field.name.clone(), column.get("name"));
        differ(
            "storage",
            storage_name(field.storage).into(),
            column.get("storage"),
        );
        differ(
            "measure",
            measure_name(field.measurement).into(),
            column.get("measure"),
        );
        differ("role", role_name(field.role).into(), column.get("role"));
        differ(
            "label",
            field.label.clone().unwrap_or_default(),
            column.get("label").filter(|l| *l != "null"),
        );
    }
    let mut out = Vec::new();
    dms_io::csv::write_table(&table, &mut out).unwrap();
    let ours = parse_csv(&String::from_utf8(out).unwrap());
    let theirs = parse_csv(&std::fs::read_to_string(dir.join(format!("{base}.csv"))).unwrap());
    if ours.len() != theirs.len() {
        problems.push(format!(
            "{base}: {} rows, Modeler {}",
            ours.len(),
            theirs.len()
        ));
        return;
    }
    let mut reported = 0;
    for (index, (want, got)) in theirs.iter().zip(&ours).enumerate() {
        for (column, (w, g)) in want.iter().zip(got).enumerate() {
            if !same(w, g) && reported < 3 {
                reported += 1;
                problems.push(format!(
                    "{base} row {index} col {column}: Modeler {w:?}, ours {g:?}"
                ));
            }
        }
    }
}

#[test]
fn corpus_sav_files_match_modeler() {
    let mut problems = Vec::new();
    let mut count = 0;
    for dir in ["corpus/ibm-demo-data", "corpus/team"] {
        let Ok(entries) = std::fs::read_dir(root().join(dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(base) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".sav"))
            else {
                continue;
            };
            if root()
                .join(format!("oracle/probes/sav_to_csv/{base}.csv"))
                .is_file()
            {
                check(base, &path, &mut problems);
                count += 1;
            }
        }
    }
    assert!(count >= 30, "only {count} files checked");
    assert!(
        problems.is_empty(),
        "{count} files, problems:\n{}",
        problems.join("\n")
    );
}
