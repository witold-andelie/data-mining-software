//! Built-in node implementations, by catalog id. Parity (`cargo xtask parity`) counts exactly
//! these entries, so an id belongs here only once its node works and is tested.

use crate::error::OperatorError;
use crate::nodes;
use crate::operator::{Factory, Operator};
use dms_stream::StreamNode;

const BUILT_IN: &[(&str, Factory)] = &[
    ("table", nodes::table::build),
    ("derive", nodes::derive::build),
    ("filler", nodes::filler::build),
    ("reclassify", nodes::reclassify::build),
    ("filter", nodes::filter::build),
    ("partition", nodes::partition::build),
    ("restructure", nodes::restructure::build),
    ("var_file", nodes::var_file::build),
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
    ("analysis", nodes::analysis::build),
    ("cart", model_node),
    ("chaid", model_node),
    ("kmeans", model_node),
    ("twostep", model_node),
    ("logistic", model_node),
    ("linear", model_node),
    ("regression", model_node),
    ("binning", nodes::binning::build),
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
/// TwoStep numbers clusters differently on larger data (the GAP in `dms_algo::twostep`);
/// Linear trains only without data preparation
/// and model selection, which a new Linear node enables (`dms_algo::linear`).
const PENDING: &[&str] = &["twostep", "linear"];

/// Model nodes whose nuggets score (`nodes::apply`) while their build nodes cannot train yet;
/// they do not count as implemented.
const SCORING_ONLY: &[&str] = &[];

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

/// A nugget scores; its build node is not implemented yet.
fn nugget_or_builder(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    if is_nugget(node) {
        nodes::apply::build(node)
    } else {
        Err(OperatorError::Settings(format!(
            "training {} is not implemented yet",
            node.type_identifier()
        )))
    }
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
    node.type_identifier().starts_with("Apply") || node.class.ends_with(".CFApplyModelNode")
}
