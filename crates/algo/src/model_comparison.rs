//! Model comparison metrics used by the Auto Classifier and Auto Numeric builders to rank and
//! filter their component models.
//!
//! Flag targets follow the Algorithms Guide, "Binary Classifier Comparison Metrics" (pp. 49-50):
//!
//! - overall accuracy: the share of records whose predicted value equals the observed one;
//! - AUC: records are grouped into segments of equal predicted probability `p̂` of the true
//!   value, sorted by descending `p̂`; with cumulative false and true positive counts `f_i`, `t_i`
//!   the area is `Σ |f_i - f_(i-1)| (t_i + t_(i-1)) / 2`, here divided by `N·P` so that it is the
//!   unit-square area the builder saves (`auc`);
//! - lift at `q` %: the expected response rate among the top `k = q % · n` records by `p̂`, over
//!   the overall response rate. The Guide prints the mean `p̂` of those records as the numerator;
//!   Modeler ranks by hits: on `auto_train/ac_lift`'s saved components the mean-`p̂` lift puts
//!   Decision List (1.7506) above C5.0 (1.7334), Modeler's order and the hit rate (1.7598) the
//!   other way. Records tied on `p̂` count with their segment's hit share, and `k` stays
//!   fractional: the expected hits under a random order within ties (McSherry & Najork, ECIR
//!   2008), i.e. linear interpolation of the cumulative gains curve. `auto_lift` decides it with
//!   Modeler's own `ac_lift` components at q = 48..80 of 360 records: floor, ceiling and rounding
//!   of `k` each contradict at least one ranking, interpolation reproduces eight of nine, five of
//!   them predicted before the run (`ac_lift_q58`, `q61`, `q63`, `q78`, `q80`). The exception,
//!   `ac_lift_q57`, is a near tie: C5.0 1.622011 versus Logistic 1.622088 (Modeler keeps C5.0
//!   first); no simple rule explains it, and it is left unfitted;
//! - maximum profit: the largest cumulative `Σ (h(x_i)·r - c)` over prefixes of the records by
//!   descending `p̂`, ending at segment boundaries (0 for the empty prefix). The Guide's stop at
//!   `p̂·r - c ≤ 0` gives Logistic 685 and C5.0 700 on `ac_lift`, where Modeler ranks Logistic
//!   first (`ac_profit_n2`); the segment maximum gives 705 and 700. A per-record maximum ties
//!   them at 705.
//!
//! Continuous targets (Auto Numeric `trainPartitionStats`): the Pearson correlation between the
//! prediction and the target, and the relative error `Σ(y - ŷ)² / Σ(y - ȳ)²`.

use crate::AlgoError;

/// One scored training record of a flag target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlagScore {
    /// The observed value is the target's true value (a hit).
    pub hit: bool,
    /// The model's predicted value equals the observed one.
    pub correct: bool,
    /// The model's probability of the true value (`p̂`, the raw propensity).
    pub propensity: f64,
}

fn count(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

fn check_flag(scores: &[FlagScore]) -> Result<(), AlgoError> {
    if scores.is_empty() {
        return Err(AlgoError::Data(
            "no scored records to compare models on".into(),
        ));
    }
    if scores
        .iter()
        .any(|s| !s.propensity.is_finite() || !(0.0..=1.0).contains(&s.propensity))
    {
        return Err(AlgoError::Data(
            "a component's propensity is missing or outside [0, 1]".into(),
        ));
    }
    Ok(())
}

pub fn accuracy(scores: &[FlagScore]) -> Result<f64, AlgoError> {
    check_flag(scores)?;
    Ok(count(scores.iter().filter(|s| s.correct).count()) / count(scores.len()))
}

/// Records sorted by descending `p̂`; ties keep their record order.
fn by_propensity(scores: &[FlagScore]) -> Vec<FlagScore> {
    let mut sorted = scores.to_vec();
    sorted.sort_by(|a, b| b.propensity.total_cmp(&a.propensity));
    sorted
}

pub fn auc(scores: &[FlagScore]) -> Result<f64, AlgoError> {
    check_flag(scores)?;
    let positives = scores.iter().filter(|s| s.hit).count();
    let negatives = scores.len() - positives;
    if positives == 0 || negatives == 0 {
        return Err(AlgoError::Data(
            "AUC needs both target values among the records".into(),
        ));
    }
    let sorted = by_propensity(scores);
    let (mut f, mut t, mut area) = (0usize, 0usize, 0.0);
    let mut i = 0;
    while let Some(first) = sorted.get(i) {
        let (f0, t0) = (f, t);
        while let Some(s) = sorted.get(i).filter(|s| s.propensity == first.propensity) {
            if s.hit {
                t += 1;
            } else {
                f += 1;
            }
            i += 1;
        }
        area += count(f - f0) * (count(t) + count(t0)) / 2.0;
    }
    Ok(area / (count(positives) * count(negatives)))
}

/// Lift at `percentile` % (1..=100) with `k = percentile · n / 100` records, fractional `k`
/// and ties shared by their segment's hit rate (module docs).
pub fn lift(scores: &[FlagScore], percentile: u32) -> Result<f64, AlgoError> {
    check_flag(scores)?;
    if !(1..=100).contains(&percentile) {
        return Err(AlgoError::Settings(format!(
            "lift percentile {percentile} is outside 1..=100"
        )));
    }
    let n = count(scores.len());
    let k = f64::from(percentile) * n / 100.0;
    let positives = scores.iter().filter(|s| s.hit).count();
    if positives == 0 || k <= 0.0 {
        return Err(AlgoError::Data("lift needs a true target value".into()));
    }
    let sorted = by_propensity(scores);
    let (mut taken, mut hits, mut i) = (0.0, 0.0, 0);
    while let Some(first) = sorted.get(i).filter(|_| taken < k) {
        let (mut size, mut segment_hits) = (0usize, 0usize);
        while let Some(s) = sorted.get(i).filter(|s| s.propensity == first.propensity) {
            size += 1;
            segment_hits += usize::from(s.hit);
            i += 1;
        }
        let take = count(size).min(k - taken);
        hits += take * count(segment_hits) / count(size);
        taken += take;
    }
    Ok((hits / k) / (count(positives) / n))
}

/// Maximum profit for `revenue` per hit and `cost` per record.
pub fn max_profit(scores: &[FlagScore], revenue: f64, cost: f64) -> Result<f64, AlgoError> {
    check_flag(scores)?;
    if !revenue.is_finite() || !cost.is_finite() {
        return Err(AlgoError::Settings(
            "profit revenue and cost must be finite".into(),
        ));
    }
    let sorted = by_propensity(scores);
    let (mut best, mut running) = (0.0f64, 0.0f64);
    let mut i = 0;
    while let Some(first) = sorted.get(i) {
        while let Some(s) = sorted.get(i).filter(|s| s.propensity == first.propensity) {
            running += if s.hit { revenue - cost } else { -cost };
            i += 1;
        }
        best = best.max(running);
    }
    Ok(best)
}

/// Pearson correlation of `(prediction, observed)` pairs.
pub fn correlation(pairs: &[(f64, f64)]) -> Result<f64, AlgoError> {
    let n = check_pairs(pairs)?;
    let (mp, mo) = (
        pairs.iter().map(|p| p.0).sum::<f64>() / n,
        pairs.iter().map(|p| p.1).sum::<f64>() / n,
    );
    let (mut sp, mut so, mut spo) = (0.0, 0.0, 0.0);
    for (p, o) in pairs {
        sp += (p - mp).powi(2);
        so += (o - mo).powi(2);
        spo += (p - mp) * (o - mo);
    }
    if sp <= 0.0 || so <= 0.0 {
        return Err(AlgoError::Data(
            "correlation of a constant prediction or target".into(),
        ));
    }
    Ok(spo / (sp * so).sqrt())
}

/// Relative error `Σ(o - p)² / Σ(o - ō)²` of `(prediction, observed)` pairs.
pub fn relative_error(pairs: &[(f64, f64)]) -> Result<f64, AlgoError> {
    let n = check_pairs(pairs)?;
    let mean = pairs.iter().map(|p| p.1).sum::<f64>() / n;
    let total: f64 = pairs.iter().map(|p| (p.1 - mean).powi(2)).sum();
    if total <= 0.0 {
        return Err(AlgoError::Data(
            "relative error of a constant target".into(),
        ));
    }
    Ok(pairs.iter().map(|p| (p.1 - p.0).powi(2)).sum::<f64>() / total)
}

fn check_pairs(pairs: &[(f64, f64)]) -> Result<f64, AlgoError> {
    if pairs.len() < 2 || pairs.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return Err(AlgoError::Data(
            "comparison needs two or more finite predictions".into(),
        ));
    }
    Ok(count(pairs.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(hit: bool, propensity: f64) -> FlagScore {
        FlagScore {
            hit,
            correct: hit == (propensity > 0.5),
            propensity,
        }
    }

    #[test]
    fn auc_counts_ties_as_half() {
        // Worked by hand: a hit tied with a miss on p̂ counts half.
        let perfect = [score(true, 0.9), score(true, 0.8), score(false, 0.2)];
        assert_eq!(auc(&perfect).unwrap(), 1.0);
        let tied = [score(true, 0.5), score(false, 0.5)];
        assert_eq!(auc(&tied).unwrap(), 0.5);
        let mixed = [
            score(true, 0.9),
            score(false, 0.6),
            score(true, 0.4),
            score(false, 0.1),
        ];
        assert_eq!(auc(&mixed).unwrap(), 0.75);
    }

    #[test]
    fn lift_profit_and_accuracy() {
        let scores = [
            score(true, 0.9),
            score(true, 0.7),
            score(false, 0.6),
            score(false, 0.3),
            score(true, 0.2),
        ];
        // Top 40 % = 2 records, both hits, response rate 3/5.
        assert!((lift(&scores, 40).unwrap() - 1.0 / 0.6).abs() < 1e-15);
        // 30 % of 5 = 1.5 records: the 0.9 hit and half of the 0.7 hit.
        assert!((lift(&scores, 30).unwrap() - (1.5 / 1.5) / 0.6).abs() < 1e-15);
        // r = 10, c = 5: cumulative +5 +10 +5 0 +5, best 10.
        assert_eq!(max_profit(&scores, 10.0, 5.0).unwrap(), 10.0);
        // A tie at the cut counts its share: top 50 % of four, the last two tied 1 hit of 2.
        let tied = [
            score(true, 0.9),
            score(false, 0.6),
            score(true, 0.6),
            score(false, 0.1),
        ];
        assert!((lift(&tied, 50).unwrap() - (1.5 / 2.0) / 0.5).abs() < 1e-15);
        assert_eq!(accuracy(&scores).unwrap(), 0.6);
    }

    #[test]
    fn numeric_measures() {
        let pairs = [(1.0, 1.0), (2.0, 2.5), (3.0, 2.5)];
        // Observed mean 2: Σ(o - ō)² = 1.5, Σ(o - p)² = 0.5.
        assert!((relative_error(&pairs).unwrap() - 1.0 / 3.0).abs() < 1e-15);
        assert!(correlation(&[(1.0, 2.0), (2.0, 4.0), (3.0, 6.0)]).unwrap() > 1.0 - 1e-15);
        assert!(correlation(&[(1.0, 1.0), (1.0, 2.0)]).is_err());
    }
}
