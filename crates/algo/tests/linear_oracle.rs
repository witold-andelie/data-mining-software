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
use dms_algo::linear::{self, LinearSettings, Selection, Style};
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

#[test]
fn preparation_and_selection_match_modeler() {
    for case in [
        "lin_fwd_drug",
        "lin_fwd_goods",
        "lin_adp_none_drug",
        "lin_adp_none_goods",
        "lin_default_drug",
        "lin_default_goods",
    ] {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../oracle/linear")
            .join(case);
        let xml = std::fs::read_dir(dir.join("models"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|x| x == "xml"))
            .unwrap();
        let oracle = Pmml::parse(&std::fs::read_to_string(xml).unwrap()).unwrap();
        let (target, inputs): (&str, &[&str]) = if case.ends_with("drug") {
            ("Na", &["Age", "Sex", "BP", "Cholesterol", "K"])
        } else {
            ("increase", &["Class", "Cost", "Promotion", "Before"])
        };
        check_at(
            case,
            &dir,
            oracle,
            target,
            inputs,
            "_applylinear_",
            &format!("$L-{target}"),
            LinearSettings {
                selection: if case.contains("adp_none") {
                    Selection::None
                } else {
                    Selection::Aicc
                },
                preparation: (!case.contains("fwd")).then(dms_algo::adp::AdpSettings::default),
                ..LinearSettings::default()
            },
        );
    }
}

fn check(case: &str, style: Style, apply: &str, score: &str) {
    let oracle = oracle_model(case);
    check_at(
        case,
        &case_dir(case),
        oracle,
        "Na",
        &["Age", "K"],
        apply,
        score,
        LinearSettings {
            style,
            ..LinearSettings::default()
        },
    );
}

#[allow(clippy::too_many_arguments, reason = "oracle case specification")]
fn check_at(
    case: &str,
    dir: &std::path::Path,
    oracle: Pmml,
    target: &str,
    inputs: &[&str],
    apply: &str,
    score: &str,
    settings: LinearSettings,
) {
    let table = training_table(
        &oracle,
        &std::fs::read_to_string(data_file(dir, "_type_")).unwrap(),
    );
    let position = |name: &str| table.model().position(name).unwrap();
    let positions: Vec<usize> = inputs.iter().map(|n| position(n)).collect();
    let dataset = Dataset {
        table: &table,
        target: position(target),
        inputs: &positions,
        frequency: None,
    };
    let ours = linear::train(&dataset, &settings).unwrap();
    if let (Model::GeneralRegression(a), Model::GeneralRegression(b)) = (&ours.model, &oracle.model)
    {
        assert_eq!(a.factors, b.factors, "{case}: selected factors");
        assert_eq!(a.covariates, b.covariates, "{case}: selected covariates");
        assert_eq!(
            a.pp_matrix, b.pp_matrix,
            "{case}: selected effects/categories"
        );
    }
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
    let csv = std::fs::read_to_string(data_file(dir, apply)).unwrap();
    let rows = parse_modeler_csv(&csv);
    let header: Vec<&str> = rows[0].iter().map(|c| c.text.as_str()).collect();
    let column = |name: &str| header.iter().position(|h| *h == name).unwrap();
    let scorer = ours.bind(inputs).unwrap();
    let (mut matched, mut total) = (0, 0);
    for row in rows[1..].iter().filter(|r| r.len() == header.len()) {
        let record: Vec<dms_pmml::Value> = inputs
            .iter()
            .map(|n| {
                let cell = &row[column(n)];
                if is_missing(cell) {
                    dms_pmml::Value::Missing
                } else if !cell.quoted
                    && let Ok(x) = cell.text.parse()
                {
                    dms_pmml::Value::Number(x)
                } else {
                    dms_pmml::Value::Text(cell.text.clone())
                }
            })
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
