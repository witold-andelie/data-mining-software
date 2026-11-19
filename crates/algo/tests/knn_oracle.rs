//! `car_sales_knn`: the neighbour table of the model Modeler saved in the demo stream.
//!
//! The oracle exported the records the build node read; the saved model lists, for each stored
//! case, its three nearest other cases and their distances (`fixtures/knn_car_sales_neighbors.tsv`,
//! copied from the model's `neighborsDistances` table). The trainer must reproduce every id and
//! every distance.

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test helpers fail loudly"
)]

use dms_algo::knn::{KnnData, NumericColumn, Settings, train};
use std::path::PathBuf;

const SAVED: &str = include_str!("fixtures/knn_car_sales_neighbors.tsv");

fn oracle_csv() -> PathBuf {
    let dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../oracle/ibm-demos/car_sales_knn/data");
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("01_type")
        })
        .unwrap()
}

const PREDICTORS: &[&str] = &[
    "price", "engine_s", "horsepow", "wheelbas", "width", "length", "curb_wgt", "fuel_cap", "mpg",
];

#[test]
fn knn_oracle_neighbors_match_saved_model() {
    let csv = std::fs::read_to_string(oracle_csv()).expect("read car_sales csv");
    let data = parse_csv(&csv);
    let expected = parse_saved();

    let model = train(&data, &Settings::default()).expect("train knn");
    assert_eq!(
        model.n_training(),
        expected.len(),
        "complete-case count vs saved neighborsDistances rows"
    );
    assert_eq!(model.k(), 3);
    assert_eq!(model.case_ids().len(), expected.len());

    let got = model.training_neighbors().expect("training neighbours");
    assert_eq!(got.len(), expected.len());

    let mut max_rel = 0.0_f64;
    let mut mismatches = Vec::new();
    for (row_i, (exp, got_row)) in expected.iter().zip(got.iter()).enumerate() {
        let got_id = model.case_ids().get(row_i).copied();
        if got_id != Some(exp.case_id) {
            mismatches.push(format!(
                "row {row_i}: stored case id {:?} vs saved {}",
                got_id, exp.case_id
            ));
            continue;
        }
        if got_row.len() != exp.neighbors.len() {
            mismatches.push(format!(
                "case {}: {} neighbours vs saved {}",
                exp.case_id,
                got_row.len(),
                exp.neighbors.len()
            ));
            continue;
        }
        for (g, e) in got_row.iter().zip(exp.neighbors.iter()) {
            if g.case_id != e.0 {
                mismatches.push(format!(
                    "case {}: neighbour id {} vs saved {}",
                    exp.case_id, g.case_id, e.0
                ));
            }
            let denom = e.1.abs().max(1e-15);
            let rel = (g.distance - e.1).abs() / denom;
            if rel > max_rel {
                max_rel = rel;
            }
            if rel > 1e-9 {
                mismatches.push(format!(
                    "case {} neighbour {}: distance {:.16} vs saved {:.16} rel={rel:.3e}",
                    exp.case_id, e.0, g.distance, e.1
                ));
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "oracle mismatches ({}, max rel {max_rel:.3e}):\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    assert!(
        max_rel <= 1e-9,
        "max relative distance error {max_rel:.3e} exceeds 1e-9"
    );
    println!("knn oracle max relative distance error {max_rel:.3e} over 154 cases / 462 distances");
}

struct SavedRow {
    case_id: i64,
    neighbors: Vec<(i64, f64)>,
}

fn parse_saved() -> Vec<SavedRow> {
    let mut rows = Vec::new();
    for line in SAVED
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let parts: Vec<&str> = line.split('\t').collect();
        let ids: Vec<&str> = parts[1].split(',').collect();
        let distances: Vec<&str> = parts[2].split(',').collect();
        rows.push(SavedRow {
            case_id: parse_i64(parts[0]),
            neighbors: ids
                .iter()
                .zip(&distances)
                .map(|(id, d)| (parse_i64(id), parse_f64(d)))
                .collect(),
        });
    }
    assert!(!rows.is_empty());
    rows
}

fn parse_csv(text: &str) -> KnnData {
    let mut lines = text.lines();
    let header = lines.next().expect("csv header");
    let names = split_csv(header);
    let mut positions = Vec::new();
    for name in PREDICTORS {
        let pos = names
            .iter()
            .position(|h| h == name)
            .unwrap_or_else(|| panic!("csv missing column {name}"));
        positions.push(pos);
    }
    let mut columns: Vec<Vec<Option<f64>>> = vec![Vec::new(); PREDICTORS.len()];
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let fields = split_csv(line);
        for (j, &pos) in positions.iter().enumerate() {
            let raw = fields.get(pos).map(String::as_str).unwrap_or("");
            let value = if raw.is_empty() {
                None
            } else {
                Some(parse_f64(raw))
            };
            columns[j].push(value);
        }
    }
    let numeric = PREDICTORS
        .iter()
        .zip(columns)
        .map(|(name, values)| NumericColumn {
            name: (*name).to_string(),
            values,
        })
        .collect();
    KnnData {
        numeric,
        nominal: Vec::new(),
        target: None,
        case_ids: None,
        folds: None,
    }
}

fn split_csv(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn parse_f64(s: &str) -> f64 {
    s.trim()
        .parse::<f64>()
        .unwrap_or_else(|_| panic!("not a float: {s:?}"))
}

fn parse_i64(s: &str) -> i64 {
    s.trim()
        .parse::<i64>()
        .unwrap_or_else(|_| panic!("not an integer id: {s:?}"))
}
