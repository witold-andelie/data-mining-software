//! Auto Classifier (`BuildBinaryClassifierNode`, flag target) and Auto Numeric
//! (`BuildNumericClassifierNode`, continuous target) training: train every enabled component
//! with the settings the node saves, score the training records with each component's own
//! nugget, compare the components (`dms_algo::model_comparison`: the Algorithms Guide's "Binary
//! Classifier Comparison Metrics" for a flag target, Pearson correlation and relative error for a
//! continuous one), drop those outside an enabled limit, and keep the best `numberOfModelsToUse`.
//! The trained model is a PMML `MiningModel` (`selectAll`) whose segments are the kept components
//! in rank order, each `Segment@id` naming its builder; the Auto nugget scores each segment with
//! that builder's nugget (`nodes::auto_models`). Both nodes share every step; only the measures,
//! the direction of `numvar` and the target type differ.
//!
//! Evidence (`auto_train`, Modeler 19, deterministic label noise so the components differ):
//!
//! - Components are the `ensembleAlgorithms` entries with `algorithmEnabled="true"`, in their
//!   saved order. Each `AlgorithmParameter` holds the builder setting of the same name (native
//!   builders and Linear-AS: their settings element; tree and ALM extensions: their extension
//!   parameters). A parameter with several `PropertyTryValue`s would build several models per
//!   algorithm: GAP.
//! - Ranking is a stable sort of that order by the measure. Flag target: accuracy, AUC, lift,
//!   profit descending, and `numvar` (the fields the model refers to,
//!   `Pmml::referenced_fields`: CHAID's 4 of 5 inputs in `ac_lift`) DESCENDING — `ac_numvars_n2`
//!   keeps C5.0 and Logistic (5 fields each) over CHAID (4) and Decision List (3). Continuous
//!   target: correlation descending, `numvar` ASCENDING — `an_fields_n2` keeps Regression and
//!   Linear (3 fields each, saved order) over CHAID (3, later) and Linear-AS (4). Ties keep the
//!   saved order: `ac_trees_nofs` keeps Logistic (2nd) before C&R Tree (5th) at accuracy 314/360.
//! - Limits keep a component whose measure is at least the cutoff (accuracy in percent;
//!   `ac_accuracy_limit`: 85 % keeps 88.1, 87.2 and 85.3, drops 77.5; `an_corr_limit`:
//!   correlation 0.97 keeps Linear-AS and Linear), or at most it for relative error
//!   (`an_relerr_limit`: 0.1 keeps Linear-AS, Linear, CHAID) and `numvar`.
//! - With no partition field both ranking datasets are the training records (`ac_training_n2`
//!   equals `ac_accuracy_n2`, `an_training_n2` equals `an_corr_n2`); a partition field is a GAP.
//! - Flag target: `p̂`, the probability of the true value, is the confidence of a true prediction
//!   and one minus the confidence of a false one; Decision List's confidence is already the
//!   probability of its target value, with or without a prediction (remainder records predict
//!   `$null$`, which counts as the false value). Continuous target: correlation and relative
//!   error of the prediction; both equal `an_corr`'s saved 7-decimal values for all four
//!   components (`crates/exec/tests/auto_numeric_train.rs`).
//!
//! End to end, `auto_train/an_*` (Regression, Linear-AS, Linear, CHAID) and
//! `ac_trees_nofs` are exact. C&R Tree and QUEST components hold out their overfit prevention set
//! (30 % by default) with our generator, and the default component lists include Neural Net,
//! Random Trees and LSVM, so both nodes are statistical parity (`docs/statistical-parity.md`).

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

/// The two Auto nodes: what their target is, and how they name themselves.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    /// Auto Classifier.
    Flag,
    /// Auto Numeric.
    Continuous,
}

impl Target {
    fn node_name(self) -> &'static str {
        match self {
            Target::Flag => "Auto Classifier",
            Target::Continuous => "Auto Numeric",
        }
    }
}

fn gap(target: Target, message: impl std::fmt::Display) -> OperatorError {
    OperatorError::Settings(format!("GAP: {} training: {message}", target.node_name()))
}

/// `(extension id, object or element id, is object)`.
type ExtensionId = (&'static str, &'static str, bool);

/// How a component's builder reads its parameters, and the nugget that scores its model.
struct Kind {
    builder: &'static str,
    /// The class of the synthesized builder node.
    builder_class: &'static str,
    builder_extension: Option<ExtensionId>,
    applier: &'static str,
    applier_extension: Option<ExtensionId>,
    /// Extension parameters (`tree.*`, `linreg.*`) rather than a settings element.
    inline: bool,
}

const KINDS: &[Kind] = &[
    Kind {
        builder: "BuildC5RuleNode",
        builder_class: "BuildC5RuleNode",
        builder_extension: None,
        applier: "ApplyC5RuleNode",
        applier_extension: None,
        inline: false,
    },
    Kind {
        builder: "BuildLogisticNode",
        builder_class: "BuildLogisticNode",
        builder_extension: None,
        applier: "ApplyLogisticNode",
        applier_extension: None,
        inline: false,
    },
    Kind {
        builder: "BuildDecisionListNode",
        builder_class: "BuildDecisionListNode",
        builder_extension: None,
        applier: "ApplyDecisionListNode",
        applier_extension: None,
        inline: false,
    },
    Kind {
        builder: "BuildRegressionNode",
        builder_class: "BuildRegressionNode",
        builder_extension: None,
        applier: "ApplyRegressionNode",
        applier_extension: None,
        inline: false,
    },
    Kind {
        builder: "spss.linearEngine!linearEngine",
        builder_class: "ExtensionBuildModelNode",
        builder_extension: Some(("spss.linearEngine", "linearEngine", false)),
        applier: "ExtensionApplyModelNode",
        applier_extension: Some(("spss.linearEngine", "applyLinearEngine", false)),
        inline: false,
    },
    Kind {
        builder: "LinearRegression!spss.alm-extension",
        builder_class: "CFBuildModelNode",
        builder_extension: Some(("spss.alm-extension", "LinearRegression", true)),
        applier: "CFApplyModelNode",
        applier_extension: Some(("spss.alm-extension", "LinearRegressionModel", true)),
        inline: true,
    },
    Kind {
        builder: "Chaid!spss.tree-extension",
        builder_class: "CFBuildModelNode",
        builder_extension: Some(("spss.tree-extension", "Chaid", true)),
        applier: "ApplyChaidRuleNode",
        applier_extension: None,
        inline: true,
    },
    Kind {
        builder: "Cart!spss.tree-extension",
        builder_class: "CFBuildModelNode",
        builder_extension: Some(("spss.tree-extension", "Cart", true)),
        applier: "ApplyCartRuleNode",
        applier_extension: None,
        inline: true,
    },
    Kind {
        builder: "Quest!spss.tree-extension",
        builder_class: "CFBuildModelNode",
        builder_extension: Some(("spss.tree-extension", "Quest", true)),
        applier: "ApplyQuestRuleNode",
        applier_extension: None,
        inline: true,
    },
];

fn kind(builder: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|kind| kind.builder == builder)
}

fn extension((id, object, is_object): ExtensionId) -> Extension {
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
    extension_of: Option<ExtensionId>,
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
    let kind = kind(builder).ok_or_else(|| {
        OperatorError::Settings(format!("GAP: Auto training: component {builder}"))
    })?;
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
    Correlation,
    RelativeError,
}

/// An enabled limit: keep a component whose measure is at least (`true`) or at most the cutoff.
struct Limit {
    measure: Measure,
    at_least: bool,
    cutoff: f64,
}

/// A component's comparison measures on the training records; those of the other target type,
/// and lift when neither the ranking nor a limit uses it, are `None`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Comparison {
    pub accuracy: Option<f64>,
    pub auc: Option<f64>,
    pub lift: Option<f64>,
    pub profit: Option<f64>,
    pub correlation: Option<f64>,
    pub relative_error: Option<f64>,
}

/// Lift and profit settings (flag target).
#[derive(Clone, Copy)]
struct Scoring {
    lift_percentile: u32,
    revenue: f64,
    cost: f64,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let target = match node.type_identifier().as_str() {
        "BuildBinaryClassifierNode" => Target::Flag,
        "BuildNumericClassifierNode" => Target::Continuous,
        other => {
            return Err(OperatorError::Settings(format!(
                "GAP: Auto training for {other}"
            )));
        }
    };
    let gap = |message: String| gap(target, message);
    let s = node.settings.as_ref();
    let mut expected = vec![
        ("useCrossValidation", false),
        ("restrictModelBuildTime", false),
        ("restrictOverallBuildTime", false),
        ("useWeight", false),
        ("useFrequency", false),
        ("buildAllSelectedModel", false),
        ("calculateVariableImportances", false),
        ("autoModelFieldSelection", true),
        ("expertMode", false),
        ("useEvents", false),
    ];
    if target == Target::Flag {
        expected.extend([
            ("useMisclassificationCosts", false),
            ("useFixedCosts", true),
            ("useFixedRevenue", true),
            ("useFixedWeight", true),
        ]);
    }
    for (key, value) in expected {
        if settings::flag(s, key)?.is_some_and(|saved| saved != value) {
            return Err(gap(format!("{key} other than {value}")));
        }
    }
    for key in [
        "partitionField",
        "weightField",
        "frequencyField",
        "modelTargetFieldSelection",
        "modelSplitFieldSelection",
        "eventFields",
    ] {
        if settings::text(s, key).is_some_and(|v| !v.trim().is_empty()) {
            return Err(gap(format!("{key} is not supported")));
        }
    }
    if settings::element(s, "modelInputFieldSelection")
        .is_some_and(|e| e.child_elements().next().is_some() || !e.text().trim().is_empty())
    {
        return Err(gap("custom input fields are not supported".into()));
    }
    if target == Target::Flag && settings::real(s, "fixedWeight")?.is_some_and(|w| w != 1.0) {
        return Err(gap("a profit weight other than 1".into()));
    }
    match settings::text(s, "rankingDataset")
        .as_deref()
        .map(str::trim)
    {
        None | Some("" | "test" | "training") => {}
        Some(other) => return Err(gap(format!("ranking dataset {other:?}"))),
    }
    let measure = match (
        target,
        settings::text(s, "rankingMeasure")
            .as_deref()
            .map(str::trim),
    ) {
        (Target::Flag, Some("accuracy")) => Measure::Accuracy,
        (Target::Flag, Some("area_under_curve")) => Measure::Auc,
        (Target::Flag, Some("lift") | None) => Measure::Lift,
        (Target::Flag, Some("profit")) => Measure::Profit,
        (Target::Continuous, Some("correlation") | None) => Measure::Correlation,
        (_, Some("numvar")) => Measure::Fields,
        (_, Some(other)) => return Err(gap(format!("ranking measure {other:?}"))),
    };
    let keep = settings::integer(s, "numberOfModelsToUse")?.unwrap_or(5);
    let keep = usize::try_from(keep)
        .ok()
        .filter(|keep| (1..=100).contains(keep))
        .ok_or_else(|| gap(format!("numberOfModelsToUse {keep}")))?;
    let limit_settings: &[(Measure, bool, &str, &str)] = match target {
        Target::Flag => &[
            (
                Measure::Accuracy,
                true,
                "filterByAccuracyMeasure",
                "accuracyMeasureCutoff",
            ),
            (Measure::Auc, true, "filterByAucMeasure", "aucMeasureCutoff"),
            (
                Measure::Lift,
                true,
                "filterByLiftMeasure",
                "liftMeasureCutoff",
            ),
            (
                Measure::Profit,
                true,
                "filterByProfitMeasure",
                "profitMeasureCutoff",
            ),
            (
                Measure::Fields,
                false,
                "filterByNumvarMeasure",
                "numvarMeasureCutoff",
            ),
        ],
        Target::Continuous => &[
            (
                Measure::Correlation,
                true,
                "filterByCorrelation",
                "correlationCutoff",
            ),
            (
                Measure::RelativeError,
                false,
                "filterByRelativeError",
                "relativeErrorCutoff",
            ),
            (
                Measure::Fields,
                false,
                "filterByNumvarMeasure",
                "numvarMeasureCutoff",
            ),
        ],
    };
    let mut limits = Vec::new();
    for &(measure, at_least, flag, cutoff) in limit_settings {
        if settings::flag(s, flag)?.unwrap_or(false) {
            let value = settings::real(s, cutoff)?
                .filter(|v| v.is_finite())
                .ok_or_else(|| gap(format!("{cutoff} missing")))?;
            limits.push(Limit {
                measure,
                at_least,
                // Accuracy cutoffs are percentages.
                cutoff: if measure == Measure::Accuracy {
                    value / 100.0
                } else {
                    value
                },
            });
        }
    }
    let lift_percentile = settings::integer(s, "liftPercentile")?.unwrap_or(30);
    let scoring = Scoring {
        lift_percentile: u32::try_from(lift_percentile)
            .map_err(|_| gap(format!("lift percentile {lift_percentile}")))?,
        revenue: settings::real(s, "fixedRevenue")?.unwrap_or(10.0),
        cost: settings::real(s, "fixedCost")?.unwrap_or(5.0),
    };
    let mut components = Vec::new();
    for entry in settings::children(s, "ensembleAlgorithms", "persistentObjectListMember") {
        let builder = entry
            .attribute("algorithmName")
            .ok_or_else(|| gap("a component without algorithmName".into()))?;
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
        return Err(gap("no component algorithm is enabled".into()));
    }
    let needs_lift = measure == Measure::Lift || limits.iter().any(|l| l.measure == Measure::Lift);
    Ok(Box::new(AutoBuild {
        uid: node.uid.clone(),
        target,
        measure,
        keep,
        limits,
        scoring,
        needs_lift,
        components,
    }))
}

struct AutoBuild {
    uid: String,
    target: Target,
    measure: Measure,
    keep: usize,
    limits: Vec<Limit>,
    scoring: Scoring,
    needs_lift: bool,
    components: Vec<Component>,
}

/// One trained component and its comparison measures on the training records.
struct Trained {
    builder: String,
    pmml: Pmml,
    comparison: Comparison,
    fields: usize,
}

impl Trained {
    fn value(&self, measure: Measure) -> f64 {
        let c = &self.comparison;
        let value = match measure {
            Measure::Accuracy => c.accuracy,
            Measure::Auc => c.auc,
            Measure::Lift => c.lift,
            Measure::Profit => c.profit,
            Measure::Correlation => c.correlation,
            Measure::RelativeError => c.relative_error,
            Measure::Fields => Some(f64::from(u32::try_from(self.fields).unwrap_or(u32::MAX))),
        };
        value.unwrap_or(f64::NAN)
    }
}

impl Operator for AutoBuild {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(format!(
            "an {} build node produces a model, not data",
            self.target.node_name()
        )))
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        _model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        dms_algo::with_cancel(&context.cancel, || {
            let [input] = inputs else {
                return Err(OperatorError::Inputs(format!(
                    "{} training needs one input",
                    self.target.node_name()
                )));
            };
            if input
                .model()
                .fields()
                .iter()
                .any(|f| matches!(f.role, Role::Partition | Role::Split | Role::Frequency))
            {
                return Err(gap(self.target, "partition, split or frequency fields"));
            }
            let (target, true_value) = target_field(self.target, input)?;
            let mut trained = Vec::new();
            for (index, component) in self.components.iter().enumerate() {
                dms_algo::check_cancelled()?;
                trained.push(self.train(
                    component,
                    index,
                    input,
                    target,
                    true_value.as_ref(),
                    context,
                )?);
            }
            let mut ranked: Vec<&Trained> = trained
                .iter()
                .filter(|t| {
                    self.limits.iter().all(|limit| {
                        let value = t.value(limit.measure);
                        if limit.at_least {
                            value >= limit.cutoff
                        } else {
                            value <= limit.cutoff
                        }
                    })
                })
                .collect();
            // Stable: ties keep the saved component order. Fewer fields rank first for a continuous
            // target, more for a flag target; every other measure ranks the larger value first.
            let ascending = self.measure == Measure::Fields && self.target == Target::Continuous;
            ranked.sort_by(|a, b| {
                let (a, b) = (a.value(self.measure), b.value(self.measure));
                if ascending {
                    a.total_cmp(&b)
                } else {
                    b.total_cmp(&a)
                }
            });
            ranked.truncate(self.keep);
            if ranked.is_empty() {
                return Err(OperatorError::Data(format!(
                    "{}: no component model passed the limits",
                    self.target.node_name()
                )));
            }
            Ok(NodeOutput::Model(Arc::new(ensemble(
                self.target,
                input,
                &ranked,
            )?)))
        })
    }
}

impl AutoBuild {
    fn train(
        &self,
        component: &Component,
        index: usize,
        input: &Arc<Table>,
        target: usize,
        true_value: Option<&Value>,
        context: &ExecContext,
    ) -> Result<Trained, OperatorError> {
        let builder_id = component.builder.as_str();
        let kind = kind(builder_id).ok_or_else(|| gap(self.target, builder_id))?;
        let uid = format!("{}#{index}", self.uid);
        let builder = if kind.inline {
            build::build_component(
                &node(
                    uid.clone(),
                    kind.builder_class,
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
                kind.builder_class,
                kind.builder_extension,
                Some(settings),
            ))?
        };
        let NodeOutput::Model(pmml) =
            builder.produce(std::slice::from_ref(input), None, context)?
        else {
            return Err(gap(self.target, format!("{builder_id} produced no model")));
        };
        if !pmml.transformations.is_empty() || !pmml.functions.is_empty() {
            return Err(gap(
                self.target,
                format!("{builder_id} uses global transformations"),
            ));
        }
        let applier = apply::build(&component_applier(builder_id, uid)?)?;
        let NodeOutput::Data(scored) =
            applier.produce(std::slice::from_ref(input), Some(pmml.clone()), context)?
        else {
            return Err(gap(self.target, format!("{builder_id} scored no records")));
        };
        let comparison = compare(
            self.target,
            builder_id,
            input,
            &scored,
            target,
            true_value,
            self.scoring,
            self.needs_lift,
        )?;
        let fields = pmml
            .referenced_fields()
            .map_err(|error| OperatorError::Data(format!("{builder_id}: {error}")))?
            .len();
        Ok(Trained {
            builder: component.builder.clone(),
            comparison,
            fields,
            pmml: Arc::unwrap_or_clone(pmml),
        })
    }
}

/// The input's single target field and, for a flag target, its true value.
fn target_field(target: Target, input: &Table) -> Result<(usize, Option<Value>), OperatorError> {
    let fields = input.model().fields();
    let targets: Vec<usize> = (0..fields.len())
        .filter(|&i| fields[i].role == Role::Target)
        .collect();
    let wrong = || match target {
        Target::Flag => gap(target, "the target must be one flag field"),
        Target::Continuous => gap(target, "the target must be one continuous field"),
    };
    let [index] = targets.as_slice() else {
        return Err(wrong());
    };
    match (&fields[*index].values, target) {
        (ValueSpec::Flag { true_value, .. }, Target::Flag) => {
            Ok((*index, Some(true_value.clone())))
        }
        (ValueSpec::Flag { .. }, Target::Continuous) | (_, Target::Flag) => Err(wrong()),
        (_, Target::Continuous) => Ok((*index, None)),
    }
}

/// The measures of one component from its nugget's output (`scored`: the input columns, then
/// the prediction and, for a flag target, its confidence). Lift only when `with_lift`.
#[allow(
    clippy::too_many_arguments,
    reason = "two callers, each passing its own already-parsed pieces"
)]
fn compare(
    target: Target,
    builder: &str,
    input: &Table,
    scored: &Table,
    target_index: usize,
    true_value: Option<&Value>,
    scoring: Scoring,
    with_lift: bool,
) -> Result<Comparison, OperatorError> {
    let measure = |result: Result<f64, dms_algo::AlgoError>| {
        result.map(Some).map_err(|error| match error {
            dms_algo::AlgoError::Unsupported(message) => OperatorError::Settings(message),
            other => OperatorError::Data(format!("{builder}: {other}")),
        })
    };
    let width = input.model().len();
    let rows = input.rows().iter().zip(scored.rows());
    match (target, true_value) {
        (Target::Flag, Some(true_value)) => {
            let decision_list = builder == "BuildDecisionListNode";
            let mut scores = Vec::with_capacity(input.row_count());
            for (row, out) in rows {
                let observed = row.get(target_index).cloned().unwrap_or(Value::Null);
                let prediction = out.get(width).cloned().unwrap_or(Value::Null);
                let confidence = out
                    .get(width + 1)
                    .and_then(Value::as_f64)
                    .ok_or_else(|| gap(target, format!("{builder} wrote no confidence")))?;
                let propensity = if decision_list || prediction == *true_value {
                    confidence
                } else if prediction.is_null() {
                    return Err(gap(
                        target,
                        format!("{builder} left a record without a prediction"),
                    ));
                } else {
                    1.0 - confidence
                };
                let hit = observed == *true_value;
                scores.push(FlagScore {
                    hit,
                    // A Decision List record outside every segment (`$null$`) counts as a
                    // prediction of the other value: ac_lift's saved list has tp 138, tn 141.
                    correct: if decision_list && prediction.is_null() {
                        !hit
                    } else {
                        !prediction.is_null() && prediction == observed
                    },
                    propensity,
                });
            }
            Ok(Comparison {
                accuracy: measure(metrics::accuracy(&scores))?,
                auc: measure(metrics::auc(&scores))?,
                lift: if with_lift {
                    measure(metrics::lift(&scores, scoring.lift_percentile))?
                } else {
                    None
                },
                profit: measure(metrics::max_profit(&scores, scoring.revenue, scoring.cost))?,
                ..Comparison::default()
            })
        }
        (Target::Continuous, None) => {
            let mut pairs = Vec::with_capacity(input.row_count());
            for (row, out) in rows {
                let number = |value: Option<&Value>, what: &str| {
                    value
                        .and_then(Value::as_f64)
                        .filter(|v| v.is_finite())
                        .ok_or_else(|| gap(target, format!("{builder}: {what} is not a number")))
                };
                pairs.push((
                    number(out.get(width), "the prediction")?,
                    number(row.get(target_index), "the training target")?,
                ));
            }
            Ok(Comparison {
                correlation: measure(metrics::correlation(&pairs))?,
                relative_error: measure(metrics::relative_error(&pairs))?,
                ..Comparison::default()
            })
        }
        _ => Err(gap(target, "target type")),
    }
}

/// The measures of a saved Auto Classifier or Auto Numeric nugget's components in use, scored on
/// `input` with Modeler's own component models: the evidence for the measures and the ranking that
/// does not depend on our component learners. Each entry is the component's builder and its
/// measures. `lift_percentile`, `revenue` and `cost` apply to a flag target only.
pub fn saved_comparisons(
    nugget: &StreamNode,
    input: &Arc<Table>,
    context: &ExecContext,
    lift_percentile: u32,
    revenue: f64,
    cost: f64,
) -> Result<Vec<(String, Comparison)>, OperatorError> {
    let target = match nugget.type_identifier().as_str() {
        "ApplyAutoClassifierNode" | "ApplyBinaryClassifierNode" => Target::Flag,
        "ApplyNumericPredictorNode" => Target::Continuous,
        other => {
            return Err(OperatorError::Settings(format!(
                "GAP: {other} is not an Auto nugget"
            )));
        }
    };
    let (target_index, true_value) = target_field(target, input)?;
    let scoring = Scoring {
        lift_percentile,
        revenue,
        cost,
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
            .ok_or_else(|| gap(target, "a saved component without modelBuilderId"))?;
        let NodeOutput::Data(scored) =
            apply::build(&component)?.produce(std::slice::from_ref(input), None, context)?
        else {
            return Err(gap(target, format!("{builder} scored no records")));
        };
        let comparison = compare(
            target,
            &builder,
            input,
            &scored,
            target_index,
            true_value.as_ref(),
            scoring,
            true,
        )?;
        out.push((builder, comparison));
    }
    Ok(out)
}

/// The kept components as one `selectAll` MiningModel, each segment named by its builder.
fn ensemble(target: Target, input: &Table, ranked: &[&Trained]) -> Result<Pmml, OperatorError> {
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
    let (function, algorithm) = match target {
        Target::Flag => (MiningFunction::Classification, "BinaryClassifier"),
        Target::Continuous => (MiningFunction::Regression, "NumericClassifier"),
    };
    Ok(Pmml {
        version,
        data_dictionary: dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Mining(MiningModel {
            header: ModelHeader {
                name: Some(target.node_name().into()),
                function,
                algorithm: Some(algorithm.into()),
                mining_schema: schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            method: "selectAll".into(),
            segments,
        }),
    })
}
