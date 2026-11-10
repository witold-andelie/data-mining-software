//! CHAID and Exhaustive CHAID (Algorithms Guide: "CHAID Algorithms"): predictors' categories are
//! merged while they do not differ significantly on the target (chi-squared for categorical
//! targets, F for continuous ones), and the predictor with the smallest Bonferroni-adjusted
//! p-value splits the node into one child per merged group.
//!
//! The tree comes out the way Modeler writes it (oracle `models/chaid_*`):
//! - Continuous predictors are binned once into ten ordered categories at SPSS's percentile
//!   positions (see `bin_cuts`); numeric ordinal predictors keep their values as ordered
//!   categories; both merge adjacent categories only.
//! - Nodes are numbered breadth first without gaps. Children follow the order of their
//!   categories; the default child is the largest (the first on ties).
//! - A child predicate is a `surrogate` compound of the group's predicate and `True` (default
//!   child) or `False`. Nominal groups are `or(x = a, ...)`, padded with `False` for one category.
//!   Ordered groups are ranges `and(True, x <= u)`, `and(x > l, x <= u)`, `and(x > l, True)`,
//!   whose bounds are the upper bounds of the group's last category: the bin's largest value
//!   for a continuous predictor (`chaid_default` node 4: `K <= 0.037851`; node 26: `Age <= 49`),
//!   the value itself for an ordinal one.
//! - Chi-squared tests use the target categories present in the table (`chaid_default` root:
//!   3 BP groups x 5 drugs, df 8; node 14: 2 groups x 3 drugs, df 2).
//!
//! GAP (no oracle exercises them): missing predictor values, which form their own category
//! (floating for ordered predictors); categories below the minimum child size are merged with
//! their most similar neighbour in order.
//! Supported: no frequency or case weights, no merged-category splitting; other settings return
//! [`AlgoError::Unsupported`].

use crate::cart::MinSize;
use crate::common::{
    class_summary, data_dictionary, mining_schema, regression_summary, split_text, value_range,
};
use crate::stats::{
    bonferroni_nominal, bonferroni_ordinal, bonferroni_ordinal_missing, chi_square_p, f_p,
};
use crate::{AlgoError, Dataset, categories};
use dms_pmml::{
    BooleanOperator, MiningFunction, MissingValueStrategy, Model, ModelHeader, NoTrueChildStrategy,
    Node, Operator, Pmml, Predicate, ScoreDistribution, TreeModel,
};
use dms_types::{Measurement, Storage, Value};
use std::collections::VecDeque;

/// CHAID or Exhaustive CHAID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Chaid,
    Exhaustive,
}

/// The chi-squared statistic for categorical targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChiSquare {
    Pearson,
    LikelihoodRatio,
}

/// Settings of a CHAID build node (Modeler's defaults in [`Default`]).
#[derive(Debug, Clone, PartialEq)]
pub struct ChaidSettings {
    /// `method` (default CHAID).
    pub method: Method,
    /// `max_depth` (default 5).
    pub max_depth: usize,
    /// `min_parent_records_pc` / `_abs` (default 2%).
    pub min_parent: MinSize,
    /// `min_child_records_pc` / `_abs` (default 1%).
    pub min_child: MinSize,
    /// `split_alpha` (default 0.05).
    pub split_alpha: f64,
    /// `merge_alpha` (default 0.05).
    pub merge_alpha: f64,
    /// `bonferroni_adjustment` (default true).
    pub bonferroni: bool,
    /// `chi_square` (default Pearson).
    pub chi_square: ChiSquare,
}

impl Default for ChaidSettings {
    fn default() -> Self {
        ChaidSettings {
            method: Method::Chaid,
            max_depth: 5,
            min_parent: MinSize::Percent(2.0),
            min_child: MinSize::Percent(1.0),
            split_alpha: 0.05,
            merge_alpha: 0.05,
            bonferroni: true,
            chi_square: ChiSquare::Pearson,
        }
    }
}

/// Bins of a continuous predictor.
const INTERVALS: usize = 10;

/// Trains a CHAID model.
pub fn train(dataset: &Dataset<'_>, settings: &ChaidSettings) -> Result<Pmml, AlgoError> {
    crate::check_cancelled()?;
    train_impl(dataset, settings, false)
}

/// Analytic Engine presentation of a tree trained on pre-binned ordinal inputs.
pub(crate) fn train_analytic(
    dataset: &Dataset<'_>,
    settings: &ChaidSettings,
) -> Result<Pmml, AlgoError> {
    train_impl(dataset, settings, true)
}

fn train_impl(
    dataset: &Dataset<'_>,
    settings: &ChaidSettings,
    analytic: bool,
) -> Result<Pmml, AlgoError> {
    if dataset.frequency.is_some() {
        return Err(AlgoError::Unsupported(
            "CHAID with a frequency field".into(),
        ));
    }
    let data = Data::read(dataset)?;
    if data.rows.is_empty() {
        return Err(AlgoError::Data("no records with a valid target".into()));
    }
    let tree = grow(&data, settings)?;
    let root = pmml_node(&tree, 0, &data, Predicate::True, analytic);
    let header = ModelHeader {
        name: None,
        function: if data.is_regression() {
            MiningFunction::Regression
        } else {
            MiningFunction::Classification
        },
        algorithm: Some("CHAID".into()),
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
            missing_value_strategy: if analytic {
                MissingValueStrategy::LastPrediction
            } else {
                MissingValueStrategy::None
            },
            no_true_child_strategy: NoTrueChildStrategy::ReturnNullPrediction,
            root,
        }),
    })
}

// ------------------------------------------------------------------------------------------
// Training data

/// A predictor as categories: ordered (binned continuous or numeric ordinal) or nominal.
struct Predictor {
    name: String,
    ordered: bool,
    /// Category code per record; `None` when the value is missing.
    codes: Vec<Option<usize>>,
    /// Number of categories (codes `0..categories`; the missing category is `categories`).
    categories: usize,
    /// Ordered predictors: each category's upper bound (a bin's largest value, or the value).
    uppers: Vec<f64>,
    /// Nominal predictors: the category labels.
    labels: Vec<String>,
}

enum Target {
    Classes {
        codes: Vec<usize>,
        labels: Vec<String>,
    },
    Numbers(Vec<f64>),
}

struct Data {
    predictors: Vec<Predictor>,
    target: Target,
    /// Training records (valid target).
    rows: Vec<usize>,
    priors: Vec<f64>,
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
                crate::check_cancelled()?;
                if let Some(y) = target_value(row)
                    .and_then(|v| v.as_f64())
                    .filter(|y| y.is_finite())
                {
                    numbers[i] = y;
                    rows.push(i);
                }
            }
            Target::Numbers(numbers)
        } else {
            let mut labels = categories(dataset, dataset.target);
            let mut codes = vec![0; table_rows.len()];
            for (i, row) in table_rows.iter().enumerate() {
                crate::check_cancelled()?;
                if let Some(value) = target_value(row) {
                    codes[i] = code_of(&mut labels, value.to_string());
                    rows.push(i);
                }
            }
            Target::Classes { codes, labels }
        };

        let mut predictors = Vec::new();
        for &position in dataset.inputs {
            crate::check_cancelled()?;
            let input = field(position)?;
            let cell = |row: &[Value]| {
                row.get(position)
                    .filter(|value| !input.is_missing(value))
                    .cloned()
            };
            let numeric = matches!(input.storage, Storage::Integer | Storage::Real);
            let number =
                |row: &[Value]| cell(row).and_then(|v| v.as_f64()).filter(|x| x.is_finite());
            let predictor = match input.measurement {
                Measurement::Continuous if numeric => {
                    let values: Vec<Option<f64>> = table_rows.iter().map(|r| number(r)).collect();
                    let cuts = bin_cuts(rows.iter().filter_map(|&r| values[r]).collect());
                    ordered_predictor(&input.name, &values, &cuts)
                }
                Measurement::Ordinal if numeric => {
                    let values: Vec<Option<f64>> = table_rows.iter().map(|r| number(r)).collect();
                    let mut levels: Vec<f64> = rows.iter().filter_map(|&r| values[r]).collect();
                    levels.sort_by(f64::total_cmp);
                    levels.dedup();
                    ordered_predictor(&input.name, &values, &levels)
                }
                Measurement::Continuous | Measurement::Ordinal => {
                    return Err(AlgoError::Unsupported(format!(
                        "CHAID input {} with non-numeric storage",
                        input.name
                    )));
                }
                _ => {
                    let mut labels = categories(dataset, position);
                    let codes: Vec<Option<usize>> = table_rows
                        .iter()
                        .map(|row| cell(row).map(|v| code_of(&mut labels, v.to_string())))
                        .collect();
                    Predictor {
                        name: input.name.clone(),
                        ordered: false,
                        codes,
                        categories: labels.len(),
                        uppers: Vec::new(),
                        labels,
                    }
                }
            };
            predictors.push(predictor);
        }

        let (priors, class_totals) = match &target {
            Target::Classes { codes, labels } => {
                let mut totals = vec![0.0; labels.len()];
                for &row in &rows {
                    crate::check_cancelled()?;
                    totals[codes[row]] += 1.0;
                }
                let n = rows.len() as f64;
                (totals.iter().map(|t| t / n).collect(), totals)
            }
            Target::Numbers(_) => (Vec::new(), Vec::new()),
        };
        Ok(Data {
            predictors,
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

/// An ordered predictor whose category is the first upper bound a value does not exceed.
fn ordered_predictor(name: &str, values: &[Option<f64>], uppers: &[f64]) -> Predictor {
    let codes = values
        .iter()
        .map(|value| {
            value.map(|x| {
                uppers
                    .iter()
                    .position(|&upper| x <= upper)
                    .unwrap_or(uppers.len().saturating_sub(1))
            })
        })
        .collect();
    Predictor {
        name: name.to_string(),
        ordered: true,
        codes,
        categories: uppers.len(),
        uppers: uppers.to_vec(),
        labels: Vec::new(),
    }
}

/// The upper bounds of the bins of a continuous predictor. A value and its ties, at ranks
/// `first..=last` of the `n` sorted values, fall in the bin after every decile position
/// `(n + 1) k / 10` at or below their middle rank `(first + last) / 2` (the percentile positions
/// of SPSS's default definition). This fits every oracle bin edge: `chaid_default` `K`, `Na`
/// and the tied `Age` (49 ends bin 6 though the sixth decile falls inside its ties), and
/// `chaid_exhaustive` `Age` (ranks 492 and 1725 end bins 2 and 7), where the guide's
/// cumulative share does not.
fn bin_cuts(mut values: Vec<f64>) -> Vec<f64> {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    let mut cuts: Vec<f64> = Vec::new();
    let mut previous_bin = None;
    let mut i = 0;
    while i < n {
        let value = values[i];
        let mut j = i;
        while j < n && values[j] == value {
            j += 1;
        }
        // Ranks are 1-based: first = i + 1, last = j. Compare 2 (n + 1) k <= 10 (first + last).
        let doubled_middle = 10 * (i + 1 + j);
        let bin = (1..INTERVALS)
            .filter(|&k| 2 * (n + 1) * k <= doubled_middle)
            .count();
        if previous_bin == Some(bin) {
            if let Some(last) = cuts.last_mut() {
                *last = value;
            }
        } else {
            cuts.push(value);
            previous_bin = Some(bin);
        }
        i = j;
    }
    cuts
}

// ------------------------------------------------------------------------------------------
// Tables and tests

/// The target statistics of a set of records: class counts or moments.
#[derive(Debug, Clone)]
struct Cell {
    n: f64,
    counts: Vec<f64>,
    sum: f64,
    squares: f64,
}

impl Cell {
    fn new(classes: usize) -> Cell {
        Cell {
            n: 0.0,
            counts: vec![0.0; classes],
            sum: 0.0,
            squares: 0.0,
        }
    }

    fn add_row(&mut self, target: &Target, row: usize) {
        self.n += 1.0;
        match target {
            Target::Classes { codes, .. } => self.counts[codes[row]] += 1.0,
            Target::Numbers(ys) => {
                self.sum += ys[row];
                self.squares += ys[row] * ys[row];
            }
        }
    }

    fn merge(&mut self, other: &Cell) {
        self.n += other.n;
        self.sum += other.sum;
        self.squares += other.squares;
        for (a, b) in self.counts.iter_mut().zip(&other.counts) {
            *a += b;
        }
    }
}

/// The p-value of the association between the cells (rows) and the target.
fn p_value(cells: &[&Cell], regression: bool, chi_square: ChiSquare) -> (f64, f64) {
    if regression {
        let n: f64 = cells.iter().map(|c| c.n).sum();
        let r = cells.len() as f64;
        let mean = cells.iter().map(|c| c.sum).sum::<f64>() / n;
        let between: f64 = cells
            .iter()
            .map(|c| c.n * (c.sum / c.n - mean).powi(2))
            .sum();
        let within: f64 = cells.iter().map(|c| c.squares - c.sum * c.sum / c.n).sum();
        if r < 2.0 || n <= r {
            return (1.0, 0.0);
        }
        if within <= 0.0 {
            return if between > 0.0 {
                (0.0, f64::INFINITY)
            } else {
                (1.0, 0.0)
            };
        }
        let f = (between / (r - 1.0)) / (within / (n - r));
        return (f_p(f, r - 1.0, n - r), f);
    }
    let classes = cells.first().map_or(0, |c| c.counts.len());
    let columns: Vec<usize> = (0..classes)
        .filter(|&j| cells.iter().any(|c| c.counts[j] > 0.0))
        .collect();
    let n: f64 = cells.iter().map(|c| c.n).sum();
    let df = (cells.len() as f64 - 1.0) * (columns.len() as f64 - 1.0);
    if df <= 0.0 || n <= 0.0 {
        return (1.0, 0.0);
    }
    let mut statistic = 0.0;
    for cell in cells {
        for &j in &columns {
            let column: f64 = cells.iter().map(|c| c.counts[j]).sum();
            let expected = cell.n * column / n;
            let observed = cell.counts[j];
            statistic += match chi_square {
                ChiSquare::Pearson => (observed - expected).powi(2) / expected,
                ChiSquare::LikelihoodRatio if observed > 0.0 => {
                    2.0 * observed * (observed / expected).ln()
                }
                ChiSquare::LikelihoodRatio => 0.0,
            };
        }
    }
    (chi_square_p(statistic, df), statistic)
}

// ------------------------------------------------------------------------------------------
// Merging

/// A group of a predictor's categories (the missing category is `categories`).
#[derive(Debug, Clone)]
struct Group {
    categories: Vec<usize>,
    cell: Cell,
}

struct Merged {
    groups: Vec<Group>,
    adjusted_p: f64,
}

/// Merges the predictor's categories at a node and returns the groups with the adjusted p-value
/// of the split they define, or `None` when fewer than two groups remain.
fn merge(
    data: &Data,
    settings: &ChaidSettings,
    predictor: &Predictor,
    rows: &[usize],
    min_child: f64,
) -> Option<Merged> {
    let classes = match &data.target {
        Target::Classes { labels, .. } => labels.len(),
        Target::Numbers(_) => 0,
    };
    let regression = data.is_regression();
    let mut cells = vec![Cell::new(classes); predictor.categories + 1];
    for &row in rows {
        if crate::check_cancelled().is_err() {
            return None;
        }
        let code = predictor.codes[row].unwrap_or(predictor.categories);
        cells[code].add_row(&data.target, row);
    }
    let mut groups: Vec<Group> = (0..predictor.categories)
        .filter(|&c| cells[c].n > 0.0)
        .map(|c| Group {
            categories: vec![c],
            cell: cells[c].clone(),
        })
        .collect();
    let missing = (cells[predictor.categories].n > 0.0).then(|| Group {
        categories: vec![predictor.categories],
        cell: cells[predictor.categories].clone(),
    });
    // A nominal predictor's missing category is one more category.
    let floating = if predictor.ordered {
        missing
    } else {
        groups.extend(missing);
        None
    };
    let initial = groups.len() + usize::from(floating.is_some());
    let pair_p =
        |a: &Group, b: &Group| p_value(&[&a.cell, &b.cell], regression, settings.chi_square).0;
    let full_p = |groups: &[Group]| {
        let cells: Vec<&Cell> = groups.iter().map(|g| &g.cell).collect();
        p_value(&cells, regression, settings.chi_square).0
    };

    match settings.method {
        Method::Chaid => {
            while groups.len() > 2 {
                if crate::check_cancelled().is_err() {
                    return None;
                }
                let Some((i, j, p)) = most_similar(&groups, predictor.ordered, &pair_p) else {
                    break;
                };
                if p <= settings.merge_alpha {
                    break;
                }
                merge_pair(&mut groups, i, j);
            }
        }
        Method::Exhaustive => {
            // Merge down to two groups and keep the set with the smallest p-value.
            let mut best = (full_p(&groups), groups.clone());
            while groups.len() > 2 {
                if crate::check_cancelled().is_err() {
                    return None;
                }
                let Some((i, j, _)) = most_similar(&groups, predictor.ordered, &pair_p) else {
                    break;
                };
                merge_pair(&mut groups, i, j);
                let p = full_p(&groups);
                if p < best.0 {
                    best = (p, groups.clone());
                }
            }
            groups = best.1;
        }
    }

    // Groups below the minimum child size join their most similar neighbour.
    while groups.len() > 1 {
        if crate::check_cancelled().is_err() {
            return None;
        }
        let Some(small) = groups.iter().position(|g| g.cell.n < min_child) else {
            break;
        };
        let partner = (0..groups.len())
            .filter(|&k| k != small && (!predictor.ordered || k + 1 == small || small + 1 == k))
            .max_by(|&a, &b| {
                pair_p(&groups[small], &groups[a])
                    .partial_cmp(&pair_p(&groups[small], &groups[b]))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })?;
        merge_pair(&mut groups, small.min(partner), small.max(partner));
    }

    // GAP: the floating missing category joins its most similar group when that gives the
    // smaller p-value, else it stays a group of its own.
    if let Some(missing) = floating {
        let mut alone = groups.clone();
        alone.push(missing.clone());
        let joined = (0..groups.len())
            .max_by(|&a, &b| {
                pair_p(&missing, &groups[a])
                    .partial_cmp(&pair_p(&missing, &groups[b]))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|k| {
                let mut with = groups.clone();
                with[k].categories.push(missing.categories[0]);
                with[k].cell.merge(&missing.cell);
                with
            });
        groups = match joined {
            Some(with) if with.len() >= 2 && full_p(&with) <= full_p(&alone) => with,
            _ => alone,
        };
    }

    let r = groups.len();
    if r < 2 {
        return None;
    }
    let p = full_p(&groups);
    let multiplier = if !settings.bonferroni {
        1.0
    } else if !predictor.ordered {
        bonferroni_nominal(initial, r)
    } else if groups
        .iter()
        .any(|g| g.categories.contains(&predictor.categories))
    {
        bonferroni_ordinal_missing(initial, r)
    } else {
        bonferroni_ordinal(initial, r)
    };
    Some(Merged {
        groups,
        adjusted_p: p * multiplier,
    })
}

/// The eligible pair with the largest p-value (adjacent groups only for ordered predictors);
/// the first such pair on ties.
fn most_similar(
    groups: &[Group],
    ordered: bool,
    pair_p: &impl Fn(&Group, &Group) -> f64,
) -> Option<(usize, usize, f64)> {
    let mut best: Option<(usize, usize, f64)> = None;
    for i in 0..groups.len() {
        if crate::check_cancelled().is_err() {
            return None;
        }
        let partners: Vec<usize> = if ordered {
            (i + 1..groups.len().min(i + 2)).collect()
        } else {
            (i + 1..groups.len()).collect()
        };
        for j in partners {
            let p = pair_p(&groups[i], &groups[j]);
            if best.is_none_or(|(_, _, b)| p > b) {
                best = Some((i, j, p));
            }
        }
    }
    best
}

fn merge_pair(groups: &mut Vec<Group>, i: usize, j: usize) {
    let other = groups.remove(j);
    groups[i].categories.extend(other.categories);
    groups[i].categories.sort_unstable();
    groups[i].cell.merge(&other.cell);
}

// ------------------------------------------------------------------------------------------
// Growing

struct Split {
    predictor: usize,
    groups: Vec<Group>,
    /// Indices of the children in the tree's node list, one per group.
    children: Vec<usize>,
    default_child: usize,
}

struct GrownNode {
    id: usize,
    rows: Vec<usize>,
    split: Option<Split>,
}

fn grow(data: &Data, settings: &ChaidSettings) -> Result<Vec<GrownNode>, AlgoError> {
    crate::check_cancelled()?;
    let total = data.rows.len();
    let min_parent = settings.min_parent.records(total);
    let min_child = settings.min_child.records(total);
    let mut nodes = vec![GrownNode {
        id: 0,
        rows: data.rows.clone(),
        split: None,
    }];
    let mut queue = VecDeque::from([(0usize, 0usize)]);
    while let Some((index, depth)) = queue.pop_front() {
        crate::check_cancelled()?;
        let rows = std::mem::take(&mut nodes[index].rows);
        let candidate = if depth >= settings.max_depth
            || is_pure(data, &rows)
            || (rows.len() as f64) < min_parent
        {
            None
        } else {
            best_split(data, settings, &rows, min_child)
        };
        if let Some((predictor, merged)) = candidate {
            let p = &data.predictors[predictor];
            let mut children = Vec::new();
            for group in &merged.groups {
                let child_rows: Vec<usize> = rows
                    .iter()
                    .copied()
                    .filter(|&r| {
                        group
                            .categories
                            .contains(&p.codes[r].unwrap_or(p.categories))
                    })
                    .collect();
                children.push(nodes.len());
                nodes.push(GrownNode {
                    id: nodes.len(),
                    rows: child_rows,
                    split: None,
                });
            }
            let sizes: Vec<usize> = children.iter().map(|&c| nodes[c].rows.len()).collect();
            let largest = sizes.iter().copied().max().unwrap_or(0);
            let default_child = sizes.iter().position(|&s| s == largest).unwrap_or(0);
            for &child in &children {
                queue.push_back((child, depth + 1));
            }
            nodes[index].split = Some(Split {
                predictor,
                groups: merged.groups,
                children,
                default_child,
            });
        }
        nodes[index].rows = rows;
    }
    Ok(nodes)
}

/// The predictor with the smallest adjusted p-value, when it is at most the split alpha.
fn best_split(
    data: &Data,
    settings: &ChaidSettings,
    rows: &[usize],
    min_child: f64,
) -> Option<(usize, Merged)> {
    let mut best: Option<(usize, Merged)> = None;
    for (index, predictor) in data.predictors.iter().enumerate() {
        if crate::check_cancelled().is_err() {
            return None;
        }
        if let Some(merged) = merge(data, settings, predictor, rows, min_child)
            && best
                .as_ref()
                .is_none_or(|(_, b)| merged.adjusted_p < b.adjusted_p)
        {
            best = Some((index, merged));
        }
    }
    best.filter(|(_, m)| m.adjusted_p <= settings.split_alpha)
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
// PMML

fn pmml_node(
    nodes: &[GrownNode],
    index: usize,
    data: &Data,
    predicate: Predicate,
    analytic: bool,
) -> Node {
    let node = &nodes[index];
    let (score, mut distribution) = summary(data, &node.rows);
    if analytic {
        if data.is_regression() {
            distribution.clear();
        } else {
            let denominator = node.rows.len() as f64 + distribution.len() as f64;
            for item in &mut distribution {
                item.confidence = Some((item.record_count + 1.0) / denominator);
            }
        }
    }
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
        let predictor = &data.predictors[split.predictor];
        if !analytic {
            out.default_child = Some(nodes[split.children[split.default_child]].id.to_string());
        }
        // Upper bounds of ordered groups: the bound of their last non-missing category.
        let uppers: Vec<f64> = split
            .groups
            .iter()
            .map(|group| {
                group
                    .categories
                    .iter()
                    .filter(|&&c| c < predictor.categories)
                    .max()
                    .and_then(|&c| predictor.uppers.get(c).copied())
                    .unwrap_or(f64::NAN)
            })
            .collect();
        out.children = split
            .groups
            .iter()
            .enumerate()
            .map(|(k, group)| {
                if analytic {
                    return pmml_node(
                        nodes,
                        split.children[k],
                        data,
                        analytic_predicate(predictor, group),
                        true,
                    );
                }
                let piece = group_predicate(predictor, group, k, &uppers, split.groups.len());
                let predicate = Predicate::Compound {
                    operator: BooleanOperator::Surrogate,
                    predicates: vec![
                        piece,
                        if k == split.default_child {
                            Predicate::True
                        } else {
                            Predicate::False
                        },
                    ],
                };
                pmml_node(nodes, split.children[k], data, predicate, false)
            })
            .collect();
    }
    out
}

/// AS emits exact category groups without surrogate defaults. Single edge bins extend
/// outwards; multi-bin groups retain both endpoints, including unobserved bins between them.
fn analytic_predicate(predictor: &Predictor, group: &Group) -> Predicate {
    let simple = |operator, value| Predicate::Simple {
        field: predictor.name.clone(),
        operator,
        value: Some(value),
    };
    if predictor.ordered {
        let bounds: Vec<f64> = group
            .categories
            .iter()
            .filter_map(|&c| predictor.uppers.get(c).copied())
            .collect();
        let low = bounds.first().copied().unwrap_or(0.0);
        let high = bounds.last().copied().unwrap_or(low);
        if low == high {
            let operator = if predictor.uppers.first() == Some(&low) {
                Operator::LessOrEqual
            } else if predictor.uppers.last() == Some(&high) {
                Operator::GreaterOrEqual
            } else {
                Operator::Equal
            };
            simple(operator, split_text(low))
        } else {
            Predicate::Compound {
                operator: BooleanOperator::And,
                predicates: vec![
                    simple(Operator::GreaterOrEqual, split_text(low)),
                    simple(Operator::LessOrEqual, split_text(high)),
                ],
            }
        }
    } else {
        let mut predicates: Vec<_> = group
            .categories
            .iter()
            .filter_map(|&c| predictor.labels.get(c))
            .map(|label| simple(Operator::Equal, label.clone()))
            .collect();
        if predicates.len() == 1 {
            predicates.pop().unwrap_or(Predicate::False)
        } else {
            Predicate::Compound {
                operator: BooleanOperator::Or,
                predicates,
            }
        }
    }
}

fn group_predicate(
    predictor: &Predictor,
    group: &Group,
    k: usize,
    uppers: &[f64],
    groups: usize,
) -> Predicate {
    let field = predictor.name.clone();
    let has_missing = group.categories.contains(&predictor.categories);
    let missing = || Predicate::Simple {
        field: field.clone(),
        operator: Operator::IsMissing,
        value: None,
    };
    if predictor.ordered {
        let bound = |operator: Operator, x: f64| Predicate::Simple {
            field: field.clone(),
            operator,
            value: Some(split_text(x)),
        };
        let only_missing = group.categories.iter().all(|&c| c == predictor.categories);
        if only_missing {
            return missing();
        }
        // Ordered groups are listed low to high; bounds skip a missing-only group.
        let lower = (0..k).rev().map(|i| uppers[i]).find(|x| x.is_finite());
        let upper = (k + 1 < groups && uppers[k].is_finite()).then_some(uppers[k]);
        let last = (k + 1..groups).all(|i| !uppers[i].is_finite());
        let predicates = match (lower, upper.filter(|_| !last)) {
            (None, Some(u)) => vec![Predicate::True, bound(Operator::LessOrEqual, u)],
            (Some(l), Some(u)) => vec![
                bound(Operator::GreaterThan, l),
                bound(Operator::LessOrEqual, u),
            ],
            (Some(l), None) => vec![bound(Operator::GreaterThan, l), Predicate::True],
            (None, None) => vec![Predicate::True, Predicate::True],
        };
        let range = Predicate::Compound {
            operator: BooleanOperator::And,
            predicates,
        };
        if has_missing {
            Predicate::Compound {
                operator: BooleanOperator::Or,
                predicates: vec![range, missing()],
            }
        } else {
            range
        }
    } else {
        let mut predicates: Vec<Predicate> = group
            .categories
            .iter()
            .filter(|&&c| c < predictor.categories)
            .map(|&c| Predicate::Simple {
                field: field.clone(),
                operator: Operator::Equal,
                value: Some(predictor.labels[c].clone()),
            })
            .collect();
        if has_missing {
            predicates.push(missing());
        }
        if predicates.len() < 2 {
            predicates.push(Predicate::False);
        }
        Predicate::Compound {
            operator: BooleanOperator::Or,
            predicates,
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::bin_cuts;

    #[test]
    fn values_bin_by_the_share_below_them() {
        // Twenty values: each pair fills a tenth.
        let values: Vec<f64> = (1..=20).map(f64::from).collect();
        assert_eq!(
            bin_cuts(values),
            [2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 18.0, 20.0]
        );
        // Ten values: decile positions 1.1, 2.2, ..., 9.9; every value gets its own bin.
        assert_eq!(
            bin_cuts((1..=10).map(f64::from).collect()),
            (1..=10).map(f64::from).collect::<Vec<_>>()
        );
        // Ties stay in one bin.
        assert_eq!(bin_cuts(vec![1.0; 5]), [1.0]);
    }
}
