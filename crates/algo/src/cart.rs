//! C&R Tree (Algorithms Guide: "C&RT Algorithms"): binary splits chosen by Gini, twoing or least
//! squared deviation, surrogate splits for missing values, and cost-complexity pruning.
//!
//! The tree comes out the way Modeler writes it (oracle `models/cart_*`):
//! - Nodes are numbered breadth first. A node that was considered for splitting but whose best
//!   split fails the minimum child size or the minimum impurity change still uses up two ids
//!   (`cart_default` ids skip them); nodes stopped by depth, purity or the minimum parent size do
//!   not.
//! - Every child predicate is a `surrogate` compound: the primary split, the surrogates in
//!   order, then `True` for the default child (the larger one) and `False` for the other. A
//!   numeric split sends `x <= t` left as `and(True, x <= t)` and `x > t` right as
//!   `and(x > t, True)`; a categorical one is `or(x = a, x = b, ...)`, padded with `False`
//!   when it names one category.
//! - Classification nodes predict the most probable category (the first on ties) and carry one
//!   `ScoreDistribution` per category with probability = confidence = share of the node.
//!   Regression nodes predict the mean and carry the counts of ten equal-width bins of the
//!   target's training range (values `0`..`9`).
//! - Pruning keeps the smallest tree of the weakest-link sequence whose risk is minimal (or within
//!   the standard-error rule's margin), so branches that do not lower the training risk go.
//!
//! Tie rules settled against the oracle (`GAP:` where a single case decides):
//! - Within a field the first best split in scan order wins (ascending values; category subsets
//!   in binary count order with the first category on the left, then the subset whose labels
//!   sort first); across fields the first input wins unless another improves by more than 1e-12.
//! - A numeric surrogate keeps the lowest cut among equally good ones (30 of 32 ties).
//!   GAP: `cart_regression` prefers a higher cut for the surrogate of node 4 (child node 9);
//!   Modeler's agreement is probably a float sum there.
//! - A categorical surrogate keeps the last of equally good subsets. GAP: one oracle case
//!   (`cart_twoing` node 4).
//! - Surrogates with equal association are ordered by their own improvement on the target, then
//!   by input order (`cart_unpruned` nodes 6, 13, 15; `cart_depth3` node 5; `cart_twoing` nodes
//!   0, 13, 21).
//! - A categorical split names the categories present where it was made on each side.
//!
//! Supported: empirical priors and equal misclassification costs, no frequency or case weights.
//! Other settings return [`AlgoError::Unsupported`].

use crate::common::{
    class_summary, data_dictionary, mining_schema, regression_summary, split_text, value_range,
};
use crate::{AlgoError, Dataset, categories};
use dms_pmml::{
    BooleanOperator, MiningFunction, MissingValueStrategy, Model, ModelHeader, NoTrueChildStrategy,
    Node, Operator, Pmml, Predicate, ScoreDistribution, TreeModel,
};
use dms_types::{Measurement, Storage, Value};
use std::cmp::Ordering;
use std::collections::VecDeque;

/// The impurity measure of a classification tree (regression trees always use LSD).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Impurity {
    Gini,
    Twoing,
}

/// A minimum node size, as a share of the training records or as a record count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MinSize {
    Percent(f64),
    Records(f64),
}

impl MinSize {
    pub(crate) fn records(self, total: usize) -> f64 {
        match self {
            MinSize::Percent(percent) => percent / 100.0 * total as f64,
            MinSize::Records(records) => records,
        }
    }
}

/// Settings of a C&R Tree build node (Modeler's defaults in [`Default`]).
#[derive(Debug, Clone, PartialEq)]
pub struct CartSettings {
    /// `max_depth` (default 5): splits below the root.
    pub max_depth: usize,
    /// `min_parent_records_pc` / `_abs` (default 2%).
    pub min_parent: MinSize,
    /// `min_child_records_pc` / `_abs` (default 1%).
    pub min_child: MinSize,
    /// `min_impurity` (default 0.0001).
    pub min_impurity_change: f64,
    /// `impurity_measure` (default Gini).
    pub impurity: Impurity,
    /// `max_surrogates` (default 5).
    pub max_surrogates: usize,
    /// `prune_tree` (default true).
    pub prune: bool,
    /// `use_std_err_rule` with `std_err_multiplier`: `Some(multiplier)` when on (default off).
    pub std_err_rule: Option<f64>,
}

impl Default for CartSettings {
    fn default() -> Self {
        CartSettings {
            max_depth: 5,
            min_parent: MinSize::Percent(2.0),
            min_child: MinSize::Percent(1.0),
            min_impurity_change: 0.0001,
            impurity: Impurity::Gini,
            max_surrogates: 5,
            prune: true,
            std_err_rule: None,
        }
    }
}

/// Trains a C&R Tree model.
pub fn train(dataset: &Dataset<'_>, settings: &CartSettings) -> Result<Pmml, AlgoError> {
    if dataset.frequency.is_some() {
        return Err(AlgoError::Unsupported(
            "C&R Tree with a frequency field".into(),
        ));
    }
    let data = Data::read(dataset)?;
    if data.rows.is_empty() {
        return Err(AlgoError::Data("no records with a valid target".into()));
    }
    if data.is_regression() && settings.std_err_rule.is_some() {
        return Err(AlgoError::Unsupported(
            "the standard error rule for regression trees".into(),
        ));
    }
    let mut tree = grow(&data, settings);
    if settings.prune {
        prune(&mut tree, &data, settings);
    }
    let root = pmml_node(&tree, 0, &data, Predicate::True);
    let header = ModelHeader {
        name: None,
        function: if data.is_regression() {
            MiningFunction::Regression
        } else {
            MiningFunction::Classification
        },
        algorithm: Some("C&RT".into()),
        mining_schema: mining_schema(dataset),
        output: Vec::new(),
        local_transformations: Vec::new(),
    };
    Ok(Pmml {
        version: "4.3".into(),
        data_dictionary: data_dictionary(dataset),
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Tree(TreeModel {
            header,
            missing_value_strategy: MissingValueStrategy::None,
            no_true_child_strategy: NoTrueChildStrategy::ReturnNullPrediction,
            root,
        }),
    })
}

// ------------------------------------------------------------------------------------------
// Training data

/// An input field's values on the training records.
enum Column {
    /// Continuous (`ordinal == false`) or numeric ordinal values.
    Numeric {
        values: Vec<Option<f64>>,
        ordinal: bool,
    },
    /// Category codes into `labels`.
    Categorical {
        codes: Vec<Option<usize>>,
        labels: Vec<String>,
    },
}

impl Column {
    fn is_valid(&self, row: usize) -> bool {
        match self {
            Column::Numeric { values, .. } => values[row].is_some(),
            Column::Categorical { codes, .. } => codes[row].is_some(),
        }
    }
}

enum Target {
    Classes {
        codes: Vec<usize>,
        labels: Vec<String>,
    },
    Numbers(Vec<f64>),
}

struct Data {
    names: Vec<String>,
    columns: Vec<Column>,
    target: Target,
    /// Training records (valid target), as indices into the columns.
    rows: Vec<usize>,
    /// Empirical priors per class.
    priors: Vec<f64>,
    /// Training records per class.
    class_totals: Vec<f64>,
}

impl Data {
    fn read(dataset: &Dataset<'_>) -> Result<Data, AlgoError> {
        let fields = dataset.table.model().fields();
        let field = |position: usize| {
            fields
                .get(position)
                .ok_or_else(|| AlgoError::Data(format!("no field at position {position}")))
        };
        let table_rows = dataset.table.rows();

        let target_field = field(dataset.target)?;
        let target_value = |row: &[Value]| {
            row.get(dataset.target)
                .filter(|value| !target_field.is_missing(value))
                .cloned()
        };
        let mut rows = Vec::new();
        let target = if target_field.measurement == Measurement::Continuous {
            let mut numbers = vec![0.0; table_rows.len()];
            for (i, row) in table_rows.iter().enumerate() {
                if let Some(x) = target_value(row)
                    .and_then(|v| v.as_f64())
                    .filter(|x| x.is_finite())
                {
                    numbers[i] = x;
                    rows.push(i);
                }
            }
            Target::Numbers(numbers)
        } else {
            let mut labels = categories(dataset, dataset.target);
            let mut codes = vec![0; table_rows.len()];
            for (i, row) in table_rows.iter().enumerate() {
                if let Some(value) = target_value(row) {
                    codes[i] = code_of(&mut labels, value.to_string());
                    rows.push(i);
                }
            }
            Target::Classes { codes, labels }
        };

        let mut names = Vec::new();
        let mut columns = Vec::new();
        for &position in dataset.inputs {
            let input = field(position)?;
            let numeric = matches!(input.storage, Storage::Integer | Storage::Real);
            let cell = |row: &[Value]| {
                row.get(position)
                    .filter(|value| !input.is_missing(value))
                    .cloned()
            };
            let column = match input.measurement {
                Measurement::Continuous | Measurement::Ordinal if numeric => Column::Numeric {
                    values: table_rows
                        .iter()
                        .map(|row| cell(row).and_then(|v| v.as_f64()).filter(|x| x.is_finite()))
                        .collect(),
                    ordinal: input.measurement == Measurement::Ordinal,
                },
                Measurement::Continuous | Measurement::Ordinal => {
                    return Err(AlgoError::Unsupported(format!(
                        "C&R Tree input {} with non-numeric storage",
                        input.name
                    )));
                }
                _ => {
                    let mut labels = categories(dataset, position);
                    let codes = table_rows
                        .iter()
                        .map(|row| cell(row).map(|v| code_of(&mut labels, v.to_string())))
                        .collect();
                    Column::Categorical { codes, labels }
                }
            };
            names.push(input.name.clone());
            columns.push(column);
        }

        let (priors, class_totals) = match &target {
            Target::Classes { codes, labels } => {
                let mut totals = vec![0.0; labels.len()];
                for &row in &rows {
                    totals[codes[row]] += 1.0;
                }
                let n = rows.len() as f64;
                (totals.iter().map(|t| t / n).collect(), totals)
            }
            Target::Numbers(_) => (Vec::new(), Vec::new()),
        };
        Ok(Data {
            names,
            columns,
            target,
            rows,
            priors,
            class_totals,
        })
    }

    fn is_regression(&self) -> bool {
        matches!(self.target, Target::Numbers(_))
    }
}

fn code_of(labels: &mut Vec<String>, text: String) -> usize {
    match labels.iter().position(|label| *label == text) {
        Some(code) => code,
        None => {
            labels.push(text);
            labels.len() - 1
        }
    }
}

// ------------------------------------------------------------------------------------------
// Impurity

/// The prior-weighted class terms `π(j) N_j(t) / N_j` and their sum, the node mass `p(t)`
/// (Algorithms Guide, Gini), with `N_j` the root counts of records valid for the predictor.
fn mass_terms(counts: &[f64], priors: &[f64], root: &[f64]) -> (Vec<f64>, f64) {
    let mut terms = vec![0.0; counts.len()];
    let mut mass = 0.0;
    for j in 0..counts.len() {
        if root[j] > 0.0 && counts[j] > 0.0 {
            terms[j] = priors[j] * counts[j] / root[j];
            mass += terms[j];
        }
    }
    (terms, mass)
}

fn gini(counts: &[f64], priors: &[f64], root: &[f64]) -> f64 {
    let (terms, mass) = mass_terms(counts, priors, root);
    if mass <= 0.0 {
        return 0.0;
    }
    1.0 - terms.iter().map(|t| (t / mass) * (t / mass)).sum::<f64>()
}

/// The improvement of a classification split with the given child counts.
fn class_improvement(
    impurity: Impurity,
    left: &[f64],
    right: &[f64],
    priors: &[f64],
    root: &[f64],
) -> f64 {
    let (left_terms, left_mass) = mass_terms(left, priors, root);
    let (right_terms, right_mass) = mass_terms(right, priors, root);
    let mass = left_mass + right_mass;
    if mass <= 0.0 {
        return 0.0;
    }
    match impurity {
        Impurity::Gini => {
            let parent: Vec<f64> = left.iter().zip(right).map(|(l, r)| l + r).collect();
            gini(&parent, priors, root)
                - left_mass / mass * gini(left, priors, root)
                - right_mass / mass * gini(right, priors, root)
        }
        Impurity::Twoing => {
            if left_mass <= 0.0 || right_mass <= 0.0 {
                return 0.0;
            }
            let difference: f64 = left_terms
                .iter()
                .zip(&right_terms)
                .map(|(l, r)| (l / left_mass - r / right_mass).abs())
                .sum();
            let (p_left, p_right) = (left_mass / mass, right_mass / mass);
            p_left * p_right / 4.0 * difference * difference
        }
    }
}

/// Count, sum and sum of squares of target values.
#[derive(Debug, Clone, Copy, Default)]
struct Moments {
    n: f64,
    sum: f64,
    squares: f64,
}

impl Moments {
    fn add(&mut self, y: f64) {
        self.n += 1.0;
        self.sum += y;
        self.squares += y * y;
    }

    fn merge(&mut self, other: &Moments) {
        self.n += other.n;
        self.sum += other.sum;
        self.squares += other.squares;
    }

    fn sse(&self) -> f64 {
        if self.n <= 0.0 {
            0.0
        } else {
            self.squares - self.sum * self.sum / self.n
        }
    }
}

/// The LSD improvement `(SSE(t) - SSE(tL) - SSE(tR)) / N(t)`.
fn lsd_improvement(total: &Moments, left: &Moments) -> f64 {
    let right = Moments {
        n: total.n - left.n,
        sum: total.sum - left.sum,
        squares: total.squares - left.squares,
    };
    (total.sse() - left.sse() - right.sse()) / total.n
}

// ------------------------------------------------------------------------------------------
// Splits

#[derive(Debug, Clone, PartialEq)]
enum Rule {
    /// `x <= threshold` goes left.
    Threshold(f64),
    /// Category codes that go left and right: the categories present where the split was made
    /// (`cart_no_surrogates` node 24 names only `NORMAL` because `HIGH` is absent there). Other
    /// categories satisfy neither side.
    Categories { left: Vec<usize>, right: Vec<usize> },
}

impl Rule {
    fn goes_left(&self, column: &Column, row: usize) -> Option<bool> {
        match (self, column) {
            (Rule::Threshold(t), Column::Numeric { values, .. }) => values[row].map(|x| x <= *t),
            (Rule::Categories { left, right }, Column::Categorical { codes, .. }) => codes[row]
                .and_then(|code| {
                    if left.contains(&code) {
                        Some(true)
                    } else if right.contains(&code) {
                        Some(false)
                    } else {
                        None
                    }
                }),
            _ => None,
        }
    }
}

/// The present categories split into `set` and the rest.
fn categories_rule(present: &[usize], set: Vec<usize>) -> Rule {
    let right = present
        .iter()
        .copied()
        .filter(|c| !set.contains(c))
        .collect();
    Rule::Categories { left: set, right }
}

struct Candidate {
    improvement: f64,
    rule: Rule,
    left_valid: usize,
    right_valid: usize,
}

fn best_split(
    data: &Data,
    settings: &CartSettings,
    column: &Column,
    rows: &[usize],
) -> Option<Candidate> {
    let valid: Vec<usize> = rows
        .iter()
        .copied()
        .filter(|&r| column.is_valid(r))
        .collect();
    if valid.len() < 2 {
        return None;
    }
    match column {
        Column::Numeric { values, ordinal } => {
            let mut sorted = valid;
            sorted.sort_by(|a, b| {
                values[*a]
                    .partial_cmp(&values[*b])
                    .unwrap_or(Ordering::Equal)
            });
            numeric_split(data, settings, values, *ordinal, &sorted)
        }
        Column::Categorical { codes, labels } => {
            categorical_split(data, settings, codes, labels, &valid)
        }
    }
}

/// Root class counts of the records valid for a predictor (the `N_j` of the Gini formula).
fn root_counts(
    data: &Data,
    valid: impl Fn(usize) -> bool,
    codes: &[usize],
    classes: usize,
) -> Vec<f64> {
    let mut counts = vec![0.0; classes];
    for &row in &data.rows {
        if valid(row) {
            counts[codes[row]] += 1.0;
        }
    }
    counts
}

fn numeric_split(
    data: &Data,
    settings: &CartSettings,
    values: &[Option<f64>],
    ordinal: bool,
    sorted: &[usize],
) -> Option<Candidate> {
    let value = |i: usize| values[sorted[i]].unwrap_or(f64::NAN);
    // Cut positions: the first record of each new value.
    let cuts: Vec<usize> = (1..sorted.len())
        .filter(|&i| value(i) != value(i - 1))
        .collect();
    let mut best: Option<(f64, usize)> = None;
    let mut keep = |improvement: f64, cut: usize| {
        if best.is_none_or(|(b, _)| improvement > b + 1e-15) {
            best = Some((improvement, cut));
        }
    };
    match &data.target {
        Target::Classes { codes, labels } => {
            let root = root_counts(data, |r| values[r].is_some(), codes, labels.len());
            let mut left = vec![0.0; labels.len()];
            let mut right = vec![0.0; labels.len()];
            for &row in sorted {
                right[codes[row]] += 1.0;
            }
            let mut next = 0;
            for &cut in &cuts {
                while next < cut {
                    left[codes[sorted[next]]] += 1.0;
                    right[codes[sorted[next]]] -= 1.0;
                    next += 1;
                }
                keep(
                    class_improvement(settings.impurity, &left, &right, &data.priors, &root),
                    cut,
                );
            }
        }
        Target::Numbers(ys) => {
            let mut total = Moments::default();
            for &row in sorted {
                total.add(ys[row]);
            }
            let mut left = Moments::default();
            let mut next = 0;
            for &cut in &cuts {
                while next < cut {
                    left.add(ys[sorted[next]]);
                    next += 1;
                }
                keep(lsd_improvement(&total, &left), cut);
            }
        }
    }
    let (improvement, cut) = best?;
    let threshold = if ordinal {
        value(cut - 1)
    } else {
        (value(cut - 1) + value(cut)) / 2.0
    };
    Some(Candidate {
        improvement,
        rule: Rule::Threshold(threshold),
        left_valid: cut,
        right_valid: sorted.len() - cut,
    })
}

/// Categories present among `rows`, in label order.
fn present_categories(codes: &[Option<usize>], categories: usize, rows: &[usize]) -> Vec<usize> {
    let mut present = vec![false; categories];
    for &row in rows {
        if let Some(code) = codes[row] {
            present[code] = true;
        }
    }
    (0..categories).filter(|&c| present[c]).collect()
}

/// Subsets of the present categories that contain the first one and not all of them, in binary
/// count order of the other categories.
fn subsets(present: &[usize]) -> Option<Vec<Vec<usize>>> {
    let k = present.len();
    // GAP: Modeler's search over more than 16 categories is not known; such fields are skipped.
    if !(2..=16).contains(&k) {
        return None;
    }
    Some(
        (0u32..(1 << (k - 1)) - 1)
            .map(|mask| {
                std::iter::once(present[0])
                    .chain(
                        (0..k - 1)
                            .filter(|b| mask & (1 << b) != 0)
                            .map(|b| present[b + 1]),
                    )
                    .collect()
            })
            .collect(),
    )
}

fn categorical_split(
    data: &Data,
    settings: &CartSettings,
    codes: &[Option<usize>],
    labels: &[String],
    valid: &[usize],
) -> Option<Candidate> {
    let present = present_categories(codes, labels.len(), valid);
    let code = |row: usize| codes[row].unwrap_or(0);
    let names = |set: &[usize]| set.iter().map(|&c| labels[c].as_str()).collect::<Vec<_>>();
    let mut best: Option<(f64, Vec<usize>)> = None;
    let mut keep = |improvement: f64, set: Vec<usize>| {
        let better = best.as_ref().is_none_or(|(b, best_set)| {
            improvement > b + 1e-15
                || ((improvement - b).abs() <= 1e-15 && names(&set) < names(best_set))
        });
        if better {
            best = Some((improvement, set));
        }
    };
    match &data.target {
        Target::Classes {
            codes: classes,
            labels: class_labels,
        } => {
            let k = class_labels.len();
            let root = root_counts(data, |r| codes[r].is_some(), classes, k);
            let mut by_category = vec![vec![0.0; k]; labels.len()];
            for &row in valid {
                by_category[code(row)][classes[row]] += 1.0;
            }
            for set in subsets(&present)? {
                let mut left = vec![0.0; k];
                let mut right = vec![0.0; k];
                for &c in &present {
                    let side = if set.contains(&c) {
                        &mut left
                    } else {
                        &mut right
                    };
                    for (j, n) in by_category[c].iter().enumerate() {
                        side[j] += n;
                    }
                }
                keep(
                    class_improvement(settings.impurity, &left, &right, &data.priors, &root),
                    set,
                );
            }
        }
        Target::Numbers(ys) => {
            let mut by_category = vec![Moments::default(); labels.len()];
            let mut total = Moments::default();
            for &row in valid {
                by_category[code(row)].add(ys[row]);
                total.add(ys[row]);
            }
            for set in subsets(&present)? {
                let mut left = Moments::default();
                for &c in &set {
                    left.merge(&by_category[c]);
                }
                keep(lsd_improvement(&total, &left), set);
            }
        }
    }
    let (improvement, set) = best?;
    let left_valid = valid.iter().filter(|&&r| set.contains(&code(r))).count();
    Some(Candidate {
        improvement,
        rule: categories_rule(&present, set),
        left_valid,
        right_valid: valid.len() - left_valid,
    })
}

// ------------------------------------------------------------------------------------------
// Surrogates

#[derive(Debug, Clone)]
struct Surrogate {
    input: usize,
    rule: Rule,
    /// Whether the records satisfying `rule` go to the primary's left child.
    rule_left: bool,
    association: f64,
    /// The surrogate split's own improvement on the target, which orders surrogates of equal
    /// association.
    improvement: f64,
}

/// The best surrogate on `input` for the records of `labeled` (`(row, goes left)`): the split
/// that sends the most of them where the primary does, scored by the predictive measure of
/// association `(agree - majority) / (n - majority)`.
fn best_surrogate(
    data: &Data,
    settings: &CartSettings,
    input: usize,
    labeled: &[(usize, bool)],
) -> Option<Surrogate> {
    let column = &data.columns[input];
    let both: Vec<(usize, bool)> = labeled
        .iter()
        .copied()
        .filter(|(row, _)| column.is_valid(*row))
        .collect();
    let n = both.len();
    let n_left = both.iter().filter(|(_, left)| *left).count();
    let majority = n_left.max(n - n_left);
    if n < 2 || n == majority {
        return None;
    }
    // (agreement, rule, rule goes left)
    let mut best: Option<(usize, Rule, bool)> = None;
    match column {
        Column::Numeric { values, ordinal } => {
            let mut sorted = both;
            sorted.sort_by(|a, b| {
                values[a.0]
                    .partial_cmp(&values[b.0])
                    .unwrap_or(Ordering::Equal)
            });
            let value = |i: usize| values[sorted[i].0].unwrap_or(f64::NAN);
            let mut low_and_left = 0;
            for cut in 1..n {
                if sorted[cut - 1].1 {
                    low_and_left += 1;
                }
                if value(cut) == value(cut - 1) {
                    continue;
                }
                // Low values to the left agree for the primary-left low records and the
                // primary-right high ones.
                let low_left = low_and_left + (n - cut) - (n_left - low_and_left);
                let (agree, rule_left) = if low_left >= n - low_left {
                    (low_left, true)
                } else {
                    (n - low_left, false)
                };
                // The lowest cut wins among equal agreements.
                if best.as_ref().is_none_or(|(b, _, _)| agree > *b) {
                    let threshold = if *ordinal {
                        value(cut - 1)
                    } else {
                        (value(cut - 1) + value(cut)) / 2.0
                    };
                    best = Some((agree, Rule::Threshold(threshold), rule_left));
                }
            }
        }
        Column::Categorical { codes, labels } => {
            let rows: Vec<usize> = both.iter().map(|(row, _)| *row).collect();
            let present = present_categories(codes, labels.len(), &rows);
            let mut count = vec![0usize; labels.len()];
            let mut count_left = vec![0usize; labels.len()];
            for &(row, left) in &both {
                if let Some(code) = codes[row] {
                    count[code] += 1;
                    if left {
                        count_left[code] += 1;
                    }
                }
            }
            for set in subsets(&present)? {
                let in_set: usize = set.iter().map(|&c| count[c]).sum();
                let in_set_left: usize = set.iter().map(|&c| count_left[c]).sum();
                let set_left = in_set_left + (n - in_set) - (n_left - in_set_left);
                let (agree, rule_left) = if set_left >= n - set_left {
                    (set_left, true)
                } else {
                    (n - set_left, false)
                };
                // GAP: the last of equally good subsets wins (one oracle case).
                if best.as_ref().is_none_or(|(b, _, _)| agree >= *b) {
                    best = Some((agree, categories_rule(&present, set), rule_left));
                }
            }
        }
    }
    let (agree, rule, rule_left) = best?;
    let association = (agree as f64 - majority as f64) / (n - majority) as f64;
    if association <= 0.0 {
        return None;
    }
    let sides: Vec<(usize, bool)> = labeled
        .iter()
        .filter_map(|&(row, _)| rule.goes_left(column, row).map(|left| (row, left)))
        .collect();
    Some(Surrogate {
        input,
        improvement: split_improvement(data, settings, column, &sides),
        rule,
        rule_left,
        association,
    })
}

/// The improvement of the split that sends the `sides` records left (`true`) or right.
fn split_improvement(
    data: &Data,
    settings: &CartSettings,
    column: &Column,
    sides: &[(usize, bool)],
) -> f64 {
    match &data.target {
        Target::Classes { codes, labels } => {
            let root = root_counts(data, |r| column.is_valid(r), codes, labels.len());
            let mut left = vec![0.0; labels.len()];
            let mut right = vec![0.0; labels.len()];
            for &(row, goes_left) in sides {
                if goes_left {
                    left[codes[row]] += 1.0;
                } else {
                    right[codes[row]] += 1.0;
                }
            }
            class_improvement(settings.impurity, &left, &right, &data.priors, &root)
        }
        Target::Numbers(ys) => {
            let mut total = Moments::default();
            let mut left = Moments::default();
            for &(row, goes_left) in sides {
                total.add(ys[row]);
                if goes_left {
                    left.add(ys[row]);
                }
            }
            lsd_improvement(&total, &left)
        }
    }
}

fn surrogates(
    data: &Data,
    settings: &CartSettings,
    primary: usize,
    rule: &Rule,
    rows: &[usize],
) -> Vec<Surrogate> {
    let max = settings.max_surrogates;
    if max == 0 {
        return Vec::new();
    }
    let labeled: Vec<(usize, bool)> = rows
        .iter()
        .filter_map(|&row| {
            rule.goes_left(&data.columns[primary], row)
                .map(|left| (row, left))
        })
        .collect();
    let mut found: Vec<Surrogate> = (0..data.columns.len())
        .filter(|&input| input != primary)
        .filter_map(|input| best_surrogate(data, settings, input, &labeled))
        .collect();
    // Higher association first; on ties the larger improvement, then the earlier input
    // (`cart_unpruned` nodes 6, 13, 15; `cart_depth3` node 5; `cart_twoing` nodes 0, 13, 21).
    let descending = |a: f64, b: f64| {
        if (a - b).abs() <= 1e-12 {
            Ordering::Equal
        } else {
            b.partial_cmp(&a).unwrap_or(Ordering::Equal)
        }
    };
    found.sort_by(|a, b| {
        descending(a.association, b.association)
            .then(descending(a.improvement, b.improvement))
            .then(a.input.cmp(&b.input))
    });
    found.truncate(max);
    found
}

// ------------------------------------------------------------------------------------------
// Growing

struct Split {
    input: usize,
    rule: Rule,
    surrogates: Vec<Surrogate>,
    /// Whether the left child is the default child.
    default_left: bool,
    /// Indices of the children in the tree's node list.
    left: usize,
    right: usize,
}

struct GrownNode {
    id: usize,
    rows: Vec<usize>,
    parent: Option<usize>,
    split: Option<Split>,
}

struct Tree {
    nodes: Vec<GrownNode>,
}

fn grow(data: &Data, settings: &CartSettings) -> Tree {
    let total = data.rows.len();
    let min_parent = settings.min_parent.records(total);
    let min_child = settings.min_child.records(total);
    let mut nodes = vec![GrownNode {
        id: 0,
        rows: data.rows.clone(),
        parent: None,
        split: None,
    }];
    let mut next_id = 1;
    let mut queue = VecDeque::from([(0usize, 0usize)]);
    while let Some((index, depth)) = queue.pop_front() {
        let rows = std::mem::take(&mut nodes[index].rows);
        let stop =
            depth >= settings.max_depth || is_pure(data, &rows) || (rows.len() as f64) < min_parent;
        let split = if stop {
            None
        } else {
            let made = split_node(data, settings, &rows, min_child);
            if made.is_none() {
                // The ids of the children the node did not get are used up.
                next_id += 2;
            }
            made
        };
        if let Some((input, rule, surrogates, default_left, left_rows, right_rows)) = split {
            let (left, right) = (nodes.len(), nodes.len() + 1);
            for (offset, child_rows) in [left_rows, right_rows].into_iter().enumerate() {
                nodes.push(GrownNode {
                    id: next_id + offset,
                    rows: child_rows,
                    parent: Some(index),
                    split: None,
                });
            }
            next_id += 2;
            nodes[index].split = Some(Split {
                input,
                rule,
                surrogates,
                default_left,
                left,
                right,
            });
            queue.push_back((left, depth + 1));
            queue.push_back((right, depth + 1));
        }
        nodes[index].rows = rows;
    }
    Tree { nodes }
}

type MadeSplit = (usize, Rule, Vec<Surrogate>, bool, Vec<usize>, Vec<usize>);

/// The node's split, or `None` when its best split fails the stopping rules.
fn split_node(
    data: &Data,
    settings: &CartSettings,
    rows: &[usize],
    min_child: f64,
) -> Option<MadeSplit> {
    let mut best: Option<(usize, Candidate)> = None;
    for (input, column) in data.columns.iter().enumerate() {
        if let Some(candidate) = best_split(data, settings, column, rows)
            && best
                .as_ref()
                .is_none_or(|(_, b)| candidate.improvement > b.improvement + 1e-12)
        {
            best = Some((input, candidate));
        }
    }
    let (input, candidate) = best.filter(|(_, c)| {
        c.improvement >= settings.min_impurity_change
            && c.left_valid as f64 >= min_child
            && c.right_valid as f64 >= min_child
    })?;
    let column = &data.columns[input];
    let surrogates = surrogates(data, settings, input, &candidate.rule, rows);
    let default_left = candidate.left_valid >= candidate.right_valid;
    let (mut left_rows, mut right_rows) = (Vec::new(), Vec::new());
    for &row in rows {
        let left = candidate.rule.goes_left(column, row).or_else(|| {
            surrogates.iter().find_map(|s| {
                s.rule
                    .goes_left(&data.columns[s.input], row)
                    .map(|hit| hit == s.rule_left)
            })
        });
        if left.unwrap_or(default_left) {
            left_rows.push(row);
        } else {
            right_rows.push(row);
        }
    }
    Some((
        input,
        candidate.rule,
        surrogates,
        default_left,
        left_rows,
        right_rows,
    ))
}

fn is_pure(data: &Data, rows: &[usize]) -> bool {
    let Some(&first) = rows.first() else {
        return true;
    };
    match &data.target {
        Target::Classes { codes, .. } => rows.iter().all(|&r| codes[r] == codes[first]),
        Target::Numbers(ys) => rows.iter().all(|&r| ys[r] == ys[first]),
    }
}

// ------------------------------------------------------------------------------------------
// Pruning

/// The resubstitution risk of a node as a leaf: its misclassified records, or its sum of squared
/// deviations, over all training records.
fn node_risk(data: &Data, rows: &[usize]) -> f64 {
    let total = data.rows.len() as f64;
    match &data.target {
        Target::Classes { codes, labels } => {
            let mut counts = vec![0usize; labels.len()];
            for &row in rows {
                counts[codes[row]] += 1;
            }
            let majority = counts.iter().copied().max().unwrap_or(0);
            (rows.len() - majority) as f64 / total
        }
        Target::Numbers(ys) => {
            let mut moments = Moments::default();
            for &row in rows {
                moments.add(ys[row]);
            }
            moments.sse() / total
        }
    }
}

/// Weakest-link pruning (Algorithms Guide, "Pruning"): collapse the splits with the smallest
/// `g(t) = (R(t) - R(T_t)) / (|T_t| - 1)` until only the root is left, then keep the smallest tree
/// of that sequence whose risk is within the chosen margin of the minimum.
fn prune(tree: &mut Tree, data: &Data, settings: &CartSettings) {
    let risk: Vec<f64> = tree
        .nodes
        .iter()
        .map(|n| node_risk(data, &n.rows))
        .collect();
    let mut kept: Vec<bool> = tree.nodes.iter().map(|n| n.split.is_some()).collect();
    let mut sequence = vec![(kept.clone(), subtree(tree, &kept, &risk, 0).0)];
    while kept[0] {
        let links: Vec<(usize, f64)> = (0..tree.nodes.len())
            .filter(|&index| kept[index])
            .map(|index| {
                let (branch_risk, leaves) = subtree(tree, &kept, &risk, index);
                (index, (risk[index] - branch_risk) / (leaves as f64 - 1.0))
            })
            .collect();
        let weakest = links.iter().map(|(_, g)| *g).fold(f64::INFINITY, f64::min);
        for (index, g) in links {
            if g <= weakest + 1e-12 {
                kept[index] = false;
            }
        }
        // Splits below a collapsed node go with it.
        for index in 0..tree.nodes.len() {
            if kept[index] && !ancestors_kept(tree, &kept, index) {
                kept[index] = false;
            }
        }
        let tree_risk = subtree(tree, &kept, &risk, 0).0;
        sequence.push((kept.clone(), tree_risk));
    }
    let minimum = sequence
        .iter()
        .map(|(_, r)| *r)
        .fold(f64::INFINITY, f64::min);
    let margin = settings.std_err_rule.map_or(0.0, |multiplier| {
        let n = data.rows.len() as f64;
        multiplier * (minimum * (1.0 - minimum) / n).max(0.0).sqrt()
    });
    if let Some((chosen, _)) = sequence
        .iter()
        .rev()
        .find(|(_, r)| *r <= minimum + margin + 1e-12)
    {
        for (node, keep) in tree.nodes.iter_mut().zip(chosen) {
            if !keep {
                node.split = None;
            }
        }
    }
}

/// Risk and leaf count of the subtree under `index` with the `kept` splits.
fn subtree(tree: &Tree, kept: &[bool], risk: &[f64], index: usize) -> (f64, usize) {
    match &tree.nodes[index].split {
        Some(split) if kept[index] => {
            let (left_risk, left_leaves) = subtree(tree, kept, risk, split.left);
            let (right_risk, right_leaves) = subtree(tree, kept, risk, split.right);
            (left_risk + right_risk, left_leaves + right_leaves)
        }
        _ => (risk[index], 1),
    }
}

fn ancestors_kept(tree: &Tree, kept: &[bool], index: usize) -> bool {
    let mut current = tree.nodes[index].parent;
    while let Some(parent) = current {
        if !kept[parent] {
            return false;
        }
        current = tree.nodes[parent].parent;
    }
    true
}

// ------------------------------------------------------------------------------------------
// PMML

fn pmml_node(tree: &Tree, index: usize, data: &Data, predicate: Predicate) -> Node {
    let node = &tree.nodes[index];
    let (score, distribution) = summary(data, &node.rows);
    let mut out = Node {
        id: Some(node.id.to_string()),
        score: Some(score),
        record_count: Some(node.rows.len() as f64),
        default_child: None,
        predicate,
        distribution,
        children: Vec::new(),
    };
    if let Some(split) = &node.split {
        let (left, right) = (&tree.nodes[split.left], &tree.nodes[split.right]);
        let default = if split.default_left {
            left.id
        } else {
            right.id
        };
        out.default_child = Some(default.to_string());
        out.children = vec![
            pmml_node(tree, split.left, data, child_predicate(data, split, true)),
            pmml_node(tree, split.right, data, child_predicate(data, split, false)),
        ];
    }
    out
}

fn child_predicate(data: &Data, split: &Split, left: bool) -> Predicate {
    let mut parts = vec![rule_predicate(data, split.input, &split.rule, left)];
    for s in &split.surrogates {
        parts.push(rule_predicate(data, s.input, &s.rule, left == s.rule_left));
    }
    parts.push(if split.default_left == left {
        Predicate::True
    } else {
        Predicate::False
    });
    Predicate::Compound {
        operator: BooleanOperator::Surrogate,
        predicates: parts,
    }
}

/// The predicate of the records that satisfy `rule` (`satisfied`) or do not.
fn rule_predicate(data: &Data, input: usize, rule: &Rule, satisfied: bool) -> Predicate {
    let field = data.names[input].clone();
    match (rule, &data.columns[input]) {
        (Rule::Threshold(t), _) => {
            let simple = Predicate::Simple {
                field,
                operator: if satisfied {
                    Operator::LessOrEqual
                } else {
                    Operator::GreaterThan
                },
                value: Some(split_text(*t)),
            };
            let predicates = if satisfied {
                vec![Predicate::True, simple]
            } else {
                vec![simple, Predicate::True]
            };
            Predicate::Compound {
                operator: BooleanOperator::And,
                predicates,
            }
        }
        (Rule::Categories { left, right }, Column::Categorical { labels, .. }) => {
            let chosen = if satisfied { left } else { right };
            let mut predicates: Vec<Predicate> = chosen
                .iter()
                .map(|&c| Predicate::Simple {
                    field: field.clone(),
                    operator: Operator::Equal,
                    value: Some(labels[c].clone()),
                })
                .collect();
            if predicates.len() < 2 {
                predicates.push(Predicate::False);
            }
            Predicate::Compound {
                operator: BooleanOperator::Or,
                predicates,
            }
        }
        (Rule::Categories { .. }, Column::Numeric { .. }) => Predicate::False,
    }
}

/// A node's prediction and score distribution.
fn summary(data: &Data, rows: &[usize]) -> (String, Vec<ScoreDistribution>) {
    match &data.target {
        Target::Classes { codes, labels } => {
            let mut counts = vec![0.0; labels.len()];
            for &row in rows {
                counts[codes[row]] += 1.0;
            }
            class_summary(labels, &counts, &data.priors, &data.class_totals)
        }
        Target::Numbers(ys) => regression_summary(
            rows.iter().map(|&r| ys[r]),
            value_range(data.rows.iter().map(|&r| ys[r])),
        ),
    }
}
