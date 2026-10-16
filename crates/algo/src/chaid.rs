//! CHAID (Algorithms Guide: "CHAID Algorithms"). Placeholder until the algorithm lands.

use crate::{AlgoError, Dataset};
use dms_pmml::Pmml;

/// Settings of a CHAID build node.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChaidSettings {}

/// Trains a CHAID model.
pub fn train(_dataset: &Dataset<'_>, _settings: &ChaidSettings) -> Result<Pmml, AlgoError> {
    Err(AlgoError::Unsupported("CHAID".into()))
}
