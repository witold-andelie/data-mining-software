//! Temporal Causal Modeling (IBM SPSS Modeler 19 Algorithms Guide, "Temporal Causal Modeling
//! Algorithms", pp. 670-687): graphical Granger causality via Group Orthogonal Matching Pursuit
//! (GOMP) with lagged predictors, Type III group F-tests, AR(L) auxiliary models for
//! predictor-only series, and one-step / multi-step forecasting.
//!
//! Pure algorithm module: takes aligned plain vectors ([`TcmData`]) and [`Settings`], has no
//! dependency on `Dataset` or PMML, and returns a [`TcmModel`] containing every target's fitted
//! model and any induced `AR(L)` models for selected predictor-only series.
//!
//! # Decisions the Guide does not settle
//!
//! Fixed and verified against the saved Modeler models in `target/agy-tcm/` (trained on
//! `target/agy-tcm/data/tcm_kpi.csv`, 112 weekly observations of 5 `Lever*` predictor-only series
//! and 25 `KPI_*` target-and-predictor series, from the `tcm_kpi` demo stream):
//!
//! A. **Series transformation and data preparation.** Although `build_settings_node.xml.txt` sets
//!    `seriesTransformation="true"`, the saved models apply **no** differencing, log, or Box-Cox
//!    transformation to the series values: `df_e = 112 - 5 - 25 - 1 = 81` for every 5-predictor
//!    TCM target (`tcm_series_5.xml` .. `tcm_series_29.xml`) and `df_e = 112 - 5 - 5 - 1 = 101`
//!    for every AR(5) predictor-only series (`tcm_series_0.xml` .. `tcm_series_4.xml`). The
//!    "Transformed Time Series" table in `TCMTopModelOutput.json` lists only `"Missing Value
//!    Imputed"` for `KPI_16`, confirming that data preparation performs missing-value imputation
//!    only. Reproduces the saved Modeler model: all 30 series match intercepts, lag coefficients,
//!    standard errors, F-statistics, and diagnostics to `<= 3e-12` relative error.
//! B. **Boundary missing-value imputation under `LINEAR_INTERP`.** `tcm_kpi.csv` has exactly one
//!    missing value: `KPI_16` (series 20) at the first time point `t = 0` (`TSDPOutput.json`
//!    reports `missingImputType = "LINEAR_INTERP"` and `meanMissingValuePercentage = 1 / 3360`).
//!    For a leading (or trailing) missing value, Modeler linearly extrapolates from the first (or
//!    last) two non-missing observations: `x[0] = 2 * x[1] - x[2] = 2643.3036191699315`.
//!    Reproduces the saved Modeler model: Target 20's intercept matches to `7.2e-15` relative and
//!    `residualSumOfSquares = 7252027.22959242` matches to `< 1e-15` relative (whereas carrying
//!    `x[1]` backward differs by `1.7e-3`, series mean by `2.0e-3`, and OLS linear trend by
//!    `2.0e-3`), and Targets 9, 12, and 17 (which select `KPI_16`) match to `<= 3e-12`.
//! C. **Column centering and scaling in `G`, `M`, `S`.** Guide p. 672 footnote 2 notes that one
//!    could center/scale lagged columns using either per-column statistics over rows `L..m` or
//!    whole-series statistics over `0..m`. Modeler centers and scales each lagged column by its
//!    **own column mean and sample standard deviation** (denominator `m - L - 1`) over the
//!    `m - L` regression rows `L..m`. Reproduces the saved Modeler model: centering each lagged
//!    column over `L..m` makes the centered regression equivalent to exact OLS with intercept and
//!    matches every target's intercept and SSE to `<= 4e-14` relative error.
//! D. **Intercept standard error (Guide Equation 6 omission).** Guide p. 676 Equation (6) writes
//!    `SE(beta_0) = sqrt((M/S)^T Sigma* (M/S))`, omitting the `sigma_hat^2 / (m - L)` variance of
//!    `y_bar`. Modeler computes the exact OLS intercept standard error
//!    `SE(beta_0) = sqrt(sigma_hat^2 / (m - L) + (M/S)^T Sigma* (M/S))`. Reproduces the saved
//!    Modeler model: on Target 5 (`KPI_1`), omitting `sigma_hat^2 / 107` gives `0.7918202930`
//!    (`3.6e-4` relative error), whereas including it gives `0.7921061823661613`, matching
//!    `tcm_series_5.xml` (`stdError="0.7921061823661613"`) to `< 1e-15` across all 25 TCM targets.
//! E. **GOMP candidate evaluation and predictor ordering.** Algorithm 2 (`argmin`, Guide pp. 673-674)
//!    regresses the current residual `r` on each candidate's lagged matrix `X_{G_j}` **alone**
//!    (not jointly with `X_aug`) and selects the first candidate (in series index order `0..n`)
//!    whose residual sum of squares satisfies `||r_j||_2^2 < cost - epsilon2` (`epsilon2 = 1e-8`).
//!    Reproduces the saved Modeler model: selects the exact same 5 predictors in the exact same
//!    order as `<Predictors>` in `tcm_series_5.xml` .. `tcm_series_29.xml` for all 25/25 targets
//!    (whereas joint re-estimation in `argmin` matches only 3/25 in order and 9/25 as sets).
//!    (`TCMGraph.json`'s `targetPredictorIndexMap` lists the same predictor sets in hash-map order,
//!    while `tcm_series_*.xml` preserves the GOMP selection order with the target itself first.)
//! F. **Lag storage order vs `TCMXML`.** In this module, every predictor's `coefficients`,
//!    `std_errors`, `t_stats`, `sigs`, `conf_lower`, and `conf_upper` vectors are stored in Guide
//!    Table 1 / Equation (17) order: index `0` is lag 1 (`X_{t-1}`) through index `L - 1` for lag
//!    `L` (`X_{t-L}`). Note that Modeler's saved `tcm_series_*.xml` writes `<Array>` entries in
//!    chronological reverse-lag order (`lag L` down to `lag 1`), verified on all 30 series.
//! G. **Auxiliary `AR(L)` models for predictor-only series.** Per Guide Section 2.5, every
//!    `InputOnly` series selected by at least one target receives an `AR(L)` model
//!    (`SeriesAlgorithm::Ar`, `J_sel = [s]`, `K* = 1`). Reproduces the saved Modeler model:
//!    all 5 `Lever1..Lever5` series (`tcm_series_0.xml` .. `tcm_series_4.xml`) are selected by
//!    at least one KPI target and match intercepts, lag coefficients, RSS, and `df_e = 101` to
//!    `<= 8.8e-14` relative error.
//! H. **Tail p-value evaluation (`sig`, `f_sig`).** Guide Equations (8) and (11) write
//!    `2 * (1 - prob(t <= |t_j|))` and `1 - prob(F <= |F_i|)`. Modeler's saved XML exhibits
//!    `1 - CDF` cancellation in extreme F-test tails (e.g. `KPI_3` predictor 7 has `F = 21.75`,
//!    saved `sig = 9.6256e-14` vs exact `9.6203e-14`, abs diff `5.4e-17`; `KPI_23` predictor 27
//!    has `F = 60.52`, saved `sig = 0.0` vs exact `6.35e-26`). We evaluate the upper tail
//!    directly via the regularized incomplete beta function (`crate::stats::beta_i` and `f_p`),
//!    reproducing every saved p-value to `<= 2.44e-13` absolute (`<= 1.20e-14` for F-test
//!    p-values, and `< 1e-9` relative for every `sig >= 1e-6`).
//!
//! Follows the Algorithms Guide (unverified against a saved Modeler model with these features):
//!
//! 1. **Rank deficiency and redundant predictors.** Uses the symmetric SWEEP operator (relative
//!    pivot tolerance `1e-12` on original diagonals) to compute the reflexive generalized inverse
//!    `(X_G*^T X_G*)^-` and effective rank `p_c` (Guide Section 2.6.1). Aliased lag columns get
//!    coefficient `0` and standard error `0`. In `J_sel^0` (Algorithm 1 line 5), any non-target
//!    forced predictor whose `L` lag columns are all aliased is dropped and `beta*^0, r^0` are
//!    re-estimated. All 30 `tcm_kpi` series are full rank (`p_c = K * L`), so rank-deficient
//!    behavior is verified only by synthetic unit tests.
//! 2. **Early stopping on `||r^k||_2 <= epsilon`.** Checked after initializing `r^0` as well as
//!    after each added predictor (Guide Algorithm 1 line 17), so a constant or perfectly-fit target
//!    stops without adding spurious predictors. On `tcm_kpi`, the smallest residual norm is
//!    `0.0325` (`KPI_10`, well above `epsilon = 0.001`), so early stopping does not trigger there.
//! 3. **Sample-size constraint.** Guide p. 670 states `m > L + K * L + 1`. We enforce
//!    `m > L + K_eff * L + 1` where `K_eff = min(max_predictors, eligible_predictors_for_target)`
//!    (at least `2 * L + 1 < m`), so single-series or few-series datasets are not artificially
//!    rejected by a larger `max_predictors` setting.
//! 4. **Interior linear interpolation.** Interior missing values are linearly interpolated between
//!    the nearest valid neighbors on each side (`x[left] + (t - left) * (x[right] - x[left]) /
//!    (right - left)`). If only one valid observation exists in a series, missing points cannot
//!    be extrapolated and are filled with that single value (yielding a constant series).
//! 5. **Explicit `GAP:` errors.**
//!    - Outlier detection (Guide Section 5), outlier root cause analysis (Section 6), and
//!      scenario analysis (Section 4): out of scope.
//!    - Missing-value imputation modes other than `LinearInterp` and `None` (`SeriesMean`,
//!      `KMean`, `KMedian`, `LinearTrend`): TSDP formulas are not specified in the TCM chapter.
//!    - Re-estimation (`re_estimate`) and estimation-period subsetting (`estimation_period`): not
//!      defined in the TCM chapter.

use crate::AlgoError;
use crate::stats::{beta_i, f_p, student_t_quantile};
use std::fmt::Write as _;

/// Hard limit on the number of input time series.
const MAX_SERIES: usize = 1_000;
/// Hard limit on the number of time points per series (`TSDPOutput.json`: 100,000).
const MAX_POINTS: usize = 100_000;
/// Hard limit on the lag order `L`.
const MAX_LAG: usize = 100;
/// Hard limit on the maximum predictors per target `K*`.
const MAX_PREDICTORS: usize = 50;
/// Hard limit on total lagged parameters `K* * L` in a single regression.
const MAX_PARAMETERS: usize = 500;
/// Hard limit on total raw cells (`series * points`).
const MAX_DATA_CELLS: usize = 50_000_000;
/// Hard limit on multi-step forecast horizon.
const MAX_FORECAST_STEPS: usize = 1_000;
/// Relative pivot tolerance for declaring a column redundant in SWEEP.
const SWEEP_TOL: f64 = 1e-12;
/// Minimum standard deviation below which a lagged column is treated as constant.
const CONST_STD_TOL: f64 = 1e-15;

/// Error type returned by Temporal Causal Modeling training and scoring.
pub type TcmError = AlgoError;

fn gap(what: &str) -> TcmError {
    AlgoError::Unsupported(format!("GAP: TCM {what}"))
}

/// Role of a time series in Temporal Causal Modeling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesRole {
    /// Both a target series (fitted with GOMP) and a candidate predictor for other targets.
    Both,
    /// Candidate predictor only; receives an `AR(L)` model if selected by any target.
    InputOnly,
    /// Target series only; fitted with GOMP using itself and candidate predictors, but never
    /// selected as a predictor for another target.
    TargetOnly,
}

/// A single aligned time series passed to [`train`].
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    /// Series name.
    pub name: String,
    /// Equally-spaced time observations (`None` denotes a missing value).
    pub values: Vec<Option<f64>>,
    /// Modeling role of this series.
    pub role: SeriesRole,
}

/// Input dataset for [`train`]: aligned time series plus optional forced/forbidden constraints.
#[derive(Debug, Clone, PartialEq)]
pub struct TcmData {
    /// All input/target time series, aligned on the same time grid of length `m`.
    pub series: Vec<Series>,
    /// Indices of series pre-selected into `J_sel^0` for every TCM target (`forced_inputs`).
    pub forced_inputs: Vec<usize>,
    /// Optional `(target_index, predictor_index)` pairs forbidden from selection (`J_sel^~`).
    pub forbidden_inputs: Vec<(usize, usize)>,
}

impl TcmData {
    /// Construct a [`TcmData`] with no forced or forbidden input overrides.
    pub fn new(series: Vec<Series>) -> Self {
        Self {
            series,
            forced_inputs: Vec::new(),
            forbidden_inputs: Vec::new(),
        }
    }
}

/// Missing-value imputation rule (`tcmnode` `missing_imput` property).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingImputation {
    /// Linear interpolation for interior gaps and linear extrapolation from the first/last two
    /// valid observations for leading/trailing gaps (Modeler default, verified on `tcm_kpi`).
    LinearInterp,
    /// Reject missing values (no imputation; succeeds if data have no missing values).
    None,
    /// Replace with series mean (not defined in TCM Guide; returns `GAP:`).
    SeriesMean,
    /// Mean of `k` nearby points (not defined in TCM Guide; returns `GAP:`).
    KMean(usize),
    /// Median of `k` nearby points (not defined in TCM Guide; returns `GAP:`).
    KMedian(usize),
    /// Linear trend at point (not defined in TCM Guide; returns `GAP:`).
    LinearTrend,
}

/// Build settings for Temporal Causal Modeling (Modeler defaults in [`Default`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Explicit lag count `L`. When `None`, `L` is chosen from [`Self::periodicity`] via Guide
    /// Section 2.4 (`min(s, 20)` if `s > 1` and `m >= 4s`, otherwise `5`).
    pub max_lag: Option<usize>,
    /// Seasonal periodicity `s >= 1` used only when `max_lag` is `None` (default `1`).
    pub periodicity: usize,
    /// Maximum number of predictors per target `K*`, including the target itself (default `5`).
    pub max_predictors: usize,
    /// Stopping tolerance `epsilon` on the residual L2 norm `||r^k||_2` (default `0.001`).
    pub epsilon: f64,
    /// Minimum reduction `epsilon2` in residual sum of squares required by `argmin` to prefer a
    /// candidate predictor (default `1e-8`).
    pub epsilon2: f64,
    /// Confidence level in `(0, 1)` for parameter confidence intervals (default `0.95`).
    pub confidence_level: f64,
    /// Missing-value imputation strategy (default [`MissingImputation::LinearInterp`]).
    pub missing_imputation: MissingImputation,
    /// Maximum allowed fraction of missing values in a single series, in `[0, 1)` (default
    /// `0.25`, matching `TSDPOutput.json`).
    pub missing_value_threshold: f64,
    /// Whether to fit an `AR(L)` model for each `InputOnly` series selected by at least one
    /// target (Guide Section 2.5, default `true`).
    pub build_ar_for_predictors: bool,
    /// `seriesTransformation` flag from node settings (default `true`). Only missing-value
    /// imputation is performed; when `false`, any missing value is rejected as an error.
    pub series_transformation: bool,
    /// Outlier detection (Guide Section 5, out of scope -> returns `GAP:` if `true`).
    pub outlier_detection: bool,
    /// Outlier root cause analysis (Guide Section 6, out of scope -> returns `GAP:` if `true`).
    pub root_cause_analysis: bool,
    /// Scenario analysis (Guide Section 4, out of scope -> returns `GAP:` if `true`).
    pub scenario_analysis: bool,
    /// Model re-estimation flag (`is_re_est`, returns `GAP:` if `true`).
    pub re_estimate: bool,
    /// Estimation-period subsetting flag (`use_estimation_period`, returns `GAP:` if `true`).
    pub estimation_period: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_lag: None,
            periodicity: 1,
            max_predictors: 5,
            epsilon: 0.001,
            epsilon2: 1e-8,
            confidence_level: 0.95,
            missing_imputation: MissingImputation::LinearInterp,
            missing_value_threshold: 0.25,
            build_ar_for_predictors: true,
            series_transformation: true,
            outlier_detection: false,
            root_cause_analysis: false,
            scenario_analysis: false,
            re_estimate: false,
            estimation_period: false,
        }
    }
}

/// Which algorithm produced a [`SeriesModel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesAlgorithm {
    /// Full GOMP temporal causal model (`algorithmName="TCM"`).
    Tcm,
    /// Univariate autoregressive `AR(L)` model (`algorithmName="AR"`, Guide Section 2.5).
    Ar,
}

/// Intercept estimate and its inference statistics on the original scale.
#[derive(Debug, Clone, PartialEq)]
pub struct InterceptStats {
    /// Unstandardized intercept estimate `beta_hat_0`.
    pub value: f64,
    /// Standard error of `beta_hat_0`.
    pub std_error: f64,
    /// Student t-statistic `beta_hat_0 / std_error` (`0.0` when `std_error == 0.0`).
    pub t_stat: f64,
    /// Two-sided p-value under `t(df_e)`.
    pub sig: f64,
    /// Lower confidence limit at `confidence_level`.
    pub conf_lower: f64,
    /// Upper confidence limit at `confidence_level`.
    pub conf_upper: f64,
}

/// Estimated lag parameters, inference, and Type III group F-test for one selected predictor.
#[derive(Debug, Clone, PartialEq)]
pub struct PredictorModel {
    /// Zero-based index of this predictor in [`TcmData::series`].
    pub series_index: usize,
    /// Name of this predictor series.
    pub series_name: String,
    /// Unstandardized lag coefficients ordered lag 1 (`X_{t-1}`) to lag `L` (`X_{t-L}`).
    pub coefficients: Vec<f64>,
    /// Standard errors of lag coefficients, ordered lag 1 to lag `L`.
    pub std_errors: Vec<f64>,
    /// Student t-statistics of lag coefficients, ordered lag 1 to lag `L`.
    pub t_stats: Vec<f64>,
    /// Two-sided p-values of lag coefficients, ordered lag 1 to lag `L`.
    pub sigs: Vec<f64>,
    /// Lower confidence limits of lag coefficients, ordered lag 1 to lag `L`.
    pub conf_lower: Vec<f64>,
    /// Upper confidence limits of lag coefficients, ordered lag 1 to lag `L`.
    pub conf_upper: Vec<f64>,
    /// Type III group F-statistic testing `H_0: beta_{j,1} = ... = beta_{j,L} = 0`.
    pub f_stat: f64,
    /// Numerator degrees of freedom `r_i = rank(L_i Sigma L_i^T)`.
    pub df1: usize,
    /// Denominator degrees of freedom `df_e = m - L - p_c - 1`.
    pub df2: usize,
    /// Right-tail F-test p-value `P(F_{df1, df2} >= f_stat)`.
    pub f_sig: f64,
}

/// Goodness-of-fit and residual diagnostics for one fitted series model (Guide Section 2.6.3).
#[derive(Debug, Clone, PartialEq)]
pub struct ModelDiagnostics {
    /// Residual sum of squares `SS_e = ||y - y_hat||_2^2`.
    pub rss: f64,
    /// Error degrees of freedom `df_e = m - L - p_c - 1`.
    pub df_e: usize,
    /// Number of non-redundant lagged parameters `p_c <= K * L`.
    pub non_redundant_params: usize,
    /// Mean squared error `sigma_hat^2 = SS_e / df_e`.
    pub mse: f64,
    /// Root mean squared error `RMSE = sqrt(MSE)`.
    pub rmse: f64,
    /// Coefficient of determination `R^2 = 1 - SS_e / SS_t` (`None` when `SS_t == 0`).
    pub r_squared: Option<f64>,
    /// Bayesian Information Criterion (`None` when `SS_e <= 0`).
    pub bic: Option<f64>,
    /// Akaike Information Criterion (`None` when `SS_e <= 0`).
    pub aic: Option<f64>,
    /// Root Mean Squared Percentage Error (`None` if any `y_t == 0` in `L..m`).
    pub rmspe: Option<f64>,
}

/// Fitted model for a single target (or induced `AR(L)` predictor-only) series.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesModel {
    /// Zero-based series index in [`TcmData::series`].
    pub target_index: usize,
    /// Series name.
    pub target_name: String,
    /// Whether this series was fitted with GOMP (`Tcm`) or `AR(L)` (`Ar`).
    pub algorithm: SeriesAlgorithm,
    /// Number of lags `L`.
    pub max_lags: usize,
    /// Intercept estimate and inference.
    pub intercept: InterceptStats,
    /// Selected predictors in GOMP selection order (target itself first).
    pub predictors: Vec<PredictorModel>,
    /// Residual and model-quality diagnostics.
    pub diagnostics: ModelDiagnostics,
}

/// Result of multi-step forecasting for one modeled series (Guide Equations 20, 24, 25).
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesForecast {
    /// Zero-based series index in [`TcmData::series`].
    pub series_index: usize,
    /// Series name.
    pub series_name: String,
    /// Point forecasts for horizons `h = 1..=steps`.
    pub values: Vec<f64>,
    /// Approximated lower confidence bounds for horizons `h = 1..=steps`.
    pub lower: Vec<f64>,
    /// Approximated upper confidence bounds for horizons `h = 1..=steps`.
    pub upper: Vec<f64>,
}

/// Trained Temporal Causal Model system covering all targets and induced `AR(L)` series.
#[derive(Debug, Clone, PartialEq)]
pub struct TcmModel {
    /// Number of lags `L` used for all series models.
    pub lag: usize,
    /// Number of aligned training time points `m`.
    pub num_points: usize,
    /// Confidence level used for intervals.
    pub confidence_level: f64,
    /// Names of all series in index order `0..n`.
    pub series_names: Vec<String>,
    /// Fitted models sorted by `target_index` (both TCM targets and induced AR predictors).
    pub models: Vec<SeriesModel>,
}

impl TcmModel {
    /// Look up the fitted model for a series by its zero-based index.
    pub fn model_for(&self, series_index: usize) -> Option<&SeriesModel> {
        self.models.iter().find(|m| m.target_index == series_index)
    }

    /// Look up the fitted model for a series by its name.
    pub fn model_by_name(&self, name: &str) -> Option<&SeriesModel> {
        self.models.iter().find(|m| m.target_name == name)
    }

    /// One-step-ahead prediction for `target_index` from aligned historical series (Guide Eq. 19).
    ///
    /// `history` must have one slice per series in [`Self::series_names`], each of the same
    /// length `>= self.lag`, containing only finite numbers. Uses the last `self.lag` values of
    /// each selected predictor (`history[j][len - 1]` is lag 1, ..., `history[j][len - L]` is
    /// lag `L`).
    pub fn predict_one_step(
        &self,
        target_index: usize,
        history: &[&[f64]],
    ) -> Result<f64, TcmError> {
        let model = self.model_for(target_index).ok_or_else(|| {
            AlgoError::Data(format!("no fitted TCM/AR model for series {target_index}"))
        })?;
        let len = validate_history(history, self.series_names.len(), self.lag)?;
        eval_one_step(model, history, len)
    }

    /// In-sample one-step-ahead fitted values for `target_index` over `t = L..m` (Guide Eq. 17).
    ///
    /// Returns a vector of length `m - L`, where entry `i` predicts observation `t = L + i`.
    pub fn fit_series(
        &self,
        target_index: usize,
        history: &[&[f64]],
    ) -> Result<Vec<f64>, TcmError> {
        let model = self.model_for(target_index).ok_or_else(|| {
            AlgoError::Data(format!("no fitted TCM/AR model for series {target_index}"))
        })?;
        let len = validate_history(history, self.series_names.len(), self.lag)?;
        let n_obs = len.saturating_sub(self.lag);
        let mut fitted = Vec::with_capacity(n_obs);
        for t in self.lag..len {
            let mut y_hat = model.intercept.value;
            for pred in &model.predictors {
                let series = history.get(pred.series_index).ok_or_else(|| {
                    AlgoError::Data(format!(
                        "predictor index {} out of range",
                        pred.series_index
                    ))
                })?;
                for (lag_idx, &coef) in pred.coefficients.iter().enumerate() {
                    let pos = t
                        .checked_sub(lag_idx + 1)
                        .ok_or_else(|| AlgoError::Data("lag index underflow".into()))?;
                    let x_val = *series
                        .get(pos)
                        .ok_or_else(|| AlgoError::Data("history index out of range".into()))?;
                    y_hat += coef * x_val;
                }
            }
            fitted.push(y_hat);
        }
        Ok(fitted)
    }

    /// Multi-step-ahead forecasts (`h = 1..=steps`) with approximated forecasting intervals
    /// (Guide Equations 20, 24, 25) for all modeled series.
    ///
    /// Requires that every predictor referenced by any model has its own fitted model (true
    /// whenever `build_ar_for_predictors` was `true`, or whenever `steps == 1`).
    pub fn forecast(
        &self,
        history: &[&[f64]],
        steps: usize,
    ) -> Result<Vec<SeriesForecast>, TcmError> {
        if steps == 0 || steps > MAX_FORECAST_STEPS {
            return Err(AlgoError::Settings(format!(
                "forecast steps must be in 1..={MAX_FORECAST_STEPS}, got {steps}"
            )));
        }
        let hist_len = validate_history(history, self.series_names.len(), self.lag)?;
        let n_series = self.series_names.len();
        let mut model_lookup: Vec<Option<&SeriesModel>> = vec![None; n_series];
        for m in &self.models {
            if let Some(slot) = model_lookup.get_mut(m.target_index) {
                *slot = Some(m);
            }
        }
        if steps > 1 {
            for m in &self.models {
                for p in &m.predictors {
                    let has_model = model_lookup
                        .get(p.series_index)
                        .and_then(|opt| *opt)
                        .is_some();
                    if !has_model {
                        return Err(AlgoError::Data(format!(
                            "multi-step forecast requires a model for predictor '{}' (index {})",
                            p.series_name, p.series_index
                        )));
                    }
                }
            }
        }

        // Buffers for observed + forecasted values and forecast error variances per series.
        // Indices 0..L hold the last L observed points; L + (h - 1) holds step h forecast.
        let buf_len = self.lag + steps;
        let mut vals: Vec<Vec<f64>> = Vec::with_capacity(n_series);
        for s in history {
            let start = hist_len.saturating_sub(self.lag);
            let mut row = Vec::with_capacity(buf_len);
            if let Some(tail) = s.get(start..hist_len) {
                row.extend_from_slice(tail);
            }
            row.resize(buf_len, 0.0);
            vals.push(row);
        }
        // Error variance at observed points (indices 0..L) is 0.0.
        let mut vars: Vec<Vec<f64>> = vec![vec![0.0; buf_len]; n_series];

        for h in 1..=steps {
            let cur = self.lag + h - 1;
            for m in &self.models {
                let mut y_hat = m.intercept.value;
                let mut v_hat = m.diagnostics.mse;
                for pred in &m.predictors {
                    let p_vals = vals
                        .get(pred.series_index)
                        .ok_or_else(|| AlgoError::Data("predictor buffer missing".into()))?;
                    let p_vars = vars.get(pred.series_index).ok_or_else(|| {
                        AlgoError::Data("predictor variance buffer missing".into())
                    })?;
                    for (lag_idx, &coef) in pred.coefficients.iter().enumerate() {
                        let lag = lag_idx + 1;
                        let pos = cur
                            .checked_sub(lag)
                            .ok_or_else(|| AlgoError::Data("forecast lag underflow".into()))?;
                        let x_v = *p_vals.get(pos).ok_or_else(|| {
                            AlgoError::Data("forecast buffer index out of range".into())
                        })?;
                        let x_var = *p_vars.get(pos).ok_or_else(|| {
                            AlgoError::Data("forecast var index out of range".into())
                        })?;
                        y_hat += coef * x_v;
                        v_hat += coef * coef * x_var;
                    }
                }
                if let Some(row) = vals.get_mut(m.target_index)
                    && let Some(slot) = row.get_mut(cur)
                {
                    *slot = y_hat;
                }
                if let Some(row) = vars.get_mut(m.target_index)
                    && let Some(slot) = row.get_mut(cur)
                {
                    *slot = v_hat.max(0.0);
                }
            }
        }

        let alpha = 1.0 - self.confidence_level;
        let prob = 1.0 - 0.5 * alpha;
        let mut out = Vec::with_capacity(self.models.len());
        for m in &self.models {
            let t_crit = if m.diagnostics.df_e > 0 {
                student_t_quantile(prob, m.diagnostics.df_e as f64).unwrap_or(0.0)
            } else {
                0.0
            };
            let mut f_vals = Vec::with_capacity(steps);
            let mut lower = Vec::with_capacity(steps);
            let mut upper = Vec::with_capacity(steps);
            let row_v = vals
                .get(m.target_index)
                .ok_or_else(|| AlgoError::Data("missing forecast values".into()))?;
            let row_var = vars
                .get(m.target_index)
                .ok_or_else(|| AlgoError::Data("missing forecast variances".into()))?;
            for h in 1..=steps {
                let idx = self.lag + h - 1;
                let y = *row_v.get(idx).unwrap_or(&0.0);
                let sd = row_var.get(idx).copied().unwrap_or(0.0).max(0.0).sqrt();
                let half = t_crit * sd;
                f_vals.push(y);
                lower.push(y - half);
                upper.push(y + half);
            }
            out.push(SeriesForecast {
                series_index: m.target_index,
                series_name: m.target_name.clone(),
                values: f_vals,
                lower,
                upper,
            });
        }
        Ok(out)
    }

    /// Format the trained model as human-readable text for inspection and snapshot checks.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "Temporal Causal Model (series={}, points={}, lag={}, models={}, conf={:.4})",
            self.series_names.len(),
            self.num_points,
            self.lag,
            self.models.len(),
            self.confidence_level
        );
        for m in &self.models {
            let algo = match m.algorithm {
                SeriesAlgorithm::Tcm => "TCM",
                SeriesAlgorithm::Ar => "AR",
            };
            let _ = writeln!(
                out,
                "\n[{}] target={} ({}) lag={} rss={:.6} df_e={} rmse={:.6} r2={}",
                algo,
                m.target_index,
                m.target_name,
                m.max_lags,
                m.diagnostics.rss,
                m.diagnostics.df_e,
                m.diagnostics.rmse,
                match m.diagnostics.r_squared {
                    Some(v) => format!("{v:.6}"),
                    None => "NA".to_string(),
                }
            );
            let _ = writeln!(
                out,
                "  Intercept: val={:.6} se={:.6} t={:.6} sig={:.6} ci=[{:.6}, {:.6}]",
                m.intercept.value,
                m.intercept.std_error,
                m.intercept.t_stat,
                m.intercept.sig,
                m.intercept.conf_lower,
                m.intercept.conf_upper
            );
            for p in &m.predictors {
                let _ = write!(
                    out,
                    "  Predictor {} ({}): F={:.6} df=({},{}) sig={:.6} coefs(lag1..{})=[",
                    p.series_index, p.series_name, p.f_stat, p.df1, p.df2, p.f_sig, m.max_lags
                );
                for (i, c) in p.coefficients.iter().enumerate() {
                    if i > 0 {
                        let _ = write!(out, ", ");
                    }
                    let _ = write!(out, "{c:.6}");
                }
                let _ = writeln!(out, "]");
            }
        }
        out
    }
}

/// Train a Temporal Causal Model on `data` using `settings`.
///
/// Fits a GOMP model (Guide Algorithms 1 & 2) for every series whose role is [`SeriesRole::Both`]
/// or [`SeriesRole::TargetOnly`], computes post-estimation inference and diagnostics (Guide
/// Section 2.6), and optionally fits `AR(L)` models (Guide Section 2.5) for any
/// [`SeriesRole::InputOnly`] series selected as a predictor by at least one target.
pub fn train(data: &TcmData, settings: &Settings) -> Result<TcmModel, TcmError> {
    crate::check_cancelled()?;
    validate_settings(settings)?;
    let (n_series, m) = validate_data(data, settings)?;
    let lag = select_lag(settings, m)?;

    // Step 1: Data preparation (imputation of missing values).
    let prepared = prepare_series(data, settings, m)?;

    // Collect targets and validate sample-size constraint per target (Guide p. 670).
    let targets: Vec<usize> = data
        .series
        .iter()
        .enumerate()
        .filter_map(|(i, s)| match s.role {
            SeriesRole::Both | SeriesRole::TargetOnly => Some(i),
            SeriesRole::InputOnly => None,
        })
        .collect();
    if targets.is_empty() {
        return Err(AlgoError::Data(
            "no target series (expected at least one Both or TargetOnly series)".into(),
        ));
    }

    // Validate forced and forbidden indices.
    for &idx in &data.forced_inputs {
        if idx >= n_series {
            return Err(AlgoError::Settings(format!(
                "forced input index {idx} out of range (series count {n_series})"
            )));
        }
    }
    for &(t_idx, p_idx) in &data.forbidden_inputs {
        if t_idx >= n_series || p_idx >= n_series {
            return Err(AlgoError::Settings(format!(
                "forbidden pair ({t_idx}, {p_idx}) out of range (series count {n_series})"
            )));
        }
    }

    // Precompute lag blocks (raw, column means/stds, standardized, and block SWEEP ginv) once for
    // every series.
    let n_obs = m
        .checked_sub(lag)
        .ok_or_else(|| AlgoError::Data("fewer points than lags".into()))?;
    let blocks = precompute_blocks(&prepared, lag, n_obs)?;

    let mut fitted_models: Vec<SeriesModel> = Vec::with_capacity(targets.len());
    let mut selected_input_only = vec![false; n_series];

    for &target_idx in &targets {
        crate::check_cancelled()?;
        let forbidden = build_forbidden_mask(data, target_idx, n_series);
        let eligible_count = forbidden.iter().filter(|&&f| !f).count();
        let k_eff = settings.max_predictors.min(eligible_count.max(1));
        check_sample_size(m, lag, k_eff)?;

        let j_init = build_initial_predictors(data, target_idx, &forbidden);
        let selected = run_gomp(
            &prepared, &blocks, target_idx, &j_init, &forbidden, lag, n_obs, settings,
        )?;

        for &pid in &selected {
            if let Some(s) = data.series.get(pid)
                && s.role == SeriesRole::InputOnly
                && let Some(flag) = selected_input_only.get_mut(pid)
            {
                *flag = true;
            }
        }

        let series_model = build_series_model(
            data,
            &prepared,
            &blocks,
            target_idx,
            &selected,
            SeriesAlgorithm::Tcm,
            lag,
            n_obs,
            settings.confidence_level,
        )?;
        fitted_models.push(series_model);
    }

    // Section 2.5: Fit AR(L) models for any InputOnly series selected by at least one target.
    if settings.build_ar_for_predictors {
        for (s_idx, &used) in selected_input_only.iter().enumerate() {
            if !used {
                continue;
            }
            crate::check_cancelled()?;
            check_sample_size(m, lag, 1)?;
            let selected = vec![s_idx];
            let ar_model = build_series_model(
                data,
                &prepared,
                &blocks,
                s_idx,
                &selected,
                SeriesAlgorithm::Ar,
                lag,
                n_obs,
                settings.confidence_level,
            )?;
            fitted_models.push(ar_model);
        }
    }

    fitted_models.sort_by_key(|sm| sm.target_index);

    let series_names = data.series.iter().map(|s| s.name.clone()).collect();
    Ok(TcmModel {
        lag,
        num_points: m,
        confidence_level: settings.confidence_level,
        series_names,
        models: fitted_models,
    })
}

fn validate_settings(s: &Settings) -> Result<(), TcmError> {
    if s.outlier_detection {
        return Err(gap("outlier detection is out of scope"));
    }
    if s.root_cause_analysis {
        return Err(gap("outlier root cause analysis is out of scope"));
    }
    if s.scenario_analysis {
        return Err(gap("scenario analysis is out of scope"));
    }
    if s.re_estimate {
        return Err(gap("re-estimation is not defined in the Algorithms Guide"));
    }
    if s.estimation_period {
        return Err(gap(
            "estimation period filtering is not defined in the Algorithms Guide",
        ));
    }
    match s.missing_imputation {
        MissingImputation::LinearInterp | MissingImputation::None => {}
        MissingImputation::SeriesMean => {
            return Err(gap(
                "missing imputation mode SeriesMean is not defined in the TCM Algorithms Guide",
            ));
        }
        MissingImputation::KMean(_) => {
            return Err(gap(
                "missing imputation mode KMean is not defined in the TCM Algorithms Guide",
            ));
        }
        MissingImputation::KMedian(_) => {
            return Err(gap(
                "missing imputation mode KMedian is not defined in the TCM Algorithms Guide",
            ));
        }
        MissingImputation::LinearTrend => {
            return Err(gap(
                "missing imputation mode LinearTrend is not defined in the TCM Algorithms Guide",
            ));
        }
    }
    if let Some(l) = s.max_lag
        && (l == 0 || l > MAX_LAG)
    {
        return Err(AlgoError::Settings(format!(
            "max_lag must be in 1..={MAX_LAG}, got {l}"
        )));
    }
    if s.periodicity == 0 || s.periodicity > MAX_POINTS {
        return Err(AlgoError::Settings(format!(
            "periodicity must be in 1..={MAX_POINTS}, got {}",
            s.periodicity
        )));
    }
    if s.max_predictors == 0 || s.max_predictors > MAX_PREDICTORS {
        return Err(AlgoError::Settings(format!(
            "max_predictors must be in 1..={MAX_PREDICTORS}, got {}",
            s.max_predictors
        )));
    }
    if !s.epsilon.is_finite() || s.epsilon < 0.0 {
        return Err(AlgoError::Settings(format!(
            "epsilon must be finite and >= 0, got {}",
            s.epsilon
        )));
    }
    if !s.epsilon2.is_finite() || s.epsilon2 < 0.0 {
        return Err(AlgoError::Settings(format!(
            "epsilon2 must be finite and >= 0, got {}",
            s.epsilon2
        )));
    }
    if !s.confidence_level.is_finite() || s.confidence_level <= 0.0 || s.confidence_level >= 1.0 {
        return Err(AlgoError::Settings(format!(
            "confidence_level must be in (0, 1), got {}",
            s.confidence_level
        )));
    }
    if !s.missing_value_threshold.is_finite()
        || s.missing_value_threshold < 0.0
        || s.missing_value_threshold >= 1.0
    {
        return Err(AlgoError::Settings(format!(
            "missing_value_threshold must be in [0, 1), got {}",
            s.missing_value_threshold
        )));
    }
    Ok(())
}

fn validate_data(data: &TcmData, settings: &Settings) -> Result<(usize, usize), TcmError> {
    let n_series = data.series.len();
    if n_series == 0 {
        return Err(AlgoError::Data("input contains no series".into()));
    }
    if n_series > MAX_SERIES {
        return Err(AlgoError::Data(format!(
            "series count {n_series} exceeds limit {MAX_SERIES}"
        )));
    }
    let m = data.series.first().map(|s| s.values.len()).unwrap_or(0);
    if m == 0 {
        return Err(AlgoError::Data("input series have zero time points".into()));
    }
    if m > MAX_POINTS {
        return Err(AlgoError::Data(format!(
            "time point count {m} exceeds limit {MAX_POINTS}"
        )));
    }
    let cells = n_series
        .checked_mul(m)
        .ok_or_else(|| AlgoError::Data("data cell count overflow".into()))?;
    if cells > MAX_DATA_CELLS {
        return Err(AlgoError::Data(format!(
            "total cells {cells} exceeds limit {MAX_DATA_CELLS}"
        )));
    }
    for (idx, s) in data.series.iter().enumerate() {
        if s.values.len() != m {
            return Err(AlgoError::Data(format!(
                "series '{}' (index {idx}) has length {}, expected {m}",
                s.name,
                s.values.len()
            )));
        }
        let mut missing = 0usize;
        for (t, v) in s.values.iter().enumerate() {
            match v {
                Some(x) if !x.is_finite() => {
                    return Err(AlgoError::Data(format!(
                        "non-finite value in series '{}' at index {t}",
                        s.name
                    )));
                }
                None => missing += 1,
                Some(_) => {}
            }
        }
        let missing_frac = missing as f64 / m as f64;
        if missing_frac > settings.missing_value_threshold {
            return Err(AlgoError::Data(format!(
                "series '{}' missing fraction {:.4} exceeds threshold {:.4}",
                s.name, missing_frac, settings.missing_value_threshold
            )));
        }
        if missing == m {
            return Err(AlgoError::Data(format!(
                "series '{}' has only missing values",
                s.name
            )));
        }
    }
    Ok((n_series, m))
}

fn select_lag(settings: &Settings, m: usize) -> Result<usize, TcmError> {
    let lag = match settings.max_lag {
        Some(l) => l,
        None => {
            let s = settings.periodicity;
            if s > 1 && m >= s.saturating_mul(4) {
                s.min(20)
            } else {
                5
            }
        }
    };
    if lag == 0 || lag > MAX_LAG {
        return Err(AlgoError::Settings(format!(
            "selected lag {lag} out of range 1..={MAX_LAG}"
        )));
    }
    let max_params = settings
        .max_predictors
        .checked_mul(lag)
        .ok_or_else(|| AlgoError::Settings("parameter count overflow".into()))?;
    if max_params > MAX_PARAMETERS {
        return Err(AlgoError::Settings(format!(
            "max_predictors * lag ({max_params}) exceeds limit {MAX_PARAMETERS}"
        )));
    }
    if m <= lag {
        return Err(AlgoError::Data(format!(
            "not enough time points ({m}) for lag {lag}"
        )));
    }
    Ok(lag)
}

fn check_sample_size(m: usize, lag: usize, k_eff: usize) -> Result<(), TcmError> {
    let min_required = lag
        .saturating_add(k_eff.saturating_mul(lag))
        .saturating_add(1);
    if m <= min_required {
        return Err(AlgoError::Data(format!(
            "insufficient time points m={m} for lag={lag} and k={k_eff}: requires m > L + K*L + 1 = {min_required}"
        )));
    }
    Ok(())
}

fn prepare_series(
    data: &TcmData,
    settings: &Settings,
    m: usize,
) -> Result<Vec<Vec<f64>>, TcmError> {
    let mut prepared = Vec::with_capacity(data.series.len());
    for s in &data.series {
        let has_missing = s.values.iter().any(Option::is_none);
        if !has_missing {
            let clean: Vec<f64> = s.values.iter().filter_map(|&v| v).collect();
            prepared.push(clean);
            continue;
        }
        if !settings.series_transformation || settings.missing_imputation == MissingImputation::None
        {
            return Err(AlgoError::Data(format!(
                "series '{}' contains missing values while imputation is disabled",
                s.name
            )));
        }
        // LinearInterp: interior linear interpolation + boundary linear extrapolation.
        let valid: Vec<(usize, f64)> = s
            .values
            .iter()
            .enumerate()
            .filter_map(|(t, v)| v.map(|x| (t, x)))
            .collect();
        let first = valid
            .first()
            .copied()
            .ok_or_else(|| AlgoError::Data(format!("series '{}' has no valid points", s.name)))?;
        let mut out = Vec::with_capacity(m);
        if valid.len() == 1 {
            out.resize(m, first.1);
            prepared.push(out);
            continue;
        }
        let second = *valid.get(1).ok_or_else(|| {
            AlgoError::Data(format!("series '{}' has insufficient valid points", s.name))
        })?;
        let last = *valid
            .last()
            .ok_or_else(|| AlgoError::Data(format!("series '{}' has no valid points", s.name)))?;
        let prev_last = *valid.get(valid.len() - 2).ok_or_else(|| {
            AlgoError::Data(format!("series '{}' has insufficient valid points", s.name))
        })?;

        let mut v_cursor = 0usize;
        for (t, opt) in s.values.iter().enumerate() {
            if let Some(val) = opt {
                out.push(*val);
                continue;
            }
            let imputed = if t < first.0 {
                let dt = (second.0 - first.0) as f64;
                let slope = (second.1 - first.1) / dt;
                first.1 - (first.0 - t) as f64 * slope
            } else if t > last.0 {
                let dt = (last.0 - prev_last.0) as f64;
                let slope = (last.1 - prev_last.1) / dt;
                last.1 + (t - last.0) as f64 * slope
            } else {
                while let Some(&(next_t, _)) = valid.get(v_cursor + 1) {
                    if next_t < t {
                        v_cursor += 1;
                    } else {
                        break;
                    }
                }
                let (t_left, x_left) = *valid
                    .get(v_cursor)
                    .ok_or_else(|| AlgoError::Data("interpolation cursor out of range".into()))?;
                let (t_right, x_right) = *valid
                    .get(v_cursor + 1)
                    .ok_or_else(|| AlgoError::Data("interpolation cursor out of range".into()))?;
                let span = (t_right - t_left) as f64;
                let frac = (t - t_left) as f64 / span;
                x_left + frac * (x_right - x_left)
            };
            if !imputed.is_finite() {
                return Err(AlgoError::Data(format!(
                    "imputed value in series '{}' at index {t} is non-finite",
                    s.name
                )));
            }
            out.push(imputed);
        }
        prepared.push(out);
    }
    Ok(prepared)
}

/// Precomputed lagged representation for a single series.
struct SeriesBlock {
    /// Column-major raw lagged columns: `raw_cols[lag_idx][obs]`, `lag_idx = 0..L` (lag 1..=L).
    raw_cols: Vec<Vec<f64>>,
    /// Per-column mean `M_j` over `n_obs` regression rows.
    means: Vec<f64>,
    /// Per-column sample standard deviation `S_j` over `n_obs` regression rows (`0.0` if constant).
    stds: Vec<f64>,
    /// Column-major standardized lagged columns: `std_cols[lag_idx][obs]`.
    std_cols: Vec<Vec<f64>>,
    /// SWEEP generalized inverse `(X_{G_j}^{*T} X_{G_j}^*)^-` (flat `L x L`), used by `argmin`.
    gram_ginv: Vec<f64>,
}

fn precompute_blocks(
    prepared: &[Vec<f64>],
    lag: usize,
    n_obs: usize,
) -> Result<Vec<SeriesBlock>, TcmError> {
    let denom = if n_obs > 1 { (n_obs - 1) as f64 } else { 1.0 };
    let mut blocks = Vec::with_capacity(prepared.len());
    for series in prepared {
        let mut raw_cols = Vec::with_capacity(lag);
        let mut means = Vec::with_capacity(lag);
        let mut stds = Vec::with_capacity(lag);
        let mut std_cols = Vec::with_capacity(lag);
        for l in 1..=lag {
            let start = lag
                .checked_sub(l)
                .ok_or_else(|| AlgoError::Data("lag window underflow".into()))?;
            let end = start
                .checked_add(n_obs)
                .ok_or_else(|| AlgoError::Data("lag window overflow".into()))?;
            let slice = series
                .get(start..end)
                .ok_or_else(|| AlgoError::Data("lag slice out of range".into()))?;
            let raw = slice.to_vec();
            let mean = raw.iter().sum::<f64>() / (n_obs as f64);
            let ss = raw.iter().map(|&v| (v - mean) * (v - mean)).sum::<f64>();
            let sd = (ss / denom).sqrt();
            let std_col = if sd > CONST_STD_TOL {
                raw.iter().map(|&v| (v - mean) / sd).collect()
            } else {
                vec![0.0; n_obs]
            };
            raw_cols.push(raw);
            means.push(mean);
            stds.push(if sd > CONST_STD_TOL { sd } else { 0.0 });
            std_cols.push(std_col);
        }
        // Build L x L Gram matrix and its SWEEP generalized inverse for Algorithm 2.
        let mut gram = vec![0.0; lag * lag];
        for i in 0..lag {
            let ci = std_cols
                .get(i)
                .ok_or_else(|| AlgoError::Data("block col missing".into()))?;
            for j in i..lag {
                let cj = std_cols
                    .get(j)
                    .ok_or_else(|| AlgoError::Data("block col missing".into()))?;
                let dot: f64 = ci.iter().zip(cj.iter()).map(|(&a, &b)| a * b).sum();
                if let Some(cell) = gram.get_mut(i * lag + j) {
                    *cell = dot;
                }
                if let Some(cell) = gram.get_mut(j * lag + i) {
                    *cell = dot;
                }
            }
        }
        let (gram_ginv, _) = sweep_ginv(&gram, lag);
        blocks.push(SeriesBlock {
            raw_cols,
            means,
            stds,
            std_cols,
            gram_ginv,
        });
    }
    Ok(blocks)
}

fn build_forbidden_mask(data: &TcmData, target_idx: usize, n_series: usize) -> Vec<bool> {
    let mut forbidden = vec![false; n_series];
    for (idx, s) in data.series.iter().enumerate() {
        if s.role == SeriesRole::TargetOnly
            && idx != target_idx
            && let Some(slot) = forbidden.get_mut(idx)
        {
            *slot = true;
        }
    }
    for &(t_idx, p_idx) in &data.forbidden_inputs {
        if t_idx == target_idx
            && p_idx != target_idx
            && let Some(slot) = forbidden.get_mut(p_idx)
        {
            *slot = true;
        }
    }
    forbidden
}

fn build_initial_predictors(data: &TcmData, target_idx: usize, forbidden: &[bool]) -> Vec<usize> {
    let mut j_init = vec![target_idx];
    for &f_idx in &data.forced_inputs {
        if f_idx != target_idx
            && !forbidden.get(f_idx).copied().unwrap_or(true)
            && !j_init.contains(&f_idx)
        {
            j_init.push(f_idx);
        }
    }
    j_init
}

struct RegressionFit {
    /// Standardized coefficients `beta_hat*` of length `p = K * L`.
    beta_star: Vec<f64>,
    /// Reflexive generalized inverse `(X_G*^T X_G*)^-` (flat `p x p`).
    xtx_ginv: Vec<f64>,
    /// Active (non-redundant) flags per column of length `p`.
    active: Vec<bool>,
    /// Residual vector `r = y_c - X_G* beta_hat*` of length `n_obs`.
    residual: Vec<f64>,
}

/// Solve standardized centered regression `min ||y_c - X_G* beta*||_2^2` via SWEEP.
fn solve_standardized(
    blocks: &[SeriesBlock],
    selected: &[usize],
    yc: &[f64],
    lag: usize,
    n_obs: usize,
) -> Result<RegressionFit, TcmError> {
    let p = selected
        .len()
        .checked_mul(lag)
        .ok_or_else(|| AlgoError::Data("parameter count overflow".into()))?;
    let mut cols: Vec<&[f64]> = Vec::with_capacity(p);
    for &sid in selected {
        let blk = blocks
            .get(sid)
            .ok_or_else(|| AlgoError::Data(format!("series block {sid} missing")))?;
        for col in &blk.std_cols {
            cols.push(col.as_slice());
        }
    }

    let mut gram = vec![0.0; p * p];
    let mut rhs = vec![0.0; p];
    for i in 0..p {
        let ci = *cols
            .get(i)
            .ok_or_else(|| AlgoError::Data("column index out of range".into()))?;
        let dot_y: f64 = ci.iter().zip(yc.iter()).map(|(&a, &b)| a * b).sum();
        if let Some(slot) = rhs.get_mut(i) {
            *slot = dot_y;
        }
        for j in i..p {
            let cj = *cols
                .get(j)
                .ok_or_else(|| AlgoError::Data("column index out of range".into()))?;
            let dot: f64 = ci.iter().zip(cj.iter()).map(|(&a, &b)| a * b).sum();
            if let Some(slot) = gram.get_mut(i * p + j) {
                *slot = dot;
            }
            if let Some(slot) = gram.get_mut(j * p + i) {
                *slot = dot;
            }
        }
    }

    let (xtx_ginv, active) = sweep_ginv(&gram, p);
    let beta_star = mat_vec(&xtx_ginv, p, &rhs);

    let mut residual = yc.to_vec();
    for (j, (&b, col)) in beta_star.iter().zip(cols.iter()).enumerate() {
        if b == 0.0 || !active.get(j).copied().unwrap_or(false) {
            continue;
        }
        for (r_i, &x_ij) in residual.iter_mut().zip(col.iter()) {
            *r_i -= b * x_ij;
        }
    }
    if residual.len() != n_obs {
        return Err(AlgoError::Data("residual length mismatch".into()));
    }

    Ok(RegressionFit {
        beta_star,
        xtx_ginv,
        active,
        residual,
    })
}

/// Algorithm 1 (GOMP) and Algorithm 2 (`argmin`), Guide pp. 672-674.
#[allow(clippy::too_many_arguments)]
fn run_gomp(
    prepared: &[Vec<f64>],
    blocks: &[SeriesBlock],
    target_idx: usize,
    j_init: &[usize],
    forbidden: &[bool],
    lag: usize,
    n_obs: usize,
    settings: &Settings,
) -> Result<Vec<usize>, TcmError> {
    let target_series = prepared
        .get(target_idx)
        .ok_or_else(|| AlgoError::Data(format!("target index {target_idx} out of range")))?;
    let y = target_series
        .get(lag..lag + n_obs)
        .ok_or_else(|| AlgoError::Data("target lag slice out of range".into()))?;
    let y_mean = y.iter().sum::<f64>() / (n_obs as f64);
    let yc: Vec<f64> = y.iter().map(|&v| v - y_mean).collect();

    // Lines 1-4: initial fit on J_sel^0.
    let mut j_sel = j_init.to_vec();
    let mut fit = solve_standardized(blocks, &j_sel, &yc, lag, n_obs)?;

    // Line 5: remove any completely redundant non-target series from J_sel^0.
    if j_sel.len() > 1 {
        let mut kept = Vec::with_capacity(j_sel.len());
        for (k, &sid) in j_sel.iter().enumerate() {
            let start = k * lag;
            let end = start + lag;
            let any_active = fit
                .active
                .get(start..end)
                .map(|sl| sl.iter().any(|&a| a))
                .unwrap_or(false);
            if sid == target_idx || any_active {
                kept.push(sid);
            }
        }
        let changed = kept.len() != j_sel.len();
        j_sel = kept;
        // Line 6: if |J_sel^0| >= K*, keep first K* and stop.
        if j_sel.len() >= settings.max_predictors {
            j_sel.truncate(settings.max_predictors);
            return Ok(j_sel);
        }
        // Line 7: update beta*^0 and r^0 if redundant series were deleted.
        if changed {
            fit = solve_standardized(blocks, &j_sel, &yc, lag, n_obs)?;
        }
    } else if j_sel.len() >= settings.max_predictors {
        return Ok(j_sel);
    }

    // Early stop if initial residual norm is already within epsilon.
    let mut r_norm2: f64 = fit.residual.iter().map(|&v| v * v).sum();
    if r_norm2.sqrt() <= settings.epsilon {
        return Ok(j_sel);
    }

    // Line 8: iteratively add up to (K* - |J_sel^0|) predictors.
    let max_iters = settings.max_predictors.saturating_sub(j_sel.len());
    for _ in 0..max_iters {
        crate::check_cancelled()?;
        // Line 9: Algorithm 2 (argmin).
        let next_j = argmin_candidate(
            blocks,
            &fit.residual,
            r_norm2,
            &j_sel,
            forbidden,
            lag,
            settings.epsilon2,
        )?;
        // Line 10: if no candidate improves cost by more than epsilon2, stop.
        let Some(chosen) = next_j else {
            break;
        };
        // Lines 11-16: add chosen predictor and re-estimate.
        j_sel.push(chosen);
        fit = solve_standardized(blocks, &j_sel, &yc, lag, n_obs)?;
        r_norm2 = fit.residual.iter().map(|&v| v * v).sum();
        // Line 17: stopping criterion ||r^k||_2 <= epsilon.
        if r_norm2.sqrt() <= settings.epsilon {
            break;
        }
    }

    Ok(j_sel)
}

/// Algorithm 2: `argmin` over all eligible candidate series (Guide pp. 673-674).
fn argmin_candidate(
    blocks: &[SeriesBlock],
    residual: &[f64],
    initial_cost: f64,
    j_sel: &[usize],
    forbidden: &[bool],
    lag: usize,
    epsilon2: f64,
) -> Result<Option<usize>, TcmError> {
    let mut best_cost = initial_cost;
    let mut best_j: Option<usize> = None;
    let mut rhs = vec![0.0; lag];

    for (j, blk) in blocks.iter().enumerate() {
        if j_sel.contains(&j) || forbidden.get(j).copied().unwrap_or(true) {
            continue;
        }
        // Compute rhs = X_{G_j}^{*T} r
        for (l, slot) in rhs.iter_mut().enumerate() {
            let col = blk
                .std_cols
                .get(l)
                .ok_or_else(|| AlgoError::Data("candidate block col missing".into()))?;
            *slot = col.iter().zip(residual.iter()).map(|(&a, &b)| a * b).sum();
        }
        // beta_j = (X_{G_j}^{*T} X_{G_j}^*)^- rhs
        let beta_j = mat_vec(&blk.gram_ginv, lag, &rhs);
        // Compute ||r - X_{G_j}^* beta_j||_2^2 directly (numerically stable even when small).
        let mut sse_j = 0.0;
        for (i, &r_i) in residual.iter().enumerate() {
            let mut pred_i = 0.0;
            for (l, &b_l) in beta_j.iter().enumerate() {
                if b_l != 0.0
                    && let Some(col) = blk.std_cols.get(l)
                    && let Some(&x_il) = col.get(i)
                {
                    pred_i += b_l * x_il;
                }
            }
            let diff = r_i - pred_i;
            sse_j += diff * diff;
        }
        if sse_j < best_cost - epsilon2 {
            best_cost = sse_j;
            best_j = Some(j);
        }
    }
    Ok(best_j)
}

/// Post-estimation step (Guide Section 2.6): unstandardize coefficients, compute standard errors,
/// t-statistics, p-values, confidence intervals, Type III group F-tests, and quality diagnostics.
#[allow(clippy::too_many_arguments)]
fn build_series_model(
    data: &TcmData,
    prepared: &[Vec<f64>],
    blocks: &[SeriesBlock],
    target_idx: usize,
    selected: &[usize],
    algorithm: SeriesAlgorithm,
    lag: usize,
    n_obs: usize,
    confidence_level: f64,
) -> Result<SeriesModel, TcmError> {
    let target_name = data
        .series
        .get(target_idx)
        .map(|s| s.name.clone())
        .ok_or_else(|| AlgoError::Data(format!("target index {target_idx} out of range")))?;
    let target_series = prepared
        .get(target_idx)
        .ok_or_else(|| AlgoError::Data(format!("target index {target_idx} out of range")))?;
    let y = target_series
        .get(lag..lag + n_obs)
        .ok_or_else(|| AlgoError::Data("target slice out of range".into()))?;
    let y_mean = y.iter().sum::<f64>() / (n_obs as f64);
    let yc: Vec<f64> = y.iter().map(|&v| v - y_mean).collect();
    let sst: f64 = yc.iter().map(|&v| v * v).sum();

    let fit = solve_standardized(blocks, selected, &yc, lag, n_obs)?;
    let p = selected.len() * lag;
    let p_c = fit.active.iter().filter(|&&a| a).count();
    let df_e = n_obs.saturating_sub(p_c + 1);
    if df_e == 0 {
        return Err(AlgoError::Data(format!(
            "zero error degrees of freedom for target '{target_name}' (n_obs={n_obs}, p_c={p_c})"
        )));
    }

    // Collect column means and stds across selected predictors.
    let mut means = Vec::with_capacity(p);
    let mut stds = Vec::with_capacity(p);
    for &sid in selected {
        let blk = blocks
            .get(sid)
            .ok_or_else(|| AlgoError::Data(format!("block {sid} missing")))?;
        means.extend_from_slice(&blk.means);
        stds.extend_from_slice(&blk.stds);
    }

    // Equations (3) & (4): unstandardized coefficients & intercept.
    let mut beta = vec![0.0; p];
    let mut b0 = y_mean;
    for j in 0..p {
        let sd = stds.get(j).copied().unwrap_or(0.0);
        let mu = means.get(j).copied().unwrap_or(0.0);
        let bs = fit.beta_star.get(j).copied().unwrap_or(0.0);
        let act = fit.active.get(j).copied().unwrap_or(false);
        if act && sd > 0.0 {
            let bj = bs / sd;
            if let Some(slot) = beta.get_mut(j) {
                *slot = bj;
            }
            b0 -= mu * bj;
        }
    }

    // Recompute predictions and residuals on the original scale for exact SSE / RMSPE.
    let mut sse = 0.0;
    let mut sum_pct_sq = 0.0;
    let mut any_zero_y = false;
    for (i, &yi) in y.iter().enumerate() {
        let mut yhat = b0;
        for (k, &sid) in selected.iter().enumerate() {
            let blk = blocks
                .get(sid)
                .ok_or_else(|| AlgoError::Data(format!("block {sid} missing")))?;
            for l in 0..lag {
                let bj = beta.get(k * lag + l).copied().unwrap_or(0.0);
                if bj != 0.0
                    && let Some(col) = blk.raw_cols.get(l)
                    && let Some(&x_val) = col.get(i)
                {
                    yhat += bj * x_val;
                }
            }
        }
        let err = yi - yhat;
        sse += err * err;
        if yi == 0.0 {
            any_zero_y = true;
        } else {
            let rel = err / yi;
            sum_pct_sq += rel * rel;
        }
    }

    let mse = sse / (df_e as f64);
    let rmse = mse.sqrt();

    // Sigma* = mse * (X_G*^T X_G*)^-
    let sigma_star: Vec<f64> = fit.xtx_ginv.iter().map(|&v| mse * v).collect();

    // Critical t value for 100(1 - alpha)% confidence intervals.
    let alpha = 1.0 - confidence_level;
    let t_crit = student_t_quantile(1.0 - 0.5 * alpha, df_e as f64).unwrap_or(0.0);

    // Intercept SE: sqrt(mse / n_obs + (M/S)^T Sigma* (M/S)) (Decision D).
    let ms: Vec<f64> = (0..p)
        .map(|j| {
            let sd = stds.get(j).copied().unwrap_or(0.0);
            let mu = means.get(j).copied().unwrap_or(0.0);
            let act = fit.active.get(j).copied().unwrap_or(false);
            if act && sd > 0.0 { mu / sd } else { 0.0 }
        })
        .collect();
    let sigma_ms = mat_vec(&sigma_star, p, &ms);
    let quad_b0: f64 = ms.iter().zip(sigma_ms.iter()).map(|(&a, &b)| a * b).sum();
    let var_b0 = (quad_b0 + mse / (n_obs as f64)).max(0.0);
    let se_b0 = var_b0.sqrt();
    let (t_b0, sig_b0) = t_stat_and_sig(b0, se_b0, df_e);
    let intercept = InterceptStats {
        value: b0,
        std_error: se_b0,
        t_stat: t_b0,
        sig: sig_b0,
        conf_lower: b0 - se_b0 * t_crit,
        conf_upper: b0 + se_b0 * t_crit,
    };

    // Per-predictor lag statistics and Type III group F-tests (Guide Equations 5, 7-11).
    let mut predictors = Vec::with_capacity(selected.len());
    for (k, &sid) in selected.iter().enumerate() {
        let p_name = data
            .series
            .get(sid)
            .map(|s| s.name.clone())
            .ok_or_else(|| AlgoError::Data(format!("predictor index {sid} out of range")))?;
        let mut coefs = Vec::with_capacity(lag);
        let mut ses = Vec::with_capacity(lag);
        let mut t_stats = Vec::with_capacity(lag);
        let mut sigs = Vec::with_capacity(lag);
        let mut ci_lo = Vec::with_capacity(lag);
        let mut ci_hi = Vec::with_capacity(lag);

        let base = k * lag;
        for l in 0..lag {
            let j = base + l;
            let bj = beta.get(j).copied().unwrap_or(0.0);
            let sd = stds.get(j).copied().unwrap_or(0.0);
            let act = fit.active.get(j).copied().unwrap_or(false);
            let diag_s = sigma_star.get(j * p + j).copied().unwrap_or(0.0).max(0.0);
            let se_j = if act && sd > 0.0 {
                diag_s.sqrt() / sd
            } else {
                0.0
            };
            let (tj, sig_j) = t_stat_and_sig(bj, se_j, df_e);
            coefs.push(bj);
            ses.push(se_j);
            t_stats.push(tj);
            sigs.push(sig_j);
            ci_lo.push(bj - se_j * t_crit);
            ci_hi.push(bj + se_j * t_crit);
        }

        // Group F-test on standardized block (invariant to column scaling S_j):
        // F_i = (beta*_i^T (Sigma*_{ii})^- beta*_i) / r_i
        let (f_stat, df1, f_sig) =
            compute_group_f(&fit.beta_star, &sigma_star, &fit.active, p, base, lag, df_e);

        predictors.push(PredictorModel {
            series_index: sid,
            series_name: p_name,
            coefficients: coefs,
            std_errors: ses,
            t_stats,
            sigs,
            conf_lower: ci_lo,
            conf_upper: ci_hi,
            f_stat,
            df1,
            df2: df_e,
            f_sig,
        });
    }

    // Model quality measures (Guide Section 2.6.3, Equations 12-15').
    let n_obs_f = n_obs as f64;
    let r_squared = if sst > 0.0 {
        Some(1.0 - sse / sst)
    } else {
        None
    };
    let (bic, aic) = if sse > 0.0 {
        let log_term = n_obs_f * (sse / n_obs_f).ln();
        let k_params = (p_c + 1) as f64;
        (
            Some(log_term + k_params * n_obs_f.ln()),
            Some(log_term + 2.0 * k_params),
        )
    } else {
        (None, None)
    };
    let rmspe = if !any_zero_y {
        Some((sum_pct_sq / n_obs_f).sqrt())
    } else {
        None
    };

    Ok(SeriesModel {
        target_index: target_idx,
        target_name,
        algorithm,
        max_lags: lag,
        intercept,
        predictors,
        diagnostics: ModelDiagnostics {
            rss: sse,
            df_e,
            non_redundant_params: p_c,
            mse,
            rmse,
            r_squared,
            bic,
            aic,
            rmspe,
        },
    })
}

fn t_stat_and_sig(estimate: f64, std_error: f64, df_e: usize) -> (f64, f64) {
    if std_error <= 0.0 || !std_error.is_finite() {
        return (0.0, 1.0);
    }
    let t = estimate / std_error;
    if !t.is_finite() {
        return (0.0, 1.0);
    }
    let df = df_e as f64;
    // Two-sided p-value: 2 * (1 - F_t(|t|)) = I_{df / (df + t^2)}(df / 2, 1/2).
    let sig = if t == 0.0 {
        1.0
    } else {
        let x = df / (df + t * t);
        beta_i(0.5 * df, 0.5, x).clamp(0.0, 1.0)
    };
    (t, sig)
}

fn compute_group_f(
    beta_star: &[f64],
    sigma_star: &[f64],
    active: &[bool],
    p: usize,
    base: usize,
    lag: usize,
    df_e: usize,
) -> (f64, usize, f64) {
    let active_indices: Vec<usize> = (base..base + lag)
        .filter(|&j| active.get(j).copied().unwrap_or(false))
        .collect();
    let d = active_indices.len();
    if d == 0 {
        return (0.0, 0, 1.0);
    }
    let mut sub_sig = vec![0.0; d * d];
    let mut sub_b = Vec::with_capacity(d);
    for (r, &jr) in active_indices.iter().enumerate() {
        sub_b.push(beta_star.get(jr).copied().unwrap_or(0.0));
        for (c, &jc) in active_indices.iter().enumerate() {
            let val = sigma_star.get(jr * p + jc).copied().unwrap_or(0.0);
            if let Some(slot) = sub_sig.get_mut(r * d + c) {
                *slot = val;
            }
        }
    }
    let (sub_inv, sub_act) = sweep_ginv(&sub_sig, d);
    let rank = sub_act.iter().filter(|&&a| a).count();
    if rank == 0 {
        return (0.0, 0, 1.0);
    }
    let inv_b = mat_vec(&sub_inv, d, &sub_b);
    let quad: f64 = sub_b.iter().zip(inv_b.iter()).map(|(&a, &b)| a * b).sum();
    let f_val = (quad / (rank as f64)).max(0.0);
    let sig = f_p(f_val, rank as f64, df_e as f64).clamp(0.0, 1.0);
    (f_val, rank, sig)
}

/// Symmetric SWEEP operator producing a reflexive generalized inverse `G^-` of a symmetric
/// positive semi-definite `n x n` matrix `G` stored flat in row-major order.
///
/// Returns `(ginv, active)` where `active[k]` is true iff column `k` was swept (non-redundant).
fn sweep_ginv(g: &[f64], n: usize) -> (Vec<f64>, Vec<bool>) {
    let mut w = g.to_vec();
    let mut d0 = Vec::with_capacity(n);
    for i in 0..n {
        d0.push(g.get(i * n + i).copied().unwrap_or(0.0));
    }
    let mut active = vec![false; n];
    let mut row_k = vec![0.0; n];

    for k in 0..n {
        let diag0 = d0.get(k).copied().unwrap_or(0.0);
        let piv = w.get(k * n + k).copied().unwrap_or(0.0);
        if diag0 <= 0.0 || piv <= SWEEP_TOL * diag0 || !piv.is_finite() {
            continue;
        }
        // Snapshot row k before in-place update.
        for (j, slot) in row_k.iter_mut().enumerate() {
            *slot = w.get(k * n + j).copied().unwrap_or(0.0);
        }
        let inv_piv = 1.0 / piv;
        for i in 0..n {
            if i == k {
                continue;
            }
            let wik = row_k.get(i).copied().unwrap_or(0.0);
            let factor = wik * inv_piv;
            for j in 0..n {
                if j == k {
                    continue;
                }
                let wkj = row_k.get(j).copied().unwrap_or(0.0);
                if let Some(cell) = w.get_mut(i * n + j) {
                    *cell -= factor * wkj;
                }
            }
        }
        for i in 0..n {
            if i == k {
                continue;
            }
            let val = row_k.get(i).copied().unwrap_or(0.0) * inv_piv;
            if let Some(cell) = w.get_mut(i * n + k) {
                *cell = val;
            }
            if let Some(cell) = w.get_mut(k * n + i) {
                *cell = val;
            }
        }
        if let Some(cell) = w.get_mut(k * n + k) {
            *cell = -inv_piv;
        }
        if let Some(slot) = active.get_mut(k) {
            *slot = true;
        }
    }

    let mut inv = vec![0.0; n * n];
    for i in 0..n {
        if !active.get(i).copied().unwrap_or(false) {
            continue;
        }
        for j in 0..n {
            if !active.get(j).copied().unwrap_or(false) {
                continue;
            }
            let v = -w.get(i * n + j).copied().unwrap_or(0.0);
            if let Some(slot) = inv.get_mut(i * n + j) {
                *slot = v;
            }
        }
    }
    (inv, active)
}

fn mat_vec(m: &[f64], n: usize, v: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let row = m.get(i * n..(i + 1) * n).unwrap_or(&[]);
        let s: f64 = row.iter().zip(v.iter()).map(|(&a, &b)| a * b).sum();
        out.push(s);
    }
    out
}

fn validate_history(
    history: &[&[f64]],
    expected_series: usize,
    lag: usize,
) -> Result<usize, TcmError> {
    if history.len() != expected_series {
        return Err(AlgoError::Data(format!(
            "history has {} series, expected {expected_series}",
            history.len()
        )));
    }
    let len = history.first().map(|s| s.len()).unwrap_or(0);
    if len < lag {
        return Err(AlgoError::Data(format!(
            "history length {len} is shorter than model lag {lag}"
        )));
    }
    if len > MAX_POINTS {
        return Err(AlgoError::Data(format!(
            "history length {len} exceeds limit {MAX_POINTS}"
        )));
    }
    for (idx, s) in history.iter().enumerate() {
        if s.len() != len {
            return Err(AlgoError::Data(format!(
                "history series {idx} has length {}, expected {len}",
                s.len()
            )));
        }
        if s.iter().any(|x| !x.is_finite()) {
            return Err(AlgoError::Data(format!(
                "history series {idx} contains non-finite value"
            )));
        }
    }
    Ok(len)
}

fn eval_one_step(model: &SeriesModel, history: &[&[f64]], len: usize) -> Result<f64, TcmError> {
    let mut y_hat = model.intercept.value;
    for pred in &model.predictors {
        let series = history.get(pred.series_index).ok_or_else(|| {
            AlgoError::Data(format!(
                "predictor index {} out of range",
                pred.series_index
            ))
        })?;
        for (lag_idx, &coef) in pred.coefficients.iter().enumerate() {
            let pos = len
                .checked_sub(lag_idx + 1)
                .ok_or_else(|| AlgoError::Data("history lag underflow".into()))?;
            let x_val = *series
                .get(pos)
                .ok_or_else(|| AlgoError::Data("history index out of range".into()))?;
            y_hat += coef * x_val;
        }
    }
    Ok(y_hat)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn some_vec(xs: &[f64]) -> Vec<Option<f64>> {
        xs.iter().copied().map(Some).collect()
    }

    /// Hand-derived known-answer test: exact AR(1) with known intercept and slope.
    ///
    /// Data: y = [1, 3, 4, 4.5, 4.75, 4.875, 4.9375, 4.96875] satisfies exact recurrence
    /// `y_t = 2.5 + 0.5 * y_{t-1}` for all t = 1..7 (`m = 8, L = 1, n_obs = 7, df_e = 5`).
    /// Therefore OLS recovers `beta_0 = 2.5`, `beta_{0,1} = 0.5`, `SSE = 0`, `R^2 = 1`,
    /// and one-step prediction after `4.96875` is `2.5 + 0.5 * 4.96875 = 4.984375`.
    #[test]
    fn tcm_known_answer_exact_ar1() {
        let mut y = vec![1.0];
        for _ in 0..7 {
            let prev = y.last().copied().unwrap_or(0.0);
            y.push(2.5 + 0.5 * prev);
        }
        let data = TcmData::new(vec![Series {
            name: "Y".into(),
            values: some_vec(&y),
            role: SeriesRole::Both,
        }]);
        let settings = Settings {
            max_lag: Some(1),
            max_predictors: 1,
            ..Settings::default()
        };
        let model = train(&data, &settings).expect("AR(1) should train");
        let sm = model.model_for(0).expect("target 0 model");
        assert!((sm.intercept.value - 2.5).abs() < 1e-12);
        assert_eq!(sm.predictors.len(), 1);
        assert!((sm.predictors[0].coefficients[0] - 0.5).abs() < 1e-12);
        assert!(sm.diagnostics.rss < 1e-24);
        assert_eq!(sm.diagnostics.df_e, 5);
        assert!((sm.diagnostics.r_squared.unwrap_or(0.0) - 1.0).abs() < 1e-14);

        let pred = model
            .predict_one_step(0, &[&y])
            .expect("one-step prediction");
        assert!((pred - 4.984375).abs() < 1e-12);

        // Two-step forecast: h=1 -> 4.984375, h=2 -> 2.5 + 0.5 * 4.984375 = 4.9921875.
        let fc = model.forecast(&[&y], 2).expect("2-step forecast");
        assert_eq!(fc.len(), 1);
        assert!((fc[0].values[0] - 4.984375).abs() < 1e-12);
        assert!((fc[0].values[1] - 4.9921875).abs() < 1e-12);
    }

    /// Hand-derived known-answer test with noise: OLS + SE + t + F on a tiny 1-lag AR(1).
    ///
    /// Let `y = [0, 2, 1, 3, 2, 4, 3, 5]` (`m = 8, L = 1, n_obs = 7`).
    /// Lag-1 vector `x = [0, 2, 1, 3, 2, 4, 3]`, target `y_reg = [2, 1, 3, 2, 4, 3, 5]`.
    /// Means: `x_bar = 15/7`, `y_bar = 20/7`.
    /// Centered sums:
    ///   `S_xx = sum (x_i - 15/7)^2 = (225 + 1 + 64 + 36 + 1 + 169 + 36) / 49 = 532/49 = 76/7`
    ///   `S_xy = sum (x_i - 15/7)(y_i - 20/7) = (90 + 13 - 8 - 36 - 8 + 13 + 90) / 49 = 154/49 = 22/7`
    ///   `S_yy = sum (y_i - 20/7)^2 = (36 + 169 + 1 + 36 + 64 + 1 + 225) / 49 = 532/49 = 76/7`
    /// Therefore:
    ///   `beta_1 = S_xy / S_xx = 22 / 76 = 11 / 38`
    ///   `beta_0 = 20/7 - (11/38)*(15/7) = (760 - 165) / 266 = 595 / 266 = 85 / 38`
    ///   `SSE = S_yy - beta_1 * S_xy = 76/7 - (11/38)*(22/7) = 2646 / 266 = 189 / 19`
    ///   `df_e = 7 - 1 - 1 = 5`, `MSE = 189 / 95`
    ///   `SE(beta_1) = sqrt(MSE / S_xx) = sqrt((189/95) / (76/7)) = sqrt(1323 / 7220)`
    ///   `SE(beta_0) = sqrt(MSE * (1/7 + (15/7)^2 / (76/7))) = sqrt(MSE * 301 / 532)`
    ///   `t_1 = beta_1 / SE(beta_1)`, `F_1 = t_1^2` (since `df1 = 1`).
    #[test]
    fn tcm_known_answer_noisy_ar1_inference() {
        let y = [0.0, 2.0, 1.0, 3.0, 2.0, 4.0, 3.0, 5.0];
        let data = TcmData::new(vec![Series {
            name: "Y".into(),
            values: some_vec(&y),
            role: SeriesRole::Both,
        }]);
        let settings = Settings {
            max_lag: Some(1),
            max_predictors: 1,
            ..Settings::default()
        };
        let model = train(&data, &settings).expect("noisy AR(1) should train");
        let sm = model.model_for(0).expect("target 0");

        let exp_b1 = 11.0_f64 / 38.0;
        let exp_b0 = 85.0_f64 / 38.0;
        let exp_sse = 189.0_f64 / 19.0;
        let exp_mse = 189.0_f64 / 95.0;
        let exp_se_b1 = (1323.0_f64 / 7220.0).sqrt();
        let exp_se_b0 = (exp_mse * (301.0 / 532.0)).sqrt();
        let exp_t1 = exp_b1 / exp_se_b1;
        let exp_f1 = exp_t1 * exp_t1;
        let exp_r2 = 1.0 - exp_sse / (76.0 / 7.0);

        assert!((sm.intercept.value - exp_b0).abs() < 1e-13);
        assert!((sm.intercept.std_error - exp_se_b0).abs() < 1e-13);
        assert!((sm.predictors[0].coefficients[0] - exp_b1).abs() < 1e-13);
        assert!((sm.predictors[0].std_errors[0] - exp_se_b1).abs() < 1e-13);
        assert!((sm.predictors[0].t_stats[0] - exp_t1).abs() < 1e-12);
        assert!((sm.predictors[0].f_stat - exp_f1).abs() < 1e-12);
        assert_eq!(sm.predictors[0].df1, 1);
        assert_eq!(sm.predictors[0].df2, 5);
        assert!((sm.diagnostics.rss - exp_sse).abs() < 1e-13);
        assert!((sm.diagnostics.mse - exp_mse).abs() < 1e-13);
        assert!((sm.diagnostics.r_squared.unwrap_or(0.0) - exp_r2).abs() < 1e-13);
    }

    /// Hand-derived causal discovery: `Y_t = 1.0 + 0.4 Y_{t-1} + 2.0 X_{t-1}`, while `Z` is
    /// an unrelated series. GOMP with `K* = 2, L = 1` must select `[Y, X]` and also build an
    /// `AR(1)` model for `InputOnly` series `X` (and NOT for unselected `InputOnly` series `Z`).
    #[test]
    fn tcm_known_answer_causal_selection_and_ar_only() {
        let m = 30;
        let mut x = Vec::with_capacity(m);
        let mut z = Vec::with_capacity(m);
        for t in 0..m {
            let tf = t as f64;
            x.push((0.7 * tf).sin() + 0.3 * (1.9 * tf).cos());
            z.push((2.3 * tf + 0.5).cos() - 0.4 * (0.31 * tf).sin());
        }
        let mut y = vec![0.5];
        for t in 1..m {
            y.push(1.0 + 0.4 * y[t - 1] + 2.0 * x[t - 1]);
        }
        let data = TcmData::new(vec![
            Series {
                name: "Y".into(),
                values: some_vec(&y),
                role: SeriesRole::Both,
            },
            Series {
                name: "X".into(),
                values: some_vec(&x),
                role: SeriesRole::InputOnly,
            },
            Series {
                name: "Z".into(),
                values: some_vec(&z),
                role: SeriesRole::InputOnly,
            },
        ]);
        let settings = Settings {
            max_lag: Some(1),
            max_predictors: 2,
            ..Settings::default()
        };
        let model = train(&data, &settings).expect("causal model should train");
        let ym = model.model_for(0).expect("Y model");
        assert_eq!(ym.algorithm, SeriesAlgorithm::Tcm);
        let pred_ids: Vec<usize> = ym.predictors.iter().map(|p| p.series_index).collect();
        assert_eq!(pred_ids, vec![0, 1]);
        assert!((ym.intercept.value - 1.0).abs() < 1e-11);
        assert!((ym.predictors[0].coefficients[0] - 0.4).abs() < 1e-11);
        assert!((ym.predictors[1].coefficients[0] - 2.0).abs() < 1e-11);

        // X was selected -> gets an AR(1) model; Z was not selected -> no model.
        let xm = model.model_for(1).expect("AR model for selected X");
        assert_eq!(xm.algorithm, SeriesAlgorithm::Ar);
        assert_eq!(xm.predictors.len(), 1);
        assert_eq!(xm.predictors[0].series_index, 1);
        assert!(model.model_for(2).is_none());

        // to_text() produces non-empty output containing series names.
        let text = model.to_text();
        assert!(text.contains("target=0 (Y)"));
        assert!(text.contains("target=1 (X)"));
    }

    /// Property test: affine rescaling of a predictor leaves predictions, SSE, R^2, and F-stat
    /// invariant while scaling its coefficients and SEs inversely.
    #[test]
    fn tcm_property_scale_invariance() {
        let m = 40;
        let x: Vec<f64> = (0..m)
            .map(|t| (0.4 * t as f64).sin() + 0.1 * t as f64)
            .collect();
        let mut y = vec![2.0];
        for t in 1..m {
            y.push(0.5 + 0.3 * y[t - 1] + 1.5 * x[t - 1] + 0.05 * ((t * 7) % 5) as f64);
        }
        let scale = 100.0;
        let shift = -50.0;
        let x_scaled: Vec<f64> = x.iter().map(|&v| scale * v + shift).collect();

        let d1 = TcmData::new(vec![
            Series {
                name: "Y".into(),
                values: some_vec(&y),
                role: SeriesRole::Both,
            },
            Series {
                name: "X".into(),
                values: some_vec(&x),
                role: SeriesRole::InputOnly,
            },
        ]);
        let d2 = TcmData::new(vec![
            Series {
                name: "Y".into(),
                values: some_vec(&y),
                role: SeriesRole::Both,
            },
            Series {
                name: "X".into(),
                values: some_vec(&x_scaled),
                role: SeriesRole::InputOnly,
            },
        ]);
        let settings = Settings {
            max_lag: Some(2),
            max_predictors: 2,
            ..Settings::default()
        };
        let m1 = train(&d1, &settings).expect("m1");
        let m2 = train(&d2, &settings).expect("m2");
        let s1 = m1.model_for(0).expect("s1");
        let s2 = m2.model_for(0).expect("s2");

        assert!((s1.diagnostics.rss - s2.diagnostics.rss).abs() < 1e-11);
        assert!((s1.predictors[1].f_stat - s2.predictors[1].f_stat).abs() < 1e-9);
        for l in 0..2 {
            assert!(
                (s1.predictors[1].coefficients[l] - scale * s2.predictors[1].coefficients[l]).abs()
                    < 1e-11
            );
            assert!(
                (s1.predictors[1].std_errors[l] - scale * s2.predictors[1].std_errors[l]).abs()
                    < 1e-11
            );
        }
        let p1 = m1.predict_one_step(0, &[&y, &x]).expect("p1");
        let p2 = m2.predict_one_step(0, &[&y, &x_scaled]).expect("p2");
        assert!((p1 - p2).abs() < 1e-11);
    }

    /// Never-crash test: empty data returns an error.
    #[test]
    fn tcm_never_crash_empty_data() {
        let empty = TcmData::new(Vec::new());
        assert!(matches!(
            train(&empty, &Settings::default()),
            Err(AlgoError::Data(_))
        ));

        let zero_len = TcmData::new(vec![Series {
            name: "A".into(),
            values: Vec::new(),
            role: SeriesRole::Both,
        }]);
        assert!(matches!(
            train(&zero_len, &Settings::default()),
            Err(AlgoError::Data(_))
        ));
    }

    /// Never-crash test: single series with default settings succeeds cleanly.
    #[test]
    fn tcm_never_crash_one_series() {
        let y: Vec<f64> = (0..25).map(|t| (0.5 * t as f64).sin() + 10.0).collect();
        let data = TcmData::new(vec![Series {
            name: "Only".into(),
            values: some_vec(&y),
            role: SeriesRole::Both,
        }]);
        let model = train(&data, &Settings::default()).expect("single series trains");
        assert_eq!(model.models.len(), 1);
        assert_eq!(model.models[0].predictors.len(), 1);
        assert!(
            model
                .predict_one_step(0, &[&y])
                .expect("predict")
                .is_finite()
        );
    }

    /// Never-crash test: constant series (both as target and as candidate predictor).
    #[test]
    fn tcm_never_crash_constant_series() {
        let c = vec![42.0; 25];
        let data = TcmData::new(vec![Series {
            name: "Const".into(),
            values: some_vec(&c),
            role: SeriesRole::Both,
        }]);
        let model = train(&data, &Settings::default()).expect("constant series should not panic");
        let sm = model.model_for(0).expect("const model");
        assert!((sm.intercept.value - 42.0).abs() < 1e-12);
        assert_eq!(sm.diagnostics.non_redundant_params, 0);
        assert_eq!(sm.diagnostics.rss, 0.0);
        assert!(sm.predictors[0].coefficients.iter().all(|&v| v == 0.0));
        let pred = model.predict_one_step(0, &[&c]).expect("predict const");
        assert!((pred - 42.0).abs() < 1e-12);
    }

    /// Never-crash test: fewer points than lags (and insufficient degrees of freedom).
    #[test]
    fn tcm_never_crash_fewer_points_than_lags() {
        let short = TcmData::new(vec![Series {
            name: "Short".into(),
            values: some_vec(&[1.0, 2.0, 3.0]),
            role: SeriesRole::Both,
        }]);
        assert!(matches!(
            train(&short, &Settings::default()),
            Err(AlgoError::Data(_))
        ));

        // Exactly m == 2*L + 1 = 11 for L=5 has df_e = 0 -> must error.
        let exact_zero_df = TcmData::new(vec![Series {
            name: "ExactZeroDf".into(),
            values: some_vec(&(0..11).map(|i| i as f64).collect::<Vec<_>>()),
            role: SeriesRole::Both,
        }]);
        assert!(matches!(
            train(&exact_zero_df, &Settings::default()),
            Err(AlgoError::Data(_))
        ));
    }

    /// Never-crash test: perfectly collinear series (both as candidates and as forced inputs).
    #[test]
    fn tcm_never_crash_perfectly_collinear() {
        let m = 35;
        let a: Vec<f64> = (0..m)
            .map(|t| (0.6 * t as f64).sin() + 0.2 * (1.7 * t as f64).cos())
            .collect();
        let b: Vec<f64> = a.iter().map(|&v| 3.0 * v + 7.0).collect();
        let mut y = vec![1.0];
        for t in 1..m {
            y.push(0.5 + 0.2 * y[t - 1] + 0.8 * a[t - 1] + 0.01 * ((t * 13) % 7) as f64);
        }
        // As candidate predictors: GOMP picks A (or B) and never adds its duplicate.
        let data = TcmData::new(vec![
            Series {
                name: "Y".into(),
                values: some_vec(&y),
                role: SeriesRole::Both,
            },
            Series {
                name: "A".into(),
                values: some_vec(&a),
                role: SeriesRole::InputOnly,
            },
            Series {
                name: "B".into(),
                values: some_vec(&b),
                role: SeriesRole::InputOnly,
            },
        ]);
        let settings = Settings {
            max_lag: Some(2),
            max_predictors: 3,
            ..Settings::default()
        };
        let model = train(&data, &settings).expect("collinear candidates should train");
        let ym = model.model_for(0).expect("Y model");
        assert_eq!(ym.predictors.len(), 2);

        // As forced inputs: redundant series B is deleted in Algorithm 1 line 5.
        let mut forced = data;
        forced.forced_inputs = vec![1, 2];
        let model2 = train(&forced, &settings).expect("collinear forced inputs should train");
        let ym2 = model2.model_for(0).expect("Y model");
        assert_eq!(ym2.predictors.len(), 2);
        assert!(ym2.intercept.value.is_finite());
    }

    /// Never-crash test: missing value in the middle is linearly interpolated.
    #[test]
    fn tcm_never_crash_missing_in_middle() {
        let mut y: Vec<Option<f64>> = (0..25).map(|t| Some(2.0 * t as f64 + 1.0)).collect();
        // True value at t=10 is 21.0; linear interpolation between 19.0 and 23.0 recovers 21.0!
        y[10] = None;
        let data = TcmData::new(vec![Series {
            name: "Linear".into(),
            values: y,
            role: SeriesRole::Both,
        }]);
        let settings = Settings {
            max_lag: Some(1),
            max_predictors: 1,
            ..Settings::default()
        };
        let model = train(&data, &settings).expect("missing in middle should train");
        let sm = model.model_for(0).expect("model 0");
        // y_t = y_{t-1} + 2.0 exactly!
        assert!((sm.intercept.value - 2.0).abs() < 1e-11);
        assert!((sm.predictors[0].coefficients[0] - 1.0).abs() < 1e-11);
        assert!(sm.diagnostics.rss < 1e-22);
    }

    /// Never-crash test: thousands of series hits the hard limit cleanly with an error.
    #[test]
    fn tcm_never_crash_thousands_of_series() {
        let series: Vec<Series> = (0..2_000)
            .map(|i| Series {
                name: format!("S{i}"),
                values: some_vec(&[1.0; 20]),
                role: SeriesRole::Both,
            })
            .collect();
        let data = TcmData::new(series);
        assert!(matches!(
            train(&data, &Settings::default()),
            Err(AlgoError::Data(_))
        ));
    }

    /// Verify explicit `GAP:` errors on out-of-scope / undefined settings.
    #[test]
    fn tcm_gap_settings_return_explicit_gap() {
        let data = TcmData::new(vec![Series {
            name: "A".into(),
            values: some_vec(&(0..25).map(|i| (i as f64).sin()).collect::<Vec<_>>()),
            role: SeriesRole::Both,
        }]);
        let cases = [
            Settings {
                outlier_detection: true,
                ..Settings::default()
            },
            Settings {
                root_cause_analysis: true,
                ..Settings::default()
            },
            Settings {
                scenario_analysis: true,
                ..Settings::default()
            },
            Settings {
                re_estimate: true,
                ..Settings::default()
            },
            Settings {
                estimation_period: true,
                ..Settings::default()
            },
            Settings {
                missing_imputation: MissingImputation::SeriesMean,
                ..Settings::default()
            },
            Settings {
                missing_imputation: MissingImputation::KMean(2),
                ..Settings::default()
            },
            Settings {
                missing_imputation: MissingImputation::KMedian(2),
                ..Settings::default()
            },
            Settings {
                missing_imputation: MissingImputation::LinearTrend,
                ..Settings::default()
            },
        ];
        for s in &cases {
            match train(&data, s) {
                Err(AlgoError::Unsupported(msg)) => assert!(msg.contains("GAP:"), "got: {msg}"),
                other => panic!("expected GAP Unsupported error, got {other:?}"),
            }
        }
    }

    /// Verify `TargetOnly`, `forced_inputs`, and `forbidden_inputs` constraints.
    #[test]
    fn tcm_roles_forced_and_forbidden_inputs() {
        let m = 30;
        let x: Vec<f64> = (0..m).map(|t| (0.7 * t as f64).sin()).collect();
        let z: Vec<f64> = (0..m).map(|t| (1.3 * t as f64).cos()).collect();
        let mut y = vec![0.5];
        for t in 1..m {
            y.push(1.0 + 0.3 * y[t - 1] + 2.0 * x[t - 1] + 0.1 * z[t - 1]);
        }

        // Making X TargetOnly prevents Y from selecting X.
        let data_to = TcmData::new(vec![
            Series {
                name: "Y".into(),
                values: some_vec(&y),
                role: SeriesRole::Both,
            },
            Series {
                name: "X".into(),
                values: some_vec(&x),
                role: SeriesRole::TargetOnly,
            },
            Series {
                name: "Z".into(),
                values: some_vec(&z),
                role: SeriesRole::InputOnly,
            },
        ]);
        let settings = Settings {
            max_lag: Some(1),
            max_predictors: 2,
            ..Settings::default()
        };
        let m_to = train(&data_to, &settings).expect("TargetOnly should train");
        let y_preds: Vec<usize> = m_to
            .model_for(0)
            .expect("Y")
            .predictors
            .iter()
            .map(|p| p.series_index)
            .collect();
        assert_eq!(y_preds, vec![0, 2]);

        // Forbidding (0, 1) explicitly also forces Y to pick Z instead of X.
        let mut data_forb = TcmData::new(vec![
            Series {
                name: "Y".into(),
                values: some_vec(&y),
                role: SeriesRole::Both,
            },
            Series {
                name: "X".into(),
                values: some_vec(&x),
                role: SeriesRole::InputOnly,
            },
            Series {
                name: "Z".into(),
                values: some_vec(&z),
                role: SeriesRole::InputOnly,
            },
        ]);
        data_forb.forbidden_inputs.push((0, 1));
        let m_forb = train(&data_forb, &settings).expect("forbidden should train");
        let y_preds2: Vec<usize> = m_forb
            .model_for(0)
            .expect("Y")
            .predictors
            .iter()
            .map(|p| p.series_index)
            .collect();
        assert_eq!(y_preds2, vec![0, 2]);

        // Forcing Z into J_sel^0 with K* = 2 stops immediately with [Y, Z].
        let mut data_forced = TcmData::new(vec![
            Series {
                name: "Y".into(),
                values: some_vec(&y),
                role: SeriesRole::Both,
            },
            Series {
                name: "X".into(),
                values: some_vec(&x),
                role: SeriesRole::InputOnly,
            },
            Series {
                name: "Z".into(),
                values: some_vec(&z),
                role: SeriesRole::InputOnly,
            },
        ]);
        data_forced.forced_inputs.push(2);
        let m_forced = train(&data_forced, &settings).expect("forced should train");
        let y_preds3: Vec<usize> = m_forced
            .model_for(0)
            .expect("Y")
            .predictors
            .iter()
            .map(|p| p.series_index)
            .collect();
        assert_eq!(y_preds3, vec![0, 2]);
    }

    /// Verify Guide Section 2.4 automatic lag selection from periodicity `s` and length `m`.
    #[test]
    fn tcm_auto_lag_selection() {
        let s1 = Settings {
            periodicity: 1,
            ..Settings::default()
        };
        assert_eq!(select_lag(&s1, 100).expect("s=1"), 5);

        let s12 = Settings {
            periodicity: 12,
            ..Settings::default()
        };
        assert_eq!(select_lag(&s12, 47).expect("m < 4s"), 5);
        assert_eq!(select_lag(&s12, 48).expect("m >= 4s"), 12);

        let s30 = Settings {
            periodicity: 30,
            ..Settings::default()
        };
        assert_eq!(select_lag(&s30, 120).expect("capped at 20"), 20);
    }

    /// Verify leading & trailing linear extrapolation and hostile input validation.
    #[test]
    fn tcm_boundary_extrapolation_and_hostile_inputs() {
        // Exact arithmetic progression y_t = 10 + 3*t with both ends missing.
        let mut vals: Vec<Option<f64>> = (0..20).map(|t| Some(10.0 + 3.0 * t as f64)).collect();
        vals[0] = None;
        vals[19] = None;
        let data = TcmData::new(vec![Series {
            name: "Lin".into(),
            values: vals,
            role: SeriesRole::Both,
        }]);
        let settings = Settings {
            max_lag: Some(1),
            max_predictors: 1,
            ..Settings::default()
        };
        let model = train(&data, &settings).expect("boundary extrap trains");
        let sm = model.model_for(0).expect("model 0");
        assert!((sm.intercept.value - 3.0).abs() < 1e-11);
        assert!((sm.predictors[0].coefficients[0] - 1.0).abs() < 1e-11);

        // Missing with MissingImputation::None returns error.
        let no_imp = Settings {
            missing_imputation: MissingImputation::None,
            ..settings.clone()
        };
        assert!(matches!(train(&data, &no_imp), Err(AlgoError::Data(_))));

        // Non-finite values return error.
        let bad_nan = TcmData::new(vec![Series {
            name: "Nan".into(),
            values: vec![Some(1.0), Some(f64::NAN), Some(3.0), Some(4.0), Some(5.0)],
            role: SeriesRole::Both,
        }]);
        assert!(matches!(
            train(&bad_nan, &settings),
            Err(AlgoError::Data(_))
        ));

        // Mismatched lengths return error.
        let mismatch = TcmData::new(vec![
            Series {
                name: "A".into(),
                values: some_vec(&[1.0; 20]),
                role: SeriesRole::Both,
            },
            Series {
                name: "B".into(),
                values: some_vec(&[1.0; 19]),
                role: SeriesRole::Both,
            },
        ]);
        assert!(matches!(
            train(&mismatch, &settings),
            Err(AlgoError::Data(_))
        ));

        // Invalid settings return Settings error.
        let bad_conf = Settings {
            confidence_level: 1.0,
            ..Settings::default()
        };
        assert!(matches!(
            train(&data, &bad_conf),
            Err(AlgoError::Settings(_))
        ));

        // Short history to predict_one_step returns Data error.
        assert!(matches!(
            model.predict_one_step(0, &[&[]]),
            Err(AlgoError::Data(_))
        ));
    }
}
