//! KNN (`spss.KNN_id!KNNbuilder` / `KNNapplier`): the build node's settings and the nugget.
//!
//! The demo streams save only a few settings (`_use_focalcase`, `_neighbour_details`,
//! `_use_caselabels`, `_choose_vfold`, ...): the rest are the node's defaults, which the trainer's
//! [`dms_algo::knn::Settings`] reproduces (k = 3, Euclidean distance, rescaled continuous fields).
//! The names the node would give to the number of neighbours or to feature selection are not
//! known, so they stay at their defaults. A setting that would add fields or choose the neighbours
//! differently is a `GAP:`: neighbour details, case labels and automatic k.
//!
//! The nugget of a model without a target identifies neighbours and, without saved details, adds
//! no fields (`ibm-demos/car_sales_knn`). Scoring a target is a `GAP:`.

use super::field_util::Column;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use crate::{ExecContext, NodeOutput};
use dms_algo::knn::Settings;
use dms_pmml::{Model, Pmml};
use dms_stream::StreamNode;
use dms_types::Table;
use std::sync::Arc;

fn gap(what: impl std::fmt::Display) -> OperatorError {
    OperatorError::Settings(format!("GAP: KNN {what}"))
}

pub(super) fn build_settings(node: &StreamNode) -> Result<Settings, OperatorError> {
    let s = node.settings.as_ref();
    for (key, what) in [
        ("_neighbour_details", "with neighbour details"),
        ("_use_caselabels", "with case labels"),
        ("_choose_vfold", "with automatic k selection"),
    ] {
        if settings::flag(s, key)?.unwrap_or(false) {
            return Err(gap(what));
        }
    }
    Ok(Settings::default())
}

struct Apply {
    member: Option<String>,
    details: bool,
}

pub(super) fn apply(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let member = settings::element(s, "model")
        .and_then(|model| model.attribute("pmml"))
        .map(str::to_string)
        .or_else(|| {
            node.containers
                .iter()
                .find(|c| matches!(c.name.as_deref(), Some("PMML" | "model")))
                .map(|c| c.member.clone())
        });
    Ok(Box::new(Apply {
        member,
        details: settings::flag(s, "_neighbour_details")?.unwrap_or(false),
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

impl Apply {
    fn score(
        &self,
        inputs: &[Arc<Table>],
        model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("a KNN nugget needs one input".into()))?;
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
        let Model::Knn(knn) = &pmml.model else {
            return Err(OperatorError::Data(
                "the nugget's model is not a KNN model".into(),
            ));
        };
        if knn.target.is_some() {
            return Err(gap("scoring a target (the stored cases are not kept)"));
        }
        if self.details {
            return Err(gap("neighbour details"));
        }
        // identify neighbours without details: the nugget adds no fields
        let columns: Vec<Column> = Vec::new();
        super::field_util::with_columns(input, columns)
    }
}
