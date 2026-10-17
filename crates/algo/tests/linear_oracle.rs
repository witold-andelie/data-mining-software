//! Linear and Regression against Modeler: train on the records `models/linear_enter` and
//! `regression_enter` fed their build nodes and compare every coefficient with the PMML Modeler
//! exported, and the nuggets' `$L-` / `$E-` scores.

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

mod support;

use dms_algo::Dataset;
use dms_algo::linear::{self, LinearSettings, Style};
use dms_pmml::{Model, Pmml};
use support::*;

fn betas(pmml: &Pmml) -> Vec<(String, f64)> {
    match &pmml.model {
        Model::GeneralRegression(model) => model
            .param_matrix
            .iter()
            .map(|cell| (cell.parameter.clone(), cell.beta))
            .collect(),
        other => panic!("not a general regression model: {other:?}"),
    }
}

#[test]
fn linear_matches_modeler() {
    check("linear_enter", Style::Linear, "_applylinear_", "$L-Na");
    check(
        "regression_enter",
        Style::Regression,
        "_applyregression_",
        "$E-Na",
    );
}

fn check(case: &str, style: Style, apply: &str, score: &str) {
    let oracle = oracle_model(case);
    let table = training_table(
        &oracle,
        &std::fs::read_to_string(data_file(&case_dir(case), "_type_")).unwrap(),
    );
    let position = |name: &str| table.model().position(name).unwrap();
    let inputs = [position("Age"), position("K")];
    let dataset = Dataset {
        table: &table,
        target: position("Na"),
        inputs: &inputs,
        frequency: None,
    };
    let settings = LinearSettings {
        style,
        ..LinearSettings::default()
    };
    let ours = linear::train(&dataset, &settings).unwrap();
    let (a, b) = (betas(&ours), betas(&oracle));
    assert_eq!(a.len(), b.len());
    let worst = a
        .iter()
        .zip(&b)
        .map(|((pa, x), (pb, y))| {
            assert_eq!(pa, pb);
            (x - y).abs() / y.abs().max(1e-300)
        })
        .fold(0.0, f64::max);
    let csv = std::fs::read_to_string(data_file(&case_dir(case), apply)).unwrap();
    let rows = parse_modeler_csv(&csv);
    let header: Vec<&str> = rows[0].iter().map(|c| c.text.as_str()).collect();
    let column = |name: &str| header.iter().position(|h| *h == name).unwrap();
    let scorer = ours.bind(&["Age", "K"]).unwrap();
    let (mut matched, mut total) = (0, 0);
    for row in rows[1..].iter().filter(|r| r.len() == header.len()) {
        let record: Vec<dms_pmml::Value> = ["Age", "K"]
            .iter()
            .map(|n| dms_pmml::Value::Number(row[column(n)].text.parse().unwrap()))
            .collect();
        let ours = scorer.score(&record).unwrap();
        let theirs: f64 = row[column(score)].text.parse().unwrap();
        total += 1;
        if matches!(ours.value, dms_pmml::Value::Number(x) if close(x, theirs)) {
            matched += 1;
        }
    }
    println!("{case}: coefficients max rel diff {worst:e}, scores {matched}/{total}");
    assert!(worst < 1e-9, "coefficients differ by {worst:e}");
    assert_eq!(matched, total);
}
