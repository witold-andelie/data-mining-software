//! Model nuggets (`ApplyCartRuleNode`, `ApplyChaidRuleNode`, `ApplyLogisticNode`,
//! `ApplyKMeansNode`, `ApplyTwoStepNode`, `ApplyRegressionNode`,
//! `LinearRegressionModel!spss.alm-extension`,
//! `neuralnetmodel!spss.neuralnet-extension`): score every record with the model and append
//! Modeler's score fields (`docs/models.md`).
//!
//! The model is the one the linked build node trained in this evaluation, else the PMML the
//! nugget saved. Classic nuggets keep PMML at `properties/model@pmml`. Clementine 11 C&R Tree
//! nuggets keep an inline `RuleTreeModel` as `properties/model` with no PMML container
//! (`ibm-demos/telco_dataaudit`); [`dms_pmml::Pmml::parse_rule_tree`] maps it onto the same
//! `TreeModel` the PMML path scores. Extension-framework nuggets keep a `PMML` container. Neural
//! Net stores that member as `SCF\x01` zlib-wrapped PMML (`corpus/models_ml/nn_fixed_a.str`,
//! `data/0002.dat`); [`dms_strfmt::read_model_xml`] unwraps it. Training is not implemented, so
//! the nugget scores this saved model.
//! Score fields, from the oracle exports of the team streams, `models/linear_enter`,
//! `models/regression_enter`, `corpus/models_ml/nn_*`, and `crates/pmml/tests/oracle.rs`:
//!
//! | nugget | fields |
//! |---|---|
//! | C&R Tree, CHAID | `$R-<target>` prediction; `$RC-<target>` confidence (`calculateConfidences`); `$RRP-<target>` raw propensity, the probability of the second (true) category (`calculateRawPropensities`) |
//! | Logistic | `$L-<target>` prediction; `$LP-<target>` confidence (`calculateConfidences`); `$LRP-<target>` raw propensity (`calculateRawPropensities`); `$LP-<category>` for every target category when enabled (below) |
//! | Linear | `$L-<target>` prediction (`models/linear_enter`, `team/linear_regression`) |
//! | GLE | regression `$G-<target>`; binomial logit `$L-<target>` `$LC-<target>`; binomial identity/log `$G-<target>` `$GC-<target>` (`oracle/gle`) |
//! | Regression | `$E-<target>` prediction (`models/regression_enter`) |
//! | K-Means | `$KM-<model name>`: `cluster-<cluster name>`; `$KMD-<model name>`: the Euclidean distance to that center, when the nugget's `buildSettings` set `generateDistanceField` |
//! | TwoStep | `$T-<model name>`: the cluster name (`cluster-<n>`), by the log-likelihood distance; an integer with numeric labels (`buildSettings` `clusterLabelType` `NUMBER`) |
//! | Apriori | `$A-<model>-k` consequent, `$AC-<model>-k` the ranking criterion's value (confidence by default), `$A-Rule_ID-k` rule id, for k = 1..=`associationMaxPredictions` (`apriori_default`: 3). The nugget settings (`checkPredictions`, `ruleCriteria`, `repeatPredictions`, `ignoreUnmatchedItems`) are fixed by probe `apriori_eval`, see `dms_pmml::ScoringOptions`; a builder-linked nugget scores with the defaults. Transactional models (`usageType=group`) score each row on its prefix up to that row (`trans_base` grid 104: `eggs` milk, `jam` milk+butter). Nominal and false-flag baskets score `field=value` items (`nominal_set` blue/circle predicts red and square). GAP: `maxLengthSet` other than 10. |
//! | Neural Net | `$N-<target>` (`scoring.isPrediction`, default true). Categorical targets also `$NC-<target>` when `scoring.isConfidence` (default true): `onProbability` is the winning probability (`nn_default_a`), `onIncrease` is that minus the second (`nn_fixed_a` confidence-increase, row 10001 is 0.131866732354273). `$NP-<category>` in `DataField` order, the first `scoring.maxCategories` when `scoring.isProbabilityCategory` (`nn_fixed_a`: all three, or only `$NP-A` when the cap is 1; a missing cap emits all). Then `$RP-<target>` and `$AP-<target>` (`nn_partition`). Continuous targets emit none of those (`nn_regression`, `nn_rbf`). Targets follow `NeuralOutput` order (`nn_mixed_targets`: `$N-r,$N-y,$NC-y`). |
//! | Time Series | `$FutureFlag` after the period field; per target `$TS-`, `$TSLCI-`, `$TSUCI-` (`ts_arima`, `ts_expert`, `ts_simple`, `ts_holt`). Training is a GAP; the nugget scores the saved ARIMA or exponential smoothing model. |
//!
//! Neural score fields come from the nugget properties attachment (`Parameter` names above),
//! not from scripting names. With no attachment the defaults are prediction, confidence,
//! `onProbability`, and no category probabilities or propensity. `confidenceBasedOn` other
//! than `onProbability` or `onIncrease` is an error. `scoring.isPropensity` with no
//! `adjustedPropensity` extension is an error. Raw propensity is `P(flagTypeTrueValue)`
//! (`nn_partition` true value `"1"`).
//! GAP: two categorical targets at once have no oracle; their category fields would be
//! `$NP-<target>-<category>`. GAP: raw propensity with no flag extension uses the second
//! category (no non-flag neural oracle).
//!
//! `$LP-<category>` is a real field, one per target category, in `Prediction::probabilities`
//! order (the target `DataField` values). The value is that category's probability; a missing
//! model input is an empty prediction, so the cell is null, as are `$L-` and `$LP-`. The nugget
//! writes them when `appendProbabilities` is true (`models/logreg_multinomial`: `$LP-drugA` …
//! `$LP-drugY` after `$LP-Drug`). The binomial procedure writes them even when the flag is false
//! (`models/logreg_binomial`: `logisticProcedure` `Binomial`, `appendProbabilities` false,
//! `$LP-1` and `$LP-2`). A multinomial nugget with the flag false does not
//! (`team/logistic_regression`, `team/model_comparison`: `$L-`, `$LP-`, `$LRP-` only). None of
//! those oracles has a missing logistic input.
//!
//! When the input already has a field of that name (another model on the same target), every
//! score field of the nugget takes the first free index of the prediction field: `$R1-`,
//! `$RC1-`, `$RRP1-` (`team/model_comparison`). Category probabilities use that same index
//! (`$LP1-<category>`).
//! GAP: K-Means cluster distance (`showClusterProximity`) and TwoStep outlier handling
//! (`excludeOutliers`, a noise cluster) are rejected until an oracle covers them.

use super::extension_params::{self, flag};
use super::field_util::{Column, with_columns};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::NodeOutput;
use crate::settings;
use dms_pmml::{
    DataType, NeuralNetworkModel, NeuralOutputKind, NeuralTarget, Pmml, Prediction, Usage,
    Value as PmmlValue,
};
use dms_stream::StreamNode;
use dms_types::{Measurement, Storage, Table, Value};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Decision trees and rule sets: `$R-`, `$RC-`, `$RRP-`.
    Rule,
    /// Analytic Engine Random Trees and Tree-AS (`models_as`).
    RandomTrees,
    TreeAs,
    /// Analytic Engine linear SVM: `$L-`, `$LC-` (classification only).
    LinearSvm,
    /// Logistic regression: `$L-`, `$LP-`, `$LRP-`, and `$LP-<category>` when enabled.
    Logistic,
    /// GLE: `$G-<target>` for regression and binomial identity/log; `$L-`/`$LC-` for logit.
    Gle,
    /// Linear (ALM) nugget: `$L-<target>` only.
    Linear,
    /// Classic Regression nugget: `$E-<target>` only.
    Regression,
    /// K-Means: `$KM-<model name>`.
    KMeans,
    /// TwoStep: `$T-<model name>`.
    TwoStep,
    /// Neural Net: `$N-`, `$NC-`, `$NP-`, `$RP-`, `$AP-`.
    Neural,
    /// PCA/Factor: `$F-<model name>-<k>`, one real per factor (`models_next/factor_*`).
    Factor,
    /// Anomaly Detection: `$O-Anomaly`, `$O-AnomalyIndex`, `$O-PeerGroup`, then `$O-Field-<k>`
    /// and `$O-FieldImpact-<k>` for the `numAnomalyFields` largest contributions
    /// (`models_next/anomaly_default`).
    Anomaly,
    /// Kohonen: `$KX-<model name>`, `$KY-<model name>` (the winning unit's map position) and,
    /// with `appendClusterLabel`, `$KXY-<model name>` (its cluster name, `X=0, Y=2`;
    /// `models_next/kohonen_*`).
    Kohonen,
    /// Decision List: `$D-<target>` (the first matching segment's score, null for a disabled
    /// segment), `$DP-<target>` (its share of correct training records, `displayProbabilityField`)
    /// and `$DI-<target>` (its rule id, `displayRuleIdField`; `ibm-demos/pm_decisionlist`).
    DecisionList,
    /// Apriori: `$A-`, `$AC-`, `$A-Rule_ID-`.
    Apriori,
}

fn is_gle_nugget(id: &str) -> bool {
    id == "spss.gle!gle_applier"
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    if matches!(
        node.type_identifier().as_str(),
        "ApplyAutoClassifierNode" | "ApplyBinaryClassifierNode" | "ApplyNumericPredictorNode"
    ) {
        return super::auto_models::apply(node);
    }
    if matches!(
        node.type_identifier().as_str(),
        "ApplyFeatureSelectionNode" | "FeatureSelectionModelNode"
    ) {
        return super::feature_selection::apply(node);
    }
    if node.type_identifier() == "ApplyAutoClusterNode" {
        return super::auto_cluster::apply(node);
    }
    if node.type_identifier() == "ApplySequenceNode" {
        return super::sequence::apply(node);
    }
    if matches!(
        node.type_identifier().as_str(),
        "ApplyC5RuleNode" | "ApplyC5SplitModelNode"
    ) {
        return super::c50::apply(node);
    }
    if node.type_identifier() == "spss.ts!ts_applier" {
        return super::time_series::apply(node);
    }
    let s = node.settings.as_ref();
    let type_id = node.type_identifier();
    if is_gle_nugget(&type_id) {
        match settings::text(s, "_GenerateSQL").as_deref().map(str::trim) {
            None | Some("" | "false" | "udf" | "native") => {}
            Some(other) => {
                return Err(OperatorError::Settings(format!(
                    "GAP: GLE _GenerateSQL={other:?}"
                )));
            }
        }
        let kind = Kind::Gle;
        let model_name = settings::text(s, "modelName")
            .filter(|name| !name.trim().is_empty())
            .or_else(|| node.name.clone())
            .unwrap_or_else(|| "GLE".to_string());
        let model_el = settings::element(s, "model");
        let pmml_member = model_el
            .and_then(|model| model.attribute("pmml"))
            .map(str::to_string)
            .or_else(|| {
                node.containers
                    .iter()
                    .find(|container| {
                        matches!(container.name.as_deref(), Some("PMML" | "ANOMALY_XML"))
                    })
                    .map(|container| container.member.clone())
            });
        return Ok(Box::new(Apply {
            kind,
            pmml_member,
            confidences: true,
            raw_propensities: false,
            all_probabilities: false,
            distance: false,
            numeric_labels: false,
            decision_fields: (false, false),
            tree_id: false,
            cluster_label: false,
            model_name,
            properties: node.properties_attachment.clone(),
            anomaly: None,
            apriori: AprioriOptions {
                max_predictions: 0,
                ..APRIORI_DEFAULTS
            },
            inline_model_xml: None,
        }));
    }
    let kind = match type_id.as_str() {
        "ApplyCartRuleNode" | "ApplyChaidRuleNode" | "ApplyQuestRuleNode" | "ApplyC5RuleNode" => {
            Kind::Rule
        }
        "ApplyLogisticNode" => Kind::Logistic,
        "spss.randomTrees!applyRandomTrees" => Kind::RandomTrees,
        "spss.treeEngine!applyTree" => Kind::TreeAs,
        "spss.linearEngine!applyLinearEngine" => Kind::Linear,
        "spss.linearSVM!applyLinearSVM" => Kind::LinearSvm,
        "LinearRegressionModel!spss.alm-extension" | "ApplyLinearRegressionSplitModelNode" => {
            Kind::Linear
        }
        "ApplyRegressionNode" => Kind::Regression,
        "ApplyKMeansNode" => Kind::KMeans,
        "ApplyTwoStepNode" => Kind::TwoStep,
        "ApplyAprioriNode" | "ApplyCarmaNode" => Kind::Apriori,
        "neuralnetmodel!spss.neuralnet-extension"
        | "ApplyNetNode"
        | "ApplyNeuralNetSplitModelNode" => Kind::Neural,
        "ApplyPCANode" => Kind::Factor,
        "ApplyAnomalyNode" => Kind::Anomaly,
        "ApplyKohonenNode" => Kind::Kohonen,
        "ApplyDecisionListNode" => Kind::DecisionList,
        other => {
            return Err(OperatorError::Settings(format!(
                "model nugget {other} is not supported yet"
            )));
        }
    };
    let analytic = matches!(kind, Kind::RandomTrees | Kind::TreeAs | Kind::LinearSvm)
        || node.type_identifier() == "spss.linearEngine!applyLinearEngine";
    if analytic {
        validate_analytic_settings(s)?;
    }
    if kind == Kind::DecisionList
        && (settings::flag(s, "calculateRawPropensities")?.unwrap_or(false)
            || settings::flag(s, "calculateAdjustedPropensities")?.unwrap_or(false))
    {
        return Err(OperatorError::Settings(
            "Decision List propensities are not supported yet".into(),
        ));
    }
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
    let anomaly = if kind == Kind::Anomaly {
        match settings::text(s, "anomalyScoreMethod")
            .as_deref()
            .map(str::trim)
        {
            None | Some("flagAndScore") => {}
            Some(other) => {
                return Err(OperatorError::Settings(format!(
                    "anomaly scoring method {other} is not supported yet"
                )));
            }
        }
        if settings::flag(s, "discardRecords")?.unwrap_or(false) {
            return Err(OperatorError::Settings(
                "discarding records (discardRecords) is not supported yet".into(),
            ));
        }
        Some(AnomalyOptions {
            cutoff: settings::real(s, "model/cutoffValue")?,
            fields: usize::try_from(settings::integer(s, "numAnomalyFields")?.unwrap_or(3))
                .map_err(|_| OperatorError::Settings("numAnomalyFields is negative".into()))?,
        })
    } else {
        None
    };
    let apriori = if kind == Kind::Apriori {
        apriori_predictions(s)?
    } else {
        AprioriOptions {
            max_predictions: 0,
            ..APRIORI_DEFAULTS
        }
    };
    let model_name = settings::text(s, "modelName")
        .filter(|name| !name.trim().is_empty())
        .or_else(|| node.name.clone())
        .unwrap_or_else(|| match kind {
            Kind::TwoStep => "TwoStep".to_string(),
            _ => "K-Means".to_string(),
        });
    let model_el = settings::element(s, "model");
    let pmml_member = model_el
        .and_then(|model| model.attribute("pmml"))
        .map(str::to_string)
        .or_else(|| {
            node.containers
                .iter()
                .find(|container| matches!(container.name.as_deref(), Some("PMML" | "ANOMALY_XML")))
                .map(|container| container.member.clone())
        });
    let inline_model_xml = if pmml_member.is_none() {
        model_el.and_then(rule_tree_xml)
    } else {
        None
    };
    Ok(Box::new(Apply {
        kind,
        pmml_member,
        confidences: settings::flag(s, if analytic { "_calculate_conf" } else { "calculateConfidences" })?.unwrap_or(true)
            // `rt_noconf`, `rt_batch`: oracle runs the builder before exporting
            // terminal inputs. Its replacement-linked nugget regains the default
            // confidence field; detached copies retain `_calculate_conf=false`.
            // Reproduce this schema while scoring the saved model (no training).
            || (kind == Kind::RandomTrees
                && settings::text(s, "refreshLinkState").as_deref() == Some("TurnOnReplaceLink")
                && settings::text(s, "repalcebuilderNodeId").is_some_and(|id| !id.trim().is_empty())),
        raw_propensities: settings::flag(s, "calculateRawPropensities")?.unwrap_or(false),
        // `$LP-<category>`: the flag, or always for the binomial procedure
        // (`models/logreg_binomial` leaves `appendProbabilities` false and still writes them).
        all_probabilities: kind == Kind::Logistic
            && (settings::flag(s, "appendProbabilities")?.unwrap_or(false)
                || settings::text(s, "logisticProcedure")
                    .is_some_and(|procedure| procedure.trim() == "Binomial")),
        distance: kind == Kind::KMeans
            && build_setting(s, "generateDistanceField").as_deref() == Some("true"),
        numeric_labels: kind == Kind::TwoStep
            && build_setting(s, "clusterLabelType").as_deref() == Some("NUMBER"),
        decision_fields: (
            settings::flag(s, "displayProbabilityField")?.unwrap_or(true),
            settings::flag(s, "displayRuleIdField")?.unwrap_or(true),
        ),
        tree_id: kind == Kind::TreeAs && settings::flag(s, "_display_rule_id")?.unwrap_or(true),
        cluster_label: kind == Kind::Kohonen
            && settings::flag(s, "appendClusterLabel")?.unwrap_or(true),
        model_name,
        properties: node.properties_attachment.clone(),
        anomaly,
        apriori,
        inline_model_xml,
    }))
}

fn rule_tree_xml(model: &dms_strfmt::fastinfoset::Element) -> Option<String> {
    let class = model.attribute("class")?;
    if class.rsplit('.').next() != Some("RuleTreeModel") {
        return None;
    }
    Some(model.to_xml())
}

/// The nugget's own Anomaly Detection settings.
struct AnomalyOptions {
    /// `model/cutoffValue`: the saved model's anomaly cutoff.
    cutoff: Option<f64>,
    /// `numAnomalyFields`: how many contributing fields each record reports.
    fields: usize,
}

/// The Apriori nugget settings that change scores (see `dms_pmml::ScoringOptions`).
type AprioriOptions = dms_pmml::ScoringOptions;

/// The values `apriori_default` stores: 3 predictions, no repeats, `notInBasket`, criterion
/// confidence, unmatched items ignored.
const APRIORI_DEFAULTS: AprioriOptions = dms_pmml::ScoringOptions {
    max_predictions: 3,
    allow_repeats: false,
    check: dms_pmml::BasketCheck::NotInBasket,
    criterion: dms_pmml::RuleCriterion::Confidence,
    exact_antecedent: false,
};

/// The nugget's `associationMaxPredictions`, `repeatPredictions`, `checkPredictions`,
/// `ruleCriteria` and `ignoreUnmatchedItems` (probe `apriori_eval`, see `dms_pmml`).
///
/// A nugget that the builder replaces (`refreshLinkState` `TurnOnReplaceLink` with a builder
/// id) scores with the defaults whatever it stores: in `apriori_more/score_*` and
/// `apriori_eval/score_matrix` such a nugget carrying `noCheck`, repeats, criterion support,
/// one prediction or `ignoreUnmatchedItems` false scored every row exactly like the default
/// nugget, whether its table ran before or after the builder, while unlinked copies applied
/// each setting. Random Trees' confidence field follows the same rule.
fn apriori_predictions(
    s: Option<&dms_strfmt::fastinfoset::Element>,
) -> Result<AprioriOptions, OperatorError> {
    if settings::text(s, "refreshLinkState").as_deref() == Some("TurnOnReplaceLink")
        && settings::text(s, "repalcebuilderNodeId").is_some_and(|id| !id.trim().is_empty())
    {
        return Ok(APRIORI_DEFAULTS);
    }
    let count = settings::integer(s, "associationMaxPredictions")?.unwrap_or(3);
    if !(1..=10_000).contains(&count) {
        return Err(OperatorError::Settings(format!(
            "Apriori associationMaxPredictions must be from 1 to 10000, found {count}"
        )));
    }
    let allow_repeats = settings::flag(s, "repeatPredictions")?.unwrap_or(false);
    let check = match settings::text(s, "checkPredictions")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("notInBasket") => dms_pmml::BasketCheck::NotInBasket,
        Some("inBasket") => dms_pmml::BasketCheck::InBasket,
        Some("noCheck") => dms_pmml::BasketCheck::NoCheck,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Apriori checkPredictions {other:?} is not a known value"
            )));
        }
    };
    let criterion = match settings::text(s, "ruleCriteria").as_deref().map(str::trim) {
        None | Some("") | Some("confidence") => dms_pmml::RuleCriterion::Confidence,
        Some("support") => dms_pmml::RuleCriterion::Support,
        Some("ruleSupport") => dms_pmml::RuleCriterion::RuleSupport,
        Some("lift") => dms_pmml::RuleCriterion::Lift,
        Some("deployability") => dms_pmml::RuleCriterion::Deployability,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Apriori ruleCriteria {other:?} is not a known value"
            )));
        }
    };
    let exact_antecedent = !settings::flag(s, "ignoreUnmatchedItems")?.unwrap_or(true);
    match settings::integer(s, "maxLengthSet")? {
        None | Some(10) => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Apriori maxLengthSet {other} is not supported yet (GAP: the oracles use 10)"
            )));
        }
    }
    let max_predictions = usize::try_from(count).map_err(|_| {
        OperatorError::Settings(format!(
            "Apriori associationMaxPredictions must be from 1 to 10000, found {count}"
        ))
    })?;
    Ok(AprioriOptions {
        max_predictions,
        allow_repeats,
        check,
        criterion,
        exact_antecedent,
    })
}

struct Apply {
    kind: Kind,
    /// The attachment holding the saved PMML.
    pmml_member: Option<String>,
    confidences: bool,
    raw_propensities: bool,
    /// Logistic: one real `$LP-<category>` field per target category.
    all_probabilities: bool,
    /// K-Means: also the distance to the cluster center (`$KMD-`).
    distance: bool,
    /// TwoStep: clusters are numbers (`$T-` is an integer field).
    numeric_labels: bool,
    /// Kohonen: also the cluster name (`$KXY-`).
    cluster_label: bool,
    /// Decision List: the probability and rule id fields.
    decision_fields: (bool, bool),
    tree_id: bool,
    model_name: String,
    /// Extension properties attachment (`scoring.isPrediction`, ...). Neural Net only.
    properties: Option<String>,
    anomaly: Option<AnomalyOptions>,
    /// Apriori scoring options (`associationMaxPredictions` etc.; defaults for other nuggets).
    apriori: AprioriOptions,
    /// Clementine 11 inline `RuleTreeModel` XML when the nugget saved no PMML member.
    inline_model_xml: Option<String>,
}

/// `models_as`: native and the saved default (`false`) produce identical score fields.
/// Propensity scoring has no successful oracle in these cases; never silently ignore it.
fn validate_analytic_settings(
    s: Option<&dms_strfmt::fastinfoset::Element>,
) -> Result<(), OperatorError> {
    for name in [
        "calculateRawPropensities",
        "calculateAdjustedPropensities",
        "_calculateRawProp",
    ] {
        if settings::flag(s, name)
            .map_err(|e| OperatorError::Settings(format!("GAP: AS {e:?}")))?
            .unwrap_or(false)
        {
            return Err(OperatorError::Settings(format!(
                "GAP: AS {name}=true has no scoring oracle"
            )));
        }
    }
    for name in ["_calculate_conf", "_display_rule_id"] {
        settings::flag(s, name).map_err(|e| OperatorError::Settings(format!("GAP: AS {e:?}")))?;
    }
    match settings::text(s, "_GenerateSQL").as_deref().map(str::trim) {
        None | Some("false" | "native") => Ok(()),
        Some(other) => Err(OperatorError::Settings(format!(
            "GAP: AS _GenerateSQL={other:?} has no scoring oracle"
        ))),
    }
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
    fn saved_model_xml(&self, context: &ExecContext) -> Result<String, OperatorError> {
        let member = self.pmml_member.as_deref().ok_or_else(|| {
            OperatorError::Settings(
                "the nugget saved no PMML model and its build node did not run".into(),
            )
        })?;
        saved_model_xml(member, context)
    }

    /// Apriori nugget columns. Tabular baskets are one row each (`apriori_default`).
    /// Transactional baskets score each row on its prefix up to that row
    /// (`trans_base` grid basket 104: `eggs` predicts `milk`, `jam` predicts `milk`
    /// and `butter`). Tabular and nominal baskets score the whole row.
    /// GAP: a second Apriori nugget on the same fields uses `$A1-` / `$AC1-` / `$A1-Rule_ID-`.
    /// No oracle shows that index.
    fn score_apriori(&self, input: &Table, pmml: &Pmml) -> Result<Table, OperatorError> {
        let dms_pmml::Model::Association(model) = &pmml.model else {
            return Err(OperatorError::Data(
                "the nugget's model is not an AssociationModel".into(),
            ));
        };
        let model_name = model
            .header
            .name
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.model_name.clone());
        let taken = |name: &str| input.model().position(name).is_some();
        let index = (0..MAX_CATEGORY_FIELDS)
            .map(|n| if n == 0 { String::new() } else { n.to_string() })
            .find(|suffix| {
                !taken(&format!("$A{suffix}-{model_name}-1"))
                    && !taken(&format!("$AC{suffix}-{model_name}-1"))
                    && !taken(&format!("$A{suffix}-Rule_ID-1"))
            })
            .ok_or_else(|| OperatorError::Data("no free $A- score field index".into()))?;
        let group = model
            .header
            .mining_schema
            .iter()
            .find(|field| field.usage == Usage::Group);
        let hits = if let Some(group) = group {
            let id_at = input.model().position(&group.name).ok_or_else(|| {
                OperatorError::Data(format!("Apriori needs transaction id {:?}", group.name))
            })?;
            let content = model
                .header
                .mining_schema
                .iter()
                .find(|field| field.usage == Usage::Active)
                .ok_or_else(|| {
                    OperatorError::Data("Apriori transactional model names no content field".into())
                })?;
            let content_at = input.model().position(&content.name).ok_or_else(|| {
                OperatorError::Data(format!("Apriori needs content field {:?}", content.name))
            })?;
            transactional_hits(model, input, id_at, content_at, &self.apriori)?
        } else {
            flag_hits(pmml, model, input, &self.apriori)?
        };
        let mut columns = Vec::new();
        for slot in 1..=self.apriori.max_predictions {
            let prediction = hits
                .iter()
                .map(|row| match row.get(slot - 1) {
                    Some(hit) => Value::string(hit.item.clone()),
                    None => Value::Null,
                })
                .collect();
            let confidence = hits
                .iter()
                .map(|row| match row.get(slot - 1) {
                    Some(hit) => Value::real(hit.confidence),
                    None => Value::Null,
                })
                .collect();
            let rule_id = hits
                .iter()
                .map(|row| match row.get(slot - 1) {
                    Some(hit) => Value::Integer(hit.rule_id),
                    None => Value::Null,
                })
                .collect();
            columns.push(column(
                &format!("$A{index}-{model_name}-{slot}"),
                Storage::String,
                prediction,
            ));
            columns.push(column(
                &format!("$AC{index}-{model_name}-{slot}"),
                Storage::Real,
                confidence,
            ));
            columns.push(column(
                &format!("$A{index}-Rule_ID-{slot}"),
                Storage::Integer,
                rule_id,
            ));
        }
        with_columns(input, columns)
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
        let (pmml, valid_intervals) = match model {
            Some(model) => (model, Vec::new()),
            None => {
                if let Some(xml) = &self.inline_model_xml {
                    (
                        Arc::new(
                            Pmml::parse_rule_tree(xml)
                                .map_err(|error| OperatorError::Data(format!("model: {error}")))?,
                        ),
                        Vec::new(),
                    )
                } else {
                    let xml = self.saved_model_xml(context)?;
                    let intervals = if self.kind == Kind::Linear {
                        alm_valid_intervals(&xml, input.model())?
                    } else {
                        Vec::new()
                    };
                    (
                        Arc::new(
                            Pmml::parse(&xml)
                                .map_err(|error| OperatorError::Data(format!("model: {error}")))?,
                        ),
                        intervals,
                    )
                }
            }
        };
        if self.kind == Kind::Apriori {
            return self.score_apriori(input, &pmml);
        }
        let names: Vec<&str> = input
            .model()
            .fields()
            .iter()
            .map(|field| field.name.as_str())
            .collect();
        let scorer = pmml
            .bind(&names)
            .map_err(|error| OperatorError::Data(format!("model: {error}")))?;
        if self.kind == Kind::Factor {
            let scores = input
                .rows()
                .iter()
                .map(|row| {
                    let record: Vec<PmmlValue> = row.iter().map(to_pmml).collect();
                    scorer
                        .score_factors(&record)
                        .map_err(|error| OperatorError::Data(format!("model: {error}")))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let count = scores.first().map_or(0, Vec::len);
            // Score fields carry the model's own name (`factor_varimax`'s grid nugget, labelled
            // "Grid scores", still writes `$F-models_next-1`).
            let model_name = match &pmml.model {
                dms_pmml::Model::Mining(mining) => mining.header.name.clone(),
                _ => None,
            }
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| self.model_name.clone());
            let columns = (0..count)
                .map(|k| {
                    let values = scores
                        .iter()
                        .map(|row| match row.get(k) {
                            Some(PmmlValue::Number(x)) => Value::Real(*x),
                            _ => Value::Null,
                        })
                        .collect();
                    column(
                        &format!("$F-{}-{}", model_name, k + 1),
                        Storage::Real,
                        values,
                    )
                })
                .collect();
            return with_columns(input, columns);
        }
        if let Some(options) = &self.anomaly {
            let scores = input
                .rows()
                .iter()
                .map(|row| {
                    let record: Vec<PmmlValue> = row.iter().map(to_pmml).collect();
                    scorer
                        .score_anomaly(&record)
                        .map_err(|error| OperatorError::Data(format!("model: {error}")))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let cutoff = match &pmml.model {
                dms_pmml::Model::Clustering(model) => model.anomaly_cutoff,
                _ => None,
            }
            .or(options.cutoff)
            .ok_or_else(|| OperatorError::Data("the anomaly model has no cutoff".into()))?;
            return with_columns(input, anomaly_columns(&scores, cutoff, options.fields));
        }
        if self.kind == Kind::DecisionList {
            let dms_pmml::Model::RuleSet(model) = &pmml.model else {
                return Err(OperatorError::Data(
                    "a Decision List needs a rule set".into(),
                ));
            };
            let hits = input
                .rows()
                .iter()
                .map(|row| {
                    let record: Vec<PmmlValue> = row.iter().map(to_pmml).collect();
                    scorer
                        .first_rule(&record)
                        .map(|hit| hit.and_then(|index| model.rules.get(index)))
                        .map_err(|error| OperatorError::Data(format!("model: {error}")))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let target = target_name(&pmml)?;
            let storage = target_storage(&pmml, &target);
            let mut columns = vec![column(
                &format!("$D-{target}"),
                storage,
                hits.iter()
                    .map(|hit| match hit {
                        Some(rule) if !rule.disabled => {
                            typed(&PmmlValue::Text(rule.score.clone()), storage)
                        }
                        _ => Value::Null,
                    })
                    .collect(),
            )];
            if self.decision_fields.0 {
                columns.push(column(
                    &format!("$DP-{target}"),
                    Storage::Real,
                    hits.iter()
                        .map(|hit| match hit {
                            Some(rule) => match (rule.correct, rule.record_count) {
                                (Some(correct), Some(total)) if total > 0.0 => {
                                    Value::Real(correct / total)
                                }
                                _ => Value::Null,
                            },
                            None => Value::Null,
                        })
                        .collect(),
                ));
            }
            if self.decision_fields.1 {
                columns.push(column(
                    &format!("$DI-{target}"),
                    Storage::String,
                    hits.iter()
                        .map(|hit| {
                            hit.and_then(|rule| rule.id.as_deref())
                                .map_or(Value::Null, Value::string)
                        })
                        .collect(),
                ));
            }
            return with_columns(input, columns);
        }
        if self.kind == Kind::Neural {
            let options = neural_options(self.properties.as_deref(), context)?;
            let predictions = input
                .rows()
                .iter()
                .map(|row| {
                    let record: Vec<PmmlValue> = row.iter().map(to_pmml).collect();
                    scorer
                        .score_neural(&record)
                        .map_err(|error| OperatorError::Data(format!("model: {error}")))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let columns = neural_columns(input, &pmml, &predictions, &options)?;
            return with_columns(input, columns);
        }
        // QUEST nuggets treat a category the model does not list as missing, so its surrogates
        // decide (`quest_nofs/*` grid record 10043: blank `cat` scores A 0.571428 like a
        // missing value). Other tree algorithms have no such oracle and keep PMML's default.
        let unlisted = quest_category_fields(&pmml, input.model());
        let predictions = input
            .rows()
            .iter()
            .map(|row| {
                // Saved ALM DataDictionary intervals define valid input values. A score
                // outside that domain is invalid (null), so an Auto Numeric ensemble
                // excludes this component, rather than extrapolating it (an_default grid 4).
                if valid_intervals.iter().any(|(index, intervals)| {
                    row.get(*index)
                        .and_then(Value::as_f64)
                        .is_some_and(|x| !intervals.iter().any(|interval| interval.contains(x)))
                }) {
                    return Ok(Prediction {
                        value: PmmlValue::Missing,
                        confidence: None,
                        probabilities: Vec::new(),
                        entity_id: None,
                        affinity: None,
                    });
                }
                let mut record: Vec<PmmlValue> = row.iter().map(to_pmml).collect();
                for (index, allowed) in &unlisted {
                    if let Some(PmmlValue::Text(value)) = record.get(*index)
                        && !allowed.iter().any(|category| category == value)
                        && let Some(cell) = record.get_mut(*index)
                    {
                        *cell = PmmlValue::Missing;
                    }
                }
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
        if self.kind == Kind::Kohonen {
            let position = |p: &Prediction| {
                let dms_pmml::Model::Clustering(model) = &pmml.model else {
                    return None;
                };
                let name = p.entity_id.as_deref()?;
                model
                    .clusters
                    .iter()
                    .find(|c| c.id.as_deref().or(c.name.as_deref()) == Some(name))
                    .and_then(|c| c.map_position)
            };
            let coordinate = |p: &Prediction, y: bool| {
                position(p).map_or(Value::Null, |(x1, x2)| {
                    Value::Integer((if y { x2 } else { x1 }) as i64)
                })
            };
            // Like PCA/Factor, the fields carry the model's own name (`kohonen_default`'s grid
            // nugget, labelled "Grid scores", writes `$KX-models_next`).
            let model_name = match &pmml.model {
                dms_pmml::Model::Clustering(model) => model.header.name.clone(),
                _ => None,
            }
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| self.model_name.clone());
            let mut columns = vec![
                column(
                    &format!("$KX-{model_name}"),
                    Storage::Integer,
                    predictions.iter().map(|p| coordinate(p, false)).collect(),
                ),
                column(
                    &format!("$KY-{model_name}"),
                    Storage::Integer,
                    predictions.iter().map(|p| coordinate(p, true)).collect(),
                ),
            ];
            if self.cluster_label {
                columns.push(column(
                    &format!("$KXY-{model_name}"),
                    Storage::String,
                    predictions
                        .iter()
                        .map(|p| typed(&p.value, Storage::String))
                        .collect(),
                ));
            }
            return Ok(columns);
        }
        if self.kind == Kind::TwoStep {
            let storage = if self.numeric_labels {
                Storage::Integer
            } else {
                Storage::String
            };
            let values = predictions
                .iter()
                .map(|p| typed(&p.value, storage))
                .collect();
            return Ok(vec![column(
                &format!("$T-{}", self.model_name),
                storage,
                values,
            )]);
        }
        let target = target_name(pmml)?;
        let gle_regression = self.kind == Kind::Gle
            && matches!(
                &pmml.model,
                dms_pmml::Model::GeneralRegression(model)
                    if model.header.function == dms_pmml::MiningFunction::Regression
            );
        let gle_glm_class = self.kind == Kind::Gle
            && matches!(
                &pmml.model,
                dms_pmml::Model::GeneralRegression(model)
                    if model.header.function == dms_pmml::MiningFunction::Classification
                        && model.model_type == dms_pmml::GeneralModelType::GeneralizedLinear
            );
        let (value_prefix, confidence_prefix, propensity_prefix) = match self.kind {
            Kind::Gle if gle_regression || gle_glm_class => ("$G", "$GC", "$GRP"),
            Kind::Gle => ("$L", "$LC", "$LRP"),
            Kind::Logistic | Kind::Linear => ("$L", "$LP", "$LRP"),
            Kind::LinearSvm => ("$L", "$LC", "$LRP"),
            Kind::Regression => ("$E", "$EC", "$ERP"),
            Kind::Neural | Kind::Factor | Kind::Anomaly | Kind::DecisionList => {
                return Err(OperatorError::Data(
                    "neural network, factor and anomaly score fields are not built on this path"
                        .into(),
                ));
            }
            Kind::Rule
            | Kind::RandomTrees
            | Kind::TreeAs
            | Kind::KMeans
            | Kind::TwoStep
            | Kind::Kohonen => ("$R", "$RC", "$RRP"),
            Kind::Apriori => {
                return Err(OperatorError::Data(
                    "Apriori score fields are not built on this path".into(),
                ));
            }
        };
        let continuous_only = matches!(self.kind, Kind::Linear | Kind::Regression)
            || gle_regression
            || (matches!(
                self.kind,
                Kind::RandomTrees | Kind::TreeAs | Kind::LinearSvm
            ) && pmml
                .data_dictionary
                .iter()
                .any(|field| field.name == target && field.optype == dms_pmml::OpType::Continuous));
        // The first index whose prediction field is free.
        let index = (0..)
            .map(|n: usize| if n == 0 { String::new() } else { n.to_string() })
            .find(|n| !taken(&format!("{value_prefix}{n}-{target}")))
            .unwrap_or_default();
        // A GLE regression predicts a mean, real even for an integer (count) target
        // (`gle/gle_poisson_*`: `$G-y_poisson` 1.0768...).
        let storage = if gle_regression {
            Storage::Real
        } else {
            target_storage(pmml, &target)
        };
        let mut columns = vec![column(
            &format!("{value_prefix}{index}-{target}"),
            storage,
            predictions
                .iter()
                .map(|p| typed(&p.value, storage))
                .collect(),
        )];
        if self.confidences && !continuous_only {
            columns.push(column(
                &format!("{confidence_prefix}{index}-{target}"),
                Storage::Real,
                predictions.iter().map(|p| real(p.confidence)).collect(),
            ));
        }
        if self.all_probabilities && !continuous_only {
            let categories = category_names(pmml, &target, predictions);
            if categories.len() > MAX_CATEGORY_FIELDS {
                return Err(OperatorError::Data(format!(
                    "the logistic model lists {} target categories; at most {MAX_CATEGORY_FIELDS} \
                     probability fields are written",
                    categories.len()
                )));
            }
            for category in &categories {
                columns.push(column(
                    &format!("$LP{index}-{category}"),
                    Storage::Real,
                    predictions
                        .iter()
                        .map(|prediction| real(category_probability(prediction, category)))
                        .collect(),
                ));
            }
        }
        if self.raw_propensities && !continuous_only {
            columns.push(column(
                &format!("{propensity_prefix}{index}-{target}"),
                Storage::Real,
                predictions
                    .iter()
                    .map(|p| real(p.probabilities.get(1).map(|(_, probability)| *probability)))
                    .collect(),
            ));
        }
        if self.tree_id {
            columns.push(column(
                &format!("$RI{index}-{target}"),
                Storage::Integer,
                predictions
                    .iter()
                    .map(|p| {
                        p.entity_id
                            .as_deref()
                            .and_then(|id| id.parse::<i64>().ok())
                            .map_or(Value::Null, Value::Integer)
                    })
                    .collect(),
            ));
        }
        Ok(columns)
    }
}

/// A record is an anomaly when its index exceeds the cutoff (`anomaly_default`: the cutoff
/// 1.37201 is the top training index 1.372014362108979 to six digits, and that record is `T`).
/// Its fields are reported by decreasing contribution (to 12 digits), the first input on a tie.
/// A record the
/// model screens out (an unseen category) gets nulls.
fn anomaly_columns(
    scores: &[Option<dms_pmml::AnomalyScore>],
    cutoff: f64,
    fields: usize,
) -> Vec<Column> {
    let flag = scores
        .iter()
        .map(|score| match score {
            Some(score) => Value::string(if score.index > cutoff { "T" } else { "F" }),
            None => Value::Null,
        })
        .collect();
    let mut flag = column("$O-Anomaly", Storage::String, flag);
    flag.field.measurement = Measurement::Flag;
    let mut columns = vec![
        flag,
        column(
            "$O-AnomalyIndex",
            Storage::Real,
            scores
                .iter()
                .map(|score| score.as_ref().map_or(Value::Null, |s| Value::Real(s.index)))
                .collect(),
        ),
        column(
            "$O-PeerGroup",
            Storage::Integer,
            scores
                .iter()
                .map(|score| {
                    score.as_ref().map_or(Value::Null, |s| {
                        i64::try_from(s.cluster + 1).map_or(Value::Null, Value::Integer)
                    })
                })
                .collect(),
        ),
    ];
    let ranked: Vec<Vec<(String, f64)>> = scores
        .iter()
        .map(|score| {
            let mut ranked = score
                .as_ref()
                .map(|s| s.contributions.clone())
                .unwrap_or_default();
            // Contributions equal but for float noise tie, and the first input wins
            // (`models_next2/anomaly_peers` row 1: cat 0.233313761554428 and ord
            // 0.233313761554427 in Modeler, the other way round here).
            let key = |x: f64| -> f64 { format!("{x:.11e}").parse().unwrap_or(x) };
            ranked.sort_by(|a, b| key(b.1).total_cmp(&key(a.1)));
            ranked
        })
        .collect();
    for k in 0..fields {
        columns.push(column(
            &format!("$O-Field-{}", k + 1),
            Storage::String,
            ranked
                .iter()
                .map(|r| {
                    r.get(k)
                        .map_or(Value::Null, |(name, _)| Value::string(name.as_str()))
                })
                .collect(),
        ));
        columns.push(column(
            &format!("$O-FieldImpact-{}", k + 1),
            Storage::Real,
            ranked
                .iter()
                .map(|r| {
                    r.get(k)
                        .map_or(Value::Null, |(_, share)| Value::Real(*share))
                })
                .collect(),
        ));
    }
    columns
}

fn flag_hits(
    pmml: &Pmml,
    model: &dms_pmml::AssociationModel,
    input: &Table,
    options: &AprioriOptions,
) -> Result<Vec<Vec<dms_pmml::ScoredAssociation>>, OperatorError> {
    // Active fields: flags with a true value score the field name when true;
    // valued fields (`trueflags_false`, `nominal_set`: no flag true value in the PMML)
    // score `field=value` for every non-null value.
    let mut specs = Vec::new();
    for field in model
        .header
        .mining_schema
        .iter()
        .filter(|field| field.usage == Usage::Active)
    {
        let position = input
            .model()
            .position(&field.name)
            .ok_or_else(|| OperatorError::Data(format!("Apriori needs field {:?}", field.name)))?;
        let true_value = pmml
            .data_dictionary
            .iter()
            .find(|data| data.name == field.name)
            .and_then(|data| data.flag_true_value.clone());
        specs.push((position, field.name.clone(), true_value));
    }
    let mut rows = Vec::with_capacity(input.rows().len());
    for row in input.rows() {
        let mut present = HashSet::new();
        for (position, name, true_value) in &specs {
            let Some(value) = row.get(*position) else {
                continue;
            };
            if *value == Value::Null {
                continue;
            }
            if let Some(true_text) = true_value {
                if value.to_string() == *true_text {
                    present.insert(name.clone());
                }
            } else {
                present.insert(format!("{name}={value}"));
            }
        }
        rows.push(
            dms_pmml::score_rules_by(model, &present, options)
                .map_err(|error| OperatorError::Data(format!("Apriori: {error}")))?,
        );
    }
    Ok(rows)
}

fn transactional_hits(
    model: &dms_pmml::AssociationModel,
    input: &Table,
    id_at: usize,
    content_at: usize,
    options: &AprioriOptions,
) -> Result<Vec<Vec<dms_pmml::ScoredAssociation>>, OperatorError> {
    use dms_pmml::{ScoredAssociation, score_rules_by};
    let known: HashSet<&str> = model.items.iter().map(|item| item.value.as_str()).collect();
    let mut out: Vec<Vec<ScoredAssociation>> = vec![Vec::new(); input.rows().len()];
    let mut start = 0;
    while start < input.rows().len() {
        let id = input
            .rows()
            .get(start)
            .and_then(|row| row.get(id_at))
            .cloned()
            .unwrap_or(Value::Null);
        if id == Value::Null {
            return Err(OperatorError::Data(format!(
                "Apriori transaction id is null at row {}",
                start + 1
            )));
        }
        let mut end = start.saturating_add(1);
        while end < input.rows().len() {
            let next = input
                .rows()
                .get(end)
                .and_then(|row| row.get(id_at))
                .cloned()
                .unwrap_or(Value::Null);
            if next == Value::Null {
                return Err(OperatorError::Data(format!(
                    "Apriori transaction id is null at row {}",
                    end + 1
                )));
            }
            if next != id {
                break;
            }
            end = end.saturating_add(1);
        }
        // Score each row on its prefix up to that row (`trans_base` grid basket 104:
        // `bread` null, `eggs` milk, `jam` milk+butter). `ignoreUnmatchedItems` true skips
        // values that are not model items; false keeps them in the basket, where no
        // antecedent can equal it (`score_trans_unmatched` basket 106, `cheese`: nothing).
        let mut present = HashSet::new();
        for row_index in start..end {
            let Some(content) = input
                .rows()
                .get(row_index)
                .and_then(|row| row.get(content_at))
            else {
                continue;
            };
            if *content == Value::Null {
                continue;
            }
            let text = content.to_string();
            if options.exact_antecedent || known.contains(text.as_str()) {
                present.insert(text);
            }
            let scored = score_rules_by(model, &present, options)
                .map_err(|error| OperatorError::Data(format!("Apriori: {error}")))?;
            if let Some(slot) = out.get_mut(row_index) {
                *slot = scored;
            }
        }
        start = end;
    }
    Ok(out)
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
        dms_pmml::Model::RuleSet(model) => &model.header,
        dms_pmml::Model::GeneralRegression(model) => &model.header,
        dms_pmml::Model::Regression(model) => &model.header,
        dms_pmml::Model::Neural(model) => &model.header,
        dms_pmml::Model::Clustering(model) => &model.header,
        dms_pmml::Model::Mining(model) => &model.header,
        dms_pmml::Model::Association(_) => {
            return Err(OperatorError::Data(
                "an association model has no single target".into(),
            ));
        }
        dms_pmml::Model::TimeSeries { header, .. } => header,
        dms_pmml::Model::Sequence(_) => {
            return Err(OperatorError::Data(
                "a sequence model has no single target".into(),
            ));
        }
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

/// For a QUEST tree, the input positions of its categorical fields and their listed categories.
fn quest_category_fields(pmml: &Pmml, model: &dms_types::DataModel) -> Vec<(usize, Vec<String>)> {
    let dms_pmml::Model::Tree(tree) = &pmml.model else {
        return Vec::new();
    };
    if tree.header.algorithm.as_deref() != Some("Quest") {
        return Vec::new();
    }
    pmml.data_dictionary
        .iter()
        .filter(|field| field.optype != dms_pmml::OpType::Continuous && !field.values.is_empty())
        .filter_map(|field| {
            model
                .position(&field.name)
                .map(|index| (index, field.values.clone()))
        })
        .collect()
}

pub(super) fn to_pmml(value: &Value) -> PmmlValue {
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
pub(super) fn typed(value: &PmmlValue, storage: Storage) -> Value {
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

/// A model file can list as many categories as it likes; cap the columns we allocate.
const MAX_CATEGORY_FIELDS: usize = 10_000;

/// Category names in score order. A row with no probabilities (a missing input) does not
/// decide the columns; the target's `DataField` values are that same order.
fn category_names(pmml: &Pmml, target: &str, predictions: &[Prediction]) -> Vec<String> {
    if let Some(prediction) = predictions.iter().find(|p| !p.probabilities.is_empty()) {
        return prediction
            .probabilities
            .iter()
            .map(|(category, _)| category.clone())
            .collect();
    }
    pmml.data_dictionary
        .iter()
        .find(|field| field.name == target)
        .map(|field| field.values.clone())
        .unwrap_or_default()
}

/// The probability of `category` on this row. An empty list is a missing score (null), not a
/// zero. Names are the strings `Prediction::probabilities` already uses.
fn category_probability(prediction: &Prediction, category: &str) -> Option<f64> {
    prediction
        .probabilities
        .iter()
        .find(|(name, _)| name == category)
        .map(|(_, probability)| *probability)
}

struct ValidInterval {
    low: Option<f64>,
    high: Option<f64>,
    closed_low: bool,
    closed_high: bool,
}

impl ValidInterval {
    fn contains(&self, x: f64) -> bool {
        x.is_finite()
            && self
                .low
                .is_none_or(|low| x > low || (self.closed_low && x == low))
            && self
                .high
                .is_none_or(|high| x < high || (self.closed_high && x == high))
    }
}

/// PMML 4.3 DataDictionary/Interval validity for saved ALM models. Kept in the nugget
/// adapter until the general PMML data-field model retains intervals. Other model families
/// have separate treatments; applying ALM's rule to them has no oracle.
/// Specification: https://dmg.org/pmml/v4-3/DataDictionary.html#xsdElement_Interval
fn alm_valid_intervals(
    xml: &str,
    model: &dms_types::DataModel,
) -> Result<Vec<(usize, Vec<ValidInterval>)>, OperatorError> {
    let document = roxmltree::Document::parse(xml)
        .map_err(|error| OperatorError::Data(format!("ALM PMML: {error}")))?;
    let root = document.root_element();
    let Some(alm) = root.children().find(|e| {
        e.is_element()
            && e.tag_name().name() == "GeneralRegressionModel"
            && e.attribute("algorithmName") == Some("ALM")
    }) else {
        return Ok(Vec::new());
    };
    let active: HashSet<_> = alm
        .children()
        .filter(|e| e.is_element() && e.tag_name().name() == "MiningSchema")
        .flat_map(|e| e.children())
        .filter(|e| {
            e.is_element()
                && e.tag_name().name() == "MiningField"
                && matches!(e.attribute("usageType"), None | Some("active"))
        })
        .filter_map(|e| e.attribute("name"))
        .collect();
    let mut ranges = Vec::new();
    for field in root
        .children()
        .filter(|e| e.is_element() && e.tag_name().name() == "DataDictionary")
        .flat_map(|e| e.children())
        .filter(|e| e.is_element() && e.tag_name().name() == "DataField")
    {
        let Some(name) = field.attribute("name").filter(|name| active.contains(name)) else {
            continue;
        };
        let index = model
            .position(name)
            .ok_or_else(|| OperatorError::Data(format!("ALM input {name:?} is absent")))?;
        let mut intervals = Vec::new();
        for interval in field
            .children()
            .filter(|e| e.is_element() && e.tag_name().name() == "Interval")
        {
            let margin = |key| -> Result<Option<f64>, OperatorError> {
                interval
                    .attribute(key)
                    .map(|text| {
                        text.parse::<f64>()
                            .ok()
                            .filter(|x| x.is_finite())
                            .ok_or_else(|| {
                                OperatorError::Data(format!("ALM {name}: invalid {key}"))
                            })
                    })
                    .transpose()
            };
            let low = margin("leftMargin")?;
            let high = margin("rightMargin")?;
            if low.zip(high).is_some_and(|(low, high)| low > high) {
                return Err(OperatorError::Data(format!(
                    "ALM {name}: reversed interval"
                )));
            }
            let (closed_low, closed_high) = match interval.attribute("closure") {
                Some("closedClosed") => (true, true),
                Some("closedOpen") => (true, false),
                Some("openClosed") => (false, true),
                Some("openOpen") => (false, false),
                other => {
                    return Err(OperatorError::Settings(format!(
                        "GAP: ALM interval closure {other:?}"
                    )));
                }
            };
            if intervals.len() >= 10_000 {
                return Err(OperatorError::Data("ALM: too many intervals".into()));
            }
            intervals.push(ValidInterval {
                low,
                high,
                closed_low,
                closed_high,
            });
        }
        if !intervals.is_empty() {
            ranges.push((index, intervals));
        }
    }
    Ok(ranges)
}

/// Which neural score fields the nugget writes. Defaults match a nugget with no properties
/// attachment: prediction and confidence on, `onProbability`, no category probabilities, no
/// propensity. The `nn_*` streams all have the attachment.
struct NeuralFields {
    prediction: bool,
    confidence: bool,
    on_increase: bool,
    category_probabilities: bool,
    /// `None` emits every category. `Some(1)` is `$NP-` of the first `DataField` value only
    /// (`nn_fixed_a` "One category probability").
    max_categories: Option<usize>,
    raw_propensity: bool,
    adjusted_propensity: bool,
    /// `CalculatePropensity = Calculate`: raw and adjusted propensity for flag targets
    /// (`nn_partition`; `scoring.isRawPropensity`/`isPropensity` stay false there).
    calculate_propensity: bool,
}

fn neural_options(
    attachment: Option<&str>,
    context: &ExecContext,
) -> Result<NeuralFields, OperatorError> {
    let Some(attachment) = attachment else {
        return Ok(NeuralFields {
            prediction: true,
            confidence: true,
            on_increase: false,
            category_probabilities: false,
            max_categories: None,
            raw_propensity: false,
            adjusted_propensity: false,
            calculate_propensity: false,
        });
    };
    let params = extension_params::load_parameters(Some(attachment), context, "neural network")?;
    let max_categories = match extension_params::text(&params, "scoring.maxCategories")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") => None,
        Some(value) => {
            let count = value.parse::<usize>().map_err(|_| {
                OperatorError::Settings(format!(
                    "scoring.maxCategories: expected an integer, found {value:?}"
                ))
            })?;
            if count > MAX_CATEGORY_FIELDS {
                return Err(OperatorError::Settings(format!(
                    "scoring.maxCategories {count} exceeds {MAX_CATEGORY_FIELDS}"
                )));
            }
            Some(count)
        }
    };
    let on_increase = match extension_params::text(&params, "scoring.confidenceBasedOn")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("onProbability") => false,
        Some("onIncrease") => true,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "scoring.confidenceBasedOn {other:?} is not onProbability or onIncrease"
            )));
        }
    };
    Ok(NeuralFields {
        prediction: flag(&params, "scoring.isPrediction")?.unwrap_or(true),
        confidence: flag(&params, "scoring.isConfidence")?.unwrap_or(true),
        on_increase,
        category_probabilities: flag(&params, "scoring.isProbabilityCategory")?.unwrap_or(false),
        max_categories,
        raw_propensity: flag(&params, "scoring.isRawPropensity")?.unwrap_or(false),
        adjusted_propensity: flag(&params, "scoring.isPropensity")?.unwrap_or(false),
        calculate_propensity: extension_params::text(&params, "CalculatePropensity").as_deref()
            == Some("Calculate"),
    })
}

fn neural_columns(
    input: &Table,
    pmml: &Pmml,
    predictions: &[Vec<NeuralTarget>],
    options: &NeuralFields,
) -> Result<Vec<Column>, OperatorError> {
    let network = match &pmml.model {
        dms_pmml::Model::Neural(model) => model,
        _ => {
            return Err(OperatorError::Data(
                "the nugget's saved model is not a neural network".into(),
            ));
        }
    };
    let targets = neural_target_order(network);
    let first = targets
        .first()
        .ok_or_else(|| OperatorError::Data("the neural network names no target field".into()))?;
    let taken = |name: &str| input.model().position(name).is_some();
    let index = (0..MAX_CATEGORY_FIELDS)
        .map(|n| if n == 0 { String::new() } else { n.to_string() })
        .find(|suffix| !taken(&format!("$N{suffix}-{}", first.0)))
        .ok_or_else(|| OperatorError::Data("no free $N- score field index".into()))?;
    let categorical_targets = targets
        .iter()
        .filter(|(_, categorical)| *categorical)
        .count();
    if (options.raw_propensity || options.adjusted_propensity) && categorical_targets == 0 {
        return Err(OperatorError::Settings(
            "neural propensity is not defined for a continuous target".into(),
        ));
    }
    if options.adjusted_propensity
        && categorical_targets > 0
        && network.adjusted_propensity.is_none()
    {
        return Err(OperatorError::Data(
            "scoring.isPropensity is set but the PMML has no adjustedPropensity extension".into(),
        ));
    }

    let mut columns = Vec::new();
    for (name, categorical) in &targets {
        let storage = target_storage(pmml, name);
        if options.prediction {
            let mut values = Vec::with_capacity(predictions.len());
            for row in predictions {
                values.push(typed(&row_target(row, name)?.value, storage));
            }
            columns.push(column(&format!("$N{index}-{name}"), storage, values));
        }
        if *categorical && options.confidence {
            let mut values = Vec::with_capacity(predictions.len());
            for row in predictions {
                values.push(real(neural_confidence(
                    row_target(row, name)?,
                    options.on_increase,
                )));
            }
            columns.push(column(&format!("$NC{index}-{name}"), Storage::Real, values));
        }
        if *categorical && options.category_probabilities {
            let categories = neural_categories(pmml, network, name);
            let limit = options.max_categories.unwrap_or(categories.len());
            for category in categories.iter().take(limit) {
                let field = if categorical_targets > 1 {
                    format!("$NP{index}-{name}-{category}")
                } else {
                    format!("$NP{index}-{category}")
                };
                let mut values = Vec::with_capacity(predictions.len());
                for row in predictions {
                    values.push(real(category_probability_of(
                        row_target(row, name)?,
                        category,
                    )?));
                }
                columns.push(column(&field, Storage::Real, values));
            }
        }
        let flag_target = pmml
            .data_dictionary
            .iter()
            .any(|field| field.name == *name && field.flag_true_value.is_some());
        let calculated = options.calculate_propensity && flag_target;
        if *categorical && (options.raw_propensity || calculated) {
            let mut values = Vec::with_capacity(predictions.len());
            for row in predictions {
                values.push(real(raw_propensity(pmml, name, row_target(row, name)?)?));
            }
            columns.push(column(&format!("$RP{index}-{name}"), Storage::Real, values));
        }
        if *categorical
            && (options.adjusted_propensity
                || (calculated && network.adjusted_propensity.is_some()))
        {
            let mut values = Vec::with_capacity(predictions.len());
            for row in predictions {
                let raw = raw_propensity(pmml, name, row_target(row, name)?)?;
                let adjusted = match raw {
                    None => None,
                    Some(raw) => Some(
                        network
                            .adjusted_propensity(raw)
                            .map_err(|error| OperatorError::Data(format!("model: {error}")))?,
                    ),
                };
                values.push(real(adjusted));
            }
            columns.push(column(&format!("$AP{index}-{name}"), Storage::Real, values));
        }
    }
    Ok(columns)
}

/// `(name, categorical)` in first-seen `NeuralOutput` order (`nn_mixed_targets` emits `r` then `y`).
fn neural_target_order(model: &NeuralNetworkModel) -> Vec<(String, bool)> {
    let mut targets: Vec<(String, bool)> = Vec::new();
    for output in &model.outputs {
        let categorical = matches!(output.kind, NeuralOutputKind::Category(_));
        if let Some(existing) = targets.iter_mut().find(|target| target.0 == output.target) {
            if categorical {
                existing.1 = true;
            }
        } else {
            targets.push((output.target.clone(), categorical));
        }
    }
    targets
}

fn neural_categories(pmml: &Pmml, model: &NeuralNetworkModel, target: &str) -> Vec<String> {
    if let Some(field) = pmml
        .data_dictionary
        .iter()
        .find(|field| field.name == target)
        && !field.values.is_empty()
    {
        return field.values.clone();
    }
    model
        .outputs
        .iter()
        .filter_map(|output| match &output.kind {
            NeuralOutputKind::Category(category) if output.target == target => {
                Some(category.clone())
            }
            _ => None,
        })
        .collect()
}

fn row_target<'a>(row: &'a [NeuralTarget], name: &str) -> Result<&'a NeuralTarget, OperatorError> {
    row.iter()
        .find(|target| target.name == name)
        .ok_or_else(|| {
            OperatorError::Data(format!("neural network score is missing target {name:?}"))
        })
}

fn neural_confidence(target: &NeuralTarget, on_increase: bool) -> Option<f64> {
    if target.probabilities.is_empty() {
        return None;
    }
    let mut best = f64::NEG_INFINITY;
    let mut second = f64::NEG_INFINITY;
    for (_, probability) in &target.probabilities {
        if *probability > best {
            second = best;
            best = *probability;
        } else if *probability > second {
            second = *probability;
        }
    }
    if !best.is_finite() {
        return None;
    }
    if on_increase {
        let second = if second.is_finite() { second } else { 0.0 };
        Some(best - second)
    } else {
        Some(best)
    }
}

fn category_probability_of(
    target: &NeuralTarget,
    category: &str,
) -> Result<Option<f64>, OperatorError> {
    if target.probabilities.is_empty() && target.value == PmmlValue::Missing {
        return Ok(None);
    }
    target
        .probabilities
        .iter()
        .find(|(name, _)| name == category)
        .map(|(_, probability)| Some(*probability))
        .ok_or_else(|| {
            OperatorError::Data(format!(
                "neural network has no probability for category {category:?}"
            ))
        })
}

/// `P(flagTypeTrueValue)` when the target declares one (`nn_partition` `"1"`).
/// GAP: with no flag extension, the second category. No non-flag neural oracle covers that.
fn raw_propensity(
    pmml: &Pmml,
    target_name: &str,
    target: &NeuralTarget,
) -> Result<Option<f64>, OperatorError> {
    if target.probabilities.is_empty() {
        return Ok(None);
    }
    let flag = pmml
        .data_dictionary
        .iter()
        .find(|field| field.name == target_name)
        .and_then(|field| field.flag_true_value.as_deref());
    let category = if let Some(flag) = flag {
        flag.to_string()
    } else {
        target
            .probabilities
            .get(1)
            .map(|(name, _)| name.clone())
            .ok_or_else(|| {
                OperatorError::Data(format!(
                    "raw propensity for {target_name:?} needs a flag true value or a second category"
                ))
            })?
    };
    category_probability_of(target, &category)
}

/// The PMML text of a nugget's saved model attachment: a Modeler model file (`SCF` + 0x01), an
/// Analytic Engine length-prefixed ZIP (`PMML.xml`), or plain PMML (an imported model).
pub(super) fn saved_model_xml(
    member: &str,
    context: &ExecContext,
) -> Result<String, OperatorError> {
    let bytes = context
        .attachments
        .get(member)
        .ok_or_else(|| OperatorError::Io(format!("the stream file has no attachment {member}")))?;
    let text = if bytes.starts_with(b"SCF\x01") {
        dms_strfmt::read_model_xml(bytes).map_err(|error| {
            OperatorError::Data(format!(
                "the model in {member} is not readable PMML: {error}"
            ))
        })?
    } else if dms_strfmt::zip_payload(bytes).is_ok() {
        // `models_as/*`: the AS PMML container is a length-prefixed ZIP with
        // `.meta` and `PMML.xml`. Reuse the bounded reader used by Time Series.
        let members =
            dms_strfmt::read_length_prefixed_zip(bytes, &dms_strfmt::ArchiveLimits::default())
                .map_err(|error| OperatorError::Data(format!("{member}: {error}")))?;
        let pmml = members
            .iter()
            .find(|m| m.name == "PMML.xml")
            .ok_or_else(|| {
                OperatorError::Data(format!("{member}: GAP: AS container has no PMML.xml"))
            })?;
        std::str::from_utf8(&pmml.bytes)
            .map_err(|error| OperatorError::Data(format!("{member}/PMML.xml: {error}")))?
            .to_string()
    } else {
        std::str::from_utf8(bytes)
            .map_err(|_| {
                OperatorError::Data(format!(
                    "the model in {member} is not PMML (a format we cannot read)"
                ))
            })?
            .to_string()
    };
    Ok(text)
}
