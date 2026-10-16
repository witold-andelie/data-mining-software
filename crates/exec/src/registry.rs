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
];

/// Model nodes whose nuggets score (`nodes::apply`) while their build nodes cannot train yet;
/// they do not count as implemented.
const SCORING_ONLY: &[&str] = &["cart", "chaid", "logistic", "kmeans"];

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

/// A nugget scores; its build node is not implemented yet.
fn nugget_or_builder(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    if node.type_identifier().starts_with("Apply") {
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
    BUILT_IN.iter().any(|(id, _)| *id == catalog_id) || PLANNED.contains(&catalog_id)
}

pub fn implemented() -> impl Iterator<Item = &'static str> {
    BUILT_IN.iter().map(|(id, _)| *id)
}
