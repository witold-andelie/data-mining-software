//! Generalized linear mixed models (IBM SPSS Modeler 19 Algorithms Guide,
//! “Generalized linear mixed models algorithms”, pp. 191–225).
//!
//! Formulas are taken from the page images `target/grok-glmm/pages/guide_p191.png` …
//! `guide_p225.png`. The chapter text `target/grok-glmm/guide_glmm_chapter.txt` is
//! used for prose only. Callers pass plain vectors; there is no [`crate::Dataset`]
//! or PMML in this module.
//!
//! Say “reproduces the saved Modeler model” only for numbers checked against
//! `target/grok-glmm/model/`. Everything else “follows the Algorithms Guide” and is
//! unverified.
//!
//! # Decisions the Guide does not settle
//!
//! A. **REPL-PQL (method 3), not ML.** Guide p. 198: “We use method 3, REPL-PQL.”
//!    The saved information criteria use N* = N − p_x = 2124 and d = 3 covariance
//!    parameters: AICC = −2ℓ + 2 d N*/(N* − d − 1) and BIC = −2ℓ + d ln(N*) recover
//!    10793.7928882874 and 10810.7647366797 from minus2LogLikelihood
//!    10787.7815675326. ML would use N* = 2133 and would miss both criteria.
//!
//! B. **Residual degrees of freedom = N − rank(X) = 2133 − 9 = 2124.** Guide
//!    p. 207. Every saved `FTest/@df2` is 2124, and the t intervals use
//!    t_{1−α/2, 2124}.
//!
//! C. **Over-parameterised dummies, last level redundant.** Guide uses the GLM
//!    over-parameterised model and SWEEP. Saved ParamMatrix / ParameterEstimates
//!    have intercept first, then each factor’s levels in ascending code order,
//!    and the last level of each factor has `beta=0`, `df=0`, `isRedundant="true"`
//!    (`[school_setting=3]`, `[school_type=2]`, `[teaching_method=1]`,
//!    `[gender=1]`, `[lunch=2]`).
//!
//! D. **`SortFactors = Ascending` sorts numeric codes low to high.** Saved
//!    PPMatrix / labels list school_setting 1,2,3; teaching_method 0,1; lunch 1,2.
//!    Confirmed by those dummy labels.
//!
//! E. **Parameter names start at `P0000001`.** Saved ParameterList (same as
//!    GENLIN, not GLE’s `P0000000`).
//!
//! F. **minus2LogLikelihood is −2 ℓ_R on the original X scale.** Guide p. 201:
//!    −2ℓ_R = −2ℓ_R* − 2 ln|A| after undoing the covariate centering/scaling of
//!    pp. 195–196. AICC/BIC are built from that −2ℓ_R with d = 3.
//!
//! G. **Inner loop: one Fisher-scoring step (`MaxFisherIterNum=1`) then Newton
//!    on −2ℓ_R; parameter convergence 1e-6 absolute; singularity 1e-12.** Saved
//!    BuildSettings. Covariance of θ is 2 H^{−1} (Guide p. 199) with H the
//!    observed Hessian of −2ℓ_R. That recovers the two G-side standard errors
//!    to 2e-5 relative; the residual SE is 1.7% high (0.2519 vs 0.2476) and is
//!    reported honestly rather than scaled.
//!
//! H. **Wald intervals for variance components use the log transform**
//!    exp(ln σ̂² ± z_{1−α/2} · se(σ̂²)/σ̂²) (Guide p. 199). School variance
//!    17.2184670059646, se 6.74761786204855 gives the saved limits
//!    7.98773037382014 and 37.1163762621726. z = estimate/se and
//!    p = P(χ²_1 > z²).
//!
//! I. **Fixed-effect tests use t with Residual df, model-based Ĉ₁₁ = (X′V⁻¹X)⁻.**
//!    Saved `AsymEffCov=Model`, t = estimate/se, CI = estimate ± t_{1−α/2,2124} se.
//!    Intercept 47.2581526305317 / 3.25272194648629 = 14.5288018490427.
//!
//! J. **Type III F for a main effect is the Wald statistic of that effect’s
//!    non-redundant dummies, divided by rank(L Σ L′).** Guide p. 204. For a 1-df
//!    effect this equals t² (school_type F = 4.38733423135071 =
//!    (−2.09459643639311)²). Corrected-model L is every non-intercept
//!    non-redundant column (df1 = 8, F = 138.911028379804).
//!
//! K. **Covariate columns are centred and scaled as Guide pp. 195–196; dummies
//!    and the intercept are not.** Sample SD uses the N−1 denominator. The
//!    saved intercept 47.258… is on the original scale (back-transformed).
//!
//! L. **Random intercepts with covariance type VC, residual identity, processed
//!    as G-side + R-side jointly by REPL** (Guide p. 195 case 4). Saved
//!    `RandomSpecs` two intercept-only VC blocks and `ResidualCovarianceType=ID`.
//!    School variance 17.2184670059646, classroom-within-school 7.79320700458728,
//!    residual 7.83286410949453.
//!
//! M. **Normal + identity: the pseudo-target v is y and the outer PQL loop does
//!    not change v** (Guide pp. 197–199). The inner LMM is the whole estimation.
//!    Reproduces the saved Modeler model for this combination.
//!
//! Other distributions and links follow the Guide’s pseudo-likelihood outer loop
//! and are unverified. Items that return `GAP:` are listed on the corresponding
//! public enums and in `target/GROK_GLMM_REPORT.md`.

use crate::AlgoError;
use crate::stats::{chi_square_p, f_p, gamma_q, student_t_cdf, student_t_quantile};

const MAX_RECORDS: usize = 1_000_000;
const MAX_PARAMETERS: usize = 256;
const MAX_RANDOM_LEVELS: usize = 2_048;
const MAX_CATEGORIES: usize = 512;
const MAX_ITERATIONS_CAP: usize = 10_000;
const MAX_SPECS: usize = 8;
const ETA_CAP: f64 = 80.0;
const MU_FLOOR: f64 = 1e-12;
const VAR_FLOOR: f64 = 1e-12;
const TWO_PI: f64 = 2.0 * std::f64::consts::PI;
const LN_TWO_PI: f64 = 1.837_877_066_409_345_3;

/// Target distribution (Guide Table 19-1). Multinomial is `GAP:`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Distribution {
    /// Normal (verified with identity against the saved model).
    Normal,
    /// Inverse Gaussian (unverified).
    InverseGaussian,
    /// Gamma (unverified).
    Gamma,
    /// Negative binomial with a fixed ancillary `k` (unverified).
    Negbin {
        /// Ancillary parameter k ≥ 0.
        k: f64,
    },
    /// Poisson (unverified).
    Poisson,
    /// Binomial events/trials or 0/1 (unverified).
    Binomial,
}

/// Link function (Guide Tables 19-3 and 19-4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Link {
    /// Identity (verified for normal).
    Identity,
    /// Log (unverified).
    Log,
    /// Logit (unverified).
    Logit,
    /// Probit (unverified).
    Probit,
    /// Complementary log-log (unverified).
    Cloglog,
    /// Log-complement (unverified).
    Logc,
    /// Negative log-log (unverified).
    Nloglog,
    /// Power(α); `|α| < 2.2e-16` is treated as 0 (log), Guide p. 194.
    Power {
        /// Power parameter.
        alpha: f64,
    },
    /// Negative-binomial canonical link (unverified).
    Negbin {
        /// Ancillary k.
        k: f64,
    },
}

/// Covariance structure of a random-effect block or of the residual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CovarianceType {
    /// Variance components (verified for a random intercept).
    VarianceComponents,
    /// Identity; for a single random intercept this is the same as VC.
    Identity,
    /// Diagonal; for a single random intercept this is the same as VC.
    Diagonal,
    /// Unstructured. `GAP:` unless the block is 1×1.
    Unstructured,
    /// Compound symmetry. `GAP:` unless the block is 1×1.
    CompoundSymmetry,
    /// First-order autoregressive. `GAP:`.
    Ar1,
    /// ARMA(1,1). `GAP:`.
    Arma11,
    /// Toeplitz. `GAP:`.
    Toeplitz,
}

/// Denominator degrees of freedom for t and F (Guide pp. 207–208).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DfMethod {
    /// N − rank(X). Reproduces the saved model (`DegreesFreedom=Residual`).
    Residual,
    /// Satterthwaite’s approximation (Guide p. 208). Unverified.
    Satterthwaite,
}

/// Estimated covariance of the fixed effects (Guide p. 201).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CovarianceEstimator {
    /// Model-based Ĉ₁₁. Reproduces the saved model (`AsymEffCov=Model`).
    Model,
    /// Sandwich / robust (Guide p. 202). Unverified.
    Robust,
}

/// Sort order of factor levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueOrder {
    /// Numeric codes low to high, otherwise lexicographic. Saved model.
    Ascending,
    /// Numeric codes high to low.
    Descending,
    /// First appearance in the records.
    DataOrder,
}

/// A continuous fixed-effect column.
#[derive(Debug, Clone, PartialEq)]
pub struct NumericColumn {
    /// Field name (parameter label).
    pub name: String,
    /// One value per record.
    pub values: Vec<f64>,
}

/// A categorical fixed-effect column.
#[derive(Debug, Clone, PartialEq)]
pub struct FactorColumn {
    /// Field name.
    pub name: String,
    /// Level code per record (for example `"1"`). Empty is missing.
    pub levels: Vec<String>,
}

/// One fixed-effect term, in the order the design matrix lists effects.
#[derive(Debug, Clone, PartialEq)]
pub enum FixedTerm {
    /// Continuous covariate.
    Covariate(NumericColumn),
    /// Factor, over-parameterised dummies, last level redundant under SWEEP.
    Factor(FactorColumn),
}

/// One G-side random-effect specification.
#[derive(Debug, Clone, PartialEq)]
pub struct RandomSpec {
    /// Names of the subject fields (for example `["school"]` or `["school","classroom"]`).
    pub subject_names: Vec<String>,
    /// `subject_keys[i][j]` is subject field `j` for record `i`.
    pub subject_keys: Vec<Vec<String>>,
    /// Include a random intercept for this subject. Random slopes are `GAP:`.
    pub intercept: bool,
    /// Random slopes (each a numeric column). Non-empty is `GAP:`.
    pub slopes: Vec<NumericColumn>,
    /// Covariance type of this block. Only VC / Identity / Diagonal for an intercept.
    pub covariance: CovarianceType,
}

/// Records a GLMM trains on. The caller converts a table into these vectors.
#[derive(Debug, Clone, PartialEq)]
pub struct GlmmInput {
    /// Target field name (documentation / scoring).
    pub target_name: String,
    /// Target values, one per record.
    pub y: Vec<f64>,
    /// Fixed effects in design order (factors and covariates interleaved).
    pub fixed: Vec<FixedTerm>,
    /// Random-effect blocks in specification order.
    pub random: Vec<RandomSpec>,
    /// Scale weights ω. Missing, non-finite, or `≤ 0` drops the record.
    pub weight: Option<Vec<f64>>,
    /// Frequency weights. Non-integers round to nearest; `< 0.5` drops the record.
    pub frequency: Option<Vec<f64>>,
    /// Offset. Missing / non-finite drops the record.
    pub offset: Option<Vec<f64>>,
    /// Binomial trials `m`. Ignored for other distributions.
    pub trials: Option<Vec<f64>>,
}

/// Build settings. Defaults match the saved test-scores stream.
#[derive(Debug, Clone, PartialEq)]
pub struct GlmmSettings {
    /// Target distribution.
    pub distribution: Distribution,
    /// Link function.
    pub link: Link,
    /// Include a fixed intercept.
    pub intercept: bool,
    /// Residual covariance. Only [`CovarianceType::Identity`] is implemented.
    pub residual: CovarianceType,
    /// Degrees-of-freedom method.
    pub df_method: DfMethod,
    /// Model-based or robust covariance of β.
    pub covariance: CovarianceEstimator,
    /// Factor-level order.
    pub value_order: ValueOrder,
    /// Maximum inner Newton / Fisher iterations.
    pub max_iterations: usize,
    /// Maximum outer PQL iterations.
    pub max_outer_iterations: usize,
    /// Maximum step-halves per inner iteration.
    pub max_step_halving: usize,
    /// Parameter-convergence tolerance of the inner LMM.
    pub parameter_convergence: f64,
    /// Absolute (`false`) or relative (`true`) inner parameter convergence.
    pub parameter_relative: bool,
    /// Optional log-likelihood convergence of the inner LMM.
    pub likelihood_convergence: Option<f64>,
    /// Absolute (`false`) or relative (`true`) likelihood convergence.
    pub likelihood_relative: bool,
    /// Optional Hessian convergence of the inner LMM.
    pub hessian_convergence: Option<f64>,
    /// Absolute (`false`) or relative (`true`) Hessian convergence.
    pub hessian_relative: bool,
    /// Number of Fisher-scoring inner steps before Newton.
    pub max_fisher_iterations: usize,
    /// SWEEP singularity tolerance (relative to the original diagonal).
    pub singularity: f64,
    /// Confidence level as a percent (saved model: 95).
    pub confidence: f64,
}

impl Default for GlmmSettings {
    fn default() -> Self {
        GlmmSettings {
            distribution: Distribution::Normal,
            link: Link::Identity,
            intercept: true,
            residual: CovarianceType::Identity,
            df_method: DfMethod::Residual,
            covariance: CovarianceEstimator::Model,
            value_order: ValueOrder::Ascending,
            max_iterations: 100,
            max_outer_iterations: 100,
            max_step_halving: 8,
            parameter_convergence: 1e-6,
            parameter_relative: false,
            likelihood_convergence: None,
            likelihood_relative: false,
            hessian_convergence: None,
            hessian_relative: false,
            max_fisher_iterations: 1,
            singularity: 1e-12,
            confidence: 95.0,
        }
    }
}

/// One fixed-effect parameter (over-parameterised dummy or covariate).
#[derive(Debug, Clone, PartialEq)]
pub struct ParameterEstimate {
    /// `P0000001`, …
    pub name: String,
    /// `Intercept`, `[school_setting=1]`, `pretest`, …
    pub label: String,
    /// Estimate on the original scale.
    pub estimate: f64,
    /// Model-based (or robust) standard error. `None` if redundant.
    pub std_error: Option<f64>,
    /// t = estimate / se. `None` if redundant.
    pub t: Option<f64>,
    /// Two-sided t p-value.
    pub p_value: Option<f64>,
    /// Lower Wald confidence limit.
    pub lower: Option<f64>,
    /// Upper Wald confidence limit.
    pub upper: Option<f64>,
    /// 1 if the column was swept, 0 if redundant.
    pub df: u32,
    /// Last (dependent) dummy of a factor, or a SWEEP-aliased column.
    pub redundant: bool,
}

/// One G-side or residual variance component.
#[derive(Debug, Clone, PartialEq)]
pub struct VarianceComponent {
    /// Display name.
    pub name: String,
    /// Index of the random spec, or `None` for the residual.
    pub spec_index: Option<usize>,
    /// Estimate.
    pub estimate: f64,
    /// Wald standard error from 2 H⁻¹.
    pub std_error: Option<f64>,
    /// z = estimate / se.
    pub z: Option<f64>,
    /// Two-sided normal p-value.
    pub p_value: Option<f64>,
    /// Lower log-transform Wald limit.
    pub lower: Option<f64>,
    /// Upper log-transform Wald limit.
    pub upper: Option<f64>,
}

/// Type III F test of a fixed effect, or the corrected model.
#[derive(Debug, Clone, PartialEq)]
pub struct FTest {
    /// Effect name, or `"corrected_model"`.
    pub name: String,
    /// F statistic.
    pub f: Option<f64>,
    /// Numerator degrees of freedom.
    pub df1: f64,
    /// Denominator degrees of freedom.
    pub df2: f64,
    /// p-value of the F.
    pub p_value: Option<f64>,
}

/// Best linear unbiased predictor of one random intercept.
#[derive(Debug, Clone, PartialEq)]
pub struct RandomPrediction {
    /// Random-spec index.
    pub spec_index: usize,
    /// Subject key.
    pub subject_key: Vec<String>,
    /// BLUP.
    pub blup: f64,
    /// Square root of the corresponding Ĉ₂₂ diagonal.
    pub std_error: Option<f64>,
}

/// Predicted linear predictor and mean for one record.
#[derive(Debug, Clone, PartialEq)]
pub struct Score {
    /// x′β + offset (random effects at 0).
    pub eta_fixed: f64,
    /// x′β + z′γ̂ + offset, or `eta_fixed` if the subject is unknown.
    pub eta: f64,
    /// g⁻¹(eta).
    pub mu: f64,
    /// Standard error of `eta_fixed`.
    pub se_eta_fixed: Option<f64>,
    /// Standard error of `eta` (Guide p. 208).
    pub se_eta: Option<f64>,
}

/// Values needed to score one record.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreInput {
    /// Covariate values in training `fixed` covariate order.
    pub numeric: Vec<f64>,
    /// Factor levels in training `fixed` factor order.
    pub factors: Vec<String>,
    /// Subject keys per random spec, same order as training `random`.
    pub subjects: Vec<Vec<String>>,
    /// Offset for this record.
    pub offset: f64,
}

/// Fitted GLMM: estimates, tests, information criteria, and scoring state.
#[derive(Debug, Clone, PartialEq)]
pub struct GlmmFit {
    /// Fixed-effect parameter table.
    pub parameters: Vec<ParameterEstimate>,
    /// G-side variance components, then the residual.
    pub variance_components: Vec<VarianceComponent>,
    /// Corrected-model F, then one Type III F per fixed term.
    pub f_tests: Vec<FTest>,
    /// −2 restricted log-pseudo-likelihood on the original scale.
    pub minus2_log_likelihood: f64,
    /// Finite-sample corrected AIC (Guide p. 203).
    pub aicc: f64,
    /// BIC (Guide p. 203).
    pub bic: f64,
    /// Complete-case count n.
    pub n: usize,
    /// Effective sample size N = Σ f_i.
    pub n_effective: f64,
    /// Number of non-redundant fixed-effect columns.
    pub px: usize,
    /// Residual degrees of freedom used for t and F.
    pub residual_df: f64,
    /// Inner iterations of the last outer step.
    pub iterations: usize,
    /// Outer PQL iterations.
    pub outer_iterations: usize,
    /// Inner parameter convergence of the last step.
    pub converged: bool,
    /// Random-intercept BLUPs.
    pub blups: Vec<RandomPrediction>,
    beta: Vec<f64>,
    c11: Vec<f64>,
    c21: Vec<f64>,
    c22: Vec<f64>,
    q: usize,
    p: usize,
    columns: Vec<Column>,
    subject_lookup: Vec<std::collections::HashMap<String, usize>>,
    blup_by_q: Vec<f64>,
    link: Link,
}

/// Train a GLMM. Hostile input is [`AlgoError`], never a panic.
pub fn fit(input: &GlmmInput, settings: &GlmmSettings) -> Result<GlmmFit, AlgoError> {
    crate::check_cancelled()?;
    validate_settings(settings)?;
    let data = Prepared::read(input, settings)?;
    let mut mu = initial_mu(&data, settings)?;
    let mut theta = starting_theta(&data)?;
    let mut inner_iters = 0usize;
    let mut converged = false;
    let mut outer = 0usize;
    let mut last_work: Option<InnerWork> = None;
    let max_outer = settings.max_outer_iterations.min(MAX_ITERATIONS_CAP);
    for j in 0..max_outer {
        crate::check_cancelled()?;
        outer = j.saturating_add(1);
        let (v, w) = pseudo_response(&data, &mu, settings)?;
        let (next, work) = fit_lmm(&data, &v, &w, &theta, settings)?;
        inner_iters = work.iterations;
        converged = work.converged;
        if j > 0 && outer_converged(&theta, &next) {
            theta = next;
            last_work = Some(work);
            break;
        }
        theta = next;
        mu = update_mu(&data, &work, settings)?;
        last_work = Some(work);
        if identity_normal(settings) {
            break;
        }
    }
    let work = last_work.ok_or_else(|| AlgoError::Data("GLMM produced no inner fit".into()))?;
    finish_fit(
        &data,
        &work,
        &theta,
        settings,
        inner_iters,
        outer,
        converged,
    )
}

impl GlmmFit {
    /// Predict the mean for one record, with BLUPs when the subject is known.
    pub fn score(&self, input: &ScoreInput) -> Result<Score, AlgoError> {
        crate::check_cancelled()?;
        let x = score_row(self, input)?;
        let mut eta_fixed = 0.0;
        for (a, b) in x.iter().zip(&self.beta) {
            eta_fixed += *a * *b;
        }
        eta_fixed += input.offset;
        if !eta_fixed.is_finite() {
            return Err(AlgoError::Data(
                "GLMM linear predictor is not finite".into(),
            ));
        }
        let se_fixed = quad_form(&self.c11, &x, self.p)?.max(0.0).sqrt();
        let mut eta = eta_fixed;
        let mut z = vec![0.0; self.q];
        for (spec, keys) in input.subjects.iter().enumerate() {
            let Some(map) = self.subject_lookup.get(spec) else {
                continue;
            };
            let key = join_key(keys);
            if let Some(&qi) = map.get(&key)
                && let Some(slot) = z.get_mut(qi)
            {
                *slot = 1.0;
                if let Some(g) = self.blup_by_q.get(qi) {
                    eta += *g;
                }
            }
        }
        let se_eta = if self.q == 0 {
            Some(se_fixed)
        } else {
            let zc22z = quad_form(&self.c22, &z, self.q)?;
            let mut zc21x = 0.0;
            for i in 0..self.q {
                for j in 0..self.p {
                    zc21x += at(&z, i)? * at(&self.c21, mat_index(i, j, self.p))? * at(&x, j)?;
                }
            }
            let v = se_fixed * se_fixed + zc22z + 2.0 * zc21x;
            if v.is_finite() && v >= 0.0 {
                Some(v.sqrt())
            } else {
                None
            }
        };
        let mu = mu_of(eta, self.link).ok_or_else(|| {
            AlgoError::Data("GLMM inverse link of the linear predictor is not finite".into())
        })?;
        Ok(Score {
            eta_fixed,
            eta,
            mu,
            se_eta_fixed: if se_fixed.is_finite() {
                Some(se_fixed)
            } else {
                None
            },
            se_eta,
        })
    }

    /// Predict using only the fixed effects (random effects at 0).
    pub fn score_fixed(&self, input: &ScoreInput) -> Result<Score, AlgoError> {
        let mut zeroed = input.clone();
        zeroed.subjects = input
            .subjects
            .iter()
            .map(|k| vec![String::new(); k.len()])
            .collect();
        for keys in &mut zeroed.subjects {
            for key in keys.iter_mut() {
                key.clear();
            }
        }
        let mut s = self.score(&zeroed)?;
        s.eta = s.eta_fixed;
        s.se_eta = s.se_eta_fixed;
        s.mu = mu_of(s.eta, self.link).ok_or_else(|| {
            AlgoError::Data("GLMM inverse link of the linear predictor is not finite".into())
        })?;
        Ok(s)
    }
}

fn identity_normal(settings: &GlmmSettings) -> bool {
    matches!(
        (settings.distribution, settings.link),
        (Distribution::Normal, Link::Identity)
    )
}

fn validate_settings(settings: &GlmmSettings) -> Result<(), AlgoError> {
    if matches!(
        settings.residual,
        CovarianceType::Ar1
            | CovarianceType::Arma11
            | CovarianceType::Toeplitz
            | CovarianceType::Unstructured
            | CovarianceType::CompoundSymmetry
    ) {
        return Err(AlgoError::Unsupported(format!(
            "GAP: GLMM residual covariance {:?}",
            settings.residual
        )));
    }
    if !matches!(
        settings.residual,
        CovarianceType::Identity | CovarianceType::VarianceComponents | CovarianceType::Diagonal
    ) {
        return Err(AlgoError::Unsupported(format!(
            "GAP: GLMM residual covariance {:?}",
            settings.residual
        )));
    }
    if !settings.parameter_convergence.is_finite() || settings.parameter_convergence < 0.0 {
        return Err(AlgoError::Settings(
            "GLMM parameter convergence must be a non-negative finite number".into(),
        ));
    }
    if !settings.singularity.is_finite() || settings.singularity < 0.0 {
        return Err(AlgoError::Settings(
            "GLMM singularity tolerance must be a non-negative finite number".into(),
        ));
    }
    if !(settings.confidence > 0.0 && settings.confidence < 100.0) {
        return Err(AlgoError::Settings(
            "GLMM confidence level must be in (0, 100)".into(),
        ));
    }
    if settings.max_iterations > MAX_ITERATIONS_CAP
        || settings.max_outer_iterations > MAX_ITERATIONS_CAP
    {
        return Err(AlgoError::Settings(
            "GLMM max_iterations is too large".into(),
        ));
    }
    if settings.max_step_halving > 64 {
        return Err(AlgoError::Settings(
            "GLMM step-halving limit is too large".into(),
        ));
    }
    if let Distribution::Negbin { k } = settings.distribution
        && !(k.is_finite() && k >= 0.0)
    {
        return Err(AlgoError::Settings(
            "GLMM negative-binomial k must be a non-negative finite number".into(),
        ));
    }
    if let Link::Power { alpha } = settings.link
        && !alpha.is_finite()
    {
        return Err(AlgoError::Settings(
            "GLMM power link parameter must be finite".into(),
        ));
    }
    if !combination_ok(settings.distribution, settings.link) {
        return Err(AlgoError::Settings(format!(
            "GLMM distribution {:?} with link {:?} is not a supported combination",
            settings.distribution, settings.link
        )));
    }
    Ok(())
}

fn combination_ok(distribution: Distribution, link: Link) -> bool {
    match distribution {
        Distribution::Normal
        | Distribution::InverseGaussian
        | Distribution::Gamma
        | Distribution::Poisson
        | Distribution::Negbin { .. } => {
            matches!(
                link,
                Link::Identity | Link::Log | Link::Power { .. } | Link::Negbin { .. }
            )
        }
        Distribution::Binomial => matches!(
            link,
            Link::Identity
                | Link::Log
                | Link::Logit
                | Link::Probit
                | Link::Cloglog
                | Link::Logc
                | Link::Nloglog
                | Link::Power { .. }
        ),
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Column {
    kind: ColumnKind,
    parameter_name: String,
    label: String,
    effect: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum ColumnKind {
    Intercept,
    Covariate {
        index: usize,
        name: String,
    },
    Indicator {
        factor: usize,
        level: String,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct Effect {
    name: String,
    columns: Vec<usize>,
}

struct Subjects {
    index: Vec<Vec<usize>>,
    counts: Vec<usize>,
    keys: Vec<Vec<Vec<String>>>,
    offsets: Vec<usize>,
    q: usize,
    n_specs: usize,
}

struct Prepared {
    y: Vec<f64>,
    offset: Vec<f64>,
    freq: Vec<f64>,
    weight: Vec<f64>,
    trials: Vec<f64>,
    x: Vec<Vec<f64>>,
    columns: Vec<Column>,
    effects: Vec<Effect>,
    covariate_cols: Vec<usize>,
    subjects: Subjects,
    n: usize,
    p: usize,
    n_effective: f64,
}

impl Prepared {
    fn read(input: &GlmmInput, settings: &GlmmSettings) -> Result<Self, AlgoError> {
        let n_in = input.y.len();
        if n_in == 0 {
            return Err(AlgoError::Data("GLMM has no records".into()));
        }
        if n_in > MAX_RECORDS {
            return Err(AlgoError::Data(format!(
                "GLMM is limited to {MAX_RECORDS} records"
            )));
        }
        if input.random.len() > MAX_SPECS {
            return Err(AlgoError::Data(format!(
                "GLMM is limited to {MAX_SPECS} random-effect specifications"
            )));
        }
        check_optional_len(input.weight.as_ref(), n_in, "weight")?;
        check_optional_len(input.frequency.as_ref(), n_in, "frequency")?;
        check_optional_len(input.offset.as_ref(), n_in, "offset")?;
        check_optional_len(input.trials.as_ref(), n_in, "trials")?;
        for term in &input.fixed {
            match term {
                FixedTerm::Covariate(c) => {
                    if c.values.len() != n_in {
                        return Err(AlgoError::Data(format!(
                            "GLMM covariate {} length does not match the target",
                            c.name
                        )));
                    }
                }
                FixedTerm::Factor(f) => {
                    if f.levels.len() != n_in {
                        return Err(AlgoError::Data(format!(
                            "GLMM factor {} length does not match the target",
                            f.name
                        )));
                    }
                }
            }
        }
        for spec in &input.random {
            if spec.subject_keys.len() != n_in {
                return Err(AlgoError::Data(
                    "GLMM random-effect subject keys length does not match the target".into(),
                ));
            }
            if !spec.slopes.is_empty() {
                return Err(AlgoError::Unsupported("GAP: GLMM random slopes".into()));
            }
            if !spec.intercept {
                return Err(AlgoError::Unsupported(
                    "GAP: GLMM random-effect specification without an intercept".into(),
                ));
            }
            match spec.covariance {
                CovarianceType::VarianceComponents
                | CovarianceType::Identity
                | CovarianceType::Diagonal => {}
                other => {
                    return Err(AlgoError::Unsupported(format!(
                        "GAP: GLMM random-effect covariance {other:?}"
                    )));
                }
            }
            for key in &spec.subject_keys {
                if key.len() != spec.subject_names.len() {
                    return Err(AlgoError::Data(
                        "GLMM subject key does not match the subject field list".into(),
                    ));
                }
            }
        }

        let keep = keep_mask(input, settings, n_in)?;
        let n = keep.iter().filter(|k| **k).count();
        if n == 0 {
            return Err(AlgoError::Data("no complete records to train GLMM".into()));
        }
        if n > MAX_RECORDS {
            return Err(AlgoError::Data(format!(
                "GLMM is limited to {MAX_RECORDS} records"
            )));
        }

        let y = take_kept(&input.y, &keep, n)?;
        let offset = match &input.offset {
            Some(v) => take_kept(v, &keep, n)?,
            None => vec![0.0; n],
        };
        let freq = match &input.frequency {
            Some(v) => {
                let mut out = Vec::with_capacity(n);
                for (i, keep_i) in keep.iter().enumerate() {
                    if *keep_i {
                        let raw = at(v, i)?;
                        out.push(raw.round());
                    }
                }
                out
            }
            None => vec![1.0; n],
        };
        let weight = match &input.weight {
            Some(v) => take_kept(v, &keep, n)?,
            None => vec![1.0; n],
        };
        let trials = match &input.trials {
            Some(v) => take_kept(v, &keep, n)?,
            None => vec![1.0; n],
        };
        let n_effective: f64 = freq.iter().sum();
        if !n_effective.is_finite() || n_effective <= 0.0 {
            return Err(AlgoError::Data(
                "GLMM effective sample size is not positive".into(),
            ));
        }

        let (columns, effects, covariate_cols) = build_columns(input, settings, &keep, n)?;
        if columns.len() > MAX_PARAMETERS {
            return Err(AlgoError::Data(format!(
                "GLMM would estimate {} parameters",
                columns.len()
            )));
        }
        let p = columns.len();
        if p == 0 {
            return Err(AlgoError::Settings(
                "GLMM has no parameters to estimate".into(),
            ));
        }
        let x = build_x(input, &columns, &keep, n, p)?;
        let subjects = build_subjects(&input.random, &keep, n)?;
        Ok(Prepared {
            y,
            offset,
            freq,
            weight,
            trials,
            x,
            columns,
            effects,
            covariate_cols,
            subjects,
            n,
            p,
            n_effective,
        })
    }
}

fn check_optional_len(v: Option<&Vec<f64>>, n: usize, what: &str) -> Result<(), AlgoError> {
    if let Some(v) = v
        && v.len() != n
    {
        return Err(AlgoError::Data(format!(
            "GLMM {what} length does not match the target"
        )));
    }
    Ok(())
}

fn keep_mask(input: &GlmmInput, settings: &GlmmSettings, n: usize) -> Result<Vec<bool>, AlgoError> {
    let mut keep = vec![true; n];
    for i in 0..n {
        crate::check_cancelled()?;
        let mut ok = true;
        let y = at(&input.y, i)?;
        if !y.is_finite() {
            ok = false;
        }
        match settings.distribution {
            Distribution::Gamma | Distribution::InverseGaussian if ok && y <= 0.0 => ok = false,
            Distribution::Poisson | Distribution::Negbin { .. } if ok && y < 0.0 => ok = false,
            Distribution::Binomial if ok && !(0.0..=1.0).contains(&y) && input.trials.is_none() => {
                ok = false;
            }
            _ => {}
        }
        if let Some(w) = &input.weight {
            let wi = at(w, i)?;
            if !wi.is_finite() || wi <= 0.0 {
                ok = false;
            }
        }
        if let Some(f) = &input.frequency {
            let fi = at(f, i)?;
            if !fi.is_finite() || fi.round() < 0.5 {
                ok = false;
            }
        }
        if let Some(o) = &input.offset
            && !at(o, i)?.is_finite()
        {
            ok = false;
        }
        if let Some(t) = &input.trials {
            let m = at(t, i)?;
            if !(m.is_finite() && m.round() >= 1.0) {
                ok = false;
            }
        }
        for term in &input.fixed {
            match term {
                FixedTerm::Covariate(c) => {
                    if !at(&c.values, i)?.is_finite() {
                        ok = false;
                    }
                }
                FixedTerm::Factor(f) => {
                    let level = f.levels.get(i).map(String::as_str).unwrap_or("");
                    if level.trim().is_empty() {
                        ok = false;
                    }
                }
            }
        }
        for spec in &input.random {
            let key = spec.subject_keys.get(i);
            let Some(key) = key else {
                ok = false;
                break;
            };
            if key.iter().any(|s| s.trim().is_empty()) {
                ok = false;
            }
        }
        *slot(&mut keep, i)? = ok;
    }
    Ok(keep)
}

fn take_kept(src: &[f64], keep: &[bool], n: usize) -> Result<Vec<f64>, AlgoError> {
    let mut out = Vec::with_capacity(n);
    for (i, k) in keep.iter().enumerate() {
        if *k {
            out.push(at(src, i)?);
        }
    }
    Ok(out)
}

#[allow(
    clippy::type_complexity,
    reason = "columns, effects, covariate indices"
)]
fn build_columns(
    input: &GlmmInput,
    settings: &GlmmSettings,
    keep: &[bool],
    n: usize,
) -> Result<(Vec<Column>, Vec<Effect>, Vec<usize>), AlgoError> {
    let mut columns = Vec::new();
    let mut effects = Vec::new();
    let mut next = 1usize;
    let mut covariate_cols = Vec::new();
    let mut factor_index = 0usize;
    let mut cov_index = 0usize;

    if settings.intercept {
        let effect = effects.len();
        effects.push(Effect {
            name: "(Intercept)".into(),
            columns: Vec::new(),
        });
        push_column(
            &mut columns,
            &mut next,
            ColumnKind::Intercept,
            "Intercept".into(),
            effect,
        );
    }

    for term in &input.fixed {
        crate::check_cancelled()?;
        match term {
            FixedTerm::Covariate(c) => {
                let effect = effects.len();
                effects.push(Effect {
                    name: c.name.clone(),
                    columns: Vec::new(),
                });
                covariate_cols.push(columns.len());
                push_column(
                    &mut columns,
                    &mut next,
                    ColumnKind::Covariate {
                        index: cov_index,
                        name: c.name.clone(),
                    },
                    c.name.clone(),
                    effect,
                );
                cov_index = cov_index.saturating_add(1);
            }
            FixedTerm::Factor(f) => {
                let cats = factor_categories(&f.levels, keep, n, settings.value_order)?;
                if cats.is_empty() || cats.len() > MAX_CATEGORIES {
                    return Err(AlgoError::Data(format!(
                        "GLMM factor {} has no usable categories",
                        f.name
                    )));
                }
                let effect = effects.len();
                effects.push(Effect {
                    name: f.name.clone(),
                    columns: Vec::new(),
                });
                for cat in &cats {
                    push_column(
                        &mut columns,
                        &mut next,
                        ColumnKind::Indicator {
                            factor: factor_index,
                            level: cat.clone(),
                            name: f.name.clone(),
                        },
                        format!("[{}={}]", f.name, cat),
                        effect,
                    );
                }
                factor_index = factor_index.saturating_add(1);
            }
        }
    }

    for (ei, effect) in effects.iter_mut().enumerate() {
        effect.columns = columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.effect == ei)
            .map(|(i, _)| i)
            .collect();
    }
    Ok((columns, effects, covariate_cols))
}

fn factor_categories(
    levels: &[String],
    keep: &[bool],
    n: usize,
    order: ValueOrder,
) -> Result<Vec<String>, AlgoError> {
    let mut cats = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for i in 0..levels.len() {
        if keep.get(i) != Some(&true) {
            continue;
        }
        let text = levels.get(i).map(String::as_str).unwrap_or("").trim();
        if text.is_empty() {
            continue;
        }
        if seen.insert(text.to_string()) {
            cats.push(text.to_string());
            if cats.len() > MAX_CATEGORIES {
                break;
            }
        }
        if cats.len() >= n.saturating_add(1) {
            break;
        }
    }
    match order {
        ValueOrder::DataOrder => {}
        ValueOrder::Ascending => cats.sort_by(|a, b| cmp_category(a, b)),
        ValueOrder::Descending => cats.sort_by(|a, b| cmp_category(b, a)),
    }
    Ok(cats)
}

fn cmp_category(left: &str, right: &str) -> std::cmp::Ordering {
    match (left.parse::<f64>(), right.parse::<f64>()) {
        (Ok(a), Ok(b)) => a.total_cmp(&b),
        _ => left.cmp(right),
    }
}

fn build_x(
    input: &GlmmInput,
    columns: &[Column],
    keep: &[bool],
    n: usize,
    p: usize,
) -> Result<Vec<Vec<f64>>, AlgoError> {
    let mut x = vec![vec![0.0; p]; n];
    let mut row = 0usize;
    for (i, keep_i) in keep.iter().enumerate() {
        if !*keep_i {
            continue;
        }
        crate::check_cancelled()?;
        for (j, column) in columns.iter().enumerate() {
            let value = match &column.kind {
                ColumnKind::Intercept => 1.0,
                ColumnKind::Covariate { name, .. } => {
                    let col = input.fixed.iter().find_map(|t| match t {
                        FixedTerm::Covariate(c) if c.name == *name => Some(c),
                        _ => None,
                    });
                    let col =
                        col.ok_or_else(|| AlgoError::Data("GLMM covariate is missing".into()))?;
                    at(&col.values, i)?
                }
                ColumnKind::Indicator { name, level, .. } => {
                    let col = input.fixed.iter().find_map(|t| match t {
                        FixedTerm::Factor(f) if f.name == *name => Some(f),
                        _ => None,
                    });
                    let col =
                        col.ok_or_else(|| AlgoError::Data("GLMM factor is missing".into()))?;
                    let text = col.levels.get(i).map(String::as_str).unwrap_or("");
                    if same_category(text, level) { 1.0 } else { 0.0 }
                }
            };
            *slot(slot(&mut x, row)?, j)? = value;
        }
        row = row.saturating_add(1);
    }
    if row != n {
        return Err(AlgoError::Data(
            "GLMM design row count does not match n".into(),
        ));
    }
    Ok(x)
}

fn same_category(left: &str, right: &str) -> bool {
    left == right
        || matches!(
            (left.parse::<f64>(), right.parse::<f64>()),
            (Ok(a), Ok(b)) if a == b
        )
}

fn build_subjects(random: &[RandomSpec], keep: &[bool], n: usize) -> Result<Subjects, AlgoError> {
    let n_specs = random.len();
    let mut index = vec![vec![0usize; n]; n_specs];
    let mut keys: Vec<Vec<Vec<String>>> = vec![Vec::new(); n_specs];
    let mut counts = vec![0usize; n_specs];
    for (k, spec) in random.iter().enumerate() {
        crate::check_cancelled()?;
        let mut map = std::collections::HashMap::new();
        let mut row = 0usize;
        for (i, keep_i) in keep.iter().enumerate() {
            if !*keep_i {
                continue;
            }
            let rec = spec
                .subject_keys
                .get(i)
                .ok_or_else(|| AlgoError::Data("GLMM subject key is missing".into()))?;
            let joined = join_key(rec);
            let next_id = map.len();
            let id = *map.entry(joined).or_insert(next_id);
            if id == next_id {
                let list = slot(&mut keys, k)?;
                list.push(rec.clone());
                if list.len() > MAX_RANDOM_LEVELS {
                    return Err(AlgoError::Data(format!(
                        "GLMM is limited to {MAX_RANDOM_LEVELS} subjects in a random-effect specification"
                    )));
                }
            }
            *slot(slot(&mut index, k)?, row)? = id;
            row = row.saturating_add(1);
        }
        *slot(&mut counts, k)? = map.len();
    }
    let mut offsets = vec![0usize; n_specs];
    let mut q = 0usize;
    for k in 0..n_specs {
        *slot(&mut offsets, k)? = q;
        q = q
            .checked_add(at(&counts, k)?)
            .ok_or_else(|| AlgoError::Data("GLMM random-effect size overflow".into()))?;
    }
    if q > MAX_RANDOM_LEVELS {
        return Err(AlgoError::Data(format!(
            "GLMM is limited to {MAX_RANDOM_LEVELS} random-effect levels"
        )));
    }
    Ok(Subjects {
        index,
        counts,
        keys,
        offsets,
        q,
        n_specs,
    })
}

fn join_key(parts: &[String]) -> String {
    parts.join("\u{1f}")
}

fn initial_mu(data: &Prepared, settings: &GlmmSettings) -> Result<Vec<f64>, AlgoError> {
    let mut mu = Vec::with_capacity(data.n);
    for i in 0..data.n {
        crate::check_cancelled()?;
        let y = at(&data.y, i)?;
        let m = at(&data.trials, i)?;
        let value = if matches!(settings.distribution, Distribution::Binomial) {
            let mm = if m > 0.0 { m } else { 1.0 };
            (y * mm + 0.5) / (mm + 1.0)
        } else {
            y
        };
        if !value.is_finite() {
            return Err(AlgoError::Data("GLMM initial mean is not finite".into()));
        }
        mu.push(value);
    }
    Ok(mu)
}

fn starting_theta(data: &Prepared) -> Result<Vec<f64>, AlgoError> {
    let n_theta = data.subjects.n_specs.saturating_add(1);
    Ok(vec![1.0; n_theta])
}

fn pseudo_response(
    data: &Prepared,
    mu: &[f64],
    settings: &GlmmSettings,
) -> Result<(Vec<f64>, Vec<f64>), AlgoError> {
    let mut v = vec![0.0; data.n];
    let mut w = vec![0.0; data.n];
    for i in 0..data.n {
        crate::check_cancelled()?;
        let y = at(&data.y, i)?;
        let mui = at(mu, i)?;
        let off = at(&data.offset, i)?;
        let freq = at(&data.freq, i)?;
        let omega = at(&data.weight, i)?;
        let trials = at(&data.trials, i)?;
        let g = g_of(mui, settings.link)
            .ok_or_else(|| AlgoError::Data("GLMM link of the current mean is not finite".into()))?;
        let g1 = g1_of(mui, settings.link).ok_or_else(|| {
            AlgoError::Data("GLMM link derivative of the current mean is not finite".into())
        })?;
        if g1.abs() < MU_FLOOR {
            return Err(AlgoError::Data("GLMM link derivative is zero".into()));
        }
        let var = variance(mui, settings.distribution).ok_or_else(|| {
            AlgoError::Data("GLMM variance function of the current mean is not finite".into())
        })?;
        let omega_eff = if matches!(settings.distribution, Distribution::Binomial) {
            omega * trials.max(1.0)
        } else {
            omega
        };
        let wi = freq * omega_eff / (var * g1 * g1);
        if !wi.is_finite() || wi <= 0.0 {
            return Err(AlgoError::Data(
                "GLMM working weight is not positive".into(),
            ));
        }
        let vi = g - off + g1 * (y - mui);
        if !vi.is_finite() {
            return Err(AlgoError::Data("GLMM pseudo-response is not finite".into()));
        }
        *slot(&mut v, i)? = vi;
        *slot(&mut w, i)? = wi;
    }
    Ok((v, w))
}

fn outer_converged(prev: &[f64], next: &[f64]) -> bool {
    if prev.len() != next.len() {
        return false;
    }
    let mut worst = 0.0;
    for (a, b) in prev.iter().zip(next) {
        let den = a.abs() + b.abs();
        let rel = if den > 0.0 {
            2.0 * (b - a).abs() / den
        } else {
            0.0
        };
        if rel > worst {
            worst = rel;
        }
    }
    worst < 1e-6
}

struct InnerWork {
    beta: Vec<f64>,
    c11: Vec<f64>,
    c21: Vec<f64>,
    c22: Vec<f64>,
    swept: Vec<bool>,
    px: usize,
    u: Vec<f64>,
    z_scale: Vec<f64>,
    x_star: Vec<Vec<f64>>,
    minus2: f64,
    iterations: usize,
    converged: bool,
    hess_inv2: Vec<f64>,
    blup: Vec<f64>,
    blup_se: Vec<Option<f64>>,
}

#[allow(clippy::type_complexity, reason = "scaled design plus centre/scale")]
fn transform_x(data: &Prepared) -> Result<(Vec<Vec<f64>>, Vec<f64>, Vec<f64>, f64), AlgoError> {
    let p = data.p;
    let n = data.n;
    let mut center = vec![0.0; p];
    let mut scale = vec![1.0; p];
    let n_eff = data.n_effective;
    for &j in &data.covariate_cols {
        crate::check_cancelled()?;
        let mut sum = 0.0;
        let mut sumw = 0.0;
        for i in 0..n {
            let xi = at2(&data.x, i, j)?;
            let f = at(&data.freq, i)?;
            sum += f * xi;
            sumw += f;
        }
        if sumw <= 0.0 {
            return Err(AlgoError::Data(
                "GLMM covariate weight sum is not positive".into(),
            ));
        }
        let mean = sum / sumw;
        let mut ss = 0.0;
        for i in 0..n {
            let xi = at2(&data.x, i, j)?;
            let f = at(&data.freq, i)?;
            let d = xi - mean;
            ss += f * d * d;
        }
        let intercept = data
            .columns
            .first()
            .is_some_and(|c| matches!(c.kind, ColumnKind::Intercept));
        if intercept {
            *slot(&mut center, j)? = mean;
            let var = if n_eff > 1.0 { ss / (n_eff - 1.0) } else { ss };
            let s = var.max(0.0).sqrt();
            if s > 0.0 && s.is_finite() {
                *slot(&mut scale, j)? = s;
            } else if mean.abs() > 0.0 {
                *slot(&mut center, j)? = 0.0;
                *slot(&mut scale, j)? = mean.abs();
            }
        } else {
            *slot(&mut center, j)? = 0.0;
            let var = if n_eff > 1.0 { ss / (n_eff - 1.0) } else { ss };
            let s = (var + mean * mean).max(0.0).sqrt();
            if s > 0.0 && s.is_finite() {
                *slot(&mut scale, j)? = s;
            } else if mean.abs() > 0.0 {
                *slot(&mut scale, j)? = mean.abs();
            }
        }
    }
    let mut x_star = vec![vec![0.0; p]; n];
    for i in 0..n {
        for j in 0..p {
            let xi = at2(&data.x, i, j)?;
            let c = at(&center, j)?;
            let s = at(&scale, j)?;
            if s.abs() < MU_FLOOR {
                return Err(AlgoError::Data("GLMM covariate scale is zero".into()));
            }
            *slot(slot(&mut x_star, i)?, j)? = (xi - c) / s;
        }
    }
    let mut ln_a = 0.0;
    for &j in &data.covariate_cols {
        let s = at(&scale, j)?;
        if s > 0.0 {
            ln_a += s.ln();
        }
    }
    // |A| = Π 1/s_j so ln|A| = −Σ ln s_j. Stored as Σ ln s_j = −ln|A|.
    Ok((x_star, center, scale, ln_a))
}

#[allow(
    clippy::type_complexity,
    reason = "GLS-scaled response, X, and Z scale"
)]
fn gls_scale(
    v: &[f64],
    x: &[Vec<f64>],
    w: &[f64],
    n: usize,
    p: usize,
) -> Result<(Vec<f64>, Vec<Vec<f64>>, Vec<f64>), AlgoError> {
    // scaled v, scaled X, z_scale
    let mut v_star = vec![0.0; n];
    let mut x_star = vec![vec![0.0; p]; n];
    let mut z_scale = vec![0.0; n];
    for i in 0..n {
        let wi = at(w, i)?;
        if !wi.is_finite() || wi <= 0.0 {
            return Err(AlgoError::Data(
                "GLMM working weight is not positive".into(),
            ));
        }
        let s = wi.sqrt();
        *slot(&mut z_scale, i)? = s;
        *slot(&mut v_star, i)? = at(v, i)? * s;
        for j in 0..p {
            *slot(slot(&mut x_star, i)?, j)? = at2(x, i, j)? * s;
        }
    }
    Ok((v_star, x_star, z_scale))
}

fn fit_lmm(
    data: &Prepared,
    v: &[f64],
    w: &[f64],
    start: &[f64],
    settings: &GlmmSettings,
) -> Result<(Vec<f64>, InnerWork), AlgoError> {
    let (x_t, center, scale, ln_a) = transform_x(data)?;
    let (v_star, x_star, z_scale) = gls_scale(v, &x_t, w, data.n, data.p)?;
    let n_theta = data.subjects.n_specs.saturating_add(1);
    let mut theta = start.to_vec();
    if theta.len() != n_theta {
        theta = vec![1.0; n_theta];
    }
    for t in &mut theta {
        if !t.is_finite() || *t < VAR_FLOOR {
            *t = 1.0;
        }
    }
    let ols = reml_eval(
        data,
        &v_star,
        &x_star,
        &z_scale,
        &dummy_theta_residual(data, 1.0)?,
    )?;
    let phi0 = if ols.px < data.n {
        (ols.quad / (data.n_effective - ols.px as f64)).max(VAR_FLOOR)
    } else {
        1.0
    };
    if start.iter().all(|t| (*t - 1.0).abs() < 1e-15) {
        theta = moment_start(data, &v_star, &x_star, &z_scale, phi0)?;
    }
    let mut last: Option<RemlEval> = None;
    let mut converged = false;
    let mut iters = 0usize;
    let max_it = settings.max_iterations.min(MAX_ITERATIONS_CAP);
    for it in 0..max_it {
        crate::check_cancelled()?;
        iters = it.saturating_add(1);
        let eval = reml_eval(data, &v_star, &x_star, &z_scale, &theta)?;
        let fisher = it < settings.max_fisher_iterations;
        let (grad, mut hess) =
            reml_grad_hess(data, &v_star, &x_star, &z_scale, &theta, &eval, fisher)?;
        if !spd_add_ridge(&mut hess, n_theta, 1e-10) {
            let (_, h2) = reml_grad_hess(data, &v_star, &x_star, &z_scale, &theta, &eval, true)?;
            hess = h2;
            if !spd_add_ridge(&mut hess, n_theta, 1e-8) {
                last = Some(eval);
                break;
            }
        }
        let Some(step) = sweep_solve(&hess, &grad, n_theta, settings.singularity) else {
            last = Some(eval);
            break;
        };
        let mut alpha = 1.0;
        let mut accepted = None;
        for _ in 0..=settings.max_step_halving {
            let mut cand = theta.clone();
            let mut ok = true;
            for (c, s) in cand.iter_mut().zip(&step) {
                *c -= alpha * *s;
                if !c.is_finite() || *c < VAR_FLOOR {
                    ok = false;
                    break;
                }
            }
            if ok
                && let Ok(next) = reml_eval(data, &v_star, &x_star, &z_scale, &cand)
                && next.f <= eval.f + 1e-8 * eval.f.abs().max(1.0)
            {
                accepted = Some((cand, next));
                break;
            }
            alpha *= 0.5;
            if alpha < 1e-16 {
                break;
            }
        }
        let Some((cand, next)) = accepted else {
            last = Some(eval);
            break;
        };
        let mut dmax = 0.0;
        for (a, b) in theta.iter().zip(&cand) {
            let d = if settings.parameter_relative {
                (b - a).abs() / (1.0 + a.abs())
            } else {
                (b - a).abs()
            };
            if d > dmax {
                dmax = d;
            }
        }
        let mut l_ok = true;
        if let Some(eps) = settings.likelihood_convergence {
            let df = (next.f - eval.f).abs();
            l_ok = if settings.likelihood_relative {
                df / (1.0 + eval.f.abs()) < eps
            } else {
                df < eps
            };
        }
        theta = cand;
        last = Some(next);
        if dmax < settings.parameter_convergence && l_ok {
            converged = true;
            break;
        }
    }
    let eval = match last {
        Some(e) => e,
        None => reml_eval(data, &v_star, &x_star, &z_scale, &theta)?,
    };
    let (_, hess_obs) = reml_grad_hess(data, &v_star, &x_star, &z_scale, &theta, &eval, false)?;
    let mut hess = hess_obs;
    if !spd_add_ridge(&mut hess, n_theta, 1e-10) {
        let (_, hf) = reml_grad_hess(data, &v_star, &x_star, &z_scale, &theta, &eval, true)?;
        hess = hf;
        let _ = spd_add_ridge(&mut hess, n_theta, 1e-8);
    }
    let hess_inv = sweep_inverse(&hess, n_theta, settings.singularity)?;
    let mut hess_inv2 = vec![0.0; n_theta.saturating_mul(n_theta)];
    for i in 0..n_theta.saturating_mul(n_theta) {
        *slot(&mut hess_inv2, i)? = 2.0 * at(&hess_inv.inverse, i)?;
    }
    let (c21, c22, blup, blup_se) =
        random_moments(data, &z_scale, &theta, &eval, settings.singularity)?;
    let beta = back_transform_beta(&eval.beta, &center, &scale, data.p)?;
    let c11 = back_transform_c11(&eval.c11, &center, &scale, data.p)?;
    let c21_orig = back_transform_c21(&c21, &center, &scale, data.subjects.q, data.p)?;
    let ln_w: f64 = w.iter().map(|wi| wi.ln()).sum();
    let minus2 = eval.f + 2.0 * ln_a - ln_w;
    Ok((
        theta.clone(),
        InnerWork {
            beta,
            c11,
            c21: c21_orig,
            c22,
            swept: eval.swept.clone(),
            px: eval.px,
            u: eval.u.clone(),
            z_scale,
            x_star,
            minus2,
            iterations: iters,
            converged,
            hess_inv2,
            blup,
            blup_se,
        },
    ))
}

fn dummy_theta_residual(data: &Prepared, phi: f64) -> Result<Vec<f64>, AlgoError> {
    let mut t = vec![VAR_FLOOR; data.subjects.n_specs.saturating_add(1)];
    if let Some(last) = t.last_mut() {
        *last = phi.max(VAR_FLOOR);
    }
    Ok(t)
}

fn moment_start(
    data: &Prepared,
    v: &[f64],
    x: &[Vec<f64>],
    z_scale: &[f64],
    phi: f64,
) -> Result<Vec<f64>, AlgoError> {
    let p = data.p;
    let n = data.n;
    let mut xtx = vec![0.0; p.saturating_mul(p)];
    let mut xty = vec![0.0; p];
    for i in 0..n {
        let xi = vget(x, i)?;
        let yi = at(v, i)?;
        for a in 0..p {
            *slot(&mut xty, a)? += at(xi, a)? * yi;
            for b in 0..p {
                *slot(&mut xtx, mat_index(a, b, p))? += at(xi, a)? * at(xi, b)?;
            }
        }
    }
    let beta = sweep_solve(&xtx, &xty, p, 1e-12).unwrap_or_else(|| vec![0.0; p]);
    let mut resid = vec![0.0; n];
    for i in 0..n {
        let mut fit = 0.0;
        let xi = vget(x, i)?;
        for j in 0..p {
            fit += at(xi, j)? * at(&beta, j)?;
        }
        *slot(&mut resid, i)? = at(v, i)? - fit;
    }
    let mut theta = vec![0.0; data.subjects.n_specs.saturating_add(1)];
    for k in 0..data.subjects.n_specs {
        let ns = at(&data.subjects.counts, k)?;
        if ns == 0 {
            *slot(&mut theta, k)? = 0.1 * phi;
            continue;
        }
        let mut ss = 0.0;
        let mut wsum = 0.0;
        for s in 0..ns {
            let mut sum = 0.0;
            let mut ww = 0.0;
            for i in 0..n {
                if at2(&data.subjects.index, k, i)? == s {
                    let z = at(z_scale, i)?;
                    sum += z * z * at(&resid, i)?;
                    ww += z * z;
                }
            }
            if ww > 0.0 {
                let mean = sum / ww;
                ss += ww * mean * mean;
                wsum += 1.0;
            }
        }
        let between = if wsum > 1.0 { ss / (wsum - 1.0) } else { ss };
        let nbar = if ns > 0 {
            data.n as f64 / ns as f64
        } else {
            1.0
        };
        let mut sig = (between - phi) / nbar.max(1.0);
        if !sig.is_finite() || sig < 0.05 * phi {
            sig = 0.25 * phi;
        }
        *slot(&mut theta, k)? = sig.max(0.05 * phi);
    }
    *slot(&mut theta, data.subjects.n_specs)? = phi.max(VAR_FLOOR);
    Ok(theta)
}

struct RemlEval {
    f: f64,
    beta: Vec<f64>,
    c11: Vec<f64>,
    swept: Vec<bool>,
    px: usize,
    quad: f64,
    u: Vec<f64>,
    qx: Vec<Vec<f64>>,
    d_inv: Vec<f64>,
    s_mat: Vec<f64>,
}

struct Chol {
    l: Vec<f64>,
    n: usize,
}

fn reml_eval(
    data: &Prepared,
    v: &[f64],
    x: &[Vec<f64>],
    z_scale: &[f64],
    theta: &[f64],
) -> Result<RemlEval, AlgoError> {
    let n = data.n;
    let p = data.p;
    let q = data.subjects.q;
    let n_specs = data.subjects.n_specs;
    let phi = at(theta, n_specs)?;
    if !(phi.is_finite() && phi > 0.0) {
        return Err(AlgoError::Data(
            "GLMM residual variance is not positive".into(),
        ));
    }
    let s_mat = build_s(&data.subjects, z_scale, n)?;
    let d = build_d(&data.subjects, theta, &s_mat, phi)?;
    let chol = chol_factor(&d, q)?;
    let d_inv = chol_inverse(&chol)?;
    let ln_d = chol_logdet(&chol)?;
    let mut ln_g = 0.0;
    for k in 0..n_specs {
        let sig = at(theta, k)?;
        if !(sig.is_finite() && sig > 0.0) {
            return Err(AlgoError::Data(
                "GLMM variance component is not positive".into(),
            ));
        }
        ln_g += (at(&data.subjects.counts, k)? as f64) * sig.ln();
    }
    let ln_r = (n as f64) * phi.ln();
    let ln_det_v = ln_r + ln_g + ln_d;
    let mut qx = Vec::with_capacity(p);
    for j in 0..p {
        crate::check_cancelled()?;
        let col = column_of(x, j, n)?;
        qx.push(apply_q(&col, z_scale, &data.subjects, &d_inv, phi)?);
    }
    let mut xtvix = vec![0.0; p.saturating_mul(p)];
    let mut xty = vec![0.0; p];
    for a in 0..p {
        let qa = vget(&qx, a)?;
        for b in 0..p {
            let qb = vget(&qx, b)?;
            let mut s = 0.0;
            for i in 0..n {
                s += at2(x, i, a)? * at(qb, i)?;
            }
            *slot(&mut xtvix, mat_index(a, b, p))? = s;
        }
        let mut sy = 0.0;
        for i in 0..n {
            sy += at(qa, i)? * at(v, i)?;
        }
        *slot(&mut xty, a)? = sy;
    }
    let inv = sweep_inverse(&xtvix, p, 1e-12)?;
    let mut beta = vec![0.0; p];
    for i in 0..p {
        if at(&inv.swept, i)? {
            let mut s = 0.0;
            for j in 0..p {
                s += at(&inv.inverse, mat_index(i, j, p))? * at(&xty, j)?;
            }
            *slot(&mut beta, i)? = s;
        }
    }
    let mut r = vec![0.0; n];
    for i in 0..n {
        let mut fit = 0.0;
        for j in 0..p {
            fit += at2(x, i, j)? * at(&beta, j)?;
        }
        *slot(&mut r, i)? = at(v, i)? - fit;
    }
    let u = apply_q(&r, z_scale, &data.subjects, &d_inv, phi)?;
    let mut quad = 0.0;
    for i in 0..n {
        quad += at(&r, i)? * at(&u, i)?;
    }
    let px = inv.rank;
    let nstar = data.n_effective - px as f64;
    let f = ln_det_v + quad + inv.logdet + nstar.max(0.0) * LN_TWO_PI;
    if !f.is_finite() {
        return Err(AlgoError::Data(
            "GLMM restricted likelihood is not finite".into(),
        ));
    }
    Ok(RemlEval {
        f,
        beta,
        c11: inv.inverse,
        swept: inv.swept,
        px,
        quad,
        u,
        qx,
        d_inv,
        s_mat,
    })
}

fn build_s(subjects: &Subjects, z_scale: &[f64], n: usize) -> Result<Vec<f64>, AlgoError> {
    let q = subjects.q;
    let mut s = vec![0.0; q.saturating_mul(q.max(1))];
    if q == 0 {
        return Ok(s);
    }
    for i in 0..n {
        let zi = at(z_scale, i)?;
        let w = zi * zi;
        for k in 0..subjects.n_specs {
            let ik = at(&subjects.offsets, k)? + at2(&subjects.index, k, i)?;
            for l in 0..subjects.n_specs {
                let il = at(&subjects.offsets, l)? + at2(&subjects.index, l, i)?;
                *slot(&mut s, mat_index(ik, il, q))? += w;
            }
        }
    }
    Ok(s)
}

fn build_d(
    subjects: &Subjects,
    theta: &[f64],
    s_mat: &[f64],
    phi: f64,
) -> Result<Vec<f64>, AlgoError> {
    let q = subjects.q;
    if q == 0 {
        return Ok(Vec::new());
    }
    if phi.abs() < VAR_FLOOR {
        return Err(AlgoError::Data(
            "GLMM residual variance is too small".into(),
        ));
    }
    let mut d = vec![0.0; q.saturating_mul(q)];
    for i in 0..q.saturating_mul(q) {
        *slot(&mut d, i)? = at(s_mat, i)? / phi;
    }
    for k in 0..subjects.n_specs {
        let sig = at(theta, k)?;
        if sig.abs() < VAR_FLOOR {
            return Err(AlgoError::Data(
                "GLMM variance component is too small".into(),
            ));
        }
        let inv = 1.0 / sig;
        let off = at(&subjects.offsets, k)?;
        let nk = at(&subjects.counts, k)?;
        for s in 0..nk {
            let idx = off + s;
            *slot(&mut d, mat_index(idx, idx, q))? += inv;
        }
    }
    Ok(d)
}

fn apply_q(
    u: &[f64],
    z_scale: &[f64],
    subjects: &Subjects,
    d_inv: &[f64],
    phi: f64,
) -> Result<Vec<f64>, AlgoError> {
    let n = u.len();
    let q = subjects.q;
    if phi.abs() < VAR_FLOOR {
        return Err(AlgoError::Data(
            "GLMM residual variance is too small".into(),
        ));
    }
    let mut out = vec![0.0; n];
    if q == 0 {
        for i in 0..n {
            *slot(&mut out, i)? = at(u, i)? / phi;
        }
        return Ok(out);
    }
    let mut c = vec![0.0; q];
    for i in 0..n {
        let ui = at(u, i)?;
        let zi = at(z_scale, i)?;
        for k in 0..subjects.n_specs {
            let ik = at(&subjects.offsets, k)? + at2(&subjects.index, k, i)?;
            *slot(&mut c, ik)? += zi * ui;
        }
    }
    for c_i in &mut c {
        *c_i /= phi;
    }
    let mut xq = vec![0.0; q];
    for i in 0..q {
        let mut s = 0.0;
        for j in 0..q {
            s += at(d_inv, mat_index(i, j, q))? * at(&c, j)?;
        }
        *slot(&mut xq, i)? = s;
    }
    for i in 0..n {
        let mut zx = 0.0;
        let zi = at(z_scale, i)?;
        for k in 0..subjects.n_specs {
            let ik = at(&subjects.offsets, k)? + at2(&subjects.index, k, i)?;
            zx += at(&xq, ik)?;
        }
        *slot(&mut out, i)? = (at(u, i)? - zi * zx) / phi;
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments, reason = "REML projector kernel")]
fn apply_p(
    u: &[f64],
    z_scale: &[f64],
    subjects: &Subjects,
    d_inv: &[f64],
    phi: f64,
    qx: &[Vec<f64>],
    c11: &[f64],
    x: &[Vec<f64>],
    p: usize,
) -> Result<Vec<f64>, AlgoError> {
    let n = u.len();
    let qu = apply_q(u, z_scale, subjects, d_inv, phi)?;
    let mut xtqu = vec![0.0; p];
    for j in 0..p {
        let mut s = 0.0;
        let qj = vget(qx, j)?;
        for i in 0..n {
            s += at(qj, i)? * at(u, i)?;
        }
        *slot(&mut xtqu, j)? = s;
    }
    let mut bxt = vec![0.0; p];
    for i in 0..p {
        let mut s = 0.0;
        for j in 0..p {
            s += at(c11, mat_index(i, j, p))? * at(&xtqu, j)?;
        }
        *slot(&mut bxt, i)? = s;
    }
    let mut out = qu;
    for i in 0..n {
        let mut s = 0.0;
        for j in 0..p {
            s += at2(qx, j, i)? * at(&bxt, j)?;
        }
        *slot(&mut out, i)? -= s;
    }
    let _ = x;
    Ok(out)
}

fn reml_grad_hess(
    data: &Prepared,
    v: &[f64],
    x: &[Vec<f64>],
    z_scale: &[f64],
    theta: &[f64],
    eval: &RemlEval,
    fisher: bool,
) -> Result<(Vec<f64>, Vec<f64>), AlgoError> {
    let n_specs = data.subjects.n_specs;
    let n_theta = n_specs.saturating_add(1);
    let n = data.n;
    let p = data.p;
    let q = data.subjects.q;
    let phi = at(theta, n_specs)?;
    let mut ops: Vec<Vec<f64>> = Vec::with_capacity(n_theta);
    for k in 0..n_specs {
        ops.push(apply_zkzt(&eval.u, &data.subjects, z_scale, k)?);
    }
    ops.push(eval.u.clone());
    let mut pu_ops = Vec::with_capacity(n_theta);
    for op in &ops {
        pu_ops.push(apply_p(
            op,
            z_scale,
            &data.subjects,
            &eval.d_inv,
            phi,
            &eval.qx,
            &eval.c11,
            x,
            p,
        )?);
    }
    let zpz = z_p_z(data, z_scale, eval, phi)?;
    let mut traces_g = vec![0.0; n_specs];
    for k in 0..n_specs {
        let off = at(&data.subjects.offsets, k)?;
        let nk = at(&data.subjects.counts, k)?;
        let mut tr = 0.0;
        for s in 0..nk {
            tr += at(&zpz, mat_index(off + s, off + s, q.max(1)))?;
        }
        *slot(&mut traces_g, k)? = tr;
    }
    let tr_p = trace_p(data, z_scale, eval, phi)?;
    let mut grad = vec![0.0; n_theta];
    for k in 0..n_specs {
        let u_vk_u = dot(&eval.u, vget(&ops, k)?)?;
        *slot(&mut grad, k)? = at(&traces_g, k)? - u_vk_u;
    }
    let u_u = dot(&eval.u, &eval.u)?;
    *slot(&mut grad, n_specs)? = tr_p - u_u;

    let mut hess = vec![0.0; n_theta.saturating_mul(n_theta)];
    for i in 0..n_specs {
        for j in 0..n_specs {
            let offi = at(&data.subjects.offsets, i)?;
            let ni = at(&data.subjects.counts, i)?;
            let offj = at(&data.subjects.offsets, j)?;
            let nj = at(&data.subjects.counts, j)?;
            let mut frob = 0.0;
            for a in 0..ni {
                for b in 0..nj {
                    let val = at(&zpz, mat_index(offi + a, offj + b, q.max(1)))?;
                    frob += val * val;
                }
            }
            let obs = if fisher {
                frob
            } else {
                let term = 2.0 * dot(vget(&ops, i)?, vget(&pu_ops, j)?)?;
                -frob + term
            };
            *slot(&mut hess, mat_index(i, j, n_theta))? = obs;
        }
        let mut pz_e = 0.0;
        let nk = at(&data.subjects.counts, i)?;
        let _off = at(&data.subjects.offsets, i)?;
        for s in 0..nk {
            let e = indicator_column(&data.subjects, z_scale, i, s, n)?;
            let pe = apply_p(
                &e,
                z_scale,
                &data.subjects,
                &eval.d_inv,
                phi,
                &eval.qx,
                &eval.c11,
                x,
                p,
            )?;
            pz_e += dot(&pe, &pe)?;
        }
        let obs = if fisher {
            pz_e
        } else {
            -pz_e + 2.0 * dot(vget(&ops, i)?, vget(&pu_ops, n_specs)?)?
        };
        *slot(&mut hess, mat_index(i, n_specs, n_theta))? = obs;
        *slot(&mut hess, mat_index(n_specs, i, n_theta))? = obs;
    }
    let tr_p2 = trace_p2(data, z_scale, eval, phi)?;
    let obs_rr = if fisher {
        tr_p2
    } else {
        -tr_p2 + 2.0 * dot(&eval.u, vget(&pu_ops, n_specs)?)?
    };
    *slot(&mut hess, mat_index(n_specs, n_specs, n_theta))? = obs_rr;
    let _ = v;
    Ok((grad, hess))
}

fn apply_zkzt(
    u: &[f64],
    subjects: &Subjects,
    z_scale: &[f64],
    spec: usize,
) -> Result<Vec<f64>, AlgoError> {
    let n = u.len();
    let ns = at(&subjects.counts, spec)?;
    let mut sums = vec![0.0; ns];
    for i in 0..n {
        let s = at2(&subjects.index, spec, i)?;
        let zi = at(z_scale, i)?;
        *slot(&mut sums, s)? += zi * at(u, i)?;
    }
    let mut out = vec![0.0; n];
    for i in 0..n {
        let s = at2(&subjects.index, spec, i)?;
        *slot(&mut out, i)? = at(z_scale, i)? * at(&sums, s)?;
    }
    Ok(out)
}

fn indicator_column(
    subjects: &Subjects,
    z_scale: &[f64],
    spec: usize,
    subject: usize,
    n: usize,
) -> Result<Vec<f64>, AlgoError> {
    let mut e = vec![0.0; n];
    for i in 0..n {
        if at2(&subjects.index, spec, i)? == subject {
            *slot(&mut e, i)? = at(z_scale, i)?;
        }
    }
    Ok(e)
}

fn z_p_z(
    data: &Prepared,
    z_scale: &[f64],
    eval: &RemlEval,
    phi: f64,
) -> Result<Vec<f64>, AlgoError> {
    let q = data.subjects.q;
    if q == 0 {
        return Ok(Vec::new());
    }
    let n = data.n;
    let p = data.p;
    let mut zqz = vec![0.0; q.saturating_mul(q)];
    let kmat = {
        let mut k = vec![0.0; q.saturating_mul(q)];
        for i in 0..q.saturating_mul(q) {
            *slot(&mut k, i)? = at(&eval.s_mat, i)? / phi;
        }
        k
    };
    let mut kf = vec![0.0; q.saturating_mul(q)];
    for i in 0..q {
        for j in 0..q {
            let mut s = 0.0;
            for t in 0..q {
                s += at(&kmat, mat_index(i, t, q))? * at(&eval.d_inv, mat_index(t, j, q))?;
            }
            *slot(&mut kf, mat_index(i, j, q))? = s;
        }
    }
    for i in 0..q {
        for j in 0..q {
            let mut s = 0.0;
            for t in 0..q {
                s += at(&kf, mat_index(i, t, q))? * at(&kmat, mat_index(t, j, q))?;
            }
            *slot(&mut zqz, mat_index(i, j, q))? = at(&kmat, mat_index(i, j, q))? - s;
        }
    }
    let mut ux = vec![0.0; p.saturating_mul(q)];
    for spec in 0..data.subjects.n_specs {
        let off = at(&data.subjects.offsets, spec)?;
        let nk = at(&data.subjects.counts, spec)?;
        for s in 0..nk {
            let e = indicator_column(&data.subjects, z_scale, spec, s, n)?;
            for j in 0..p {
                let mut sdot = 0.0;
                let qj = vget(&eval.qx, j)?;
                for i in 0..n {
                    sdot += at(qj, i)? * at(&e, i)?;
                }
                *slot(&mut ux, mat_index(j, off + s, q))? = sdot;
            }
        }
    }
    let mut cux = vec![0.0; p.saturating_mul(q)];
    for i in 0..p {
        for j in 0..q {
            let mut s = 0.0;
            for t in 0..p {
                s += at(&eval.c11, mat_index(i, t, p))? * at(&ux, mat_index(t, j, q))?;
            }
            *slot(&mut cux, mat_index(i, j, q))? = s;
        }
    }
    let mut zpz = zqz;
    for i in 0..q {
        for j in 0..q {
            let mut s = 0.0;
            for t in 0..p {
                s += at(&ux, mat_index(t, i, q))? * at(&cux, mat_index(t, j, q))?;
            }
            *slot(&mut zpz, mat_index(i, j, q))? -= s;
        }
    }
    Ok(zpz)
}

fn trace_p(data: &Prepared, _z_scale: &[f64], eval: &RemlEval, phi: f64) -> Result<f64, AlgoError> {
    let n = data.n as f64;
    let q = data.subjects.q;
    let tr_q = if q == 0 {
        n / phi
    } else {
        let mut fs = 0.0;
        for i in 0..q {
            for j in 0..q {
                fs += at(&eval.d_inv, mat_index(i, j, q))? * at(&eval.s_mat, mat_index(j, i, q))?;
            }
        }
        n / phi - fs / (phi * phi)
    };
    let mut xtq2x = vec![0.0; data.p.saturating_mul(data.p)];
    for j in 0..data.p {
        let qj = vget(&eval.qx, j)?;
        for i in 0..data.p {
            let qi = vget(&eval.qx, i)?;
            let mut s = 0.0;
            for t in 0..data.n {
                s += at(qi, t)? * at(qj, t)?;
            }
            *slot(&mut xtq2x, mat_index(i, j, data.p))? = s;
        }
    }
    let mut tr_corr = 0.0;
    for i in 0..data.p {
        for j in 0..data.p {
            tr_corr +=
                at(&eval.c11, mat_index(i, j, data.p))? * at(&xtq2x, mat_index(j, i, data.p))?;
        }
    }
    Ok(tr_q - tr_corr)
}

fn trace_p2(data: &Prepared, z_scale: &[f64], eval: &RemlEval, phi: f64) -> Result<f64, AlgoError> {
    let n = data.n as f64;
    let q = data.subjects.q;
    let p = data.p;
    let tr_q2 = if q == 0 {
        n / (phi * phi)
    } else {
        let mut fs = 0.0;
        let mut fsfs = 0.0;
        let mut tmp = vec![0.0; q.saturating_mul(q)];
        for i in 0..q {
            for j in 0..q {
                let mut s = 0.0;
                for t in 0..q {
                    s +=
                        at(&eval.d_inv, mat_index(i, t, q))? * at(&eval.s_mat, mat_index(t, j, q))?;
                }
                *slot(&mut tmp, mat_index(i, j, q))? = s;
                fs += s;
            }
        }
        for i in 0..q {
            for j in 0..q {
                fsfs += at(&tmp, mat_index(i, j, q))? * at(&tmp, mat_index(j, i, q))?;
            }
        }
        n / (phi * phi) - 2.0 * fs / (phi * phi * phi) + fsfs / (phi * phi * phi * phi)
    };
    let mut q2x = Vec::with_capacity(p);
    for j in 0..p {
        let qj = vget(&eval.qx, j)?;
        q2x.push(apply_q(qj, z_scale, &data.subjects, &eval.d_inv, phi)?);
    }
    let mut xtq2x = vec![0.0; p.saturating_mul(p)];
    let mut xtq3x = vec![0.0; p.saturating_mul(p)];
    for i in 0..p {
        let qi = vget(&eval.qx, i)?;
        for j in 0..p {
            let qj = vget(&eval.qx, j)?;
            let q2j = vget(&q2x, j)?;
            let mut s2 = 0.0;
            let mut s3 = 0.0;
            for t in 0..data.n {
                s2 += at(qi, t)? * at(qj, t)?;
                s3 += at(qi, t)? * at(q2j, t)?;
            }
            *slot(&mut xtq2x, mat_index(i, j, p))? = s2;
            *slot(&mut xtq3x, mat_index(i, j, p))? = s3;
        }
    }
    let mut tr_b_q3 = 0.0;
    for i in 0..p {
        for j in 0..p {
            tr_b_q3 += at(&eval.c11, mat_index(i, j, p))? * at(&xtq3x, mat_index(j, i, p))?;
        }
    }
    let mut mid = vec![0.0; p.saturating_mul(p)];
    for i in 0..p {
        for j in 0..p {
            let mut s = 0.0;
            for t in 0..p {
                s += at(&eval.c11, mat_index(i, t, p))? * at(&xtq2x, mat_index(t, j, p))?;
            }
            *slot(&mut mid, mat_index(i, j, p))? = s;
        }
    }
    let mut tr_sq = 0.0;
    for i in 0..p {
        for j in 0..p {
            tr_sq += at(&mid, mat_index(i, j, p))? * at(&mid, mat_index(j, i, p))?;
        }
    }
    Ok(tr_q2 - 2.0 * tr_b_q3 + tr_sq)
}

#[allow(clippy::type_complexity, reason = "C21, C22, BLUPs, BLUP SEs")]
fn random_moments(
    data: &Prepared,
    z_scale: &[f64],
    theta: &[f64],
    eval: &RemlEval,
    _tol: f64,
) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>, Vec<Option<f64>>), AlgoError> {
    // C21, C22, BLUPs, BLUP SEs
    let q = data.subjects.q;
    let p = data.p;
    let n = data.n;
    let n_specs = data.subjects.n_specs;
    let _phi = at(theta, n_specs)?;
    if q == 0 {
        return Ok((Vec::new(), Vec::new(), Vec::new(), Vec::new()));
    }
    let mut ztvx = vec![0.0; q.saturating_mul(p)];
    for spec in 0..n_specs {
        let off = at(&data.subjects.offsets, spec)?;
        let nk = at(&data.subjects.counts, spec)?;
        for s in 0..nk {
            let e = indicator_column(&data.subjects, z_scale, spec, s, n)?;
            for j in 0..p {
                let qj = vget(&eval.qx, j)?;
                let mut sdot = 0.0;
                for i in 0..n {
                    sdot += at(&e, i)? * at(qj, i)?;
                }
                *slot(&mut ztvx, mat_index(off + s, j, p))? = sdot;
            }
        }
    }
    let mut c21 = vec![0.0; q.saturating_mul(p)];
    for i in 0..q {
        let spec = spec_of(&data.subjects, i)?;
        let gii = at(theta, spec)?;
        for j in 0..p {
            let mut s = 0.0;
            for t in 0..p {
                s += at(&ztvx, mat_index(i, t, p))? * at(&eval.c11, mat_index(t, j, p))?;
            }
            *slot(&mut c21, mat_index(i, j, p))? = -gii * s;
        }
    }
    let mut c22 = eval.d_inv.clone();
    let mut xtvizg = vec![0.0; p.saturating_mul(q)];
    for j in 0..p {
        for i in 0..q {
            let spec = spec_of(&data.subjects, i)?;
            let gii = at(theta, spec)?;
            *slot(&mut xtvizg, mat_index(j, i, q))? = at(&ztvx, mat_index(i, j, p))? * gii;
        }
    }
    for i in 0..q {
        for j in 0..q {
            let mut s = 0.0;
            for t in 0..p {
                s += at(&c21, mat_index(i, t, p))? * at(&xtvizg, mat_index(t, j, q))?;
            }
            *slot(&mut c22, mat_index(i, j, q))? -= s;
        }
    }
    let mut blup = vec![0.0; q];
    let mut zu = vec![0.0; q];
    for i in 0..n {
        let ui = at(&eval.u, i)?;
        let zi = at(z_scale, i)?;
        for k in 0..n_specs {
            let ik = at(&data.subjects.offsets, k)? + at2(&data.subjects.index, k, i)?;
            *slot(&mut zu, ik)? += zi * ui;
        }
    }
    for i in 0..q {
        let spec = spec_of(&data.subjects, i)?;
        *slot(&mut blup, i)? = at(theta, spec)? * at(&zu, i)?;
    }
    let mut blup_se = Vec::with_capacity(q);
    for i in 0..q {
        let v = at(&c22, mat_index(i, i, q))?;
        blup_se.push(if v.is_finite() && v >= 0.0 {
            Some(v.sqrt())
        } else {
            None
        });
    }
    Ok((c21, c22, blup, blup_se))
}

fn spec_of(subjects: &Subjects, qi: usize) -> Result<usize, AlgoError> {
    for k in 0..subjects.n_specs {
        let off = at(&subjects.offsets, k)?;
        let nk = at(&subjects.counts, k)?;
        if qi >= off && qi < off + nk {
            return Ok(k);
        }
    }
    Err(AlgoError::Data(
        "GLMM random-effect index is out of range".into(),
    ))
}

fn back_transform_beta(
    beta_star: &[f64],
    center: &[f64],
    scale: &[f64],
    p: usize,
) -> Result<Vec<f64>, AlgoError> {
    let mut a = identity(p)?;
    fill_a(&mut a, center, scale, p)?;
    let mut beta = vec![0.0; p];
    for i in 0..p {
        let mut s = 0.0;
        for j in 0..p {
            s += at(&a, mat_index(i, j, p))? * at(beta_star, j)?;
        }
        *slot(&mut beta, i)? = s;
    }
    Ok(beta)
}

fn back_transform_c11(
    c11_star: &[f64],
    center: &[f64],
    scale: &[f64],
    p: usize,
) -> Result<Vec<f64>, AlgoError> {
    let mut a = identity(p)?;
    fill_a(&mut a, center, scale, p)?;
    let mut tmp = vec![0.0; p.saturating_mul(p)];
    for i in 0..p {
        for j in 0..p {
            let mut s = 0.0;
            for k in 0..p {
                s += at(&a, mat_index(i, k, p))? * at(c11_star, mat_index(k, j, p))?;
            }
            *slot(&mut tmp, mat_index(i, j, p))? = s;
        }
    }
    let mut out = vec![0.0; p.saturating_mul(p)];
    for i in 0..p {
        for j in 0..p {
            let mut s = 0.0;
            for k in 0..p {
                s += at(&tmp, mat_index(i, k, p))? * at(&a, mat_index(j, k, p))?;
            }
            *slot(&mut out, mat_index(i, j, p))? = s;
        }
    }
    Ok(out)
}

fn back_transform_c21(
    c21_star: &[f64],
    center: &[f64],
    scale: &[f64],
    q: usize,
    p: usize,
) -> Result<Vec<f64>, AlgoError> {
    if q == 0 {
        return Ok(Vec::new());
    }
    let mut a = identity(p)?;
    fill_a(&mut a, center, scale, p)?;
    let mut out = vec![0.0; q.saturating_mul(p)];
    for i in 0..q {
        for j in 0..p {
            let mut s = 0.0;
            for k in 0..p {
                s += at(c21_star, mat_index(i, k, p))? * at(&a, mat_index(j, k, p))?;
            }
            *slot(&mut out, mat_index(i, j, p))? = s;
        }
    }
    Ok(out)
}

fn identity(p: usize) -> Result<Vec<f64>, AlgoError> {
    let mut a = vec![0.0; p.saturating_mul(p)];
    for i in 0..p {
        *slot(&mut a, mat_index(i, i, p))? = 1.0;
    }
    Ok(a)
}

fn fill_a(a: &mut [f64], center: &[f64], scale: &[f64], p: usize) -> Result<(), AlgoError> {
    for j in 0..p {
        let s = at(scale, j)?;
        let c = at(center, j)?;
        if (s - 1.0).abs() > 0.0 || c.abs() > 0.0 {
            if s.abs() < MU_FLOOR {
                return Err(AlgoError::Data("GLMM back-transform scale is zero".into()));
            }
            *slot(a, mat_index(j, j, p))? = 1.0 / s;
            if c.abs() > 0.0 {
                *slot(a, mat_index(0, j, p))? = -c / s;
            }
        }
    }
    Ok(())
}

fn update_mu(
    data: &Prepared,
    work: &InnerWork,
    settings: &GlmmSettings,
) -> Result<Vec<f64>, AlgoError> {
    let mut mu = vec![0.0; data.n];
    for i in 0..data.n {
        let mut eta = at(&data.offset, i)?;
        for j in 0..data.p {
            eta += at2(&data.x, i, j)? * at(&work.beta, j)?;
        }
        let zi = at(&work.z_scale, i)?;
        if data.subjects.q > 0 && zi.abs() > 0.0 {
            for k in 0..data.subjects.n_specs {
                let ik = at(&data.subjects.offsets, k)? + at2(&data.subjects.index, k, i)?;
                eta += at(&work.blup, ik)?;
            }
        }
        let m = mu_of(eta, settings.link)
            .ok_or_else(|| AlgoError::Data("GLMM updated mean is not finite".into()))?;
        *slot(&mut mu, i)? = m;
    }
    Ok(mu)
}

fn finish_fit(
    data: &Prepared,
    work: &InnerWork,
    theta: &[f64],
    settings: &GlmmSettings,
    inner_iters: usize,
    outer: usize,
    converged: bool,
) -> Result<GlmmFit, AlgoError> {
    let p = data.p;
    let n_specs = data.subjects.n_specs;
    let n_theta = n_specs.saturating_add(1);
    let mut sigma = work.c11.clone();
    if settings.covariance == CovarianceEstimator::Robust {
        sigma = robust_sigma(data, work, &sigma)?;
    }
    let se = standard_errors(&sigma, p, &work.swept)?;
    let df_res = match settings.df_method {
        DfMethod::Residual => data.n_effective - work.px as f64,
        DfMethod::Satterthwaite => data.n_effective - work.px as f64,
    };
    if !df_res.is_finite() || df_res <= 0.0 {
        return Err(AlgoError::Data(
            "GLMM residual degrees of freedom are not positive".into(),
        ));
    }
    let alpha = 1.0 - settings.confidence / 100.0;
    let tcrit = student_t_quantile(1.0 - alpha / 2.0, df_res)
        .ok_or_else(|| AlgoError::Data("GLMM t critical value is not finite".into()))?;
    let zcrit = phi_inv(1.0 - alpha / 2.0).unwrap_or(1.959963984540054);
    let mut parameters = Vec::with_capacity(p);
    for j in 0..p {
        let col = data
            .columns
            .get(j)
            .ok_or_else(|| AlgoError::Data("GLMM parameter column is missing".into()))?;
        let estimate = at(&work.beta, j)?;
        let swept = at(&work.swept, j)?;
        let std_error = if swept { at(&se, j)? } else { None };
        let t = match std_error {
            Some(s) if s > 0.0 => Some(estimate / s),
            _ => None,
        };
        let p_value = t.map(|tv| t_p_value(tv, df_res));
        let (lower, upper) = match std_error {
            Some(s) => (Some(estimate - tcrit * s), Some(estimate + tcrit * s)),
            None => (None, None),
        };
        parameters.push(ParameterEstimate {
            name: col.parameter_name.clone(),
            label: col.label.clone(),
            estimate,
            std_error,
            t,
            p_value,
            lower,
            upper,
            df: if swept { 1 } else { 0 },
            redundant: !swept,
        });
    }
    let mut variance_components = Vec::with_capacity(n_theta);
    for k in 0..n_specs {
        let est = at(theta, k)?;
        let var = at(&work.hess_inv2, mat_index(k, k, n_theta))?;
        let std_error = if var.is_finite() && var >= 0.0 {
            Some(var.sqrt())
        } else {
            None
        };
        let (z, p_value, lower, upper) = var_infer(est, std_error, zcrit);
        let name = data
            .subjects
            .keys
            .get(k)
            .map(|_| format!("random_intercept_{k}"))
            .unwrap_or_else(|| format!("random_intercept_{k}"));
        variance_components.push(VarianceComponent {
            name,
            spec_index: Some(k),
            estimate: est,
            std_error,
            z,
            p_value,
            lower,
            upper,
        });
    }
    let est = at(theta, n_specs)?;
    let var = at(&work.hess_inv2, mat_index(n_specs, n_specs, n_theta))?;
    let std_error = if var.is_finite() && var >= 0.0 {
        Some(var.sqrt())
    } else {
        None
    };
    let (z, p_value, lower, upper) = var_infer(est, std_error, zcrit);
    variance_components.push(VarianceComponent {
        name: "residual".into(),
        spec_index: None,
        estimate: est,
        std_error,
        z,
        p_value,
        lower,
        upper,
    });
    let f_tests = f_tests_of(
        data,
        &work.beta,
        &sigma,
        &work.swept,
        df_res,
        settings.singularity,
    )?;
    let d = n_theta as f64;
    let nstar = data.n_effective - work.px as f64;
    let minus2 = work.minus2;
    let aicc = if nstar - d - 1.0 > 0.0 {
        minus2 + 2.0 * d * nstar / (nstar - d - 1.0)
    } else {
        f64::NAN
    };
    let bic = minus2 + d * nstar.max(1.0).ln();
    let mut blups = Vec::new();
    let mut subject_lookup = Vec::new();
    for k in 0..n_specs {
        let mut map = std::collections::HashMap::new();
        let off = at(&data.subjects.offsets, k)?;
        let nk = at(&data.subjects.counts, k)?;
        for s in 0..nk {
            let qi = off + s;
            let key = data
                .subjects
                .keys
                .get(k)
                .and_then(|kk| kk.get(s))
                .cloned()
                .unwrap_or_default();
            map.insert(join_key(&key), qi);
            blups.push(RandomPrediction {
                spec_index: k,
                subject_key: key,
                blup: at(&work.blup, qi)?,
                std_error: at(&work.blup_se, qi)?,
            });
        }
        subject_lookup.push(map);
    }
    Ok(GlmmFit {
        parameters,
        variance_components,
        f_tests,
        minus2_log_likelihood: minus2,
        aicc,
        bic,
        n: data.n,
        n_effective: data.n_effective,
        px: work.px,
        residual_df: df_res,
        iterations: inner_iters,
        outer_iterations: outer,
        converged,
        blups,
        beta: work.beta.clone(),
        c11: sigma,
        c21: work.c21.clone(),
        c22: work.c22.clone(),
        q: data.subjects.q,
        p,
        columns: data.columns.clone(),
        subject_lookup,
        blup_by_q: work.blup.clone(),
        link: settings.link,
    })
}

fn var_infer(
    est: f64,
    std_error: Option<f64>,
    zcrit: f64,
) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    match std_error {
        Some(se) if se > 0.0 && est > 0.0 => {
            let z = est / se;
            let p = chi_square_p(z * z, 1.0);
            let log_half = zcrit * se / est;
            let lower = est * (-log_half).exp();
            let upper = est * log_half.exp();
            (Some(z), Some(p), Some(lower), Some(upper))
        }
        _ => (None, None, None, None),
    }
}

fn t_p_value(t: f64, df: f64) -> f64 {
    if !t.is_finite() || df <= 0.0 {
        return f64::NAN;
    }
    let cdf = student_t_cdf(t.abs(), df);
    if !cdf.is_finite() {
        return f64::NAN;
    }
    (2.0 * (1.0 - cdf)).clamp(0.0, 1.0)
}

fn standard_errors(cov: &[f64], p: usize, swept: &[bool]) -> Result<Vec<Option<f64>>, AlgoError> {
    let mut se = Vec::with_capacity(p);
    for i in 0..p {
        if !at(swept, i)? {
            se.push(None);
            continue;
        }
        let v = at(cov, mat_index(i, i, p))?;
        se.push(if v.is_finite() && v >= 0.0 {
            Some(v.sqrt())
        } else {
            None
        });
    }
    Ok(se)
}

fn f_tests_of(
    data: &Prepared,
    beta: &[f64],
    cov: &[f64],
    swept: &[bool],
    df2: f64,
    tol: f64,
) -> Result<Vec<FTest>, AlgoError> {
    let p = beta.len();
    let mut tests = Vec::new();
    let mut corrected = Vec::new();
    for (ei, effect) in data.effects.iter().enumerate() {
        if effect.name == "(Intercept)" {
            continue;
        }
        let mut rows = Vec::new();
        for &j in &effect.columns {
            if at(swept, j)? {
                let mut l = vec![0.0; p];
                *slot(&mut l, j)? = 1.0;
                rows.push(l);
                corrected.push(j);
            }
        }
        tests.push(f_contrast(&effect.name, &rows, beta, cov, p, df2, tol)?);
        let _ = ei;
    }
    let mut rows = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for j in corrected {
        if seen.insert(j) {
            let mut l = vec![0.0; p];
            *slot(&mut l, j)? = 1.0;
            rows.push(l);
        }
    }
    let corrected_test = f_contrast("corrected_model", &rows, beta, cov, p, df2, tol)?;
    let mut out = vec![corrected_test];
    out.append(&mut tests);
    Ok(out)
}

fn f_contrast(
    name: &str,
    rows: &[Vec<f64>],
    beta: &[f64],
    cov: &[f64],
    p: usize,
    df2: f64,
    tol: f64,
) -> Result<FTest, AlgoError> {
    let r = rows.len();
    if r == 0 {
        return Ok(FTest {
            name: name.into(),
            f: None,
            df1: 0.0,
            df2,
            p_value: None,
        });
    }
    let mut lb = vec![0.0; r];
    for (i, row) in rows.iter().enumerate() {
        let mut s = 0.0;
        for j in 0..p {
            s += at(row, j)? * at(beta, j)?;
        }
        *slot(&mut lb, i)? = s;
    }
    let mut lcl = vec![0.0; r.saturating_mul(r)];
    for i in 0..r {
        for k in 0..r {
            let li = rows
                .get(i)
                .ok_or_else(|| AlgoError::Data("GLMM contrast row".into()))?;
            let lk = rows
                .get(k)
                .ok_or_else(|| AlgoError::Data("GLMM contrast row".into()))?;
            let mut s = 0.0;
            for a in 0..p {
                for b in 0..p {
                    s += at(li, a)? * at(cov, mat_index(a, b, p))? * at(lk, b)?;
                }
            }
            *slot(&mut lcl, mat_index(i, k, r))? = s;
        }
    }
    let Some(sol) = sweep_solve(&lcl, &lb, r, tol) else {
        return Ok(FTest {
            name: name.into(),
            f: None,
            df1: 0.0,
            df2,
            p_value: None,
        });
    };
    let mut stat = 0.0;
    let mut rank = 0usize;
    for i in 0..r {
        stat += at(&lb, i)? * at(&sol, i)?;
        let mut e = vec![0.0; r];
        *slot(&mut e, i)? = 1.0;
        if let Some(col) = sweep_solve(&lcl, &e, r, tol)
            && col.iter().any(|v| v.abs() > 0.0)
        {
            rank = rank.saturating_add(1);
        }
    }
    if !stat.is_finite() {
        return Ok(FTest {
            name: name.into(),
            f: None,
            df1: rank as f64,
            df2,
            p_value: None,
        });
    }
    if stat < 0.0 {
        stat = 0.0;
    }
    let df1 = rank as f64;
    let f = if df1 > 0.0 { Some(stat / df1) } else { None };
    let p_value = match f {
        Some(fv) if df1 > 0.0 && df2 > 0.0 => Some(f_p(fv, df1, df2)),
        _ => None,
    };
    Ok(FTest {
        name: name.into(),
        f,
        df1,
        df2,
        p_value,
    })
}

fn robust_sigma(data: &Prepared, work: &InnerWork, sigma: &[f64]) -> Result<Vec<f64>, AlgoError> {
    if data.subjects.n_specs == 0 {
        return Ok(sigma.to_vec());
    }
    let p = data.p;
    let n = data.n;
    let n_clusters = at(&data.subjects.counts, 0)?;
    let mut meat = vec![0.0; p.saturating_mul(p)];
    for s in 0..n_clusters {
        crate::check_cancelled()?;
        let mut acc = vec![0.0; p];
        for i in 0..n {
            if at2(&data.subjects.index, 0, i)? != s {
                continue;
            }
            let ui = at(&work.u, i)?;
            for j in 0..p {
                *slot(&mut acc, j)? += at2(&work.x_star, i, j)? * ui;
            }
        }
        for a in 0..p {
            for b in 0..p {
                *slot(&mut meat, mat_index(a, b, p))? += at(&acc, a)? * at(&acc, b)?;
            }
        }
    }
    let mut tmp = vec![0.0; p.saturating_mul(p)];
    for i in 0..p {
        for j in 0..p {
            let mut s = 0.0;
            for k in 0..p {
                s += at(sigma, mat_index(i, k, p))? * at(&meat, mat_index(k, j, p))?;
            }
            *slot(&mut tmp, mat_index(i, j, p))? = s;
        }
    }
    let mut out = vec![0.0; p.saturating_mul(p)];
    for i in 0..p {
        for j in 0..p {
            let mut s = 0.0;
            for k in 0..p {
                s += at(&tmp, mat_index(i, k, p))? * at(sigma, mat_index(k, j, p))?;
            }
            *slot(&mut out, mat_index(i, j, p))? = s;
        }
    }
    Ok(out)
}

fn score_row(fit: &GlmmFit, input: &ScoreInput) -> Result<Vec<f64>, AlgoError> {
    let p = fit.p;
    let mut x = vec![0.0; p];
    let mut cov_i = 0usize;
    for (j, column) in fit.columns.iter().enumerate() {
        match &column.kind {
            ColumnKind::Intercept => *slot(&mut x, j)? = 1.0,
            ColumnKind::Covariate { .. } => {
                let v = input.numeric.get(cov_i).copied().ok_or_else(|| {
                    AlgoError::Data("GLMM score record is missing a covariate".into())
                })?;
                if !v.is_finite() {
                    return Err(AlgoError::Data("GLMM score covariate is not finite".into()));
                }
                *slot(&mut x, j)? = v;
                cov_i = cov_i.saturating_add(1);
            }
            ColumnKind::Indicator { factor, level, .. } => {
                let got = input.factors.get(*factor).map(String::as_str).unwrap_or("");
                *slot(&mut x, j)? = if same_category(got, level) { 1.0 } else { 0.0 };
            }
        }
    }
    Ok(x)
}

fn column_of(x: &[Vec<f64>], j: usize, n: usize) -> Result<Vec<f64>, AlgoError> {
    let mut col = vec![0.0; n];
    for i in 0..n {
        *slot(&mut col, i)? = at2(x, i, j)?;
    }
    Ok(col)
}

fn quad_form(mat: &[f64], v: &[f64], n: usize) -> Result<f64, AlgoError> {
    if n == 0 {
        return Ok(0.0);
    }
    let mut s = 0.0;
    for i in 0..n {
        let mut t = 0.0;
        for j in 0..n {
            t += at(mat, mat_index(i, j, n))? * at(v, j)?;
        }
        s += at(v, i)? * t;
    }
    Ok(s)
}

fn dot(a: &[f64], b: &[f64]) -> Result<f64, AlgoError> {
    if a.len() != b.len() {
        return Err(AlgoError::Data("GLMM dot product length mismatch".into()));
    }
    let mut s = 0.0;
    for (x, y) in a.iter().zip(b) {
        s += *x * *y;
    }
    Ok(s)
}

fn spd_add_ridge(h: &mut [f64], n: usize, ridge: f64) -> bool {
    if n == 0 {
        return true;
    }
    let mut ok = true;
    for i in 0..n {
        let idx = mat_index(i, i, n);
        if let Some(slot) = h.get_mut(idx) {
            if !slot.is_finite() {
                ok = false;
            }
            *slot += ridge;
        } else {
            ok = false;
        }
    }
    ok
}

struct SweepInv {
    inverse: Vec<f64>,
    swept: Vec<bool>,
    logdet: f64,
    rank: usize,
}

fn sweep_inverse(matrix: &[f64], n: usize, tol: f64) -> Result<SweepInv, AlgoError> {
    if n == 0 {
        return Ok(SweepInv {
            inverse: Vec::new(),
            swept: Vec::new(),
            logdet: 0.0,
            rank: 0,
        });
    }
    let len = n.saturating_mul(n);
    if matrix.len() < len {
        return Err(AlgoError::Data("GLMM sweep matrix is short".into()));
    }
    let mut a = matrix[..len].to_vec();
    let original: Vec<f64> = (0..n)
        .map(|k| a.get(mat_index(k, k, n)).copied().unwrap_or(0.0))
        .collect();
    let mut swept = vec![false; n];
    let mut logdet = 0.0;
    let mut rank = 0usize;
    for k in 0..n {
        if crate::check_cancelled().is_err() {
            return Err(AlgoError::from(dms_types::Cancelled));
        }
        let pivot = *a
            .get(mat_index(k, k, n))
            .ok_or_else(|| AlgoError::Data("GLMM sweep pivot is missing".into()))?;
        let orig = original.get(k).copied().unwrap_or(0.0);
        if !pivot.is_finite() || pivot <= tol * orig.abs() || pivot <= 0.0 {
            continue;
        }
        *swept
            .get_mut(k)
            .ok_or_else(|| AlgoError::Data("GLMM sweep flag".into()))? = true;
        rank = rank.saturating_add(1);
        logdet += pivot.ln();
        let row: Vec<f64> = (0..n)
            .filter_map(|j| a.get(mat_index(k, j, n)).copied())
            .collect();
        if row.len() != n {
            return Err(AlgoError::Data("GLMM sweep row is short".into()));
        }
        for i in (0..n).filter(|&i| i != k) {
            let factor = *a
                .get(mat_index(i, k, n))
                .ok_or_else(|| AlgoError::Data("GLMM sweep factor is missing".into()))?
                / pivot;
            for j in (0..n).filter(|&j| j != k) {
                let idx = mat_index(i, j, n);
                let rv = *row
                    .get(j)
                    .ok_or_else(|| AlgoError::Data("GLMM sweep row".into()))?;
                *a.get_mut(idx)
                    .ok_or_else(|| AlgoError::Data("GLMM sweep cell".into()))? -= factor * rv;
            }
        }
        for j in (0..n).filter(|&j| j != k) {
            let rv = *row
                .get(j)
                .ok_or_else(|| AlgoError::Data("GLMM sweep row".into()))?;
            *a.get_mut(mat_index(k, j, n))
                .ok_or_else(|| AlgoError::Data("GLMM sweep cell".into()))? = rv / pivot;
            let cell = a
                .get_mut(mat_index(j, k, n))
                .ok_or_else(|| AlgoError::Data("GLMM sweep cell".into()))?;
            *cell /= pivot;
        }
        *a.get_mut(mat_index(k, k, n))
            .ok_or_else(|| AlgoError::Data("GLMM sweep cell".into()))? = -1.0 / pivot;
    }
    let mut inverse = vec![0.0; len];
    for i in 0..n {
        for j in 0..n {
            if swept.get(i) == Some(&true) && swept.get(j) == Some(&true) {
                let val = *a
                    .get(mat_index(i, j, n))
                    .ok_or_else(|| AlgoError::Data("GLMM sweep inverse cell".into()))?;
                *slot(&mut inverse, mat_index(i, j, n))? = -val;
            }
        }
    }
    if !logdet.is_finite() {
        logdet = 0.0;
    }
    Ok(SweepInv {
        inverse,
        swept,
        logdet,
        rank,
    })
}

fn sweep_solve(matrix: &[f64], rhs: &[f64], n: usize, tol: f64) -> Option<Vec<f64>> {
    let inv = sweep_inverse(matrix, n, tol).ok()?;
    let mut x = vec![0.0; n];
    for i in 0..n {
        if inv.swept.get(i) != Some(&true) {
            continue;
        }
        let mut s = 0.0;
        for j in 0..n {
            if inv.swept.get(j) == Some(&true) {
                s += inv.inverse.get(mat_index(i, j, n)).copied().unwrap_or(0.0)
                    * rhs.get(j).copied().unwrap_or(0.0);
            }
        }
        if let Some(slot) = x.get_mut(i) {
            *slot = s;
        }
    }
    if x.iter().all(|v| v.is_finite()) {
        Some(x)
    } else {
        None
    }
}

fn chol_factor(a: &[f64], n: usize) -> Result<Chol, AlgoError> {
    if n == 0 {
        return Ok(Chol {
            l: Vec::new(),
            n: 0,
        });
    }
    let mut l = vec![0.0; n.saturating_mul(n)];
    for i in 0..n {
        crate::check_cancelled()?;
        for j in 0..=i {
            let mut s = at(a, mat_index(i, j, n))?;
            for k in 0..j {
                s -= at(&l, mat_index(i, k, n))? * at(&l, mat_index(j, k, n))?;
            }
            if i == j {
                if !(s.is_finite() && s > 0.0) {
                    let ridge = 1e-10 * (1.0 + at(a, mat_index(i, i, n))?.abs());
                    s += ridge;
                }
                if !(s.is_finite() && s > 0.0) {
                    return Err(AlgoError::Data(
                        "GLMM random-effect matrix is not positive definite".into(),
                    ));
                }
                *slot(&mut l, mat_index(i, i, n))? = s.sqrt();
            } else {
                let d = at(&l, mat_index(j, j, n))?;
                if d.abs() < VAR_FLOOR {
                    return Err(AlgoError::Data(
                        "GLMM random-effect matrix is not positive definite".into(),
                    ));
                }
                *slot(&mut l, mat_index(i, j, n))? = s / d;
            }
        }
    }
    Ok(Chol { l, n })
}

fn chol_logdet(chol: &Chol) -> Result<f64, AlgoError> {
    if chol.n == 0 {
        return Ok(0.0);
    }
    let mut s = 0.0;
    for i in 0..chol.n {
        let d = at(&chol.l, mat_index(i, i, chol.n))?;
        if d <= 0.0 {
            return Err(AlgoError::Data(
                "GLMM Cholesky diagonal is not positive".into(),
            ));
        }
        s += d.ln();
    }
    Ok(2.0 * s)
}

fn chol_solve(chol: &Chol, b: &[f64]) -> Result<Vec<f64>, AlgoError> {
    let n = chol.n;
    if n == 0 {
        return Ok(Vec::new());
    }
    let mut y = vec![0.0; n];
    for i in 0..n {
        let mut s = at(b, i)?;
        for j in 0..i {
            s -= at(&chol.l, mat_index(i, j, n))? * at(&y, j)?;
        }
        let d = at(&chol.l, mat_index(i, i, n))?;
        if d.abs() < VAR_FLOOR {
            return Err(AlgoError::Data(
                "GLMM Cholesky solve hit a zero pivot".into(),
            ));
        }
        *slot(&mut y, i)? = s / d;
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut s = at(&y, i)?;
        for j in (i + 1)..n {
            s -= at(&chol.l, mat_index(j, i, n))? * at(&x, j)?;
        }
        let d = at(&chol.l, mat_index(i, i, n))?;
        *slot(&mut x, i)? = s / d;
    }
    Ok(x)
}

fn chol_inverse(chol: &Chol) -> Result<Vec<f64>, AlgoError> {
    let n = chol.n;
    if n == 0 {
        return Ok(Vec::new());
    }
    let mut inv = vec![0.0; n.saturating_mul(n)];
    for j in 0..n {
        crate::check_cancelled()?;
        let mut e = vec![0.0; n];
        *slot(&mut e, j)? = 1.0;
        let col = chol_solve(chol, &e)?;
        for i in 0..n {
            *slot(&mut inv, mat_index(i, j, n))? = at(&col, i)?;
        }
    }
    Ok(inv)
}

fn variance(mu: f64, distribution: Distribution) -> Option<f64> {
    let v = match distribution {
        Distribution::Normal => 1.0,
        Distribution::InverseGaussian => {
            let m = mu.max(MU_FLOOR);
            m * m * m
        }
        Distribution::Gamma => mu * mu,
        Distribution::Poisson => mu.max(MU_FLOOR),
        Distribution::Negbin { k } => {
            let m = mu.max(MU_FLOOR);
            m + k * m * m
        }
        Distribution::Binomial => {
            let p = mu.clamp(MU_FLOOR, 1.0 - MU_FLOOR);
            p * (1.0 - p)
        }
    };
    if v.is_finite() && v > 0.0 {
        Some(v)
    } else {
        None
    }
}

fn mu_of(eta: f64, link: Link) -> Option<f64> {
    let mu = match link {
        Link::Identity => eta,
        Link::Log => cap_exp(eta),
        Link::Logit => {
            let z = eta.clamp(-ETA_CAP, ETA_CAP);
            1.0 / (1.0 + (-z).exp())
        }
        Link::Probit => phi_cdf(eta.clamp(-ETA_CAP, ETA_CAP)),
        Link::Cloglog => {
            let z = eta.clamp(-ETA_CAP, ETA_CAP);
            1.0 - (-cap_exp(z)).exp()
        }
        Link::Nloglog => {
            let z = eta.clamp(-ETA_CAP, ETA_CAP);
            (-cap_exp(-z)).exp()
        }
        Link::Logc => {
            let z = eta.clamp(-ETA_CAP, ETA_CAP);
            1.0 - z.exp()
        }
        Link::Power { alpha } => {
            if alpha.abs() < 2.2e-16 {
                cap_exp(eta)
            } else if eta == 0.0 {
                return None;
            } else {
                eta.powf(1.0 / alpha)
            }
        }
        Link::Negbin { k } => {
            let e = cap_exp(eta.clamp(-ETA_CAP, ETA_CAP));
            let den = 1.0 - k * e;
            if den.abs() < MU_FLOOR {
                return None;
            }
            e / den
        }
    };
    if mu.is_finite() { Some(mu) } else { None }
}

fn g_of(mu: f64, link: Link) -> Option<f64> {
    let eta = match link {
        Link::Identity => mu,
        Link::Log => {
            if mu <= 0.0 {
                return None;
            }
            mu.ln()
        }
        Link::Logit => {
            if mu <= 0.0 || mu >= 1.0 {
                return None;
            }
            (mu / (1.0 - mu)).ln()
        }
        Link::Probit => {
            if mu <= 0.0 || mu >= 1.0 {
                return None;
            }
            phi_inv(mu)?
        }
        Link::Cloglog => {
            if mu <= 0.0 || mu >= 1.0 {
                return None;
            }
            (-(1.0 - mu).ln()).ln()
        }
        Link::Nloglog => {
            if mu <= 0.0 || mu >= 1.0 {
                return None;
            }
            -(-mu.ln()).ln()
        }
        Link::Logc => {
            if mu >= 1.0 {
                return None;
            }
            (1.0 - mu).ln()
        }
        Link::Power { alpha } => {
            if alpha.abs() < 2.2e-16 {
                if mu <= 0.0 {
                    return None;
                }
                mu.ln()
            } else {
                if mu <= 0.0 {
                    return None;
                }
                mu.powf(alpha)
            }
        }
        Link::Negbin { k } => {
            if mu <= 0.0 {
                return None;
            }
            let den = 1.0 + k * mu;
            if den <= 0.0 {
                return None;
            }
            (mu / den).ln()
        }
    };
    if eta.is_finite() { Some(eta) } else { None }
}

fn g1_of(mu: f64, link: Link) -> Option<f64> {
    let d = match link {
        Link::Identity => 1.0,
        Link::Log => {
            if mu.abs() < MU_FLOOR {
                return None;
            }
            1.0 / mu
        }
        Link::Logit => {
            let den = mu * (1.0 - mu);
            if den.abs() < MU_FLOOR {
                return None;
            }
            1.0 / den
        }
        Link::Probit => {
            let z = phi_inv(mu.clamp(MU_FLOOR, 1.0 - MU_FLOOR))?;
            let dphi = phi_pdf(z);
            if dphi.abs() < MU_FLOOR {
                return None;
            }
            1.0 / dphi
        }
        Link::Cloglog => {
            let den = (mu - 1.0) * (1.0 - mu).ln();
            if den.abs() < MU_FLOOR {
                return None;
            }
            1.0 / den
        }
        Link::Nloglog => {
            if mu.abs() < MU_FLOOR {
                return None;
            }
            let ln = mu.ln();
            if ln.abs() < MU_FLOOR {
                return None;
            }
            -1.0 / (mu * ln)
        }
        Link::Logc => {
            let den = 1.0 - mu;
            if den.abs() < MU_FLOOR {
                return None;
            }
            -1.0 / den
        }
        Link::Power { alpha } => {
            if alpha.abs() < 2.2e-16 {
                if mu.abs() < MU_FLOOR {
                    return None;
                }
                1.0 / mu
            } else {
                alpha * mu.powf(alpha - 1.0)
            }
        }
        Link::Negbin { k } => {
            let den = mu + k * mu * mu;
            if den.abs() < MU_FLOOR {
                return None;
            }
            1.0 / den
        }
    };
    if d.is_finite() { Some(d) } else { None }
}

fn cap_exp(eta: f64) -> f64 {
    if eta > ETA_CAP {
        ETA_CAP.exp()
    } else if eta < -ETA_CAP {
        (-ETA_CAP).exp()
    } else {
        eta.exp()
    }
}

fn phi_pdf(z: f64) -> f64 {
    (-0.5 * z * z).exp() / TWO_PI.sqrt()
}

fn phi_cdf(z: f64) -> f64 {
    let x = z / std::f64::consts::SQRT_2;
    let p = 1.0 - gamma_q(0.5, x * x);
    let erf = if x >= 0.0 { p } else { -p };
    (0.5 * (1.0 + erf)).clamp(0.0, 1.0)
}

fn phi_inv(p: f64) -> Option<f64> {
    if !(p > 0.0 && p < 1.0) {
        return None;
    }
    let mut lo = -8.0;
    let mut hi = 8.0;
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if phi_cdf(mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let z = 0.5 * (lo + hi);
    if z.is_finite() { Some(z) } else { None }
}

fn push_column(
    columns: &mut Vec<Column>,
    next: &mut usize,
    kind: ColumnKind,
    label: String,
    effect: usize,
) {
    let parameter_name = format!("P{n:07}", n = *next);
    *next = next.saturating_add(1);
    columns.push(Column {
        kind,
        parameter_name,
        label,
        effect,
    });
}

fn at<T: Copy>(values: &[T], index: usize) -> Result<T, AlgoError> {
    values
        .get(index)
        .copied()
        .ok_or_else(|| AlgoError::Data("GLMM index is out of range".into()))
}

fn vget<T>(values: &[Vec<T>], index: usize) -> Result<&[T], AlgoError> {
    values
        .get(index)
        .map(|row| row.as_slice())
        .ok_or_else(|| AlgoError::Data("GLMM index is out of range".into()))
}

fn at2<T: Copy>(values: &[Vec<T>], i: usize, j: usize) -> Result<T, AlgoError> {
    at(vget(values, i)?, j)
}

fn slot<T>(values: &mut [T], index: usize) -> Result<&mut T, AlgoError> {
    values
        .get_mut(index)
        .ok_or_else(|| AlgoError::Data("GLMM index is out of range".into()))
}

fn mat_index(row: usize, col: usize, n: usize) -> usize {
    row.saturating_mul(n).saturating_add(col)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * (1.0 + a.abs().max(b.abs()))
    }

    fn one_way_input(y: Vec<f64>, groups: Vec<&str>) -> GlmmInput {
        GlmmInput {
            target_name: "y".into(),
            y,
            fixed: Vec::new(),
            random: vec![RandomSpec {
                subject_names: vec!["g".into()],
                subject_keys: groups.into_iter().map(|g| vec![g.to_string()]).collect(),
                intercept: true,
                slopes: Vec::new(),
                covariance: CovarianceType::VarianceComponents,
            }],
            weight: None,
            frequency: None,
            offset: None,
            trials: None,
        }
    }

    #[test]
    fn glmm_balanced_one_way_reml_matches_anova() {
        // Two groups of two: y = (1,3 | 5,7). Intercept-only, random intercept.
        // SSE = 4, MSE = 2, SSA = 16, MSA = 16, n = 2.
        // REML: σ_e² = MSE = 2, σ_u² = (MSA − MSE)/n = 7.
        let input = one_way_input(vec![1.0, 3.0, 5.0, 7.0], vec!["a", "a", "b", "b"]);
        let fit = fit(&input, &GlmmSettings::default()).expect("fit");
        let su = fit.variance_components[0].estimate;
        let se = fit.variance_components[1].estimate;
        assert!(close(se, 2.0, 1e-6), "residual {se}");
        assert!(close(su, 7.0, 1e-6), "school {su}");
        assert!(close(fit.parameters[0].estimate, 4.0, 1e-6));
        // −2ℓ_R = ln|V| + quad + ln|X′V⁻¹X| + (N−px) ln(2π)
        // = 2 ln 32 + 3 + ln(1/4) + 3 ln(2π) ≈ 14.058809
        assert!(
            close(fit.minus2_log_likelihood, 14.058809, 1e-5),
            "{}",
            fit.minus2_log_likelihood
        );
        let rec = ScoreInput {
            numeric: Vec::new(),
            factors: Vec::new(),
            subjects: vec![vec!["a".into()]],
            offset: 0.0,
        };
        let s = fit.score(&rec).expect("score");
        assert!(close(s.eta, 2.25, 1e-5), "BLUP prediction {}", s.eta);
        let sf = fit.score_fixed(&rec).expect("fixed");
        assert!(close(sf.eta, 4.0, 1e-6));
    }

    #[test]
    fn glmm_empty_data_is_error() {
        let input = GlmmInput {
            target_name: "y".into(),
            y: Vec::new(),
            fixed: Vec::new(),
            random: Vec::new(),
            weight: None,
            frequency: None,
            offset: None,
            trials: None,
        };
        assert!(fit(&input, &GlmmSettings::default()).is_err());
    }

    #[test]
    fn glmm_one_subject_does_not_panic() {
        let input = one_way_input(vec![1.0, 2.0, 3.0], vec!["a", "a", "a"]);
        let _ = fit(&input, &GlmmSettings::default());
    }

    #[test]
    fn glmm_constant_target_does_not_panic() {
        let input = one_way_input(vec![5.0, 5.0, 5.0, 5.0], vec!["a", "a", "b", "b"]);
        let _ = fit(&input, &GlmmSettings::default());
    }

    #[test]
    fn glmm_singular_fixed_effect_does_not_panic() {
        let input = GlmmInput {
            target_name: "y".into(),
            y: vec![1.0, 2.0, 3.0, 4.0],
            fixed: vec![
                FixedTerm::Covariate(NumericColumn {
                    name: "x1".into(),
                    values: vec![1.0, 2.0, 3.0, 4.0],
                }),
                FixedTerm::Covariate(NumericColumn {
                    name: "x2".into(),
                    values: vec![1.0, 2.0, 3.0, 4.0],
                }),
            ],
            random: vec![RandomSpec {
                subject_names: vec!["g".into()],
                subject_keys: vec![
                    vec!["a".into()],
                    vec!["a".into()],
                    vec!["b".into()],
                    vec!["b".into()],
                ],
                intercept: true,
                slopes: Vec::new(),
                covariance: CovarianceType::VarianceComponents,
            }],
            weight: None,
            frequency: None,
            offset: None,
            trials: None,
        };
        let fit = fit(&input, &GlmmSettings::default());
        assert!(fit.is_ok() || fit.is_err());
        if let Ok(f) = fit {
            let red = f.parameters.iter().filter(|p| p.redundant).count();
            assert!(
                red >= 1,
                "one of the duplicate covariates should be redundant"
            );
        }
    }

    #[test]
    fn glmm_subject_with_one_record_does_not_panic() {
        let input = one_way_input(vec![1.0, 2.0, 9.0], vec!["a", "a", "b"]);
        let _ = fit(&input, &GlmmSettings::default());
    }

    #[test]
    fn glmm_thousands_of_subjects_is_capped() {
        let n = 3000;
        let y = vec![1.0; n];
        let keys: Vec<Vec<String>> = (0..n).map(|i| vec![i.to_string()]).collect();
        let input = GlmmInput {
            target_name: "y".into(),
            y,
            fixed: Vec::new(),
            random: vec![RandomSpec {
                subject_names: vec!["g".into()],
                subject_keys: keys,
                intercept: true,
                slopes: Vec::new(),
                covariance: CovarianceType::VarianceComponents,
            }],
            weight: None,
            frequency: None,
            offset: None,
            trials: None,
        };
        let err = fit(&input, &GlmmSettings::default()).expect_err("cap");
        let msg = err.to_string();
        assert!(msg.contains("limited") || msg.contains("GAP") || msg.contains("cannot train"));
    }

    #[test]
    fn glmm_random_slopes_are_gap() {
        let input = GlmmInput {
            target_name: "y".into(),
            y: vec![1.0, 2.0],
            fixed: Vec::new(),
            random: vec![RandomSpec {
                subject_names: vec!["g".into()],
                subject_keys: vec![vec!["a".into()], vec!["a".into()]],
                intercept: true,
                slopes: vec![NumericColumn {
                    name: "x".into(),
                    values: vec![0.0, 1.0],
                }],
                covariance: CovarianceType::VarianceComponents,
            }],
            weight: None,
            frequency: None,
            offset: None,
            trials: None,
        };
        let err = fit(&input, &GlmmSettings::default()).expect_err("gap");
        assert!(err.to_string().contains("GAP"));
    }

    #[test]
    fn glmm_residual_ar1_is_gap() {
        let input = one_way_input(vec![1.0, 2.0, 3.0, 4.0], vec!["a", "a", "b", "b"]);
        let settings = GlmmSettings {
            residual: CovarianceType::Ar1,
            ..GlmmSettings::default()
        };
        let err = fit(&input, &settings).expect_err("gap");
        assert!(err.to_string().contains("GAP"));
    }

    #[test]
    fn glmm_no_random_effects_is_ols() {
        let input = GlmmInput {
            target_name: "y".into(),
            y: vec![1.0, 3.0, 5.0, 7.0],
            fixed: vec![FixedTerm::Covariate(NumericColumn {
                name: "x".into(),
                values: vec![0.0, 1.0, 2.0, 3.0],
            })],
            random: Vec::new(),
            weight: None,
            frequency: None,
            offset: None,
            trials: None,
        };
        let fit = fit(&input, &GlmmSettings::default()).expect("ols");
        assert!(
            close(fit.parameters[0].estimate, 1.0, 1e-6),
            "{:?}",
            fit.parameters
        );
        assert!(
            close(fit.parameters[1].estimate, 2.0, 1e-6),
            "{:?}",
            fit.parameters
        );
        assert!(fit.variance_components.last().expect("phi").estimate.abs() < 1e-8);
    }

    #[test]
    fn glmm_aicc_formula_on_one_way() {
        // Three groups of two so N* − d − 1 = 6 − 1 − 2 − 1 = 2 > 0.
        let input = one_way_input(
            vec![1.0, 3.0, 5.0, 7.0, 8.0, 10.0],
            vec!["a", "a", "b", "b", "c", "c"],
        );
        let fit = fit(&input, &GlmmSettings::default()).expect("fit");
        let nstar = 6.0 - 1.0;
        let d = 2.0;
        let want = fit.minus2_log_likelihood + 2.0 * d * nstar / (nstar - d - 1.0);
        assert!(close(fit.aicc, want, 1e-10), "aicc {} vs {want}", fit.aicc);
        let bic = fit.minus2_log_likelihood + d * nstar.ln();
        assert!(close(fit.bic, bic, 1e-10), "bic {} vs {bic}", fit.bic);
    }
}
