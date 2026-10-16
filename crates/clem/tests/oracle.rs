//! CLEM against Modeler 19 on every expression of the `clem_semantics` probe
//! (`cargo xtask probe clem_semantics`, then `cargo test -p dms-clem --features corpus`).
//!
//! The probe derives one field per expression from its own nine-record input and exports the
//! records; each expression here is evaluated on the same records and printed the way Modeler
//! exports it. Expressions Modeler rejects count as agreed when we reject them too. The full
//! list of differences goes to `clem-oracle-report.txt` under the cargo target tmp dir; the test
//! fails when fewer expressions agree than `AGREED_AT_LEAST`, so progress is kept.
#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_clem::{Bindings, Compiled, Datum, compile, compile_condition, parse};
use dms_types::{DataModel, Field, MissingSpec, Storage, Value, calendar};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Expressions that agree with Modeler (values, or both reject); raise it as CLEM grows.
const AGREED_AT_LEAST: usize = 758;

fn probe_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/probes/clem_semantics")
}

/// Modeler's CSV export: strings are quoted but inner quotes are not escaped, so a quote
/// closes a cell only before a comma, a line end or the end of the file. An unquoted empty
/// cell is `None` (null); a quoted one is the empty string.
fn read_modeler_csv(text: &str) -> Vec<Vec<Option<String>>> {
    let chars: Vec<char> = text.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let ends_cell = |c: Option<char>| matches!(c, None | Some(',' | '\r' | '\n'));
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if at(i) == Some('"') {
            let mut cell = String::new();
            i += 1;
            while let Some(c) = at(i) {
                i += 1;
                if c == '"' && ends_cell(at(i)) {
                    break;
                }
                cell.push(c);
            }
            record.push(Some(cell));
        } else {
            let start = i;
            while !ends_cell(at(i)) {
                i += 1;
            }
            let cell: String = chars[start..i].iter().collect();
            record.push((!cell.is_empty()).then_some(cell));
        }
        match at(i) {
            Some(',') => i += 1,
            Some('\r') if at(i + 1) == Some('\n') => {
                i += 2;
                records.push(std::mem::take(&mut record));
            }
            Some('\r' | '\n') => {
                i += 1;
                records.push(std::mem::take(&mut record));
            }
            _ => {}
        }
    }
    if !record.is_empty() {
        records.push(record);
    }
    records
}

/// The probe's input as Modeler typed it: empty numbers and dates are null, empty strings
/// stay empty strings.
fn input() -> (DataModel, Vec<Vec<Value>>) {
    let storages = [
        ("id", Storage::Integer),
        ("a", Storage::Integer),
        ("b", Storage::Integer),
        ("x", Storage::Real),
        ("y", Storage::Real),
        ("s", Storage::String),
        ("m", Storage::Integer),
        ("w", Storage::String),
        ("d1", Storage::Date),
        ("d2", Storage::Date),
    ];
    let model = DataModel::new(
        storages
            .iter()
            .map(|(name, storage)| Field::new(*name, *storage))
            .collect(),
    )
    .unwrap();
    let text = std::fs::read_to_string(probe_dir().join("input.csv")).unwrap_or_else(|e| {
        panic!("no clem_semantics oracle ({e}); run `cargo xtask probe clem_semantics`")
    });
    let rows = text
        .lines()
        .skip(1)
        .map(|line| {
            line.split(',')
                .zip(storages)
                .map(|(cell, (_, storage))| match storage {
                    Storage::String => Value::string(cell),
                    _ if cell.is_empty() => Value::Null,
                    Storage::Integer => Value::Integer(cell.parse().unwrap()),
                    Storage::Real => Value::Real(cell.parse().unwrap()),
                    _ => {
                        let mut parts = cell.split('-').map(|p| p.parse::<i64>().unwrap());
                        let (y, m, d) = (parts.next(), parts.next(), parts.next());
                        Value::Date(
                            calendar::days_from_civil(
                                y.unwrap(),
                                u32::try_from(m.unwrap()).unwrap(),
                                u32::try_from(d.unwrap()).unwrap(),
                            )
                            .unwrap(),
                        )
                    }
                })
                .collect()
        })
        .collect();
    (model, rows)
}

/// A value as Modeler exports it with `decimal_places` 15.
fn exported(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Real(r) => Some(format!("{r:.15}")),
        other => Some(other.to_string()),
    }
}

#[derive(Default)]
struct Tally {
    agreed: usize,
    total: usize,
}

#[test]
fn clem_matches_modeler_on_probe_expressions() {
    let dir = probe_dir();
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    let (model, rows) = input();
    // The `blanks` cases run behind a Type node that defines blanks: -4 and null on `m`,
    // "z" and white space on `w`.
    let mut fields = model.fields().to_vec();
    for field in &mut fields {
        match field.name.as_str() {
            "m" => {
                field.missing = MissingSpec {
                    values: vec![Value::Integer(-4)],
                    null_is_blank: true,
                    ..MissingSpec::default()
                };
            }
            "w" => {
                field.missing = MissingSpec {
                    values: vec![Value::string("z")],
                    whitespace_is_blank: true,
                    ..MissingSpec::default()
                };
            }
            _ => {}
        }
    }
    let blanks_model = DataModel::new(fields).unwrap();
    let mut exports: HashMap<String, Vec<Vec<Option<String>>>> = HashMap::new();
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut report = String::new();
    let text = |entry: &serde_json::Value, key: &str| -> String {
        entry[key]
            .as_str()
            .unwrap_or_else(|| panic!("{key} in {entry}"))
            .to_string()
    };
    let cases = manifest["expressions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| (entry, &model))
        .chain(
            manifest["blanks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| (entry, &blanks_model)),
        );
    for (entry, model) in cases {
        let bindings = Bindings {
            model,
            current_field: None,
        };
        let (source, field) = (text(entry, "expr"), text(entry, "field"));
        let group = entry["group"].as_str().unwrap_or("blanks").to_string();
        let flag = text(entry, "kind") == "L";
        let modeler_error = entry["error"].as_str();
        // Ours: one exported cell per record, or the first error.
        let compiled: Result<Compiled, String> = parse(&source)
            .and_then(|expr| {
                if flag {
                    compile_condition(&expr, &bindings)
                } else {
                    compile(&expr, &bindings)
                }
            })
            .map_err(|e| e.to_string());
        let ours: Result<Vec<Option<String>>, String> = compiled.and_then(|compiled| {
            compiled
                .eval_records(&rows)
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|datum| {
                    if flag {
                        let truth = datum.truth().map_err(|e| e.to_string())?;
                        // Null conditions store the false value (oracle rule 33); the flag
                        // values take the storage Modeler inferred for them.
                        let key = if truth == Some(true) { "true" } else { "false" };
                        let value = text(entry, key);
                        Ok(Some(match entry["storage"].as_str() {
                            Some("Real") => format!("{:.15}", value.parse::<f64>().unwrap()),
                            _ => value,
                        }))
                    } else {
                        match datum {
                            Datum::Value(value) => Ok(exported(&value)),
                            Datum::Bool(_) => Err("a condition is no field value".into()),
                        }
                    }
                })
                .collect()
        });
        let agreed = match (modeler_error, &ours) {
            (Some(_), Err(_)) => true,
            (Some(error), Ok(_)) => {
                writeln!(
                    report,
                    "{field} {source:?}: Modeler rejects ({error}), we accept"
                )
                .unwrap();
                false
            }
            (None, ours) => {
                let file = text(entry, "file");
                let export = exports.entry(file.clone()).or_insert_with(|| {
                    read_modeler_csv(&std::fs::read_to_string(dir.join(&file)).unwrap())
                });
                let column = export[0]
                    .iter()
                    .position(|name| name.as_deref() == Some(field.as_str()))
                    .unwrap_or_else(|| panic!("{field} not in {file}"));
                let theirs: Vec<Option<String>> = export[1..]
                    .iter()
                    .map(|record| record.get(column).cloned().flatten())
                    .collect();
                match ours {
                    Err(error) => {
                        writeln!(
                            report,
                            "{field} {source:?}: we reject ({error}); Modeler {theirs:?}"
                        )
                        .unwrap();
                        false
                    }
                    Ok(ours) if *ours == theirs => true,
                    Ok(ours) => {
                        writeln!(
                            report,
                            "{field} {source:?}:\n  ours    {ours:?}\n  Modeler {theirs:?}"
                        )
                        .unwrap();
                        false
                    }
                }
            }
        };
        let tally = tallies.entry(group).or_default();
        tally.total += 1;
        tally.agreed += usize::from(agreed);
    }
    let agreed: usize = tallies.values().map(|t| t.agreed).sum();
    let total: usize = tallies.values().map(|t| t.total).sum();
    let mut summary = format!("CLEM oracle: {agreed}/{total} expressions agree\n");
    for (group, tally) in &tallies {
        writeln!(summary, "  {group}: {}/{}", tally.agreed, tally.total).unwrap();
    }
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("clem-oracle-report.txt");
    std::fs::write(&path, format!("{summary}\n{report}")).unwrap();
    println!("{summary}report: {}", path.display());
    assert!(
        agreed >= AGREED_AT_LEAST,
        "{summary}fewer than {AGREED_AT_LEAST} agree; see {}",
        path.display()
    );
}
