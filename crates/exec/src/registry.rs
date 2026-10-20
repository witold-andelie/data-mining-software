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
    ("sas_import", nodes::sas_import::build),
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
    ("quest", model_node),
    ("c50", model_node),
    ("chaid", model_node),
    ("tree_as", model_node),
    ("linear_as", model_node),
    ("lsvm", model_node),
    ("kmeans", model_node),
    ("kohonen", model_node),
    ("apriori", model_node),
    ("carma", model_node),
    ("twostep", model_node),
    ("logistic", model_node),
    ("linear", model_node),
    ("neural_net", model_node),
    ("regression", model_node),
    ("pca_factor", model_node),
    ("anomaly", model_node),
    ("random_trees", model_node),
    ("feature_selection", model_node),
    ("auto_cluster", model_node),
    ("decision_list", model_node),
    ("sequence", model_node),
    ("time_series", model_node),
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
    ("sas_export", nodes::sas_export::build),
    ("statistics_export", nodes::statistics_export::build),
];

/// Registered while their implementation is under way; not counted as implemented.
const PENDING: &[&str] = &[];

/// Implemented builders held to statistical rather than exact parity
/// (`docs/statistical-parity.md`): their training draws from our own generator or converges
/// where Modeler stops early, so they match Modeler's metrics, not its bits.
const STATISTICAL: &[&str] = &[
    "auto_cluster",
    "cart",
    "kohonen",
    "lsvm",
    "neural_net",
    "quest",
    "random_trees",
    "sequence",
    "time_series",
];

/// Model nodes whose nuggets score (`nodes::apply`) while their build nodes cannot train yet;
/// they do not count as implemented.
/// Auto Classifier / Numeric: saved components and Ensemble scoring (`nodes::auto_models`);
/// their builders remain explicit GAPs and do not count toward implemented coverage.
/// Tree-AS trains through the shared CHAID learner; Linear-AS trains with its
/// correlation/sweep selection (`linearas/lin_default`, `lin_factor`); LSVM trains the
/// documented objective to convergence (statistical parity, `dms_algo::lsvm`).
const SCORING_ONLY: &[&str] = &["auto_classifier", "auto_numeric"];

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

/// Whether an implemented node meets statistical rather than exact parity.
pub fn is_statistical(catalog_id: &str) -> bool {
    STATISTICAL.contains(&catalog_id)
}

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
