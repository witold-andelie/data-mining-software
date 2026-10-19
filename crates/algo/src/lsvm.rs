//! Linear SVM (`spss.linearSVM!linearSVM`, Analytic Engine; Algorithms Guide "Linear Support
//! Vector Machine (LSVM) Algorithms", eq. 2.2 / 2.6).
//!
//! **Statistical parity** (`docs/statistical-parity.md`, rule 3), not exact parity. Modeler
//! solves the objective below with an ADMM that stops before the optimum, by an undocumented
//! rule (`oracle/models_as/lsvm_default`: objective 0.868048 against the optimum 0.867945;
//! `lsvm_numeric`: 0.556714 against 0.550353; `lsvm_numeric` intercept -0.0155 against the
//! optimum's -0.0359). We solve the documented objective to convergence instead, so our
//! coefficients are the optimum and are reported, never compared, against Modeler's. What is
//! compared (`crates/exec/tests/lsvm.rs`) is the objective of both coefficient vectors on the
//! same design, the predicted classes on every scored oracle record and the continuous
//! predictions.
//!
//! - Objective: `sum_i max(0, 1 - y_i w'x_i)^2 + lambda * ||w||^2` for a binary target
//!   (`y = +1` for the category the nugget's `ParamMatrix` targets, `-1` for the other), and
//!   `sum_i max(0, |y_i - w'x_i| - epsilon)^2 + lambda * ||w||^2` for a continuous target. The
//!   L1 penalty replaces `||w||^2` by `||w||_1` (`_penaltyFunction` L1, `models_as/lsvm_l1`).
//! - Design: an intercept column that is penalised like every other coefficient, covariates
//!   raw (no standardisation) and every category of a categorical input as its own indicator
//!   column (no reference level). Measured on the oracles this design puts Modeler's own
//!   coefficients closest to the optimum (`lsvm_default`: 0.868048 against 0.867945).
//! - The squared hinge is convex, continuously differentiable and piecewise quadratic, so the
//!   L2 problem is solved by the generalized Newton method (Hessian `2 X_A'X_A + 2 lambda I` on
//!   the records `A` inside the margin) with an Armijo line search, which reaches the exact
//!   minimiser in a few steps. The L1 problem uses the same Hessian in a proximal Newton method:
//!   each step solves the L1-regularised quadratic model by cyclic coordinate descent
//!   (soft-thresholding, so coefficients are exactly 0), then a line search on the true
//!   objective. Both are deterministic. Iterations are bounded; a run that does not converge
//!   within [`LsvmSettings::max_iterations`] returns [`AlgoError::Data`], never a half-solved
//!   model.
//! - The target of a classification model is a flag. Modeler lists a flag's true value first
//!   and writes the `ParamMatrix` for the false value (`lsvm_default`: `flagTypeTrueValue` Y,
//!   `targetReferenceCategory` Y, cells for `N`); the logit of that category is the decision
//!   value `w'x`, so the nugget's softmax gives `$LC-` = logistic(|w'x|) and predicts the
//!   reference category when `w'x <= 0`.
//! - The PMML is a `GeneralRegressionModel` shaped like the nugget Modeler saves
//!   (`oracle/models_as/lsvm_*/models/*.xml`): `multinomialLogistic` for a flag and
//!   `generalLinear` for a continuous target, `algorithmName` `linearSVM`, parameters `P0000000`
//!   (the intercept), then the covariates, then one indicator per category of each factor,
//!   covariates with `missingValueReplacement` 0.0, `MiningField`s without
//!   `invalidValueTreatment` (the PMML default) and a `DataDictionary` listing every field of the
//!   training data, including the ones the model does not use. The importance,
//!   `ModelExplanation`, `sigma` and `percentile` extensions are not reproduced (the shared PMML
//!   type does not carry them and no score depends on them).
//!
//! GAPs: more than two target categories (eq. 2.4, no oracle), a categorical target that is not
//! a flag, frequency weights, missing input values (Modeler's treatment is unverified; records
//! with a missing target are dropped), and settings other than `lambda`, `epsilon`, the
//! intercept and the penalty.

use crate::common::{data_field, mining_field};
use crate::{AlgoError, Dataset};
use dms_pmml::{
    GeneralModelType, GeneralRegressionModel, InvalidTreatment, MiningFunction, Model, ModelHeader,
    PCell, PPCell, Pmml, Usage,
};
use dms_types::{Field, Measurement, Storage, Table, Value, ValueSpec};

const MAX_RECORDS: usize = 1_000_000;
const MAX_PARAMETERS: usize = 1_000;
/// `DataDictionary` fields (`id` and other unused fields included).
const MAX_DICTIONARY_FIELDS: usize = 100_000;
/// Category values a `DataDictionary` may list in total.
const MAX_DICTIONARY_CATEGORIES: usize = 100_000;
/// Design cells (`records * parameters`) held in memory.
const MAX_CELLS: usize = 50_000_000;
/// Hessian work per Newton step (`records * parameters^2`).
const MAX_WORK: u128 = 20_000_000_000;
const MAX_ITERATIONS: usize = 10_000;
/// Coordinate-descent sweeps per proximal Newton step.
const MAX_SWEEPS: usize = 1_000;
/// Stop when the largest gradient (or subgradient violation) is below this times `1 + F`.
const GRADIENT_TOLERANCE: f64 = 1e-11;
/// Stop when the largest step is below this times `1 + max |w|`.
const STEP_TOLERANCE: f64 = 1e-12;
/// Accept a stalled line search when the gradient is already this small.
const STALL_TOLERANCE: f64 = 1e-7;
/// Sufficient decrease of the Armijo rule.
const ARMIJO: f64 = 1e-4;
/// Rounding slack of the Armijo test, relative to `1 + |F|`.
const SLACK: f64 = 1e-14;

/// The penalty of the objective (`_penaltyFunction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Penalty {
    /// `lambda * ||w||_1`.
    L1,
    /// `lambda * ||w||_2^2` (the default).
    L2,
}

/// Settings of a Linear SVM build node (Modeler's saved defaults in [`Default`]).
#[derive(Debug, Clone, PartialEq)]
pub struct LsvmSettings {
    /// `_lambda` (default 0.1); must be positive.
    pub lambda: f64,
    /// `_epsilon` (default 0.1): the insensitive zone of the continuous loss.
    pub epsilon: f64,
    /// `_intercept` (default true).
    pub intercept: bool,
    /// `_penaltyFunction` (default L2).
    pub penalty: Penalty,
    /// Newton iterations allowed before the run is reported as not converged.
    pub max_iterations: usize,
    /// The model name (`modelName`).
    pub name: Option<String>,
}

impl Default for LsvmSettings {
    fn default() -> Self {
        LsvmSettings {
            lambda: 0.1,
            epsilon: 0.1,
            intercept: true,
            penalty: Penalty::L2,
            max_iterations: 500,
            name: None,
        }
    }
}

/// A trained model with the numbers the validation reports.
#[derive(Debug, Clone)]
pub struct Solution {
    pub pmml: Pmml,
    /// The objective at the solution (`F(w)` above).
    pub objective: f64,
    /// Newton iterations used.
    pub iterations: usize,
    /// The coefficients by label (`Intercept`, `spend`, `[region=east]`), in design order.
    pub coefficients: Vec<(String, f64)>,
}

/// Trains on the target and inputs of `dataset`.
pub fn train(dataset: &Dataset<'_>, settings: &LsvmSettings) -> Result<Pmml, AlgoError> {
    solve(dataset, settings).map(|solution| solution.pmml)
}

/// Trains and reports the objective, iteration count and labelled coefficients.
pub fn solve(dataset: &Dataset<'_>, settings: &LsvmSettings) -> Result<Solution, AlgoError> {
    validate(settings)?;
    if dataset.frequency.is_some() {
        return Err(gap("with a frequency field"));
    }
    let table = dataset.table;
    let target = TargetSpec::of(dataset)?;
    if matches!(target, TargetSpec::Continuous) && settings.penalty == Penalty::L1 {
        return Err(gap("regression with the L1 penalty (no oracle)"));
    }
    let terms = design_terms(dataset, settings.intercept)?;
    let problem = Problem::read(table, dataset.target, &target, &terms, settings)?;
    let (weights, iterations) = problem.solve(settings.max_iterations)?;
    let objective = problem.objective(&weights);
    if !objective.is_finite() {
        return Err(AlgoError::Data("the LSVM objective is not finite".into()));
    }
    let coefficients = terms
        .iter()
        .map(Term::label)
        .zip(weights.iter().copied())
        .collect();
    Ok(Solution {
        pmml: to_pmml(dataset, settings, &terms, &weights, &target)?,
        objective,
        iterations,
        coefficients,
    })
}

/// The objective of the coefficients stored in `pmml` (ours or Modeler's) on the records of
/// `dataset`, with the design the PMML itself describes (`PPMatrix`) and the loss and penalty of
/// `settings`. The positive category is the one the `ParamMatrix` targets. Used to compare our
/// solution with Modeler's early-stopped one on the same data.
pub fn evaluate_objective(
    dataset: &Dataset<'_>,
    settings: &LsvmSettings,
    pmml: &Pmml,
) -> Result<f64, AlgoError> {
    validate(settings)?;
    let (terms, weights, target) = pmml_coefficients(pmml)?;
    let problem = Problem::read(dataset.table, dataset.target, &target, &terms, settings)?;
    Ok(problem.objective(&weights))
}

/// The labelled coefficients of an LSVM `GeneralRegressionModel`.
pub fn coefficients(pmml: &Pmml) -> Result<Vec<(String, f64)>, AlgoError> {
    let (terms, weights, _) = pmml_coefficients(pmml)?;
    Ok(terms.iter().map(Term::label).zip(weights).collect())
}

fn gap(what: &str) -> AlgoError {
    AlgoError::Unsupported(format!("GAP: LSVM {what}"))
}

fn validate(settings: &LsvmSettings) -> Result<(), AlgoError> {
    if !settings.lambda.is_finite() || settings.lambda <= 0.0 {
        return Err(AlgoError::Settings(
            "LSVM lambda must be a positive finite number".into(),
        ));
    }
    if !settings.epsilon.is_finite() || settings.epsilon < 0.0 {
        return Err(AlgoError::Settings(
            "LSVM epsilon must be a non-negative finite number".into(),
        ));
    }
    if settings.max_iterations == 0 || settings.max_iterations > MAX_ITERATIONS {
        return Err(AlgoError::Settings(format!(
            "LSVM iterations must be from 1 to {MAX_ITERATIONS}"
        )));
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------
// Design

/// What the target is.
#[derive(Debug, Clone)]
enum TargetSpec {
    /// A flag: `positive` (`y = +1`) is the category the `ParamMatrix` targets (the false
    /// value), `reference` the other (the true value).
    Binary {
        positive: String,
        reference: String,
    },
    Continuous,
}

impl TargetSpec {
    fn of(dataset: &Dataset<'_>) -> Result<TargetSpec, AlgoError> {
        let field = dataset
            .table
            .model()
            .fields()
            .get(dataset.target)
            .ok_or_else(|| AlgoError::Settings("the LSVM target is not a field".into()))?;
        if field.measurement == Measurement::Continuous {
            if !matches!(field.storage, Storage::Integer | Storage::Real) {
                return Err(gap("continuous target with non-numeric storage"));
            }
            return Ok(TargetSpec::Continuous);
        }
        match &field.values {
            ValueSpec::Flag {
                true_value,
                false_value,
            } => {
                let (positive, reference) = (false_value.to_string(), true_value.to_string());
                // The saved `_targetOrder` is ASCENDING. The oracles cannot tell whether the
                // targeted category is the false value or the first in that order (Y/N is
                // both), so only a flag where the two agree is accepted.
                if positive > reference {
                    return Err(gap(&format!(
                        "flag target {} whose false value does not sort before its true value \
                         (the targeted category is unverified)",
                        field.name
                    )));
                }
                Ok(TargetSpec::Binary {
                    positive,
                    reference,
                })
            }
            _ => Err(gap(&format!(
                "categorical target {} that is not a flag (multi-class eq. 2.4 has no oracle)",
                field.name
            ))),
        }
    }
}

/// One column of the design.
#[derive(Debug, Clone, PartialEq)]
enum Term {
    Intercept,
    Covariate { name: String },
    Level { name: String, value: String },
}

impl Term {
    fn label(&self) -> String {
        match self {
            Term::Intercept => "Intercept".into(),
            Term::Covariate { name } => name.clone(),
            Term::Level { name, value } => format!("[{name}={value}]"),
        }
    }
}

/// The intercept, the covariates, then one indicator per category of each factor.
fn design_terms(dataset: &Dataset<'_>, intercept: bool) -> Result<Vec<Term>, AlgoError> {
    let fields = dataset.table.model().fields();
    let mut seen: Vec<usize> = Vec::new();
    let mut covariates = Vec::new();
    let mut factors = Vec::new();
    for &position in dataset.inputs {
        if position == dataset.target {
            return Err(AlgoError::Settings(
                "the LSVM target is also an input".into(),
            ));
        }
        if seen.contains(&position) {
            continue;
        }
        seen.push(position);
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Settings("an LSVM input is not a field".into()))?;
        match field.measurement {
            Measurement::Typeless | Measurement::Collection | Measurement::Geospatial => {
                return Err(gap(&format!(
                    "input {} with measurement {:?}",
                    field.name, field.measurement
                )));
            }
            Measurement::Continuous => {
                if !matches!(field.storage, Storage::Integer | Storage::Real) {
                    return Err(gap(&format!(
                        "covariate {} with non-numeric storage",
                        field.name
                    )));
                }
                covariates.push(Term::Covariate {
                    name: field.name.clone(),
                });
            }
            _ => factors.push(position),
        }
    }
    let mut terms = Vec::new();
    if intercept {
        terms.push(Term::Intercept);
    }
    terms.extend(covariates);
    for position in factors {
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Settings("an LSVM input is not a field".into()))?;
        let levels = crate::field_categories(dataset.table, position);
        if levels.is_empty() {
            return Err(AlgoError::Data(format!(
                "the LSVM factor {} has no categories",
                field.name
            )));
        }
        if terms.len().saturating_add(levels.len()) > MAX_PARAMETERS {
            return Err(AlgoError::Data(format!(
                "LSVM is limited to {MAX_PARAMETERS} parameters"
            )));
        }
        terms.extend(levels.into_iter().map(|value| Term::Level {
            name: field.name.clone(),
            value,
        }));
    }
    if terms.len() > MAX_PARAMETERS {
        return Err(AlgoError::Data(format!(
            "LSVM is limited to {MAX_PARAMETERS} parameters"
        )));
    }
    if terms.is_empty() {
        return Err(AlgoError::Settings(
            "LSVM has no parameters to estimate".into(),
        ));
    }
    Ok(terms)
}

/// A value equals a category label, as text or as the same number.
fn matches_text(value: &Value, label: &str) -> bool {
    let text = value.to_string();
    text == label
        || matches!(
            (label.parse::<f64>(), text.parse::<f64>()),
            (Ok(a), Ok(b)) if a == b
        )
}

// ------------------------------------------------------------------------------------------
// The optimisation problem

struct Problem {
    n: usize,
    p: usize,
    /// Row-major design, `n * p`.
    x: Vec<f64>,
    /// `+1` / `-1`, or the continuous target.
    t: Vec<f64>,
    regression: bool,
    epsilon: f64,
    lambda: f64,
    penalty: Penalty,
}

impl Problem {
    fn read(
        table: &Table,
        target_position: usize,
        target: &TargetSpec,
        terms: &[Term],
        settings: &LsvmSettings,
    ) -> Result<Problem, AlgoError> {
        let fields = table.model().fields();
        let target_field = fields
            .get(target_position)
            .ok_or_else(|| AlgoError::Settings("the LSVM target is not a field".into()))?;
        let positions = terms
            .iter()
            .map(|term| match term {
                Term::Intercept => Ok(None),
                Term::Covariate { name } | Term::Level { name, .. } => table
                    .model()
                    .position(name)
                    .map(Some)
                    .ok_or_else(|| AlgoError::Data(format!("the LSVM field {name} is missing"))),
            })
            .collect::<Result<Vec<Option<usize>>, AlgoError>>()?;
        let p = terms.len();
        if p == 0 || p > MAX_PARAMETERS {
            return Err(AlgoError::Data(format!(
                "LSVM needs from 1 to {MAX_PARAMETERS} parameters, found {p}"
            )));
        }
        let rows = table.rows();
        let cells = rows
            .len()
            .checked_mul(p)
            .filter(|&cells| rows.len() <= MAX_RECORDS && cells <= MAX_CELLS)
            .ok_or_else(|| {
                AlgoError::Data(format!(
                    "LSVM is limited to {MAX_RECORDS} records and {MAX_CELLS} design cells"
                ))
            })?;
        let mut x = Vec::with_capacity(cells);
        let mut t = Vec::with_capacity(rows.len());
        for row in rows {
            let Some(value) = row
                .get(target_position)
                .filter(|value| !target_field.is_missing(value))
            else {
                continue;
            };
            let label = match target {
                TargetSpec::Continuous => match value.as_f64().filter(|y| y.is_finite()) {
                    Some(y) => y,
                    None => {
                        return Err(AlgoError::Data(format!(
                            "the LSVM target {} has a non-numeric value",
                            target_field.name
                        )));
                    }
                },
                TargetSpec::Binary {
                    positive,
                    reference,
                } => {
                    if matches_text(value, positive) {
                        1.0
                    } else if matches_text(value, reference) {
                        -1.0
                    } else {
                        continue;
                    }
                }
            };
            for (term, position) in terms.iter().zip(&positions) {
                x.push(design_cell(term, *position, row, fields)?);
            }
            t.push(label);
        }
        if t.is_empty() {
            return Err(AlgoError::Data(
                "no records with a target value to train LSVM".into(),
            ));
        }
        if matches!(target, TargetSpec::Binary { .. }) {
            let positives = t.iter().filter(|&&label| label > 0.0).count();
            if positives == 0 || positives == t.len() {
                return Err(AlgoError::Data(
                    "LSVM needs records of both target categories".into(),
                ));
            }
        }
        Ok(Problem {
            n: t.len(),
            p,
            x,
            t,
            regression: matches!(target, TargetSpec::Continuous),
            epsilon: settings.epsilon,
            lambda: settings.lambda,
            penalty: settings.penalty,
        })
    }

    fn row(&self, i: usize) -> &[f64] {
        let start = i.saturating_mul(self.p);
        self.x
            .get(start..start.saturating_add(self.p))
            .unwrap_or(&[])
    }

    /// The loss of one record at linear predictor `u`, its derivative with respect to `u`, and
    /// whether the record is inside the margin (its second derivative is then 2).
    fn point(&self, u: f64, t: f64) -> (f64, f64, bool) {
        if self.regression {
            let e = t - u;
            let excess = e.abs() - self.epsilon;
            if excess > 0.0 {
                (excess * excess, -2.0 * excess * e.signum(), true)
            } else {
                (0.0, 0.0, false)
            }
        } else {
            let margin = 1.0 - t * u;
            if margin > 0.0 {
                (margin * margin, -2.0 * t * margin, true)
            } else {
                (0.0, 0.0, false)
            }
        }
    }

    fn penalty_value(&self, w: &[f64]) -> f64 {
        match self.penalty {
            Penalty::L2 => self.lambda * w.iter().map(|v| v * v).sum::<f64>(),
            Penalty::L1 => self.lambda * w.iter().map(|v| v.abs()).sum::<f64>(),
        }
    }

    fn objective(&self, w: &[f64]) -> f64 {
        let loss: f64 = self
            .t
            .iter()
            .enumerate()
            .map(|(i, &t)| self.point(dot(self.row(i), w), t).0)
            .sum();
        loss + self.penalty_value(w)
    }

    /// The loss, its gradient and its generalized Hessian `2 X_A'X_A` at `w`.
    fn smooth(&self, w: &[f64]) -> (f64, Vec<f64>, Vec<f64>) {
        let p = self.p;
        let mut loss = 0.0;
        let mut gradient = vec![0.0; p];
        let mut hessian = vec![0.0; p.saturating_mul(p)];
        for (i, &t) in self.t.iter().enumerate() {
            let xi = self.row(i);
            let (l, slope, active) = self.point(dot(xi, w), t);
            loss += l;
            if !active {
                continue;
            }
            for (j, (g, &xj)) in gradient.iter_mut().zip(xi).enumerate() {
                *g += slope * xj;
                for (k, &xk) in xi.iter().enumerate().take(j + 1) {
                    if let Some(cell) = hessian.get_mut(j * p + k) {
                        *cell += 2.0 * xj * xk;
                    }
                }
            }
        }
        for j in 0..p {
            for k in 0..j {
                let lower = hessian.get(j * p + k).copied().unwrap_or(0.0);
                if let Some(cell) = hessian.get_mut(k * p + j) {
                    *cell = lower;
                }
            }
        }
        (loss, gradient, hessian)
    }

    fn solve(&self, max_iterations: usize) -> Result<(Vec<f64>, usize), AlgoError> {
        let work = (self.n as u128)
            .saturating_mul(self.p as u128)
            .saturating_mul(self.p as u128);
        if work > MAX_WORK {
            return Err(AlgoError::Data(format!(
                "LSVM with {} records and {} parameters is too large",
                self.n, self.p
            )));
        }
        match self.penalty {
            Penalty::L2 => self.newton_l2(max_iterations),
            Penalty::L1 => self.proximal_newton_l1(max_iterations),
        }
    }

    /// Armijo backtracking along `d` from `w`; `delta` is the predicted decrease (at most 0).
    fn backtrack(&self, w: &[f64], d: &[f64], f: f64, delta: f64) -> Option<(Vec<f64>, f64)> {
        let mut step = 1.0;
        for _ in 0..60 {
            let trial: Vec<f64> = w.iter().zip(d).map(|(w, d)| w + step * d).collect();
            let value = self.objective(&trial);
            if value.is_finite() && value <= f + ARMIJO * step * delta + SLACK * (1.0 + f.abs()) {
                return Some((trial, value));
            }
            step *= 0.5;
        }
        None
    }

    fn not_converged(&self, iterations: usize, measure: f64) -> AlgoError {
        AlgoError::Data(format!(
            "LSVM did not converge in {iterations} iterations (optimality measure {measure:e})"
        ))
    }

    /// Generalized Newton for the L2 penalty.
    fn newton_l2(&self, max_iterations: usize) -> Result<(Vec<f64>, usize), AlgoError> {
        let p = self.p;
        let mut w = vec![0.0; p];
        let mut f = self.objective(&w);
        let mut measure = f64::INFINITY;
        for iteration in 0..max_iterations {
            let (_, mut gradient, mut hessian) = self.smooth(&w);
            for (j, (g, &wj)) in gradient.iter_mut().zip(&w).enumerate() {
                *g += 2.0 * self.lambda * wj;
                if let Some(cell) = hessian.get_mut(j * p + j) {
                    *cell += 2.0 * self.lambda;
                }
            }
            measure = max_abs(&gradient);
            if !measure.is_finite() {
                return Err(AlgoError::Data("the LSVM gradient is not finite".into()));
            }
            if measure <= GRADIENT_TOLERANCE * (1.0 + f.abs()) {
                return Ok((w, iteration));
            }
            let negative: Vec<f64> = gradient.iter().map(|g| -g).collect();
            let direction = cholesky_solve(hessian, &negative, p).ok_or_else(|| {
                AlgoError::Data("the LSVM Newton system is not positive definite".into())
            })?;
            let delta = dot(&gradient, &direction).min(0.0);
            let Some((trial, value)) = self.backtrack(&w, &direction, f, delta) else {
                if measure <= STALL_TOLERANCE * (1.0 + f.abs()) {
                    return Ok((w, iteration));
                }
                return Err(self.not_converged(iteration, measure));
            };
            let step = trial
                .iter()
                .zip(&w)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f64::max);
            w = trial;
            f = value;
            if step <= STEP_TOLERANCE * (1.0 + max_abs(&w)) {
                return Ok((w, iteration + 1));
            }
        }
        Err(self.not_converged(max_iterations, measure))
    }

    /// Proximal Newton for the L1 penalty: coordinate descent on the quadratic model, then a
    /// line search on the true objective.
    fn proximal_newton_l1(&self, max_iterations: usize) -> Result<(Vec<f64>, usize), AlgoError> {
        let p = self.p;
        let mut w = vec![0.0; p];
        let mut f = self.objective(&w);
        let mut measure = f64::INFINITY;
        for iteration in 0..max_iterations {
            let (_, gradient, mut hessian) = self.smooth(&w);
            measure = w
                .iter()
                .zip(&gradient)
                .map(|(&wj, &gj)| {
                    if wj == 0.0 {
                        (gj.abs() - self.lambda).max(0.0)
                    } else {
                        (gj + self.lambda * wj.signum()).abs()
                    }
                })
                .fold(0.0, f64::max);
            if !measure.is_finite() {
                return Err(AlgoError::Data("the LSVM gradient is not finite".into()));
            }
            if measure <= GRADIENT_TOLERANCE * (1.0 + f.abs()) {
                return Ok((w, iteration));
            }
            let trace: f64 = (0..p)
                .map(|j| hessian.get(j * p + j).copied().unwrap_or(0.0))
                .sum();
            let ridge = 1e-10 * (1.0 + trace / p.max(1) as f64);
            for j in 0..p {
                if let Some(cell) = hessian.get_mut(j * p + j) {
                    *cell += ridge;
                }
            }
            let direction = self.lasso_step(&w, &gradient, &hessian);
            if max_abs(&direction) <= STEP_TOLERANCE * (1.0 + max_abs(&w)) {
                return Ok((w, iteration));
            }
            let l1_new: f64 = w.iter().zip(&direction).map(|(w, d)| (w + d).abs()).sum();
            let l1_old: f64 = w.iter().map(|w| w.abs()).sum();
            let delta = (dot(&gradient, &direction) + self.lambda * (l1_new - l1_old)).min(0.0);
            let Some((trial, value)) = self.backtrack(&w, &direction, f, delta) else {
                if measure <= STALL_TOLERANCE * (1.0 + f.abs()) {
                    return Ok((w, iteration));
                }
                return Err(self.not_converged(iteration, measure));
            };
            w = trial;
            f = value;
        }
        Err(self.not_converged(max_iterations, measure))
    }

    /// Minimises `g'd + d'Hd/2 + lambda ||w + d||_1` by cyclic coordinate descent.
    fn lasso_step(&self, w: &[f64], gradient: &[f64], hessian: &[f64]) -> Vec<f64> {
        let p = self.p;
        let mut d = vec![0.0; p];
        let mut hd = vec![0.0; p];
        for _ in 0..MAX_SWEEPS {
            let mut change = 0.0f64;
            for (j, (&wj, &gj)) in w.iter().zip(gradient).enumerate() {
                let a = hessian.get(j * p + j).copied().unwrap_or(0.0);
                if a <= 0.0 {
                    continue;
                }
                let dj_old = d.get(j).copied().unwrap_or(0.0);
                let c = gj + hd.get(j).copied().unwrap_or(0.0) - a * dj_old;
                let z = soft_threshold(a * wj - c, self.lambda) / a;
                let dj = z - wj;
                let shift = dj - dj_old;
                if shift == 0.0 {
                    continue;
                }
                for (k, h) in hd.iter_mut().enumerate() {
                    *h += hessian.get(k * p + j).copied().unwrap_or(0.0) * shift;
                }
                if let Some(slot) = d.get_mut(j) {
                    *slot = dj;
                }
                change = change.max(shift.abs());
            }
            if change <= 1e-13 * (1.0 + max_abs(w) + max_abs(&d)) {
                break;
            }
        }
        d
    }
}

/// The design cell of `term` for one record (`position` is the term's field; none for the
/// intercept).
fn design_cell(
    term: &Term,
    position: Option<usize>,
    row: &[Value],
    fields: &[Field],
) -> Result<f64, AlgoError> {
    let Some(position) = position else {
        return Ok(1.0);
    };
    let field = fields
        .get(position)
        .ok_or_else(|| AlgoError::Data(format!("no field at position {position}")))?;
    let value = row
        .get(position)
        .filter(|value| !field.is_missing(value))
        .ok_or_else(|| gap(&format!("with a missing value of input {}", field.name)))?;
    match term {
        Term::Intercept => Ok(1.0),
        Term::Covariate { .. } => value
            .as_f64()
            .filter(|x| x.is_finite())
            .ok_or_else(|| AlgoError::Data(format!("input {} is not a finite number", field.name))),
        Term::Level { value: level, .. } => Ok(if matches_text(value, level) { 1.0 } else { 0.0 }),
    }
}

fn soft_threshold(v: f64, threshold: f64) -> f64 {
    if v > threshold {
        v - threshold
    } else if v < -threshold {
        v + threshold
    } else {
        0.0
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

fn max_abs(values: &[f64]) -> f64 {
    values.iter().fold(0.0, |m, v| m.max(v.abs()))
}

/// Solves `a x = b` for a symmetric positive definite `a` (row-major, `p * p`) by Cholesky;
/// `None` when `a` is not positive definite or the solution is not finite.
fn cholesky_solve(mut a: Vec<f64>, b: &[f64], p: usize) -> Option<Vec<f64>> {
    if a.len() != p.checked_mul(p)? || b.len() != p {
        return None;
    }
    for j in 0..p {
        let mut diagonal = *a.get(j * p + j)?;
        for k in 0..j {
            let l = *a.get(j * p + k)?;
            diagonal -= l * l;
        }
        if diagonal.is_nan() || diagonal <= 0.0 || !diagonal.is_finite() {
            return None;
        }
        let root = diagonal.sqrt();
        *a.get_mut(j * p + j)? = root;
        for i in (j + 1)..p {
            let mut s = *a.get(i * p + j)?;
            for k in 0..j {
                s -= *a.get(i * p + k)? * *a.get(j * p + k)?;
            }
            *a.get_mut(i * p + j)? = s / root;
        }
    }
    let mut y = vec![0.0; p];
    for i in 0..p {
        let mut s = *b.get(i)?;
        for k in 0..i {
            s -= *a.get(i * p + k)? * *y.get(k)?;
        }
        *y.get_mut(i)? = s / *a.get(i * p + i)?;
    }
    let mut x = vec![0.0; p];
    for i in (0..p).rev() {
        let mut s = *y.get(i)?;
        for k in (i + 1)..p {
            s -= *a.get(k * p + i)? * *x.get(k)?;
        }
        *x.get_mut(i)? = s / *a.get(i * p + i)?;
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

// ------------------------------------------------------------------------------------------
// PMML

fn to_pmml(
    dataset: &Dataset<'_>,
    settings: &LsvmSettings,
    terms: &[Term],
    weights: &[f64],
    target: &TargetSpec,
) -> Result<Pmml, AlgoError> {
    let table = dataset.table;
    let mut covariates = Vec::new();
    let mut factors: Vec<String> = Vec::new();
    for term in terms {
        match term {
            Term::Covariate { name } => covariates.push(name.clone()),
            Term::Level { name, .. } if !factors.contains(name) => factors.push(name.clone()),
            _ => {}
        }
    }
    let fields = table.model().fields();
    // The nugget's `DataDictionary` lists every field of the training data, including the ones
    // the model does not use (`models_as/lsvm_default` lists `id`).
    if fields.len() > MAX_DICTIONARY_FIELDS {
        return Err(AlgoError::Data(format!(
            "LSVM is limited to {MAX_DICTIONARY_FIELDS} dictionary fields"
        )));
    }
    let mut dictionary = Vec::with_capacity(fields.len());
    let mut category_values = 0usize;
    for position in 0..fields.len() {
        let Some(entry) = data_field(table, position) else {
            continue;
        };
        category_values = category_values.saturating_add(entry.values.len());
        if category_values > MAX_DICTIONARY_CATEGORIES {
            return Err(AlgoError::Data(format!(
                "LSVM is limited to {MAX_DICTIONARY_CATEGORIES} dictionary category values"
            )));
        }
        dictionary.push(entry);
    }
    if let Some(entry) = dictionary.get_mut(dataset.target)
        && let TargetSpec::Binary {
            positive,
            reference,
        } = target
    {
        // Modeler lists a flag's true value first and marks it.
        entry.values = vec![reference.clone(), positive.clone()];
        entry.flag_true_value = Some(reference.clone());
    }
    let mut schema = Vec::new();
    let mut seen: Vec<usize> = Vec::new();
    for &position in dataset.inputs {
        let Some(field) = fields.get(position) else {
            continue;
        };
        if seen.contains(&position)
            || !(covariates.contains(&field.name) || factors.contains(&field.name))
        {
            continue;
        }
        seen.push(position);
        if let Some(mut mining) = mining_field(table, position, Usage::Active) {
            if covariates.contains(&field.name) {
                mining.missing_replacement = Some("0.0".into());
            }
            schema.push(mining);
        }
    }
    schema.extend(mining_field(table, dataset.target, Usage::Target));
    // The nugget leaves `invalidValueTreatment` unset, which is the PMML default.
    for mining in &mut schema {
        mining.invalid_treatment = InvalidTreatment::ReturnInvalid;
    }

    let names: Vec<String> = (0..terms.len()).map(|k| format!("P{k:07}")).collect();
    let mut pp_matrix = Vec::new();
    let mut param_matrix = Vec::new();
    let (function, model_type, target_category, target_reference_category) = match target {
        TargetSpec::Binary {
            positive,
            reference,
        } => (
            MiningFunction::Classification,
            GeneralModelType::MultinomialLogistic,
            Some(positive.clone()),
            Some(reference.clone()),
        ),
        TargetSpec::Continuous => (
            MiningFunction::Regression,
            GeneralModelType::GeneralLinear,
            None,
            None,
        ),
    };
    for ((term, name), &beta) in terms.iter().zip(&names).zip(weights) {
        match term {
            Term::Intercept => {}
            Term::Covariate { name: field } => pp_matrix.push(PPCell {
                value: "1".into(),
                predictor: field.clone(),
                parameter: name.clone(),
            }),
            Term::Level { name: field, value } => pp_matrix.push(PPCell {
                value: value.clone(),
                predictor: field.clone(),
                parameter: name.clone(),
            }),
        }
        param_matrix.push(PCell {
            target_category: target_category.clone(),
            parameter: name.clone(),
            beta,
        });
    }
    Ok(Pmml {
        version: "4.3".into(),
        data_dictionary: dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::GeneralRegression(GeneralRegressionModel {
            header: ModelHeader {
                name: settings.name.clone(),
                function,
                algorithm: Some("linearSVM".into()),
                mining_schema: schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            model_type,
            target_reference_category,
            link_function: None,
            link_parameter: None,
            parameters: names,
            factors,
            covariates,
            pp_matrix,
            param_matrix,
        }),
    })
}

/// The design columns, coefficients and target of an LSVM `GeneralRegressionModel`.
fn pmml_coefficients(pmml: &Pmml) -> Result<(Vec<Term>, Vec<f64>, TargetSpec), AlgoError> {
    let Model::GeneralRegression(model) = &pmml.model else {
        return Err(AlgoError::Data(
            "the LSVM model is not a GeneralRegressionModel".into(),
        ));
    };
    let target = match model.header.function {
        MiningFunction::Regression => TargetSpec::Continuous,
        _ => {
            let positive = model
                .param_matrix
                .iter()
                .find_map(|cell| cell.target_category.clone())
                .ok_or_else(|| AlgoError::Data("the LSVM model has no target category".into()))?;
            let reference = model.target_reference_category.clone().ok_or_else(|| {
                AlgoError::Data("the LSVM model has no reference category".into())
            })?;
            TargetSpec::Binary {
                positive,
                reference,
            }
        }
    };
    let mut terms = Vec::new();
    let mut weights = Vec::new();
    for parameter in &model.parameters {
        let cells: Vec<&PPCell> = model
            .pp_matrix
            .iter()
            .filter(|cell| cell.parameter == *parameter)
            .collect();
        let term = match cells.as_slice() {
            [] => Term::Intercept,
            [cell] if model.factors.contains(&cell.predictor) => Term::Level {
                name: cell.predictor.clone(),
                value: cell.value.clone(),
            },
            [cell] if cell.value.trim() == "1" => Term::Covariate {
                name: cell.predictor.clone(),
            },
            _ => {
                return Err(gap(&format!(
                    "parameter {parameter} is not an intercept, covariate or factor level"
                )));
            }
        };
        let beta: f64 = model
            .param_matrix
            .iter()
            .filter(|cell| cell.parameter == *parameter)
            .map(|cell| cell.beta)
            .sum();
        terms.push(term);
        weights.push(beta);
    }
    if terms.is_empty() {
        return Err(AlgoError::Data("the LSVM model has no parameters".into()));
    }
    Ok((terms, weights, target))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A problem from rows of `(features, target)`.
    fn problem(
        rows: &[(Vec<f64>, f64)],
        regression: bool,
        penalty: Penalty,
        lambda: f64,
    ) -> Problem {
        let p = rows.first().map_or(0, |(x, _)| x.len());
        Problem {
            n: rows.len(),
            p,
            x: rows.iter().flat_map(|(x, _)| x.iter().copied()).collect(),
            t: rows.iter().map(|(_, t)| *t).collect(),
            regression,
            epsilon: 0.1,
            lambda,
            penalty,
        }
    }

    /// Records that overlap, so some sit inside the margin at the optimum.
    fn classes() -> Vec<(Vec<f64>, f64)> {
        vec![
            (vec![1.0, 0.2, 0.9], 1.0),
            (vec![1.0, 0.4, -0.3], 1.0),
            (vec![1.0, 1.5, 0.1], 1.0),
            (vec![1.0, 1.1, 0.4], 1.0),
            (vec![1.0, -0.2, 0.7], -1.0),
            (vec![1.0, -1.3, -0.5], -1.0),
            (vec![1.0, 0.9, 0.3], -1.0),
            (vec![1.0, -0.8, 0.2], -1.0),
        ]
    }

    #[test]
    fn l2_newton_reaches_a_stationary_point() {
        let rows = classes();
        let problem = problem(&rows, false, Penalty::L2, 0.1);
        let (w, iterations) = problem.solve(100).unwrap();
        assert!(iterations < 30, "{iterations}");
        let (_, mut gradient, _) = problem.smooth(&w);
        for (g, wj) in gradient.iter_mut().zip(&w) {
            *g += 2.0 * 0.1 * wj;
        }
        assert!(max_abs(&gradient) < 1e-9, "{gradient:?}");
        // No nearby point is better.
        let best = problem.objective(&w);
        for j in 0..w.len() {
            for delta in [1e-4, -1e-4] {
                let mut moved = w.clone();
                moved[j] += delta;
                assert!(problem.objective(&moved) >= best);
            }
        }
    }

    #[test]
    fn l1_zeroes_a_useless_column_and_is_optimal() {
        // The third column is noise: the L1 penalty at this lambda drops it.
        let rows = vec![
            (vec![1.0, 2.0, 0.01], 1.0),
            (vec![1.0, 1.5, -0.01], 1.0),
            (vec![1.0, 2.5, 0.02], 1.0),
            (vec![1.0, -2.0, -0.01], -1.0),
            (vec![1.0, -1.5, 0.01], -1.0),
            (vec![1.0, -2.5, -0.02], -1.0),
        ];
        let problem = problem(&rows, false, Penalty::L1, 0.5);
        let (w, _) = problem.solve(100).unwrap();
        assert_eq!(w[2], 0.0, "{w:?}");
        let best = problem.objective(&w);
        for j in 0..w.len() {
            for delta in [1e-5, -1e-5] {
                let mut moved = w.clone();
                moved[j] += delta;
                assert!(problem.objective(&moved) >= best - 1e-12);
            }
        }
    }

    #[test]
    fn regression_ignores_residuals_inside_the_zone() {
        let rows: Vec<(Vec<f64>, f64)> = (0..12)
            .map(|i| {
                let x = f64::from(i) / 4.0;
                (vec![1.0, x], 2.0 * x + 0.05 * f64::from(i % 3) - 0.05)
            })
            .collect();
        let problem = problem(&rows, true, Penalty::L2, 0.01);
        let (w, _) = problem.solve(100).unwrap();
        let (_, mut gradient, _) = problem.smooth(&w);
        for (g, wj) in gradient.iter_mut().zip(&w) {
            *g += 2.0 * 0.01 * wj;
        }
        assert!(max_abs(&gradient) < 1e-9, "{gradient:?}");
        assert!((w[1] - 2.0).abs() < 0.1, "{w:?}");
    }

    #[test]
    fn solving_twice_gives_identical_bits() {
        let rows = classes();
        for penalty in [Penalty::L1, Penalty::L2] {
            let problem = problem(&rows, false, penalty, 0.1);
            let a = problem.solve(100).unwrap();
            let b = problem.solve(100).unwrap();
            assert_eq!(a.1, b.1);
            assert!(
                a.0.iter()
                    .zip(&b.0)
                    .all(|(x, y)| x.to_bits() == y.to_bits())
            );
        }
    }

    #[test]
    fn too_few_iterations_is_an_error_not_a_model() {
        let rows = classes();
        for penalty in [Penalty::L1, Penalty::L2] {
            let problem = problem(&rows, false, penalty, 0.1);
            let error = problem.solve(1).unwrap_err();
            assert!(
                matches!(&error, AlgoError::Data(m) if m.contains("did not converge")),
                "{error:?}"
            );
        }
    }

    #[test]
    fn cholesky_rejects_what_it_cannot_solve() {
        assert!(cholesky_solve(vec![1.0, 2.0, 2.0, 1.0], &[1.0, 1.0], 2).is_none());
        assert!(cholesky_solve(vec![f64::NAN], &[1.0], 1).is_none());
        assert!(cholesky_solve(vec![1.0], &[1.0, 2.0], 1).is_none());
        let x = cholesky_solve(vec![4.0, 1.0, 1.0, 3.0], &[1.0, 2.0], 2).unwrap();
        assert!((4.0 * x[0] + x[1] - 1.0).abs() < 1e-12);
        assert!((x[0] + 3.0 * x[1] - 2.0).abs() < 1e-12);
    }

    #[test]
    fn invalid_settings_are_rejected() {
        for settings in [
            LsvmSettings {
                lambda: 0.0,
                ..LsvmSettings::default()
            },
            LsvmSettings {
                lambda: f64::NAN,
                ..LsvmSettings::default()
            },
            LsvmSettings {
                epsilon: -1.0,
                ..LsvmSettings::default()
            },
            LsvmSettings {
                max_iterations: 0,
                ..LsvmSettings::default()
            },
            LsvmSettings {
                max_iterations: usize::MAX,
                ..LsvmSettings::default()
            },
        ] {
            assert!(validate(&settings).is_err(), "{settings:?}");
        }
        assert!(validate(&LsvmSettings::default()).is_ok());
    }
}
