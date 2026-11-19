//! Cox proportional hazards regression (IBM SPSS Modeler 19 Algorithms Guide, "COXREG
//! Algorithms", pp. 97-107).
//!
//! Pure algorithm module: takes plain vectors ([`CoxData`]) and [`Settings`], fits a stratified
//! proportional hazards model via Newton-Raphson on Breslow's partial likelihood, optionally
//! selects covariates by forward or backward stepwise selection (Score entry; LR, Wald, or
//! Conditional removal), estimates per-stratum baseline cumulative hazard and survival functions,
//! and scores cumulative hazard and survival probabilities.
//!
//! # Decisions the Guide does not settle
//!
//! Fixed and verified against the saved Modeler model (`target/agy-cox/model/saved_cox_model.pmml.txt`,
//! trained on `target/agy-cox/data/telco.csv` from the `telco_coxreg` demo stream):
//!
//! A. **Tie handling (partial likelihood).** Uses Breslow's partial likelihood (Guide pp. 98-99).
//!    Reproduces the saved Modeler model: all 12 parameter estimates match `ParamMatrix` to
//!    `4.5e-15` relative error (whereas Efron's tie rule differs by up to `2.4e-2` relative).
//!    [`TieHandling::Efron`] is not in the Guide or `coxregnode` properties and returns a `GAP:`
//!    error.
//! B. **Baseline cumulative hazard estimator.** Uses the Kalbfleisch-Prentice iterative full
//!    likelihood estimator for `alpha_i` (`H_0(t) = -ln prod_{t_i <= t} alpha_i`, Guide
//!    pp. 100-101), solved via bracketed Newton-Raphson in `z_i = -ln(alpha_i)` starting from the
//!    Breslow initial value `d_i / S_0(t_i)`. Reproduces the saved Modeler model: all 64 rows of
//!    `X-BaseCumHazardTable` match to `3.6e-11` relative error (every untied jump to `< 1e-13`;
//!    the `~3e-11` residual on heavily tied jumps reflects Modeler's inner stopping tolerance on
//!    `alpha_i`), whereas the Breslow baseline differs by `5.9e-2` relative.
//! C. **Baseline step convention and reference point.** The Guide p. 100 writes `S_0(t)` as
//!    left-continuous (`S_0(t_1) = 1`, `S_0(t_i+) = p_i`), while the saved PMML's
//!    `X-BaseCumHazardTable` records right-continuous jumps `H_0(t_i+) = sum_{t_r <= t_i} -ln(alpha_r)`
//!    (e.g. `time="1"` has `cumHazard="0.612731262966062"`), evaluated at covariate reference
//!    point `0` (`X-BasePoint` = `0` for all 12 parameters). Reproduces the saved Modeler model:
//!    [`ReferencePoint::Zero`] is the default (`ReferencePoint::Mean` is also available to state
//!    `H_0` at the weighted covariate means). For scoring `H(t | x) = H_0(t) exp(beta.(x - ref))`,
//!    both reference points produce identical predictions.
//! D. **Newton-Raphson convergence rule.** Guide p. 99 defines relative parameter change `BCON`
//!    and relative log-likelihood change `LCON` but does not state whether they combine with AND
//!    or OR. On `telco_coxreg` (`p_converge = 1e-4`, `l_converge = 1e-5`), iteration 5 has
//!    `BCON = 6.59e-3` and `LCON = 1.84e-6`, while iteration 6 has `BCON = 2.12e-5` and
//!    `LCON = 1.93e-11`. Stopping when **both** enabled criteria hold (`BCON < p_converge` AND
//!    `LCON < l_converge`) stops at iteration 6 and reproduces the saved Modeler model's betas to
//!    `4.5e-15` relative error (OR on relative `LCON` stops at iteration 5 with `2.1e-5` error;
//!    note that OR with absolute `|dl| < l_converge` would also stop at iteration 6, so the saved
//!    model rules out OR-with-relative-LCON). When a previous parameter or log-likelihood has
//!    magnitude `< 1e-12`, the absolute change is used to avoid division by zero.
//! E. **Factor indicator coding and parameter order.** Factors use Indicator coding with the LAST
//!    level as the reference category (`K - 1` dummy columns for levels `0..K-1`, level `K-1` all
//!    zeros). Reproduces the saved Modeler model: `FactorList` `x-Basis` matrices code value `0`
//!    as `1` and value `1` as `0`, and all 7 binary factor betas match sign and magnitude to
//!    `4.5e-15`. After stepwise selection, retained covariates are ordered in their original input
//!    order, matching `ParameterList` (`address, employ, reside, equip, callcard, longmon,
//!    equipmon, multline, voice, internet, callid, ebill`).
//! F. **Stepwise selection on `telco_coxreg`.** In `build_node_settings.txt`, `_groups` (strata)
//!    is empty and `_separate_line = custcat` is a plot option, not a stratum; `custcat` appears
//!    in `FactorList` as a candidate predictor. Forward Stepwise (`p_entry = 0.05`,
//!    `p_removal = 0.10`, `RemovalCriterion::Lr`) over the 28 `DataDictionary` candidate
//!    predictors reproduces the saved Modeler model's exact 12 predictors in 12 entry steps
//!    (0 removals) and stops when `tollfree` has Score `3.2431` (`p = 0.0717 >= 0.05`).
//!
//! Follows the Algorithms Guide (unverified against a saved Modeler model with these features):
//!
//! 1. **Wald statistic typo on Guide p. 102.** The Guide prints `beta_j' B_{11,j} beta_j` where
//!    `B_{11,j}` is the submatrix of `A_{11}^{-1}` (missing the inverse superscript). We use
//!    `beta_j' B_{11,j}^{-1} beta_j` so that for a single parameter `Wald = (beta / SE)^2`
//!    (follows the Algorithms Guide intent; unverified on multi-level factors).
//! 2. **Stepwise control flow, tie-breaking, and cycle prevention.** Entry picks the excluded
//!    covariate with smallest Score p-value (ties broken by largest Score statistic, then lowest
//!    covariate index) when `p < p_entry`. Removal picks the included covariate with largest
//!    removal p-value (ties broken by smallest statistic, then largest covariate index) when
//!    `p > p_removal`, never removing the covariate just entered in the same step (nor re-entering
//!    a covariate just removed in `BackwardsStepwise`). A visited-subset set prevents cycles if
//!    `p_entry >= p_removal` (follows the Algorithms Guide; unverified on BackwardsStepwise,
//!    Wald, and Conditional removal).
//! 3. **Step-halving and separation guard.** Newton-Raphson halves the step up to 10 times if a
//!    full step decreases the partial log-likelihood or produces non-finite values (never triggers
//!    on `telco_coxreg`). If `max |beta_r| > 100.0`, the information matrix becomes singular, or
//!    convergence is not reached within `max_iterations`, training returns `Err(CoxError::Data)`
//!    rather than a diverging or unconverged fit (unverified).
//! 4. **Baseline at `R_i = D_i` and scoring beyond `max_event_time`.** Under Kalbfleisch-Prentice,
//!    if all remaining at-risk cases fail at the last event time (`R_k = D_k`), `alpha_k = 0`,
//!    `H_0(t_k) = +inf`, and `S(t_k | x) = 0.0` (Guide p. 100: `p_k = 0`). [`CoxModel::survival`]
//!    and [`CoxModel::cumulative_hazard`] evaluate the right-continuous step function for all
//!    `t >= 0` (`H_0(t) = 0` before the first event, constant at `H_0(t_k)` after the last event).
//!    [`CoxModel::conditional_survival`] implements Guide p. 106 scoring, returning `Ok(None)`
//!    (`$null$`) when any active model covariate is missing or `t0 + future_t > max_event_time`.
//! 5. **Blank handling (Guide p. 106).** Complete-case deletion across all input covariates, time,
//!    event, stratum (if provided), and weight (if provided) before estimation and stepwise
//!    selection so nested models share identical samples. A factor with 1 level contributes 0
//!    indicator parameters (skipped in Enter if other covariates exist, or ineligible in Stepwise;
//!    if no estimable parameters exist in Enter, fits the null model).
//! 6. **Explicit `GAP:` errors.**
//!    - [`TieHandling::Efron`]: not defined in the Algorithms Guide or `coxregnode` properties.
//!    - [`ModelType::Custom`] (interactions / custom terms): not specified in the COXREG chapter.
//!    - Time-dependent covariates (`Settings::time_dependent`): explicitly out of scope.
//!    - Survival / hazard / LML plots (`Settings::plots`): explicitly out of scope.

use crate::AlgoError;
use crate::stats::chi_square_p;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

/// Hard limit on the number of input records.
const MAX_RECORDS: usize = 1_000_000;
/// Hard limit on the number of candidate covariates.
const MAX_COVARIATES: usize = 512;
/// Hard limit on the number of expanded indicator/numeric parameters.
const MAX_PARAMETERS: usize = 512;
/// Hard limit on the number of levels in a single factor.
const MAX_CATEGORIES: usize = 1_000;
/// Hard limit on the number of distinct strata.
const MAX_STRATA: usize = 10_000;
/// Hard limit on Newton-Raphson iterations per model fit.
const MAX_ITERATIONS: usize = 1_000;
/// Hard limit on stepwise entry/removal steps.
const MAX_STEPWISE_STEPS: usize = 2_000;
/// Hard limit on design matrix cells (`records * total_parameters`).
const MAX_DESIGN_CELLS: usize = 50_000_000;
/// Hard limit on inner Newton iterations for Kalbfleisch-Prentice baseline hazards.
const MAX_KP_ITERATIONS: usize = 100;
/// Maximum backtracking step-halvings per Newton-Raphson iteration.
const MAX_STEP_HALVING: usize = 10;
/// Divergence guard on `|beta_r|` to catch complete/quasi-complete separation.
const MAX_BETA_ABS: f64 = 100.0;
/// Relative pivot threshold for declaring an information or covariance block singular.
const SINGULARITY_TOL: f64 = 1e-12;

/// Error type returned by Cox regression training and scoring.
pub type CoxError = AlgoError;

fn gap(what: &str) -> CoxError {
    AlgoError::Unsupported(format!("GAP: Cox regression {what}"))
}

/// A single candidate predictor column for Cox regression.
#[derive(Debug, Clone, PartialEq)]
pub enum Covariate {
    /// Continuous numeric covariate (`None` = system-missing).
    Numeric {
        /// Field name.
        name: String,
        /// Per-record values.
        values: Vec<Option<f64>>,
    },
    /// Categorical factor coded with Indicator contrasts (last level = reference).
    Factor {
        /// Field name.
        name: String,
        /// Ordered category labels; the last entry is the reference level.
        levels: Vec<String>,
        /// Per-record zero-based category indices (`None` = system-missing).
        codes: Vec<Option<u32>>,
    },
}

impl Covariate {
    /// Name of this covariate.
    pub fn name(&self) -> &str {
        match self {
            Covariate::Numeric { name, .. } | Covariate::Factor { name, .. } => name,
        }
    }

    /// Number of records stored in this covariate.
    pub fn len(&self) -> usize {
        match self {
            Covariate::Numeric { values, .. } => values.len(),
            Covariate::Factor { codes, .. } => codes.len(),
        }
    }

    /// Whether this covariate contains no records.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Training data for Cox proportional hazards regression.
#[derive(Debug, Clone, PartialEq)]
pub struct CoxData {
    /// Observed survival or censoring times (`>= 0.0`, finite).
    pub time: Vec<f64>,
    /// Status indicator: `true` = uncensored event, `false` = right-censored.
    pub event: Vec<bool>,
    /// Optional stratification variable (`groups` property); `None` entries are excluded as missing.
    pub strata: Option<Vec<Option<u32>>>,
    /// Optional positive case weights (`w_l` in Guide pp. 98-101); defaults to `1.0` per record.
    pub weights: Option<Vec<f64>>,
    /// Candidate covariates (continuous or categorical).
    pub covariates: Vec<Covariate>,
}

/// A single covariate value supplied when scoring a record with [`CoxModel`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CoxValue {
    /// Continuous numeric value.
    Numeric(f64),
    /// Zero-based factor category code matching the training [`Covariate::Factor`] levels.
    Factor(u32),
    /// Missing value.
    Missing,
}

/// Variable selection method (`coxregnode` `method` property).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Enter all candidate covariates simultaneously.
    Enter,
    /// Forward stepwise selection: Score entry, then removal by [`RemovalCriterion`].
    Stepwise,
    /// Backward stepwise selection: start with all covariates, remove by [`RemovalCriterion`],
    /// then check Score re-entry.
    BackwardsStepwise,
}

/// Removal test criterion for stepwise methods (`coxregnode` `removal_criterion` property).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalCriterion {
    /// Likelihood-ratio test (`-2(l_reduced - l_full)` using refitted MPLEs).
    Lr,
    /// Wald test (`beta_j' C_{jj}^{-1} beta_j`).
    Wald,
    /// Conditional likelihood-ratio test using conditional reduced estimates (Guide pp. 102-103).
    Conditional,
}

/// Partial-likelihood tie handling rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TieHandling {
    /// Breslow approximation (Algorithms Guide pp. 98-99; confirmed by saved Modeler model).
    Breslow,
    /// Efron approximation (not in the Algorithms Guide or `coxregnode`; returns `GAP:`).
    Efron,
}

/// Baseline cumulative hazard estimator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaselineMethod {
    /// Kalbfleisch-Prentice iterative full-likelihood estimator (Guide pp. 100-101; default).
    KalbfleischPrentice,
    /// Breslow cumulative hazard estimator (`sum d_i / S_0(t_i)`).
    Breslow,
}

/// Covariate point at which the baseline cumulative hazard `H_0(t)` is stated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferencePoint {
    /// Origin `x = 0` (`X-BasePoint` = 0 in saved Modeler PMML; default).
    Zero,
    /// Overall weighted covariate means `x = x_bar` (Guide p. 99 centering).
    Mean,
}

/// Model term structure (`coxregnode` `model_type` property).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelType {
    /// Main effects only (supported).
    MainEffects,
    /// Custom terms / interactions (returns `GAP:`).
    Custom,
}

/// Configuration for Cox proportional hazards training (`coxregnode` properties).
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Variable selection method (default [`Method::Enter`]).
    pub method: Method,
    /// Removal statistic for stepwise methods (default [`RemovalCriterion::Lr`]).
    pub removal_criterion: RemovalCriterion,
    /// Significance threshold for forward entry (`probability_entry`, default `0.05`).
    pub probability_entry: f64,
    /// Significance threshold for backward removal (`probability_removal`, default `0.10`).
    pub probability_removal: f64,
    /// Maximum Newton-Raphson iterations (`max_iterations`, default `20`).
    pub max_iterations: usize,
    /// Relative parameter convergence tolerance (`p_converge`, default `1e-4`; `0.0` disables).
    pub parameter_convergence: f64,
    /// Relative log-likelihood convergence tolerance (`l_converge`, default `1e-5`; `0.0` disables).
    pub log_likelihood_convergence: f64,
    /// Tie handling method (default [`TieHandling::Breslow`]).
    pub ties: TieHandling,
    /// Baseline cumulative hazard estimator (default [`BaselineMethod::KalbfleischPrentice`]).
    pub baseline_method: BaselineMethod,
    /// Covariate reference point for the baseline table (default [`ReferencePoint::Zero`]).
    pub reference_point: ReferencePoint,
    /// Main effects vs custom terms (default [`ModelType::MainEffects`]).
    pub model_type: ModelType,
    /// Whether time-dependent covariates are requested (out of scope: `true` returns `GAP:`).
    pub time_dependent: bool,
    /// Whether survival/hazard/LML plot generation is requested (out of scope: `true` returns `GAP:`).
    pub plots: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            method: Method::Enter,
            removal_criterion: RemovalCriterion::Lr,
            probability_entry: 0.05,
            probability_removal: 0.10,
            max_iterations: 20,
            parameter_convergence: 1e-4,
            log_likelihood_convergence: 1e-5,
            ties: TieHandling::Breslow,
            baseline_method: BaselineMethod::KalbfleischPrentice,
            reference_point: ReferencePoint::Zero,
            model_type: ModelType::MainEffects,
            time_dependent: false,
            plots: false,
        }
    }
}

/// Kind of a covariate recorded in the trained model schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CovariateKind {
    /// Continuous numeric covariate.
    Numeric,
    /// Categorical factor with its ordered level labels (last = reference).
    Factor {
        /// Ordered category labels.
        levels: Vec<String>,
    },
}

/// Summary of an entered covariate term in the final model.
#[derive(Debug, Clone, PartialEq)]
pub struct EnteredTerm {
    /// Index of the covariate in the original [`CoxData::covariates`] slice.
    pub covariate_index: usize,
    /// Covariate name.
    pub name: String,
    /// Whether numeric or factor.
    pub kind: CovariateKind,
    /// Starting index in [`CoxModel::parameters`].
    pub param_start: usize,
    /// Number of estimated parameters (degrees of freedom) for this term.
    pub df: usize,
    /// Overall Wald chi-square statistic for this term.
    pub wald: f64,
    /// Asymptotic chi-square p-value for the term's Wald statistic.
    pub p_value: f64,
}

/// Estimated regression parameter in the final Cox model.
#[derive(Debug, Clone, PartialEq)]
pub struct ParameterEstimate {
    /// Parameter display label (`name` for numeric, `name=level` for multi-level factor).
    pub name: String,
    /// Index of the source covariate in [`CoxData::covariates`].
    pub covariate_index: usize,
    /// Name of the source covariate.
    pub covariate_name: String,
    /// Zero-based level index if this parameter is a factor indicator (`None` for numeric).
    pub level_index: Option<usize>,
    /// Level label if this parameter is a factor indicator (`None` for numeric).
    pub level_label: Option<String>,
    /// Maximum partial likelihood estimate (`beta`).
    pub beta: f64,
    /// Asymptotic standard error `sqrt(Cov[j, j])`.
    pub std_error: f64,
    /// Single-parameter Wald statistic `(beta / std_error)^2`.
    pub wald: f64,
    /// Degrees of freedom (`1`).
    pub df: usize,
    /// Asymptotic chi-square p-value (`df = 1`).
    pub p_value: f64,
    /// Hazard ratio `exp(beta)`.
    pub exp_beta: f64,
}

/// One event-time point in a stratum's baseline hazard / survival table.
#[derive(Debug, Clone, PartialEq)]
pub struct BaselinePoint {
    /// Observed uncensored event time `t_i`.
    pub time: f64,
    /// Baseline cumulative hazard `H_0(t_i)` at [`CoxModel::reference_point`].
    pub cum_hazard: f64,
    /// Baseline survival `S_0(t_i) = exp(-H_0(t_i))` at [`CoxModel::reference_point`].
    pub survival: f64,
    /// Asymptotic standard error of `H_0(t_i)` (Guide pp. 101, 105).
    pub std_error_cum_hazard: f64,
    /// Weighted event count `d_i` at `t_i`.
    pub events: f64,
    /// Weighted number at risk just prior to `t_i`.
    pub at_risk: f64,
}

/// Baseline cumulative hazard and survival table for one stratum.
#[derive(Debug, Clone, PartialEq)]
pub struct StratumBaseline {
    /// Stratum identifier (`None` when the model is unstratified).
    pub stratum: Option<u32>,
    /// Maximum observed time (event or censored) in this stratum.
    pub max_time: f64,
    /// Maximum observed uncensored event time in this stratum (`None` if censored-only).
    pub max_event_time: Option<f64>,
    /// Ordered baseline points at each distinct uncensored event time.
    pub points: Vec<BaselinePoint>,
}

/// Action taken at one step of stepwise variable selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepAction {
    /// Covariate entered into the model (Score test).
    Enter,
    /// Covariate removed from the model (LR, Wald, or Conditional test).
    Remove,
}

/// Audit record for one step of stepwise variable selection.
#[derive(Debug, Clone, PartialEq)]
pub struct StepRecord {
    /// One-based step number.
    pub step: usize,
    /// Whether the covariate was entered or removed.
    pub action: StepAction,
    /// Index of the covariate in [`CoxData::covariates`].
    pub covariate_index: usize,
    /// Name of the covariate.
    pub covariate_name: String,
    /// Entry Score statistic or removal test statistic.
    pub statistic: f64,
    /// Degrees of freedom of the test.
    pub df: usize,
    /// Chi-square significance (p-value) of the test.
    pub p_value: f64,
    /// Partial log-likelihood after the step.
    pub log_likelihood: f64,
    /// Model chi-square `2 * (l_current - l_initial)` (Guide p. 104).
    pub model_chi_square: f64,
    /// Improvement chi-square `2 * (l_current - l_previous)` (Guide p. 104).
    pub improvement_chi_square: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CovariateSchema {
    name: String,
    kind: CovariateKind,
}

/// Fitted Cox proportional hazards regression model.
#[derive(Debug, Clone, PartialEq)]
pub struct CoxModel {
    /// Terms entered in the final model (in original input order).
    pub terms: Vec<EnteredTerm>,
    /// Estimated regression parameters (in original input order).
    pub parameters: Vec<ParameterEstimate>,
    /// Asymptotic covariance matrix `I^{-1}` (`p x p`).
    pub covariance: Vec<Vec<f64>>,
    /// Null-model partial log-likelihood `l(0)`.
    pub initial_log_likelihood: f64,
    /// Final partial log-likelihood `l(beta_hat)`.
    pub log_likelihood: f64,
    /// Newton-Raphson iterations performed for the final model fit.
    pub iterations: usize,
    /// Number of complete records used for training.
    pub n_records: usize,
    /// Number of uncensored event records used for training.
    pub n_events: usize,
    /// Number of right-censored records used for training.
    pub n_censored: usize,
    /// Covariate reference point `x_0` at which baseline tables are evaluated (length `p`).
    pub reference_point: Vec<f64>,
    /// Overall weighted means `x_bar` of the estimated parameter columns (length `p`).
    pub covariate_means: Vec<f64>,
    /// Per-stratum baseline cumulative hazard and survival tables.
    pub baselines: Vec<StratumBaseline>,
    /// Stepwise selection trace (empty for [`Method::Enter`]).
    pub step_history: Vec<StepRecord>,
    /// Schema of all training covariates (for validating scoring records).
    covariate_schemas: Vec<CovariateSchema>,
    /// Whether the model was trained with stratification.
    stratified: bool,
}

impl CoxModel {
    /// Computes the linear predictor `beta . (x - reference_point)` for `record`.
    ///
    /// `record` must contain one [`CoxValue`] per training covariate in [`CoxData::covariates`]
    /// order. Covariates not in the final model may be [`CoxValue::Missing`].
    pub fn linear_predictor(&self, record: &[CoxValue]) -> Result<f64, CoxError> {
        if record.len() != self.covariate_schemas.len() {
            return Err(CoxError::Data(format!(
                "record has {} values, expected {} training covariates",
                record.len(),
                self.covariate_schemas.len()
            )));
        }
        let mut lp = 0.0;
        for (param_idx, param) in self.parameters.iter().enumerate() {
            let val = record
                .get(param.covariate_index)
                .ok_or_else(|| CoxError::Data("covariate index out of range".into()))?;
            let schema = self
                .covariate_schemas
                .get(param.covariate_index)
                .ok_or_else(|| CoxError::Data("covariate schema out of range".into()))?;
            let x_val = match (&schema.kind, val, param.level_index) {
                (CovariateKind::Numeric, CoxValue::Numeric(v), None) => {
                    if !v.is_finite() {
                        return Err(CoxError::Data(format!(
                            "non-finite numeric value for covariate '{}'",
                            schema.name
                        )));
                    }
                    *v
                }
                (CovariateKind::Factor { levels }, CoxValue::Factor(code), Some(level_idx)) => {
                    let code_usize = usize::try_from(*code)
                        .map_err(|_| CoxError::Data("factor code overflow".into()))?;
                    if code_usize >= levels.len() {
                        return Err(CoxError::Data(format!(
                            "factor code {} out of range for '{}' ({} levels)",
                            code,
                            schema.name,
                            levels.len()
                        )));
                    }
                    if code_usize == level_idx { 1.0 } else { 0.0 }
                }
                (_, CoxValue::Missing, _) => {
                    return Err(CoxError::Data(format!(
                        "missing value for active covariate '{}'",
                        schema.name
                    )));
                }
                _ => {
                    return Err(CoxError::Data(format!(
                        "value type mismatch for covariate '{}'",
                        schema.name
                    )));
                }
            };
            let ref_val = self.reference_point.get(param_idx).copied().unwrap_or(0.0);
            lp += param.beta * (x_val - ref_val);
        }
        if !lp.is_finite() {
            return Err(CoxError::Data("non-finite linear predictor".into()));
        }
        Ok(lp)
    }

    fn find_stratum(&self, stratum: Option<u32>) -> Result<&StratumBaseline, CoxError> {
        if self.stratified && stratum.is_none() {
            return Err(CoxError::Data(
                "stratum is required for a stratified Cox model".into(),
            ));
        }
        if !self.stratified && stratum.is_some() {
            return Err(CoxError::Data(
                "stratum provided for an unstratified Cox model".into(),
            ));
        }
        self.baselines
            .iter()
            .find(|b| b.stratum == stratum)
            .ok_or_else(|| CoxError::Data(format!("unknown stratum {:?}", stratum)))
    }

    /// Evaluates the baseline cumulative hazard `H_0(t)` for a stratum (step function,
    /// `0.0` before the first observed event time).
    pub fn baseline_cumulative_hazard(
        &self,
        stratum: Option<u32>,
        t: f64,
    ) -> Result<f64, CoxError> {
        if !t.is_finite() || t < 0.0 {
            return Err(CoxError::Data(
                "time t must be finite and non-negative".into(),
            ));
        }
        let table = self.find_stratum(stratum)?;
        let idx = table.points.partition_point(|pt| pt.time <= t);
        if idx == 0 {
            Ok(0.0)
        } else {
            Ok(table
                .points
                .get(idx - 1)
                .map(|pt| pt.cum_hazard)
                .unwrap_or(0.0))
        }
    }

    /// Computes cumulative hazard `H(t | x) = H_0(t) * exp(beta . (x - reference_point))`.
    pub fn cumulative_hazard(
        &self,
        record: &[CoxValue],
        stratum: Option<u32>,
        t: f64,
    ) -> Result<f64, CoxError> {
        let h0 = self.baseline_cumulative_hazard(stratum, t)?;
        let lp = self.linear_predictor(record)?;
        if h0 == 0.0 {
            return Ok(0.0);
        }
        if h0.is_infinite() {
            return Ok(f64::INFINITY);
        }
        let h = h0 * lp.exp();
        if h.is_nan() {
            return Err(CoxError::Data("cumulative hazard evaluated to NaN".into()));
        }
        Ok(h)
    }

    /// Computes survival probability `S(t | x) = exp(-H(t | x))`.
    pub fn survival(
        &self,
        record: &[CoxValue],
        stratum: Option<u32>,
        t: f64,
    ) -> Result<f64, CoxError> {
        let h = self.cumulative_hazard(record, stratum, t)?;
        if h.is_infinite() {
            return Ok(0.0);
        }
        Ok((-h).exp().clamp(0.0, 1.0))
    }

    /// Computes conditional survival `S(t0 + future_t | t0, x) = S(t0 + future_t | x) / S(t0 | x)`
    /// following Guide p. 106 ("Scoring" and "Blank Handling").
    ///
    /// Returns `Ok(None)` (`$null$`) if any active model covariate is missing, if the stratum is
    /// censored-only, if `t0 + future_t` exceeds the stratum's longest observed uncensored
    /// survival time, or if `S(t0 | x) == 0.0`.
    pub fn conditional_survival(
        &self,
        record: &[CoxValue],
        stratum: Option<u32>,
        t0: f64,
        future_t: f64,
    ) -> Result<Option<f64>, CoxError> {
        if !t0.is_finite() || t0 < 0.0 || !future_t.is_finite() || future_t < 0.0 {
            return Err(CoxError::Data(
                "t0 and future_t must be finite and non-negative".into(),
            ));
        }
        if record.len() != self.covariate_schemas.len() {
            return Err(CoxError::Data(format!(
                "record has {} values, expected {} training covariates",
                record.len(),
                self.covariate_schemas.len()
            )));
        }
        // Check missingness on active covariates first (Guide p. 106 -> $null$).
        for term in &self.terms {
            if matches!(record.get(term.covariate_index), Some(CoxValue::Missing)) {
                return Ok(None);
            }
        }
        let table = self.find_stratum(stratum)?;
        let Some(max_ev) = table.max_event_time else {
            return Ok(None);
        };
        let total_t = t0 + future_t;
        if !total_t.is_finite() || total_t > max_ev {
            return Ok(None);
        }
        let s0 = self.survival(record, stratum, t0)?;
        if s0 <= 0.0 {
            return Ok(None);
        }
        let s1 = self.survival(record, stratum, total_t)?;
        Ok(Some((s1 / s0).clamp(0.0, 1.0)))
    }

    /// Formats a human-readable summary containing betas, standard errors, log-likelihood,
    /// step history, and per-stratum baseline cumulative hazard tables.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "Cox Proportional Hazards Model");
        let _ = writeln!(
            out,
            "  Records: {} (events: {}, censored: {})",
            self.n_records, self.n_events, self.n_censored
        );
        let _ = writeln!(
            out,
            "  Initial LL: {:.10} (-2LL: {:.10})",
            self.initial_log_likelihood,
            -2.0 * self.initial_log_likelihood
        );
        let _ = writeln!(
            out,
            "  Final LL:   {:.10} (-2LL: {:.10}), iterations: {}",
            self.log_likelihood,
            -2.0 * self.log_likelihood,
            self.iterations
        );
        let _ = writeln!(out, "\nParameters:");
        let _ = writeln!(
            out,
            "  {:<20} {:>14} {:>12} {:>12} {:>4} {:>12} {:>12} {:>12}",
            "Name", "Beta", "SE", "Wald", "df", "Sig", "Exp(B)", "RefPoint"
        );
        for (i, p) in self.parameters.iter().enumerate() {
            let ref_pt = self.reference_point.get(i).copied().unwrap_or(0.0);
            let _ = writeln!(
                out,
                "  {:<20} {:>14.8} {:>12.8} {:>12.4} {:>4} {:>12.4e} {:>12.6} {:>12.6}",
                p.name, p.beta, p.std_error, p.wald, p.df, p.p_value, p.exp_beta, ref_pt
            );
        }
        if !self.step_history.is_empty() {
            let _ = writeln!(out, "\nStep History:");
            for s in &self.step_history {
                let verb = match s.action {
                    StepAction::Enter => "ENTER ",
                    StepAction::Remove => "REMOVE",
                };
                let _ = writeln!(
                    out,
                    "  Step {:>2}: {} {:<16} stat={:>10.4} df={} p={:.4e} LL={:.6}",
                    s.step, verb, s.covariate_name, s.statistic, s.df, s.p_value, s.log_likelihood
                );
            }
        }
        for b in &self.baselines {
            let _ = writeln!(
                out,
                "\nBaseline Table (stratum={:?}, max_time={}, points={}):",
                b.stratum,
                b.max_time,
                b.points.len()
            );
            let _ = writeln!(
                out,
                "  {:>10} {:>18} {:>14} {:>14} {:>8} {:>10}",
                "Time", "CumHazard", "Survival", "SE(H0)", "Events", "AtRisk"
            );
            for pt in &b.points {
                let _ = writeln!(
                    out,
                    "  {:>10.4} {:>18.10} {:>14.10} {:>14.8} {:>8.2} {:>10.2}",
                    pt.time,
                    pt.cum_hazard,
                    pt.survival,
                    pt.std_error_cum_hazard,
                    pt.events,
                    pt.at_risk
                );
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------------------------
// Internal preprocessed representation

struct StratumSlice {
    stratum: Option<u32>,
    start: usize,
    end: usize,
    max_time: f64,
    max_event_time: Option<f64>,
}

struct PreparedCovariate {
    covariate_index: usize,
    name: String,
    kind: CovariateKind,
    /// Column labels (length `df`; empty for a 1-level factor).
    param_names: Vec<String>,
    level_indices: Vec<Option<usize>>,
    level_labels: Vec<Option<String>>,
    /// Column-major raw values for sorted complete records (`df` columns of length `n`).
    raw_cols: Vec<Vec<f64>>,
    /// Weighted means of each column across all complete records (length `df`).
    means: Vec<f64>,
    /// Column-major mean-centered values (`df` columns of length `n`).
    centered_cols: Vec<Vec<f64>>,
}

struct PreparedData {
    n: usize,
    n_events: usize,
    n_censored: usize,
    time: Vec<f64>,
    event: Vec<bool>,
    weight: Vec<f64>,
    strata: Vec<StratumSlice>,
    stratified: bool,
    covariates: Vec<PreparedCovariate>,
    schemas: Vec<CovariateSchema>,
}

struct FitResult {
    beta: Vec<f64>,
    log_likelihood: f64,
    cov: Vec<f64>,
    iterations: usize,
}

/// Trains a Cox proportional hazards regression model on `data` with `settings`.
pub fn train(data: &CoxData, settings: &Settings) -> Result<CoxModel, CoxError> {
    crate::check_cancelled()?;
    validate_settings(settings)?;
    let prep = prepare_data(data)?;

    let initial_ll = null_log_likelihood(&prep);

    let (selected, step_history) = match settings.method {
        Method::Enter => {
            let all: Vec<usize> = prep
                .covariates
                .iter()
                .enumerate()
                .filter(|(_, c)| !c.raw_cols.is_empty())
                .map(|(i, _)| i)
                .collect();
            (all, Vec::new())
        }
        Method::Stepwise => run_forward_stepwise(&prep, settings, initial_ll)?,
        Method::BackwardsStepwise => run_backward_stepwise(&prep, settings, initial_ll)?,
    };

    // Sort selected covariates back to original input order (confirmed by saved PMML).
    let mut ordered = selected;
    ordered.sort_unstable();
    ordered.dedup();

    let fit = fit_model(&prep, &ordered, settings)?;
    build_model(&prep, &ordered, &fit, initial_ll, step_history, settings)
}

fn validate_settings(settings: &Settings) -> Result<(), CoxError> {
    if settings.time_dependent {
        return Err(gap("time-dependent covariates are out of scope"));
    }
    if settings.plots {
        return Err(gap("survival/hazard/LML plots are out of scope"));
    }
    if settings.ties == TieHandling::Efron {
        return Err(gap(
            "Efron tie handling is not specified in the Algorithms Guide or coxregnode",
        ));
    }
    if settings.model_type == ModelType::Custom {
        return Err(gap(
            "custom terms / interactions are not specified in the COXREG Algorithms Guide",
        ));
    }
    if !settings.probability_entry.is_finite()
        || settings.probability_entry <= 0.0
        || settings.probability_entry >= 1.0
    {
        return Err(CoxError::Settings(
            "probability_entry must be finite and in (0, 1)".into(),
        ));
    }
    if !settings.probability_removal.is_finite()
        || settings.probability_removal <= 0.0
        || settings.probability_removal >= 1.0
    {
        return Err(CoxError::Settings(
            "probability_removal must be finite and in (0, 1)".into(),
        ));
    }
    if settings.max_iterations == 0 || settings.max_iterations > MAX_ITERATIONS {
        return Err(CoxError::Settings(format!(
            "max_iterations must be in 1..={MAX_ITERATIONS}"
        )));
    }
    if !settings.parameter_convergence.is_finite() || settings.parameter_convergence < 0.0 {
        return Err(CoxError::Settings(
            "parameter_convergence must be finite and >= 0".into(),
        ));
    }
    if !settings.log_likelihood_convergence.is_finite() || settings.log_likelihood_convergence < 0.0
    {
        return Err(CoxError::Settings(
            "log_likelihood_convergence must be finite and >= 0".into(),
        ));
    }
    Ok(())
}

fn prepare_data(data: &CoxData) -> Result<PreparedData, CoxError> {
    let n_raw = data.time.len();
    if n_raw == 0 {
        return Err(CoxError::Data("no records provided".into()));
    }
    if n_raw > MAX_RECORDS {
        return Err(CoxError::Data(format!(
            "record count {n_raw} exceeds limit {MAX_RECORDS}"
        )));
    }
    if data.event.len() != n_raw {
        return Err(CoxError::Data(
            "event vector length does not match time vector length".into(),
        ));
    }
    if let Some(strata) = &data.strata
        && strata.len() != n_raw
    {
        return Err(CoxError::Data(
            "strata vector length does not match time vector length".into(),
        ));
    }
    if let Some(weights) = &data.weights
        && weights.len() != n_raw
    {
        return Err(CoxError::Data(
            "weights vector length does not match time vector length".into(),
        ));
    }
    if data.covariates.len() > MAX_COVARIATES {
        return Err(CoxError::Data(format!(
            "covariate count {} exceeds limit {MAX_COVARIATES}",
            data.covariates.len()
        )));
    }

    let mut total_params: usize = 0;
    let mut schemas = Vec::with_capacity(data.covariates.len());
    for cov in &data.covariates {
        if cov.len() != n_raw {
            return Err(CoxError::Data(format!(
                "covariate '{}' length {} does not match record count {n_raw}",
                cov.name(),
                cov.len()
            )));
        }
        match cov {
            Covariate::Numeric { name, .. } => {
                total_params = total_params
                    .checked_add(1)
                    .ok_or_else(|| CoxError::Data("parameter count overflow".into()))?;
                schemas.push(CovariateSchema {
                    name: name.clone(),
                    kind: CovariateKind::Numeric,
                });
            }
            Covariate::Factor { name, levels, .. } => {
                if levels.is_empty() {
                    return Err(CoxError::Data(format!("factor '{name}' has no levels")));
                }
                if levels.len() > MAX_CATEGORIES {
                    return Err(CoxError::Data(format!(
                        "factor '{name}' has {} levels, exceeding limit {MAX_CATEGORIES}",
                        levels.len()
                    )));
                }
                let df = levels.len() - 1;
                total_params = total_params
                    .checked_add(df)
                    .ok_or_else(|| CoxError::Data("parameter count overflow".into()))?;
                schemas.push(CovariateSchema {
                    name: name.clone(),
                    kind: CovariateKind::Factor {
                        levels: levels.clone(),
                    },
                });
            }
        }
    }
    if total_params > MAX_PARAMETERS {
        return Err(CoxError::Data(format!(
            "total expanded parameters {total_params} exceeds limit {MAX_PARAMETERS}"
        )));
    }
    if n_raw
        .checked_mul(total_params.max(1))
        .is_none_or(|cells| cells > MAX_DESIGN_CELLS)
    {
        return Err(CoxError::Data(format!(
            "design size {n_raw} x {total_params} exceeds limit {MAX_DESIGN_CELLS}"
        )));
    }

    // Complete-case filtering (Guide p. 106: exclude records with any missing input/output).
    let mut valid_indices: Vec<usize> = Vec::with_capacity(n_raw);
    for row in 0..n_raw {
        let t = data
            .time
            .get(row)
            .copied()
            .ok_or_else(|| CoxError::Data("time index out of bounds".into()))?;
        if !t.is_finite() || t < 0.0 {
            return Err(CoxError::Data(format!(
                "invalid survival time {t} at record {row} (must be finite and >= 0)"
            )));
        }
        if let Some(weights) = &data.weights {
            let w = weights
                .get(row)
                .copied()
                .ok_or_else(|| CoxError::Data("weight index out of bounds".into()))?;
            if !w.is_finite() || w <= 0.0 {
                return Err(CoxError::Data(format!(
                    "invalid case weight {w} at record {row} (must be finite and > 0)"
                )));
            }
        }
        if let Some(strata) = &data.strata
            && strata.get(row).copied().flatten().is_none()
        {
            continue;
        }
        let mut complete = true;
        for cov in &data.covariates {
            match cov {
                Covariate::Numeric { name, values } => match values.get(row).copied().flatten() {
                    Some(v) => {
                        if !v.is_finite() {
                            return Err(CoxError::Data(format!(
                                "non-finite value {v} in numeric covariate '{name}' at record {row}"
                            )));
                        }
                    }
                    None => {
                        complete = false;
                        break;
                    }
                },
                Covariate::Factor {
                    name,
                    levels,
                    codes,
                } => match codes.get(row).copied().flatten() {
                    Some(code) => {
                        let code_usize = usize::try_from(code)
                            .map_err(|_| CoxError::Data("factor code overflow".into()))?;
                        if code_usize >= levels.len() {
                            return Err(CoxError::Data(format!(
                                "factor code {code} out of range for '{name}' at record {row}"
                            )));
                        }
                    }
                    None => {
                        complete = false;
                        break;
                    }
                },
            }
        }
        if complete {
            valid_indices.push(row);
        }
    }

    if valid_indices.is_empty() {
        return Err(CoxError::Data(
            "no complete records remain after blank handling".into(),
        ));
    }

    // Group and sort by (stratum, time ascending, original row index for stability).
    let stratified = data.strata.is_some();
    let mut by_stratum: BTreeMap<Option<u32>, Vec<usize>> = BTreeMap::new();
    for &row in &valid_indices {
        let s = data
            .strata
            .as_ref()
            .and_then(|v| v.get(row).copied().flatten());
        by_stratum.entry(s).or_default().push(row);
    }
    if by_stratum.len() > MAX_STRATA {
        return Err(CoxError::Data(format!(
            "stratum count {} exceeds limit {MAX_STRATA}",
            by_stratum.len()
        )));
    }

    let n = valid_indices.len();
    let mut sorted_rows: Vec<usize> = Vec::with_capacity(n);
    let mut strata_slices: Vec<StratumSlice> = Vec::with_capacity(by_stratum.len());
    let mut total_events: usize = 0;

    for (stratum_key, mut rows) in by_stratum {
        rows.sort_by(|&a, &b| {
            let ta = data.time.get(a).copied().unwrap_or(0.0);
            let tb = data.time.get(b).copied().unwrap_or(0.0);
            ta.total_cmp(&tb).then_with(|| a.cmp(&b))
        });
        let start = sorted_rows.len();
        let mut max_time = 0.0_f64;
        let mut max_event_time: Option<f64> = None;
        for &r in &rows {
            let t = data.time.get(r).copied().unwrap_or(0.0);
            let ev = data.event.get(r).copied().unwrap_or(false);
            if t > max_time {
                max_time = t;
            }
            if ev {
                total_events += 1;
                max_event_time = Some(match max_event_time {
                    Some(prev) => prev.max(t),
                    None => t,
                });
            }
            sorted_rows.push(r);
        }
        let end = sorted_rows.len();
        strata_slices.push(StratumSlice {
            stratum: stratum_key,
            start,
            end,
            max_time,
            max_event_time,
        });
    }

    if total_events == 0 {
        return Err(CoxError::Data(
            "no uncensored events in training data".into(),
        ));
    }

    let mut time = Vec::with_capacity(n);
    let mut event = Vec::with_capacity(n);
    let mut weight = Vec::with_capacity(n);
    let mut total_weight = 0.0;
    for &r in &sorted_rows {
        time.push(data.time.get(r).copied().unwrap_or(0.0));
        event.push(data.event.get(r).copied().unwrap_or(false));
        let w = data
            .weights
            .as_ref()
            .and_then(|ws| ws.get(r).copied())
            .unwrap_or(1.0);
        weight.push(w);
        total_weight += w;
    }

    // Expand covariates into raw and mean-centered columns.
    let mut prepared_covs = Vec::with_capacity(data.covariates.len());
    for (cov_idx, cov) in data.covariates.iter().enumerate() {
        match cov {
            Covariate::Numeric { name, values } => {
                let mut col = Vec::with_capacity(n);
                let mut wsum = 0.0;
                for (pos, &r) in sorted_rows.iter().enumerate() {
                    let v = values.get(r).copied().flatten().unwrap_or(0.0);
                    let w = weight.get(pos).copied().unwrap_or(1.0);
                    wsum += w * v;
                    col.push(v);
                }
                let mean = wsum / total_weight;
                let centered: Vec<f64> = col.iter().map(|&v| v - mean).collect();
                prepared_covs.push(PreparedCovariate {
                    covariate_index: cov_idx,
                    name: name.clone(),
                    kind: CovariateKind::Numeric,
                    param_names: vec![name.clone()],
                    level_indices: vec![None],
                    level_labels: vec![None],
                    raw_cols: vec![col],
                    means: vec![mean],
                    centered_cols: vec![centered],
                });
            }
            Covariate::Factor {
                name,
                levels,
                codes,
            } => {
                let df = levels.len().saturating_sub(1);
                let mut raw_cols = Vec::with_capacity(df);
                let mut means = Vec::with_capacity(df);
                let mut centered_cols = Vec::with_capacity(df);
                let mut param_names = Vec::with_capacity(df);
                let mut level_indices = Vec::with_capacity(df);
                let mut level_labels = Vec::with_capacity(df);
                for lv in 0..df {
                    let mut col = Vec::with_capacity(n);
                    let mut wsum = 0.0;
                    for (pos, &r) in sorted_rows.iter().enumerate() {
                        let code = codes.get(r).copied().flatten().unwrap_or(0) as usize;
                        let v = if code == lv { 1.0 } else { 0.0 };
                        let w = weight.get(pos).copied().unwrap_or(1.0);
                        wsum += w * v;
                        col.push(v);
                    }
                    let mean = wsum / total_weight;
                    let centered: Vec<f64> = col.iter().map(|&v| v - mean).collect();
                    let label = levels.get(lv).cloned().unwrap_or_default();
                    let p_name = if df == 1 {
                        name.clone()
                    } else {
                        format!("{name}={label}")
                    };
                    param_names.push(p_name);
                    level_indices.push(Some(lv));
                    level_labels.push(Some(label));
                    raw_cols.push(col);
                    means.push(mean);
                    centered_cols.push(centered);
                }
                prepared_covs.push(PreparedCovariate {
                    covariate_index: cov_idx,
                    name: name.clone(),
                    kind: CovariateKind::Factor {
                        levels: levels.clone(),
                    },
                    param_names,
                    level_indices,
                    level_labels,
                    raw_cols,
                    means,
                    centered_cols,
                });
            }
        }
    }

    Ok(PreparedData {
        n,
        n_events: total_events,
        n_censored: n - total_events,
        time,
        event,
        weight,
        strata: strata_slices,
        stratified,
        covariates: prepared_covs,
        schemas,
    })
}

// ---------------------------------------------------------------------------------------------
// Partial likelihood, score, and information matrix (Breslow ties, O(N p^2) reverse sweep)

fn null_log_likelihood(prep: &PreparedData) -> f64 {
    let mut ll = 0.0;
    for s in &prep.strata {
        let mut s0 = 0.0;
        let mut idx = s.end;
        while idx > s.start {
            let tj = prep.time.get(idx - 1).copied().unwrap_or(0.0);
            let mut dj = 0.0;
            while idx > s.start && prep.time.get(idx - 1).copied().unwrap_or(0.0) == tj {
                idx -= 1;
                let w = prep.weight.get(idx).copied().unwrap_or(1.0);
                s0 += w;
                if prep.event.get(idx).copied().unwrap_or(false) {
                    dj += w;
                }
            }
            if dj > 0.0 && s0 > 0.0 {
                ll -= dj * s0.ln();
            }
        }
    }
    ll
}

fn collect_centered_columns<'a>(prep: &'a PreparedData, selected: &[usize]) -> Vec<&'a [f64]> {
    let mut cols = Vec::new();
    for &idx in selected {
        if let Some(cov) = prep.covariates.get(idx) {
            for col in &cov.centered_cols {
                cols.push(col.as_slice());
            }
        }
    }
    cols
}

/// Evaluates partial log-likelihood `l(beta)` only (used for step-halving and Conditional test).
fn eval_log_likelihood(
    prep: &PreparedData,
    cols: &[&[f64]],
    beta: &[f64],
) -> Result<f64, CoxError> {
    let p = cols.len();
    if p == 0 {
        return Ok(null_log_likelihood(prep));
    }
    let mut eta = vec![0.0; prep.n];
    for (j, (&col, &bj)) in cols.iter().zip(beta.iter()).enumerate() {
        let _ = j;
        if bj == 0.0 {
            continue;
        }
        for (e, &x) in eta.iter_mut().zip(col.iter()) {
            *e += x * bj;
        }
    }

    let mut ll = 0.0;
    for s in &prep.strata {
        if s.start >= s.end {
            continue;
        }
        let mut eta_max = f64::NEG_INFINITY;
        for i in s.start..s.end {
            let e = eta.get(i).copied().unwrap_or(0.0);
            if !e.is_finite() {
                return Err(CoxError::Data("non-finite linear predictor".into()));
            }
            if e > eta_max {
                eta_max = e;
            }
        }
        let mut s0 = 0.0;
        let mut idx = s.end;
        while idx > s.start {
            let tj = prep.time.get(idx - 1).copied().unwrap_or(0.0);
            let mut dj = 0.0;
            let mut event_eta_sum = 0.0;
            while idx > s.start && prep.time.get(idx - 1).copied().unwrap_or(0.0) == tj {
                idx -= 1;
                let w = prep.weight.get(idx).copied().unwrap_or(1.0);
                let e = eta.get(idx).copied().unwrap_or(0.0);
                s0 += w * (e - eta_max).exp();
                if prep.event.get(idx).copied().unwrap_or(false) {
                    dj += w;
                    event_eta_sum += w * e;
                }
            }
            if dj > 0.0 {
                if s0 <= 0.0 || !s0.is_finite() {
                    return Err(CoxError::Data(
                        "risk set denominator underflowed or overflowed".into(),
                    ));
                }
                ll += event_eta_sum - dj * (s0.ln() + eta_max);
            }
        }
    }
    if !ll.is_finite() {
        return Err(CoxError::Data("non-finite partial log-likelihood".into()));
    }
    Ok(ll)
}

/// Evaluates `(log_likelihood, score_vector, information_matrix_flat)` at `beta`.
fn eval_ll_score_info(
    prep: &PreparedData,
    cols: &[&[f64]],
    beta: &[f64],
) -> Result<(f64, Vec<f64>, Vec<f64>), CoxError> {
    let p = cols.len();
    if p == 0 {
        return Ok((null_log_likelihood(prep), Vec::new(), Vec::new()));
    }
    let mut eta = vec![0.0; prep.n];
    for (&col, &bj) in cols.iter().zip(beta.iter()) {
        if bj == 0.0 {
            continue;
        }
        for (e, &x) in eta.iter_mut().zip(col.iter()) {
            *e += x * bj;
        }
    }

    let mut ll = 0.0;
    let mut grad = vec![0.0; p];
    let mut info = vec![0.0; p * p];
    let mut s1 = vec![0.0; p];
    let mut s2 = vec![0.0; p * p];
    let mut sj = vec![0.0; p];
    let mut mu = vec![0.0; p];
    let mut xi = vec![0.0; p];

    for s in &prep.strata {
        if s.start >= s.end {
            continue;
        }
        let mut eta_max = f64::NEG_INFINITY;
        for i in s.start..s.end {
            let e = eta.get(i).copied().unwrap_or(0.0);
            if !e.is_finite() {
                return Err(CoxError::Data("non-finite linear predictor".into()));
            }
            if e > eta_max {
                eta_max = e;
            }
        }
        let mut s0 = 0.0;
        s1.fill(0.0);
        s2.fill(0.0);
        let mut idx = s.end;
        while idx > s.start {
            let tj = prep.time.get(idx - 1).copied().unwrap_or(0.0);
            let mut dj = 0.0;
            let mut event_eta_sum = 0.0;
            sj.fill(0.0);
            while idx > s.start && prep.time.get(idx - 1).copied().unwrap_or(0.0) == tj {
                idx -= 1;
                let w = prep.weight.get(idx).copied().unwrap_or(1.0);
                let e = eta.get(idx).copied().unwrap_or(0.0);
                let wi = w * (e - eta_max).exp();
                s0 += wi;
                for (r, &col) in cols.iter().enumerate() {
                    let xr = col.get(idx).copied().unwrap_or(0.0);
                    if let Some(slot) = xi.get_mut(r) {
                        *slot = xr;
                    }
                    if let Some(slot) = s1.get_mut(r) {
                        *slot += wi * xr;
                    }
                }
                for r in 0..p {
                    let xr = xi.get(r).copied().unwrap_or(0.0);
                    let w_xr = wi * xr;
                    let row_off = r * p;
                    for c in r..p {
                        let xc = xi.get(c).copied().unwrap_or(0.0);
                        if let Some(slot) = s2.get_mut(row_off + c) {
                            *slot += w_xr * xc;
                        }
                    }
                }
                if prep.event.get(idx).copied().unwrap_or(false) {
                    dj += w;
                    event_eta_sum += w * e;
                    for r in 0..p {
                        let xr = xi.get(r).copied().unwrap_or(0.0);
                        if let Some(slot) = sj.get_mut(r) {
                            *slot += w * xr;
                        }
                    }
                }
            }
            if dj > 0.0 {
                if s0 <= 0.0 || !s0.is_finite() {
                    return Err(CoxError::Data(
                        "risk set denominator underflowed or overflowed".into(),
                    ));
                }
                ll += event_eta_sum - dj * (s0.ln() + eta_max);
                let inv_s0 = 1.0 / s0;
                for r in 0..p {
                    let m = s1.get(r).copied().unwrap_or(0.0) * inv_s0;
                    if let Some(slot) = mu.get_mut(r) {
                        *slot = m;
                    }
                    let sr = sj.get(r).copied().unwrap_or(0.0);
                    if let Some(slot) = grad.get_mut(r) {
                        *slot += sr - dj * m;
                    }
                }
                for r in 0..p {
                    let mr = mu.get(r).copied().unwrap_or(0.0);
                    let row_off = r * p;
                    for c in r..p {
                        let mc = mu.get(c).copied().unwrap_or(0.0);
                        let s2_rc = s2.get(row_off + c).copied().unwrap_or(0.0);
                        let contrib = dj * (s2_rc * inv_s0 - mr * mc);
                        if let Some(slot) = info.get_mut(row_off + c) {
                            *slot += contrib;
                        }
                    }
                }
            }
        }
    }

    // Symmetrize lower triangle of info.
    for r in 0..p {
        for c in (r + 1)..p {
            let val = info.get(r * p + c).copied().unwrap_or(0.0);
            if let Some(slot) = info.get_mut(c * p + r) {
                *slot = val;
            }
        }
    }
    if !ll.is_finite() {
        return Err(CoxError::Data("non-finite partial log-likelihood".into()));
    }
    Ok((ll, grad, info))
}

// ---------------------------------------------------------------------------------------------
// Dense linear algebra (no panics, partial-pivoting Gaussian elimination)

/// Solves `A x = b` for an `n x n` matrix `A` (row-major flat). Returns `None` if singular.
fn solve_linear(a: &[f64], b: &[f64], n: usize) -> Option<Vec<f64>> {
    if n == 0 {
        return Some(Vec::new());
    }
    if a.len() != n * n || b.len() != n {
        return None;
    }
    let mut mat = a.to_vec();
    let mut rhs = b.to_vec();

    // Scale reference from max diagonal/entry magnitude.
    let max_entry = mat.iter().fold(0.0_f64, |acc, &v| acc.max(v.abs()));
    if !max_entry.is_finite() || max_entry == 0.0 {
        return None;
    }
    let tol = SINGULARITY_TOL * max_entry;

    for k in 0..n {
        let mut pivot_row = k;
        let mut pivot_val = mat.get(k * n + k).copied().unwrap_or(0.0).abs();
        for i in (k + 1)..n {
            let v = mat.get(i * n + k).copied().unwrap_or(0.0).abs();
            if v > pivot_val {
                pivot_val = v;
                pivot_row = i;
            }
        }
        if !pivot_val.is_finite() || pivot_val <= tol {
            return None;
        }
        if pivot_row != k {
            for j in k..n {
                let a_kj = mat.get(k * n + j).copied().unwrap_or(0.0);
                let a_pj = mat.get(pivot_row * n + j).copied().unwrap_or(0.0);
                if let Some(slot) = mat.get_mut(k * n + j) {
                    *slot = a_pj;
                }
                if let Some(slot) = mat.get_mut(pivot_row * n + j) {
                    *slot = a_kj;
                }
            }
            rhs.swap(k, pivot_row);
        }
        let diag = mat.get(k * n + k).copied().unwrap_or(0.0);
        let rhs_k = rhs.get(k).copied().unwrap_or(0.0);
        for i in (k + 1)..n {
            let factor = mat.get(i * n + k).copied().unwrap_or(0.0) / diag;
            if factor == 0.0 {
                continue;
            }
            if let Some(slot) = mat.get_mut(i * n + k) {
                *slot = 0.0;
            }
            for j in (k + 1)..n {
                let a_kj = mat.get(k * n + j).copied().unwrap_or(0.0);
                if let Some(slot) = mat.get_mut(i * n + j) {
                    *slot -= factor * a_kj;
                }
            }
            if let Some(slot) = rhs.get_mut(i) {
                *slot -= factor * rhs_k;
            }
        }
    }

    let mut x = vec![0.0; n];
    for k in (0..n).rev() {
        let mut sum = rhs.get(k).copied().unwrap_or(0.0);
        for j in (k + 1)..n {
            let a_kj = mat.get(k * n + j).copied().unwrap_or(0.0);
            let x_j = x.get(j).copied().unwrap_or(0.0);
            sum -= a_kj * x_j;
        }
        let diag = mat.get(k * n + k).copied().unwrap_or(0.0);
        let xk = sum / diag;
        if !xk.is_finite() {
            return None;
        }
        if let Some(slot) = x.get_mut(k) {
            *slot = xk;
        }
    }
    Some(x)
}

/// Computes `A^{-1}` for an `n x n` matrix `A` via Gauss-Jordan with partial pivoting.
fn invert_matrix(a: &[f64], n: usize) -> Option<Vec<f64>> {
    if n == 0 {
        return Some(Vec::new());
    }
    if a.len() != n * n {
        return None;
    }
    let mut mat = a.to_vec();
    let mut inv = vec![0.0; n * n];
    for i in 0..n {
        if let Some(slot) = inv.get_mut(i * n + i) {
            *slot = 1.0;
        }
    }
    let max_entry = mat.iter().fold(0.0_f64, |acc, &v| acc.max(v.abs()));
    if !max_entry.is_finite() || max_entry == 0.0 {
        return None;
    }
    let tol = SINGULARITY_TOL * max_entry;

    for k in 0..n {
        let mut pivot_row = k;
        let mut pivot_val = mat.get(k * n + k).copied().unwrap_or(0.0).abs();
        for i in (k + 1)..n {
            let v = mat.get(i * n + k).copied().unwrap_or(0.0).abs();
            if v > pivot_val {
                pivot_val = v;
                pivot_row = i;
            }
        }
        if !pivot_val.is_finite() || pivot_val <= tol {
            return None;
        }
        if pivot_row != k {
            for j in 0..n {
                let a_kj = mat.get(k * n + j).copied().unwrap_or(0.0);
                let a_pj = mat.get(pivot_row * n + j).copied().unwrap_or(0.0);
                if let Some(slot) = mat.get_mut(k * n + j) {
                    *slot = a_pj;
                }
                if let Some(slot) = mat.get_mut(pivot_row * n + j) {
                    *slot = a_kj;
                }
                let b_kj = inv.get(k * n + j).copied().unwrap_or(0.0);
                let b_pj = inv.get(pivot_row * n + j).copied().unwrap_or(0.0);
                if let Some(slot) = inv.get_mut(k * n + j) {
                    *slot = b_pj;
                }
                if let Some(slot) = inv.get_mut(pivot_row * n + j) {
                    *slot = b_kj;
                }
            }
        }
        let diag = mat.get(k * n + k).copied().unwrap_or(0.0);
        let inv_diag = 1.0 / diag;
        for j in 0..n {
            if let Some(slot) = mat.get_mut(k * n + j) {
                *slot *= inv_diag;
            }
            if let Some(slot) = inv.get_mut(k * n + j) {
                *slot *= inv_diag;
            }
        }
        for i in 0..n {
            if i == k {
                continue;
            }
            let factor = mat.get(i * n + k).copied().unwrap_or(0.0);
            if factor == 0.0 {
                continue;
            }
            for j in 0..n {
                let mkj = mat.get(k * n + j).copied().unwrap_or(0.0);
                let ikj = inv.get(k * n + j).copied().unwrap_or(0.0);
                if let Some(slot) = mat.get_mut(i * n + j) {
                    *slot -= factor * mkj;
                }
                if let Some(slot) = inv.get_mut(i * n + j) {
                    *slot -= factor * ikj;
                }
            }
        }
    }
    if inv.iter().any(|v| !v.is_finite()) {
        return None;
    }
    Some(inv)
}

// ---------------------------------------------------------------------------------------------
// Newton-Raphson solver

fn fit_model(
    prep: &PreparedData,
    selected: &[usize],
    settings: &Settings,
) -> Result<FitResult, CoxError> {
    let cols = collect_centered_columns(prep, selected);
    let p = cols.len();
    if p == 0 {
        return Ok(FitResult {
            beta: Vec::new(),
            log_likelihood: null_log_likelihood(prep),
            cov: Vec::new(),
            iterations: 0,
        });
    }

    let mut beta = vec![0.0; p];
    let (mut ll, mut grad, mut info) = eval_ll_score_info(prep, &cols, &beta)?;
    let mut converged = false;
    let mut iterations = 0;

    let p_tol =
        if settings.parameter_convergence == 0.0 && settings.log_likelihood_convergence == 0.0 {
            1e-12
        } else {
            settings.parameter_convergence
        };
    let l_tol =
        if settings.parameter_convergence == 0.0 && settings.log_likelihood_convergence == 0.0 {
            1e-12
        } else {
            settings.log_likelihood_convergence
        };

    for it in 1..=settings.max_iterations {
        crate::check_cancelled()?;
        iterations = it;

        let delta = solve_linear(&info, &grad, p).ok_or_else(|| {
            CoxError::Data(
                "information matrix is singular (constant, collinear, or separated covariates)"
                    .into(),
            )
        })?;

        // Backtracking step-halving if full step decreases log-likelihood or overflows.
        let mut step = 1.0;
        let mut accepted = None;
        for _ in 0..=MAX_STEP_HALVING {
            let candidate: Vec<f64> = beta
                .iter()
                .zip(delta.iter())
                .map(|(&b, &d)| b + step * d)
                .collect();
            if candidate
                .iter()
                .all(|&v| v.is_finite() && v.abs() <= MAX_BETA_ABS)
                && let Ok(ll_cand) = eval_log_likelihood(prep, &cols, &candidate)
                && ll_cand >= ll - 1e-12 * (1.0 + ll.abs())
            {
                accepted = Some((candidate, ll_cand));
                break;
            }
            step *= 0.5;
        }

        let (beta_new, ll_new) = accepted.ok_or_else(|| {
            CoxError::Data(
                "Newton-Raphson failed to find an improving finite step (possible separation)"
                    .into(),
            )
        })?;

        // Guide p. 99 relative convergence criteria (AND when both enabled).
        let mut bcon = 0.0_f64;
        for (&b_old, &b_new) in beta.iter().zip(beta_new.iter()) {
            let diff = (b_new - b_old).abs();
            let rel = if b_old.abs() > 1e-12 {
                diff / b_old.abs()
            } else {
                diff
            };
            if rel > bcon {
                bcon = rel;
            }
        }
        let lcon = if ll.abs() > 1e-12 {
            (ll_new - ll).abs() / ll.abs()
        } else {
            (ll_new - ll).abs()
        };

        let (ll_eval, grad_new, info_new) = eval_ll_score_info(prep, &cols, &beta_new)?;
        beta = beta_new;
        ll = ll_eval;
        grad = grad_new;
        info = info_new;

        let p_ok = p_tol == 0.0 || bcon < p_tol;
        let l_ok = l_tol == 0.0 || lcon < l_tol;
        if p_ok && l_ok {
            converged = true;
            break;
        }
    }

    if !converged {
        return Err(CoxError::Data(format!(
            "Newton-Raphson did not converge within {} iterations (possible separation)",
            settings.max_iterations
        )));
    }

    let cov = invert_matrix(&info, p)
        .ok_or_else(|| CoxError::Data("information matrix is singular at final estimate".into()))?;

    Ok(FitResult {
        beta,
        log_likelihood: ll,
        cov,
        iterations,
    })
}

// ---------------------------------------------------------------------------------------------
// Stepwise selection statistics & drivers (Guide pp. 101-104)

/// Computes Score entry statistic `(stat, df, p_value)` for adding candidate `cand_idx` to
/// `in_model` (evaluated at `[beta_in, 0]`, Guide p. 102). Returns `None` if collinear/singular.
fn score_entry_stat(
    prep: &PreparedData,
    in_model: &[usize],
    fit: &FitResult,
    cand_idx: usize,
) -> Result<Option<(f64, usize, f64)>, CoxError> {
    let Some(cand_cov) = prep.covariates.get(cand_idx) else {
        return Ok(None);
    };
    let k = cand_cov.centered_cols.len();
    if k == 0 {
        return Ok(None);
    }

    let mut cols = collect_centered_columns(prep, in_model);
    let p_in = cols.len();
    for col in &cand_cov.centered_cols {
        cols.push(col.as_slice());
    }
    let p_tot = p_in + k;
    let mut beta_all = vec![0.0; p_tot];
    for (dst, &src) in beta_all.iter_mut().zip(fit.beta.iter()) {
        *dst = src;
    }

    let (_, grad_all, info_all) = eval_ll_score_info(prep, &cols, &beta_all)?;

    // D_{x_i}: slice [p_in .. p_tot]
    let d_x: Vec<f64> = grad_all.get(p_in..p_tot).unwrap_or_default().to_vec();

    // Schur complement: M = A_{22,i} - A_{21,i} A_{11}^{-1} A_{12,i}
    let mut schur = vec![0.0; k * k];
    for r in 0..k {
        for c in 0..k {
            let a22 = info_all
                .get((p_in + r) * p_tot + (p_in + c))
                .copied()
                .unwrap_or(0.0);
            let mut quad = 0.0;
            if p_in > 0 {
                // Compute (A_{21} * A_{11}^{-1})[r, m] * A_{12}[m, c]
                for a in 0..p_in {
                    let a21_ra = info_all.get((p_in + r) * p_tot + a).copied().unwrap_or(0.0);
                    let mut tmp = 0.0;
                    for b in 0..p_in {
                        let cov_ab = fit.cov.get(a * p_in + b).copied().unwrap_or(0.0);
                        let a12_bc = info_all.get(b * p_tot + (p_in + c)).copied().unwrap_or(0.0);
                        tmp += cov_ab * a12_bc;
                    }
                    quad += a21_ra * tmp;
                }
            }
            if let Some(slot) = schur.get_mut(r * k + c) {
                *slot = a22 - quad;
            }
        }
    }

    let Some(sol) = solve_linear(&schur, &d_x, k) else {
        return Ok(None);
    };
    let stat: f64 = d_x.iter().zip(sol.iter()).map(|(&a, &b)| a * b).sum();
    if !stat.is_finite() {
        return Ok(None);
    }
    let stat = stat.max(0.0);
    let p_val = chi_square_p(stat, k as f64);
    Ok(Some((stat, k, p_val)))
}

/// Computes the removal test statistic `(stat, df, p_value)` for removing `in_model[pos]`
/// according to `settings.removal_criterion`.
fn removal_stat(
    prep: &PreparedData,
    in_model: &[usize],
    fit: &FitResult,
    pos: usize,
    settings: &Settings,
) -> Result<(f64, usize, f64), CoxError> {
    // Locate parameter range [start .. start + k) for in_model[pos].
    let mut start = 0usize;
    let mut k = 0usize;
    for (idx_pos, &cov_idx) in in_model.iter().enumerate() {
        let df = prep
            .covariates
            .get(cov_idx)
            .map(|c| c.centered_cols.len())
            .unwrap_or(0);
        if idx_pos == pos {
            k = df;
            break;
        }
        start += df;
    }
    let p_full = fit.beta.len();

    let stat = match settings.removal_criterion {
        RemovalCriterion::Lr => {
            let reduced: Vec<usize> = in_model
                .iter()
                .enumerate()
                .filter(|&(i, _)| i != pos)
                .map(|(_, &v)| v)
                .collect();
            let red_fit = fit_model(prep, &reduced, settings)?;
            (-2.0 * (red_fit.log_likelihood - fit.log_likelihood)).max(0.0)
        }
        RemovalCriterion::Wald => {
            let beta_j: Vec<f64> = fit
                .beta
                .get(start..(start + k))
                .unwrap_or_default()
                .to_vec();
            let mut c_jj = vec![0.0; k * k];
            for r in 0..k {
                for c in 0..k {
                    let val = fit
                        .cov
                        .get((start + r) * p_full + (start + c))
                        .copied()
                        .unwrap_or(0.0);
                    if let Some(slot) = c_jj.get_mut(r * k + c) {
                        *slot = val;
                    }
                }
            }
            match solve_linear(&c_jj, &beta_j, k) {
                Some(sol) => beta_j
                    .iter()
                    .zip(sol.iter())
                    .map(|(&a, &b)| a * b)
                    .sum::<f64>()
                    .max(0.0),
                None => 0.0,
            }
        }
        RemovalCriterion::Conditional => {
            // Guide pp. 102-103:
            // beta_tilde_(i) = beta_hat_(i) - C_{12} C_{22}^{-1} beta_hat_i
            // Cond = -2 * (l(beta_tilde_(i)) - l(full))
            let reduced: Vec<usize> = in_model
                .iter()
                .enumerate()
                .filter(|&(i, _)| i != pos)
                .map(|(_, &v)| v)
                .collect();
            let p_rem = p_full.saturating_sub(k);
            if p_rem == 0 {
                (-2.0 * (null_log_likelihood(prep) - fit.log_likelihood)).max(0.0)
            } else {
                let beta_j: Vec<f64> = fit
                    .beta
                    .get(start..(start + k))
                    .unwrap_or_default()
                    .to_vec();
                let mut c_22 = vec![0.0; k * k];
                for r in 0..k {
                    for c in 0..k {
                        let val = fit
                            .cov
                            .get((start + r) * p_full + (start + c))
                            .copied()
                            .unwrap_or(0.0);
                        if let Some(slot) = c_22.get_mut(r * k + c) {
                            *slot = val;
                        }
                    }
                }
                let inv_beta = solve_linear(&c_22, &beta_j, k).unwrap_or_else(|| vec![0.0; k]);
                let mut beta_cond = Vec::with_capacity(p_rem);
                for idx_r in 0..p_full {
                    if idx_r >= start && idx_r < start + k {
                        continue;
                    }
                    let mut adj = 0.0;
                    for c in 0..k {
                        let c12 = fit
                            .cov
                            .get(idx_r * p_full + (start + c))
                            .copied()
                            .unwrap_or(0.0);
                        let v = inv_beta.get(c).copied().unwrap_or(0.0);
                        adj += c12 * v;
                    }
                    let b_r = fit.beta.get(idx_r).copied().unwrap_or(0.0);
                    beta_cond.push(b_r - adj);
                }
                let red_cols = collect_centered_columns(prep, &reduced);
                let ll_cond = eval_log_likelihood(prep, &red_cols, &beta_cond)?;
                (-2.0 * (ll_cond - fit.log_likelihood)).max(0.0)
            }
        }
    };

    let p_val = chi_square_p(stat, k as f64);
    Ok((stat, k, p_val))
}

fn canonical_key(indices: &[usize]) -> Vec<usize> {
    let mut k = indices.to_vec();
    k.sort_unstable();
    k
}

fn find_best_entry(
    prep: &PreparedData,
    in_model: &[usize],
    fit: &FitResult,
    forbidden: Option<usize>,
) -> Result<Option<(usize, f64, usize, f64)>, CoxError> {
    let mut best: Option<(usize, f64, usize, f64)> = None;
    for cov_idx in 0..prep.covariates.len() {
        if in_model.contains(&cov_idx) || forbidden == Some(cov_idx) {
            continue;
        }
        let Some((stat, df, p_val)) = score_entry_stat(prep, in_model, fit, cov_idx)? else {
            continue;
        };
        let better = match best {
            None => true,
            Some((_, best_stat, _, best_p)) => {
                p_val < best_p || (p_val == best_p && stat > best_stat)
            }
        };
        if better {
            best = Some((cov_idx, stat, df, p_val));
        }
    }
    Ok(best)
}

struct RemovalCandidate {
    pos: usize,
    cov_idx: usize,
    stat: f64,
    df: usize,
    p_val: f64,
}

fn find_worst_removal(
    prep: &PreparedData,
    in_model: &[usize],
    fit: &FitResult,
    forbidden_cov: Option<usize>,
    settings: &Settings,
) -> Result<Option<RemovalCandidate>, CoxError> {
    let mut worst: Option<RemovalCandidate> = None;
    for (pos, &cov_idx) in in_model.iter().enumerate() {
        if forbidden_cov == Some(cov_idx) {
            continue;
        }
        let (stat, df, p_val) = removal_stat(prep, in_model, fit, pos, settings)?;
        let worse = match &worst {
            None => true,
            Some(w) => p_val > w.p_val || (p_val == w.p_val && stat < w.stat),
        };
        if worse {
            worst = Some(RemovalCandidate {
                pos,
                cov_idx,
                stat,
                df,
                p_val,
            });
        }
    }
    Ok(worst)
}

fn run_forward_stepwise(
    prep: &PreparedData,
    settings: &Settings,
    initial_ll: f64,
) -> Result<(Vec<usize>, Vec<StepRecord>), CoxError> {
    let mut in_model: Vec<usize> = Vec::new();
    let mut fit = fit_model(prep, &in_model, settings)?;
    let mut visited: HashSet<Vec<usize>> = HashSet::new();
    visited.insert(Vec::new());
    let mut history: Vec<StepRecord> = Vec::new();
    let mut step = 0usize;

    while step < MAX_STEPWISE_STEPS {
        crate::check_cancelled()?;
        let Some((enter_idx, stat, df, p_val)) = find_best_entry(prep, &in_model, &fit, None)?
        else {
            break;
        };
        if p_val >= settings.probability_entry {
            break;
        }
        let mut next_model = in_model.clone();
        next_model.push(enter_idx);
        let key = canonical_key(&next_model);
        if visited.contains(&key) {
            break;
        }
        let prev_ll = fit.log_likelihood;
        let next_fit = fit_model(prep, &next_model, settings)?;
        step += 1;
        let cov_name = prep
            .covariates
            .get(enter_idx)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        history.push(StepRecord {
            step,
            action: StepAction::Enter,
            covariate_index: enter_idx,
            covariate_name: cov_name,
            statistic: stat,
            df,
            p_value: p_val,
            log_likelihood: next_fit.log_likelihood,
            model_chi_square: 2.0 * (next_fit.log_likelihood - initial_ll),
            improvement_chi_square: 2.0 * (next_fit.log_likelihood - prev_ll),
        });
        visited.insert(key);
        in_model = next_model;
        fit = next_fit;

        // Backward removal check (never remove the variable just entered).
        while step < MAX_STEPWISE_STEPS {
            let Some(rem) = find_worst_removal(prep, &in_model, &fit, Some(enter_idx), settings)?
            else {
                break;
            };
            if rem.p_val <= settings.probability_removal {
                break;
            }
            let mut reduced = in_model.clone();
            reduced.remove(rem.pos);
            let rem_key = canonical_key(&reduced);
            if visited.contains(&rem_key) {
                break;
            }
            let prev_ll = fit.log_likelihood;
            let red_fit = fit_model(prep, &reduced, settings)?;
            step += 1;
            let cov_name = prep
                .covariates
                .get(rem.cov_idx)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            history.push(StepRecord {
                step,
                action: StepAction::Remove,
                covariate_index: rem.cov_idx,
                covariate_name: cov_name,
                statistic: rem.stat,
                df: rem.df,
                p_value: rem.p_val,
                log_likelihood: red_fit.log_likelihood,
                model_chi_square: 2.0 * (red_fit.log_likelihood - initial_ll),
                improvement_chi_square: 2.0 * (red_fit.log_likelihood - prev_ll),
            });
            visited.insert(rem_key);
            in_model = reduced;
            fit = red_fit;
        }
    }

    Ok((in_model, history))
}

fn run_backward_stepwise(
    prep: &PreparedData,
    settings: &Settings,
    initial_ll: f64,
) -> Result<(Vec<usize>, Vec<StepRecord>), CoxError> {
    let mut in_model: Vec<usize> = prep
        .covariates
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.raw_cols.is_empty())
        .map(|(i, _)| i)
        .collect();
    let mut fit = fit_model(prep, &in_model, settings)?;
    let mut visited: HashSet<Vec<usize>> = HashSet::new();
    visited.insert(canonical_key(&in_model));
    let mut history: Vec<StepRecord> = Vec::new();
    let mut step = 0usize;

    while step < MAX_STEPWISE_STEPS && !in_model.is_empty() {
        crate::check_cancelled()?;
        let Some(rem) = find_worst_removal(prep, &in_model, &fit, None, settings)? else {
            break;
        };
        if rem.p_val <= settings.probability_removal {
            break;
        }
        let mut reduced = in_model.clone();
        reduced.remove(rem.pos);
        let rem_key = canonical_key(&reduced);
        if visited.contains(&rem_key) {
            break;
        }
        let prev_ll = fit.log_likelihood;
        let red_fit = fit_model(prep, &reduced, settings)?;
        step += 1;
        let cov_name = prep
            .covariates
            .get(rem.cov_idx)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        history.push(StepRecord {
            step,
            action: StepAction::Remove,
            covariate_index: rem.cov_idx,
            covariate_name: cov_name,
            statistic: rem.stat,
            df: rem.df,
            p_value: rem.p_val,
            log_likelihood: red_fit.log_likelihood,
            model_chi_square: 2.0 * (red_fit.log_likelihood - initial_ll),
            improvement_chi_square: 2.0 * (red_fit.log_likelihood - prev_ll),
        });
        visited.insert(rem_key);
        in_model = reduced;
        fit = red_fit;

        // Check if any previously excluded covariate can re-enter (excluding the one just removed).
        if step < MAX_STEPWISE_STEPS
            && let Some((enter_idx, enter_stat, enter_df, enter_p)) =
                find_best_entry(prep, &in_model, &fit, Some(rem.cov_idx))?
            && enter_p < settings.probability_entry
        {
            let mut next_model = in_model.clone();
            next_model.push(enter_idx);
            let next_key = canonical_key(&next_model);
            if !visited.contains(&next_key) {
                let prev_ll = fit.log_likelihood;
                let next_fit = fit_model(prep, &next_model, settings)?;
                step += 1;
                let cov_name = prep
                    .covariates
                    .get(enter_idx)
                    .map(|c| c.name.clone())
                    .unwrap_or_default();
                history.push(StepRecord {
                    step,
                    action: StepAction::Enter,
                    covariate_index: enter_idx,
                    covariate_name: cov_name,
                    statistic: enter_stat,
                    df: enter_df,
                    p_value: enter_p,
                    log_likelihood: next_fit.log_likelihood,
                    model_chi_square: 2.0 * (next_fit.log_likelihood - initial_ll),
                    improvement_chi_square: 2.0 * (next_fit.log_likelihood - prev_ll),
                });
                visited.insert(next_key);
                in_model = next_model;
                fit = next_fit;
            }
        }
    }

    Ok((in_model, history))
}

// ---------------------------------------------------------------------------------------------
// Baseline cumulative hazard & model assembly (Guide pp. 100-101, 105)

/// Solves Kalbfleisch-Prentice tie equation:
/// `sum_{l in D_i} (w_l * r_l) / (1 - exp(-z * r_l)) = s0` for `z = -ln(alpha_i) > 0`.
fn solve_kp_jump(events: &[(f64, f64)], s0: f64) -> f64 {
    let wd_sum: f64 = events.iter().map(|&(w, r)| w * r).sum();
    if s0 - wd_sum <= 1e-15 * s0 {
        return f64::INFINITY;
    }
    if let [(w, r)] = events {
        let ratio = (w * r) / s0;
        if ratio >= 1.0 {
            return f64::INFINITY;
        }
        return -(-ratio).ln_1p() / *r;
    }

    let d_sum: f64 = events.iter().map(|&(w, _)| w).sum();
    // Breslow initial value z0 = d_i / S_0 is strictly below the unique root z*.
    let mut z = (d_sum / s0).max(1e-15);
    let mut lo = z;
    let mut hi = f64::INFINITY;

    for _ in 0..MAX_KP_ITERATIONS {
        let mut f = -s0;
        let mut df = 0.0;
        for &(w, r) in events {
            let zr = z * r;
            // 1 - exp(-zr) = -(-zr).exp_m1()
            let denom = if zr > 700.0 { 1.0 } else { -(-zr).exp_m1() };
            if denom <= 0.0 {
                f = f64::INFINITY;
                break;
            }
            let ez = if zr > 700.0 { 0.0 } else { (-zr).exp() };
            f += (w * r) / denom;
            df -= (w * r * r * ez) / (denom * denom);
        }
        if f > 0.0 {
            lo = lo.max(z);
        } else {
            hi = hi.min(z);
        }
        if f.abs() <= 1e-14 * s0 {
            break;
        }
        let z_next = if df.is_finite() && df < 0.0 {
            let cand = z - f / df;
            if cand > lo && cand < hi {
                cand
            } else if hi.is_finite() {
                0.5 * (lo + hi)
            } else {
                z * 2.0
            }
        } else if hi.is_finite() {
            0.5 * (lo + hi)
        } else {
            z * 2.0
        };
        if (z_next - z).abs() <= 1e-14 * z.max(1.0) {
            z = z_next;
            break;
        }
        z = z_next;
    }
    z
}

fn compute_baselines(
    prep: &PreparedData,
    cols_centered: &[&[f64]],
    beta: &[f64],
    cov: &[f64],
    means: &[f64],
    ref_point: &[f64],
    method: BaselineMethod,
) -> Result<Vec<StratumBaseline>, CoxError> {
    let p = cols_centered.len();
    let mut eta_c = vec![0.0; prep.n];
    for (&col, &bj) in cols_centered.iter().zip(beta.iter()) {
        if bj == 0.0 {
            continue;
        }
        for (e, &x) in eta_c.iter_mut().zip(col.iter()) {
            *e += x * bj;
        }
    }

    // Shift from centered coordinates (x - x_bar) to reference point (x - ref_point):
    // H_0(t; ref_point) = H_0(t; x_bar) * exp((ref_point - x_bar) . beta).
    let mut ref_shift = 0.0;
    for j in 0..p {
        let rj = ref_point.get(j).copied().unwrap_or(0.0);
        let mj = means.get(j).copied().unwrap_or(0.0);
        let bj = beta.get(j).copied().unwrap_or(0.0);
        ref_shift += (rj - mj) * bj;
    }

    let mut baselines = Vec::with_capacity(prep.strata.len());

    for s in &prep.strata {
        if s.start >= s.end || s.max_event_time.is_none() {
            baselines.push(StratumBaseline {
                stratum: s.stratum,
                max_time: s.max_time,
                max_event_time: s.max_event_time,
                points: Vec::new(),
            });
            continue;
        }

        // Shift per-stratum linear predictors by eta_max to avoid exp() overflow.
        let mut eta_max = f64::NEG_INFINITY;
        for i in s.start..s.end {
            let e = eta_c.get(i).copied().unwrap_or(0.0);
            if e > eta_max {
                eta_max = e;
            }
        }
        let stratum_scale = (ref_shift - eta_max).exp();
        if !stratum_scale.is_finite() {
            return Err(CoxError::Data(
                "baseline hazard reference-point scaling overflowed".into(),
            ));
        }

        // First collect distinct event-time risk-set summaries from high time to low time.
        struct EventStep {
            time: f64,
            dh_shifted: f64,
            events_w: f64,
            at_risk_w: f64,
            var_base_term: f64,
            a_term: Vec<f64>,
            breslow_dh: f64,
        }

        let mut rev_steps: Vec<EventStep> = Vec::new();
        let mut s0_exp = 0.0;
        let mut s1_exp = vec![0.0; p];
        let mut at_risk_w = 0.0;
        let mut idx = s.end;

        while idx > s.start {
            let tj = prep.time.get(idx - 1).copied().unwrap_or(0.0);
            let mut events_list: Vec<(f64, f64)> = Vec::new();
            let mut dj = 0.0;
            while idx > s.start && prep.time.get(idx - 1).copied().unwrap_or(0.0) == tj {
                idx -= 1;
                let w = prep.weight.get(idx).copied().unwrap_or(1.0);
                let r = (eta_c.get(idx).copied().unwrap_or(0.0) - eta_max).exp();
                let wr = w * r;
                s0_exp += wr;
                at_risk_w += w;
                for (j, &col) in cols_centered.iter().enumerate() {
                    let xj = col.get(idx).copied().unwrap_or(0.0);
                    if let Some(slot) = s1_exp.get_mut(j) {
                        *slot += wr * xj;
                    }
                }
                if prep.event.get(idx).copied().unwrap_or(false) {
                    dj += w;
                    events_list.push((w, r));
                }
            }
            if !events_list.is_empty() {
                let breslow_dh = dj / s0_exp;
                let dh_shifted = match method {
                    BaselineMethod::KalbfleischPrentice => solve_kp_jump(&events_list, s0_exp),
                    BaselineMethod::Breslow => breslow_dh,
                };
                let inv_s0_sq = 1.0 / (s0_exp * s0_exp);
                let var_base_term = dj * inv_s0_sq;
                let a_term: Vec<f64> = (0..p)
                    .map(|j| dj * s1_exp.get(j).copied().unwrap_or(0.0) * inv_s0_sq)
                    .collect();
                rev_steps.push(EventStep {
                    time: tj,
                    dh_shifted,
                    events_w: dj,
                    at_risk_w,
                    var_base_term,
                    a_term,
                    breslow_dh,
                });
            }
        }

        rev_steps.reverse();
        let mut points = Vec::with_capacity(rev_steps.len());
        let mut cum_h_shifted = 0.0;
        let mut cum_breslow = 0.0;
        let mut cum_var_base = 0.0;
        let mut cum_a = vec![0.0; p];

        for st in rev_steps {
            cum_h_shifted += st.dh_shifted;
            cum_breslow += st.breslow_dh;
            cum_var_base += st.var_base_term;
            for j in 0..p {
                if let Some(slot) = cum_a.get_mut(j) {
                    *slot += st.a_term.get(j).copied().unwrap_or(0.0);
                }
            }
            let cum_hazard = if cum_h_shifted.is_infinite() {
                f64::INFINITY
            } else {
                cum_h_shifted * stratum_scale
            };
            let survival = if cum_hazard.is_infinite() {
                0.0
            } else {
                (-cum_hazard).exp().clamp(0.0, 1.0)
            };

            // Asymptotic variance of H_0(t; ref_point) (Guide pp. 101, 105).
            let mut a_ref = vec![0.0; p];
            for j in 0..p {
                let rj = ref_point.get(j).copied().unwrap_or(0.0);
                let mj = means.get(j).copied().unwrap_or(0.0);
                let aj = cum_a.get(j).copied().unwrap_or(0.0);
                if let Some(slot) = a_ref.get_mut(j) {
                    *slot = aj - cum_breslow * (rj - mj);
                }
            }
            let mut quad = 0.0;
            for r in 0..p {
                let ar = a_ref.get(r).copied().unwrap_or(0.0);
                for c in 0..p {
                    let ac = a_ref.get(c).copied().unwrap_or(0.0);
                    let cov_rc = cov.get(r * p + c).copied().unwrap_or(0.0);
                    quad += ar * cov_rc * ac;
                }
            }
            let var_h0 = if cum_hazard.is_infinite() {
                f64::INFINITY
            } else {
                ((cum_var_base + quad) * stratum_scale * stratum_scale).max(0.0)
            };

            points.push(BaselinePoint {
                time: st.time,
                cum_hazard,
                survival,
                std_error_cum_hazard: var_h0.sqrt(),
                events: st.events_w,
                at_risk: st.at_risk_w,
            });
        }

        baselines.push(StratumBaseline {
            stratum: s.stratum,
            max_time: s.max_time,
            max_event_time: s.max_event_time,
            points,
        });
    }

    Ok(baselines)
}

fn build_model(
    prep: &PreparedData,
    selected: &[usize],
    fit: &FitResult,
    initial_ll: f64,
    step_history: Vec<StepRecord>,
    settings: &Settings,
) -> Result<CoxModel, CoxError> {
    let p = fit.beta.len();
    let cols_centered = collect_centered_columns(prep, selected);

    let mut terms = Vec::with_capacity(selected.len());
    let mut parameters = Vec::with_capacity(p);
    let mut means = Vec::with_capacity(p);

    let mut offset = 0usize;
    for &cov_idx in selected {
        let Some(cov) = prep.covariates.get(cov_idx) else {
            continue;
        };
        let df = cov.centered_cols.len();
        if df == 0 {
            continue;
        }
        // Term-level Wald statistic.
        let beta_block: Vec<f64> = fit
            .beta
            .get(offset..(offset + df))
            .unwrap_or_default()
            .to_vec();
        let mut c_block = vec![0.0; df * df];
        for r in 0..df {
            for c in 0..df {
                let v = fit
                    .cov
                    .get((offset + r) * p + (offset + c))
                    .copied()
                    .unwrap_or(0.0);
                if let Some(slot) = c_block.get_mut(r * df + c) {
                    *slot = v;
                }
            }
        }
        let term_wald = match solve_linear(&c_block, &beta_block, df) {
            Some(sol) => beta_block
                .iter()
                .zip(sol.iter())
                .map(|(&a, &b)| a * b)
                .sum::<f64>()
                .max(0.0),
            None => 0.0,
        };
        let term_p = chi_square_p(term_wald, df as f64);

        terms.push(EnteredTerm {
            covariate_index: cov.covariate_index,
            name: cov.name.clone(),
            kind: cov.kind.clone(),
            param_start: offset,
            df,
            wald: term_wald,
            p_value: term_p,
        });

        for local in 0..df {
            let j = offset + local;
            let b = fit.beta.get(j).copied().unwrap_or(0.0);
            let var = fit.cov.get(j * p + j).copied().unwrap_or(0.0).max(0.0);
            let se = var.sqrt();
            let wald = if var > 0.0 { (b * b) / var } else { 0.0 };
            let p_val = chi_square_p(wald, 1.0);
            parameters.push(ParameterEstimate {
                name: cov.param_names.get(local).cloned().unwrap_or_default(),
                covariate_index: cov.covariate_index,
                covariate_name: cov.name.clone(),
                level_index: cov.level_indices.get(local).copied().flatten(),
                level_label: cov.level_labels.get(local).cloned().flatten(),
                beta: b,
                std_error: se,
                wald,
                df: 1,
                p_value: p_val,
                exp_beta: b.exp(),
            });
            means.push(cov.means.get(local).copied().unwrap_or(0.0));
        }
        offset += df;
    }

    let reference_point = match settings.reference_point {
        ReferencePoint::Zero => vec![0.0; p],
        ReferencePoint::Mean => means.clone(),
    };

    let baselines = compute_baselines(
        prep,
        &cols_centered,
        &fit.beta,
        &fit.cov,
        &means,
        &reference_point,
        settings.baseline_method,
    )?;

    let mut covariance = Vec::with_capacity(p);
    for r in 0..p {
        let row: Vec<f64> = (0..p)
            .map(|c| fit.cov.get(r * p + c).copied().unwrap_or(0.0))
            .collect();
        covariance.push(row);
    }

    Ok(CoxModel {
        terms,
        parameters,
        covariance,
        initial_log_likelihood: initial_ll,
        log_likelihood: fit.log_likelihood,
        iterations: fit.iterations,
        n_records: prep.n,
        n_events: prep.n_events,
        n_censored: prep.n_censored,
        reference_point,
        covariate_means: means,
        baselines,
        step_history,
        covariate_schemas: prep.schemas.clone(),
        stratified: prep.stratified,
    })
}

// ---------------------------------------------------------------------------------------------
// Unit tests

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-computed known-answer test WITHOUT ties (6 records, 1 numeric covariate `x`):
    ///
    /// Records:
    ///   (t=1, ev=1, x=1), (t=2, ev=0, x=0), (t=3, ev=1, x=0),
    ///   (t=4, ev=1, x=1), (t=5, ev=0, x=0), (t=6, ev=0, x=0).
    ///
    /// At beta = 0:
    ///   t=1: R_1 = {1..6} (n=6, sum x = 2, sum x^2 = 2), D_1 = {1} (d=1, s=1)
    ///        ll_1 = -ln(6), u_1 = 1 - 1/3 = 2/3, I_1 = 1/3 - 1/9 = 2/9
    ///   t=3: R_3 = {3..6} (n=4, sum x = 1, sum x^2 = 1), D_3 = {3} (d=1, s=0)
    ///        ll_3 = -ln(4), u_3 = 0 - 1/4 = -1/4, I_3 = 1/4 - 1/16 = 3/16
    ///   t=4: R_4 = {4..6} (n=3, sum x = 1, sum x^2 = 1), D_4 = {4} (d=1, s=1)
    ///        ll_4 = -ln(3), u_4 = 1 - 1/3 = 2/3, I_4 = 1/3 - 1/9 = 2/9
    ///   Totals at beta = 0:
    ///     l(0) = -(ln 6 + ln 4 + ln 3) = -ln(72)
    ///     u(0) = 2/3 - 1/4 + 2/3 = 13/12
    ///     I(0) = 4/9 + 3/16 = 91/144
    ///     First Newton step: beta^(1) = u(0) / I(0) = (13/12) / (91/144) = 12/7.
    ///
    /// Exact MPLE (`r = exp(beta)`):
    ///   u(beta) = (1 - 2r/(2r+4)) - r/(r+3) + (1 - r/(r+2)) = 4/(r+2) - r/(r+3) = 0
    ///   => r^2 - 2r - 12 = 0 => r = 1 + sqrt(13), beta_hat = ln(1 + sqrt(13)).
    ///
    /// Exact Kalbfleisch-Prentice baseline jumps at x = 0:
    ///   dh_1 = -(1/r) ln((r+4)/(2r+4))
    ///   dh_3 = -ln((r+2)/(r+3))
    ///   dh_4 = -(1/r) ln(2/(r+2)).
    #[test]
    fn cox_known_answer_no_ties() -> Result<(), Box<dyn std::error::Error>> {
        let data = CoxData {
            time: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            event: vec![true, false, true, true, false, false],
            strata: None,
            weights: None,
            covariates: vec![Covariate::Numeric {
                name: "x".into(),
                values: vec![
                    Some(1.0),
                    Some(0.0),
                    Some(0.0),
                    Some(1.0),
                    Some(0.0),
                    Some(0.0),
                ],
            }],
        };

        // Check l(0), u(0), I(0), and first Newton step directly.
        let prep = prepare_data(&data)?;
        let cols = collect_centered_columns(&prep, &[0]);
        let (ll0, u0, i0) = eval_ll_score_info(&prep, &cols, &[0.0])?;
        let u0_val = u0.first().copied().ok_or("missing u0")?;
        let i0_val = i0.first().copied().ok_or("missing i0")?;
        assert!((ll0 - (-(72.0_f64.ln()))).abs() < 1e-12);
        assert!((u0_val - 13.0 / 12.0).abs() < 1e-12);
        assert!((i0_val - 91.0 / 144.0).abs() < 1e-12);
        let step1 = u0_val / i0_val;
        assert!((step1 - 12.0 / 7.0).abs() < 1e-12);

        // Full fit with tight tolerance -> exact analytic MPLE and KP baseline.
        let settings = Settings {
            parameter_convergence: 1e-12,
            log_likelihood_convergence: 1e-12,
            ..Settings::default()
        };
        let model = train(&data, &settings)?;
        let r = 1.0 + 13.0_f64.sqrt();
        let beta_exact = r.ln();
        let beta_got = model.parameters.first().ok_or("missing param")?.beta;
        assert!((beta_got - beta_exact).abs() < 1e-11);

        let dh1 = -((r + 4.0) / (2.0 * r + 4.0)).ln() / r;
        let dh3 = -((r + 2.0) / (r + 3.0)).ln();
        let dh4 = -(2.0 / (r + 2.0)).ln() / r;
        let pts = &model.baselines.first().ok_or("missing baseline")?.points;
        assert_eq!(pts.len(), 3);
        assert!((pts.first().ok_or("pt0")?.cum_hazard - dh1).abs() < 1e-11);
        assert!((pts.get(1).ok_or("pt1")?.cum_hazard - (dh1 + dh3)).abs() < 1e-11);
        assert!((pts.get(2).ok_or("pt2")?.cum_hazard - (dh1 + dh3 + dh4)).abs() < 1e-11);

        // Verify survival and cumulative hazard at x=0 and x=1.
        let h_x1 = model.cumulative_hazard(&[CoxValue::Numeric(1.0)], None, 4.0)?;
        assert!((h_x1 - (dh1 + dh3 + dh4) * r).abs() < 1e-10);
        let s_x1 = model.survival(&[CoxValue::Numeric(1.0)], None, 4.0)?;
        assert!((s_x1 - (-h_x1).exp()).abs() < 1e-12);
        Ok(())
    }

    /// Hand-computed known-answer test WITH ties (6 records, 1 numeric covariate `x`):
    ///
    /// Records:
    ///   (t=1, ev=1, x=1), (t=1, ev=1, x=0), (t=2, ev=1, x=1),
    ///   (t=2, ev=0, x=0), (t=3, ev=0, x=0), (t=3, ev=0, x=0).
    ///
    /// At beta = 0 (Breslow ties):
    ///   t=1: R_1 = {1..6} (n=6, sum x=2, sum x^2=2), D_1 = {1,2} (d=2, s=1)
    ///        ll_1 = -2 ln(6), u_1 = 1 - 2*(1/3) = 1/3, I_1 = 2*(1/3 - 1/9) = 4/9
    ///   t=2: R_2 = {3..6} (n=4, sum x=1, sum x^2=1), D_2 = {3} (d=1, s=1)
    ///        ll_2 = -ln(4), u_2 = 1 - 1/4 = 3/4, I_2 = 1/4 - 1/16 = 3/16
    ///   Totals at beta = 0:
    ///     l(0) = -(2 ln 6 + ln 4) = -ln(144)
    ///     u(0) = 1/3 + 3/4 = 13/12
    ///     I(0) = 4/9 + 3/16 = 91/144
    ///     First Newton step: beta^(1) = 12/7.
    ///
    /// Exact Breslow MPLE (`r = exp(beta)`):
    ///   u(beta) = (1 - 2r/(r+2)) + 3/(r+3) = 0 => r^2 - 2r - 12 = 0 => beta_hat = ln(1 + sqrt(13)).
    ///
    /// KP tied-event jump `z_1` at t=1 satisfies:
    ///   r / (1 - exp(-z_1 * r)) + 1 / (1 - exp(-z_1)) = 2r + 4.
    #[test]
    fn cox_known_answer_with_ties() -> Result<(), Box<dyn std::error::Error>> {
        let data = CoxData {
            time: vec![1.0, 1.0, 2.0, 2.0, 3.0, 3.0],
            event: vec![true, true, true, false, false, false],
            strata: None,
            weights: None,
            covariates: vec![Covariate::Numeric {
                name: "x".into(),
                values: vec![
                    Some(1.0),
                    Some(0.0),
                    Some(1.0),
                    Some(0.0),
                    Some(0.0),
                    Some(0.0),
                ],
            }],
        };

        let prep = prepare_data(&data)?;
        let cols = collect_centered_columns(&prep, &[0]);
        let (ll0, u0, i0) = eval_ll_score_info(&prep, &cols, &[0.0])?;
        let u0_val = u0.first().copied().ok_or("missing u0")?;
        let i0_val = i0.first().copied().ok_or("missing i0")?;
        assert!((ll0 - (-(144.0_f64.ln()))).abs() < 1e-12);
        assert!((u0_val - 13.0 / 12.0).abs() < 1e-12);
        assert!((i0_val - 91.0 / 144.0).abs() < 1e-12);
        assert!((u0_val / i0_val - 12.0 / 7.0).abs() < 1e-12);

        let settings = Settings {
            parameter_convergence: 1e-12,
            log_likelihood_convergence: 1e-12,
            ..Settings::default()
        };
        let model = train(&data, &settings)?;
        let r = 1.0 + 13.0_f64.sqrt();
        let beta_got = model.parameters.first().ok_or("missing param")?.beta;
        assert!((beta_got - r.ln()).abs() < 1e-11);

        // Verify KP equation residual at t=1 (d=2 ties).
        let pts = &model.baselines.first().ok_or("missing baseline")?.points;
        let z1 = pts.first().ok_or("pt0")?.cum_hazard;
        let lhs = r / (1.0 - (-z1 * r).exp()) + 1.0 / (1.0 - (-z1).exp());
        let rhs = 2.0 * r + 4.0;
        assert!((lhs - rhs).abs() < 1e-10, "lhs={lhs} rhs={rhs}");

        // Verify untied jump at t=2.
        let dh2_exact = -(3.0 / (r + 3.0)).ln() / r;
        let dh2_got = pts.get(1).ok_or("pt1")?.cum_hazard - z1;
        assert!((dh2_got - dh2_exact).abs() < 1e-11);
        Ok(())
    }

    #[test]
    fn cox_properties_monotonic_survival_determinism_and_censored_stratum()
    -> Result<(), Box<dyn std::error::Error>> {
        let data = CoxData {
            time: vec![
                1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 2.0, 4.0, 7.0,
            ],
            event: vec![
                true, false, true, true, false, true, false, true, true, false, false, false,
                false, false, false,
            ],
            strata: Some(vec![
                Some(1),
                Some(1),
                Some(1),
                Some(1),
                Some(1),
                Some(1),
                Some(1),
                Some(1),
                Some(1),
                Some(1),
                Some(1),
                Some(1),
                Some(2), // censored-only stratum!
                Some(2),
                Some(2),
            ]),
            weights: None,
            covariates: vec![
                Covariate::Numeric {
                    name: "x".into(),
                    values: vec![
                        Some(2.0),
                        Some(1.5),
                        Some(0.5),
                        Some(1.2),
                        Some(0.3),
                        Some(1.8),
                        Some(0.9),
                        Some(0.1),
                        Some(1.1),
                        Some(0.4),
                        Some(0.2),
                        Some(0.7),
                        Some(0.5),
                        Some(1.5),
                        Some(0.0),
                    ],
                },
                Covariate::Factor {
                    name: "grp".into(),
                    levels: vec!["A".into(), "B".into(), "C".into()],
                    codes: vec![
                        Some(0),
                        Some(1),
                        Some(2),
                        Some(1),
                        Some(2),
                        Some(0),
                        Some(0),
                        Some(2),
                        Some(1),
                        Some(1),
                        Some(0),
                        Some(2),
                        Some(0),
                        Some(1),
                        Some(2),
                    ],
                },
            ],
        };

        let settings = Settings::default();
        let m1 = train(&data, &settings)?;
        let m2 = train(&data, &settings)?;
        assert_eq!(m1, m2, "training twice must produce identical results");

        // Verify factor indicator coding: last level ("C", code=2) is reference (contributes 0).
        let lp_ref = m1.linear_predictor(&[CoxValue::Numeric(0.0), CoxValue::Factor(2)])?;
        assert!(lp_ref.abs() < 1e-15);
        let lp_a = m1.linear_predictor(&[CoxValue::Numeric(0.0), CoxValue::Factor(0)])?;
        let beta_a = m1.parameters.get(1).ok_or("beta A")?.beta;
        assert!((lp_a - beta_a).abs() < 1e-15);

        // Non-decreasing baseline hazard & survival in (0, 1] for stratum 1.
        let b1 = m1
            .baselines
            .iter()
            .find(|b| b.stratum == Some(1))
            .ok_or("missing stratum 1")?;
        let mut prev_h = 0.0;
        for pt in &b1.points {
            assert!(pt.cum_hazard >= prev_h);
            assert!(pt.survival > 0.0 && pt.survival <= 1.0);
            prev_h = pt.cum_hazard;
        }
        for &t in &[0.0, 0.5, 1.0, 2.5, 4.0, 6.0, 8.0] {
            let s = m1.survival(&[CoxValue::Numeric(0.5), CoxValue::Factor(1)], Some(1), t)?;
            assert!(s > 0.0 && s <= 1.0);
        }

        // Censored-only stratum 2 does not crash and gives H0 = 0, S = 1.
        let h_s2 =
            m1.cumulative_hazard(&[CoxValue::Numeric(0.5), CoxValue::Factor(1)], Some(2), 5.0)?;
        assert_eq!(h_s2, 0.0);
        let s_s2 = m1.survival(&[CoxValue::Numeric(0.5), CoxValue::Factor(1)], Some(2), 5.0)?;
        assert_eq!(s_s2, 1.0);

        // Reference point Zero vs Mean give identical survival predictions.
        let m_mean = train(
            &data,
            &Settings {
                reference_point: ReferencePoint::Mean,
                ..Settings::default()
            },
        )?;
        let s_zero = m1.survival(&[CoxValue::Numeric(1.2), CoxValue::Factor(0)], Some(1), 3.5)?;
        let s_mean =
            m_mean.survival(&[CoxValue::Numeric(1.2), CoxValue::Factor(0)], Some(1), 3.5)?;
        assert!((s_zero - s_mean).abs() < 1e-12);

        // Conditional survival beyond max_event_time returns Ok(None).
        let cond_ok = m1.conditional_survival(
            &[CoxValue::Numeric(0.5), CoxValue::Factor(1)],
            Some(1),
            1.0,
            2.0,
        )?;
        assert!(cond_ok.is_some());
        let cond_null = m1.conditional_survival(
            &[CoxValue::Numeric(0.5), CoxValue::Factor(1)],
            Some(1),
            1.0,
            10.0,
        )?;
        assert!(cond_null.is_none());

        assert!(m1.to_text().contains("Cox Proportional Hazards Model"));
        Ok(())
    }

    #[test]
    fn cox_stepwise_removal_criteria_and_backwards() -> Result<(), Box<dyn std::error::Error>> {
        // Synthetic overlapping dataset where `strong` lowers survival time strongly (without
        // complete separation) and `noise` is unrelated to survival time.
        let n = 40usize;
        let mut time = Vec::with_capacity(n);
        let mut event = Vec::with_capacity(n);
        let mut x_strong = Vec::with_capacity(n);
        let mut x_noise = Vec::with_capacity(n);
        for i in 0..n {
            let high_risk = i % 2 == 0;
            let rank = (i / 2) as f64;
            // High-risk times in [1, 20], low-risk times in [6, 44] -> overlapping, strong effect.
            let t = if high_risk {
                1.0 + rank
            } else {
                6.0 + 2.0 * rank
            };
            let ev = i % 4 != 3;
            let noise = ((i * 7 + 3) % 11) as f64 - 5.0;
            time.push(t);
            event.push(ev);
            x_strong.push(Some(if high_risk { 1.0 } else { 0.0 }));
            x_noise.push(Some(noise));
        }
        let data = CoxData {
            time,
            event,
            strata: None,
            weights: None,
            covariates: vec![
                Covariate::Numeric {
                    name: "strong".into(),
                    values: x_strong,
                },
                Covariate::Numeric {
                    name: "noise".into(),
                    values: x_noise,
                },
            ],
        };

        for method in [Method::Stepwise, Method::BackwardsStepwise] {
            for crit in [
                RemovalCriterion::Lr,
                RemovalCriterion::Wald,
                RemovalCriterion::Conditional,
            ] {
                let model = train(
                    &data,
                    &Settings {
                        method,
                        removal_criterion: crit,
                        ..Settings::default()
                    },
                )?;
                assert_eq!(model.terms.len(), 1);
                assert_eq!(model.terms.first().ok_or("missing term")?.name, "strong");
            }
        }
        Ok(())
    }

    #[test]
    fn cox_case_weights_and_blank_handling() -> Result<(), Box<dyn std::error::Error>> {
        // Duplicating records with weight 1 must match single records with weight 2 under Breslow.
        let weighted = CoxData {
            time: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 99.0],
            event: vec![true, false, true, true, false, false, true],
            strata: None,
            weights: Some(vec![2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 1.0]),
            covariates: vec![Covariate::Numeric {
                name: "x".into(),
                values: vec![
                    Some(1.0),
                    Some(0.0),
                    Some(0.0),
                    Some(1.0),
                    Some(0.0),
                    Some(0.0),
                    None, // missing value -> excluded by blank handling
                ],
            }],
        };
        let duplicated = CoxData {
            time: vec![1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0, 5.0, 5.0, 6.0, 6.0],
            event: vec![
                true, true, false, false, true, true, true, true, false, false, false, false,
            ],
            strata: None,
            weights: None,
            covariates: vec![Covariate::Numeric {
                name: "x".into(),
                values: vec![
                    Some(1.0),
                    Some(1.0),
                    Some(0.0),
                    Some(0.0),
                    Some(0.0),
                    Some(0.0),
                    Some(1.0),
                    Some(1.0),
                    Some(0.0),
                    Some(0.0),
                    Some(0.0),
                    Some(0.0),
                ],
            }],
        };
        let settings = Settings {
            parameter_convergence: 1e-12,
            log_likelihood_convergence: 1e-12,
            baseline_method: BaselineMethod::Breslow,
            ..Settings::default()
        };
        let mw = train(&weighted, &settings)?;
        let md = train(&duplicated, &settings)?;
        assert_eq!(mw.n_records, 6);
        assert_eq!(md.n_records, 12);
        let bw = mw.parameters.first().ok_or("bw")?.beta;
        let bd = md.parameters.first().ok_or("bd")?.beta;
        assert!((bw - bd).abs() < 1e-12, "bw={bw} bd={bd}");
        let sew = mw.parameters.first().ok_or("sew")?.std_error;
        let sed = md.parameters.first().ok_or("sed")?.std_error;
        assert!((sew - sed).abs() < 1e-12, "sew={sew} sed={sed}");
        Ok(())
    }

    #[test]
    fn cox_never_crash_edge_cases_and_gaps() -> Result<(), Box<dyn std::error::Error>> {
        // 1. Empty data -> error
        let empty = CoxData {
            time: vec![],
            event: vec![],
            strata: None,
            weights: None,
            covariates: vec![],
        };
        assert!(train(&empty, &Settings::default()).is_err());

        // 2. One record (uncensored, 1 covariate) -> singular info, error, no panic
        let one = CoxData {
            time: vec![5.0],
            event: vec![true],
            strata: None,
            weights: None,
            covariates: vec![Covariate::Numeric {
                name: "x".into(),
                values: vec![Some(1.0)],
            }],
        };
        assert!(train(&one, &Settings::default()).is_err());

        // 3. No events -> error
        let no_events = CoxData {
            time: vec![1.0, 2.0, 3.0],
            event: vec![false, false, false],
            strata: None,
            weights: None,
            covariates: vec![Covariate::Numeric {
                name: "x".into(),
                values: vec![Some(1.0), Some(2.0), Some(3.0)],
            }],
        };
        assert!(train(&no_events, &Settings::default()).is_err());

        // 4. Constant covariate -> singular error in Enter, empty model in Stepwise
        let constant = CoxData {
            time: vec![1.0, 2.0, 3.0, 4.0],
            event: vec![true, true, false, false],
            strata: None,
            weights: None,
            covariates: vec![Covariate::Numeric {
                name: "c".into(),
                values: vec![Some(5.0), Some(5.0), Some(5.0), Some(5.0)],
            }],
        };
        assert!(train(&constant, &Settings::default()).is_err());
        let step_const = train(
            &constant,
            &Settings {
                method: Method::Stepwise,
                ..Settings::default()
            },
        )?;
        assert!(step_const.parameters.is_empty());

        // 5. Perfect separation (all events have x=1 and strictly earlier times than x=0)
        //    -> bounded error, never an infinite loop or NaN
        let separated = CoxData {
            time: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            event: vec![true, true, true, false, false, false],
            strata: None,
            weights: None,
            covariates: vec![Covariate::Numeric {
                name: "x".into(),
                values: vec![
                    Some(1.0),
                    Some(1.0),
                    Some(1.0),
                    Some(0.0),
                    Some(0.0),
                    Some(0.0),
                ],
            }],
        };
        assert!(train(&separated, &Settings::default()).is_err());

        // 6. Factor with one level -> 0 parameters, does not crash
        let one_level = CoxData {
            time: vec![1.0, 2.0, 3.0, 4.0],
            event: vec![true, false, true, false],
            strata: None,
            weights: None,
            covariates: vec![Covariate::Factor {
                name: "f".into(),
                levels: vec!["only".into()],
                codes: vec![Some(0), Some(0), Some(0), Some(0)],
            }],
        };
        let m_one_lv = train(&one_level, &Settings::default())?;
        assert!(m_one_lv.parameters.is_empty());
        assert_eq!(
            m_one_lv
                .baselines
                .first()
                .ok_or("missing baseline")?
                .points
                .len(),
            2
        );

        // 7. Thousands of strata (2000 strata within MAX_STRATA, plus > MAX_STRATA rejected)
        let n_strata = 2_000u32;
        let mut t = Vec::with_capacity((n_strata * 2) as usize);
        let mut ev = Vec::with_capacity((n_strata * 2) as usize);
        let mut st = Vec::with_capacity((n_strata * 2) as usize);
        let mut xv = Vec::with_capacity((n_strata * 2) as usize);
        for s in 0..n_strata {
            t.push(1.0);
            ev.push(true);
            st.push(Some(s));
            xv.push(Some(if s % 3 == 0 { 0.0 } else { 1.0 }));
            t.push(2.0);
            ev.push(false);
            st.push(Some(s));
            xv.push(Some(if s % 3 == 0 { 1.0 } else { 0.0 }));
        }
        let many_strata = CoxData {
            time: t,
            event: ev,
            strata: Some(st),
            weights: None,
            covariates: vec![Covariate::Numeric {
                name: "x".into(),
                values: xv,
            }],
        };
        let m_strata = train(&many_strata, &Settings::default())?;
        assert_eq!(m_strata.baselines.len(), 2_000);

        // Exceeding MAX_STRATA -> explicit error
        let too_many_strata = CoxData {
            time: vec![1.0; MAX_STRATA + 1],
            event: vec![true; MAX_STRATA + 1],
            strata: Some((0..=(MAX_STRATA as u32)).map(Some).collect()),
            weights: None,
            covariates: vec![],
        };
        assert!(train(&too_many_strata, &Settings::default()).is_err());

        // 8. Explicit GAP errors
        let valid = CoxData {
            time: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            event: vec![true, false, true, true, false, false],
            strata: None,
            weights: None,
            covariates: vec![Covariate::Numeric {
                name: "x".into(),
                values: vec![
                    Some(1.0),
                    Some(0.0),
                    Some(0.0),
                    Some(1.0),
                    Some(0.0),
                    Some(0.0),
                ],
            }],
        };
        for gap_settings in [
            Settings {
                ties: TieHandling::Efron,
                ..Settings::default()
            },
            Settings {
                model_type: ModelType::Custom,
                ..Settings::default()
            },
            Settings {
                time_dependent: true,
                ..Settings::default()
            },
            Settings {
                plots: true,
                ..Settings::default()
            },
        ] {
            match train(&valid, &gap_settings) {
                Err(err) => assert!(
                    err.to_string().contains("GAP:"),
                    "expected GAP error, got: {err}"
                ),
                Ok(_) => return Err("expected GAP error".into()),
            }
        }
        Ok(())
    }
}
