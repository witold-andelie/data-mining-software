//! K-nearest neighbours (IBM SPSS Modeler 19 Algorithms Guide, "KNN Algorithms", pp. 237–243).
//!
//! Pure algorithm module: the caller supplies plain column vectors ([`KnnData`])
//! and [`Settings`]. Training stores the complete cases, optionally rescales
//! continuous features, optionally selects features and `k`, and optionally
//! weights dimensions by feature importance. Neighbour search is exact (the
//! Guide's tree is an optimisation, not used). Prediction follows Guide p. 242.
//!
//! Formulas are taken from the page images `target/grok-knn/pages/guide_p237.png`
//! … `guide_p243.png`. The chapter text `target/grok-knn/guide_knn_chapter.txt`
//! is used for prose only.
//!
//! # Decisions the Guide does not settle
//!
//! Fixed and verified against the saved Modeler model
//! (`target/grok-knn/model/saved_knn_model.pmml.txt`, trained on
//! `target/grok-knn/data/car_sales_type_output.csv` from the `car_sales_knn`
//! demo stream). The PMML has a 4-byte prefix `00 00 76 31` then XML.
//! `AnalysisOptions` = `Euclidean;Normalized;one-of-c;Mean;3;9`. There is no
//! target (`response=""`, MiningSchema nine `active` fields only).
//!
//! A. **Adjusted normalisation maps to \([-1, 1]\)**. Guide p. 238:
//!    \(x_{pn} = 2(x^0_{pn} - \min_p)/(\max_p - \min_p) - 1\). Min/max are the
//!    complete-case training extrema of each predictor. Reproduces the saved
//!    Modeler model: all 462 stored distances match Euclidean on that space
//!    with max relative error \(3.81\times 10^{-15}\). Scaling to \([0, 1]\)
//!    instead yields relative error \(0.5\) on every distance (exactly half).
//!    The DataDictionary intervals match those extrema (mpg max 46 lives on a
//!    `partition=1` row; excluding it would make mpg max 45 and the distances
//!    would not match).
//! B. **Distance is unweighted Euclidean**. Guide p. 238 with \(w_{(p)}=1\)
//!    (p. 239: weight is 1 when feature importance is not used). PMML
//!    `DistMetric=Euclidean`, `IsFeatureWeight=false`. Reproduces the saved
//!    Modeler model (same 462 distances as A). City-block and z-score
//!    encodings do not match.
//! C. **\(k=3\) is specified, not chosen by V-fold CV**. PMML
//!    `IsAutoKSelection=false`, `SpecifiedK=3`. Reproduces the saved Modeler
//!    model: every `neighborsDistances` row has three neighbours.
//! D. **Case ids are 1-based input row numbers (`$casecount`)**. Dropped
//!    incomplete rows still consume a number. Neighbour ids in the saved table
//!    are those `$casecount` values, not sequential `ID` 1..154 (using `ID`
//!    as the neighbour key mismatches). Reproduces the saved Modeler model:
//!    all 154 neighbour triples match. Five CSV rows are dropped (1-based
//!    3, 16, 34, 39, 110) because a predictor is blank (Guide p. 240).
//! E. **A training case is not its own neighbour**. Including self mismatches
//!    all 154 saved rows (self would be nearest with distance 0). Reproduces
//!    the saved Modeler model with self excluded.
//! F. **No target; both partitions are training cases**. MiningSchema has no
//!    predicted field. `partition` is `FocalVar` only. `originalData` holds
//!    152 `partition=0` and 2 `partition=1` complete rows (CSV has 159 data
//!    rows: 157 + 2). Reproduces the saved Modeler model only when all 154
//!    complete rows are stored (see mpg max in A).
//! G. **Feature selection, importance weighting, and automatic \(k\) are off**.
//!    PMML `IsFeatureSelection=false`, `IsFeatureImportance=false`,
//!    `IsFeatureWeight=false`, `IsAutoKSelection=false`, `selectedFeatures=9`.
//!    Reproduces the saved Modeler model with every predictor kept and
//!    \(w_{(p)}=1\).
//!
//! Follows the Algorithms Guide (unverified against a saved Modeler model):
//!
//! 1. **Neighbour-list ties**. The saved table has no equal distances, so the
//!    order of equal-distance neighbours is unverified. We sort by increasing
//!    distance, then by increasing case id.
//! 2. **City-block distance** (Guide p. 238), **no rescaling**, **one-of-c**
//!    categorical encoding (p. 238), **majority vote / Laplace** and
//!    **mean / median** scoring (p. 242), **wrapper feature selection** and
//!    **V-fold \(k\) selection** (pp. 239–240), **feature importance**
//!    \(FI_{(p)}=e_{(p)}+1/m\) (p. 241) then \(w_{(p)}=FI_{(p)}/\sum FI\)
//!    (p. 239). Error used for selection/importance is the classification
//!    fraction wrong, or the sum of squared residual, not the displayed
//!    percent (the ratio tests are then scale-free).
//! 3. **Constant predictor under rescaling**. Guide p. 238 divides by
//!    \(\max-\min\). A zero range is encoded as \(0\) (the centre of
//!    \([-1,1]\)). Unverified.
//! 4. **Fewer than \(k\) candidates**. The Guide does not say what to do.
//!    Search returns every available candidate (a query against \(n\) stored
//!    cases yields at most \(n\); a stored case looking at the others, at
//!    most \(n-1\)).
//! 5. **Scaling is estimated once** on all complete training cases. V-fold
//!    and leave-one-out only change the neighbour pool. Unverified (the saved
//!    model does not select \(k\)).
//! 6. **Feature-level weights on one-of-c**. Importance is computed per
//!    original predictor (Guide p. 239: the encoded dummy set is added
//!    together). Each dummy of that predictor then carries that predictor's
//!    \(w_{(p)}\). Unverified.
//!
//! Explicit `GAP:` errors:
//!
//! - Automatic \(k\), or combined \(k\) and feature selection, when no fold
//!   vector is supplied: Guide p. 239 assumes a CV variable with values
//!   \(1,\ldots,V\) and does not define Modeler's RNG for random folds.
//! - Feature selection, importance weighting, or automatic \(k\) when there
//!   is no target: those procedures need an error rate or SSE.

use crate::AlgoError;
use std::cmp::Ordering;
use std::fmt::Write as _;

/// Maximum complete cases stored.
const MAX_RECORDS: usize = 100_000;
/// Maximum original predictors (numeric + nominal).
const MAX_PREDICTORS: usize = 512;
/// Maximum encoded dimensions (one-of-c expands nominals).
const MAX_ENCODED_DIM: usize = 4_096;
/// Maximum categories on one nominal predictor.
const MAX_CATEGORIES: usize = 1_000;
/// Maximum requested \(k\).
const MAX_K: usize = 10_000;
/// Maximum distinct fold labels.
const MAX_FOLDS: usize = 1_000;
/// Bound on `n * n * dim` work for leave-one-out / pairwise search.
const MAX_PAIR_WORK: u128 = 50_000_000;
/// Bound on feature-selection / k-selection leave-one-out evaluations.
const MAX_SELECTION_EVALS: u128 = 2_000_000;

/// Error returned by KNN training and scoring.
pub type KnnError = AlgoError;

fn gap(what: &str) -> KnnError {
    AlgoError::Unsupported(format!("GAP: KNN {what}"))
}

fn data_err(message: impl Into<String>) -> KnnError {
    AlgoError::Data(message.into())
}

fn settings_err(message: impl Into<String>) -> KnnError {
    AlgoError::Settings(message.into())
}

/// Distance metric (`distance_computation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistanceMetric {
    /// Square root of the weighted sum of squared differences (Guide p. 238).
    Euclidean,
    /// Weighted sum of absolute differences (Guide p. 238).
    CityBlock,
}

/// How a continuous target is predicted from the neighbour responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangePrediction {
    /// Arithmetic mean of the \(k\) neighbour responses (Guide p. 242).
    Mean,
    /// Median of the \(k\) neighbour responses (Guide p. 242).
    Median,
}

/// Stopping rule for wrapper feature selection (Guide p. 240).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SelectionStopping {
    /// Add this many features on top of the forced set. `None` uses
    /// \(J_{add}=\max\{\min(20,P^0)-J_{forced},0\}\).
    FixedCount(Option<usize>),
    /// Stop on the relative change in error; `min_change` is \(\Delta_{min}\).
    ErrorRatio {
        /// Specified minimum relative change \(\Delta_{min}\) (PMML default 0.01).
        min_change: f64,
    },
}

/// A continuous predictor column (`None` = missing).
#[derive(Debug, Clone, PartialEq)]
pub struct NumericColumn {
    /// Field name.
    pub name: String,
    /// Per-record values.
    pub values: Vec<Option<f64>>,
}

/// A nominal predictor column: zero-based codes into `labels`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NominalColumn {
    /// Field name.
    pub name: String,
    /// Category labels in code order.
    pub labels: Vec<String>,
    /// Per-record codes (`None` = missing).
    pub codes: Vec<Option<u32>>,
}

/// Optional target column.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// Continuous response.
    Continuous {
        /// Field name.
        name: String,
        /// Per-record values (`None` = missing).
        values: Vec<Option<f64>>,
    },
    /// Categorical response.
    Categorical {
        /// Field name.
        name: String,
        /// Category labels in code order.
        labels: Vec<String>,
        /// Per-record codes (`None` = missing).
        codes: Vec<Option<u32>>,
    },
}

/// Training table: numeric columns, nominal columns, optional target.
///
/// All columns must have the same length. Case ids default to 1-based row
/// numbers of this table (Modeler's `$casecount`), including rows later
/// dropped as incomplete.
#[derive(Debug, Clone, PartialEq)]
pub struct KnnData {
    /// Continuous predictors, in caller order.
    pub numeric: Vec<NumericColumn>,
    /// Nominal predictors, in caller order (after the numeric ones in the
    /// combined predictor index used by [`Settings::forced_entry`]).
    pub nominal: Vec<NominalColumn>,
    /// Optional target. Identify-neighbours models leave this `None`.
    pub target: Option<Target>,
    /// Optional case ids, one per input row. `None` means `1, 2, …, n`.
    pub case_ids: Option<Vec<i64>>,
    /// Optional V-fold labels (`1..=V`) for automatic \(k\) / combined
    /// selection, one per input row. Missing fold on a complete case is an
    /// error when automatic \(k\) is requested.
    pub folds: Option<Vec<Option<u32>>>,
}

/// A query record aligned with the training predictors.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    /// Numeric predictor values, same order as [`KnnData::numeric`].
    pub numeric: Vec<Option<f64>>,
    /// Nominal codes, same order as [`KnnData::nominal`].
    pub nominal: Vec<Option<u32>>,
}

/// Build-node settings. [`Default`] matches the saved `car_sales_knn` model.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Adjusted normalisation of continuous features (Guide p. 238; default true).
    pub rescale: bool,
    /// Distance metric (default Euclidean).
    pub metric: DistanceMetric,
    /// Specified \(k\) when [`Self::automatic_k`] is false (default 3).
    pub k: usize,
    /// Select \(k\) by V-fold CV in `min_k..=max_k` (default false).
    pub automatic_k: bool,
    /// Inclusive lower bound for automatic \(k\) (default 3).
    pub min_k: usize,
    /// Inclusive upper bound for automatic \(k\) (default 5).
    pub max_k: usize,
    /// Weight encoded dimensions by normalised feature importance (default false).
    pub weight_by_importance: bool,
    /// Mean or median for a continuous target (default Mean).
    pub range_prediction: RangePrediction,
    /// Wrapper forward feature selection (default false).
    pub feature_selection: bool,
    /// Combined-predictor indices forced into the model when selecting features.
    pub forced_entry: Vec<usize>,
    /// Feature-selection stopping rule (default automatic fixed count).
    pub selection_stopping: SelectionStopping,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            rescale: true,
            metric: DistanceMetric::Euclidean,
            k: 3,
            automatic_k: false,
            min_k: 3,
            max_k: 5,
            weight_by_importance: false,
            range_prediction: RangePrediction::Mean,
            feature_selection: false,
            forced_entry: Vec::new(),
            selection_stopping: SelectionStopping::FixedCount(None),
        }
    }
}

/// One neighbour of a query or training case.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Neighbor {
    /// Case id (`$casecount` when the caller did not supply ids).
    pub case_id: i64,
    /// Distance in the encoded, weighted feature space.
    pub distance: f64,
}

/// Prediction for a scored record.
#[derive(Debug, Clone, PartialEq)]
pub enum Prediction {
    /// Categorical target: majority-vote label and Laplace probabilities.
    Category {
        /// Chosen category label.
        label: String,
        /// `(label, probability)` in training label order.
        probabilities: Vec<(String, f64)>,
    },
    /// Continuous target value.
    Value(f64),
    /// Incomplete query (Guide p. 242 `$null$`).
    Missing,
}

/// Trained nearest-neighbour model.
#[derive(Debug, Clone, PartialEq)]
pub struct KnnModel {
    case_ids: Vec<i64>,
    encoded: Vec<f64>,
    n: usize,
    dim: usize,
    weights: Vec<f64>,
    selected_cols: Vec<usize>,
    k: usize,
    metric: DistanceMetric,
    rescale: bool,
    numeric_names: Vec<String>,
    numeric_min: Vec<f64>,
    numeric_max: Vec<f64>,
    nominal_names: Vec<String>,
    nominal_labels: Vec<Vec<String>>,
    predictor_names: Vec<String>,
    selected_predictors: Vec<bool>,
    target_kind: TargetKind,
    range_prediction: RangePrediction,
}

#[derive(Debug, Clone, PartialEq)]
enum TargetKind {
    None,
    Continuous {
        name: String,
        values: Vec<f64>,
    },
    Categorical {
        name: String,
        labels: Vec<String>,
        codes: Vec<u32>,
        class_counts: Vec<usize>,
    },
}

struct Packed {
    case_ids: Vec<i64>,
    numeric: Vec<Vec<f64>>,
    nominal: Vec<Vec<u32>>,
    folds: Option<Vec<u32>>,
    target: TargetKind,
}

struct Encoding {
    numeric_min: Vec<f64>,
    numeric_max: Vec<f64>,
    dim: usize,
    /// Encoded-column range `[start, start+len)` of each original predictor.
    pred_cols: Vec<(usize, usize)>,
}

struct Fit<'a> {
    encoded: &'a [f64],
    n: usize,
    dim: usize,
    packed: &'a Packed,
    encoding: &'a Encoding,
    settings: &'a Settings,
}

impl KnnModel {
    /// Number of stored complete training cases.
    pub fn n_training(&self) -> usize {
        self.n
    }

    /// Selected \(k\).
    pub fn k(&self) -> usize {
        self.k
    }

    /// Distance metric used for search.
    pub fn metric(&self) -> DistanceMetric {
        self.metric
    }

    /// Whether continuous features were rescaled.
    pub fn rescale(&self) -> bool {
        self.rescale
    }

    /// Case ids of the stored training cases, in storage order.
    pub fn case_ids(&self) -> &[i64] {
        &self.case_ids
    }

    /// Original predictor names (numeric then nominal).
    pub fn predictor_names(&self) -> &[String] {
        &self.predictor_names
    }

    /// Whether each original predictor was kept after feature selection.
    pub fn selected_predictors(&self) -> &[bool] {
        &self.selected_predictors
    }

    /// Encoded-dimension weights (1 when importance weighting is off).
    pub fn weights(&self) -> &[f64] {
        &self.weights
    }

    /// Training min and max of each numeric predictor, used for rescaling.
    pub fn numeric_range(&self) -> (&[f64], &[f64]) {
        (&self.numeric_min, &self.numeric_max)
    }

    /// \(k\) nearest stored cases of `query`.
    ///
    /// Incomplete queries return an empty list (they cannot be scored;
    /// [`Self::predict`] returns [`Prediction::Missing`]). `exclude` drops a
    /// stored case with that id (used so a training case is not its own
    /// neighbour).
    pub fn neighbors(
        &self,
        query: &Query,
        exclude: Option<i64>,
    ) -> Result<Vec<Neighbor>, KnnError> {
        crate::check_cancelled()?;
        let Some(encoded) = self.encode_query(query)? else {
            return Ok(Vec::new());
        };
        self.nearest(&encoded, exclude, self.k)
    }

    /// \(k\) nearest *other* training cases of every stored case, in storage
    /// order. Reproduces the saved Modeler `neighborsDistances` table on
    /// `car_sales_knn` (ids and distances; see module docs A–E).
    pub fn training_neighbors(&self) -> Result<Vec<Vec<Neighbor>>, KnnError> {
        self.check_pair_work()?;
        let mut out = Vec::with_capacity(self.n);
        for i in 0..self.n {
            crate::check_cancelled()?;
            let row = self
                .row(i)
                .ok_or_else(|| data_err("internal: training row missing"))?;
            let exclude = self.case_ids.get(i).copied();
            out.push(self.nearest(row, exclude, self.k)?);
        }
        Ok(out)
    }

    /// Classify or regress `query` from its \(k\) nearest training cases.
    ///
    /// A model trained without a target returns a data error. An incomplete
    /// query returns [`Prediction::Missing`].
    pub fn predict(&self, query: &Query) -> Result<Prediction, KnnError> {
        crate::check_cancelled()?;
        if matches!(self.target_kind, TargetKind::None) {
            return Err(data_err(
                "KNN model has no target (identify-neighbours); cannot predict",
            ));
        }
        let Some(encoded) = self.encode_query(query)? else {
            return Ok(Prediction::Missing);
        };
        let neighbors = self.nearest(&encoded, None, self.k)?;
        self.predict_from_neighbors(&neighbors)
    }

    /// Classify or regress a stored training case from its other neighbours
    /// (leave-one-out). A model without a target returns a data error.
    pub fn predict_training_case(&self, case_id: i64) -> Result<Prediction, KnnError> {
        if matches!(self.target_kind, TargetKind::None) {
            return Err(data_err(
                "KNN model has no target (identify-neighbours); cannot predict",
            ));
        }
        let pos = self
            .case_ids
            .iter()
            .position(|id| *id == case_id)
            .ok_or_else(|| data_err(format!("no training case with id {case_id}")))?;
        let row = self
            .row(pos)
            .ok_or_else(|| data_err("internal: training row missing"))?;
        let neighbors = self.nearest(row, Some(case_id), self.k)?;
        self.predict_from_neighbors(&neighbors)
    }

    /// Plain-text summary of \(k\), the metric, scaling, weights, and size.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let metric = match self.metric {
            DistanceMetric::Euclidean => "Euclidean",
            DistanceMetric::CityBlock => "CityBlock",
        };
        let target = match &self.target_kind {
            TargetKind::None => "none".to_string(),
            TargetKind::Continuous { name, .. } => format!("continuous:{name}"),
            TargetKind::Categorical { name, labels, .. } => {
                format!("categorical:{name}({} classes)", labels.len())
            }
        };
        let _ = writeln!(
            out,
            "KNN(k={}, metric={metric}, rescale={}, n={}, dim={}, target={target})",
            self.k, self.rescale, self.n, self.dim
        );
        for (i, name) in self.predictor_names.iter().enumerate() {
            let kept = self.selected_predictors.get(i).copied().unwrap_or(false);
            let _ = writeln!(out, "  predictor[{i}] {name} selected={kept}");
        }
        for (i, (name, (lo, hi))) in self
            .numeric_names
            .iter()
            .zip(self.numeric_min.iter().zip(self.numeric_max.iter()))
            .enumerate()
        {
            let _ = writeln!(out, "  numeric[{i}] {name} min={lo:.10} max={hi:.10}");
        }
        for (i, w) in self.weights.iter().enumerate() {
            let _ = writeln!(out, "  weight[{i}]={w:.10}");
        }
        out
    }

    fn row(&self, i: usize) -> Option<&[f64]> {
        let start = i.checked_mul(self.dim)?;
        let end = start.checked_add(self.dim)?;
        self.encoded.get(start..end)
    }

    fn check_pair_work(&self) -> Result<(), KnnError> {
        check_pair_limit(self.n, self.selected_cols.len())
    }

    fn encode_query(&self, query: &Query) -> Result<Option<Vec<f64>>, KnnError> {
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
        let mut encoded = vec![0.0; self.dim];
        let mut col = 0;
        for (j, value) in query.numeric.iter().enumerate() {
            let Some(x) = *value else {
                return Ok(None);
            };
            if !x.is_finite() {
                return Err(data_err(format!(
                    "non-finite numeric query value for '{}'",
                    self.numeric_names.get(j).map_or("?", String::as_str)
                )));
            }
            let lo = self
                .numeric_min
                .get(j)
                .copied()
                .ok_or_else(|| data_err("internal: numeric min missing"))?;
            let hi = self
                .numeric_max
                .get(j)
                .copied()
                .ok_or_else(|| data_err("internal: numeric max missing"))?;
            let z = scale(x, lo, hi, self.rescale)?;
            let slot = encoded
                .get_mut(col)
                .ok_or_else(|| data_err("internal: encoded query overflow"))?;
            *slot = z;
            col += 1;
        }
        for (j, code) in query.nominal.iter().enumerate() {
            let labels = self
                .nominal_labels
                .get(j)
                .ok_or_else(|| data_err("internal: nominal labels missing"))?;
            let Some(code) = *code else {
                return Ok(None);
            };
            let idx = usize::try_from(code)
                .map_err(|_| data_err("nominal query code does not fit usize"))?;
            if idx >= labels.len() {
                return Err(data_err(format!(
                    "nominal query code {code} out of range for '{}'",
                    self.nominal_names.get(j).map_or("?", String::as_str)
                )));
            }
            for c in 0..labels.len() {
                let slot = encoded
                    .get_mut(col)
                    .ok_or_else(|| data_err("internal: encoded query overflow"))?;
                *slot = if c == idx { 1.0 } else { 0.0 };
                col += 1;
            }
        }
        Ok(Some(encoded))
    }

    fn nearest(
        &self,
        query: &[f64],
        exclude: Option<i64>,
        k: usize,
    ) -> Result<Vec<Neighbor>, KnnError> {
        if k == 0 {
            return Ok(Vec::new());
        }
        let mut found: Vec<Neighbor> = Vec::new();
        for i in 0..self.n {
            let id = self
                .case_ids
                .get(i)
                .copied()
                .ok_or_else(|| data_err("internal: case id missing"))?;
            if exclude == Some(id) {
                continue;
            }
            let row = self
                .row(i)
                .ok_or_else(|| data_err("internal: training row missing"))?;
            let d = distance(query, row, &self.weights, &self.selected_cols, self.metric)?;
            found.push(Neighbor {
                case_id: id,
                distance: d,
            });
        }
        sort_neighbors(&mut found);
        if found.len() > k {
            found.truncate(k);
        }
        Ok(found)
    }

    fn predict_from_neighbors(&self, neighbors: &[Neighbor]) -> Result<Prediction, KnnError> {
        if neighbors.is_empty() {
            return Ok(Prediction::Missing);
        }
        match &self.target_kind {
            TargetKind::None => Err(data_err("KNN model has no target")),
            TargetKind::Continuous { values, .. } => {
                let mut ys = Vec::with_capacity(neighbors.len());
                for n in neighbors {
                    let pos = index_of_id(&self.case_ids, n.case_id)?;
                    let y = values
                        .get(pos)
                        .copied()
                        .ok_or_else(|| data_err("internal: target value missing"))?;
                    ys.push(y);
                }
                let yhat = match self.range_prediction {
                    RangePrediction::Mean => mean(&ys)?,
                    RangePrediction::Median => median(&ys)?,
                };
                Ok(Prediction::Value(yhat))
            }
            TargetKind::Categorical {
                labels,
                codes,
                class_counts,
                ..
            } => {
                let j_cats = labels.len();
                if j_cats == 0 {
                    return Err(data_err("categorical target has no labels"));
                }
                let mut votes = vec![0usize; j_cats];
                for n in neighbors {
                    let pos = index_of_id(&self.case_ids, n.case_id)?;
                    let code = codes
                        .get(pos)
                        .copied()
                        .ok_or_else(|| data_err("internal: target code missing"))?;
                    let idx = usize::try_from(code)
                        .map_err(|_| data_err("target code does not fit usize"))?;
                    let slot = votes
                        .get_mut(idx)
                        .ok_or_else(|| data_err("target code out of range"))?;
                    *slot += 1;
                }
                let k_used = neighbors.len();
                let denom = (k_used as f64) + (j_cats as f64);
                if denom == 0.0 {
                    return Err(data_err("Laplace denominator is zero"));
                }
                let mut probabilities = Vec::with_capacity(j_cats);
                for (idx, label) in labels.iter().enumerate() {
                    let kj = votes.get(idx).copied().unwrap_or(0) as f64;
                    probabilities.push((label.clone(), (kj + 1.0) / denom));
                }
                let label = majority_label(labels, &votes, class_counts)?;
                Ok(Prediction::Category {
                    label,
                    probabilities,
                })
            }
        }
    }
}

/// Trains a nearest-neighbour model on the complete cases of `data`.
pub fn train(data: &KnnData, settings: &Settings) -> Result<KnnModel, KnnError> {
    crate::check_cancelled()?;
    validate_settings(settings)?;
    let packed = pack(data)?;
    let encoding = build_encoding(data, &packed)?;
    let encoded = encode_all(&packed, &encoding, settings.rescale)?;
    let n = packed.case_ids.len();
    let dim = encoding.dim;
    let n_pred = data.numeric.len() + data.nominal.len();
    let (selected_pred, k, selected_cols, weights) = {
        let fit = Fit {
            encoded: &encoded,
            n,
            dim,
            packed: &packed,
            encoding: &encoding,
            settings,
        };
        let mut selected_pred = vec![true; n_pred];
        let mut k = settings.k;
        if settings.automatic_k && settings.feature_selection {
            let (best_k, mask) = combined_k_and_features(&fit)?;
            k = best_k;
            selected_pred = mask;
        } else if settings.feature_selection {
            selected_pred = forward_select(&fit, settings.k)?;
        } else if settings.automatic_k {
            k = select_k(&fit, &selected_pred)?;
        }
        let selected_cols = cols_of(&selected_pred, &encoding);
        let weights = if settings.weight_by_importance {
            importance_weights(&fit, &selected_pred, &selected_cols, k)?
        } else {
            vec![1.0; dim]
        };
        (selected_pred, k, selected_cols, weights)
    };

    let mut numeric_names = Vec::with_capacity(data.numeric.len());
    for col in &data.numeric {
        numeric_names.push(col.name.clone());
    }
    let mut nominal_names = Vec::new();
    let mut nominal_labels = Vec::new();
    for col in &data.nominal {
        nominal_names.push(col.name.clone());
        nominal_labels.push(col.labels.clone());
    }
    let mut predictor_names = Vec::with_capacity(n_pred);
    for col in &data.numeric {
        predictor_names.push(col.name.clone());
    }
    for col in &data.nominal {
        predictor_names.push(col.name.clone());
    }

    Ok(KnnModel {
        case_ids: packed.case_ids,
        encoded,
        n,
        dim,
        weights,
        selected_cols,
        k,
        metric: settings.metric,
        rescale: settings.rescale,
        numeric_names,
        numeric_min: encoding.numeric_min,
        numeric_max: encoding.numeric_max,
        nominal_names,
        nominal_labels,
        predictor_names,
        selected_predictors: selected_pred,
        target_kind: packed.target,
        range_prediction: settings.range_prediction,
    })
}

fn validate_settings(settings: &Settings) -> Result<(), KnnError> {
    if settings.k == 0 || settings.k > MAX_K {
        return Err(settings_err(format!(
            "k={} is outside 1..={MAX_K}",
            settings.k
        )));
    }
    if settings.min_k == 0
        || settings.max_k == 0
        || settings.min_k > MAX_K
        || settings.max_k > MAX_K
    {
        return Err(settings_err(format!(
            "automatic k range {}..={} is outside 1..={MAX_K}",
            settings.min_k, settings.max_k
        )));
    }
    if settings.min_k > settings.max_k {
        return Err(settings_err(format!(
            "min_k {} is greater than max_k {}",
            settings.min_k, settings.max_k
        )));
    }
    if let SelectionStopping::ErrorRatio { min_change } = settings.selection_stopping
        && !(min_change.is_finite() && min_change >= 0.0)
    {
        return Err(settings_err(format!(
            "feature-selection min_change {min_change} must be finite and >= 0"
        )));
    }
    if let SelectionStopping::FixedCount(Some(count)) = settings.selection_stopping
        && count > MAX_PREDICTORS
    {
        return Err(settings_err(format!(
            "feature-selection count {count} exceeds {MAX_PREDICTORS}"
        )));
    }
    Ok(())
}

fn n_rows(data: &KnnData) -> Result<usize, KnnError> {
    let mut n = None;
    let mut consider = |len: usize, what: &str| -> Result<(), KnnError> {
        match n {
            None => n = Some(len),
            Some(existing) if existing == len => {}
            Some(existing) => {
                return Err(data_err(format!(
                    "column '{what}' has {len} rows, expected {existing}"
                )));
            }
        }
        Ok(())
    };
    for col in &data.numeric {
        consider(col.values.len(), &col.name)?;
    }
    for col in &data.nominal {
        consider(col.codes.len(), &col.name)?;
        if col.labels.len() > MAX_CATEGORIES {
            return Err(data_err(format!(
                "nominal '{}' has {} categories, max {MAX_CATEGORIES}",
                col.name,
                col.labels.len()
            )));
        }
        if col.labels.is_empty() {
            return Err(data_err(format!(
                "nominal '{}' has no category labels",
                col.name
            )));
        }
        for (a, lab) in col.labels.iter().enumerate() {
            if col.labels.iter().take(a).any(|s| s == lab) {
                return Err(data_err(format!(
                    "nominal '{}' has duplicate label '{lab}'",
                    col.name
                )));
            }
        }
    }
    match &data.target {
        Some(Target::Continuous { name, values }) => consider(values.len(), name)?,
        Some(Target::Categorical {
            name,
            codes,
            labels,
        }) => {
            consider(codes.len(), name)?;
            if labels.is_empty() {
                return Err(data_err(format!("target '{name}' has no category labels")));
            }
            if labels.len() > MAX_CATEGORIES {
                return Err(data_err(format!(
                    "target '{name}' has {} categories, max {MAX_CATEGORIES}",
                    labels.len()
                )));
            }
        }
        None => {}
    }
    if let Some(ids) = &data.case_ids {
        consider(ids.len(), "case_ids")?;
    }
    if let Some(folds) = &data.folds {
        consider(folds.len(), "folds")?;
    }
    n.ok_or_else(|| data_err("KNN data has no predictor or target columns"))
}

fn pack(data: &KnnData) -> Result<Packed, KnnError> {
    let n_in = n_rows(data)?;
    if data.numeric.is_empty() && data.nominal.is_empty() {
        return Err(data_err("KNN needs at least one predictor"));
    }
    let n_pred = data.numeric.len() + data.nominal.len();
    if n_pred > MAX_PREDICTORS {
        return Err(data_err(format!(
            "KNN has {n_pred} predictors, max {MAX_PREDICTORS}"
        )));
    }
    let mut case_ids = Vec::new();
    let mut numeric: Vec<Vec<f64>> = vec![Vec::new(); data.numeric.len()];
    let mut nominal: Vec<Vec<u32>> = vec![Vec::new(); data.nominal.len()];
    let mut folds_out: Option<Vec<u32>> = data.folds.as_ref().map(|_| Vec::new());
    let mut cont_y: Option<Vec<f64>> = None;
    let mut cat_y: Option<Vec<u32>> = None;
    let mut cat_counts: Vec<usize> = match &data.target {
        Some(Target::Categorical { labels, .. }) => vec![0; labels.len()],
        _ => Vec::new(),
    };

    for i in 0..n_in {
        crate::check_cancelled()?;
        if !row_complete(data, i)? {
            continue;
        }
        let id = match &data.case_ids {
            Some(ids) => ids
                .get(i)
                .copied()
                .ok_or_else(|| data_err("case id missing"))?,
            None => {
                let plus = i
                    .checked_add(1)
                    .ok_or_else(|| data_err("case id overflow"))?;
                i64::try_from(plus).map_err(|_| data_err("case id overflow"))?
            }
        };
        if case_ids.contains(&id) {
            return Err(data_err(format!("duplicate case id {id}")));
        }
        for (j, col) in data.numeric.iter().enumerate() {
            let x = col
                .values
                .get(i)
                .and_then(|v| *v)
                .ok_or_else(|| data_err("internal: complete numeric missing"))?;
            numeric
                .get_mut(j)
                .ok_or_else(|| data_err("internal: numeric column missing"))?
                .push(x);
        }
        for (j, col) in data.nominal.iter().enumerate() {
            let code = col
                .codes
                .get(i)
                .and_then(|v| *v)
                .ok_or_else(|| data_err("internal: complete nominal missing"))?;
            nominal
                .get_mut(j)
                .ok_or_else(|| data_err("internal: nominal column missing"))?
                .push(code);
        }
        if let (Some(src), Some(dst)) = (&data.folds, folds_out.as_mut()) {
            let fold = src
                .get(i)
                .and_then(|v| *v)
                .ok_or_else(|| data_err(format!("complete case {id} has no fold label")))?;
            if fold == 0 {
                return Err(data_err("fold labels must be >= 1"));
            }
            dst.push(fold);
        }
        match &data.target {
            Some(Target::Continuous { values, .. }) => {
                let y = values
                    .get(i)
                    .and_then(|v| *v)
                    .ok_or_else(|| data_err("internal: complete target missing"))?;
                cont_y.get_or_insert_with(Vec::new).push(y);
            }
            Some(Target::Categorical { codes, labels, .. }) => {
                let code = codes
                    .get(i)
                    .and_then(|v| *v)
                    .ok_or_else(|| data_err("internal: complete target missing"))?;
                let idx = usize::try_from(code).map_err(|_| data_err("target code overflow"))?;
                if idx >= labels.len() {
                    return Err(data_err("target code out of range"));
                }
                if let Some(c) = cat_counts.get_mut(idx) {
                    *c += 1;
                }
                cat_y.get_or_insert_with(Vec::new).push(code);
            }
            None => {}
        }
        case_ids.push(id);
        if case_ids.len() > MAX_RECORDS {
            return Err(data_err(format!("KNN complete cases exceed {MAX_RECORDS}")));
        }
    }
    if case_ids.is_empty() {
        return Err(data_err(
            "no complete cases (every row is missing an input or the target)",
        ));
    }
    let target = match &data.target {
        None => TargetKind::None,
        Some(Target::Continuous { name, .. }) => TargetKind::Continuous {
            name: name.clone(),
            values: cont_y.ok_or_else(|| data_err("continuous target values missing"))?,
        },
        Some(Target::Categorical { name, labels, .. }) => TargetKind::Categorical {
            name: name.clone(),
            labels: labels.clone(),
            codes: cat_y.ok_or_else(|| data_err("categorical target codes missing"))?,
            class_counts: cat_counts,
        },
    };
    Ok(Packed {
        case_ids,
        numeric,
        nominal,
        folds: folds_out,
        target,
    })
}

fn row_complete(data: &KnnData, i: usize) -> Result<bool, KnnError> {
    for col in &data.numeric {
        match col.values.get(i) {
            None => {
                return Err(data_err(format!(
                    "numeric '{}' is shorter than the table",
                    col.name
                )));
            }
            Some(None) => return Ok(false),
            Some(Some(x)) if !x.is_finite() => {
                return Err(data_err(format!(
                    "non-finite value in numeric '{}' row {i}",
                    col.name
                )));
            }
            Some(Some(_)) => {}
        }
    }
    for col in &data.nominal {
        match col.codes.get(i) {
            None => {
                return Err(data_err(format!(
                    "nominal '{}' is shorter than the table",
                    col.name
                )));
            }
            Some(None) => return Ok(false),
            Some(Some(code)) => {
                let idx = usize::try_from(*code).map_err(|_| data_err("nominal code overflow"))?;
                if idx >= col.labels.len() {
                    return Err(data_err(format!(
                        "nominal '{}' code {code} out of range",
                        col.name
                    )));
                }
            }
        }
    }
    match &data.target {
        Some(Target::Continuous { values, name }) => match values.get(i) {
            None => {
                return Err(data_err(format!(
                    "target '{name}' is shorter than the table"
                )));
            }
            Some(None) => return Ok(false),
            Some(Some(y)) if !y.is_finite() => {
                return Err(data_err(format!("non-finite target '{name}' row {i}")));
            }
            Some(Some(_)) => {}
        },
        Some(Target::Categorical {
            codes,
            labels,
            name,
        }) => match codes.get(i) {
            None => {
                return Err(data_err(format!(
                    "target '{name}' is shorter than the table"
                )));
            }
            Some(None) => return Ok(false),
            Some(Some(code)) => {
                let idx = usize::try_from(*code).map_err(|_| data_err("target code overflow"))?;
                if idx >= labels.len() {
                    return Err(data_err(format!(
                        "target '{name}' code {code} out of range"
                    )));
                }
            }
        },
        None => {}
    }
    Ok(true)
}

fn build_encoding(data: &KnnData, packed: &Packed) -> Result<Encoding, KnnError> {
    let mut numeric_min = Vec::new();
    let mut numeric_max = Vec::new();
    for col in &packed.numeric {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for &x in col {
            lo = lo.min(x);
            hi = hi.max(x);
        }
        if !lo.is_finite() || !hi.is_finite() {
            return Err(data_err("numeric predictor has no finite values"));
        }
        numeric_min.push(lo);
        numeric_max.push(hi);
    }
    let mut pred_cols = Vec::new();
    let mut start = 0usize;
    for _ in &data.numeric {
        pred_cols.push((start, 1));
        start = start
            .checked_add(1)
            .ok_or_else(|| data_err("encoded dimension overflow"))?;
    }
    for col in &data.nominal {
        let card = col.labels.len();
        pred_cols.push((start, card));
        start = start
            .checked_add(card)
            .ok_or_else(|| data_err("encoded dimension overflow"))?;
    }
    if start > MAX_ENCODED_DIM {
        return Err(data_err(format!(
            "encoded dimension {start} exceeds {MAX_ENCODED_DIM}"
        )));
    }
    if start == 0 {
        return Err(data_err("KNN encoded dimension is 0"));
    }
    Ok(Encoding {
        numeric_min,
        numeric_max,
        dim: start,
        pred_cols,
    })
}

fn encode_all(packed: &Packed, encoding: &Encoding, rescale: bool) -> Result<Vec<f64>, KnnError> {
    let n = packed.case_ids.len();
    let dim = encoding.dim;
    let cells = n
        .checked_mul(dim)
        .ok_or_else(|| data_err("encoded table size overflow"))?;
    let mut out = vec![0.0; cells];
    for i in 0..n {
        crate::check_cancelled()?;
        let row_start = i
            .checked_mul(dim)
            .ok_or_else(|| data_err("encoded row overflow"))?;
        let mut col = 0usize;
        for (j, xs) in packed.numeric.iter().enumerate() {
            let x = xs
                .get(i)
                .copied()
                .ok_or_else(|| data_err("internal: packed numeric missing"))?;
            let lo = encoding
                .numeric_min
                .get(j)
                .copied()
                .ok_or_else(|| data_err("internal: min missing"))?;
            let hi = encoding
                .numeric_max
                .get(j)
                .copied()
                .ok_or_else(|| data_err("internal: max missing"))?;
            let z = scale(x, lo, hi, rescale)?;
            let slot = out
                .get_mut(row_start + col)
                .ok_or_else(|| data_err("internal: encode overflow"))?;
            *slot = z;
            col += 1;
        }
        for (j, codes) in packed.nominal.iter().enumerate() {
            let code = codes
                .get(i)
                .copied()
                .ok_or_else(|| data_err("internal: packed nominal missing"))?;
            let pred_index = packed
                .numeric
                .len()
                .checked_add(j)
                .ok_or_else(|| data_err("predictor index overflow"))?;
            let (_start, card) = encoding
                .pred_cols
                .get(pred_index)
                .copied()
                .ok_or_else(|| data_err("internal: nominal dummy range missing"))?;
            let idx = usize::try_from(code).map_err(|_| data_err("nominal code overflow"))?;
            if idx >= card {
                return Err(data_err("nominal dummy index out of range"));
            }
            for c in 0..card {
                let slot = out
                    .get_mut(row_start + col)
                    .ok_or_else(|| data_err("internal: encode overflow"))?;
                *slot = if c == idx { 1.0 } else { 0.0 };
                col += 1;
            }
        }
        if col != dim {
            return Err(data_err("internal: encoded row width mismatch"));
        }
    }
    Ok(out)
}

fn scale(x: f64, lo: f64, hi: f64, rescale: bool) -> Result<f64, KnnError> {
    if !rescale {
        return Ok(x);
    }
    let span = hi - lo;
    if span == 0.0 {
        return Ok(0.0);
    }
    let z = 2.0 * (x - lo) / span - 1.0;
    if !z.is_finite() {
        return Err(data_err(
            "adjusted normalisation produced a non-finite value",
        ));
    }
    Ok(z)
}

fn distance(
    a: &[f64],
    b: &[f64],
    weights: &[f64],
    cols: &[usize],
    metric: DistanceMetric,
) -> Result<f64, KnnError> {
    let mut acc = 0.0;
    for &p in cols {
        let av = a
            .get(p)
            .copied()
            .ok_or_else(|| data_err("distance: query column missing"))?;
        let bv = b
            .get(p)
            .copied()
            .ok_or_else(|| data_err("distance: stored column missing"))?;
        let w = weights.get(p).copied().unwrap_or(1.0);
        if !w.is_finite() || w < 0.0 {
            return Err(data_err(
                "feature weight is not a finite non-negative number",
            ));
        }
        let diff = av - bv;
        match metric {
            DistanceMetric::Euclidean => acc += w * diff * diff,
            DistanceMetric::CityBlock => acc += w * diff.abs(),
        }
    }
    let d = match metric {
        DistanceMetric::Euclidean => acc.sqrt(),
        DistanceMetric::CityBlock => acc,
    };
    if !d.is_finite() {
        return Err(data_err("distance is not finite"));
    }
    Ok(d)
}

fn cols_of(selected_pred: &[bool], encoding: &Encoding) -> Vec<usize> {
    let mut cols = Vec::new();
    for (p, &kept) in selected_pred.iter().enumerate() {
        if !kept {
            continue;
        }
        if let Some(&(start, len)) = encoding.pred_cols.get(p) {
            let end = start.saturating_add(len);
            for c in start..end {
                cols.push(c);
            }
        }
    }
    cols
}

fn require_target(packed: &Packed, what: &str) -> Result<(), KnnError> {
    match packed.target {
        TargetKind::None => Err(gap(what)),
        _ => Ok(()),
    }
}

fn combined_k_and_features(fit: &Fit<'_>) -> Result<(usize, Vec<bool>), KnnError> {
    require_target(
        fit.packed,
        "combined k and feature selection needs a target",
    )?;
    if fit.packed.folds.is_none() {
        return Err(gap(
            "automatic k selection without a fold field (the Guide does not define the RNG)",
        ));
    }
    let mut best_k = fit.settings.min_k;
    let mut best_mask: Option<Vec<bool>> = None;
    let mut best_err = f64::INFINITY;
    for k in fit.settings.min_k..=fit.settings.max_k {
        crate::check_cancelled()?;
        let mask = forward_select(fit, k)?;
        let cols = cols_of(&mask, fit.encoding);
        let err = cv_error(fit, &cols, k)?;
        if err < best_err {
            best_err = err;
            best_k = k;
            best_mask = Some(mask);
        }
    }
    let mask = best_mask.ok_or_else(|| data_err("combined selection produced no feature set"))?;
    Ok((best_k, mask))
}

fn select_k(fit: &Fit<'_>, selected_pred: &[bool]) -> Result<usize, KnnError> {
    require_target(fit.packed, "automatic k selection needs a target")?;
    if fit.packed.folds.is_none() {
        return Err(gap(
            "automatic k selection without a fold field (the Guide does not define the RNG)",
        ));
    }
    let cols = cols_of(selected_pred, fit.encoding);
    let mut best_k = fit.settings.min_k;
    let mut best_err = f64::INFINITY;
    for k in fit.settings.min_k..=fit.settings.max_k {
        crate::check_cancelled()?;
        let err = cv_error(fit, &cols, k)?;
        if err < best_err {
            best_err = err;
            best_k = k;
        }
    }
    Ok(best_k)
}

fn forward_select(fit: &Fit<'_>, k: usize) -> Result<Vec<bool>, KnnError> {
    require_target(fit.packed, "feature selection needs a target")?;
    let n_pred = fit.encoding.pred_cols.len();
    let mut selected = vec![false; n_pred];
    for &idx in &fit.settings.forced_entry {
        if idx >= n_pred {
            return Err(settings_err(format!(
                "forced_entry index {idx} is outside 0..{n_pred}"
            )));
        }
        if let Some(slot) = selected.get_mut(idx) {
            *slot = true;
        }
    }
    let j_forced = selected.iter().filter(|s| **s).count();
    let j_add = match fit.settings.selection_stopping {
        SelectionStopping::FixedCount(Some(count)) => count,
        SelectionStopping::FixedCount(None) => {
            let cap = 20usize.min(n_pred);
            cap.saturating_sub(j_forced)
        }
        SelectionStopping::ErrorRatio { .. } => n_pred.saturating_sub(j_forced),
    };
    if j_add == 0 {
        return Ok(selected);
    }

    let mut evals: u128 = 0;
    let mut current_err = loo_error(fit, &cols_of(&selected, fit.encoding), k)?;
    if current_err == 0.0 {
        return Ok(selected);
    }
    let mut added = 0usize;
    loop {
        crate::check_cancelled()?;
        if added >= j_add {
            break;
        }
        let mut best_p: Option<usize> = None;
        let mut best_err = f64::INFINITY;
        for p in 0..n_pred {
            if selected.get(p).copied().unwrap_or(true) {
                continue;
            }
            evals = evals.saturating_add(1);
            if evals > MAX_SELECTION_EVALS {
                return Err(data_err(format!(
                    "KNN feature-selection evaluations exceed {MAX_SELECTION_EVALS}"
                )));
            }
            let mut trial = selected.clone();
            if let Some(slot) = trial.get_mut(p) {
                *slot = true;
            }
            let err = loo_error(fit, &cols_of(&trial, fit.encoding), k)?;
            if err < best_err {
                best_err = err;
                best_p = Some(p);
            }
        }
        let Some(p_star) = best_p else {
            break;
        };
        let accept = match fit.settings.selection_stopping {
            SelectionStopping::FixedCount(_) => StopChoice::KeepNew,
            SelectionStopping::ErrorRatio { min_change } => {
                stop_error_ratio(current_err, best_err, min_change)
            }
        };
        match accept {
            StopChoice::KeepOld => break,
            StopChoice::KeepNew | StopChoice::Continue => {
                if let Some(slot) = selected.get_mut(p_star) {
                    *slot = true;
                }
                added += 1;
                current_err = best_err;
                if current_err == 0.0 {
                    break;
                }
                if matches!(
                    fit.settings.selection_stopping,
                    SelectionStopping::ErrorRatio { .. }
                ) && accept == StopChoice::KeepNew
                {
                    break;
                }
            }
        }
    }
    Ok(selected)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StopChoice {
    KeepNew,
    KeepOld,
    Continue,
}

fn stop_error_ratio(e_j: f64, e_next: f64, delta: f64) -> StopChoice {
    // Guide p. 240: stop and take S_{J+1} if e_{J+1}=0 or (e_J >= e_{J+1} and rel <= Δ).
    // Stop and keep S_J if e_J < e_{J+1} and rel > 2Δ.
    if !e_j.is_finite() || !e_next.is_finite() {
        return StopChoice::KeepOld;
    }
    if e_next == 0.0 {
        return StopChoice::KeepNew;
    }
    if e_j == 0.0 {
        return StopChoice::KeepOld;
    }
    let rel = (e_j - e_next).abs() / e_j;
    if e_j >= e_next && rel <= delta {
        return StopChoice::KeepNew;
    }
    if e_j < e_next && rel > 2.0 * delta {
        return StopChoice::KeepOld;
    }
    StopChoice::Continue
}

fn loo_error(fit: &Fit<'_>, cols: &[usize], k: usize) -> Result<f64, KnnError> {
    check_pair_limit(fit.n, cols.len())?;
    let weights = vec![1.0; fit.dim];
    pool_error(fit, cols, &weights, k, None)
}

fn cv_error(fit: &Fit<'_>, cols: &[usize], k: usize) -> Result<f64, KnnError> {
    let folds = fit.packed.folds.as_ref().ok_or_else(|| {
        gap("automatic k selection without a fold field (the Guide does not define the RNG)")
    })?;
    let mut labels: Vec<u32> = Vec::new();
    for &f in folds {
        if !labels.contains(&f) {
            labels.push(f);
        }
    }
    labels.sort_unstable();
    if labels.len() < 2 {
        return Err(data_err("automatic k selection needs at least two folds"));
    }
    if labels.len() > MAX_FOLDS {
        return Err(data_err(format!(
            "fold count {} exceeds {MAX_FOLDS}",
            labels.len()
        )));
    }
    check_pair_limit(fit.n, cols.len())?;
    let weights = vec![1.0; fit.dim];
    let mut sum = 0.0;
    let v = labels.len() as f64;
    for &fold in &labels {
        crate::check_cancelled()?;
        let e = pool_error(fit, cols, &weights, k, Some(fold))?;
        sum += e;
    }
    Ok(sum / v)
}

fn pool_error(
    fit: &Fit<'_>,
    cols: &[usize],
    weights: &[f64],
    k: usize,
    holdout_fold: Option<u32>,
) -> Result<f64, KnnError> {
    match &fit.packed.target {
        TargetKind::None => Err(gap("error evaluation needs a target")),
        TargetKind::Continuous { values, .. } => {
            let mut sse = 0.0;
            for i in 0..fit.n {
                if !is_eval_row(fit.packed, i, holdout_fold) {
                    continue;
                }
                let y = values
                    .get(i)
                    .copied()
                    .ok_or_else(|| data_err("internal: y missing"))?;
                let yhat = predict_index(fit, cols, weights, k, i, holdout_fold)?;
                let Some(Prediction::Value(pred)) = yhat else {
                    continue;
                };
                let d = y - pred;
                sse += d * d;
            }
            Ok(sse)
        }
        TargetKind::Categorical { codes, .. } => {
            let mut wrong = 0.0;
            let mut total = 0.0;
            for i in 0..fit.n {
                if !is_eval_row(fit.packed, i, holdout_fold) {
                    continue;
                }
                let y = codes
                    .get(i)
                    .copied()
                    .ok_or_else(|| data_err("internal: y missing"))?;
                let yhat = predict_index(fit, cols, weights, k, i, holdout_fold)?;
                let Some(Prediction::Category { label, .. }) = yhat else {
                    continue;
                };
                let got = label_index(&fit.packed.target, &label)?;
                let yi = usize::try_from(y).map_err(|_| data_err("target code overflow"))?;
                if got != yi {
                    wrong += 1.0;
                }
                total += 1.0;
            }
            if total == 0.0 {
                Ok(0.0)
            } else {
                Ok(wrong / total)
            }
        }
    }
}

fn is_eval_row(packed: &Packed, i: usize, holdout_fold: Option<u32>) -> bool {
    match holdout_fold {
        None => true,
        Some(h) => packed.folds.as_ref().and_then(|f| f.get(i).copied()) == Some(h),
    }
}

fn predict_index(
    fit: &Fit<'_>,
    cols: &[usize],
    weights: &[f64],
    k: usize,
    query_i: usize,
    holdout_fold: Option<u32>,
) -> Result<Option<Prediction>, KnnError> {
    let query = row_slice(fit.encoded, fit.n, fit.dim, query_i)?;
    let mut found: Vec<(usize, f64, i64)> = Vec::new();
    for j in 0..fit.n {
        if j == query_i {
            continue;
        }
        if let Some(h) = holdout_fold
            && let Some(folds) = fit.packed.folds.as_ref()
            && folds.get(j).copied() == Some(h)
        {
            continue;
        }
        let row = row_slice(fit.encoded, fit.n, fit.dim, j)?;
        let d = distance(query, row, weights, cols, fit.settings.metric)?;
        let id = fit
            .packed
            .case_ids
            .get(j)
            .copied()
            .ok_or_else(|| data_err("internal: case id missing"))?;
        found.push((j, d, id));
    }
    found.sort_by(|a, b| match a.1.total_cmp(&b.1) {
        Ordering::Equal => a.2.cmp(&b.2),
        other => other,
    });
    if found.len() > k {
        found.truncate(k);
    }
    if found.is_empty() {
        return Ok(None);
    }
    match &fit.packed.target {
        TargetKind::None => Ok(None),
        TargetKind::Continuous { values, .. } => {
            let mut ys = Vec::with_capacity(found.len());
            for (j, _, _) in &found {
                let y = values
                    .get(*j)
                    .copied()
                    .ok_or_else(|| data_err("internal: y missing"))?;
                ys.push(y);
            }
            let yhat = match fit.settings.range_prediction {
                RangePrediction::Mean => mean(&ys)?,
                RangePrediction::Median => median(&ys)?,
            };
            Ok(Some(Prediction::Value(yhat)))
        }
        TargetKind::Categorical {
            labels,
            codes,
            class_counts,
            ..
        } => {
            let mut votes = vec![0usize; labels.len()];
            for (j, _, _) in &found {
                let code = codes
                    .get(*j)
                    .copied()
                    .ok_or_else(|| data_err("internal: y missing"))?;
                let idx = usize::try_from(code).map_err(|_| data_err("target code overflow"))?;
                let slot = votes
                    .get_mut(idx)
                    .ok_or_else(|| data_err("target code out of range"))?;
                *slot += 1;
            }
            let label = majority_label(labels, &votes, class_counts)?;
            Ok(Some(Prediction::Category {
                label,
                probabilities: Vec::new(),
            }))
        }
    }
}

fn row_slice(encoded: &[f64], n: usize, dim: usize, i: usize) -> Result<&[f64], KnnError> {
    if i >= n {
        return Err(data_err("row index out of range"));
    }
    let start = i
        .checked_mul(dim)
        .ok_or_else(|| data_err("internal: row index overflow"))?;
    let end = start
        .checked_add(dim)
        .ok_or_else(|| data_err("internal: row index overflow"))?;
    encoded
        .get(start..end)
        .ok_or_else(|| data_err("internal: row missing"))
}

fn label_index(target: &TargetKind, label: &str) -> Result<usize, KnnError> {
    match target {
        TargetKind::Categorical { labels, .. } => labels
            .iter()
            .position(|l| l == label)
            .ok_or_else(|| data_err(format!("unknown category label '{label}'"))),
        _ => Err(data_err("label_index on a non-categorical target")),
    }
}

fn importance_weights(
    fit: &Fit<'_>,
    selected_pred: &[bool],
    selected_cols: &[usize],
    k: usize,
) -> Result<Vec<f64>, KnnError> {
    require_target(fit.packed, "feature importance weighting needs a target")?;
    let m = selected_pred.iter().filter(|s| **s).count();
    if m == 0 {
        return Ok(vec![1.0; fit.dim]);
    }
    let inv_m = 1.0 / (m as f64);
    let mut fi_pred = vec![0.0; selected_pred.len()];
    let mut sum_fi = 0.0;
    for (p, &kept) in selected_pred.iter().enumerate() {
        if !kept {
            continue;
        }
        let mut reduced = selected_pred.to_vec();
        if let Some(slot) = reduced.get_mut(p) {
            *slot = false;
        }
        let cols = cols_of(&reduced, fit.encoding);
        let e = loo_error(fit, &cols, k)?;
        let fi = e + inv_m;
        if let Some(slot) = fi_pred.get_mut(p) {
            *slot = fi;
        }
        sum_fi += fi;
    }
    let mut weights = vec![0.0; fit.dim];
    if sum_fi == 0.0 {
        for &c in selected_cols {
            if let Some(slot) = weights.get_mut(c) {
                *slot = 1.0;
            }
        }
        return Ok(weights);
    }
    for (p, &kept) in selected_pred.iter().enumerate() {
        if !kept {
            continue;
        }
        let w = fi_pred.get(p).copied().unwrap_or(0.0) / sum_fi;
        if let Some(&(start, len)) = fit.encoding.pred_cols.get(p) {
            let end = start.saturating_add(len);
            for c in start..end {
                if let Some(slot) = weights.get_mut(c) {
                    *slot = w;
                }
            }
        }
    }
    Ok(weights)
}

fn check_pair_limit(n: usize, dim: usize) -> Result<(), KnnError> {
    let work = (n as u128)
        .saturating_mul(n as u128)
        .saturating_mul(dim.max(1) as u128);
    if work > MAX_PAIR_WORK {
        return Err(data_err(format!(
            "KNN pairwise work {work} exceeds {MAX_PAIR_WORK}"
        )));
    }
    Ok(())
}

fn mean(ys: &[f64]) -> Result<f64, KnnError> {
    if ys.is_empty() {
        return Err(data_err("mean of no neighbour responses"));
    }
    let mut s = 0.0;
    for &y in ys {
        s += y;
    }
    let m = s / (ys.len() as f64);
    if !m.is_finite() {
        return Err(data_err("mean prediction is not finite"));
    }
    Ok(m)
}

fn median(ys: &[f64]) -> Result<f64, KnnError> {
    if ys.is_empty() {
        return Err(data_err("median of no neighbour responses"));
    }
    let mut v = ys.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let k = v.len();
    let value = if k % 2 == 1 {
        let mid = k / 2;
        v.get(mid)
            .copied()
            .ok_or_else(|| data_err("median index"))?
    } else {
        let right = k / 2;
        let left = right - 1;
        let a = v
            .get(left)
            .copied()
            .ok_or_else(|| data_err("median index"))?;
        let b = v
            .get(right)
            .copied()
            .ok_or_else(|| data_err("median index"))?;
        (a + b) / 2.0
    };
    if !value.is_finite() {
        return Err(data_err("median prediction is not finite"));
    }
    Ok(value)
}

fn majority_label(
    labels: &[String],
    votes: &[usize],
    class_counts: &[usize],
) -> Result<String, KnnError> {
    let mut best_idx: Option<usize> = None;
    let mut best_vote = 0usize;
    let mut best_n = 0usize;
    let mut best_label: &str = "";
    for (idx, label) in labels.iter().enumerate() {
        let vote = votes.get(idx).copied().unwrap_or(0);
        let n_j = class_counts.get(idx).copied().unwrap_or(0);
        let take = match best_idx {
            None => true,
            Some(_) => {
                vote > best_vote
                    || (vote == best_vote && n_j > best_n)
                    || (vote == best_vote && n_j == best_n && label.as_str() < best_label)
            }
        };
        if take {
            best_idx = Some(idx);
            best_vote = vote;
            best_n = n_j;
            best_label = label.as_str();
        }
    }
    let idx = best_idx.ok_or_else(|| data_err("majority vote over no categories"))?;
    labels
        .get(idx)
        .cloned()
        .ok_or_else(|| data_err("majority label missing"))
}

fn index_of_id(ids: &[i64], id: i64) -> Result<usize, KnnError> {
    ids.iter()
        .position(|x| *x == id)
        .ok_or_else(|| data_err(format!("neighbour id {id} is not a training case")))
}

fn sort_neighbors(found: &mut [Neighbor]) {
    found.sort_by(|a, b| match a.distance.total_cmp(&b.distance) {
        Ordering::Equal => a.case_id.cmp(&b.case_id),
        other => other,
    });
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

    fn data_xy(x: &[f64], y: &[f64]) -> KnnData {
        KnnData {
            numeric: vec![num_f("x", x), num_f("y", y)],
            nominal: Vec::new(),
            target: None,
            case_ids: None,
            folds: None,
        }
    }

    fn query_xy(x: f64, y: f64) -> Query {
        Query {
            numeric: vec![Some(x), Some(y)],
            nominal: Vec::new(),
        }
    }

    fn settings_k(k: usize) -> Settings {
        Settings {
            k,
            ..Settings::default()
        }
    }

    #[test]
    fn knn_known_answer_adjusted_euclidean() {
        // (0,0), (1,0), (0,1). min=0, max=1 → encoded (−1,−1), (1,−1), (−1,1).
        // d(1,2)=d(1,3)=2. Tie broken by smaller case id: 2 then 3.
        let model =
            train(&data_xy(&[0.0, 1.0, 0.0], &[0.0, 0.0, 1.0]), &settings_k(2)).expect("train");
        let nns = model.training_neighbors().expect("neighbours");
        let first = nns.first().expect("row 0");
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].case_id, 2);
        assert_eq!(first[1].case_id, 3);
        assert!((first[0].distance - 2.0).abs() < 1e-12);
        assert!((first[1].distance - 2.0).abs() < 1e-12);
        let q = model.neighbors(&query_xy(0.0, 0.0), Some(1)).expect("q");
        assert_eq!(q[0].case_id, 2);
        assert!((q[0].distance - 2.0).abs() < 1e-12);
    }

    #[test]
    fn knn_known_answer_city_block_no_rescale() {
        let settings = Settings {
            rescale: false,
            metric: DistanceMetric::CityBlock,
            k: 1,
            ..Settings::default()
        };
        let model = train(&data_xy(&[0.0, 3.0, 0.0], &[0.0, 4.0, 1.0]), &settings).expect("train");
        let nns = model.neighbors(&query_xy(0.0, 0.0), Some(1)).expect("q");
        assert_eq!(nns[0].case_id, 3);
        assert!((nns[0].distance - 1.0).abs() < 1e-12);
    }

    #[test]
    fn knn_known_answer_one_of_c() {
        let data = KnnData {
            numeric: Vec::new(),
            nominal: vec![NominalColumn {
                name: "c".into(),
                labels: vec!["A".into(), "B".into()],
                codes: vec![Some(0), Some(1)],
            }],
            target: None,
            case_ids: None,
            folds: None,
        };
        let settings = Settings {
            rescale: false,
            k: 1,
            ..Settings::default()
        };
        let model = train(&data, &settings).expect("train");
        let nns = model
            .neighbors(
                &Query {
                    numeric: Vec::new(),
                    nominal: vec![Some(0)],
                },
                Some(1),
            )
            .expect("q");
        assert_eq!(nns[0].case_id, 2);
        assert!((nns[0].distance - 2.0_f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn knn_known_answer_classification_vote_and_laplace() {
        // Three points, two class A (ids 1,2) and one class B (id 3).
        // Query at (1,1) with k=3: votes A=2, B=1 → A.
        // Laplace J=2, k=3: P(A)=(2+1)/(3+2)=0.6, P(B)=(1+1)/5=0.4.
        let data = KnnData {
            numeric: vec![num_f("x", &[0.0, 1.0, 0.0]), num_f("y", &[0.0, 0.0, 1.0])],
            nominal: Vec::new(),
            target: Some(Target::Categorical {
                name: "cls".into(),
                labels: vec!["A".into(), "B".into()],
                codes: vec![Some(0), Some(0), Some(1)],
            }),
            case_ids: None,
            folds: None,
        };
        let model = train(&data, &settings_k(3)).expect("train");
        let pred = model.predict(&query_xy(1.0, 1.0)).expect("predict");
        match pred {
            Prediction::Category {
                label,
                probabilities,
            } => {
                assert_eq!(label, "A");
                assert_eq!(probabilities.len(), 2);
                assert!((probabilities[0].1 - 0.6).abs() < 1e-12);
                assert!((probabilities[1].1 - 0.4).abs() < 1e-12);
            }
            other => panic!("expected category, got {other:?}"),
        }
    }

    #[test]
    fn knn_known_answer_vote_tie_uses_training_size_then_label() {
        // k=2, neighbours one A and one B. Training sizes A=2, B=1 → A.
        let data = KnnData {
            numeric: vec![num_f("x", &[0.0, 0.0, 10.0]), num_f("y", &[0.0, 1.0, 0.0])],
            nominal: Vec::new(),
            target: Some(Target::Categorical {
                name: "cls".into(),
                labels: vec!["A".into(), "B".into()],
                codes: vec![Some(0), Some(1), Some(0)],
            }),
            case_ids: None,
            folds: None,
        };
        let model = train(&data, &settings_k(2)).expect("train");
        let pred = model.predict(&query_xy(0.0, 0.5)).expect("predict");
        match pred {
            Prediction::Category { label, .. } => assert_eq!(label, "A"),
            other => panic!("expected category, got {other:?}"),
        }
    }

    #[test]
    fn knn_known_answer_regression_mean_and_median() {
        let data = KnnData {
            numeric: vec![num_f("x", &[0.0, 1.0, 2.0, 3.0])],
            nominal: Vec::new(),
            target: Some(Target::Continuous {
                name: "y".into(),
                values: vec![Some(1.0), Some(2.0), Some(10.0), Some(20.0)],
            }),
            case_ids: None,
            folds: None,
        };
        let mean_model = train(
            &data,
            &Settings {
                rescale: false,
                k: 2,
                range_prediction: RangePrediction::Mean,
                ..Settings::default()
            },
        )
        .expect("mean");
        let med_model = train(
            &data,
            &Settings {
                rescale: false,
                k: 3,
                range_prediction: RangePrediction::Median,
                ..Settings::default()
            },
        )
        .expect("median");
        let q = Query {
            numeric: vec![Some(0.0)],
            nominal: Vec::new(),
        };
        match mean_model.predict(&q).expect("p") {
            Prediction::Value(v) => {
                // neighbours of (0) among stored, includes self: ids 1 (d=0) and 2 (d=1) → mean (1+2)/2=1.5
                assert!((v - 1.5).abs() < 1e-12, "mean={v}");
            }
            other => panic!("{other:?}"),
        }
        match med_model.predict(&q).expect("p") {
            Prediction::Value(v) => {
                // three nearest: y=1,2,10 → median 2
                assert!((v - 2.0).abs() < 1e-12, "median={v}");
            }
            other => panic!("{other:?}"),
        }
        match med_model.predict_training_case(1).expect("loo") {
            Prediction::Value(v) => {
                // LOO k=3 but only 3 others: 2,10,20 even-k median (2+10)/2=6? k=3, 3 others, odd median 10.
                assert!((v - 10.0).abs() < 1e-12, "loo median={v}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn knn_known_answer_case_ids_skip_missing() {
        let data = KnnData {
            numeric: vec![num("x", &[Some(0.0), None, Some(1.0)])],
            nominal: Vec::new(),
            target: None,
            case_ids: None,
            folds: None,
        };
        let model = train(&data, &settings_k(1)).expect("train");
        assert_eq!(model.n_training(), 2);
        assert_eq!(model.case_ids(), &[1, 3]);
        let nns = model.training_neighbors().expect("n");
        assert_eq!(nns[0][0].case_id, 3);
    }

    #[test]
    fn knn_property_distance_symmetric_and_zero_to_self() {
        let model = train(
            &data_xy(&[0.0, 1.0, 2.0, 3.0], &[1.0, 0.0, 4.0, 5.0]),
            &settings_k(3),
        )
        .expect("train");
        let nns = model.training_neighbors().expect("n");
        for (i, row) in nns.iter().enumerate() {
            for n in row {
                assert!(n.distance >= 0.0 && n.distance.is_finite());
                assert_ne!(n.case_id, model.case_ids()[i]);
            }
        }
        let q = query_xy(0.0, 1.0);
        let with_self = model.neighbors(&q, None).expect("s");
        assert_eq!(with_self[0].case_id, 1);
        assert!(with_self[0].distance.abs() < 1e-12);
    }

    #[test]
    fn knn_gap_auto_k_without_folds() {
        let data = KnnData {
            numeric: vec![num_f("x", &[0.0, 1.0, 2.0, 3.0])],
            nominal: Vec::new(),
            target: Some(Target::Continuous {
                name: "y".into(),
                values: vec![Some(0.0), Some(1.0), Some(2.0), Some(3.0)],
            }),
            case_ids: None,
            folds: None,
        };
        let settings = Settings {
            automatic_k: true,
            min_k: 1,
            max_k: 2,
            ..Settings::default()
        };
        let err = train(&data, &settings).expect_err("gap");
        match err {
            AlgoError::Unsupported(msg) => assert!(msg.starts_with("GAP: KNN"), "{msg}"),
            other => panic!("expected GAP, got {other}"),
        }
    }

    #[test]
    fn knn_gap_feature_selection_without_target() {
        let settings = Settings {
            feature_selection: true,
            ..Settings::default()
        };
        let err = train(&data_xy(&[0.0, 1.0], &[0.0, 1.0]), &settings).expect_err("gap");
        match err {
            AlgoError::Unsupported(msg) => assert!(msg.starts_with("GAP: KNN"), "{msg}"),
            other => panic!("expected GAP, got {other}"),
        }
    }

    #[test]
    fn knn_automatic_k_with_folds_picks_smallest_tied() {
        // y = x, four points, two folds. k=1 is a perfect local predictor.
        let data = KnnData {
            numeric: vec![num_f("x", &[0.0, 1.0, 10.0, 11.0])],
            nominal: Vec::new(),
            target: Some(Target::Continuous {
                name: "y".into(),
                values: vec![Some(0.0), Some(1.0), Some(10.0), Some(11.0)],
            }),
            case_ids: None,
            folds: Some(vec![Some(1), Some(2), Some(1), Some(2)]),
        };
        let settings = Settings {
            rescale: false,
            automatic_k: true,
            min_k: 1,
            max_k: 3,
            k: 3,
            ..Settings::default()
        };
        let model = train(&data, &settings).expect("train");
        assert_eq!(model.k(), 1);
        let text = model.to_text();
        assert!(text.contains("KNN(k=1"), "{text}");
    }

    #[test]
    fn knn_never_crash() {
        let empty = KnnData {
            numeric: vec![num("x", &[])],
            nominal: Vec::new(),
            target: None,
            case_ids: None,
            folds: None,
        };
        assert!(matches!(
            train(&empty, &Settings::default()),
            Err(AlgoError::Data(_))
        ));

        let one = KnnData {
            numeric: vec![num_f("x", &[1.0])],
            nominal: Vec::new(),
            target: None,
            case_ids: None,
            folds: None,
        };
        let m = train(&one, &settings_k(3)).expect("one");
        let nns = m.training_neighbors().expect("n");
        assert_eq!(nns.len(), 1);
        assert!(nns[0].is_empty());

        let small = data_xy(&[0.0, 1.0], &[0.0, 1.0]);
        let m = train(&small, &settings_k(100)).expect("k>n");
        let nns = m.training_neighbors().expect("n");
        assert_eq!(nns[0].len(), 1);

        let constant = KnnData {
            numeric: vec![num_f("c", &[2.0, 2.0, 2.0]), num_f("x", &[0.0, 1.0, 2.0])],
            nominal: Vec::new(),
            target: None,
            case_ids: None,
            folds: None,
        };
        let m = train(&constant, &Settings::default()).expect("constant");
        assert_eq!(m.n_training(), 3);
        let _ = m.training_neighbors().expect("n");

        let all_missing = KnnData {
            numeric: vec![num("m", &[None, None]), num_f("x", &[1.0, 2.0])],
            nominal: Vec::new(),
            target: None,
            case_ids: None,
            folds: None,
        };
        assert!(matches!(
            train(&all_missing, &Settings::default()),
            Err(AlgoError::Data(_))
        ));

        let dups = data_xy(&[1.0, 1.0, 1.0], &[1.0, 1.0, 1.0]);
        let m = train(&dups, &settings_k(2)).expect("dups");
        let nns = m.training_neighbors().expect("n");
        assert_eq!(nns[0].len(), 2);
        assert!(nns[0][0].distance.abs() < 1e-12);

        let huge = Settings {
            k: MAX_K + 1,
            ..Settings::default()
        };
        assert!(matches!(train(&dups, &huge), Err(AlgoError::Settings(_))));

        let nan = KnnData {
            numeric: vec![num("x", &[Some(f64::NAN), Some(1.0)])],
            nominal: Vec::new(),
            target: None,
            case_ids: None,
            folds: None,
        };
        assert!(matches!(
            train(&nan, &Settings::default()),
            Err(AlgoError::Data(_))
        ));

        let incomplete_query =
            train(&data_xy(&[0.0, 1.0], &[0.0, 1.0]), &Settings::default()).expect("m");
        let miss = Query {
            numeric: vec![None, Some(0.0)],
            nominal: Vec::new(),
        };
        assert!(
            incomplete_query
                .neighbors(&miss, None)
                .expect("q")
                .is_empty()
        );
    }
}
