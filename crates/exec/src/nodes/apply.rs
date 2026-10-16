//! Model nuggets (`ApplyCartRuleNode`, `ApplyChaidRuleNode`, `ApplyLogisticNode`,
//! `ApplyKMeansNode`, `ApplyTwoStepNode`): score every record with the model and append Modeler's score fields
//! (`docs/models.md`).
//!
//! The model is the one the linked build node trained in this evaluation, else the PMML the
//! nugget saved (`properties/model@pmml`, an attachment of the stream file). Score fields, from
//! the oracle exports of the team streams and `crates/pmml/tests/oracle.rs`:
//!
//! | nugget | fields |
//! |---|---|
//! | C&R Tree, CHAID | `$R-<target>` prediction; `$RC-<target>` confidence (`calculateConfidences`); `$RRP-<target>` raw propensity, the probability of the second (true) category (`calculateRawPropensities`) |
//! | Logistic | `$L-`, `$LP-` (confidence), `$LRP-` (raw propensity), same switches |
//! | K-Means | `$KM-<model name>`: `cluster-<cluster name>`; `$KMD-<model name>`: the Euclidean distance to that center, when the nugget's `buildSettings` set `generateDistanceField` |
//! | TwoStep | `$T-<model name>`: the cluster name (`cluster-<n>`), by the log-likelihood distance |
//!
//! When the input already has a field of that name (another model on the same target), the
//! fields get the first free index: `$R1-`, `$RC1-`, `$RRP1-` (`team/model_comparison`).
//! GAP: K-Means cluster distance (`showClusterProximity`) and TwoStep outlier handling
//! (`excludeOutliers`, a noise cluster) are rejected until an oracle covers them.

use super::field_util::{Column, with_columns};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::NodeOutput;
use crate::settings;
use dms_pmml::{DataType, Pmml, Prediction, Usage, Value as PmmlValue};
use dms_stream::StreamNode;
use dms_types::{Measurement, Storage, Table, Value};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Decision trees and rule sets: `$R-`, `$RC-`, `$RRP-`.
    Rule,
    /// Logistic regression: `$L-`, `$LP-`, `$LRP-`.
    Logistic,
    /// K-Means: `$KM-<model name>`.
    KMeans,
    /// TwoStep: `$T-<model name>`.
    TwoStep,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let kind = match node.type_identifier().as_str() {
        "ApplyCartRuleNode" | "ApplyChaidRuleNode" | "ApplyQuestRuleNode" | "ApplyC5RuleNode" => {
            Kind::Rule
        }
        "ApplyLogisticNode" => Kind::Logistic,
        "ApplyKMeansNode" => Kind::KMeans,
        "ApplyTwoStepNode" => Kind::TwoStep,
        other => {
            return Err(OperatorError::Settings(format!(
                "model nugget {other} is not supported yet"
            )));
        }
    };
    if kind == Kind::KMeans && settings::flag(s, "showClusterProximity")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "K-Means cluster distance (showClusterProximity) is not supported yet".into(),
        ));
    }
    if kind == Kind::TwoStep && build_setting(s, "excludeOutliers").as_deref() == Some("true") {
        return Err(OperatorError::Settings(
            "TwoStep outlier handling (excludeOutliers) is not supported yet".into(),
        ));
    }
    let model_name = settings::text(s, "modelName")
        .filter(|name| !name.trim().is_empty())
        .or_else(|| node.name.clone())
        .unwrap_or_else(|| match kind {
            Kind::TwoStep => "TwoStep".to_string(),
            _ => "K-Means".to_string(),
        });
    Ok(Box::new(Apply {
        kind,
        pmml_member: settings::element(s, "model")
            .and_then(|model| model.attribute("pmml"))
            .map(str::to_string),
        confidences: settings::flag(s, "calculateConfidences")?.unwrap_or(true),
        raw_propensities: settings::flag(s, "calculateRawPropensities")?.unwrap_or(false),
        distance: kind == Kind::KMeans
            && build_setting(s, "generateDistanceField").as_deref() == Some("true"),
        model_name,
    }))
}

struct Apply {
    kind: Kind,
    /// The attachment holding the saved PMML.
    pmml_member: Option<String>,
    confidences: bool,
    raw_propensities: bool,
    /// K-Means: also the distance to the cluster center (`$KMD-`).
    distance: bool,
    model_name: String,
}

/// A value of the nugget's `buildSettings/propertySetting` list (the build node's settings).
fn build_setting(s: Option<&dms_strfmt::fastinfoset::Element>, name: &str) -> Option<String> {
    settings::children(s, "buildSettings", "propertySetting")
        .into_iter()
        .find(|setting| settings::text(Some(setting), "name").as_deref() == Some(name))
        .and_then(|setting| settings::element(Some(setting), "value"))
        .map(|value| value.text())
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
    fn saved_model(&self, context: &ExecContext) -> Result<Pmml, OperatorError> {
        let member = self.pmml_member.as_deref().ok_or_else(|| {
            OperatorError::Settings(
                "the nugget saved no PMML model and its build node did not run".into(),
            )
        })?;
        let bytes = context.attachments.get(member).ok_or_else(|| {
            OperatorError::Io(format!("the stream file has no attachment {member}"))
        })?;
        let text = std::str::from_utf8(bytes).map_err(|_| {
            OperatorError::Data(format!(
                "the model in {member} is not PMML (a format we cannot read)"
            ))
        })?;
        Pmml::parse(text).map_err(|error| OperatorError::Data(format!("{member}: {error}")))
    }

    fn score(
        &self,
        inputs: &[Arc<Table>],
        model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("a model nugget needs one input".into()))?;
        let pmml = match model {
            Some(model) => model,
            None => Arc::new(self.saved_model(context)?),
        };
        let names: Vec<&str> = input
            .model()
            .fields()
            .iter()
            .map(|field| field.name.as_str())
            .collect();
        let scorer = pmml
            .bind(&names)
            .map_err(|error| OperatorError::Data(format!("model: {error}")))?;
        let predictions = input
            .rows()
            .iter()
            .map(|row| {
                let record: Vec<PmmlValue> = row.iter().map(to_pmml).collect();
                scorer
                    .score(&record)
                    .map_err(|error| OperatorError::Data(format!("model: {error}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let columns = self.columns(input, &pmml, &predictions)?;
        with_columns(input, columns)
    }

    fn columns(
        &self,
        input: &Table,
        pmml: &Pmml,
        predictions: &[Prediction],
    ) -> Result<Vec<Column>, OperatorError> {
        let taken = |name: &str| input.model().position(name).is_some();
        if self.kind == Kind::KMeans {
            let name = format!("$KM-{}", self.model_name);
            let values = predictions
                .iter()
                .map(|p| match &p.value {
                    PmmlValue::Missing => Value::Null,
                    value => Value::string(format!("cluster-{}", text(value))),
                })
                .collect();
            let mut columns = vec![column(&name, Storage::String, values)];
            if self.distance {
                columns.push(column(
                    &format!("$KMD-{}", self.model_name),
                    Storage::Real,
                    predictions.iter().map(|p| real(p.affinity)).collect(),
                ));
            }
            return Ok(columns);
        }
        if self.kind == Kind::TwoStep {
            let values = predictions
                .iter()
                .map(|p| match &p.value {
                    PmmlValue::Missing => Value::Null,
                    value => Value::string(text(value)),
                })
                .collect();
            return Ok(vec![column(
                &format!("$T-{}", self.model_name),
                Storage::String,
                values,
            )]);
        }
        let target = target_name(pmml)?;
        let (value_prefix, confidence_prefix, propensity_prefix) = match self.kind {
            Kind::Logistic => ("$L", "$LP", "$LRP"),
            _ => ("$R", "$RC", "$RRP"),
        };
        // The first index whose prediction field is free.
        let index = (0..)
            .map(|n: usize| if n == 0 { String::new() } else { n.to_string() })
            .find(|n| !taken(&format!("{value_prefix}{n}-{target}")))
            .unwrap_or_default();
        let storage = target_storage(pmml, &target);
        let mut columns = vec![column(
            &format!("{value_prefix}{index}-{target}"),
            storage,
            predictions
                .iter()
                .map(|p| typed(&p.value, storage))
                .collect(),
        )];
        if self.confidences {
            columns.push(column(
                &format!("{confidence_prefix}{index}-{target}"),
                Storage::Real,
                predictions.iter().map(|p| real(p.confidence)).collect(),
            ));
        }
        if self.raw_propensities {
            columns.push(column(
                &format!("{propensity_prefix}{index}-{target}"),
                Storage::Real,
                predictions
                    .iter()
                    .map(|p| real(p.probabilities.get(1).map(|(_, probability)| *probability)))
                    .collect(),
            ));
        }
        Ok(columns)
    }
}

fn column(name: &str, storage: Storage, values: Vec<Value>) -> Column {
    let mut field = dms_types::Field::new(name, storage);
    field.measurement = match storage {
        Storage::Real | Storage::Integer => Measurement::Continuous,
        _ => Measurement::Categorical,
    };
    Column { field, values }
}

fn target_name(pmml: &Pmml) -> Result<String, OperatorError> {
    let header = match &pmml.model {
        dms_pmml::Model::Tree(model) => &model.header,
        dms_pmml::Model::GeneralRegression(model) => &model.header,
        dms_pmml::Model::Regression(model) => &model.header,
        dms_pmml::Model::Clustering(model) => &model.header,
        dms_pmml::Model::Unsupported(element) => {
            return Err(OperatorError::Data(format!(
                "model {element} cannot be scored yet"
            )));
        }
    };
    header
        .mining_schema
        .iter()
        .find(|field| field.usage == Usage::Target)
        .map(|field| field.name.clone())
        .ok_or_else(|| OperatorError::Data("the model names no target field".into()))
}

fn target_storage(pmml: &Pmml, target: &str) -> Storage {
    match pmml
        .data_dictionary
        .iter()
        .find(|field| field.name == target)
        .map(|field| &field.data_type)
    {
        Some(DataType::Integer) => Storage::Integer,
        Some(DataType::Double | DataType::Float) => Storage::Real,
        _ => Storage::String,
    }
}

fn to_pmml(value: &Value) -> PmmlValue {
    match value {
        Value::Null => PmmlValue::Missing,
        Value::Integer(i) => PmmlValue::Number(*i as f64),
        Value::Real(r) => PmmlValue::Number(*r),
        Value::String(s) => PmmlValue::Text(s.to_string()),
        other => PmmlValue::Text(other.to_string()),
    }
}

fn text(value: &PmmlValue) -> String {
    match value {
        PmmlValue::Missing => String::new(),
        PmmlValue::Number(n) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{n:.0}"),
        PmmlValue::Number(n) => n.to_string(),
        PmmlValue::Text(s) => s.clone(),
    }
}

/// A predicted value in the target's storage.
fn typed(value: &PmmlValue, storage: Storage) -> Value {
    match (value, storage) {
        (PmmlValue::Missing, _) => Value::Null,
        (PmmlValue::Number(n), Storage::Real) => Value::Real(*n),
        (PmmlValue::Text(s), Storage::Real) => s.trim().parse().map_or(Value::Null, Value::Real),
        (PmmlValue::Number(n), Storage::Integer) if n.fract() == 0.0 => Value::Integer(*n as i64),
        (PmmlValue::Text(s), Storage::Integer) => {
            s.trim().parse().map_or(Value::Null, Value::Integer)
        }
        (value, _) => Value::string(text(value)),
    }
}

fn real(value: Option<f64>) -> Value {
    value.map_or(Value::Null, Value::real)
}
