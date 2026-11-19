//! Bayes Net (`spss.bayesian_id!buildBN` / `applyBN`): the build node's settings.
//!
//! The node saves its settings as inline `_...` properties (`ibm-demos/bayes_bankloan`). They map
//! to [`dms_algo::bayes_net::Settings`]; the training is [`dms_algo::bayes_net_model`]. A setting
//! that would change the result and is not implemented is a `GAP:`: custom always-selected
//! inputs, a frequency field, continued training of an existing model and the adjusted
//! propensity.
//!
//! Fields come from the Type roles (one target, inputs `Input` and `Both`).

use crate::error::OperatorError;
use crate::settings;
use dms_algo::bayes_net::{
    IndependenceTest, MissingValueHandling, ParameterLearningMethod, Settings, StructureType,
};
use dms_stream::StreamNode;

fn gap(what: impl std::fmt::Display) -> OperatorError {
    OperatorError::Settings(format!("GAP: Bayes Net {what}"))
}

/// The learner's settings from the node's `_` properties; absent ones have the node's declared
/// default.
pub(super) fn build_settings(node: &StreamNode) -> Result<Settings, OperatorError> {
    let s = node.settings.as_ref();
    let text = |key: &str| settings::text(s, key).map(|v| v.trim().to_string());
    let structure_type = match text("_structure_type").as_deref() {
        None | Some("" | "TAN") => StructureType::Tan,
        Some("MarkovBlanket") => StructureType::MarkovBlanket,
        Some(other) => return Err(gap(format!("_structure_type={other:?}"))),
    };
    let parameter_learning_method = match text("_parameter_learning_method").as_deref() {
        None | Some("" | "Likelihood") => ParameterLearningMethod::Likelihood,
        Some("Bayes") => ParameterLearningMethod::Bayes,
        Some(other) => return Err(gap(format!("_parameter_learning_method={other:?}"))),
    };
    let independence = match text("_independence").as_deref() {
        None | Some("" | "Likelihood") => IndependenceTest::Likelihood,
        Some("Pearson") => IndependenceTest::Pearson,
        Some(other) => return Err(gap(format!("_independence={other:?}"))),
    };
    let significance_level = settings::real(s, "_significance_level")?.unwrap_or(0.01);
    if !(significance_level > 0.0 && significance_level < 1.0) {
        return Err(OperatorError::Settings(format!(
            "Bayes Net _significance_level must be between 0 and 1, found {significance_level}"
        )));
    }
    let maximal_conditioning_set =
        usize::try_from(settings::integer(s, "_maximal_conditioning_set")?.unwrap_or(5)).map_err(
            |_| OperatorError::Settings("Bayes Net _maximal_conditioning_set is negative".into()),
        )?;
    let use_feature_selection = settings::flag(s, "_use_feature_selection")?.unwrap_or(false);
    // The input limit belongs to feature selection; without it every input is used.
    let maximum_number_inputs = if use_feature_selection {
        let limit = settings::integer(s, "_maximum_number_inputs")?.unwrap_or(10);
        Some(usize::try_from(limit).map_err(|_| {
            OperatorError::Settings("Bayes Net _maximum_number_inputs is negative".into())
        })?)
    } else {
        None
    };
    let missing_values = if settings::flag(s, "_missing_values")?.unwrap_or(true) {
        MissingValueHandling::Pairwise
    } else {
        MissingValueHandling::CompleteRecordsOnly
    };
    if settings::element(s, "_inputs_always_selected")
        .is_some_and(|e| e.child_elements().next().is_some() || !e.text().trim().is_empty())
    {
        return Err(gap("_inputs_always_selected"));
    }
    for (key, what) in [
        ("_use_frequency", "with a frequency field"),
        (
            "_continue_training_existing_model",
            "continuing to train an existing model",
        ),
        ("calculateAdjustedPropensities", "the adjusted propensity"),
    ] {
        if settings::flag(s, key)?.unwrap_or(false) {
            return Err(gap(what));
        }
    }
    Ok(Settings {
        structure_type,
        use_feature_selection,
        parameter_learning_method,
        independence,
        significance_level,
        maximal_conditioning_set,
        inputs_always_selected: Vec::new(),
        maximum_number_inputs,
        missing_values,
    })
}
