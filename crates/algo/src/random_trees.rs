//! Random Trees (Algorithms Guide, "Random Trees Modeling Algorithms"): an ensemble of
//! CART-like trees, each grown on a bootstrap sample, with a random predictor subset considered at
//! every split, no pruning, and an accuracy-based stop rule.
//!
//! **Statistical parity** (`docs/statistical-parity.md`, rules 1, 2 and 5): Modeler draws the
//! bootstrap frequencies and predictor subsets from an undisclosed generator seeded by an
//! undisclosed hash of the tree and node ids. We use the documented distributions with our own
//! seedable generator (`crate::common::Rng`, SplitMix64), so the model is compared by accuracy or
//! RMSE and by the number of trees kept, never bit for bit (`crates/exec/tests/random_trees.rs`).
//! The seed is the node's `_seed` setting (default 1); Modeler has no such setting, and the PMML
//! stores no counts with which the draws could be recovered. That test trains 30 seeds for each of
//! the twelve Random Trees oracles: in the run of 2026-10-20 Modeler's metric and its nine trees
//! lay inside our seed range widened by 10 % of its width in every case, and Modeler ranked 15.75
//! of 30 on the metric and 11.46 of 30 on the tree count on average (acceptance is 6 to 24).
//!
//! Documented pieces that are implemented:
//! - Filtering (guide 5.1): identity and constant fields, more than `_maxPCTmissing` (70%) missing,
//!   one category above `_excludeSingleCatPCT` (95%), more than `_maxCategorynum` (49) categories,
//!   continuous fields with |CV| below `_minFieldVariation` (0.05), and date/time fields are
//!   excluded. Rows with a missing target or with every predictor missing are ignored (guide 5.2).
//! - Missing predictors are replaced by the lower median (continuous) or the mode (categorical,
//!   first category on ties, the flag's true value before its false value). The replacements are
//!   written to the PMML `MiningField`s, so scoring imputes the same values. A continuous target
//!   has no replacement. These values match the eight oracle models
//!   (`models_as/rt_*`, `models_next/rt_*`: `spend` 40, `x1` 26, `x2` 16.5, ...).
//! - Bootstrap (guide 6.1): each case's bootstrap frequency is an independent
//!   `Binomial(N, 1/N)` draw, `N` the number of training cases. The counts are not rescaled to
//!   sum to `N`.
//! - Splits (guide 6.5, 6.6): weighted Gini for a categorical target and weighted least squares
//!   deviation for a continuous one. Numeric thresholds are observed values (`x <= t`), because
//!   the in-memory path checks every value (guide 6.6). Categorical splits enumerate the subsets
//!   of the categories present in the node, up to 16 (the same exhaustive search as C&R Tree);
//!   above that Modeler uses Chou's algorithm, which we do not implement (`GAP:`). Ordinal
//!   predictors split between consecutive declared categories and are written as category sets,
//!   because our scorer does not evaluate Modeler's numeric encoding of string ordinals.
//! - Stopping (guide 6.7): a pure node, no split with both children at least
//!   `_minChildnodeSize`, depth `_maxDepth`, and `_maxNodenum` stop a tree.
//! - Accuracy stop (guide 6.7 rule 6, `_useStopRuleForAccuracy`): after the first tree, ten trees
//!   are evaluated on the OOB testing cases (guide 6.3 and 7.1: a case is scored only by the trees
//!   for which it is OOB). Classification uses their vote, regression their mean, and the measure
//!   is accuracy or R-square. If the best prefix improves on the baseline by more than 1%, it
//!   becomes the new baseline; otherwise the ensemble stops. The tree that closes a window is not
//!   kept: the ensemble is the window's block without its last tree. Every oracle keeps 9 trees
//!   (with 100 or 10 trees allowed, flag and continuous targets) even when the tenth ties; the
//!   guide's "ensemble with the best accuracy" read as the best prefix would keep a single tree
//!   whenever OOB noise makes the first tree the best, which no oracle shows. Known bias: Modeler
//!   stops after the first window in all 12 oracles, ours in about a third of the seeds (the
//!   tree-count rank of Modeler's 9 is 6.75 of 30 on average). The first tree's OOB accuracy,
//!   our baseline, is noisy, so a window often improves on it by more than 1%; how Modeler
//!   measures the baseline is not documented.
//! - Scoring (guide 8.2): majority vote for classification, arithmetic mean for regression.
//!   Modeler's PMML stores each tree as a `MiningModel` segment whose leaf score is the tree's
//!   assignment; we write the same representation.
//!
//! Not implemented: analysis/frequency weights, imbalance classification, weighted predictor
//! sampling, under-bagging (`_samplesize`), importance-bias correction (no oracle sets it), Chou
//! partitioning, and the interactive builder. The caller rejects those settings as `GAP:`. The
//! PMML does not include predictor importance (Modeler keeps it outside the exported model).

use crate::cart::{Impurity, class_improvement};
use crate::common::{Rng, number_text, split_text};
use crate::{AlgoError, Dataset};
use dms_pmml::{
    DataField, MiningField, MiningFunction, MiningModel, MiningSegment, MissingValueStrategy,
    Model, ModelHeader, NoTrueChildStrategy, Node, Pmml, Predicate, ScoreDistribution, TreeModel,
    Usage,
};
use dms_types::{Measurement, Storage, Table, Value, ValueSpec};
use std::collections::VecDeque;

/// Settings of a Random Trees build node; the defaults are Modeler's.
#[derive(Debug, Clone, PartialEq)]
pub struct RandomTreesSettings {
    /// `_numOfModels` (default 100): the largest number of trees.
    pub max_trees: usize,
    /// `_maxDepth` (default 10).
    pub max_depth: usize,
    /// `_minChildnodeSize` (default 5): the minimum child size as a percentage of the root
    /// node's distinct training cases (guide 6.6). The regression oracles have 9-15 leaves on
    /// 48 rows with this setting of 5, so the minimum is about 1.6 (5 % of the ~32 distinct cases
    /// in a bootstrap sample), neither the weighted count nor an absolute five records.
    pub min_child: f64,
    /// `_maxNodenum` (default 10000).
    pub max_nodes: usize,
    /// `_useStopRuleForAccuracy` (default true).
    pub accuracy_stop: bool,
    /// Relative accuracy improvement that continues the accuracy stop's window (1%).
    pub accuracy_delta: f64,
    /// Trees evaluated per accuracy window, the first tree included (10).
    pub accuracy_trials: usize,
    /// `_maxPCTmissing` (default 70).
    pub max_missing_percent: f64,
    /// `_excludeSingleCatPCT` (default 95).
    pub single_category_percent: f64,
    /// `_maxCategorynum` (default 49).
    pub max_categories: usize,
    /// `_minFieldVariation` (default 0.05).
    pub min_cv: f64,
    /// `_max_num_predictors` (default 10000).
    pub max_predictors: usize,
    /// `_numOfBins` (default 10): the guide's equal-frequency tiles, not used by the in-memory
    /// path because it checks every observed value. Other values have no oracle (`GAP:`).
    pub num_bins: usize,
    /// Our seed (default 1; Modeler's generator and hash are undisclosed).
    pub seed: u64,
}

impl Default for RandomTreesSettings {
    fn default() -> Self {
        RandomTreesSettings {
            max_trees: 100,
            max_depth: 10,
            min_child: 5.0,
            max_nodes: 10_000,
            accuracy_stop: true,
            accuracy_delta: 0.01,
            accuracy_trials: 10,
            max_missing_percent: 70.0,
            single_category_percent: 95.0,
            max_categories: 49,
            min_cv: 0.05,
            max_predictors: 10_000,
            num_bins: 10,
            seed: 1,
        }
    }
}

/// Trains a Random Trees ensemble.
pub fn train(dataset: &Dataset<'_>, settings: &RandomTreesSettings) -> Result<Pmml, AlgoError> {
    validate(settings)?;
    if dataset.frequency.is_some() {
        return Err(AlgoError::Unsupported(
            "Random Trees with a frequency field".into(),
        ));
    }
    if dataset.inputs.len() > settings.max_predictors {
        return Err(AlgoError::Unsupported(format!(
            "Random Trees with more than _max_num_predictors={} predictors",
            settings.max_predictors
        )));
    }
    let rows = dataset.table.row_count();
    if rows > 1_000_000
        || rows.saturating_mul(dataset.table.model().len()) > 16_000_000
        || rows.saturating_mul(settings.max_trees) > 100_000_000
    {
        return Err(AlgoError::Unsupported(
            "Random Trees data limit: 1000000 rows, 16000000 cells, 100000000 row-tree units"
                .into(),
        ));
    }
    let data = Data::read(dataset, settings)?;
    if data.columns.is_empty() {
        return Err(AlgoError::Data(
            "no predictor remains after Random Trees' variable filtering".into(),
        ));
    }
    let (trees, kept) = grow(&data, settings)?;
    if trees.is_empty() || kept == 0 {
        return Err(AlgoError::Data("Random Trees grew no tree".into()));
    }
    pmml(dataset, &data, &trees[..kept], settings)
}

fn validate(settings: &RandomTreesSettings) -> Result<(), AlgoError> {
    let bad = |what: &str| AlgoError::Settings(format!("Random Trees {what}"));
    if settings.max_trees == 0 || settings.max_trees > 100_000 {
        return Err(bad("_numOfModels must be from 1 to 100000"));
    }
    if settings.max_depth > 60 {
        return Err(bad("_maxDepth must be at most 60"));
    }
    if !settings.min_child.is_finite() || !(0.0..=100.0).contains(&settings.min_child) {
        return Err(bad("_minChildnodeSize must be a percentage from 0 to 100"));
    }
    if settings.max_nodes < 3 {
        return Err(bad("_maxNodenum must be at least 3"));
    }
    if !settings.accuracy_delta.is_finite() || settings.accuracy_delta < 0.0 {
        return Err(bad("the accuracy improvement threshold is not a number"));
    }
    if settings.accuracy_trials < 2 {
        return Err(bad("the accuracy window must contain at least two trees"));
    }
    for (name, value) in [
        ("_maxPCTmissing", settings.max_missing_percent),
        ("_excludeSingleCatPCT", settings.single_category_percent),
    ] {
        if !value.is_finite() || !(0.0..=100.0).contains(&value) {
            return Err(bad(&format!("{name} must be from 0 to 100")));
        }
    }
    if settings.max_categories == 0 {
        return Err(bad("_maxCategorynum must be positive"));
    }
    if !settings.min_cv.is_finite() || settings.min_cv < 0.0 {
        return Err(bad("_minFieldVariation must be a non-negative number"));
    }
    if settings.max_predictors == 0 {
        return Err(bad("_max_num_predictors must be positive"));
    }
    if settings.num_bins < 2 {
        return Err(bad("_numOfBins must be at least 2"));
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------
// Training data

/// An input field after filtering and imputation.
struct Column {
    name: String,
    kind: Kind,
    /// PMML `missingValueReplacement`.
    replacement: String,
}

enum Kind {
    /// Continuous or numeric ordinal values, split by `x <= t`.
    Numeric(Vec<f64>),
    /// Category codes into `labels` (declared order), split by subsets.
    Categorical {
        codes: Vec<usize>,
        labels: Vec<String>,
        /// Ordinal: only prefixes of the declared order are candidate splits.
        ordered: bool,
    },
}

enum Target {
    Classes {
        codes: Vec<usize>,
        labels: Vec<String>,
    },
    Numbers(Vec<f64>),
}

struct Data {
    columns: Vec<Column>,
    target: Target,
    /// Empirical class priors (classification only).
    priors: Vec<f64>,
    /// PMML replacement of the target (mode; `None` for regression).
    target_replacement: Option<String>,
    rows: usize,
}

impl Data {
    fn read(dataset: &Dataset<'_>, settings: &RandomTreesSettings) -> Result<Data, AlgoError> {
        let fields = dataset.table.model().fields();
        let table_rows = dataset.table.rows();
        let field = |position: usize| {
            fields
                .get(position)
                .ok_or_else(|| AlgoError::Data(format!("no field at position {position}")))
        };

        // The target and the raw predictor values; invalid values count as missing.
        let target_field = field(dataset.target)?;
        let target_text = |row: &[Value]| {
            row.get(dataset.target)
                .filter(|value| !target_field.is_missing(value))
                .map(|value| value.to_string())
        };
        let target_number = |row: &[Value]| {
            row.get(dataset.target)
                .filter(|value| !target_field.is_missing(value))
                .and_then(Value::as_f64)
                .filter(|x| x.is_finite())
        };
        let mut target_valid = vec![false; table_rows.len()];
        let target = if target_field.measurement == Measurement::Continuous {
            let mut values = vec![0.0; table_rows.len()];
            for (i, row) in table_rows.iter().enumerate() {
                if let Some(x) = target_number(row) {
                    values[i] = x;
                    target_valid[i] = true;
                }
            }
            Target::Numbers(values)
        } else {
            let labels = category_order(dataset.table, dataset.target);
            let mut codes = vec![0usize; table_rows.len()];
            for (i, row) in table_rows.iter().enumerate() {
                if let Some(text) = target_text(row)
                    && let Some(code) = labels.iter().position(|label| *label == text)
                {
                    codes[i] = code;
                    target_valid[i] = true;
                }
            }
            Target::Classes { codes, labels }
        };

        // Raw predictor values.
        let mut raw: Vec<RawColumn> = Vec::new();
        for &position in dataset.inputs {
            let input = field(position)?;
            let numeric = matches!(input.storage, Storage::Integer | Storage::Real);
            let continuous = input.measurement == Measurement::Continuous;
            let numeric_ordinal = input.measurement == Measurement::Ordinal && numeric;
            if matches!(
                input.storage,
                Storage::Date | Storage::Time | Storage::Timestamp
            ) {
                raw.push(RawColumn::Skipped);
                continue;
            }
            if continuous || numeric_ordinal {
                let values = table_rows
                    .iter()
                    .map(|row| {
                        row.get(position)
                            .filter(|value| !input.is_missing(value))
                            .and_then(Value::as_f64)
                            .filter(|x| x.is_finite())
                    })
                    .collect();
                raw.push(RawColumn::Numeric {
                    name: input.name.clone(),
                    values,
                    continuous,
                });
            } else {
                let labels = category_order(dataset.table, position);
                let values = table_rows
                    .iter()
                    .map(|row| {
                        row.get(position)
                            .filter(|value| !input.is_missing(value))
                            .map(|value| value.to_string())
                            .and_then(|text| labels.iter().position(|label| *label == text))
                    })
                    .collect();
                raw.push(RawColumn::Categorical {
                    name: input.name.clone(),
                    labels,
                    values,
                    ordered: input.measurement == Measurement::Ordinal,
                });
            }
        }

        // Rows with a valid target and at least one predictor present.
        let mut rows = Vec::new();
        for row in 0..table_rows.len() {
            if !target_valid.get(row).copied().unwrap_or(false) {
                continue;
            }
            let any_present = raw.iter().any(|column| match column {
                RawColumn::Skipped => false,
                RawColumn::Numeric { values, .. } => values.get(row).is_some_and(Option::is_some),
                RawColumn::Categorical { values, .. } => {
                    values.get(row).is_some_and(Option::is_some)
                }
            });
            if any_present {
                rows.push(row);
            }
        }
        let n = rows.len();
        if n == 0 {
            return Err(AlgoError::Data("no records with a valid target".into()));
        }

        // Filtering (guide 5.1) and imputation values.
        let mut usable = Vec::new();
        for (index, column) in raw.iter().enumerate() {
            let keep = match column {
                RawColumn::Skipped => false,
                RawColumn::Numeric {
                    values, continuous, ..
                } => {
                    let present: Vec<f64> = rows
                        .iter()
                        .filter_map(|&row| values.get(row).copied().flatten())
                        .collect();
                    if present.is_empty() {
                        false
                    } else {
                        let missing = n.saturating_sub(present.len()) as f64 / n as f64 * 100.0;
                        if missing > settings.max_missing_percent {
                            false
                        } else {
                            let distinct = distinct_numbers(&present);
                            let constant = distinct <= 1;
                            let cv = if *continuous {
                                coefficient_of_variation(&present)
                                    .is_some_and(|cv| cv.abs() < settings.min_cv)
                            } else {
                                false
                            };
                            !constant && !cv
                        }
                    }
                }
                RawColumn::Categorical { values, labels, .. } => {
                    let mut counts = vec![0usize; labels.len()];
                    let mut present = 0usize;
                    for &row in &rows {
                        if let Some(code) = values.get(row).copied().flatten() {
                            counts[code] += 1;
                            present += 1;
                        }
                    }
                    if present == 0 {
                        false
                    } else {
                        let missing = n.saturating_sub(present) as f64 / n as f64 * 100.0;
                        let distinct = counts.iter().filter(|&&count| count > 0).count();
                        let identity = distinct == present;
                        let constant = distinct <= 1;
                        let overwhelming = counts.iter().copied().max().unwrap_or(0) as f64
                            / present as f64
                            * 100.0
                            > settings.single_category_percent;
                        let too_many = distinct > settings.max_categories;
                        !(missing > settings.max_missing_percent
                            || identity
                            || constant
                            || overwhelming
                            || too_many)
                    }
                }
            };
            if keep {
                usable.push(index);
            }
        }

        let mut columns = Vec::new();
        for index in usable {
            let column = raw
                .get(index)
                .ok_or_else(|| AlgoError::Data("Random Trees column index".into()))?;
            match column {
                RawColumn::Numeric { name, values, .. } => {
                    let present: Vec<f64> = rows
                        .iter()
                        .filter_map(|&row| values.get(row).copied().flatten())
                        .collect();
                    let replacement = lower_median(&present)
                        .ok_or_else(|| AlgoError::Data(format!("no observed value for {name}")))?;
                    let compact = rows
                        .iter()
                        .map(|&row| values.get(row).copied().flatten().unwrap_or(replacement))
                        .collect();
                    columns.push(Column {
                        name: name.clone(),
                        kind: Kind::Numeric(compact),
                        replacement: number_text(replacement),
                    });
                }
                RawColumn::Categorical {
                    name,
                    labels,
                    values,
                    ordered,
                    ..
                } => {
                    let mut counts = vec![0usize; labels.len()];
                    for &row in &rows {
                        if let Some(code) = values.get(row).copied().flatten() {
                            counts[code] += 1;
                        }
                    }
                    let mode = counts
                        .iter()
                        .enumerate()
                        .fold(None::<usize>, |best, (code, &count)| match best {
                            Some(best_code) if count <= counts[best_code] => Some(best_code),
                            _ => Some(code),
                        })
                        .unwrap_or(0);
                    let replacement = labels.get(mode).cloned().unwrap_or_default();
                    let compact = rows
                        .iter()
                        .map(|&row| values.get(row).copied().flatten().unwrap_or(mode))
                        .collect();
                    columns.push(Column {
                        name: name.clone(),
                        kind: Kind::Categorical {
                            codes: compact,
                            labels: labels.clone(),
                            ordered: *ordered,
                        },
                        replacement,
                    });
                }
                RawColumn::Skipped => {}
            }
        }

        // Compact target and priors.
        let (target, priors, target_replacement) = match target {
            Target::Classes { codes, labels } => {
                let compact: Vec<usize> = rows
                    .iter()
                    .map(|&row| codes.get(row).copied().unwrap_or(0))
                    .collect();
                let mut counts = vec![0.0; labels.len()];
                for &code in &compact {
                    if let Some(count) = counts.get_mut(code) {
                        *count += 1.0;
                    }
                }
                let total: f64 = counts.iter().sum();
                let priors = counts
                    .iter()
                    .map(|count| if total > 0.0 { count / total } else { 0.0 })
                    .collect();
                let mode = counts
                    .iter()
                    .enumerate()
                    .fold(None::<(usize, f64)>, |best, (code, &count)| match best {
                        Some((_, best_count)) if count <= best_count => best,
                        _ => Some((code, count)),
                    })
                    .map(|(code, _)| code)
                    .unwrap_or(0);
                let replacement = labels.get(mode).cloned();
                (
                    Target::Classes {
                        codes: compact,
                        labels,
                    },
                    priors,
                    replacement,
                )
            }
            Target::Numbers(values) => {
                let compact: Vec<f64> = rows
                    .iter()
                    .map(|&row| values.get(row).copied().unwrap_or(0.0))
                    .collect();
                if compact.iter().any(|value| value.abs() > 1e150) {
                    return Err(AlgoError::Data(
                        "Random Trees target values would overflow the moments".into(),
                    ));
                }
                (Target::Numbers(compact), Vec::new(), None)
            }
        };

        Ok(Data {
            columns,
            target,
            priors,
            target_replacement,
            rows: n,
        })
    }

    fn is_regression(&self) -> bool {
        matches!(self.target, Target::Numbers(_))
    }

    fn target_numbers(&self) -> Option<&[f64]> {
        match &self.target {
            Target::Numbers(values) => Some(values),
            Target::Classes { .. } => None,
        }
    }

    fn label(&self, code: usize) -> String {
        match &self.target {
            Target::Classes { labels, .. } => labels.get(code).cloned().unwrap_or_default(),
            Target::Numbers(_) => String::new(),
        }
    }
}

enum RawColumn {
    Skipped,
    Numeric {
        name: String,
        values: Vec<Option<f64>>,
        continuous: bool,
    },
    Categorical {
        name: String,
        labels: Vec<String>,
        values: Vec<Option<usize>>,
        ordered: bool,
    },
}

fn category_order(table: &Table, position: usize) -> Vec<String> {
    let Some(field) = table.model().fields().get(position) else {
        return Vec::new();
    };
    let valid = |value: &&Value| !field.is_missing(value);
    match &field.values {
        ValueSpec::Categories(values) => values.iter().filter(valid).map(text).collect(),
        ValueSpec::Flag {
            true_value,
            false_value,
        } => [true_value, false_value]
            .into_iter()
            .filter(valid)
            .map(text)
            .collect(),
        _ => {
            let mut seen: Vec<String> = Vec::new();
            for row in table.rows() {
                if let Some(value) = row.get(position).filter(valid) {
                    let value = text(value);
                    if !seen.contains(&value) {
                        seen.push(value);
                    }
                }
            }
            seen
        }
    }
}

fn text(value: &Value) -> String {
    value.to_string()
}

fn distinct_numbers(values: &[f64]) -> usize {
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted.dedup_by(|a, b| a.total_cmp(b).is_eq());
    sorted.len()
}

fn coefficient_of_variation(values: &[f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    if mean == 0.0 || !mean.is_finite() {
        return None;
    }
    let variance = values
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum::<f64>()
        / (n - 1.0);
    Some(variance.max(0.0).sqrt() / mean)
}

fn lower_median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted.get((sorted.len() - 1) / 2).copied()
}

// ------------------------------------------------------------------------------------------
// Growing

struct Case {
    row: usize,
    weight: f64,
}

struct GrownNode {
    id: usize,
    cases: Vec<Case>,
    split: Option<Split>,
}

struct Split {
    input: usize,
    rule: Rule,
    left: usize,
    right: usize,
}

#[derive(Clone, PartialEq, Debug)]
enum Rule {
    /// `x <= threshold` goes left.
    Threshold(f64),
    /// Codes that go left; every other category goes right.
    Categories { left: Vec<usize> },
}

impl Rule {
    fn left(&self, column: &Column, row: usize) -> bool {
        match (self, &column.kind) {
            (Rule::Threshold(threshold), Kind::Numeric(values)) => {
                values.get(row).is_some_and(|value| *value <= *threshold)
            }
            (Rule::Categories { left, .. }, Kind::Categorical { codes, .. }) => {
                codes.get(row).is_some_and(|code| left.contains(code))
            }
            _ => false,
        }
    }
}

struct Tree {
    nodes: Vec<GrownNode>,
    /// Bootstrap class totals (`N_w,j` of the Gini formula).
    class_totals: Vec<f64>,
}

struct Prediction {
    class: Option<usize>,
    number: f64,
}

fn grow(data: &Data, settings: &RandomTreesSettings) -> Result<(Vec<Tree>, usize), AlgoError> {
    let mut trees = Vec::new();
    let mut ensemble = Ensemble::new(data);
    let mut baseline_accuracy = f64::NEG_INFINITY;
    let mut baseline_size = 1usize;
    let mut window_accuracy = f64::NEG_INFINITY;
    for index in 0..settings.max_trees {
        let counts = bootstrap(data.rows, settings.seed, index);
        let tree = grow_tree(data, settings, index, &counts)?;
        // Guide 6.3 and 7.1: a case evaluates the ensemble only through the trees that took it
        // as an OOB (testing) case, i.e. the trees whose bootstrap frequency for it is zero.
        let testing: Vec<(usize, Prediction)> = (0..data.rows)
            .filter(|row| counts.get(*row).copied().unwrap_or(0.0) == 0.0)
            .map(|row| (row, tree.predict(data, row)))
            .collect();
        ensemble.add(&testing);
        trees.push(tree);
        let size = trees.len();
        let Some(accuracy) = ensemble.accuracy(data) else {
            continue;
        };
        if !accuracy.is_finite() {
            // No testing case: the rule cannot decide; keep the requested trees.
            continue;
        }
        if size == 1 {
            baseline_accuracy = accuracy;
            baseline_size = 1;
            window_accuracy = accuracy;
            continue;
        }
        // Guide 6.7 rule 6: `trials` trees are evaluated per window, the baseline tree included.
        // A relative improvement of the window's best accuracy above the documented 1% moves the
        // baseline and opens a new window; otherwise the growing stops.
        let closes = settings.accuracy_stop
            && size.saturating_sub(baseline_size) + 1 >= settings.accuracy_trials;
        if !closes {
            window_accuracy = window_accuracy.max(accuracy);
            continue;
        }
        if relative_improvement(window_accuracy, baseline_accuracy) > settings.accuracy_delta {
            baseline_accuracy = window_accuracy;
            baseline_size = size;
            window_accuracy = accuracy;
        } else {
            // The ensemble is the window's block without the tree that closed it. Every oracle
            // keeps 9 trees, with 100 or 10 trees allowed and with a flag, set or continuous
            // target, even where the tenth tree ties the best accuracy; a best-prefix reading
            // keeps a single tree whenever OOB noise makes the first tree the best, which no
            // oracle shows.
            return Ok((trees, size.saturating_sub(1).max(1)));
        }
    }
    let kept = trees.len();
    Ok((trees, kept))
}

fn relative_improvement(best: f64, baseline: f64) -> f64 {
    if !best.is_finite() || !baseline.is_finite() {
        return 0.0;
    }
    if baseline == 0.0 {
        if best > 0.0 { f64::INFINITY } else { 0.0 }
    } else {
        (best - baseline) / baseline.abs()
    }
}

/// Independent `Binomial(n, 1/n)` draws, one per case (guide 6.1 with `f_k = 1`, `alpha = 1`).
fn bootstrap(n: usize, seed: u64, tree: usize) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    let p = 1.0 / n as f64;
    // P(0) = (1 - p)^n; the recurrence keeps every draw in probability space without
    // materializing the distribution.
    let p0 = if n == 1 {
        0.0
    } else {
        (n as f64 * (-p).ln_1p()).exp()
    };
    let mut rng = Rng(mix(seed, tree as u64));
    (0..n)
        .map(|_| {
            let u = rng.unit();
            if p0 >= 1.0 {
                return 0.0;
            }
            let mut probability = p0;
            let mut cumulative = p0;
            let mut count = 0usize;
            // Stop at n or when the tail is exhausted (rounded to one); guard the loop.
            while u > cumulative && count < n {
                count += 1;
                let k = count as f64;
                probability *= (n as f64 - k + 1.0) / k * p / (1.0 - p);
                cumulative += probability;
                if !probability.is_finite() {
                    break;
                }
            }
            count as f64
        })
        .collect()
}

/// SplitMix64's finalizer, used to derive independent seeds from `(seed, tree)`.
fn mix(seed: u64, index: u64) -> u64 {
    let mut z = seed
        .wrapping_add(index.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn grow_tree(
    data: &Data,
    settings: &RandomTreesSettings,
    tree_index: usize,
    counts: &[f64],
) -> Result<Tree, AlgoError> {
    let cases: Vec<Case> = (0..data.rows)
        .filter_map(|row| {
            let weight = counts.get(row).copied().unwrap_or(0.0);
            (weight > 0.0).then_some(Case { row, weight })
        })
        .collect();
    // `_minChildnodeSize` is a percentage of the root node (guide 6.7): the root's distinct
    // training cases (`|ℓ(q,r)|` of guide 6.6) set the minimum for every split of this tree.
    // The regression oracles have 9-15 leaves on 48 rows with the setting 5, so the minimum is
    // about 1.6 (5 % of the ~32 distinct cases in a bootstrap sample), not the weighted count.
    let min_child = settings.min_child / 100.0 * cases.len() as f64;
    let class_totals = match &data.target {
        Target::Classes { codes, labels } => {
            let mut totals = vec![0.0; labels.len()];
            for case in &cases {
                if let Some(code) = codes.get(case.row)
                    && let Some(total) = totals.get_mut(*code)
                {
                    *total += case.weight;
                }
            }
            totals
        }
        Target::Numbers(_) => Vec::new(),
    };
    let mut nodes = vec![GrownNode {
        id: 1,
        cases,
        split: None,
    }];
    let mut queue = VecDeque::from([(0usize, 0usize)]);
    while let Some((index, depth)) = queue.pop_front() {
        if depth >= settings.max_depth || nodes.len().saturating_add(2) > settings.max_nodes {
            continue;
        }
        let cases = std::mem::take(
            &mut nodes
                .get_mut(index)
                .ok_or_else(|| AlgoError::Data("Random Trees node index".into()))?
                .cases,
        );
        if cases.is_empty() || is_pure(data, &cases) {
            nodes[index].cases = cases;
            continue;
        }
        let selected = select_predictors(
            data,
            settings.seed,
            tree_index,
            nodes[index].id,
            data.is_regression(),
        );
        let Some((input, rule)) = best_split(data, min_child, &cases, &selected, &class_totals)
        else {
            nodes[index].cases = cases;
            continue;
        };
        let mut left_cases = Vec::new();
        let mut right_cases = Vec::new();
        for case in cases {
            if rule.left(&data.columns[input], case.row) {
                left_cases.push(case);
            } else {
                right_cases.push(case);
            }
        }
        if left_cases.is_empty() || right_cases.is_empty() {
            nodes[index].cases = Vec::new();
            nodes[index].cases.extend(left_cases);
            nodes[index].cases.extend(right_cases);
            continue;
        }
        let left_weight: f64 = left_cases.iter().map(|case| case.weight).sum();
        let right_weight: f64 = right_cases.iter().map(|case| case.weight).sum();
        if left_weight < min_child || right_weight < min_child {
            nodes[index].cases = Vec::new();
            nodes[index].cases.extend(left_cases);
            nodes[index].cases.extend(right_cases);
            continue;
        }
        let id = nodes[index].id;
        let left = nodes.len();
        let right = nodes.len() + 1;
        nodes.push(GrownNode {
            id: id
                .checked_mul(2)
                .ok_or_else(|| AlgoError::Data("Random Trees node id overflow".into()))?,
            cases: left_cases,
            split: None,
        });
        nodes.push(GrownNode {
            id: id
                .checked_mul(2)
                .and_then(|id| id.checked_add(1))
                .ok_or_else(|| AlgoError::Data("Random Trees node id overflow".into()))?,
            cases: right_cases,
            split: None,
        });
        nodes[index].split = Some(Split {
            input,
            rule,
            left,
            right,
        });
        queue.push_back((left, depth + 1));
        queue.push_back((right, depth + 1));
    }
    Ok(Tree {
        nodes,
        class_totals,
    })
}

fn is_pure(data: &Data, cases: &[Case]) -> bool {
    let Some(first) = cases.first() else {
        return true;
    };
    match &data.target {
        Target::Classes { codes, .. } => {
            let code = codes.get(first.row).copied();
            cases
                .iter()
                .all(|case| codes.get(case.row).copied() == code)
        }
        Target::Numbers(values) => {
            let value = values.get(first.row).copied();
            cases
                .iter()
                .all(|case| values.get(case.row).copied() == value)
        }
    }
}

/// The random predictor subset of a node (guide 6.2): `floor(sqrt(M))` for classification and
/// `floor(M/3)` for regression, uniformly without replacement. The seed is a hash of the tree and
/// node ids, so the subset does not depend on the requested number of trees.
fn select_predictors(
    data: &Data,
    seed: u64,
    tree: usize,
    node: usize,
    regression: bool,
) -> Vec<usize> {
    let m = data.columns.len();
    if m == 0 {
        return Vec::new();
    }
    let count = if regression {
        m / 3
    } else {
        (m as f64).sqrt().floor() as usize
    }
    .clamp(1, m);
    let mut indices: Vec<usize> = (0..m).collect();
    let mut rng = Rng(mix(mix(seed, tree as u64), node as u64));
    for i in 0..count {
        let remaining = m - i;
        if remaining == 0 {
            break;
        }
        let offset = (rng.next_u64() % remaining as u64) as usize;
        indices.swap(i, i + offset);
    }
    indices.truncate(count);
    indices.sort_unstable();
    indices
}

/// The best split among the selected predictors, if one satisfies the minimum child size.
fn best_split(
    data: &Data,
    min_child: f64,
    cases: &[Case],
    selected: &[usize],
    class_totals: &[f64],
) -> Option<(usize, Rule)> {
    let mut best: Option<(f64, usize, Rule)> = None;
    for &input in selected {
        let column = data.columns.get(input)?;
        let candidate = match &column.kind {
            Kind::Numeric(values) => numeric_split(data, min_child, values, cases, class_totals),
            Kind::Categorical {
                codes,
                labels,
                ordered,
            } => categorical_split(
                data,
                min_child,
                codes,
                labels,
                *ordered,
                cases,
                class_totals,
            ),
        };
        if let Some((improvement, rule)) = candidate
            && best
                .as_ref()
                .is_none_or(|(current, _, _)| improvement > current + 1e-15)
        {
            best = Some((improvement, input, rule));
        }
    }
    best.map(|(_, input, rule)| (input, rule))
}

fn numeric_split(
    data: &Data,
    min_child: f64,
    values: &[f64],
    cases: &[Case],
    class_totals: &[f64],
) -> Option<(f64, Rule)> {
    let mut sorted: Vec<&Case> = cases.iter().collect();
    sorted.sort_by(|a, b| {
        values
            .get(a.row)
            .copied()
            .unwrap_or(f64::NAN)
            .total_cmp(&values.get(b.row).copied().unwrap_or(f64::NAN))
    });
    let value = |index: usize| {
        sorted
            .get(index)
            .and_then(|case| values.get(case.row).copied())
            .unwrap_or(f64::NAN)
    };
    let mut best: Option<(f64, usize)> = None;
    match &data.target {
        Target::Classes { codes, labels } => {
            let classes = labels.len();
            let mut left = vec![0.0; classes];
            let mut right = vec![0.0; classes];
            for case in &sorted {
                if let Some(code) = codes.get(case.row)
                    && let Some(weight) = right.get_mut(*code)
                {
                    *weight += case.weight;
                }
            }
            let mut left_weight = 0.0;
            for cut in 1..sorted.len() {
                let case = sorted[cut - 1];
                if let Some(code) = codes.get(case.row) {
                    if let Some(weight) = left.get_mut(*code) {
                        *weight += case.weight;
                    }
                    if let Some(weight) = right.get_mut(*code) {
                        *weight -= case.weight;
                    }
                }
                left_weight += case.weight;
                if value(cut) == value(cut - 1) {
                    continue;
                }
                let right_weight: f64 = right.iter().sum();
                if left_weight < min_child || right_weight < min_child {
                    continue;
                }
                let improvement =
                    class_improvement(Impurity::Gini, &left, &right, &data.priors, class_totals);
                if best
                    .as_ref()
                    .is_none_or(|(current, _)| improvement > current + 1e-15)
                {
                    best = Some((improvement, cut));
                }
            }
            let (improvement, cut) = best.filter(|(improvement, _)| improvement.is_finite())?;
            Some((improvement, Rule::Threshold(value(cut - 1))))
        }
        Target::Numbers(ys) => {
            let mut total = WeightedMoments::default();
            for case in &sorted {
                if let Some(y) = ys.get(case.row) {
                    total.add(*y, case.weight);
                }
            }
            let mut left = WeightedMoments::default();
            for cut in 1..sorted.len() {
                let case = sorted[cut - 1];
                if let Some(y) = ys.get(case.row) {
                    left.add(*y, case.weight);
                }
                if value(cut) == value(cut - 1) {
                    continue;
                }
                if left.n < min_child || total.n - left.n < min_child {
                    continue;
                }
                let improvement = lsd_improvement(&total, &left);
                if best
                    .as_ref()
                    .is_none_or(|(current, _)| improvement > current + 1e-15)
                {
                    best = Some((improvement, cut));
                }
            }
            let (improvement, cut) = best?;
            Some((improvement, Rule::Threshold(value(cut - 1))))
        }
    }
}

fn categorical_split(
    data: &Data,
    min_child: f64,
    codes: &[usize],
    labels: &[String],
    ordered: bool,
    cases: &[Case],
    class_totals: &[f64],
) -> Option<(f64, Rule)> {
    let mut present = vec![false; labels.len()];
    for case in cases {
        if let Some(code) = codes.get(case.row)
            && let Some(slot) = present.get_mut(*code)
        {
            *slot = true;
        }
    }
    let present: Vec<usize> = present
        .iter()
        .enumerate()
        .filter_map(|(code, &seen)| seen.then_some(code))
        .collect();
    if present.len() < 2 {
        return None;
    }
    // GAP: Chou's large-category partitioner is not implemented; the oracle fields have at most
    // four categories and the exhaustive search below is the documented one for small fields.
    if present.len() > 16 {
        return None;
    }
    let mut best: Option<(f64, Vec<usize>)> = None;
    let mut consider = |improvement: f64, set: Vec<usize>| {
        if best
            .as_ref()
            .is_none_or(|(current, _)| improvement > current + 1e-15)
        {
            best = Some((improvement, set));
        }
    };
    match &data.target {
        Target::Classes {
            codes: classes,
            labels: classes_labels,
        } => {
            let k = classes_labels.len();
            let mut by_category = vec![vec![0.0; k]; labels.len()];
            for case in cases {
                if let Some(code) = codes.get(case.row)
                    && let Some(target) = classes.get(case.row)
                    && let Some(counts) = by_category.get_mut(*code)
                    && let Some(weight) = counts.get_mut(*target)
                {
                    *weight += case.weight;
                }
            }
            let mut parent = vec![0.0; k];
            for counts in &by_category {
                for (total, count) in parent.iter_mut().zip(counts) {
                    *total += count;
                }
            }
            let subsets = candidate_subsets(&present, ordered);
            for set in subsets {
                let mut left = vec![0.0; k];
                for &code in &set {
                    if let Some(counts) = by_category.get(code) {
                        for (slot, count) in left.iter_mut().zip(counts) {
                            *slot += count;
                        }
                    }
                }
                if left.iter().sum::<f64>() < min_child
                    || parent.iter().sum::<f64>() - left.iter().sum::<f64>() < min_child
                {
                    continue;
                }
                let right: Vec<f64> = parent
                    .iter()
                    .zip(&left)
                    .map(|(total, part)| total - part)
                    .collect();
                consider(
                    class_improvement(Impurity::Gini, &left, &right, &data.priors, class_totals),
                    set,
                );
            }
            let (improvement, set) = best.filter(|(improvement, _)| improvement.is_finite())?;
            Some((improvement, categories_rule(set)))
        }
        Target::Numbers(ys) => {
            let mut by_category = vec![WeightedMoments::default(); labels.len()];
            let mut total = WeightedMoments::default();
            for case in cases {
                if let Some(code) = codes.get(case.row)
                    && let Some(y) = ys.get(case.row)
                    && let Some(moments) = by_category.get_mut(*code)
                {
                    moments.add(*y, case.weight);
                    total.add(*y, case.weight);
                }
            }
            for set in candidate_subsets(&present, ordered) {
                let mut left = WeightedMoments::default();
                for &code in &set {
                    if let Some(moments) = by_category.get(code) {
                        left.merge(moments);
                    }
                }
                if left.n < min_child || total.n - left.n < min_child {
                    continue;
                }
                consider(lsd_improvement(&total, &left), set);
            }
            let (improvement, set) = best?;
            Some((improvement, categories_rule(set)))
        }
    }
}

/// Candidate category sets: all subsets containing the first present category for nominal fields,
/// and the prefixes of the declared order for ordinal ones.
fn candidate_subsets(present: &[usize], ordered: bool) -> Vec<Vec<usize>> {
    if ordered {
        (1..present.len())
            .map(|cut| present.iter().copied().take(cut).collect())
            .collect()
    } else {
        let k = present.len();
        if k > 16 {
            return Vec::new();
        }
        let end = 1u32.checked_shl((k - 1) as u32).unwrap_or(0);
        (0..end.saturating_sub(1))
            .map(|mask| {
                std::iter::once(present[0])
                    .chain(
                        (0..k - 1)
                            .filter(|bit| mask & (1 << bit) != 0)
                            .map(|bit| present[bit + 1]),
                    )
                    .collect()
            })
            .collect()
    }
}

fn categories_rule(set: Vec<usize>) -> Rule {
    Rule::Categories { left: set }
}

#[derive(Debug, Clone, Copy, Default)]
struct WeightedMoments {
    n: f64,
    sum: f64,
    squares: f64,
}

impl WeightedMoments {
    fn add(&mut self, y: f64, weight: f64) {
        self.n += weight;
        self.sum += weight * y;
        self.squares += weight * y * y;
    }

    fn merge(&mut self, other: &WeightedMoments) {
        self.n += other.n;
        self.sum += other.sum;
        self.squares += other.squares;
    }

    fn sse(&self) -> f64 {
        if self.n <= 0.0 {
            0.0
        } else {
            (self.squares - self.sum * self.sum / self.n).max(0.0)
        }
    }
}

fn lsd_improvement(total: &WeightedMoments, left: &WeightedMoments) -> f64 {
    let right = WeightedMoments {
        n: total.n - left.n,
        sum: total.sum - left.sum,
        squares: total.squares - left.squares,
    };
    if total.n <= 0.0 {
        return 0.0;
    }
    (total.sse() - left.sse() - right.sse()) / total.n
}

impl Tree {
    fn predict(&self, data: &Data, row: usize) -> Prediction {
        let mut index = 0usize;
        loop {
            let Some(node) = self.nodes.get(index) else {
                return Prediction {
                    class: None,
                    number: f64::NAN,
                };
            };
            let Some(split) = &node.split else {
                return self.leaf(data, node);
            };
            let column = data.columns.get(split.input);
            let goes_left = column.is_some_and(|column| split.rule.left(column, row));
            let next = if goes_left { split.left } else { split.right };
            if next == index || next >= self.nodes.len() {
                return self.leaf(data, node);
            }
            index = next;
        }
    }

    fn leaf(&self, data: &Data, node: &GrownNode) -> Prediction {
        match &data.target {
            Target::Classes { codes, labels } => {
                let mut counts = vec![0.0; labels.len()];
                for case in &node.cases {
                    if let Some(code) = codes.get(case.row)
                        && let Some(count) = counts.get_mut(*code)
                    {
                        *count += case.weight;
                    }
                }
                let mut best = 0usize;
                let mut best_weight = f64::NEG_INFINITY;
                for (code, &count) in counts.iter().enumerate() {
                    let total = self.class_totals.get(code).copied().unwrap_or(0.0);
                    let weight = if total > 0.0 {
                        data.priors.get(code).copied().unwrap_or(0.0) * count / total
                    } else {
                        0.0
                    };
                    if weight > best_weight + 1e-15
                        || ((weight - best_weight).abs() <= 1e-15
                            && count > counts.get(best).copied().unwrap_or(0.0))
                    {
                        best = code;
                        best_weight = weight;
                    }
                }
                Prediction {
                    class: Some(best),
                    number: f64::NAN,
                }
            }
            Target::Numbers(values) => {
                let mut moments = WeightedMoments::default();
                for case in &node.cases {
                    if let Some(y) = values.get(case.row) {
                        moments.add(*y, case.weight);
                    }
                }
                Prediction {
                    class: None,
                    number: if moments.n > 0.0 {
                        moments.sum / moments.n
                    } else {
                        f64::NAN
                    },
                }
            }
        }
    }
}

// ------------------------------------------------------------------------------------------
// Accuracy stop

enum Ensemble {
    Classes { votes: Vec<Vec<(usize, f64)>> },
    Numbers { sum: Vec<f64>, count: Vec<u32> },
}

impl Ensemble {
    fn new(data: &Data) -> Ensemble {
        if data.is_regression() {
            Ensemble::Numbers {
                sum: vec![0.0; data.rows],
                count: vec![0; data.rows],
            }
        } else {
            Ensemble::Classes {
                votes: vec![Vec::new(); data.rows],
            }
        }
    }

    fn add(&mut self, predictions: &[(usize, Prediction)]) {
        match self {
            Ensemble::Classes { votes } => {
                for (row, prediction) in predictions {
                    let Some(class) = prediction.class else {
                        continue;
                    };
                    let Some(entry) = votes.get_mut(*row) else {
                        continue;
                    };
                    if let Some((_, count)) = entry.iter_mut().find(|(code, _)| *code == class) {
                        *count += 1.0;
                    } else {
                        entry.push((class, 1.0));
                    }
                }
            }
            Ensemble::Numbers { sum, count } => {
                for (row, prediction) in predictions {
                    if !prediction.number.is_finite() {
                        continue;
                    }
                    if let (Some(sum), Some(count)) = (sum.get_mut(*row), count.get_mut(*row)) {
                        *sum += prediction.number;
                        *count = count.saturating_add(1);
                    }
                }
            }
        }
    }

    fn accuracy(&self, data: &Data) -> Option<f64> {
        match (self, &data.target) {
            (Ensemble::Classes { votes }, Target::Classes { codes, .. }) => {
                let mut seen = 0usize;
                let mut correct = 0usize;
                for (row, entry) in votes.iter().enumerate() {
                    if entry.is_empty() {
                        continue;
                    }
                    seen += 1;
                    let class = entry
                        .iter()
                        .fold(None::<(usize, f64)>, |best, &(code, count)| match best {
                            Some((best_code, best_count))
                                if count < best_count
                                    || (count == best_count && code > best_code) =>
                            {
                                Some((best_code, best_count))
                            }
                            _ => Some((code, count)),
                        })
                        .map(|(code, _)| code);
                    if class == codes.get(row).copied() {
                        correct += 1;
                    }
                }
                (seen > 0).then_some(correct as f64 / seen as f64)
            }
            (Ensemble::Numbers { sum, count }, Target::Numbers { .. }) => {
                let values = data.target_numbers()?;
                let mut seen = 0usize;
                let mut mean = 0.0;
                for (row, &n) in count.iter().enumerate() {
                    if n > 0
                        && let Some(value) = values.get(row)
                    {
                        seen += 1;
                        mean += *value;
                    }
                }
                if seen == 0 {
                    return None;
                }
                mean /= seen as f64;
                let mut sse = 0.0;
                let mut sst = 0.0;
                for (row, &n) in count.iter().enumerate() {
                    let Some(value) = values.get(row) else {
                        continue;
                    };
                    if n > 0 {
                        let prediction = sum.get(row).copied().unwrap_or(0.0) / n as f64;
                        sse += (value - prediction) * (value - prediction);
                    }
                    sst += (value - mean) * (value - mean);
                }
                Some(if sst > 0.0 { 1.0 - sse / sst } else { 1.0 })
            }
            _ => None,
        }
    }
}

// ------------------------------------------------------------------------------------------
// PMML

fn pmml(
    dataset: &Dataset<'_>,
    data: &Data,
    trees: &[Tree],
    _settings: &RandomTreesSettings,
) -> Result<Pmml, AlgoError> {
    let fields = dataset.table.model().fields();
    let dictionary: Vec<DataField> = (0..fields.len())
        .filter_map(|position| data_field_for(dataset.table, position))
        .collect();
    let top_schema = schema(dataset, data, &vec![true; data.columns.len()]);
    let function = if data.is_regression() {
        MiningFunction::Regression
    } else {
        MiningFunction::Classification
    };
    let mut segments = Vec::new();
    for tree in trees {
        let used: Vec<bool> = data
            .columns
            .iter()
            .enumerate()
            .map(|(index, _)| tree_uses(std::slice::from_ref(tree), index))
            .collect();
        let root = pmml_node(data, tree, 0, Predicate::True)?;
        segments.push(MiningSegment {
            predicate: Predicate::True,
            model: Box::new(Model::Tree(TreeModel {
                header: ModelHeader {
                    name: None,
                    function: function.clone(),
                    algorithm: None,
                    mining_schema: schema(dataset, data, &used),
                    output: Vec::new(),
                    local_transformations: Vec::new(),
                },
                missing_value_strategy: MissingValueStrategy::None,
                no_true_child_strategy: NoTrueChildStrategy::ReturnNullPrediction,
                root,
            })),
        });
    }
    Ok(Pmml {
        version: "4.3".into(),
        data_dictionary: dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Mining(MiningModel {
            header: ModelHeader {
                name: None,
                function,
                algorithm: None,
                mining_schema: top_schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            method: if data.is_regression() {
                "average".into()
            } else {
                "majorityVote".into()
            },
            segments,
        }),
    })
}

fn tree_uses(trees: &[Tree], index: usize) -> bool {
    trees.iter().any(|tree| {
        tree.nodes
            .iter()
            .filter_map(|node| node.split.as_ref())
            .any(|split| split.input == index)
    })
}

/// The model's `DataDictionary` entry, with a flag's true value first (the order the Random Trees
/// oracle writes: `respond` Y, N).
fn data_field_for(table: &Table, position: usize) -> Option<DataField> {
    let field = table.model().fields().get(position)?;
    let mut values = category_order(table, position);
    if let ValueSpec::Flag { true_value, .. } = &field.values {
        let true_text = true_value.to_string();
        if let Some(index) = values.iter().position(|value| *value == true_text) {
            values.swap(0, index);
        }
    }
    Some(DataField {
        name: field.name.clone(),
        optype: if field.measurement == Measurement::Continuous {
            dms_pmml::OpType::Continuous
        } else if field.measurement == Measurement::Ordinal {
            dms_pmml::OpType::Ordinal
        } else {
            dms_pmml::OpType::Categorical
        },
        data_type: match field.storage {
            Storage::Integer => dms_pmml::DataType::Integer,
            Storage::Real => dms_pmml::DataType::Double,
            Storage::Date => dms_pmml::DataType::Date,
            Storage::Time => dms_pmml::DataType::Time,
            Storage::Timestamp => dms_pmml::DataType::DateTime,
            _ => dms_pmml::DataType::String,
        },
        values: if field.measurement == Measurement::Continuous {
            Vec::new()
        } else {
            values
        },
        missing_values: Vec::new(),
        flag_true_value: None,
    })
}

/// The target and the selected inputs (in table order), with missing replacements.
fn schema(dataset: &Dataset<'_>, data: &Data, used: &[bool]) -> Vec<MiningField> {
    let mut out = Vec::new();
    for (index, column) in data.columns.iter().enumerate() {
        if !used.get(index).copied().unwrap_or(false) {
            continue;
        }
        out.push(MiningField {
            name: column.name.clone(),
            usage: Usage::Active,
            missing_replacement: Some(column.replacement.clone()),
            outliers: dms_pmml::Outliers::AsIs,
            low_value: None,
            high_value: None,
            invalid_treatment: dms_pmml::InvalidTreatment::AsMissing,
            direction: None,
        });
    }
    if let Some(field) = dataset.table.model().fields().get(dataset.target) {
        out.push(MiningField {
            name: field.name.clone(),
            usage: Usage::Target,
            missing_replacement: data.target_replacement.clone(),
            outliers: dms_pmml::Outliers::AsIs,
            low_value: None,
            high_value: None,
            invalid_treatment: dms_pmml::InvalidTreatment::AsMissing,
            direction: None,
        });
    }
    out
}

fn pmml_node(
    data: &Data,
    tree: &Tree,
    index: usize,
    predicate: Predicate,
) -> Result<Node, AlgoError> {
    pmml_node_at(data, tree, index, predicate, 0)
}

fn pmml_node_at(
    data: &Data,
    tree: &Tree,
    index: usize,
    predicate: Predicate,
    depth: usize,
) -> Result<Node, AlgoError> {
    if depth > 60 {
        return Err(AlgoError::Data("Random Trees depth exceeded".into()));
    }
    let node = tree
        .nodes
        .get(index)
        .ok_or_else(|| AlgoError::Data("Random Trees node index".into()))?;
    let mut out = Node {
        id: Some(node.id.to_string()),
        score: None,
        record_count: None,
        default_child: None,
        predicate,
        distribution: Vec::<ScoreDistribution>::new(),
        children: Vec::new(),
    };
    if let Some(split) = &node.split {
        let column = data
            .columns
            .get(split.input)
            .ok_or_else(|| AlgoError::Data("Random Trees split input".into()))?;
        let left = pmml_node_at(
            data,
            tree,
            split.left,
            rule_predicate(column, &split.rule),
            depth + 1,
        )?;
        let right = pmml_node_at(data, tree, split.right, Predicate::True, depth + 1)?;
        out.children = vec![left, right];
    } else {
        let prediction = tree.leaf(data, node);
        out.score = if data.is_regression() {
            prediction
                .number
                .is_finite()
                .then(|| number_text(prediction.number))
        } else {
            prediction.class.map(|code| data.label(code))
        };
    }
    Ok(out)
}

fn rule_predicate(column: &Column, rule: &Rule) -> Predicate {
    match (rule, &column.kind) {
        (Rule::Threshold(threshold), _) => Predicate::Simple {
            field: column.name.clone(),
            operator: dms_pmml::Operator::LessOrEqual,
            value: Some(split_text(*threshold)),
        },
        (Rule::Categories { left, .. }, Kind::Categorical { labels, .. }) => {
            let values = left
                .iter()
                .filter_map(|code| labels.get(*code).cloned())
                .collect();
            Predicate::SimpleSet {
                field: column.name.clone(),
                is_in: true,
                values,
            }
        }
        (Rule::Categories { .. }, Kind::Numeric(_)) => Predicate::False,
    }
}

// Keep the unused import warning away when the PMML helpers above change.
const _: Option<MiningField> = None;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_counts_are_binomial_draws() {
        for n in [1usize, 2, 10, 519] {
            let counts = bootstrap(n, 7, 0);
            assert_eq!(counts.len(), n);
            assert!(
                counts
                    .iter()
                    .all(|count| count.is_finite() && *count >= 0.0)
            );
            let mean = counts.iter().sum::<f64>() / n as f64;
            assert!((mean - 1.0).abs() < 0.2, "n={n}: mean {mean}");
        }
    }

    #[test]
    fn predictor_subsets_use_the_documented_sizes() {
        let data = Data {
            columns: (0..9)
                .map(|position| Column {
                    name: format!("x{position}"),
                    kind: Kind::Numeric(vec![1.0]),
                    replacement: "1".into(),
                })
                .collect(),
            target: Target::Classes {
                codes: vec![0],
                labels: vec!["a".into(), "b".into()],
            },
            priors: vec![0.5, 0.5],
            target_replacement: None,
            rows: 1,
        };
        assert_eq!(select_predictors(&data, 1, 0, 1, false).len(), 3);
        assert_eq!(select_predictors(&data, 1, 0, 1, true).len(), 3);
        assert_eq!(select_predictors(&data, 1, 0, 2, false).len(), 3);
        assert_eq!(select_predictors(&data, 1, 0, 2, true).len(), 3);
    }

    #[test]
    fn categorical_subsets_are_the_documented_ones() {
        let nominal = candidate_subsets(&[0, 1, 2, 3], false);
        assert_eq!(nominal.len(), 7);
        assert!(nominal.iter().all(|set| set.contains(&0)));
        assert!(nominal.iter().all(|set| set.len() < 4));
        let ordinal = candidate_subsets(&[0, 1, 2, 3], true);
        assert_eq!(ordinal, vec![vec![0], vec![0, 1], vec![0, 1, 2]]);
    }

    #[test]
    fn lower_median_and_cv() {
        assert_eq!(lower_median(&[3.0, 1.0, 2.0, 4.0]), Some(2.0));
        assert_eq!(lower_median(&[]), None);
        assert!(coefficient_of_variation(&[10.0, 10.0]).is_some_and(|cv| cv < 1e-12));
        assert_eq!(coefficient_of_variation(&[0.0, 0.0]), None);
    }
}
