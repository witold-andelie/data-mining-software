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
//! K-Means (`BuildKMeansNode`) keeps its settings in the node's XML (`docs/formats/nodes/kmeans.md`):
//! `numClusters`; with `expertMode` the `encodingValueForSets` and, when `stoppingCondition` is
//! `CUSTOM`, `numberIterations` and `changeTolerance` (otherwise Modeler's defaults). Inputs are the
//! `modelInputFieldSelection` fields unless `autoModelFieldSelection`, else the Type's `Input`
//! and `Both` fields.
//!
//! TwoStep (`BuildTwoStepNode`) also keeps its settings in the node's XML
//! (`docs/formats/nodes/twostep.md`): `standardizeFields`, `autoCluster` with `minClusters` and
//! `maxClusters`, else `numClusters`, `clusteringCriterion` (`BIC`, `AIC`), `clusterLabelType`
//! (`STRING`, `NUMBER`) and `clusterLabel`; inputs as for K-Means. Outlier handling
//! (`excludeOutliers`) and the Euclidean distance are rejected.
//!
//! Logistic (`BuildLogisticNode`) keeps its settings in the node's XML
//! (`docs/formats/nodes/logreg.md`): `logisticProcedure` (`Multinomial` default, `Binomial`),
//! `includeConstant`, `LogregMethod` / `LogregBinomialMethod` (Enter only), `modelType`
//! (`mainEffects` only), expert convergence (`maximumIterations`, `maximumStepHalving`,
//! `parameterConvergence`, `logLikelihoodConvergence`, `singularityTolerance`),
//! `multinomialBaseCategory`, and the Type roles or `modelTargetFieldSelection` /
//! `modelInputFieldSelection` when `autoModelFieldSelection` is false. Stepwise methods, custom
//! terms, full factorial models, a non-zero cell `delta`, and scaling are rejected.
//!
//! Linear (`LinearRegression!spss.alm-extension`) is an extension node like C&R Tree: fields from
//! `linreg.TargetField`/`linreg.InputFieldList` with `role_use`, `linreg.Intercept`; None or
//! forward-stepwise selection by AICC, AdjustedRSquare or Fstatistics, Pin/Pout, custom step
//! and effect limits, and Linear ADP. The default is ADP plus forward AICC (`dms_algo::linear`).
//! A single Type Split field trains independent models after the Partition filter.
//!
//! Regression (`BuildRegressionNode`) reads the node XML: `method` (`enter` only),
//! `includeConstant`, the field selections.
//!
//! Rejected until implemented: frequency and analysis weights, multiple split fields, boosting and
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
use dms_algo::anomaly::{self, AnomalyCutoff, AnomalySettings};
use dms_algo::cart::{self, CartSettings, Impurity, MinSize};
use dms_algo::chaid::{self, ChaidSettings, ChiSquare, Method};
use dms_algo::factor::{self, FactorCount, FactorSettings};
use dms_algo::kmeans::{self, KMeansSettings};
use dms_algo::linear::{self, LinearSettings, Selection, Style};
use dms_algo::logistic::{self, LogisticSettings, Method as LogisticMethod, Procedure};
use dms_algo::twostep::{self, Criterion, TwoStepSettings};
use dms_pmml::Pmml;
use dms_stream::StreamNode;
use dms_types::{DataModel, Role, Table};
use std::collections::HashMap;
use std::sync::Arc;

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    if node.type_identifier() == "BuildFeatureSelectionNode" {
        return super::feature_selection::build(node);
    }
    if node.type_identifier() == "BuildC5RuleNode" {
        return super::c50::build(node);
    }
    let s = node.settings.as_ref();
    let algorithm = match node.type_identifier().as_str() {
        "Cart!spss.tree-extension" => Algorithm::Cart,
        "Chaid!spss.tree-extension" => Algorithm::Chaid,
        "BuildKMeansNode" => Algorithm::KMeans(kmeans_settings(node)?),
        "BuildTwoStepNode" => Algorithm::TwoStep(twostep_settings(node)?),
        "BuildLogisticNode" => Algorithm::Logistic(logistic_settings(node)?),
        "LinearRegression!spss.alm-extension" => Algorithm::Linear,
        "BuildRegressionNode" => Algorithm::Regression(regression_settings(node)?),
        "BuildPCANode" => Algorithm::Factor(factor_settings(node)?),
        "BuildAnomalyNode" => Algorithm::Anomaly(anomaly_settings(node)?),
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

#[derive(Debug, Clone)]
enum Algorithm {
    Cart,
    Chaid,
    /// The settings, and the custom input fields (`None`: the Type roles).
    KMeans((KMeansSettings, Option<Vec<String>>)),
    TwoStep((TwoStepSettings, Option<Vec<String>>)),
    /// The settings, an optional custom target, and optional custom inputs.
    Logistic(LogisticSpec),
    /// Extension parameters, read when training.
    Linear,
    /// The settings, an optional custom target, and optional custom inputs.
    Regression(RegressionSpec),
    /// PCA/Factor settings and the custom input fields (`None`: the Type roles).
    Factor((FactorSettings, Option<Vec<String>>)),
    /// Anomaly Detection settings and the custom input fields (`None`: the Type roles).
    Anomaly((AnomalySettings, Option<Vec<String>>)),
}

type RegressionSpec = (LinearSettings, Option<String>, Option<Vec<String>>);

type LogisticSpec = (LogisticSettings, Option<String>, Option<Vec<String>>);

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
        let table = self.training_records(input)?;
        if let Algorithm::KMeans((settings, custom)) = &self.algorithm {
            let fields = cluster_inputs(&table, custom.as_deref())?;
            let pmml = kmeans::train(&table, &fields, settings)
                .map_err(|error| OperatorError::Data(format!("K-Means: {error}")))?;
            return Ok(NodeOutput::Model(Arc::new(pmml)));
        }
        if let Algorithm::Factor((settings, custom)) = &self.algorithm {
            let fields = cluster_inputs(&table, custom.as_deref())?;
            let pmml = factor::train(&table, &fields, settings)
                .map_err(|error| OperatorError::Data(format!("PCA/Factor: {error}")))?;
            return Ok(NodeOutput::Model(Arc::new(pmml)));
        }
        if let Algorithm::Anomaly((settings, custom)) = &self.algorithm {
            let fields = cluster_inputs(&table, custom.as_deref())?;
            let pmml = anomaly::train(&table, &fields, settings)
                .map_err(|error| OperatorError::Data(format!("Anomaly: {error}")))?;
            return Ok(NodeOutput::Model(Arc::new(pmml)));
        }
        if let Algorithm::TwoStep((settings, custom)) = &self.algorithm {
            let fields = cluster_inputs(&table, custom.as_deref())?;
            let pmml = twostep::train(&table, &fields, settings)
                .map_err(|error| OperatorError::Data(format!("TwoStep: {error}")))?;
            return Ok(NodeOutput::Model(Arc::new(pmml)));
        }
        if let Algorithm::Logistic((settings, target, inputs)) = &self.algorithm {
            let (target, fields) = supervised_fields(&table, target.as_deref(), inputs.as_deref())?;
            let dataset = Dataset {
                table: &table,
                target,
                inputs: &fields,
                frequency: None,
            };
            let pmml = logistic::train(&dataset, settings)
                .map_err(|error| OperatorError::Data(format!("Logistic: {error}")))?;
            return Ok(NodeOutput::Model(Arc::new(pmml)));
        }
        if let Algorithm::Regression((settings, target, inputs)) = &self.algorithm {
            let (target, fields) = supervised_fields(&table, target.as_deref(), inputs.as_deref())?;
            let dataset = Dataset {
                table: &table,
                target,
                inputs: &fields,
                frequency: None,
            };
            let pmml = linear::train(&dataset, settings)
                .map_err(|error| OperatorError::Data(format!("Regression: {error}")))?;
            return Ok(NodeOutput::Model(Arc::new(pmml)));
        }
        let parameters = self.parameters(context)?;
        tree_checks(&parameters)?;
        let prefix = match self.algorithm {
            Algorithm::Linear => "linreg.",
            _ => "tree.",
        };
        let (target, fields) = roles(&table, &parameters, prefix)?;
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
            Algorithm::Linear => train_linear(&dataset, &linear_settings(&parameters)?)
                .map_err(|error| OperatorError::Data(format!("Linear: {error}")))?,
            Algorithm::KMeans(_)
            | Algorithm::TwoStep(_)
            | Algorithm::Logistic(_)
            | Algorithm::Regression(_)
            | Algorithm::Factor(_)
            | Algorithm::Anomaly(_) => {
                return Err(OperatorError::Settings("clustering has no target".into()));
            }
        };
        Ok(NodeOutput::Model(Arc::new(pmml)))
    }
}

/// Train independently within one Type Split field, after the partition filter. The team
/// linear_regression stream has Class as Split; its terminal scores exercise all four groups.
/// GAP: team row 161 (Meat, Cost=104.68) has a missing oracle prediction, despite finite
/// inputs; our fitted model predicts 2.695437257275511. All 199 numeric oracle scores match.
/// Cost exceeds Meat's training maximum 100.15, but unused Confection Before also exceeds
/// its training range without a missing score. The split PMML export is empty. Need a probe
/// exporting individual split models and scoring just-inside/outside selected and unused
/// predictor bounds; do not invent a range restriction from one missing score.
/// GAP: multiple split fields and empty/invalid split groups need oracle cases.
fn train_linear(
    dataset: &Dataset<'_>,
    settings: &LinearSettings,
) -> Result<Pmml, dms_algo::AlgoError> {
    use dms_algo::AlgoError;
    use dms_pmml::{
        MiningFunction, MiningModel, MiningSegment, Model, ModelHeader, Predicate, Usage,
    };
    let splits: Vec<_> = dataset
        .table
        .model()
        .fields()
        .iter()
        .enumerate()
        .filter(|(_, f)| f.role == Role::Split)
        .collect();
    let (position, field) = match splits.as_slice() {
        [] => return linear::train(dataset, settings),
        [(position, field)] => (*position, *field),
        _ => {
            return Err(AlgoError::Unsupported(
                "Linear with multiple split fields".into(),
            ));
        }
    };
    if dataset.inputs.contains(&position) || dataset.target == position {
        return Err(AlgoError::Settings(
            "Linear split field cannot also be an input or target".into(),
        ));
    }
    if dataset.inputs.len() > 256
        || dataset.table.row_count() > 1_000_000
        || dataset
            .table
            .row_count()
            .saturating_mul(dataset.table.model().len())
            > 16_000_000
    {
        return Err(AlgoError::Data(
            "Linear split limit: 256 inputs / 1000000 rows / 16000000 cells".into(),
        ));
    }
    let mut groups = std::collections::BTreeMap::<String, Vec<Vec<dms_types::Value>>>::new();
    for row in dataset.table.rows() {
        let value = row
            .get(position)
            .filter(|v| !field.is_missing(v))
            .ok_or_else(|| AlgoError::Unsupported("Linear missing split value".into()))?;
        if groups.len() >= 128 && !groups.contains_key(&value.to_string()) {
            return Err(AlgoError::Data("Linear limit: 128 split groups".into()));
        }
        groups
            .entry(value.to_string())
            .or_default()
            .push(row.clone());
    }
    let mut segments = Vec::new();
    for (value, rows) in groups {
        let table = Table::new(dataset.table.model().clone(), rows)
            .map_err(|e| AlgoError::Data(e.to_string()))?;
        let mut pmml = linear::train(
            &Dataset {
                table: &table,
                ..*dataset
            },
            settings,
        )
        .map_err(|e| AlgoError::Data(format!("split {}={value}: {e}", field.name)))?;
        if let Model::GeneralRegression(m) = &mut pmml.model {
            m.header.local_transformations = pmml.transformations;
        }
        segments.push(MiningSegment {
            predicate: Predicate::Simple {
                field: field.name.clone(),
                operator: dms_pmml::Operator::Equal,
                value: Some(value),
            },
            model: Box::new(pmml.model),
        });
    }
    if segments.is_empty() {
        return Err(AlgoError::Data("Linear has no split groups".into()));
    }
    let mut dictionary = dms_algo::data_dictionary(dataset);
    dictionary.extend(dms_algo::data_field(dataset.table, position));
    let mut schema = dms_algo::mining_schema(dataset);
    schema.extend(dms_algo::mining_field(
        dataset.table,
        position,
        Usage::Active,
    ));
    Ok(Pmml {
        version: "4.3".into(),
        data_dictionary: dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Mining(MiningModel {
            header: ModelHeader {
                name: settings.name.clone(),
                function: MiningFunction::Regression,
                algorithm: Some("ALM".into()),
                mining_schema: schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            method: "selectFirst".into(),
            segments,
        }),
    })
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

/// K-Means settings from the node's XML.
/// PCA/Factor (`BuildPCANode`, `oracle/models_next/factor_*`): principal components of the
/// correlation matrix with complete records, factors by eigenvalue or by count, no rotation or
/// varimax, regression scores. Anything else is rejected (`crates/algo/src/factor.rs`).
fn factor_settings(
    node: &StreamNode,
) -> Result<(FactorSettings, Option<Vec<String>>), OperatorError> {
    let s = node.settings.as_ref();
    let unsupported =
        |what: &str| OperatorError::Settings(format!("PCA/Factor {what} is not supported yet"));
    let expert = settings::flag(s, "expertMode")?.unwrap_or(false);
    let text = |name: &str| settings::text(s, name).map(|t| t.trim().to_string());
    if text("extractionMethod")
        .as_deref()
        .is_some_and(|m| m != "Principal Components")
    {
        return Err(unsupported("extraction method"));
    }
    if expert {
        if text("matrixType")
            .as_deref()
            .is_some_and(|m| m != "Correlation")
        {
            return Err(unsupported("covariance matrix"));
        }
        if !settings::flag(s, "includeCompleteRecs")?.unwrap_or(true) {
            return Err(unsupported("pairwise deletion"));
        }
        if text("factorScoresMethod")
            .as_deref()
            .is_some_and(|m| m != "Regression")
        {
            return Err(unsupported("factor score method"));
        }
    }
    if settings::flag(s, "useWeight")?.unwrap_or(false)
        || settings::flag(s, "useFrequency")?.unwrap_or(false)
    {
        return Err(unsupported("weights or frequencies"));
    }
    let count = if !expert {
        FactorCount::Eigenvalue(1.0)
    } else {
        match text("limitFactors").as_deref() {
            None | Some("By Eigenvalue") => {
                FactorCount::Eigenvalue(settings::real(s, "eigenvalueLimit")?.unwrap_or(1.0))
            }
            Some("By Maximum") => {
                let max = settings::integer(s, "maxFactorLimit")?.unwrap_or(5);
                FactorCount::Maximum(
                    usize::try_from(max).map_err(|_| unsupported("maxFactorLimit"))?,
                )
            }
            Some(_) => return Err(unsupported("factor limit")),
        }
    };
    let varimax = if !expert {
        None
    } else {
        match text("rotationMethod").as_deref() {
            None | Some("No Rotation") => None,
            Some("Varimax") => {
                let sweeps = settings::integer(s, "maxIterations")?.unwrap_or(25);
                Some(usize::try_from(sweeps).map_err(|_| unsupported("maxIterations"))?)
            }
            Some(_) => return Err(unsupported("rotation")),
        }
    };
    let custom = if settings::flag(s, "autoModelFieldSelection")?.unwrap_or(true) {
        None
    } else {
        Some(
            settings::children(s, "modelInputFieldSelection", "field")
                .into_iter()
                .map(|f| f.text().trim().to_string())
                .filter(|name| !name.is_empty())
                .collect(),
        )
    };
    let name = (!settings::flag(s, "autoModelName")?.unwrap_or(true))
        .then(|| settings::text(s, "modelName"))
        .flatten()
        .filter(|n| !n.trim().is_empty());
    Ok((
        FactorSettings {
            count,
            varimax,
            name,
        },
        custom,
    ))
}

/// Anomaly Detection (`BuildAnomalyNode`, `oracle/models_next/anomaly_default`): the cutoff
/// method from the main settings, the expert settings only in expert mode (Modeler's defaults
/// otherwise). Noise handling, no imputation and weights are rejected
/// (`crates/algo/src/anomaly.rs`).
fn anomaly_settings(
    node: &StreamNode,
) -> Result<(AnomalySettings, Option<Vec<String>>), OperatorError> {
    let s = node.settings.as_ref();
    let unsupported = |what: &str| {
        OperatorError::Settings(format!("Anomaly Detection {what} is not supported yet"))
    };
    let count = |name: &str, default: i64| -> Result<usize, OperatorError> {
        usize::try_from(settings::integer(s, name)?.unwrap_or(default))
            .map_err(|_| unsupported(name))
    };
    let cutoff = match settings::text(s, "anomalyMethod").as_deref().map(str::trim) {
        Some("perRecords") | None => {
            AnomalyCutoff::Percent(settings::real(s, "perRecords")?.unwrap_or(1.0))
        }
        Some("numRecords") => AnomalyCutoff::Records(count("numRecords", 10)?),
        Some("indexLevel") => AnomalyCutoff::Index(settings::real(s, "indexLevel")?.unwrap_or(2.0)),
        Some(other) => return Err(unsupported(&format!("cutoff method {other}"))),
    };
    if settings::flag(s, "useWeight")?.unwrap_or(false)
        || settings::flag(s, "useFrequency")?.unwrap_or(false)
    {
        return Err(unsupported("weights or frequencies"));
    }
    let expert = settings::flag(s, "expertMode")?.unwrap_or(false);
    let mut anomaly_settings = AnomalySettings {
        cutoff,
        auto_groups: true,
        min_groups: 1,
        max_groups: 15,
        groups: 10,
        adjustment: 6.0,
        name: (!settings::flag(s, "autoModelName")?.unwrap_or(true))
            .then(|| settings::text(s, "modelName"))
            .flatten()
            .filter(|n| !n.trim().is_empty()),
    };
    if expert {
        if settings::real(s, "noiseLevel")?.unwrap_or(0.0) != 0.0 {
            return Err(unsupported("noise handling (noiseLevel)"));
        }
        if !settings::flag(s, "imputeMissValues")?.unwrap_or(true) {
            return Err(unsupported("without imputation (imputeMissValues)"));
        }
        anomaly_settings.auto_groups = settings::flag(s, "autoPeerGroup")?.unwrap_or(true);
        anomaly_settings.min_groups = count("minPeerGroups", 1)?;
        anomaly_settings.max_groups = count("maxPeerGroups", 15)?;
        anomaly_settings.groups = count("numPeerGroups", 10)?;
        anomaly_settings.adjustment = settings::real(s, "adjustmentCoeff")?.unwrap_or(6.0);
    }
    let custom = if settings::flag(s, "autoModelFieldSelection")?.unwrap_or(true) {
        None
    } else {
        Some(
            settings::children(s, "modelInputFieldSelection", "field")
                .into_iter()
                .map(|f| f.text().trim().to_string())
                .filter(|name| !name.is_empty())
                .collect(),
        )
    };
    Ok((anomaly_settings, custom))
}

fn kmeans_settings(
    node: &StreamNode,
) -> Result<(KMeansSettings, Option<Vec<String>>), OperatorError> {
    let s = node.settings.as_ref();
    let defaults = KMeansSettings::default();
    let expert = settings::flag(s, "expertMode")?.unwrap_or(false);
    let custom_stop =
        expert && settings::text(s, "stoppingCondition").is_some_and(|c| c.trim() == "CUSTOM");
    let clusters = settings::integer(s, "numClusters")?.unwrap_or(5);
    let iterations = if custom_stop {
        settings::integer(s, "numberIterations")?.unwrap_or(20)
    } else {
        20
    };
    let custom = if settings::flag(s, "autoModelFieldSelection")?.unwrap_or(true) {
        None
    } else {
        Some(
            settings::children(s, "modelInputFieldSelection", "field")
                .into_iter()
                .map(|f| f.text().trim().to_string())
                .filter(|name| !name.is_empty())
                .collect(),
        )
    };
    let name = (!settings::flag(s, "autoModelName")?.unwrap_or(true))
        .then(|| settings::text(s, "modelName"))
        .flatten()
        .filter(|n| !n.trim().is_empty());
    Ok((
        KMeansSettings {
            clusters: usize::try_from(clusters.max(1)).unwrap_or(1),
            max_iterations: usize::try_from(iterations.max(0)).unwrap_or(0),
            tolerance: if custom_stop {
                settings::real(s, "changeTolerance")?.unwrap_or(defaults.tolerance)
            } else {
                defaults.tolerance
            },
            set_encoding: if expert {
                settings::real(s, "encodingValueForSets")?.unwrap_or(defaults.set_encoding)
            } else {
                defaults.set_encoding
            },
            name,
        },
        custom,
    ))
}

/// TwoStep settings from the node's XML.
fn twostep_settings(
    node: &StreamNode,
) -> Result<(TwoStepSettings, Option<Vec<String>>), OperatorError> {
    let s = node.settings.as_ref();
    let defaults = TwoStepSettings::default();
    if settings::flag(s, "excludeOutliers")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "TwoStep outlier handling (excludeOutliers) is not supported yet".into(),
        ));
    }
    if settings::text(s, "distanceMeasure").is_some_and(|d| d.trim() == "Euclidean") {
        return Err(OperatorError::Settings(
            "TwoStep with the Euclidean distance is not supported yet".into(),
        ));
    }
    let count = |name: &str, default: usize| -> Result<usize, OperatorError> {
        Ok(settings::integer(s, name)?
            .and_then(|n| usize::try_from(n).ok())
            .filter(|&n| n > 0)
            .unwrap_or(default))
    };
    let criterion = match settings::text(s, "clusteringCriterion")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("BIC") => Criterion::Bic,
        Some("AIC") => Criterion::Aic,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "TwoStep clusteringCriterion {other:?} is not supported"
            )));
        }
    };
    let (_, custom) = kmeans_settings(node)?;
    let name = (!settings::flag(s, "autoModelName")?.unwrap_or(true))
        .then(|| settings::text(s, "modelName"))
        .flatten()
        .filter(|n| !n.trim().is_empty());
    Ok((
        TwoStepSettings {
            standardize: settings::flag(s, "standardizeFields")?.unwrap_or(true),
            auto_clusters: settings::flag(s, "autoCluster")?.unwrap_or(true),
            min_clusters: count("minClusters", defaults.min_clusters)?,
            max_clusters: count("maxClusters", defaults.max_clusters)?,
            clusters: count("numClusters", defaults.clusters)?,
            criterion,
            label: settings::text(s, "clusterLabel")
                .filter(|l| !l.is_empty())
                .unwrap_or(defaults.label),
            numeric_labels: settings::text(s, "clusterLabelType")
                .is_some_and(|t| t.trim() == "NUMBER"),
            name,
        },
        custom,
    ))
}

/// Logistic settings from the node's XML.
fn logistic_settings(node: &StreamNode) -> Result<LogisticSpec, OperatorError> {
    let s = node.settings.as_ref();
    let unsupported =
        |what: &str| OperatorError::Settings(format!("Logistic: {what} is not supported yet"));
    let procedure = match settings::text(s, "logisticProcedure")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("Multinomial") => Procedure::Multinomial,
        Some("Binomial") => Procedure::Binomial,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Logistic logisticProcedure {other:?} is not supported"
            )));
        }
    };
    match procedure {
        Procedure::Multinomial => {}
        Procedure::Binomial => match settings::text(s, "LogregBinomialMethod")
            .as_deref()
            .map(str::trim)
        {
            None | Some("") | Some("Enter") => {}
            Some(other) => return Err(unsupported(&format!("binomial method {other}"))),
        },
    }
    match settings::text(s, "modelType").as_deref().map(str::trim) {
        None | Some("") | Some("mainEffects") => {}
        Some(other) => return Err(unsupported(&format!("modelType {other}"))),
    }
    match settings::text(s, "scale").as_deref().map(str::trim) {
        None | Some("") | Some("none") => {}
        Some(other) => return Err(unsupported(&format!("scale {other}"))),
    }
    if !settings::children(s, "customEffects", "persistentObjectListMember").is_empty() {
        return Err(unsupported("custom terms"));
    }
    for row in settings::children(s, "logisticInputsTable/rows", "row") {
        match settings::text(Some(row), "contrast")
            .as_deref()
            .map(str::trim)
        {
            None | Some("") | Some("Indicator") => {}
            Some(other) => return Err(unsupported(&format!("binomial contrast {other}"))),
        }
    }
    let defaults = LogisticSettings::default();
    let method = match procedure {
        Procedure::Binomial => LogisticMethod::Enter,
        Procedure::Multinomial => {
            match settings::text(s, "LogregMethod").as_deref().map(str::trim) {
                None | Some("") | Some("enter") => LogisticMethod::Enter,
                Some("stepwise") => LogisticMethod::Stepwise,
                Some("forwards") => LogisticMethod::Forwards,
                Some("backwards") => LogisticMethod::Backwards,
                Some(other) => return Err(unsupported(&format!("method {other}"))),
            }
        }
    };
    if method != LogisticMethod::Enter {
        let entry = settings::text(s, "stepwiseEntryMethod").unwrap_or_default();
        let removal = settings::text(s, "stepwiseRemovalMethod").unwrap_or_default();
        if !matches!(entry.trim(), "" | "Score") || !matches!(removal.trim(), "" | "LR") {
            return Err(unsupported(&format!(
                "stepwise entry {entry:?} / removal {removal:?} (only Score / LR)"
            )));
        }
        if settings::flag(s, "LogregEnableMaxTerms")?.unwrap_or(false)
            || settings::integer(s, "minTerms")?.unwrap_or(0) != 0
        {
            return Err(unsupported("stepwise term limits"));
        }
    }
    let custom_target =
        settings::text(s, "modelTargetFieldSelection").filter(|name| !name.trim().is_empty());
    let custom_inputs = if settings::flag(s, "autoModelFieldSelection")?.unwrap_or(true) {
        None
    } else {
        let names: Vec<String> = settings::children(s, "modelInputFieldSelection", "field")
            .into_iter()
            .map(|f| f.text().trim().to_string())
            .filter(|name| !name.is_empty())
            .collect();
        if names.is_empty() { None } else { Some(names) }
    };
    let name = (!settings::flag(s, "autoModelName")?.unwrap_or(true))
        .then(|| settings::text(s, "modelName"))
        .flatten()
        .filter(|n| !n.trim().is_empty());
    let base = settings::text(s, "multinomialBaseCategory").filter(|c| !c.trim().is_empty());
    Ok((
        LogisticSettings {
            procedure,
            include_constant: settings::flag(s, "includeConstant")?.unwrap_or(true),
            method,
            p_in: settings::real(s, "pIn")?.unwrap_or(defaults.p_in),
            p_out: settings::real(s, "pOut")?.unwrap_or(defaults.p_out),
            max_iterations: settings::integer(s, "maximumIterations")?
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(defaults.max_iterations),
            max_step_halving: settings::integer(s, "maximumStepHalving")?
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(defaults.max_step_halving),
            parameter_convergence: settings::real(s, "parameterConvergence")?
                .unwrap_or(defaults.parameter_convergence),
            log_likelihood_convergence: settings::real(s, "logLikelihoodConvergence")?
                .unwrap_or(defaults.log_likelihood_convergence),
            singularity: settings::real(s, "singularityTolerance")?.unwrap_or(defaults.singularity),
            delta: settings::real(s, "delta")?.unwrap_or(defaults.delta),
            base_category: base,
            name,
        },
        if settings::flag(s, "autoModelFieldSelection")?.unwrap_or(true) {
            None
        } else {
            custom_target
        },
        custom_inputs,
    ))
}

/// The target and inputs a supervised model uses: the custom names, else the Type roles.
fn supervised_fields(
    table: &Table,
    target: Option<&str>,
    inputs: Option<&[String]>,
) -> Result<(usize, Vec<usize>), OperatorError> {
    let model = table.model();
    let position = |name: &str| {
        model.position(name).ok_or_else(|| {
            OperatorError::Data(format!("the model field {name} is not in the input"))
        })
    };
    let target = if let Some(name) = target.filter(|name| !name.is_empty()) {
        position(name)?
    } else {
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
        target
    };
    let inputs = if let Some(names) = inputs.filter(|names| !names.is_empty()) {
        names
            .iter()
            .map(|name| position(name))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        (0..model.fields().len())
            .filter(|&i| matches!(model.fields()[i].role, Role::Input | Role::Both))
            .collect()
    };
    Ok((target, inputs))
}

/// Feature Selection shares the classic supervised field resolution.
pub(super) fn feature_selection_fields(
    table: &Table,
    target: Option<&str>,
    inputs: Option<&[String]>,
) -> Result<(usize, Vec<usize>), OperatorError> {
    supervised_fields(table, target, inputs)
}

/// The fields a clustering model uses: the custom list, else the Type's inputs.
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
            .filter(|&i| matches!(model.fields()[i].role, Role::Input | Role::Both))
            .collect()),
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

/// Linear settings from the extension parameters; unsupported settings fail explicitly.
fn linear_settings(p: &Parameters) -> Result<LinearSettings, OperatorError> {
    for (key, expected) in [
        ("linreg.MaxCategoryNumber", "1000"),
        ("linreg.IsOutRegModel", "false"),
    ] {
        if p.text(key).is_some_and(|v| v != expected) {
            return Err(OperatorError::Settings(format!(
                "Linear custom {key} is not supported yet"
            )));
        }
    }
    let selection = match p.text("linreg.VarSelectionMethod").as_deref() {
        Some("none") => Selection::None,
        None | Some("") | Some("forwardStepwise") => {
            match p.text("linreg.CriteriaForForwardStepwise").as_deref() {
                None | Some("") | Some("AICC") => Selection::Aicc,
                Some("Fstatistics") => Selection::FStatistics,
                Some("AdjustedRSquare") => Selection::AdjustedRSquare,
                Some(other) => {
                    return Err(OperatorError::Settings(format!(
                        "Linear forward criterion {other:?} is not supported yet (ASE needs an overfit prevention set)"
                    )));
                }
            }
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Linear model selection {other:?} is not supported yet"
            )));
        }
    };
    let auto = match p.text("linreg.AutoDataPreparation").as_deref() {
        None => true,
        _ => p.flag("linreg.AutoDataPreparation")?,
    };
    let preparation = if auto {
        Some(linear_adp_settings(p)?)
    } else {
        None
    };
    for (key, required) in [
        ("linreg.ListwiseDeletion", "true"),
        ("linreg.ExcludeUserMissing", "true"),
    ] {
        if p.text(key).is_some_and(|v| v != required) {
            return Err(OperatorError::Settings(format!(
                "Linear {key} other than {required} is not supported yet"
            )));
        }
    }
    if !p.list("linreg.EffectList").is_empty()
        || p.text("linreg.EffectList").is_some_and(|v| !v.is_empty())
    {
        return Err(OperatorError::Settings(
            "Linear custom/interaction effects are not supported yet".into(),
        ));
    }
    let limit = |flag: &str, key: &str, max: usize| -> Result<Option<usize>, OperatorError> {
        if !p.flag(flag)? {
            return Ok(None);
        }
        let value = p
            .text(key)
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| *n > 0 && *n <= max)
            .ok_or_else(|| {
                OperatorError::Settings(format!("Linear {key} must be an integer in 1..={max}"))
            })?;
        Ok(Some(value))
    };
    for weight in [
        "linreg.RegressionWeightField",
        "linreg.FrequencyWeightField",
    ] {
        if p.text(weight).is_some_and(|w| !w.is_empty()) {
            return Err(OperatorError::Settings(format!(
                "Linear {weight} is not supported yet"
            )));
        }
    }
    let intercept = match p.text("linreg.Intercept").as_deref() {
        None | Some("") => true,
        Some(_) => p.flag("linreg.Intercept")?,
    };
    Ok(LinearSettings {
        style: Style::Linear,
        intercept,
        selection,
        preparation,
        pin: p.real("linreg.PIn")?.unwrap_or(0.05),
        pout: p.real("linreg.POut")?.unwrap_or(0.1),
        entry_tolerance: p.real("linreg.EffectEntryTolerance")?.unwrap_or(1e-4),
        max_steps: limit("linreg.UseCustomMaxSteps", "linreg.MaxSteps", 10000)?,
        max_effects: limit("linreg.UseCustomMaxEffects", "linreg.MaxEffects", 256)?,
        name: None,
    })
}

/// ALM uses only the preparation subset observed in the Linear probes. Standalone ADP's
/// saved rescaling/target/selection defaults do not activate those operations in ALM.
fn linear_adp_settings(p: &Parameters) -> Result<dms_algo::adp::AdpSettings, OperatorError> {
    for (key, value) in [
        ("adp.outliersOperation", "coerceToCutoffVal"),
        ("adp.scaleFeatureRescalingMethod", "zScore"),
    ] {
        if p.text(key).is_some_and(|v| v != value) {
            return Err(OperatorError::Settings(format!(
                "Linear {key} other than {value} is not supported yet"
            )));
        }
    }
    for key in [
        "adp.analysisWeight",
        "adp.FrequencyWeightField",
        "adp.targetfield",
    ] {
        if p.text(key).is_some_and(|v| !v.is_empty()) {
            return Err(OperatorError::Settings(format!(
                "Linear {key} is not supported yet"
            )));
        }
    }
    if !p.list("adp.inputfields").is_empty() {
        return Err(OperatorError::Settings(
            "Linear custom ADP inputs are not supported yet".into(),
        ));
    }
    // Reject custom settings for operations whose defaults ALM overrides, or which the
    // current preparation subset cannot apply. Dates/missing/recasting fail on affected data.
    for (key, expected) in [
        ("adp.trimBlanks", true),
        ("adp.checkFeatureMeasureLevel", true),
        ("adp.checkTargetMeasureLevel", true),
        ("adp.featureSelection", true),
        ("adp.featureConstruction", true),
        ("adp.supervisedBinning", true),
        ("adp.rescaleScaleTargetUsingBoxCox", true),
        ("adp.detectTargetOutliers", true),
        ("adp.reorderNominalCatsPriorToMerging", true),
        ("adp.reorderNominalTargetCategories", true),
        ("adp.excludeAlmostConstantCatFeature", false),
        ("adp.excludeFeatureIfHasManyUniqueCatogories", true),
        (
            "adp.excludeFeatureIfHasHigherPercentageOfMissingValue",
            true,
        ),
        ("adp.replaceScaleFeaturesMissingValue", true),
        ("adp.replaceNominalFeaturesMissingValue", true),
        ("adp.replaceOrdinalFeaturesMissingValue", true),
        ("adp.replaceScaleTargetMissingValue", true),
        ("adp.replaceNominalTargetMissingValue", true),
        ("adp.replaceOrdinalTargetMissingValue", true),
        ("adp.unsupervisedMergeNominal", true),
        ("adp.unsupervisedMergeOrdinal", true),
    ] {
        if p.text(key).is_some() && p.flag(key)? != expected {
            return Err(OperatorError::Settings(format!(
                "Linear custom {key} is not supported yet"
            )));
        }
    }
    for (key, expected) in [
        ("adp.maxPercentageOfMissingvalue", 50.0),
        ("adp.maxNOfUniqueCategories", 100.0),
        ("adp.scaleToOrdinalValuesNumThreshold", 5.0),
        ("adp.ordinalToScaleCategoriesNumThreshold", 10.0),
        ("adp.catFeatureScreenThreshold", 95.0),
        ("adp.zScore_mean", 0.0),
        ("adp.zScore_SD", 1.0),
        ("adp.minMax_min", 0.0),
        ("adp.minMax_max", 100.0),
        ("adp.targetMean", 0.0),
        ("adp.targetSD", 1.0),
        ("adp.featureSelectionPValue", 0.05),
        ("adp.homogeneousSubsetsPValue", 0.05),
        ("adp.supervisedMergingPValue", 0.05),
        ("adp.unsupervisedMergingMinPercentageOfCases", 10.0),
        ("adp.adjustThreshold", 0.0),
    ] {
        if p.real(key)?.is_some_and(|v| v != expected) {
            return Err(OperatorError::Settings(format!(
                "Linear custom {key} is not supported yet"
            )));
        }
    }
    let defaults = dms_algo::adp::AdpSettings::default();
    Ok(dms_algo::adp::AdpSettings {
        outliers: if p.text("adp.detectFeatureOutliers").is_some() {
            p.flag("adp.detectFeatureOutliers")?
        } else {
            defaults.outliers
        },
        cutoff: p.real("adp.outlierIdentCutoff")?.unwrap_or(defaults.cutoff),
        merge: if p.text("adp.supervisedMerging").is_some() {
            p.flag("adp.supervisedMerging")?
        } else {
            defaults.merge
        },
        // ALM's threshold is .1, despite the standalone ADP default .05 in its attachment.
        merge_alpha: defaults.merge_alpha,
        suffix: p.text("adp.nameExtForInputs").unwrap_or(defaults.suffix),
    })
}

/// Regression settings from the node's XML: the `enter` method only.
fn regression_settings(node: &StreamNode) -> Result<RegressionSpec, OperatorError> {
    let s = node.settings.as_ref();
    match settings::text(s, "method").as_deref().map(str::trim) {
        None | Some("") | Some("enter") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Regression method {other:?} is not supported yet"
            )));
        }
    }
    let auto_fields = settings::flag(s, "autoModelFieldSelection")?.unwrap_or(true);
    let target = (!auto_fields)
        .then(|| settings::text(s, "modelTargetFieldSelection"))
        .flatten()
        .filter(|name| !name.trim().is_empty());
    let inputs = (!auto_fields)
        .then(|| {
            settings::children(s, "modelInputFieldSelection", "field")
                .into_iter()
                .map(|f| f.text().trim().to_string())
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|names| !names.is_empty());
    let name = (!settings::flag(s, "autoModelName")?.unwrap_or(true))
        .then(|| settings::text(s, "modelName"))
        .flatten()
        .filter(|n| !n.trim().is_empty());
    Ok((
        LinearSettings {
            style: Style::Regression,
            intercept: settings::flag(s, "includeConstant")?.unwrap_or(true),
            name,
            ..LinearSettings::default()
        },
        target,
        inputs,
    ))
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

#[cfg(test)]
mod linear_tests {
    use super::*;

    fn parameters(entries: &[(&str, &str)]) -> Parameters {
        Parameters {
            values: entries
                .iter()
                .map(|(k, v)| (k.to_string(), Parameter::Value(v.to_string())))
                .collect(),
        }
    }

    #[test]
    fn linear_defaults_and_custom_selection_limits() {
        let s = linear_settings(&parameters(&[])).unwrap();
        assert_eq!(s.selection, Selection::Aicc);
        assert_eq!(s.preparation.unwrap().merge_alpha, 0.1);
        let s = linear_settings(&parameters(&[
            ("linreg.AutoDataPreparation", "false"),
            ("linreg.CriteriaForForwardStepwise", "Fstatistics"),
            ("linreg.PIn", "0.02"),
            ("linreg.POut", "0.2"),
            ("linreg.UseCustomMaxSteps", "true"),
            ("linreg.MaxSteps", "4"),
            ("linreg.UseCustomMaxEffects", "true"),
            ("linreg.MaxEffects", "2"),
        ]))
        .unwrap();
        assert_eq!(s.selection, Selection::FStatistics);
        assert_eq!((s.pin, s.pout), (0.02, 0.2));
        assert_eq!((s.max_steps, s.max_effects), (Some(4), Some(2)));
        assert!(s.preparation.is_none());
        assert_eq!(
            linear_settings(&parameters(&[(
                "linreg.CriteriaForForwardStepwise",
                "AdjustedRSquare"
            )]))
            .unwrap()
            .selection,
            Selection::AdjustedRSquare
        );
    }

    #[test]
    fn unimplemented_and_malformed_linear_settings_fail() {
        for entries in [
            vec![("linreg.VarSelectionMethod", "bestSubsets")],
            vec![("linreg.CriteriaForForwardStepwise", "ASE")],
            vec![("linreg.AutoDataPreparation", "yes")],
            vec![("adp.supervisedMergingPValue", "0.2")],
            vec![
                ("linreg.UseCustomMaxSteps", "true"),
                ("linreg.MaxSteps", "18446744073709551616"),
            ],
            vec![
                ("linreg.UseCustomMaxEffects", "true"),
                ("linreg.MaxEffects", "-1"),
            ],
        ] {
            assert!(
                linear_settings(&parameters(&entries)).is_err(),
                "{entries:?}"
            );
        }
    }
}
