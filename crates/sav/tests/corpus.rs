//! Real-file corpus: every pinned `.sav` reads (`cargo test -p dms-sav --features corpus`,
//! after `cargo xtask corpus sync ...`). The files are IBM demo data and the team's data; they
//! never enter git. When Modeler CSV exports exist under `oracle/probes/sav_to_csv/`, values
//! are compared with them.
#![cfg(feature = "corpus")]

use dms_sav::{Limits, MissingValues, SavFile, Value, read};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn corpus_files() -> Vec<PathBuf> {
    let manifest = std::fs::read_to_string(root().join("xtask/corpus.tsv")).expect("corpus.tsv");
    let files: Vec<PathBuf> = manifest
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let label = fields.next()?;
            let path = fields.next()?;
            path.ends_with(".sav")
                .then(|| root().join("corpus").join(label).join(path))
        })
        .collect();
    assert!(!files.is_empty(), "no .sav files in the corpus manifest");
    for path in &files {
        assert!(
            path.is_file(),
            "{} missing: run `cargo xtask corpus sync`",
            path.display()
        );
    }
    files
}

#[test]
fn every_corpus_sav_reads() {
    let mut failures = Vec::new();
    let mut total_rows = 0usize;
    for path in corpus_files() {
        let bytes = std::fs::read(&path).unwrap();
        match read(&bytes, &Limits::default()) {
            Ok(file) => {
                total_rows += file.rows.len();
                if file.variables.is_empty() {
                    failures.push(format!("{}: no variables", path.display()));
                }
                if let Some(n) = file.header.case_count
                    && n != file.rows.len() as u64
                {
                    failures.push(format!(
                        "{}: header says {n} cases, read {}",
                        path.display(),
                        file.rows.len()
                    ));
                }
                if !file.warnings.is_empty() {
                    eprintln!("{}: warnings {:?}", path.display(), file.warnings);
                }
            }
            Err(error) => failures.push(format!("{}: {error}", path.display())),
        }
    }
    eprintln!("read all corpus .sav files, {total_rows} rows in total");
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// RFC 4180-ish parser; returns rows of (text, was_quoted).
fn parse_csv(text: &str) -> Vec<Vec<(String, bool)>> {
    let mut rows = Vec::new();
    let mut row: Vec<(String, bool)> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                cell.push(c);
            }
            continue;
        }
        match c {
            '"' => {
                in_quotes = true;
                quoted = true;
            }
            ',' => {
                row.push((std::mem::take(&mut cell), quoted));
                quoted = false;
            }
            '\r' => {}
            '\n' => {
                row.push((std::mem::take(&mut cell), quoted));
                quoted = false;
                rows.push(std::mem::take(&mut row));
            }
            _ => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push((cell, quoted));
        rows.push(row);
    }
    rows
}

/// Days from 1970-01-01 to a civil date (Howard Hinnant's algorithm), inverse below.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// SPSS seconds (since 1582-10-14) to "YYYY-MM-DD".
fn spss_date(seconds: f64) -> String {
    const SPSS_TO_UNIX: i64 = 12_219_379_200;
    let secs = seconds.floor() as i64 - SPSS_TO_UNIX;
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

fn is_user_missing(file: &SavFile, col: usize, x: f64) -> bool {
    match file.variables.get(col).map(|v| &v.missing) {
        Some(MissingValues::Discrete(vals)) => vals.contains(&Value::Number(x)),
        Some(MissingValues::Range { low, high }) => (*low..=*high).contains(&x),
        Some(MissingValues::RangeAndDiscrete { low, high, value }) => {
            (*low..=*high).contains(&x) || x == *value
        }
        _ => false,
    }
}

fn compare(file: &SavFile, csv: &[Vec<(String, bool)>]) -> Vec<String> {
    let mut problems = Vec::new();
    let Some(header) = csv.first() else {
        return vec!["empty csv".into()];
    };
    let names: Vec<&str> = header.iter().map(|(n, _)| n.as_str()).collect();
    // Modeler may drop or rename fields; match by name.
    let columns: Vec<(usize, usize)> = names
        .iter()
        .enumerate()
        .filter_map(|(ci, n)| {
            file.variables
                .iter()
                .position(|v| v.name == *n)
                .map(|vi| (ci, vi))
        })
        .collect();
    if columns.len() != names.len() {
        problems.push(format!(
            "only {} of {} csv columns matched variable names",
            columns.len(),
            names.len()
        ));
    }
    let body = csv.get(1..).unwrap_or_default();
    if body.len() != file.rows.len() {
        problems.push(format!(
            "csv has {} rows, sav has {}",
            body.len(),
            file.rows.len()
        ));
    }
    let mut cells = 0usize;
    for (ri, (crow, vrow)) in body.iter().zip(&file.rows).enumerate() {
        for (ci, vi) in &columns {
            let (Some((s, _)), Some(v)) = (crow.get(*ci), vrow.get(*vi)) else {
                continue;
            };
            cells += 1;
            let ok = match v {
                Value::SystemMissing => s.is_empty(),
                Value::String(t) => t == s,
                Value::Number(x) => {
                    if s.is_empty() {
                        is_user_missing(file, *vi, *x)
                    } else if let Ok(y) = s.parse::<f64>() {
                        (x - y).abs() <= 0.0005 || (x - y).abs() <= 1e-9 * x.abs()
                    } else if file.variables.get(*vi).is_some_and(|v| v.is_date_time()) {
                        let d = spss_date(*x);
                        s.starts_with(&d)
                    } else {
                        false
                    }
                }
            };
            if !ok {
                problems.push(format!(
                    "row {ri} col {} ({}): sav {v:?} vs csv {s:?}",
                    ci,
                    names.get(*ci).copied().unwrap_or("?")
                ));
                if problems.len() > 8 {
                    return problems;
                }
            }
        }
    }
    eprintln!("compared {cells} cells");
    problems
}

#[test]
fn values_match_modeler_csv_exports() {
    let dir = root().join("oracle/probes/sav_to_csv");
    if !dir.is_dir() {
        eprintln!("no {}; skipping oracle comparison", dir.display());
        return;
    }
    let mut failures = Vec::new();
    let mut compared = 0;
    for path in corpus_files() {
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let csv_path = dir.join(format!("{stem}.csv"));
        let Ok(text) = std::fs::read_to_string(&csv_path) else {
            continue;
        };
        let file = read(&std::fs::read(&path).unwrap(), &Limits::default()).unwrap();
        let problems = compare(&file, &parse_csv(&text));
        compared += 1;
        for p in problems {
            failures.push(format!("{stem}: {p}"));
        }
    }
    eprintln!("compared {compared} files with Modeler CSV exports");
    assert!(compared > 0, "no CSV exports found in {}", dir.display());
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
