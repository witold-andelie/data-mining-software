//! GLMM (`GLMM!spss.glmm-extension` / `GLMMModel!spss.glmm-extension`): the build node's settings.
//!
//! The node saves its settings as extension parameters (`glmm.*`) in a properties attachment
//! (`ibm-demos/testscores_glmm`); the fixed effects and the random-effect blocks are structured
//! values. They map to [`dms_algo::glmm::GlmmSettings`] and [`dms_algo::glmm_model`]. A setting
//! that would change the result and is not implemented is a `GAP:`: interactions between
//! fixed effects, random slopes, covariance structures other than variance components,
//! Satterthwaite degrees of freedom, weights, offsets and trials.
//!
//! The nugget scores the fixed part of the model with the saved PMML (`$L-<target>`), the same
//! way as a linear model's.

use crate::error::OperatorError;
use dms_algo::glmm::{
    CovarianceEstimator, CovarianceType, DfMethod, Distribution, GlmmSettings, Link, ValueOrder,
};
use dms_types::Table;

fn gap(what: impl std::fmt::Display) -> OperatorError {
    OperatorError::Settings(format!("GAP: GLMM {what}"))
}

/// The settings of a GLMM build node.
#[derive(Debug, Clone)]
pub(super) struct BuildSettings {
    pub settings: GlmmSettings,
    pub target: String,
    /// Fixed effects, one field each, in design order.
    pub fixed: Vec<String>,
    /// Random-effect blocks: subject fields, intercept, covariance type.
    pub random: Vec<(Vec<String>, bool, CovarianceType)>,
}

type Node<'a, 'b> = roxmltree::Node<'a, 'b>;

fn parameter<'a, 'b>(root: Node<'a, 'b>, name: &str) -> Option<Node<'a, 'b>> {
    root.descendants()
        .find(|n| n.has_tag_name("Parameter") && n.attribute("name") == Some(name))
}

fn text(root: Node<'_, '_>, name: &str) -> Option<String> {
    parameter(root, name)
        .and_then(|n| n.attribute("value"))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// The field names of an `Attribute name="Fields"` list below `node`.
fn field_list(node: Node<'_, '_>) -> Vec<String> {
    node.children()
        .filter(|a| a.has_tag_name("Attribute") && a.attribute("name") == Some("Fields"))
        .flat_map(|a| a.descendants().filter(|n| n.has_tag_name("Value")))
        .filter_map(|n| n.attribute("value").map(str::to_string))
        .collect()
}

pub(super) fn build_settings(xml: &str) -> Result<BuildSettings, OperatorError> {
    let document = roxmltree::Document::parse(xml)
        .map_err(|error| OperatorError::Data(format!("GLMM settings: {error}")))?;
    let root = document.root_element();
    for key in [
        "glmm.RegressionWeightField",
        "glmm.FrequencyWeightField",
        "glmm.OffsetField",
        "glmm.TrialsField",
    ] {
        if text(root, key).is_some() {
            return Err(gap(format!("with {key}")));
        }
    }
    if text(root, "glmm.UseTrials").as_deref() == Some("true") {
        return Err(gap("with trials"));
    }
    if !matches!(text(root, "glmm.UseOffset").as_deref(), None | Some("None")) {
        return Err(gap("with an offset"));
    }
    let target = text(root, "glmm.TargetField")
        .ok_or_else(|| OperatorError::Settings("GLMM needs a target field".into()))?;

    let mut fixed = Vec::new();
    if let Some(list) = parameter(root, "glmm.FixedEffectList") {
        for effect in list
            .children()
            .filter(|n| n.has_tag_name("ListValue"))
            .flat_map(|l| l.children().filter(|n| n.has_tag_name("StructuredValue")))
        {
            let fields = field_list(effect);
            let [field] = fields.as_slice() else {
                return Err(gap("with an interaction between fixed effects"));
            };
            fixed.push(field.clone());
        }
    }
    if fixed.is_empty() {
        return Err(gap("without fixed effects (the Type roles are not used)"));
    }

    let mut random = Vec::new();
    if let Some(list) = parameter(root, "glmm.RandomSpecs") {
        for spec in list
            .children()
            .filter(|n| n.has_tag_name("ListValue"))
            .flat_map(|l| l.children().filter(|n| n.has_tag_name("StructuredValue")))
        {
            let attribute = |name: &str| {
                spec.children()
                    .find(|a| a.has_tag_name("Attribute") && a.attribute("name") == Some(name))
            };
            if attribute("EffectList")
                .is_some_and(|a| a.descendants().any(|n| n.has_tag_name("Value")))
            {
                return Err(gap("with random slopes"));
            }
            if attribute("GroupSpec")
                .is_some_and(|a| a.descendants().any(|n| n.has_tag_name("Value")))
            {
                return Err(gap("with a random-effect group"));
            }
            let intercept =
                attribute("Intercept").and_then(|a| a.attribute("value")) != Some("false");
            let subjects = attribute("SubjectSpec")
                .and_then(|a| a.children().find(|n| n.has_tag_name("StructuredValue")))
                .map(field_list)
                .unwrap_or_default();
            if subjects.is_empty() {
                return Err(gap("with a random effect that has no subject"));
            }
            let covariance = match attribute("CovarianceType").and_then(|a| a.attribute("value")) {
                None | Some("VC") => CovarianceType::VarianceComponents,
                Some("ID") => CovarianceType::Identity,
                Some("DIAG") => CovarianceType::Diagonal,
                Some(other) => return Err(gap(format!("random-effect covariance {other:?}"))),
            };
            random.push((subjects, intercept, covariance));
        }
    }
    if random.is_empty() {
        return Err(gap("without random effects"));
    }

    let mut settings = GlmmSettings {
        distribution: match text(root, "glmm.DistributionOfTarget").as_deref() {
            None | Some("NORMAL") => Distribution::Normal,
            Some("BINOMIAL") => Distribution::Binomial,
            Some("GAMMA") => Distribution::Gamma,
            Some("IGAUSS") => Distribution::InverseGaussian,
            Some("POISSON") => Distribution::Poisson,
            Some(other) => return Err(gap(format!("distribution {other:?}"))),
        },
        link: match text(root, "glmm.LinkFunctionType").as_deref() {
            None | Some("IDENTITY") => Link::Identity,
            Some("LOG") => Link::Log,
            Some("LOGIT") => Link::Logit,
            Some("PROBIT") => Link::Probit,
            Some("CLOGLOG") => Link::Cloglog,
            Some("LOGC") => Link::Logc,
            Some("NLOGLOG") => Link::Nloglog,
            Some(other) => return Err(gap(format!("link {other:?}"))),
        },
        intercept: text(root, "glmm.Intercept").as_deref() != Some("false"),
        ..GlmmSettings::default()
    };
    settings.residual = match text(root, "glmm.ResidualCovarianceType").as_deref() {
        None | Some("ID") => CovarianceType::Identity,
        Some("DIAG") => CovarianceType::Diagonal,
        Some(other) => return Err(gap(format!("residual covariance {other:?}"))),
    };
    settings.df_method = match text(root, "glmm.DegreesFreedom").as_deref() {
        None | Some("Residual") => DfMethod::Residual,
        Some(other) => return Err(gap(format!("degrees of freedom {other:?}"))),
    };
    settings.covariance = match text(root, "glmm.AsymEffCov").as_deref() {
        None | Some("Model") => CovarianceEstimator::Model,
        Some("Robust") => CovarianceEstimator::Robust,
        Some(other) => return Err(gap(format!("covariance {other:?}"))),
    };
    settings.value_order = match text(root, "glmm.SortFactors").as_deref() {
        None | Some("Ascending") => ValueOrder::Ascending,
        Some("Descending") => ValueOrder::Descending,
        Some(other) => return Err(gap(format!("factor order {other:?}"))),
    };
    if let Some(n) = text(root, "glmm.MaxIterations").and_then(|v| v.parse::<usize>().ok()) {
        settings.max_iterations = n;
    }
    if let Some(c) = text(root, "glmm.CIn").and_then(|v| v.parse::<f64>().ok()) {
        settings.confidence = c;
    }
    if let Some(t) = text(root, "glmm.SingTolerance").and_then(|v| v.parse::<f64>().ok()) {
        settings.singularity = t;
    }
    Ok(BuildSettings {
        settings,
        target,
        fixed,
        random,
    })
}

impl BuildSettings {
    /// The positions of the fields the node names in `table`.
    pub(super) fn positions(
        &self,
        table: &Table,
    ) -> Result<(usize, Vec<usize>, Vec<dms_algo::glmm_model::RandomBlock>), OperatorError> {
        let position = |name: &str| {
            table.model().position(name).ok_or_else(|| {
                OperatorError::Data(format!("GLMM: the field {name:?} is not in the input"))
            })
        };
        let target = position(&self.target)?;
        let fixed = self
            .fixed
            .iter()
            .map(|n| position(n))
            .collect::<Result<Vec<_>, _>>()?;
        let random = self
            .random
            .iter()
            .map(|(subjects, intercept, covariance)| {
                Ok(dms_algo::glmm_model::RandomBlock {
                    subjects: subjects
                        .iter()
                        .map(|n| position(n))
                        .collect::<Result<Vec<_>, _>>()?,
                    intercept: *intercept,
                    covariance: *covariance,
                })
            })
            .collect::<Result<Vec<_>, OperatorError>>()?;
        Ok((target, fixed, random))
    }
}
