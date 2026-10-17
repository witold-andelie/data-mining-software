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
//! - Ordinal fields split as for single trees (`c50`).
//!
//! Trial trees are not case-weighted trees: `c50_weighted/c50_w2_default` grows a tree on
//! trial 2's weights as case weights and splits `x <= 5` and `x = 8, flag = T`, which
//! trial 2 leaves whole, and it merges same-class sibling leaves, which trials do not.
//! With merging off and a minimum branch weight from 10 to 12 (instead of 2) our learner
//! reproduces trial 2 of `c50_boost_tree` exactly; the oracles bound it only to
//! (9.24, 12.475], so the rule behind it is a GAP.
//!
//! GAP (blocks training): from trial 3 on the exported trees prune more than that and
//! carry all-zero class distributions on some leaves while keeping their record counts
//! (also seen in `c50_weight`), and the tree that reweights the cases is not always the
//! exported one: in `c50_boost_tree` trial 3 the exported `blue, x <= 7 -> C` leaf
//! reweights as `B`, and in `c50_boost_t10` trial 4 the exported leaf `flag = F, x > 7
//! -> B` reweights one value of `x` as `C`. Boosted rule sets stay rejected too.

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
