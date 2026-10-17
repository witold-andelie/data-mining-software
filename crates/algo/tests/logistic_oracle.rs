//! Logistic regression against Modeler: train on the records each `models/logreg_*` oracle case
//! fed its build node, compare every coefficient with the PMML Modeler exported, and score the
//! nugget's records with both models.

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

mod support;

use dms_algo::Dataset;
use dms_algo::logistic::{self, LogisticSettings, Procedure};
use dms_pmml::{Model, Pmml, Value as PmmlValue};
use support::{
    case_dir, close, data_file, is_missing, oracle_model, parse_modeler_csv, training_table,
};

struct Case {
    name: &'static str,
    target: &'static str,
    inputs: &'static [&'static str],
    procedure: Procedure,
}

const CASES: &[Case] = &[
    Case {
        name: "logreg_multinomial",
        target: "Drug",
        inputs: &["Age", "Sex", "BP", "Cholesterol", "Na", "K"],
        procedure: Procedure::Multinomial,
    },
    Case {
        name: "logreg_binomial",
        target: "heart disease",
        inputs: &["age", "resting blood pressure", "max heart rate"],
        procedure: Procedure::Binomial,
    },
];

fn general(pmml: &Pmml) -> &dms_pmml::GeneralRegressionModel {
    match &pmml.model {
        Model::GeneralRegression(model) => model,
        other => panic!("not a general regression model: {other:?}"),
    }
}

fn rel_diff(a: f64, b: f64) -> f64 {
    if a == b {
        0.0
    } else {
        (a - b).abs() / a.abs().max(b.abs()).max(1e-15)
    }
}

fn coefficient_rel_diff(ours: &Pmml, oracle: &Pmml) -> Result<f64, String> {
    let (a, b) = (general(ours), general(oracle));
    if a.parameters != b.parameters {
        return Err(format!(
            "parameters {:?} vs {:?}",
            a.parameters, b.parameters
        ));
    }
    if a.factors != b.factors {
        return Err(format!("factors {:?} vs {:?}", a.factors, b.factors));
    }
    if a.covariates != b.covariates {
        return Err(format!(
            "covariates {:?} vs {:?}",
            a.covariates, b.covariates
        ));
    }
    if a.pp_matrix.len() != b.pp_matrix.len()
        || a.pp_matrix.iter().zip(&b.pp_matrix).any(|(x, y)| {
            x.predictor != y.predictor || x.parameter != y.parameter || x.value != y.value
        })
    {
        return Err(format!("PPMatrix {:?} vs {:?}", a.pp_matrix, b.pp_matrix));
    }
    if a.param_matrix.len() != b.param_matrix.len() {
        return Err(format!(
            "ParamMatrix length {} vs {}",
            a.param_matrix.len(),
            b.param_matrix.len()
        ));
    }
    let mut worst: f64 = 0.0;
    for (x, y) in a.param_matrix.iter().zip(&b.param_matrix) {
        if x.parameter != y.parameter || x.target_category != y.target_category {
            return Err(format!(
                "PCell {}/{:?} vs {}/{:?}",
                x.parameter, x.target_category, y.parameter, y.target_category
            ));
        }
        worst = worst.max(rel_diff(x.beta, y.beta));
    }
    Ok(worst)
}

/// Scores the nugget's records with our model and the oracle model (`$L-`, `$LP-`).
fn score_both(
    ours: &Pmml,
    oracle: &Pmml,
    target: &str,
    csv: &str,
    inputs: &[&str],
) -> (usize, usize) {
    let rows = parse_modeler_csv(csv);
    let header: Vec<&str> = rows[0].iter().map(|c| c.text.as_str()).collect();
    let column = |name: &str| header.iter().position(|h| *h == name);
    let positions: Vec<usize> = inputs.iter().map(|name| column(name).unwrap()).collect();
    let predicted = column(&format!("$L-{target}")).unwrap();
    let ours = ours.bind(inputs).unwrap();
    let theirs = oracle.bind(inputs).unwrap();
    let mut matched = 0;
    let mut total = 0;
    for row in rows[1..].iter().filter(|r| r.len() == header.len()) {
        let record: Vec<PmmlValue> = positions
            .iter()
            .map(|&i| {
                let cell = &row[i];
                if is_missing(cell) {
                    PmmlValue::Missing
                } else if !cell.quoted
                    && let Ok(x) = cell.text.trim().parse::<f64>()
                {
                    PmmlValue::Number(x)
                } else {
                    PmmlValue::Text(cell.text.clone())
                }
            })
            .collect();
        let ours_pred = ours.score(&record).unwrap();
        let oracle_pred = theirs.score(&record).unwrap();
        total += 1;
        let expected = &row[predicted].text;
        let same_csv = match &ours_pred.value {
            PmmlValue::Number(x) => expected.parse::<f64>().is_ok_and(|y| close(*x, y)),
            PmmlValue::Text(t) => {
                t == expected
                    || matches!((t.parse::<f64>(), expected.parse::<f64>()), (Ok(a), Ok(b)) if a == b)
            }
            PmmlValue::Missing => is_missing(&row[predicted]),
        };
        let same_oracle = match (&ours_pred.value, &oracle_pred.value) {
            (PmmlValue::Text(a), PmmlValue::Text(b)) => {
                a == b || matches!((a.parse::<f64>(), b.parse::<f64>()), (Ok(x), Ok(y)) if x == y)
            }
            (PmmlValue::Number(a), PmmlValue::Number(b)) => close(*a, *b),
            (left, right) => left == right,
        };
        if same_csv && same_oracle {
            matched += 1;
        }
    }
    (matched, total)
}

#[test]
fn logistic_matches_modeler() {
    let mut failures = Vec::new();
    for case in CASES {
        let oracle = oracle_model(case.name);
        let table = training_table(
            &oracle,
            &std::fs::read_to_string(data_file(&case_dir(case.name), "_type_")).unwrap(),
        );
        let target = table.model().position(case.target).unwrap();
        let inputs: Vec<usize> = case
            .inputs
            .iter()
            .map(|n| table.model().position(n).unwrap())
            .collect();
        let settings = LogisticSettings {
            procedure: case.procedure,
            ..LogisticSettings::default()
        };
        let ours = logistic::train(
            &Dataset {
                table: &table,
                target,
                inputs: &inputs,
                frequency: None,
            },
            &settings,
        )
        .unwrap();

        let coeff = coefficient_rel_diff(&ours, &oracle);
        let apply =
            std::fs::read_to_string(data_file(&case_dir(case.name), "_applylogreg_")).unwrap();
        let (matched, total) = score_both(&ours, &oracle, case.target, &apply, case.inputs);
        match &coeff {
            Ok(worst) => println!(
                "{}: coefficients max rel diff {worst:.6e}, scores {matched}/{total}",
                case.name
            ),
            Err(message) => println!(
                "{}: coefficients {message}, scores {matched}/{total}",
                case.name
            ),
        }
        let coeffs_ok = match (&coeff, case.procedure) {
            (Ok(worst), Procedure::Binomial) => *worst <= 1e-6,
            // Separated data: the 19th iterate; weakly identified parameters differ by
            // floating-point order (dms_algo::logistic).
            (Ok(worst), Procedure::Multinomial) => *worst <= 2e-3,
            (Err(_), _) => false,
        };
        if !coeffs_ok || matched != total {
            failures.push(case.name);
        }
    }
    assert!(failures.is_empty(), "cases that differ: {failures:?}");
}
