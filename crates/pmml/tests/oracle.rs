//! Oracle corpus test: binds and scores the PMML models exported by Modeler (`oracle/team/**/models/*.xml`)
//! on the records reaching each model nugget (`oracle/team/**/data/*_apply*.csv`) and compares with
//! the Modeler score columns (`$R-`, `$RC-`, `$RRP-`, `$L-`, `$LP-`, `$LRP-`, `$KM-`).
//!
//! ## Mapping from `Prediction` to Modeler `$` Score Columns
//!
//! | Apply Node Type | Algorithm Name | Added Columns | Meaning in `Prediction` |
//! |---|---|---|---|
//! | `applycart` | `C&R Tree` | `$R-<target>`, `$RC-<target>` | `$R-`: `Prediction.value` (predicted category); `$RC-`: `Prediction.confidence` (Laplace-smoothed `(recordCount + 1) / (total + k)` of the predicted category) |
//! | `applychaid` | `CHAID` | `$R-<target>`, `$RC-<target>`, `$RRP-<target>` | `$R-`: `Prediction.value`; `$RC-`: `Prediction.confidence` (Laplace-smoothed `(recordCount + 1) / (total + k)` of predicted category); `$RRP-`: raw propensity = Laplace-smoothed probability of the positive/second target category (`Prediction.probabilities[1]`) |
//! | `applylogreg` | `Logistic` | `$L-<target>`, `$LP-<target>`, `$LRP-<target>` | `$L-`: `Prediction.value`; `$LP-`: `Prediction.confidence` (probability of predicted category); `$LRP-`: raw propensity = probability of positive/second target category (`Prediction.probabilities[1]`) |
//! | `applylinear` | `Linear Regression` | `$L-<target>` | `$L-`: `Prediction.value` (predicted continuous number) |
//! | `applykmeans` | `KMeans` | `$KM-<model>` | `$KM-`: `Prediction.value` formatted as `"cluster-<name>"` when cluster `@name` is numeric (e.g. `"1"` -> `"cluster-1"`) |

#![cfg(feature = "corpus")]

use dms_pmml::{Pmml, Prediction, Value};
use serde_json::Value as Json;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
struct Cell {
    text: String,
    quoted: bool,
}

fn oracle_team_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("oracle")
        .join("team")
}

fn algorithm_for_apply(apply_type: &str) -> Option<&'static str> {
    match apply_type {
        "applycart" => Some("C&R Tree"),
        "applychaid" => Some("CHAID"),
        "applylogreg" => Some("Logistic"),
        "applylinear" => Some("Linear Regression"),
        "applykmeans" => Some("KMeans"),
        _ => None,
    }
}

fn score_column_count(apply_type: &str) -> usize {
    match apply_type {
        "applycart" => 2,
        "applychaid" => 3,
        "applylogreg" => 3,
        "applylinear" => 1,
        "applykmeans" => 1,
        _ => 0,
    }
}

/// Parses Modeler's CSV export format (where quotes inside quoted cells are not escaped, so a
/// quote closes a cell only before `,`, CR, LF, or EOF).
fn parse_modeler_csv(text: &str) -> Vec<Vec<Cell>> {
    let bom = char::from_u32(0xfeff).unwrap_or(' ');
    let cr = char::from(13u8);
    let lf = char::from(10u8);
    let text = text.trim_start_matches(bom);
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            let next = chars.peek().copied();
            if c == '"'
                && next.is_some()
                && next != Some(',')
                && next != Some(cr)
                && next != Some(lf)
            {
                cell.push('"');
            } else if c == '"' {
                in_quotes = false;
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
            ',' => row.push(Cell {
                text: std::mem::take(&mut cell),
                quoted: std::mem::take(&mut quoted),
            }),
            c if c == cr => {}
            c if c == lf => {
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

fn cell_to_value(cell: &Cell) -> Value {
    if !cell.quoted && cell.text.is_empty() {
        return Value::Missing;
    }
    if cell.text == "$null$" {
        return Value::Missing;
    }
    if !cell.quoted
        && let Ok(n) = cell.text.trim().parse::<f64>()
        && n.is_finite()
    {
        return Value::Number(n);
    }
    Value::Text(cell.text.clone())
}

fn numbers_match(expected_str: &str, actual: f64) -> bool {
    let Ok(expected) = expected_str.trim().parse::<f64>() else {
        return false;
    };
    let tol = 1e-9 * expected.abs().max(actual.abs()).max(1.0);
    (expected - actual).abs() <= tol
}

fn category_matches(expected: &Cell, actual: &Value, is_kmeans: bool) -> bool {
    let Value::Text(s) = actual else {
        return false;
    };
    if is_kmeans {
        let formatted = if s.starts_with("cluster-") {
            s.clone()
        } else {
            format!("cluster-{s}")
        };
        return expected.text == formatted || expected.text == *s;
    }
    if expected.quoted {
        return expected.text == *s;
    }
    if expected.text == *s {
        return true;
    }
    if let (Ok(a), Ok(b)) = (expected.text.trim().parse::<f64>(), s.trim().parse::<f64>()) {
        return a == b;
    }
    false
}

fn row_matches_prediction(apply_type: &str, pred: &Prediction, score_cells: &[Cell]) -> bool {
    match apply_type {
        "applycart" => {
            // Columns: $R-<target>, $RC-<target>
            let [r_cell, rc_cell] = score_cells else {
                return false;
            };
            let cat_ok = category_matches(r_cell, &pred.value, false);
            let conf_ok = pred
                .confidence
                .is_some_and(|c| numbers_match(&rc_cell.text, c));
            cat_ok && conf_ok
        }
        "applychaid" | "applylogreg" => {
            // Columns: $R-/$L-<target>, $RC-/$LP-<target>, $RRP-/$LRP-<target>
            let [r_cell, rc_cell, rrp_cell] = score_cells else {
                return false;
            };
            let cat_ok = category_matches(r_cell, &pred.value, false);
            let conf_ok = pred
                .confidence
                .is_some_and(|c| numbers_match(&rc_cell.text, c));
            let raw_prop = pred
                .probabilities
                .get(1)
                .or_else(|| pred.probabilities.last())
                .map(|(_, p)| *p);
            let prop_ok = raw_prop.is_some_and(|p| numbers_match(&rrp_cell.text, p));
            cat_ok && conf_ok && prop_ok
        }
        "applylinear" => {
            // Column: $L-<target>
            let [l_cell] = score_cells else {
                return false;
            };
            let Value::Number(y) = pred.value else {
                return false;
            };
            numbers_match(&l_cell.text, y)
        }
        "applykmeans" => {
            // Column: $KM-<model name>
            let [km_cell] = score_cells else {
                return false;
            };
            category_matches(km_cell, &pred.value, true)
        }
        _ => false,
    }
}

#[test]
fn score_team_oracle_exports() {
    let mut fully_matched = 0usize;
    let team_dir = oracle_team_dir();
    let mut stream_dirs: Vec<PathBuf> = fs::read_dir(&team_dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    stream_dirs.sort();

    let mut total_exports = 0usize;

    for stream_dir in stream_dirs {
        let stream_name = stream_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
        let manifest_path = stream_dir.join("manifest.json");
        if !manifest_path.is_file() {
            continue;
        }
        let manifest: Json =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        let data_entries = manifest
            .get("data")
            .and_then(Json::as_array)
            .cloned()
            .unwrap_or_default();
        let model_entries = manifest
            .get("models")
            .and_then(Json::as_array)
            .cloned()
            .unwrap_or_default();

        for data_entry in &data_entries {
            let apply_type = data_entry.get("type").and_then(Json::as_str).unwrap_or("");
            let Some(expected_algo) = algorithm_for_apply(apply_type) else {
                continue;
            };
            total_exports += 1;

            let label = data_entry.get("label").and_then(Json::as_str).unwrap_or("");
            let data_file_str = data_entry.get("file").and_then(Json::as_str).unwrap_or("");
            let data_file_name = Path::new(data_file_str)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?");
            let csv_path = stream_dir.join("data").join(data_file_name);
            let csv_text = fs::read_to_string(&csv_path).unwrap();
            let csv_rows = parse_modeler_csv(&csv_text);
            let header_cells = csv_rows.first().expect("CSV header row");
            let num_score_cols = score_column_count(apply_type);
            let num_input_cols = header_cells.len().saturating_sub(num_score_cols);
            let input_names: Vec<&str> = header_cells[..num_input_cols]
                .iter()
                .map(|c| c.text.as_str())
                .collect();
            let data_rows = &csv_rows[1..];

            let candidates: Vec<&Json> = model_entries
                .iter()
                .filter(|m| {
                    m.get("label").and_then(Json::as_str) == Some(label)
                        && m.get("algorithm").and_then(Json::as_str) == Some(expected_algo)
                })
                .collect();
            assert!(
                !candidates.is_empty(),
                "no candidate model for {stream_name}/{data_file_name} ({label:?}, {expected_algo:?})"
            );

            let mut best_model_name = String::new();
            let mut best_matched = 0usize;
            let mut had_parseable_candidate = false;

            for candidate in &candidates {
                let model_file_str = candidate.get("file").and_then(Json::as_str).unwrap_or("");
                let model_file_name = Path::new(model_file_str)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?");
                let model_path = stream_dir.join("models").join(model_file_name);
                let xml = fs::read_to_string(&model_path).unwrap_or_default();

                let Ok(pmml) = Pmml::parse(&xml) else {
                    if best_model_name.is_empty() {
                        best_model_name = model_file_name.to_string();
                    }
                    continue;
                };
                had_parseable_candidate = true;
                let scorer = pmml.bind(&input_names).unwrap_or_else(|e| {
                    panic!("bind failed for {stream_name}/{model_file_name}: {e}")
                });

                let mut matched = 0usize;
                for row in data_rows {
                    let record: Vec<Value> =
                        row[..num_input_cols].iter().map(cell_to_value).collect();
                    let score_cells = &row[num_input_cols..];
                    let pred = scorer.score(&record).unwrap_or_else(|e| {
                        panic!("score failed for {stream_name}/{model_file_name}: {e}")
                    });
                    if row_matches_prediction(apply_type, &pred, score_cells) {
                        matched += 1;
                    }
                }

                if matched > best_matched || best_model_name.is_empty() {
                    best_matched = matched;
                    best_model_name = model_file_name.to_string();
                }
            }

            assert!(
                had_parseable_candidate,
                "no parseable PMML candidate for {stream_name}/{data_file_name}"
            );
            println!(
                "{stream_name}/{data_file_name}: {best_model_name} {best_matched}/{} rows",
                data_rows.len()
            );
            if best_matched == data_rows.len() {
                fully_matched += 1;
            }
        }
    }

    assert_eq!(total_exports, 10);
    // The other exports were scored by nuggets whose saved model differs from the one the
    // oracle run retrained (random upstream nodes, unlinked or split models); see docs/models.md.
    assert!(
        fully_matched >= FULLY_MATCHED_AT_LEAST,
        "{fully_matched} exports fully matched, expected at least {FULLY_MATCHED_AT_LEAST}"
    );
}

/// Exports whose every row matches; raise it as scoring improves, never lower it.
const FULLY_MATCHED_AT_LEAST: usize = 6;
