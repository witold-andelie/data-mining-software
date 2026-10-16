//! Built-in node implementations, by catalog id. Parity (`cargo xtask parity`) counts exactly
//! these entries, so an id belongs here only once its node works and is tested.

use crate::nodes;
use crate::operator::Factory;

const BUILT_IN: &[(&str, Factory)] = &[
    ("table", nodes::table::build),
    ("derive", nodes::derive::build),
    ("filler", nodes::filler::build),
    ("reclassify", nodes::reclassify::build),
    ("filter", nodes::filter::build),
    ("partition", nodes::partition::build),
    ("var_file", nodes::var_file::build),
    ("statistics_file", nodes::statistics_file::build),
    ("type", nodes::r#type::build),
];

pub fn factory(catalog_id: &str) -> Option<Factory> {
    BUILT_IN
        .iter()
        .find(|(id, _)| *id == catalog_id)
        .map(|(_, factory)| *factory)
}

pub fn is_implemented(catalog_id: &str) -> bool {
    factory(catalog_id).is_some()
}

pub fn implemented() -> impl Iterator<Item = &'static str> {
    BUILT_IN.iter().map(|(id, _)| *id)
}
