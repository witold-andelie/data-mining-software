//! C&R Tree against Modeler: train on the records each `models/cart_*` oracle case fed its build
//! node, compare the tree with the PMML Modeler exported, and score the nugget's records with both.

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

mod support;

use dms_algo::Dataset;
use dms_algo::cart::{self, CartSettings, Impurity, MinSize};
use dms_pmml::Pmml;
use support::*;

/// Cases that must match exactly; the others are reported.
const DONE: &[&str] = &[
    "cart_default",
    "cart_depth3",
    "cart_unpruned",
    "cart_min_records",
    "cart_no_surrogates",
    "cart_twoing",
    "cart_surrogates",
];

struct Case {
    name: &'static str,
    target: &'static str,
    inputs: &'static [&'static str],
    settings: CartSettings,
}

const DRUG: &[&str] = &["Age", "Sex", "BP", "Cholesterol", "Na", "K"];
const DRUG_FOR_NA: &[&str] = &["Age", "Sex", "BP", "Cholesterol", "K"];
const CREDIT: &[&str] = &["Age", "Income", "Credit_cards", "Education", "Car_loans"];

/// The cases of `xtask/oracle/probes/model_streams.py`.
fn cases() -> Vec<Case> {
    let unpruned = CartSettings {
        prune: false,
        ..CartSettings::default()
    };
    vec![
        Case {
            name: "cart_default",
            target: "Drug",
            inputs: DRUG,
            settings: CartSettings::default(),
        },
        Case {
            name: "cart_depth3",
            target: "Drug",
            inputs: DRUG,
            settings: CartSettings {
                max_depth: 3,
                ..CartSettings::default()
            },
        },
        Case {
            name: "cart_unpruned",
            target: "Drug",
            inputs: DRUG,
            settings: unpruned.clone(),
        },
        Case {
            name: "cart_min_records",
            target: "Drug",
            inputs: DRUG,
            settings: CartSettings {
                min_parent: MinSize::Records(30.0),
                min_child: MinSize::Records(15.0),
                ..unpruned.clone()
            },
        },
        Case {
            name: "cart_regression",
            target: "Na",
            inputs: DRUG_FOR_NA,
            settings: CartSettings {
                max_depth: 4,
                ..unpruned.clone()
            },
        },
        Case {
            name: "cart_surrogates",
            target: "Credit_rating",
            inputs: CREDIT,
            settings: CartSettings {
                max_depth: 3,
                max_surrogates: 2,
                ..unpruned.clone()
            },
        },
        Case {
            name: "cart_no_surrogates",
            target: "Drug",
            inputs: DRUG,
            settings: CartSettings {
                max_surrogates: 0,
                ..unpruned.clone()
            },
        },
        Case {
            name: "cart_twoing",
            target: "Drug",
            inputs: DRUG,
            settings: CartSettings {
                impurity: Impurity::Twoing,
                ..unpruned
            },
        },
    ]
}

#[test]
fn cart_trains_the_trees_modeler_trains() {
    let mut failures = Vec::new();
    for case in cases() {
        let dir = case_dir(case.name);
        let xml = std::fs::read_dir(dir.join("models"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "xml"))
            .unwrap();
        let oracle = Pmml::parse(&std::fs::read_to_string(xml).unwrap()).unwrap();
        let table = training_table(
            &oracle,
            &std::fs::read_to_string(data_file(&dir, "_type_")).unwrap(),
        );
        let position = |name: &str| table.model().position(name).unwrap();
        let inputs: Vec<usize> = case.inputs.iter().map(|n| position(n)).collect();
        let dataset = Dataset {
            table: &table,
            target: position(case.target),
            inputs: &inputs,
            frequency: None,
        };
        let ours = cart::train(&dataset, &case.settings).unwrap();
        let difference = first_difference(tree(&ours), tree(&oracle));
        let (matched, rows) = score_both(
            &ours,
            case.target,
            &std::fs::read_to_string(data_file(&dir, "_applycart_")).unwrap(),
            case.inputs,
        );
        let verdict = difference
            .clone()
            .map_or_else(|| "equal".to_string(), |d| format!("first difference {d}"));
        println!("{}: tree {verdict}, scores {matched}/{rows}", case.name);
        if DONE.contains(&case.name) && (difference.is_some() || matched != rows) {
            failures.push(case.name);
        }
    }
    assert!(
        failures.is_empty(),
        "cases that no longer match: {failures:?}"
    );
}
