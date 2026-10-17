//! Clean-room C5.0 classification trees. Sources: IBM SPSS Modeler 19 Scripting
//! and Automation Guide pp.226–227; Algorithms Guide pp.1,51; PMML 4.3 TreeModel.
//! No See5/C5.0 implementation or Modeler executable was consulted.
//!
//! Evidence (`models_ml/` unless qualified):
//! - `c50_default`, `c50_expert`: information gain ratio, numeric midpoint cuts,
//!   multiway symbols; depth-first ids; flags true before false; largest default
//!   child (first on ties); parent prediction resolves tied class counts.
//! - `models/c50_default`: numeric cut description-length correction, statistical
//!   leaf pruning, six-significant-digit thresholds (K = 0.0373315). Rounding is
//!   applied only when it preserves the observed partition. GAP: closer inputs
//!   need a threshold-precision probe. All eight oracle trees use exact comparison.
//! - `c50_min_child`: at least two branches must reach min_child_records.
//! - Of equal gain ratios the field first in input order wins (`models/c50_default`:
//!   `Sex` over `Cholesterol`, mirror-image partitions; `c50_weight`: `flag` over
//!   `noise`). `c50_costs`/`c50_weight`: nominal wins an equal-ratio tie with an
//!   ordinal field having identical partitions (`code` over `ord`). GAP: the
//!   oracles do not distinguish a measurement preference from an ordinal penalty.
//! - `c50_costs` (B->C 8, C->B 2): minimize sum(count[actual]*cost[actual][predicted]),
//!   retaining the parent's class on ties. Counts stay raw. Bound the number of
//!   misclassified cases, then multiply by their mean cost; for zero observed
//!   errors retain the ordinary bound. Under amber/flag=T/x<=9 this gives leaf
//!   8.75954415583688 versus subtree 7.567663573829303, so the split stays.
//! - Growth uses raw counts even with costs. The average-gain denominator includes
//!   zero-gain fields: x gain 0.09645172825007178 exceeds 0.5040579037580681/7
//!   at that 18-case node (but not the previous positive-candidates-only average
//!   0.12601447593951703). This reproduces `c50_costs` and the seven earlier trees
//!   without cost-dependent class factors. GAP: counting only globally variable
//!   fields can also fit this fixture; duplicate/constant-field probes are needed.
//! - `c50_costs`: after pruning, merge symbolic sibling leaves with the same score,
//!   even without groupSymbolicValues (red/code in {2,3}); recompute distributions.
//! - Case weights scaled to sum to the record count are implemented in the algorithm
//!   (`C50Settings::weight_field`, diagnostic only) but the node rejects `useWeight`.
//!   `c50_weight` leaf counts use eight significant decimal digits, NOT f32 (e.g.
//!   3.1093117408906883 -> 3.1093117, not 3.1093118). Totals sum printed child counts.
//!   GAP: node 34 (red/code=3/flag=T) prunes x<=9 although its weighted bounds are
//!   5.403606975510416 versus 3.197396295622889. Nodes 23/24 (red/code=1/5<x<=11)
//!   have all-zero distributions despite nonzero counts. Internal distributions
//!   are NOT always child sums: node 4 A=15.546559 versus child sum 15.5465589;
//!   nodes 3/4/8/9/13/14/33 match independently rounded raw counts instead. The
//!   internal summation below is still diagnostic; 20 of 36 nodes match exactly,
//!   and the full weighted contract is explicitly ignored, not claimed passing.
//! - `c50_group_symbols`: greedy gain-ratio grouping of symbolic categories.
//! - `c50_prune90`: pruning confidence is (100 - severity)/100.
//! - `c50_no_global_prune`: local pruning alone. GAP: pruning slack (.5 global,
//!   .1 local) is inferred from the two default trees; a low-support balanced
//!   exception probe varying total sample size would distinguish a global error
//!   budget from a fixed slack. Subtree raising is not established.
//! - Tree confidence is (class_count+1)/(node_count+target_categories), with zero
//!   for absent classes in the exported distribution; raw binary propensity uses
//!   confidence and its complement, including absent classes (`c50_partition`).
//! - Rule sets (`modelFormat` `ruleSet`): [`crate::c50_rules`].
//!
//! Rejected settings (never silently substituted): boosting/trials
//! (`c50_boost_tree`, `c50_boost_rules`: reweighting settled, weighted trees GAP; see `c50_boost`),
//! costs combined with weights/rules/boosting (no combined oracle), case/frequency
//! weights (`c50_weight`: GAP pruning and internal distributions), winnowing
//! (`c50_winnow`: GAP elimination criterion), Simple Generality/expected noise
//! (`c50_generality`, `c50_noise20`: GAP preset mapping), adjusted-propensity
//! training (`c50_partition`: GAP calibration pooling/rounding), cross-validation,
//! split-field training and missing/unknown training predictors (no captured
//! c50_missing oracle). PMML scoring supports rules and boosts independently.
//!
//! Follow-up probes (specifications only; none run here):
//! - Copy the 384 rows of models_ml/tree_train.csv. Keep inputs x,cat,flag,ord,code,
//!   constant,noise; y nominal A/B/C; disable partitioning and boosting; expert
//!   min-child=2, severity=75. Costs default=1, B->C=8, C->B=2. Export one contrast
//!   at a time removing constant, adding 1/4/16 constant copies, then removing ord.
//!   This separates gain-denominator hypotheses without changing the records.
//! - Same data/settings, costs off: swap ord/code input order; then change only
//!   ord's measurement to nominal. This distinguishes tie preference/penalty.
//! - Same 384 rows, costs off, weightField=weight, useWeight=true: toggle only
//!   global pruning; repeat with weights multiplied by 10, then all weights=1.
//!   Export both PMML and training scores; inspect nodes by path, not numeric id.
//!   Also isolate the 12 red/code=3/flag=T rows with x in {4,6,8,10,12,14}:
//!   two rows per x, labels C/(A,C)/C/A/A/A respectively, weight=384/494 each.
//!   Compare this alone and with the remaining 372 rows restored to distinguish
//!   local pruning from a tree-wide error budget. Re-export the full weighted tree
//!   with row order reversed to test the zero-distribution/rounding anomalies.
//! - For numeric output precision, x={1.0000001,1.0000002}, ten A then ten B,
//!   y nominal, minimum child=2, costs/weights/boosting off; inspect threshold.

use crate::{AlgoError, Dataset, categories, data_dictionary, mining_schema};
use dms_pmml::{
    MiningFunction, MissingValueStrategy, Model, ModelHeader, NoTrueChildStrategy, Node, Operator,
    Pmml, Predicate, ScoreDistribution, TreeModel,
};
use dms_types::{Measurement, Value};

const MAX_ROWS: usize = 100_000;
const MAX_INPUTS: usize = 256;
const MAX_CATEGORIES: usize = 1024;
const MAX_NODES: usize = 10_000;
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct C50Settings {
    pub min_child_records: usize,
    pub pruning_severity: f64,
    pub global_pruning: bool,
    pub group_symbolics: bool,
    /// Diagnostic case weights; the exec node rejects these until the GAPs above close.
    pub weight_field: Option<usize>,
    /// Explicit actual/predicted costs; absent off-diagonal entries use default_cost.
    pub costs: Option<MisclassificationCosts>,
    /// A rule set instead of a tree (`modelFormat` `ruleSet`): [`crate::c50_rules`].
    pub rules: bool,
    /// Boosting trials (`useBoosting` with `boostingTrials`): [`crate::c50_boost`].
    pub boost_trials: Option<usize>,
}

impl Default for C50Settings {
    fn default() -> Self {
        Self {
            min_child_records: 2,
            pruning_severity: 75.0,
            global_pruning: true,
            group_symbolics: false,
            weight_field: None,
            costs: None,
            rules: false,
            boost_trials: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MisclassificationCosts {
    pub default_cost: f64,
    pub entries: Vec<(String, String, f64)>,
}

impl MisclassificationCosts {
    fn matrix(&self, classes: &[String]) -> Result<Vec<Vec<f64>>, AlgoError> {
        if !self.default_cost.is_finite() || self.default_cost <= 0.0 {
            return Err(AlgoError::Settings(
                "C5.0 default cost must be finite and positive".into(),
            ));
        }
        if self.entries.len() > MAX_NODES {
            return Err(AlgoError::Unsupported("C5.0 cost entry limit".into()));
        }
        let mut matrix = vec![vec![self.default_cost; classes.len()]; classes.len()];
        for (i, row) in matrix.iter_mut().enumerate() {
            if let Some(diagonal) = row.get_mut(i) {
                *diagonal = 0.0;
            }
        }
        let mut seen = std::collections::HashSet::new();
        for (actual, predicted, cost) in &self.entries {
            if !cost.is_finite() || *cost < 0.0 || (actual == predicted && *cost != 0.0) {
                return Err(AlgoError::Settings(
                    "C5.0 costs must be finite, nonnegative, and zero on the diagonal".into(),
                ));
            }
            let from = classes.iter().position(|s| s == actual);
            let to = classes.iter().position(|s| s == predicted);
            let (Some(from), Some(to)) = (from, to) else {
                return Err(AlgoError::Settings(
                    "C5.0 cost refers to an unknown target category".into(),
                ));
            };
            if !seen.insert((from, to)) {
                return Err(AlgoError::Settings("C5.0 duplicate cost entry".into()));
            }
            if let Some(cell) = matrix.get_mut(from).and_then(|r| r.get_mut(to)) {
                *cell = *cost;
            }
        }
        if matrix
            .iter()
            .flatten()
            .any(|c| *c > f64::MAX / (MAX_ROWS as f64 * MAX_CATEGORIES as f64))
        {
            return Err(AlgoError::Settings("C5.0 costs are too large".into()));
        }
        Ok(matrix)
    }
}

enum Column {
    Numeric {
        name: String,
        values: Vec<f64>,
    },
    Symbolic {
        name: String,
        values: Vec<usize>,
        labels: Vec<String>,
        flag: bool,
        ordinal: bool,
    },
}

struct Data {
    columns: Vec<Column>,
    classes: Vec<String>,
    targets: Vec<usize>,
    /// Case weights scaled to sum to the number of records (all 1 without a weight field).
    weights: Vec<f64>,
    costs: Option<Vec<Vec<f64>>>,
}

impl Data {
    fn read(dataset: &Dataset<'_>, weight_field: Option<usize>) -> Result<Self, AlgoError> {
        let fields = dataset.table.model().fields();
        let target = fields
            .get(dataset.target)
            .ok_or_else(|| AlgoError::Data("invalid C5.0 target index".into()))?;
        if target.measurement == Measurement::Continuous {
            return Err(AlgoError::Data("C5.0 needs a categorical target".into()));
        }
        if dataset.table.row_count() > MAX_ROWS || dataset.inputs.len() > MAX_INPUTS {
            return Err(AlgoError::Unsupported(format!(
                "C5.0 is limited to {MAX_ROWS} records and {MAX_INPUTS} inputs"
            )));
        }
        let classes = categories(dataset, dataset.target);
        if classes.is_empty() || classes.len() > MAX_CATEGORIES {
            return Err(AlgoError::Data(
                "C5.0 target has no categories or too many categories".into(),
            ));
        }
        let mut row_indices = Vec::new();
        let mut targets = Vec::new();
        for (i, row) in dataset.table.rows().iter().enumerate() {
            let Some(value) = row.get(dataset.target).filter(|v| !target.is_missing(v)) else {
                continue;
            };
            let text = value.to_string();
            let class = classes
                .iter()
                .position(|c| c == &text)
                .ok_or_else(|| AlgoError::Data(format!("C5.0 unknown target {text:?}")))?;
            row_indices.push(i);
            targets.push(class);
        }
        if targets.is_empty() {
            return Err(AlgoError::Data("C5.0 has no valid targets".into()));
        }
        let weights = match weight_field {
            None => vec![1.0; targets.len()],
            Some(position) => {
                let raw = row_indices
                    .iter()
                    .map(
                        |&i| match dataset.table.rows().get(i).and_then(|r| r.get(position)) {
                            Some(Value::Integer(n)) if *n > 0 => Ok(*n as f64),
                            Some(Value::Real(w)) if w.is_finite() && *w > 0.0 => Ok(*w),
                            _ => Err(AlgoError::Unsupported(
                                "C5.0 case weights that are missing or not positive".into(),
                            )),
                        },
                    )
                    .collect::<Result<Vec<f64>, _>>()?;
                let total = raw.iter().sum::<f64>();
                if !total.is_finite() {
                    return Err(AlgoError::Data("C5.0 case-weight sum overflow".into()));
                }
                let scale = raw.len() as f64 / total;
                let scaled: Vec<f64> = raw.into_iter().map(|w| w * scale).collect();
                if scaled.iter().any(|w| !w.is_finite() || *w <= 0.0) {
                    return Err(AlgoError::Data(
                        "C5.0 case-weight normalization overflow or underflow".into(),
                    ));
                }
                scaled
            }
        };
        let mut columns = Vec::new();
        for &position in dataset.inputs {
            if position == dataset.target {
                return Err(AlgoError::Settings("C5.0 target is also an input".into()));
            }
            let field = fields
                .get(position)
                .ok_or_else(|| AlgoError::Data("invalid C5.0 input index".into()))?;
            let values = row_indices
                .iter()
                .map(|&i| {
                    dataset
                        .table
                        .rows()
                        .get(i)
                        .and_then(|r| r.get(position))
                        .filter(|v| !field.is_missing(v))
                        .ok_or_else(|| {
                            AlgoError::Unsupported(format!(
                                "C5.0 missing training predictor {}",
                                field.name
                            ))
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if field.measurement == Measurement::Continuous {
                let numbers = values
                    .into_iter()
                    .map(|v| match v {
                        Value::Integer(n) => Ok(*n as f64),
                        Value::Real(n) if n.is_finite() => Ok(*n),
                        _ => Err(AlgoError::Data(format!(
                            "C5.0 input {} is not finite numeric data",
                            field.name
                        ))),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                columns.push(Column::Numeric {
                    name: field.name.clone(),
                    values: numbers,
                });
            } else {
                let mut labels = categories(dataset, position);
                if labels.len() > MAX_CATEGORIES {
                    return Err(AlgoError::Unsupported(
                        "C5.0 predictor category limit".into(),
                    ));
                }
                let flag = field.measurement == Measurement::Flag;
                if flag {
                    labels.reverse();
                }
                let codes = values
                    .iter()
                    .map(|v| {
                        labels
                            .iter()
                            .position(|s| s == &v.to_string())
                            .ok_or_else(|| {
                                AlgoError::Unsupported(format!(
                                    "C5.0 unknown training category in {}",
                                    field.name
                                ))
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                columns.push(Column::Symbolic {
                    name: field.name.clone(),
                    values: codes,
                    labels,
                    flag,
                    ordinal: field.measurement == Measurement::Ordinal,
                });
            }
        }
        Ok(Self {
            columns,
            classes,
            targets,
            weights,
            costs: None,
        })
    }

    /// The weighted number of records in `rows`.
    fn weight(&self, rows: &[usize]) -> f64 {
        rows.iter().filter_map(|&r| self.weights.get(r)).sum()
    }

    fn counts(&self, rows: &[usize]) -> Vec<f64> {
        let mut counts = vec![0.0; self.classes.len()];
        for &row in rows {
            if let Some(c) = self.targets.get(row).and_then(|&i| counts.get_mut(i)) {
                *c += self.weights.get(row).copied().unwrap_or(0.0);
            }
        }
        counts
    }
}

fn entropy(counts: &[f64]) -> f64 {
    let total: f64 = counts.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    counts
        .iter()
        .filter(|&&c| c > 0.0)
        .map(|c| {
            let p = c / total;
            -p * p.log2()
        })
        .sum()
}

struct Split {
    gain: f64,
    ratio: f64,
    groups: Vec<(Predicate, Vec<usize>)>,
    ordinal: bool,
}

fn candidate(
    data: &Data,
    rows: &[usize],
    column: &Column,
    min_child: usize,
    grouping: bool,
) -> Option<Split> {
    let parent_counts = data.counts(rows);
    let base = entropy(&parent_counts);
    let total = data.weight(rows);
    let min_child = min_child as f64;
    match column {
        Column::Numeric { name, values } => {
            let mut sorted: Vec<(f64, usize, f64)> = rows
                .iter()
                .filter_map(|&r| {
                    let w = data.weights.get(r).copied()?;
                    values
                        .get(r)
                        .zip(data.targets.get(r))
                        .map(|(&v, &c)| (v, c, w))
                })
                .collect();
            sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut left = vec![0.0; data.classes.len()];
            let mut right = parent_counts;
            let mut best: Option<(f64, f64, f64)> = None;
            let mut cuts = 0usize;
            let mut n = 0.0;
            for pair in sorted.windows(2) {
                let [(value, class, weight), (next, _, _)] = pair else {
                    continue;
                };
                if let Some(c) = left.get_mut(*class) {
                    *c += weight;
                }
                if let Some(c) = right.get_mut(*class) {
                    *c -= weight;
                }
                n += weight;
                if value == next || n < min_child || total - n < min_child {
                    continue;
                }
                cuts = cuts.saturating_add(1);
                let gain =
                    base - n / total * entropy(&left) - (total - n) / total * entropy(&right);
                if best.is_none_or(|(g, _, _)| gain > g + 1e-12) {
                    let midpoint = value / 2.0 + next / 2.0;
                    let cut = if midpoint >= *next { *value } else { midpoint };
                    // `models/c50_default`: exported K cut is 0.0373315, not
                    // 0.037331500000000004. Keep the training partition intact
                    // when six significant digits cannot separate close values.
                    let cut = format!("{cut:.5e}")
                        .parse::<f64>()
                        .ok()
                        .filter(|rounded| *rounded >= *value && *rounded < *next)
                        .unwrap_or(cut);
                    best = Some((gain, cut, n));
                }
            }
            let (gain, cut, n) = best?;
            let gain = gain - (cuts.max(1) as f64).log2() / total;
            let split_info = entropy(&[n, total - n]);
            if gain <= 1e-10 || split_info <= 0.0 {
                return None;
            }
            let (left, right): (Vec<_>, Vec<_>) = rows
                .iter()
                .copied()
                .partition(|&r| values.get(r).is_some_and(|v| *v <= cut));
            Some(Split {
                gain,
                ratio: gain / split_info,
                ordinal: false,
                groups: vec![
                    (
                        Predicate::Simple {
                            field: name.clone(),
                            operator: Operator::LessOrEqual,
                            value: Some(cut.to_string()),
                        },
                        left,
                    ),
                    (
                        Predicate::Simple {
                            field: name.clone(),
                            operator: Operator::GreaterThan,
                            value: Some(cut.to_string()),
                        },
                        right,
                    ),
                ],
            })
        }
        Column::Symbolic {
            name,
            values,
            labels,
            flag,
            ordinal,
        } => {
            let mut groups: Vec<(Vec<usize>, Vec<usize>)> = labels
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    (
                        vec![i],
                        rows.iter()
                            .copied()
                            .filter(|&r| values.get(r) == Some(&i))
                            .collect(),
                    )
                })
                .filter(|(_, rs): &(Vec<usize>, Vec<usize>)| !rs.is_empty())
                .collect();
            // Identically pure symbols are interchangeable even without explicit grouping.
            if !flag {
                merge_groups(data, &mut groups, grouping);
            }
            if groups
                .iter()
                .filter(|(_, rs)| data.weight(rs) >= min_child)
                .count()
                < 2
            {
                return None;
            }
            let (gain, ratio) = group_quality(data, &groups, base, total);
            if gain <= 1e-10 {
                return None;
            }
            let sets = !flag && (grouping || groups.iter().any(|(labels, _)| labels.len() > 1));
            let groups = groups
                .into_iter()
                .map(|(codes, rs)| {
                    let symbols: Vec<String> = codes
                        .iter()
                        .filter_map(|&c| labels.get(c).cloned())
                        .collect();
                    let pred = if sets {
                        Predicate::SimpleSet {
                            field: name.clone(),
                            is_in: true,
                            values: symbols,
                        }
                    } else {
                        Predicate::Simple {
                            field: name.clone(),
                            operator: Operator::Equal,
                            value: symbols.first().cloned(),
                        }
                    };
                    (pred, rs)
                })
                .collect();
            Some(Split {
                gain,
                ratio,
                groups,
                ordinal: *ordinal,
            })
        }
    }
}

fn group_quality(
    data: &Data,
    groups: &[(Vec<usize>, Vec<usize>)],
    base: f64,
    total: f64,
) -> (f64, f64) {
    let gain = base
        - groups
            .iter()
            .map(|(_, rs)| data.weight(rs) / total * entropy(&data.counts(rs)))
            .sum::<f64>();
    let info = entropy(
        &groups
            .iter()
            .map(|(_, rs)| data.weight(rs))
            .collect::<Vec<_>>(),
    );
    (gain, if info > 0.0 { gain / info } else { 0.0 })
}

fn merge_groups(data: &Data, groups: &mut Vec<(Vec<usize>, Vec<usize>)>, grouping: bool) {
    if groups.len() > 64 {
        return;
    }
    let total: f64 = groups.iter().map(|(_, rs)| data.weight(rs)).sum();
    let mut all = Vec::new();
    for (_, rows) in groups.iter() {
        all.extend(rows);
    }
    let base = entropy(&data.counts(&all));
    for _ in 0..MAX_CATEGORIES {
        if groups.len() <= 2 {
            break;
        }
        let current = group_quality(data, groups, base, total).1;
        let mut best = None;
        for i in 0..groups.len() {
            for j in i.saturating_add(1)..groups.len() {
                let Some((_, a)) = groups.get(i) else {
                    continue;
                };
                let Some((_, b)) = groups.get(j) else {
                    continue;
                };
                if !grouping {
                    let ca = data.counts(a);
                    let cb = data.counts(b);
                    let (wa, wb) = (data.weight(a), data.weight(b));
                    let pure_a = ca.iter().position(|c| *c == wa);
                    let pure_b = cb.iter().position(|c| *c == wb);
                    if pure_a.is_none() || pure_a != pure_b {
                        continue;
                    }
                }
                let mut trial = groups.clone();
                let (codes, rows) = trial.remove(j);
                if let Some((a, b)) = trial.get_mut(i) {
                    a.extend(codes);
                    b.extend(rows);
                }
                let quality = group_quality(data, &trial, base, total).1;
                if quality > current + 1e-12
                    && best.as_ref().is_none_or(|(_, _, q)| quality > *q + 1e-12)
                {
                    best = Some((i, j, quality));
                }
            }
        }
        let Some((i, j, _)) = best else {
            break;
        };
        let (codes, rows) = groups.remove(j);
        if let Some((a, b)) = groups.get_mut(i) {
            a.extend(codes);
            a.sort_unstable();
            b.extend(rows);
        }
    }
}

fn winner(counts: &[f64], parent: usize) -> usize {
    let max = counts.iter().copied().fold(0.0, f64::max);
    if counts.get(parent) == Some(&max) {
        parent
    } else {
        counts.iter().position(|c| *c == max).unwrap_or(0)
    }
}

fn cost_winner(counts: &[f64], parent: usize, costs: Option<&[Vec<f64>]>) -> usize {
    let Some(costs) = costs else {
        return winner(counts, parent);
    };
    let losses: Vec<f64> = (0..counts.len())
        .map(|predicted| {
            counts
                .iter()
                .enumerate()
                .map(|(actual, count)| {
                    count
                        * costs
                            .get(actual)
                            .and_then(|r| r.get(predicted))
                            .copied()
                            .unwrap_or(0.0)
                })
                .sum()
        })
        .collect();
    let best = losses.iter().copied().fold(f64::INFINITY, f64::min);
    if losses.get(parent) == Some(&best) {
        parent
    } else {
        losses.iter().position(|c| *c == best).unwrap_or(0)
    }
}

fn grow(
    data: &Data,
    rows: &[usize],
    settings: &C50Settings,
    parent: usize,
    depth: usize,
    budget: &mut usize,
) -> Result<Node, AlgoError> {
    if depth > MAX_DEPTH || *budget >= MAX_NODES {
        return Err(AlgoError::Unsupported(
            "C5.0 tree depth/node limit exceeded".into(),
        ));
    }
    *budget = budget.saturating_add(1);
    let counts = data.counts(rows);
    let class = cost_winner(&counts, parent, data.costs.as_deref());
    let total = data.weight(rows);
    let distribution = data
        .classes
        .iter()
        .zip(&counts)
        .map(|(value, &count)| ScoreDistribution {
            value: value.clone(),
            record_count: count,
            probability: Some(count / total),
            confidence: Some(if count > 0.0 {
                (count + 1.0) / (total + data.classes.len() as f64)
            } else {
                0.0
            }),
        })
        .collect();
    let mut node = Node {
        id: None,
        score: data.classes.get(class).cloned(),
        record_count: Some(total),
        default_child: None,
        predicate: Predicate::True,
        distribution,
        children: Vec::new(),
    };
    if counts.iter().filter(|&&c| c > 0.0).count() < 2
        || total < settings.min_child_records as f64 * 2.0
    {
        return Ok(node);
    }
    // Input order breaks ties except identical nominal/ordinal partitions.
    let candidates: Vec<Split> = data
        .columns
        .iter()
        .filter_map(|col| {
            candidate(
                data,
                rows,
                col,
                settings.min_child_records,
                settings.group_symbolics,
            )
        })
        .collect();
    if candidates.is_empty() {
        return Ok(node);
    }
    // Include zero-gain fields; otherwise cost-tree node 5 cannot grow its x split.
    let average = candidates.iter().map(|c| c.gain).sum::<f64>() / data.columns.len() as f64;
    let mut best: Option<Split> = None;
    for split in candidates {
        if split.gain >= average - 1e-9
            && best.as_ref().is_none_or(|s| {
                split.ratio > s.ratio + 1e-12
                    || ((split.ratio - s.ratio).abs() <= 1e-12
                        && s.ordinal
                        && !split.ordinal
                        && split.groups.len() == s.groups.len()
                        && split
                            .groups
                            .iter()
                            .all(|(_, rows)| s.groups.iter().any(|(_, other)| rows == other)))
            })
        {
            best = Some(split);
        }
    }
    if let Some(split) = best {
        for (predicate, group) in split.groups {
            let mut child = grow(data, &group, settings, class, depth + 1, budget)?;
            child.predicate = predicate;
            node.children.push(child);
        }
    }
    Ok(node)
}

fn validate_settings(dataset: &Dataset<'_>, settings: &C50Settings) -> Result<(), AlgoError> {
    if settings.costs.is_some()
        && (settings.rules || settings.boost_trials.is_some() || settings.weight_field.is_some())
    {
        return Err(AlgoError::Unsupported(
            "C5.0 costs combined with rules, boosting, or case weights".into(),
        ));
    }
    if dataset.frequency.is_some() {
        return Err(AlgoError::Unsupported("C5.0 frequency weights".into()));
    }
    if settings.min_child_records == 0
        || settings.min_child_records > MAX_ROWS
        || !settings.pruning_severity.is_finite()
        || !(1.0..100.0).contains(&settings.pruning_severity)
    {
        return Err(AlgoError::Settings(
            "C5.0 minimum child records or pruning severity out of range".into(),
        ));
    }
    Ok(())
}

/// Pruned tree whose leaves become C5.0 rules. Growth and pruning match
/// [`train_tree`] (`c50_rules_voting`: 13 leaves, the same as `c50_default`).
/// GAP: C4.5 extracts from the unpruned tree; extra unpruned splits on this
/// fixture are not in the exported rules.
pub(crate) fn rule_tree(dataset: &Dataset<'_>, settings: &C50Settings) -> Result<Node, AlgoError> {
    validate_settings(dataset, settings)?;
    let data = Data::read(dataset, settings.weight_field)?;
    let rows: Vec<usize> = (0..data.targets.len()).collect();
    let mut root = grow(&data, &rows, settings, 0, 0, &mut 0)?;
    prune(&mut root, settings, None);
    Ok(root)
}

/// One-sided Wilson upper bound with continuity correction; zero-error bound is
/// the exact binomial bound. Numerical inversion avoids an inverse-normal dependency.
/// Tree pruning and rule-condition dropping share this estimate (`c50_rules_voting`).
pub(crate) fn error_bound(total: f64, errors: f64, confidence: f64) -> f64 {
    if total <= 0.0 {
        return 0.0;
    }
    if errors <= 0.0 {
        return total * (1.0 - confidence.powf(1.0 / total));
    }
    if errors + 0.5 >= total {
        return total;
    }
    let mut low = 0.0;
    let mut high = 8.0;
    for _ in 0..64 {
        let z = (low + high) / 2.0;
        if normal_cdf(z) < 1.0 - confidence {
            low = z;
        } else {
            high = z;
        }
    }
    let z = (low + high) / 2.0;
    let p = (errors + 0.5) / total;
    total
        * (p + z * z / (2.0 * total)
            + z * (p * (1.0 - p) / total + z * z / (4.0 * total * total)).sqrt())
        / (1.0 + z * z / total)
}

fn normal_cdf(z: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.2316419 * z.abs());
    let tail = (-z * z / 2.0).exp() / (2.0 * std::f64::consts::PI).sqrt()
        * t
        * (0.319381530
            + t * (-0.356563782 + t * (1.781477937 + t * (-1.821255978 + t * 1.330274429))));
    if z >= 0.0 { 1.0 - tail } else { tail }
}

fn prune(node: &mut Node, settings: &C50Settings, costs: Option<&[Vec<f64>]>) -> f64 {
    let total = node.record_count.unwrap_or(0.0);
    let correct = node
        .distribution
        .iter()
        .find(|d| Some(&d.value) == node.score.as_ref())
        .map_or(0.0, |d| d.record_count);
    let mut leaf = error_bound(
        total,
        total - correct,
        (100.0 - settings.pruning_severity) / 100.0,
    );
    if let Some(costs) = costs {
        let predicted = node
            .distribution
            .iter()
            .position(|d| Some(&d.value) == node.score.as_ref())
            .unwrap_or(0);
        let loss: f64 = node
            .distribution
            .iter()
            .enumerate()
            .map(|(actual, d)| {
                d.record_count
                    * costs
                        .get(actual)
                        .and_then(|r| r.get(predicted))
                        .copied()
                        .unwrap_or(0.0)
            })
            .sum();
        if total > correct {
            leaf *= loss / (total - correct);
        }
    }
    if node.children.is_empty() {
        return leaf;
    }
    let subtree: f64 = node
        .children
        .iter_mut()
        .map(|c| prune(c, settings, costs))
        .sum();
    let slack = if settings.global_pruning { 0.5 } else { 0.1 };
    if leaf <= subtree + slack {
        node.children.clear();
        leaf
    } else {
        subtree
    }
}

fn number(node: &mut Node, next: &mut usize) {
    node.id = Some(next.to_string());
    *next = next.saturating_add(1);
    let mut largest: Option<(f64, String)> = None;
    for child in &mut node.children {
        number(child, next);
        let count = child.record_count.unwrap_or(0.0);
        if largest.as_ref().is_none_or(|(n, _)| count > *n) {
            largest = child.id.as_ref().map(|id| (count, id.clone()));
        }
    }
    node.default_child = largest.map(|(_, id)| id);
}

fn merge_cost_leaves(node: &mut Node) {
    for child in &mut node.children {
        merge_cost_leaves(child);
    }
    let mut merged = false;
    let mut i = 0;
    while i < node.children.len() {
        let mut j = i.saturating_add(1);
        while j < node.children.len() {
            let pair = node.children.get(i).zip(node.children.get(j));
            let values = pair.and_then(|(a, b)| {
                if !a.children.is_empty() || !b.children.is_empty() || a.score != b.score {
                    return None;
                }
                let (af, mut av) = symbolic_branch(&a.predicate)?;
                let (bf, bv) = symbolic_branch(&b.predicate)?;
                if af != bf {
                    return None;
                }
                av.extend(bv);
                Some((af, av))
            });
            if let Some((field, values)) = values {
                let b = node.children.remove(j);
                if let Some(a) = node.children.get_mut(i) {
                    a.predicate = Predicate::SimpleSet {
                        field,
                        is_in: true,
                        values,
                    };
                    let total = a.record_count.unwrap_or(0.0) + b.record_count.unwrap_or(0.0);
                    a.record_count = Some(total);
                    for (ad, bd) in a.distribution.iter_mut().zip(&b.distribution) {
                        ad.record_count += bd.record_count;
                    }
                    refresh_distribution(a);
                }
                merged = true;
            } else {
                j = j.saturating_add(1);
            }
        }
        i = i.saturating_add(1);
    }
    if merged {
        for child in &mut node.children {
            if let Some((field, values)) = symbolic_branch(&child.predicate) {
                child.predicate = Predicate::SimpleSet {
                    field,
                    is_in: true,
                    values,
                };
            }
        }
    }
}

fn symbolic_branch(predicate: &Predicate) -> Option<(String, Vec<String>)> {
    match predicate {
        Predicate::Simple {
            field,
            operator: Operator::Equal,
            value: Some(value),
        } => Some((field.clone(), vec![value.clone()])),
        Predicate::SimpleSet {
            field,
            is_in: true,
            values,
        } => Some((field.clone(), values.clone())),
        _ => None,
    }
}

fn refresh_distribution(node: &mut Node) {
    let total = node.record_count.unwrap_or(0.0);
    let classes = node.distribution.len() as f64;
    for d in &mut node.distribution {
        d.probability = Some(if total > 0.0 {
            d.record_count / total
        } else {
            0.0
        });
        d.confidence = Some(if d.record_count > 0.0 {
            (d.record_count + 1.0) / (total + classes)
        } else {
            0.0
        });
    }
}

/// `c50_weight`: leaf class counts have eight significant decimal digits;
/// Totals are f64 sums of exported children. Summing internal distributions is
/// only a diagnostic hypothesis: the oracle disagrees at nodes 3/4/8/9/13/14/33.
/// This is NOT f32 rounding: 3.1093117408906883 exports as 3.1093117, whereas its
/// nearest f32 is 3.109311819076538. GAP: zero distributions at nodes 23 and 24.
fn finalize_weighted_counts(node: &mut Node) -> Result<(), AlgoError> {
    if node.children.is_empty() {
        for d in &mut node.distribution {
            d.record_count = format!("{:.7e}", d.record_count)
                .parse::<f64>()
                .map_err(|e| AlgoError::Data(format!("C5.0 weighted count formatting: {e}")))?;
        }
        node.record_count = Some(node.distribution.iter().map(|d| d.record_count).sum());
    } else {
        for child in &mut node.children {
            finalize_weighted_counts(child)?;
        }
        node.record_count = Some(node.children.iter().filter_map(|c| c.record_count).sum());
        for (i, d) in node.distribution.iter_mut().enumerate() {
            d.record_count = node
                .children
                .iter()
                .filter_map(|c| c.distribution.get(i))
                .map(|d| d.record_count)
                .sum();
        }
    }
    refresh_distribution(node);
    Ok(())
}

pub fn train(dataset: &Dataset<'_>, settings: &C50Settings) -> Result<Pmml, AlgoError> {
    if let Some(trials) = settings.boost_trials {
        return crate::c50_boost::train(dataset, settings, trials);
    }
    if settings.rules {
        return crate::c50_rules::train(dataset, settings);
    }
    train_tree(dataset, settings)
}

/// One decision tree (no boosting, no rule set). Weighted training is diagnostic:
/// see the module GAPs; the exec node deliberately rejects `useWeight`.
pub fn train_tree(dataset: &Dataset<'_>, settings: &C50Settings) -> Result<Pmml, AlgoError> {
    validate_settings(dataset, settings)?;
    let mut data = Data::read(dataset, settings.weight_field)?;
    data.costs = settings
        .costs
        .as_ref()
        .map(|c| c.matrix(&data.classes))
        .transpose()?;
    let rows: Vec<usize> = (0..data.targets.len()).collect();
    let mut root = grow(&data, &rows, settings, 0, 0, &mut 0)?;
    prune(&mut root, settings, data.costs.as_deref());
    if data.costs.is_some() {
        merge_cost_leaves(&mut root);
    }
    if settings.weight_field.is_some() {
        finalize_weighted_counts(&mut root)?;
    }
    number(&mut root, &mut 0);
    Ok(Pmml {
        version: "4.3".into(),
        data_dictionary: data_dictionary(dataset),
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Tree(TreeModel {
            header: ModelHeader {
                name: None,
                function: MiningFunction::Classification,
                algorithm: Some("C5".into()),
                mining_schema: mining_schema(dataset),
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            missing_value_strategy: MissingValueStrategy::WeightedConfidence,
            no_true_child_strategy: NoTrueChildStrategy::ReturnNullPrediction,
            root,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dms_types::{DataModel, Field, Storage, Table, ValueSpec};

    fn sample() -> Table {
        let mut x = Field::new("x", Storage::Real);
        x.measurement = Measurement::Continuous;
        let mut y = Field::new("y", Storage::String);
        y.measurement = Measurement::Nominal;
        y.values = ValueSpec::Categories(vec![Value::string("a"), Value::string("b")]);
        Table::new(
            DataModel::new(vec![x, y]).unwrap(),
            (0..20)
                .map(|i| {
                    vec![
                        Value::Real(i as f64),
                        Value::string(if i < 10 { "a" } else { "b" }),
                    ]
                })
                .collect(),
        )
        .unwrap()
    }
    #[test]
    fn numeric_midpoint_and_smoothed_confidence() {
        let table = sample();
        let dataset = Dataset {
            table: &table,
            target: 1,
            inputs: &[0],
            frequency: None,
        };
        let pmml = train(&dataset, &C50Settings::default()).unwrap();
        let s = pmml.bind(&["x"]).unwrap();
        let p = s.score(&[dms_pmml::Value::Number(9.5)]).unwrap();
        assert_eq!(p.value, dms_pmml::Value::Text("a".into()));
        assert_eq!(p.confidence, Some(11.0 / 12.0));
        assert_eq!(
            s.score(&[dms_pmml::Value::Number(9.6)]).unwrap().value,
            dms_pmml::Value::Text("b".into())
        );
    }

    #[test]
    fn cost_direction_parent_ties_and_raw_counts() {
        let classes = vec!["A".into(), "B".into(), "C".into()];
        let costs = MisclassificationCosts {
            default_cost: 1.0,
            entries: vec![("B".into(), "C".into(), 8.0), ("C".into(), "B".into(), 2.0)],
        }
        .matrix(&classes)
        .unwrap();
        assert_eq!(
            costs,
            vec![
                vec![0.0, 1.0, 1.0],
                vec![1.0, 0.0, 8.0],
                vec![1.0, 2.0, 0.0]
            ]
        );
        assert_eq!(cost_winner(&[2.0, 2.0, 2.0], 1, Some(&costs)), 0);
        assert_eq!(cost_winner(&[2.0, 0.0, 2.0], 2, Some(&costs)), 2);
        assert_eq!(cost_winner(&[2.0, 0.0, 2.0], 1, Some(&costs)), 0);

        // A cheaper prediction may differ from the majority without changing
        // record counts or smoothed confidence.
        let table = sample();
        let pmml = train_tree(
            &Dataset {
                table: &table,
                target: 1,
                inputs: &[],
                frequency: None,
            },
            &C50Settings {
                costs: Some(MisclassificationCosts {
                    default_cost: 1.0,
                    entries: vec![("a".into(), "b".into(), 0.25)],
                }),
                ..C50Settings::default()
            },
        )
        .unwrap();
        let Model::Tree(tree) = pmml.model else {
            panic!("tree");
        };
        assert_eq!(tree.root.score.as_deref(), Some("b"));
        assert_eq!(tree.root.record_count, Some(20.0));
        assert_eq!(
            tree.root
                .distribution
                .iter()
                .map(|d| d.record_count)
                .collect::<Vec<_>>(),
            [10.0, 10.0]
        );
        assert_eq!(tree.root.distribution[1].confidence, Some(0.5));
    }

    #[test]
    fn malformed_costs_and_unverified_combinations_are_errors() {
        let table = sample();
        let dataset = Dataset {
            table: &table,
            target: 1,
            inputs: &[0],
            frequency: None,
        };
        for costs in [
            MisclassificationCosts {
                default_cost: f64::NAN,
                entries: vec![],
            },
            MisclassificationCosts {
                default_cost: 0.0,
                entries: vec![],
            },
            MisclassificationCosts {
                default_cost: 1.0,
                entries: vec![("a".into(), "b".into(), f64::INFINITY)],
            },
            MisclassificationCosts {
                default_cost: 1.0,
                entries: vec![("a".into(), "b".into(), -1.0)],
            },
            MisclassificationCosts {
                default_cost: 1.0,
                entries: vec![("a".into(), "b".into(), f64::MAX)],
            },
            MisclassificationCosts {
                default_cost: 1.0,
                entries: vec![("absent".into(), "b".into(), 1.0)],
            },
            MisclassificationCosts {
                default_cost: 1.0,
                entries: vec![("a".into(), "a".into(), 1.0)],
            },
            MisclassificationCosts {
                default_cost: 1.0,
                entries: vec![("a".into(), "b".into(), 2.0); 2],
            },
            MisclassificationCosts {
                default_cost: 1.0,
                entries: vec![("a".into(), "b".into(), 2.0); MAX_NODES + 1],
            },
        ] {
            assert!(
                train_tree(
                    &dataset,
                    &C50Settings {
                        costs: Some(costs),
                        ..C50Settings::default()
                    }
                )
                .is_err()
            );
        }
        for mut settings in [
            C50Settings {
                rules: true,
                ..C50Settings::default()
            },
            C50Settings {
                boost_trials: Some(2),
                ..C50Settings::default()
            },
            C50Settings {
                weight_field: Some(0),
                ..C50Settings::default()
            },
        ] {
            settings.costs = Some(MisclassificationCosts {
                default_cost: 1.0,
                entries: vec![],
            });
            assert!(matches!(
                train_tree(&dataset, &settings),
                Err(AlgoError::Unsupported(_))
            ));
        }
    }

    #[test]
    fn unrepresentable_case_weight_totals_return_errors() {
        let original = sample();
        let mut fields = original.model().fields().to_vec();
        fields.push(Field::new("weight", Storage::Real));
        for weight in [f64::MAX, f64::MIN_POSITIVE / 1000.0] {
            let rows = original
                .rows()
                .iter()
                .map(|r| {
                    let mut row = r.clone();
                    row.push(Value::Real(weight));
                    row
                })
                .collect();
            let table = Table::new(DataModel::new(fields.clone()).unwrap(), rows).unwrap();
            assert!(
                train_tree(
                    &Dataset {
                        table: &table,
                        target: 1,
                        inputs: &[0],
                        frequency: None
                    },
                    &C50Settings {
                        weight_field: Some(2),
                        ..C50Settings::default()
                    }
                )
                .is_err()
            );
        }
    }
    #[test]
    fn malformed_settings_and_indices_return_errors() {
        let table = sample();
        for target in [2, usize::MAX] {
            assert!(
                train(
                    &Dataset {
                        table: &table,
                        target,
                        inputs: &[0],
                        frequency: None
                    },
                    &C50Settings::default()
                )
                .is_err()
            );
        }
        for severity in [f64::NAN, f64::INFINITY, -1.0, 100.0] {
            assert!(
                train(
                    &Dataset {
                        table: &table,
                        target: 1,
                        inputs: &[0],
                        frequency: None
                    },
                    &C50Settings {
                        pruning_severity: severity,
                        ..C50Settings::default()
                    }
                )
                .is_err()
            );
        }
        assert!(
            train(
                &Dataset {
                    table: &table,
                    target: 1,
                    inputs: &[usize::MAX],
                    frequency: None
                },
                &C50Settings::default()
            )
            .is_err()
        );
    }
}
