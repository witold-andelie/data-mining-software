//! Built-in node implementations, by catalog id. Parity (`cargo xtask parity`) counts exactly
//! these entries, so an id belongs here only once its node works and is tested.

use crate::error::OperatorError;
use crate::nodes;
use crate::operator::{Factory, Operator};
use dms_stream::StreamNode;

const BUILT_IN: &[(&str, Factory)] = &[
    ("table", nodes::table::build),
    ("database_import", nodes::database::build_import),
    ("database_export", nodes::database::build_export),
    ("set_globals", nodes::set_globals::build),
    ("derive", nodes::derive::build),
    ("ensemble", nodes::ensemble::build),
    ("filler", nodes::filler::build),
    ("reclassify", nodes::reclassify::build),
    ("filter", nodes::filter::build),
    ("anonymize", nodes::anonymize::build),
    ("partition", nodes::partition::build),
    ("restructure", nodes::restructure::build),
    ("rfm_aggregate", nodes::rfm_aggregate::build),
    ("rfm_analysis", nodes::rfm_analysis::build),
    ("var_file", nodes::var_file::build),
    ("fixed_file", nodes::fixed_file::build),
    ("json_import", nodes::json_import::build),
    ("user_input", nodes::user_input::build),
    ("statistics_file", nodes::statistics_file::build),
    ("type", nodes::r#type::build),
    ("select", nodes::select::build),
    ("sample", nodes::sample::build),
    ("sort", nodes::sort::build),
    ("distinct", nodes::distinct::build),
    ("aggregate", nodes::aggregate::build),
    ("append", nodes::append::build),
    ("merge", nodes::merge::build),
    ("balance", nodes::balance::build),
    ("data_audit", nodes::data_audit::build),
    ("statistics", nodes::statistics::build),
    ("means", nodes::means::build),
    ("matrix", nodes::matrix::build),
    ("distribution", nodes::distribution::build),
    ("histogram", nodes::histogram::build),
    ("graphboard", nodes::graphboard::build),
    ("plot", nodes::plot::build),
    ("multiplot", nodes::plot::build_multiplot),
    ("time_intervals", nodes::time_intervals::build),
    ("time_plot", nodes::time_plot::build),
    ("collection", nodes::collection::build),
    ("web", nodes::web::build),
    ("evaluation", nodes::evaluation::build),
    ("analysis", nodes::analysis::build),
    ("report", nodes::report::build),
    ("transform", nodes::transform::build),
    ("cart", model_node),
    ("c50", model_node),
    ("chaid", model_node),
    ("tree_as", model_node),
    ("linear_as", model_node),
    ("kmeans", model_node),
    ("apriori", model_node),
    ("carma", model_node),
    ("twostep", model_node),
    ("logistic", model_node),
    ("linear", model_node),
    ("regression", model_node),
    ("pca_factor", model_node),
    ("anomaly", model_node),
    ("feature_selection", model_node),
    ("decision_list", model_node),
    ("binning", nodes::binning::build),
    ("auto_data_prep", nodes::auto_data_prep::build),
    ("set_to_flag", nodes::set_to_flag::build),
    ("transpose", nodes::transpose::build),
    ("field_reorder", nodes::field_reorder::build),
    ("history", nodes::history::build),
    ("flat_file_export", nodes::flat_file_export::build),
    ("json_export", nodes::json_export::build),
    ("xml_export", nodes::xml_export::build),
    ("excel_export", nodes::excel_export::build),
    ("statistics_export", nodes::statistics_export::build),
];

/// Registered while their implementation is under way; not counted as implemented.
const PENDING: &[&str] = &[];

/// Model nodes whose nuggets score (`nodes::apply`) while their build nodes cannot train yet;
/// they do not count as implemented.
/// Neural Net nuggets score from saved PMML. Training stays unimplemented, so a failed builder
/// leaves the nugget its saved model (`corpus/models_ml/nn_fixed_a.str`). Not counted as
/// implemented.
/// Kohonen likewise (`corpus/models_next/kohonen_*.str`): training starts from random weights
/// drawn with an unknown generator (`randomSeed`), so it is a GAP.
/// Auto Cluster: the nugget scores its saved component model (`nodes::auto_cluster`);
/// training (`BuildAutoClusteringModelNode`) is not implemented.
/// Auto Classifier / Numeric: saved components and Ensemble scoring (`nodes::auto_models`);
/// their builders remain explicit GAPs and do not count toward implemented coverage.
/// Sequence: the nugget scores its saved rules; training is a GAP (`nodes::sequence`).
/// Time Series (`corpus/models_next/ts_arima.str`, `ts_expert.str`,
/// `corpus/models_next2/ts_simple.str`, `ts_holt.str`): the nugget scores the saved ARIMA
/// or exponential smoothing model; estimation (`spss.ts!ts_builder`) is a GAP.
/// Analytic Engine Random Trees and LSVM score their saved PMML.
/// Tree-AS trains through the shared CHAID learner; Linear-AS trains with its
/// correlation/sweep selection (`linearas/lin_default`, `lin_factor`).
const SCORING_ONLY: &[&str] = &[
    "neural_net",
    "kohonen",
    "sequence",
    "time_series",
    "auto_cluster",
    "auto_classifier",
    "auto_numeric",
    "random_trees",
    "lsvm",
];

pub fn factory(catalog_id: &str) -> Option<Factory> {
    BUILT_IN
        .iter()
        .find(|(id, _)| *id == catalog_id)
        .map(|(_, factory)| *factory)
        .or_else(|| {
            SCORING_ONLY
                .contains(&catalog_id)
                .then_some(nugget_or_builder as Factory)
        })
}

/// A model node that trains (`nodes::build`) and scores (`nodes::apply`).
fn model_node(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    if is_nugget(node) {
        nodes::apply::build(node)
    } else {
        nodes::build::build(node)
    }
}

/// Why each scoring-only builder cannot train, by builder type. The evidence for each reason is
/// in `plan/STATUS.md` (2026-10-19 training review) and the nugget modules.
const TRAINING_GAPS: &[(&str, &str)] = &[
    (
        "spss.ts!ts_builder",
        "Time Series estimation is not reproduced: the saved smoothing parameters are the optimizer's stopping point (ts_simple alpha 0.9999889 on a series whose optimum is alpha = 1), which the 1e-9 comparison cannot meet without Modeler's optimizer",
    ),
    (
        "spss.linearSVM!linearSVM",
        "LSVM training is not reproduced: Modeler's ADMM stops before the optimum (lsvm_default objective 0.868048 vs optimum 0.867945, lsvm_numeric 0.556714 vs 0.550353) and its blocks, inner solver and stopping rule are undocumented",
    ),
    (
        "spss.randomTrees!RT",
        "Random Trees training is not reproduced: bootstrap counts are binomial draws and predictor subsets are seeded by a hash of tree and node id, with an undocumented generator and hash; the PMML trees carry no record counts to identify them",
    ),
    (
        "TrainKohonenNode",
        "Kohonen training starts from random weights and a random record order drawn with an unknown generator",
    ),
    (
        "neuralnet!spss.neuralnet-extension",
        "Neural Net training draws random initial weights and a random 70/30 split with an unknown generator",
    ),
    (
        "BuildSequenceNode",
        "Sequence training orders tied rules by no recoverable key",
    ),
    (
        "BuildAutoClusteringModelNode",
        "Auto Cluster training needs a Kohonen component, whose training is a GAP",
    ),
    (
        "BuildBinaryClassifierNode",
        "Auto Classifier training needs Neural Net, Random Trees and LSVM components, whose training is a GAP",
    ),
    (
        "BuildNumericClassifierNode",
        "Auto Numeric training needs Neural Net, Random Trees and LSVM components, whose training is a GAP",
    ),
];

/// A nugget scores; its build node is a GAP (`TRAINING_GAPS`).
fn nugget_or_builder(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    if is_nugget(node) {
        return nodes::apply::build(node);
    }
    let builder = node.type_identifier();
    let reason = TRAINING_GAPS
        .iter()
        .find(|(id, _)| *id == builder)
        .map_or("its training is not implemented", |(_, reason)| reason);
    Err(OperatorError::Settings(format!(
        "GAP: training {builder}: {reason}; score the nugget's saved model"
    )))
}

/// Nodes the planner implements itself, without an operator (supernodes are expanded).
const PLANNED: &[&str] = &["supernode"];

pub fn is_implemented(catalog_id: &str) -> bool {
    (BUILT_IN.iter().any(|(id, _)| *id == catalog_id) && !PENDING.contains(&catalog_id))
        || PLANNED.contains(&catalog_id)
}

pub fn implemented() -> impl Iterator<Item = &'static str> {
    BUILT_IN.iter().map(|(id, _)| *id)
}

/// A model nugget: classic `Apply*` nodes and extension-framework nuggets (`CFApplyModelNode`,
/// whose type is the model's, such as `LinearRegressionModel!spss.alm-extension`).
fn is_nugget(node: &StreamNode) -> bool {
    node.type_identifier().starts_with("Apply")
        || node.class.ends_with(".CFApplyModelNode")
        || node.class.ends_with(".ExtensionApplyModelNode")
        || node.type_identifier() == "FeatureSelectionModelNode"
}
