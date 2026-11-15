//! Python model nodes (Modeling palette): Random Forest, XGBoost Tree, XGBoost Linear, Gaussian
//! Mixture and One-Class SVM, trained with scikit-learn and xgboost in the user's own Python (`dms_python::models`,
//! docs/python-bridge.md).
//!
//! The build node trains and outputs [`NodeOutput::PythonModel`]; the engine then saves the
//! pickled model as a stream attachment and adds or updates the model nugget (the same node type
//! with `model_builder` set and `pythonModel` naming the attachment). The nugget scores every
//! record with the saved model.
//!
//! Settings are this application's own (the scripting property names where the Python Scripting
//! and Automation Guide gives them); Modeler's `.str` storage of these nodes has not been observed.
//! Fields: `target` and `inputs`, else the Type roles (Target; Input and Both). With
//! `use_partitioned_data` (default true) only the first partition (training) trains.
//!
//! Score fields (inferred, no oracle): Random Forest `$R-<target>` (prediction) and, for a
//! categorical target, `$RP-<target>` (its probability); XGBoost Tree `$XT-`/`$XTP-` and XGBoost
//! Linear `$XL-`/`$XLP-` likewise; Gaussian Mixture `$GMM-<model>` (cluster)
//! and `$GMMP-<model>` (its probability); One-Class SVM `$O-<model>` (`T` for an outlier) and
//! `$OP-<model>` (the decision score, negative for outliers). An existing field takes the next
//! free index (`$R1-`), as other nuggets do.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput, PythonModel};
use crate::schema::{self, ParamValue};
use crate::settings;
use dms_python::BridgeError;
use dms_stream::StreamNode;
use dms_types::{Measurement, Role, Storage, Table, Value, ValueSpec};
use std::sync::Arc;

/// The settings that hold a nugget's saved model attachment.
pub const MODEL_SETTING: &str = "pythonModel";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    RandomForest,
    XgboostTree,
    XgboostLinear,
    GaussianMixture,
    OneClassSvm,
}

impl Kind {
    fn of(catalog_id: &str) -> Option<Kind> {
        match catalog_id {
            "random_forest" => Some(Kind::RandomForest),
            "xgboost_tree" => Some(Kind::XgboostTree),
            "xgboost_linear" => Some(Kind::XgboostLinear),
            "gmm" => Some(Kind::GaussianMixture),
            "one_class_svm" => Some(Kind::OneClassSvm),
            _ => None,
        }
    }

    /// The name `models.py` uses.
    fn script_name(self) -> &'static str {
        match self {
            Kind::RandomForest => "random_forest",
            Kind::XgboostTree => "xgboost_tree",
            Kind::XgboostLinear => "xgboost_linear",
            Kind::GaussianMixture => "gaussian_mixture",
            Kind::OneClassSvm => "one_class_svm",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::RandomForest => "Random Forest",
            Kind::XgboostTree => "XGBoost Tree",
            Kind::XgboostLinear => "XGBoost Linear",
            Kind::GaussianMixture => "Gaussian Mixture",
            Kind::OneClassSvm => "One-Class SVM",
        }
    }

    fn catalog_id(self) -> &'static str {
        match self {
            Kind::RandomForest => "random_forest",
            Kind::XgboostTree => "xgboost_tree",
            Kind::XgboostLinear => "xgboost_linear",
            Kind::GaussianMixture => "gmm",
            Kind::OneClassSvm => "one_class_svm",
        }
    }

    fn supervised(self) -> bool {
        matches!(
            self,
            Kind::RandomForest | Kind::XgboostTree | Kind::XgboostLinear
        )
    }
}

fn catalog_id(node: &StreamNode) -> Option<&'static str> {
    match dms_catalog::resolve(node) {
        dms_catalog::Resolution::Node(spec) => Some(spec.id),
        _ => None,
    }
}

/// Builds the build node or, with `model_builder` set, its nugget.
pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let kind = catalog_id(node)
        .and_then(Kind::of)
        .ok_or_else(|| OperatorError::Settings("not a Python model node".into()))?;
    if node.model_builder.is_some() {
        let attachment = settings::text(node.settings.as_ref(), MODEL_SETTING)
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| {
                OperatorError::Settings(format!(
                    "{} nugget: no saved model; run its build node",
                    kind.label()
                ))
            })?;
        return Ok(Box::new(Nugget { kind, attachment }));
    }
    let specs = schema::schema(kind.catalog_id()).unwrap_or_default();
    let mut options = serde_json::Map::new();
    let mut target = None;
    let mut inputs = Vec::new();
    let mut partitioned = true;
    let mut custom = false;
    for spec in specs {
        let value = schema::read(node.settings.as_ref(), spec)
            .map_err(|e| OperatorError::Settings(format!("{}: {e}", spec.key)))?;
        match (spec.key, value) {
            ("target", ParamValue::Text(text)) if !text.trim().is_empty() => {
                target = Some(text.trim().to_string());
            }
            ("inputs", ParamValue::Fields(fields)) => inputs = fields,
            ("use_partitioned_data", ParamValue::Bool(on)) => partitioned = on,
            ("custom_fields", ParamValue::Bool(on)) => custom = on,
            (key, ParamValue::Bool(b)) => {
                options.insert(key.into(), b.into());
            }
            (key, ParamValue::Int(n)) => {
                options.insert(key.into(), n.into());
            }
            (key, ParamValue::Real(x)) => {
                options.insert(key.into(), x.into());
            }
            (key, ParamValue::Text(text)) if key != "target" => {
                options.insert(key.into(), text.into());
            }
            _ => {}
        }
    }
    if !custom {
        // Roles from the incoming Type settings decide.
        target = None;
        inputs.clear();
    }
    let name = node
        .label
        .clone()
        .or_else(|| node.name.clone())
        .unwrap_or_else(|| kind.label().to_string());
    Ok(Box::new(Builder {
        kind,
        target,
        inputs,
        partitioned,
        options,
        name,
    }))
}

fn bridge(kind: Kind, error: BridgeError) -> OperatorError {
    match error {
        BridgeError::Cancelled => OperatorError::Cancelled,
        BridgeError::NotFound { .. }
        | BridgeError::TooOld { .. }
        | BridgeError::MissingPackages { .. } => OperatorError::Settings(error.to_string()),
        BridgeError::Process(_) | BridgeError::Io(_) | BridgeError::Timeout(_) => {
            OperatorError::Io(format!("{}: {error}", kind.label()))
        }
        _ => OperatorError::Data(format!("{}: {error}", kind.label())),
    }
}

struct Builder {
    kind: Kind,
    target: Option<String>,
    inputs: Vec<String>,
    partitioned: bool,
    options: serde_json::Map<String, serde_json::Value>,
    name: String,
}

/// Records of the training partition (the partition field's first value; `1_Training` when its
/// values are not instantiated).
fn training(input: &Table) -> Result<Table, OperatorError> {
    let Some(position) = input
        .model()
        .fields()
        .iter()
        .position(|f| f.role == Role::Partition)
    else {
        return Ok(input.clone());
    };
    let declared = input
        .model()
        .fields()
        .get(position)
        .and_then(|field| match &field.values {
            ValueSpec::Categories(values) => values.first().cloned(),
            _ => None,
        });
    let keep = |value: &Value| match (&declared, value) {
        (Some(first), value) => value == first,
        (None, Value::String(text)) => text.starts_with("1_"),
        (None, Value::Integer(n)) => *n == 1,
        _ => false,
    };
    let rows = input
        .rows()
        .iter()
        .filter(|row| row.get(position).is_some_and(keep))
        .cloned()
        .collect();
    Table::new(input.model().clone(), rows).map_err(|e| OperatorError::Data(e.to_string()))
}

impl Builder {
    fn fields(
        &self,
        input: &Table,
    ) -> Result<(Option<String>, &'static str, Vec<String>), OperatorError> {
        let model = input.model();
        let missing = |name: &str| {
            OperatorError::Data(format!(
                "{}: field {name:?} is not in the data",
                self.kind.label()
            ))
        };
        let target = if self.kind.supervised() {
            let name = match &self.target {
                Some(name) => name.clone(),
                None => model
                    .fields()
                    .iter()
                    .find(|f| f.role == Role::Target)
                    .map(|f| f.name.clone())
                    .ok_or_else(|| {
                        OperatorError::Settings(format!(
                            "{}: no target field; choose one or set a Target role with a Type node",
                            self.kind.label()
                        ))
                    })?,
            };
            model.field(&name).ok_or_else(|| missing(&name))?;
            Some(name)
        } else {
            None
        };
        let target_kind = match target.as_deref().and_then(|name| model.field(name)) {
            Some(field) => match field.measurement {
                Measurement::Continuous => "continuous",
                Measurement::Default | Measurement::Typeless => match field.storage {
                    Storage::Integer | Storage::Real => "continuous",
                    _ => "categorical",
                },
                _ => "categorical",
            },
            None => "categorical",
        };
        let inputs: Vec<String> = if self.inputs.is_empty() {
            model
                .fields()
                .iter()
                .filter(|f| matches!(f.role, Role::Input | Role::Both))
                .filter(|f| Some(&f.name) != target.as_ref())
                .map(|f| f.name.clone())
                .collect()
        } else {
            for name in &self.inputs {
                model.field(name).ok_or_else(|| missing(name))?;
            }
            self.inputs.clone()
        };
        if inputs.is_empty() {
            return Err(OperatorError::Settings(format!(
                "{}: no input fields; choose them or set Input roles with a Type node",
                self.kind.label()
            )));
        }
        if let Some(field) = inputs
            .iter()
            .filter_map(|name| model.field(name))
            .find(|f| f.storage == Storage::List)
        {
            return Err(OperatorError::Data(format!(
                "{}: list field {:?} cannot be a model input",
                self.kind.label(),
                field.name
            )));
        }
        Ok((target, target_kind, inputs))
    }
}

fn summary_document(kind: Kind, summary: &serde_json::Value) -> Document {
    let number = |key: &str| summary.get(key).and_then(serde_json::Value::as_f64);
    let mut facts = vec![vec![
        "Records".to_string(),
        summary
            .get("records")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            .to_string(),
    ]];
    for (key, label) in [
        ("training_accuracy", "Training accuracy"),
        ("training_r2", "Training R²"),
        ("bic", "BIC"),
    ] {
        if let Some(value) = number(key) {
            facts.push(vec![label.to_string(), format!("{value:.4}")]);
        }
    }
    if let Some(n) = summary
        .get("training_outliers")
        .and_then(serde_json::Value::as_u64)
    {
        facts.push(vec!["Training outliers".into(), n.to_string()]);
    }
    if let Some(converged) = summary
        .get("converged")
        .and_then(serde_json::Value::as_bool)
    {
        facts.push(vec!["Converged".into(), converged.to_string()]);
    }
    let mut tables = vec![DocumentTable {
        title: None,
        header: Vec::new(),
        rows: facts,
    }];
    let pairs = |key: &str| -> Vec<Vec<String>> {
        summary
            .get(key)
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let pair = item.as_array()?;
                        let name = pair.first()?.as_str()?.to_string();
                        let value = pair.get(1)?;
                        let value = value
                            .as_u64()
                            .map(|n| n.to_string())
                            .or_else(|| value.as_f64().map(|x| format!("{x:.4}")))?;
                        Some(vec![name, value])
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let importance = pairs("importance");
    if !importance.is_empty() {
        tables.push(DocumentTable {
            title: Some("Predictor importance".into()),
            header: vec!["Predictor".into(), "Importance".into()],
            rows: importance,
        });
    }
    let clusters = pairs("clusters");
    if !clusters.is_empty() {
        tables.push(DocumentTable {
            title: Some("Clusters".into()),
            header: vec!["Cluster".into(), "Records".into()],
            rows: clusters,
        });
    }
    Document {
        title: kind.label().to_string(),
        tables,
    }
}

impl Operator for Builder {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(format!(
            "{} is a build node and produces a model, not data",
            self.kind.label()
        )))
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        _model: Option<Arc<dms_pmml::Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        let input = inputs.first().ok_or_else(|| {
            OperatorError::Inputs(format!("{} needs one input", self.kind.label()))
        })?;
        let (target, target_kind, fields) = self.fields(input)?;
        let data = if self.partitioned {
            training(input)?
        } else {
            input.as_ref().clone()
        };
        // Only the fields the model uses travel to Python.
        let mut keep: Vec<&str> = fields.iter().map(String::as_str).collect();
        if let Some(target) = &target {
            keep.push(target);
        }
        let data = project(&data, &keep)?;
        let python = dms_python::resolve(context.python.as_deref(), &context.cancel)
            .map_err(|e| bridge(self.kind, e))?;
        let parameters = serde_json::json!({
            "kind": self.kind.script_name(),
            "target": target,
            "target_kind": target_kind,
            "inputs": fields,
            "name": self.name,
            "options": self.options,
        });
        let trained = dms_python::models::train(&python, &data, &parameters, &context.cancel)
            .map_err(|e| bridge(self.kind, e))?;
        Ok(NodeOutput::PythonModel(Arc::new(PythonModel {
            kind: self.kind.catalog_id().to_string(),
            bytes: trained.model.into(),
            summary: summary_document(self.kind, &trained.summary),
        })))
    }
}

/// The columns `names`, in that order.
fn project(table: &Table, names: &[&str]) -> Result<Table, OperatorError> {
    let model = table.model();
    let positions: Vec<usize> = names
        .iter()
        .filter_map(|name| model.position(name))
        .collect();
    let fields = positions
        .iter()
        .filter_map(|&i| model.fields().get(i).cloned())
        .collect();
    let model =
        dms_types::DataModel::new(fields).map_err(|e| OperatorError::Data(e.to_string()))?;
    let rows = table
        .rows()
        .iter()
        .map(|row| {
            positions
                .iter()
                .map(|&i| row.get(i).cloned().unwrap_or(Value::Null))
                .collect()
        })
        .collect();
    Table::new(model, rows).map_err(|e| OperatorError::Data(e.to_string()))
}

struct Nugget {
    kind: Kind,
    attachment: String,
}

impl Operator for Nugget {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs.first().ok_or_else(|| {
            OperatorError::Inputs(format!("{} nugget needs one input", self.kind.label()))
        })?;
        let model = context.attachments.get(&self.attachment).ok_or_else(|| {
            OperatorError::Settings(format!(
                "{} nugget: the saved model {} is missing; run its build node",
                self.kind.label(),
                self.attachment
            ))
        })?;
        let python = dms_python::resolve(context.python.as_deref(), &context.cancel)
            .map_err(|e| bridge(self.kind, e))?;
        // List fields cannot travel to Python and are never model inputs.
        let sendable: Vec<&str> = input
            .model()
            .fields()
            .iter()
            .filter(|f| f.storage != Storage::List)
            .map(|f| f.name.as_str())
            .collect();
        let sent = project(input, &sendable)?;
        let scored = dms_python::models::score(&python, &sent, model, &context.cancel)
            .map_err(|e| bridge(self.kind, e))?;
        // Python returns the score fields only; append them to the input records.
        let mut fields = input.model().fields().to_vec();
        fields.extend(scored.model().fields().iter().cloned().map(|mut f| {
            f.measurement = match f.storage {
                Storage::String => Measurement::Nominal,
                _ => Measurement::Continuous,
            };
            if f.name.starts_with("$O") && f.storage == Storage::String {
                f.measurement = Measurement::Flag;
            }
            f.role = Role::None;
            f
        }));
        let model =
            dms_types::DataModel::new(fields).map_err(|e| OperatorError::Data(e.to_string()))?;
        if scored.row_count() != input.row_count() {
            return Err(OperatorError::Data(format!(
                "{} nugget: scoring returned {} records for {}",
                self.kind.label(),
                scored.row_count(),
                input.row_count()
            )));
        }
        let rows = input
            .rows()
            .iter()
            .zip(scored.rows())
            .map(|(row, scores)| row.iter().chain(scores).cloned().collect())
            .collect();
        Table::new(model, rows).map_err(|e| OperatorError::Data(e.to_string()))
    }
}
