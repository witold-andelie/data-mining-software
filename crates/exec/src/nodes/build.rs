//! Model build nodes (`docs/models.md`): train on the records that reach the node and hand the
//! model to the linked nugget (`Operator::produce` returns `NodeOutput::Model`).
//!
//! Supported: C&R Tree (`Cart!spss.tree-extension`) and CHAID (`Chaid!spss.tree-extension`).
//! They are extension-framework nodes: their algorithm settings are `<Parameter name= value=>`
//! entries of the XML attachment named by the node's `properties` attribute
//! (`docs/formats/nodes/cart.md`, `chaid.md`). Fields: with `role_use` the
//! attachment's `tree.TargetField` and `tree.InputFieldList`, otherwise the Type roles (one
//! target, inputs `Input` and `Both`). With `usePartitionedData` and a partition field, only the
//! training partition (the field's first value) trains.
//!
//! Rejected until implemented: frequency and analysis weights, split models, boosting and
//! bagging, the interactive builder, misclassification costs; for C&R Tree the overfit
//! prevention set (Modeler draws it with its own random generator; CHAID ignores the setting,
//! `models/chaid_default` trains on all records with 30% set), custom priors and ordered
//! twoing; for CHAID splitting merged categories.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::NodeOutput;
use crate::settings;
use dms_algo::Dataset;
use dms_algo::cart::{self, CartSettings, Impurity, MinSize};
use dms_algo::chaid::{self, ChaidSettings, ChiSquare, Method};
use dms_pmml::Pmml;
use dms_stream::StreamNode;
use dms_types::{DataModel, Role, Table};
use std::collections::HashMap;
use std::sync::Arc;

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let algorithm = match node.type_identifier().as_str() {
        "Cart!spss.tree-extension" => Algorithm::Cart,
        "Chaid!spss.tree-extension" => Algorithm::Chaid,
        other => {
            return Err(OperatorError::Settings(format!(
                "training {other} is not implemented yet"
            )));
        }
    };
    if settings::flag(s, "useFrequency")?.unwrap_or(false)
        || settings::flag(s, "useWeight")?.unwrap_or(false)
    {
        return Err(OperatorError::Settings(
            "model building with frequency or weight fields is not supported yet".into(),
        ));
    }
    Ok(Box::new(Build {
        algorithm,
        parameters_member: node.properties_attachment.clone(),
        use_partition: settings::flag(s, "usePartitionedData")?.unwrap_or(true),
    }))
}

#[derive(Debug, Clone, Copy)]
enum Algorithm {
    Cart,
    Chaid,
}

struct Build {
    algorithm: Algorithm,
    /// The attachment holding the extension parameters.
    parameters_member: Option<String>,
    use_partition: bool,
}

impl Operator for Build {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "a model build node produces a model, not data".into(),
        ))
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        _model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("a build node needs one input".into()))?;
        let parameters = self.parameters(context)?;
        tree_checks(&parameters)?;
        let table = self.training_records(input)?;
        let (target, fields) = roles(&table, &parameters, "tree.")?;
        let dataset = Dataset {
            table: &table,
            target,
            inputs: &fields,
            frequency: None,
        };
        let pmml = match self.algorithm {
            Algorithm::Cart => cart::train(&dataset, &cart_settings(&parameters)?)
                .map_err(|error| OperatorError::Data(format!("C&R Tree: {error}")))?,
            Algorithm::Chaid => chaid::train(&dataset, &chaid_settings(&parameters)?)
                .map_err(|error| OperatorError::Data(format!("CHAID: {error}")))?,
        };
        Ok(NodeOutput::Model(Arc::new(pmml)))
    }
}

impl Build {
    fn parameters(&self, context: &ExecContext) -> Result<Parameters, OperatorError> {
        let member = self.parameters_member.as_deref().ok_or_else(|| {
            OperatorError::Settings("the build node has no parameters attachment".into())
        })?;
        let bytes = context.attachments.get(member).ok_or_else(|| {
            OperatorError::Io(format!("the stream file has no attachment {member}"))
        })?;
        let text = std::str::from_utf8(bytes)
            .map_err(|_| OperatorError::Data(format!("{member} is not UTF-8 text")))?;
        Parameters::parse(text).map_err(|error| OperatorError::Data(format!("{member}: {error}")))
    }

    /// The records that train: the training partition when the node uses partitioned data.
    fn training_records(&self, input: &Table) -> Result<Table, OperatorError> {
        let model = input.model();
        let partition = model
            .fields()
            .iter()
            .position(|field| field.role == Role::Partition);
        let Some(position) = partition.filter(|_| self.use_partition) else {
            return Ok(input.clone());
        };
        let training = match &model.fields()[position].values {
            dms_types::ValueSpec::Categories(values) => values.first().cloned(),
            _ => None,
        }
        .ok_or_else(|| {
            OperatorError::Data("the partition field does not declare its partitions".into())
        })?;
        let rows = input
            .rows()
            .iter()
            .filter(|row| row.get(position) == Some(&training))
            .cloned()
            .collect();
        DataModel::new(model.fields().to_vec())
            .and_then(|model| Table::new(model, rows))
            .map_err(|error| OperatorError::Data(format!("training records: {error}")))
    }
}

/// The target and inputs: the attachment's custom fields with `role_use`, else the Type roles.
fn roles(
    table: &Table,
    parameters: &Parameters,
    prefix: &str,
) -> Result<(usize, Vec<usize>), OperatorError> {
    let model = table.model();
    let position = |name: &str| {
        model.position(name).ok_or_else(|| {
            OperatorError::Data(format!("the model field {name} is not in the input"))
        })
    };
    if parameters.flag("role_use")? {
        let target = parameters
            .text(&format!("{prefix}TargetField"))
            .filter(|t| !t.is_empty())
            .ok_or_else(|| OperatorError::Settings("custom fields without a target".into()))?;
        let inputs = parameters
            .list(&format!("{prefix}InputFieldList"))
            .iter()
            .map(|name| position(name))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok((position(&target)?, inputs));
    }
    let fields = model.fields();
    let targets: Vec<usize> = (0..fields.len())
        .filter(|&i| fields[i].role == Role::Target)
        .collect();
    let [target] = targets[..] else {
        return Err(OperatorError::Settings(format!(
            "the model needs exactly one target field, the input has {}",
            targets.len()
        )));
    };
    let inputs = (0..fields.len())
        .filter(|&i| matches!(fields[i].role, Role::Input | Role::Both))
        .collect();
    Ok((target, inputs))
}

/// Choices the tree algorithms share that are not supported yet.
fn tree_checks(p: &Parameters) -> Result<(), OperatorError> {
    let unsupported = |what: &str| OperatorError::Settings(format!("{what} is not supported yet"));
    if p.text("objective_goal").is_some_and(|g| g != "standard") {
        return Err(unsupported("boosting, bagging and very large datasets"));
    }
    if p.text("SingleModel").is_some_and(|m| m != "Single") {
        return Err(unsupported("the interactive tree builder"));
    }
    if p.text("objective_choose")
        .is_some_and(|c| c != "buildNewModel")
    {
        return Err(unsupported("continuing an existing model"));
    }
    if !p.list("split_distribute.split_fields").is_empty() {
        return Err(unsupported("split models"));
    }
    if p.flag("tree.UseCosts")? {
        return Err(unsupported("misclassification costs"));
    }
    Ok(())
}

/// The tree depth and node sizes both trees use.
fn tree_sizes(p: &Parameters) -> Result<(Option<usize>, MinSize, MinSize), OperatorError> {
    let depth = if p.text("MaxTreeDepth").as_deref() == Some("Custom") {
        p.real("tree.MaxTreeDepth")?.map(|d| d.max(0.0) as usize)
    } else {
        None
    };
    let sizes = if p
        .text("tree.UsePercentageForNodeSizeRequirement")
        .as_deref()
        == Some("absolute")
    {
        (
            MinSize::Records(p.real("tree.MinParentRecordsABS")?.unwrap_or(100.0)),
            MinSize::Records(p.real("tree.MinChildRecordsABS")?.unwrap_or(50.0)),
        )
    } else {
        (
            MinSize::Percent(p.real("tree.MinParentRecordsPct")?.unwrap_or(2.0)),
            MinSize::Percent(p.real("tree.MinChildRecordsPct")?.unwrap_or(1.0)),
        )
    };
    Ok((depth, sizes.0, sizes.1))
}

fn chaid_settings(p: &Parameters) -> Result<ChaidSettings, OperatorError> {
    let unsupported =
        |what: &str| OperatorError::Settings(format!("CHAID: {what} is not supported yet"));
    let defaults = ChaidSettings::default();
    if p.flag("tree.SplitMergedCategories")? {
        return Err(unsupported("splitting merged categories"));
    }
    let method = match p.text("method").as_deref() {
        None | Some("Chaid") => Method::Chaid,
        Some("ExhaustiveChaid") => Method::Exhaustive,
        Some(other) => return Err(unsupported(&format!("method {other}"))),
    };
    let chi_square = match p.text("tree.ChiSquareType").as_deref() {
        None | Some("pearson") => ChiSquare::Pearson,
        Some("LR") => ChiSquare::LikelihoodRatio,
        Some(other) => return Err(unsupported(&format!("chi-square {other}"))),
    };
    let (depth, min_parent, min_child) = tree_sizes(p)?;
    Ok(ChaidSettings {
        method,
        max_depth: depth.unwrap_or(defaults.max_depth),
        min_parent,
        min_child,
        split_alpha: p.real("tree.SplitAlpha")?.unwrap_or(defaults.split_alpha),
        merge_alpha: p.real("tree.MergeAlpha")?.unwrap_or(defaults.merge_alpha),
        bonferroni: p
            .text("tree.BonferroniAdjustment")
            .map_or(defaults.bonferroni, |v| v == "true"),
        chi_square,
    })
}

fn cart_settings(p: &Parameters) -> Result<CartSettings, OperatorError> {
    let unsupported =
        |what: &str| OperatorError::Settings(format!("C&R Tree: {what} is not supported yet"));
    if p.text("tree.Priors").is_some_and(|priors| priors != "Data") {
        return Err(unsupported("equal or custom priors"));
    }
    if p.flag("tree.UseOverFitPreventionSet")?
        && p.real("tree.OverFitPreventionSetPct")?.unwrap_or(0.0) > 0.0
    {
        return Err(unsupported("an overfit prevention set (set its size to 0)"));
    }
    let defaults = CartSettings::default();
    let (depth, min_parent, min_child) = tree_sizes(p)?;
    let max_depth = depth.unwrap_or(defaults.max_depth);
    let impurity = match p.text("tree.ImpurityMeasure").as_deref() {
        None | Some("Gini") => Impurity::Gini,
        Some("Twoing") => Impurity::Twoing,
        Some(other) => return Err(unsupported(&format!("the {other} impurity measure"))),
    };
    let std_err_rule = if p.flag("tree.UseStdErrRule")? {
        Some(p.real("tree.StdErrMultiplier")?.unwrap_or(1.0))
    } else {
        None
    };
    Ok(CartSettings {
        max_depth,
        min_parent,
        min_child,
        min_impurity_change: p
            .real("tree.MinImpurity")?
            .unwrap_or(defaults.min_impurity_change),
        impurity,
        max_surrogates: p
            .real("tree.MaxSurrogates")?
            .map_or(defaults.max_surrogates, |n| n.max(0.0) as usize),
        prune: p
            .text("tree.PruneTree")
            .map_or(defaults.prune, |v| v == "true"),
        std_err_rule,
    })
}

/// The `Parameter` entries of an extension node's attachment: a value or a list of values.
struct Parameters {
    values: HashMap<String, Parameter>,
}

enum Parameter {
    Value(String),
    List(Vec<String>),
}

impl Parameters {
    fn parse(text: &str) -> Result<Parameters, String> {
        let document = roxmltree::Document::parse(text).map_err(|error| error.to_string())?;
        let mut values = HashMap::new();
        for element in document
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "Parameter")
        {
            let Some(name) = element.attribute("name") else {
                continue;
            };
            let parameter = match element.attribute("value") {
                Some(value) => Parameter::Value(value.to_string()),
                None => Parameter::List(
                    element
                        .descendants()
                        .filter(|n| n.is_element() && n.tag_name().name() == "Value")
                        .filter_map(|n| n.attribute("value"))
                        .map(str::to_string)
                        .collect(),
                ),
            };
            values.insert(name.to_string(), parameter);
        }
        Ok(Parameters { values })
    }

    fn text(&self, name: &str) -> Option<String> {
        match self.values.get(name) {
            Some(Parameter::Value(value)) => Some(value.clone()),
            _ => None,
        }
    }

    fn list(&self, name: &str) -> Vec<String> {
        match self.values.get(name) {
            Some(Parameter::List(values)) => values.clone(),
            _ => Vec::new(),
        }
    }

    fn flag(&self, name: &str) -> Result<bool, OperatorError> {
        match self.text(name).as_deref() {
            None | Some("false") => Ok(false),
            Some("true") => Ok(true),
            Some(other) => Err(OperatorError::Settings(format!(
                "parameter {name}: {other:?} is not true or false"
            ))),
        }
    }

    fn real(&self, name: &str) -> Result<Option<f64>, OperatorError> {
        self.text(name)
            .map(|text| {
                text.trim().parse::<f64>().map_err(|_| {
                    OperatorError::Settings(format!("parameter {name}: {text:?} is not a number"))
                })
            })
            .transpose()
    }
}
