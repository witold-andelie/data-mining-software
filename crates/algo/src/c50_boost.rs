//! C5.0 boosting (`useBoosting`, `boostingTrials`). Not implemented: training is rejected.
//!
//! Modeler exports one PMML `TreeModel` per trial, one after another (`models_ml 1`, ...),
//! each with an `spss` `boost` extension. Evidence: `oracle/models_ml/c50_boost_tree`
//! (5 trials) and `oracle/c50_boost/*` (probe `c50_boost_streams`: 2, 3, 4, 10 trials,
//! reversed records, inputs x/cat/flag only). Settled by replaying Modeler's exported
//! trees over the training data and matching every exported leaf count:
//! - Trial 1 is the ordinary tree; every case weighs 1.
//! - After a trial, the cases it classifies correctly and those it misclassifies each
//!   form a pool. Both pools move halfway to half the total weight: correct cases are
//!   scaled by `(C + T/2) / 2 / C`, and each misclassified case gains the same amount,
//!   `((E + T/2) / 2 - E) / errors`.
//! - A case that can no longer win the vote is dropped for all later trials: counting
//!   one vote per tree, its class trails another class by more than the trials left
//!   (a possible tie keeps it). The remaining weights are then rescaled to sum to the
//!   number of remaining cases (5 trials: 383 after trial 3, 376 after trial 4).
//!   Record order does not matter (`c50_boost_reverse` equals `c50_boost_tree`).
//! - Hence a tree depends on the trial count: trees 1-3 of a 3-trial run differ from
//!   those of a 4-trial run. `boostingTrials` 2 builds three trees.
//! - Ordinal fields split as ordered binary cuts over their values in sorted order
//!   (`ord` high < low < middle: `ord in {high, low}` versus `{middle}`).
//!
//! GAP (blocks training): the trial trees are grown on non-integer weights, the same
//! unresolved weighted-tree learner as `c50_weight` (see `c50`). With the exact
//! trial-2 weights our learner still splits two nodes Modeler leaves whole (`x <= 5`,
//! and `x = 8` under `flag = T`), although neither the gains nor any error bound
//! (weighted or by case count) explains it. Weighted trees also export leaves whose
//! class distribution is all zero while their record count is kept, and from trial 3
//! on the tree that reweights the cases is not always the exported one: in
//! `c50_boost_tree` trial 3 the exported `blue, x <= 7 -> C` leaf reweights as `B`, and
//! in `c50_boost_t10` trial 4 the exported leaf `flag = F, x > 7 -> B` reweights one
//! value of `x` as `C`. Boosted rule sets stay rejected too.

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
