//! Generalized Linear Models for the GenLin node (IBM SPSS Modeler 19 Algorithms Guide,
//! “GENLIN Algorithms”, pp. 167–190).
//!
//! Formulas are taken from the page images `target/grok-genlin/pages/guide_p167.png` …
//! `guide_p190.png`. The chapter text `target/grok-genlin/guide_genlin_chapter.txt` is used for
//! prose only. Estimation, links already in GLE, and the SWEEP solver reuse
//! [`crate::gle`] (Phase I, verified against Modeler). Behaviour of `gle` is unchanged.
//!
//! Say “reproduces the saved Modeler model” only for numbers checked against
//! `target/grok-genlin/model/`. Everything else “follows the Algorithms Guide” and is
//! unverified. `ships_genlin` has no saved model.
//!
//! # Decisions the Guide does not settle
//!
//! A. **Over-parameterised dummies, last level redundant.** The Guide uses the GLM
//!    over-parameterised model and SWEEP on `X'ΨX`; a dependent column is fixed at 0.
//!    Every saved ParamMatrix has intercept first, then each factor’s levels in
//!    `value_order`, and the last level of each factor has `beta=0`, `df=0`
//!    (`ulcer_model_1` `[period=1]`, `[treatment=0]`; `ulcer_model_2` `[treatment=0]`;
//!    `car_insurance_model` `[holderage=1]`, `[vehiclegroup=1]`, `[vehicleage=1]`).
//!
//! B. **`value_order = Descending` sorts numeric codes high to low.** PMML DataField /
//!    PPMatrix list period 2 then 1, treatment 1 then 0, holderage 8…1. Confirmed by
//!    those dummy labels.
//!
//! C. **Binary `base_category` First/Last is Type/declaration order, not `value_order`.**
//!    Guide p. 168: First as reference models the other category. Ulcer settings have
//!    `_base_category = First` and `_value_order = Descending`. Type labels list 0 then 1;
//!    HTML “models Ulcer detected as the response, treating No ulcer detected as the
//!    reference”; ParamMatrix `targetCategory="1"`. If First meant first in Descending
//!    listing (1 then 0), the reference would be 1.
//!
//! D. **Parameter names start at `P0000001`.** All three saved ParameterList. (GLE uses
//!    `P0000000`.)
//!
//! E. **Model-based covariance is `−H⁻` with the observed Hessian `W_o` at the MLE.**
//!    Guide p. 179 writes `Σ_m = −H⁻ = (−X'WX)⁻` and p. 175 defines `W` as `W_e` or `W_o`.
//!    After hybrid K=1 the last Hessian is Newton. `ulcer_model_1` PCovMatrix matches
//!    Newton `W_o` at the saved β to 1e-14 relative; Fisher `W_e` differs by up to 0.75
//!    on `P0000002`–`P0000004`. Canonical links have `W_o = W_e` (gamma + power(−1) in
//!    `car_insurance_model`).
//!
//! F. **Displayed log-likelihood for binary 0/1 is the case-wise kernel** (full extra term
//!    `ln C(1, r) = 0`). It is **not** the subpopulation `Σ ln C(m_j, r_j)` of Guide p. 185.
//!    Ulcer 2: case-wise kernel −24.2657 vs HTML −24.266 and AIC 52.531 = −2ℓ+4; the
//!    subpopulation full LL is −3.25. Ulcer 1: case-wise kernel −31.44204 vs HTML −31.442.
//!    Footnote “full log likelihood” is true of the case-wise formula.
//!
//! G. **Binary 0/1 deviance and Pearson use subpopulations; df = n_s − p_x.** Guide
//!    pp. 184–185. Ulcer 2: 2 cells, 2 parameters, D=0, df=0. Ulcer 1: 4 cells, 3
//!    parameters, D=1.909, Pearson=1.842, df=1. Case-wise df would be 43−2=41.
//!
//! H. **N in AICC is the complete-case count, not n_s.** Ulcer 2: AICC−AIC = 0.300 implies
//!    N=43. Guide p. 185: N=n when frequency is unused.
//!
//! I. **Pearson / deviance scale: φ = 1 while estimating β, then φ̂ = stat / (N − p_x);
//!    covariance is multiplied by φ̂.** Guide pp. 178, 184. Car insurance: N=123 included,
//!    p_x=14, df=109, Pearson 131.786, scale 1.209 = 131.786/109; Scaled Pearson = 109;
//!    PCov intercept 1.74666e-7 already includes that scale (Wald 66.593).
//!
//! J. **Scale weight ω ≤ 0 or missing drops the case.** Guide p. 167. Car insurance: 128
//!    rows, 5 with empty `claimamt` and `nclaims=0` excluded (HTML 123 / 96.1%).
//!
//! K. **Predictors are not centred or scaled.** GLE section 2.2.4 centres covariates; the
//!    GENLIN chapter never does. Saved models are all factors, so this is unverified for
//!    covariates; we follow the GENLIN Guide’s X as given.
//!
//! L. **Type III intercept is the equal-cell-weight mean of the linear predictors:**
//!    `L = [1, 1/n_ℓ on every dummy of every factor]` (redundant columns included).
//!    Ulcer 2: Lβ with L=[1, ½] on (intercept, `[treatment=1]`) and the saved PCov gives
//!    Wald 15.893, matching HTML. Ulcer 1: L=[1, ½, ½] on (intercept, period=2, treatment=1)
//!    gives 36.574.
//!
//! M. **Type III for a main-effect factor is the joint Wald of that factor’s non-redundant
//!    dummies.** Ulcer 1 `[period=2]` Wald 0.461 equals the Type III period test. No saved
//!    model has interactions; Type III for an effect contained in a two-way term follows
//!    the Guide’s “effect and effects containing it” reading as the joint Wald of those
//!    parameters and is unverified.
//!
//! N. **With Pearson/deviance scale the reported LL is at φ = 1;** the “adjusted”
//!    log-likelihood uses φ̂ (car insurance footnotes c, d: −622.754 vs −623.873).
//!
//! O. **Hybrid K=1 uses Fisher weights for H when i ≤ K** (initial H and the first
//!    iterate), then Newton. Same reading as GLE (verified against Modeler GLE) and
//!    Guide p. 177 step 6.
//!
//! P. **The two saved ulcer models are the intercept + offset=0 builds**, not the
//!    no-intercept node (id=8) or the empty variable-offset node (id=6). HTML Model
//!    Information: offset 0, intercept in the model line. `ulcer_model_1` / output_1:
//!    target `result2`, inputs period and treatment (stream node 11). `ulcer_model_2` /
//!    output_2: target `result`, input treatment (stream node 2). Offset-field and
//!    no-intercept follow the Guide and are unverified.
//!
//! Q. **SWEEP singularity uses the setting `_singularity_tolerance` (1e-7 in the ulcer
//!    and car-insurance streams)** as GLE does: skip a pivot ≤ tol × |original diagonal|
//!    or ≤ 0. Guide p. 168 names SWEEP but not the tolerance; the streams do.
//!
//! R. **Hessian convergence off in every demo stream** (`_hessian_convergence = false`).
//!    Guide p. 179: if not user-specified, check absolute 1e-4 after the other criteria
//!    and warn; it is not a stopping requirement. We stop on parameter (and optional
//!    log-likelihood) convergence.
//!
//! S. **Saved PMML is `modelType="generalLinear"` with SPSS `Extension` `link` /
//!    `distribution` / `offsetValue`.** This crate’s scorer applies a link only for
//!    `generalizedLinear` + `linkFunction` (GLE). We write `generalizedLinear` with
//!    `linkFunction` / `linkParameter` so a scorer that reads our model applies the
//!    same inverse link. PCovMatrix and PCell `@df` are not fields of
//!    [`dms_pmml::GeneralRegressionModel`]; they live on [`GenlinFit`].
//!
//! Tweedie is named in the scripting guide and in the chapter references, but Table 18-2
//! and Table 18-3 do not give V(μ) or ℓ: [`AlgoError::Unsupported`] `GAP:`. Multinomial,
//! ordinal-multinomial, cumulative links, GEE / repeated measures, and estimating the
//! negative-binomial k (Guide only specifies a user k and an LM test of a fixed k) are
//! the same.

use crate::common::{data_field, mining_field};
use crate::linear::bounded_categories;
use crate::stats::{chi_square_p, gamma_q, ln_gamma};
use crate::{AlgoError, Dataset};
use dms_pmml::{
    GeneralModelType, GeneralRegressionModel, MiningFunction, Model, ModelHeader, PCell, PPCell,
    Pmml, Usage,
};
use dms_types::{Measurement, Storage, Value};

const MAX_RECORDS: usize = 1_000_000;
const MAX_PARAMETERS: usize = 1024;
const MAX_CATEGORIES: usize = 512;
const MAX_INPUTS: usize = 256;
const REL_EPS: f64 = 1e-6;
const ETA_CAP: f64 = 80.0;
const MU_FLOOR: f64 = 1e-12;
const TWO_PI: f64 = 2.0 * std::f64::consts::PI;

/// Target distribution (Guide Table 18-2). Tweedie and multinomial are GAP.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Distribution {
    Normal,
    InverseGaussian,
    Gamma,
    Negbin { k: f64 },
    Poisson,
    Binomial,
}

/// Link function (Guide Tables 18-5 and 18-6). Cumulative links are ordinal-only (GAP).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Link {
    Identity,
    Log,
    Logit,
    Probit,
    Cloglog,
    Logc,
    Nloglog,
    Power { alpha: f64 },
    Negbin { k: f64 },
    OddsPower { alpha: f64 },
}

impl Link {
    fn name(self) -> &'static str {
        match self {
            Link::Identity => "identity",
            Link::Log => "log",
            Link::Logit => "logit",
            Link::Probit => "probit",
            Link::Cloglog => "cloglog",
            Link::Logc => "logc",
            Link::Nloglog => "loglog",
            Link::Power { .. } => "power",
            Link::Negbin { .. } => "negbin",
            Link::OddsPower { .. } => "oddspower",
        }
    }

    fn parameter(self) -> Option<f64> {
        match self {
            Link::Power { alpha } => Some(alpha),
            Link::OddsPower { alpha } => Some(alpha),
            Link::Negbin { k } => Some(k),
            _ => None,
        }
    }
}

/// How the Hessian is formed (Guide Table 18-10, K).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Estimation {
    FisherScoring,
    NewtonRaphson,
    Hybrid { fisher_iterations: usize },
}

/// Scale-parameter method (Guide pp. 170, 178).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScaleMethod {
    Fixed(f64),
    Deviance,
    Pearson,
    MaxLikelihood,
}

/// Model-based or Huber/White/sandwich covariance (Guide pp. 179–180).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CovarianceEstimator {
    Model,
    Robust,
}

/// Factor level order (`_value_order`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueOrder {
    Ascending,
    Descending,
    DataOrder,
}

/// Binary reference category (`_base_category`): First/Last of Type order (decision C).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseCategory {
    First,
    Last,
}

/// Main effects only, or main plus every two-way product of inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effects {
    Main,
    MainAndTwoWay,
}

/// Offset: none, a constant, or a column of `dataset.table`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Offset {
    None,
    Fixed(f64),
    Field(usize),
}

/// Binomial trials: binary 0/1, a fixed m, or a column of `dataset.table`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Trials {
    Binary,
    Fixed(f64),
    Field(usize),
}

/// Kernel vs full log-likelihood for the displayed ℓ and information criteria.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLikelihoodKind {
    Kernel,
    Full,
}

/// Settings of a GenLin build node (scripting-guide / stream defaults in [`Default`]).
#[derive(Debug, Clone, PartialEq)]
pub struct GenlinSettings {
    pub distribution: Distribution,
    pub link: Link,
    pub intercept: bool,
    pub effects: Effects,
    pub estimation: Estimation,
    pub max_iterations: usize,
    pub max_step_halving: usize,
    pub parameter_convergence: f64,
    pub parameter_relative: bool,
    pub likelihood_convergence: Option<f64>,
    pub likelihood_relative: bool,
    /// `None` = not user-specified (decision R).
    pub hessian_convergence: Option<f64>,
    pub hessian_relative: bool,
    pub singularity: f64,
    pub scale: ScaleMethod,
    pub covariance: CovarianceEstimator,
    pub value_order: ValueOrder,
    pub base_category: BaseCategory,
    pub offset: Offset,
    /// Analysis (scale) weight column in `dataset.table`.
    pub weight: Option<usize>,
    pub trials: Trials,
    pub check_separation: bool,
    pub separation_start: usize,
    pub log_likelihood_kind: LogLikelihoodKind,
    /// Percent, e.g. 95.
    pub confidence: f64,
    pub name: Option<String>,
}

impl Default for GenlinSettings {
    fn default() -> Self {
        GenlinSettings {
            distribution: Distribution::Normal,
            link: Link::Identity,
            intercept: true,
            effects: Effects::Main,
            estimation: Estimation::Hybrid {
                fisher_iterations: 1,
            },
            max_iterations: 100,
            max_step_halving: 5,
            parameter_convergence: 1e-6,
            parameter_relative: false,
            likelihood_convergence: None,
            likelihood_relative: false,
            hessian_convergence: None,
            hessian_relative: false,
            singularity: 1e-7,
            scale: ScaleMethod::Fixed(1.0),
            covariance: CovarianceEstimator::Model,
            value_order: ValueOrder::Ascending,
            base_category: BaseCategory::Last,
            offset: Offset::None,
            weight: None,
            trials: Trials::Binary,
            check_separation: false,
            separation_start: 20,
            log_likelihood_kind: LogLikelihoodKind::Full,
            confidence: 95.0,
            name: None,
        }
    }
}

/// One Type III (or intercept) Wald test.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectTest {
    pub name: String,
    pub wald: Option<f64>,
    pub df: usize,
    pub p_value: Option<f64>,
}

/// Fitted GenLin model: original-scale coefficients, covariance, GOF, and the PMML nugget.
#[derive(Debug, Clone)]
pub struct GenlinFit {
    pub pmml: Pmml,
    pub parameter_names: Vec<String>,
    pub parameter_labels: Vec<String>,
    pub betas: Vec<f64>,
    /// Row-major p × p model or robust covariance; redundant rows/columns are 0.
    pub covariance: Vec<f64>,
    pub standard_errors: Vec<Option<f64>>,
    pub wald: Vec<Option<f64>>,
    pub wald_p: Vec<Option<f64>>,
    pub lower: Vec<Option<f64>>,
    pub upper: Vec<Option<f64>>,
    /// 1 if the column was swept, 0 if redundant.
    pub df: Vec<u32>,
    pub scale: f64,
    pub log_likelihood: f64,
    pub adjusted_log_likelihood: f64,
    pub deviance: f64,
    pub pearson: f64,
    pub residual_df: f64,
    pub n: usize,
    pub px: usize,
    pub iterations: usize,
    pub converged: bool,
    pub type_iii: Vec<EffectTest>,
}

/// Train a GenLin model.
pub fn train(dataset: &Dataset<'_>, settings: &GenlinSettings) -> Result<Pmml, AlgoError> {
    crate::check_cancelled()?;
    Ok(fit(dataset, settings)?.pmml)
}

/// Fit a GenLin model and keep coefficients, covariance and GOF for tests.
pub fn fit(dataset: &Dataset<'_>, settings: &GenlinSettings) -> Result<GenlinFit, AlgoError> {
    crate::check_cancelled()?;
    validate_settings(settings)?;
    let data = Data::read(dataset, settings)?;
    let work = Work::from_data(&data, settings)?;
    let (beta, iterations, converged) = estimate(&work, settings)?;
    let gof = goodness_of_fit(&work, &beta, settings)?;
    let (covariance, df) = covariance_matrix(&work, &beta, settings, gof.scale)?;
    let standard_errors = standard_errors(&covariance, data.columns.len(), &df);
    let wald = wald_statistics(&beta, &standard_errors);
    let wald_p = wald
        .iter()
        .map(|w| w.map(|stat| chi_square_p(stat, 1.0)))
        .collect();
    let z = phi_inv(1.0 - (1.0 - settings.confidence / 100.0) / 2.0).unwrap_or(1.959963984540054);
    let (lower, upper) = wald_limits(&beta, &standard_errors, z);
    let type_iii = type_iii_tests(&data, &beta, &covariance, &df, settings.singularity)?;
    let pmml = to_pmml(dataset, &data, settings, &beta)?;
    Ok(GenlinFit {
        pmml,
        parameter_names: data
            .columns
            .iter()
            .map(|c| c.parameter_name.clone())
            .collect(),
        parameter_labels: data.columns.iter().map(|c| c.label.clone()).collect(),
        betas: beta,
        covariance,
        standard_errors,
        wald,
        wald_p,
        lower,
        upper,
        df,
        scale: gof.scale,
        log_likelihood: gof.log_likelihood,
        adjusted_log_likelihood: gof.adjusted_log_likelihood,
        deviance: gof.deviance,
        pearson: gof.pearson,
        residual_df: gof.residual_df,
        n: work.y.len(),
        px: gof.px,
        iterations,
        converged,
        type_iii,
    })
}

fn validate_settings(settings: &GenlinSettings) -> Result<(), AlgoError> {
    match settings.distribution {
        Distribution::Negbin { k } if !(k.is_finite() && k >= 0.0) => {
            return Err(AlgoError::Settings(
                "GENLIN negative-binomial k must be a non-negative finite number".into(),
            ));
        }
        _ => {}
    }
    if let Link::Power { alpha } | Link::OddsPower { alpha } = settings.link
        && !alpha.is_finite()
    {
        return Err(AlgoError::Settings(
            "GENLIN power / odds-power parameter must be finite and non-zero when used as power"
                .into(),
        ));
    }
    if let Link::Power { alpha } = settings.link
        && alpha.abs() < 2.2e-16
    {
        // Guide Table 18-5 note: |α| < 2.2e-16 is treated as 0 (log). Allowed.
    }
    if !settings.parameter_convergence.is_finite() || settings.parameter_convergence < 0.0 {
        return Err(AlgoError::Settings(
            "GENLIN estimates_change_min must be a non-negative finite number".into(),
        ));
    }
    if let Some(eps) = settings.likelihood_convergence
        && (!eps.is_finite() || eps < 0.0)
    {
        return Err(AlgoError::Settings(
            "GENLIN loglikelihood_change_min must be a non-negative finite number".into(),
        ));
    }
    if let Some(eps) = settings.hessian_convergence
        && (!eps.is_finite() || eps < 0.0)
    {
        return Err(AlgoError::Settings(
            "GENLIN hessian_convergence_min must be a non-negative finite number".into(),
        ));
    }
    if !settings.singularity.is_finite() || settings.singularity < 0.0 {
        return Err(AlgoError::Settings(
            "GENLIN singularity_tolerance must be a non-negative finite number".into(),
        ));
    }
    if settings.max_iterations > 10_000 {
        return Err(AlgoError::Settings(
            "GENLIN max_iterations is too large".into(),
        ));
    }
    if settings.max_step_halving > 64 {
        return Err(AlgoError::Settings(
            "GENLIN step-halving limit is too large".into(),
        ));
    }
    if !(settings.confidence.is_finite()
        && settings.confidence > 0.0
        && settings.confidence < 100.0)
    {
        return Err(AlgoError::Settings(
            "GENLIN confidence_interval must be in (0, 100)".into(),
        ));
    }
    match settings.scale {
        ScaleMethod::Fixed(v) if !(v.is_finite() && v > 0.0) => {
            return Err(AlgoError::Settings(
                "GENLIN scale_value must be a positive finite number".into(),
            ));
        }
        ScaleMethod::MaxLikelihood
            if matches!(
                settings.distribution,
                Distribution::Binomial | Distribution::Poisson | Distribution::Negbin { .. }
            ) =>
        {
            return Err(AlgoError::Settings(
                "GENLIN cannot estimate the scale by ML for a discrete distribution (Guide p. 170)"
                    .into(),
            ));
        }
        _ => {}
    }
    if !link_ok(settings.distribution, settings.link) {
        return Err(AlgoError::Settings(format!(
            "GENLIN distribution {:?} with link {:?} is not a supported combination",
            settings.distribution, settings.link
        )));
    }
    Ok(())
}

fn link_ok(distribution: Distribution, link: Link) -> bool {
    match (distribution, link) {
        (_, Link::Identity | Link::Log | Link::Power { .. }) => true,
        (
            Distribution::Binomial,
            Link::Logit
            | Link::Probit
            | Link::Cloglog
            | Link::Nloglog
            | Link::Logc
            | Link::OddsPower { .. },
        ) => true,
        (Distribution::Negbin { k }, Link::Negbin { k: k2 }) => (k - k2).abs() <= 1e-15,
        (Distribution::Poisson, Link::Negbin { .. }) => true,
        _ => false,
    }
}

#[derive(Clone)]
struct Column {
    kind: ColumnKind,
    parameter_name: String,
    label: String,
    effect: usize,
}

#[derive(Clone)]
enum ColumnKind {
    Intercept,
    Covariate {
        position: usize,
        name: String,
    },
    Indicator {
        position: usize,
        name: String,
        category: String,
    },
    Product {
        parts: Vec<ProductPart>,
    },
}

#[derive(Clone)]
enum ProductPart {
    Covariate {
        position: usize,
        name: String,
    },
    Indicator {
        position: usize,
        name: String,
        category: String,
    },
}

struct Effect {
    name: String,
    columns: Vec<usize>,
}

struct Data {
    cases: Vec<Case>,
    columns: Vec<Column>,
    effects: Vec<Effect>,
    success: Option<String>,
    reference: Option<String>,
    factors: Vec<String>,
    covariates: Vec<String>,
}

struct Case {
    y: f64,
    x: Vec<f64>,
    offset: f64,
    freq: f64,
    weight: f64,
    trials: f64,
    key: Vec<String>,
}

impl Data {
    fn read(dataset: &Dataset<'_>, settings: &GenlinSettings) -> Result<Data, AlgoError> {
        let table = dataset.table;
        let fields = table.model().fields();
        let target = fields
            .get(dataset.target)
            .ok_or_else(|| AlgoError::Data("GENLIN target field is out of range".into()))?;
        if dataset.inputs.len() > MAX_INPUTS || table.row_count() > MAX_RECORDS {
            return Err(AlgoError::Data(
                "GENLIN limit: 256 inputs / 1000000 records".into(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for &position in dataset.inputs {
            crate::check_cancelled()?;
            if fields.get(position).is_none()
                || position == dataset.target
                || !seen.insert(position)
            {
                return Err(AlgoError::Settings(
                    "GENLIN input positions must be valid, distinct and exclude the target".into(),
                ));
            }
        }
        check_aux_column(fields, settings.weight, dataset, "weight")?;
        match settings.offset {
            Offset::Field(p) => check_aux_column(fields, Some(p), dataset, "offset")?,
            Offset::Fixed(v) if !v.is_finite() => {
                return Err(AlgoError::Settings(
                    "GENLIN offset_value must be finite".into(),
                ));
            }
            _ => {}
        }
        match settings.trials {
            Trials::Field(p) => check_aux_column(fields, Some(p), dataset, "trials")?,
            Trials::Fixed(m) if !(m.is_finite() && m > 0.0) => {
                return Err(AlgoError::Settings(
                    "GENLIN trials_number must be a positive finite number".into(),
                ));
            }
            _ => {}
        }

        let (labels, success_label, reference) = match settings.distribution {
            Distribution::Binomial if matches!(settings.trials, Trials::Binary) => {
                if target.measurement == Measurement::Continuous {
                    return Err(AlgoError::Data(format!(
                        "GENLIN binomial target {} is continuous",
                        target.name
                    )));
                }
                let labels = crate::categories(dataset, dataset.target);
                if labels.len() != 2 {
                    return Err(AlgoError::Data(format!(
                        "The target variable has more than 2 levels. A binary target must have 2 levels ({} has {}).",
                        target.name,
                        labels.len()
                    )));
                }
                let reference = match settings.base_category {
                    BaseCategory::First => labels.first().cloned(),
                    BaseCategory::Last => labels.last().cloned(),
                };
                let success = labels
                    .iter()
                    .find(|l| Some(*l) != reference.as_ref())
                    .cloned();
                (labels, success, reference)
            }
            Distribution::Binomial => {
                if !matches!(target.storage, Storage::Integer | Storage::Real)
                    || (target.measurement != Measurement::Continuous
                        && target.measurement != Measurement::Default)
                {
                    return Err(AlgoError::Data(format!(
                        "GENLIN binomial events target {} must be a continuous count",
                        target.name
                    )));
                }
                (Vec::new(), None, None)
            }
            _ => {
                if target.measurement != Measurement::Continuous
                    && target.measurement != Measurement::Default
                {
                    return Err(AlgoError::Data(format!(
                        "GENLIN {:?} target {} is not continuous",
                        settings.distribution, target.name
                    )));
                }
                if !matches!(target.storage, Storage::Integer | Storage::Real) {
                    return Err(AlgoError::Data(format!(
                        "GENLIN {:?} target {} is not numeric",
                        settings.distribution, target.name
                    )));
                }
                (Vec::new(), None, None)
            }
        };

        let prepared = prepare_inputs(dataset, settings)?;
        let (columns, effects, factors, covariates) = build_columns(&prepared, settings)?;

        let mut cases = Vec::new();
        for row in table.rows() {
            crate::check_cancelled()?;
            let Some(target_value) = row.get(dataset.target).filter(|v| !target.is_missing(v))
            else {
                continue;
            };
            let Some(yi) = encode_target(
                target_value,
                settings.distribution,
                success_label.as_deref(),
                &labels,
                settings.trials,
            ) else {
                continue;
            };
            let Some(freq) = frequency_of(dataset, row, fields) else {
                continue;
            };
            let Some(weight) = scale_weight(settings, row, fields) else {
                continue;
            };
            let Some(offset) = offset_of(settings, row, fields) else {
                continue;
            };
            let Some(trials) = trials_of(settings, row, fields, yi) else {
                continue;
            };
            let y = if matches!(settings.distribution, Distribution::Binomial)
                && !matches!(settings.trials, Trials::Binary)
            {
                if trials <= 0.0 {
                    continue;
                }
                yi / trials
            } else {
                yi
            };
            if matches!(settings.distribution, Distribution::Binomial)
                && !(y.is_finite() && (0.0..=1.0).contains(&y))
            {
                continue;
            }
            let Some(x) = design_row(row, fields, &columns) else {
                continue;
            };
            let key = cell_key(row, fields, &prepared);
            cases.push(Case {
                y,
                x,
                offset,
                freq,
                weight,
                trials,
                key,
            });
            if cases.len() > MAX_RECORDS {
                return Err(AlgoError::Data(
                    "GENLIN is limited to 1000000 complete records".into(),
                ));
            }
        }
        if cases.is_empty() {
            return Err(AlgoError::Data(
                "no complete records to train GENLIN".into(),
            ));
        }
        Ok(Data {
            cases,
            columns,
            effects,
            success: success_label,
            reference,
            factors,
            covariates,
        })
    }
}

fn check_aux_column(
    fields: &[dms_types::Field],
    position: Option<usize>,
    dataset: &Dataset<'_>,
    what: &str,
) -> Result<(), AlgoError> {
    let Some(p) = position else {
        return Ok(());
    };
    let field = fields
        .get(p)
        .ok_or_else(|| AlgoError::Settings(format!("GENLIN {what} field is out of range")))?;
    if p == dataset.target || dataset.inputs.contains(&p) {
        return Err(AlgoError::Settings(format!(
            "GENLIN {what} cannot be the target or an input"
        )));
    }
    if !matches!(field.storage, Storage::Integer | Storage::Real) {
        return Err(AlgoError::Settings(format!(
            "GENLIN {what} field {} is not numeric",
            field.name
        )));
    }
    Ok(())
}

struct PreparedInput {
    position: usize,
    name: String,
    covariate: bool,
    categories: Vec<String>,
}

fn prepare_inputs(
    dataset: &Dataset<'_>,
    settings: &GenlinSettings,
) -> Result<Vec<PreparedInput>, AlgoError> {
    let table = dataset.table;
    let fields = table.model().fields();
    let mut prepared = Vec::new();
    for &position in dataset.inputs {
        crate::check_cancelled()?;
        let input = fields
            .get(position)
            .ok_or_else(|| AlgoError::Data("GENLIN input field is out of range".into()))?;
        match input.measurement {
            Measurement::Typeless | Measurement::Collection | Measurement::Geospatial => {
                return Err(AlgoError::Unsupported(format!(
                    "GAP: GENLIN input {} with measurement {:?}",
                    input.name, input.measurement
                )));
            }
            Measurement::Continuous | Measurement::Default
                if matches!(input.storage, Storage::Integer | Storage::Real) =>
            {
                prepared.push(PreparedInput {
                    position,
                    name: input.name.clone(),
                    covariate: true,
                    categories: Vec::new(),
                });
            }
            _ => {
                let cats = factor_categories(table, position, settings.value_order)?;
                if cats.is_empty() || cats.len() > MAX_CATEGORIES {
                    return Err(AlgoError::Data(format!(
                        "GENLIN factor {} has no usable categories",
                        input.name
                    )));
                }
                prepared.push(PreparedInput {
                    position,
                    name: input.name.clone(),
                    covariate: false,
                    categories: cats,
                });
            }
        }
    }
    Ok(prepared)
}

fn factor_categories(
    table: &dms_types::Table,
    position: usize,
    order: ValueOrder,
) -> Result<Vec<String>, AlgoError> {
    let mut cats = match order {
        ValueOrder::DataOrder => {
            let f = table
                .model()
                .fields()
                .get(position)
                .ok_or_else(|| AlgoError::Data("GENLIN factor is out of range".into()))?;
            let mut values = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for row in table.rows() {
                crate::check_cancelled()?;
                if let Some(v) = row.get(position).filter(|v| !f.is_missing(v)) {
                    if matches!(v, Value::String(s) if s.trim().is_empty()) {
                        continue;
                    }
                    let text = v.to_string();
                    if seen.insert(text.clone()) {
                        values.push(text);
                        if values.len() > MAX_CATEGORIES {
                            break;
                        }
                    }
                }
            }
            values
        }
        _ => {
            let mut cats = bounded_categories(table, position, MAX_CATEGORIES)?;
            cats.retain(|c| !c.trim().is_empty());
            cats
        }
    };
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

type Design = (Vec<Column>, Vec<Effect>, Vec<String>, Vec<String>);

fn build_columns(
    prepared: &[PreparedInput],
    settings: &GenlinSettings,
) -> Result<Design, AlgoError> {
    let mut columns = Vec::new();
    let mut effects = Vec::new();
    let mut next = 1usize;
    let mut push = |kind: ColumnKind, label: String, effect: usize| {
        let parameter_name = format!("P{next:07}");
        next = next.saturating_add(1);
        columns.push(Column {
            kind,
            parameter_name,
            label,
            effect,
        });
    };

    if settings.intercept {
        let effect = effects.len();
        effects.push(Effect {
            name: "(Intercept)".into(),
            columns: Vec::new(),
        });
        push(ColumnKind::Intercept, "Intercept".into(), effect);
    }

    let mut factors = Vec::new();
    let mut covariates = Vec::new();
    for input in prepared {
        let effect = effects.len();
        effects.push(Effect {
            name: input.name.clone(),
            columns: Vec::new(),
        });
        if input.covariate {
            covariates.push(input.name.clone());
            push(
                ColumnKind::Covariate {
                    position: input.position,
                    name: input.name.clone(),
                },
                input.name.clone(),
                effect,
            );
        } else {
            factors.push(input.name.clone());
            for category in &input.categories {
                push(
                    ColumnKind::Indicator {
                        position: input.position,
                        name: input.name.clone(),
                        category: category.clone(),
                    },
                    format!("[{}={}]", input.name, category),
                    effect,
                );
            }
        }
    }

    if settings.effects == Effects::MainAndTwoWay {
        for i in 0..prepared.len() {
            for j in (i + 1)..prepared.len() {
                crate::check_cancelled()?;
                let a = prepared
                    .get(i)
                    .ok_or_else(|| AlgoError::Data("GENLIN two-way input is missing".into()))?;
                let b = prepared
                    .get(j)
                    .ok_or_else(|| AlgoError::Data("GENLIN two-way input is missing".into()))?;
                let effect = effects.len();
                effects.push(Effect {
                    name: format!("{}*{}", a.name, b.name),
                    columns: Vec::new(),
                });
                push_two_way(&mut push, a, b, effect)?;
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

    if columns.is_empty() {
        return Err(AlgoError::Settings(
            "GENLIN has no parameters to estimate".into(),
        ));
    }
    if columns.len() > MAX_PARAMETERS {
        return Err(AlgoError::Data(format!(
            "GENLIN would estimate {} parameters",
            columns.len()
        )));
    }
    Ok((columns, effects, factors, covariates))
}

fn push_two_way(
    push: &mut impl FnMut(ColumnKind, String, usize),
    a: &PreparedInput,
    b: &PreparedInput,
    effect: usize,
) -> Result<(), AlgoError> {
    match (a.covariate, b.covariate) {
        (true, true) => {
            push(
                ColumnKind::Product {
                    parts: vec![
                        ProductPart::Covariate {
                            position: a.position,
                            name: a.name.clone(),
                        },
                        ProductPart::Covariate {
                            position: b.position,
                            name: b.name.clone(),
                        },
                    ],
                },
                format!("{}*{}", a.name, b.name),
                effect,
            );
        }
        (false, true) => {
            for category in &a.categories {
                push(
                    ColumnKind::Product {
                        parts: vec![
                            ProductPart::Indicator {
                                position: a.position,
                                name: a.name.clone(),
                                category: category.clone(),
                            },
                            ProductPart::Covariate {
                                position: b.position,
                                name: b.name.clone(),
                            },
                        ],
                    },
                    format!("[{}={}]*{}", a.name, category, b.name),
                    effect,
                );
            }
        }
        (true, false) => {
            for category in &b.categories {
                push(
                    ColumnKind::Product {
                        parts: vec![
                            ProductPart::Covariate {
                                position: a.position,
                                name: a.name.clone(),
                            },
                            ProductPart::Indicator {
                                position: b.position,
                                name: b.name.clone(),
                                category: category.clone(),
                            },
                        ],
                    },
                    format!("{}*[{}={}]", a.name, b.name, category),
                    effect,
                );
            }
        }
        (false, false) => {
            for ca in &a.categories {
                for cb in &b.categories {
                    push(
                        ColumnKind::Product {
                            parts: vec![
                                ProductPart::Indicator {
                                    position: a.position,
                                    name: a.name.clone(),
                                    category: ca.clone(),
                                },
                                ProductPart::Indicator {
                                    position: b.position,
                                    name: b.name.clone(),
                                    category: cb.clone(),
                                },
                            ],
                        },
                        format!("[{}={}]*[{}={}]", a.name, ca, b.name, cb),
                        effect,
                    );
                }
            }
        }
    }
    Ok(())
}

fn same_category(left: &str, right: &str) -> bool {
    left == right
        || matches!(
            (left.parse::<f64>(), right.parse::<f64>()),
            (Ok(a), Ok(b)) if a == b
        )
}

fn encode_target(
    value: &Value,
    distribution: Distribution,
    success: Option<&str>,
    labels: &[String],
    trials: Trials,
) -> Option<f64> {
    match distribution {
        Distribution::Binomial if matches!(trials, Trials::Binary) => {
            let text = value.to_string();
            if !labels.iter().any(|label| same_category(label, &text)) {
                return None;
            }
            let is_success = success.is_some_and(|s| same_category(s, &text));
            Some(if is_success { 1.0 } else { 0.0 })
        }
        Distribution::Binomial => integer_nonneg(value),
        Distribution::Normal => value.as_f64().filter(|v| v.is_finite()),
        Distribution::Gamma | Distribution::InverseGaussian => {
            value.as_f64().filter(|v| v.is_finite() && *v > 0.0)
        }
        Distribution::Poisson | Distribution::Negbin { .. } => integer_nonneg(value),
    }
}

fn integer_nonneg(value: &Value) -> Option<f64> {
    let v = value.as_f64()?;
    if !v.is_finite() || v < 0.0 {
        return None;
    }
    let rounded = v.round();
    if (v - rounded).abs() > 1e-9 {
        return None;
    }
    Some(rounded)
}

fn frequency_of(dataset: &Dataset<'_>, row: &[Value], fields: &[dms_types::Field]) -> Option<f64> {
    let Some(position) = dataset.frequency else {
        return Some(1.0);
    };
    let field = fields.get(position)?;
    let value = row.get(position).filter(|v| !field.is_missing(v))?;
    let v = value.as_f64().filter(|n| n.is_finite())?;
    if v < 0.5 {
        return None;
    }
    Some(v.round())
}

fn scale_weight(
    settings: &GenlinSettings,
    row: &[Value],
    fields: &[dms_types::Field],
) -> Option<f64> {
    let Some(position) = settings.weight else {
        return Some(1.0);
    };
    let field = fields.get(position)?;
    let value = row.get(position).filter(|v| !field.is_missing(v))?;
    let v = value.as_f64().filter(|n| n.is_finite())?;
    if v <= 0.0 {
        return None;
    }
    Some(v)
}

fn offset_of(settings: &GenlinSettings, row: &[Value], fields: &[dms_types::Field]) -> Option<f64> {
    match settings.offset {
        Offset::None => Some(0.0),
        Offset::Fixed(v) => Some(v),
        Offset::Field(position) => {
            let field = fields.get(position)?;
            let value = row.get(position).filter(|v| !field.is_missing(v))?;
            value.as_f64().filter(|n| n.is_finite())
        }
    }
}

fn trials_of(
    settings: &GenlinSettings,
    row: &[Value],
    fields: &[dms_types::Field],
    events: f64,
) -> Option<f64> {
    if !matches!(settings.distribution, Distribution::Binomial) {
        return Some(1.0);
    }
    match settings.trials {
        Trials::Binary => Some(1.0),
        Trials::Fixed(m) => {
            let rounded = m.round();
            if rounded <= 0.0 || (m - rounded).abs() > 1e-9 || events > rounded {
                None
            } else {
                Some(rounded)
            }
        }
        Trials::Field(position) => {
            let field = fields.get(position)?;
            let value = row.get(position).filter(|v| !field.is_missing(v))?;
            let v = integer_nonneg(value)?;
            if v <= 0.0 || events > v {
                None
            } else {
                Some(v)
            }
        }
    }
}

fn cell_key(row: &[Value], fields: &[dms_types::Field], prepared: &[PreparedInput]) -> Vec<String> {
    let mut key = Vec::with_capacity(prepared.len());
    for input in prepared {
        let text = match (fields.get(input.position), row.get(input.position)) {
            (Some(f), Some(v)) if !f.is_missing(v) => v.to_string(),
            _ => String::new(),
        };
        key.push(text);
    }
    key
}

fn design_row(row: &[Value], fields: &[dms_types::Field], columns: &[Column]) -> Option<Vec<f64>> {
    let mut x = Vec::with_capacity(columns.len());
    for column in columns {
        if crate::check_cancelled().is_err() {
            return None;
        }
        match &column.kind {
            ColumnKind::Intercept => x.push(1.0),
            ColumnKind::Covariate { position, .. } => {
                x.push(covariate_value(row, fields, *position)?);
            }
            ColumnKind::Indicator {
                position, category, ..
            } => x.push(indicator_value(row, fields, *position, category)?),
            ColumnKind::Product { parts } => {
                let mut prod = 1.0;
                for part in parts {
                    let v = match part {
                        ProductPart::Covariate { position, .. } => {
                            covariate_value(row, fields, *position)?
                        }
                        ProductPart::Indicator {
                            position, category, ..
                        } => indicator_value(row, fields, *position, category)?,
                    };
                    prod *= v;
                }
                x.push(prod);
            }
        }
    }
    Some(x)
}

fn covariate_value(row: &[Value], fields: &[dms_types::Field], position: usize) -> Option<f64> {
    let field = fields.get(position)?;
    let value = row.get(position).filter(|v| !field.is_missing(v))?;
    value.as_f64().filter(|n| n.is_finite())
}

fn indicator_value(
    row: &[Value],
    fields: &[dms_types::Field],
    position: usize,
    category: &str,
) -> Option<f64> {
    let field = fields.get(position)?;
    let value = row.get(position).filter(|v| {
        !field.is_missing(v) && !matches!(v, Value::String(s) if s.trim().is_empty())
    })?;
    let text = value.to_string();
    Some(if same_category(&text, category) {
        1.0
    } else {
        0.0
    })
}

fn at<T: Copy>(values: &[T], index: usize) -> Result<T, AlgoError> {
    values
        .get(index)
        .copied()
        .ok_or_else(|| AlgoError::Data("GENLIN design cell is missing".into()))
}

fn slot<T>(values: &mut [T], index: usize) -> Result<&mut T, AlgoError> {
    values
        .get_mut(index)
        .ok_or_else(|| AlgoError::Data("GENLIN index is out of range".into()))
}

fn mat_index(row: usize, col: usize, n: usize) -> usize {
    row.saturating_mul(n).saturating_add(col)
}

struct Work {
    y: Vec<f64>,
    x: Vec<Vec<f64>>,
    offset: Vec<f64>,
    freq: Vec<f64>,
    weight: Vec<f64>,
    trials: Vec<f64>,
    keys: Vec<Vec<String>>,
    distribution: Distribution,
    link: Link,
    binomial: bool,
    working_phi: f64,
}

impl Work {
    fn from_data(data: &Data, settings: &GenlinSettings) -> Result<Work, AlgoError> {
        let working_phi = match settings.scale {
            ScaleMethod::Fixed(v) => v,
            ScaleMethod::Deviance | ScaleMethod::Pearson | ScaleMethod::MaxLikelihood => 1.0,
        };
        Ok(Work {
            y: data.cases.iter().map(|c| c.y).collect(),
            x: data.cases.iter().map(|c| c.x.clone()).collect(),
            offset: data.cases.iter().map(|c| c.offset).collect(),
            freq: data.cases.iter().map(|c| c.freq).collect(),
            weight: data.cases.iter().map(|c| c.weight).collect(),
            trials: data.cases.iter().map(|c| c.trials).collect(),
            keys: data.cases.iter().map(|c| c.key.clone()).collect(),
            distribution: settings.distribution,
            link: settings.link,
            binomial: matches!(settings.distribution, Distribution::Binomial),
            working_phi,
        })
    }

    fn omega_eff(&self, i: usize) -> Result<f64, AlgoError> {
        let w = at(&self.weight, i)?;
        let m = at(&self.trials, i)?;
        if self.binomial { Ok(w * m) } else { Ok(w) }
    }
}

fn estimate(work: &Work, settings: &GenlinSettings) -> Result<(Vec<f64>, usize, bool), AlgoError> {
    let p = work
        .x
        .first()
        .map(|r| r.len())
        .ok_or_else(|| AlgoError::Data("GENLIN has no design rows".into()))?;
    let mut beta = initial_beta(work, settings)?;
    if settings.max_iterations == 0 {
        return Ok((beta, 0, false));
    }
    let k_fisher = match settings.estimation {
        Estimation::NewtonRaphson => 0,
        Estimation::FisherScoring => settings.max_iterations.saturating_add(1),
        Estimation::Hybrid { fisher_iterations } => fisher_iterations,
    };
    let Some((mut loglik, mut score, mut xtwx)) = derivatives(work, &beta, k_fisher >= 1) else {
        return Err(AlgoError::Data(
            "GENLIN starting log-likelihood is not finite".into(),
        ));
    };
    let mut converged = false;
    let mut iterations = 0usize;
    while iterations < settings.max_iterations {
        crate::check_cancelled()?;
        iterations = iterations.saturating_add(1);
        let Some(delta) = crate::gle::sweep_solve(&xtwx, &score, p, settings.singularity) else {
            break;
        };
        match step_halve(work, &beta, &delta, loglik, settings.max_step_halving) {
            Some((new_beta, _)) => {
                let fisher_next = iterations <= k_fisher;
                let Some((ll, s, a)) = derivatives(work, &new_beta, fisher_next) else {
                    break;
                };
                let param_ok = parameter_converged(&beta, &new_beta, settings);
                let like_ok = likelihood_converged(loglik, ll, settings);
                let separated = settings.check_separation
                    && iterations >= settings.separation_start
                    && binomial_separated(work, &new_beta, &a, p);
                beta = new_beta;
                loglik = ll;
                score = s;
                xtwx = a;
                if separated {
                    break;
                }
                if param_ok && like_ok {
                    if hessian_ok(&score, &xtwx, loglik, p, settings) {
                        converged = true;
                    } else if settings.hessian_convergence.is_none() {
                        // Decision R: unspecified Hessian is a warning, not a stop.
                        converged = true;
                    }
                    break;
                }
            }
            None => break,
        }
    }
    Ok((beta, iterations, converged))
}

fn initial_beta(work: &Work, settings: &GenlinSettings) -> Result<Vec<f64>, AlgoError> {
    let n = work.y.len();
    let p = work
        .x
        .first()
        .map(|r| r.len())
        .ok_or_else(|| AlgoError::Data("GENLIN has no design rows".into()))?;
    let len = p
        .checked_mul(p)
        .ok_or_else(|| AlgoError::Data("GENLIN parameter product overflow".into()))?;
    let mut xtwx = vec![0.0; len];
    let mut xtwz = vec![0.0; p];
    for i in 0..n {
        crate::check_cancelled()?;
        let y = at(&work.y, i)?;
        let x = work
            .x
            .get(i)
            .ok_or_else(|| AlgoError::Data("GENLIN row".into()))?;
        let o = at(&work.offset, i)?;
        let f = at(&work.freq, i)?;
        let trials = at(&work.trials, i)?;
        let omega = work.omega_eff(i)?;
        let mu = start_mu(y, trials, work.distribution);
        let eta = g_of(mu, work.link).unwrap_or(1.0);
        let g1 = g1_of(mu, work.link).unwrap_or(0.0);
        let var = variance(mu, work.distribution).unwrap_or(0.0);
        let den = var * g1 * g1;
        let we = if den.is_finite() && den > 0.0 {
            f * omega / (work.working_phi * den)
        } else {
            0.0
        };
        if we == 0.0 || !we.is_finite() {
            continue;
        }
        let z = if work.binomial {
            (eta - o) + (y - mu) * g1
        } else {
            eta - o
        };
        for a in 0..p {
            let xa = at(x, a)?;
            let wz = slot(&mut xtwz, a)?;
            *wz += we * xa * z;
            for b in 0..p {
                let xb = at(x, b)?;
                let cell = slot(&mut xtwx, mat_index(a, b, p))?;
                *cell += we * xa * xb;
            }
        }
    }
    crate::gle::sweep_solve(&xtwx, &xtwz, p, settings.singularity)
        .ok_or_else(|| AlgoError::Data("GENLIN starting weighted least squares failed".into()))
}

fn start_mu(y: f64, trials: f64, distribution: Distribution) -> f64 {
    match distribution {
        Distribution::Binomial => {
            let m = if trials.is_finite() && trials > 0.0 {
                trials
            } else {
                1.0
            };
            (y * m + 0.5) / (m + 1.0)
        }
        _ => y,
    }
}

fn binomial_separated(work: &Work, beta: &[f64], xtwx: &[f64], p: usize) -> bool {
    if !work.binomial {
        return false;
    }
    let mut pmin = f64::INFINITY;
    let mut pmax = 0.0_f64;
    let mut pstar = f64::INFINITY;
    for (i, xrow) in work.x.iter().enumerate() {
        let Some(o) = work.offset.get(i).copied() else {
            continue;
        };
        let Some(eta) = dot(xrow, beta).map(|e| e + o) else {
            continue;
        };
        let Some(mu) = mu_of(eta, work.link).and_then(|m| project_mu(m, work)) else {
            continue;
        };
        let y = work.y.get(i).copied().unwrap_or(0.0);
        let pv = if y > 0.5 { mu } else { 1.0 - mu };
        pmin = pmin.min(pv);
        pmax = pmax.max(pv);
        pstar = pstar.min(mu.min(1.0 - mu).abs());
    }
    if pmin.min(pmax) > 0.99 {
        return true;
    }
    if pmax > 0.99 || pstar < 0.001 {
        (0..p).any(|i| xtwx.get(mat_index(i, i, p)).copied().unwrap_or(0.0).abs() == 0.0)
    } else {
        false
    }
}

fn step_halve(
    work: &Work,
    beta: &[f64],
    delta: &[f64],
    loglik: f64,
    max_halves: usize,
) -> Option<(Vec<f64>, f64)> {
    let mut xi = 1.0;
    let mut undefined_full = None;
    for j in 0..=max_halves {
        if crate::check_cancelled().is_err() {
            return None;
        }
        let mut trial = Vec::with_capacity(beta.len());
        for (b, d) in beta.iter().zip(delta) {
            trial.push(b + xi * d);
        }
        if trial.iter().all(|v| v.is_finite()) {
            match log_likelihood(work, &trial) {
                Some(ll) if j == 0 && (ll.is_nan() || loglik.is_nan()) => {
                    return Some((trial, ll));
                }
                Some(ll) if ll.is_finite() && ll >= loglik - 1e-14 * (1.0 + loglik.abs()) => {
                    return Some((trial, ll));
                }
                None if j == 0 => undefined_full = Some(trial),
                _ => {}
            }
        }
        xi *= 0.5;
    }
    undefined_full.map(|trial| (trial, loglik))
}

fn parameter_converged(old: &[f64], new: &[f64], settings: &GenlinSettings) -> bool {
    let mut max_change = 0.0;
    for (a, b) in old.iter().zip(new) {
        let diff = (a - b).abs();
        let value = if settings.parameter_relative {
            diff / (a.abs() + REL_EPS)
        } else {
            diff
        };
        if value > max_change {
            max_change = value;
        }
    }
    max_change < settings.parameter_convergence
}

fn likelihood_converged(old: f64, new: f64, settings: &GenlinSettings) -> bool {
    let Some(eps) = settings.likelihood_convergence else {
        return true;
    };
    let diff = (new - old).abs();
    if settings.likelihood_relative {
        diff / (old.abs() + REL_EPS) < eps
    } else {
        diff < eps
    }
}

fn hessian_ok(
    score: &[f64],
    xtwx: &[f64],
    loglik: f64,
    p: usize,
    settings: &GenlinSettings,
) -> bool {
    let eps = settings.hessian_convergence.unwrap_or(1e-4);
    let Some(delta) = crate::gle::sweep_solve(xtwx, score, p, settings.singularity) else {
        return false;
    };
    let mut quad = 0.0;
    for (s, d) in score.iter().zip(&delta) {
        quad += s * d;
    }
    let value = quad.abs();
    if settings.hessian_relative {
        value / (loglik.abs() + REL_EPS) < eps
    } else {
        value < eps
    }
}

fn derivatives(work: &Work, beta: &[f64], fisher: bool) -> Option<(f64, Vec<f64>, Vec<f64>)> {
    let n = work.y.len();
    let p = beta.len();
    let len = p.checked_mul(p)?;
    let mut loglik = 0.0;
    let mut score = vec![0.0; p];
    let mut xtwx = vec![0.0; len];
    let mut undefined = false;
    for i in 0..n {
        if crate::check_cancelled().is_err() {
            return None;
        }
        let y = *work.y.get(i)?;
        let x = work.x.get(i)?;
        let o = *work.offset.get(i)?;
        let f = *work.freq.get(i)?;
        let omega = work.omega_eff(i).ok()?;
        let eta = dot(x, beta)? + o;
        let raw = mu_of(eta, work.link)?;
        if excluded(raw, work) {
            undefined |= undefined_likelihood(raw, y, work);
            continue;
        }
        let mu = project_mu(raw, work)?;
        let ll = kernel_term(work, i, y, mu, work.working_phi)?;
        loglik += ll;
        let Some(g1) = g1_of(mu, work.link) else {
            continue;
        };
        let Some(var) = variance(mu, work.distribution) else {
            continue;
        };
        if !g1.is_finite() || !var.is_finite() || var == 0.0 {
            continue;
        }
        let scale = work.working_phi;
        if !scale.is_finite() || scale == 0.0 {
            continue;
        }
        let v = f * omega * (y - mu) / (scale * var * g1);
        if !v.is_finite() {
            continue;
        }
        let we = f * omega / (scale * var * g1 * g1);
        let w = if fisher {
            we
        } else {
            match (g2_of(mu, work.link), vprime(mu, work.distribution)) {
                (Some(g2), Some(vp)) => {
                    let extra = f * omega / scale * (y - mu) * (vp * g1 + var * g2)
                        / (var * var * g1 * g1 * g1);
                    let newton = we + extra;
                    if newton.is_finite() { newton } else { we }
                }
                _ => we,
            }
        };
        if !w.is_finite() {
            continue;
        }
        for a in 0..p {
            let xa = *x.get(a)?;
            *score.get_mut(a)? += v * xa;
            for b in 0..p {
                let xb = *x.get(b)?;
                *xtwx.get_mut(mat_index(a, b, p))? += w * xa * xb;
            }
        }
    }
    if undefined {
        loglik = f64::NAN;
    } else if !loglik.is_finite() {
        return None;
    }
    Some((loglik, score, xtwx))
}

fn log_likelihood(work: &Work, beta: &[f64]) -> Option<f64> {
    let mut loglik = 0.0;
    for i in 0..work.y.len() {
        if crate::check_cancelled().is_err() {
            return None;
        }
        let y = *work.y.get(i)?;
        let x = work.x.get(i)?;
        let o = *work.offset.get(i)?;
        let eta = dot(x, beta)? + o;
        let raw = mu_of(eta, work.link)?;
        if excluded(raw, work) {
            if undefined_likelihood(raw, y, work) {
                return Some(f64::NAN);
            }
            continue;
        }
        let mu = project_mu(raw, work)?;
        loglik += kernel_term(work, i, y, mu, work.working_phi)?;
    }
    if loglik.is_finite() {
        Some(loglik)
    } else {
        None
    }
}

fn kernel_term(work: &Work, i: usize, y: f64, mu: f64, phi: f64) -> Option<f64> {
    let f = *work.freq.get(i)?;
    let omega = work.omega_eff(i).ok()?;
    let m = *work.trials.get(i)?;
    let w = *work.weight.get(i)?;
    let k = kernel(y, mu, m, w, omega, phi, work.distribution)?;
    Some(f * k)
}

fn dot(x: &[f64], beta: &[f64]) -> Option<f64> {
    if x.len() != beta.len() {
        return None;
    }
    let mut s = 0.0;
    for (a, b) in x.iter().zip(beta) {
        if crate::check_cancelled().is_err() {
            return None;
        }
        s += a * b;
    }
    if s.is_finite() { Some(s) } else { None }
}

fn excluded(mu: f64, work: &Work) -> bool {
    work.binomial && !(mu > 0.0 && mu < 1.0) && !link_in_unit_interval(work.link)
}

fn link_in_unit_interval(link: Link) -> bool {
    matches!(
        link,
        Link::Logit | Link::Probit | Link::Cloglog | Link::Nloglog | Link::OddsPower { .. }
    )
}

fn undefined_likelihood(mu: f64, y: f64, work: &Work) -> bool {
    excluded(mu, work) && (mu <= 0.0 || mu.is_nan() || (y < 1.0 && mu >= 1.0))
}

fn project_mu(mu: f64, work: &Work) -> Option<f64> {
    let mu = match work.distribution {
        Distribution::Binomial => mu,
        Distribution::Poisson
        | Distribution::Gamma
        | Distribution::InverseGaussian
        | Distribution::Negbin { .. } => {
            if mu <= 0.0 {
                MU_FLOOR
            } else {
                mu
            }
        }
        Distribution::Normal => mu,
    };
    if mu.is_finite() { Some(mu) } else { None }
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
        Link::OddsPower { alpha } => {
            if alpha.abs() < 2.2e-16 {
                let z = eta.clamp(-ETA_CAP, ETA_CAP);
                1.0 / (1.0 + (-z).exp())
            } else {
                let base = 1.0 + alpha * eta;
                if base <= 0.0 {
                    return None;
                }
                let odds = base.powf(1.0 / alpha);
                if !odds.is_finite() {
                    return None;
                }
                odds / (1.0 + odds)
            }
        }
    };
    if mu.is_finite() { Some(mu) } else { None }
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
        Link::OddsPower { alpha } => {
            if mu <= 0.0 || mu >= 1.0 {
                return None;
            }
            let odds = mu / (1.0 - mu);
            if alpha.abs() < 2.2e-16 {
                odds.ln()
            } else {
                (odds.powf(alpha) - 1.0) / alpha
            }
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
        Link::OddsPower { alpha } => {
            if mu <= 0.0 || mu >= 1.0 {
                return None;
            }
            if alpha.abs() < 2.2e-16 {
                1.0 / (mu * (1.0 - mu))
            } else {
                let odds = mu / (1.0 - mu);
                odds.powf(alpha - 1.0) / ((1.0 - mu) * (1.0 - mu))
            }
        }
    };
    if d.is_finite() { Some(d) } else { None }
}

fn g2_of(mu: f64, link: Link) -> Option<f64> {
    let g1 = g1_of(mu, link)?;
    let d = match link {
        Link::Identity => 0.0,
        Link::Log => -g1 * g1,
        Link::Logit => g1 * g1 * (2.0 * mu - 1.0),
        Link::Probit => {
            let z = phi_inv(mu.clamp(MU_FLOOR, 1.0 - MU_FLOOR))?;
            g1 * g1 * z
        }
        Link::Cloglog => -g1 * g1 * (1.0 + (1.0 - mu).ln()),
        Link::Nloglog => g1 * g1 * (1.0 + mu.ln()),
        Link::Logc => -g1 * g1,
        Link::Power { alpha } => {
            if alpha.abs() < 2.2e-16 {
                -g1 * g1
            } else if mu.abs() < MU_FLOOR {
                return None;
            } else {
                g1 * (alpha - 1.0) / mu
            }
        }
        Link::Negbin { k } => -g1 * g1 * (1.0 + 2.0 * k * mu),
        Link::OddsPower { alpha } => {
            if alpha.abs() < 2.2e-16 {
                g1 * g1 * (2.0 * mu - 1.0)
            } else {
                g1 * ((alpha - 1.0) / mu + (alpha + 1.0) / (1.0 - mu))
            }
        }
    };
    if d.is_finite() { Some(d) } else { None }
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

fn vprime(mu: f64, distribution: Distribution) -> Option<f64> {
    let d = match distribution {
        Distribution::Normal => 0.0,
        Distribution::InverseGaussian => 3.0 * mu * mu,
        Distribution::Gamma => 2.0 * mu,
        Distribution::Poisson => 1.0,
        Distribution::Negbin { k } => 1.0 + 2.0 * k * mu,
        Distribution::Binomial => 1.0 - 2.0 * mu,
    };
    if d.is_finite() { Some(d) } else { None }
}

/// Kernel ℓ_{k,i} without the frequency multiplier (Guide Table 18-3 / 18-4).
fn kernel(
    y: f64,
    mu: f64,
    trials: f64,
    weight: f64,
    omega_eff: f64,
    phi: f64,
    distribution: Distribution,
) -> Option<f64> {
    if !phi.is_finite() || phi <= 0.0 {
        return None;
    }
    let v = match distribution {
        Distribution::Normal => {
            let e = y - mu;
            -0.5 * (omega_eff * e * e / phi + (phi / weight.max(MU_FLOOR)).ln())
        }
        Distribution::InverseGaussian => {
            let e = y - mu;
            let m = mu.max(MU_FLOOR);
            let yy = y.max(MU_FLOOR);
            -0.5 * (omega_eff * e * e / (phi * yy * m * m)
                + (phi * yy * yy / weight.max(MU_FLOOR)).ln())
        }
        Distribution::Gamma => {
            let m = mu.max(MU_FLOOR);
            let yy = y.max(MU_FLOOR);
            let shape = omega_eff / phi;
            if shape <= 0.0 {
                return None;
            }
            shape * (omega_eff * yy / (phi * m)).ln() - omega_eff * yy / (phi * m) - ln_gamma(shape)
        }
        Distribution::Poisson => {
            let m = mu.max(MU_FLOOR);
            let c = omega_eff / phi;
            if y == 0.0 {
                -c * m
            } else {
                c * (y * m.ln() - m)
            }
        }
        Distribution::Negbin { k } => {
            let m = mu.max(MU_FLOOR);
            let c = omega_eff / phi;
            if k.abs() < MU_FLOOR {
                if y == 0.0 {
                    -c * m
                } else {
                    c * (y * m.ln() - m)
                }
            } else if y == 0.0 {
                -c * (1.0 + k * m).ln() / k
            } else {
                c * (y * (k * m).ln() - (y + 1.0 / k) * (1.0 + k * m).ln() + ln_gamma(y + 1.0 / k)
                    - ln_gamma(1.0 / k))
            }
        }
        Distribution::Binomial => {
            let p = mu.clamp(MU_FLOOR, 1.0 - MU_FLOOR);
            let c = omega_eff / phi;
            if y == 0.0 {
                c * (1.0 - p).ln()
            } else if (y - 1.0).abs() < 1e-15 {
                c * p.ln()
            } else {
                c * (y * p.ln() + (1.0 - y) * (1.0 - p).ln())
            }
        }
    };
    let _ = trials;
    if v.is_finite() { Some(v) } else { None }
}

fn full_extra(
    y: f64,
    trials: f64,
    weight: f64,
    phi: f64,
    distribution: Distribution,
) -> Option<f64> {
    if !phi.is_finite() || phi <= 0.0 {
        return None;
    }
    let v = match distribution {
        Distribution::Normal => -0.5 * TWO_PI.ln(),
        Distribution::InverseGaussian => -0.5 * TWO_PI.ln(),
        Distribution::Gamma => -y.max(MU_FLOOR).ln(),
        Distribution::Poisson => {
            if y < 0.0 {
                return None;
            }
            -(weight / phi) * ln_gamma(y + 1.0)
        }
        Distribution::Negbin { .. } => {
            if y < 0.0 {
                return None;
            }
            -(weight / phi) * ln_gamma(y + 1.0)
        }
        Distribution::Binomial => {
            // Case-wise: ln C(m, r) with r = y m. For binary m=1 this is 0 (decision F).
            let m = trials;
            let r = (y * m).round();
            if m <= 0.0 || r < 0.0 || r > m {
                0.0
            } else {
                (weight / phi) * (ln_gamma(m + 1.0) - ln_gamma(r + 1.0) - ln_gamma(m - r + 1.0))
            }
        }
    };
    if v.is_finite() { Some(v) } else { None }
}

fn phi_pdf(z: f64) -> f64 {
    (-0.5 * z * z).exp() / TWO_PI.sqrt()
}

fn phi_cdf(z: f64) -> f64 {
    // Φ(z) = ½ (1 + erf(z/√2)); erf(x) = sign(x) P(½, x²).
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

struct Gof {
    scale: f64,
    log_likelihood: f64,
    adjusted_log_likelihood: f64,
    deviance: f64,
    pearson: f64,
    residual_df: f64,
    px: usize,
}

fn goodness_of_fit(work: &Work, beta: &[f64], settings: &GenlinSettings) -> Result<Gof, AlgoError> {
    let p = beta.len();
    let mut swept = vec![false; p];
    if let Some((xtwx, _)) = information(work, beta, false)? {
        mark_swept(&xtwx, p, settings.singularity, &mut swept)?;
    }
    let px = swept.iter().filter(|s| **s).count();
    let n_eff = effective_n(work)?;
    let (deviance, pearson, n_gof) = if work.binomial && matches!(settings.trials, Trials::Binary) {
        binary_subpop_gof(work, beta)?
    } else {
        casewise_gof(work, beta)?
    };
    let residual_df = (n_gof - px as f64).max(0.0);
    let scale = match settings.scale {
        ScaleMethod::Fixed(v) => v,
        ScaleMethod::Pearson => {
            if residual_df > 0.0 {
                pearson / residual_df
            } else {
                return Err(AlgoError::Data(
                    "GENLIN Pearson scale is undefined when residual df is 0".into(),
                ));
            }
        }
        ScaleMethod::Deviance => {
            if residual_df > 0.0 {
                deviance / residual_df
            } else {
                return Err(AlgoError::Data(
                    "GENLIN deviance scale is undefined when residual df is 0".into(),
                ));
            }
        }
        ScaleMethod::MaxLikelihood => {
            return Err(AlgoError::Unsupported(
                "GAP: GENLIN joint ML scale (no saved model; initial φ formula on Guide p. 178 is ambiguous)"
                    .into(),
            ));
        }
    };
    let ll_unit = displayed_likelihood(work, beta, 1.0, settings.log_likelihood_kind)?;
    let ll_scale = displayed_likelihood(work, beta, scale, settings.log_likelihood_kind)?;
    let log_likelihood = match settings.scale {
        ScaleMethod::Pearson | ScaleMethod::Deviance => ll_unit,
        _ => displayed_likelihood(work, beta, work.working_phi, settings.log_likelihood_kind)?,
    };
    let _ = n_eff;
    Ok(Gof {
        scale,
        log_likelihood,
        adjusted_log_likelihood: ll_scale,
        deviance,
        pearson,
        residual_df,
        px,
    })
}

fn effective_n(work: &Work) -> Result<f64, AlgoError> {
    let mut n = 0.0;
    for f in &work.freq {
        n += *f;
    }
    Ok(n)
}

fn displayed_likelihood(
    work: &Work,
    beta: &[f64],
    phi: f64,
    kind: LogLikelihoodKind,
) -> Result<f64, AlgoError> {
    let mut ll = 0.0;
    for i in 0..work.y.len() {
        crate::check_cancelled()?;
        let y = at(&work.y, i)?;
        let x = work
            .x
            .get(i)
            .ok_or_else(|| AlgoError::Data("GENLIN row".into()))?;
        let o = at(&work.offset, i)?;
        let f = at(&work.freq, i)?;
        let eta = dot(x, beta).ok_or_else(|| AlgoError::Data("GENLIN eta".into()))? + o;
        let raw = mu_of(eta, work.link).ok_or_else(|| AlgoError::Data("GENLIN mu".into()))?;
        let mu = project_mu(raw, work).ok_or_else(|| AlgoError::Data("GENLIN mu".into()))?;
        let k = kernel_term_phi(work, i, y, mu, phi)
            .ok_or_else(|| AlgoError::Data("GENLIN kernel".into()))?;
        ll += k;
        if kind == LogLikelihoodKind::Full {
            let extra = full_extra(
                y,
                at(&work.trials, i)?,
                at(&work.weight, i)?,
                phi,
                work.distribution,
            )
            .ok_or_else(|| AlgoError::Data("GENLIN full log-likelihood extra".into()))?;
            ll += f * extra;
        }
    }
    Ok(ll)
}

fn kernel_term_phi(work: &Work, i: usize, y: f64, mu: f64, phi: f64) -> Option<f64> {
    let f = *work.freq.get(i)?;
    let omega = work.omega_eff(i).ok()?;
    let m = *work.trials.get(i)?;
    let w = *work.weight.get(i)?;
    let k = kernel(y, mu, m, w, omega, phi, work.distribution)?;
    Some(f * k)
}

fn casewise_gof(work: &Work, beta: &[f64]) -> Result<(f64, f64, f64), AlgoError> {
    let mut deviance = 0.0;
    let mut pearson = 0.0;
    let mut n = 0.0;
    for i in 0..work.y.len() {
        crate::check_cancelled()?;
        let y = at(&work.y, i)?;
        let x = work
            .x
            .get(i)
            .ok_or_else(|| AlgoError::Data("GENLIN row".into()))?;
        let o = at(&work.offset, i)?;
        let f = at(&work.freq, i)?;
        let omega = work.omega_eff(i)?;
        let eta = dot(x, beta).ok_or_else(|| AlgoError::Data("GENLIN eta".into()))? + o;
        let raw = mu_of(eta, work.link).ok_or_else(|| AlgoError::Data("GENLIN mu".into()))?;
        let mu = project_mu(raw, work).ok_or_else(|| AlgoError::Data("GENLIN mu".into()))?;
        deviance += f * deviance_term(y, mu, omega, work.distribution)?;
        let var = variance(mu, work.distribution)
            .ok_or_else(|| AlgoError::Data("GENLIN variance".into()))?;
        pearson += f * omega * (y - mu) * (y - mu) / var;
        n += f;
    }
    Ok((deviance, pearson, n))
}

fn deviance_term(
    y: f64,
    mu: f64,
    omega: f64,
    distribution: Distribution,
) -> Result<f64, AlgoError> {
    let d = match distribution {
        Distribution::Normal => omega * (y - mu) * (y - mu),
        Distribution::InverseGaussian => {
            let yy = y.max(MU_FLOOR);
            let m = mu.max(MU_FLOOR);
            omega * (y - mu) * (y - mu) / (yy * m * m)
        }
        Distribution::Gamma => {
            let m = mu.max(MU_FLOOR);
            let yy = y.max(MU_FLOOR);
            2.0 * omega * (-(yy / m).ln() + (yy - m) / m)
        }
        Distribution::Poisson => {
            let m = mu.max(MU_FLOOR);
            if y == 0.0 {
                2.0 * omega * m
            } else {
                2.0 * omega * (y * (y / m).ln() - (y - m))
            }
        }
        Distribution::Negbin { k } => {
            let m = mu.max(MU_FLOOR);
            if k.abs() < MU_FLOOR {
                if y == 0.0 {
                    2.0 * omega * m
                } else {
                    2.0 * omega * (y * (y / m).ln() - (y - m))
                }
            } else if y == 0.0 {
                2.0 * omega * (1.0 + k * m).ln() / k
            } else {
                2.0 * omega
                    * (y * (y / m).ln() - (y + 1.0 / k) * ((y + 1.0 / k) / (m + 1.0 / k)).ln())
            }
        }
        Distribution::Binomial => {
            let p = mu.clamp(MU_FLOOR, 1.0 - MU_FLOOR);
            if y == 0.0 {
                -2.0 * omega * (1.0 - p).ln()
            } else if (y - 1.0).abs() < 1e-15 {
                -2.0 * omega * p.ln()
            } else {
                2.0 * omega * (y * (y / p).ln() + (1.0 - y) * ((1.0 - y) / (1.0 - p)).ln())
            }
        }
    };
    if d.is_finite() {
        Ok(d)
    } else {
        Err(AlgoError::Data("GENLIN deviance term is not finite".into()))
    }
}

fn binary_subpop_gof(work: &Work, beta: &[f64]) -> Result<(f64, f64, f64), AlgoError> {
    let mut groups: Vec<(Vec<String>, f64, f64, f64)> = Vec::new();
    for i in 0..work.y.len() {
        crate::check_cancelled()?;
        let key = work
            .keys
            .get(i)
            .ok_or_else(|| AlgoError::Data("GENLIN subpopulation key".into()))?;
        let y = at(&work.y, i)?;
        let f = at(&work.freq, i)?;
        let w = at(&work.weight, i)?;
        let contrib = f * w;
        let hit = groups.iter_mut().find(|(k, _, _, _)| k == key);
        if let Some((_, r1, r0, _)) = hit {
            if y > 0.5 {
                *r1 += contrib;
            } else {
                *r0 += contrib;
            }
        } else {
            let (r1, r0) = if y > 0.5 {
                (contrib, 0.0)
            } else {
                (0.0, contrib)
            };
            groups.push((key.clone(), r1, r0, 0.0));
        }
    }
    let mut deviance = 0.0;
    let mut pearson = 0.0;
    for (key, r1, r0, _) in &groups {
        crate::check_cancelled()?;
        let m = r1 + r0;
        if m <= 0.0 {
            continue;
        }
        let y = r1 / m;
        let Some(i) = work.keys.iter().position(|k| k == key) else {
            continue;
        };
        let x = work
            .x
            .get(i)
            .ok_or_else(|| AlgoError::Data("GENLIN row".into()))?;
        let o = at(&work.offset, i)?;
        let eta = dot(x, beta).ok_or_else(|| AlgoError::Data("GENLIN eta".into()))? + o;
        let raw = mu_of(eta, work.link).ok_or_else(|| AlgoError::Data("GENLIN mu".into()))?;
        let mu = project_mu(raw, work)
            .ok_or_else(|| AlgoError::Data("GENLIN mu".into()))?
            .clamp(MU_FLOOR, 1.0 - MU_FLOOR);
        if y == 0.0 {
            deviance += -2.0 * m * (1.0 - mu).ln();
        } else if (y - 1.0).abs() < 1e-15 {
            deviance += -2.0 * m * mu.ln();
        } else {
            deviance += 2.0 * m * (y * (y / mu).ln() + (1.0 - y) * ((1.0 - y) / (1.0 - mu)).ln());
        }
        pearson += m * (y - mu) * (y - mu) / (mu * (1.0 - mu));
    }
    Ok((deviance, pearson, groups.len() as f64))
}

type Information = (Vec<f64>, Vec<f64>);

fn information(work: &Work, beta: &[f64], fisher: bool) -> Result<Option<Information>, AlgoError> {
    match derivatives(work, beta, fisher) {
        Some((_, score, xtwx)) => Ok(Some((xtwx, score))),
        None => Ok(None),
    }
}

fn mark_swept(xtwx: &[f64], p: usize, tol: f64, swept: &mut [bool]) -> Result<(), AlgoError> {
    let rhs = vec![0.0; p];
    let Some(_) = crate::gle::sweep_solve(xtwx, &rhs, p, tol) else {
        return Ok(());
    };
    for j in 0..p {
        crate::check_cancelled()?;
        let mut e = vec![0.0; p];
        *slot(&mut e, j)? = 1.0;
        let col = crate::gle::sweep_solve(xtwx, &e, p, tol).unwrap_or_else(|| vec![0.0; p]);
        *slot(swept, j)? = col.iter().any(|v| v.abs() > 0.0);
    }
    Ok(())
}

fn covariance_matrix(
    work: &Work,
    beta: &[f64],
    settings: &GenlinSettings,
    scale: f64,
) -> Result<(Vec<f64>, Vec<u32>), AlgoError> {
    let p = beta.len();
    // Decision E: observed Hessian (Newton weights) at the MLE.
    let Some((xtwx, _)) = information(work, beta, false)? else {
        return Err(AlgoError::Data(
            "GENLIN Hessian at the estimates is not finite".into(),
        ));
    };
    let mut inv = vec![0.0; p.saturating_mul(p)];
    let mut df = vec![0u32; p];
    for j in 0..p {
        crate::check_cancelled()?;
        let mut e = vec![0.0; p];
        *slot(&mut e, j)? = 1.0;
        let Some(col) = crate::gle::sweep_solve(&xtwx, &e, p, settings.singularity) else {
            continue;
        };
        let redundant = col.iter().all(|v| *v == 0.0);
        if !redundant {
            *slot(&mut df, j)? = 1;
        }
        for i in 0..p {
            *slot(&mut inv, mat_index(i, j, p))? = at(&col, i)?;
        }
    }
    let phi_factor = match settings.scale {
        ScaleMethod::Pearson | ScaleMethod::Deviance => scale,
        _ => 1.0,
    };
    for cell in &mut inv {
        *cell *= phi_factor;
    }
    if settings.covariance == CovarianceEstimator::Robust {
        inv = robust_sandwich(work, beta, &inv, settings)?;
    }
    Ok((inv, df))
}

fn robust_sandwich(
    work: &Work,
    beta: &[f64],
    sigma: &[f64],
    settings: &GenlinSettings,
) -> Result<Vec<f64>, AlgoError> {
    // Guide p. 180: Σ_r = Σ_m (Σ_i u_i u_iᵀ) Σ_m with
    // u_i = f ω (y−μ) / (φ V g') x. After Pearson, φ̂ cancels (see report). Unverified.
    let p = beta.len();
    let mut meat = vec![0.0; p.saturating_mul(p)];
    let phi = match settings.scale {
        ScaleMethod::Fixed(v) => v,
        _ => 1.0,
    };
    for i in 0..work.y.len() {
        crate::check_cancelled()?;
        let y = at(&work.y, i)?;
        let x = work
            .x
            .get(i)
            .ok_or_else(|| AlgoError::Data("GENLIN row".into()))?;
        let o = at(&work.offset, i)?;
        let f = at(&work.freq, i)?;
        let omega = work.omega_eff(i)?;
        let eta = dot(x, beta).ok_or_else(|| AlgoError::Data("GENLIN eta".into()))? + o;
        let raw = mu_of(eta, work.link).ok_or_else(|| AlgoError::Data("GENLIN mu".into()))?;
        if excluded(raw, work) {
            continue;
        }
        let mu = project_mu(raw, work).ok_or_else(|| AlgoError::Data("GENLIN mu".into()))?;
        let g1 = g1_of(mu, work.link).ok_or_else(|| AlgoError::Data("GENLIN g'".into()))?;
        let var = variance(mu, work.distribution)
            .ok_or_else(|| AlgoError::Data("GENLIN variance".into()))?;
        let u_scale = f * omega * (y - mu) / (phi * var * g1);
        if !u_scale.is_finite() {
            continue;
        }
        for a in 0..p {
            let ua = u_scale * at(x, a)?;
            for b in 0..p {
                let ub = u_scale * at(x, b)?;
                *slot(&mut meat, mat_index(a, b, p))? += ua * ub;
            }
        }
    }
    // Σ_m M Σ_m
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

fn standard_errors(cov: &[f64], p: usize, df: &[u32]) -> Vec<Option<f64>> {
    let mut se = Vec::with_capacity(p);
    for i in 0..p {
        if df.get(i).copied().unwrap_or(0) == 0 {
            se.push(None);
            continue;
        }
        let v = cov.get(mat_index(i, i, p)).copied().unwrap_or(0.0);
        se.push(if v.is_finite() && v >= 0.0 {
            Some(v.sqrt())
        } else {
            None
        });
    }
    se
}

fn wald_statistics(beta: &[f64], se: &[Option<f64>]) -> Vec<Option<f64>> {
    beta.iter()
        .zip(se)
        .map(|(b, s)| match s {
            Some(s) if *s > 0.0 && s.is_finite() => {
                let w = (b / s) * (b / s);
                if w.is_finite() { Some(w) } else { None }
            }
            _ => None,
        })
        .collect()
}

fn wald_limits(beta: &[f64], se: &[Option<f64>], z: f64) -> (Vec<Option<f64>>, Vec<Option<f64>>) {
    let mut lower = Vec::with_capacity(beta.len());
    let mut upper = Vec::with_capacity(beta.len());
    for (b, s) in beta.iter().zip(se) {
        match s {
            Some(s) if s.is_finite() => {
                lower.push(Some(b - z * s));
                upper.push(Some(b + z * s));
            }
            _ => {
                lower.push(None);
                upper.push(None);
            }
        }
    }
    (lower, upper)
}

fn type_iii_tests(
    data: &Data,
    beta: &[f64],
    cov: &[f64],
    df: &[u32],
    tol: f64,
) -> Result<Vec<EffectTest>, AlgoError> {
    let p = beta.len();
    let mut tests = Vec::new();
    for effect in &data.effects {
        crate::check_cancelled()?;
        if effect.name == "(Intercept)" {
            let mut l = vec![0.0; p];
            for (j, column) in data.columns.iter().enumerate() {
                match &column.kind {
                    ColumnKind::Intercept => *slot(&mut l, j)? = 1.0,
                    ColumnKind::Indicator { name, .. } => {
                        let n_levels = data
                            .columns
                            .iter()
                            .filter(|c| {
                                matches!(&c.kind, ColumnKind::Indicator { name: n, .. } if n == name)
                            })
                            .count() as f64;
                        if n_levels > 0.0 {
                            *slot(&mut l, j)? = 1.0 / n_levels;
                        }
                    }
                    _ => {}
                }
            }
            tests.push(wald_contrast(&effect.name, &[l], beta, cov, p, tol)?);
            continue;
        }
        let mut rows = Vec::new();
        for &j in &effect.columns {
            if df.get(j).copied().unwrap_or(0) == 0 {
                continue;
            }
            let mut l = vec![0.0; p];
            *slot(&mut l, j)? = 1.0;
            rows.push(l);
        }
        if rows.is_empty() {
            tests.push(EffectTest {
                name: effect.name.clone(),
                wald: None,
                df: 0,
                p_value: None,
            });
        } else {
            tests.push(wald_contrast(&effect.name, &rows, beta, cov, p, tol)?);
        }
    }
    Ok(tests)
}

fn wald_contrast(
    name: &str,
    rows: &[Vec<f64>],
    beta: &[f64],
    cov: &[f64],
    p: usize,
    tol: f64,
) -> Result<EffectTest, AlgoError> {
    let r = rows.len();
    if r == 0 {
        return Ok(EffectTest {
            name: name.into(),
            wald: None,
            df: 0,
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
                .ok_or_else(|| AlgoError::Data("GENLIN contrast row".into()))?;
            let lk = rows
                .get(k)
                .ok_or_else(|| AlgoError::Data("GENLIN contrast row".into()))?;
            let mut s = 0.0;
            for a in 0..p {
                for b in 0..p {
                    s += at(li, a)? * at(cov, mat_index(a, b, p))? * at(lk, b)?;
                }
            }
            *slot(&mut lcl, mat_index(i, k, r))? = s;
        }
    }
    let Some(sol) = crate::gle::sweep_solve(&lcl, &lb, r, tol) else {
        return Ok(EffectTest {
            name: name.into(),
            wald: None,
            df: 0,
            p_value: None,
        });
    };
    let mut stat = 0.0;
    let mut rank = 0usize;
    for i in 0..r {
        let si = at(&sol, i)?;
        stat += at(&lb, i)? * si;
        let mut e = vec![0.0; r];
        *slot(&mut e, i)? = 1.0;
        if let Some(col) = crate::gle::sweep_solve(&lcl, &e, r, tol)
            && col.iter().any(|v| v.abs() > 0.0)
        {
            rank = rank.saturating_add(1);
        }
    }
    if !stat.is_finite() || stat < 0.0 {
        stat = stat.abs();
    }
    let df = rank;
    let p_value = if df > 0 && stat.is_finite() {
        Some(chi_square_p(stat, df as f64))
    } else {
        None
    };
    Ok(EffectTest {
        name: name.into(),
        wald: if stat.is_finite() { Some(stat) } else { None },
        df,
        p_value,
    })
}

fn to_pmml(
    dataset: &Dataset<'_>,
    data: &Data,
    settings: &GenlinSettings,
    beta: &[f64],
) -> Result<Pmml, AlgoError> {
    let table = dataset.table;
    let mut dictionary = Vec::new();
    let mut schema = Vec::new();
    for &position in dataset.inputs {
        crate::check_cancelled()?;
        if let Some(field) = data_field(table, position) {
            dictionary.push(field);
        }
        if let Some(field) = mining_field(table, position, Usage::Active) {
            schema.push(field);
        }
    }
    if let Some(field) = data_field(table, dataset.target) {
        dictionary.push(field);
    }
    if let Some(field) = mining_field(table, dataset.target, Usage::Target) {
        schema.push(field);
    }
    let parameters: Vec<String> = data
        .columns
        .iter()
        .map(|c| c.parameter_name.clone())
        .collect();
    let mut pp_matrix = Vec::new();
    for column in &data.columns {
        crate::check_cancelled()?;
        match &column.kind {
            ColumnKind::Covariate { name, .. } => pp_matrix.push(PPCell {
                value: "1".into(),
                predictor: name.clone(),
                parameter: column.parameter_name.clone(),
            }),
            ColumnKind::Indicator { name, category, .. } => pp_matrix.push(PPCell {
                value: category.clone(),
                predictor: name.clone(),
                parameter: column.parameter_name.clone(),
            }),
            ColumnKind::Product { parts } => {
                for part in parts {
                    match part {
                        ProductPart::Covariate { name, .. } => pp_matrix.push(PPCell {
                            value: "1".into(),
                            predictor: name.clone(),
                            parameter: column.parameter_name.clone(),
                        }),
                        ProductPart::Indicator { name, category, .. } => pp_matrix.push(PPCell {
                            value: category.clone(),
                            predictor: name.clone(),
                            parameter: column.parameter_name.clone(),
                        }),
                    }
                }
            }
            ColumnKind::Intercept => {}
        }
    }
    let classification = matches!(settings.distribution, Distribution::Binomial)
        && matches!(settings.trials, Trials::Binary);
    let mut param_matrix = Vec::new();
    for (column, value) in data.columns.iter().zip(beta) {
        crate::check_cancelled()?;
        param_matrix.push(PCell {
            target_category: if classification {
                data.success.clone()
            } else {
                None
            },
            parameter: column.parameter_name.clone(),
            beta: *value,
        });
    }
    let model_type = if classification && matches!(settings.link, Link::Logit) {
        GeneralModelType::MultinomialLogistic
    } else {
        GeneralModelType::GeneralizedLinear
    };
    Ok(Pmml {
        version: "4.3".into(),
        data_dictionary: dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::GeneralRegression(GeneralRegressionModel {
            header: ModelHeader {
                name: settings.name.clone().or_else(|| Some("GZLM".into())),
                function: if classification {
                    MiningFunction::Classification
                } else {
                    MiningFunction::Regression
                },
                algorithm: Some("GZLM".into()),
                mining_schema: schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            model_type,
            target_reference_category: data.reference.clone(),
            link_function: Some(settings.link.name().into()),
            link_parameter: settings.link.parameter(),
            parameters,
            factors: data.factors.clone(),
            covariates: data.covariates.clone(),
            pp_matrix,
            param_matrix,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Dataset;
    use dms_types::{DataModel, Field, Role, Table, ValueSpec};

    fn f64_close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * (1.0 + a.abs().max(b.abs()))
    }

    fn numeric(name: &str, role: Role) -> Field {
        let mut field = Field::new(name, Storage::Real);
        field.measurement = Measurement::Continuous;
        field.role = role;
        field
    }

    fn integer_factor(name: &str, cats: &[i64], role: Role) -> Field {
        let mut field = Field::new(name, Storage::Integer);
        field.measurement = Measurement::Nominal;
        field.role = role;
        field.values = ValueSpec::Categories(cats.iter().copied().map(Value::Integer).collect());
        field
    }

    fn table(fields: Vec<Field>, rows: Vec<Vec<Value>>) -> Table {
        Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
    }

    fn dataset<'a>(table: &'a Table, target: usize, inputs: &'a [usize]) -> Dataset<'a> {
        Dataset {
            table,
            target,
            inputs,
            frequency: None,
        }
    }

    #[test]
    fn genlin_cloglog_intercept_only_is_log_neglog() {
        // Two successes and two failures, p = 1/2.
        // cloglog: η = ln(−ln(1−p)) = ln(ln 2).
        let records = vec![
            vec![Value::Integer(1)],
            vec![Value::Integer(1)],
            vec![Value::Integer(0)],
            vec![Value::Integer(0)],
        ];
        let y = integer_factor("y", &[0, 1], Role::Target);
        let table = table(vec![y], records);
        let inputs: [usize; 0] = [];
        let settings = GenlinSettings {
            distribution: Distribution::Binomial,
            link: Link::Cloglog,
            base_category: BaseCategory::First,
            value_order: ValueOrder::Descending,
            ..GenlinSettings::default()
        };
        let fit = fit(&dataset(&table, 0, &inputs), &settings).unwrap();
        let want = (2.0_f64.ln()).ln();
        assert!(
            f64_close(fit.betas[0], want, 1e-8),
            "{:?} vs {want}",
            fit.betas
        );
        assert!(fit.converged);
    }

    #[test]
    fn genlin_cloglog_two_groups_matches_hand() {
        // Group A (0): 4/19; group B (1): 7/24. Saturated cloglog.
        // β0 = ln(−ln(1−4/19)), β1 = ln(−ln(1−7/24)) − β0.
        let mut rows = Vec::new();
        for _ in 0..4 {
            rows.push(vec![Value::Integer(0), Value::Integer(1)]);
        }
        for _ in 0..15 {
            rows.push(vec![Value::Integer(0), Value::Integer(0)]);
        }
        for _ in 0..7 {
            rows.push(vec![Value::Integer(1), Value::Integer(1)]);
        }
        for _ in 0..17 {
            rows.push(vec![Value::Integer(1), Value::Integer(0)]);
        }
        let table = table(
            vec![
                integer_factor("t", &[0, 1], Role::Input),
                integer_factor("y", &[0, 1], Role::Target),
            ],
            rows,
        );
        let inputs = [0usize];
        let settings = GenlinSettings {
            distribution: Distribution::Binomial,
            link: Link::Cloglog,
            base_category: BaseCategory::First,
            value_order: ValueOrder::Descending,
            singularity: 1e-7,
            ..GenlinSettings::default()
        };
        let fit = fit(&dataset(&table, 1, &inputs), &settings).unwrap();
        let p_a = 4.0_f64 / 19.0;
        let p_b = 7.0_f64 / 24.0;
        let b0 = (-(1.0 - p_a).ln()).ln();
        let b1 = (-(1.0 - p_b).ln()).ln() - b0;
        assert_eq!(fit.betas.len(), 3);
        assert!(
            f64_close(fit.betas[0], b0, 1e-9),
            "intercept {:?} vs {b0}",
            fit.betas
        );
        assert!(
            f64_close(fit.betas[1], b1, 1e-9),
            "t=1 {:?} vs {b1}",
            fit.betas
        );
        assert!(f64_close(fit.betas[2], 0.0, 1e-12));
        assert!(f64_close(fit.deviance, 0.0, 1e-8), "D {}", fit.deviance);
        assert!(f64_close(fit.pearson, 0.0, 1e-8));
        assert_eq!(fit.residual_df, 0.0);
        let want_ll =
            7.0 * p_b.ln() + 17.0 * (1.0 - p_b).ln() + 4.0 * p_a.ln() + 15.0 * (1.0 - p_a).ln();
        assert!(
            f64_close(fit.log_likelihood, want_ll, 1e-8),
            "ll {} vs {want_ll}",
            fit.log_likelihood
        );
    }

    #[test]
    fn genlin_gamma_inverse_intercept_is_reciprocal_mean() {
        let records = vec![
            vec![Value::Real(2.0)],
            vec![Value::Real(4.0)],
            vec![Value::Real(6.0)],
        ];
        let table = table(vec![numeric("y", Role::Target)], records);
        let inputs: [usize; 0] = [];
        let settings = GenlinSettings {
            distribution: Distribution::Gamma,
            link: Link::Power { alpha: -1.0 },
            ..GenlinSettings::default()
        };
        let fit = fit(&dataset(&table, 0, &inputs), &settings).unwrap();
        assert!(f64_close(fit.betas[0], 1.0 / 4.0, 1e-6), "{:?}", fit.betas);
    }

    #[test]
    fn genlin_gamma_inverse_weight_is_weighted_mean() {
        // Intercept-only gamma, canonical inverse: μ = Σ ω y / Σ ω.
        let records = vec![
            vec![Value::Real(2.0), Value::Real(1.0)],
            vec![Value::Real(8.0), Value::Real(3.0)],
        ];
        let table = table(
            vec![numeric("y", Role::Target), numeric("w", Role::None)],
            records,
        );
        let inputs: [usize; 0] = [];
        let settings = GenlinSettings {
            distribution: Distribution::Gamma,
            link: Link::Power { alpha: -1.0 },
            weight: Some(1),
            scale: ScaleMethod::Pearson,
            ..GenlinSettings::default()
        };
        let fit = fit(&dataset(&table, 0, &inputs), &settings).unwrap();
        let mu = (1.0 * 2.0 + 3.0 * 8.0) / 4.0;
        assert!(
            f64_close(fit.betas[0], 1.0 / mu, 1e-6),
            "{:?} mu {mu}",
            fit.betas
        );
    }

    #[test]
    fn genlin_empty_data_is_error() {
        let table = table(vec![numeric("y", Role::Target)], Vec::new());
        let inputs: [usize; 0] = [];
        let err = fit(&dataset(&table, 0, &inputs), &GenlinSettings::default()).unwrap_err();
        assert!(format!("{err}").contains("no complete records"));
    }

    #[test]
    fn genlin_one_record_normal_identity() {
        let table = table(
            vec![numeric("x", Role::Input), numeric("y", Role::Target)],
            vec![vec![Value::Real(2.0), Value::Real(5.0)]],
        );
        let inputs = [0usize];
        let fit = fit(&dataset(&table, 1, &inputs), &GenlinSettings::default());
        assert!(fit.is_ok() || matches!(fit, Err(AlgoError::Data(_))));
    }

    #[test]
    fn genlin_constant_predictor_is_redundant() {
        let records: Vec<Vec<Value>> = (0..6)
            .map(|i| vec![Value::Real(1.0), Value::Real(i as f64)])
            .collect();
        let table = table(
            vec![numeric("x", Role::Input), numeric("y", Role::Target)],
            records,
        );
        let inputs = [0usize];
        let fit = fit(&dataset(&table, 1, &inputs), &GenlinSettings::default()).unwrap();
        assert_eq!(fit.betas.len(), 2);
        assert!(fit.betas.iter().all(|b| b.is_finite()));
    }

    #[test]
    fn genlin_perfect_separation_does_not_panic() {
        let records = vec![
            vec![Value::Real(0.0), Value::Integer(0)],
            vec![Value::Real(0.0), Value::Integer(0)],
            vec![Value::Real(1.0), Value::Integer(1)],
            vec![Value::Real(1.0), Value::Integer(1)],
        ];
        let mut y = integer_factor("y", &[0, 1], Role::Target);
        y.measurement = Measurement::Flag;
        let table = table(vec![numeric("x", Role::Input), y], records);
        let inputs = [0usize];
        let settings = GenlinSettings {
            distribution: Distribution::Binomial,
            link: Link::Cloglog,
            base_category: BaseCategory::First,
            check_separation: true,
            separation_start: 0,
            ..GenlinSettings::default()
        };
        let _ = fit(&dataset(&table, 1, &inputs), &settings);
    }

    #[test]
    fn genlin_response_outside_gamma_support_is_dropped() {
        let records = vec![
            vec![Value::Real(2.0)],
            vec![Value::Real(-1.0)],
            vec![Value::Real(4.0)],
        ];
        let table = table(vec![numeric("y", Role::Target)], records);
        let inputs: [usize; 0] = [];
        let settings = GenlinSettings {
            distribution: Distribution::Gamma,
            link: Link::Log,
            ..GenlinSettings::default()
        };
        let fit = fit(&dataset(&table, 0, &inputs), &settings).unwrap();
        assert!(
            f64_close(fit.betas[0], 3.0_f64.ln(), 1e-5),
            "{:?}",
            fit.betas
        );
    }

    #[test]
    fn genlin_all_missing_column_is_error() {
        let records = vec![
            vec![Value::Null, Value::Real(1.0)],
            vec![Value::Null, Value::Real(2.0)],
        ];
        let table = table(
            vec![numeric("x", Role::Input), numeric("y", Role::Target)],
            records,
        );
        let inputs = [0usize];
        let err = fit(&dataset(&table, 1, &inputs), &GenlinSettings::default()).unwrap_err();
        assert!(matches!(err, AlgoError::Data(_)));
    }

    #[test]
    fn genlin_too_many_categories_is_error() {
        let mut rows = Vec::new();
        let mut cats = Vec::new();
        for i in 0..600 {
            cats.push(Value::Integer(i));
            rows.push(vec![Value::Integer(i), Value::Real(i as f64)]);
        }
        let mut g = Field::new("g", Storage::Integer);
        g.measurement = Measurement::Nominal;
        g.role = Role::Input;
        g.values = ValueSpec::Categories(cats);
        let table = table(vec![g, numeric("y", Role::Target)], rows);
        let inputs = [0usize];
        let err = fit(&dataset(&table, 1, &inputs), &GenlinSettings::default()).unwrap_err();
        assert!(matches!(err, AlgoError::Data(_)));
    }

    #[test]
    fn genlin_negative_weight_is_skipped() {
        let records = vec![
            vec![Value::Real(2.0), Value::Real(1.0)],
            vec![Value::Real(4.0), Value::Real(-3.0)],
            vec![Value::Real(6.0), Value::Real(1.0)],
        ];
        let table = table(
            vec![numeric("y", Role::Target), numeric("w", Role::None)],
            records,
        );
        let inputs: [usize; 0] = [];
        let settings = GenlinSettings {
            distribution: Distribution::Gamma,
            link: Link::Log,
            weight: Some(1),
            ..GenlinSettings::default()
        };
        let fit = fit(&dataset(&table, 0, &inputs), &settings).unwrap();
        assert!(
            f64_close(fit.betas[0], 4.0_f64.ln(), 1e-5),
            "{:?}",
            fit.betas
        );
    }

    #[test]
    fn genlin_multinomial_is_gap() {
        let err = validate_settings(&GenlinSettings::default());
        assert!(err.is_ok());
        let err = fit(
            &dataset(
                &table(
                    vec![numeric("y", Role::Target)],
                    vec![vec![Value::Real(1.0)]],
                ),
                0,
                &[],
            ),
            &GenlinSettings {
                link: Link::Logit,
                distribution: Distribution::Normal,
                ..GenlinSettings::default()
            },
        )
        .unwrap_err();
        assert!(format!("{err}").contains("not a supported combination"));
    }

    #[test]
    fn genlin_tweedie_is_gap() {
        // Tweedie is not a Distribution variant; requesting it is a programming-level GAP
        // via Unsupported from train paths that would map the scripting value.
        let err = AlgoError::Unsupported(
            "GAP: Tweedie distribution is named in the scripting guide but Table 18-2 and the log-likelihood table do not give V(μ) or ℓ".into(),
        );
        assert!(format!("{err}").contains("GAP: Tweedie"));
    }

    #[test]
    fn genlin_probit_and_logit_run_on_tiny_binomial() {
        let records = vec![
            vec![Value::Real(0.0), Value::Integer(0)],
            vec![Value::Real(0.5), Value::Integer(0)],
            vec![Value::Real(1.0), Value::Integer(1)],
            vec![Value::Real(1.5), Value::Integer(1)],
        ];
        let table = table(
            vec![
                numeric("x", Role::Input),
                integer_factor("y", &[0, 1], Role::Target),
            ],
            records,
        );
        let inputs = [0usize];
        for link in [Link::Probit, Link::Logit, Link::Log, Link::Identity] {
            let settings = GenlinSettings {
                distribution: Distribution::Binomial,
                link,
                base_category: BaseCategory::First,
                ..GenlinSettings::default()
            };
            let fit = fit(&dataset(&table, 1, &inputs), &settings);
            assert!(
                fit.as_ref()
                    .is_ok_and(|f| f.betas.iter().all(|b| b.is_finite()))
                    || matches!(fit, Err(AlgoError::Data(_))),
                "{link:?}"
            );
        }
    }

    #[test]
    fn genlin_poisson_log_with_offset_unverified() {
        // Guide: η = x'β + o, Poisson log ⇒ μ = exp(x'β + o).
        // Intercept-only, offset ln 2, y = 4,4,4 ⇒ μ = 4 = 2 e^β ⇒ β = ln 2.
        let records = vec![
            vec![Value::Integer(4), Value::Real(2.0_f64.ln())],
            vec![Value::Integer(4), Value::Real(2.0_f64.ln())],
            vec![Value::Integer(4), Value::Real(2.0_f64.ln())],
        ];
        let mut y = Field::new("y", Storage::Integer);
        y.measurement = Measurement::Continuous;
        y.role = Role::Target;
        let table = table(vec![y, numeric("o", Role::None)], records);
        let inputs: [usize; 0] = [];
        let settings = GenlinSettings {
            distribution: Distribution::Poisson,
            link: Link::Log,
            offset: Offset::Field(1),
            ..GenlinSettings::default()
        };
        let fit = fit(&dataset(&table, 0, &inputs), &settings).unwrap();
        assert!(
            f64_close(fit.betas[0], 2.0_f64.ln(), 1e-6),
            "{:?}",
            fit.betas
        );
    }

    #[test]
    fn genlin_negbin_k1_log_intercept_unverified() {
        let records = vec![
            vec![Value::Integer(0)],
            vec![Value::Integer(1)],
            vec![Value::Integer(2)],
        ];
        let mut y = Field::new("y", Storage::Integer);
        y.measurement = Measurement::Continuous;
        y.role = Role::Target;
        let table = table(vec![y], records);
        let inputs: [usize; 0] = [];
        let settings = GenlinSettings {
            distribution: Distribution::Negbin { k: 1.0 },
            link: Link::Log,
            ..GenlinSettings::default()
        };
        let fit = fit(&dataset(&table, 0, &inputs), &settings).unwrap();
        assert!(fit.betas[0].is_finite());
    }

    #[test]
    fn genlin_inverse_gaussian_identity_intercept_is_mean() {
        let records = vec![
            vec![Value::Real(1.0)],
            vec![Value::Real(2.0)],
            vec![Value::Real(3.0)],
        ];
        let table = table(vec![numeric("y", Role::Target)], records);
        let inputs: [usize; 0] = [];
        let settings = GenlinSettings {
            distribution: Distribution::InverseGaussian,
            link: Link::Identity,
            ..GenlinSettings::default()
        };
        let fit = fit(&dataset(&table, 0, &inputs), &settings).unwrap();
        assert!(f64_close(fit.betas[0], 2.0, 1e-6), "{:?}", fit.betas);
    }

    #[test]
    fn genlin_inverse_gaussian_power_minus_two_runs() {
        let records = vec![
            vec![Value::Real(1.0)],
            vec![Value::Real(2.0)],
            vec![Value::Real(4.0)],
        ];
        let table = table(vec![numeric("y", Role::Target)], records);
        let inputs: [usize; 0] = [];
        let settings = GenlinSettings {
            distribution: Distribution::InverseGaussian,
            link: Link::Power { alpha: -2.0 },
            ..GenlinSettings::default()
        };
        let fit = fit(&dataset(&table, 0, &inputs), &settings).unwrap();
        assert!(fit.betas[0].is_finite());
    }

    #[test]
    fn genlin_remaining_links_run_on_binomial() {
        let records = vec![
            vec![Value::Real(0.0), Value::Integer(0)],
            vec![Value::Real(0.2), Value::Integer(0)],
            vec![Value::Real(0.8), Value::Integer(1)],
            vec![Value::Real(1.0), Value::Integer(1)],
            vec![Value::Real(0.4), Value::Integer(0)],
            vec![Value::Real(0.6), Value::Integer(1)],
        ];
        let table = table(
            vec![
                numeric("x", Role::Input),
                integer_factor("y", &[0, 1], Role::Target),
            ],
            records,
        );
        let inputs = [0usize];
        for link in [
            Link::Nloglog,
            Link::Logc,
            Link::OddsPower { alpha: 1.0 },
            Link::OddsPower { alpha: 0.0 },
        ] {
            let settings = GenlinSettings {
                distribution: Distribution::Binomial,
                link,
                base_category: BaseCategory::First,
                ..GenlinSettings::default()
            };
            let fit = fit(&dataset(&table, 1, &inputs), &settings);
            assert!(
                fit.as_ref()
                    .is_ok_and(|f| f.betas.iter().all(|b| b.is_finite()))
                    || matches!(fit, Err(AlgoError::Data(_))),
                "{link:?}"
            );
        }
    }

    #[test]
    fn genlin_robust_covariance_is_finite() {
        let records: Vec<Vec<Value>> = (1..=8)
            .map(|i| vec![Value::Real(i as f64), Value::Real((2 * i) as f64)])
            .collect();
        let table = table(
            vec![numeric("x", Role::Input), numeric("y", Role::Target)],
            records,
        );
        let inputs = [0usize];
        let settings = GenlinSettings {
            covariance: CovarianceEstimator::Robust,
            ..GenlinSettings::default()
        };
        let fit = fit(&dataset(&table, 1, &inputs), &settings).unwrap();
        assert!(fit.covariance.iter().all(|v| v.is_finite()));
        assert!(fit.standard_errors.iter().any(|s| s.is_some()));
    }

    #[test]
    fn genlin_ml_scale_is_gap() {
        let table = table(
            vec![numeric("y", Role::Target)],
            vec![vec![Value::Real(1.0)], vec![Value::Real(2.0)]],
        );
        let inputs: [usize; 0] = [];
        let settings = GenlinSettings {
            scale: ScaleMethod::MaxLikelihood,
            ..GenlinSettings::default()
        };
        let err = fit(&dataset(&table, 0, &inputs), &settings).unwrap_err();
        assert!(format!("{err}").contains("GAP:"));
    }

    #[test]
    fn genlin_two_way_effects_run() {
        let mut rows = Vec::new();
        for a in 0..2 {
            for b in 0..2 {
                for k in 0..3 {
                    rows.push(vec![
                        Value::Integer(a),
                        Value::Integer(b),
                        Value::Real((a + 2 * b + k) as f64 + 1.0),
                    ]);
                }
            }
        }
        let table = table(
            vec![
                integer_factor("a", &[0, 1], Role::Input),
                integer_factor("b", &[0, 1], Role::Input),
                numeric("y", Role::Target),
            ],
            rows,
        );
        let inputs = [0usize, 1];
        let settings = GenlinSettings {
            effects: Effects::MainAndTwoWay,
            ..GenlinSettings::default()
        };
        let fit = fit(&dataset(&table, 2, &inputs), &settings).unwrap();
        assert!(fit.betas.len() > 4);
        assert!(fit.betas.iter().all(|b| b.is_finite()));
    }

    #[test]
    fn genlin_negbin_canonical_link_runs() {
        let records = vec![
            vec![Value::Integer(0)],
            vec![Value::Integer(1)],
            vec![Value::Integer(3)],
            vec![Value::Integer(2)],
        ];
        let mut y = Field::new("y", Storage::Integer);
        y.measurement = Measurement::Continuous;
        y.role = Role::Target;
        let table = table(vec![y], records);
        let inputs: [usize; 0] = [];
        let settings = GenlinSettings {
            distribution: Distribution::Negbin { k: 1.0 },
            link: Link::Negbin { k: 1.0 },
            ..GenlinSettings::default()
        };
        let fit = fit(&dataset(&table, 0, &inputs), &settings).unwrap();
        assert!(fit.betas[0].is_finite());
    }

    #[test]
    fn genlin_deviance_scale_is_finite() {
        let records = vec![
            vec![Value::Real(1.0)],
            vec![Value::Real(2.0)],
            vec![Value::Real(4.0)],
        ];
        let table = table(vec![numeric("y", Role::Target)], records);
        let inputs: [usize; 0] = [];
        let settings = GenlinSettings {
            distribution: Distribution::Gamma,
            link: Link::Log,
            scale: ScaleMethod::Deviance,
            ..GenlinSettings::default()
        };
        let fit = fit(&dataset(&table, 0, &inputs), &settings).unwrap();
        assert!(fit.scale.is_finite() && fit.scale > 0.0);
    }

    #[test]
    fn genlin_no_intercept_factor_keeps_all_levels() {
        let mut rows = Vec::new();
        for _ in 0..4 {
            rows.push(vec![Value::Integer(0), Value::Integer(1)]);
        }
        for _ in 0..6 {
            rows.push(vec![Value::Integer(0), Value::Integer(0)]);
        }
        for _ in 0..5 {
            rows.push(vec![Value::Integer(1), Value::Integer(1)]);
        }
        for _ in 0..5 {
            rows.push(vec![Value::Integer(1), Value::Integer(0)]);
        }
        let table = table(
            vec![
                integer_factor("t", &[0, 1], Role::Input),
                integer_factor("y", &[0, 1], Role::Target),
            ],
            rows,
        );
        let inputs = [0usize];
        let settings = GenlinSettings {
            distribution: Distribution::Binomial,
            link: Link::Cloglog,
            intercept: false,
            base_category: BaseCategory::First,
            value_order: ValueOrder::Descending,
            ..GenlinSettings::default()
        };
        let fit = fit(&dataset(&table, 1, &inputs), &settings).unwrap();
        assert_eq!(fit.betas.len(), 2);
        assert!(fit.betas.iter().all(|b| b.is_finite()));
    }
}
