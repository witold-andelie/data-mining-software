//! Auto Cluster nugget (`ApplyAutoClusterNode`, `oracle/models_next2/acluster_default`): the
//! saved component models are `resultItem` children of the node (`useModel="true"` for the
//! ones in use), each a `modelOutput` holding an ordinary clustering nugget and its PMML
//! attachment.
//!
//! Rules (`acluster_default`: K-Means in use, TwoStep and Kohonen not; the model's
//! `useMultipleModels` false):
//! - With one component in use, the nugget writes that component's cluster as the field its
//!   saved outputs name (`$XC-autocluster`): `cluster-1`, ... from the K-Means nugget.
//!
//! GAP: several components in use (`useMultipleModels` true) have no oracle and are rejected;
//! component types other than K-Means, TwoStep and Kohonen are rejected. Training
//! (`BuildAutoClusteringModelNode`) is not implemented.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::NodeOutput;
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Measurement, Table, Value};
use std::sync::Arc;

use super::field_util::{Column, with_columns};

pub fn apply(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    if settings::element(s, "model")
        .and_then(|model| model.attribute("useMultipleModels"))
        .is_some_and(|value| value == "true")
    {
        return Err(OperatorError::Settings(
            "Auto Cluster with several models in use is not supported yet (GAP)".into(),
        ));
    }
    let in_use: Vec<_> = node
        .result_items
        .iter()
        .filter(|item| item.attribute("useModel") == Some("true"))
        .collect();
    let [item] = in_use.as_slice() else {
        return Err(OperatorError::Settings(format!(
            "Auto Cluster with {} models in use is not supported yet (GAP)",
            in_use.len()
        )));
    };
    let component = super::auto_models::component(node, item, 0)?;
    if !matches!(
        component.type_identifier().as_str(),
        "ApplyKMeansNode" | "ApplyTwoStepNode" | "ApplyKohonenNode"
    ) {
        return Err(OperatorError::Settings(
            "GAP: Auto Cluster component is not a supported clustering nugget".into(),
        ));
    }
    // The nugget's own output field (`model/outputs/output/name`, prefix `$XC`).
    let field = settings::children(s, "model/outputs", "output")
        .into_iter()
        .filter_map(|output| settings::text(Some(output), "name"))
        .map(|name| name.trim().to_string())
        .find(|name| name.starts_with("$XC-"))
        .ok_or_else(|| {
            OperatorError::Settings("the Auto Cluster nugget names no $XC- output".into())
        })?;
    Ok(Box::new(AutoCluster {
        component: super::apply::build(&component)?,
        field,
    }))
}

struct AutoCluster {
    component: Box<dyn Operator>,
    field: String,
}

impl Operator for AutoCluster {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("a model nugget needs one input".into()))?;
        let width = input.model().fields().len();
        let scored = match self.component.produce(inputs, None, context)? {
            NodeOutput::Data(table) => table,
            _ => {
                return Err(OperatorError::Data(
                    "the Auto Cluster component produced no records".into(),
                ));
            }
        };
        let source = scored.model().fields().get(width).cloned().ok_or_else(|| {
            OperatorError::Data("the Auto Cluster component wrote no cluster".into())
        })?;
        let mut field = dms_types::Field::new(self.field.clone(), source.storage);
        field.measurement = Measurement::Nominal;
        let values = scored
            .rows()
            .iter()
            .map(|row| row.get(width).cloned().unwrap_or(Value::Null))
            .collect();
        with_columns(input, vec![Column { field, values }])
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        _model: Option<Arc<dms_pmml::Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        self.run(inputs, context)
            .map(|table| NodeOutput::Data(Arc::new(table)))
    }
}
