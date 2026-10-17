//! Replays Modeler's exported C5.0 trial trees over the training rows and checks that
//! [`dms_algo::c50_boost::Reweighting`] reproduces every exported leaf count.
//!
//! Known GAP: the tree that reweights is not always the exported one. For
//! `models_ml/c50_boost_tree`, `c50_boost/c50_boost_reverse` and
//! `c50_boost/c50_boost_t10`, trial 3's leaf `cat = blue, x <= 7` (under
//! `x > 3` and `x > 5`) votes and counts as `B` instead of its exported score
//! `C`. With that override, tree 4 matches and later trees are not required.
//! `c50_boost_t4` tree 4 does not match. `c50_boost_xcf` matches through tree 5
//! on the exported scores. A leaf whose class counts are all zero is compared
//! on its `recordCount` only.
#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use dms_algo::c50_boost::Reweighting;
use dms_pmml::{BooleanOperator, Model, Node, Operator, Pmml, Predicate, TreeModel, Usage};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Modeler prints eight significant digits. A relative gap of `1e-6` covers that.
const TOLERANCE: f64 = 1e-6;

struct Case {
    label: &'static str,
    name: &'static str,
    /// Last 1-based tree whose leaf counts match.
    last_match: usize,
    /// Trial 3's `blue, x <= 7` leaf reweights as `B`.
    override_blue: bool,
    /// `(1-based tree, active cases)` measured before scoring that tree.
    active: &'static [(usize, usize)],
}

const CASES: &[Case] = &[
    Case {
        label: "models_ml",
        name: "c50_boost_tree",
        last_match: 4,
        override_blue: true,
        active: &[(4, 383), (5, 376)],
    },
    Case {
        label: "c50_boost",
        name: "c50_boost_reverse",
        last_match: 4,
        override_blue: true,
        active: &[(4, 383), (5, 376)],
    },
    Case {
        label: "c50_boost",
        name: "c50_boost_t3",
        last_match: 3,
        override_blue: false,
        active: &[(3, 382)],
    },
    Case {
        label: "c50_boost",
        name: "c50_boost_xcf",
        last_match: 5,
        override_blue: false,
        active: &[],
    },
    Case {
        label: "c50_boost",
        name: "c50_boost_t10",
        last_match: 4,
        override_blue: true,
        active: &[],
    },
    Case {
        label: "c50_boost",
        name: "c50_boost_t4",
        last_match: 3,
        override_blue: false,
        active: &[],
    },
];

struct Rows {
    columns: HashMap<String, usize>,
    target: usize,
    cells: Vec<Vec<String>>,
}

struct LeafFact {
    id: String,
    record_count: f64,
    distribution: Vec<(String, f64)>,
    zero_distribution: bool,
}

struct Gap {
    difference: f64,
    failures: usize,
    detail: String,
}

struct Replay {
    text: String,
    last_match: usize,
}

#[test]
fn exported_trial_leaf_counts_follow_the_reweighting() {
    for case in CASES {
        let report = replay(case);
        println!("{}", report.text);
        assert_eq!(
            report.last_match, case.last_match,
            "{}/{}:\n{}",
            case.label, case.name, report.text
        );
    }
}

fn replay(case: &Case) -> Replay {
    let dir = support::repo_root()
        .join("oracle")
        .join(case.label)
        .join(case.name);
    let rows = read_rows(&dir);
    let pmml = read_pmml(&dir);
    let trees = trial_trees(&pmml);
    let target_name = target_name(trees[0]);
    let classes = class_names(&pmml, &target_name);
    let target_column = *rows.columns.get(&target_name).unwrap_or_else(|| {
        panic!(
            "{}/{} training rows have no {target_name:?}",
            case.label, case.name
        )
    });
    assert_eq!(
        rows.target, target_column,
        "type export target column drifted"
    );

    let mut state = Reweighting::new(rows.cells.len(), classes.len())
        .unwrap_or_else(|error| panic!("{}/{} reweighting: {error}", case.label, case.name));
    let mut lines = Vec::new();
    let mut last_match = 0usize;
    let mut stop_at: Option<(usize, Gap)> = None;

    for (index, tree) in trees.iter().enumerate() {
        let trial = index + 1;
        let active = state.active().iter().filter(|flag| **flag).count();
        let weight_sum: f64 = state.weights().iter().sum();
        assert!(
            (weight_sum - active as f64).abs() <= 1e-6,
            "{}/{} tree {trial}: active weights sum to {weight_sum}, active {active}",
            case.label,
            case.name
        );
        if let Some((_, expected)) = case.active.iter().find(|(tree_n, _)| *tree_n == trial) {
            assert_eq!(
                active, *expected,
                "{}/{} tree {trial} active cases",
                case.label, case.name
            );
        }

        let blue = if case.override_blue && trial == 3 {
            let ids = blue_leaf_ids(tree);
            assert_eq!(
                ids.len(),
                1,
                "{}/{} tree 3 blue override leaves: {ids:?}",
                case.label,
                case.name
            );
            ids
        } else {
            Vec::new()
        };
        let (predicted, actual, gap) =
            compare_tree(tree, &rows, target_column, &classes, state.weights(), &blue);
        if gap.failures == 0 {
            last_match = trial;
            lines.push(format!(
                "{}/{} tree {trial}/{}: match (active {active}, worst {:.3e})",
                case.label,
                case.name,
                trees.len(),
                gap.difference
            ));
        } else {
            lines.push(format!(
                "{}/{} tree {trial}/{}: differ (active {active}, {} failing leaves, largest {:.6e} {})",
                case.label,
                case.name,
                trees.len(),
                gap.failures,
                gap.difference,
                gap.detail
            ));
            stop_at = Some((trial, gap));
            break;
        }
        if trial == trees.len() {
            break;
        }
        // Trials still to be built. `boostingTrials` 2 exports three trees, so this
        // is the number of exported trees, not the node setting.
        let trials_left = trees.len() - trial;
        state
            .apply_trial(&predicted, &actual, trials_left)
            .unwrap_or_else(|error| {
                panic!(
                    "{}/{} trial {trial} reweighting: {error}",
                    case.label, case.name
                )
            });
    }

    let summary = if let Some((trial, gap)) = stop_at {
        format!(
            "{}/{}: last matching tree {last_match} of {}; stopped at tree {trial}, largest leaf difference {:.6e} ({})",
            case.label,
            case.name,
            trees.len(),
            gap.difference,
            gap.detail
        )
    } else {
        format!(
            "{}/{}: last matching tree {last_match} of {} (all exported trees)",
            case.label,
            case.name,
            trees.len()
        )
    };
    lines.push(summary);
    Replay {
        text: lines.join("\n"),
        last_match,
    }
}

fn compare_tree(
    tree: &TreeModel,
    rows: &Rows,
    target_column: usize,
    classes: &[String],
    weights: &[f64],
    blue_ids: &[String],
) -> (Vec<usize>, Vec<usize>, Gap) {
    let leaves = leaf_facts(tree);
    let mut by_id: HashMap<String, usize> = HashMap::new();
    let mut totals = vec![0.0; leaves.len()];
    let mut class_weight = vec![vec![0.0; classes.len()]; leaves.len()];
    for (index, leaf) in leaves.iter().enumerate() {
        assert!(
            by_id.insert(leaf.id.clone(), index).is_none(),
            "duplicate leaf id {}",
            leaf.id
        );
    }

    let mut predicted = Vec::with_capacity(rows.cells.len());
    let mut actual = Vec::with_capacity(rows.cells.len());
    for (row_index, row) in rows.cells.iter().enumerate() {
        let leaf = route(&tree.root, row, &rows.columns, 0);
        let id = leaf
            .id
            .as_deref()
            .unwrap_or_else(|| panic!("routed node has no id"));
        let slot = *by_id
            .get(id)
            .unwrap_or_else(|| panic!("routed id {id} is not a leaf"));
        let truth_name = row.get(target_column).map(String::as_str).unwrap_or("");
        let truth = class_index(classes, truth_name, "actual");
        let weight = weights[row_index];
        totals[slot] += weight;
        class_weight[slot][truth] += weight;
        let score = leaf
            .score
            .as_deref()
            .unwrap_or_else(|| panic!("leaf {id} has no score"));
        let voted = if blue_ids.iter().any(|blue| blue == id) {
            "B"
        } else {
            score
        };
        predicted.push(class_index(classes, voted, "predicted"));
        actual.push(truth);
    }

    let mut gap = Gap {
        difference: 0.0,
        failures: 0,
        detail: String::new(),
    };
    for (index, leaf) in leaves.iter().enumerate() {
        let mut failed = note(
            &mut gap,
            &leaf.id,
            "recordCount",
            totals[index],
            leaf.record_count,
        );
        if !leaf.zero_distribution {
            for (class_name, expected) in &leaf.distribution {
                let class = class_index(classes, class_name, "distribution");
                failed |= note(
                    &mut gap,
                    &leaf.id,
                    class_name,
                    class_weight[index][class],
                    *expected,
                );
            }
        }
        if failed {
            gap.failures += 1;
        }
    }
    (predicted, actual, gap)
}

/// Records the largest relative gap. Returns whether this one comparison fails.
fn note(gap: &mut Gap, leaf: &str, what: &str, got: f64, expected: f64) -> bool {
    let difference = relative_difference(got, expected);
    let fails = !difference.is_finite() || difference > TOLERANCE;
    let worse = gap.detail.is_empty()
        || (!difference.is_finite() && gap.difference.is_finite())
        || difference > gap.difference;
    if worse {
        gap.difference = if difference.is_finite() {
            difference
        } else {
            f64::INFINITY
        };
        gap.detail = format!("leaf {leaf} {what} expected {expected} got {got}");
    }
    fails
}

fn relative_difference(got: f64, expected: f64) -> f64 {
    let diff = (got - expected).abs();
    if expected == 0.0 {
        diff
    } else {
        diff / expected.abs()
    }
}

fn leaf_facts(tree: &TreeModel) -> Vec<LeafFact> {
    let mut leaves = Vec::new();
    collect_leaves(&tree.root, &mut leaves);
    assert!(!leaves.is_empty(), "tree has no leaf");
    leaves
}

fn collect_leaves(node: &Node, leaves: &mut Vec<LeafFact>) {
    if node.children.is_empty() {
        let id = node.id.clone().unwrap_or_else(|| panic!("leaf has no id"));
        let record_count = node
            .record_count
            .unwrap_or_else(|| panic!("leaf {id} has no recordCount"));
        let distribution = node
            .distribution
            .iter()
            .map(|entry| (entry.value.clone(), entry.record_count))
            .collect::<Vec<_>>();
        let zero_distribution = distribution.iter().all(|(_, count)| *count == 0.0);
        leaves.push(LeafFact {
            id,
            record_count,
            distribution,
            zero_distribution,
        });
        return;
    }
    for child in &node.children {
        collect_leaves(child, leaves);
    }
}

/// Ids of the trial-3 leaf `cat = blue, x <= 7` under `x > 3, x > 5` scored `C`.
fn blue_leaf_ids(tree: &TreeModel) -> Vec<String> {
    let mut path = Vec::new();
    let mut ids = Vec::new();
    find_blue(&tree.root, &mut path, &mut ids);
    ids
}

fn find_blue(node: &Node, path: &mut Vec<Predicate>, ids: &mut Vec<String>) {
    if node.children.is_empty() {
        let scored_c = node.score.as_deref() == Some("C");
        if scored_c && is_blue_path(path) {
            ids.push(
                node.id
                    .clone()
                    .unwrap_or_else(|| panic!("blue leaf has no id")),
            );
        }
        return;
    }
    for child in &node.children {
        path.push(child.predicate.clone());
        find_blue(child, path, ids);
        path.pop();
    }
}

/// The path is exactly `x > 3`, `x > 5`, `cat = blue`, `x <= 7`, in any order.
fn is_blue_path(path: &[Predicate]) -> bool {
    if path.len() != 4 {
        return false;
    }
    let mut greater_3 = false;
    let mut greater_5 = false;
    let mut less_equal_7 = false;
    let mut cat_blue = false;
    for predicate in path {
        let Predicate::Simple {
            field,
            operator,
            value,
        } = predicate
        else {
            return false;
        };
        let value = value.as_deref().unwrap_or("");
        if field == "cat" && *operator == Operator::Equal && value == "blue" {
            cat_blue = true;
        } else if field == "x" && *operator == Operator::GreaterThan && number_is(value, 3.0) {
            greater_3 = true;
        } else if field == "x" && *operator == Operator::GreaterThan && number_is(value, 5.0) {
            greater_5 = true;
        } else if field == "x" && *operator == Operator::LessOrEqual && number_is(value, 7.0) {
            less_equal_7 = true;
        } else {
            return false;
        }
    }
    greater_3 && greater_5 && less_equal_7 && cat_blue
}

fn number_is(text: &str, expected: f64) -> bool {
    text.parse::<f64>()
        .ok()
        .is_some_and(|value| value.is_finite() && value == expected)
}

fn route<'a>(
    node: &'a Node,
    row: &[String],
    columns: &HashMap<String, usize>,
    depth: usize,
) -> &'a Node {
    assert!(depth <= 64, "tree deeper than 64 at node {:?}", node.id);
    if node.children.is_empty() {
        return node;
    }
    for child in &node.children {
        if holds(&child.predicate, row, columns, 0) {
            return route(child, row, columns, depth + 1);
        }
    }
    panic!("no child of node {:?} matched the record", node.id);
}

fn holds(
    predicate: &Predicate,
    row: &[String],
    columns: &HashMap<String, usize>,
    depth: usize,
) -> bool {
    assert!(depth <= 64, "predicate deeper than 64");
    match predicate {
        Predicate::True => true,
        Predicate::False => false,
        Predicate::Simple {
            field,
            operator,
            value,
        } => {
            let cell = cell_of(row, columns, field);
            if cell.is_empty() {
                return matches!(operator, Operator::IsMissing);
            }
            let rhs = value.as_deref().unwrap_or("");
            match operator {
                Operator::Equal => cells_equal(cell, rhs),
                Operator::NotEqual => !cells_equal(cell, rhs),
                Operator::LessThan => cell_number(cell, field) < cell_number(rhs, field),
                Operator::LessOrEqual => cell_number(cell, field) <= cell_number(rhs, field),
                Operator::GreaterThan => cell_number(cell, field) > cell_number(rhs, field),
                Operator::GreaterOrEqual => cell_number(cell, field) >= cell_number(rhs, field),
                Operator::IsMissing => false,
                Operator::IsNotMissing => true,
            }
        }
        Predicate::SimpleSet {
            field,
            is_in,
            values,
        } => {
            let cell = cell_of(row, columns, field);
            if cell.is_empty() {
                return false;
            }
            let found = values.iter().any(|value| cells_equal(cell, value));
            if *is_in { found } else { !found }
        }
        Predicate::Compound {
            operator,
            predicates,
        } => match operator {
            BooleanOperator::And => predicates
                .iter()
                .all(|child| holds(child, row, columns, depth + 1)),
            BooleanOperator::Or => predicates
                .iter()
                .any(|child| holds(child, row, columns, depth + 1)),
            BooleanOperator::Xor => {
                predicates
                    .iter()
                    .filter(|child| holds(child, row, columns, depth + 1))
                    .count()
                    % 2
                    == 1
            }
            BooleanOperator::Surrogate => {
                panic!("trial tree uses a surrogate predicate, which these exports do not")
            }
        },
    }
}

fn cell_of<'a>(row: &'a [String], columns: &HashMap<String, usize>, field: &str) -> &'a str {
    let slot = *columns
        .get(field)
        .unwrap_or_else(|| panic!("record has no field {field:?}"));
    row.get(slot).map(String::as_str).unwrap_or("")
}

fn cells_equal(cell: &str, expected: &str) -> bool {
    if cell == expected {
        return true;
    }
    match (cell.parse::<f64>(), expected.parse::<f64>()) {
        (Ok(left), Ok(right)) if left.is_finite() && right.is_finite() => left == right,
        _ => false,
    }
}

fn cell_number(cell: &str, field: &str) -> f64 {
    let value = cell
        .parse::<f64>()
        .unwrap_or_else(|_| panic!("non-numeric {cell:?} for {field}"));
    assert!(value.is_finite(), "non-finite {cell:?} for {field}");
    value
}

fn class_index(classes: &[String], name: &str, what: &str) -> usize {
    classes
        .iter()
        .position(|class| class == name)
        .unwrap_or_else(|| panic!("{what} class {name:?} is not among {classes:?}"))
}

fn trial_trees(pmml: &Pmml) -> Vec<&TreeModel> {
    let Model::Mining(mining) = &pmml.model else {
        panic!("boosted export is not a MiningModel of trial trees");
    };
    assert_eq!(mining.method, "c50ConfidenceVote");
    mining
        .segments
        .iter()
        .map(|segment| match segment.model.as_ref() {
            Model::Tree(tree) => tree,
            _ => panic!("boost segment is not a TreeModel"),
        })
        .collect()
}

fn target_name(tree: &TreeModel) -> String {
    tree.header
        .mining_schema
        .iter()
        .find(|field| field.usage == Usage::Target)
        .unwrap_or_else(|| panic!("trial tree has no target"))
        .name
        .clone()
}

fn class_names(pmml: &Pmml, target: &str) -> Vec<String> {
    let field = pmml
        .data_dictionary
        .iter()
        .find(|field| field.name == target)
        .unwrap_or_else(|| panic!("data dictionary has no {target:?}"));
    assert!(
        !field.values.is_empty(),
        "target {target:?} lists no classes"
    );
    field.values.clone()
}

fn read_pmml(dir: &Path) -> Pmml {
    let path = one_file(&dir.join("models"), |name| name.ends_with(".xml"), "model");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    Pmml::parse(&text).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn read_rows(dir: &Path) -> Rows {
    let path = one_file(
        &dir.join("data"),
        |name| name.ends_with(".csv") && name.split("_type_").nth(1).is_some(),
        "type export",
    );
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let mut lines = text.lines();
    let header = support::parse_csv_line(
        lines
            .next()
            .unwrap_or_else(|| panic!("empty {}", path.display())),
    );
    let mut columns = HashMap::new();
    for (index, name) in header.iter().enumerate() {
        assert!(
            columns.insert(name.clone(), index).is_none(),
            "duplicate column {name}"
        );
    }
    let target = *columns
        .get("y")
        .unwrap_or_else(|| panic!("type export {} has no y", path.display()));
    let mut cells = Vec::new();
    for (index, line) in lines.enumerate() {
        if line.is_empty() {
            continue;
        }
        let row = support::parse_csv_line(line);
        assert_eq!(
            row.len(),
            header.len(),
            "{} row {}: {} cells",
            path.display(),
            index + 1,
            row.len()
        );
        cells.push(row);
    }
    assert!(!cells.is_empty(), "no rows in {}", path.display());
    Rows {
        columns,
        target,
        cells,
    }
}

fn one_file(dir: &Path, pred: impl Fn(&str) -> bool, what: &str) -> PathBuf {
    let mut hits: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
        .map(|entry| entry.unwrap_or_else(|error| panic!("read {}: {error}", dir.display())))
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(&pred)
        })
        .collect();
    hits.sort();
    assert_eq!(hits.len(), 1, "{what} in {}: {hits:?}", dir.display());
    hits.remove(0)
}
