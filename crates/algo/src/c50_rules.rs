//! C5.0 rule sets (`modelFormat` `ruleSet`): rules derived from the tree, simplified and
//! selected, exported as a PMML RuleSet (`oracle/models_ml/c50_rules_voting`,
//! `c50_rules_firsthit`). Placeholder: not implemented yet.

use crate::c50::C50Settings;
use crate::{AlgoError, Dataset};
use dms_pmml::Pmml;

pub fn train(_dataset: &Dataset<'_>, _settings: &C50Settings) -> Result<Pmml, AlgoError> {
    Err(AlgoError::Unsupported("C5.0 rule-set training".into()))
}
