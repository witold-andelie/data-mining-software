//! Self-Learning Response Model (`spss.selflearning!selflearning` / `applyselflearning`): the
//! build node's settings and the nugget.
//!
//! The build node saves its settings as inline `_...` properties (`ibm-demos/pm_selflearn`). The
//! target (the product) and the response field are named by the node; the predictors are the Type
//! inputs (or the node's own `_inputs`). Continuing the training of an existing model needs the
//! state Modeler keeps in the node's `lastModel` container, which is not read: a node that has
//! one is a `GAP:`.
//!
//! The nugget writes, for each rank `k` up to `_max_predictions`, `$S-<target>-k` (the product
//! at that rank) and `$SC-<target>-k` (its score). The score is the response probability of the
//! product's Naive Bayes model, shrunk towards 0.5 with the model's size when the reliability
//! option is on; a randomisation above 0 and offer preferences are `GAP:`s (their random
//! numbers are not specified).

use super::field_util::{Column, with_columns};
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use crate::{ExecContext, NodeOutput};
use dms_algo::slrm::{Settings, SortOrder};
use dms_pmml::{Model, Pmml, Value as PValue};
use dms_stream::StreamNode;
use dms_types::{Field, Measurement, Storage, Table, Value};
use std::sync::Arc;

fn gap(what: impl std::fmt::Display) -> OperatorError {
    OperatorError::Settings(format!("GAP: SLRM {what}"))
}

/// The trainer's settings and the node's own field choices.
#[derive(Debug, Clone)]
pub(super) struct BuildSettings {
    pub settings: Settings,
    pub target: String,
    pub response: String,
    pub inputs: Option<Vec<String>>,
}

fn common(node: &StreamNode) -> Result<Settings, OperatorError> {
    let s = node.settings.as_ref();
    let randomization = settings::real(s, "_randomization")?.unwrap_or(0.0);
    if randomization != 0.0 {
        return Err(gap(
            "with a randomisation above 0 (its random numbers are not specified)",
        ));
    }
    let mut out = Settings::default();
    if let Some(n) = settings::integer(s, "_max_predictions")? {
        out.max_predictions = usize::try_from(n)
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| OperatorError::Settings(format!("SLRM _max_predictions is {n}")))?;
    }
    if let Some(flag) = settings::flag(s, "_model_reliability")? {
        out.model_reliability = flag;
    }
    out.sort = match settings::text(s, "_sort").as_deref().map(str::trim) {
        None | Some("" | "descending") => SortOrder::Descending,
        Some("ascending") => SortOrder::Ascending,
        Some(other) => return Err(gap(format!("_sort={other:?}"))),
    };
    Ok(out)
}

pub(super) fn build_settings(node: &StreamNode) -> Result<BuildSettings, OperatorError> {
    let s = node.settings.as_ref();
    let text = |key: &str| settings::text(s, key).map(|v| v.trim().to_string());
    if settings::flag(s, "_use_frequency")?.unwrap_or(false) {
        return Err(gap("with a frequency field"));
    }
    if settings::flag(s, "_custom_fields")?.unwrap_or(false) {
        return Err(gap("with custom field roles"));
    }
    if node
        .containers
        .iter()
        .any(|c| c.name.as_deref() == Some("lastModel"))
        && settings::flag(s, "_continue_training_existing_model")?.unwrap_or(false)
    {
        return Err(gap(
            "continuing the training of a saved model (Modeler's model state is not read)",
        ));
    }
    let inputs: Vec<String> = settings::children(s, "_inputs", "field")
        .into_iter()
        .map(|e| e.text().trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    Ok(BuildSettings {
        settings: common(node)?,
        target: text("_target")
            .filter(|n| !n.is_empty())
            .ok_or_else(|| OperatorError::Settings("SLRM needs a target field".into()))?,
        response: text("_target_response")
            .filter(|n| !n.is_empty())
            .ok_or_else(|| OperatorError::Settings("SLRM needs a response field".into()))?,
        inputs: (!inputs.is_empty()).then_some(inputs),
    })
}

struct Apply {
    member: Option<String>,
    max_predictions: usize,
    reliability: bool,
    descending: bool,
}

pub(super) fn apply(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let settings = common(node)?;
    let member = node
        .containers
        .iter()
        .find(|c| matches!(c.name.as_deref(), Some("model" | "PMML")))
        .map(|c| c.member.clone());
    Ok(Box::new(Apply {
        member,
        max_predictions: settings.max_predictions,
        reliability: settings.model_reliability,
        descending: settings.sort == SortOrder::Descending,
    }))
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

fn to_pmml(field: &Field, value: &Value) -> PValue {
    if field.is_missing(value) {
        return PValue::Missing;
    }
    match value {
        Value::Integer(n) => PValue::Number(*n as f64),
        Value::Real(x) => PValue::Number(*x),
        Value::String(text) => PValue::Text(text.to_string()),
        other => PValue::Text(other.to_string()),
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
            .ok_or_else(|| OperatorError::Inputs("an SLRM nugget needs one input".into()))?;
        let pmml = match model {
            Some(model) => model,
            None => {
                let member = self.member.as_deref().ok_or_else(|| {
                    OperatorError::Settings(
                        "the nugget saved no model and its build node did not run".into(),
                    )
                })?;
                let xml = super::apply::saved_model_xml(member, context)?;
                Arc::new(
                    Pmml::parse(&xml)
                        .map_err(|error| OperatorError::Data(format!("model: {error}")))?,
                )
            }
        };
        let Model::Slrm(slrm) = &pmml.model else {
            return Err(OperatorError::Data(
                "the nugget's model is not an SLRM model".into(),
            ));
        };
        let target = slrm.target.clone();
        let fields = input.model().fields();
        let positions: Vec<(String, usize)> = slrm
            .input_fields()
            .filter_map(|name| input.model().position(name).map(|p| (name.to_string(), p)))
            .collect();
        let ranks = self.max_predictions.min(slrm.products.len());
        let mut products: Vec<Vec<Value>> = vec![Vec::with_capacity(input.rows().len()); ranks];
        let mut scores: Vec<Vec<Value>> = vec![Vec::with_capacity(input.rows().len()); ranks];
        for row in input.rows() {
            let record: Vec<(String, PValue)> = positions
                .iter()
                .filter_map(|(name, p)| {
                    Some((name.clone(), to_pmml(fields.get(*p)?, row.get(*p)?)))
                })
                .collect();
            let mut scored = slrm.scores(
                |name| record.iter().find(|(n, _)| n == name).map(|(_, v)| v),
                self.reliability,
            );
            scored.sort_by(|a, b| {
                let order = a.score.total_cmp(&b.score);
                let order = if self.descending {
                    order.reverse()
                } else {
                    order
                };
                order.then(a.product.cmp(&b.product))
            });
            for k in 0..ranks {
                match scored.get(k).and_then(|s| {
                    slrm.products
                        .get(s.product)
                        .map(|p| (p.name.clone(), s.score))
                }) {
                    Some((name, score)) => {
                        if let Some(column) = products.get_mut(k) {
                            column.push(Value::string(name));
                        }
                        if let Some(column) = scores.get_mut(k) {
                            column.push(Value::real(score));
                        }
                    }
                    None => {
                        if let Some(column) = products.get_mut(k) {
                            column.push(Value::Null);
                        }
                        if let Some(column) = scores.get_mut(k) {
                            column.push(Value::Null);
                        }
                    }
                }
            }
        }
        let mut columns = Vec::new();
        for (k, (names, values)) in products.into_iter().zip(scores).enumerate() {
            let mut product_field = Field::new(format!("$S-{target}-{}", k + 1), Storage::String);
            product_field.measurement = Measurement::Categorical;
            columns.push(Column {
                field: product_field,
                values: names,
            });
            let mut score_field = Field::new(format!("$SC-{target}-{}", k + 1), Storage::Real);
            score_field.measurement = Measurement::Continuous;
            columns.push(Column {
                field: score_field,
                values,
            });
        }
        with_columns(input, columns)
    }
}
