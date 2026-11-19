//! Generalized Linear (`spss.generalizedlinear_id!buildgeneralizedlinear` / `applygeneralizedlinear`):
//! the build node's settings.
//!
//! The node saves its settings as inline `_...` properties (`ibm-demos/ulcer_genlin`,
//! `car-insurance_genlin`, `ships_genlin`). They map to [`dms_algo::genlin::GenlinSettings`]; the
//! training is [`dms_algo::genlin`]. A setting that would change the result and is not
//! implemented is a `GAP:`: the Tweedie and multinomial distributions, the cumulative links, an
//! estimated negative binomial parameter and turning the parameter convergence test off.
//!
//! Fields come from the Type roles (one target, inputs `Input` and `Both`); the weight, offset and
//! trials fields come from the node's own properties and are taken out of the inputs.

use crate::error::OperatorError;
use crate::settings;
use dms_algo::genlin::{
    BaseCategory, CovarianceEstimator, Distribution, Effects, Estimation, GenlinSettings, Link,
    LogLikelihoodKind, Offset, ScaleMethod, Trials, ValueOrder,
};
use dms_stream::StreamNode;
use dms_types::Table;

fn gap(what: impl std::fmt::Display) -> OperatorError {
    OperatorError::Settings(format!("GAP: Generalized Linear {what}"))
}

/// The trainer's settings, and the fields the node names that the trainer addresses by position.
#[derive(Debug, Clone)]
pub(super) struct BuildSettings {
    pub settings: GenlinSettings,
    pub weight_field: Option<String>,
    pub offset_field: Option<String>,
    pub trials_field: Option<String>,
    /// The node's own target (`_target`); else the Type role decides.
    pub target: Option<String>,
    /// The node's own inputs (`_inputs`); else the Type roles decide.
    pub inputs: Option<Vec<String>>,
}

impl BuildSettings {
    /// The settings with the named fields resolved to columns of `table`.
    pub(super) fn resolve(&self, table: &Table) -> Result<GenlinSettings, OperatorError> {
        let position = |name: &str| {
            table.model().position(name).ok_or_else(|| {
                OperatorError::Data(format!(
                    "Generalized Linear: the field {name:?} is not in the input"
                ))
            })
        };
        let mut out = self.settings.clone();
        if let Some(name) = &self.weight_field {
            out.weight = Some(position(name)?);
        }
        if let Some(name) = &self.offset_field {
            out.offset = Offset::Field(position(name)?);
        }
        if let Some(name) = &self.trials_field {
            out.trials = Trials::Field(position(name)?);
        }
        Ok(out)
    }

    /// The names of the fields that are not predictors.
    pub(super) fn side_fields(&self) -> impl Iterator<Item = &str> {
        [&self.weight_field, &self.offset_field, &self.trials_field]
            .into_iter()
            .flatten()
            .map(String::as_str)
    }
}

pub(super) fn build_settings(node: &StreamNode) -> Result<BuildSettings, OperatorError> {
    let s = node.settings.as_ref();
    let text = |key: &str| settings::text(s, key).map(|v| v.trim().to_string());
    let flag = |key: &str, default: bool| -> Result<bool, OperatorError> {
        Ok(settings::flag(s, key)?.unwrap_or(default))
    };
    let real = |key: &str, default: f64| -> Result<f64, OperatorError> {
        Ok(settings::real(s, key)?.unwrap_or(default))
    };
    let count = |key: &str, default: usize| -> Result<usize, OperatorError> {
        match settings::integer(s, key)? {
            None => Ok(default),
            Some(n) => usize::try_from(n).map_err(|_| {
                OperatorError::Settings(format!("Generalized Linear {key} is negative"))
            }),
        }
    };

    let negbin_k = || -> Result<f64, OperatorError> {
        if !matches!(
            text("_negbin_para_type").as_deref(),
            None | Some("" | "SPECIFY")
        ) {
            return Err(gap("with an estimated negative binomial parameter"));
        }
        let k = real("_negbin_parameter", 1.0)?;
        if k.is_finite() && k >= 0.0 {
            Ok(k)
        } else {
            Err(OperatorError::Settings(format!(
                "Generalized Linear _negbin_parameter must not be negative, found {k}"
            )))
        }
    };
    let distribution = match text("_distribution").as_deref() {
        None | Some("" | "NORMAL") => Distribution::Normal,
        Some("BINOMIAL") => Distribution::Binomial,
        Some("GAMMA") => Distribution::Gamma,
        Some("IGAUSS") => Distribution::InverseGaussian,
        Some("POISSON") => Distribution::Poisson,
        Some("NEGBIN") => Distribution::Negbin { k: negbin_k()? },
        Some(other) => return Err(gap(format!("_distribution={other:?}"))),
    };
    let power = real("_power", 1.0)?;
    let link = match text("_link_function").as_deref() {
        None | Some("" | "IDENTITY") => Link::Identity,
        Some("LOG") => Link::Log,
        Some("LOGIT") => Link::Logit,
        Some("PROBIT") => Link::Probit,
        Some("CLOGLOG") => Link::Cloglog,
        Some("LOGC") => Link::Logc,
        Some("NLOGLOG") => Link::Nloglog,
        Some("POWER") => Link::Power { alpha: power },
        Some("ODDSPOWER") => Link::OddsPower { alpha: power },
        Some("NEGBIN") => Link::Negbin { k: negbin_k()? },
        Some(other) => return Err(gap(format!("_link_function={other:?}"))),
    };
    let effects = match text("_model_type").as_deref() {
        None | Some("" | "MainEffects") => Effects::Main,
        Some("MainAndAllTwoWayEffects") => Effects::MainAndTwoWay,
        Some(other) => return Err(gap(format!("_model_type={other:?}"))),
    };
    let estimation = match text("_method").as_deref() {
        None | Some("" | "Hybrid") => Estimation::Hybrid {
            fisher_iterations: count("_max_fisher_iterations", 1)?,
        },
        Some("Fisher") => Estimation::FisherScoring,
        Some("NewtonRaphson") => Estimation::NewtonRaphson,
        Some(other) => return Err(gap(format!("_method={other:?}"))),
    };
    let relative = |key: &str| -> Result<bool, OperatorError> {
        match text(key).as_deref() {
            None | Some("" | "Absolute") => Ok(false),
            Some("Relative") => Ok(true),
            Some(other) => Err(gap(format!("{key}={other:?}"))),
        }
    };
    if !flag("_estimates_change", true)? {
        return Err(gap("with the parameter convergence test turned off"));
    }
    let likelihood_convergence = flag("_loglikelihood_change", false)?
        .then(|| real("_loglikelihood_change_min", 1e-6))
        .transpose()?;
    let hessian_convergence = flag("_hessian_convergence", false)?
        .then(|| real("_hessian_convergence_min", 1e-4))
        .transpose()?;
    let scale = match text("_scale_method").as_deref() {
        None | Some("" | "FixedValue") => ScaleMethod::Fixed(real("_scale_value", 1.0)?),
        Some("PearsonChiSquare") => ScaleMethod::Pearson,
        Some("Deviance") => ScaleMethod::Deviance,
        Some("MaxLikelihoodEstimate") => ScaleMethod::MaxLikelihood,
        Some(other) => return Err(gap(format!("_scale_method={other:?}"))),
    };
    let covariance = match text("_covariance_matrix").as_deref() {
        None | Some("" | "ModelEstimator") => CovarianceEstimator::Model,
        Some("RobustEstimator") => CovarianceEstimator::Robust,
        Some(other) => return Err(gap(format!("_covariance_matrix={other:?}"))),
    };
    let value_order = match text("_value_order").as_deref() {
        None | Some("" | "Ascending") => ValueOrder::Ascending,
        Some("Descending") => ValueOrder::Descending,
        Some("DataOrder") => ValueOrder::DataOrder,
        Some(other) => return Err(gap(format!("_value_order={other:?}"))),
    };
    let base_category = match text("_base_category").as_deref() {
        None | Some("" | "Last") => BaseCategory::Last,
        Some("First") => BaseCategory::First,
        Some(other) => return Err(gap(format!("_base_category={other:?}"))),
    };
    let log_likelihood_kind = match text("_loglikelihood_function").as_deref() {
        None | Some("" | "Full") => LogLikelihoodKind::Full,
        Some("Kernel") => LogLikelihoodKind::Kernel,
        Some(other) => return Err(gap(format!("_loglikelihood_function={other:?}"))),
    };

    // offset: a field, or a constant (0 is no offset)
    let mut offset_field = None;
    let offset = match text("_offset_type").as_deref() {
        None | Some("") => Offset::None,
        Some("Variable") => {
            offset_field = text("_offset_field").filter(|name| !name.is_empty());
            Offset::None
        }
        Some("FixedValue") => match real("_offset_value", 0.0)? {
            0.0 => Offset::None,
            v => Offset::Fixed(v),
        },
        Some(other) => return Err(gap(format!("_offset_type={other:?}"))),
    };
    let weight_field = if flag("_use_weight", false)? {
        text("_weight_field").filter(|name| !name.is_empty())
    } else {
        None
    };
    let mut trials_field = None;
    let trials = if flag("_target_represents_trials", false)? {
        match text("_trials_type").as_deref() {
            None | Some("" | "Variable") => {
                trials_field = Some(
                    text("_trials_field")
                        .filter(|name| !name.is_empty())
                        .ok_or_else(|| {
                            OperatorError::Settings(
                                "Generalized Linear needs a trials field".into(),
                            )
                        })?,
                );
                Trials::Binary
            }
            Some("FixedValue") => Trials::Fixed(real("_trials_number", 10.0)?),
            Some(other) => return Err(gap(format!("_trials_type={other:?}"))),
        }
    } else {
        Trials::Binary
    };

    let settings = GenlinSettings {
        distribution,
        link,
        intercept: flag("_include_intercept", true)?,
        effects,
        estimation,
        max_iterations: count("_max_iterations", 100)?,
        max_step_halving: count("_max_step_halving", 5)?,
        parameter_convergence: real("_estimates_change_min", 1e-6)?,
        parameter_relative: relative("_estimates_change_type")?,
        likelihood_convergence,
        likelihood_relative: relative("_loglikelihood_change_type")?,
        hessian_convergence,
        hessian_relative: relative("_hessian_convergence_type")?,
        singularity: real("_singularity_tolerance", 1e-7)?,
        scale,
        covariance,
        value_order,
        base_category,
        offset,
        weight: None,
        trials,
        check_separation: flag("_check_separation", false)?,
        separation_start: count("_start_iteration", 20)?,
        log_likelihood_kind,
        confidence: real("_confidence_interval", 95.0)?,
        name: None,
    };
    let inputs: Vec<String> = settings::children(s, "_inputs", "field")
        .into_iter()
        .map(|e| e.text().trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    Ok(BuildSettings {
        settings,
        weight_field,
        offset_field,
        trials_field,
        target: text("_target").filter(|name| !name.is_empty()),
        inputs: (!inputs.is_empty()).then_some(inputs),
    })
}
