//! C5.0 boosting (`useBoosting`, `boostingTrials`). Training is rejected.
//! [`Reweighting`] applies the case-weight update between trials.
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

/// Same bounds as single-tree C5.0.
const MAX_CASES: usize = 100_000;
const MAX_CLASSES: usize = 1_024;

/// Case weights between C5.0 boosting trials.
///
/// Every case starts active and weighing 1, with no votes. Each
/// [`Self::apply_trial`] adds one vote, drops cases whose true class can no
/// longer win, moves the correct and wrong pools halfway toward half the
/// active weight, and rescales the active weights so they sum to the number
/// of active cases. Dropped cases stay inactive and weigh 0.
#[derive(Debug)]
pub struct Reweighting {
    weights: Vec<f64>,
    active: Vec<bool>,
    /// `votes[case * n_classes + class]`, one count per tree that predicted the class.
    votes: Vec<u32>,
    n_classes: usize,
}

impl Reweighting {
    /// `n_cases` active cases and `n_classes` target classes, all weights 1.
    ///
    /// # Errors
    ///
    /// The case count or the class count is outside the C5.0 limits, or the
    /// vote table would overflow.
    pub fn new(n_cases: usize, n_classes: usize) -> Result<Self, AlgoError> {
        if n_cases > MAX_CASES || n_classes == 0 || n_classes > MAX_CLASSES {
            return Err(AlgoError::Data(
                "C5.0 boosting reweighting case or class count is out of range".into(),
            ));
        }
        let vote_cells = n_cases.checked_mul(n_classes).ok_or_else(|| {
            AlgoError::Data("C5.0 boosting reweighting vote table is too large".into())
        })?;
        Ok(Self {
            weights: vec![1.0; n_cases],
            active: vec![true; n_cases],
            votes: vec![0; vote_cells],
            n_classes,
        })
    }

    /// Weight of every case, in case order. Dropped cases are 0.
    pub fn weights(&self) -> &[f64] {
        &self.weights
    }

    /// `true` while the case can still be drawn into a later trial.
    pub fn active(&self) -> &[bool] {
        &self.active
    }

    /// Applies the trial whose leaf scores are `predicted`.
    ///
    /// `predicted` and `actual` are class indices, one per case. `trials_left`
    /// is the number of trials still to be built after this one. A case is
    /// dropped when `votes[actual] + trials_left` is strictly less than the
    /// largest vote of another class.
    ///
    /// # Errors
    ///
    /// A slice length differs from the case count, a class index is out of
    /// range, a vote would overflow, or a pool weight is not finite.
    pub fn apply_trial(
        &mut self,
        predicted: &[usize],
        actual: &[usize],
        trials_left: usize,
    ) -> Result<(), AlgoError> {
        self.check_trial(predicted, actual)?;
        self.add_votes(predicted)?;
        self.drop_hopeless(actual, trials_left)?;
        let pools = self.pools(predicted, actual)?;
        self.move_pools(predicted, actual, &pools)?;
        self.rescale()
    }

    fn check_trial(&self, predicted: &[usize], actual: &[usize]) -> Result<(), AlgoError> {
        if predicted.len() != self.weights.len() || actual.len() != self.weights.len() {
            return Err(AlgoError::Data(
                "C5.0 boosting reweighting length does not match the case count".into(),
            ));
        }
        for (&pred, &truth) in predicted.iter().zip(actual.iter()) {
            crate::check_cancelled()?;
            if pred >= self.n_classes || truth >= self.n_classes {
                return Err(AlgoError::Data(
                    "C5.0 boosting class index is out of range".into(),
                ));
            }
        }
        Ok(())
    }

    fn add_votes(&mut self, predicted: &[usize]) -> Result<(), AlgoError> {
        for (case, &class) in predicted.iter().enumerate() {
            crate::check_cancelled()?;
            let slot = self.slot(case, class)?;
            let vote = self.votes.get_mut(slot).ok_or_else(|| {
                AlgoError::Data("C5.0 boosting vote index is out of range".into())
            })?;
            *vote = vote
                .checked_add(1)
                .ok_or_else(|| AlgoError::Data("C5.0 boosting vote count overflow".into()))?;
        }
        Ok(())
    }

    fn drop_hopeless(&mut self, actual: &[usize], trials_left: usize) -> Result<(), AlgoError> {
        let n_classes = self.n_classes;
        let votes = &self.votes;
        let mut dropping = Vec::new();
        for (case, (&active, &truth)) in self.active.iter().zip(actual).enumerate() {
            crate::check_cancelled()?;
            if !active {
                continue;
            }
            let start = case
                .checked_mul(n_classes)
                .ok_or_else(|| AlgoError::Data("C5.0 boosting vote index overflow".into()))?;
            let end = start
                .checked_add(n_classes)
                .ok_or_else(|| AlgoError::Data("C5.0 boosting vote index overflow".into()))?;
            let row = votes.get(start..end).ok_or_else(|| {
                AlgoError::Data("C5.0 boosting vote index is out of range".into())
            })?;
            if class_cannot_win(row, truth, trials_left) {
                dropping.push(case);
            }
        }
        for case in dropping {
            crate::check_cancelled()?;
            if let Some(flag) = self.active.get_mut(case) {
                *flag = false;
            }
            if let Some(weight) = self.weights.get_mut(case) {
                *weight = 0.0;
            }
        }
        Ok(())
    }

    fn pools(&self, predicted: &[usize], actual: &[usize]) -> Result<Pools, AlgoError> {
        let mut pools = Pools::default();
        for ((&active, &weight), (&pred, &truth)) in self
            .active
            .iter()
            .zip(self.weights.iter())
            .zip(predicted.iter().zip(actual.iter()))
        {
            crate::check_cancelled()?;
            if !active {
                continue;
            }
            if !weight.is_finite() {
                return Err(AlgoError::Data(
                    "C5.0 boosting case weight is not finite".into(),
                ));
            }
            if pred == truth {
                pools.correct_weight += weight;
                pools.correct_cases += 1;
            } else {
                pools.wrong_weight += weight;
                pools.wrong_cases += 1;
            }
        }
        if !pools.correct_weight.is_finite() || !pools.wrong_weight.is_finite() {
            return Err(AlgoError::Data(
                "C5.0 boosting class-pool weight is not finite".into(),
            ));
        }
        Ok(pools)
    }

    fn move_pools(
        &mut self,
        predicted: &[usize],
        actual: &[usize],
        pools: &Pools,
    ) -> Result<(), AlgoError> {
        let total = pools.correct_weight + pools.wrong_weight;
        if !total.is_finite() {
            return Err(AlgoError::Data(
                "C5.0 boosting active weight is not finite".into(),
            ));
        }
        let half = total / 2.0;
        let correct_factor = if pools.correct_cases == 0 {
            None
        } else if pools.correct_weight == 0.0 {
            return Err(AlgoError::Data(
                "C5.0 boosting correct pool has no weight".into(),
            ));
        } else {
            let factor = ((pools.correct_weight + half) / 2.0) / pools.correct_weight;
            if !factor.is_finite() {
                return Err(AlgoError::Data(
                    "C5.0 boosting correct-pool scale is not finite".into(),
                ));
            }
            Some(factor)
        };
        let wrong_gain = if pools.wrong_cases == 0 {
            None
        } else {
            let cases = pools.wrong_cases as f64;
            let gain = ((pools.wrong_weight + half) / 2.0 - pools.wrong_weight) / cases;
            if !gain.is_finite() {
                return Err(AlgoError::Data(
                    "C5.0 boosting wrong-pool gain is not finite".into(),
                ));
            }
            Some(gain)
        };
        for (case, (&active, (&pred, &truth))) in self
            .active
            .iter()
            .zip(predicted.iter().zip(actual.iter()))
            .enumerate()
        {
            crate::check_cancelled()?;
            if !active {
                continue;
            }
            if pred == truth {
                if let Some(factor) = correct_factor {
                    self.weights[case] *= factor;
                }
            } else if let Some(gain) = wrong_gain {
                self.weights[case] += gain;
            }
        }
        Ok(())
    }

    fn rescale(&mut self) -> Result<(), AlgoError> {
        let mut active_count = 0usize;
        let mut sum = 0.0;
        for (&active, &weight) in self.active.iter().zip(self.weights.iter()) {
            crate::check_cancelled()?;
            if !active {
                continue;
            }
            if !weight.is_finite() {
                return Err(AlgoError::Data(
                    "C5.0 boosting case weight is not finite".into(),
                ));
            }
            active_count += 1;
            sum += weight;
        }
        if active_count == 0 {
            return Ok(());
        }
        if sum == 0.0 || !sum.is_finite() {
            return Err(AlgoError::Data(
                "C5.0 boosting active weights cannot be rescaled".into(),
            ));
        }
        let scale = (active_count as f64) / sum;
        if !scale.is_finite() {
            return Err(AlgoError::Data(
                "C5.0 boosting rescale factor is not finite".into(),
            ));
        }
        for (active, weight) in self.active.iter().zip(self.weights.iter_mut()) {
            crate::check_cancelled()?;
            if *active {
                *weight *= scale;
                if !weight.is_finite() {
                    return Err(AlgoError::Data(
                        "C5.0 boosting case weight is not finite".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    fn slot(&self, case: usize, class: usize) -> Result<usize, AlgoError> {
        case.checked_mul(self.n_classes)
            .and_then(|base| base.checked_add(class))
            .ok_or_else(|| AlgoError::Data("C5.0 boosting vote index overflow".into()))
    }
}

/// `true` when `votes[truth] + trials_left` is strictly below the best other class.
fn class_cannot_win(votes: &[u32], truth: usize, trials_left: usize) -> bool {
    let mut best_other: Option<u32> = None;
    for (class, &count) in votes.iter().enumerate() {
        if class == truth {
            continue;
        }
        best_other = Some(best_other.map_or(count, |best| best.max(count)));
    }
    let Some(best_other) = best_other else {
        return false;
    };
    let mine = u64::from(votes.get(truth).copied().unwrap_or(0));
    let extra = u64::try_from(trials_left).unwrap_or(u64::MAX);
    mine.saturating_add(extra) < u64::from(best_other)
}

#[derive(Debug, Default)]
struct Pools {
    correct_weight: f64,
    wrong_weight: f64,
    correct_cases: usize,
    wrong_cases: usize,
}

pub fn train(
    _dataset: &Dataset<'_>,
    _settings: &C50Settings,
    _trials: usize,
) -> Result<Pmml, AlgoError> {
    crate::check_cancelled()?;
    Err(AlgoError::Unsupported("C5.0 boosting".into()))
}

#[cfg(test)]
mod tests {
    use super::{Reweighting, train};
    use crate::c50::C50Settings;
    use crate::{AlgoError, Dataset};
    use dms_types::{DataModel, Field, Measurement, Storage, Table, Value, ValueSpec};

    fn tiny_table() -> Table {
        let mut x = Field::new("x", Storage::Real);
        x.measurement = Measurement::Continuous;
        let mut y = Field::new("y", Storage::String);
        y.measurement = Measurement::Nominal;
        y.values = ValueSpec::Categories(vec![Value::string("a"), Value::string("b")]);
        Table::new(
            DataModel::new(vec![x, y]).unwrap(),
            vec![vec![Value::Real(0.0), Value::string("a")]],
        )
        .unwrap()
    }

    #[test]
    fn training_stays_unsupported() {
        let table = tiny_table();
        let dataset = Dataset {
            table: &table,
            target: 1,
            inputs: &[0],
            frequency: None,
        };
        let error = train(&dataset, &C50Settings::default(), 5).unwrap_err();
        assert!(matches!(error, AlgoError::Unsupported(message) if message == "C5.0 boosting"));
    }

    #[test]
    fn trial_one_starts_at_weight_one_and_rejects_malformed_input() {
        let weights = Reweighting::new(4, 3).unwrap();
        assert_eq!(weights.weights(), &[1.0, 1.0, 1.0, 1.0]);
        assert_eq!(weights.active(), &[true, true, true, true]);
        assert!(Reweighting::new(1, 0).is_err());
        assert!(Reweighting::new(100_001, 2).is_err());
        assert!(Reweighting::new(1, 1_025).is_err());

        let mut weights = Reweighting::new(2, 2).unwrap();
        assert!(weights.apply_trial(&[0], &[0, 0], 0).is_err());
        assert!(weights.apply_trial(&[2, 0], &[0, 0], 0).is_err());
        assert!(weights.apply_trial(&[0, 0], &[0, 3], 0).is_err());
        assert_eq!(weights.weights(), &[1.0, 1.0]);
        assert_eq!(weights.active(), &[true, true]);
    }

    #[test]
    fn pools_move_halfway_toward_half_the_weight() {
        // Three classes, one trial left: a wrong case is one vote behind, which
        // ties the trials it can still win, so nobody is dropped.
        let mut weights = Reweighting::new(3, 3).unwrap();
        weights.apply_trial(&[0, 0, 0], &[0, 1, 2], 1).unwrap();
        assert_eq!(weights.active(), &[true, true, true]);
        // C = 1, E = 2, T = 3. Correct scale (1 + 1.5) / 2 / 1 = 1.25.
        // Each wrong case gains ((2 + 1.5) / 2 - 2) / 2 = -0.125.
        assert_eq!(weights.weights(), &[1.25, 0.875, 0.875]);
        let sum: f64 = weights.weights().iter().sum();
        assert_eq!(sum, 3.0);
    }

    #[test]
    fn a_tie_keeps_the_case_and_a_deficit_drops_it() {
        let mut kept = Reweighting::new(1, 2).unwrap();
        // Predicted 0, truth 1, one trial left: 0 + 1 == 1, keep.
        kept.apply_trial(&[0], &[1], 1).unwrap();
        assert_eq!(kept.active(), &[true]);
        assert_eq!(kept.weights(), &[1.0]);

        let mut dropped = Reweighting::new(1, 2).unwrap();
        // No trials left: 0 + 0 < 1, drop for good.
        dropped.apply_trial(&[0], &[1], 0).unwrap();
        assert_eq!(dropped.active(), &[false]);
        assert_eq!(dropped.weights(), &[0.0]);
    }

    #[test]
    fn dropped_weight_is_rescaled_onto_the_remaining_cases() {
        let mut weights = Reweighting::new(3, 3).unwrap();
        weights.apply_trial(&[0, 0, 0], &[0, 1, 2], 1).unwrap();
        // Case 1 is now two votes behind with nothing left, so it drops.
        // Cases 0 and 2 are correct and keep unequal weights, then rescale to 2.
        weights.apply_trial(&[0, 0, 2], &[0, 1, 2], 0).unwrap();
        assert_eq!(weights.active(), &[true, false, true]);
        assert_eq!(weights.weights()[1], 0.0);
        let sum: f64 = weights.weights().iter().sum();
        assert!((sum - 2.0).abs() < 1e-12, "sum {sum}");
        assert!(
            (weights.weights()[0] - 20.0 / 17.0).abs() < 1e-9,
            "correct {}",
            weights.weights()[0]
        );
        assert!(
            (weights.weights()[2] - 14.0 / 17.0).abs() < 1e-9,
            "other {}",
            weights.weights()[2]
        );

        weights.apply_trial(&[1, 1, 1], &[0, 1, 2], 5).unwrap();
        assert!(!weights.active()[1]);
        assert_eq!(weights.weights()[1], 0.0);
    }

    #[test]
    fn one_class_never_drops() {
        let mut weights = Reweighting::new(2, 1).unwrap();
        weights.apply_trial(&[0, 0], &[0, 0], 0).unwrap();
        assert_eq!(weights.active(), &[true, true]);
        assert_eq!(weights.weights(), &[1.0, 1.0]);
    }
}
