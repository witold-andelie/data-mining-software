//! Neural Net training (`neuralnet!spss.neuralnet-extension`, multilayer perceptron) at
//! **statistical parity** (`docs/statistical-parity.md`): Modeler draws its initial weights and
//! holdout from an undisclosed generator, so we implement the Algorithms Guide ("Neural
//! Networks Algorithms") with our own seedable generator and accept the result when Modeler's
//! single run lies inside the band of our seeds (`crates/exec/tests/neural_training.rs`).
//!
//! Encoding (exact, read from Modeler's saved PMML `oracle/models_ml/nn_*`):
//! - Inputs and targets are listed by field name. A continuous field is standardized with the
//!   mean and the sample (n−1) standard deviation of the training records and written as
//!   `NormContinuous` through its minimum and maximum (`xNorm`: 0 → −1.562340901, 4.5 →
//!   1.562340901 on `nn_fixed_a`). A categorical or flag field becomes one indicator per
//!   category of its Type (`catValue0..2`, `flagValue0..1`), with no reference category.
//! - Records with a missing target are left out. Listwise deletion drops a record with any
//!   missing input (`nn_missing_listwise`); imputation replaces a missing continuous input by
//!   the midpoint of its observed range and a categorical one by its most frequent category,
//!   recorded as `MiningField@missingValueReplacement` (`nn_missing_impute`: x 2, z 1, cat
//!   blue, flag 0).
//!
//! Network and training (Guide, with the oracle case that shows each choice):
//! - Hidden layers use `tanh`. A custom architecture has `identity` outputs trained on the
//!   sum-of-squares error (`nn_fixed_a`, also with categorical targets; the scorer then uses
//!   rectified simplemax). The automatic architecture with only categorical targets has
//!   `softmax` outputs trained on cross-entropy (`nn_default_a`).
//! - Weights are trained in batch by Møller's scaled conjugate gradient with σ = 5e-5 and
//!   λ = 5e-7 (the saved `MLPTrainingInitialSigma`, `MLPTrainingInitialLambda`).
//! - Custom architectures start from the best of four uniform draws in [−0.5, 0.5]
//!   (`MLPTrainingIntervalCenter` 0, `MLPTrainingIntervalOffset` 0.5). Pre-training them by the
//!   alternated simulated annealing procedure moves our band to 0.84–0.98 training accuracy
//!   on `nn_fixed_a`, far above Modeler's 0.744, and to 0.82–0.98 on `nn_one_cycle` against its
//!   0.389, so Modeler does not pre-train them.
//! - The automatic architecture runs the Guide's expert selection: a 70/30 split, an initial
//!   `k = min(⌊(P+R)/2⌋, 20, h)` hidden units (`h`: the most units without more weights than
//!   records; the Guide's `g(R,P)` formula is not in the public text, ⌊(P+R)/2⌋ = 5 is
//!   `nn_default_a`'s choice), alternated simulated annealing (K1 = K2 = 4, K3 = 3), and the
//!   documented up/down search by test error.
//! - Stopping, after every successful step: the epoch cap (`MLPStoppingTrainingEpochsMaximum
//!   Number`; 250 when computed automatically), relative training-error change below 1e-4,
//!   training error ratio below 1e-3, and the minimum accuracy when enabled (`nn_accuracy`,
//!   on the overfit prevention set if any). With an overfit prevention set the weights of the
//!   lowest test error are reported. The Guide also stops once the test error has not fallen
//!   for one step; Modeler cannot have done so on `nn_overfit30` (0.883 training accuracy,
//!   which our runs reach only without that stop), so the set selects weights but does not
//!   end training.
//!
//! GAP: radial basis function networks, ensembles (bagging, boosting), model updates,
//! online/mini-batch training, gradient descent, other rescalings or activations, and a time
//! limit that would bind (ours trains in well under the saved 15 minutes) are refused by the
//! build node.

use crate::AlgoError;
use crate::common::{data_field, mining_field};
use dms_pmml::{
    Activation, ContinuousNorm, DerivedField, Expr, MiningFunction, Model, ModelHeader,
    NeuralInput, NeuralLayer, NeuralNetworkModel, NeuralOutput, NeuralOutputKind, Neuron,
    Normalization, OpType, Outliers, Pmml, Usage,
};
use dms_types::{Measurement, Table, Value, ValueSpec};

/// How missing input values are handled (`neuralnet.MissingValueHandling`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingValues {
    Listwise,
    Impute,
}

/// The hidden layers (`neuralnet.HiddenLayers`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hidden {
    /// Expert architecture selection of one hidden layer between `min` and `max` units.
    Automatic { min: usize, max: usize },
    /// One or two layers of the given sizes.
    Custom(Vec<usize>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NeuralSettings {
    pub hidden: Hidden,
    /// Epoch cap; `None` when Modeler computes it automatically.
    pub max_epochs: Option<usize>,
    /// Stop once this accuracy (percent) is reached.
    pub min_accuracy: Option<f64>,
    /// Share of the training records held out to select weights (0 to 1).
    pub overfit_share: f64,
    pub missing: MissingValues,
    pub seed: u64,
}

impl Default for NeuralSettings {
    fn default() -> Self {
        NeuralSettings {
            hidden: Hidden::Automatic { min: 1, max: 50 },
            max_epochs: None,
            min_accuracy: None,
            overfit_share: 0.3,
            missing: MissingValues::Listwise,
            seed: 12345,
        }
    }
}

/// The records and roles a network trains on; several targets are allowed.
pub struct NeuralData<'a> {
    pub table: &'a Table,
    pub targets: &'a [usize],
    pub inputs: &'a [usize],
}

const AUTOMATIC_EPOCHS: usize = 250;
const RELATIVE_CHANGE: f64 = 1e-4;
const ERROR_RATIO: f64 = 1e-3;
const SIGMA0: f64 = 5e-5;
const LAMBDA0: f64 = 5e-7;
const INIT_OFFSET: f64 = 0.5;
const MAX_UNITS: usize = 1000;
const MAX_WEIGHTS: usize = 1_000_000;

// ---------------------------------------------------------------------------------------------
// Our seedable generator (SplitMix64): Modeler's is undisclosed (statistical parity).

struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn uniform(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }
}

// ---------------------------------------------------------------------------------------------
// Encoding.

enum Column {
    Continuous {
        position: usize,
        name: String,
        mean: f64,
        sd: f64,
        min: f64,
        max: f64,
    },
    Categorical {
        position: usize,
        name: String,
        categories: Vec<String>,
    },
}

impl Column {
    fn width(&self) -> usize {
        match self {
            Column::Continuous { .. } => 1,
            Column::Categorical { categories, .. } => categories.len(),
        }
    }
}

fn categories_of(table: &Table, position: usize) -> Vec<String> {
    let Some(field) = table.model().fields().get(position) else {
        return Vec::new();
    };
    match &field.values {
        ValueSpec::Categories(values) => values
            .iter()
            .filter(|v| !field.is_missing(v))
            .map(Value::to_string)
            .collect(),
        ValueSpec::Flag {
            true_value,
            false_value,
        } => vec![false_value.to_string(), true_value.to_string()],
        _ => crate::common::field_categories(table, position),
    }
}

fn is_continuous(table: &Table, position: usize) -> bool {
    table
        .model()
        .fields()
        .get(position)
        .is_some_and(|f| f.measurement == Measurement::Continuous)
}

/// A cell as the network reads it: a number, a category text, or missing.
fn cell(table: &Table, row: &[Value], position: usize) -> Option<Value> {
    let field = table.model().fields().get(position)?;
    let value = row.get(position)?;
    if field.is_missing(value) {
        None
    } else {
        Some(value.clone())
    }
}

struct Encoded {
    inputs: Vec<Column>,
    targets: Vec<Column>,
    /// Imputed values per input (by input column index), as PMML replacement texts.
    replacements: Vec<Option<String>>,
    x: Vec<Vec<f64>>,
    y: Vec<Vec<f64>>,
    /// Target output ranges: `(start, width, categorical)`.
    groups: Vec<(usize, usize, bool)>,
}

fn sorted_by_name(table: &Table, positions: &[usize]) -> Vec<usize> {
    let mut out = positions.to_vec();
    out.sort_by(|a, b| {
        let name = |p: &usize| table.model().fields().get(*p).map(|f| f.name.clone());
        name(a).cmp(&name(b))
    });
    out
}

fn encode(data: &NeuralData<'_>, missing: MissingValues) -> Result<Encoded, AlgoError> {
    let table = data.table;
    let inputs = sorted_by_name(table, data.inputs);
    let targets = sorted_by_name(table, data.targets);
    if inputs.is_empty() || targets.is_empty() {
        return Err(AlgoError::Settings(
            "a neural network needs inputs and a target".into(),
        ));
    }
    // Records with every target present; inputs per the missing-value rule.
    let mut kept: Vec<&Vec<Value>> = table
        .rows()
        .iter()
        .filter(|row| targets.iter().all(|&t| cell(table, row, t).is_some()))
        .collect();
    let input_ok = |row: &Vec<Value>, p: usize, cats: &[String]| -> bool {
        match cell(table, row, p) {
            None => false,
            Some(v) if is_continuous(table, p) => v.as_f64().is_some_and(f64::is_finite),
            Some(v) => cats.iter().any(|c| *c == v.to_string()),
        }
    };
    let input_categories: Vec<Vec<String>> = inputs
        .iter()
        .map(|&p| {
            if is_continuous(table, p) {
                Vec::new()
            } else {
                categories_of(table, p)
            }
        })
        .collect();
    let mut replacements: Vec<Option<String>> = vec![None; inputs.len()];
    let mut imputed: Vec<Option<Value>> = vec![None; inputs.len()];
    match missing {
        MissingValues::Listwise => kept.retain(|row| {
            inputs
                .iter()
                .zip(&input_categories)
                .all(|(&p, cats)| input_ok(row, p, cats))
        }),
        MissingValues::Impute => {
            for (i, (&p, cats)) in inputs.iter().zip(&input_categories).enumerate() {
                let present: Vec<&Vec<Value>> = kept
                    .iter()
                    .copied()
                    .filter(|row| input_ok(row, p, cats))
                    .collect();
                if present.is_empty() {
                    return Err(AlgoError::Data(format!(
                        "input {} has no valid value to impute from",
                        table.model().fields()[p].name
                    )));
                }
                if is_continuous(table, p) {
                    let values = present
                        .iter()
                        .filter_map(|row| row.get(p).and_then(Value::as_f64));
                    let (low, high) = crate::common::value_range(values);
                    let mid = (low + high) / 2.0;
                    replacements[i] = Some(crate::common::number_text(mid));
                    imputed[i] = Some(Value::Real(mid));
                } else {
                    let mut counts = vec![0usize; cats.len()];
                    for row in &present {
                        if let Some(v) = row.get(p)
                            && let Some(k) = cats.iter().position(|c| *c == v.to_string())
                        {
                            counts[k] += 1;
                        }
                    }
                    // The first most frequent category in Type order.
                    let mut best = 0;
                    for (k, &n) in counts.iter().enumerate() {
                        if n > counts[best] {
                            best = k;
                        }
                    }
                    replacements[i] = cats.get(best).cloned();
                    imputed[i] = cats.get(best).map(|c| Value::string(c.as_str()));
                }
            }
        }
    }
    if kept.len() < 2 {
        return Err(AlgoError::Data(
            "a neural network needs at least two complete training records".into(),
        ));
    }
    let value_at = |row: &Vec<Value>, i: usize| -> Option<Value> {
        let p = inputs[i];
        if input_ok(row, p, &input_categories[i]) {
            row.get(p).cloned()
        } else {
            imputed[i].clone()
        }
    };
    let column = |p: usize, values: Vec<Value>, cats: Vec<String>| -> Result<Column, AlgoError> {
        let name = table.model().fields()[p].name.clone();
        if is_continuous(table, p) {
            let xs: Vec<f64> = values.iter().filter_map(Value::as_f64).collect();
            let n = xs.len() as f64;
            let mean = xs.iter().sum::<f64>() / n;
            let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (n - 1.0);
            let sd = var.sqrt();
            if !(sd.is_finite() && sd > 0.0) {
                return Err(AlgoError::Data(format!(
                    "continuous field {name} is constant on the training records"
                )));
            }
            let (min, max) = crate::common::value_range(xs.iter().copied());
            Ok(Column::Continuous {
                position: p,
                name,
                mean,
                sd,
                min,
                max,
            })
        } else {
            if cats.is_empty() {
                return Err(AlgoError::Data(format!(
                    "categorical field {name} has no categories"
                )));
            }
            Ok(Column::Categorical {
                position: p,
                name,
                categories: cats,
            })
        }
    };
    let mut input_columns = Vec::with_capacity(inputs.len());
    for (i, &p) in inputs.iter().enumerate() {
        let values: Vec<Value> = kept.iter().filter_map(|row| value_at(row, i)).collect();
        input_columns.push(column(p, values, input_categories[i].clone())?);
    }
    let mut target_columns = Vec::with_capacity(targets.len());
    for &p in &targets {
        let values: Vec<Value> = kept.iter().filter_map(|row| row.get(p).cloned()).collect();
        let cats = if is_continuous(table, p) {
            Vec::new()
        } else {
            categories_of(table, p)
        };
        target_columns.push(column(p, values, cats)?);
    }
    let encode_value =
        |column: &Column, value: Option<&Value>, out: &mut Vec<f64>| -> Result<(), AlgoError> {
            match column {
                Column::Continuous { mean, sd, name, .. } => {
                    let x = value.and_then(Value::as_f64).ok_or_else(|| {
                        AlgoError::Data(format!("{name}: a training value is not numeric"))
                    })?;
                    out.push((x - mean) / sd);
                }
                Column::Categorical {
                    categories, name, ..
                } => {
                    let text = value.map(Value::to_string).ok_or_else(|| {
                        AlgoError::Data(format!("{name}: a training value is missing"))
                    })?;
                    if !categories.contains(&text) {
                        return Err(AlgoError::Data(format!(
                            "{name}: the training value {text:?} is not a category of its Type"
                        )));
                    }
                    out.extend(
                        categories
                            .iter()
                            .map(|c| if *c == text { 1.0 } else { 0.0 }),
                    );
                }
            }
            Ok(())
        };
    let mut x = Vec::with_capacity(kept.len());
    let mut y = Vec::with_capacity(kept.len());
    for row in &kept {
        let mut xi = Vec::new();
        for (i, column) in input_columns.iter().enumerate() {
            encode_value(column, value_at(row, i).as_ref(), &mut xi)?;
        }
        let mut yi = Vec::new();
        for column in &target_columns {
            let p = match column {
                Column::Continuous { position, .. } | Column::Categorical { position, .. } => {
                    *position
                }
            };
            encode_value(column, row.get(p), &mut yi)?;
        }
        x.push(xi);
        y.push(yi);
    }
    let mut groups = Vec::new();
    let mut start = 0;
    for column in &target_columns {
        let width = column.width();
        groups.push((start, width, matches!(column, Column::Categorical { .. })));
        start += width;
    }
    Ok(Encoded {
        inputs: input_columns,
        targets: target_columns,
        replacements,
        x,
        y,
        groups,
    })
}

// ---------------------------------------------------------------------------------------------
// The network.

#[derive(Clone)]
struct Net {
    /// Units per layer, inputs first.
    sizes: Vec<usize>,
    softmax: bool,
}

impl Net {
    fn weight_count(&self) -> usize {
        self.sizes.windows(2).map(|w| w[1] * (w[0] + 1)).sum()
    }

    /// Activations of every layer for one record.
    fn forward(&self, w: &[f64], x: &[f64]) -> Vec<Vec<f64>> {
        let mut acts = vec![x.to_vec()];
        let mut offset = 0;
        let layers = self.sizes.len() - 1;
        for l in 0..layers {
            let (fan_in, units) = (self.sizes[l], self.sizes[l + 1]);
            let prev = &acts[l];
            let mut out = Vec::with_capacity(units);
            for u in 0..units {
                let base = offset + u * (fan_in + 1);
                let mut z = w[base];
                for (k, a) in prev.iter().enumerate() {
                    z += w[base + 1 + k] * a;
                }
                out.push(if l + 1 < layers { z.tanh() } else { z });
            }
            offset += units * (fan_in + 1);
            acts.push(out);
        }
        if self.softmax
            && let Some(last) = acts.last_mut()
        {
            let max = last.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let sum: f64 = last.iter().map(|z| (z - max).exp()).sum();
            for z in last.iter_mut() {
                *z = (*z - max).exp() / sum;
            }
        }
        acts
    }

    /// The error over the records and its gradient.
    fn error_gradient(&self, w: &[f64], xs: &[&Vec<f64>], ys: &[&Vec<f64>]) -> (f64, Vec<f64>) {
        let mut grad = vec![0.0; w.len()];
        let mut error = 0.0;
        let layers = self.sizes.len() - 1;
        for (x, y) in xs.iter().zip(ys) {
            let acts = self.forward(w, x);
            let out = &acts[layers];
            let mut delta: Vec<f64> = out.iter().zip(y.iter()).map(|(o, t)| o - t).collect();
            if self.softmax {
                error -= out
                    .iter()
                    .zip(y.iter())
                    .map(|(o, t)| t * o.max(1e-300).ln())
                    .sum::<f64>();
            } else {
                error += 0.5 * delta.iter().map(|d| d * d).sum::<f64>();
            }
            // Backpropagation from the output layer.
            let mut offsets = Vec::with_capacity(layers);
            let mut offset = 0;
            for l in 0..layers {
                offsets.push(offset);
                offset += self.sizes[l + 1] * (self.sizes[l] + 1);
            }
            for l in (0..layers).rev() {
                let (fan_in, units) = (self.sizes[l], self.sizes[l + 1]);
                let prev = &acts[l];
                let mut next = vec![0.0; fan_in];
                for (u, &d) in delta.iter().enumerate().take(units) {
                    let base = offsets[l] + u * (fan_in + 1);
                    grad[base] += d;
                    for k in 0..fan_in {
                        grad[base + 1 + k] += d * prev[k];
                        next[k] += d * w[base + 1 + k];
                    }
                }
                if l > 0 {
                    delta = next
                        .iter()
                        .zip(prev.iter())
                        .map(|(s, a)| s * (1.0 - a * a))
                        .collect();
                }
            }
        }
        (error, grad)
    }

    fn error(&self, w: &[f64], xs: &[&Vec<f64>], ys: &[&Vec<f64>]) -> f64 {
        self.error_gradient(w, xs, ys).0
    }
}

/// The error of predicting every output by its mean (the Guide's error-ratio denominator).
fn null_error(net: &Net, ys: &[&Vec<f64>]) -> f64 {
    let Some(first) = ys.first() else {
        return 0.0;
    };
    let n = ys.len() as f64;
    let mut mean = vec![0.0; first.len()];
    for y in ys {
        for (m, v) in mean.iter_mut().zip(y.iter()) {
            *m += v / n;
        }
    }
    if net.softmax {
        -ys.iter()
            .map(|y| {
                y.iter()
                    .zip(&mean)
                    .map(|(t, p)| t * p.max(1e-300).ln())
                    .sum::<f64>()
            })
            .sum::<f64>()
    } else {
        0.5 * ys
            .iter()
            .map(|y| {
                y.iter()
                    .zip(&mean)
                    .map(|(t, m)| (t - m) * (t - m))
                    .sum::<f64>()
            })
            .sum::<f64>()
    }
}

/// Percentage of records whose categorical targets are all predicted right; for continuous
/// targets alone, 100 R².
fn accuracy(
    net: &Net,
    w: &[f64],
    xs: &[&Vec<f64>],
    ys: &[&Vec<f64>],
    groups: &[(usize, usize, bool)],
) -> f64 {
    let layers = net.sizes.len() - 1;
    let categorical: Vec<_> = groups.iter().filter(|g| g.2).collect();
    if categorical.is_empty() {
        let null = null_error(
            &Net {
                sizes: net.sizes.clone(),
                softmax: false,
            },
            ys,
        );
        let sse = 0.5
            * xs.iter()
                .zip(ys)
                .map(|(x, y)| {
                    let acts = net.forward(w, x);
                    acts[layers]
                        .iter()
                        .zip(y.iter())
                        .map(|(o, t)| (o - t) * (o - t))
                        .sum::<f64>()
                })
                .sum::<f64>();
        return if null > 0.0 {
            100.0 * (1.0 - sse / null)
        } else {
            0.0
        };
    }
    let right = xs
        .iter()
        .zip(ys)
        .filter(|(x, y)| {
            let acts = net.forward(w, x);
            let out = &acts[layers];
            categorical.iter().all(|&&(start, width, _)| {
                argmax(&out[start..start + width]) == argmax(&y[start..start + width])
            })
        })
        .count();
    100.0 * right as f64 / xs.len().max(1) as f64
}

fn argmax(values: &[f64]) -> usize {
    let mut best = 0;
    for (k, v) in values.iter().enumerate() {
        if *v > values[best] {
            best = k;
        }
    }
    best
}

struct Split<'a> {
    xs: Vec<&'a Vec<f64>>,
    ys: Vec<&'a Vec<f64>>,
}

struct Stopping {
    max_epochs: usize,
    min_accuracy: Option<f64>,
}

/// Møller's scaled conjugate gradient with the Guide's stopping rules.
fn scg(
    net: &Net,
    mut w: Vec<f64>,
    train: &Split<'_>,
    test: Option<&Split<'_>>,
    stop: &Stopping,
    groups: &[(usize, usize, bool)],
) -> Vec<f64> {
    let n = w.len();
    let null = null_error(net, &train.ys);
    let (mut e, mut g) = net.error_gradient(&w, &train.xs, &train.ys);
    let mut r: Vec<f64> = g.iter().map(|v| -v).collect();
    let mut p = r.clone();
    let mut success = true;
    let mut lambda = LAMBDA0;
    let mut lambda_bar = 0.0;
    let mut delta = 0.0;
    let mut p_norm2 = 0.0;
    let mut best_test = test.map(|t| (net.error(&w, &t.xs, &t.ys), w.clone()));
    let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    for k in 1..=stop.max_epochs {
        if success {
            p_norm2 = dot(&p, &p);
            if p_norm2 <= 0.0 || !p_norm2.is_finite() {
                break;
            }
            let sigma = SIGMA0 / p_norm2.sqrt();
            let probe: Vec<f64> = w.iter().zip(&p).map(|(a, b)| a + sigma * b).collect();
            let (_, g2) = net.error_gradient(&probe, &train.xs, &train.ys);
            let s: Vec<f64> = g2.iter().zip(&g).map(|(a, b)| (a - b) / sigma).collect();
            delta = dot(&p, &s);
        }
        delta += (lambda - lambda_bar) * p_norm2;
        if delta <= 0.0 {
            lambda_bar = 2.0 * (lambda - delta / p_norm2);
            delta = -delta + lambda * p_norm2;
            lambda = lambda_bar;
        }
        let mu = dot(&p, &r);
        if mu == 0.0 || !mu.is_finite() {
            break;
        }
        let alpha = mu / delta;
        let candidate: Vec<f64> = w.iter().zip(&p).map(|(a, b)| a + alpha * b).collect();
        let (e_new, g_new) = net.error_gradient(&candidate, &train.xs, &train.ys);
        let comparison = 2.0 * delta * (e - e_new) / (mu * mu);
        let previous = e;
        if comparison >= 0.0 && e_new.is_finite() {
            w = candidate;
            e = e_new;
            g = g_new;
            let r_new: Vec<f64> = g.iter().map(|v| -v).collect();
            lambda_bar = 0.0;
            success = true;
            if k % n == 0 {
                p = r_new.clone();
            } else {
                let beta = (dot(&r_new, &r_new) - dot(&r_new, &r)) / mu;
                p = r_new.iter().zip(&p).map(|(a, b)| a + beta * b).collect();
            }
            r = r_new;
            if comparison >= 0.75 {
                lambda /= 4.0;
            }
        } else {
            lambda_bar = lambda;
            success = false;
        }
        if comparison < 0.25 {
            lambda += delta * (1.0 - comparison) / p_norm2;
        }
        if !success {
            continue;
        }
        if let (Some(t), Some(best)) = (test, best_test.as_mut()) {
            let err = net.error(&w, &t.xs, &t.ys);
            if err < best.0 {
                *best = (err, w.clone());
            }
        }
        if let Some(target) = stop.min_accuracy {
            let reached = match test {
                Some(t) => accuracy(net, &w, &t.xs, &t.ys, groups),
                None => accuracy(net, &w, &train.xs, &train.ys, groups),
            };
            if reached >= target {
                return w;
            }
        }
        if previous.abs() > 0.0 && ((previous - e) / previous).abs() < RELATIVE_CHANGE {
            break;
        }
        if null > 0.0 && e / null < ERROR_RATIO {
            break;
        }
    }
    match best_test {
        Some((_, best)) => best,
        None => w,
    }
}

fn random_weights(net: &Net, rng: &mut Rng) -> Vec<f64> {
    (0..net.weight_count())
        .map(|_| rng.uniform(-INIT_OFFSET, INIT_OFFSET))
        .collect()
}

/// The best of `draws` uniform weight vectors by training error.
fn best_draw(net: &Net, train: &Split<'_>, rng: &mut Rng, draws: usize) -> Vec<f64> {
    let mut best: Option<(f64, Vec<f64>)> = None;
    for _ in 0..draws {
        let w = random_weights(net, rng);
        let e = net.error(&w, &train.xs, &train.ys);
        if best.as_ref().is_none_or(|(b, _)| e < *b) {
            best = Some((e, w));
        }
    }
    best.map(|(_, w)| w).unwrap_or_default()
}

/// Alternated simulated annealing and training (K1 = K2 = 4, K3 = 3) from `start`, or from the
/// best of K2 draws. The perturbation range shrinks linearly with k1 (the Guide's `a(k1)` is
/// not in the public text).
fn annealed(
    net: &Net,
    start: Option<Vec<f64>>,
    train: &Split<'_>,
    rng: &mut Rng,
    groups: &[(usize, usize, bool)],
) -> Vec<f64> {
    const K1: usize = 4;
    const K2: usize = 4;
    const K3: usize = 3;
    let null = null_error(net, &train.ys);
    let ratio = |e: f64| if null > 0.0 { e / null } else { 0.0 };
    let stop = Stopping {
        max_epochs: AUTOMATIC_EPOCHS,
        min_accuracy: None,
    };
    let mut results: Vec<(f64, Vec<f64>)> = Vec::new();
    for round in 0..K3 {
        let mut w = match (&start, round) {
            (Some(s), 0) => s.clone(),
            _ => best_draw(net, train, rng, K2),
        };
        let mut k1 = 0;
        loop {
            w = scg(net, w, train, None, &stop, groups);
            let e = net.error(&w, &train.xs, &train.ys);
            if ratio(e) <= 0.05 {
                break;
            }
            k1 += 1;
            if k1 >= K1 {
                break;
            }
            let a = INIT_OFFSET * (K1 - k1) as f64 / K1 as f64;
            let mut best: Option<(f64, Vec<f64>)> = None;
            for _ in 0..K2 {
                let candidate: Vec<f64> = w.iter().map(|v| v + rng.uniform(-a, a)).collect();
                let ce = net.error(&candidate, &train.xs, &train.ys);
                if best.as_ref().is_none_or(|(b, _)| ce < *b) {
                    best = Some((ce, candidate));
                }
            }
            match best {
                Some((ce, candidate)) if ce < e => w = candidate,
                _ => break,
            }
        }
        let e = net.error(&w, &train.xs, &train.ys);
        results.push((e, w));
        if ratio(e) <= 0.1 {
            break;
        }
    }
    results
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, w)| w)
        .unwrap_or_default()
}

/// The network without hidden unit `unit` of its single hidden layer.
fn without_unit(net: &Net, w: &[f64], unit: usize) -> (Net, Vec<f64>) {
    let (inputs, hidden, outputs) = (net.sizes[0], net.sizes[1], net.sizes[2]);
    let mut out = Vec::new();
    for u in 0..hidden {
        if u != unit {
            out.extend_from_slice(&w[u * (inputs + 1)..(u + 1) * (inputs + 1)]);
        }
    }
    let base = hidden * (inputs + 1);
    for o in 0..outputs {
        let row = &w[base + o * (hidden + 1)..base + (o + 1) * (hidden + 1)];
        out.push(row[0]);
        for (u, v) in row[1..].iter().enumerate() {
            if u != unit {
                out.push(*v);
            }
        }
    }
    (
        Net {
            sizes: vec![inputs, hidden - 1, outputs],
            softmax: net.softmax,
        },
        out,
    )
}

/// The network with one more hidden unit, its new weights drawn uniformly.
fn with_unit(net: &Net, w: &[f64], rng: &mut Rng) -> (Net, Vec<f64>) {
    let (inputs, hidden, outputs) = (net.sizes[0], net.sizes[1], net.sizes[2]);
    let base = hidden * (inputs + 1);
    let mut out = w[..base].to_vec();
    out.extend((0..=inputs).map(|_| rng.uniform(-INIT_OFFSET, INIT_OFFSET)));
    for o in 0..outputs {
        out.extend_from_slice(&w[base + o * (hidden + 1)..base + (o + 1) * (hidden + 1)]);
        out.push(rng.uniform(-INIT_OFFSET, INIT_OFFSET));
    }
    (
        Net {
            sizes: vec![inputs, hidden + 1, outputs],
            softmax: net.softmax,
        },
        out,
    )
}

/// Expert architecture selection (Guide): one hidden layer, searched down or up by test error.
fn select_architecture(
    inputs: usize,
    outputs: usize,
    softmax: bool,
    (min, max): (usize, usize),
    records: &Split<'_>,
    rng: &mut Rng,
    groups: &[(usize, usize, bool)],
) -> (Net, Vec<f64>) {
    // 70/30 split of the training sample (at most 1000 records).
    let mut train = Split {
        xs: Vec::new(),
        ys: Vec::new(),
    };
    let mut test = Split {
        xs: Vec::new(),
        ys: Vec::new(),
    };
    for (x, y) in records.xs.iter().zip(&records.ys).take(1000) {
        if rng.unit() < 0.3 {
            test.xs.push(*x);
            test.ys.push(*y);
        } else {
            train.xs.push(*x);
            train.ys.push(*y);
        }
    }
    if test.xs.is_empty() || train.xs.is_empty() {
        test = Split {
            xs: records.xs.clone(),
            ys: records.ys.clone(),
        };
        train = Split {
            xs: records.xs.clone(),
            ys: records.ys.clone(),
        };
    }
    let m = records.xs.len();
    let most = m.saturating_sub(outputs) / (inputs + outputs + 1);
    let k0 = ((inputs + outputs) / 2)
        .min(20)
        .min(most)
        .clamp(min.max(1), max.max(1));
    let net0 = Net {
        sizes: vec![inputs, k0, outputs],
        softmax,
    };
    let w0 = annealed(&net0, None, &train, rng, groups);
    let null = null_error(&net0, &train.ys);
    let ratio = |net: &Net, w: &[f64]| {
        if null > 0.0 {
            net.error(w, &train.xs, &train.ys) / null
        } else {
            0.0
        }
    };
    let test_error = |net: &Net, w: &[f64]| net.error(w, &test.xs, &test.ys);
    let mut best = (test_error(&net0, &w0), net0.clone(), w0.clone());
    let mut tried: Vec<usize> = vec![k0];
    let mut down = k0 > min;
    if !down && ratio(&net0, &w0) <= 0.01 {
        return (best.1, best.2);
    }
    let (mut net, mut w, mut previous) = (net0.clone(), w0.clone(), best.0);
    loop {
        let (next_net, start) = if down {
            // Remove the hidden unit whose removal leaves the smallest test error.
            let hidden = net.sizes[1];
            let mut weakest: Option<(f64, Net, Vec<f64>)> = None;
            for unit in 0..hidden {
                let (smaller, sw) = without_unit(&net, &w, unit);
                let e = test_error(&smaller, &sw);
                if weakest.as_ref().is_none_or(|(b, _, _)| e < *b) {
                    weakest = Some((e, smaller, sw));
                }
            }
            match weakest {
                Some((_, n, sw)) => (n, sw),
                None => break,
            }
        } else {
            with_unit(&net, &w, rng)
        };
        let k = next_net.sizes[1];
        if tried.contains(&k) || k < min.max(1) || k > max {
            break;
        }
        tried.push(k);
        let trained = annealed(&next_net, Some(start), &train, rng, groups);
        let e = test_error(&next_net, &trained);
        if e < best.0 || (e == best.0 && k < best.1.sizes[1]) {
            best = (e, next_net.clone(), trained.clone());
        }
        if e < previous {
            let keep_going = if down {
                k > min.max(1)
            } else {
                k < max && ratio(&next_net, &trained) > 0.01
            };
            previous = e;
            net = next_net;
            w = trained;
            if keep_going {
                continue;
            }
            break;
        }
        if down && k + 1 == k0 && ratio(&net0, &w0) > 0.01 {
            // The first step down failed: search upward from the initial network.
            down = false;
            net = net0.clone();
            w = w0.clone();
            previous = test_error(&net0, &w0);
            continue;
        }
        break;
    }
    (best.1, best.2)
}

/// Train a multilayer perceptron and write it as Modeler's `NeuralNetwork` PMML.
pub fn train(data: &NeuralData<'_>, settings: &NeuralSettings) -> Result<Pmml, AlgoError> {
    let encoded = encode(data, settings.missing)?;
    let inputs = encoded.x.first().map_or(0, Vec::len);
    let outputs = encoded.y.first().map_or(0, Vec::len);
    let all_categorical = encoded.groups.iter().all(|g| g.2);
    let automatic = matches!(settings.hidden, Hidden::Automatic { .. });
    let softmax = automatic && all_categorical;
    if let Hidden::Custom(layers) = &settings.hidden
        && (layers.is_empty()
            || layers.len() > 2
            || layers.iter().any(|&u| u == 0 || u > MAX_UNITS))
    {
        return Err(AlgoError::Settings(
            "a custom architecture needs one or two hidden layers of 1 to 1000 units".into(),
        ));
    }
    let mut rng = Rng(settings.seed);
    // The overfit prevention set.
    let mut train = Split {
        xs: Vec::new(),
        ys: Vec::new(),
    };
    let mut test = Split {
        xs: Vec::new(),
        ys: Vec::new(),
    };
    for (x, y) in encoded.x.iter().zip(&encoded.y) {
        if settings.overfit_share > 0.0 && rng.unit() < settings.overfit_share {
            test.xs.push(x);
            test.ys.push(y);
        } else {
            train.xs.push(x);
            train.ys.push(y);
        }
    }
    if train.xs.is_empty() {
        return Err(AlgoError::Data(
            "the overfit prevention set took every record".into(),
        ));
    }
    let test = (!test.xs.is_empty()).then_some(test);
    let (net, start) = match &settings.hidden {
        Hidden::Custom(layers) => {
            let mut sizes = vec![inputs];
            sizes.extend(layers);
            sizes.push(outputs);
            let net = Net { sizes, softmax };
            if net.weight_count() > MAX_WEIGHTS {
                return Err(AlgoError::Settings(
                    "the network has too many weights".into(),
                ));
            }
            let w = best_draw(&net, &train, &mut rng, 4);
            (net, w)
        }
        Hidden::Automatic { min, max } => select_architecture(
            inputs,
            outputs,
            softmax,
            (*min, (*max).min(MAX_UNITS)),
            &train,
            &mut rng,
            &encoded.groups,
        ),
    };
    let stop = Stopping {
        max_epochs: settings.max_epochs.unwrap_or(AUTOMATIC_EPOCHS),
        min_accuracy: settings.min_accuracy,
    };
    let weights = scg(&net, start, &train, test.as_ref(), &stop, &encoded.groups);
    Ok(pmml(data.table, &encoded, &net, &weights, settings.missing))
}

// ---------------------------------------------------------------------------------------------
// PMML in Modeler's shape (`oracle/models_ml/nn_fixed_a/models/01_models_ml.xml`).

fn derived_names(column: &Column) -> Vec<String> {
    match column {
        Column::Continuous { name, .. } => vec![format!("{name}Norm")],
        Column::Categorical {
            name, categories, ..
        } => (0..categories.len())
            .map(|k| format!("{name}Value{k}"))
            .collect(),
    }
}

fn derived_fields(column: &Column) -> Vec<DerivedField> {
    match column {
        Column::Continuous {
            name,
            mean,
            sd,
            min,
            max,
            ..
        } => vec![DerivedField {
            name: format!("{name}Norm"),
            data_type: dms_pmml::DataType::Double,
            optype: OpType::Continuous,
            expr: Expr::NormContinuous {
                field: name.clone(),
                points: vec![(*min, (min - mean) / sd), (*max, (max - mean) / sd)],
                outliers: Outliers::AsIs,
                map_missing_to: None,
            },
        }],
        Column::Categorical {
            name, categories, ..
        } => categories
            .iter()
            .enumerate()
            .map(|(k, c)| DerivedField {
                name: format!("{name}Value{k}"),
                data_type: dms_pmml::DataType::Double,
                optype: OpType::Categorical,
                expr: Expr::NormDiscrete {
                    field: name.clone(),
                    value: c.clone(),
                    map_missing_to: None,
                },
            })
            .collect(),
    }
}

fn pmml(table: &Table, encoded: &Encoded, net: &Net, w: &[f64], missing: MissingValues) -> Pmml {
    let position = |c: &Column| match c {
        Column::Continuous { position, .. } | Column::Categorical { position, .. } => *position,
    };
    let mut data_dictionary = Vec::new();
    for column in encoded.inputs.iter().chain(&encoded.targets) {
        if let Some(mut field) = data_field(table, position(column)) {
            if let Some(f) = table.model().fields().get(position(column)) {
                field.missing_values = f.missing.values.iter().map(Value::to_string).collect();
            }
            data_dictionary.push(field);
        }
    }
    let mut transformations: Vec<DerivedField> = encoded
        .inputs
        .iter()
        .chain(&encoded.targets)
        .flat_map(derived_fields)
        .collect();
    transformations.sort_by(|a, b| a.name.cmp(&b.name));
    let mut mining_schema = Vec::new();
    for (column, replacement) in encoded.inputs.iter().zip(&encoded.replacements) {
        if let Some(mut field) = mining_field(table, position(column), Usage::Active) {
            if missing == MissingValues::Impute {
                field.missing_replacement = replacement.clone();
            }
            mining_schema.push(field);
        }
    }
    for column in &encoded.targets {
        if let Some(field) = mining_field(table, position(column), Usage::Target) {
            mining_schema.push(field);
        }
    }
    let input_names: Vec<String> = encoded.inputs.iter().flat_map(derived_names).collect();
    let inputs: Vec<NeuralInput> = input_names
        .iter()
        .enumerate()
        .map(|(k, name)| NeuralInput {
            id: k.to_string(),
            expr: Expr::FieldRef {
                field: name.clone(),
                map_missing_to: None,
            },
        })
        .collect();
    let mut layers = Vec::new();
    let mut previous: Vec<String> = (0..inputs.len()).map(|k| k.to_string()).collect();
    let mut next_id = inputs.len();
    let mut offset = 0;
    let count = net.sizes.len() - 1;
    for l in 0..count {
        let (fan_in, units) = (net.sizes[l], net.sizes[l + 1]);
        let last = l + 1 == count;
        let activation = if last {
            Activation::Identity
        } else {
            Activation::Tanh
        };
        let mut neurons = Vec::with_capacity(units);
        let mut ids = Vec::with_capacity(units);
        for u in 0..units {
            let base = offset + u * (fan_in + 1);
            let id = next_id.to_string();
            next_id += 1;
            neurons.push(Neuron {
                id: id.clone(),
                bias: w[base],
                activation,
                connections: previous
                    .iter()
                    .enumerate()
                    .map(|(k, from)| (from.clone(), w[base + 1 + k]))
                    .collect(),
                width: None,
                altitude: 1.0,
            });
            ids.push(id);
        }
        offset += units * (fan_in + 1);
        layers.push(NeuralLayer {
            activation,
            normalization: if last && net.softmax {
                Normalization::Softmax
            } else {
                Normalization::None
            },
            neurons,
        });
        previous = ids;
    }
    let mut outputs = Vec::new();
    let mut index = 0;
    for column in &encoded.targets {
        match column {
            Column::Continuous {
                name,
                mean,
                sd,
                min,
                max,
                ..
            } => {
                outputs.push(NeuralOutput {
                    neuron_id: previous.get(index).cloned().unwrap_or_default(),
                    neuron_index: index,
                    target: name.clone(),
                    kind: NeuralOutputKind::Continuous(ContinuousNorm {
                        points: vec![(*min, (min - mean) / sd), (*max, (max - mean) / sd)],
                        outliers: Outliers::AsIs,
                        map_missing_to: None,
                    }),
                });
                index += 1;
            }
            Column::Categorical {
                name, categories, ..
            } => {
                for category in categories {
                    outputs.push(NeuralOutput {
                        neuron_id: previous.get(index).cloned().unwrap_or_default(),
                        neuron_index: index,
                        target: name.clone(),
                        kind: NeuralOutputKind::Category(category.clone()),
                    });
                    index += 1;
                }
            }
        }
    }
    let classification = encoded
        .targets
        .iter()
        .all(|c| matches!(c, Column::Categorical { .. }));
    Pmml {
        version: "4.2".into(),
        data_dictionary,
        transformations,
        functions: Vec::new(),
        model: Model::Neural(NeuralNetworkModel {
            header: ModelHeader {
                name: None,
                function: if classification {
                    MiningFunction::Classification
                } else {
                    MiningFunction::Regression
                },
                algorithm: Some("MLP".into()),
                mining_schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            activation: Activation::Tanh,
            inputs,
            layers,
            outputs,
            adjusted_propensity: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_matches_finite_differences() {
        let net = Net {
            sizes: vec![3, 4, 2, 3],
            softmax: false,
        };
        let mut rng = Rng(7);
        let w = random_weights(&net, &mut rng);
        let xs = [vec![0.3, -1.0, 0.5], vec![1.2, 0.1, -0.4]];
        let ys = [vec![1.0, 0.0, 0.0], vec![0.0, 0.0, 1.0]];
        let x: Vec<&Vec<f64>> = xs.iter().collect();
        let y: Vec<&Vec<f64>> = ys.iter().collect();
        for softmax in [false, true] {
            let net = Net {
                softmax,
                ..net.clone()
            };
            let (_, g) = net.error_gradient(&w, &x, &y);
            for (i, gi) in g.iter().enumerate() {
                let mut up = w.clone();
                up[i] += 1e-6;
                let mut down = w.clone();
                down[i] -= 1e-6;
                let numeric = (net.error(&up, &x, &y) - net.error(&down, &x, &y)) / 2e-6;
                assert!((numeric - gi).abs() < 1e-5, "weight {i}: {numeric} vs {gi}");
            }
        }
    }

    #[test]
    fn generator_is_reproducible_and_uniform() {
        let mut a = Rng(12345);
        let mut b = Rng(12345);
        let draws: Vec<f64> = (0..1000).map(|_| a.unit()).collect();
        assert!(
            draws
                .iter()
                .zip((0..1000).map(|_| b.unit()))
                .all(|(x, y)| *x == y)
        );
        let mean = draws.iter().sum::<f64>() / 1000.0;
        assert!((mean - 0.5).abs() < 0.05, "{mean}");
        assert!(draws.iter().all(|d| (0.0..1.0).contains(d)));
    }
}
