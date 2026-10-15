//! Built-in node implementations, by catalog id. Parity (`cargo xtask parity`) counts exactly
//! these entries, so an id belongs here only once its node works and is tested.

use crate::nodes;
use crate::operator::Factory;

const BUILT_IN: &[(&str, Factory)] = &[("table", nodes::table::build)];

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
