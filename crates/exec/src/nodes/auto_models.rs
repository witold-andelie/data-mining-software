//! Saved Auto nuggets. Components use the ordinary nugget adapters; classification shares
//! the Ensemble arithmetic. Training is deliberately registered as scoring-only.
//!
//! Verified: models_next2/ac_default, ac_restricted and an_default, all terminal cells.
//! Numeric scores are the mean and sample standard deviation / sqrt(valid model count).
//! In an_default grid record 10004, ALM abstains outside its PMML input domain, leaving
//! four components. Legacy PASW Auto scoring is a GAP: pm_binaryclassifier's oracle runs
//! a replacement-linked builder first, and identical saved component scores have different
//! oracle predictions. Those results cannot validate a saved-model combination rule.

use super::field_util::{Column, with_columns};
use crate::operator::{Arity, Operator};
use crate::settings;
use crate::{ExecContext, NodeOutput, OperatorError};
use dms_pmml::{Model, Pmml};
use dms_stream::{Container, StreamNode};
use dms_strfmt::fastinfoset::Element;
use dms_types::{Field, Measurement, Storage, Table, Value};
use std::sync::Arc;

fn gap(message: impl std::fmt::Display) -> OperatorError {
    OperatorError::Settings(format!("GAP: Auto nugget: {message}"))
}

/// Reconstruct an ordinary nugget from a saved result item, including both attachment
/// layouts. Auto Cluster uses this too, so the result-item interpretation has one owner.
pub(crate) fn component(
    parent: &StreamNode,
    item: &Element,
    index: usize,
) -> Result<StreamNode, OperatorError> {
    let output = item
        .child("modelOutput")
        .ok_or_else(|| gap("component has no modelOutput"))?;
    let class = output
        .attribute("class")
        .map(dms_stream::simple_class_name)
        .unwrap_or_default();
    let identifier = match class {
        "C5RuleOutput" => "ApplyC5RuleNode",
        "CartRuleOutput" => "ApplyCartRuleNode",
        "ChaidRuleOutput" => "ApplyChaidRuleNode",
        "QuestRuleOutput" => "ApplyQuestRuleNode",
        "LogisticOutput" => "ApplyLogisticNode",
        "DecisionListModelOutput" => "ApplyDecisionListNode",
        "RegressionOutput" => "ApplyRegressionNode",
        "KMeansOutput" => "ApplyKMeansNode",
        "TwoStepOutput" => "ApplyTwoStepNode",
        "KohonenNetOutput" => "ApplyKohonenNode",
        "ExtensionModelOutput" => match (
            output.attribute("extensionID"),
            output.attribute("extensionElementID"),
        ) {
            (Some("spss.randomTrees"), Some("randomTreesModel")) => {
                "spss.randomTrees!applyRandomTrees"
            }
            (Some("spss.linearSVM"), Some("linearSVMModel")) => "spss.linearSVM!applyLinearSVM",
            (Some("spss.treeEngine"), Some("treeModel")) => "spss.treeEngine!applyTree",
            (Some("spss.linearEngine"), Some("linearEngineModel")) => {
                "spss.linearEngine!applyLinearEngine"
            }
            other => return Err(gap(format!("component extension {other:?}"))),
        },
        "CFModelOutput" => match (
            output.attribute("extensionID"),
            output.attribute("extensionObjectID"),
        ) {
            (Some("spss.alm-extension"), Some("LinearRegressionModel")) => {
                "LinearRegressionModel!spss.alm-extension"
            }
            (Some("spss.neuralnet-extension"), Some("neuralnetmodel")) => {
                "neuralnetmodel!spss.neuralnet-extension"
            }
            other => return Err(gap(format!("component object {other:?}"))),
        },
        other => return Err(gap(format!("component class {other:?}"))),
    };
    let mut containers = Vec::new();
    for container in output
        .child_elements()
        .filter(|e| e.name.local.as_ref() == "container")
    {
        if let Some(member) = container.attribute("containerData") {
            containers.push(Container {
                name: container.child("containerName").map(Element::text),
                member: member.to_string(),
            });
        }
    }
    for container in settings::children(Some(output), "contentContainers/containers", "container") {
        if let Some(member) = container
            .child("content")
            .and_then(|e| e.attribute("content"))
        {
            containers.push(Container {
                name: container.attribute("name").map(str::to_string),
                member: member.to_string(),
            });
        }
    }
    // The adapters dispatch on type_identifier, which also accepts these qualified names
    // as the class's simple name. No nested Auto component can recurse here.
    let (class, extension) = match identifier.split_once('!') {
        Some(("LinearRegressionModel" | "neuralnetmodel", _)) => {
            let (object, id) = identifier
                .split_once('!')
                .ok_or_else(|| gap("invalid object identifier"))?;
            (
                "CFApplyModelNode".to_string(),
                Some(dms_stream::Extension::Object {
                    extension_id: id.into(),
                    object_id: object.into(),
                }),
            )
        }
        Some((id, element)) => (
            "ExtensionApplyModelNode".to_string(),
            Some(dms_stream::Extension::Element {
                extension_id: id.into(),
                element_id: element.into(),
            }),
        ),
        None => (identifier.to_string(), None),
    };
    Ok(StreamNode {
        uid: format!("{}#{index}", parent.uid),
        class,
        extension,
        model_builder: output
            .child("modelBuildInfo")
            .and_then(|e| e.attribute("modelBuilderId"))
            .map(str::to_string),
        name: item.attribute("modelName").map(str::to_string),
        label: None,
        position: None,
        properties_attachment: output.attribute("properties").map(str::to_string),
        containers,
        settings: output.child("properties").cloned(),
        binning_output: None,
        result_items: Vec::new(),
        subdiagram: None,
    })
}

pub fn apply(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    build_auto(node).map_err(|error| match error {
        OperatorError::Settings(message) if !message.contains("GAP:") => gap(message),
        other => other,
    })
}

fn build_auto(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    validate_settings(s)?;
    if settings::text(s, "model/applicationVersion").as_deref() != Some("19") {
        return Err(gap(
            "legacy Auto model scoring is unverified; pm_binaryclassifier's oracle retrains a replacement-linked model (training is out of scope)",
        ));
    }
    if settings::element(s, "model").and_then(|m| m.attribute("useMultipleModels")) != Some("true")
    {
        return Err(gap(
            "only multiple-model Auto nuggets have verified scoring",
        ));
    }
    for (key, expected) in [
        ("filterEnsembledFields", true),
        ("useSmartScore", true),
        ("isEnsembleUpdate", false),
        ("useAccumulatedWeightedEvaluation", false),
    ] {
        if settings::flag(s, key)?.is_some_and(|value| value != expected) {
            return Err(gap(format!("{key} must be {expected}")));
        }
    }
    let numeric = node.type_identifier() == "ApplyNumericPredictorNode";
    let target = settings::text(s, "model/targetField/name")
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| gap("no target field"))?;
    let combine = if numeric {
        if settings::text(s, "rangeEnsembleMethod").is_some_and(|value| value.trim() != "mean") {
            return Err(gap("only mean numeric combination is supported"));
        }
        Combine::Numeric {
            standard_error: settings::flag(s, "calculateStandardError")?.unwrap_or(true),
        }
    } else {
        for (key, allowed) in [
            ("ensembleSetsMethod", &["confidenceWeightedVoting"][..]),
            ("ensembleSetTiesMethod", &["random"][..]),
        ] {
            if settings::text(s, key).is_some_and(|value| !allowed.contains(&value.trim())) {
                return Err(gap(format!("{key} is unsupported")));
            }
        }
        Combine::Flag(super::ensemble::FlagCombiner::new(s)?)
    };
    Ok(Box::new(AutoModels {
        node: node.clone(),
        target,
        combine,
    }))
}

/// The components in use of the nugget's saved Auto model.
fn saved_components(node: &StreamNode) -> Result<Vec<Scorer>, OperatorError> {
    let mut components = Vec::new();
    for item in &node.result_items {
        match item.attribute("useModel") {
            Some("false") => continue,
            Some("true") => {}
            _ => return Err(gap("invalid component useModel")),
        }
        if components.len() >= 256 {
            return Err(gap("more than 256 components"));
        }
        components.push(Scorer {
            nugget: super::apply::build(&component(node, item, components.len())?)?,
            model: None,
        });
    }
    if components.is_empty() {
        return Err(gap("no components in use"));
    }
    Ok(components)
}

/// The components of a model trained in this run (`nodes::auto_build`): a `selectAll`
/// MiningModel whose segments name their builders. Each segment is scored by that builder's
/// nugget as a model of its own over the shared data dictionary.
fn trained_components(node: &StreamNode, pmml: &Pmml) -> Result<Vec<Scorer>, OperatorError> {
    let Model::Mining(mining) = &pmml.model else {
        return Err(gap("the trained Auto model is not a MiningModel"));
    };
    if mining.method != "selectAll" || mining.segments.is_empty() {
        return Err(gap("the trained Auto model has no selectAll segments"));
    }
    mining
        .segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            let builder = segment
                .id
                .as_deref()
                .ok_or_else(|| gap("a trained Auto segment has no id"))?;
            Ok(Scorer {
                nugget: super::apply::build(&super::auto_build::component_applier(
                    builder,
                    format!("{}#{index}", node.uid),
                )?)?,
                model: Some(Arc::new(Pmml {
                    version: pmml.version.clone(),
                    data_dictionary: pmml.data_dictionary.clone(),
                    transformations: Vec::new(),
                    functions: Vec::new(),
                    model: (*segment.model).clone(),
                })),
            })
        })
        .collect()
}

/// A component nugget and, for a trained component, the model it scores.
struct Scorer {
    nugget: Box<dyn Operator>,
    model: Option<Arc<Pmml>>,
}

enum Combine {
    Flag(super::ensemble::FlagCombiner),
    Numeric { standard_error: bool },
}
struct AutoModels {
    node: StreamNode,
    target: String,
    combine: Combine,
}

impl Operator for AutoModels {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        self.score(inputs, &saved_components(&self.node)?, context)
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        let components = match &model {
            Some(pmml) => trained_components(&self.node, pmml)?,
            None => saved_components(&self.node)?,
        };
        self.score(inputs, &components, context)
            .map(|table| NodeOutput::Data(Arc::new(table)))
    }
}

impl AutoModels {
    fn score(
        &self,
        inputs: &[Arc<Table>],
        components: &[Scorer],
        context: &ExecContext,
    ) -> Result<Table, OperatorError> {
        let [input] = inputs else {
            return Err(OperatorError::Inputs("Auto nugget needs one input".into()));
        };
        let width = input.model().len();
        let mut scored = Vec::new();
        for component in components {
            let NodeOutput::Data(table) =
                component
                    .nugget
                    .produce(inputs, component.model.clone(), context)?
            else {
                return Err(OperatorError::Data(
                    "Auto component produced no records".into(),
                ));
            };
            if table.row_count() != input.row_count() || table.model().len() <= width {
                return Err(OperatorError::Data(
                    "Auto component changed record count or wrote no prediction".into(),
                ));
            }
            scored.push(table);
        }
        let mut predictions = Vec::with_capacity(input.row_count());
        let mut scores = Vec::with_capacity(input.row_count());
        let first_field = scored
            .first()
            .and_then(|t| t.model().fields().get(width))
            .ok_or_else(|| gap("component wrote no prediction"))?;
        let mut prediction = Field::new(
            format!(
                "{}-{}",
                if matches!(self.combine, Combine::Numeric { .. }) {
                    "$XR"
                } else {
                    "$XF"
                },
                self.target
            ),
            first_field.storage,
        );
        match &self.combine {
            Combine::Flag(_) => {
                let target = input
                    .model()
                    .field(&self.target)
                    .ok_or_else(|| gap("target metadata missing"))?;
                prediction.measurement = target.measurement;
                prediction.values = target.values.clone();
            }
            Combine::Numeric { .. } => prediction.measurement = Measurement::Continuous,
        }
        for row_index in 0..input.row_count() {
            let values: Vec<_> = scored
                .iter()
                .map(|t| {
                    t.rows()
                        .get(row_index)
                        .and_then(|r| r.get(width))
                        .cloned()
                        .unwrap_or(Value::Null)
                })
                .collect();
            let pair = match &self.combine {
                Combine::Flag(combiner) => {
                    let target = input
                        .model()
                        .field(&self.target)
                        .ok_or_else(|| gap("target metadata missing"))?;
                    let votes: Vec<_> = scored
                        .iter()
                        .zip(&values)
                        .map(|(t, value)| {
                            (
                                value.clone(),
                                t.rows()
                                    .get(row_index)
                                    .and_then(|r| r.get(width.saturating_add(1)))
                                    .cloned()
                                    .unwrap_or(Value::Null),
                            )
                        })
                        .collect();
                    combiner.score(target, &votes)?
                }
                Combine::Numeric { .. } => numeric_score(&values)?,
            };
            predictions.push(pair[0].clone());
            scores.push(pair[1].clone());
        }
        let mut columns = vec![Column {
            field: prediction,
            values: predictions,
        }];
        if !matches!(
            self.combine,
            Combine::Numeric {
                standard_error: false
            }
        ) {
            let mut field = Field::new(
                format!(
                    "{}-{}",
                    if matches!(self.combine, Combine::Numeric { .. }) {
                        "$XRE"
                    } else {
                        "$XFC"
                    },
                    self.target
                ),
                Storage::Real,
            );
            field.measurement = Measurement::Continuous;
            columns.push(Column {
                field,
                values: scores,
            });
        }
        if columns
            .iter()
            .any(|column| input.model().field(&column.field.name).is_some())
        {
            return Err(gap("Auto output field name already exists"));
        }
        with_columns(input, columns)
    }
}

fn validate_settings(s: Option<&Element>) -> Result<(), OperatorError> {
    const KNOWN: &[&str] = &[
        "accumulatedFactor",
        "accumulatedReducingLimit",
        "annotation",
        "autoEnsembleWeightsReevaluation",
        "autoLabel",
        "buildSettings",
        "builderNodeId",
        "builderStreamId",
        "cacheEnabled",
        "calculateStandardError",
        "camlRoundNum",
        "chartType",
        "createdBy",
        "creationDate",
        "crossValidationMean",
        "crossValidationSE",
        "cumulativePlot",
        "doReadValues",
        "enabled",
        "ensembleFlagEvaluationMethod",
        "ensembleFlagTiesMethod",
        "ensembleFlagsMethod",
        "ensembleSetTiesMethod",
        "ensembleSetsMethod",
        "fieldMappingEssentialFields",
        "filterEnsembledFields",
        "importedModel",
        "includeBaseline",
        "includeBestLine",
        "isEnsembleUpdate",
        "keywords",
        "label",
        "lastSavedAs",
        "lastSavedBy",
        "lastSavedDate",
        "model",
        "name",
        "plotNTiles",
        "position",
        "rankingMeasure",
        "refreshLinkState",
        "repalcebuilderNodeId",
        "repositoryLastModified",
        "repositoryPath",
        "repositoryVersionMarker",
        "scriptExecutionModeChange",
        "toolTipText",
        "useAccumulatedFactor",
        "useAccumulatedReducing",
        "useAccumulatedWeightedEvaluation",
        "useCrossValidation",
        "viPmml",
        "useSmartScore",
        "rangeEnsembleMethod",
    ];
    let s = s.ok_or_else(|| gap("no saved settings"))?;
    for child in s.child_elements() {
        if !KNOWN.contains(&child.name.local.as_ref()) {
            return Err(gap(format!("unsupported setting {}", child.name.local)));
        }
    }
    if s.child("fieldMappingEssentialFields")
        .is_some_and(|e| e.child_elements().next().is_some() || !e.text().trim().is_empty())
    {
        return Err(gap("field mapping is unsupported"));
    }
    Ok(())
}

fn numeric_score(values: &[Value]) -> Result<[Value; 2], OperatorError> {
    let values: Vec<_> = values
        .iter()
        .filter_map(Value::as_f64)
        .filter(|x| x.is_finite())
        .collect();
    let count = u32::try_from(values.len()).map_err(|_| gap("too many numeric components"))?;
    if count == 0 {
        return Ok([Value::Null, Value::Null]);
    }
    let mean = values.iter().sum::<f64>() / f64::from(count);
    let error = if count > 1 {
        (values.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
            / (f64::from(count) * f64::from(count - 1)))
        .sqrt()
    } else {
        0.0
    };
    Ok([Value::real(mean), Value::real(error)])
}
