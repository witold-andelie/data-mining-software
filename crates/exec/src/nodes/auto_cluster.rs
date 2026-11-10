//! Auto Cluster (`BuildAutoClusteringModelNode` / `ApplyAutoClusterNode`,
//! `oracle/models_next2/acluster_default`).
//!
//! The builder trains the clustering components listed in `ensembleAlgorithms` with the settings
//! the node saves — K-Means, TwoStep and Kohonen in the oracle — evaluates each on the records
//! that reach the node with [`dms_algo::cluster_evaluation`], ranks them by
//! `rankingMeasure="silhouette"` (largest first, the order Modeler lists the result items), and
//! marks the best `useModel="true"`. The PMML of that component is the builder's output, so the
//! linked Auto Cluster nugget scores it in memory through the same `dms-pmml` scorer as a saved
//! component.
//!
//! Component settings are read from the `AlgorithmParameter` children, not from an
//! `AlgorithmDefaults` shortcut. K-Means honors `expertMode`/`stoppingCondition` like the
//! standalone builder; TwoStep honors standardization, the automatic cluster range and BIC/AIC;
//! Kohonen honors the two training phases, the map dimensions in expert mode and the seed rule
//! below. Any other component type, an enabled/disabled algorithm list this module does not
//! know, or filters that would remove components is a `GAP:` error, as are several models in use.
//!
//! Kohonen's seed: when the component saves `useRandomSeed="true"`, its `randomSeed` is used.
//! Modeler's `useRandomSeed="false"` (the oracle) means the seed is not disclosed, so we use our
//! generator's fixed default `123` (`KohonenSettings::default().seed`). This is statistical
//! parity (`docs/statistical-parity.md`, rule 2); the corpus test holds Modeler's silhouette to
//! the widened band of 30 seeds and checks that K-Means still ranks first for each of them.
//! Modeler's 0.4009831637956 ranks 3rd of our 30 (band 0.3588..0.4866); its map has 10
//! non-empty units, smallest 1 and largest 8, as ours do (9 to 11 units per seed). One oracle
//! cannot show a bias, so the rank is reported, not asserted.
//!
//! The nugget keeps the saved-component path: when no builder supplies a model, the
//! existing `resultItem` with `useModel="true"` is reconstructed and scored, exactly as before.
//! A supplied model takes precedence. Direct scoring reproduces the component's label format:
//! K-Means `cluster-<n>`, TwoStep's cluster name (`cluster-1`), and Kohonen's `X=<x>, Y=<y>`.
//!
//! GAP: several components in use (`useMultipleModels="true"`), filters on, ranking measures
//! other than silhouette, variable importance, cross-validation, partitions, weights,
//! frequencies and events. Other component types are rejected rather than approximated.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::NodeOutput;
use crate::settings;
use dms_algo::cluster_evaluation::{self, ClusterEvaluation};
use dms_algo::kmeans::{self, KMeansSettings};
use dms_algo::kohonen::{self, KohonenSettings, Phase as KohonenPhase};
use dms_algo::twostep::{self, Criterion, TwoStepSettings};
use dms_pmml::{Model, Pmml, Usage, Value as PmmlValue};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::{Field, Measurement, Role, Storage, Table, Value};
use std::sync::Arc;

use super::field_util::{Column, with_columns};

/// The fixed default seed when a component does not disclose one (`useRandomSeed=false`).
pub const DEFAULT_KOHONEN_SEED: u64 = 123;

/// The component algorithms Auto Cluster trains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentAlgorithm {
    KMeans,
    TwoStep,
    Kohonen,
}

/// One trained and evaluated component.
#[derive(Debug, Clone, PartialEq)]
pub struct TrainedComponent {
    pub algorithm: ComponentAlgorithm,
    /// Modeler's result-item model name (`K-means 1`, `TwoStep 1`, `Kohonen 1`).
    pub label: String,
    pub model: Pmml,
    pub evaluation: ClusterEvaluation,
    /// `resultItem/@useModel`.
    pub use_model: bool,
}

/// The trained components, ranked by silhouette (best first), as the builder ranks them.
#[derive(Debug, Clone, PartialEq)]
pub struct TrainedAutoCluster {
    pub components: Vec<TrainedComponent>,
}

impl TrainedAutoCluster {
    /// The component marked `useModel="true"`.
    pub fn selected(&self) -> Option<&TrainedComponent> {
        self.components
            .iter()
            .find(|component| component.use_model)
            .or_else(|| self.components.first())
    }

    pub fn selected_model(&self) -> Option<&Pmml> {
        self.selected().map(|component| &component.model)
    }
}

/// The builder node (`BuildAutoClusteringModelNode`): trains when the plan asks for its model.
pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let settings = AutoClusterSettings::parse(node)?;
    Ok(Box::new(BuildAutoCluster { settings }))
}

struct BuildAutoCluster {
    settings: AutoClusterSettings,
}

impl Operator for BuildAutoCluster {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "an Auto Cluster build node produces a model, not data".into(),
        ))
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        _model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        dms_algo::with_cancel(&context.cancel, || {
            let input = inputs.first().ok_or_else(|| {
                OperatorError::Inputs("an Auto Cluster build node needs one input".into())
            })?;
            let trained = train_settings(&self.settings, input)?;
            let model = trained
                .selected_model()
                .ok_or_else(|| OperatorError::Data("Auto Cluster trained no component".into()))?;
            Ok(NodeOutput::Model(Arc::new(model.clone())))
        })
    }
}

/// Parsed builder settings. Kept separate so tests and the operator train the same way.
#[derive(Debug, Clone)]
struct AutoClusterSettings {
    algorithms: Vec<ComponentSettings>,
    custom_inputs: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
enum ComponentSettings {
    KMeans(KMeansSettings),
    TwoStep(TwoStepSettings),
    Kohonen(KohonenSettings),
}

impl AutoClusterSettings {
    fn parse(node: &StreamNode) -> Result<AutoClusterSettings, OperatorError> {
        let s = node.settings.as_ref().ok_or_else(|| {
            OperatorError::Settings("the Auto Cluster node has no settings".into())
        })?;
        let gap = |what: &str| {
            OperatorError::Settings(format!("Auto Cluster {what} is not supported yet (GAP)"))
        };

        match settings::text(Some(s), "rankingMeasure")
            .as_deref()
            .map(str::trim)
        {
            None | Some("") | Some("silhouette") => {}
            Some(other) => {
                return Err(gap(&format!("ranking measure {other:?}")));
            }
        }
        if settings::flag(Some(s), "calculateVariableImportances")?.unwrap_or(false) {
            return Err(gap("variable importance"));
        }
        if settings::flag(Some(s), "useCrossValidation")?.unwrap_or(false) {
            return Err(gap("cross-validation"));
        }
        if settings::flag(Some(s), "useWeight")?.unwrap_or(false)
            || settings::flag(Some(s), "useFrequency")?.unwrap_or(false)
            || settings::flag(Some(s), "useEvents")?.unwrap_or(false)
        {
            return Err(gap("weights, frequencies or events"));
        }
        for key in [
            "filterBySilhouetteMeasure",
            "filterByClusterRatioMeasure",
            "filterByMaxNumClustersMeasure",
            "filterByMinNumClustersMeasure",
            "filterBySmallestClusterMeasure",
            "filterByLargestClusterMeasure",
            "filterByEvaluationFieldImportance",
        ] {
            if settings::flag(Some(s), key)?.unwrap_or(false) {
                return Err(gap(key));
            }
        }
        if settings::flag(Some(s), "buildAllSelectedModel")?.unwrap_or(false) {
            return Err(gap("building every selected model"));
        }
        for key in ["weightField", "frequencyField", "partitionField"] {
            if settings::text(Some(s), key).is_some_and(|value| !value.trim().is_empty()) {
                return Err(gap(key));
            }
        }
        if !settings::children(Some(s), "eventFields", "field").is_empty() {
            return Err(gap("event fields"));
        }
        if !settings::children(
            Some(s),
            "ensembleAlgorithmsInactive",
            "persistentObjectListMember",
        )
        .is_empty()
        {
            return Err(gap("inactive ensemble algorithms"));
        }
        let keep = settings::integer(Some(s), "numberOfModelsToUse")?.unwrap_or(5);
        if keep < 1 {
            return Err(OperatorError::Settings(
                "Auto Cluster numberOfModelsToUse must be at least 1".into(),
            ));
        }

        let custom_inputs = if settings::flag(Some(s), "autoModelFieldSelection")?.unwrap_or(true) {
            None
        } else {
            let names: Vec<String> =
                settings::children(Some(s), "modelInputFieldSelection", "field")
                    .into_iter()
                    .map(|field| field.text().trim().to_string())
                    .filter(|name| !name.is_empty())
                    .collect();
            if names.is_empty() {
                return Err(OperatorError::Settings(
                    "Auto Cluster custom field selection is empty".into(),
                ));
            }
            Some(names)
        };

        let entries =
            settings::children(Some(s), "ensembleAlgorithms", "persistentObjectListMember");
        if entries.is_empty() {
            return Err(OperatorError::Settings(
                "Auto Cluster lists no ensemble algorithm".into(),
            ));
        }
        let mut algorithms = Vec::new();
        for entry in entries {
            if entry
                .attribute("algorithmEnabled")
                .is_some_and(|value| value.eq_ignore_ascii_case("false"))
            {
                continue;
            }
            let name = entry
                .attribute("algorithmName")
                .map(str::trim)
                .unwrap_or_default();
            let component = match name {
                "BuildKMeansNode" => ComponentSettings::KMeans(kmeans_component(entry)?),
                "BuildTwoStepNode" => ComponentSettings::TwoStep(twostep_component(entry)?),
                "TrainKohonenNode" => ComponentSettings::Kohonen(kohonen_component(entry)?),
                other => {
                    return Err(OperatorError::Settings(format!(
                        "Auto Cluster component {other:?} is not supported yet (GAP)"
                    )));
                }
            };
            algorithms.push(component);
        }
        if algorithms.is_empty() {
            return Err(OperatorError::Settings(
                "Auto Cluster has no enabled component".into(),
            ));
        }
        Ok(AutoClusterSettings {
            algorithms,
            custom_inputs,
        })
    }
}

/// Trains every component on `table`, evaluates and ranks it. Public for the corpus tests, which
/// exercise the same component settings as the stream.
pub fn train(node: &StreamNode, table: &Table) -> Result<TrainedAutoCluster, OperatorError> {
    let settings = AutoClusterSettings::parse(node)?;
    train_settings(&settings, table)
}

/// Trains and ranks the stream's components with the Kohonen seed replaced by `seed`.
///
/// This is the validation entry point for the 30-seed statistical-parity band
/// (`docs/statistical-parity.md`, rule 2). Production training uses the stream's seed; this keeps
/// every other saved component setting identical while varying only the undisclosed seed.
pub fn train_with_kohonen_seed(
    node: &StreamNode,
    table: &Table,
    seed: u64,
) -> Result<TrainedAutoCluster, OperatorError> {
    let mut settings = AutoClusterSettings::parse(node)?;
    for component in &mut settings.algorithms {
        if let ComponentSettings::Kohonen(kohonen) = component {
            kohonen.seed = seed;
        }
    }
    train_settings(&settings, table)
}

fn train_settings(
    settings: &AutoClusterSettings,
    table: &Table,
) -> Result<TrainedAutoCluster, OperatorError> {
    if table
        .model()
        .fields()
        .iter()
        .any(|field| field.role == Role::Partition)
    {
        return Err(OperatorError::Settings(
            "Auto Cluster with a partition field is not supported yet (GAP)".into(),
        ));
    }
    let inputs = cluster_inputs(table, settings.custom_inputs.as_deref())?;
    if inputs.is_empty() {
        return Err(OperatorError::Data(
            "Auto Cluster has no input fields to train on".into(),
        ));
    }
    let mut components = Vec::with_capacity(settings.algorithms.len());
    for algorithm in &settings.algorithms {
        dms_algo::check_cancelled()?;
        let (kind, label, pmml) = match algorithm {
            ComponentSettings::KMeans(component) => {
                let pmml = kmeans::train(table, &inputs, component)
                    .map_err(|error| component_error("K-Means", error))?;
                (
                    ComponentAlgorithm::KMeans,
                    component
                        .name
                        .clone()
                        .unwrap_or_else(|| "K-means 1".to_string()),
                    pmml,
                )
            }
            ComponentSettings::TwoStep(component) => {
                let pmml = twostep::train(table, &inputs, component)
                    .map_err(|error| component_error("TwoStep", error))?;
                (
                    ComponentAlgorithm::TwoStep,
                    component
                        .name
                        .clone()
                        .unwrap_or_else(|| "TwoStep 1".to_string()),
                    pmml,
                )
            }
            ComponentSettings::Kohonen(component) => {
                let pmml = kohonen::train(table, &inputs, component)
                    .map_err(|error| component_error("Kohonen", error))?;
                (
                    ComponentAlgorithm::Kohonen,
                    component
                        .name
                        .clone()
                        .unwrap_or_else(|| "Kohonen 1".to_string()),
                    pmml,
                )
            }
        };
        let evaluation = cluster_evaluation::evaluate(&pmml, table, &inputs)
            .map_err(|error| component_error(&label, error))?;
        if !evaluation.silhouette.is_finite() {
            return Err(OperatorError::Data(format!(
                "{label}: silhouette is not a finite number"
            )));
        }
        components.push(TrainedComponent {
            algorithm: kind,
            label,
            model: pmml,
            evaluation,
            use_model: false,
        });
    }
    if components.is_empty() {
        return Err(OperatorError::Data(
            "Auto Cluster trained no component".into(),
        ));
    }
    // Modeler lists the components best first; ties keep the ensembleAlgorithms order.
    components.sort_by(|left, right| {
        right
            .evaluation
            .silhouette
            .total_cmp(&left.evaluation.silhouette)
    });
    if let Some(best) = components.first_mut() {
        best.use_model = true;
    }
    Ok(TrainedAutoCluster { components })
}

fn component_error(name: &str, error: dms_algo::AlgoError) -> OperatorError {
    match error {
        dms_algo::AlgoError::Cancelled => OperatorError::Cancelled,
        dms_algo::AlgoError::Unsupported(message) => {
            OperatorError::Settings(format!("GAP: {name}: {message}"))
        }
        dms_algo::AlgoError::Settings(message) => {
            OperatorError::Settings(format!("{name}: {message}"))
        }
        dms_algo::AlgoError::Data(message) => OperatorError::Data(format!("{name}: {message}")),
    }
}

fn cluster_inputs(table: &Table, custom: Option<&[String]>) -> Result<Vec<usize>, OperatorError> {
    let model = table.model();
    match custom {
        Some(names) => names
            .iter()
            .map(|name| {
                model.position(name).ok_or_else(|| {
                    OperatorError::Data(format!("the model field {name} is not in the input"))
                })
            })
            .collect(),
        None => Ok((0..model.fields().len())
            .filter(|&index| matches!(model.fields()[index].role, Role::Input | Role::Both))
            .collect()),
    }
}

fn algorithm_parameter<'a>(entry: &'a Element, name: &str) -> Option<&'a str> {
    entry
        .child_elements()
        .filter(|parameter| parameter.attribute("propertyName") == Some(name))
        .find_map(|parameter| {
            parameter
                .child("PropertyTryValue")
                .and_then(|value| value.attribute("propertyValue"))
        })
}

fn text(entry: &Element, name: &str) -> Option<String> {
    algorithm_parameter(entry, name)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn flag(entry: &Element, name: &str) -> Result<Option<bool>, OperatorError> {
    match text(entry, name).as_deref() {
        None => Ok(None),
        Some("true") => Ok(Some(true)),
        Some("false") => Ok(Some(false)),
        Some(other) => Err(OperatorError::Settings(format!(
            "Auto Cluster component setting {name} expects true or false, found {other:?}"
        ))),
    }
}

fn integer(entry: &Element, name: &str) -> Result<Option<i64>, OperatorError> {
    text(entry, name)
        .map(|value| {
            value.parse::<i64>().map_err(|_| {
                OperatorError::Settings(format!(
                    "Auto Cluster component setting {name} expects an integer, found {value:?}"
                ))
            })
        })
        .transpose()
}

fn real(entry: &Element, name: &str) -> Result<Option<f64>, OperatorError> {
    text(entry, name)
        .map(|value| {
            value
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite())
                .ok_or_else(|| {
                    OperatorError::Settings(format!(
                        "Auto Cluster component setting {name} expects a number, found {value:?}"
                    ))
                })
        })
        .transpose()
}

fn positive(entry: &Element, name: &str, default: usize) -> Result<usize, OperatorError> {
    match integer(entry, name)? {
        None => Ok(default),
        Some(value) => usize::try_from(value)
            .ok()
            .filter(|&value| value > 0)
            .ok_or_else(|| {
                OperatorError::Settings(format!(
                    "Auto Cluster component setting {name} must be positive, found {value}"
                ))
            }),
    }
}

fn kmeans_component(entry: &Element) -> Result<KMeansSettings, OperatorError> {
    let defaults = KMeansSettings::default();
    let expert = flag(entry, "expertMode")?.unwrap_or(false);
    let custom_stop =
        expert && text(entry, "stoppingCondition").is_some_and(|value| value == "CUSTOM");
    for key in ["generateDistanceField", "showClusterProximity"] {
        if flag(entry, key)?.unwrap_or(false) {
            return Err(OperatorError::Settings(format!(
                "Auto Cluster K-Means {key} is not supported yet (GAP)"
            )));
        }
    }
    match text(entry, "optimization").as_deref() {
        None | Some("memory") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Auto Cluster K-Means optimization {other:?} is not supported yet (GAP)"
            )));
        }
    }
    Ok(KMeansSettings {
        clusters: positive(entry, "numClusters", defaults.clusters)?,
        max_iterations: if custom_stop {
            positive(entry, "numberIterations", defaults.max_iterations)?
        } else {
            defaults.max_iterations
        },
        tolerance: if custom_stop {
            real(entry, "changeTolerance")?.unwrap_or(defaults.tolerance)
        } else {
            defaults.tolerance
        },
        set_encoding: if expert {
            real(entry, "encodingValueForSets")?.unwrap_or(defaults.set_encoding)
        } else {
            defaults.set_encoding
        },
        name: Some("K-means 1".into()),
    })
}

fn twostep_component(entry: &Element) -> Result<TwoStepSettings, OperatorError> {
    let defaults = TwoStepSettings::default();
    if flag(entry, "excludeOutliers")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "Auto Cluster TwoStep outlier handling is not supported yet (GAP)".into(),
        ));
    }
    match text(entry, "distanceMeasure").as_deref() {
        None | Some("Loglikelihood") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Auto Cluster TwoStep distance {other:?} is not supported yet (GAP)"
            )));
        }
    }
    let criterion = match text(entry, "clusteringCriterion").as_deref() {
        None | Some("BIC") => Criterion::Bic,
        Some("AIC") => Criterion::Aic,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Auto Cluster TwoStep clustering criterion {other:?} is not supported"
            )));
        }
    };
    Ok(TwoStepSettings {
        standardize: flag(entry, "standardizeFields")?.unwrap_or(defaults.standardize),
        auto_clusters: flag(entry, "autoCluster")?.unwrap_or(defaults.auto_clusters),
        min_clusters: positive(entry, "minClusters", defaults.min_clusters)?,
        max_clusters: positive(entry, "maxClusters", defaults.max_clusters)?,
        clusters: positive(entry, "numClusters", defaults.clusters)?,
        criterion,
        label: defaults.label,
        numeric_labels: defaults.numeric_labels,
        name: Some("TwoStep 1".into()),
    })
}

fn kohonen_component(entry: &Element) -> Result<KohonenSettings, OperatorError> {
    let defaults = KohonenSettings::default();
    match text(entry, "optimization").as_deref() {
        None | Some("memory") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Auto Cluster Kohonen optimization {other:?} is not supported yet (GAP)"
            )));
        }
    }
    match text(entry, "stoppingCondition").as_deref() {
        None | Some("default") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Auto Cluster Kohonen stopping condition {other:?} is not supported yet (GAP)"
            )));
        }
    }
    match text(entry, "learningRateDecayMode").as_deref() {
        None | Some("linear") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Auto Cluster Kohonen learning-rate decay {other:?} is not supported yet (GAP)"
            )));
        }
    }
    let expert = flag(entry, "expertMode")?.unwrap_or(defaults.expert);
    let dimension = |name: &str, default: usize| -> Result<usize, OperatorError> {
        let value = positive(entry, name, default)?;
        if expert && !(1..=99).contains(&value) {
            return Err(OperatorError::Settings(format!(
                "Auto Cluster Kohonen {name} must be from 1 to 99, found {value}"
            )));
        }
        Ok(value)
    };
    let phase = |number: usize, default: KohonenPhase| -> Result<KohonenPhase, OperatorError> {
        Ok(KohonenPhase {
            neighbourhood: positive(
                entry,
                &format!("learningPhase{number}Neighbourhood"),
                default.neighbourhood,
            )?,
            eta: real(entry, &format!("learningPhase{number}Eta"))?.unwrap_or(default.eta),
            cycles: positive(
                entry,
                &format!("learningPhase{number}EtaDecayCycles"),
                default.cycles,
            )?,
        })
    };
    // A component either discloses a seed (`useRandomSeed=true`) or it does not. In the latter
    // case Modeler's seed is unknown, so use our generator's documented fixed default.
    let seed = if flag(entry, "useRandomSeed")?.unwrap_or(false) {
        let seed = integer(entry, "randomSeed")?.unwrap_or(defaults.seed as i64);
        if !(0..=2_147_483_647).contains(&seed) {
            return Err(OperatorError::Settings(format!(
                "Auto Cluster Kohonen randomSeed must be from 0 to 2147483647, found {seed}"
            )));
        }
        u64::try_from(seed).map_err(|_| {
            OperatorError::Settings(format!(
                "Auto Cluster Kohonen randomSeed {seed} is negative"
            ))
        })?
    } else {
        DEFAULT_KOHONEN_SEED
    };
    Ok(KohonenSettings {
        expert,
        width: dimension("outputLayerWidth", defaults.width)?,
        length: dimension("outputLayerLength", defaults.length)?,
        phases: [phase(1, defaults.phases[0])?, phase(2, defaults.phases[1])?],
        decay: defaults.decay,
        seed,
        name: Some("Kohonen 1".into()),
    })
}

/// The saved-component fallback, built lazily so a builder-supplied model can replace it.
enum Saved {
    Component(Box<dyn Operator>),
}

/// The Auto Cluster nugget: score a builder-supplied model, else the saved component.
pub fn apply(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node
        .settings
        .as_ref()
        .ok_or_else(|| OperatorError::Settings("the Auto Cluster nugget has no settings".into()))?;
    // The nugget's own output field (`model/outputs/output/name`, prefix `$XC`).
    let field = settings::children(Some(s), "model/outputs", "output")
        .into_iter()
        .filter_map(|output| settings::text(Some(output), "name"))
        .map(|name| name.trim().to_string())
        .find(|name| name.starts_with("$XC-"))
        .ok_or_else(|| {
            OperatorError::Settings("the Auto Cluster nugget names no $XC- output".into())
        })?;
    let multiple = settings::element(Some(s), "model")
        .and_then(|model| model.attribute("useMultipleModels"))
        .is_some_and(|value| value == "true");
    let saved = if multiple {
        Err(OperatorError::Settings(
            "Auto Cluster with several models in use is not supported yet (GAP)".into(),
        ))
    } else {
        saved_component(node)
    };
    Ok(Box::new(AutoCluster { field, saved }))
}

fn saved_component(node: &StreamNode) -> Result<Saved, OperatorError> {
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
    Ok(Saved::Component(super::apply::build(&component)?))
}

struct AutoCluster {
    field: String,
    /// Deferred: a builder-linked model may replace the saved component.
    saved: Result<Saved, OperatorError>,
}

impl Operator for AutoCluster {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("a model nugget needs one input".into()))?;
        let Saved::Component(component) = self.saved.as_ref().map_err(Clone::clone)?;
        let width = input.model().fields().len();
        let scored = match component.produce(inputs, None, context)? {
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
        model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("a model nugget needs one input".into()))?;
        let table = match model {
            Some(model) => score_model(&self.field, &model, input)?,
            None => self.run(inputs, context)?,
        };
        Ok(NodeOutput::Data(Arc::new(table)))
    }
}

/// Scores a trained component with its own PMML and writes the Auto Cluster label field.
fn score_model(field_name: &str, model: &Pmml, input: &Table) -> Result<Table, OperatorError> {
    let clustering = match &model.model {
        Model::Clustering(clustering) => clustering,
        _ => {
            return Err(OperatorError::Data(
                "the Auto Cluster builder produced a model that is not a clustering model".into(),
            ));
        }
    };
    let labels = match clustering.header.algorithm.as_deref() {
        Some("KMeans") => LabelStyle::Prefixed,
        Some("TwoStep") | Some("Kohonen") => LabelStyle::Direct,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "GAP: Auto Cluster model algorithm {other:?} is not supported"
            )));
        }
        None => {
            return Err(OperatorError::Settings(
                "GAP: Auto Cluster model names no algorithm".into(),
            ));
        }
    };
    let names = model_inputs(model);
    if names.is_empty() {
        return Err(OperatorError::Data(
            "the Auto Cluster model names no input fields".into(),
        ));
    }
    let mut positions = Vec::with_capacity(names.len());
    for name in &names {
        positions.push(input.model().position(name).ok_or_else(|| {
            OperatorError::Data(format!("the model field {name} is not in the input"))
        })?);
    }
    let references: Vec<&str> = names.iter().map(String::as_str).collect();
    let scorer = model.bind(&references).map_err(|error| {
        OperatorError::Data(format!("the Auto Cluster model cannot score: {error}"))
    })?;
    let mut values = Vec::with_capacity(input.row_count());
    for row in input.rows() {
        let record: Vec<PmmlValue> = positions
            .iter()
            .map(|&position| row.get(position).map_or(PmmlValue::Missing, to_pmml))
            .collect();
        let prediction = scorer.score(&record).map_err(|error| {
            OperatorError::Data(format!(
                "the Auto Cluster model cannot score a record: {error}"
            ))
        })?;
        let label = prediction
            .entity_id
            .clone()
            .or_else(|| match &prediction.value {
                PmmlValue::Text(text) => Some(text.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                OperatorError::Data("the Auto Cluster model produced no cluster".into())
            })?;
        let label = match labels {
            LabelStyle::Prefixed if !label.starts_with("cluster-") => format!("cluster-{label}"),
            _ => label,
        };
        values.push(Value::String(label.into()));
    }
    let mut field = Field::new(field_name.to_string(), Storage::String);
    field.measurement = Measurement::Nominal;
    with_columns(input, vec![Column { field, values }])
}

#[derive(Clone, Copy)]
enum LabelStyle {
    /// K-Means names its clusters `1`, `2`, ...; the nugget writes `cluster-1`, ...
    Prefixed,
    /// TwoStep and Kohonen already name their clusters.
    Direct,
}

fn model_inputs(model: &Pmml) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if let Model::Clustering(clustering) = &model.model {
        for mining in &clustering.header.mining_schema {
            if mining.usage == Usage::Active
                && !clustering
                    .header
                    .local_transformations
                    .iter()
                    .any(|derived| derived.name == mining.name)
                && !names.contains(&mining.name)
            {
                names.push(mining.name.clone());
            }
        }
    }
    if names.is_empty() {
        names.extend(model.data_dictionary.iter().map(|field| field.name.clone()));
    }
    names
}

fn to_pmml(value: &Value) -> PmmlValue {
    match value {
        Value::Null => PmmlValue::Missing,
        Value::Integer(number) => PmmlValue::Number(*number as f64),
        Value::Real(number) => PmmlValue::Number(*number),
        Value::String(text) => PmmlValue::Text(text.to_string()),
        Value::Date(number) | Value::Time(number) | Value::Timestamp(number) => {
            PmmlValue::Number(*number as f64)
        }
        Value::List(_) => PmmlValue::Text(value.to_string()),
    }
}
