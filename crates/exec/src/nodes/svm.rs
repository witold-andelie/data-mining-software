//! SVM (`spss.svm_id!buildSVM` / `applySVM`): the build node's settings.
//!
//! The node saves its settings as inline `_...` properties (`ibm-demos/svm_cancer`). They map to
//! [`dms_algo::svm::Settings`]; the training is [`dms_algo::svm_model`]. The node's own `_target`
//! and `_inputs` (a field list) take the place of the Type roles when they are set. An unknown
//! kernel is a `GAP:`.

use crate::error::OperatorError;
use crate::settings;
use dms_algo::svm::{Kernel, Settings};
use dms_stream::StreamNode;

fn gap(what: impl std::fmt::Display) -> OperatorError {
    OperatorError::Settings(format!("GAP: SVM {what}"))
}

/// The trainer's settings and the node's own field choices.
#[derive(Debug, Clone)]
pub(super) struct BuildSettings {
    pub settings: Settings,
    pub target: Option<String>,
    pub inputs: Option<Vec<String>>,
}

pub(super) fn build_settings(node: &StreamNode) -> Result<BuildSettings, OperatorError> {
    let s = node.settings.as_ref();
    let text = |key: &str| settings::text(s, key).map(|v| v.trim().to_string());
    let real = |key: &str, default: f64| -> Result<f64, OperatorError> {
        Ok(settings::real(s, key)?.unwrap_or(default))
    };
    let kernel = match text("_kernel").as_deref() {
        None | Some("" | "RBF") => Kernel::Rbf {
            gamma: real("_rbf_gamma", 0.1)?,
        },
        Some("Polynomial") => {
            let degree = settings::integer(s, "_degree")?.unwrap_or(3);
            Kernel::Polynomial {
                gamma: real("_gamma", 1.0)?,
                coef0: real("_bias", 0.0)?,
                degree: u32::try_from(degree)
                    .map_err(|_| OperatorError::Settings(format!("SVM _degree is {degree}")))?,
            }
        }
        Some("Sigmoid") => Kernel::Sigmoid {
            gamma: real("_gamma", 1.0)?,
            coef0: real("_bias", 0.0)?,
        },
        Some(other) => return Err(gap(format!("_kernel={other:?}"))),
    };
    let defaults = Settings::default();
    let inputs: Vec<String> = settings::children(s, "_inputs", "field")
        .into_iter()
        .map(|e| e.text().trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    Ok(BuildSettings {
        settings: Settings {
            c: real("_regularization", defaults.c)?,
            stopping_criteria: real("_stopping_criteria", defaults.stopping_criteria)?,
            kernel,
            precision: real("_precision", defaults.precision)?,
            ..defaults
        },
        target: text("_target").filter(|name| !name.is_empty()),
        inputs: (!inputs.is_empty()).then_some(inputs),
    })
}
