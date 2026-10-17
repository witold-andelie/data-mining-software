//! Clean-room C5.0 classification trees. Sources: IBM SPSS Modeler 19 Scripting
//! and Automation Guide pp.226–227; Algorithms Guide pp.1,51; PMML 4.3 TreeModel.
//! No See5/C5.0 implementation or Modeler executable was consulted.
//!
//! Evidence (`models_ml/` unless qualified):
//! - `c50_default`, `c50_expert`: information gain ratio, numeric midpoint cuts,
//!   multiway symbols; depth-first ids; flags true before false; largest default
//!   child (first on ties); parent prediction resolves tied class counts.
//! - `models/c50_default`: numeric cut description-length correction, statistical
//!   leaf pruning; all splits/counts/scores are checked by the corpus test.
//! - `c50_min_child`: at least two branches must reach min_child_records.
//! - `c50_group_symbols`: greedy gain-ratio grouping of symbolic categories.
//! - `c50_prune90`: pruning confidence is (100 - severity)/100.
//! - `c50_no_global_prune`: local pruning alone. GAP: pruning slack (.5 global,
//!   .1 local) is inferred from the two default trees; a low-support balanced
//!   exception probe varying total sample size would distinguish a global error
//!   budget from a fixed slack. Subtree raising is not established.
//! - Tree confidence is (class_count+1)/(node_count+target_categories), with zero
//!   for absent classes in the exported distribution; raw binary propensity uses
//!   confidence and its complement, including absent classes (`c50_partition`).
//!
//! Rejected settings (never silently substituted): RuleSet training
//! (`c50_rules_voting`, `c50_rules_firsthit`: GAP rule simplification/selection),
//! boosting/trials (`c50_boost_tree`, `c50_boost_rules`: GAP reweighting/stopping),
//! costs (`c50_costs`: GAP cost-dependent training weights), case/frequency
//! weights (`c50_weight`: GAP normalized effective counts), winnowing
//! (`c50_winnow`: GAP elimination criterion), Simple Generality/expected noise
//! (`c50_generality`, `c50_noise20`: GAP preset mapping), adjusted-propensity
//! training (`c50_partition`: GAP calibration pooling/rounding), cross-validation,
//! split-field training and missing/unknown training predictors (no captured
//! c50_missing oracle). PMML scoring supports rules and boosts independently.

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
}

impl Default for C50Settings {
    fn default() -> Self {
        Self {
            min_child_records: 2,
            pruning_severity: 75.0,
            global_pruning: true,
            group_symbolics: false,
        }
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
    },
}

struct Data {
    columns: Vec<Column>,
    classes: Vec<String>,
    targets: Vec<usize>,
}

impl Data {
    fn read(dataset: &Dataset<'_>) -> Result<Self, AlgoError> {
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
                });
            }
        }
        Ok(Self {
            columns,
            classes,
            targets,
        })
    }

    fn counts(&self, rows: &[usize]) -> Vec<f64> {
        let mut counts = vec![0.0; self.classes.len()];
        for &row in rows {
            if let Some(c) = self.targets.get(row).and_then(|&i| counts.get_mut(i)) {
                *c += 1.0;
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
    let total = rows.len() as f64;
    match column {
        Column::Numeric { name, values } => {
            let mut sorted: Vec<(f64, usize)> = rows
                .iter()
                .filter_map(|&r| {
                    values
                        .get(r)
                        .zip(data.targets.get(r))
                        .map(|(&v, &c)| (v, c))
                })
                .collect();
            sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut left = vec![0.0; data.classes.len()];
            let mut right = parent_counts;
            let mut best: Option<(f64, f64, usize)> = None;
            let mut cuts = 0usize;
            for (i, pair) in sorted.windows(2).enumerate() {
                let [(value, class), (next, _)] = pair else {
                    continue;
                };
                if let Some(c) = left.get_mut(*class) {
                    *c += 1.0;
                }
                if let Some(c) = right.get_mut(*class) {
                    *c -= 1.0;
                }
                let n = i.saturating_add(1);
                if value == next || n < min_child || sorted.len().saturating_sub(n) < min_child {
                    continue;
                }
                cuts = cuts.saturating_add(1);
                let gain = base
                    - n as f64 / total * entropy(&left)
                    - (total - n as f64) / total * entropy(&right);
                if best.is_none_or(|(g, _, _)| gain > g + 1e-12) {
                    let midpoint = value / 2.0 + next / 2.0;
                    let cut = if midpoint >= *next { *value } else { midpoint };
                    best = Some((gain, cut, n));
                }
            }
            let (gain, cut, n) = best?;
            let gain = gain - (cuts.max(1) as f64).log2() / total;
            let split_info = entropy(&[n as f64, total - n as f64]);
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
                .filter(|(_, rs)| rs.len() >= min_child)
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
            .map(|(_, rs)| rs.len() as f64 / total * entropy(&data.counts(rs)))
            .sum::<f64>();
    let info = entropy(
        &groups
            .iter()
            .map(|(_, rs)| rs.len() as f64)
            .collect::<Vec<_>>(),
    );
    (gain, if info > 0.0 { gain / info } else { 0.0 })
}

fn merge_groups(data: &Data, groups: &mut Vec<(Vec<usize>, Vec<usize>)>, grouping: bool) {
    if groups.len() > 64 {
        return;
    }
    let total = groups.iter().map(|(_, rs)| rs.len()).sum::<usize>() as f64;
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
                    let pure_a = ca.iter().position(|c| *c == a.len() as f64);
                    let pure_b = cb.iter().position(|c| *c == b.len() as f64);
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
    let class = winner(&counts, parent);
    let total = rows.len() as f64;
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
        || rows.len() < settings.min_child_records.saturating_mul(2)
    {
        return Ok(node);
    }
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
    let average = candidates.iter().map(|c| c.gain).sum::<f64>() / candidates.len() as f64;
    let mut best: Option<Split> = None;
    for split in candidates {
        if split.gain >= average - 1e-9
            && best.as_ref().is_none_or(|s| split.ratio > s.ratio + 1e-12)
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

/// One-sided Wilson upper bound with continuity correction; zero-error bound is
/// the exact binomial bound. Numerical inversion avoids an inverse-normal dependency.
fn error_bound(total: f64, errors: f64, confidence: f64) -> f64 {
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

fn prune(node: &mut Node, settings: &C50Settings) -> f64 {
    let total = node.record_count.unwrap_or(0.0);
    let correct = node
        .distribution
        .iter()
        .find(|d| Some(&d.value) == node.score.as_ref())
        .map_or(0.0, |d| d.record_count);
    let leaf = error_bound(
        total,
        total - correct,
        (100.0 - settings.pruning_severity) / 100.0,
    );
    if node.children.is_empty() {
        return leaf;
    }
    let subtree: f64 = node.children.iter_mut().map(|c| prune(c, settings)).sum();
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

pub fn train(dataset: &Dataset<'_>, settings: &C50Settings) -> Result<Pmml, AlgoError> {
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
    let data = Data::read(dataset)?;
    let rows: Vec<usize> = (0..data.targets.len()).collect();
    let mut root = grow(&data, &rows, settings, 0, 0, &mut 0)?;
    prune(&mut root, settings);
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
