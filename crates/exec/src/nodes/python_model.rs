//! Python nodes: the model nodes Random Forest, XGBoost Tree, XGBoost Linear, Gaussian Mixture,
//! One-Class SVM, HDBSCAN and KDE (Modeling), trained with scikit-learn, xgboost and hdbscan, plus
//! SMOTE (Record Ops: appends synthetic records of the smaller target classes, imbalanced-learn)
//! t-SNE (Graphs: a 2-D embedding drawn as a scatter), KDE Simulation (Output: records drawn from a
//! kernel density of the inputs) and Extension Model (the user's own build and score scripts:
//! the build script leaves `modelerModel`, which is pickled with the score script; the score
//! script adds fields to `modelerData`, and the nugget appends the added ones), all in the user's
//! own Python (`dms_python::models`,
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
//! `$OP-<model>` (the decision score, negative for outliers); HDBSCAN `$HB-<model>` (cluster, or
//! `noise`) and `$HBP-<model>` (membership strength); KDE `$KDE-<model>` (log density). An existing field takes the next
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
    Hdbscan,
    Kde,
    /// Record Ops: oversamples the smaller target classes (not a model).
    Smote,
    /// Graphs: a 2-D embedding drawn as a scatter (not a model).
    Tsne,
    /// Output: records drawn from a kernel density of the inputs (not a model).
    KdeSimulation,
    /// The user's own build and score scripts.
    ExtensionModel,
}

impl Kind {
    fn of(catalog_id: &str) -> Option<Kind> {
        match catalog_id {
            "random_forest" => Some(Kind::RandomForest),
            "xgboost_tree" => Some(Kind::XgboostTree),
            "xgboost_linear" => Some(Kind::XgboostLinear),
            "gmm" => Some(Kind::GaussianMixture),
            "one_class_svm" => Some(Kind::OneClassSvm),
            "hdbscan" => Some(Kind::Hdbscan),
            "kde" => Some(Kind::Kde),
            "smote" => Some(Kind::Smote),
            "tsne" => Some(Kind::Tsne),
            "kde_simulation" => Some(Kind::KdeSimulation),
            "extension_model" => Some(Kind::ExtensionModel),
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
            Kind::Hdbscan => "hdbscan",
            Kind::Kde => "kde",
            Kind::Smote => "smote",
            Kind::Tsne => "tsne",
            Kind::KdeSimulation => "kde_simulation",
            Kind::ExtensionModel => "extension_model",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::RandomForest => "Random Forest",
            Kind::XgboostTree => "XGBoost Tree",
            Kind::XgboostLinear => "XGBoost Linear",
            Kind::GaussianMixture => "Gaussian Mixture",
            Kind::OneClassSvm => "One-Class SVM",
            Kind::Hdbscan => "HDBSCAN",
            Kind::Kde => "KDE",
            Kind::Smote => "SMOTE",
            Kind::Tsne => "t-SNE",
            Kind::KdeSimulation => "KDE Simulation",
            Kind::ExtensionModel => "Extension Model",
        }
    }

    fn catalog_id(self) -> &'static str {
        match self {
            Kind::RandomForest => "random_forest",
            Kind::XgboostTree => "xgboost_tree",
            Kind::XgboostLinear => "xgboost_linear",
            Kind::GaussianMixture => "gmm",
            Kind::OneClassSvm => "one_class_svm",
            Kind::Hdbscan => "hdbscan",
            Kind::Kde => "kde",
            Kind::Smote => "smote",
            Kind::Tsne => "tsne",
            Kind::KdeSimulation => "kde_simulation",
            Kind::ExtensionModel => "extension_model",
        }
    }

    fn supervised(self) -> bool {
        matches!(
            self,
            Kind::RandomForest | Kind::XgboostTree | Kind::XgboostLinear | Kind::Smote
        )
    }

    /// SMOTE and t-SNE transform data; the others train a model.
    fn trains(self) -> bool {
        !matches!(self, Kind::Smote | Kind::Tsne | Kind::KdeSimulation)
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
    if node.model_builder.is_some() && kind.trains() {
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
    // Only model nodes train on the training partition.
    let mut partitioned = kind.trains();
    let mut custom = false;
    let mut color = None;
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
            ("color_field", ParamValue::Text(text)) => {
                color = Some(text.trim().to_string()).filter(|t| !t.is_empty());
            }
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
        color = None;
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
        color,
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
    /// t-SNE: the field the points are coloured by (default: the Target role field).
    color: Option<String>,
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
        if self.kind == Kind::Smote && target_kind == "continuous" {
            return Err(OperatorError::Settings(
                "SMOTE: the target must be categorical (set its measurement with a Type node)"
                    .into(),
            ));
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

fn summary_document(kind: Kind, summary: &serde_json::Value, printed: &str) -> Document {
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
        ("mean_log_density", "Mean log density"),
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
    // What a build script printed.
    let lines: Vec<Vec<String>> = printed
        .lines()
        .take(10_000)
        .map(|line| vec![line.to_string()])
        .collect();
    if !lines.is_empty() {
        tables.push(DocumentTable {
            title: Some("Output".into()),
            header: Vec::new(),
            rows: lines,
        });
    }
    Document {
        title: kind.label().to_string(),
        tables,
    }
}

impl Builder {
    fn python(&self, context: &ExecContext) -> Result<dms_python::PythonCommand, OperatorError> {
        dms_python::resolve(context.python.as_deref(), &context.cancel)
            .map_err(|e| bridge(self.kind, e))
    }

    /// KDE Simulation: records drawn from a kernel density of the numeric inputs.
    fn kde_simulation(&self, input: &Table, context: &ExecContext) -> Result<Table, OperatorError> {
        let (_, _, fields) = self.fields(input)?;
        let names: Vec<&str> = fields.iter().map(String::as_str).collect();
        let data = project(input, &names)?;
        let parameters = serde_json::json!({"inputs": fields, "options": self.options});
        let python = self.python(context)?;
        dms_python::models::transform(
            &python,
            &data,
            &parameters,
            "kde_simulation",
            &context.cancel,
        )
        .map_err(|e| bridge(self.kind, e))
    }

    /// SMOTE: the input records followed by synthetic ones.
    fn smote(&self, input: &Table, context: &ExecContext) -> Result<Table, OperatorError> {
        let (target, _, fields) = self.fields(input)?;
        let sendable: Vec<&str> = input
            .model()
            .fields()
            .iter()
            .filter(|f| f.storage != Storage::List)
            .map(|f| f.name.as_str())
            .collect();
        let data = project(input, &sendable)?;
        let parameters = serde_json::json!({
            "target": target,
            "inputs": fields,
            "options": self.options,
        });
        let python = self.python(context)?;
        dms_python::models::transform(&python, &data, &parameters, "smote", &context.cancel)
            .map_err(|e| bridge(self.kind, e))
    }

    /// t-SNE: the embedding as a document whose table the results panel draws as a scatter.
    fn tsne(&self, input: &Table, context: &ExecContext) -> Result<Document, OperatorError> {
        let (_, _, fields) = self.fields(input)?;
        let color = match &self.color {
            Some(name) => {
                input.model().field(name).ok_or_else(|| {
                    OperatorError::Data(format!("t-SNE: field {name:?} is not in the data"))
                })?;
                Some(name.clone())
            }
            None => input
                .model()
                .fields()
                .iter()
                .find(|f| f.role == Role::Target)
                .map(|f| f.name.clone()),
        };
        let mut keep: Vec<&str> = fields.iter().map(String::as_str).collect();
        if let Some(color) = color.as_deref().filter(|c| !keep.contains(c)) {
            keep.push(color);
        }
        let data = project(input, &keep)?;
        let parameters = serde_json::json!({
            "inputs": fields,
            "color": color,
            "options": self.options,
        });
        let python = self.python(context)?;
        let embedding =
            dms_python::models::transform(&python, &data, &parameters, "tsne", &context.cancel)
                .map_err(|e| bridge(self.kind, e))?;
        let header = embedding
            .model()
            .fields()
            .iter()
            .map(|f| f.name.clone())
            .collect();
        let rows = embedding
            .rows()
            .iter()
            .map(|row| {
                row.iter()
                    .map(|value| match value {
                        Value::Null => "$null$".to_string(),
                        Value::Real(x) => x.to_string(),
                        Value::Integer(n) => n.to_string(),
                        Value::String(s) => s.to_string(),
                        other => format!("{other:?}"),
                    })
                    .collect()
            })
            .collect();
        Ok(Document {
            title: "t-SNE".into(),
            tables: vec![DocumentTable {
                title: None,
                header,
                rows,
            }],
        })
    }
}

impl Operator for Builder {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs.first().ok_or_else(|| {
            OperatorError::Inputs(format!("{} needs one input", self.kind.label()))
        })?;
        match self.kind {
            Kind::Smote => return self.smote(input, context),
            Kind::KdeSimulation => return self.kde_simulation(input, context),
            _ => {}
        }
        Err(OperatorError::Inputs(format!(
            "{} produces a {}, not data",
            self.kind.label(),
            if self.kind == Kind::Tsne {
                "graph"
            } else {
                "model"
            }
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
        match self.kind {
            Kind::Smote | Kind::KdeSimulation => {
                return Ok(NodeOutput::Data(Arc::new(self.run(inputs, context)?)));
            }
            Kind::Tsne => return Ok(NodeOutput::Document(Arc::new(self.tsne(input, context)?))),
            _ => {}
        }
        // An Extension Model's scripts choose their own fields: every field but lists travels.
        let (target, target_kind, fields) = if self.kind == Kind::ExtensionModel {
            (None, "categorical", Vec::new())
        } else {
            self.fields(input)?
        };
        let data = if self.partitioned {
            training(input)?
        } else {
            input.as_ref().clone()
        };
        let keep: Vec<&str> = if self.kind == Kind::ExtensionModel {
            data.model()
                .fields()
                .iter()
                .filter(|f| f.storage != Storage::List)
                .map(|f| f.name.as_str())
                .collect()
        } else {
            // Only the fields the model uses travel to Python.
            let mut keep: Vec<&str> = fields.iter().map(String::as_str).collect();
            if let Some(target) = &target {
                keep.push(target);
            }
            keep
        };
        let data = project(&data, &keep)?;
        let python = self.python(context)?;
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
            summary: summary_document(self.kind, &trained.summary, &trained.text),
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
