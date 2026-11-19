//! Self-Learning Response Model (IBM SPSS Modeler 19 Algorithms Guide,
//! "Self-Learning Response Model Algorithms", pp. 337–341).
//!
//! Pure algorithm module: the caller supplies plain column vectors ([`SlrmData`])
//! and [`Settings`]. There is no dependency on [`crate::Dataset`]. Training
//! builds one Naive Bayes classifier per product (target category) for the
//! binary response flag, bins continuous predictors, and can update the stored
//! cell counts from new records without a full rebuild. Scoring returns a
//! per-product response probability and a ranked offer list.
//!
//! Formulas are taken from the page images `target/grok-slrm/pages/guide_p337.png`
//! … `guide_p341.png`. The chapter text `target/grok-slrm/guide_slrm_chapter.txt`
//! is used for prose only. Continuous binning is not in the Guide; it is read
//! from the saved Modeler model.
//!
//! # Decisions the Guide does not settle
//!
//! Fixed and verified against the saved Modeler model
//! (`target/grok-slrm/model/saved_slrm_model.pmml.txt`, SPSS Self Learning
//! Component 1.0, PMML 3.2 `NaiveBayesModel`, trained from the `pm_selflearn`
//! demo). The file has a 4-byte big-endian length prefix then XML. Four
//! products: Mortgage, Car loan, Savings, Pension. Attachment 3
//! (`saved_attachment_3.txt`) holds `SLContVar` min/max and 10-bin encoders.
//!
//! A. **The saved counts are train1 + train2 + train3 (22 465 records).**
//!    Mortgage `trainingSampleSize=1838` with `TargetValueCount` 1=467, 0=1371
//!    equals train1+2 campaign `1` (1834 = 463/1371) plus the four Mortgage
//!    rows of train3. Car loan 13951 = 13928+23. Savings 5285 and Pension 1391
//!    are train1 only. Reproduces the saved Modeler model: every
//!    `TargetValueCounts` total matches.
//! B. **One Naive Bayes per product, target = `response` (0/1).** Each PMML
//!    `NaiveBayesModel/@modelName` is a product; `BayesOutput/@fieldName` is
//!    `response`. Predictors are the 19 continuous fields plus `branch`,
//!    `gender`, `marital`, `months_customer`. Reproduces the saved Modeler
//!    model (four models, 23 `BayesInput`s each).
//! C. **Continuous predictors use 10 equal-width bins on the first training
//!    pass min/max.** Attachment 3 `SLContVar` for `age` is min=10, max=96,
//!    `Int 10`; cuts `18.6, 27.2, …, 87.4` equal `(max-min)/10`. Income max
//!    78675 is train1 max (train2/train3 reach 88016). Intervals are
//!    `openClosed` `(L, R]` except the last `openOpen` `(R, +∞)`. Reproduces
//!    the saved Modeler model: every uncollapsed 10-bin `Discretize` matches.
//! D. **Empty-bin collapse only when both response classes occur.** Mortgage
//!    and Car loan (both `N_k > 0`) drop leading/trailing empty original bins
//!    and replace each internal empty run from cut `L` through cut `R` with
//!    `(L+R)/2`. Mortgage `age` original cuts include 78.8 and 87.4; the
//!    saved model has 83.1. `purchase` collapses to one cut at 0.5. Savings
//!    and Pension (all `response=0`) keep all 10 original cuts. Reproduces
//!    the saved Modeler model: every `Discretize` cut on all four products
//!    matches after train(train1)+update(train2+train3).
//! E. **PairCounts list occupied bins as `0..k-1` in occupancy order.** After
//!    collapse this matches `DiscretizeBin/@binValue`. Without collapse
//!    (Savings `bad_payment`) original bin 9 is exported as `value="1"`.
//!    Reproduces the saved Modeler model: every `PairCounts` count matches.
//! F. **`λ = f = 1/N` per product, `K = 2`.** PMML `@threshold` is
//!    `1/1838` (Mortgage), `1/13951` (Car loan), and the Savings/Pension
//!    reciprocals. Attachment 3 `π_1 = (467+λ)/(1838+2λ)` and purchase
//!    `p(bin=0 | y=1) = f / (467 + 2f)` so `M_j` is the collapsed bin count.
//!    Reproduces the saved Modeler model.
//! G. **Missing predictor values are pairwise.** Car loan
//!    `age_youngest_child` pair total is 13950 against `N = 13951` (one
//!    blank in train2). Reproduces the saved Modeler model.
//! H. **A case with missing target or missing response, or with every
//!    predictor missing, is ignored** (Guide p. 339). Constant predictors
//!    are **kept** (Savings `gold_card` is all 0 and still a `BayesInput`).
//! I. **Continue-training freezes min/max and original 10-bin edges, then
//!    re-derives collapse from current occupancy.** After train2 fills Car
//!    loan `average#balance#feed#index` original bin 8, the saved model has
//!    all 10 uncollapsed cuts. Reproduces the saved Modeler model on the
//!    train-then-update sequence.
//!
//! Follows the Algorithms Guide (unverified against a saved Modeler model):
//!
//! 1. **Scoring posterior** (p. 337–338) with the estimates in F, omitting a
//!    predictor that is missing or that falls in a zero-count bin.
//! 2. **Offer score** (p. 340)
//!    `P_i = [α P_{ri} + (1-α)(W_i/(W_i+W_emp) P + W_emp/(W_i+W_emp)·0.5)]
//!    · S_i / max(S) · F_i` with `W_emp = 500` when reliability is on, else
//!    0. Default `α = 0`, `S_i = 1`, `F_i = 1`. Rank `max_predictions`
//!    offers. Tie-break: product index.
//! 3. **Information measure** `I_{y,x} = M(Y,X) / H_Y` (p. 341) from the
//!    unsmoothed pair counts.
//!
//! Explicit `GAP:` errors:
//!
//! - Model assessment by simulation (p. 338–339) and predictor importance
//!   by simulated leave-one-predictor-out (p. 340): the Guide does not
//!   specify the RNG.
//! - Non-integer frequency weights: the saved model has none.
//! - Randomisation scores `P_ri` use a documented LCG Uniform(0,1) when
//!   `α > 0` and are unverified; they are not a `GAP:` error.

use crate::AlgoError;
use std::fmt::Write as _;

/// Maximum records in one `train` / `update` call.
const MAX_RECORDS: usize = 1_000_000;
/// Maximum numeric + nominal predictors.
const MAX_PREDICTORS: usize = 512;
/// Maximum products (target categories).
const MAX_PRODUCTS: usize = 1_024;
/// Maximum categories on one nominal predictor or on the target.
const MAX_CATEGORIES: usize = 1_000;
/// Maximum equal-width bins for a continuous predictor.
const MAX_BINS: usize = 100;
/// Default equal-width bin count (attachment 3 `SLContVar`).
const DEFAULT_BINS: usize = 10;
/// Guide p. 339: `W_emp` when “Take account of model reliability” is on.
const RELIABILITY_CASES: f64 = 500.0;
/// Two response classes, even when one count is 0.
const N_CLASS: usize = 2;

/// Error returned by SLRM training and scoring.
pub type SlrmError = AlgoError;

fn gap(what: &str) -> SlrmError {
    AlgoError::Unsupported(format!("GAP: SLRM {what}"))
}

fn data_err(message: impl Into<String>) -> SlrmError {
    AlgoError::Data(message.into())
}

fn settings_err(message: impl Into<String>) -> SlrmError {
    AlgoError::Settings(message.into())
}

/// A continuous predictor column (`None` = missing).
#[derive(Debug, Clone, PartialEq)]
pub struct NumericColumn {
    /// Field name.
    pub name: String,
    /// Per-record values.
    pub values: Vec<Option<f64>>,
}

/// A nominal column: zero-based codes into `labels`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NominalColumn {
    /// Field name.
    pub name: String,
    /// Category labels in code order.
    pub labels: Vec<String>,
    /// Per-record codes (`None` = missing).
    pub codes: Vec<Option<u32>>,
}

/// Training table: numeric predictors, nominal predictors, product target, response flag.
///
/// All columns must have the same length. The caller converts from a dataset.
#[derive(Debug, Clone, PartialEq)]
pub struct SlrmData {
    /// Continuous predictors, in caller order.
    pub numeric: Vec<NumericColumn>,
    /// Nominal predictors, in caller order.
    pub nominal: Vec<NominalColumn>,
    /// Product / offer field (nominal or flag).
    pub target: NominalColumn,
    /// Response flag, aligned with the rows (`true` = responded).
    pub response: Vec<Option<bool>>,
    /// Optional non-negative integer frequency, one per row.
    pub frequency: Option<Vec<Option<u32>>>,
}

/// A query record aligned with the training predictors.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    /// Numeric predictor values, same order as [`SlrmData::numeric`].
    pub numeric: Vec<Option<f64>>,
    /// Nominal codes, same order as [`SlrmData::nominal`].
    pub nominal: Vec<Option<u32>>,
}

/// Rank order of the scored offers (Guide p. 340).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    /// Highest score first (the saved stream).
    Descending,
    /// Lowest score first.
    Ascending,
}

/// Build-node and scoring settings. [`Default`] matches the saved `pm_selflearn` node.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Equal-width bins for each continuous predictor (default 10).
    pub n_bins: usize,
    /// Randomisation factor `α` in `[0, 1]` (default 0).
    pub randomization: f64,
    /// Seed for `P_ri` when `randomization > 0` (unverified LCG).
    pub scoring_random_seed: u32,
    /// If true, shrink each probability toward 0.5 with `W_emp = 500`.
    pub model_reliability: bool,
    /// How many ranked offers to return (default 2).
    pub max_predictions: usize,
    /// Rank order of offers (default descending).
    pub sort: SortOrder,
    /// Per-product preferences `S_i` (empty = all 1).
    pub preferences: Vec<f64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            n_bins: DEFAULT_BINS,
            randomization: 0.0,
            scoring_random_seed: 876_547,
            model_reliability: true,
            max_predictions: 2,
            sort: SortOrder::Descending,
            preferences: Vec::new(),
        }
    }
}

/// Optional per-query include/exclude filters `F_i` (Guide p. 340).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScoreOpts<'a> {
    /// `F_i ∈ {0,1}`; length must match the number of products. `None` = all included.
    pub include: Option<&'a [bool]>,
    /// Override stored preferences. `None` uses [`Settings::preferences`].
    pub preferences: Option<&'a [f64]>,
}

/// One ranked offer from [`SlrmModel::score`].
#[derive(Debug, Clone, PartialEq)]
pub struct OfferScore {
    /// Index into [`SlrmModel::product_name`].
    pub product_index: usize,
    /// Product label.
    pub product_label: String,
    /// Naive Bayes `P(response = 1 | x, product)`.
    pub probability: f64,
    /// Final offer score `P_i` (Guide p. 340), also the reported confidence.
    pub score: f64,
}

/// Occupied-bin pair counts as the saved PMML writes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairCount {
    /// Exported bin index or category label.
    pub value: String,
    /// Count of `response = 1`.
    pub n1: u64,
    /// Count of `response = 0`.
    pub n0: u64,
}

/// Trained SLRM: one Naive Bayes per product, frozen numeric ranges, cell counts.
#[derive(Debug, Clone, PartialEq)]
pub struct SlrmModel {
    settings: Settings,
    numeric_names: Vec<String>,
    numeric_min: Vec<f64>,
    numeric_max: Vec<f64>,
    nominal_names: Vec<String>,
    nominal_labels: Vec<Vec<String>>,
    products: Vec<Product>,
}

#[derive(Debug, Clone, PartialEq)]
struct Product {
    label: String,
    /// `N_0`, `N_1`.
    n: [u64; 2],
    /// `[field][orig_bin][class]`.
    numeric: Vec<Vec<[u64; 2]>>,
    /// `[field][category][class]`.
    nominal: Vec<Vec<[u64; 2]>>,
}

/// Train a new SLRM on `data`.
///
/// Numeric min/max (and therefore the original equal-width edges) are taken
/// from this table and frozen. Empty-bin collapse is derived from occupancy
/// after the counts are collected.
pub fn train(data: &SlrmData, settings: &Settings) -> Result<SlrmModel, SlrmError> {
    crate::check_cancelled()?;
    validate_settings(settings)?;
    let n = data_len(data)?;
    if n == 0 {
        return Err(data_err("SLRM training table is empty"));
    }
    if n > MAX_RECORDS {
        return Err(data_err(format!(
            "SLRM training table has {n} records, maximum {MAX_RECORDS}"
        )));
    }
    check_schema_limits(data)?;
    let (numeric_min, numeric_max) = numeric_ranges(data, settings.n_bins)?;
    let mut model = SlrmModel {
        settings: settings.clone(),
        numeric_names: data.numeric.iter().map(|c| c.name.clone()).collect(),
        numeric_min,
        numeric_max,
        nominal_names: data.nominal.iter().map(|c| c.name.clone()).collect(),
        nominal_labels: data.nominal.iter().map(|c| c.labels.clone()).collect(),
        products: empty_products(&data.target.labels, data, settings.n_bins)?,
    };
    accumulate(&mut model, data)?;
    Ok(model)
}

/// Add `data` to an existing model (Guide p. 339: one pass of the new records).
///
/// Numeric min/max stay as they were at [`train`]. New products (extra target
/// labels) are appended. Collapse is re-derived from the updated occupancy.
pub fn update(model: &mut SlrmModel, data: &SlrmData) -> Result<(), SlrmError> {
    crate::check_cancelled()?;
    let n = data_len(data)?;
    if n > MAX_RECORDS {
        return Err(data_err(format!(
            "SLRM update table has {n} records, maximum {MAX_RECORDS}"
        )));
    }
    check_update_schema(model, data)?;
    extend_products(model, data)?;
    accumulate(model, data)
}

impl SlrmModel {
    /// Number of products (target categories) stored.
    pub fn n_products(&self) -> usize {
        self.products.len()
    }

    /// Label of product `index`.
    pub fn product_name(&self, index: usize) -> Option<&str> {
        self.products.get(index).map(|p| p.label.as_str())
    }

    /// Continuous predictor names, in caller order.
    pub fn numeric_names(&self) -> &[String] {
        &self.numeric_names
    }

    /// Nominal predictor names, in caller order.
    pub fn nominal_names(&self) -> &[String] {
        &self.nominal_names
    }

    /// Frozen min and max of numeric field `field` (first training pass).
    pub fn numeric_range(&self, field: usize) -> Option<(f64, f64)> {
        let lo = self.numeric_min.get(field).copied()?;
        let hi = self.numeric_max.get(field).copied()?;
        Some((lo, hi))
    }

    /// `(N_0, N_1)` for product `product`.
    pub fn target_counts(&self, product: usize) -> Option<(u64, u64)> {
        let p = self.products.get(product)?;
        let n0 = p.n.first().copied()?;
        let n1 = p.n.get(1).copied()?;
        Some((n0, n1))
    }

    /// `N = N_0 + N_1` for product `product`.
    pub fn n_cases(&self, product: usize) -> Option<u64> {
        let (n0, n1) = self.target_counts(product)?;
        n0.checked_add(n1)
    }

    /// `λ = f = 1/N` for product `product` (PMML `@threshold`).
    pub fn threshold(&self, product: usize) -> Option<f64> {
        let n = self.n_cases(product)?;
        if n == 0 {
            return None;
        }
        Some(1.0 / n as f64)
    }

    /// `Discretize` interior cuts for product `product`, numeric field `field`.
    ///
    /// Length `k` means `k + 1` bins. Both-class products use collapsed cuts;
    /// single-class products use the original `n_bins - 1` equal-width cuts.
    pub fn numeric_cuts(&self, product: usize, field: usize) -> Result<Vec<f64>, SlrmError> {
        let p = self
            .products
            .get(product)
            .ok_or_else(|| data_err("product index out of range"))?;
        let lo = *self
            .numeric_min
            .get(field)
            .ok_or_else(|| data_err("numeric field index out of range"))?;
        let hi = *self
            .numeric_max
            .get(field)
            .ok_or_else(|| data_err("numeric field index out of range"))?;
        let orig = original_cuts(lo, hi, self.settings.n_bins);
        let counts = p
            .numeric
            .get(field)
            .ok_or_else(|| data_err("numeric field index out of range"))?;
        if both_classes(p) && !orig.is_empty() {
            let occ = occupancy(counts);
            Ok(collapse_cuts(&orig, &occ)?)
        } else {
            Ok(orig)
        }
    }

    /// PairCounts for product `product`, numeric field `field`, as the PMML writes them.
    pub fn numeric_pair_counts(
        &self,
        product: usize,
        field: usize,
    ) -> Result<Vec<PairCount>, SlrmError> {
        let p = self
            .products
            .get(product)
            .ok_or_else(|| data_err("product index out of range"))?;
        let counts = p
            .numeric
            .get(field)
            .ok_or_else(|| data_err("numeric field index out of range"))?;
        let lo = *self
            .numeric_min
            .get(field)
            .ok_or_else(|| data_err("numeric field index out of range"))?;
        let hi = *self
            .numeric_max
            .get(field)
            .ok_or_else(|| data_err("numeric field index out of range"))?;
        let orig = original_cuts(lo, hi, self.settings.n_bins);
        if both_classes(p) && !orig.is_empty() {
            let occ = occupancy(counts);
            let collapsed = collapse_cuts(&orig, &occ)?;
            let n_coll = collapsed.len().saturating_add(1);
            let mut merged = vec![[0_u64; N_CLASS]; n_coll];
            for (i, cell) in counts.iter().enumerate() {
                let c0 = cell.first().copied().unwrap_or(0);
                let c1 = cell.get(1).copied().unwrap_or(0);
                if c0 == 0 && c1 == 0 {
                    continue;
                }
                let mapped = map_orig_to_collapsed(i, counts.len(), &orig, &collapsed, lo, hi)?;
                let slot = merged
                    .get_mut(mapped)
                    .ok_or_else(|| data_err("internal: collapsed bin out of range"))?;
                add_u64(
                    slot.first_mut()
                        .ok_or_else(|| data_err("internal: class 0"))?,
                    c0,
                )?;
                add_u64(
                    slot.get_mut(1)
                        .ok_or_else(|| data_err("internal: class 1"))?,
                    c1,
                )?;
            }
            Ok(export_occupied(&merged))
        } else {
            Ok(export_occupied(counts))
        }
    }

    /// PairCounts for product `product`, nominal field `field` (empty categories omitted).
    pub fn nominal_pair_counts(
        &self,
        product: usize,
        field: usize,
    ) -> Result<Vec<PairCount>, SlrmError> {
        let p = self
            .products
            .get(product)
            .ok_or_else(|| data_err("product index out of range"))?;
        let counts = p
            .nominal
            .get(field)
            .ok_or_else(|| data_err("nominal field index out of range"))?;
        let labels = self
            .nominal_labels
            .get(field)
            .ok_or_else(|| data_err("nominal field index out of range"))?;
        let mut out = Vec::new();
        for (i, cell) in counts.iter().enumerate() {
            let c0 = cell.first().copied().unwrap_or(0);
            let c1 = cell.get(1).copied().unwrap_or(0);
            if c0 == 0 && c1 == 0 {
                continue;
            }
            let label = labels
                .get(i)
                .ok_or_else(|| data_err("internal: nominal label missing"))?;
            out.push(PairCount {
                value: label.clone(),
                n1: c1,
                n0: c0,
            });
        }
        Ok(out)
    }

    /// Guide p. 341 information measure `I_{y,x}` for a numeric field, or `None` if undefined.
    pub fn numeric_information(
        &self,
        product: usize,
        field: usize,
    ) -> Result<Option<f64>, SlrmError> {
        let pcs = self.numeric_pair_counts(product, field)?;
        Ok(information_from_pairs(pcs.iter().map(|p| (p.n0, p.n1))))
    }

    /// Guide p. 341 information measure `I_{y,x}` for a nominal field, or `None` if undefined.
    pub fn nominal_information(
        &self,
        product: usize,
        field: usize,
    ) -> Result<Option<f64>, SlrmError> {
        let pcs = self.nominal_pair_counts(product, field)?;
        Ok(information_from_pairs(pcs.iter().map(|p| (p.n0, p.n1))))
    }

    /// Naive Bayes `P(response = 1 | query, product)`.
    pub fn response_probability(&self, product: usize, query: &Query) -> Result<f64, SlrmError> {
        crate::check_cancelled()?;
        self.check_query(query)?;
        let p = self
            .products
            .get(product)
            .ok_or_else(|| data_err("product index out of range"))?;
        posterior_one(self, p, product, query)
    }

    /// Ranked offers for `query` using the stored settings (`F_i = 1` for every product).
    pub fn score(&self, query: &Query) -> Result<Vec<OfferScore>, SlrmError> {
        self.score_with(query, &ScoreOpts::default())
    }

    /// Ranked offers for `query` with include filters and optional preference override.
    pub fn score_with(
        &self,
        query: &Query,
        opts: &ScoreOpts<'_>,
    ) -> Result<Vec<OfferScore>, SlrmError> {
        crate::check_cancelled()?;
        self.check_query(query)?;
        let n_prod = self.products.len();
        if let Some(inc) = opts.include
            && inc.len() != n_prod
        {
            return Err(data_err(format!(
                "include filter has {} entries, expected {n_prod}",
                inc.len()
            )));
        }
        let prefs = match opts.preferences {
            Some(p) => {
                if p.len() != n_prod {
                    return Err(settings_err(format!(
                        "preferences has {} entries, expected {n_prod}",
                        p.len()
                    )));
                }
                p
            }
            None if self.settings.preferences.is_empty() => &[][..],
            None => {
                if self.settings.preferences.len() != n_prod {
                    return Err(settings_err(format!(
                        "settings preferences has {} entries, expected {n_prod}",
                        self.settings.preferences.len()
                    )));
                }
                self.settings.preferences.as_slice()
            }
        };
        let mut max_s = 0.0_f64;
        if prefs.is_empty() {
            max_s = 1.0;
        } else {
            for &s in prefs {
                if !s.is_finite() || s < 0.0 {
                    return Err(settings_err("offer preferences must be finite and ≥ 0"));
                }
                if s > max_s {
                    max_s = s;
                }
            }
        }
        let alpha = self.settings.randomization;
        let mut rng = Lcg::new(self.settings.scoring_random_seed);
        let w_emp = if self.settings.model_reliability {
            RELIABILITY_CASES
        } else {
            0.0
        };
        let mut scored = Vec::new();
        for i in 0..n_prod {
            if let Some(inc) = opts.include {
                let keep = *inc
                    .get(i)
                    .ok_or_else(|| data_err("include filter missing"))?;
                if !keep {
                    continue;
                }
            }
            let p = self
                .products
                .get(i)
                .ok_or_else(|| data_err("internal: product missing"))?;
            let prob = posterior_one(self, p, i, query)?;
            let w_i = (p.n.first().copied().unwrap_or(0) + p.n.get(1).copied().unwrap_or(0)) as f64;
            let shrunk = shrink_probability(prob, w_i, w_emp)?;
            let pr = if alpha > 0.0 { rng.unit() } else { 0.0 };
            let mixed = alpha * pr + (1.0 - alpha) * shrunk;
            let s_i = if prefs.is_empty() {
                1.0
            } else {
                *prefs.get(i).ok_or_else(|| data_err("preference missing"))?
            };
            let scale = if max_s > 0.0 { s_i / max_s } else { 0.0 };
            let score = mixed * scale;
            scored.push(OfferScore {
                product_index: i,
                product_label: p.label.clone(),
                probability: prob,
                score,
            });
        }
        match self.settings.sort {
            SortOrder::Descending => scored.sort_by(|a, b| match b.score.total_cmp(&a.score) {
                std::cmp::Ordering::Equal => a.product_index.cmp(&b.product_index),
                o => o,
            }),
            SortOrder::Ascending => scored.sort_by(|a, b| match a.score.total_cmp(&b.score) {
                std::cmp::Ordering::Equal => a.product_index.cmp(&b.product_index),
                o => o,
            }),
        }
        let m = self.settings.max_predictions.min(scored.len());
        scored.truncate(m);
        Ok(scored)
    }

    /// Model assessment by simulated data. The Guide does not specify the RNG.
    pub fn assess(&self) -> Result<(), SlrmError> {
        let _ = self;
        Err(gap(
            "model assessment by simulation: the Guide does not specify the RNG for n_round × n_sample draws",
        ))
    }

    /// Simulated leave-one-predictor-out importance. The Guide does not specify the RNG.
    pub fn predictor_importance_by_simulation(&self) -> Result<(), SlrmError> {
        let _ = self;
        Err(gap(
            "predictor importance by simulation: the Guide does not specify the RNG",
        ))
    }

    /// Plain-text dump of products, counts, and discretize cuts.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "SLRM(products={}, numeric={}, nominal={}, n_bins={}, reliability={}, alpha={})",
            self.products.len(),
            self.numeric_names.len(),
            self.nominal_names.len(),
            self.settings.n_bins,
            self.settings.model_reliability,
            self.settings.randomization
        );
        for (i, p) in self.products.iter().enumerate() {
            let n0 = p.n.first().copied().unwrap_or(0);
            let n1 = p.n.get(1).copied().unwrap_or(0);
            let n = n0.saturating_add(n1);
            let _ = writeln!(out, "product[{i}] {} N={n} N1={n1} N0={n0}", p.label);
            for (j, name) in self.numeric_names.iter().enumerate() {
                match self.numeric_cuts(i, j) {
                    Ok(cuts) => {
                        let _ = write!(out, "  numeric {name} cuts=");
                        for (k, c) in cuts.iter().enumerate() {
                            if k > 0 {
                                let _ = write!(out, ",");
                            }
                            let _ = write!(out, "{c:.10}");
                        }
                        let _ = writeln!(out);
                    }
                    Err(e) => {
                        let _ = writeln!(out, "  numeric {name} cuts=error({e})");
                    }
                }
                if let Ok(pcs) = self.numeric_pair_counts(i, j) {
                    for pc in pcs {
                        let _ = writeln!(out, "    pc {} n1={} n0={}", pc.value, pc.n1, pc.n0);
                    }
                }
            }
            for (j, name) in self.nominal_names.iter().enumerate() {
                let _ = writeln!(out, "  nominal {name}");
                if let Ok(pcs) = self.nominal_pair_counts(i, j) {
                    for pc in pcs {
                        let _ = writeln!(out, "    pc {} n1={} n0={}", pc.value, pc.n1, pc.n0);
                    }
                }
            }
        }
        out
    }

    fn check_query(&self, query: &Query) -> Result<(), SlrmError> {
        if query.numeric.len() != self.numeric_names.len() {
            return Err(data_err(format!(
                "query has {} numeric fields, expected {}",
                query.numeric.len(),
                self.numeric_names.len()
            )));
        }
        if query.nominal.len() != self.nominal_names.len() {
            return Err(data_err(format!(
                "query has {} nominal fields, expected {}",
                query.nominal.len(),
                self.nominal_names.len()
            )));
        }
        for (j, v) in query.numeric.iter().enumerate() {
            if let Some(x) = *v
                && !x.is_finite()
            {
                return Err(data_err(format!(
                    "non-finite numeric query value for '{}'",
                    self.numeric_names.get(j).map_or("?", String::as_str)
                )));
            }
        }
        Ok(())
    }
}

fn validate_settings(settings: &Settings) -> Result<(), SlrmError> {
    if settings.n_bins == 0 || settings.n_bins > MAX_BINS {
        return Err(settings_err(format!(
            "n_bins must be in 1..={MAX_BINS}, got {}",
            settings.n_bins
        )));
    }
    if !(0.0..=1.0).contains(&settings.randomization) || !settings.randomization.is_finite() {
        return Err(settings_err(
            "randomization α must be a finite value in [0, 1]",
        ));
    }
    if settings.max_predictions == 0 {
        return Err(settings_err("max_predictions must be ≥ 1"));
    }
    for &s in &settings.preferences {
        if !s.is_finite() || s < 0.0 {
            return Err(settings_err("offer preferences must be finite and ≥ 0"));
        }
    }
    Ok(())
}

fn data_len(data: &SlrmData) -> Result<usize, SlrmError> {
    let n = data.response.len();
    if data.target.codes.len() != n {
        return Err(data_err("target codes length does not match response"));
    }
    for col in &data.numeric {
        if col.values.len() != n {
            return Err(data_err(format!(
                "numeric field '{}' length does not match response",
                col.name
            )));
        }
    }
    for col in &data.nominal {
        if col.codes.len() != n {
            return Err(data_err(format!(
                "nominal field '{}' length does not match response",
                col.name
            )));
        }
    }
    if let Some(freq) = &data.frequency
        && freq.len() != n
    {
        return Err(data_err("frequency length does not match response"));
    }
    Ok(n)
}

fn check_schema_limits(data: &SlrmData) -> Result<(), SlrmError> {
    let n_pred = data
        .numeric
        .len()
        .checked_add(data.nominal.len())
        .ok_or_else(|| data_err("predictor count overflow"))?;
    if n_pred > MAX_PREDICTORS {
        return Err(data_err(format!(
            "SLRM has {n_pred} predictors, maximum {MAX_PREDICTORS}"
        )));
    }
    if data.target.labels.len() > MAX_PRODUCTS {
        return Err(data_err(format!(
            "SLRM target has {} products, maximum {MAX_PRODUCTS}",
            data.target.labels.len()
        )));
    }
    if data.target.labels.len() > MAX_CATEGORIES {
        return Err(data_err(format!(
            "SLRM target has {} categories, maximum {MAX_CATEGORIES}",
            data.target.labels.len()
        )));
    }
    for col in &data.nominal {
        if col.labels.len() > MAX_CATEGORIES {
            return Err(data_err(format!(
                "nominal field '{}' has {} categories, maximum {MAX_CATEGORIES}",
                col.name,
                col.labels.len()
            )));
        }
    }
    Ok(())
}

fn numeric_ranges(data: &SlrmData, n_bins: usize) -> Result<(Vec<f64>, Vec<f64>), SlrmError> {
    let _ = n_bins;
    let mut mins = Vec::with_capacity(data.numeric.len());
    let mut maxs = Vec::with_capacity(data.numeric.len());
    for col in &data.numeric {
        let mut lo: Option<f64> = None;
        let mut hi: Option<f64> = None;
        for v in &col.values {
            let Some(x) = *v else { continue };
            if !x.is_finite() {
                return Err(data_err(format!(
                    "non-finite numeric value in field '{}'",
                    col.name
                )));
            }
            lo = Some(match lo {
                Some(m) if m <= x => m,
                _ => x,
            });
            hi = Some(match hi {
                Some(m) if m >= x => m,
                _ => x,
            });
        }
        let (lo, hi) = match (lo, hi) {
            (Some(a), Some(b)) => (a, b),
            _ => (0.0, 0.0),
        };
        mins.push(lo);
        maxs.push(hi);
    }
    Ok((mins, maxs))
}

fn empty_products(
    labels: &[String],
    data: &SlrmData,
    n_bins: usize,
) -> Result<Vec<Product>, SlrmError> {
    let mut out = Vec::with_capacity(labels.len());
    for label in labels {
        out.push(empty_product(label.clone(), data, n_bins)?);
    }
    Ok(out)
}

fn empty_product(label: String, data: &SlrmData, n_bins: usize) -> Result<Product, SlrmError> {
    let mut numeric = Vec::with_capacity(data.numeric.len());
    for _ in &data.numeric {
        numeric.push(vec![[0_u64; N_CLASS]; n_bins]);
    }
    let mut nominal = Vec::with_capacity(data.nominal.len());
    for col in &data.nominal {
        nominal.push(vec![[0_u64; N_CLASS]; col.labels.len()]);
    }
    Ok(Product {
        label,
        n: [0, 0],
        numeric,
        nominal,
    })
}

fn check_update_schema(model: &SlrmModel, data: &SlrmData) -> Result<(), SlrmError> {
    if data.numeric.len() != model.numeric_names.len() {
        return Err(data_err("update numeric fields do not match the model"));
    }
    if data.nominal.len() != model.nominal_names.len() {
        return Err(data_err("update nominal fields do not match the model"));
    }
    for (i, col) in data.numeric.iter().enumerate() {
        let name = model
            .numeric_names
            .get(i)
            .ok_or_else(|| data_err("internal: numeric name missing"))?;
        if &col.name != name {
            return Err(data_err(format!(
                "update numeric field '{0}' does not match model field '{name}'",
                col.name
            )));
        }
    }
    for (i, col) in data.nominal.iter().enumerate() {
        let name = model
            .nominal_names
            .get(i)
            .ok_or_else(|| data_err("internal: nominal name missing"))?;
        if &col.name != name {
            return Err(data_err(format!(
                "update nominal field '{0}' does not match model field '{name}'",
                col.name
            )));
        }
        let have = model
            .nominal_labels
            .get(i)
            .ok_or_else(|| data_err("internal: nominal labels missing"))?;
        for (k, lab) in have.iter().enumerate() {
            match col.labels.get(k) {
                Some(l) if l == lab => {}
                Some(l) => {
                    return Err(data_err(format!(
                        "update nominal field '{}' label {k} '{l}' does not match '{lab}'",
                        col.name
                    )));
                }
                None => {
                    return Err(data_err(format!(
                        "update nominal field '{}' is missing label {k} '{lab}'",
                        col.name
                    )));
                }
            }
        }
        if col.labels.len() > MAX_CATEGORIES {
            return Err(data_err(format!(
                "nominal field '{}' has {} categories, maximum {MAX_CATEGORIES}",
                col.name,
                col.labels.len()
            )));
        }
    }
    for (i, lab) in model.products.iter().enumerate() {
        match data.target.labels.get(i) {
            Some(l) if l == &lab.label => {}
            Some(l) => {
                return Err(data_err(format!(
                    "update target label {i} '{l}' does not match '{}'",
                    lab.label
                )));
            }
            None => {
                return Err(data_err(format!(
                    "update target is missing product '{}' at index {i}",
                    lab.label
                )));
            }
        }
    }
    if data.target.labels.len() > MAX_PRODUCTS {
        return Err(data_err(format!(
            "SLRM target has {} products, maximum {MAX_PRODUCTS}",
            data.target.labels.len()
        )));
    }
    Ok(())
}

fn extend_products(model: &mut SlrmModel, data: &SlrmData) -> Result<(), SlrmError> {
    for i in model.products.len()..data.target.labels.len() {
        let label = data
            .target
            .labels
            .get(i)
            .ok_or_else(|| data_err("internal: new product label missing"))?
            .clone();
        model
            .products
            .push(empty_product(label, data, model.settings.n_bins)?);
    }
    for (i, col) in data.nominal.iter().enumerate() {
        let have = model
            .nominal_labels
            .get_mut(i)
            .ok_or_else(|| data_err("internal: nominal labels missing"))?;
        if col.labels.len() > have.len() {
            for extra in col.labels.iter().skip(have.len()) {
                have.push(extra.clone());
            }
            for p in &mut model.products {
                let slot = p
                    .nominal
                    .get_mut(i)
                    .ok_or_else(|| data_err("internal: nominal counts missing"))?;
                while slot.len() < have.len() {
                    slot.push([0, 0]);
                }
            }
        }
    }
    Ok(())
}

fn accumulate(model: &mut SlrmModel, data: &SlrmData) -> Result<(), SlrmError> {
    let n = data.response.len();
    let n_pred = model.numeric_names.len() + model.nominal_names.len();
    for row in 0..n {
        crate::check_cancelled()?;
        let Some(resp) = data.response.get(row).copied().flatten() else {
            continue;
        };
        let Some(code) = data.target.codes.get(row).copied().flatten() else {
            continue;
        };
        let pi = usize::try_from(code).map_err(|_| data_err("target code does not fit usize"))?;
        if pi >= model.products.len() {
            return Err(data_err(format!("target code {code} out of range")));
        }
        let class = usize::from(resp);
        let weight = match &data.frequency {
            None => 1_u64,
            Some(freq) => match freq.get(row).copied().flatten() {
                None => continue,
                Some(0) => continue,
                Some(w) => u64::from(w),
            },
        };
        if n_pred > 0 && record_all_predictors_missing(data, row) {
            continue;
        }
        add_record(model, data, row, pi, class, weight)?;
    }
    Ok(())
}

fn record_all_predictors_missing(data: &SlrmData, row: usize) -> bool {
    for col in &data.numeric {
        if let Some(Some(x)) = col.values.get(row)
            && x.is_finite()
        {
            return false;
        }
    }
    for col in &data.nominal {
        if col.codes.get(row).copied().flatten().is_some() {
            return false;
        }
    }
    true
}

fn add_record(
    model: &mut SlrmModel,
    data: &SlrmData,
    row: usize,
    pi: usize,
    class: usize,
    weight: u64,
) -> Result<(), SlrmError> {
    {
        let p = model
            .products
            .get_mut(pi)
            .ok_or_else(|| data_err("internal: product missing"))?;
        let slot =
            p.n.get_mut(class)
                .ok_or_else(|| data_err("internal: class missing"))?;
        add_u64(slot, weight)?;
    }
    let n_bins = model.settings.n_bins;
    for f in 0..data.numeric.len() {
        let x = match data.numeric.get(f).and_then(|c| c.values.get(row)).copied() {
            Some(Some(v)) if v.is_finite() => v,
            Some(Some(_)) => {
                return Err(data_err(format!(
                    "non-finite numeric value in field '{}'",
                    data.numeric.get(f).map_or("?", |c| c.name.as_str())
                )));
            }
            _ => continue,
        };
        let lo = *model
            .numeric_min
            .get(f)
            .ok_or_else(|| data_err("internal: numeric min missing"))?;
        let hi = *model
            .numeric_max
            .get(f)
            .ok_or_else(|| data_err("internal: numeric max missing"))?;
        let cuts = original_cuts(lo, hi, n_bins);
        let b = bin_of(x, &cuts);
        let p = model
            .products
            .get_mut(pi)
            .ok_or_else(|| data_err("internal: product missing"))?;
        let field = p
            .numeric
            .get_mut(f)
            .ok_or_else(|| data_err("internal: numeric counts missing"))?;
        let cell = field
            .get_mut(b)
            .ok_or_else(|| data_err("internal: bin out of range"))?;
        let slot = cell
            .get_mut(class)
            .ok_or_else(|| data_err("internal: class missing"))?;
        add_u64(slot, weight)?;
    }
    for f in 0..data.nominal.len() {
        let Some(code) = data
            .nominal
            .get(f)
            .and_then(|c| c.codes.get(row))
            .copied()
            .flatten()
        else {
            continue;
        };
        let cat = usize::try_from(code).map_err(|_| data_err("nominal code does not fit usize"))?;
        let p = model
            .products
            .get_mut(pi)
            .ok_or_else(|| data_err("internal: product missing"))?;
        let field = p
            .nominal
            .get_mut(f)
            .ok_or_else(|| data_err("internal: nominal counts missing"))?;
        if cat >= field.len() {
            return Err(data_err(format!(
                "nominal code {code} out of range for '{}'",
                data.nominal.get(f).map_or("?", |c| c.name.as_str())
            )));
        }
        let cell = field
            .get_mut(cat)
            .ok_or_else(|| data_err("internal: category missing"))?;
        let slot = cell
            .get_mut(class)
            .ok_or_else(|| data_err("internal: class missing"))?;
        add_u64(slot, weight)?;
    }
    Ok(())
}

fn add_u64(slot: &mut u64, w: u64) -> Result<(), SlrmError> {
    *slot = slot
        .checked_add(w)
        .ok_or_else(|| data_err("SLRM count overflow"))?;
    Ok(())
}

fn both_classes(p: &Product) -> bool {
    p.n.first().copied().unwrap_or(0) > 0 && p.n.get(1).copied().unwrap_or(0) > 0
}

fn occupancy(counts: &[[u64; N_CLASS]]) -> Vec<bool> {
    counts
        .iter()
        .map(|c| c.first().copied().unwrap_or(0) + c.get(1).copied().unwrap_or(0) > 0)
        .collect()
}

fn original_cuts(lo: f64, hi: f64, n_bins: usize) -> Vec<f64> {
    if n_bins <= 1 || lo == hi || !lo.is_finite() || !hi.is_finite() {
        return Vec::new();
    }
    let width = (hi - lo) / n_bins as f64;
    if !width.is_finite() || width == 0.0 {
        return Vec::new();
    }
    let mut cuts = Vec::with_capacity(n_bins.saturating_sub(1));
    for i in 1..n_bins {
        cuts.push(lo + width * i as f64);
    }
    cuts
}

fn bin_of(x: f64, cuts: &[f64]) -> usize {
    for (i, &c) in cuts.iter().enumerate() {
        if x <= c {
            return i;
        }
    }
    cuts.len()
}

fn collapse_cuts(cuts: &[f64], occupied: &[bool]) -> Result<Vec<f64>, SlrmError> {
    if occupied.is_empty() {
        return Ok(Vec::new());
    }
    if occupied.len() != cuts.len().saturating_add(1) {
        return Err(data_err("internal: occupancy length does not match cuts"));
    }
    let mut first = None;
    let mut last = None;
    for (i, &occ) in occupied.iter().enumerate() {
        if occ {
            if first.is_none() {
                first = Some(i);
            }
            last = Some(i);
        }
    }
    let (Some(first), Some(last)) = (first, last) else {
        return Ok(Vec::new());
    };
    let mut new_cuts = Vec::new();
    let mut k = first;
    while k < last {
        let occ_k = *occupied
            .get(k)
            .ok_or_else(|| data_err("internal: occupancy index"))?;
        let occ_next = *occupied
            .get(k.saturating_add(1))
            .ok_or_else(|| data_err("internal: occupancy index"))?;
        if occ_k && occ_next {
            let cut = *cuts.get(k).ok_or_else(|| data_err("internal: cut index"))?;
            new_cuts.push(cut);
            k = k.saturating_add(1);
        } else if occ_k && !occ_next {
            let mut j = k.saturating_add(1);
            while j <= last {
                let occ_j = *occupied
                    .get(j)
                    .ok_or_else(|| data_err("internal: occupancy index"))?;
                if occ_j {
                    break;
                }
                j = j.saturating_add(1);
            }
            let left = *cuts.get(k).ok_or_else(|| data_err("internal: cut index"))?;
            let right_idx = j.saturating_sub(1);
            let right = *cuts
                .get(right_idx)
                .ok_or_else(|| data_err("internal: cut index"))?;
            new_cuts.push((left + right) / 2.0);
            k = j;
        } else {
            k = k.saturating_add(1);
        }
    }
    Ok(new_cuts)
}

fn map_orig_to_collapsed(
    orig_i: usize,
    n_orig: usize,
    orig_cuts: &[f64],
    collapsed: &[f64],
    lo: f64,
    hi: f64,
) -> Result<usize, SlrmError> {
    let x = orig_representative(orig_i, n_orig, orig_cuts, lo, hi)?;
    Ok(bin_of(x, collapsed))
}

fn orig_representative(
    orig_i: usize,
    n_orig: usize,
    orig_cuts: &[f64],
    lo: f64,
    hi: f64,
) -> Result<f64, SlrmError> {
    if orig_cuts.is_empty() {
        return Ok(lo);
    }
    if orig_i == 0 {
        return orig_cuts
            .first()
            .copied()
            .ok_or_else(|| data_err("internal: first cut missing"));
    }
    if orig_i + 1 >= n_orig || orig_i >= orig_cuts.len() {
        let last = orig_cuts
            .last()
            .copied()
            .ok_or_else(|| data_err("internal: last cut missing"))?;
        if hi > last {
            return Ok(hi);
        }
        return Ok(last + 1.0);
    }
    orig_cuts
        .get(orig_i)
        .copied()
        .ok_or_else(|| data_err("internal: cut missing"))
}

fn export_occupied(counts: &[[u64; N_CLASS]]) -> Vec<PairCount> {
    let mut out = Vec::new();
    let mut k = 0_usize;
    for cell in counts {
        let c0 = cell.first().copied().unwrap_or(0);
        let c1 = cell.get(1).copied().unwrap_or(0);
        if c0 == 0 && c1 == 0 {
            continue;
        }
        out.push(PairCount {
            value: k.to_string(),
            n1: c1,
            n0: c0,
        });
        k = k.saturating_add(1);
    }
    out
}

fn posterior_one(
    model: &SlrmModel,
    p: &Product,
    _product: usize,
    query: &Query,
) -> Result<f64, SlrmError> {
    let n0 = p.n.first().copied().unwrap_or(0);
    let n1 = p.n.get(1).copied().unwrap_or(0);
    let n = n0.saturating_add(n1);
    if n == 0 {
        return Ok(0.5);
    }
    let n_f = n as f64;
    let lambda = 1.0 / n_f;
    let k_f = N_CLASS as f64;
    let pi1 = (n1 as f64 + lambda) / (n_f + k_f * lambda);
    let pi0 = (n0 as f64 + lambda) / (n_f + k_f * lambda);
    let mut log1 = ln_pos(pi1)?;
    let mut log0 = ln_pos(pi0)?;
    let f = lambda;
    for (j, xv) in query.numeric.iter().enumerate() {
        let Some(x) = *xv else { continue };
        let Some(cell) = numeric_cell(model, p, j, x)? else {
            continue;
        };
        if cell.m == 0 {
            continue;
        }
        let m_f = cell.m as f64;
        let p1 = (cell.n1 as f64 + f) / (cell.n1_field as f64 + m_f * f);
        let p0 = (cell.n0 as f64 + f) / (cell.n0_field as f64 + m_f * f);
        log1 += ln_pos(p1)?;
        log0 += ln_pos(p0)?;
    }
    for (j, cv) in query.nominal.iter().enumerate() {
        let Some(code) = *cv else { continue };
        let Some(cell) = nominal_cell(p, j, code)? else {
            continue;
        };
        if cell.m == 0 {
            continue;
        }
        let m_f = cell.m as f64;
        let p1 = (cell.n1 as f64 + f) / (cell.n1_field as f64 + m_f * f);
        let p0 = (cell.n0 as f64 + f) / (cell.n0_field as f64 + m_f * f);
        log1 += ln_pos(p1)?;
        log0 += ln_pos(p0)?;
    }
    log_sigmoid(log1, log0)
}

/// Occupied-bin counts used in one Dirichlet factor of the posterior.
struct OccupiedCell {
    n0: u64,
    n1: u64,
    m: usize,
    n0_field: u64,
    n1_field: u64,
}

fn numeric_cell(
    model: &SlrmModel,
    p: &Product,
    field: usize,
    x: f64,
) -> Result<Option<OccupiedCell>, SlrmError> {
    let counts = p
        .numeric
        .get(field)
        .ok_or_else(|| data_err("numeric field index out of range"))?;
    let lo = *model
        .numeric_min
        .get(field)
        .ok_or_else(|| data_err("numeric field index out of range"))?;
    let hi = *model
        .numeric_max
        .get(field)
        .ok_or_else(|| data_err("numeric field index out of range"))?;
    let orig = original_cuts(lo, hi, model.settings.n_bins);
    if both_classes(p) && !orig.is_empty() {
        let occ = occupancy(counts);
        let collapsed = collapse_cuts(&orig, &occ)?;
        let b = bin_of(x, &collapsed);
        let n_coll = collapsed.len().saturating_add(1);
        let mut merged = vec![[0_u64; N_CLASS]; n_coll];
        for (i, cell) in counts.iter().enumerate() {
            let c0 = cell.first().copied().unwrap_or(0);
            let c1 = cell.get(1).copied().unwrap_or(0);
            if c0 == 0 && c1 == 0 {
                continue;
            }
            let mapped = map_orig_to_collapsed(i, counts.len(), &orig, &collapsed, lo, hi)?;
            let slot = merged
                .get_mut(mapped)
                .ok_or_else(|| data_err("internal: collapsed bin"))?;
            add_u64(slot.first_mut().ok_or_else(|| data_err("class 0"))?, c0)?;
            add_u64(slot.get_mut(1).ok_or_else(|| data_err("class 1"))?, c1)?;
        }
        let cell = merged
            .get(b)
            .ok_or_else(|| data_err("internal: query bin"))?;
        let c0 = cell.first().copied().unwrap_or(0);
        let c1 = cell.get(1).copied().unwrap_or(0);
        if c0 == 0 && c1 == 0 {
            return Ok(None);
        }
        let (n0_j, n1_j, m) = occupied_totals(&merged);
        Ok(Some(OccupiedCell {
            n0: c0,
            n1: c1,
            m,
            n0_field: n0_j,
            n1_field: n1_j,
        }))
    } else {
        let b = bin_of(x, &orig);
        let cell = counts
            .get(b)
            .ok_or_else(|| data_err("internal: query bin"))?;
        let c0 = cell.first().copied().unwrap_or(0);
        let c1 = cell.get(1).copied().unwrap_or(0);
        if c0 == 0 && c1 == 0 {
            return Ok(None);
        }
        let (n0_j, n1_j, m) = occupied_totals(counts);
        Ok(Some(OccupiedCell {
            n0: c0,
            n1: c1,
            m,
            n0_field: n0_j,
            n1_field: n1_j,
        }))
    }
}

fn nominal_cell(p: &Product, field: usize, code: u32) -> Result<Option<OccupiedCell>, SlrmError> {
    let counts = p
        .nominal
        .get(field)
        .ok_or_else(|| data_err("nominal field index out of range"))?;
    let cat = usize::try_from(code).map_err(|_| data_err("nominal code does not fit usize"))?;
    let Some(cell) = counts.get(cat) else {
        return Ok(None);
    };
    let c0 = cell.first().copied().unwrap_or(0);
    let c1 = cell.get(1).copied().unwrap_or(0);
    if c0 == 0 && c1 == 0 {
        return Ok(None);
    }
    let (n0_j, n1_j, m) = occupied_totals(counts);
    Ok(Some(OccupiedCell {
        n0: c0,
        n1: c1,
        m,
        n0_field: n0_j,
        n1_field: n1_j,
    }))
}

fn occupied_totals(counts: &[[u64; N_CLASS]]) -> (u64, u64, usize) {
    let mut n0 = 0_u64;
    let mut n1 = 0_u64;
    let mut m = 0_usize;
    for cell in counts {
        let c0 = cell.first().copied().unwrap_or(0);
        let c1 = cell.get(1).copied().unwrap_or(0);
        if c0 == 0 && c1 == 0 {
            continue;
        }
        n0 = n0.saturating_add(c0);
        n1 = n1.saturating_add(c1);
        m = m.saturating_add(1);
    }
    (n0, n1, m)
}

fn ln_pos(x: f64) -> Result<f64, SlrmError> {
    if x > 0.0 && x.is_finite() {
        Ok(x.ln())
    } else {
        Err(data_err("internal: non-positive probability in log"))
    }
}

fn log_sigmoid(log1: f64, log0: f64) -> Result<f64, SlrmError> {
    let m = log1.max(log0);
    let e1 = (log1 - m).exp();
    let e0 = (log0 - m).exp();
    let den = e1 + e0;
    if den == 0.0 || !den.is_finite() {
        return Err(data_err("internal: cannot normalise posterior"));
    }
    let p = e1 / den;
    if p.is_finite() {
        Ok(p)
    } else {
        Err(data_err("internal: non-finite posterior"))
    }
}

fn shrink_probability(p: f64, w_i: f64, w_emp: f64) -> Result<f64, SlrmError> {
    let den = w_i + w_emp;
    if den == 0.0 {
        return Ok(0.5);
    }
    let s = (w_i / den) * p + (w_emp / den) * 0.5;
    if s.is_finite() {
        Ok(s)
    } else {
        Err(data_err("internal: non-finite shrunk probability"))
    }
}

fn information_from_pairs<I>(pairs: I) -> Option<f64>
where
    I: Iterator<Item = (u64, u64)>,
{
    let mut n0 = 0_f64;
    let mut n1 = 0_f64;
    let mut rows: Vec<(f64, f64)> = Vec::new();
    for (c0, c1) in pairs {
        let a = c0 as f64;
        let b = c1 as f64;
        if a == 0.0 && b == 0.0 {
            continue;
        }
        n0 += a;
        n1 += b;
        rows.push((a, b));
    }
    let n = n0 + n1;
    if n == 0.0 {
        return None;
    }
    let hy = entropy2(n0 / n, n1 / n)?;
    if hy == 0.0 {
        return Some(0.0);
    }
    let mut hyx = 0.0;
    for (a, b) in rows {
        let nx = a + b;
        if nx == 0.0 {
            continue;
        }
        let h = entropy2(a / nx, b / nx)?;
        hyx += (nx / n) * h;
    }
    let mi = hy - hyx;
    if mi.is_finite() && hy > 0.0 {
        Some((mi / hy).clamp(0.0, 1.0))
    } else {
        None
    }
}

fn entropy2(p0: f64, p1: f64) -> Option<f64> {
    let mut h = 0.0;
    for p in [p0, p1] {
        if p > 0.0 {
            h -= p * p.ln();
        }
    }
    if h.is_finite() { Some(h) } else { None }
}

/// Unverified linear congruential generator for `P_ri` (Guide p. 339).
struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u32) -> Self {
        Self {
            state: u64::from(seed) | 1,
        }
    }

    fn unit(&mut self) -> f64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let bits = (self.state >> 11) & ((1_u64 << 53) - 1);
        bits as f64 / (1_u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(name: &str, values: &[Option<f64>]) -> NumericColumn {
        NumericColumn {
            name: name.to_string(),
            values: values.to_vec(),
        }
    }

    fn num_f(name: &str, values: &[f64]) -> NumericColumn {
        NumericColumn {
            name: name.to_string(),
            values: values.iter().copied().map(Some).collect(),
        }
    }

    fn nom(name: &str, labels: &[&str], codes: &[Option<u32>]) -> NominalColumn {
        NominalColumn {
            name: name.to_string(),
            labels: labels.iter().map(|s| (*s).to_string()).collect(),
            codes: codes.to_vec(),
        }
    }

    fn data_two_products() -> SlrmData {
        // product A (0): x=0 resp=0, x=1 resp=1
        // product B (1): x=0 resp=0, x=0 resp=0
        SlrmData {
            numeric: vec![num_f("x", &[0.0, 1.0, 0.0, 0.0])],
            nominal: Vec::new(),
            target: nom("offer", &["A", "B"], &[Some(0), Some(0), Some(1), Some(1)]),
            response: vec![Some(false), Some(true), Some(false), Some(false)],
            frequency: None,
        }
    }

    #[test]
    fn slrm_known_answer_counts_and_posterior() {
        let data = SlrmData {
            numeric: vec![num_f("x", &[0.0, 1.0])],
            nominal: Vec::new(),
            target: nom("offer", &["A"], &[Some(0), Some(0)]),
            response: vec![Some(false), Some(true)],
            frequency: None,
        };
        let settings = Settings {
            n_bins: 2,
            model_reliability: false,
            max_predictions: 1,
            ..Settings::default()
        };
        let model = train(&data, &settings).expect("train");
        assert_eq!(model.target_counts(0), Some((1, 1)));
        let cuts = model.numeric_cuts(0, 0).expect("cuts");
        assert_eq!(cuts.len(), 1);
        assert!((cuts[0] - 0.5).abs() < 1e-12);
        let pcs = model.numeric_pair_counts(0, 0).expect("pc");
        assert_eq!(pcs.len(), 2);
        assert_eq!(pcs[0].n0, 1);
        assert_eq!(pcs[0].n1, 0);
        assert_eq!(pcs[1].n0, 0);
        assert_eq!(pcs[1].n1, 1);
        let q = Query {
            numeric: vec![Some(1.0)],
            nominal: Vec::new(),
        };
        let p = model.response_probability(0, &q).expect("p");
        // λ=f=1/2, π=0.5, M=2, p(bin1|y=1)=(1+0.5)/(1+1)=0.75, p(bin1|y=0)=0.25
        // P = 0.75*0.5 / (0.75*0.5 + 0.25*0.5) = 0.75
        assert!((p - 0.75).abs() < 1e-12, "p={p}");
        let offers = model.score(&q).expect("score");
        assert_eq!(offers.len(), 1);
        assert!((offers[0].probability - 0.75).abs() < 1e-12);
        assert!((offers[0].score - 0.75).abs() < 1e-12);
    }

    #[test]
    fn slrm_known_answer_equal_width_cuts() {
        let mut x = Vec::new();
        let mut y = Vec::new();
        let mut r = Vec::new();
        for v in [10.0, 96.0, 50.0, 18.6] {
            x.push(v);
            y.push(Some(0));
            r.push(Some(true));
        }
        // both classes so collapse can drop empty bins; seed the other class
        r[0] = Some(false);
        let data = SlrmData {
            numeric: vec![num_f("age", &x)],
            nominal: Vec::new(),
            target: nom("offer", &["A"], &y),
            response: r,
            frequency: None,
        };
        let settings = Settings {
            n_bins: 10,
            ..Settings::default()
        };
        let model = train(&data, &settings).expect("train");
        let (lo, hi) = model.numeric_range(0).expect("range");
        assert!((lo - 10.0).abs() < 1e-12);
        assert!((hi - 96.0).abs() < 1e-12);
        let orig = original_cuts(10.0, 96.0, 10);
        assert_eq!(orig.len(), 9);
        assert!((orig[0] - 18.6).abs() < 1e-12);
        assert!((orig[7] - 78.8).abs() < 1e-12);
        assert!((orig[8] - 87.4).abs() < 1e-12);
    }

    #[test]
    fn slrm_known_answer_empty_bin_collapse_midpoint() {
        // min=0 max=10, 10 bins, width=1. Occupy bins 0 and 9 only, both classes.
        let data = SlrmData {
            numeric: vec![num_f("x", &[0.0, 0.0, 10.0, 10.0])],
            nominal: Vec::new(),
            target: nom("o", &["A"], &[Some(0), Some(0), Some(0), Some(0)]),
            response: vec![Some(false), Some(true), Some(false), Some(true)],
            frequency: None,
        };
        let settings = Settings {
            n_bins: 10,
            ..Settings::default()
        };
        let model = train(&data, &settings).expect("train");
        let cuts = model.numeric_cuts(0, 0).expect("cuts");
        assert_eq!(cuts.len(), 1, "cuts={cuts:?}");
        // original cuts 1,2,…,9; empty run 1..8 → midpoint (1+9)/2 = 5
        assert!((cuts[0] - 5.0).abs() < 1e-12, "cut={}", cuts[0]);
    }

    #[test]
    fn slrm_known_answer_binary_field_cut_half() {
        let data = SlrmData {
            numeric: vec![num_f("flag", &[0.0, 0.0, 1.0, 1.0])],
            nominal: Vec::new(),
            target: nom("o", &["A"], &[Some(0); 4]),
            response: vec![Some(false), Some(false), Some(true), Some(true)],
            frequency: None,
        };
        let model = train(&data, &Settings::default()).expect("train");
        let cuts = model.numeric_cuts(0, 0).expect("cuts");
        assert_eq!(cuts.len(), 1);
        assert!((cuts[0] - 0.5).abs() < 1e-12);
        let pcs = model.numeric_pair_counts(0, 0).expect("pc");
        assert_eq!(pcs.len(), 2);
        assert_eq!((pcs[0].n0, pcs[0].n1), (2, 0));
        assert_eq!((pcs[1].n0, pcs[1].n1), (0, 2));
    }

    #[test]
    fn slrm_known_answer_single_class_keeps_ten_bins() {
        let data = SlrmData {
            numeric: vec![num_f("flag", &[0.0, 0.0, 1.0])],
            nominal: Vec::new(),
            target: nom("o", &["A"], &[Some(0); 3]),
            response: vec![Some(false), Some(false), Some(false)],
            frequency: None,
        };
        let model = train(&data, &Settings::default()).expect("train");
        let cuts = model.numeric_cuts(0, 0).expect("cuts");
        assert_eq!(cuts.len(), 9, "single-class keeps 10 original bins");
        assert!((cuts[0] - 0.1).abs() < 1e-12);
        let pcs = model.numeric_pair_counts(0, 0).expect("pc");
        // occupied original bins 0 and 9 → exported 0, 1
        assert_eq!(pcs.len(), 2);
        assert_eq!(pcs[0].value, "0");
        assert_eq!(pcs[0].n0, 2);
        assert_eq!(pcs[1].value, "1");
        assert_eq!(pcs[1].n0, 1);
    }

    #[test]
    fn slrm_known_answer_reliability_shrink() {
        let data = SlrmData {
            numeric: vec![num_f("x", &[0.0, 1.0])],
            nominal: Vec::new(),
            target: nom("o", &["A"], &[Some(0), Some(0)]),
            response: vec![Some(false), Some(true)],
            frequency: None,
        };
        let settings = Settings {
            n_bins: 2,
            model_reliability: true,
            max_predictions: 1,
            ..Settings::default()
        };
        let model = train(&data, &settings).expect("train");
        let q = Query {
            numeric: vec![Some(1.0)],
            nominal: Vec::new(),
        };
        let offers = model.score(&q).expect("score");
        let p = 0.75;
        let w = 2.0;
        let want = (w / (w + 500.0)) * p + (500.0 / (w + 500.0)) * 0.5;
        assert!((offers[0].probability - p).abs() < 1e-12);
        assert!(
            (offers[0].score - want).abs() < 1e-12,
            "score={} want={want}",
            offers[0].score
        );
    }

    #[test]
    fn slrm_update_equals_batch() {
        // Same min/max and occupied bins in the first batch and in the union,
        // so frozen ranges plus re-derived collapse match a single train.
        let x = [0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        let t = [0_u32, 0, 1, 1, 0, 1];
        let r = [false, true, false, true, true, false];
        let all = SlrmData {
            numeric: vec![num_f("x", &x)],
            nominal: Vec::new(),
            target: nom(
                "o",
                &["A", "B"],
                &t.iter().map(|c| Some(*c)).collect::<Vec<_>>(),
            ),
            response: r.iter().map(|b| Some(*b)).collect(),
            frequency: None,
        };
        let first = SlrmData {
            numeric: vec![num_f("x", &x[..4])],
            nominal: Vec::new(),
            target: nom(
                "o",
                &["A", "B"],
                &t[..4].iter().map(|c| Some(*c)).collect::<Vec<_>>(),
            ),
            response: r[..4].iter().map(|b| Some(*b)).collect(),
            frequency: None,
        };
        let second = SlrmData {
            numeric: vec![num_f("x", &x[4..])],
            nominal: Vec::new(),
            target: nom(
                "o",
                &["A", "B"],
                &t[4..].iter().map(|c| Some(*c)).collect::<Vec<_>>(),
            ),
            response: r[4..].iter().map(|b| Some(*b)).collect(),
            frequency: None,
        };
        let settings = Settings::default();
        let batch = train(&all, &settings).expect("batch");
        let mut inc = train(&first, &settings).expect("first");
        update(&mut inc, &second).expect("update");
        for prod in 0..2 {
            assert_eq!(
                inc.target_counts(prod),
                batch.target_counts(prod),
                "target counts product {prod}"
            );
            let c1 = inc.numeric_cuts(prod, 0).expect("c1");
            let c2 = batch.numeric_cuts(prod, 0).expect("c2");
            assert_eq!(c1.len(), c2.len(), "cut count product {prod}");
            for (a, b) in c1.iter().zip(c2.iter()) {
                assert!((a - b).abs() < 1e-12);
            }
            assert_eq!(
                inc.numeric_pair_counts(prod, 0).expect("p1"),
                batch.numeric_pair_counts(prod, 0).expect("p2"),
                "pair counts product {prod}"
            );
        }
        assert_eq!(inc.to_text(), batch.to_text());
    }

    #[test]
    fn slrm_never_crash() {
        let empty = SlrmData {
            numeric: vec![num("x", &[])],
            nominal: Vec::new(),
            target: nom("o", &["A"], &[]),
            response: vec![],
            frequency: None,
        };
        assert!(matches!(
            train(&empty, &Settings::default()),
            Err(AlgoError::Data(_))
        ));

        let one = SlrmData {
            numeric: vec![num_f("x", &[1.0])],
            nominal: Vec::new(),
            target: nom("o", &["A"], &[Some(0)]),
            response: vec![Some(true)],
            frequency: None,
        };
        let m = train(&one, &Settings::default()).expect("one");
        let q = Query {
            numeric: vec![Some(1.0)],
            nominal: Vec::new(),
        };
        let _ = m.score(&q).expect("score one");

        let constant = SlrmData {
            numeric: vec![num_f("c", &[2.0, 2.0, 2.0])],
            nominal: Vec::new(),
            target: nom("o", &["A"], &[Some(0), Some(0), Some(0)]),
            response: vec![Some(true), Some(false), Some(true)],
            frequency: None,
        };
        let m = train(&constant, &Settings::default()).expect("constant");
        let _ = m
            .score(&Query {
                numeric: vec![Some(2.0)],
                nominal: Vec::new(),
            })
            .expect("score constant");

        let no_resp = SlrmData {
            numeric: vec![num_f("x", &[0.0, 1.0, 2.0])],
            nominal: Vec::new(),
            target: nom("o", &["A"], &[Some(0), Some(0), Some(0)]),
            response: vec![Some(false), Some(false), Some(false)],
            frequency: None,
        };
        let m = train(&no_resp, &Settings::default()).expect("no resp");
        assert_eq!(m.target_counts(0), Some((3, 0)));
        let _ = m
            .score(&Query {
                numeric: vec![Some(1.0)],
                nominal: Vec::new(),
            })
            .expect("score no resp");

        let labels: Vec<String> = (0..MAX_CATEGORIES + 1).map(|i| i.to_string()).collect();
        let many = SlrmData {
            numeric: Vec::new(),
            nominal: vec![NominalColumn {
                name: "cat".into(),
                labels,
                codes: vec![Some(0), Some(1)],
            }],
            target: nom("o", &["A"], &[Some(0), Some(0)]),
            response: vec![Some(true), Some(false)],
            frequency: None,
        };
        assert!(matches!(
            train(&many, &Settings::default()),
            Err(AlgoError::Data(_))
        ));

        let miss_all = SlrmData {
            numeric: vec![num("x", &[None, Some(1.0)])],
            nominal: Vec::new(),
            target: nom("o", &["A"], &[Some(0), Some(0)]),
            response: vec![Some(true), Some(false)],
            frequency: None,
        };
        let m = train(&miss_all, &Settings::default()).expect("miss");
        assert_eq!(m.n_cases(0), Some(1));

        assert!(matches!(
            m.assess(),
            Err(AlgoError::Unsupported(s)) if s.starts_with("GAP: SLRM")
        ));
        assert!(matches!(
            m.predictor_importance_by_simulation(),
            Err(AlgoError::Unsupported(s)) if s.starts_with("GAP: SLRM")
        ));
        let text = m.to_text();
        assert!(text.contains("SLRM("), "{text}");
        assert!(text.contains("product[0]"), "{text}");
    }

    #[test]
    fn slrm_known_answer_information_perfect() {
        let data = SlrmData {
            numeric: vec![num_f("flag", &[0.0, 0.0, 1.0, 1.0])],
            nominal: Vec::new(),
            target: nom("o", &["A"], &[Some(0); 4]),
            response: vec![Some(false), Some(false), Some(true), Some(true)],
            frequency: None,
        };
        let model = train(&data, &Settings::default()).expect("train");
        let i = model
            .numeric_information(0, 0)
            .expect("i")
            .expect("defined");
        assert!((i - 1.0).abs() < 1e-12, "I={i}");
    }

    #[test]
    fn slrm_known_answer_nominal_empty_category_omitted() {
        let data = SlrmData {
            numeric: Vec::new(),
            nominal: vec![nom(
                "m",
                &["0", "12", "24"],
                &[Some(1), Some(1), Some(2), Some(2)],
            )],
            target: nom("o", &["A"], &[Some(0); 4]),
            response: vec![Some(true), Some(false), Some(true), Some(false)],
            frequency: None,
        };
        let model = train(&data, &Settings::default()).expect("train");
        let pcs = model.nominal_pair_counts(0, 0).expect("pc");
        assert_eq!(pcs.len(), 2);
        assert_eq!(pcs[0].value, "12");
        assert_eq!(pcs[1].value, "24");
    }

    #[test]
    fn slrm_two_products_independent_counts() {
        let data = data_two_products();
        let model = train(&data, &Settings::default()).expect("train");
        assert_eq!(model.n_products(), 2);
        assert_eq!(model.product_name(0), Some("A"));
        assert_eq!(model.product_name(1), Some("B"));
        assert_eq!(model.target_counts(0), Some((1, 1)));
        assert_eq!(model.target_counts(1), Some((2, 0)));
        let t0 = model.threshold(0).expect("t");
        assert!((t0 - 0.5).abs() < 1e-12);
    }

    #[test]
    fn slrm_settings_reject_bad_alpha() {
        let data = SlrmData {
            numeric: vec![num_f("x", &[0.0])],
            nominal: Vec::new(),
            target: nom("o", &["A"], &[Some(0)]),
            response: vec![Some(true)],
            frequency: None,
        };
        let s = Settings {
            randomization: 2.0,
            ..Settings::default()
        };
        assert!(matches!(train(&data, &s), Err(AlgoError::Settings(_))));
        let s = Settings {
            n_bins: 0,
            ..Settings::default()
        };
        assert!(matches!(train(&data, &s), Err(AlgoError::Settings(_))));
    }

    #[test]
    fn slrm_pairwise_missing_does_not_drop_case() {
        let data = SlrmData {
            numeric: vec![
                num("x", &[Some(0.0), None, Some(1.0)]),
                num_f("z", &[0.0, 1.0, 2.0]),
            ],
            nominal: Vec::new(),
            target: nom("o", &["A"], &[Some(0), Some(0), Some(0)]),
            response: vec![Some(true), Some(false), Some(true)],
            frequency: None,
        };
        let model = train(&data, &Settings::default()).expect("train");
        assert_eq!(model.n_cases(0), Some(3));
        let pcs_x = model.numeric_pair_counts(0, 0).expect("x");
        let total_x: u64 = pcs_x.iter().map(|p| p.n0 + p.n1).sum();
        assert_eq!(total_x, 2);
        let pcs_z = model.numeric_pair_counts(0, 1).expect("z");
        let total_z: u64 = pcs_z.iter().map(|p| p.n0 + p.n1).sum();
        assert_eq!(total_z, 3);
    }
}
