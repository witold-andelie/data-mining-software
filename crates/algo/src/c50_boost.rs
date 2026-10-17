//! C5.0 boosting (`useBoosting`, `boostingTrials`): trials of trees on reweighted cases,
//! exported as a PMML MiningModel (`oracle/models_ml/c50_boost_tree`). Placeholder: not
//! implemented yet.

use crate::c50::C50Settings;
use crate::{AlgoError, Dataset};
use dms_pmml::Pmml;

pub fn train(
    _dataset: &Dataset<'_>,
    _settings: &C50Settings,
    _trials: usize,
) -> Result<Pmml, AlgoError> {
    Err(AlgoError::Unsupported("C5.0 boosting".into()))
}
