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
//! - Newton steps are solved with the SWEEP operator, and from the 20th iteration a completely
//!   separated fit stops (`logreg_multinomial`: DRUG1n is completely separated; Modeler's model is
//!   the 19th iterate of this path, log-likelihood -1.33358e-5, coefficients within 1.2e-3
//!   relative, the rest only on weakly identified drugC parameters, where floating-point order
//!   matters).
//! - Former GAP, kept for context (`logreg_multinomial`): the MLE is at infinity.
//!   Twenty Newton steps reach ℓ ≈ 0 and the same predicted class as Modeler on every record;
//!   some coefficients (weakly identified, e.g. K for drugC) differ because they live in the
//!   null space of the information matrix. The binomial HEART case has a unique MLE and
//!   matches Modeler to 1e-8.
//!
//! - Convergence: when a Newton step changes no parameter by more than `parameterConvergence`,
//!   Modeler moves only a quarter of that step and stops (`team/model_comparison`: every
//!   coefficient of the stepwise model sits at 0.2500 of the last step; the binomial
//!   `logreg_binomial` coefficients move from 6e-9 to 3e-9 of Modeler's).
//! - Stepwise selection (multinomial `stepwise`, `forwards`, `backwards`; Algorithms Guide
//!   "Stepwise Variable Selection"): from the constant-only model, the input whose score test has
//!   the smallest p-value enters while it is below `pIn`; with `stepwise`, after each entry the
//!   input whose likelihood-ratio test has the largest p-value leaves while it is above `pOut`
//!   (never the one just entered); `backwards` starts from every input and only removes. The
//!   model keeps the inputs in input order (`team/model_comparison`: `Credit_rating` on Age,
//!   Income and Credit_cards out of five inputs, 6/6 terminals). Records are those complete on the
//!   inputs of each fitted model.
//!
//! Supported: Enter and the stepwise methods above (Score entry, LR removal), main effects, with
//! or without a constant. The binomial stepwise methods,
//! custom terms, full factorial, a non-zero cell `delta`, scaling, frequency weights
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

/// How the multinomial procedure chooses its effects (`LogregMethod`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `enter`: every input.
    Enter,
    /// `stepwise`: forward entry by the score test, then removal by the likelihood-ratio test.
    Stepwise,
    /// `forwards`: forward entry only.
    Forwards,
    /// `backwards`: start with every input and remove by the likelihood-ratio test.
    Backwards,
}

/// Settings of a Logistic build node (Modeler's defaults in [`Default`]).
#[derive(Debug, Clone, PartialEq)]
pub struct LogisticSettings {
    /// `logistic_procedure` (default Multinomial).
    pub procedure: Procedure,
    /// `include_constant` (default true).
    pub include_constant: bool,
    /// `method` (default Enter).
    pub method: Method,
    /// `p_in` / `pIn`: the entry significance (default 0.05).
    pub p_in: f64,
    /// `p_out` / `pOut`: the removal significance (default 0.1).
    pub p_out: f64,
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
            method: Method::Enter,
            p_in: 0.05,
            p_out: 0.1,
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
    if settings.method != Method::Enter {
        let inputs = select_effects(dataset, settings)?;
        let chosen = Dataset {
            inputs: &inputs,
            ..*dataset
        };
        let data = Data::read(&chosen, settings)?;
        let beta = newton(&data, settings)?;
        let pmml_beta = reparameterise(&data, &beta);
        return Ok(to_pmml(&chosen, &data, settings, &pmml_beta));
    }

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
    for iteration in 0..settings.max_iterations {
        // From the 20th iteration on, complete separation (every record predicted in its own
        // category) stops at the current estimates (`models/logreg_multinomial`: Modeler's
        // coefficients are the 19th iterate's, log-likelihood -1.33358e-5).
        if iteration + 1 >= SEPARATION_CHECK && completely_separated(data, &beta) {
            break;
        }
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
        if change < settings.parameter_convergence {
            // The converging step goes a quarter of the way: Modeler's estimates sit at 0.25 of
            // the last Newton step on every coefficient (team/model_comparison stepwise model).
            for (b, n) in beta.iter_mut().zip(&new_beta) {
                *b += 0.25 * (n - *b);
            }
            break;
        }
        beta = new_beta;
        if settings.log_likelihood_convergence > 0.0
            && (new_loglik - loglik).abs() < settings.log_likelihood_convergence
        {
            break;
        }
    }
    Ok(beta)
}

/// The iteration from which the estimation checks for complete separation.
const SEPARATION_CHECK: usize = 20;

/// Every record's most probable category is its own.
fn completely_separated(data: &Data, beta: &[f64]) -> bool {
    let m = data.non_ref.len();
    let p = data.p_est;
    data.records.iter().all(|record| {
        let Some(pi) = softmax_non_ref(&record.x, beta, m, p) else {
            return false;
        };
        let reference = 1.0 - pi.iter().sum::<f64>();
        let (mut best, mut best_class) = (reference, data.est_ref);
        for (j, &class) in data.non_ref.iter().enumerate() {
            if pi[j] > best {
                best = pi[j];
                best_class = class;
            }
        }
        best_class == record.class
    })
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

/// Solves `matrix * x = rhs` for the symmetric information matrix with the SWEEP operator in
/// parameter order (the Algorithms Guide's estimation; Gaussian elimination with pivoting follows
/// another path on separated data). A parameter whose pivot falls to `tol` times its original
/// diagonal or below is aliased: its component of `x` is 0.
fn solve_system(matrix: &[f64], rhs: &[f64], n: usize, tol: f64) -> Option<Vec<f64>> {
    let len = n.checked_mul(n)?;
    if matrix.len() < len || rhs.len() < n {
        return None;
    }
    let mut a = matrix[..len].to_vec();
    let original: Vec<f64> = (0..n).map(|k| a[k * n + k]).collect();
    let mut swept = vec![false; n];
    for k in 0..n {
        let pivot = a[k * n + k];
        if !pivot.is_finite() || pivot <= tol * original[k].abs() || pivot <= 0.0 {
            continue;
        }
        swept[k] = true;
        let row: Vec<f64> = (0..n).map(|j| a[k * n + j]).collect();
        for i in (0..n).filter(|&i| i != k) {
            let factor = a[i * n + k] / pivot;
            for j in (0..n).filter(|&j| j != k) {
                a[i * n + j] -= factor * row[j];
            }
        }
        for j in (0..n).filter(|&j| j != k) {
            a[k * n + j] = row[j] / pivot;
            a[j * n + k] /= pivot;
        }
        a[k * n + k] = -1.0 / pivot;
    }
    // The swept block of `a` now holds minus the inverse of that block of `matrix`.
    let mut x = vec![0.0; n];
    for i in (0..n).filter(|&i| swept[i]) {
        x[i] = (0..n)
            .filter(|&j| swept[j])
            .map(|j| -a[i * n + j] * rhs[j])
            .sum();
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

// ------------------------------------------------------------------------------------------
// Stepwise selection (NOMREG, Algorithms Guide "Stepwise Variable Selection")

/// A fitted model on a subset of the inputs: its data, estimates and log-likelihood.
struct Fit {
    data: Data,
    beta: Vec<f64>,
    loglik: f64,
}

fn fit(
    dataset: &Dataset<'_>,
    inputs: &[usize],
    settings: &LogisticSettings,
) -> Result<Fit, AlgoError> {
    let subset = Dataset { inputs, ..*dataset };
    let data = Data::read(&subset, settings)?;
    let beta = newton(&data, settings)?;
    let loglik = loglik_at(&data, &beta)
        .ok_or_else(|| AlgoError::Data("the logistic log-likelihood is not finite".into()))?;
    Ok(Fit { data, beta, loglik })
}

/// The key of an estimated parameter, to carry estimates between nested models.
fn param_key(param: &Param) -> String {
    match &param.kind {
        ParamKind::Intercept => "I".into(),
        ParamKind::Covariate { field } => format!("C:{field}"),
        ParamKind::Factor { field, value, .. } => format!("F:{field}={value}"),
    }
}

/// The score statistic for adding the parameters of `larger` that `fit` lacks, at `fit`'s
/// estimates (zero for the new parameters), and its degrees of freedom.
fn score_statistic(fit: &Fit, larger: &Data, settings: &LogisticSettings) -> Option<(f64, f64)> {
    let m = larger.non_ref.len();
    let p = larger.p_est;
    let small_p = fit.data.p_est;
    let mut small: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (param, est) in fit.data.params.iter().zip(&fit.data.est_of) {
        if let Some(s) = est {
            small.insert(param_key(param), *s);
        }
    }
    let mut beta = vec![0.0; m.checked_mul(p)?];
    let mut added = 0usize;
    for (param, est) in larger.params.iter().zip(&larger.est_of) {
        let Some(s) = est else { continue };
        match small.get(&param_key(param)) {
            Some(&t) => {
                for j in 0..m {
                    beta[j * p + s] = fit.beta.get(j * small_p + t).copied().unwrap_or(0.0);
                }
            }
            None => added += 1,
        }
    }
    let (_, score, info) = derivatives(larger, &beta)?;
    let solved = solve_system(&info, &score, m * p, settings.singularity)?;
    let statistic: f64 = score.iter().zip(&solved).map(|(u, x)| u * x).sum();
    Some((statistic, (added * m) as f64))
}

/// The inputs a stepwise, forward or backward method keeps, in input order.
fn select_effects(
    dataset: &Dataset<'_>,
    settings: &LogisticSettings,
) -> Result<Vec<usize>, AlgoError> {
    if settings.procedure != Procedure::Multinomial {
        return Err(AlgoError::Unsupported(
            "stepwise methods of the binomial procedure".into(),
        ));
    }
    if !settings.include_constant {
        return Err(AlgoError::Unsupported(
            "stepwise selection without a constant".into(),
        ));
    }
    let all = dataset.inputs;
    let ordered = |chosen: &[usize]| -> Vec<usize> {
        all.iter().copied().filter(|p| chosen.contains(p)).collect()
    };
    let mut chosen: Vec<usize> = match settings.method {
        Method::Backwards => all.to_vec(),
        _ => Vec::new(),
    };
    let removal = |chosen: &mut Vec<usize>, keep: Option<usize>| -> Result<bool, AlgoError> {
        let current = fit(dataset, &ordered(chosen), settings)?;
        let mut worst: Option<(usize, f64)> = None;
        for &effect in chosen.iter() {
            if Some(effect) == keep {
                continue;
            }
            let without: Vec<usize> = chosen.iter().copied().filter(|&e| e != effect).collect();
            let reduced = fit(dataset, &ordered(&without), settings)?;
            let df = (current.data.p_est - reduced.data.p_est) * current.data.non_ref.len();
            let statistic = (2.0 * (current.loglik - reduced.loglik)).max(0.0);
            let p = crate::stats::chi_square_p(statistic, df as f64);
            if worst.is_none_or(|(_, w)| p > w) {
                worst = Some((effect, p));
            }
        }
        match worst {
            Some((effect, p)) if p > settings.p_out => {
                chosen.retain(|&e| e != effect);
                Ok(true)
            }
            _ => Ok(false),
        }
    };
    if settings.method == Method::Backwards {
        while !chosen.is_empty() && removal(&mut chosen, None)? {}
        return Ok(ordered(&chosen));
    }
    // Forward entry; stepwise also removes after each entry. Each subset is entered at most
    // once, so the search ends.
    let mut seen: Vec<Vec<usize>> = vec![ordered(&chosen)];
    loop {
        let current = fit(dataset, &ordered(&chosen), settings)?;
        let mut best: Option<(usize, f64)> = None;
        for &candidate in all.iter().filter(|p| !chosen.contains(p)) {
            let mut with = chosen.clone();
            with.push(candidate);
            let subset = ordered(&with);
            let larger = Data::read(
                &Dataset {
                    inputs: &subset,
                    ..*dataset
                },
                settings,
            )?;
            let Some((statistic, df)) = score_statistic(&current, &larger, settings) else {
                continue;
            };
            let p = crate::stats::chi_square_p(statistic, df);
            if best.is_none_or(|(_, b)| p < b) {
                best = Some((candidate, p));
            }
        }
        let Some((entered, p)) = best.filter(|&(_, p)| p < settings.p_in) else {
            break;
        };
        let _ = p;
        chosen.push(entered);
        if settings.method == Method::Stepwise {
            while removal(&mut chosen, Some(entered))? {}
        }
        let key = ordered(&chosen);
        if seen.contains(&key) {
            break;
        }
        seen.push(key);
    }
    Ok(ordered(&chosen))
}
