//! Temporal Causal Model (`tcm!pasw.timeseries-extension` / `tcmmodel!pasw.timeseries-extension`):
//! the nugget.
//!
//! The nugget keeps the models as an `SCF` container of `TCMXML` documents (`InputTCMXML`), the
//! data-preparation settings (`InputTSDPSetting`, JSON: the list of series in field order) and the
//! output tables. The build node's settings and training are in `build.rs` and
//! [`dms_algo::tcm_model`].
//!
//! The nugget adds one field per target series, `$TCM-<series>`: the one-step-ahead fitted value
//! of the model at each record (the records are the time points, in table order). The first `L`
//! records, and any record whose lags are missing, get `$null$`. The nugget of the demo extends
//! no forecasts (`extendsToFuture` false); extending the series into the future and the
//! confidence limits are `GAP:`s.

use super::field_util::{Column, with_columns};
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::{ExecContext, NodeOutput};
use dms_pmml::{Model, Pmml, TcmModel};
use dms_stream::StreamNode;
use dms_types::{Field, Measurement, Storage, Table, Value};
use std::sync::Arc;

struct Apply {
    /// The attachment with the `TCMXML` documents (`InputTCMXML`).
    models: Option<String>,
    /// The attachment with the series list (`InputTSDPSetting`).
    setup: Option<String>,
}

pub(super) fn apply(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let container = |name: &str| {
        node.containers
            .iter()
            .find(|c| c.name.as_deref() == Some(name))
            .map(|c| c.member.clone())
    };
    Ok(Box::new(Apply {
        models: container("InputTCMXML"),
        setup: container("InputTSDPSetting"),
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

/// The model of a nugget saved in a stream.
fn saved_model(
    models: &str,
    setup: &str,
    context: &ExecContext,
) -> Result<TcmModel, OperatorError> {
    let attachment = |member: &str| {
        context
            .attachments
            .get(member)
            .ok_or_else(|| OperatorError::Io(format!("the stream file has no attachment {member}")))
    };
    let setup_members = dms_strfmt::read_members(attachment(setup)?)
        .map_err(|error| OperatorError::Data(format!("{setup}: {error}")))?;
    let json = setup_members
        .iter()
        .find(|m| m.name == "TSDPOutput.json")
        .ok_or_else(|| OperatorError::Data(format!("{setup} has no TSDPOutput.json")))?;
    let json: serde_json::Value = serde_json::from_slice(&json.body)
        .map_err(|error| OperatorError::Data(format!("TSDPOutput.json: {error}")))?;
    let series: Vec<String> = json
        .get("settings")
        .and_then(|s| s.get("metricFieldList"))
        .and_then(serde_json::Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if series.is_empty() {
        return Err(OperatorError::Data(
            "TSDPOutput.json lists no metric fields".into(),
        ));
    }
    let members = dms_strfmt::read_members(attachment(models)?)
        .map_err(|error| OperatorError::Data(format!("{models}: {error}")))?;
    let documents: Vec<String> = members
        .iter()
        .filter(|m| m.body.starts_with(b"<?xml") && m.body.windows(6).any(|w| w == b"TCMXML"))
        .filter_map(|m| String::from_utf8(m.body.clone()).ok())
        .collect();
    let documents: Vec<&str> = documents.iter().map(String::as_str).collect();
    TcmModel::from_documents(series, &documents)
        .map_err(|error| OperatorError::Data(format!("model: {error}")))
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
            .ok_or_else(|| OperatorError::Inputs("a TCM nugget needs one input".into()))?;
        let tcm = match model.as_deref() {
            Some(Pmml {
                model: Model::Tcm(model),
                ..
            }) => model.clone(),
            Some(_) => {
                return Err(OperatorError::Data(
                    "the nugget's model is not a TCM model".into(),
                ));
            }
            None => {
                let (Some(models), Some(setup)) = (&self.models, &self.setup) else {
                    return Err(OperatorError::Settings(
                        "the nugget saved no model and its build node did not run".into(),
                    ));
                };
                saved_model(models, setup, context)?
            }
        };
        let fields = input.model().fields();
        let columns: Vec<Vec<Option<f64>>> = tcm
            .series
            .iter()
            .map(|name| {
                let position = input.model().position(name).ok_or_else(|| {
                    OperatorError::Data(format!("model: the input has no series {name:?}"))
                })?;
                let field = fields.get(position);
                Ok(input
                    .rows()
                    .iter()
                    .map(|row| {
                        row.get(position)
                            .filter(|v| field.is_none_or(|f| !f.is_missing(v)))
                            .and_then(Value::as_f64)
                    })
                    .collect())
            })
            .collect::<Result<_, OperatorError>>()?;
        let mut out = Vec::new();
        for target in tcm.targets.iter().filter(|t| !t.ar_only) {
            let name = tcm
                .series
                .get(target.series)
                .ok_or_else(|| OperatorError::Data("a model names an unknown series".into()))?;
            let mut field = Field::new(format!("$TCM-{name}"), Storage::Real);
            field.measurement = Measurement::Continuous;
            out.push(Column {
                field,
                values: tcm
                    .fitted(target, &columns)
                    .into_iter()
                    .map(|v| v.map_or(Value::Null, Value::real))
                    .collect(),
            });
        }
        with_columns(input, out)
    }
}
