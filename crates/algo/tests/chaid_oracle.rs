//! CHAID against Modeler: train on the records each `models/chaid_*` oracle case fed its build
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
use dms_algo::cart::MinSize;
use dms_algo::chaid::{self, ChaidSettings, Method};
use dms_pmml::{Node, Pmml, Predicate};
use support::*;

/// Cases that must match exactly; the others are reported.
const DONE: &[&str] = &[
    "chaid_default",
    "chaid_depth2",
    "chaid_exhaustive",
    "chaid_alpha",
    "chaid_regression",
];

struct Case {
    name: &'static str,
    target: &'static str,
    inputs: &'static [&'static str],
    settings: ChaidSettings,
}

const DRUG: &[&str] = &["Age", "Sex", "BP", "Cholesterol", "Na", "K"];
const CREDIT: &[&str] = &["Age", "Income", "Credit_cards", "Education", "Car_loans"];
const CREDIT_FOR_AGE: &[&str] = &[
    "Credit_rating",
    "Income",
    "Credit_cards",
    "Education",
    "Car_loans",
];

/// The cases of `xtask/oracle/probes/model_streams.py`.
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "chaid_default",
            target: "Drug",
            inputs: DRUG,
            settings: ChaidSettings::default(),
        },
        Case {
            name: "chaid_depth2",
            target: "Drug",
            inputs: DRUG,
            settings: ChaidSettings {
                max_depth: 2,
                ..ChaidSettings::default()
            },
        },
        Case {
            name: "chaid_exhaustive",
            target: "Credit_rating",
            inputs: CREDIT,
            settings: ChaidSettings {
                method: Method::Exhaustive,
                ..ChaidSettings::default()
            },
        },
        Case {
            name: "chaid_alpha",
            target: "Credit_rating",
            inputs: CREDIT,
            settings: ChaidSettings {
                split_alpha: 0.01,
                merge_alpha: 0.10,
                bonferroni: false,
                ..ChaidSettings::default()
            },
        },
        Case {
            name: "chaid_regression",
            target: "Age",
            inputs: CREDIT_FOR_AGE,
            settings: ChaidSettings {
                max_depth: 3,
                min_parent: MinSize::Percent(2.0),
                ..ChaidSettings::default()
            },
        },
    ]
}

#[test]
fn chaid_trains_the_trees_modeler_trains() {
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
        let ours = chaid::train(&dataset, &case.settings).unwrap();
        let difference = first_difference(tree(&ours), tree(&oracle));
        let (matched, rows) = score_both(
            &ours,
            case.target,
            &std::fs::read_to_string(data_file(&dir, "_applychaid_")).unwrap(),
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

/// `CHAID_DUMP=<case>`: print our tree for that case (debugging aid).
#[test]
fn dump_tree_when_asked() {
    let Ok(wanted) = std::env::var("CHAID_DUMP") else {
        return;
    };
    fn walk(node: &Node, depth: usize) {
        let piece = match &node.predicate {
            Predicate::Compound { predicates, .. } => predicates.first().map(show),
            other => Some(show(other)),
        };
        let counts: Vec<String> = node
            .distribution
            .iter()
            .map(|d| format!("{}:{}", d.value, d.record_count))
            .collect();
        println!(
            "{}id={} n={} score={} {} [{}]",
            "  ".repeat(depth),
            node.id.as_deref().unwrap_or("?"),
            node.record_count.unwrap_or(0.0),
            node.score.as_deref().unwrap_or(""),
            piece.unwrap_or_default(),
            counts.join(",")
        );
        for child in &node.children {
            walk(child, depth + 1);
        }
    }
    for case in cases().into_iter().filter(|c| c.name == wanted) {
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
        walk(tree(&chaid::train(&dataset, &case.settings).unwrap()), 0);
    }
}
