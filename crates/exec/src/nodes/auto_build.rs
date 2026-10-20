//! Auto Classifier training (`BuildBinaryClassifierNode`): train every enabled component with
//! the settings the node saves, score the training records with each component's own nugget,
//! compare the components with the Algorithms Guide's "Binary Classifier Comparison Metrics"
//! (`dms_algo::model_comparison`), drop those below an enabled limit, and keep the best
//! `numberOfModelsToUse`. The trained model is a PMML `MiningModel` (`selectAll`) whose segments
//! are the kept components in rank order, each `Segment@id` naming its builder; the Auto nugget
//! scores each segment with that builder's nugget (`nodes::auto_models`).
//!
//! Evidence (`auto_train`, Modeler 19, deterministic label noise so the components differ):
//!
//! - Components are the `ensembleAlgorithms` entries with `algorithmEnabled="true"`, in their
//!   saved order. Each `AlgorithmParameter` holds the builder setting of the same name (native
//!   builders: their settings element; tree and linear extensions: their extension parameters).
//!   A parameter with several `PropertyTryValue`s would build several models per algorithm: GAP.
//! - Ranking is a stable sort of that order by the measure: accuracy, AUC, lift, profit
//!   descending; `numvar` (the fields the model refers to, `Pmml::referenced_fields`: CHAID's
//!   4 of 5 inputs in `ac_lift`) descending for a flag target. Ties keep the saved order: `ac_trees_nofs` keeps Logistic (2nd) before C&R Tree
//!   (5th) at accuracy 314/360 each; `ac_numvars_n2` keeps C5.0 and Logistic (5 fields each)
//!   over CHAID (4) and Decision List (3).
//! - Limits keep a component whose measure is at least the cutoff (accuracy in percent;
//!   `ac_accuracy_limit`: 85 % keeps 88.1, 87.2 and 85.3, drops 77.5), or at most it for `numvar`.
//! - With no partition field both ranking datasets are the training records (`ac_training_n2`
//!   equals `ac_accuracy_n2`); a partition field is a GAP.
//! - `p̂`, the probability of the true value, is the confidence of a true prediction and one
//!   minus the confidence of a false one; Decision List's confidence is already the probability
//!   of its target value, with or without a prediction (remainder records predict `$null$`).
//!
//! C&R Tree and QUEST components hold out their overfit prevention set (30 % by default) with our
//! generator, so a stream that enables them with a nonzero set is statistical parity
//! (`docs/statistical-parity.md`); with the set at 0 (`ac_trees_nofs`) they are exact.

use super::{apply, build};
use crate::operator::{Arity, Operator};
use crate::settings;
use crate::{ExecContext, NodeOutput, OperatorError};
use dms_algo::model_comparison::{self as metrics, FlagScore};
use dms_pmml::{MiningFunction, MiningModel, MiningSegment, Model, ModelHeader, Pmml, Predicate};
use dms_stream::{Extension, StreamNode};
use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
use dms_types::{Role, Table, Value, ValueSpec};
use std::sync::Arc;

fn gap(message: impl std::fmt::Display) -> OperatorError {
    OperatorError::Settings(format!("GAP: Auto Classifier training: {message}"))
}

/// How a component's builder reads its parameters, and the nugget that scores its model.
struct Kind {
    builder: &'static str,
    /// The extension a builder or nugget is identified by; `None` for native nodes.
    builder_extension: Option<(&'static str, &'static str, bool)>,
    applier: &'static str,
    applier_extension: Option<(&'static str, &'static str, bool)>,
    /// Native builders read `AlgorithmParameter`s from their settings element, extension
    /// builders from their parameters.
    inline: bool,
}

/// `(extension id, object or element id, is object)`.
const KINDS: &[Kind] = &[
    Kind {
        builder: "BuildC5RuleNode",
        builder_extension: None,
        applier: "ApplyC5RuleNode",
        applier_extension: None,
        inline: false,
    },
    Kind {
        builder: "BuildLogisticNode",
        builder_extension: None,
        applier: "ApplyLogisticNode",
        applier_extension: None,
        inline: false,
    },
    Kind {
        builder: "BuildDecisionListNode",
        builder_extension: None,
        applier: "ApplyDecisionListNode",
        applier_extension: None,
        inline: false,
    },
    Kind {
        builder: "Chaid!spss.tree-extension",
        builder_extension: Some(("spss.tree-extension", "Chaid", true)),
        applier: "ApplyChaidRuleNode",
        applier_extension: None,
        inline: true,
    },
    Kind {
        builder: "Cart!spss.tree-extension",
        builder_extension: Some(("spss.tree-extension", "Cart", true)),
        applier: "ApplyCartRuleNode",
        applier_extension: None,
        inline: true,
    },
    Kind {
        builder: "Quest!spss.tree-extension",
        builder_extension: Some(("spss.tree-extension", "Quest", true)),
        applier: "ApplyQuestRuleNode",
        applier_extension: None,
        inline: true,
    },
];

fn kind(builder: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|kind| kind.builder == builder)
}

fn extension((id, object, is_object): (&str, &str, bool)) -> Extension {
    if is_object {
        Extension::Object {
            extension_id: id.into(),
            object_id: object.into(),
        }
    } else {
        Extension::Element {
            extension_id: id.into(),
            element_id: object.into(),
        }
    }
}

fn name(local: &str) -> QualifiedName {
    QualifiedName {
        prefix: None,
        namespace: None,
        local: Arc::from(local),
    }
}

fn node(
    uid: String,
    class: &str,
    extension_of: Option<(&str, &str, bool)>,
    settings: Option<Element>,
) -> StreamNode {
    StreamNode {
        uid,
        class: format!("com.spss.clementine.component.node.{class}"),
        extension: extension_of.map(extension),
        model_builder: None,
        name: None,
        label: None,
        position: None,
        properties_attachment: None,
        containers: Vec::new(),
        settings,
        binning_output: None,
        result_items: Vec::new(),
        subdiagram: None,
    }
}

/// The nugget that scores a component built by `builder` (a segment's `id`).
pub(crate) fn component_applier(builder: &str, uid: String) -> Result<StreamNode, OperatorError> {
    let kind = kind(builder).ok_or_else(|| gap(format!("component {builder}")))?;
    Ok(node(uid, kind.applier, kind.applier_extension, None))
}

struct Component {
    builder: String,
    parameters: Vec<(String, String)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Measure {
    Accuracy,
    Auc,
    Lift,
    Profit,
    Fields,
}

struct Limits {
    accuracy: Option<f64>,
    auc: Option<f64>,
    lift: Option<f64>,
    profit: Option<f64>,
    fields: Option<f64>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    for (key, expected) in [
        ("useMisclassificationCosts", false),
        ("useCrossValidation", false),
        ("restrictModelBuildTime", false),
        ("restrictOverallBuildTime", false),
        ("useWeight", false),
        ("useFrequency", false),
        ("buildAllSelectedModel", false),
        ("calculateVariableImportances", false),
        ("autoModelFieldSelection", true),
        ("useFixedCosts", true),
        ("useFixedRevenue", true),
        ("useFixedWeight", true),
    ] {
        if settings::flag(s, key)?.is_some_and(|value| value != expected) {
            return Err(gap(format!("{key} other than {expected}")));
        }
    }
    if settings::real(s, "fixedWeight")?.is_some_and(|weight| weight != 1.0) {
        return Err(gap("a profit weight other than 1"));
    }
    let measure = match settings::text(s, "rankingMeasure")
        .as_deref()
        .map(str::trim)
    {
        Some("accuracy") => Measure::Accuracy,
        Some("area_under_curve") => Measure::Auc,
        Some("lift") | None => Measure::Lift,
        Some("profit") => Measure::Profit,
        Some("numvar") => Measure::Fields,
        Some(other) => return Err(gap(format!("ranking measure {other:?}"))),
    };
    let keep = settings::integer(s, "numberOfModelsToUse")?.unwrap_or(5);
    let keep = usize::try_from(keep)
        .ok()
        .filter(|keep| (1..=100).contains(keep))
        .ok_or_else(|| gap(format!("numberOfModelsToUse {keep}")))?;
    let limit = |flag: &str, cutoff: &str| -> Result<Option<f64>, OperatorError> {
        if settings::flag(s, flag)?.unwrap_or(false) {
            settings::real(s, cutoff)?
                .filter(|v| v.is_finite())
                .map(Some)
                .ok_or_else(|| gap(format!("{cutoff} missing")))
        } else {
            Ok(None)
        }
    };
    let limits = Limits {
        accuracy: limit("filterByAccuracyMeasure", "accuracyMeasureCutoff")?,
        auc: limit("filterByAucMeasure", "aucMeasureCutoff")?,
        lift: limit("filterByLiftMeasure", "liftMeasureCutoff")?,
        profit: limit("filterByProfitMeasure", "profitMeasureCutoff")?,
        fields: limit("filterByNumvarMeasure", "numvarMeasureCutoff")?,
    };
    let lift_percentile = settings::integer(s, "liftPercentile")?.unwrap_or(30);
    let lift_percentile = u32::try_from(lift_percentile)
        .map_err(|_| gap(format!("lift percentile {lift_percentile}")))?;
    let revenue = settings::real(s, "fixedRevenue")?.unwrap_or(10.0);
    let cost = settings::real(s, "fixedCost")?.unwrap_or(5.0);
    let mut components = Vec::new();
    for entry in settings::children(s, "ensembleAlgorithms", "persistentObjectListMember") {
        let builder = entry
            .attribute("algorithmName")
            .ok_or_else(|| gap("a component without algorithmName"))?;
        match entry.attribute("algorithmEnabled") {
            Some("false") => continue,
            Some("true") => {}
            other => return Err(gap(format!("{builder} algorithmEnabled {other:?}"))),
        }
        if kind(builder).is_none() {
            return Err(gap(format!("component {builder} is not supported")));
        }
        let mut parameters = Vec::new();
        for parameter in entry
            .child_elements()
            .filter(|e| e.name.local.as_ref() == "AlgorithmParameter")
        {
            let key = parameter
                .attribute("propertyName")
                .ok_or_else(|| gap(format!("{builder} parameter without a name")))?;
            let values: Vec<&str> = parameter
                .child_elements()
                .filter(|e| e.name.local.as_ref() == "PropertyTryValue")
                .filter_map(|e| e.attribute("propertyValue"))
                .collect();
            match values.as_slice() {
                [value] => parameters.push((key.to_string(), value.to_string())),
                [] => {}
                _ => return Err(gap(format!("{builder} tries several values of {key}"))),
            }
        }
        components.push(Component {
            builder: builder.to_string(),
            parameters,
        });
    }
    if components.is_empty() {
        return Err(gap("no component algorithm is enabled"));
    }
    Ok(Box::new(AutoClassifierBuild {
        uid: node.uid.clone(),
        measure,
        keep,
        limits,
        lift_percentile,
        revenue,
        cost,
        components,
    }))
}

struct AutoClassifierBuild {
    uid: String,
    measure: Measure,
    keep: usize,
    limits: Limits,
    lift_percentile: u32,
    revenue: f64,
    cost: f64,
    components: Vec<Component>,
}

/// One trained component and its comparison measures on the training records.
struct Trained {
    builder: String,
    pmml: Pmml,
    accuracy: f64,
    auc: f64,
    lift: f64,
    profit: f64,
    fields: usize,
}

impl Operator for AutoClassifierBuild {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "an Auto Classifier build node produces a model, not data".into(),
        ))
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        _model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        let [input] = inputs else {
            return Err(OperatorError::Inputs(
                "Auto Classifier training needs one input".into(),
            ));
        };
        let fields = input.model().fields();
        if fields
            .iter()
            .any(|f| matches!(f.role, Role::Partition | Role::Split | Role::Frequency))
        {
            return Err(gap("partition, split or frequency fields"));
        }
        let targets: Vec<usize> = (0..fields.len())
            .filter(|&i| fields[i].role == Role::Target)
            .collect();
        let [target] = targets.as_slice() else {
            return Err(gap("the target must be one flag field"));
        };
        let ValueSpec::Flag { true_value, .. } = &fields[*target].values else {
            return Err(gap("the target must be one flag field"));
        };
        let mut trained = Vec::new();
        for (index, component) in self.components.iter().enumerate() {
            trained.push(self.train(component, index, input, *target, true_value, context)?);
        }
        let kept: Vec<&Trained> = trained
            .iter()
            .filter(|t| {
                let at_least = |cutoff: Option<f64>, value: f64| cutoff.is_none_or(|c| value >= c);
                at_least(self.limits.accuracy, t.accuracy * 100.0)
                    && at_least(self.limits.auc, t.auc)
                    && at_least(self.limits.lift, t.lift)
                    && at_least(self.limits.profit, t.profit)
                    && self
                        .limits
                        .fields
                        .is_none_or(|c| f64::from(u32::try_from(t.fields).unwrap_or(u32::MAX)) <= c)
            })
            .collect();
        let mut ranked = kept;
        ranked.sort_by(|a, b| match self.measure {
            Measure::Accuracy => b.accuracy.total_cmp(&a.accuracy),
            Measure::Auc => b.auc.total_cmp(&a.auc),
            Measure::Lift => b.lift.total_cmp(&a.lift),
            Measure::Profit => b.profit.total_cmp(&a.profit),
            Measure::Fields => b.fields.cmp(&a.fields),
        });
        ranked.truncate(self.keep);
        if ranked.is_empty() {
            return Err(OperatorError::Data(
                "Auto Classifier: no component model passed the limits".into(),
            ));
        }
        Ok(NodeOutput::Model(Arc::new(ensemble(input, &ranked)?)))
    }
}

impl AutoClassifierBuild {
    fn train(
        &self,
        component: &Component,
        index: usize,
        input: &Arc<Table>,
        target: usize,
        true_value: &Value,
        context: &ExecContext,
    ) -> Result<Trained, OperatorError> {
        let kind = kind(&component.builder).ok_or_else(|| gap(&component.builder))?;
        let uid = format!("{}#{index}", self.uid);
        let builder = if kind.inline {
            build::build_component(
                &node(
                    uid.clone(),
                    "CFBuildModelNode",
                    kind.builder_extension,
                    None,
                ),
                component.parameters.clone(),
            )?
        } else {
            let children = component
                .parameters
                .iter()
                .map(|(key, value)| {
                    Node::Element(Element {
                        name: name(key),
                        namespace_declarations: Vec::new(),
                        attributes: Vec::new(),
                        children: vec![Node::Text(Arc::from(value.as_str()))],
                    })
                })
                .collect();
            let settings = Element {
                name: name("properties"),
                namespace_declarations: Vec::new(),
                attributes: Vec::new(),
                children,
            };
            build::build(&node(
                uid.clone(),
                kind.builder,
                kind.builder_extension,
                Some(settings),
            ))?
        };
        let NodeOutput::Model(pmml) =
            builder.produce(std::slice::from_ref(input), None, context)?
        else {
            return Err(gap(format!("{} produced no model", component.builder)));
        };
        if !pmml.transformations.is_empty() || !pmml.functions.is_empty() {
            return Err(gap(format!(
                "{} uses global transformations",
                component.builder
            )));
        }
        let applier = apply::build(&component_applier(&component.builder, uid)?)?;
        let NodeOutput::Data(scored) =
            applier.produce(std::slice::from_ref(input), Some(pmml.clone()), context)?
        else {
            return Err(gap(format!("{} scored no records", component.builder)));
        };
        let scores = flag_scores(&component.builder, input, &scored, target, true_value)?;
        let comparison = compare(
            &component.builder,
            &scores,
            (self.measure == Measure::Lift || self.limits.lift.is_some())
                .then_some(self.lift_percentile),
            self.revenue,
            self.cost,
        )?;
        let fields = pmml
            .referenced_fields()
            .map_err(|error| OperatorError::Data(format!("{}: {error}", component.builder)))?
            .len();
        Ok(Trained {
            builder: component.builder.clone(),
            accuracy: comparison.accuracy,
            auc: comparison.auc,
            lift: comparison.lift.unwrap_or(f64::NAN),
            profit: comparison.profit,
            fields,
            pmml: Arc::unwrap_or_clone(pmml),
        })
    }
}

/// A component's comparison measures on the training records.
#[derive(Debug, Clone, Copy)]
pub struct Comparison {
    pub accuracy: f64,
    pub auc: f64,
    /// Computed only when the ranking or a limit uses it (a fractional cut is a GAP).
    pub lift: Option<f64>,
    pub profit: f64,
}

/// One `FlagScore` per record from a component nugget's output (`scored`: the input columns,
/// then the prediction and its confidence).
fn flag_scores(
    builder: &str,
    input: &Table,
    scored: &Table,
    target: usize,
    true_value: &Value,
) -> Result<Vec<FlagScore>, OperatorError> {
    let width = input.model().len();
    let decision_list = builder == "BuildDecisionListNode";
    let mut scores = Vec::with_capacity(input.row_count());
    for (row, out) in input.rows().iter().zip(scored.rows()) {
        let observed = row.get(target).cloned().unwrap_or(Value::Null);
        let prediction = out.get(width).cloned().unwrap_or(Value::Null);
        let confidence = out
            .get(width + 1)
            .and_then(Value::as_f64)
            .ok_or_else(|| gap(format!("{builder} wrote no confidence")))?;
        let propensity = if decision_list || prediction == *true_value {
            confidence
        } else if prediction.is_null() {
            return Err(gap(format!("{builder} left a record without a prediction")));
        } else {
            1.0 - confidence
        };
        let hit = observed == *true_value;
        scores.push(FlagScore {
            hit,
            // A Decision List record outside every segment (`$null$`) counts as a prediction
            // of the other value: ac_lift's saved list has tp 138, tn 141.
            correct: if decision_list && prediction.is_null() {
                !hit
            } else {
                !prediction.is_null() && prediction == observed
            },
            propensity,
        });
    }
    Ok(scores)
}

fn compare(
    builder: &str,
    scores: &[FlagScore],
    lift_percentile: Option<u32>,
    revenue: f64,
    cost: f64,
) -> Result<Comparison, OperatorError> {
    let measure = |result: Result<f64, dms_algo::AlgoError>| {
        result.map_err(|error| match error {
            dms_algo::AlgoError::Unsupported(message) => OperatorError::Settings(message),
            other => OperatorError::Data(format!("{builder}: {other}")),
        })
    };
    Ok(Comparison {
        accuracy: measure(metrics::accuracy(scores))?,
        auc: measure(metrics::auc(scores))?,
        lift: lift_percentile
            .map(|q| measure(metrics::lift(scores, q)))
            .transpose()?,
        profit: measure(metrics::max_profit(scores, revenue, cost))?,
    })
}

/// The measures of a saved Auto Classifier nugget's components in use, scored on `input` with
/// Modeler's own component models: the evidence for the measures and the ranking that does not
/// depend on our component learners. Each entry is the component's builder and its measures.
pub fn saved_comparisons(
    nugget: &StreamNode,
    input: &Arc<Table>,
    context: &ExecContext,
    lift_percentile: u32,
    revenue: f64,
    cost: f64,
) -> Result<Vec<(String, Comparison)>, OperatorError> {
    let fields = input.model().fields();
    let targets: Vec<usize> = (0..fields.len())
        .filter(|&i| fields[i].role == Role::Target)
        .collect();
    let [target] = targets.as_slice() else {
        return Err(gap("the target must be one flag field"));
    };
    let ValueSpec::Flag { true_value, .. } = &fields[*target].values else {
        return Err(gap("the target must be one flag field"));
    };
    let mut out = Vec::new();
    for (index, item) in nugget
        .result_items
        .iter()
        .filter(|item| item.attribute("useModel") == Some("true"))
        .enumerate()
    {
        let component = super::auto_models::component(nugget, item, index)?;
        let builder = component
            .model_builder
            .clone()
            .ok_or_else(|| gap("a saved component without modelBuilderId"))?;
        let NodeOutput::Data(scored) =
            apply::build(&component)?.produce(std::slice::from_ref(input), None, context)?
        else {
            return Err(gap(format!("{builder} scored no records")));
        };
        let scores = flag_scores(&builder, input, &scored, *target, true_value)?;
        let comparison = compare(&builder, &scores, Some(lift_percentile), revenue, cost)?;
        out.push((builder, comparison));
    }
    Ok(out)
}

/// The kept components as one `selectAll` MiningModel, each segment named by its builder.
fn ensemble(input: &Table, ranked: &[&Trained]) -> Result<Pmml, OperatorError> {
    let mut dictionary: Vec<dms_pmml::DataField> = Vec::new();
    let mut schema: Vec<dms_pmml::MiningField> = Vec::new();
    let mut segments = Vec::new();
    for trained in ranked {
        for field in &trained.pmml.data_dictionary {
            if !dictionary.iter().any(|known| known.name == field.name) {
                dictionary.push(field.clone());
            }
        }
        if let Some(header) = trained.pmml.model.header() {
            for field in &header.mining_schema {
                if !schema.iter().any(|known| known.name == field.name) {
                    schema.push(field.clone());
                }
            }
        }
        segments.push(MiningSegment {
            id: Some(trained.builder.clone()),
            predicate: Predicate::True,
            model: Box::new(trained.pmml.model.clone()),
        });
    }
    // Keep the input's field order for the shared dictionary.
    dictionary.sort_by_key(|field| input.model().position(&field.name).unwrap_or(usize::MAX));
    let version = ranked
        .first()
        .map(|t| t.pmml.version.clone())
        .unwrap_or_else(|| "4.2".into());
    Ok(Pmml {
        version,
        data_dictionary: dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Mining(MiningModel {
            header: ModelHeader {
                name: Some("Auto Classifier".into()),
                function: MiningFunction::Classification,
                algorithm: Some("BinaryClassifier".into()),
                mining_schema: schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            method: "selectAll".into(),
            segments,
        }),
    })
}
