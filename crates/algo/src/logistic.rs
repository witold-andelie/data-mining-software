//! Logistic regression (`BuildLogisticNode`; Algorithms Guide: "Logistic Regression
//! Algorithms"): Newton–Raphson / Fisher scoring of a generalised logit (NOMREG) for the
//! multinomial procedure and of the dichotomous logit for the binomial procedure.
//!
//! The model is a PMML `GeneralRegressionModel` (`modelType` `multinomialLogistic`) shaped like
//! Modeler's export (`oracle/models/logreg_*`, `oracle/team/logistic_regression`):
//! - Data dictionary and mining schema: the target, then factors (categorical inputs in input
//!   order), then covariates (continuous inputs in input order)
//!   (`logreg_multinomial`: Drug, Sex, BP, Cholesterol, Age, Na, K).
//! - `ParameterList`: intercept (`Intercept`, or `Constant` for the binomial procedure), then
//!   covariates, then one indicator per factor category, including the reference whose β is 0
//!   (`logreg_multinomial`: Age, Na, K, then `[Sex=F]`, `[Sex=M]`, …).
//! - Factor dummy coding uses the last category as the reference
//!   (`logreg_multinomial`: Sex=M, BP=NORMAL, Cholesterol=NORMAL have β = 0, `df` 0). The
//!   binomial procedure uses Indicator coding with the first category as the reference
//!   (`docs/formats/nodes/logreg.md`); no oracle has a categorical binomial input (GAP).
//! - Estimation uses the last target category as the reference (the guide). The PMML omits the
//!   first DataField category and stores the reparameterised logits
//!   (`logreg_multinomial`: PCells for drugB, drugC, drugX, drugY; `logreg_binomial`: PCells for
//!   `2` with `targetReferenceCategory` `2`).
//! - Starting intercepts are `ln(n_j / n_J)`; other parameters start at 0. Step-halving uses
//!   `1, 1/2, …, 1/2^V` (`maximumStepHalving` default 5). Convergence: largest |Δβ| or |score|
//!   below `parameterConvergence` (1e-6), or |Δℓ| below `logLikelihoodConvergence` when that is
//!   positive (0 disables it). Records with a missing target or input are dropped.
//! - GAP (`logreg_multinomial`): DRUG1n is completely separated, so the MLE is at infinity.
//!   Twenty Newton steps reach ℓ ≈ 0 and the same predicted class as Modeler on every record;
//!   some coefficients (weakly identified, e.g. K for drugC) differ because they live in the
//!   null space of the information matrix. The binomial HEART case has a unique MLE and
//!   matches Modeler to 1e-8.
//!
//! Supported: Enter method, main effects, with or without a constant. Stepwise / forwards /
//! backwards, custom terms, full factorial, a non-zero cell `delta`, scaling, frequency weights
//! and non-Indicator binomial contrasts return [`AlgoError::Unsupported`].

use crate::common::{data_field, field_categories, mining_field};
use crate::{AlgoError, Dataset, categories};
use dms_pmml::{
    GeneralModelType, GeneralRegressionModel, MiningFunction, Model, ModelHeader, PCell, PPCell,
    Pmml, Usage,
};
use dms_types::{Measurement, Storage, Value};

/// Which of Modeler's two logistic procedures to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Procedure {
    /// NOMREG generalised logit (`logisticProcedure` Multinomial).
    Multinomial,
    /// Dichotomous logit (`logisticProcedure` Binomial).
    Binomial,
}

/// Settings of a Logistic build node (Modeler's defaults in [`Default`]).
#[derive(Debug, Clone, PartialEq)]
pub struct LogisticSettings {
    /// `logistic_procedure` (default Multinomial).
    pub procedure: Procedure,
    /// `include_constant` (default true).
    pub include_constant: bool,
    /// `max_iterations` (default 20).
    pub max_iterations: usize,
    /// `max_steps` / `maximumStepHalving` (default 5).
    pub max_step_halving: usize,
    /// `p_converge` / `parameterConvergence` (default 1e-6).
    pub parameter_convergence: f64,
    /// `l_converge` / `logLikelihoodConvergence` (default 0: unused).
    pub log_likelihood_convergence: f64,
    /// `tolerance` / `singularityTolerance` (default 1e-8).
    pub singularity: f64,
    /// `delta` added to zero cells (default 0; any other value is rejected).
    pub delta: f64,
    /// `multinomial_base_category`: estimation reference; `None` uses the last target category.
    pub base_category: Option<String>,
    /// The model name (`modelName`).
    pub name: Option<String>,
}

impl Default for LogisticSettings {
    fn default() -> Self {
        LogisticSettings {
            procedure: Procedure::Multinomial,
            include_constant: true,
            max_iterations: 20,
            max_step_halving: 5,
            parameter_convergence: 1e-6,
            log_likelihood_convergence: 0.0,
            singularity: 1e-8,
            delta: 0.0,
            base_category: None,
            name: None,
        }
    }
}

const MAX_RECORDS: usize = 1_000_000;
const MAX_PARAMETERS: usize = 4096;

/// Trains a logistic regression model on the target and inputs of `dataset`.
pub fn train(dataset: &Dataset<'_>, settings: &LogisticSettings) -> Result<Pmml, AlgoError> {
    if dataset.frequency.is_some() {
        return Err(AlgoError::Unsupported(
            "logistic regression with a frequency field".into(),
        ));
    }
    validate_settings(settings)?;

    let data = Data::read(dataset, settings)?;
    let beta = newton(&data, settings)?;
    let pmml_beta = reparameterise(&data, &beta);
    Ok(to_pmml(dataset, &data, settings, &pmml_beta))
}

fn validate_settings(settings: &LogisticSettings) -> Result<(), AlgoError> {
    if settings.delta != 0.0 {
        return Err(AlgoError::Unsupported(
            "logistic regression with a non-zero cell delta".into(),
        ));
    }
    if !settings.parameter_convergence.is_finite() || settings.parameter_convergence < 0.0 {
        return Err(AlgoError::Settings(
            "parameterConvergence must be a non-negative finite number".into(),
        ));
    }
    if !settings.log_likelihood_convergence.is_finite() || settings.log_likelihood_convergence < 0.0
    {
        return Err(AlgoError::Settings(
            "logLikelihoodConvergence must be a non-negative finite number".into(),
        ));
    }
    if !settings.singularity.is_finite() || settings.singularity < 0.0 {
        return Err(AlgoError::Settings(
            "singularityTolerance must be a non-negative finite number".into(),
        ));
    }
    if settings.max_iterations > 10_000 {
        return Err(AlgoError::Settings("maximumIterations is too large".into()));
    }
    if settings.max_step_halving > 64 {
        return Err(AlgoError::Settings(
            "maximumStepHalving is too large".into(),
        ));
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------
// Training data

struct Factor {
    position: usize,
    name: String,
    labels: Vec<String>,
    /// Category index that is not estimated (last for multinomial, first for binomial).
    reference: usize,
}

struct Covariate {
    position: usize,
    name: String,
}

struct Record {
    x: Vec<f64>,
    class: usize,
    weight: f64,
}

struct Data {
    labels: Vec<String>,
    factors: Vec<Factor>,
    covariates: Vec<Covariate>,
    params: Vec<Param>,
    /// Estimated (non-redundant) column of each full parameter; `None` for structural zeros.
    est_of: Vec<Option<usize>>,
    p_est: usize,
    /// Target category used as the estimation reference (the guide's last category by default).
    est_ref: usize,
    /// `labels` indices of the J − 1 non-reference categories, in DataField order.
    non_ref: Vec<usize>,
    records: Vec<Record>,
}

struct Param {
    name: String,
    kind: ParamKind,
}

enum ParamKind {
    Intercept,
    Covariate {
        field: String,
    },
    Factor {
        field: String,
        value: String,
        redundant: bool,
    },
}

impl Data {
    fn read(dataset: &Dataset<'_>, settings: &LogisticSettings) -> Result<Data, AlgoError> {
        let fields = dataset.table.model().fields();
        let field = |position: usize| {
            fields
                .get(position)
                .ok_or_else(|| AlgoError::Data(format!("no field at position {position}")))
        };
        let target_field = field(dataset.target)?;
        if target_field.measurement == Measurement::Continuous {
            return Err(AlgoError::Data(format!(
                "logistic regression target {} is continuous",
                target_field.name
            )));
        }
        let labels = categories(dataset, dataset.target);
        if labels.len() < 2 {
            return Err(AlgoError::Data(format!(
                "logistic regression target {} needs at least two categories",
                target_field.name
            )));
        }
        if settings.procedure == Procedure::Binomial && labels.len() != 2 {
            return Err(AlgoError::Data(format!(
                "the binomial procedure needs a flag or two-category target, {} has {}",
                target_field.name,
                labels.len()
            )));
        }

        let est_ref = match &settings.base_category {
            Some(name) if !name.is_empty() => labels
                .iter()
                .position(|label| label == name)
                .ok_or_else(|| {
                    AlgoError::Settings(format!(
                        "multinomial base category {name} is not a value of {}",
                        target_field.name
                    ))
                })?,
            _ => labels.len().saturating_sub(1),
        };
        let non_ref: Vec<usize> = (0..labels.len()).filter(|&c| c != est_ref).collect();
        if non_ref.is_empty() {
            return Err(AlgoError::Data(
                "logistic regression has no non-reference target category".into(),
            ));
        }

        let mut factors = Vec::new();
        let mut covariates = Vec::new();
        for &position in dataset.inputs {
            let input = field(position)?;
            match input.measurement {
                Measurement::Typeless | Measurement::Collection | Measurement::Geospatial => {
                    return Err(AlgoError::Unsupported(format!(
                        "logistic regression input {} with measurement {:?}",
                        input.name, input.measurement
                    )));
                }
                Measurement::Continuous => {
                    if !matches!(input.storage, Storage::Integer | Storage::Real) {
                        return Err(AlgoError::Unsupported(format!(
                            "logistic regression covariate {} with non-numeric storage",
                            input.name
                        )));
                    }
                    covariates.push(Covariate {
                        position,
                        name: input.name.clone(),
                    });
                }
                _ => {
                    let factor_labels = field_categories(dataset.table, position);
                    if factor_labels.is_empty() {
                        return Err(AlgoError::Data(format!(
                            "logistic regression factor {} has no categories",
                            input.name
                        )));
                    }
                    let reference = match settings.procedure {
                        Procedure::Multinomial => factor_labels.len().saturating_sub(1),
                        Procedure::Binomial => 0,
                    };
                    factors.push(Factor {
                        position,
                        name: input.name.clone(),
                        labels: factor_labels,
                        reference,
                    });
                }
            }
        }

        let params = parameter_list(settings.include_constant, &covariates, &factors);
        let mut est_of = Vec::with_capacity(params.len());
        let mut p_est = 0usize;
        for param in &params {
            let estimated = match param.kind {
                ParamKind::Factor { redundant, .. } => !redundant,
                _ => true,
            };
            if estimated {
                est_of.push(Some(p_est));
                p_est = p_est.saturating_add(1);
            } else {
                est_of.push(None);
            }
        }
        if p_est == 0 {
            return Err(AlgoError::Settings(
                "logistic regression has no parameters to estimate".into(),
            ));
        }
        let npar = non_ref
            .len()
            .checked_mul(p_est)
            .ok_or_else(|| AlgoError::Data("too many logistic parameters".into()))?;
        if npar > MAX_PARAMETERS {
            return Err(AlgoError::Data(format!(
                "logistic regression would estimate {npar} parameters"
            )));
        }

        let table_rows = dataset.table.rows();
        if table_rows.len() > MAX_RECORDS {
            return Err(AlgoError::Data(format!(
                "logistic regression is limited to {MAX_RECORDS} records"
            )));
        }
        let mut records = Vec::new();
        for row in table_rows {
            let Some(target_value) = row
                .get(dataset.target)
                .filter(|value| !target_field.is_missing(value))
            else {
                continue;
            };
            let Some(class) = class_of(&labels, target_value) else {
                continue;
            };
            let Some(x) = design_row(
                row,
                fields,
                settings.include_constant,
                &covariates,
                &factors,
            ) else {
                continue;
            };
            records.push(Record {
                x,
                class,
                weight: 1.0,
            });
        }
        if records.is_empty() {
            return Err(AlgoError::Data(
                "no complete records to train logistic regression".into(),
            ));
        }
        Ok(Data {
            labels,
            factors,
            covariates,
            params,
            est_of,
            p_est,
            est_ref,
            non_ref,
            records,
        })
    }
}

fn parameter_list(intercept: bool, covariates: &[Covariate], factors: &[Factor]) -> Vec<Param> {
    let mut params = Vec::new();
    let mut next = 1usize;
    let mut push = |kind: ParamKind| {
        params.push(Param {
            name: format!("P{next:07}"),
            kind,
        });
        next = next.saturating_add(1);
    };
    if intercept {
        push(ParamKind::Intercept);
    }
    for covariate in covariates {
        push(ParamKind::Covariate {
            field: covariate.name.clone(),
        });
    }
    for factor in factors {
        for (index, label) in factor.labels.iter().enumerate() {
            push(ParamKind::Factor {
                field: factor.name.clone(),
                value: label.clone(),
                redundant: index == factor.reference,
            });
        }
    }
    params
}

fn class_of(labels: &[String], value: &Value) -> Option<usize> {
    let text = value.to_string();
    labels.iter().position(|label| {
        if label == &text {
            return true;
        }
        matches!(
            (label.parse::<f64>(), text.parse::<f64>()),
            (Ok(a), Ok(b)) if a == b
        )
    })
}

fn design_row(
    row: &[Value],
    fields: &[dms_types::Field],
    intercept: bool,
    covariates: &[Covariate],
    factors: &[Factor],
) -> Option<Vec<f64>> {
    let mut x = Vec::new();
    if intercept {
        x.push(1.0);
    }
    for covariate in covariates {
        let field = fields.get(covariate.position)?;
        let value = row
            .get(covariate.position)
            .filter(|v| !field.is_missing(v))?;
        let number = value.as_f64().filter(|n| n.is_finite())?;
        x.push(number);
    }
    for factor in factors {
        let field = fields.get(factor.position)?;
        let value = row.get(factor.position).filter(|v| !field.is_missing(v))?;
        let code = class_of(&factor.labels, value)?;
        for (index, _) in factor.labels.iter().enumerate() {
            if index == factor.reference {
                continue;
            }
            x.push(if index == code { 1.0 } else { 0.0 });
        }
    }
    Some(x)
}

// ------------------------------------------------------------------------------------------
// Newton–Raphson

fn newton(data: &Data, settings: &LogisticSettings) -> Result<Vec<f64>, AlgoError> {
    let m = data.non_ref.len();
    let p = data.p_est;
    let Some(npar) = m.checked_mul(p) else {
        return Err(AlgoError::Data("too many logistic parameters".into()));
    };
    let mut beta = starting_beta(data);
    if beta.len() != npar {
        return Err(AlgoError::Data(
            "logistic starting values do not match the parameters".into(),
        ));
    }
    for _ in 0..settings.max_iterations {
        let Some((loglik, score, info)) = derivatives(data, &beta) else {
            break;
        };
        if !loglik.is_finite() {
            break;
        }
        if max_abs(&score) < settings.parameter_convergence {
            break;
        }
        let Some(delta) = solve_system(&info, &score, npar, settings.singularity) else {
            break;
        };
        if max_abs(&delta) == 0.0 {
            break;
        }
        let Some((new_beta, new_loglik)) =
            step_halve(data, &beta, &delta, loglik, settings.max_step_halving)
        else {
            break;
        };
        let change = beta
            .iter()
            .zip(&new_beta)
            .map(|(old, new)| (old - new).abs())
            .fold(0.0, f64::max);
        beta = new_beta;
        if change < settings.parameter_convergence {
            break;
        }
        if settings.log_likelihood_convergence > 0.0
            && (new_loglik - loglik).abs() < settings.log_likelihood_convergence
        {
            break;
        }
    }
    Ok(beta)
}

fn starting_beta(data: &Data) -> Vec<f64> {
    let m = data.non_ref.len();
    let p = data.p_est;
    let mut beta = vec![0.0; m.saturating_mul(p)];
    if p == 0 {
        return beta;
    }
    let mut counts = vec![0.0; data.labels.len()];
    for record in &data.records {
        if let Some(count) = counts.get_mut(record.class) {
            *count += record.weight;
        }
    }
    let n_ref = counts.get(data.est_ref).copied().unwrap_or(0.0);
    if n_ref <= 0.0 {
        return beta;
    }
    for (j, &class) in data.non_ref.iter().enumerate() {
        let n_j = counts.get(class).copied().unwrap_or(0.0);
        if n_j > 0.0
            && let Some(slot) = beta.get_mut(j.saturating_mul(p))
        {
            *slot = (n_j / n_ref).ln();
        }
    }
    beta
}

fn derivatives(data: &Data, beta: &[f64]) -> Option<(f64, Vec<f64>, Vec<f64>)> {
    let m = data.non_ref.len();
    let p = data.p_est;
    let npar = m.checked_mul(p)?;
    let mut loglik = 0.0;
    let mut score = vec![0.0; npar];
    let mut info = vec![0.0; npar.checked_mul(npar)?];
    for record in &data.records {
        let pi = softmax_non_ref(&record.x, beta, m, p)?;
        let y_non_ref = data.non_ref.iter().position(|&c| c == record.class);
        let ll = case_loglik(y_non_ref, &record.x, beta, m, p)?;
        loglik += record.weight * ll;
        for j in 0..m {
            let residual =
                if y_non_ref == Some(j) { 1.0 } else { 0.0 } - pi.get(j).copied().unwrap_or(0.0);
            let wres = record.weight * residual;
            let row = j.saturating_mul(p);
            for s in 0..p {
                let xs = record.x.get(s).copied().unwrap_or(0.0);
                if let Some(g) = score.get_mut(row + s) {
                    *g += wres * xs;
                }
            }
            for jp in 0..m {
                let pi_j = pi.get(j).copied().unwrap_or(0.0);
                let pi_jp = pi.get(jp).copied().unwrap_or(0.0);
                let wpi = record.weight * pi_j * if j == jp { 1.0 - pi_jp } else { -pi_jp };
                let col = jp.saturating_mul(p);
                for s in 0..p {
                    let xs = record.x.get(s).copied().unwrap_or(0.0);
                    let wpi_xs = wpi * xs;
                    for t in 0..p {
                        let xt = record.x.get(t).copied().unwrap_or(0.0);
                        let index = (row + s).saturating_mul(npar) + (col + t);
                        if let Some(cell) = info.get_mut(index) {
                            *cell += wpi_xs * xt;
                        }
                    }
                }
            }
        }
    }
    Some((loglik, score, info))
}

fn softmax_non_ref(x: &[f64], beta: &[f64], m: usize, p: usize) -> Option<Vec<f64>> {
    let eta = linear_predictors(x, beta, m, p)?;
    let mut max_z = 0.0;
    for &e in &eta {
        if e > max_z {
            max_z = e;
        }
    }
    let mut weights = Vec::with_capacity(m);
    let mut sum = (0.0 - max_z).exp();
    for &e in &eta {
        let w = (e - max_z).exp();
        sum += w;
        weights.push(w);
    }
    if !sum.is_finite() || sum <= 0.0 {
        return Some(vec![0.0; m]);
    }
    Some(weights.into_iter().map(|w| w / sum).collect())
}

fn linear_predictors(x: &[f64], beta: &[f64], m: usize, p: usize) -> Option<Vec<f64>> {
    if x.len() != p {
        return None;
    }
    let mut eta = vec![0.0; m];
    for j in 0..m {
        let start = j.saturating_mul(p);
        let slot = eta.get_mut(j)?;
        *slot = 0.0;
        for s in 0..p {
            let b = *beta.get(start + s)?;
            let xs = *x.get(s)?;
            *slot += b * xs;
        }
    }
    Some(eta)
}

fn case_loglik(
    y_non_ref: Option<usize>,
    x: &[f64],
    beta: &[f64],
    m: usize,
    p: usize,
) -> Option<f64> {
    let eta = linear_predictors(x, beta, m, p)?;
    let mut max_z = 0.0;
    for &e in &eta {
        if e > max_z {
            max_z = e;
        }
    }
    let mut sum = (0.0 - max_z).exp();
    for &e in &eta {
        sum += (e - max_z).exp();
    }
    if !sum.is_finite() || sum <= 0.0 {
        return Some(f64::NEG_INFINITY);
    }
    let log_sum = sum.ln();
    Some(match y_non_ref {
        None => -max_z - log_sum,
        Some(j) => eta.get(j).copied().unwrap_or(0.0) - max_z - log_sum,
    })
}

fn loglik_at(data: &Data, beta: &[f64]) -> Option<f64> {
    let m = data.non_ref.len();
    let p = data.p_est;
    let mut loglik = 0.0;
    for record in &data.records {
        let y_non_ref = data.non_ref.iter().position(|&c| c == record.class);
        let ll = case_loglik(y_non_ref, &record.x, beta, m, p)?;
        loglik += record.weight * ll;
    }
    Some(loglik)
}

fn step_halve(
    data: &Data,
    beta: &[f64],
    delta: &[f64],
    loglik: f64,
    max_halves: usize,
) -> Option<(Vec<f64>, f64)> {
    let mut xi = 1.0;
    for _ in 0..=max_halves {
        let mut trial = Vec::with_capacity(beta.len());
        for (b, d) in beta.iter().zip(delta) {
            trial.push(b + xi * d);
        }
        if trial.iter().all(|v| v.is_finite())
            && let Some(l_new) = loglik_at(data, &trial)
            && l_new.is_finite()
            && l_new >= loglik - 1e-14 * (1.0 + loglik.abs())
        {
            return Some((trial, l_new));
        }
        xi *= 0.5;
    }
    None
}

fn max_abs(values: &[f64]) -> f64 {
    values.iter().fold(0.0, |m, v| m.max(v.abs()))
}

/// Gaussian elimination with partial pivoting. A column whose pivot is below `tol` is treated
/// as aliased and its unknown is left at 0 (SPSS `SINGULAR`).
fn solve_system(matrix: &[f64], rhs: &[f64], n: usize, tol: f64) -> Option<Vec<f64>> {
    if n == 0 {
        return Some(Vec::new());
    }
    let len = n.checked_mul(n)?;
    if matrix.len() < len || rhs.len() < n {
        return None;
    }
    let mut a = matrix[..len].to_vec();
    let mut b = rhs[..n].to_vec();
    for k in 0..n {
        let mut pivot_row = k;
        let mut pivot_abs = a.get(k * n + k).copied().unwrap_or(0.0).abs();
        for i in k + 1..n {
            let v = a.get(i * n + k).copied().unwrap_or(0.0).abs();
            if v > pivot_abs {
                pivot_abs = v;
                pivot_row = i;
            }
        }
        if pivot_abs < tol {
            continue;
        }
        if pivot_row != k {
            for j in 0..n {
                let i0 = k * n + j;
                let i1 = pivot_row * n + j;
                if let (Some(&u), Some(&v)) = (a.get(i0), a.get(i1)) {
                    if let Some(slot) = a.get_mut(i0) {
                        *slot = v;
                    }
                    if let Some(slot) = a.get_mut(i1) {
                        *slot = u;
                    }
                }
            }
            if let (Some(&u), Some(&v)) = (b.get(k), b.get(pivot_row)) {
                if let Some(slot) = b.get_mut(k) {
                    *slot = v;
                }
                if let Some(slot) = b.get_mut(pivot_row) {
                    *slot = u;
                }
            }
        }
        let akk = a.get(k * n + k).copied().unwrap_or(0.0);
        if akk.abs() < tol {
            continue;
        }
        for i in k + 1..n {
            let aik = a.get(i * n + k).copied().unwrap_or(0.0);
            let f = aik / akk;
            if let Some(slot) = a.get_mut(i * n + k) {
                *slot = 0.0;
            }
            for j in k + 1..n {
                let akj = a.get(k * n + j).copied().unwrap_or(0.0);
                if let Some(slot) = a.get_mut(i * n + j) {
                    *slot -= f * akj;
                }
            }
            let bk = b.get(k).copied().unwrap_or(0.0);
            if let Some(slot) = b.get_mut(i) {
                *slot -= f * bk;
            }
        }
    }
    let mut x = vec![0.0; n];
    for k in (0..n).rev() {
        let akk = a.get(k * n + k).copied().unwrap_or(0.0);
        if akk.abs() < tol {
            continue;
        }
        let mut s = b.get(k).copied().unwrap_or(0.0);
        for j in k + 1..n {
            let akj = a.get(k * n + j).copied().unwrap_or(0.0);
            let xj = x.get(j).copied().unwrap_or(0.0);
            s -= akj * xj;
        }
        if let Some(slot) = x.get_mut(k) {
            *slot = s / akk;
        }
    }
    if x.iter().all(|v| v.is_finite()) {
        Some(x)
    } else {
        None
    }
}

// ------------------------------------------------------------------------------------------
// PMML

/// Full (including structural-zero) coefficients per target category, with the first DataField
/// category as the omitted PMML reference.
fn reparameterise(data: &Data, beta: &[f64]) -> Vec<Vec<f64>> {
    let j = data.labels.len();
    let q = data.params.len();
    let mut full = vec![vec![0.0; q]; j];
    let p = data.p_est;
    for (logit, &class) in data.non_ref.iter().enumerate() {
        let start = logit.saturating_mul(p);
        if let Some(row) = full.get_mut(class) {
            for (param_index, est) in data.est_of.iter().enumerate() {
                if let Some(s) = *est
                    && let Some(slot) = row.get_mut(param_index)
                {
                    *slot = beta.get(start + s).copied().unwrap_or(0.0);
                }
            }
        }
    }
    let first = 0usize;
    let beta_first = full.get(first).cloned().unwrap_or_else(|| vec![0.0; q]);
    let mut pmml = vec![vec![0.0; q]; j];
    for class in 0..j {
        if class == first {
            continue;
        }
        let beta_c = full.get(class).cloned().unwrap_or_else(|| vec![0.0; q]);
        if let Some(row) = pmml.get_mut(class) {
            for (slot, (c, f)) in row.iter_mut().zip(beta_c.iter().zip(&beta_first)) {
                *slot = c - f;
            }
        }
    }
    pmml
}

fn to_pmml(
    dataset: &Dataset<'_>,
    data: &Data,
    settings: &LogisticSettings,
    pmml_beta: &[Vec<f64>],
) -> Pmml {
    let table = dataset.table;
    let mut dictionary = Vec::new();
    let mut schema = Vec::new();
    if let Some(field) = data_field(table, dataset.target) {
        dictionary.push(field);
    }
    if let Some(field) = mining_field(table, dataset.target, Usage::Target) {
        schema.push(field);
    }
    for factor in &data.factors {
        if let Some(field) = data_field(table, factor.position) {
            dictionary.push(field);
        }
        if let Some(field) = mining_field(table, factor.position, Usage::Active) {
            schema.push(field);
        }
    }
    for covariate in &data.covariates {
        if let Some(field) = data_field(table, covariate.position) {
            dictionary.push(field);
        }
        if let Some(field) = mining_field(table, covariate.position, Usage::Active) {
            schema.push(field);
        }
    }

    let parameters: Vec<String> = data.params.iter().map(|p| p.name.clone()).collect();
    let factors: Vec<String> = data.factors.iter().map(|f| f.name.clone()).collect();
    let covariates: Vec<String> = data.covariates.iter().map(|c| c.name.clone()).collect();

    let mut pp_matrix = Vec::new();
    for param in &data.params {
        match &param.kind {
            ParamKind::Covariate { field } => pp_matrix.push(PPCell {
                value: "1".into(),
                predictor: field.clone(),
                parameter: param.name.clone(),
            }),
            ParamKind::Factor { field, value, .. } => pp_matrix.push(PPCell {
                value: value.clone(),
                predictor: field.clone(),
                parameter: param.name.clone(),
            }),
            ParamKind::Intercept => {}
        }
    }

    let mut param_matrix = Vec::new();
    for (class, label) in data.labels.iter().enumerate() {
        if class == 0 {
            continue;
        }
        let row = pmml_beta.get(class);
        for (q, param) in data.params.iter().enumerate() {
            let beta = row.and_then(|r| r.get(q)).copied().unwrap_or(0.0);
            param_matrix.push(PCell {
                target_category: Some(label.clone()),
                parameter: param.name.clone(),
                beta,
            });
        }
    }

    let algorithm = match settings.procedure {
        Procedure::Multinomial => "multinomialLogistic",
        Procedure::Binomial => "x-contrastLogistic",
    };
    let target_reference_category = match settings.procedure {
        Procedure::Multinomial => None,
        Procedure::Binomial => data.labels.last().cloned(),
    };

    Pmml {
        version: "4.3".into(),
        data_dictionary: dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::GeneralRegression(GeneralRegressionModel {
            header: ModelHeader {
                name: settings.name.clone(),
                function: MiningFunction::Classification,
                algorithm: Some(algorithm.into()),
                mining_schema: schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            model_type: GeneralModelType::MultinomialLogistic,
            target_reference_category,
            link_function: None,
            link_parameter: None,
            parameters,
            factors,
            covariates,
            pp_matrix,
            param_matrix,
        }),
    }
}
