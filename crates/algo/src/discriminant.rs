//! Linear and quadratic discriminant analysis (IBM SPSS Modeler 19 Algorithms Guide,
//! "DISCRIMINANT Algorithms", pp. 117-128).
//!
//! Pure algorithm module: takes plain vectors ([`DiscriminantData`]) and [`Settings`], computes
//! group statistics, within-groups and total SSCP/covariance matrices, optional direct or stepwise
//! variable selection (Wilks' lambda, unexplained variance, Mahalanobis distance, smallest F, or
//! Rao's V), canonical discriminant functions via Cholesky + symmetric eigenanalysis, Fisher's
//! linear classification functions, optional Box's M test, classification (within-groups or
//! separate-groups covariance), optional leave-one-out cross-validation, and variable importance.
//!
//! # Decisions the Guide does not settle
//!
//! Verified against IBM SPSS Modeler's saved `telco_custcat_discriminant` outputs
//! (`target/agy-disc/model/saved_output_tables.html.txt`,
//! `target/agy-disc/model/saved_dictionary.spssml.txt`, and
//! `target/agy-disc/model/saved_discriminant_model.pmml.txt`, trained on
//! `target/agy-disc/data/telco.csv` with candidate predictors `region, age, marital, address,
//! income, ed, employ, retire, gender, reside` and target `custcat`):
//!
//! A. **Stepwise criteria mode and defaults.** In `build_node_settings.txt`,
//!    `_criteria = UseValue` means the F-to-enter / F-to-remove thresholds (defaults `3.84` and
//!    `2.71`, per `scripting_discriminantnode.txt` and HTML footnotes `b` and `c`) govern stepwise
//!    selection rather than `_probability_entry` / `_probability_removal` (`0.05` / `0.10`).
//!    Default maximum steps is `2 * p` (`20` for `p = 10`, HTML footnote `a`). Default tolerance
//!    limit is `0.001` (Guide p. 119). Reproduces the saved Modeler output: exact 3-step path
//!    (`ed`, `employ`, `reside`), stopping when `address` has `F_enter = 3.514 < 3.84`, matching
//!    every printed number in "Variables Entered/Removed", "Variables in the Analysis", and
//!    "Variables Not in the Analysis".
//! B. **Symmetric sweep operator and Wilks' lambda if removed.** Using Dempster's symmetric sweep
//!    (`W*_{11} = -W_{11}^{-1}`, Guide p. 120), the diagonal of swept variables is negative in
//!    both `W*` and `T*`, so `Lambda_if_removed(i) = Lambda * (w*_{ii} / t*_{ii})` (for `q > 1`;
//!    omitted when `q = 1`). Reproduces the saved Modeler output: all 6 rows of "Variables in the
//!    Analysis" (`ed: 0.951, 0.940`; `employ: 0.844, 0.834`; `reside: 0.807`) and all 34 rows of
//!    "Variables Not in the Analysis" (steps 0-3) match every printed digit.
//! C. **Canonical eigenvector sign convention.** Generalized eigenvectors of `(T - W)V = lambda WV`
//!    are determined up to `+/- 1`. Each function `k` is oriented so that the entry of the
//!    standardized canonical coefficient column `D[:, k]` with the largest absolute value is
//!    positive (ties broken by lowest variable index). Reproduces the saved Modeler output: all
//!    unstandardized coefficients (`B`) and constants match `saved_dictionary.spssml.txt` to
//!    `< 8e-14` with identical signs, all group centroids match `saved_dictionary.spssml.txt` to
//!    `< 1.4e-14`, and every entry of the Standardized Coefficients and Structure Matrix tables
//!    matches every printed digit and sign.
//! D. **Structure matrix row ordering.** Guide p. 123 defines `R = S^{-1} W V` extended with
//!    zeros for non-selected variables (excluding variables with `w_{ii} = 0`), while the saved
//!    HTML table orders rows by dominant canonical function (ascending) and descending absolute
//!    correlation within that function. Reproduces the saved Modeler output: all 10 candidate
//!    predictors (3 selected, 7 non-selected) appear in the exact row order and values of the
//!    saved Structure Matrix table.
//! E. **Selected predictor ordering.** After selection, retained predictors are ordered in their
//!    original upstream input order for canonical/Fisher coefficients (on `telco`, entry order and
//!    input order coincide: `ed, employ, reside`).
//! F. **Classification tie-breaking and within-groups covariance.** With `AllEqual` priors
//!    (`0.25`) and `WithinGroups` covariance (`D_j = I_m` in canonical space), classifying by
//!    highest `g_j = ln(p_j) - 0.5 * sum_k (f_k - fbar_{jk})^2` (ties broken by lowest group
//!    index) reproduces the saved Modeler output: all 16 cells of the 4x4 Classification Results
//!    confusion matrix (`125, 11, 61, 69; 49, 15, 58, 95; 102, 14, 112, 53; 40, 16, 37, 143`) and
//!    `39.5%` overall accuracy match identically.
//! G. **Variable importance (`MiningField@importance` in PMML).** The DISCRIMINANT Algorithms
//!    chapter (pp. 117-128) does not define variable importance, and `saved_output_tables.html.txt`
//!    does not contain an importance table; Modeler computes `MiningField@importance` via its
//!    separate variance-based sensitivity (Saltelli) post-processor whose exact sampling design is
//!    not in the DISCRIMINANT chapter. We implement deterministic empirical total-effect variance
//!    sensitivity `E_{X_{-i}}[Var_{X_i}(P(G | X))]` (exact empirical marginal when a predictor has
//!    `<= 64` distinct values, otherwise 64 equal-frequency quantile means, normalized to sum to
//!    `1`). On `telco_custcat_discriminant`, this produces `[0.7427137, 0.1955511, 0.0617352]`
//!    against the saved PMML `[0.7398977, 0.1980336, 0.06206868]` (max absolute discrepancy
//!    `2.82e-3`; NOT claimed as an exact reproduction).
//!
//! Follows the Algorithms Guide (unverified against a saved Modeler output with these options):
//!
//! 1. **Fisher's classification function sign typo (Guide p. 121).** Guide p. 121 writes
//!    `b_{ij} = (n - g) sum_l w*_{il} Xbar_{lj}` omitting the leading minus sign even though
//!    `W*_{11} = -W_{11}^{-1}` on p. 120 and the immediately preceding `V` and `D^2_{ab}` formulas
//!    on p. 121 include `-(n - g)`. We use `b_{ij} = -(n - g) sum_l w*_{il} Xbar_{lj} =
//!    (C^{-1} Xbar_j)_i` so that `a_j + b_j' x` yields the exact same decision boundaries as
//!    canonical within-groups Mahalanobis classification.
//! 2. **Leave-one-out Sherman-Morrison typos (Guide p. 127).** Guide p. 127 prints `Sigma_j^{-1}`
//!    and `(n_j - g)` inside the rank-1 update of `Sigma_{jk}^{-1}` for the pooled covariance
//!    matrix without case `(j, k)`. We use the exact Sherman-Morrison identity for pooled
//!    covariance `Sigma` with weight `f_{jk}`:
//!    `Sigma_{jk}^{-1} = ((n - g - f_{jk})/(n - g)) * (Sigma^{-1} + (f_{jk} n_j Sigma^{-1} d d'
//!    Sigma^{-1}) / ((n_j - f_{jk})(n - g) - f_{jk} n_j d' Sigma^{-1} d))` where `d = X_{jk} - M_j`,
//!    verified by unit test against brute-force refits with each case omitted.
//! 3. **Other stepwise criteria, probability thresholds, inclusion levels, separate-groups
//!    covariance, and Box's M.** Implemented from Guide pp. 119-126 (unverified against a Modeler
//!    oracle for those non-default modes). Stepwise tie-breaking prefers highest `F`-to-enter
//!    (lowest `F`-to-remove), then lowest predictor index, with a visited-subset cycle guard.
//! 4. **Explicit `GAP:` errors.**
//!    - [`MissingHandling::MeanSubstitution`]: scoring/training mean substitution is not specified
//!      in the DISCRIMINANT Algorithms chapter (Guide pp. 125, 127 specify listwise deletion and
//!      `$null$` on missing scoring inputs).
//!    - `Settings::adjusted_propensity == true`: requires a separate test/validation partition and
//!      is outside the DISCRIMINANT Algorithms chapter.
//!    - `Settings::leave_one_out == true` with [`CovarianceMatrix::SeparateGroups`]: Guide p. 127
//!      explicitly states cross-validation applies only to linear (within-groups) discriminant
//!      analysis, not quadratic.

use crate::AlgoError;
use crate::stats::{chi_square_p, f_p, jacobi_eigen};
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

/// Hard limit on the number of input records.
const MAX_RECORDS: usize = 1_000_000;
/// Hard limit on the number of candidate predictors.
const MAX_PREDICTORS: usize = 512;
/// Hard limit on the number of distinct target groups.
const MAX_GROUPS: usize = 500;
/// Hard limit on stepwise entry/removal steps.
const MAX_STEPS: usize = 2_000;
/// Hard limit on predictor matrix cells (`records * predictors`).
const MAX_MATRIX_CELLS: usize = 50_000_000;
/// Cholesky singularity threshold specified in Guide p. 124 (Box's M / separate-groups covariance).
const CHOLESKY_SINGULAR_TOL: f64 = 1e-11;
/// Pivot threshold for sweep and linear solves.
const PIVOT_TOL: f64 = 1e-15;
/// Posterior log-ratio cutoff specified in Guide p. 126 (`exp(g_j - max g) = 0` when `<= -46`).
const POSTERIOR_LOG_CUTOFF: f64 = -46.0;
/// Maximum unique values evaluated directly per predictor in variable importance before binning.
const IMPORTANCE_MAX_EXACT_UNIQUE: usize = 64;
/// Maximum training rows sampled deterministically for variable importance when `n` is very large.
const IMPORTANCE_MAX_ROWS: usize = 4_000;

fn gap(what: &str) -> AlgoError {
    AlgoError::Unsupported(format!("GAP: Discriminant analysis {what}"))
}

/// A candidate numeric predictor column.
#[derive(Debug, Clone, PartialEq)]
pub struct Predictor {
    /// Predictor field name (or display label).
    pub name: String,
    /// Per-record values (`None` = system-missing).
    pub values: Vec<Option<f64>>,
    /// Stepwise inclusion level (Guide p. 119; default `1`).
    ///
    /// - `0`: never entered (statistics and structure correlations are still computed).
    /// - `1`: standard stepwise entry and removal.
    /// - odd `>= 3`: stepwise entry by criterion (not eligible for removal at level 1).
    /// - even `>= 2`: forced entry in upstream order before lower inclusion levels.
    pub inclusion_level: u8,
}

impl Predictor {
    /// Creates a candidate predictor with default stepwise inclusion level `1`.
    pub fn new(name: impl Into<String>, values: Vec<Option<f64>>) -> Self {
        Self {
            name: name.into(),
            values,
            inclusion_level: 1,
        }
    }
}

/// Categorical target column represented as integer group codes with optional labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Target field name.
    pub name: String,
    /// Per-record group codes (`None` = missing).
    pub codes: Vec<Option<i64>>,
    /// Ordered `(code, label)` definitions. If empty, distinct sorted observed codes are used.
    /// If non-empty, observed codes not listed here are treated as out-of-range group codes.
    pub labels: Vec<(i64, String)>,
}

impl Target {
    /// Creates a categorical target with explicit `(code, label)` pairs.
    pub fn new(
        name: impl Into<String>,
        codes: Vec<Option<i64>>,
        labels: Vec<(i64, String)>,
    ) -> Self {
        Self {
            name: name.into(),
            codes,
            labels,
        }
    }
}

/// Plain-vector input data for discriminant analysis (no dependency on `Dataset`).
#[derive(Debug, Clone, PartialEq)]
pub struct DiscriminantData {
    /// Categorical target variable.
    pub target: Target,
    /// Candidate numeric predictors in upstream order.
    pub predictors: Vec<Predictor>,
    /// Optional per-record case weights (`f_{jk}` in Guide Table 14-1). `None` means unit weights.
    pub weights: Option<Vec<Option<f64>>>,
}

/// Variable entry method (`_method`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Enter all predictors directly in upstream order subject to tolerance (Guide p. 119).
    Enter,
    /// Stepwise entry and removal (Guide pp. 119-121).
    Stepwise,
}

/// Stepwise selection criterion (`_stepwise_method`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepwiseCriterion {
    /// Minimize overall Wilks' lambda `|W_11| / |T_11|` (Guide p. 120).
    WilksLambda,
    /// Minimize sum of unexplained variations `sum_{a<b} 4 / (4 + D^2_{ab})` (Guide p. 121).
    UnexplainedVariance,
    /// Maximize smallest pairwise squared Mahalanobis distance `D^2_{ab}` (Guide p. 121).
    MahalanobisDistance,
    /// Maximize smallest pairwise F ratio `F_{ab}` (Guide p. 121).
    SmallestF,
    /// Maximize Rao's V (Lawley-Hotelling trace, Guide p. 121).
    RaosV,
}

/// Stepwise entry and removal threshold rule (`_criteria`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StepwiseCriteria {
    /// Enter if `F_enter >= f_entry`; remove if `F_remove < f_removal` (Modeler defaults `3.84` / `2.71`).
    UseFValue {
        /// Minimum partial F to enter.
        f_entry: f64,
        /// Maximum partial F to remove.
        f_removal: f64,
    },
    /// Enter if `p(F_enter) <= p_entry`; remove if `p(F_remove) > p_removal` (defaults `0.05` / `0.10`).
    UseProbability {
        /// Maximum F-to-enter significance level to enter.
        p_entry: f64,
        /// Minimum F-to-remove significance level to remove.
        p_removal: f64,
    },
}

/// Group prior probabilities for classification (`_prior_probabilities`).
#[derive(Debug, Clone, PartialEq)]
pub enum PriorProbabilities {
    /// Equal prior `1 / g` for each non-empty group.
    AllEqual,
    /// Empirical group shares `n_j / n`.
    ComputeFromSizes,
    /// Explicit positive priors (one per active group, summing to `1.0`).
    Custom(Vec<f64>),
}

/// Covariance matrix used for classification (`_covariance_matrix`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CovarianceMatrix {
    /// Pooled within-groups covariance matrix (`I_m` in canonical space; linear discriminant).
    WithinGroups,
    /// Separate per-group covariance matrices (`D_j = B' C^{(j)} B`; quadratic discriminant).
    SeparateGroups,
}

/// Missing value handling mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingHandling {
    /// Exclude records with any missing target or candidate predictor (Guide p. 125).
    Listwise,
    /// Replace missing values with means (not defined in the DISCRIMINANT chapter; returns `GAP:`).
    MeanSubstitution,
}

/// Configuration settings for discriminant analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Direct entry or stepwise selection.
    pub method: Method,
    /// Stepwise optimization criterion (used when `method == Method::Stepwise`).
    pub stepwise_criterion: StepwiseCriterion,
    /// Stepwise entry/removal thresholds (F values or probabilities).
    pub stepwise_criteria: StepwiseCriteria,
    /// Minimum increase in Rao's V required for entry when `stepwise_criterion == RaosV`.
    pub v_to_enter: f64,
    /// Minimum tolerance required for any variable in the analysis (Guide p. 119; default `0.001`).
    pub tolerance: f64,
    /// Maximum number of stepwise steps (`None` defaults to `2 * p`, capped at `MAX_STEPS`).
    pub max_steps: Option<usize>,
    /// Prior probabilities for classification.
    pub prior_probabilities: PriorProbabilities,
    /// Within-groups (linear) or separate-groups (quadratic) covariance for classification.
    pub covariance_matrix: CovarianceMatrix,
    /// Missing value handling mode.
    pub missing_handling: MissingHandling,
    /// Whether to compute leave-one-out cross-validation classification (linear only, Guide p. 127).
    pub leave_one_out: bool,
    /// Whether to compute Box's M test of equality of group covariance matrices (Guide pp. 124-125).
    pub compute_box_m: bool,
    /// Whether to compute normalized predictor importance for the retained predictors.
    pub compute_variable_importance: bool,
    /// Adjusted propensity scoring (not in the DISCRIMINANT chapter; `true` returns `GAP:`).
    pub adjusted_propensity: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            method: Method::Enter,
            stepwise_criterion: StepwiseCriterion::WilksLambda,
            stepwise_criteria: StepwiseCriteria::UseFValue {
                f_entry: 3.84,
                f_removal: 2.71,
            },
            v_to_enter: 0.0,
            tolerance: 0.001,
            max_steps: None,
            prior_probabilities: PriorProbabilities::AllEqual,
            covariance_matrix: CovarianceMatrix::WithinGroups,
            missing_handling: MissingHandling::Listwise,
            leave_one_out: false,
            compute_box_m: false,
            compute_variable_importance: true,
            adjusted_propensity: false,
        }
    }
}

/// Summary of valid and excluded records ("Analysis Case Processing Summary").
#[derive(Debug, Clone, PartialEq)]
pub struct CaseProcessingSummary {
    /// Total records supplied.
    pub total_cases: usize,
    /// Valid records used in estimation.
    pub valid_cases: usize,
    /// Excluded solely due to missing or out-of-range group code.
    pub excluded_missing_group: usize,
    /// Excluded solely due to at least one missing candidate predictor.
    pub excluded_missing_predictor: usize,
    /// Excluded due to both missing/out-of-range group code and missing predictor.
    pub excluded_missing_both: usize,
    /// Excluded due to missing or zero case weight.
    pub excluded_invalid_weight: usize,
    /// Total excluded records.
    pub total_excluded: usize,
    /// Percentage of total records that are valid (`0..=100`).
    pub valid_percent: f64,
}

/// Metadata, case counts, and prior probability for one active target group.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupInfo {
    /// Group code from [`Target`].
    pub code: i64,
    /// Display label for the group.
    pub label: String,
    /// Unweighted count `m_j` of valid cases in this group.
    pub unweighted_n: usize,
    /// Sum of case weights `n_j` in this group.
    pub weighted_n: f64,
    /// Prior probability `p_j` used for classification.
    pub prior: f64,
}

/// Mean, standard deviation, and valid N for one predictor within a group or overall.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupVariableStats {
    /// Candidate predictor index (`0..p`).
    pub predictor_index: usize,
    /// Predictor name.
    pub name: String,
    /// Weighted mean.
    pub mean: f64,
    /// Weighted sample standard deviation (`sqrt(S^2)`; `None` if `n_j <= 1`).
    pub std_dev: Option<f64>,
    /// Unweighted valid case count.
    pub unweighted_n: usize,
    /// Weighted valid case count.
    pub weighted_n: f64,
}

/// Group-level and total descriptive statistics ("Group Statistics").
#[derive(Debug, Clone, PartialEq)]
pub struct GroupStatistics {
    /// Per-group predictor statistics (`g x p`).
    pub per_group: Vec<Vec<GroupVariableStats>>,
    /// Overall predictor statistics (`p`).
    pub total: Vec<GroupVariableStats>,
}

/// Univariate test of equality of group means for one predictor (Guide p. 118).
#[derive(Debug, Clone, PartialEq)]
pub struct UnivariateAnova {
    /// Candidate predictor index (`0..p`).
    pub predictor_index: usize,
    /// Predictor name.
    pub name: String,
    /// Univariate Wilks' lambda `w_{ii} / t_{ii}`.
    pub wilks_lambda: f64,
    /// Univariate F statistic with `(df1, df2)` degrees of freedom.
    pub f_statistic: f64,
    /// Numerator degrees of freedom `g - 1`.
    pub df1: usize,
    /// Denominator degrees of freedom `n - g`.
    pub df2: f64,
    /// Significance level `P(F > f_statistic)`.
    pub sig: f64,
}

/// Action taken at one stepwise step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepAction {
    /// Predictor index entered at this step.
    Entered(usize),
    /// Predictor index removed at this step.
    Removed(usize),
}

/// Row of the "Variables in the Analysis" table at a given step.
#[derive(Debug, Clone, PartialEq)]
pub struct VariableInAnalysis {
    /// Candidate predictor index.
    pub predictor_index: usize,
    /// Predictor name.
    pub name: String,
    /// Tolerance `-1 / (w*_{ii} w_{ii})`.
    pub tolerance: f64,
    /// Partial F-to-remove.
    pub f_to_remove: f64,
    /// Wilks' lambda if this variable were removed (`None` when only 1 variable is in).
    pub wilks_lambda_if_removed: Option<f64>,
}

/// Row of the "Variables Not in the Analysis" table at a given step.
#[derive(Debug, Clone, PartialEq)]
pub struct VariableNotInAnalysis {
    /// Candidate predictor index.
    pub predictor_index: usize,
    /// Predictor name.
    pub name: String,
    /// Tolerance `w*_{ii} / w_{ii}`.
    pub tolerance: f64,
    /// Minimum tolerance across all variables if this predictor were entered.
    pub min_tolerance: f64,
    /// Partial F-to-enter (`0.0` if tolerance fails).
    pub f_to_enter: f64,
    /// Overall Wilks' lambda if this predictor were entered (`None` if tolerance fails).
    pub wilks_lambda_if_entered: Option<f64>,
}

/// Summary of one stepwise step ("Variables Entered/Removed" and "Wilks' Lambda").
#[derive(Debug, Clone, PartialEq)]
pub struct StepwiseStep {
    /// 1-based step number.
    pub step: usize,
    /// Entered or removed variable.
    pub action: StepAction,
    /// Number of variables `q` in the model after this step.
    pub num_variables: usize,
    /// Overall Wilks' lambda after this step.
    pub wilks_lambda: f64,
    /// Wilks' lambda `df1 = q`.
    pub lambda_df1: usize,
    /// Wilks' lambda `df2 = g - 1`.
    pub lambda_df2: usize,
    /// Wilks' lambda `df3 = n - g`.
    pub lambda_df3: f64,
    /// Exact or approximate F statistic (Rao's R, Guide p. 121).
    pub f_statistic: f64,
    /// F numerator degrees of freedom `q(g - 1)`.
    pub f_df1: f64,
    /// F denominator degrees of freedom `r/s + 1 - qh/2`.
    pub f_df2: f64,
    /// Significance level of `f_statistic`.
    pub f_sig: f64,
    /// `true` when `q <= 2` or `g - 1 <= 2` (exact F distribution, Guide p. 121).
    pub f_is_exact: bool,
    /// Value of the active [`StepwiseCriterion`] after this step.
    pub criterion_value: f64,
    /// Snapshot of "Variables in the Analysis" after this step.
    pub variables_in: Vec<VariableInAnalysis>,
    /// Snapshot of "Variables Not in the Analysis" after this step.
    pub variables_not_in: Vec<VariableNotInAnalysis>,
}

/// Row of the "Eigenvalues" table (Guide p. 122).
#[derive(Debug, Clone, PartialEq)]
pub struct EigenvalueRow {
    /// 1-based canonical function number.
    pub function: usize,
    /// Generalized eigenvalue `lambda_k`.
    pub eigenvalue: f64,
    /// Percentage of between-groups variance accounted for.
    pub percent_variance: f64,
    /// Cumulative percentage of variance.
    pub cumulative_percent: f64,
    /// Canonical correlation `sqrt(lambda_k / (1 + lambda_k))`.
    pub canonical_correlation: f64,
}

/// Row of the canonical "Wilks' Lambda" chi-square test table (Guide p. 123).
#[derive(Debug, Clone, PartialEq)]
pub struct WilksLambdaTestRow {
    /// First function tested (1-based).
    pub from_function: usize,
    /// Last function tested (1-based, equal to `m`).
    pub to_function: usize,
    /// Residual Wilks' lambda `prod_{i=k+1}^m 1 / (1 + lambda_i)`.
    pub wilks_lambda: f64,
    /// Bartlett chi-square statistic `-(n - (q + g)/2 - 1) ln(Lambda_k)`.
    pub chi_square: f64,
    /// Degrees of freedom `(q - k)(g - k - 1)`.
    pub df: usize,
    /// Significance level `P(Chi2 > chi_square)`.
    pub sig: f64,
}

/// Row of the "Structure Matrix" table (Guide p. 123).
#[derive(Debug, Clone, PartialEq)]
pub struct StructureRow {
    /// Candidate predictor index (`0..p`).
    pub predictor_index: usize,
    /// Predictor name.
    pub name: String,
    /// Whether this predictor was included in the final model.
    pub selected: bool,
    /// Pooled within-groups correlations with each canonical function (`m` entries).
    pub correlations: Vec<f64>,
    /// 0-based index of the canonical function with the largest absolute correlation.
    pub dominant_function: usize,
}

/// Fisher's linear classification function for one group (Guide pp. 121-122).
#[derive(Debug, Clone, PartialEq)]
pub struct FisherFunction {
    /// 0-based active group index.
    pub group_index: usize,
    /// Linear coefficients `b_{ij}` for the `q` selected predictors.
    pub coefficients: Vec<f64>,
    /// Constant term `a_j = ln(p_j) - 0.5 * sum_i b_{ij} Xbar_{ij}`.
    pub constant: f64,
}

/// Box's M test of equality of group covariance matrices (Guide pp. 124-125).
#[derive(Debug, Clone, PartialEq)]
pub struct BoxMResult {
    /// Box's M statistic.
    pub box_m: f64,
    /// Approximate F statistic (when F approximation is used).
    pub approx_f: Option<f64>,
    /// Numerator degrees of freedom `t_1` (or chi-square df).
    pub df1: f64,
    /// Denominator degrees of freedom `t_2` (when F approximation is used).
    pub df2: Option<f64>,
    /// Bartlett's chi-square statistic (used when `e_1^2 - e_2` is tiny, Guide p. 125).
    pub chi_square: Option<f64>,
    /// Significance level.
    pub sig: f64,
    /// Number of non-singular groups included in the test.
    pub groups_used: usize,
    /// Per-group `(group_index, rank, log_determinant)` (`None` if singular).
    pub log_determinants: Vec<(usize, usize, Option<f64>)>,
    /// Pooled within-groups log determinant `log |C'|`.
    pub pooled_log_determinant: f64,
}

/// Confusion matrix and accuracy summary ("Classification Results").
#[derive(Debug, Clone, PartialEq)]
pub struct ClassificationSummary {
    /// Weighted predicted counts (`g x g`: `counts[actual][predicted]`).
    pub counts: Vec<Vec<f64>>,
    /// Unweighted predicted counts (`g x g`: `unweighted_counts[actual][predicted]`).
    pub unweighted_counts: Vec<Vec<usize>>,
    /// Row percentages (`g x g`: `100 * counts[actual][predicted] / total_per_group[actual]`).
    pub percentages: Vec<Vec<f64>>,
    /// Total weighted count per actual group (`g`).
    pub total_per_group: Vec<f64>,
    /// Overall percentage of correctly classified weighted cases (`0..=100`).
    pub overall_accuracy_percent: f64,
}

/// Scoring result for a single record (Guide pp. 125-126).
#[derive(Debug, Clone, PartialEq)]
pub struct Prediction {
    /// 0-based index of the predicted group in [`DiscriminantModel::groups`].
    pub predicted_group_index: usize,
    /// Code of the predicted group.
    pub predicted_code: i64,
    /// Label of the predicted group.
    pub predicted_label: String,
    /// Canonical discriminant function scores `f = X B + a` (`m`).
    pub canonical_scores: Vec<f64>,
    /// Squared Mahalanobis (chi-square) distance from each group centroid (`g`).
    pub mahalanobis_sq: Vec<f64>,
    /// Conditional tail probabilities `P(D > d | G = j)` (`g`).
    pub p_x_given_g: Vec<f64>,
    /// Posterior group probabilities `P(G_j | X)` summing to `1.0` (`g`).
    pub posterior_probabilities: Vec<f64>,
}

/// Internal per-group canonical covariance factor for separate-groups (quadratic) classification.
#[derive(Debug, Clone, PartialEq)]
struct GroupCanonicalCov {
    /// Indices of retained (linearly independent) canonical functions (Guide p. 126).
    kept_funcs: Vec<usize>,
    /// Inverse of `D_{j11}` (`rank x rank`).
    inv_d11: Vec<Vec<f64>>,
    /// `ln |D_{j11}|`.
    log_det: f64,
}

/// A trained discriminant analysis model with full SPSS-style output tables and scoring methods.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscriminantModel {
    /// Target variable name.
    pub target_name: String,
    /// Names of all `p` candidate predictors in upstream order.
    pub predictor_names: Vec<String>,
    /// Indices (in `0..p`, ascending input order) of the `q` predictors kept in the model.
    pub selected_indices: Vec<usize>,
    /// Names of the `q` kept predictors in `selected_indices` order.
    pub selected_names: Vec<String>,
    /// Analysis Case Processing Summary.
    pub case_summary: CaseProcessingSummary,
    /// Active non-empty groups in target order (`g`).
    pub groups: Vec<GroupInfo>,
    /// Group Statistics (means, standard deviations, valid N).
    pub group_statistics: GroupStatistics,
    /// Univariate tests of equality of group means for all `p` candidates.
    pub univariate_anovas: Vec<UnivariateAnova>,
    /// Pooled within-groups covariance matrix `C = W / (n - g)` (`p x p`).
    pub within_groups_covariance: Vec<Vec<f64>>,
    /// Pooled within-groups correlation matrix `R` (`p x p`, `None` where `w_{ii} w_{ll} == 0`).
    pub within_groups_correlation: Vec<Vec<Option<f64>>>,
    /// Total covariance matrix `T / (n - 1)` (`p x p`).
    pub total_covariance: Vec<Vec<f64>>,
    /// Step 0 snapshot of "Variables Not in the Analysis" (empty when `method == Enter`).
    pub initial_variables_not_in: Vec<VariableNotInAnalysis>,
    /// Stepwise history (empty when `method == Enter`).
    pub stepwise_steps: Vec<StepwiseStep>,
    /// Eigenvalues and canonical correlations (`m = min(q, g - 1)`).
    pub eigenvalues: Vec<EigenvalueRow>,
    /// Wilks' lambda chi-square tests of canonical functions (`m`).
    pub wilks_lambda_tests: Vec<WilksLambdaTestRow>,
    /// Standardized canonical discriminant function coefficients `D` (`q x m`).
    pub standardized_coefficients: Vec<Vec<f64>>,
    /// Unstandardized canonical discriminant function coefficients `B` (`q x m`).
    pub unstandardized_coefficients: Vec<Vec<f64>>,
    /// Canonical discriminant function constants `a` (`m`).
    pub canonical_constants: Vec<f64>,
    /// Structure matrix ordered by dominant canonical function and descending `|r|`.
    pub structure_matrix: Vec<StructureRow>,
    /// Group centroids in canonical space (`g x m`).
    pub group_centroids: Vec<Vec<f64>>,
    /// Fisher's linear classification functions (`g`).
    pub fisher_functions: Vec<FisherFunction>,
    /// Optional Box's M test result.
    pub box_m: Option<BoxMResult>,
    /// Resubstitution classification table on the training records.
    pub classification: ClassificationSummary,
    /// Optional leave-one-out cross-validation classification table.
    pub leave_one_out_classification: Option<ClassificationSummary>,
    /// Normalized predictor importance `(name, importance)` for each kept predictor (sums to `1.0`).
    pub variable_importance: Vec<(String, f64)>,
    /// Covariance mode used for scoring.
    covariance_matrix: CovarianceMatrix,
    /// Per-group canonical covariance pseudo-inverses (populated when `SeparateGroups`).
    separate_group_covs: Vec<GroupCanonicalCov>,
}

/// Safe matrix element getter.
fn mat_get(m: &[Vec<f64>], r: usize, c: usize) -> f64 {
    m.get(r).and_then(|row| row.get(c)).copied().unwrap_or(0.0)
}

/// Safe matrix element setter.
fn mat_set(m: &mut [Vec<f64>], r: usize, c: usize, val: f64) {
    if let Some(slot) = m.get_mut(r).and_then(|row| row.get_mut(c)) {
        *slot = val;
    }
}

/// Safe matrix element accumulator.
fn mat_add(m: &mut [Vec<f64>], r: usize, c: usize, delta: f64) {
    if let Some(slot) = m.get_mut(r).and_then(|row| row.get_mut(c)) {
        *slot += delta;
    }
}

/// Safe matrix element divisor.
fn mat_div(m: &mut [Vec<f64>], r: usize, c: usize, denom: f64) {
    if let Some(slot) = m.get_mut(r).and_then(|row| row.get_mut(c)) {
        *slot /= denom;
    }
}

/// Safe slice element getter.
fn vec_get(v: &[f64], i: usize) -> f64 {
    v.get(i).copied().unwrap_or(0.0)
}

/// Safe slice element setter.
fn vec_set(v: &mut [f64], i: usize, val: f64) {
    if let Some(slot) = v.get_mut(i) {
        *slot = val;
    }
}

/// Safe slice element accumulator.
fn vec_add(v: &mut [f64], i: usize, delta: f64) {
    if let Some(slot) = v.get_mut(i) {
        *slot += delta;
    }
}

/// Safe slice element divisor.
fn vec_div(v: &mut [f64], i: usize, denom: f64) {
    if let Some(slot) = v.get_mut(i) {
        *slot /= denom;
    }
}

/// Validates user settings and raises explicit `GAP:` errors for out-of-scope options.
fn validate_settings(settings: &Settings) -> Result<(), AlgoError> {
    if settings.adjusted_propensity {
        return Err(gap(
            "adjusted propensity requires a separate test/validation partition and is not defined in the DISCRIMINANT Algorithms chapter",
        ));
    }
    if settings.missing_handling == MissingHandling::MeanSubstitution {
        return Err(gap(
            "mean substitution for missing values is not defined in the DISCRIMINANT Algorithms chapter",
        ));
    }
    if settings.leave_one_out && settings.covariance_matrix == CovarianceMatrix::SeparateGroups {
        return Err(gap(
            "leave-one-out cross-validation applies only to linear (within-groups) discriminant analysis, not separate-groups (quadratic)",
        ));
    }
    if !settings.tolerance.is_finite() || settings.tolerance <= 0.0 || settings.tolerance >= 1.0 {
        return Err(AlgoError::Settings(
            "tolerance must be finite and strictly between 0 and 1".into(),
        ));
    }
    if !settings.v_to_enter.is_finite() || settings.v_to_enter < 0.0 {
        return Err(AlgoError::Settings(
            "v_to_enter must be finite and non-negative".into(),
        ));
    }
    if let Some(ms) = settings.max_steps
        && (ms == 0 || ms > MAX_STEPS)
    {
        return Err(AlgoError::Settings(format!(
            "max_steps must be in 1..={MAX_STEPS}"
        )));
    }
    match settings.stepwise_criteria {
        StepwiseCriteria::UseFValue { f_entry, f_removal } => {
            if !f_entry.is_finite()
                || !f_removal.is_finite()
                || f_removal < 0.0
                || f_entry <= f_removal
            {
                return Err(AlgoError::Settings(
                    "F-to-enter must be strictly greater than F-to-remove >= 0".into(),
                ));
            }
        }
        StepwiseCriteria::UseProbability { p_entry, p_removal } => {
            if !p_entry.is_finite()
                || !p_removal.is_finite()
                || p_entry <= 0.0
                || p_removal >= 1.0
                || p_entry >= p_removal
            {
                return Err(AlgoError::Settings(
                    "probability to enter must satisfy 0 < p_entry < p_removal < 1".into(),
                ));
            }
        }
    }
    if let PriorProbabilities::Custom(ref priors) = settings.prior_probabilities {
        if priors.is_empty() {
            return Err(AlgoError::Settings(
                "custom prior probabilities must not be empty".into(),
            ));
        }
        let mut sum = 0.0;
        for &p in priors {
            if !p.is_finite() || p <= 0.0 || p >= 1.0 {
                return Err(AlgoError::Settings(
                    "each custom prior probability must be finite and in (0, 1)".into(),
                ));
            }
            sum += p;
        }
        if (sum - 1.0).abs() > 1e-6 {
            return Err(AlgoError::Settings(
                "custom prior probabilities must sum to 1.0".into(),
            ));
        }
    }
    Ok(())
}

/// Symmetric Dempster sweep operator (Guide p. 120).
///
/// Works identically for sweeping a variable IN (`a[k][k] > 0`) and sweeping OUT (`a[k][k] < 0`).
fn symmetric_sweep(a: &[Vec<f64>], k: usize) -> Result<Vec<Vec<f64>>, AlgoError> {
    let n = a.len();
    let d = mat_get(a, k, k);
    if !d.is_finite() || d.abs() <= PIVOT_TOL {
        return Err(AlgoError::Data(
            "singular pivot encountered during sweep".into(),
        ));
    }
    let mut b = vec![vec![0.0; n]; n];
    for i in 0..n {
        if i == k {
            continue;
        }
        let aik = mat_get(a, i, k);
        for j in i..n {
            if j == k {
                continue;
            }
            let val = mat_get(a, i, j) - aik * mat_get(a, k, j) / d;
            mat_set(&mut b, i, j, val);
            mat_set(&mut b, j, i, val);
        }
        let off = aik / d;
        mat_set(&mut b, i, k, off);
        mat_set(&mut b, k, i, off);
    }
    mat_set(&mut b, k, k, -1.0 / d);
    Ok(b)
}

/// Lower-triangular Cholesky decomposition `A = L L^T`. Returns `None` if not positive definite.
fn cholesky(a: &[Vec<f64>], min_diag: f64) -> Option<Vec<Vec<f64>>> {
    let n = a.len();
    let mut l = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..=i {
            let mut sum = 0.0;
            for k in 0..j {
                sum += mat_get(&l, i, k) * mat_get(&l, j, k);
            }
            if i == j {
                let diag_sq = mat_get(a, i, i) - sum;
                if !diag_sq.is_finite() || diag_sq <= 0.0 {
                    return None;
                }
                let diag = diag_sq.sqrt();
                if !diag.is_finite() || diag < min_diag {
                    return None;
                }
                mat_set(&mut l, i, j, diag);
            } else {
                let ljj = mat_get(&l, j, j);
                if ljj.abs() <= PIVOT_TOL {
                    return None;
                }
                mat_set(&mut l, i, j, (mat_get(a, i, j) - sum) / ljj);
            }
        }
    }
    Some(l)
}

/// Inverts a lower-triangular matrix `L`.
fn invert_lower(l: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n = l.len();
    let mut inv = vec![vec![0.0; n]; n];
    for i in 0..n {
        let lii = mat_get(l, i, i);
        if !lii.is_finite() || lii.abs() <= PIVOT_TOL {
            return None;
        }
        mat_set(&mut inv, i, i, 1.0 / lii);
        for j in 0..i {
            let mut sum = 0.0;
            for k in j..i {
                sum += mat_get(l, i, k) * mat_get(&inv, k, j);
            }
            mat_set(&mut inv, i, j, -sum / lii);
        }
    }
    Some(inv)
}

/// Inverts a symmetric positive-definite matrix via Cholesky decomposition.
fn invert_spd(a: &[Vec<f64>], min_diag: f64) -> Option<(Vec<Vec<f64>>, f64)> {
    let l = cholesky(a, min_diag)?;
    let n = l.len();
    let mut log_det = 0.0;
    for i in 0..n {
        log_det += 2.0 * mat_get(&l, i, i).ln();
    }
    let linv = invert_lower(&l)?;
    // A^{-1} = L^{-T} L^{-1}
    let mut inv = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in i..n {
            let mut sum = 0.0;
            for k in j..n {
                sum += mat_get(&linv, k, i) * mat_get(&linv, k, j);
            }
            mat_set(&mut inv, i, j, sum);
            mat_set(&mut inv, j, i, sum);
        }
    }
    Some((inv, log_det))
}

/// Computes `log |A|` for a symmetric positive-definite matrix via Cholesky.
fn log_det_spd(a: &[Vec<f64>]) -> Option<f64> {
    let l = cholesky(a, PIVOT_TOL)?;
    let mut ld = 0.0;
    for i in 0..l.len() {
        ld += 2.0 * mat_get(&l, i, i).ln();
    }
    Some(ld)
}

/// Matrix multiplication `A * B`.
fn mat_mul(a: &[Vec<f64>], b: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let r = a.len();
    let k = b.len();
    let c = b.first().map_or(0, Vec::len);
    let mut out = vec![vec![0.0; c]; r];
    for i in 0..r {
        for j in 0..c {
            let mut sum = 0.0;
            for m in 0..k {
                sum += mat_get(a, i, m) * mat_get(b, m, j);
            }
            mat_set(&mut out, i, j, sum);
        }
    }
    out
}

/// Matrix transpose `A^T`.
fn transpose(a: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let r = a.len();
    let c = a.first().map_or(0, Vec::len);
    let mut out = vec![vec![0.0; r]; c];
    for i in 0..r {
        for j in 0..c {
            mat_set(&mut out, j, i, mat_get(a, i, j));
        }
    }
    out
}

/// Extracts the submatrix indexed by `indices`.
fn submatrix(a: &[Vec<f64>], indices: &[usize]) -> Vec<Vec<f64>> {
    let k = indices.len();
    let mut out = vec![vec![0.0; k]; k];
    for (r, &ri) in indices.iter().enumerate() {
        for (c, &ci) in indices.iter().enumerate() {
            mat_set(&mut out, r, c, mat_get(a, ri, ci));
        }
    }
    out
}

/// Builds the sequential pseudo-inverse `D_{j11}^{-1}` specified in Guide p. 126.
///
/// Functions are tested in order `0..m`; function `k` is kept iff adding it keeps the Cholesky
/// diagonal `>= CHOLESKY_SINGULAR_TOL`.
fn separate_group_pseudo_inverse(d_j: &[Vec<f64>]) -> GroupCanonicalCov {
    let m = d_j.len();
    let mut kept: Vec<usize> = Vec::with_capacity(m);
    for func in 0..m {
        let mut trial = kept.clone();
        trial.push(func);
        let sub = submatrix(d_j, &trial);
        if cholesky(&sub, CHOLESKY_SINGULAR_TOL).is_some() {
            kept = trial;
        }
    }
    if kept.is_empty() {
        return GroupCanonicalCov {
            kept_funcs: Vec::new(),
            inv_d11: Vec::new(),
            log_det: 0.0,
        };
    }
    let sub = submatrix(d_j, &kept);
    if let Some((inv_d11, log_det)) = invert_spd(&sub, CHOLESKY_SINGULAR_TOL) {
        GroupCanonicalCov {
            kept_funcs: kept,
            inv_d11,
            log_det,
        }
    } else {
        GroupCanonicalCov {
            kept_funcs: Vec::new(),
            inv_d11: Vec::new(),
            log_det: 0.0,
        }
    }
}

/// Rao's approximate/exact F test for Wilks' lambda (Guide p. 121).
fn wilks_lambda_f(lambda: f64, q: usize, g: usize, n: f64) -> (f64, f64, f64, f64, bool) {
    let qf = q as f64;
    let hf = g.saturating_sub(1) as f64;
    let qh = qf * hf;
    let r = n - 1.0 - (qf + g as f64) / 2.0;
    let q2_h2 = qf * qf + hf * hf;
    let denom_s = qf * qf * hf * hf - 4.0;
    let s = if (q2_h2 - 5.0).abs() < 1e-12 || denom_s <= 0.0 {
        1.0
    } else {
        ((q2_h2 - 5.0) / denom_s).sqrt()
    };
    let df1 = qh;
    let df2 = r / s + 1.0 - qh / 2.0;
    let is_exact = q <= 2 || g.saturating_sub(1) <= 2;
    let lam_clamped = lambda.clamp(1e-300, 1.0);
    let lam_s = lam_clamped.powf(s);
    let f_stat = if df1 > 0.0 && df2 > 0.0 && lam_s > 0.0 {
        (((1.0 - lam_s) * df2) / (lam_s * df1)).max(0.0)
    } else {
        0.0
    };
    let sig = f_p(f_stat, df1, df2);
    (f_stat, df1, df2, sig, is_exact)
}

/// Computes tolerance of a variable currently in the analysis.
fn tol_in(w_orig: &[Vec<f64>], w_star: &[Vec<f64>], idx: usize) -> f64 {
    let wii = mat_get(w_orig, idx, idx);
    let wsii = mat_get(w_star, idx, idx);
    if wii <= 0.0 || wsii >= 0.0 || !wsii.is_finite() {
        0.0
    } else {
        (-1.0 / (wsii * wii)).clamp(0.0, 1.0)
    }
}

/// Computes `(self_tolerance, min_tolerance_if_entered)` for a candidate variable not in the model.
fn tol_if_enter(
    w_orig: &[Vec<f64>],
    w_star: &[Vec<f64>],
    cand: usize,
    in_set: &[usize],
) -> (f64, f64) {
    let wii = mat_get(w_orig, cand, cand);
    let d = mat_get(w_star, cand, cand);
    if wii <= 0.0 || d <= 0.0 || !d.is_finite() {
        return (0.0, 0.0);
    }
    let t_self = (d / wii).clamp(0.0, 1.0);
    let mut min_t = t_self;
    for &m in in_set {
        let wmm = mat_get(w_orig, m, m);
        let wsmm = mat_get(w_star, m, m);
        let wsmi = mat_get(w_star, m, cand);
        let new_wsmm = wsmm - (wsmi * wsmi) / d;
        let tm = if wmm <= 0.0 || new_wsmm >= 0.0 || !new_wsmm.is_finite() {
            0.0
        } else {
            (-1.0 / (new_wsmm * wmm)).clamp(0.0, 1.0)
        };
        if tm < min_t {
            min_t = tm;
        }
    }
    (t_self, min_t)
}

/// Shared read-only context for stepwise criterion evaluation and table snapshots.
struct StepwiseCtx<'a> {
    p: usize,
    g: usize,
    n: f64,
    tol_limit: f64,
    predictor_names: &'a [String],
    w_orig: &'a [Vec<f64>],
    t_orig: &'a [Vec<f64>],
    group_means: &'a [Vec<f64>],
    group_weights: &'a [f64],
}

impl StepwiseCtx<'_> {
    /// Evaluates a stepwise criterion for the subset `vars` with swept `W*` (`W*_{11} = -W_{11}^{-1}`).
    ///
    /// Returns `(score, higher_is_better)`.
    fn evaluate_criterion(
        &self,
        criterion: StepwiseCriterion,
        vars: &[usize],
        w_star: &[Vec<f64>],
        lambda: f64,
    ) -> (f64, bool) {
        let q = vars.len();
        let n_minus_g = self.n - self.g as f64;
        if q == 0 {
            return match criterion {
                StepwiseCriterion::WilksLambda => (1.0, false),
                StepwiseCriterion::UnexplainedVariance => {
                    ((self.g * self.g.saturating_sub(1)) as f64 / 2.0, false)
                }
                StepwiseCriterion::MahalanobisDistance
                | StepwiseCriterion::SmallestF
                | StepwiseCriterion::RaosV => (0.0, true),
            };
        }
        match criterion {
            StepwiseCriterion::WilksLambda => (lambda, false),
            StepwiseCriterion::RaosV => {
                let mut v = 0.0;
                for &i in vars {
                    for &l in vars {
                        v += mat_get(w_star, i, l)
                            * (mat_get(self.t_orig, i, l) - mat_get(self.w_orig, i, l));
                    }
                }
                (-(n_minus_g) * v, true)
            }
            StepwiseCriterion::MahalanobisDistance
            | StepwiseCriterion::SmallestF
            | StepwiseCriterion::UnexplainedVariance => {
                let mut min_d2 = f64::INFINITY;
                let mut min_f = f64::INFINITY;
                let mut sum_unexp = 0.0;
                let f_scale = (self.n - q as f64 - self.g as f64 + 1.0) / (q as f64 * n_minus_g);
                for a in 0..self.g {
                    for b in (a + 1)..self.g {
                        let mut quad = 0.0;
                        for &i in vars {
                            let di =
                                mat_get(self.group_means, a, i) - mat_get(self.group_means, b, i);
                            for &l in vars {
                                let dl = mat_get(self.group_means, a, l)
                                    - mat_get(self.group_means, b, l);
                                quad += mat_get(w_star, i, l) * di * dl;
                            }
                        }
                        let d2 = (-(n_minus_g) * quad).max(0.0);
                        if d2 < min_d2 {
                            min_d2 = d2;
                        }
                        let na = vec_get(self.group_weights, a);
                        let nb = vec_get(self.group_weights, b);
                        let fab = if na + nb > 0.0 {
                            f_scale * (na * nb / (na + nb)) * d2
                        } else {
                            0.0
                        };
                        if fab < min_f {
                            min_f = fab;
                        }
                        sum_unexp += 4.0 / (4.0 + d2);
                    }
                }
                match criterion {
                    StepwiseCriterion::MahalanobisDistance => {
                        (if min_d2.is_finite() { min_d2 } else { 0.0 }, true)
                    }
                    StepwiseCriterion::SmallestF => {
                        (if min_f.is_finite() { min_f } else { 0.0 }, true)
                    }
                    StepwiseCriterion::UnexplainedVariance => (sum_unexp, false),
                    _ => (lambda, false),
                }
            }
        }
    }

    /// Builds a snapshot of "Variables in the Analysis".
    fn snapshot_vars_in(
        &self,
        in_set: &[usize],
        w_star: &[Vec<f64>],
        t_star: &[Vec<f64>],
        lambda: f64,
    ) -> Vec<VariableInAnalysis> {
        let q = in_set.len();
        let df1 = self.g.saturating_sub(1) as f64;
        let df2 = self.n - q as f64 - self.g as f64 + 1.0;
        let mut rows = Vec::with_capacity(q);
        for &idx in in_set {
            let tol = tol_in(self.w_orig, w_star, idx);
            let ws = mat_get(w_star, idx, idx);
            let ts = mat_get(t_star, idx, idx);
            let f_rem = if df1 > 0.0 && df2 > 0.0 && ts.abs() > PIVOT_TOL {
                (((ws - ts) * df2) / (ts * df1)).max(0.0)
            } else {
                0.0
            };
            let lam_rem = if q > 1 && ts.abs() > PIVOT_TOL {
                Some((lambda * (ws / ts)).clamp(0.0, 1.0))
            } else {
                None
            };
            let name = self.predictor_names.get(idx).cloned().unwrap_or_default();
            rows.push(VariableInAnalysis {
                predictor_index: idx,
                name,
                tolerance: tol,
                f_to_remove: f_rem,
                wilks_lambda_if_removed: lam_rem,
            });
        }
        rows
    }

    /// Builds a snapshot of "Variables Not in the Analysis".
    fn snapshot_vars_not_in(
        &self,
        in_set: &[usize],
        w_star: &[Vec<f64>],
        t_star: &[Vec<f64>],
        lambda: f64,
    ) -> Vec<VariableNotInAnalysis> {
        let q = in_set.len();
        let df1 = self.g.saturating_sub(1) as f64;
        let df2 = self.n - q as f64 - self.g as f64;
        let mut rows = Vec::with_capacity(self.p.saturating_sub(q));
        for idx in 0..self.p {
            if in_set.contains(&idx) {
                continue;
            }
            let (tol, min_tol) = tol_if_enter(self.w_orig, w_star, idx, in_set);
            let ws = mat_get(w_star, idx, idx);
            let ts = mat_get(t_star, idx, idx);
            let (f_enter, lam_enter) = if min_tol > self.tol_limit
                && df1 > 0.0
                && df2 > 0.0
                && ws > PIVOT_TOL
                && ts > PIVOT_TOL
            {
                let fv = (((ts - ws) * df2) / (ws * df1)).max(0.0);
                let lv = (lambda * (ws / ts)).clamp(0.0, 1.0);
                (fv, Some(lv))
            } else {
                (0.0, None)
            };
            let name = self.predictor_names.get(idx).cloned().unwrap_or_default();
            rows.push(VariableNotInAnalysis {
                predictor_index: idx,
                name,
                tolerance: tol,
                min_tolerance: min_tol,
                f_to_enter: f_enter,
                wilks_lambda_if_entered: lam_enter,
            });
        }
        rows
    }
}

/// Checks whether a candidate variable satisfies the entry F / probability criterion.
fn passes_entry(criteria: StepwiseCriteria, f_val: f64, df1: f64, df2: f64) -> bool {
    if !f_val.is_finite() || df1 <= 0.0 || df2 <= 0.0 {
        return false;
    }
    match criteria {
        StepwiseCriteria::UseFValue { f_entry, .. } => f_val >= f_entry,
        StepwiseCriteria::UseProbability { p_entry, .. } => f_p(f_val, df1, df2) <= p_entry,
    }
}

/// Checks whether an active variable should be removed by the removal F / probability criterion.
fn should_remove(criteria: StepwiseCriteria, f_val: f64, df1: f64, df2: f64) -> bool {
    if !f_val.is_finite() || df1 <= 0.0 || df2 <= 0.0 {
        return true;
    }
    match criteria {
        StepwiseCriteria::UseFValue { f_removal, .. } => f_val < f_removal,
        StepwiseCriteria::UseProbability { p_removal, .. } => f_p(f_val, df1, df2) > p_removal,
    }
}

/// Trains a discriminant analysis model from plain predictor/target vectors.
pub fn train(data: &DiscriminantData, settings: &Settings) -> Result<DiscriminantModel, AlgoError> {
    crate::check_cancelled()?;
    validate_settings(settings)?;

    let n_records = data.target.codes.len();
    let p = data.predictors.len();
    if n_records == 0 {
        return Err(AlgoError::Data("dataset has no records".into()));
    }
    if n_records > MAX_RECORDS {
        return Err(AlgoError::Data(format!(
            "record count {n_records} exceeds hard limit {MAX_RECORDS}"
        )));
    }
    if p == 0 {
        return Err(AlgoError::Data("at least one predictor is required".into()));
    }
    if p > MAX_PREDICTORS {
        return Err(AlgoError::Data(format!(
            "predictor count {p} exceeds hard limit {MAX_PREDICTORS}"
        )));
    }
    if n_records.saturating_mul(p) > MAX_MATRIX_CELLS {
        return Err(AlgoError::Data(format!(
            "data matrix size exceeds hard limit {MAX_MATRIX_CELLS} cells"
        )));
    }

    let mut seen_names: HashSet<&str> = HashSet::with_capacity(p);
    let mut predictor_names: Vec<String> = Vec::with_capacity(p);
    for pred in &data.predictors {
        if pred.values.len() != n_records {
            return Err(AlgoError::Data(format!(
                "predictor '{}' has {} values, expected {n_records}",
                pred.name,
                pred.values.len()
            )));
        }
        if !seen_names.insert(pred.name.as_str()) {
            return Err(AlgoError::Data(format!(
                "duplicate predictor name '{}'",
                pred.name
            )));
        }
        predictor_names.push(pred.name.clone());
    }
    if let Some(ref w) = data.weights
        && w.len() != n_records
    {
        return Err(AlgoError::Data(format!(
            "weights length {} does not match record count {n_records}",
            w.len()
        )));
    }

    // Build target code-to-group mapping.
    let declared_groups: Vec<(i64, String)> = if !data.target.labels.is_empty() {
        let mut seen_codes: HashSet<i64> = HashSet::with_capacity(data.target.labels.len());
        for (code, _) in &data.target.labels {
            if !seen_codes.insert(*code) {
                return Err(AlgoError::Data(format!(
                    "duplicate target group code {code}"
                )));
            }
        }
        if data.target.labels.len() > MAX_GROUPS {
            return Err(AlgoError::Data(format!(
                "group count {} exceeds hard limit {MAX_GROUPS}",
                data.target.labels.len()
            )));
        }
        data.target.labels.clone()
    } else {
        let mut codes: Vec<i64> = data.target.codes.iter().copied().flatten().collect();
        codes.sort_unstable();
        codes.dedup();
        if codes.len() > MAX_GROUPS {
            return Err(AlgoError::Data(format!(
                "group count {} exceeds hard limit {MAX_GROUPS}",
                codes.len()
            )));
        }
        codes.into_iter().map(|c| (c, c.to_string())).collect()
    };

    let code_to_declared: BTreeMap<i64, usize> = declared_groups
        .iter()
        .enumerate()
        .map(|(idx, (code, _))| (*code, idx))
        .collect();

    // Validate finite predictor values / weights and perform listwise case filtering.
    let mut valid_rows: Vec<(usize, Vec<f64>, f64)> = Vec::new();
    let mut excluded_missing_group = 0usize;
    let mut excluded_missing_predictor = 0usize;
    let mut excluded_missing_both = 0usize;
    let mut excluded_invalid_weight = 0usize;

    for r in 0..n_records {
        if r % 4096 == 0 {
            crate::check_cancelled()?;
        }
        let weight = if let Some(ref ws) = data.weights {
            match ws.get(r).copied().flatten() {
                Some(w) if !w.is_finite() || w < 0.0 => {
                    return Err(AlgoError::Data(format!(
                        "invalid case weight {w} at record {r}"
                    )));
                }
                Some(w) if w > 0.0 => Some(w),
                _ => None,
            }
        } else {
            Some(1.0)
        };

        let group_idx = data
            .target
            .codes
            .get(r)
            .copied()
            .flatten()
            .and_then(|c| code_to_declared.get(&c).copied());

        let mut row_vals: Vec<f64> = Vec::with_capacity(p);
        let mut missing_pred = false;
        for pred in &data.predictors {
            match pred.values.get(r).copied().flatten() {
                Some(v) => {
                    if !v.is_finite() {
                        return Err(AlgoError::Data(format!(
                            "non-finite value in predictor '{}' at record {r}",
                            pred.name
                        )));
                    }
                    row_vals.push(v);
                }
                None => {
                    missing_pred = true;
                }
            }
        }

        let Some(w) = weight else {
            excluded_invalid_weight += 1;
            continue;
        };
        match (group_idx, missing_pred) {
            (Some(gi), false) => valid_rows.push((gi, row_vals, w)),
            (None, false) => excluded_missing_group += 1,
            (Some(_), true) => excluded_missing_predictor += 1,
            (None, true) => excluded_missing_both += 1,
        }
    }

    let valid_cases = valid_rows.len();
    let total_excluded = n_records.saturating_sub(valid_cases);
    let case_summary = CaseProcessingSummary {
        total_cases: n_records,
        valid_cases,
        excluded_missing_group,
        excluded_missing_predictor,
        excluded_missing_both,
        excluded_invalid_weight,
        total_excluded,
        valid_percent: 100.0 * (valid_cases as f64) / (n_records as f64),
    };

    // Identify non-empty groups and remap group indices to 0..g.
    let mut declared_counts = vec![0usize; declared_groups.len()];
    let mut declared_weights = vec![0.0f64; declared_groups.len()];
    for (gi, _, w) in &valid_rows {
        if let Some(c) = declared_counts.get_mut(*gi) {
            *c += 1;
        }
        if let Some(sw) = declared_weights.get_mut(*gi) {
            *sw += *w;
        }
    }

    let mut active_remap: Vec<Option<usize>> = vec![None; declared_groups.len()];
    let mut active_meta: Vec<(i64, String, usize, f64)> = Vec::new();
    for (decl_idx, (code, label)) in declared_groups.iter().enumerate() {
        let cnt = declared_counts.get(decl_idx).copied().unwrap_or(0);
        let sw = declared_weights.get(decl_idx).copied().unwrap_or(0.0);
        if cnt > 0 && sw > 0.0 {
            let new_idx = active_meta.len();
            if let Some(slot) = active_remap.get_mut(decl_idx) {
                *slot = Some(new_idx);
            }
            active_meta.push((*code, label.clone(), cnt, sw));
        }
    }

    let g = active_meta.len();
    let n_total: f64 = active_meta.iter().map(|m| m.3).sum();
    if g < 2 {
        return Err(AlgoError::Data(
            "at least two non-empty target groups are required".into(),
        ));
    }
    if valid_cases <= g || n_total <= g as f64 {
        return Err(AlgoError::Data(
            "total valid cases and sum of weights must exceed the number of groups".into(),
        ));
    }

    // Remap valid rows to active group index 0..g.
    let mut samples: Vec<(usize, Vec<f64>, f64)> = Vec::with_capacity(valid_cases);
    for (decl_gi, vals, w) in valid_rows {
        if let Some(gi) = active_remap.get(decl_gi).copied().flatten() {
            samples.push((gi, vals, w));
        }
    }

    // Resolve prior probabilities.
    let priors: Vec<f64> = match &settings.prior_probabilities {
        PriorProbabilities::AllEqual => vec![1.0 / (g as f64); g],
        PriorProbabilities::ComputeFromSizes => active_meta.iter().map(|m| m.3 / n_total).collect(),
        PriorProbabilities::Custom(custom) => {
            if custom.len() != g {
                return Err(AlgoError::Settings(format!(
                    "custom prior probabilities length {} does not match active group count {g}",
                    custom.len()
                )));
            }
            custom.clone()
        }
    };

    let groups: Vec<GroupInfo> = active_meta
        .iter()
        .enumerate()
        .map(|(j, (code, label, cnt, sw))| GroupInfo {
            code: *code,
            label: label.clone(),
            unweighted_n: *cnt,
            weighted_n: *sw,
            prior: vec_get(&priors, j),
        })
        .collect();
    let group_weights: Vec<f64> = groups.iter().map(|gr| gr.weighted_n).collect();

    // Pass 1: group means and overall means (Guide p. 117).
    let mut group_means = vec![vec![0.0; p]; g];
    let mut total_means = vec![0.0; p];
    for (gi, row, w) in &samples {
        for i in 0..p {
            let x = vec_get(row, i);
            mat_add(&mut group_means, *gi, i, *w * x);
            vec_add(&mut total_means, i, *w * x);
        }
    }
    for j in 0..g {
        let nj = vec_get(&group_weights, j);
        for i in 0..p {
            mat_div(&mut group_means, j, i, nj);
        }
    }
    for i in 0..p {
        vec_div(&mut total_means, i, n_total);
    }

    // Pass 2: centered SSCP matrices W, T, and individual group SSCPs (Guide pp. 117-118).
    let mut w_mat = vec![vec![0.0; p]; p];
    let mut t_mat = vec![vec![0.0; p]; p];
    let mut group_sscp = vec![vec![vec![0.0; p]; p]; g];
    for (r_idx, (gi, row, w)) in samples.iter().enumerate() {
        if r_idx % 4096 == 0 {
            crate::check_cancelled()?;
        }
        let mut dw = vec![0.0; p];
        let mut dt = vec![0.0; p];
        for i in 0..p {
            let x = vec_get(row, i);
            vec_set(&mut dw, i, x - mat_get(&group_means, *gi, i));
            vec_set(&mut dt, i, x - vec_get(&total_means, i));
        }
        for i in 0..p {
            let dwi = vec_get(&dw, i);
            let dti = vec_get(&dt, i);
            for l in i..p {
                let w_add = *w * dwi * vec_get(&dw, l);
                let t_add = *w * dti * vec_get(&dt, l);
                mat_add(&mut w_mat, i, l, w_add);
                mat_add(&mut t_mat, i, l, t_add);
                if let Some(gs) = group_sscp.get_mut(*gi) {
                    mat_add(gs, i, l, w_add);
                }
            }
        }
    }
    // Mirror upper triangle to lower triangle.
    for i in 0..p {
        for l in (i + 1)..p {
            let w_val = mat_get(&w_mat, i, l);
            let t_val = mat_get(&t_mat, i, l);
            mat_set(&mut w_mat, l, i, w_val);
            mat_set(&mut t_mat, l, i, t_val);
            for gs in &mut group_sscp {
                let g_val = mat_get(gs, i, l);
                mat_set(gs, l, i, g_val);
            }
        }
    }

    // Group statistics (means, SDs, counts).
    let mut per_group_stats = Vec::with_capacity(g);
    for (j, gr) in groups.iter().enumerate() {
        let mut g_vars = Vec::with_capacity(p);
        for (i, name) in predictor_names.iter().enumerate() {
            let ss = group_sscp.get(j).map_or(0.0, |m| mat_get(m, i, i));
            let std_dev = if gr.weighted_n > 1.0 {
                Some((ss / (gr.weighted_n - 1.0)).max(0.0).sqrt())
            } else {
                None
            };
            g_vars.push(GroupVariableStats {
                predictor_index: i,
                name: name.clone(),
                mean: mat_get(&group_means, j, i),
                std_dev,
                unweighted_n: gr.unweighted_n,
                weighted_n: gr.weighted_n,
            });
        }
        per_group_stats.push(g_vars);
    }
    let mut total_stats = Vec::with_capacity(p);
    for (i, name) in predictor_names.iter().enumerate() {
        let ss = mat_get(&t_mat, i, i);
        let std_dev = if n_total > 1.0 {
            Some((ss / (n_total - 1.0)).max(0.0).sqrt())
        } else {
            None
        };
        total_stats.push(GroupVariableStats {
            predictor_index: i,
            name: name.clone(),
            mean: vec_get(&total_means, i),
            std_dev,
            unweighted_n: valid_cases,
            weighted_n: n_total,
        });
    }
    let group_statistics = GroupStatistics {
        per_group: per_group_stats,
        total: total_stats,
    };

    // Covariance, correlation, and univariate ANOVAs (Guide p. 118).
    let n_minus_g = n_total - g as f64;
    let mut within_groups_covariance = vec![vec![0.0; p]; p];
    let mut within_groups_correlation = vec![vec![None; p]; p];
    let mut total_covariance = vec![vec![0.0; p]; p];
    for i in 0..p {
        let wii = mat_get(&w_mat, i, i);
        for l in 0..p {
            let wil = mat_get(&w_mat, i, l);
            let til = mat_get(&t_mat, i, l);
            let wll = mat_get(&w_mat, l, l);
            mat_set(&mut within_groups_covariance, i, l, wil / n_minus_g);
            mat_set(
                &mut total_covariance,
                i,
                l,
                if n_total > 1.0 {
                    til / (n_total - 1.0)
                } else {
                    0.0
                },
            );
            let corr = if wii > 0.0 && wll > 0.0 {
                Some((wil / (wii * wll).sqrt()).clamp(-1.0, 1.0))
            } else {
                None
            };
            if let Some(slot) = within_groups_correlation
                .get_mut(i)
                .and_then(|row| row.get_mut(l))
            {
                *slot = corr;
            }
        }
    }

    let mut univariate_anovas = Vec::with_capacity(p);
    let df1_u = g.saturating_sub(1);
    for (i, name) in predictor_names.iter().enumerate() {
        let wii = mat_get(&w_mat, i, i);
        let tii = mat_get(&t_mat, i, i);
        let lam_i = if tii > PIVOT_TOL {
            (wii / tii).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let f_i = if wii > PIVOT_TOL && df1_u > 0 {
            (((tii - wii) * n_minus_g) / (wii * df1_u as f64)).max(0.0)
        } else {
            0.0
        };
        let sig_i = f_p(f_i, df1_u as f64, n_minus_g);
        univariate_anovas.push(UnivariateAnova {
            predictor_index: i,
            name: name.clone(),
            wilks_lambda: lam_i,
            f_statistic: f_i,
            df1: df1_u,
            df2: n_minus_g,
            sig: sig_i,
        });
    }

    // Variable selection (Guide pp. 119-121).
    let mut w_star = w_mat.clone();
    let mut t_star = t_mat.clone();
    let mut in_set: Vec<usize> = Vec::new();
    let mut current_lambda = 1.0f64;
    let mut initial_variables_not_in: Vec<VariableNotInAnalysis> = Vec::new();
    let mut stepwise_steps: Vec<StepwiseStep> = Vec::new();

    match settings.method {
        Method::Enter => {
            for idx in 0..p {
                let inc = data.predictors.get(idx).map_or(1, |pr| pr.inclusion_level);
                if inc == 0 {
                    continue;
                }
                let (_, min_tol) = tol_if_enter(&w_mat, &w_star, idx, &in_set);
                if min_tol > settings.tolerance {
                    let ws = mat_get(&w_star, idx, idx);
                    let ts = mat_get(&t_star, idx, idx);
                    if ws > PIVOT_TOL && ts > PIVOT_TOL {
                        w_star = symmetric_sweep(&w_star, idx)?;
                        t_star = symmetric_sweep(&t_star, idx)?;
                        current_lambda = (current_lambda * (ws / ts)).clamp(1e-300, 1.0);
                        in_set.push(idx);
                    }
                }
            }
        }
        Method::Stepwise => {
            let sw_ctx = StepwiseCtx {
                p,
                g,
                n: n_total,
                tol_limit: settings.tolerance,
                predictor_names: &predictor_names,
                w_orig: &w_mat,
                t_orig: &t_mat,
                group_means: &group_means,
                group_weights: &group_weights,
            };
            let max_steps = settings
                .max_steps
                .unwrap_or_else(|| p.saturating_mul(2))
                .min(MAX_STEPS);
            initial_variables_not_in =
                sw_ctx.snapshot_vars_not_in(&in_set, &w_star, &t_star, current_lambda);

            // Distinct positive inclusion levels processed from highest down to 1 (Guide p. 119).
            let mut levels: Vec<u8> = data
                .predictors
                .iter()
                .map(|pr| pr.inclusion_level)
                .filter(|&lvl| lvl > 0)
                .collect();
            levels.sort_unstable();
            levels.dedup();
            levels.reverse();

            let mut step_num = 0usize;
            let mut visited: HashSet<Vec<usize>> = HashSet::new();
            visited.insert(Vec::new());

            for level in levels {
                if step_num >= max_steps {
                    break;
                }
                if level % 2 == 0 {
                    // Even inclusion level: enter eligible variables in upstream order.
                    for idx in 0..p {
                        if step_num >= max_steps {
                            break;
                        }
                        let inc = data.predictors.get(idx).map_or(0, |pr| pr.inclusion_level);
                        if inc != level || in_set.contains(&idx) {
                            continue;
                        }
                        let (_, min_tol) = tol_if_enter(&w_mat, &w_star, idx, &in_set);
                        let ws = mat_get(&w_star, idx, idx);
                        let ts = mat_get(&t_star, idx, idx);
                        let df2 = n_total - in_set.len() as f64 - g as f64;
                        if min_tol > settings.tolerance
                            && df2 > 0.0
                            && ws > PIVOT_TOL
                            && ts > PIVOT_TOL
                        {
                            w_star = symmetric_sweep(&w_star, idx)?;
                            t_star = symmetric_sweep(&t_star, idx)?;
                            current_lambda = (current_lambda * (ws / ts)).clamp(1e-300, 1.0);
                            in_set.push(idx);
                            step_num += 1;
                            let mut key = in_set.clone();
                            key.sort_unstable();
                            visited.insert(key);
                            let (crit_val, _) = sw_ctx.evaluate_criterion(
                                settings.stepwise_criterion,
                                &in_set,
                                &w_star,
                                current_lambda,
                            );
                            let (f_stat, f_df1, f_df2, f_sig, f_is_exact) =
                                wilks_lambda_f(current_lambda, in_set.len(), g, n_total);
                            stepwise_steps.push(StepwiseStep {
                                step: step_num,
                                action: StepAction::Entered(idx),
                                num_variables: in_set.len(),
                                wilks_lambda: current_lambda,
                                lambda_df1: in_set.len(),
                                lambda_df2: g.saturating_sub(1),
                                lambda_df3: n_minus_g,
                                f_statistic: f_stat,
                                f_df1,
                                f_df2,
                                f_sig,
                                f_is_exact,
                                criterion_value: crit_val,
                                variables_in: sw_ctx.snapshot_vars_in(
                                    &in_set,
                                    &w_star,
                                    &t_star,
                                    current_lambda,
                                ),
                                variables_not_in: sw_ctx.snapshot_vars_not_in(
                                    &in_set,
                                    &w_star,
                                    &t_star,
                                    current_lambda,
                                ),
                            });
                        }
                    }
                    continue;
                }

                // Odd inclusion level (including level 1, which also performs removals).
                loop {
                    crate::check_cancelled()?;
                    if step_num >= max_steps {
                        break;
                    }

                    // Removal phase (level 1 only, Guide p. 119).
                    if level == 1 {
                        while step_num < max_steps && !in_set.is_empty() {
                            let q = in_set.len();
                            let df1 = g.saturating_sub(1) as f64;
                            let df2 = n_total - q as f64 - g as f64 + 1.0;
                            let mut best_rem: Option<(usize, f64, f64, bool)> = None;
                            for &idx in &in_set {
                                let inc =
                                    data.predictors.get(idx).map_or(0, |pr| pr.inclusion_level);
                                if inc != 1 {
                                    continue;
                                }
                                let ws = mat_get(&w_star, idx, idx);
                                let ts = mat_get(&t_star, idx, idx);
                                let f_rem = if df1 > 0.0 && df2 > 0.0 && ts.abs() > PIVOT_TOL {
                                    (((ws - ts) * df2) / (ts * df1)).max(0.0)
                                } else {
                                    0.0
                                };
                                if !should_remove(settings.stepwise_criteria, f_rem, df1, df2) {
                                    continue;
                                }
                                let rem_vars: Vec<usize> =
                                    in_set.iter().copied().filter(|&v| v != idx).collect();
                                let mut rem_key = rem_vars.clone();
                                rem_key.sort_unstable();
                                if visited.contains(&rem_key) {
                                    continue;
                                }
                                let lam_rem = if rem_vars.is_empty() {
                                    1.0
                                } else if ts.abs() > PIVOT_TOL {
                                    (current_lambda * (ws / ts)).clamp(1e-300, 1.0)
                                } else {
                                    1.0
                                };
                                let (score, higher_is_better) = if settings.stepwise_criterion
                                    == StepwiseCriterion::WilksLambda
                                {
                                    (lam_rem, false)
                                } else if let Ok(w_trial) = symmetric_sweep(&w_star, idx) {
                                    sw_ctx.evaluate_criterion(
                                        settings.stepwise_criterion,
                                        &rem_vars,
                                        &w_trial,
                                        lam_rem,
                                    )
                                } else {
                                    continue;
                                };
                                let better = match best_rem {
                                    None => true,
                                    Some((b_idx, b_score, b_f, b_hi)) => {
                                        let diff = if b_hi {
                                            score - b_score
                                        } else {
                                            b_score - score
                                        };
                                        if diff > 1e-12 {
                                            true
                                        } else if diff.abs() <= 1e-12 {
                                            f_rem < b_f - 1e-12
                                                || ((f_rem - b_f).abs() <= 1e-12 && idx < b_idx)
                                        } else {
                                            false
                                        }
                                    }
                                };
                                if better {
                                    best_rem = Some((idx, score, f_rem, higher_is_better));
                                }
                            }
                            let Some((rem_idx, crit_val, _, _)) = best_rem else {
                                break;
                            };
                            let ws = mat_get(&w_star, rem_idx, rem_idx);
                            let ts = mat_get(&t_star, rem_idx, rem_idx);
                            w_star = symmetric_sweep(&w_star, rem_idx)?;
                            t_star = symmetric_sweep(&t_star, rem_idx)?;
                            in_set.retain(|&v| v != rem_idx);
                            current_lambda = if in_set.is_empty() {
                                1.0
                            } else if ts.abs() > PIVOT_TOL {
                                (current_lambda * (ws / ts)).clamp(1e-300, 1.0)
                            } else {
                                1.0
                            };
                            step_num += 1;
                            let mut key = in_set.clone();
                            key.sort_unstable();
                            visited.insert(key);
                            let (f_stat, f_df1, f_df2, f_sig, f_is_exact) = if in_set.is_empty() {
                                (0.0, 0.0, 0.0, 1.0, true)
                            } else {
                                wilks_lambda_f(current_lambda, in_set.len(), g, n_total)
                            };
                            stepwise_steps.push(StepwiseStep {
                                step: step_num,
                                action: StepAction::Removed(rem_idx),
                                num_variables: in_set.len(),
                                wilks_lambda: current_lambda,
                                lambda_df1: in_set.len(),
                                lambda_df2: g.saturating_sub(1),
                                lambda_df3: n_minus_g,
                                f_statistic: f_stat,
                                f_df1,
                                f_df2,
                                f_sig,
                                f_is_exact,
                                criterion_value: crit_val,
                                variables_in: sw_ctx.snapshot_vars_in(
                                    &in_set,
                                    &w_star,
                                    &t_star,
                                    current_lambda,
                                ),
                                variables_not_in: sw_ctx.snapshot_vars_not_in(
                                    &in_set,
                                    &w_star,
                                    &t_star,
                                    current_lambda,
                                ),
                            });
                        }
                        if step_num >= max_steps {
                            break;
                        }
                    }

                    // Entry phase for current odd inclusion level.
                    let q = in_set.len();
                    let df1 = g.saturating_sub(1) as f64;
                    let df2 = n_total - q as f64 - g as f64;
                    if df2 <= 0.0 {
                        break;
                    }
                    let current_v = if settings.stepwise_criterion == StepwiseCriterion::RaosV {
                        sw_ctx
                            .evaluate_criterion(
                                StepwiseCriterion::RaosV,
                                &in_set,
                                &w_star,
                                current_lambda,
                            )
                            .0
                    } else {
                        0.0
                    };

                    let mut best_enter: Option<(usize, f64, f64, f64, bool)> = None;
                    for idx in 0..p {
                        let inc = data.predictors.get(idx).map_or(0, |pr| pr.inclusion_level);
                        if inc != level || in_set.contains(&idx) {
                            continue;
                        }
                        let (_, min_tol) = tol_if_enter(&w_mat, &w_star, idx, &in_set);
                        if min_tol <= settings.tolerance {
                            continue;
                        }
                        let ws = mat_get(&w_star, idx, idx);
                        let ts = mat_get(&t_star, idx, idx);
                        if ws <= PIVOT_TOL || ts <= PIVOT_TOL {
                            continue;
                        }
                        let f_enter = (((ts - ws) * df2) / (ws * df1)).max(0.0);
                        if !passes_entry(settings.stepwise_criteria, f_enter, df1, df2) {
                            continue;
                        }
                        let mut trial_vars = in_set.clone();
                        trial_vars.push(idx);
                        let mut trial_key = trial_vars.clone();
                        trial_key.sort_unstable();
                        if visited.contains(&trial_key) {
                            continue;
                        }
                        let lam_new = (current_lambda * (ws / ts)).clamp(1e-300, 1.0);
                        let (score, higher_is_better) =
                            if settings.stepwise_criterion == StepwiseCriterion::WilksLambda {
                                (lam_new, false)
                            } else if let Ok(w_trial) = symmetric_sweep(&w_star, idx) {
                                sw_ctx.evaluate_criterion(
                                    settings.stepwise_criterion,
                                    &trial_vars,
                                    &w_trial,
                                    lam_new,
                                )
                            } else {
                                continue;
                            };
                        if settings.stepwise_criterion == StepwiseCriterion::RaosV
                            && score - current_v < settings.v_to_enter
                        {
                            continue;
                        }
                        let better = match best_enter {
                            None => true,
                            Some((b_idx, b_score, b_f, _, b_hi)) => {
                                let diff = if b_hi {
                                    score - b_score
                                } else {
                                    b_score - score
                                };
                                if diff > 1e-12 {
                                    true
                                } else if diff.abs() <= 1e-12 {
                                    f_enter > b_f + 1e-12
                                        || ((f_enter - b_f).abs() <= 1e-12 && idx < b_idx)
                                } else {
                                    false
                                }
                            }
                        };
                        if better {
                            best_enter = Some((idx, score, f_enter, lam_new, higher_is_better));
                        }
                    }

                    let Some((enter_idx, crit_val, _, lam_new, _)) = best_enter else {
                        break;
                    };
                    w_star = symmetric_sweep(&w_star, enter_idx)?;
                    t_star = symmetric_sweep(&t_star, enter_idx)?;
                    in_set.push(enter_idx);
                    current_lambda = lam_new;
                    step_num += 1;
                    let mut key = in_set.clone();
                    key.sort_unstable();
                    visited.insert(key);
                    let (f_stat, f_df1, f_df2, f_sig, f_is_exact) =
                        wilks_lambda_f(current_lambda, in_set.len(), g, n_total);
                    stepwise_steps.push(StepwiseStep {
                        step: step_num,
                        action: StepAction::Entered(enter_idx),
                        num_variables: in_set.len(),
                        wilks_lambda: current_lambda,
                        lambda_df1: in_set.len(),
                        lambda_df2: g.saturating_sub(1),
                        lambda_df3: n_minus_g,
                        f_statistic: f_stat,
                        f_df1,
                        f_df2,
                        f_sig,
                        f_is_exact,
                        criterion_value: crit_val,
                        variables_in: sw_ctx.snapshot_vars_in(
                            &in_set,
                            &w_star,
                            &t_star,
                            current_lambda,
                        ),
                        variables_not_in: sw_ctx.snapshot_vars_not_in(
                            &in_set,
                            &w_star,
                            &t_star,
                            current_lambda,
                        ),
                    });
                }
            }
        }
    }

    if in_set.is_empty() {
        return Err(AlgoError::Data(
            "no variables qualified for the discriminant analysis".into(),
        ));
    }

    // Order retained predictors in their original upstream input order.
    let mut selected_indices = in_set;
    selected_indices.sort_unstable();
    let q = selected_indices.len();
    let m = q.min(g.saturating_sub(1));
    let selected_names: Vec<String> = selected_indices
        .iter()
        .map(|&i| predictor_names.get(i).cloned().unwrap_or_default())
        .collect();

    // Canonical Discriminant Functions (Guide pp. 122-124).
    let w11 = submatrix(&w_mat, &selected_indices);
    let t11 = submatrix(&t_mat, &selected_indices);
    let mut b11 = vec![vec![0.0; q]; q];
    for i in 0..q {
        for j in 0..q {
            mat_set(&mut b11, i, j, mat_get(&t11, i, j) - mat_get(&w11, i, j));
        }
    }

    let l_chol = cholesky(&w11, PIVOT_TOL)
        .ok_or_else(|| AlgoError::Data("within-groups matrix W_11 is singular".into()))?;
    let l_inv = invert_lower(&l_chol)
        .ok_or_else(|| AlgoError::Data("failed to invert Cholesky factor of W_11".into()))?;
    let l_inv_t = transpose(&l_inv);
    let mut sym_m = mat_mul(&mat_mul(&l_inv, &b11), &l_inv_t);
    for i in 0..q {
        for j in (i + 1)..q {
            let avg = 0.5 * (mat_get(&sym_m, i, j) + mat_get(&sym_m, j, i));
            mat_set(&mut sym_m, i, j, avg);
            mat_set(&mut sym_m, j, i, avg);
        }
    }

    let (all_evals, all_evecs) = jacobi_eigen(sym_m)?;
    let raw_evals: Vec<f64> = all_evals.into_iter().take(m).map(|v| v.max(0.0)).collect();
    let mut u_m = vec![vec![0.0; m]; q];
    for r in 0..q {
        for c in 0..m {
            mat_set(&mut u_m, r, c, mat_get(&all_evecs, r, c));
        }
    }
    // V = U^{-1} (UV) = L^{-T} U_m, satisfying V^T W_11 V = I_m.
    let mut v_mat = mat_mul(&l_inv_t, &u_m);

    // Standardized D = S_11 V, with sign convention: largest |D[i][k]| positive.
    let s11_diag: Vec<f64> = (0..q)
        .map(|i| mat_get(&w11, i, i).max(0.0).sqrt())
        .collect();
    let mut standardized_coefficients = vec![vec![0.0; m]; q];
    for k in 0..m {
        let mut max_abs = -1.0f64;
        let mut max_val = 1.0f64;
        for i in 0..q {
            let d_ik = vec_get(&s11_diag, i) * mat_get(&v_mat, i, k);
            if d_ik.abs() > max_abs {
                max_abs = d_ik.abs();
                max_val = d_ik;
            }
        }
        let sign = if max_val < 0.0 { -1.0 } else { 1.0 };
        for i in 0..q {
            let v_ik = sign * mat_get(&v_mat, i, k);
            mat_set(&mut v_mat, i, k, v_ik);
            mat_set(
                &mut standardized_coefficients,
                i,
                k,
                vec_get(&s11_diag, i) * v_ik,
            );
        }
    }

    // Unstandardized coefficients B = sqrt(n - g) * V, constants a_k, group centroids (Guide pp. 123-124).
    let scale_b = n_minus_g.sqrt();
    let mut unstandardized_coefficients = vec![vec![0.0; m]; q];
    for i in 0..q {
        for k in 0..m {
            mat_set(
                &mut unstandardized_coefficients,
                i,
                k,
                scale_b * mat_get(&v_mat, i, k),
            );
        }
    }

    let mut canonical_constants = vec![0.0; m];
    for k in 0..m {
        let mut s = 0.0;
        for (i, &sel_idx) in selected_indices.iter().enumerate() {
            s += mat_get(&unstandardized_coefficients, i, k) * vec_get(&total_means, sel_idx);
        }
        vec_set(&mut canonical_constants, k, -s);
    }

    let mut group_centroids = vec![vec![0.0; m]; g];
    for j in 0..g {
        for k in 0..m {
            let mut s = vec_get(&canonical_constants, k);
            for (i, &sel_idx) in selected_indices.iter().enumerate() {
                s +=
                    mat_get(&unstandardized_coefficients, i, k) * mat_get(&group_means, j, sel_idx);
            }
            mat_set(&mut group_centroids, j, k, s);
        }
    }

    // Eigenvalues table (Guide p. 122).
    let sum_evals: f64 = raw_evals.iter().sum();
    let mut eigenvalues = Vec::with_capacity(m);
    let mut cum_pct = 0.0;
    for (k, &ev) in raw_evals.iter().enumerate() {
        let pct = if sum_evals > 0.0 {
            100.0 * ev / sum_evals
        } else {
            0.0
        };
        cum_pct += pct;
        let cc = (ev / (1.0 + ev)).max(0.0).sqrt();
        eigenvalues.push(EigenvalueRow {
            function: k + 1,
            eigenvalue: ev,
            percent_variance: pct,
            cumulative_percent: cum_pct.min(100.0),
            canonical_correlation: cc,
        });
    }

    // Canonical Wilks' Lambda chi-square tests (Guide p. 123).
    let mut wilks_lambda_tests = Vec::with_capacity(m);
    let chi2_mult = n_total - ((q + g) as f64) / 2.0 - 1.0;
    for k in 0..m {
        let mut lk = 1.0;
        for &ev in raw_evals.iter().skip(k) {
            lk /= 1.0 + ev;
        }
        let lk = lk.clamp(1e-300, 1.0);
        let chi2 = if chi2_mult > 0.0 {
            (-chi2_mult * lk.ln()).max(0.0)
        } else {
            0.0
        };
        let df = q.saturating_sub(k).saturating_mul(g.saturating_sub(k + 1));
        let sig = chi_square_p(chi2, df as f64);
        wilks_lambda_tests.push(WilksLambdaTestRow {
            from_function: k + 1,
            to_function: m,
            wilks_lambda: lk,
            chi_square: chi2,
            df,
            sig,
        });
    }

    // Structure Matrix R = S^{-1} W_{all, sel} V (Guide p. 123).
    let mut structure_matrix = Vec::with_capacity(p);
    for (i, name) in predictor_names.iter().enumerate() {
        let wii = mat_get(&w_mat, i, i);
        if wii <= 0.0 {
            continue;
        }
        let si = wii.sqrt();
        let mut corrs = vec![0.0; m];
        let mut dom_func = 0usize;
        let mut max_abs = -1.0f64;
        for k in 0..m {
            let mut sum = 0.0;
            for (l, &sel_idx) in selected_indices.iter().enumerate() {
                sum += mat_get(&w_mat, i, sel_idx) * mat_get(&v_mat, l, k);
            }
            let r_ik = (sum / si).clamp(-1.0, 1.0);
            vec_set(&mut corrs, k, r_ik);
            if r_ik.abs() > max_abs {
                max_abs = r_ik.abs();
                dom_func = k;
            }
        }
        structure_matrix.push(StructureRow {
            predictor_index: i,
            name: name.clone(),
            selected: selected_indices.contains(&i),
            correlations: corrs,
            dominant_function: dom_func,
        });
    }
    structure_matrix.sort_by(|a, b| {
        a.dominant_function
            .cmp(&b.dominant_function)
            .then_with(|| {
                let ra = vec_get(&a.correlations, a.dominant_function).abs();
                let rb = vec_get(&b.correlations, b.dominant_function).abs();
                rb.total_cmp(&ra)
            })
            .then_with(|| a.predictor_index.cmp(&b.predictor_index))
    });

    // Fisher's Linear Classification Functions (Guide pp. 121-122, sign corrected: b_j = C_11^{-1} Xbar_j).
    let (w11_inv, _) = invert_spd(&w11, PIVOT_TOL)
        .ok_or_else(|| AlgoError::Data("within-groups matrix W_11 is not invertible".into()))?;
    let mut fisher_functions = Vec::with_capacity(g);
    for (j, gr) in groups.iter().enumerate() {
        let mut coeffs = vec![0.0; q];
        for i in 0..q {
            let mut s = 0.0;
            for (l, &sel_idx) in selected_indices.iter().enumerate() {
                s += mat_get(&w11_inv, i, l) * mat_get(&group_means, j, sel_idx);
            }
            vec_set(&mut coeffs, i, n_minus_g * s);
        }
        let mut dot = 0.0;
        for (i, &sel_idx) in selected_indices.iter().enumerate() {
            dot += vec_get(&coeffs, i) * mat_get(&group_means, j, sel_idx);
        }
        let constant = gr.prior.ln() - 0.5 * dot;
        fisher_functions.push(FisherFunction {
            group_index: j,
            coefficients: coeffs,
            constant,
        });
    }

    // Separate-groups canonical covariance matrices D_j = B^T C^{(j)} B (Guide pp. 125-126).
    let mut separate_group_covs: Vec<GroupCanonicalCov> = Vec::new();
    if settings.covariance_matrix == CovarianceMatrix::SeparateGroups {
        let b_t = transpose(&unstandardized_coefficients);
        for (j, gr) in groups.iter().enumerate() {
            if gr.weighted_n <= 1.0 {
                separate_group_covs.push(GroupCanonicalCov {
                    kept_funcs: Vec::new(),
                    inv_d11: Vec::new(),
                    log_det: 0.0,
                });
                continue;
            }
            let gs_full = group_sscp
                .get(j)
                .map_or_else(|| vec![vec![0.0; p]; p], Clone::clone);
            let mut cj = submatrix(&gs_full, &selected_indices);
            let denom = gr.weighted_n - 1.0;
            for r in 0..q {
                for c in 0..q {
                    mat_div(&mut cj, r, c, denom);
                }
            }
            let mut dj = mat_mul(&mat_mul(&b_t, &cj), &unstandardized_coefficients);
            for r in 0..m {
                for c in (r + 1)..m {
                    let avg = 0.5 * (mat_get(&dj, r, c) + mat_get(&dj, c, r));
                    mat_set(&mut dj, r, c, avg);
                    mat_set(&mut dj, c, r, avg);
                }
            }
            separate_group_covs.push(separate_group_pseudo_inverse(&dj));
        }
    }

    // Optional Box's M test (Guide pp. 124-125).
    let box_m = if settings.compute_box_m {
        compute_box_m(&group_sscp, &groups, &selected_indices)
    } else {
        None
    };

    // Assemble model (temporary classification / importance filled next).
    let mut model = DiscriminantModel {
        target_name: data.target.name.clone(),
        predictor_names,
        selected_indices: selected_indices.clone(),
        selected_names: selected_names.clone(),
        case_summary,
        groups: groups.clone(),
        group_statistics,
        univariate_anovas,
        within_groups_covariance,
        within_groups_correlation,
        total_covariance,
        initial_variables_not_in,
        stepwise_steps,
        eigenvalues,
        wilks_lambda_tests,
        standardized_coefficients,
        unstandardized_coefficients,
        canonical_constants,
        structure_matrix,
        group_centroids,
        fisher_functions,
        box_m,
        classification: ClassificationSummary {
            counts: Vec::new(),
            unweighted_counts: Vec::new(),
            percentages: Vec::new(),
            total_per_group: Vec::new(),
            overall_accuracy_percent: 0.0,
        },
        leave_one_out_classification: None,
        variable_importance: Vec::new(),
        covariance_matrix: settings.covariance_matrix,
        separate_group_covs,
    };

    // Resubstitution classification table.
    let mut conf_w = vec![vec![0.0; g]; g];
    let mut conf_u = vec![vec![0usize; g]; g];
    for (r_idx, (gi, row, w)) in samples.iter().enumerate() {
        if r_idx % 4096 == 0 {
            crate::check_cancelled()?;
        }
        let sel_vals: Vec<f64> = selected_indices.iter().map(|&i| vec_get(row, i)).collect();
        let pred = model.score_selected_complete(&sel_vals);
        let pj = pred.predicted_group_index;
        mat_add(&mut conf_w, *gi, pj, *w);
        if let Some(slot) = conf_u.get_mut(*gi).and_then(|r| r.get_mut(pj)) {
            *slot += 1;
        }
    }
    model.classification = build_classification_summary(conf_w, conf_u);

    // Optional Leave-One-Out cross-validation (Guide pp. 126-127, linear only).
    if settings.leave_one_out {
        model.leave_one_out_classification = Some(compute_leave_one_out(
            &samples,
            &selected_indices,
            &groups,
            &group_means,
            &w11_inv,
            n_total,
        )?);
    }

    // Optional predictor importance (deterministic total-effect variance sensitivity on P(G|X)).
    if settings.compute_variable_importance {
        model.variable_importance =
            compute_variable_importance(&model, &samples, &selected_indices, &selected_names)?;
    }

    // Sanity-check Wilks' lambda consistency via Cholesky log-determinant (non-fatal internal check).
    let _ = log_det_spd(&w11);

    Ok(model)
}

/// Builds a [`ClassificationSummary`] from weighted and unweighted confusion matrices.
fn build_classification_summary(
    counts: Vec<Vec<f64>>,
    unweighted_counts: Vec<Vec<usize>>,
) -> ClassificationSummary {
    let g = counts.len();
    let mut percentages = vec![vec![0.0; g]; g];
    let mut total_per_group = vec![0.0; g];
    let mut total_w = 0.0;
    let mut correct_w = 0.0;
    for j in 0..g {
        let mut row_sum = 0.0;
        for k in 0..g {
            row_sum += mat_get(&counts, j, k);
        }
        vec_set(&mut total_per_group, j, row_sum);
        total_w += row_sum;
        correct_w += mat_get(&counts, j, j);
        if row_sum > 0.0 {
            for k in 0..g {
                mat_set(
                    &mut percentages,
                    j,
                    k,
                    100.0 * mat_get(&counts, j, k) / row_sum,
                );
            }
        }
    }
    let overall_accuracy_percent = if total_w > 0.0 {
        100.0 * correct_w / total_w
    } else {
        0.0
    };
    ClassificationSummary {
        counts,
        unweighted_counts,
        percentages,
        total_per_group,
        overall_accuracy_percent,
    }
}

/// Computes Box's M test of equality of group covariance matrices (Guide pp. 124-125).
fn compute_box_m(
    group_sscp: &[Vec<Vec<f64>>],
    groups: &[GroupInfo],
    selected_indices: &[usize],
) -> Option<BoxMResult> {
    let q = selected_indices.len();
    let g = groups.len();
    if q == 0 || g < 2 {
        return None;
    }

    let mut log_determinants: Vec<(usize, usize, Option<f64>)> = Vec::with_capacity(g);
    let mut pooled_sscp = vec![vec![0.0; q]; q];
    let mut n_prime = 0.0;
    let mut g_prime = 0usize;
    let mut sum_group_term = 0.0;
    let mut inv_nj_sum = 0.0;
    let mut inv_nj2_sum = 0.0;

    for (j, gr) in groups.iter().enumerate() {
        let gs_full = group_sscp.get(j)?;
        let sscp_j = submatrix(gs_full, selected_indices);
        if gr.weighted_n <= 1.0 {
            log_determinants.push((j, 0, None));
            continue;
        }
        if let Some(l) = cholesky(&sscp_j, CHOLESKY_SINGULAR_TOL) {
            let mut sum_ln_l = 0.0;
            for i in 0..q {
                sum_ln_l += mat_get(&l, i, i).ln();
            }
            let df_j = gr.weighted_n - 1.0;
            let log_det_cj = 2.0 * sum_ln_l - (q as f64) * df_j.ln();
            log_determinants.push((j, q, Some(log_det_cj)));
            sum_group_term += df_j * log_det_cj;
            n_prime += gr.weighted_n;
            g_prime += 1;
            inv_nj_sum += 1.0 / df_j;
            inv_nj2_sum += 1.0 / (df_j * df_j);
            for r in 0..q {
                for c in 0..q {
                    mat_add(&mut pooled_sscp, r, c, mat_get(&sscp_j, r, c));
                }
            }
        } else {
            log_determinants.push((j, 0, None));
        }
    }

    if g_prime < 2 || n_prime <= g_prime as f64 {
        return None;
    }
    let l_pool = cholesky(&pooled_sscp, CHOLESKY_SINGULAR_TOL)?;
    let mut sum_ln_pool = 0.0;
    for i in 0..q {
        sum_ln_pool += mat_get(&l_pool, i, i).ln();
    }
    let df_pool = n_prime - g_prime as f64;
    let pooled_log_det = 2.0 * sum_ln_pool - (q as f64) * df_pool.ln();
    let m_stat = (df_pool * pooled_log_det - sum_group_term).max(0.0);

    let pf = q as f64;
    let gf = g_prime as f64;
    let e1 = (inv_nj_sum - 1.0 / df_pool) * (2.0 * pf * pf + 3.0 * pf - 1.0)
        / (6.0 * (gf - 1.0) * (pf + 1.0));
    let e2 =
        (inv_nj2_sum - 1.0 / (df_pool * df_pool)) * (pf - 1.0) * (pf + 2.0) / (6.0 * (gf - 1.0));
    let t1 = (gf - 1.0) * pf * (pf + 1.0) / 2.0;

    let diff = e2 - e1 * e1;
    // Guide p. 125: if e2 == e2 + 0.0001*(e2 - e1^2) (i.e. |e2 - e1^2| tiny), use Bartlett chi2.
    if diff.abs() <= 1e-12 || (e2 + 0.0001 * diff - e2).abs() == 0.0 {
        let chi2 = (m_stat * (1.0 - e1)).max(0.0);
        let sig = chi_square_p(chi2, t1);
        return Some(BoxMResult {
            box_m: m_stat,
            approx_f: None,
            df1: t1,
            df2: None,
            chi_square: Some(chi2),
            sig,
            groups_used: g_prime,
            log_determinants,
            pooled_log_determinant: pooled_log_det,
        });
    }

    let t2 = (t1 + 2.0) / diff.abs();
    let (b, f_stat) = if e2 > e1 * e1 {
        let b_val = t1 / (1.0 - e1 - t1 / t2);
        (b_val, if b_val > 0.0 { m_stat / b_val } else { 0.0 })
    } else {
        let b_val = t2 / (1.0 - e1 - 2.0 / t2);
        let denom = t1 * (b_val - m_stat);
        (
            b_val,
            if denom > 0.0 {
                t2 * m_stat / denom
            } else {
                0.0
            },
        )
    };
    let _ = b;
    let f_stat = f_stat.max(0.0);
    let sig = f_p(f_stat, t1, t2);
    Some(BoxMResult {
        box_m: m_stat,
        approx_f: Some(f_stat),
        df1: t1,
        df2: Some(t2),
        chi_square: None,
        sig,
        groups_used: g_prime,
        log_determinants,
        pooled_log_determinant: pooled_log_det,
    })
}

/// Computes leave-one-out cross-validation classification for linear discriminant analysis
/// (Guide pp. 126-127, using the exact Sherman-Morrison pooled covariance update).
fn compute_leave_one_out(
    samples: &[(usize, Vec<f64>, f64)],
    selected_indices: &[usize],
    groups: &[GroupInfo],
    group_means: &[Vec<f64>],
    w11_inv: &[Vec<f64>],
    n_total: f64,
) -> Result<ClassificationSummary, AlgoError> {
    let q = selected_indices.len();
    let g = groups.len();
    let n_minus_g = n_total - g as f64;

    // Sigma^{-1} = (n - g) * W_11^{-1}.
    let mut sigma_inv = vec![vec![0.0; q]; q];
    for r in 0..q {
        for c in 0..q {
            mat_set(&mut sigma_inv, r, c, n_minus_g * mat_get(w11_inv, r, c));
        }
    }

    let sel_means: Vec<Vec<f64>> = (0..g)
        .map(|j| {
            selected_indices
                .iter()
                .map(|&idx| mat_get(group_means, j, idx))
                .collect()
        })
        .collect();

    let mut conf_w = vec![vec![0.0; g]; g];
    let mut conf_u = vec![vec![0usize; g]; g];

    for (r_idx, (j, row, f_jk)) in samples.iter().enumerate() {
        if r_idx % 2048 == 0 {
            crate::check_cancelled()?;
        }
        let gr_j = groups
            .get(*j)
            .ok_or_else(|| AlgoError::Data("invalid group index in LOO".into()))?;
        let nj = gr_j.weighted_n;
        let nj_rem = nj - *f_jk;
        let ng_rem = n_minus_g - *f_jk;
        if nj_rem <= PIVOT_TOL || ng_rem <= PIVOT_TOL {
            return Err(AlgoError::Data(
                "leave-one-out cannot omit a case that empties its group or leaves zero error df"
                    .into(),
            ));
        }

        let x: Vec<f64> = selected_indices
            .iter()
            .map(|&idx| vec_get(row, idx))
            .collect();
        // d = X_{jk} - M_j
        let mut d = vec![0.0; q];
        for i in 0..q {
            vec_set(&mut d, i, vec_get(&x, i) - mat_get(&sel_means, *j, i));
        }
        // u = Sigma^{-1} d
        let mut u = vec![0.0; q];
        for i in 0..q {
            let mut s = 0.0;
            for l in 0..q {
                s += mat_get(&sigma_inv, i, l) * vec_get(&d, l);
            }
            vec_set(&mut u, i, s);
        }
        let mut d_sinv_d = 0.0;
        for i in 0..q {
            d_sinv_d += vec_get(&d, i) * vec_get(&u, i);
        }
        let denom = nj_rem * n_minus_g - (*f_jk) * nj * d_sinv_d;
        if !denom.is_finite() || denom <= PIVOT_TOL {
            return Err(AlgoError::Data(
                "leave-one-out pooled covariance became singular when omitting a case".into(),
            ));
        }
        let scale = ng_rem / n_minus_g;
        let rank1_coeff = (*f_jk) * nj / denom;

        // Evaluate score log(P_i) - 0.5 * d_0^2(X_{jk}, M_i') for each group i.
        let mut best_i = 0usize;
        let mut best_score = f64::NEG_INFINITY;
        for (i, gr_i) in groups.iter().enumerate() {
            // diff = X_{jk} - M_{jk} if i == j else X_{jk} - M_i
            let mut diff = vec![0.0; q];
            if i == *j {
                let factor = nj / nj_rem;
                for c in 0..q {
                    vec_set(&mut diff, c, factor * vec_get(&d, c));
                }
            } else {
                for c in 0..q {
                    vec_set(&mut diff, c, vec_get(&x, c) - mat_get(&sel_means, i, c));
                }
            }
            // diff^T Sigma_{jk}^{-1} diff = scale * (diff^T Sigma^{-1} diff + rank1_coeff * (diff^T u)^2)
            let mut quad_base = 0.0;
            let mut diff_u = 0.0;
            for r in 0..q {
                let dr = vec_get(&diff, r);
                diff_u += dr * vec_get(&u, r);
                let mut row_s = 0.0;
                for c in 0..q {
                    row_s += mat_get(&sigma_inv, r, c) * vec_get(&diff, c);
                }
                quad_base += dr * row_s;
            }
            let d0_sq = (scale * (quad_base + rank1_coeff * diff_u * diff_u)).max(0.0);
            let score = gr_i.prior.ln() - 0.5 * d0_sq;
            if score > best_score {
                best_score = score;
                best_i = i;
            }
        }

        mat_add(&mut conf_w, *j, best_i, *f_jk);
        if let Some(slot) = conf_u.get_mut(*j).and_then(|r| r.get_mut(best_i)) {
            *slot += 1;
        }
    }

    Ok(build_classification_summary(conf_w, conf_u))
}

/// Computes deterministic total-effect variance sensitivity `E_{X_{-i}}[Var_{X_i}(P(G | X))]`
/// normalized to sum to `1.0` across the `q` kept predictors.
fn compute_variable_importance(
    model: &DiscriminantModel,
    samples: &[(usize, Vec<f64>, f64)],
    selected_indices: &[usize],
    selected_names: &[String],
) -> Result<Vec<(String, f64)>, AlgoError> {
    let q = selected_indices.len();
    let g = model.groups.len();
    if q == 0 {
        return Ok(Vec::new());
    }
    if q == 1 {
        let name = selected_names.first().cloned().unwrap_or_default();
        return Ok(vec![(name, 1.0)]);
    }

    // Build deterministic marginal representation `(value, probability)` for each selected predictor.
    let total_w: f64 = samples.iter().map(|s| s.2).sum();
    let mut marginals: Vec<Vec<(f64, f64)>> = Vec::with_capacity(q);
    for &sel_idx in selected_indices {
        let mut pairs: Vec<(f64, f64)> = samples
            .iter()
            .map(|(_, row, w)| (vec_get(row, sel_idx), *w))
            .collect();
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

        // Collapse exact duplicates.
        let mut unique: Vec<(f64, f64)> = Vec::new();
        for (v, w) in &pairs {
            if let Some(last) = unique.last_mut()
                && (last.0 - *v).abs() == 0.0
            {
                last.1 += *w;
            } else {
                unique.push((*v, *w));
            }
        }

        if unique.len() <= IMPORTANCE_MAX_EXACT_UNIQUE {
            for item in &mut unique {
                item.1 = if total_w > 0.0 { item.1 / total_w } else { 0.0 };
            }
            marginals.push(unique);
        } else {
            // Equal-frequency quantile bin means (64 bins).
            let bins = IMPORTANCE_MAX_EXACT_UNIQUE;
            let mut binned: Vec<(f64, f64)> = Vec::with_capacity(bins);
            let n_pairs = pairs.len();
            for b in 0..bins {
                let start = b.saturating_mul(n_pairs) / bins;
                let end = (b + 1).saturating_mul(n_pairs) / bins;
                if start >= end {
                    continue;
                }
                let mut w_sum = 0.0;
                let mut vx_sum = 0.0;
                for (v, w) in pairs.iter().take(end).skip(start) {
                    w_sum += *w;
                    vx_sum += *w * *v;
                }
                if w_sum > 0.0 && total_w > 0.0 {
                    binned.push((vx_sum / w_sum, w_sum / total_w));
                }
            }
            marginals.push(binned);
        }
    }

    // Deterministically stride rows if dataset exceeds IMPORTANCE_MAX_ROWS.
    let n_samples = samples.len();
    let stride = (n_samples / IMPORTANCE_MAX_ROWS).max(1);
    let mut raw_imp = vec![0.0; q];

    for i in 0..q {
        crate::check_cancelled()?;
        let marg = marginals.get(i).map_or(&[][..], Vec::as_slice);
        let mut tot_var = 0.0;
        let mut row_w_sum = 0.0;
        let mut row_idx = 0usize;
        while row_idx < n_samples {
            if let Some((_, row, row_w)) = samples.get(row_idx) {
                let mut x_mod: Vec<f64> = selected_indices
                    .iter()
                    .map(|&idx| vec_get(row, idx))
                    .collect();
                let mut mean_p = vec![0.0; g];
                let mut mean_p2 = vec![0.0; g];
                for &(val, prob_v) in marg {
                    vec_set(&mut x_mod, i, val);
                    let pred = model.score_selected_complete(&x_mod);
                    for j in 0..g {
                        let pj = vec_get(&pred.posterior_probabilities, j);
                        vec_add(&mut mean_p, j, prob_v * pj);
                        vec_add(&mut mean_p2, j, prob_v * pj * pj);
                    }
                }
                let mut var_row = 0.0;
                for j in 0..g {
                    let mj = vec_get(&mean_p, j);
                    let m2j = vec_get(&mean_p2, j);
                    var_row += (m2j - mj * mj).max(0.0);
                }
                tot_var += (*row_w) * var_row;
                row_w_sum += *row_w;
            }
            row_idx = row_idx.saturating_add(stride);
        }
        vec_set(
            &mut raw_imp,
            i,
            if row_w_sum > 0.0 {
                tot_var / row_w_sum
            } else {
                0.0
            },
        );
    }

    let sum_imp: f64 = raw_imp.iter().sum();
    let mut out = Vec::with_capacity(q);
    for (i, name) in selected_names.iter().enumerate() {
        let val = if sum_imp > 0.0 {
            vec_get(&raw_imp, i) / sum_imp
        } else {
            1.0 / (q as f64)
        };
        out.push((name.clone(), val));
    }
    Ok(out)
}

impl DiscriminantModel {
    /// Scores a record supplied in full candidate predictor order (`p` elements).
    ///
    /// Returns `Ok(None)` (`$null$`) if any predictor kept in the final model is missing
    /// (Guide p. 127 "Blank Handling"). Non-finite values in kept predictors return an error.
    pub fn predict(&self, record: &[Option<f64>]) -> Result<Option<Prediction>, AlgoError> {
        if record.len() != self.predictor_names.len() {
            return Err(AlgoError::Data(format!(
                "record length {} does not match candidate predictor count {}",
                record.len(),
                self.predictor_names.len()
            )));
        }
        let mut sel = Vec::with_capacity(self.selected_indices.len());
        for &idx in &self.selected_indices {
            match record.get(idx).copied().flatten() {
                Some(v) if v.is_finite() => sel.push(v),
                Some(_) => {
                    return Err(AlgoError::Data("non-finite value in scoring record".into()));
                }
                None => return Ok(None),
            }
        }
        Ok(Some(self.score_selected_complete(&sel)))
    }

    /// Scores a record supplied in kept-predictor order (`q` elements matching [`Self::selected_names`]).
    ///
    /// Returns `Ok(None)` (`$null$`) if any element is `None`.
    pub fn predict_selected(
        &self,
        selected_values: &[Option<f64>],
    ) -> Result<Option<Prediction>, AlgoError> {
        if selected_values.len() != self.selected_indices.len() {
            return Err(AlgoError::Data(format!(
                "selected value length {} does not match kept predictor count {}",
                selected_values.len(),
                self.selected_indices.len()
            )));
        }
        let mut sel = Vec::with_capacity(selected_values.len());
        for v_opt in selected_values {
            match *v_opt {
                Some(v) if v.is_finite() => sel.push(v),
                Some(_) => {
                    return Err(AlgoError::Data("non-finite value in scoring record".into()));
                }
                None => return Ok(None),
            }
        }
        Ok(Some(self.score_selected_complete(&sel)))
    }

    /// Internal complete-case scorer in canonical space (Guide pp. 125-126).
    fn score_selected_complete(&self, sel_vals: &[f64]) -> Prediction {
        let q = self.selected_indices.len();
        let m = self.canonical_constants.len();
        let g = self.groups.len();

        let mut f_scores = vec![0.0; m];
        for k in 0..m {
            let mut s = vec_get(&self.canonical_constants, k);
            for i in 0..q {
                s += mat_get(&self.unstandardized_coefficients, i, k) * vec_get(sel_vals, i);
            }
            vec_set(&mut f_scores, k, s);
        }

        let mut mahalanobis_sq = vec![0.0; g];
        let mut p_x_given_g = vec![0.0; g];
        let mut g_scores = vec![f64::NEG_INFINITY; g];

        for (j, gr) in self.groups.iter().enumerate() {
            let (chi2, log_det, rank) = match self.covariance_matrix {
                CovarianceMatrix::WithinGroups => {
                    let mut d2 = 0.0;
                    for k in 0..m {
                        let diff = vec_get(&f_scores, k) - mat_get(&self.group_centroids, j, k);
                        d2 += diff * diff;
                    }
                    (d2, 0.0, m)
                }
                CovarianceMatrix::SeparateGroups => {
                    if let Some(cov) = self.separate_group_covs.get(j) {
                        let r = cov.kept_funcs.len();
                        let mut diff = vec![0.0; r];
                        for (pos, &fk) in cov.kept_funcs.iter().enumerate() {
                            vec_set(
                                &mut diff,
                                pos,
                                vec_get(&f_scores, fk) - mat_get(&self.group_centroids, j, fk),
                            );
                        }
                        let mut d2 = 0.0;
                        for a in 0..r {
                            let da = vec_get(&diff, a);
                            for b in 0..r {
                                d2 += da * mat_get(&cov.inv_d11, a, b) * vec_get(&diff, b);
                            }
                        }
                        (d2.max(0.0), cov.log_det, r)
                    } else {
                        (0.0, 0.0, 0)
                    }
                }
            };
            vec_set(&mut mahalanobis_sq, j, chi2);
            vec_set(&mut p_x_given_g, j, chi_square_p(chi2, rank as f64));
            let gj = if gr.prior > 0.0 {
                gr.prior.ln() - 0.5 * (log_det + chi2)
            } else {
                f64::NEG_INFINITY
            };
            vec_set(&mut g_scores, j, gj);
        }

        let mut best_j = 0usize;
        let mut max_g = f64::NEG_INFINITY;
        for j in 0..g {
            let gj = vec_get(&g_scores, j);
            if gj > max_g {
                max_g = gj;
                best_j = j;
            }
        }

        let mut weights = vec![0.0; g];
        let mut w_sum = 0.0;
        for j in 0..g {
            let delta = vec_get(&g_scores, j) - max_g;
            let wj = if delta > POSTERIOR_LOG_CUTOFF && delta.is_finite() {
                delta.exp()
            } else {
                0.0
            };
            vec_set(&mut weights, j, wj);
            w_sum += wj;
        }
        let mut posterior_probabilities = vec![0.0; g];
        for j in 0..g {
            let p_j = if w_sum > 0.0 {
                vec_get(&weights, j) / w_sum
            } else if j == best_j {
                1.0
            } else {
                0.0
            };
            vec_set(&mut posterior_probabilities, j, p_j);
        }

        let (predicted_code, predicted_label) = self
            .groups
            .get(best_j)
            .map_or((0, String::new()), |gr| (gr.code, gr.label.clone()));

        Prediction {
            predicted_group_index: best_j,
            predicted_code,
            predicted_label,
            canonical_scores: f_scores,
            mahalanobis_sq,
            p_x_given_g,
            posterior_probabilities,
        }
    }

    /// Formats all discriminant tables as human-readable text for inspection and verification.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "Discriminant Analysis (target: {})", self.target_name);
        let _ = writeln!(
            out,
            "Cases: {} valid / {} total ({:.1}%)",
            self.case_summary.valid_cases,
            self.case_summary.total_cases,
            self.case_summary.valid_percent
        );

        let _ = writeln!(out, "\nPrior Probabilities for Groups:");
        for gr in &self.groups {
            let _ = writeln!(
                out,
                "  {} (code {}): prior={:.3}, N={} (weighted {:.3})",
                gr.label, gr.code, gr.prior, gr.unweighted_n, gr.weighted_n
            );
        }

        if !self.stepwise_steps.is_empty() {
            let _ = writeln!(out, "\nVariables Entered/Removed:");
            for st in &self.stepwise_steps {
                let act = match st.action {
                    StepAction::Entered(i) => {
                        format!(
                            "Entered {}",
                            self.predictor_names.get(i).map_or("", String::as_str)
                        )
                    }
                    StepAction::Removed(i) => {
                        format!(
                            "Removed {}",
                            self.predictor_names.get(i).map_or("", String::as_str)
                        )
                    }
                };
                let f_kind = if st.f_is_exact { "Exact F" } else { "Approx F" };
                let _ = writeln!(
                    out,
                    "  Step {}: {} | Lambda={:.3} (df {}, {}, {:.0}) | {}={:.3} (df {:.0}, {:.3}, sig={:.3})",
                    st.step,
                    act,
                    st.wilks_lambda,
                    st.lambda_df1,
                    st.lambda_df2,
                    st.lambda_df3,
                    f_kind,
                    st.f_statistic,
                    st.f_df1,
                    st.f_df2,
                    st.f_sig
                );
            }
        }

        let _ = writeln!(out, "\nEigenvalues:");
        for ev in &self.eigenvalues {
            let _ = writeln!(
                out,
                "  Func {}: ev={:.3}, %var={:.1}, cum%={:.1}, corr={:.3}",
                ev.function,
                ev.eigenvalue,
                ev.percent_variance,
                ev.cumulative_percent,
                ev.canonical_correlation
            );
        }

        let _ = writeln!(out, "\nWilks' Lambda:");
        for row in &self.wilks_lambda_tests {
            let _ = writeln!(
                out,
                "  {} through {}: Lambda={:.3}, chi2={:.3}, df={}, sig={:.3}",
                row.from_function,
                row.to_function,
                row.wilks_lambda,
                row.chi_square,
                row.df,
                row.sig
            );
        }

        let _ = writeln!(out, "\nStandardized Canonical Coefficients:");
        for (i, name) in self.selected_names.iter().enumerate() {
            let _ = write!(out, "  {name}:");
            for k in 0..self.canonical_constants.len() {
                let _ = write!(
                    out,
                    " {:+.3}",
                    mat_get(&self.standardized_coefficients, i, k)
                );
            }
            let _ = writeln!(out);
        }

        let _ = writeln!(out, "\nUnstandardized Canonical Coefficients:");
        for (i, name) in self.selected_names.iter().enumerate() {
            let _ = write!(out, "  {name}:");
            for k in 0..self.canonical_constants.len() {
                let _ = write!(
                    out,
                    " {:+.6}",
                    mat_get(&self.unstandardized_coefficients, i, k)
                );
            }
            let _ = writeln!(out);
        }
        let _ = write!(out, "  (Constant):");
        for &c in &self.canonical_constants {
            let _ = write!(out, " {c:+.6}");
        }
        let _ = writeln!(out);

        let _ = writeln!(out, "\nStructure Matrix:");
        for row in &self.structure_matrix {
            let mark = if row.selected { "" } else { " (not used)" };
            let _ = write!(out, "  {}{}:", row.name, mark);
            for (k, &r) in row.correlations.iter().enumerate() {
                let star = if k == row.dominant_function { "*" } else { "" };
                let _ = write!(out, " {r:+.3}{star}");
            }
            let _ = writeln!(out);
        }

        let _ = writeln!(out, "\nFunctions at Group Centroids:");
        for (j, gr) in self.groups.iter().enumerate() {
            let _ = write!(out, "  {}:", gr.label);
            for k in 0..self.canonical_constants.len() {
                let _ = write!(out, " {:+.3}", mat_get(&self.group_centroids, j, k));
            }
            let _ = writeln!(out);
        }

        let _ = writeln!(
            out,
            "\nClassification Results (overall {:.1}%):",
            self.classification.overall_accuracy_percent
        );
        for (j, gr) in self.groups.iter().enumerate() {
            let _ = write!(out, "  {}:", gr.label);
            for k in 0..self.groups.len() {
                let _ = write!(
                    out,
                    " {:.0} ({:.1}%)",
                    mat_get(&self.classification.counts, j, k),
                    mat_get(&self.classification.percentages, j, k)
                );
            }
            let _ = writeln!(out);
        }

        if let Some(ref loo) = self.leave_one_out_classification {
            let _ = writeln!(
                out,
                "\nCross-Validated (Leave-One-Out) Classification ({:.1}%):",
                loo.overall_accuracy_percent
            );
            for (j, gr) in self.groups.iter().enumerate() {
                let _ = write!(out, "  {}:", gr.label);
                for k in 0..self.groups.len() {
                    let _ = write!(
                        out,
                        " {:.0} ({:.1}%)",
                        mat_get(&loo.counts, j, k),
                        mat_get(&loo.percentages, j, k)
                    );
                }
                let _ = writeln!(out);
            }
        }

        if !self.variable_importance.is_empty() {
            let _ = writeln!(out, "\nVariable Importance:");
            for (name, imp) in &self.variable_importance {
                let _ = writeln!(out, "  {name}: {imp:.6}");
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    /// Hand-derived two-group, one-predictor example.
    /// Group 0: x = {1, 2, 3} -> mean 2, SS_0 = 2
    /// Group 1: x = {5, 6, 7} -> mean 6, SS_1 = 2
    /// Overall: n = 6, g = 2, mean = 4, W = 4, B = 3*(2-4)^2 + 3*(6-4)^2 = 24, T = 28.
    /// Eigenvalue = B/W = 6.0, canonical correlation = sqrt(6/7), Wilks' lambda = 4/28 = 1/7.
    /// Standardized D = 1.0, Unstandardized B = sqrt(4/4) = 1.0, constant = -4.0.
    /// Centroids: -2.0 and +2.0. Fisher boundary at x = 4.0.
    #[test]
    fn discriminant_hand_derived_two_group_one_var() -> Result<(), AlgoError> {
        let data = DiscriminantData {
            target: Target::new(
                "grp",
                vec![Some(0), Some(0), Some(0), Some(1), Some(1), Some(1)],
                vec![(0, "A".into()), (1, "B".into())],
            ),
            predictors: vec![Predictor::new(
                "x",
                vec![
                    Some(1.0),
                    Some(2.0),
                    Some(3.0),
                    Some(5.0),
                    Some(6.0),
                    Some(7.0),
                ],
            )],
            weights: None,
        };
        let settings = Settings {
            method: Method::Enter,
            leave_one_out: true,
            compute_box_m: true,
            ..Settings::default()
        };
        let model = train(&data, &settings)?;
        assert_eq!(model.selected_names, vec!["x"]);
        assert_eq!(model.eigenvalues.len(), 1);
        let ev0 = model
            .eigenvalues
            .first()
            .ok_or_else(|| AlgoError::Data("missing ev".into()))?;
        assert!(approx(ev0.eigenvalue, 6.0, 1e-12));
        assert!(approx(
            ev0.canonical_correlation,
            (6.0_f64 / 7.0).sqrt(),
            1e-12
        ));
        let wl0 = model
            .wilks_lambda_tests
            .first()
            .ok_or_else(|| AlgoError::Data("missing wilks".into()))?;
        assert!(approx(wl0.wilks_lambda, 1.0 / 7.0, 1e-12));
        assert!(approx(
            mat_get(&model.standardized_coefficients, 0, 0),
            1.0,
            1e-12
        ));
        assert!(approx(
            mat_get(&model.unstandardized_coefficients, 0, 0),
            1.0,
            1e-12
        ));
        assert!(approx(vec_get(&model.canonical_constants, 0), -4.0, 1e-12));
        assert!(approx(mat_get(&model.group_centroids, 0, 0), -2.0, 1e-12));
        assert!(approx(mat_get(&model.group_centroids, 1, 0), 2.0, 1e-12));
        assert!(approx(
            model.classification.overall_accuracy_percent,
            100.0,
            1e-12
        ));

        // Verify midpoint x = 4.0 gives equal 0.5 / 0.5 posteriors.
        let mid = model
            .predict(&[Some(4.0)])?
            .ok_or_else(|| AlgoError::Data("unexpected none".into()))?;
        assert!(approx(vec_get(&mid.posterior_probabilities, 0), 0.5, 1e-12));
        assert!(approx(vec_get(&mid.posterior_probabilities, 1), 0.5, 1e-12));

        // Missing input returns Ok(None).
        assert!(model.predict(&[None])?.is_none());

        // Box's M on identical group variances (both = 1.0) must be 0.0.
        let bm = model
            .box_m
            .as_ref()
            .ok_or_else(|| AlgoError::Data("missing box_m".into()))?;
        assert!(approx(bm.box_m, 0.0, 1e-12));

        // LOO should also classify all 6 points cleanly.
        let loo = model
            .leave_one_out_classification
            .as_ref()
            .ok_or_else(|| AlgoError::Data("missing loo".into()))?;
        assert!(approx(loo.overall_accuracy_percent, 100.0, 1e-12));

        assert!(!model.to_text().is_empty());
        Ok(())
    }

    /// Verifies mathematical invariants:
    /// 1. Posterior probabilities sum to 1.0 for every record.
    /// 2. Canonical discriminant scores have pooled within-groups covariance = I_m.
    /// 3. Fisher's linear classification functions match canonical WithinGroups predictions.
    /// 4. Leave-one-out Sherman-Morrison matches brute-force refitting with each case omitted.
    #[test]
    fn discriminant_properties_and_loo_matches_bruteforce() -> Result<(), AlgoError> {
        // 3 groups, 2 predictors, 18 records with mild overlap so LOO actually differs from resubstitution.
        let raw: [(i64, f64, f64); 18] = [
            (1, 1.0, 2.0),
            (1, 1.5, 1.2),
            (1, 2.2, 2.5),
            (1, 0.8, 1.8),
            (1, 2.9, 3.1),
            (1, 1.7, 2.1),
            (2, 4.0, 1.0),
            (2, 4.5, 1.8),
            (2, 3.2, 1.5),
            (2, 5.0, 2.0),
            (2, 3.8, 0.9),
            (2, 4.3, 2.3),
            (3, 2.5, 5.0),
            (3, 3.0, 4.2),
            (3, 2.0, 4.8),
            (3, 3.4, 5.5),
            (3, 2.8, 3.3),
            (3, 3.1, 4.7),
        ];
        let codes: Vec<Option<i64>> = raw.iter().map(|r| Some(r.0)).collect();
        let x1: Vec<Option<f64>> = raw.iter().map(|r| Some(r.1)).collect();
        let x2: Vec<Option<f64>> = raw.iter().map(|r| Some(r.2)).collect();
        let data = DiscriminantData {
            target: Target::new(
                "y",
                codes,
                vec![(1, "G1".into()), (2, "G2".into()), (3, "G3".into())],
            ),
            predictors: vec![Predictor::new("x1", x1), Predictor::new("x2", x2)],
            weights: None,
        };
        let settings = Settings {
            method: Method::Enter,
            prior_probabilities: PriorProbabilities::ComputeFromSizes,
            leave_one_out: true,
            ..Settings::default()
        };
        let model = train(&data, &settings)?;
        let m = model.canonical_constants.len();
        assert_eq!(m, 2);

        // 1 & 3: Posteriors sum to 1, and Fisher's linear functions pick the same class.
        let mut scores_by_group: Vec<Vec<Vec<f64>>> = vec![Vec::new(); 3];
        for &(c, a, b) in &raw {
            let pred = model
                .predict(&[Some(a), Some(b)])?
                .ok_or_else(|| AlgoError::Data("unexpected none".into()))?;
            let psum: f64 = pred.posterior_probabilities.iter().sum();
            assert!(approx(psum, 1.0, 1e-12));

            let mut best_f_j = 0usize;
            let mut best_f_val = f64::NEG_INFINITY;
            for ff in &model.fisher_functions {
                let val = ff.constant
                    + vec_get(&ff.coefficients, 0) * a
                    + vec_get(&ff.coefficients, 1) * b;
                if val > best_f_val {
                    best_f_val = val;
                    best_f_j = ff.group_index;
                }
            }
            assert_eq!(pred.predicted_group_index, best_f_j);
            let gi = usize::try_from(c - 1).unwrap_or(0);
            if let Some(bucket) = scores_by_group.get_mut(gi) {
                bucket.push(pred.canonical_scores);
            }
        }

        // 2: Within-groups covariance of canonical scores equals I_m.
        let mut w_canon = vec![vec![0.0; m]; m];
        for (j, bucket) in scores_by_group.iter().enumerate() {
            for f in bucket {
                for r in 0..m {
                    let dr = vec_get(f, r) - mat_get(&model.group_centroids, j, r);
                    for c in 0..m {
                        let dc = vec_get(f, c) - mat_get(&model.group_centroids, j, c);
                        mat_add(&mut w_canon, r, c, dr * dc);
                    }
                }
            }
        }
        let n_minus_g = (raw.len() - 3) as f64;
        for r in 0..m {
            for c in 0..m {
                let expected = if r == c { 1.0 } else { 0.0 };
                assert!(approx(mat_get(&w_canon, r, c) / n_minus_g, expected, 1e-11));
            }
        }

        // 4: Brute-force leave-one-out matches Sherman-Morrison LOO identically.
        let mut bf_counts = vec![vec![0usize; 3]; 3];
        for omit in 0..raw.len() {
            let sub_codes: Vec<Option<i64>> = raw
                .iter()
                .enumerate()
                .filter(|&(idx, _)| idx != omit)
                .map(|(_, r)| Some(r.0))
                .collect();
            let sub_x1: Vec<Option<f64>> = raw
                .iter()
                .enumerate()
                .filter(|&(idx, _)| idx != omit)
                .map(|(_, r)| Some(r.1))
                .collect();
            let sub_x2: Vec<Option<f64>> = raw
                .iter()
                .enumerate()
                .filter(|&(idx, _)| idx != omit)
                .map(|(_, r)| Some(r.2))
                .collect();
            let sub_data = DiscriminantData {
                target: Target::new(
                    "y",
                    sub_codes,
                    vec![(1, "G1".into()), (2, "G2".into()), (3, "G3".into())],
                ),
                predictors: vec![Predictor::new("x1", sub_x1), Predictor::new("x2", sub_x2)],
                weights: None,
            };
            // Note: Guide p. 127 keeps the full-sample priors P_i fixed during LOO.
            let sub_settings = Settings {
                method: Method::Enter,
                prior_probabilities: PriorProbabilities::AllEqual,
                compute_variable_importance: false,
                ..Settings::default()
            };
            let sub_model = train(&sub_data, &sub_settings)?;
            if let Some(&(c, a, b)) = raw.get(omit) {
                let pred = sub_model
                    .predict(&[Some(a), Some(b)])?
                    .ok_or_else(|| AlgoError::Data("unexpected none".into()))?;
                let gi = usize::try_from(c - 1).unwrap_or(0);
                if let Some(slot) = bf_counts
                    .get_mut(gi)
                    .and_then(|row| row.get_mut(pred.predicted_group_index))
                {
                    *slot += 1;
                }
            }
        }
        let loo = model
            .leave_one_out_classification
            .as_ref()
            .ok_or_else(|| AlgoError::Data("missing loo".into()))?;
        assert_eq!(loo.unweighted_counts, bf_counts);
        Ok(())
    }

    /// Tests separate-groups (quadratic) classification and all five stepwise criteria.
    #[test]
    fn discriminant_stepwise_criteria_and_separate_groups() -> Result<(), AlgoError> {
        let codes = vec![
            Some(1),
            Some(1),
            Some(1),
            Some(1),
            Some(1),
            Some(1),
            Some(2),
            Some(2),
            Some(2),
            Some(2),
            Some(2),
            Some(2),
            Some(3),
            Some(3),
            Some(3),
            Some(3),
            Some(3),
            Some(3),
        ];
        let x1 = vec![
            Some(1.0),
            Some(1.2),
            Some(0.9),
            Some(1.3),
            Some(1.1),
            Some(0.8),
            Some(5.0),
            Some(5.4),
            Some(4.8),
            Some(5.2),
            Some(4.9),
            Some(5.3),
            Some(9.0),
            Some(9.5),
            Some(8.7),
            Some(9.2),
            Some(8.8),
            Some(9.3),
        ];
        let x2 = vec![
            Some(0.1),
            Some(-0.2),
            Some(0.3),
            Some(0.0),
            Some(-0.1),
            Some(0.2),
            Some(4.0),
            Some(3.5),
            Some(4.5),
            Some(3.8),
            Some(4.2),
            Some(3.9),
            Some(0.2),
            Some(-0.1),
            Some(0.1),
            Some(-0.3),
            Some(0.4),
            Some(0.0),
        ];
        let noise = vec![
            Some(1.0),
            Some(2.0),
            Some(3.0),
            Some(4.0),
            Some(5.0),
            Some(6.0),
            Some(1.1),
            Some(2.0),
            Some(3.0),
            Some(4.0),
            Some(5.0),
            Some(5.9),
            Some(1.0),
            Some(2.1),
            Some(3.0),
            Some(4.0),
            Some(4.9),
            Some(6.0),
        ];
        let data = DiscriminantData {
            target: Target::new(
                "y",
                codes,
                vec![(1, "G1".into()), (2, "G2".into()), (3, "G3".into())],
            ),
            predictors: vec![
                Predictor::new("x1", x1),
                Predictor::new("x2", x2),
                Predictor::new("noise", noise),
            ],
            weights: None,
        };
        for crit in [
            StepwiseCriterion::WilksLambda,
            StepwiseCriterion::UnexplainedVariance,
            StepwiseCriterion::MahalanobisDistance,
            StepwiseCriterion::SmallestF,
            StepwiseCriterion::RaosV,
        ] {
            let settings = Settings {
                method: Method::Stepwise,
                stepwise_criterion: crit,
                stepwise_criteria: StepwiseCriteria::UseProbability {
                    p_entry: 0.05,
                    p_removal: 0.10,
                },
                covariance_matrix: CovarianceMatrix::SeparateGroups,
                ..Settings::default()
            };
            let model = train(&data, &settings)?;
            assert_eq!(model.selected_names, vec!["x1", "x2"]);
            assert!(approx(
                model.classification.overall_accuracy_percent,
                100.0,
                1e-12
            ));
        }
        Ok(())
    }

    /// Never-crash and error-handling tests on hostile/degenerate inputs and `GAP:` settings.
    #[test]
    fn discriminant_never_crash_and_gaps() -> Result<(), AlgoError> {
        // 1. Empty dataset -> error.
        let empty = DiscriminantData {
            target: Target::new("y", vec![], vec![]),
            predictors: vec![Predictor::new("x", vec![])],
            weights: None,
        };
        assert!(train(&empty, &Settings::default()).is_err());

        // 2. Single group -> error.
        let one_grp = DiscriminantData {
            target: Target::new("y", vec![Some(1), Some(1), Some(1)], vec![]),
            predictors: vec![Predictor::new("x", vec![Some(1.0), Some(2.0), Some(3.0)])],
            weights: None,
        };
        assert!(train(&one_grp, &Settings::default()).is_err());

        // 3. Constant predictor -> error when it is the only predictor; excluded when another exists.
        let const_only = DiscriminantData {
            target: Target::new("y", vec![Some(1), Some(1), Some(2), Some(2)], vec![]),
            predictors: vec![Predictor::new(
                "c",
                vec![Some(5.0), Some(5.0), Some(5.0), Some(5.0)],
            )],
            weights: None,
        };
        assert!(train(&const_only, &Settings::default()).is_err());

        // 4. Perfectly collinear predictors -> second excluded by tolerance, first kept cleanly.
        let collinear = DiscriminantData {
            target: Target::new(
                "y",
                vec![Some(1), Some(1), Some(1), Some(2), Some(2), Some(2)],
                vec![],
            ),
            predictors: vec![
                Predictor::new(
                    "a",
                    vec![
                        Some(1.0),
                        Some(2.0),
                        Some(3.0),
                        Some(6.0),
                        Some(7.0),
                        Some(8.0),
                    ],
                ),
                Predictor::new(
                    "b",
                    vec![
                        Some(2.0),
                        Some(4.0),
                        Some(6.0),
                        Some(12.0),
                        Some(14.0),
                        Some(16.0),
                    ],
                ),
            ],
            weights: None,
        };
        let col_model = train(&collinear, &Settings::default())?;
        assert_eq!(col_model.selected_names, vec!["a"]);

        // 5. More predictors than records -> tolerance drops dependent columns, never panics.
        let wide = DiscriminantData {
            target: Target::new("y", vec![Some(1), Some(1), Some(2), Some(2)], vec![]),
            predictors: vec![
                Predictor::new("p1", vec![Some(1.0), Some(2.0), Some(5.0), Some(6.0)]),
                Predictor::new("p2", vec![Some(3.0), Some(1.0), Some(4.0), Some(7.0)]),
                Predictor::new("p3", vec![Some(0.5), Some(1.5), Some(2.5), Some(3.0)]),
                Predictor::new("p4", vec![Some(9.0), Some(8.0), Some(1.0), Some(0.0)]),
                Predictor::new("p5", vec![Some(2.0), Some(2.1), Some(3.2), Some(3.5)]),
            ],
            weights: None,
        };
        let wide_model = train(&wide, &Settings::default())?;
        assert!(!wide_model.selected_indices.is_empty());
        assert!(wide_model.selected_indices.len() <= 2);

        // 6. NaN / Inf in predictor -> error.
        let nan_data = DiscriminantData {
            target: Target::new("y", vec![Some(1), Some(1), Some(2), Some(2)], vec![]),
            predictors: vec![Predictor::new(
                "x",
                vec![Some(1.0), Some(f64::NAN), Some(3.0), Some(4.0)],
            )],
            weights: None,
        };
        assert!(train(&nan_data, &Settings::default()).is_err());

        // 7. Explicit GAP: errors.
        let valid = collinear;
        let gap1 = train(
            &valid,
            &Settings {
                missing_handling: MissingHandling::MeanSubstitution,
                ..Settings::default()
            },
        );
        assert!(matches!(gap1, Err(AlgoError::Unsupported(ref msg)) if msg.contains("GAP:")));

        let gap2 = train(
            &valid,
            &Settings {
                adjusted_propensity: true,
                ..Settings::default()
            },
        );
        assert!(matches!(gap2, Err(AlgoError::Unsupported(ref msg)) if msg.contains("GAP:")));

        let gap3 = train(
            &valid,
            &Settings {
                covariance_matrix: CovarianceMatrix::SeparateGroups,
                leave_one_out: true,
                ..Settings::default()
            },
        );
        assert!(matches!(gap3, Err(AlgoError::Unsupported(ref msg)) if msg.contains("GAP:")));

        Ok(())
    }

    /// Verifies case weights match replicating rows identically.
    #[test]
    fn discriminant_weighted_cases_match_replicated_rows() -> Result<(), AlgoError> {
        let base_rows: [(i64, f64, f64, usize); 8] = [
            (1, 1.0, 2.0, 2),
            (1, 1.5, 2.5, 3),
            (1, 2.0, 1.5, 1),
            (1, 1.2, 1.8, 2),
            (2, 5.0, 6.0, 3),
            (2, 5.5, 5.2, 2),
            (2, 6.2, 6.8, 1),
            (2, 4.8, 5.8, 2),
        ];
        let w_data = DiscriminantData {
            target: Target::new(
                "y",
                base_rows.iter().map(|r| Some(r.0)).collect(),
                vec![(1, "A".into()), (2, "B".into())],
            ),
            predictors: vec![
                Predictor::new("x1", base_rows.iter().map(|r| Some(r.1)).collect()),
                Predictor::new("x2", base_rows.iter().map(|r| Some(r.2)).collect()),
            ],
            weights: Some(base_rows.iter().map(|r| Some(r.3 as f64)).collect()),
        };

        let mut rep_y = Vec::new();
        let mut rep_x1 = Vec::new();
        let mut rep_x2 = Vec::new();
        for &(y, a, b, cnt) in &base_rows {
            for _ in 0..cnt {
                rep_y.push(Some(y));
                rep_x1.push(Some(a));
                rep_x2.push(Some(b));
            }
        }
        let r_data = DiscriminantData {
            target: Target::new("y", rep_y, vec![(1, "A".into()), (2, "B".into())]),
            predictors: vec![Predictor::new("x1", rep_x1), Predictor::new("x2", rep_x2)],
            weights: None,
        };

        let settings = Settings {
            method: Method::Enter,
            prior_probabilities: PriorProbabilities::ComputeFromSizes,
            ..Settings::default()
        };
        let wm = train(&w_data, &settings)?;
        let rm = train(&r_data, &settings)?;
        assert!(approx(
            wm.eigenvalues.first().map_or(0.0, |e| e.eigenvalue),
            rm.eigenvalues.first().map_or(0.0, |e| e.eigenvalue),
            1e-12
        ));
        assert!(approx(
            mat_get(&wm.unstandardized_coefficients, 0, 0),
            mat_get(&rm.unstandardized_coefficients, 0, 0),
            1e-12
        ));
        assert!(approx(
            mat_get(&wm.unstandardized_coefficients, 1, 0),
            mat_get(&rm.unstandardized_coefficients, 1, 0),
            1e-12
        ));
        assert!(approx(
            vec_get(&wm.canonical_constants, 0),
            vec_get(&rm.canonical_constants, 0),
            1e-12
        ));
        Ok(())
    }

    /// Verifies stepwise removal triggers and inclusion levels (0 = excluded, 2 = forced first).
    #[test]
    fn discriminant_stepwise_removal_and_inclusion_levels() -> Result<(), AlgoError> {
        // 3 groups: z1 has means (+2, 0, -2), z2 has means (0, +2, -2);
        // proxy = z1 + z2 + eps has means (+2, +2, -4) so it wins Step 1 (largest univariate F).
        // Next z1 enters to separate G1 vs G2, then z2 enters to remove proxy's noise, causing
        // proxy's partial F-to-remove to collapse below 2.71 and trigger StepAction::Removed!
        let mut y = Vec::new();
        let mut z1 = Vec::new();
        let mut z2 = Vec::new();
        let mut proxy = Vec::new();
        let mut never = Vec::new();
        let mut forced = Vec::new();
        let noise_pts: [(f64, f64, f64); 10] = [
            (-1.4, 1.2, 0.6),
            (-1.1, 0.8, -0.6),
            (-0.8, 0.9, -0.6),
            (-0.5, 0.3, 0.6),
            (-0.2, 0.4, 0.0),
            (0.2, -0.4, 0.0),
            (0.5, -0.3, 0.6),
            (0.8, -0.9, -0.6),
            (1.1, -0.8, -0.6),
            (1.4, -1.2, 0.6),
        ];
        let forced_vals: [f64; 10] = [1.0, 1.0, -1.0, -1.0, 0.0, 0.0, -1.0, -1.0, 1.0, 1.0];
        let centers: [(i64, f64, f64); 3] = [(1, 2.0, 0.0), (2, 0.0, 2.0), (3, -2.0, -2.0)];
        for &(grp, c1, c2) in &centers {
            for (k, &(n1, n2, eps)) in noise_pts.iter().enumerate() {
                let v1 = c1 + n1;
                let v2 = c2 + n2;
                y.push(Some(grp));
                z1.push(Some(v1));
                z2.push(Some(v2));
                proxy.push(Some(v1 + v2 + 0.55 * eps));
                never.push(Some((grp as f64) * 10.0 + (k as f64) * 0.01));
                forced.push(Some(forced_vals.get(k).copied().unwrap_or(0.0)));
            }
        }
        let mut p_never = Predictor::new("never", never);
        p_never.inclusion_level = 0;
        let mut p_forced = Predictor::new("forced", forced);
        p_forced.inclusion_level = 2;

        let data = DiscriminantData {
            target: Target::new(
                "y",
                y,
                vec![(1, "G1".into()), (2, "G2".into()), (3, "G3".into())],
            ),
            predictors: vec![
                Predictor::new("proxy", proxy),
                Predictor::new("z1", z1),
                Predictor::new("z2", z2),
                p_never,
                p_forced,
            ],
            weights: None,
        };
        let settings = Settings {
            method: Method::Stepwise,
            stepwise_criterion: StepwiseCriterion::WilksLambda,
            stepwise_criteria: StepwiseCriteria::UseFValue {
                f_entry: 3.84,
                f_removal: 2.71,
            },
            ..Settings::default()
        };
        let model = train(&data, &settings)?;
        // `forced` (level 2) enters at step 1; `never` (level 0) is never entered;
        // `proxy` enters, then `z1` and `z2` enter, and `proxy` is removed!
        assert!(matches!(
            model.stepwise_steps.first().map(|s| s.action),
            Some(StepAction::Entered(4))
        ));
        assert!(
            model
                .stepwise_steps
                .iter()
                .any(|s| matches!(s.action, StepAction::Removed(0)))
        );
        assert!(!model.selected_names.contains(&"never".to_owned()));
        assert!(!model.selected_names.contains(&"proxy".to_owned()));
        assert!(model.selected_names.contains(&"forced".to_owned()));
        assert!(model.selected_names.contains(&"z1".to_owned()));
        assert!(model.selected_names.contains(&"z2".to_owned()));
        Ok(())
    }

    /// Verifies Guide p. 126 sequential pseudo-inverse when a group has singular covariance D_j.
    #[test]
    fn discriminant_separate_groups_singular_pseudoinverse() -> Result<(), AlgoError> {
        // Group 1 has constant x2 = 0.0 (singular 2x2 group covariance), while groups 2 & 3 vary in both.
        let data = DiscriminantData {
            target: Target::new(
                "y",
                vec![
                    Some(1),
                    Some(1),
                    Some(1),
                    Some(1),
                    Some(2),
                    Some(2),
                    Some(2),
                    Some(2),
                    Some(3),
                    Some(3),
                    Some(3),
                    Some(3),
                ],
                vec![(1, "G1".into()), (2, "G2".into()), (3, "G3".into())],
            ),
            predictors: vec![
                Predictor::new(
                    "x1",
                    vec![
                        Some(0.0),
                        Some(1.0),
                        Some(2.0),
                        Some(3.0),
                        Some(5.0),
                        Some(6.0),
                        Some(7.0),
                        Some(8.0),
                        Some(2.0),
                        Some(3.0),
                        Some(4.0),
                        Some(5.0),
                    ],
                ),
                Predictor::new(
                    "x2",
                    vec![
                        Some(0.0),
                        Some(0.0),
                        Some(0.0),
                        Some(0.0),
                        Some(1.0),
                        Some(3.0),
                        Some(2.0),
                        Some(4.0),
                        Some(6.0),
                        Some(8.0),
                        Some(7.0),
                        Some(9.0),
                    ],
                ),
            ],
            weights: None,
        };
        let settings = Settings {
            method: Method::Enter,
            covariance_matrix: CovarianceMatrix::SeparateGroups,
            ..Settings::default()
        };
        let model = train(&data, &settings)?;
        let pred = model
            .predict(&[Some(1.5), Some(0.0)])?
            .ok_or_else(|| AlgoError::Data("unexpected none".into()))?;
        let psum: f64 = pred.posterior_probabilities.iter().sum();
        assert!(approx(psum, 1.0, 1e-12));
        assert!(pred.posterior_probabilities.iter().all(|p| p.is_finite()));
        Ok(())
    }
}
