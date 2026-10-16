//! K-Means against Modeler: cluster the records each `models/kmeans_*` oracle case fed its build
//! node, compare sizes and centers with the PMML Modeler exported, and the nugget's recorded
//! cluster and distance (`$KM-`, `$KMD-`) with our model's.

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

mod support;

use dms_algo::kmeans::{self, KMeansSettings};
use dms_pmml::{Model, Pmml, Value as PmmlValue};
use dms_types::{Measurement, Table, ValueSpec};
use support::*;

struct Case {
    name: &'static str,
    inputs: &'static [&'static str],
    /// Inputs the Type node made flags (the oracle's dictionary lists them as categorical).
    flags: &'static [&'static str],
    clusters: usize,
}

const CASES: &[Case] = &[
    Case {
        name: "kmeans_3clusters",
        inputs: &["Age", "Na", "K"],
        flags: &[],
        clusters: 3,
    },
    Case {
        name: "kmeans_5clusters",
        inputs: &["Age", "Na", "K"],
        flags: &[],
        clusters: 5,
    },
    Case {
        name: "kmeans_categorical",
        inputs: &["Age", "Sex", "BP", "Cholesterol", "Na", "K"],
        flags: &["Sex", "Cholesterol"],
        clusters: 4,
    },
];

/// The training table, with the case's flags as Modeler's Type node has them: false value
/// first, true value second.
fn table_for(case: &Case, oracle: &Pmml) -> Table {
    let table = training_table(
        oracle,
        &std::fs::read_to_string(data_file(&case_dir(case.name), "_type_")).unwrap(),
    );
    let mut fields = table.model().fields().to_vec();
    for field in &mut fields {
        if case.flags.contains(&field.name.as_str())
            && let ValueSpec::Categories(values) = &field.values
            && let [false_value, true_value] = values.as_slice()
        {
            field.measurement = Measurement::Flag;
            field.values = ValueSpec::Flag {
                true_value: true_value.clone(),
                false_value: false_value.clone(),
            };
        }
    }
    Table::new(
        dms_types::DataModel::new(fields).unwrap(),
        table.rows().to_vec(),
    )
    .unwrap()
}

fn clusters(pmml: &Pmml) -> Vec<(f64, Vec<f64>)> {
    match &pmml.model {
        Model::Clustering(model) => model
            .clusters
            .iter()
            .map(|c| (c.size.unwrap_or(0.0), c.center.clone()))
            .collect(),
        other => panic!("not a clustering model: {other:?}"),
    }
}

#[test]
fn kmeans_finds_modelers_clusters() {
    let mut failures = Vec::new();
    for case in CASES {
        let oracle = oracle_model(case.name);
        let table = table_for(case, &oracle);
        let inputs: Vec<usize> = case
            .inputs
            .iter()
            .map(|n| table.model().position(n).unwrap())
            .collect();
        let settings = KMeansSettings {
            clusters: case.clusters,
            name: Some(case.name.into()),
            ..KMeansSettings::default()
        };
        let ours = kmeans::train(&table, &inputs, &settings).unwrap();

        // Sizes exactly; centers to the six digits Modeler prints.
        let (a, b) = (clusters(&ours), clusters(&oracle));
        let centers_equal = a.len() == b.len()
            && a.iter().zip(&b).all(|((sa, ca), (sb, cb))| {
                sa == sb
                    && ca.len() == cb.len()
                    && ca
                        .iter()
                        .zip(cb)
                        .all(|(x, y)| (x - y).abs() <= 5e-6 * x.abs().max(1.0))
            });

        // The nugget's cluster and distance for every record.
        let rows = parse_modeler_csv(
            &std::fs::read_to_string(data_file(&case_dir(case.name), "_applykmeans_")).unwrap(),
        );
        let header: Vec<&str> = rows[0].iter().map(|c| c.text.as_str()).collect();
        let column = |name: &str| header.iter().position(|h| *h == name).unwrap();
        let positions: Vec<usize> = case.inputs.iter().map(|n| column(n)).collect();
        let (cluster_column, distance_column) = (
            column(&format!("$KM-{}", case.name)),
            column(&format!("$KMD-{}", case.name)),
        );
        let scorer = ours.bind(case.inputs).unwrap();
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
            let prediction = scorer.score(&record).unwrap();
            total += 1;
            let label = match &prediction.value {
                PmmlValue::Text(name) => format!("cluster-{name}"),
                other => format!("{other:?}"),
            };
            let distance_ok = match (
                prediction.affinity,
                row[distance_column].text.parse::<f64>(),
            ) {
                (Some(ours), Ok(theirs)) => close(ours, theirs),
                _ => false,
            };
            if label == row[cluster_column].text && distance_ok {
                matched += 1;
            }
        }
        println!(
            "{}: clusters {}, scores {matched}/{total}",
            case.name,
            if centers_equal { "equal" } else { "differ" }
        );
        if !centers_equal || matched != total {
            failures.push(case.name);
        }
    }
    assert!(failures.is_empty(), "cases that differ: {failures:?}");
}
