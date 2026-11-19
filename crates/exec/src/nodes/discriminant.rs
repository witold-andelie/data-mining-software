//! Discriminant (`spss.discriminant_id!builddiscriminant` / `applydiscriminant`): the build
//! node's settings and the nugget.
//!
//! The node saves its settings as inline `_...` properties (`ibm-demos/telco_custcat_discriminant`).
//! They map to [`dms_algo::discriminant::Settings`]; the training is
//! [`dms_algo::discriminant_model`]. Fields come from the Type roles. A stepwise criterion that is
//! not Wilks' lambda is read as the Guide defines it; the case-wise and plot options only shape
//! the output tables and are ignored.
//!
//! The nugget keeps the canonical functions, the group centroids and the priors in its `model`
//! container, and writes `$D-<target>` (the predicted group) and `$DP-<target>` (its posterior
//! probability). A record with a missing predictor scores `$null$`.

use super::field_util::{Column, with_columns};
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use crate::{ExecContext, NodeOutput};
use dms_algo::discriminant::{
    CovarianceMatrix, Method, PriorProbabilities, Settings, StepwiseCriteria, StepwiseCriterion,
};
use dms_pmml::{DiscriminantModel, Model, Pmml};
use dms_stream::StreamNode;
use dms_types::{Field, Measurement, Storage, Table, Value};
use std::sync::Arc;

fn gap(what: impl std::fmt::Display) -> OperatorError {
    OperatorError::Settings(format!("GAP: Discriminant {what}"))
}

pub(super) fn build_settings(node: &StreamNode) -> Result<Settings, OperatorError> {
    let s = node.settings.as_ref();
    let text = |key: &str| settings::text(s, key).map(|v| v.trim().to_string());
    let method = match text("_method").as_deref() {
        None | Some("" | "Enter") => Method::Enter,
        Some("Stepwise") => Method::Stepwise,
        Some(other) => return Err(gap(format!("_method={other:?}"))),
    };
    let stepwise_criterion = match text("_stepwise_method").as_deref() {
        None | Some("" | "WilksLambda") => StepwiseCriterion::WilksLambda,
        Some("UnexplainedVariance") => StepwiseCriterion::UnexplainedVariance,
        Some("MahalanobisDistance") => StepwiseCriterion::MahalanobisDistance,
        Some("SmallestF") => StepwiseCriterion::SmallestF,
        Some("RaosV") => StepwiseCriterion::RaosV,
        Some(other) => return Err(gap(format!("_stepwise_method={other:?}"))),
    };
    let stepwise_criteria = match text("_criteria").as_deref() {
        None | Some("" | "UseValue") => StepwiseCriteria::UseFValue {
            f_entry: settings::real(s, "_F_value_entry")?.unwrap_or(3.84),
            f_removal: settings::real(s, "_F_value_removal")?.unwrap_or(2.71),
        },
        Some("UseProbability") => StepwiseCriteria::UseProbability {
            p_entry: settings::real(s, "_probability_entry")?.unwrap_or(0.05),
            p_removal: settings::real(s, "_probability_removal")?.unwrap_or(0.1),
        },
        Some(other) => return Err(gap(format!("_criteria={other:?}"))),
    };
    let prior_probabilities = match text("_prior_probabilities").as_deref() {
        None | Some("" | "AllEqual") => PriorProbabilities::AllEqual,
        Some("ComputeFromSizes") => PriorProbabilities::ComputeFromSizes,
        Some(other) => return Err(gap(format!("_prior_probabilities={other:?}"))),
    };
    let covariance_matrix = match text("_covariance_matrix").as_deref() {
        None | Some("" | "WithinGroups") => CovarianceMatrix::WithinGroups,
        Some(other) => return Err(gap(format!("_covariance_matrix={other:?}"))),
    };
    if settings::flag(s, "_adjusted_propensity")?.unwrap_or(false) {
        return Err(gap("with adjusted propensities"));
    }
    let defaults = Settings::default();
    Ok(Settings {
        method,
        stepwise_criterion,
        stepwise_criteria,
        v_to_enter: settings::real(s, "_V_to_enter")?.unwrap_or(defaults.v_to_enter),
        prior_probabilities,
        covariance_matrix,
        compute_variable_importance: false,
        ..defaults
    })
}

struct Apply {
    member: Option<String>,
}

pub(super) fn apply(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let member = node
        .containers
        .iter()
        .find(|c| c.name.as_deref() == Some("model"))
        .map(|c| c.member.clone());
    Ok(Box::new(Apply { member }))
}

impl Operator for Apply {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        self.score(inputs, None, context)
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        self.score(inputs, model, context)
            .map(|table| NodeOutput::Data(Arc::new(table)))
    }
}

impl Apply {
    fn score(
        &self,
        inputs: &[Arc<Table>],
        model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("a Discriminant nugget needs one input".into()))?;
        let model: DiscriminantModel = match model.as_deref() {
            Some(Pmml {
                model: Model::Discriminant(model),
                ..
            }) => model.clone(),
            Some(_) => {
                return Err(OperatorError::Data(
                    "the nugget's model is not a Discriminant model".into(),
                ));
            }
            None => {
                let member = self.member.as_deref().ok_or_else(|| {
                    OperatorError::Settings(
                        "the nugget saved no model and its build node did not run".into(),
                    )
                })?;
                let bytes = context.attachments.get(member).ok_or_else(|| {
                    OperatorError::Io(format!("the stream file has no attachment {member}"))
                })?;
                // 4-byte length prefix and XML text
                let text = bytes
                    .get(4..)
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .ok_or_else(|| {
                        OperatorError::Data(format!("{member} is not length-prefixed XML text"))
                    })?;
                DiscriminantModel::from_spssml(text)
                    .map_err(|error| OperatorError::Data(format!("model: {error}")))?
            }
        };
        let positions = model
            .predictors
            .iter()
            .map(|name| {
                input.model().position(name).ok_or_else(|| {
                    OperatorError::Data(format!("model: the input has no field {name:?}"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let fields = input.model().fields();
        let numeric_groups = model.groups.iter().all(|g| g.trim().parse::<i64>().is_ok());
        let mut predicted = Vec::with_capacity(input.rows().len());
        let mut probability = Vec::with_capacity(input.rows().len());
        for row in input.rows() {
            let values: Vec<Option<f64>> = positions
                .iter()
                .map(|&p| {
                    let field = fields.get(p)?;
                    let value = row.get(p)?;
                    if field.is_missing(value) {
                        return None;
                    }
                    value.as_f64()
                })
                .collect();
            match model.score(&values) {
                Some(score) => {
                    let group = model
                        .groups
                        .get(score.predicted)
                        .cloned()
                        .unwrap_or_default();
                    predicted.push(if numeric_groups {
                        group
                            .trim()
                            .parse::<i64>()
                            .map_or(Value::Null, Value::Integer)
                    } else {
                        Value::string(group)
                    });
                    probability.push(
                        score
                            .posteriors
                            .get(score.predicted)
                            .map_or(Value::Null, |p| Value::real(*p)),
                    );
                }
                None => {
                    predicted.push(Value::Null);
                    probability.push(Value::Null);
                }
            }
        }
        let storage = if numeric_groups {
            Storage::Integer
        } else {
            Storage::String
        };
        let mut value_field = Field::new(format!("$D-{}", model.target), storage);
        value_field.measurement = Measurement::Categorical;
        let mut probability_field = Field::new(format!("$DP-{}", model.target), Storage::Real);
        probability_field.measurement = Measurement::Continuous;
        with_columns(
            input,
            vec![
                Column {
                    field: value_field,
                    values: predicted,
                },
                Column {
                    field: probability_field,
                    values: probability,
                },
            ],
        )
    }
}
