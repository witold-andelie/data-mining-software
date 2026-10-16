//! C&R Tree (Algorithms Guide: "CART Algorithms"). Placeholder until the algorithm lands.

use crate::{AlgoError, Dataset};
use dms_pmml::Pmml;

/// Settings of a C&R Tree build node.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CartSettings {}

/// Trains a C&R Tree model.
pub fn train(_dataset: &Dataset<'_>, _settings: &CartSettings) -> Result<Pmml, AlgoError> {
    Err(AlgoError::Unsupported("C&R Tree".into()))
}
