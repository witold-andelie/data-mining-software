//! Support Vector Machine (`C-SVC`, binary classification), following the IBM SPSS Modeler 19
//! Algorithms Guide chapter "Support Vector Machine (SVM) Algorithms" (pp. 377-387).
//!
//! This module is self-contained pure algorithm code: callers supply plain column vectors in
//! [`SvmData`] and receive a trained [`SvmModel`] capable of computing decision values, Platt
//! posterior probabilities, class predictions, and a text dump via [`SvmModel::to_text`].
//!
//! # Decisions the Guide does not settle
//!
//! Every choice below where the Guide is ambiguous, incomplete, or disagrees with IBM SPSS Modeler
//! is settled against the two embedded Modeler models from the `svm_cancer` demo stream trained on
//! `cancer_type_output.csv` (699 records, `Class` in `{2, 4}`, `BareNuc` nominal with 11 values
//! including `"?"`, eight numeric inputs `1..=10`):
//! - **Model 1**: `saved_svm_model_1.pmml.txt` — RBF kernel (`gamma = 0.1`, `C = 10`,
//!   `stopping_criteria = 1e-3`, 66 support vectors, `absoluteValue = 0.698260102490274`,
//!   `paramA = -2.60504911656108`, `paramB = 0.354875907331652`).
//! - **Model 2**: `saved_svm_model_2.pmml.txt` — Polynomial kernel (`degree = 3`, `gamma = 1`,
//!   `coef0 = 0`, `C = 10`, `stopping_criteria = 1e-3`, 67 support vectors,
//!   `absoluteValue = -0.957022480791213`, `paramA = -2.89608412626823`,
//!   `paramB = 0.318483410603229`).
//!
//! 1. **Continuous input scaling (`[0, 1]` vs `[-1, 1]`)**:
//!    The Guide (p. 380) states "linearly scale each attribute to `[-1, 1]` or `[0, 1]`".
//!    **Decision**: Linearly scale to `[0, 1]`: `(x - min) / (max - min)` computed on the complete
//!    training records (mapping constant columns with `max == min` to `0.0`).
//!    **Evidence**: Both `saved_svm_model_1.pmml.txt` and `saved_svm_model_2.pmml.txt` emit
//!    `<NormContinuous><LinearNorm orig="1" norm="0"/><LinearNorm orig="10" norm="1"/></NormContinuous>`
//!    for all 8 numeric inputs, and `vec1` (record 1: `Clump=5, UnifSize=4, ..., Mit=1`) has entries
//!    `0.444444, 0.333333, ..., 0` (omitted from sparse array).
//!
//! 2. **Nominal input coding (one-hot vs scaled integer)**:
//!    The Guide (p. 380) states "use `(0, 1, 2, ..., m)` to represent the categories and scale the
//!    values as for continuous input variables".
//!    **Decision**: One-hot indicator columns (`1.0` for the record's category, `0.0` otherwise),
//!    one column per declared category in category order, preserving input column order. Unseen
//!    categories at scoring time produce all `0.0` indicators.
//!    **Evidence**: Both saved models expand `BareNuc` into 11 binary `0/1` `DerivedField`s
//!    (`BareNuc=1`, `BareNuc=10`, `BareNuc=2`, ..., `BareNuc=?`) inside `VectorFields` (`19` total
//!    attributes = `8` scaled numeric + `11` one-hot nominal), and every `VectorInstance` has
//!    exactly one `1` entry in indices `6..=16` (including `BareNuc=?` at index `16` in `vec6`
//!    and `vec10`). Using this coding reproduces 66/66 support vectors of Model 1 and 67/67
//!    support vectors of Model 2.
//!
//! 3. **Class sign convention and saved coefficient sign**:
//!    **Decision**: Positive ("Response") class has `y_i = +1`, negative ("NonResponse") class has
//!    `y_i = -1`, and stored support vector coefficients are signed `y_i * alpha_i`.
//!    **Evidence**: Both saved models declare `<ResponseCategory Response="4" NonResponse="2"/>`.
//!    `vec1` (row 1, `Class = 2`) has `<Coefficient value="-10"/>` and `vec4` (row 12, `Class = 4`)
//!    has `<Coefficient value="10"/>`.
//!
//! 4. **Decision function sign and constant `b` (`absoluteValue`)**:
//!    The Guide (p. 380) defines `r1` and `r2` as class means of `grad f(alpha_i)` and writes
//!    `b = (r1 + r2) / 2`, which has a sign typo (`grad_i = -y_i * b` on free SVs so `r1 ≈ -b`
//!    and `r2 ≈ +b`, making `(r1 + r2) / 2 ≈ 0`).
//!    **Decision**: The decision function is `f(x) = sum_i (y_i * alpha_i) K(x_i, x) + b`, where
//!    `b = -(sum_{0 < alpha_i < C} y_i * grad_i) / n_free` when free support vectors exist, and
//!    `b = 0.5 * (m(alpha) + M(alpha))` (`= -0.5 * (ub + lb)`) when no free support vectors exist.
//!    **Evidence**: Evaluated directly on the saved models' own support vectors and coefficients
//!    against `cancer_type_output.csv`, `-avg(y_i * grad_i)` over all free support vectors equals
//!    `0.698260102490` vs `absoluteValue = 0.698260102490274` in Model 1 (all 12 printed digits
//!    match; `(r2 - r1) / 2 = 0.698268807769` differs by `8.7e-6`), and `-0.957022480791` vs
//!    `absoluteValue = -0.957022480791213` in Model 2 (all 12 digits match; `(r2 - r1) / 2 =
//!    -0.957035792787` differs by `1.3e-5`). On free SVs, `y_i * (sum c_j K(x_j, x_i) + abs_val)`
//!    is in `[0.9995, 1.0004]`, confirming `+ b` rather than `- b`.
//!
//! 5. **SMO regularization floor `tau`, initial point, tie-breaking, and shrinking default**:
//!    The Guide (pp. 381-385) leaves the small positive constant `tau`, argmax/argmin
//!    tie-breaking, and whether shrinking is active on small datasets unspecified.
//!    **Decision**: Initialize `alpha = 0`, `grad = -1`; set `tau = 1e-12`; break ties in `i`
//!    (`>=`) and `j` (`<=`) toward the later index; stop when `m(alpha) - M(alpha) <= eps`;
//!    default `shrinking = false` (with optional `shrinking = true` implementing Guide pp. 382-383
//!    every `min(l, 1000)` iterations with `G_bar` gradient reconstruction).
//!    **Evidence**: Both saved models satisfy `m(alpha) - M(alpha) <= 1e-3` (`8.61e-4` for Model 1,
//!    `9.98e-4` for Model 2). With `shrinking = false` and these tie-break rules, Model 2
//!    (Polynomial, 1069 iterations > `min(699, 1000)`) reproduces the saved Modeler model to
//!    floating-point precision: 67/67 exact support vectors, max coefficient difference `5.33e-15`,
//!    constant difference `2.22e-16` (1 ULP), max decision difference `2.79e-13`, and Platt
//!    parameter differences `1.98e-8` (`paramA`) and `9.91e-11` (`paramB`). Enabling shrinking at
//!    iteration 699 shifts Model 2's coefficients by `4.28e-4`, proving Modeler did not shrink on
//!    Model 2. Model 1 (RBF, 168 iterations) reproduces 66/66 exact support vectors with
//!    tolerance-limited differences (`max_coeff_err = 7.47e-3`, `const_err = 2.24e-4`,
//!    `max_dec_err = 8.05e-4`, `0/699` prediction mismatches) because discrete grid distances have
//!    exact ties sensitive to 1-ULP `exp()` rounding.
//!
//! 6. **Platt probability calibration input set and Newton solver**:
//!    The Guide (p. 387) gives the regularized maximum-likelihood objective, gradient, and Hessian
//!    for `(A, B)` with `P(y = 1 | x) = 1 / (1 + exp(A * f(x) + B))`, without stating whether
//!    `f(x_i)` are out-of-fold or in-sample training decision values or specifying the optimizer
//!    stopping tolerance.
//!    **Decision**: Fit `(A, B)` directly on the training records' own decision values
//!    `f(x_i) = sum_j c_j K(x_j, x_i) + b` using damped Newton's method with Armijo backtracking
//!    line search starting from `A = 0`, `B = ln((N_- + 1) / (N_+ + 1))`, Hessian damping
//!    `sigma = 1e-12`, stopping when `|gA| < 1e-10` and `|gB| < 1e-10`. Predict positive when
//!    `P(y = 1 | x) >= 0.5` (highest predicted probability, Guide p. 387).
//!    **Evidence**: Evaluating the Guide's gradient on the in-sample training decision values at
//!    Modeler's saved `(paramA, paramB)` gives `|gA| = 2.99e-9, |gB| = 1.86e-9` for Model 1 and
//!    `|gA| = 4.78e-7, |gB| = 1.10e-7` for Model 2, confirming in-sample calibration. Our trained
//!    models reproduce `paramA`/`paramB` within `2.49e-4`/`1.60e-4` (Model 1) and
//!    `1.98e-8`/`9.91e-11` (Model 2, `shrinking = false`), with `0/699` prediction mismatches on
//!    both models.
//!
//! 7. **Out-of-scope / unspecified features (explicit `GAP:` errors)**:
//!    - `epsilon-SVR` regression (`Target::Continuous`) and one-class SVM (`Target::OneClass`).
//!    - Multiclass classification (`Target::Multiclass`): Guide does not specify multiclass coding.
//!    - Divide-and-conquer Fast SVM Training (Guide pp. 385-386): subset size `d`, block count
//!      `k`, and loop limit `T` are unspecified; datasets exceeding [`MAX_RECORDS`] return `GAP:`.
//!    - Unbalanced per-record `C_i != C_j` (Guide p. 383): no `svmnode` property or formula sets
//!      class-specific costs.
//!    - Variable importance: present in Model 1's PMML, not documented in the Guide chapter.

use crate::AlgoError;
use std::collections::HashSet;
use std::fmt::Write as _;

/// Hard upper bound on complete training records accepted by the dense-kernel SMO solver.
/// Datasets beyond this limit would require divide-and-conquer Fast SVM Training (`GAP:`).
pub const MAX_RECORDS: usize = 5_000;

/// Hard upper bound on input columns in [`SvmData`].
pub const MAX_INPUT_COLUMNS: usize = 2_000;

/// Hard upper bound on total encoded features (scaled numeric + one-hot nominal indicators).
pub const MAX_FEATURES: usize = 2_000;

/// Hard upper bound on encoded design-matrix cells (`records * features`).
pub const MAX_DESIGN_CELLS: usize = 10_000_000;

/// Hard upper bound on support vectors stored in a trained [`SvmModel`].
pub const MAX_SUPPORT_VECTORS: usize = 5_000;

/// Hard upper bound on SMO iterations allowed per training run.
pub const MAX_ITERATIONS: usize = 1_000_000;

/// Hard upper bound on polynomial kernel degree.
pub const MAX_POLY_DEGREE: u32 = 20;

/// Hard upper bound on regularization parameter `C` to prevent floating-point gradient loss.
pub const MAX_C: f64 = 1e12;

/// Small positive curvature floor `tau` used when `K_ii + K_jj - 2 K_ij <= 0` (Guide pp. 381, 385).
const TAU: f64 = 1e-12;

/// Shrink evaluation cadence cap (`min(l, 1000)`, Guide p. 382).
const SHRINK_PERIOD_CAP: usize = 1_000;

/// Maximum Newton iterations for Platt probability calibration.
const MAX_PLATT_ITERATIONS: usize = 200;

/// Hessian ridge damping for Platt Newton steps.
const PLATT_SIGMA: f64 = 1e-12;

/// Gradient stopping tolerance for Platt Newton steps.
const PLATT_GRAD_TOL: f64 = 1e-10;

/// Minimum backtracking line-search step size for Platt Newton steps.
const PLATT_MIN_STEP: f64 = 1e-12;

/// Armijo sufficient-decrease constant for Platt line search.
const PLATT_ARMIJO: f64 = 1e-4;

/// Kernel function `K(x_i, x_j)` supported by C-SVC (Guide p. 381).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kernel {
    /// Linear kernel: `K(u, v) = u^T v`.
    Linear,
    /// Polynomial kernel: `K(u, v) = (gamma * u^T v + coef0)^degree`.
    Polynomial {
        /// Multiplier `gamma` (`_gamma`, default `1.0`).
        gamma: f64,
        /// Constant bias `r` / `coef0` (`_bias`, default `0.0`).
        coef0: f64,
        /// Positive integer exponent `d` (`_degree`, default `3`).
        degree: u32,
    },
    /// Radial Basis Function kernel: `K(u, v) = exp(-gamma * ||u - v||^2)`.
    Rbf {
        /// Bandwidth parameter `gamma` (`_rbf_gamma`, default `0.1`); must be positive.
        gamma: f64,
    },
    /// Sigmoid (hyperbolic tangent) kernel: `K(u, v) = tanh(gamma * u^T v + coef0)`.
    Sigmoid {
        /// Multiplier `gamma` (`_gamma`, default `1.0`).
        gamma: f64,
        /// Constant bias `r` / `coef0` (`_bias`, default `0.0`).
        coef0: f64,
    },
}

/// Settings controlling C-SVC training (`svmnode` properties).
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Regularization parameter `C` (`_regularization`, default `10.0`); must be in `(0, MAX_C]`.
    pub c: f64,
    /// KKT stopping tolerance `epsilon` (`_stopping_criteria`, default `1e-3`); must be finite and `> 0`.
    pub stopping_criteria: f64,
    /// Kernel function and parameters (default `Kernel::Rbf { gamma: 0.1 }`).
    pub kernel: Kernel,
    /// Regression epsilon (`_precision`, default `0.1`); validated finite `>= 0` (unused for C-SVC).
    pub precision: f64,
    /// Whether to apply the Guide's shrinking heuristic (pp. 382-383) during SMO (default `false`).
    pub shrinking: bool,
    /// Maximum SMO iterations allowed (`1..=MAX_ITERATIONS`, default `100_000`).
    pub max_iterations: usize,
    /// Maximum support vectors allowed in the trained model (`1..=MAX_SUPPORT_VECTORS`, default `MAX_SUPPORT_VECTORS`).
    pub max_support_vectors: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            c: 10.0,
            stopping_criteria: 1e-3,
            kernel: Kernel::Rbf { gamma: 0.1 },
            precision: 0.1,
            shrinking: false,
            max_iterations: 100_000,
            max_support_vectors: MAX_SUPPORT_VECTORS,
        }
    }
}

/// One input column of training data.
#[derive(Debug, Clone, PartialEq)]
pub enum InputColumn {
    /// Continuous numeric input (`None` = missing).
    Numeric {
        /// Field name.
        name: String,
        /// Per-record values.
        values: Vec<Option<f64>>,
    },
    /// Nominal (categorical) input represented by category indices into `labels`.
    Nominal {
        /// Field name.
        name: String,
        /// Distinct valid category labels, in the order their one-hot indicator columns appear.
        labels: Vec<String>,
        /// Per-record category code (`None` = missing).
        codes: Vec<Option<usize>>,
    },
}

/// Target field for [`train`].
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// Binary classification target (`C-SVC`).
    Binary {
        /// Target field name.
        name: String,
        /// Positive ("Response", `y = +1`) category label.
        positive_label: String,
        /// Negative ("NonResponse", `y = -1`) category label.
        negative_label: String,
        /// Per-record class (`Some(true)` = positive, `Some(false)` = negative, `None` = missing).
        values: Vec<Option<bool>>,
    },
    /// Multiclass classification target (`> 2` classes): unsupported (`GAP:`).
    Multiclass {
        /// Target field name.
        name: String,
        /// Category labels.
        labels: Vec<String>,
        /// Per-record category code.
        codes: Vec<Option<usize>>,
    },
    /// Continuous regression target (`epsilon-SVR`): unsupported (`GAP:`).
    Continuous {
        /// Target field name.
        name: String,
        /// Per-record target values.
        values: Vec<Option<f64>>,
    },
    /// One-class SVM target: unsupported (`GAP:`).
    OneClass,
}

/// Self-contained training dataset for [`train`].
#[derive(Debug, Clone, PartialEq)]
pub struct SvmData {
    /// Total number of records across every column and target vector.
    pub num_records: usize,
    /// Input feature columns in model order.
    pub inputs: Vec<InputColumn>,
    /// Target specification.
    pub target: Target,
}

/// One input value of a record passed to [`SvmModel::decision`], [`SvmModel::probability`],
/// or [`SvmModel::predict`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputValue {
    /// Continuous numeric value (`None` = missing).
    Numeric(Option<f64>),
    /// Nominal category index into the corresponding input's `labels` (`None` = missing).
    Nominal(Option<usize>),
}

/// Learned preprocessing transformation for one input column.
#[derive(Debug, Clone, PartialEq)]
pub enum InputTransform {
    /// Linear scaling of a continuous input to `[0, 1]` via `(x - min) / (max - min)`
    /// (or constant `0.0` when `min == max`).
    Numeric {
        /// Original field name.
        name: String,
        /// Training minimum on complete records.
        min: f64,
        /// Training maximum on complete records.
        max: f64,
        /// Encoded feature index in the transformed vector space.
        feature_index: usize,
    },
    /// One-hot indicator expansion of a nominal input into `labels.len()` binary `0/1` features.
    Nominal {
        /// Original field name.
        name: String,
        /// Category labels corresponding to features `start_index..start_index + labels.len()`.
        labels: Vec<String>,
        /// Starting encoded feature index in the transformed vector space.
        start_index: usize,
    },
}

/// One support vector retained by a trained [`SvmModel`].
#[derive(Debug, Clone, PartialEq)]
pub struct SupportVector {
    /// Zero-based index of this record in the input [`SvmData`].
    pub record_index: usize,
    /// Signed Lagrange multiplier `y_i * alpha_i` (`+alpha_i` for positive, `-alpha_i` for negative).
    pub coefficient: f64,
    /// Scaled/one-hot encoded feature vector (length `num_features`).
    pub vector: Vec<f64>,
}

/// Platt sigmoid calibration parameters for `P(y = +1 | x) = 1 / (1 + exp(param_a * f(x) + param_b))`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlattParameters {
    /// Slope parameter `A` (`paramA`).
    pub param_a: f64,
    /// Intercept parameter `B` (`paramB`).
    pub param_b: f64,
}

/// Summary diagnostics from the SMO solver.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrainingSummary {
    /// Complete (non-missing) records used for training.
    pub complete_records: usize,
    /// Records excluded due to missing input or target values.
    pub excluded_records: usize,
    /// Number of positive (`y = +1`) complete training records.
    pub positive_records: usize,
    /// Number of negative (`y = -1`) complete training records.
    pub negative_records: usize,
    /// Total encoded feature dimension.
    pub num_features: usize,
    /// SMO iterations executed.
    pub iterations: usize,
    /// Final full-problem KKT gap `m(alpha) - M(alpha)`.
    pub kkt_gap: f64,
    /// Dual objective value `0.5 * alpha^T Q alpha - e^T alpha` at termination.
    pub dual_objective: f64,
}

/// A trained binary C-SVC model.
#[derive(Debug, Clone, PartialEq)]
pub struct SvmModel {
    /// Target field name.
    pub target_name: String,
    /// Positive ("Response", `y = +1`) category label.
    pub positive_label: String,
    /// Negative ("NonResponse", `y = -1`) category label.
    pub negative_label: String,
    /// Kernel function and parameters.
    pub kernel: Kernel,
    /// Regularization parameter `C` used during training.
    pub c: f64,
    /// Stopping tolerance used during training.
    pub stopping_criteria: f64,
    /// Learned input transformations in input column order.
    pub transforms: Vec<InputTransform>,
    /// Encoded feature dimension.
    pub num_features: usize,
    /// Support vectors in ascending original record order.
    pub support_vectors: Vec<SupportVector>,
    /// Decision function constant `b` (`absoluteValue` in PMML).
    pub constant: f64,
    /// Platt posterior probability parameters `(A, B)`.
    pub platt: PlattParameters,
    /// Training diagnostics.
    pub summary: TrainingSummary,
}

impl SvmModel {
    /// Evaluates the raw decision function `f(x) = sum_i (y_i * alpha_i) K(x_i, x) + b`
    /// on an already-encoded feature vector `encoded` (length `self.num_features`).
    pub fn decision_encoded(&self, encoded: &[f64]) -> Result<f64, AlgoError> {
        if encoded.len() != self.num_features {
            return Err(AlgoError::Data(format!(
                "encoded vector length {} does not match model feature count {}",
                encoded.len(),
                self.num_features
            )));
        }
        for &v in encoded {
            if !v.is_finite() {
                return Err(AlgoError::Data(
                    "encoded feature vector contains a non-finite value".into(),
                ));
            }
        }
        let mut sum = self.constant;
        for sv in &self.support_vectors {
            let k = eval_kernel(self.kernel, &sv.vector, encoded)?;
            sum += sv.coefficient * k;
        }
        if !sum.is_finite() {
            return Err(AlgoError::Data("SVM decision value is not finite".into()));
        }
        Ok(sum)
    }

    /// Transforms an input record into the model's encoded space.
    /// Returns `Ok(None)` if any input value is missing (`None`, Guide p. 387).
    pub fn encode_record(&self, record: &[InputValue]) -> Result<Option<Vec<f64>>, AlgoError> {
        if record.len() != self.transforms.len() {
            return Err(AlgoError::Data(format!(
                "record length {} does not match model input count {}",
                record.len(),
                self.transforms.len()
            )));
        }
        let mut encoded = vec![0.0; self.num_features];
        for (val, transform) in record.iter().zip(&self.transforms) {
            match (val, transform) {
                (
                    InputValue::Numeric(opt),
                    InputTransform::Numeric {
                        name,
                        min,
                        max,
                        feature_index,
                    },
                ) => {
                    let Some(raw) = *opt else {
                        return Ok(None);
                    };
                    if !raw.is_finite() {
                        return Err(AlgoError::Data(format!(
                            "non-finite numeric value in input '{name}'"
                        )));
                    }
                    let scaled = scale_numeric(raw, *min, *max);
                    let slot = encoded.get_mut(*feature_index).ok_or_else(|| {
                        AlgoError::Data("internal feature index out of bounds".into())
                    })?;
                    *slot = scaled;
                }
                (
                    InputValue::Nominal(opt),
                    InputTransform::Nominal {
                        labels,
                        start_index,
                        ..
                    },
                ) => {
                    let Some(code) = *opt else {
                        return Ok(None);
                    };
                    // Unknown categories outside training labels map to all-zero indicators
                    // (matching PMML MapValues defaultValue="0").
                    if code < labels.len()
                        && let Some(idx) = start_index.checked_add(code)
                        && let Some(slot) = encoded.get_mut(idx)
                    {
                        *slot = 1.0;
                    }
                }
                (_, InputTransform::Numeric { name, .. }) => {
                    return Err(AlgoError::Data(format!(
                        "expected numeric value for input '{name}'"
                    )));
                }
                (_, InputTransform::Nominal { name, .. }) => {
                    return Err(AlgoError::Data(format!(
                        "expected nominal value for input '{name}'"
                    )));
                }
            }
        }
        Ok(Some(encoded))
    }

    /// Evaluates the decision function `f(x)` on a raw input record.
    /// Returns `Ok(None)` if any input value is missing (`None`).
    pub fn decision(&self, record: &[InputValue]) -> Result<Option<f64>, AlgoError> {
        let Some(encoded) = self.encode_record(record)? else {
            return Ok(None);
        };
        self.decision_encoded(&encoded).map(Some)
    }

    /// Evaluates the Platt posterior probability `P(y = +1 | x)` on a raw input record.
    /// Returns `Ok(None)` if any input value is missing (`None`).
    pub fn probability(&self, record: &[InputValue]) -> Result<Option<f64>, AlgoError> {
        let Some(f) = self.decision(record)? else {
            return Ok(None);
        };
        Ok(Some(platt_probability(self.platt, f)))
    }

    /// Predicts the class label (`positive_label` or `negative_label`) with highest Platt
    /// posterior probability (`P(y = +1 | x) >= 0.5`, Guide p. 387).
    /// Returns `Ok(None)` if any input value is missing (`None`).
    pub fn predict(&self, record: &[InputValue]) -> Result<Option<&str>, AlgoError> {
        let Some(p) = self.probability(record)? else {
            return Ok(None);
        };
        if p >= 0.5 {
            Ok(Some(self.positive_label.as_str()))
        } else {
            Ok(Some(self.negative_label.as_str()))
        }
    }

    /// Formats a human-readable text dump of the model for inspection and tests.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "SVM Model (C-SVC)");
        let _ = writeln!(
            out,
            "  target: {} (positive={}, negative={})",
            self.target_name, self.positive_label, self.negative_label
        );
        let _ = writeln!(
            out,
            "  kernel: {:?}, C={}, stopping_criteria={}",
            self.kernel, self.c, self.stopping_criteria
        );
        let _ = writeln!(
            out,
            "  records: complete={} (pos={}, neg={}), excluded={}",
            self.summary.complete_records,
            self.summary.positive_records,
            self.summary.negative_records,
            self.summary.excluded_records
        );
        let _ = writeln!(
            out,
            "  solver: iterations={}, kkt_gap={:.6e}, dual_objective={:.12}",
            self.summary.iterations, self.summary.kkt_gap, self.summary.dual_objective
        );
        let _ = writeln!(out, "  constant (b): {:.15}", self.constant);
        let _ = writeln!(
            out,
            "  platt: paramA={:.15}, paramB={:.15}",
            self.platt.param_a, self.platt.param_b
        );
        let _ = writeln!(out, "  features ({}):", self.num_features);
        for tr in &self.transforms {
            match tr {
                InputTransform::Numeric {
                    name,
                    min,
                    max,
                    feature_index,
                } => {
                    let _ = writeln!(
                        out,
                        "    [{feature_index}] {name}_scaled: [{min}, {max}] -> [0, 1]"
                    );
                }
                InputTransform::Nominal {
                    name,
                    labels,
                    start_index,
                } => {
                    for (offset, label) in labels.iter().enumerate() {
                        let idx = start_index.saturating_add(offset);
                        let _ = writeln!(out, "    [{idx}] {name}={label}");
                    }
                }
            }
        }
        let _ = writeln!(out, "  support_vectors ({}):", self.support_vectors.len());
        for (i, sv) in self.support_vectors.iter().enumerate() {
            let _ = write!(
                out,
                "    sv[{i}] rec={} coeff={:.15} nz=[",
                sv.record_index, sv.coefficient
            );
            let mut first = true;
            for (col, &val) in sv.vector.iter().enumerate() {
                if val != 0.0 {
                    if !first {
                        let _ = write!(out, ", ");
                    }
                    first = false;
                    let _ = write!(out, "{col}:{val:.6}");
                }
            }
            let _ = writeln!(out, "]");
        }
        out
    }
}

/// Trains a binary C-SVC model on `data` using `settings`.
pub fn train(data: &SvmData, settings: &Settings) -> Result<SvmModel, AlgoError> {
    crate::check_cancelled()?;
    validate_settings(settings)?;

    let (target_name, positive_label, negative_label, target_values) = match &data.target {
        Target::Binary {
            name,
            positive_label,
            negative_label,
            values,
        } => (name, positive_label, negative_label, values),
        Target::Continuous { .. } => {
            return Err(gap("regression (epsilon-SVR) is out of scope"));
        }
        Target::OneClass => {
            return Err(gap("one-class SVM is out of scope"));
        }
        Target::Multiclass { .. } => {
            return Err(gap(
                "multiclass SVM (> 2 target categories) is out of scope (unspecified in Guide)",
            ));
        }
    };

    if positive_label == negative_label {
        return Err(AlgoError::Data(
            "binary SVM positive and negative target labels must be distinct".into(),
        ));
    }

    let prepared = prepare_data(data, target_values)?;
    let n = prepared.labels.len();
    let kernel_matrix = build_kernel_matrix(settings.kernel, &prepared.features)?;
    let smo = solve_smo(
        &kernel_matrix,
        &prepared.labels,
        settings.c,
        settings.stopping_criteria,
        settings.shrinking,
        settings.max_iterations,
    )?;

    let mut support_vectors = Vec::new();
    for (idx, (&alpha, (&y, (&rec_idx, vec)))) in smo
        .alpha
        .iter()
        .zip(
            prepared
                .labels
                .iter()
                .zip(prepared.record_indices.iter().zip(&prepared.features)),
        )
        .enumerate()
    {
        if alpha > 0.0 {
            if support_vectors.len() >= settings.max_support_vectors {
                return Err(AlgoError::Data(format!(
                    "support vector count exceeds limit {}",
                    settings.max_support_vectors
                )));
            }
            let coeff = y * alpha;
            if !coeff.is_finite() {
                return Err(AlgoError::Data(format!(
                    "non-finite support vector coefficient at index {idx}"
                )));
            }
            support_vectors.push(SupportVector {
                record_index: rec_idx,
                coefficient: coeff,
                vector: vec.clone(),
            });
        }
    }

    // In-sample decision values f(x_i) = sum_j c_j K(x_j, x_i) + b = y_i * (grad_i + 1) + b.
    let mut decision_values = Vec::with_capacity(n);
    for (&y, &g) in prepared.labels.iter().zip(&smo.grad) {
        let f = y * (g + 1.0) + smo.constant;
        if !f.is_finite() {
            return Err(AlgoError::Data(
                "non-finite training decision value encountered".into(),
            ));
        }
        decision_values.push(f);
    }

    let platt = fit_platt(
        &decision_values,
        &prepared.labels,
        prepared.positive_records,
        prepared.negative_records,
    )?;

    Ok(SvmModel {
        target_name: target_name.clone(),
        positive_label: positive_label.clone(),
        negative_label: negative_label.clone(),
        kernel: settings.kernel,
        c: settings.c,
        stopping_criteria: settings.stopping_criteria,
        transforms: prepared.transforms,
        num_features: prepared.num_features,
        support_vectors,
        constant: smo.constant,
        platt,
        summary: TrainingSummary {
            complete_records: n,
            excluded_records: prepared.excluded_records,
            positive_records: prepared.positive_records,
            negative_records: prepared.negative_records,
            num_features: prepared.num_features,
            iterations: smo.iterations,
            kkt_gap: smo.kkt_gap,
            dual_objective: smo.dual_objective,
        },
    })
}

fn gap(what: &str) -> AlgoError {
    AlgoError::Unsupported(format!("GAP: SVM {what}"))
}

fn validate_settings(settings: &Settings) -> Result<(), AlgoError> {
    if !settings.c.is_finite() || settings.c <= 0.0 || settings.c > MAX_C {
        return Err(AlgoError::Settings(format!(
            "SVM C must be finite in (0, {MAX_C:e}]"
        )));
    }
    if !settings.stopping_criteria.is_finite() || settings.stopping_criteria <= 0.0 {
        return Err(AlgoError::Settings(
            "SVM stopping_criteria must be a positive finite number".into(),
        ));
    }
    if !settings.precision.is_finite() || settings.precision < 0.0 {
        return Err(AlgoError::Settings(
            "SVM precision must be a non-negative finite number".into(),
        ));
    }
    if settings.max_iterations == 0 || settings.max_iterations > MAX_ITERATIONS {
        return Err(AlgoError::Settings(format!(
            "SVM max_iterations must be in 1..={MAX_ITERATIONS}"
        )));
    }
    if settings.max_support_vectors == 0 || settings.max_support_vectors > MAX_SUPPORT_VECTORS {
        return Err(AlgoError::Settings(format!(
            "SVM max_support_vectors must be in 1..={MAX_SUPPORT_VECTORS}"
        )));
    }
    match settings.kernel {
        Kernel::Linear => {}
        Kernel::Rbf { gamma } => {
            if !gamma.is_finite() || gamma <= 0.0 {
                return Err(AlgoError::Settings(
                    "SVM RBF gamma must be a positive finite number".into(),
                ));
            }
        }
        Kernel::Polynomial {
            gamma,
            coef0,
            degree,
        } => {
            if !gamma.is_finite() {
                return Err(AlgoError::Settings(
                    "SVM polynomial gamma must be finite".into(),
                ));
            }
            if !coef0.is_finite() {
                return Err(AlgoError::Settings(
                    "SVM polynomial coef0 must be finite".into(),
                ));
            }
            if degree == 0 || degree > MAX_POLY_DEGREE {
                return Err(AlgoError::Settings(format!(
                    "SVM polynomial degree must be in 1..={MAX_POLY_DEGREE}"
                )));
            }
        }
        Kernel::Sigmoid { gamma, coef0 } => {
            if !gamma.is_finite() || !coef0.is_finite() {
                return Err(AlgoError::Settings(
                    "SVM sigmoid gamma and coef0 must be finite".into(),
                ));
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------------
// Data validation, blank handling, scaling and one-hot encoding

struct PreparedData {
    record_indices: Vec<usize>,
    features: Vec<Vec<f64>>,
    labels: Vec<f64>,
    transforms: Vec<InputTransform>,
    num_features: usize,
    positive_records: usize,
    negative_records: usize,
    excluded_records: usize,
}

fn prepare_data(data: &SvmData, target_values: &[Option<bool>]) -> Result<PreparedData, AlgoError> {
    if data.num_records == 0 {
        return Err(AlgoError::Data("SVM training data has no records".into()));
    }
    if target_values.len() != data.num_records {
        return Err(AlgoError::Data(format!(
            "target length {} does not match num_records {}",
            target_values.len(),
            data.num_records
        )));
    }
    if data.inputs.is_empty() {
        return Err(AlgoError::Data(
            "SVM training data must contain at least one input column".into(),
        ));
    }
    if data.inputs.len() > MAX_INPUT_COLUMNS {
        return Err(AlgoError::Data(format!(
            "input column count {} exceeds limit {MAX_INPUT_COLUMNS}",
            data.inputs.len()
        )));
    }

    // Validate column shapes, category declarations, finite values, and count total features.
    let mut num_features: usize = 0;
    for col in &data.inputs {
        match col {
            InputColumn::Numeric { name, values } => {
                if values.len() != data.num_records {
                    return Err(AlgoError::Data(format!(
                        "numeric input '{name}' length {} does not match num_records {}",
                        values.len(),
                        data.num_records
                    )));
                }
                for &opt in values {
                    if let Some(v) = opt
                        && !v.is_finite()
                    {
                        return Err(AlgoError::Data(format!(
                            "numeric input '{name}' contains a non-finite value"
                        )));
                    }
                }
                num_features = num_features
                    .checked_add(1)
                    .ok_or_else(|| AlgoError::Data("feature count overflow".into()))?;
            }
            InputColumn::Nominal {
                name,
                labels,
                codes,
            } => {
                if codes.len() != data.num_records {
                    return Err(AlgoError::Data(format!(
                        "nominal input '{name}' length {} does not match num_records {}",
                        codes.len(),
                        data.num_records
                    )));
                }
                if labels.is_empty() {
                    return Err(AlgoError::Data(format!(
                        "nominal input '{name}' has no category labels"
                    )));
                }
                num_features = num_features
                    .checked_add(labels.len())
                    .ok_or_else(|| AlgoError::Data("feature count overflow".into()))?;
                if num_features > MAX_FEATURES {
                    return Err(AlgoError::Data(format!(
                        "encoded feature count {num_features} exceeds limit {MAX_FEATURES}"
                    )));
                }
                let mut seen = HashSet::with_capacity(labels.len());
                for label in labels {
                    if !seen.insert(label.as_str()) {
                        return Err(AlgoError::Data(format!(
                            "nominal input '{name}' has duplicate category label '{label}'"
                        )));
                    }
                }
                for &opt in codes {
                    if let Some(code) = opt
                        && code >= labels.len()
                    {
                        return Err(AlgoError::Data(format!(
                            "nominal input '{name}' has out-of-range category code {code} (labels: {})",
                            labels.len()
                        )));
                    }
                }
            }
        }
        if num_features > MAX_FEATURES {
            return Err(AlgoError::Data(format!(
                "encoded feature count {num_features} exceeds limit {MAX_FEATURES}"
            )));
        }
    }

    if num_features == 0 {
        return Err(AlgoError::Data(
            "SVM training data produces zero encoded features".into(),
        ));
    }

    // Blank handling (Guide p. 387): exclude records with any missing input or target value.
    let mut record_indices = Vec::new();
    let mut labels = Vec::new();
    let mut positive_records: usize = 0;
    let mut negative_records: usize = 0;
    for (rec_idx, &target_opt) in target_values.iter().enumerate() {
        let Some(is_pos) = target_opt else {
            continue;
        };
        let mut complete = true;
        for col in &data.inputs {
            let is_missing = match col {
                InputColumn::Numeric { values, .. } => {
                    values.get(rec_idx).copied().flatten().is_none()
                }
                InputColumn::Nominal { codes, .. } => {
                    codes.get(rec_idx).copied().flatten().is_none()
                }
            };
            if is_missing {
                complete = false;
                break;
            }
        }
        if !complete {
            continue;
        }
        record_indices.push(rec_idx);
        if is_pos {
            labels.push(1.0);
            positive_records = positive_records.saturating_add(1);
        } else {
            labels.push(-1.0);
            negative_records = negative_records.saturating_add(1);
        }
    }

    let n = record_indices.len();
    let excluded_records = data.num_records.saturating_sub(n);
    if n == 0 {
        return Err(AlgoError::Data(
            "no complete training records remain after excluding missing values".into(),
        ));
    }
    if n > MAX_RECORDS {
        return Err(gap(&format!(
            "training with {n} complete records exceeds dense-kernel limit {MAX_RECORDS}; \
             Fast SVM Training (divide-and-conquer) subset size d, partition count k and loop \
             limit T are unspecified in the Guide"
        )));
    }
    if positive_records == 0 || negative_records == 0 {
        return Err(AlgoError::Data(
            "binary C-SVC requires at least one positive and one negative complete record".into(),
        ));
    }
    let design_cells = n
        .checked_mul(num_features)
        .ok_or_else(|| AlgoError::Data("design matrix size overflow".into()))?;
    if design_cells > MAX_DESIGN_CELLS {
        return Err(AlgoError::Data(format!(
            "design matrix cells {design_cells} exceeds limit {MAX_DESIGN_CELLS}"
        )));
    }

    // Fit input transforms on the complete training records.
    let mut transforms = Vec::with_capacity(data.inputs.len());
    let mut feat_cursor: usize = 0;
    for col in &data.inputs {
        match col {
            InputColumn::Numeric { name, values } => {
                let mut min = f64::INFINITY;
                let mut max = f64::NEG_INFINITY;
                for &rec_idx in &record_indices {
                    if let Some(v) = values.get(rec_idx).copied().flatten() {
                        if v < min {
                            min = v;
                        }
                        if v > max {
                            max = v;
                        }
                    }
                }
                if !min.is_finite() || !max.is_finite() || !(max - min).is_finite() {
                    return Err(AlgoError::Data(format!(
                        "numeric input '{name}' range [{min}, {max}] is not finite"
                    )));
                }
                transforms.push(InputTransform::Numeric {
                    name: name.clone(),
                    min,
                    max,
                    feature_index: feat_cursor,
                });
                feat_cursor = feat_cursor.saturating_add(1);
            }
            InputColumn::Nominal { name, labels, .. } => {
                transforms.push(InputTransform::Nominal {
                    name: name.clone(),
                    labels: labels.clone(),
                    start_index: feat_cursor,
                });
                feat_cursor = feat_cursor.saturating_add(labels.len());
            }
        }
    }

    // Build encoded feature vectors.
    let mut features = Vec::with_capacity(n);
    for &rec_idx in &record_indices {
        let mut row = vec![0.0; num_features];
        for (col, tr) in data.inputs.iter().zip(&transforms) {
            match (col, tr) {
                (
                    InputColumn::Numeric { values, .. },
                    InputTransform::Numeric {
                        min,
                        max,
                        feature_index,
                        ..
                    },
                ) => {
                    if let Some(raw) = values.get(rec_idx).copied().flatten()
                        && let Some(slot) = row.get_mut(*feature_index)
                    {
                        *slot = scale_numeric(raw, *min, *max);
                    }
                }
                (
                    InputColumn::Nominal { codes, .. },
                    InputTransform::Nominal {
                        labels,
                        start_index,
                        ..
                    },
                ) => {
                    if let Some(code) = codes.get(rec_idx).copied().flatten()
                        && code < labels.len()
                        && let Some(idx) = start_index.checked_add(code)
                        && let Some(slot) = row.get_mut(idx)
                    {
                        *slot = 1.0;
                    }
                }
                _ => {}
            }
        }
        features.push(row);
    }

    Ok(PreparedData {
        record_indices,
        features,
        labels,
        transforms,
        num_features,
        positive_records,
        negative_records,
        excluded_records,
    })
}

fn scale_numeric(raw: f64, min: f64, max: f64) -> f64 {
    let span = max - min;
    if span == 0.0 || !span.is_finite() {
        0.0
    } else {
        (raw - min) / span
    }
}

// ------------------------------------------------------------------------------------------------
// Kernel evaluation & symmetric kernel matrix

fn eval_kernel(kernel: Kernel, a: &[f64], b: &[f64]) -> Result<f64, AlgoError> {
    let val = match kernel {
        Kernel::Linear => dot(a, b),
        Kernel::Polynomial {
            gamma,
            coef0,
            degree,
        } => {
            let base = gamma * dot(a, b) + coef0;
            pow_u32(base, degree)
        }
        Kernel::Rbf { gamma } => {
            let d2 = squared_distance(a, b);
            (-gamma * d2).exp()
        }
        Kernel::Sigmoid { gamma, coef0 } => (gamma * dot(a, b) + coef0).tanh(),
    };
    if !val.is_finite() {
        return Err(AlgoError::Data(
            "kernel evaluation produced a non-finite value".into(),
        ));
    }
    Ok(val)
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    let mut sum = 0.0;
    for (&x, &y) in a.iter().zip(b) {
        sum += x * y;
    }
    sum
}

fn squared_distance(a: &[f64], b: &[f64]) -> f64 {
    let mut sum = 0.0;
    for (&x, &y) in a.iter().zip(b) {
        let d = x - y;
        sum += d * d;
    }
    sum
}

fn pow_u32(base: f64, mut exp: u32) -> f64 {
    let mut acc = 1.0;
    let mut cur = base;
    while exp > 0 {
        if exp & 1 == 1 {
            acc *= cur;
        }
        exp >>= 1;
        if exp > 0 {
            cur *= cur;
        }
    }
    acc
}

struct KernelMatrix {
    n: usize,
    data: Vec<f64>,
}

impl KernelMatrix {
    fn get(&self, row: usize, col: usize) -> f64 {
        row.checked_mul(self.n)
            .and_then(|base| base.checked_add(col))
            .and_then(|idx| self.data.get(idx))
            .copied()
            .unwrap_or(0.0)
    }

    fn row(&self, row: usize) -> &[f64] {
        let start = row.saturating_mul(self.n);
        let end = start.saturating_add(self.n);
        self.data.get(start..end).unwrap_or(&[])
    }
}

fn build_kernel_matrix(kernel: Kernel, features: &[Vec<f64>]) -> Result<KernelMatrix, AlgoError> {
    let n = features.len();
    let total = n
        .checked_mul(n)
        .ok_or_else(|| AlgoError::Data("kernel matrix size overflow".into()))?;
    let mut data = vec![0.0; total];
    for (i, xi) in features.iter().enumerate() {
        if i & 63 == 0 {
            crate::check_cancelled()?;
        }
        let diag = match kernel {
            Kernel::Rbf { .. } => 1.0,
            _ => eval_kernel(kernel, xi, xi)?,
        };
        if let Some(slot) = i
            .checked_mul(n)
            .and_then(|r| r.checked_add(i))
            .and_then(|idx| data.get_mut(idx))
        {
            *slot = diag;
        }
        let row_i = i.saturating_mul(n);
        for (j_off, xj) in features
            .get(i.saturating_add(1)..)
            .unwrap_or(&[])
            .iter()
            .enumerate()
        {
            let j = i.saturating_add(1).saturating_add(j_off);
            let k = eval_kernel(kernel, xi, xj)?;
            if let Some(slot) = row_i.checked_add(j).and_then(|idx| data.get_mut(idx)) {
                *slot = k;
            }
            if let Some(slot) = j
                .checked_mul(n)
                .and_then(|r| r.checked_add(i))
                .and_then(|idx| data.get_mut(idx))
            {
                *slot = k;
            }
        }
    }
    Ok(KernelMatrix { n, data })
}

// ------------------------------------------------------------------------------------------------
// SMO solver (Guide pp. 380-385): WSS2, shrinking with gradient reconstruction, constant b

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AlphaStatus {
    Lower,
    Free,
    Upper,
}

struct SmoResult {
    alpha: Vec<f64>,
    grad: Vec<f64>,
    constant: f64,
    iterations: usize,
    kkt_gap: f64,
    dual_objective: f64,
}

fn status_of(alpha: f64, c: f64) -> AlphaStatus {
    if alpha >= c {
        AlphaStatus::Upper
    } else if alpha <= 0.0 {
        AlphaStatus::Lower
    } else {
        AlphaStatus::Free
    }
}

fn solve_smo(
    kmat: &KernelMatrix,
    y: &[f64],
    c: f64,
    eps: f64,
    use_shrinking: bool,
    max_iterations: usize,
) -> Result<SmoResult, AlgoError> {
    let n = y.len();
    let mut alpha = vec![0.0; n];
    let mut status = vec![AlphaStatus::Lower; n];
    let mut grad = vec![-1.0; n];
    let mut g_bar = vec![0.0; n];
    let mut active: Vec<usize> = (0..n).collect();
    let mut active_size = n;
    let mut unshrink = false;
    let shrink_period = n.clamp(1, SHRINK_PERIOD_CAP);
    let mut counter = shrink_period;

    let mut converged = false;
    let mut iterations: usize = 0;

    for iter in 0..max_iterations {
        if iter & 255 == 0 {
            crate::check_cancelled()?;
        }

        // Step 2 (Guide p. 384): select i in I_up maximizing -y_t grad_t and check stationarity.
        let mut g_max = f64::NEG_INFINITY;
        let mut g_max_idx: Option<usize> = None;
        let mut g_max2 = f64::NEG_INFINITY;

        for &t in active.get(..active_size).unwrap_or(&[]) {
            let yt = y.get(t).copied().unwrap_or(0.0);
            let gt = grad.get(t).copied().unwrap_or(0.0);
            let st = status.get(t).copied().unwrap_or(AlphaStatus::Lower);
            if yt > 0.0 {
                if st != AlphaStatus::Upper {
                    let v = -gt;
                    if v >= g_max {
                        g_max = v;
                        g_max_idx = Some(t);
                    }
                }
                if st != AlphaStatus::Lower && gt >= g_max2 {
                    g_max2 = gt;
                }
            } else {
                if st != AlphaStatus::Lower && gt >= g_max {
                    g_max = gt;
                    g_max_idx = Some(t);
                }
                if st != AlphaStatus::Upper {
                    let v = -gt;
                    if v >= g_max2 {
                        g_max2 = v;
                    }
                }
            }
        }

        if g_max + g_max2 <= eps {
            if active_size == n {
                converged = true;
                iterations = iter;
                break;
            }
            reconstruct_gradient(
                kmat,
                y,
                &alpha,
                &status,
                &g_bar,
                &active,
                active_size,
                &mut grad,
            );
            active_size = n;
            counter = shrink_period;
            continue;
        }

        // Step 3 (Guide pp. 382, 384): periodic shrinking.
        if use_shrinking {
            counter = counter.saturating_sub(1);
            if counter == 0 {
                counter = shrink_period;
                if !unshrink && g_max + g_max2 <= 10.0 * eps {
                    unshrink = true;
                    reconstruct_gradient(
                        kmat,
                        y,
                        &alpha,
                        &status,
                        &g_bar,
                        &active,
                        active_size,
                        &mut grad,
                    );
                    active_size = n;
                }
                active_size =
                    shrink_active(y, &status, &grad, &mut active, active_size, g_max, g_max2);
                continue;
            }
        }

        let Some(i) = g_max_idx else {
            if active_size == n {
                converged = true;
                iterations = iter;
                break;
            }
            reconstruct_gradient(
                kmat,
                y,
                &alpha,
                &status,
                &g_bar,
                &active,
                active_size,
                &mut grad,
            );
            active_size = n;
            counter = shrink_period;
            continue;
        };

        let yi = y.get(i).copied().unwrap_or(0.0);
        let row_i = kmat.row(i);
        let k_ii = row_i.get(i).copied().unwrap_or(0.0);

        // Second-order working set selection (Guide p. 381): select j in I_low with b_it > 0
        // minimizing -b_it^2 / a_bar_it.
        let mut g_min_idx: Option<usize> = None;
        let mut obj_diff_min = f64::INFINITY;
        for &t in active.get(..active_size).unwrap_or(&[]) {
            let yt = y.get(t).copied().unwrap_or(0.0);
            let st = status.get(t).copied().unwrap_or(AlphaStatus::Lower);
            let in_i_low =
                (yt > 0.0 && st != AlphaStatus::Lower) || (yt < 0.0 && st != AlphaStatus::Upper);
            if !in_i_low {
                continue;
            }
            let gt = grad.get(t).copied().unwrap_or(0.0);
            let b_it = g_max + yt * gt;
            if b_it <= 0.0 {
                continue;
            }
            let k_tt = kmat.get(t, t);
            let k_it = row_i.get(t).copied().unwrap_or(0.0);
            let quad = k_ii + k_tt - 2.0 * k_it;
            let a_bar = if quad > 0.0 { quad } else { TAU };
            let obj_diff = -(b_it * b_it) / a_bar;
            if obj_diff <= obj_diff_min {
                obj_diff_min = obj_diff;
                g_min_idx = Some(t);
            }
        }

        let Some(j) = g_min_idx else {
            if active_size == n {
                converged = true;
                iterations = iter;
                break;
            }
            reconstruct_gradient(
                kmat,
                y,
                &alpha,
                &status,
                &g_bar,
                &active,
                active_size,
                &mut grad,
            );
            active_size = n;
            counter = shrink_period;
            continue;
        };

        // Step 4 (Guide pp. 383-385): analytically solve the two-variable subproblem and clip.
        let yj = y.get(j).copied().unwrap_or(0.0);
        let row_j = kmat.row(j);
        let k_jj = row_j.get(j).copied().unwrap_or(0.0);
        let k_ij = row_i.get(j).copied().unwrap_or(0.0);
        let old_ai = alpha.get(i).copied().unwrap_or(0.0);
        let old_aj = alpha.get(j).copied().unwrap_or(0.0);
        let was_upper_i = status.get(i).copied() == Some(AlphaStatus::Upper);
        let was_upper_j = status.get(j).copied() == Some(AlphaStatus::Upper);

        let quad = k_ii + k_jj - 2.0 * k_ij;
        let a_bar = if quad > 0.0 { quad } else { TAU };
        let gj = grad.get(j).copied().unwrap_or(0.0);
        let b_ij = g_max + yj * gj;
        let step = b_ij / a_bar;

        let mut new_ai = old_ai + yi * step;
        let mut new_aj = old_aj - yj * step;

        if yi * yj < 0.0 {
            let diff = old_ai - old_aj;
            if diff > 0.0 {
                if new_aj < 0.0 {
                    new_aj = 0.0;
                    new_ai = diff;
                }
                if new_ai > c {
                    new_ai = c;
                    new_aj = c - diff;
                }
            } else {
                if new_ai < 0.0 {
                    new_ai = 0.0;
                    new_aj = -diff;
                }
                if new_aj > c {
                    new_aj = c;
                    new_ai = c + diff;
                }
            }
        } else {
            let sum = old_ai + old_aj;
            if sum > c {
                if new_ai > c {
                    new_ai = c;
                    new_aj = sum - c;
                }
                if new_aj > c {
                    new_aj = c;
                    new_ai = sum - c;
                }
            } else {
                if new_aj < 0.0 {
                    new_aj = 0.0;
                    new_ai = sum;
                }
                if new_ai < 0.0 {
                    new_ai = 0.0;
                    new_aj = sum;
                }
            }
        }

        // Guard against any tiny floating-point overshoot outside [0, C].
        new_ai = new_ai.clamp(0.0, c);
        new_aj = new_aj.clamp(0.0, c);
        if !new_ai.is_finite() || !new_aj.is_finite() {
            return Err(AlgoError::Data(
                "SMO update produced non-finite multiplier".into(),
            ));
        }

        if let Some(slot) = alpha.get_mut(i) {
            *slot = new_ai;
        }
        if let Some(slot) = alpha.get_mut(j) {
            *slot = new_aj;
        }
        let st_i = status_of(new_ai, c);
        let st_j = status_of(new_aj, c);
        if let Some(slot) = status.get_mut(i) {
            *slot = st_i;
        }
        if let Some(slot) = status.get_mut(j) {
            *slot = st_j;
        }

        let delta_i = yi * (new_ai - old_ai);
        let delta_j = yj * (new_aj - old_aj);

        for &t in active.get(..active_size).unwrap_or(&[]) {
            let yt = y.get(t).copied().unwrap_or(0.0);
            let k_ti = row_i.get(t).copied().unwrap_or(0.0);
            let k_tj = row_j.get(t).copied().unwrap_or(0.0);
            if let Some(gt) = grad.get_mut(t) {
                *gt += yt * (k_ti * delta_i + k_tj * delta_j);
                if !gt.is_finite() {
                    return Err(AlgoError::Data("SMO gradient became non-finite".into()));
                }
            }
        }

        // Maintain G_bar_t = C * sum_{alpha_k == C} Q_tk (Guide p. 383) for exact reconstruction.
        if use_shrinking {
            let is_upper_i = st_i == AlphaStatus::Upper;
            if is_upper_i != was_upper_i {
                let signed_c = if is_upper_i { c * yi } else { -c * yi };
                for (t, (gb, (&yt, &k_ti))) in g_bar.iter_mut().zip(y.iter().zip(row_i)).enumerate()
                {
                    *gb += yt * k_ti * signed_c;
                    if !gb.is_finite() {
                        return Err(AlgoError::Data(format!(
                            "SMO G_bar became non-finite at index {t}"
                        )));
                    }
                }
            }
            let is_upper_j = st_j == AlphaStatus::Upper;
            if is_upper_j != was_upper_j {
                let signed_c = if is_upper_j { c * yj } else { -c * yj };
                for (t, (gb, (&yt, &k_tj))) in g_bar.iter_mut().zip(y.iter().zip(row_j)).enumerate()
                {
                    *gb += yt * k_tj * signed_c;
                    if !gb.is_finite() {
                        return Err(AlgoError::Data(format!(
                            "SMO G_bar became non-finite at index {t}"
                        )));
                    }
                }
            }
        }
    }

    if !converged {
        return Err(AlgoError::Data(format!(
            "SVM SMO did not converge within {max_iterations} iterations"
        )));
    }

    // Ensure full gradient is current across all records.
    if active_size < n {
        reconstruct_gradient(
            kmat,
            y,
            &alpha,
            &status,
            &g_bar,
            &active,
            active_size,
            &mut grad,
        );
    }

    // Compute final KKT gap and dual objective.
    let mut g_max = f64::NEG_INFINITY;
    let mut g_max2 = f64::NEG_INFINITY;
    let mut dual_objective = 0.0;
    for t in 0..n {
        let yt = y.get(t).copied().unwrap_or(0.0);
        let gt = grad.get(t).copied().unwrap_or(0.0);
        let at = alpha.get(t).copied().unwrap_or(0.0);
        let st = status.get(t).copied().unwrap_or(AlphaStatus::Lower);
        dual_objective += 0.5 * at * (gt - 1.0);
        if yt > 0.0 {
            if st != AlphaStatus::Upper && -gt > g_max {
                g_max = -gt;
            }
            if st != AlphaStatus::Lower && gt > g_max2 {
                g_max2 = gt;
            }
        } else {
            if st != AlphaStatus::Lower && gt > g_max {
                g_max = gt;
            }
            if st != AlphaStatus::Upper && -gt > g_max2 {
                g_max2 = -gt;
            }
        }
    }
    let kkt_gap = if g_max.is_finite() && g_max2.is_finite() {
        (g_max + g_max2).max(0.0)
    } else {
        0.0
    };

    // Constant b: average of -y_i * grad_i over free support vectors, else midpoint of bounds.
    let mut free_sum = 0.0;
    let mut n_free: usize = 0;
    let mut ub = f64::INFINITY;
    let mut lb = f64::NEG_INFINITY;
    for t in 0..n {
        let yt = y.get(t).copied().unwrap_or(0.0);
        let gt = grad.get(t).copied().unwrap_or(0.0);
        let yg = yt * gt;
        match status.get(t).copied().unwrap_or(AlphaStatus::Lower) {
            AlphaStatus::Free => {
                free_sum += yg;
                n_free = n_free.saturating_add(1);
            }
            AlphaStatus::Upper => {
                if yt < 0.0 {
                    if yg < ub {
                        ub = yg;
                    }
                } else if yg > lb {
                    lb = yg;
                }
            }
            AlphaStatus::Lower => {
                if yt > 0.0 {
                    if yg < ub {
                        ub = yg;
                    }
                } else if yg > lb {
                    lb = yg;
                }
            }
        }
    }
    let constant = if n_free > 0 {
        -(free_sum / (n_free as f64))
    } else if ub.is_finite() && lb.is_finite() {
        -0.5 * (ub + lb)
    } else {
        0.0
    };
    if !constant.is_finite() {
        return Err(AlgoError::Data(
            "SVM decision constant is not finite".into(),
        ));
    }

    Ok(SmoResult {
        alpha,
        grad,
        constant,
        iterations,
        kkt_gap,
        dual_objective,
    })
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_gradient(
    kmat: &KernelMatrix,
    y: &[f64],
    alpha: &[f64],
    status: &[AlphaStatus],
    g_bar: &[f64],
    active: &[usize],
    active_size: usize,
    grad: &mut [f64],
) {
    let n = active.len();
    if active_size >= n {
        return;
    }
    let inactive = active.get(active_size..).unwrap_or(&[]);
    for &i in inactive {
        let gb = g_bar.get(i).copied().unwrap_or(0.0);
        if let Some(gi) = grad.get_mut(i) {
            *gi = gb - 1.0;
        }
    }
    for &j in active {
        if status.get(j).copied() == Some(AlphaStatus::Free) {
            let aj_yj = alpha.get(j).copied().unwrap_or(0.0) * y.get(j).copied().unwrap_or(0.0);
            let row_j = kmat.row(j);
            for &i in inactive {
                let yi = y.get(i).copied().unwrap_or(0.0);
                let k_ji = row_j.get(i).copied().unwrap_or(0.0);
                if let Some(gi) = grad.get_mut(i) {
                    *gi += yi * k_ji * aj_yj;
                }
            }
        }
    }
}

fn be_shrunk(st: AlphaStatus, yt: f64, gt: f64, g_max1: f64, g_max2: f64) -> bool {
    match st {
        AlphaStatus::Upper => {
            if yt > 0.0 {
                -gt > g_max1
            } else {
                -gt > g_max2
            }
        }
        AlphaStatus::Lower => {
            if yt > 0.0 {
                gt > g_max2
            } else {
                gt > g_max1
            }
        }
        AlphaStatus::Free => false,
    }
}

fn shrink_active(
    y: &[f64],
    status: &[AlphaStatus],
    grad: &[f64],
    active: &mut [usize],
    mut active_size: usize,
    g_max1: f64,
    g_max2: f64,
) -> usize {
    let mut i: usize = 0;
    while i < active_size {
        let ti = active.get(i).copied().unwrap_or(0);
        let st_i = status.get(ti).copied().unwrap_or(AlphaStatus::Free);
        let y_i = y.get(ti).copied().unwrap_or(0.0);
        let g_i = grad.get(ti).copied().unwrap_or(0.0);
        if be_shrunk(st_i, y_i, g_i, g_max1, g_max2) {
            active_size = active_size.saturating_sub(1);
            while active_size > i {
                let tail = active.get(active_size).copied().unwrap_or(0);
                let st_t = status.get(tail).copied().unwrap_or(AlphaStatus::Free);
                let y_t = y.get(tail).copied().unwrap_or(0.0);
                let g_t = grad.get(tail).copied().unwrap_or(0.0);
                if !be_shrunk(st_t, y_t, g_t, g_max1, g_max2) {
                    active.swap(i, active_size);
                    break;
                }
                active_size = active_size.saturating_sub(1);
            }
        }
        i = i.saturating_add(1);
    }
    active_size
}

// ------------------------------------------------------------------------------------------------
// Platt probability calibration (Guide p. 387)

fn platt_probability(platt: PlattParameters, f: f64) -> f64 {
    let z = platt.param_a * f + platt.param_b;
    if z >= 0.0 {
        let e = (-z).exp();
        e / (1.0 + e)
    } else {
        1.0 / (1.0 + z.exp())
    }
}

fn platt_objective(decision_values: &[f64], targets: &[f64], a: f64, b: f64) -> f64 {
    let mut total = 0.0;
    for (&f, &t) in decision_values.iter().zip(targets) {
        let z = a * f + b;
        if z >= 0.0 {
            total += t * z + (-z).exp().ln_1p();
        } else {
            total += (t - 1.0) * z + z.exp().ln_1p();
        }
    }
    total
}

fn fit_platt(
    decision_values: &[f64],
    labels: &[f64],
    n_pos: usize,
    n_neg: usize,
) -> Result<PlattParameters, AlgoError> {
    let n_pos_f = n_pos as f64;
    let n_neg_f = n_neg as f64;
    let hi_target = (n_pos_f + 1.0) / (n_pos_f + 2.0);
    let lo_target = 1.0 / (n_neg_f + 2.0);
    let targets: Vec<f64> = labels
        .iter()
        .map(|&y| if y > 0.0 { hi_target } else { lo_target })
        .collect();

    let mut a = 0.0;
    let mut b = ((n_neg_f + 1.0) / (n_pos_f + 1.0)).ln();
    let mut f_val = platt_objective(decision_values, &targets, a, b);

    for _ in 0..MAX_PLATT_ITERATIONS {
        let mut h11 = PLATT_SIGMA;
        let mut h22 = PLATT_SIGMA;
        let mut h21 = 0.0;
        let mut g1 = 0.0;
        let mut g2 = 0.0;

        for (&f, &t) in decision_values.iter().zip(&targets) {
            let z = a * f + b;
            let (p, q) = if z >= 0.0 {
                let e = (-z).exp();
                let denom = 1.0 + e;
                (e / denom, 1.0 / denom)
            } else {
                let e = z.exp();
                let denom = 1.0 + e;
                (1.0 / denom, e / denom)
            };
            let d2 = p * q;
            h11 += f * f * d2;
            h22 += d2;
            h21 += f * d2;
            let d1 = t - p;
            g1 += f * d1;
            g2 += d1;
        }

        if g1.abs() < PLATT_GRAD_TOL && g2.abs() < PLATT_GRAD_TOL {
            break;
        }

        let det = h11 * h22 - h21 * h21;
        if !det.is_finite() || det <= 0.0 {
            break;
        }
        let d_a = -(h22 * g1 - h21 * g2) / det;
        let d_b = -(-h21 * g1 + h11 * g2) / det;
        let gd = g1 * d_a + g2 * d_b;
        if !gd.is_finite() || gd >= 0.0 {
            break;
        }

        let mut step = 1.0;
        let mut accepted = false;
        while step >= PLATT_MIN_STEP {
            let new_a = a + step * d_a;
            let new_b = b + step * d_b;
            let new_f = platt_objective(decision_values, &targets, new_a, new_b);
            if new_f.is_finite() && new_f < f_val + PLATT_ARMIJO * step * gd {
                a = new_a;
                b = new_b;
                f_val = new_f;
                accepted = true;
                break;
            }
            step *= 0.5;
        }
        if !accepted {
            break;
        }
    }

    if !a.is_finite() || !b.is_finite() {
        return Err(AlgoError::Data(
            "Platt calibration produced non-finite parameters".into(),
        ));
    }

    Ok(PlattParameters {
        param_a: a,
        param_b: b,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_linear_data() -> SvmData {
        // Two 1-D points: x_0 = 0 (neg, y=-1), x_1 = 2 (pos, y=+1).
        // Scaled to [0, 1]: x_0' = 0.0, x_1' = 1.0.
        // Linear kernel K = [[0, 0], [0, 1]], Q = [[0, 0], [0, 1]].
        // Dual: min 0.5 * alpha_1^2 - (alpha_0 + alpha_1) s.t. alpha_1 - alpha_0 = 0, 0 <= alpha <= C.
        // With alpha_0 = alpha_1 = a: min 0.5 * a^2 - 2a => unconstrained optimum a = 2.0.
        // For C = 10: alpha_0 = alpha_1 = 2.0 (both free SVs).
        // Decision function: f(x') = 2*1*x' - 2*0*x' + b = 2*x' + b.
        // Free SV KKT: f(0) = -1 => b = -1.0; f(1) = 2 - 1 = +1.
        // Dual objective: 0.5 * 4 - 4 = -2.0.
        SvmData {
            num_records: 2,
            inputs: vec![InputColumn::Numeric {
                name: "x".into(),
                values: vec![Some(0.0), Some(2.0)],
            }],
            target: Target::Binary {
                name: "y".into(),
                positive_label: "pos".into(),
                negative_label: "neg".into(),
                values: vec![Some(false), Some(true)],
            },
        }
    }

    #[test]
    fn svm_known_answer_linear_two_points() -> Result<(), String> {
        let data = tiny_linear_data();
        let settings = Settings {
            c: 10.0,
            stopping_criteria: 1e-9,
            kernel: Kernel::Linear,
            ..Settings::default()
        };
        let model = train(&data, &settings).map_err(|e| format!("unexpected error: {e}"))?;
        assert_eq!(model.support_vectors.len(), 2);
        let sv0 = model.support_vectors.first().ok_or("missing sv0")?;
        let sv1 = model.support_vectors.get(1).ok_or("missing sv1")?;
        assert!((sv0.coefficient - (-2.0)).abs() < 1e-9);
        assert!((sv1.coefficient - 2.0).abs() < 1e-9);
        assert!((model.constant - (-1.0)).abs() < 1e-9);
        assert!((model.summary.dual_objective - (-2.0)).abs() < 1e-9);
        let d0 = model.decision(&[InputValue::Numeric(Some(0.0))]);
        let d1 = model.decision(&[InputValue::Numeric(Some(2.0))]);
        let d_mid = model.decision(&[InputValue::Numeric(Some(1.0))]);
        assert!(matches!(d0, Ok(Some(v)) if (v - (-1.0)).abs() < 1e-9));
        assert!(matches!(d1, Ok(Some(v)) if (v - 1.0).abs() < 1e-9));
        assert!(matches!(d_mid, Ok(Some(v)) if v.abs() < 1e-9));
        assert!(matches!(
            model.predict(&[InputValue::Numeric(Some(0.0))]),
            Ok(Some("neg"))
        ));
        assert!(matches!(
            model.predict(&[InputValue::Numeric(Some(2.0))]),
            Ok(Some("pos"))
        ));
        // Missing input at scoring time returns Ok(None) (Guide p. 387).
        assert!(matches!(
            model.decision(&[InputValue::Numeric(None)]),
            Ok(None)
        ));
        assert!(matches!(
            model.probability(&[InputValue::Numeric(None)]),
            Ok(None)
        ));
        assert!(matches!(
            model.predict(&[InputValue::Numeric(None)]),
            Ok(None)
        ));
        Ok(())
    }

    #[test]
    fn svm_known_answer_rbf_two_points() -> Result<(), String> {
        // Two 1-D points scaled to 0.0 (neg) and 1.0 (pos).
        // RBF(gamma = 1): K_00 = K_11 = 1, K_01 = e^-1.
        // Dual with alpha_0 = alpha_1 = a: min (1 - e^-1) * a^2 - 2a => a = 1 / (1 - e^-1).
        // By symmetry, b = 0, f(0) = -1, f(1) = +1.
        let data = tiny_linear_data();
        let settings = Settings {
            c: 10.0,
            stopping_criteria: 1e-9,
            kernel: Kernel::Rbf { gamma: 1.0 },
            ..Settings::default()
        };
        let model = train(&data, &settings).map_err(|e| format!("unexpected error: {e}"))?;
        let expected_a = 1.0 / (1.0 - (-1.0_f64).exp());
        assert_eq!(model.support_vectors.len(), 2);
        let sv0 = model.support_vectors.first().ok_or("missing sv0")?;
        let sv1 = model.support_vectors.get(1).ok_or("missing sv1")?;
        assert!((sv0.coefficient - (-expected_a)).abs() < 1e-9);
        assert!((sv1.coefficient - expected_a).abs() < 1e-9);
        assert!(model.constant.abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn svm_known_answer_all_boundary_svs_no_free_sv() -> Result<(), String> {
        // Same 1-D linear problem with C = 0.5 < unconstrained a = 2.0.
        // Both SVs hit the upper bound: alpha_0 = alpha_1 = 0.5 (no free SVs).
        // grad_0 = 0 - 1 = -1 (y=-1, alpha=C => ub = y*g = +1),
        // grad_1 = 0.5 - 1 = -0.5 (y=+1, alpha=C => lb = y*g = -0.5).
        // Midpoint constant: b = -0.5 * (ub + lb) = -0.5 * (1.0 - 0.5) = -0.25.
        let data = tiny_linear_data();
        let settings = Settings {
            c: 0.5,
            stopping_criteria: 1e-9,
            kernel: Kernel::Linear,
            ..Settings::default()
        };
        let model = train(&data, &settings).map_err(|e| format!("unexpected error: {e}"))?;
        assert_eq!(model.support_vectors.len(), 2);
        assert!((model.constant - (-0.25)).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn svm_known_answer_xor_polynomial_and_sigmoid() -> Result<(), String> {
        // 4 XOR points in [0, 1]^2: (0,0)->neg, (1,1)->neg, (0,1)->pos, (1,0)->pos.
        let data = SvmData {
            num_records: 4,
            inputs: vec![
                InputColumn::Numeric {
                    name: "x1".into(),
                    values: vec![Some(0.0), Some(1.0), Some(0.0), Some(1.0)],
                },
                InputColumn::Numeric {
                    name: "x2".into(),
                    values: vec![Some(0.0), Some(1.0), Some(1.0), Some(0.0)],
                },
            ],
            target: Target::Binary {
                name: "cls".into(),
                positive_label: "1".into(),
                negative_label: "0".into(),
                values: vec![Some(false), Some(false), Some(true), Some(true)],
            },
        };
        let poly_settings = Settings {
            c: 10.0,
            stopping_criteria: 1e-6,
            kernel: Kernel::Polynomial {
                gamma: 1.0,
                coef0: 1.0,
                degree: 2,
            },
            ..Settings::default()
        };
        let poly = train(&data, &poly_settings).map_err(|e| format!("poly XOR failed: {e}"))?;
        // Dual constraint: sum(y_i alpha_i) == 0, every |coeff| <= C.
        let sum_coeff: f64 = poly.support_vectors.iter().map(|sv| sv.coefficient).sum();
        assert!(sum_coeff.abs() < 1e-10);
        for sv in &poly.support_vectors {
            assert!(sv.coefficient.abs() <= poly_settings.c + 1e-12);
        }
        // XOR must be separated perfectly by degree-2 polynomial kernel.
        for (i, &(x1, x2, want)) in [
            (0.0, 0.0, "0"),
            (1.0, 1.0, "0"),
            (0.0, 1.0, "1"),
            (1.0, 0.0, "1"),
        ]
        .iter()
        .enumerate()
        {
            let pred =
                poly.predict(&[InputValue::Numeric(Some(x1)), InputValue::Numeric(Some(x2))]);
            assert!(
                matches!(pred, Ok(Some(lbl)) if lbl == want),
                "XOR row {i}: expected {want}, got {pred:?}"
            );
        }

        // Sigmoid kernel (potentially non-PSD, exercises curvature floor tau).
        let sig_settings = Settings {
            c: 1.0,
            stopping_criteria: 1e-3,
            kernel: Kernel::Sigmoid {
                gamma: 0.5,
                coef0: 0.0,
            },
            ..Settings::default()
        };
        let sig = train(&data, &sig_settings).map_err(|e| format!("sigmoid failed: {e}"))?;
        let text = sig.to_text();
        assert!(text.contains("SVM Model (C-SVC)"));
        assert!(text.contains("Sigmoid"));
        Ok(())
    }

    #[test]
    fn svm_properties_dual_constraints_and_decision_reproduction() -> Result<(), String> {
        // 8-point dataset with both numeric and nominal inputs plus one missing row.
        let data = SvmData {
            num_records: 9,
            inputs: vec![
                InputColumn::Numeric {
                    name: "v".into(),
                    values: vec![
                        Some(1.0),
                        Some(2.0),
                        Some(3.0),
                        Some(4.0),
                        Some(7.0),
                        Some(8.0),
                        Some(9.0),
                        Some(10.0),
                        None, // excluded by blank handling
                    ],
                },
                InputColumn::Nominal {
                    name: "cat".into(),
                    labels: vec!["A".into(), "B".into()],
                    codes: vec![
                        Some(0),
                        Some(0),
                        Some(1),
                        Some(0),
                        Some(1),
                        Some(1),
                        Some(0),
                        Some(1),
                        Some(0),
                    ],
                },
            ],
            target: Target::Binary {
                name: "y".into(),
                positive_label: "yes".into(),
                negative_label: "no".into(),
                values: vec![
                    Some(false),
                    Some(false),
                    Some(false),
                    Some(false),
                    Some(true),
                    Some(true),
                    Some(true),
                    Some(true),
                    Some(true),
                ],
            },
        };
        let settings = Settings {
            c: 5.0,
            stopping_criteria: 1e-6,
            kernel: Kernel::Rbf { gamma: 0.5 },
            ..Settings::default()
        };
        let model = train(&data, &settings).map_err(|e| format!("unexpected error: {e}"))?;
        assert_eq!(model.summary.complete_records, 8);
        assert_eq!(model.summary.excluded_records, 1);

        // Dual equality and box constraints.
        let sum_y_alpha: f64 = model.support_vectors.iter().map(|sv| sv.coefficient).sum();
        assert!(sum_y_alpha.abs() < 1e-10);
        for sv in &model.support_vectors {
            assert!(sv.coefficient.abs() > 0.0 && sv.coefficient.abs() <= settings.c + 1e-12);
        }
        assert!(model.summary.kkt_gap <= settings.stopping_criteria + 1e-12);

        // Scoring each free support vector must satisfy y_i * f(x_i) ≈ 1.
        for sv in &model.support_vectors {
            let f_val = model
                .decision_encoded(&sv.vector)
                .map_err(|e| format!("decision_encoded error: {e}"))?;
            let y = if sv.coefficient > 0.0 { 1.0 } else { -1.0 };
            if sv.coefficient.abs() < settings.c - 1e-9 {
                assert!((y * f_val - 1.0).abs() < 1e-4);
            }
        }

        // Scoring via raw record API matches decision_encoded, and unknown nominal category
        // maps cleanly to all-zero indicators without error.
        let raw_dec = model
            .decision(&[InputValue::Numeric(Some(1.0)), InputValue::Nominal(Some(0))])
            .map_err(|e| format!("decision error: {e}"))?
            .ok_or("unexpected None")?;
        let enc_dec = model
            .decision_encoded(&[0.0, 1.0, 0.0])
            .map_err(|e| format!("decision_encoded error: {e}"))?;
        assert!((raw_dec - enc_dec).abs() < 1e-12);
        let unseen_prob = model
            .probability(&[
                InputValue::Numeric(Some(5.0)),
                InputValue::Nominal(Some(999)),
            ])
            .map_err(|e| format!("unseen category error: {e}"))?
            .ok_or("unexpected None")?;
        assert!(unseen_prob.is_finite() && (0.0..=1.0).contains(&unseen_prob));
        Ok(())
    }

    #[test]
    fn svm_gap_errors_for_out_of_scope_targets() {
        let base_inputs = vec![InputColumn::Numeric {
            name: "x".into(),
            values: vec![Some(1.0), Some(2.0)],
        }];
        let svr = SvmData {
            num_records: 2,
            inputs: base_inputs.clone(),
            target: Target::Continuous {
                name: "z".into(),
                values: vec![Some(1.0), Some(2.0)],
            },
        };
        assert!(matches!(
            train(&svr, &Settings::default()),
            Err(AlgoError::Unsupported(msg)) if msg.contains("GAP:")
        ));

        let one_class = SvmData {
            num_records: 2,
            inputs: base_inputs.clone(),
            target: Target::OneClass,
        };
        assert!(matches!(
            train(&one_class, &Settings::default()),
            Err(AlgoError::Unsupported(msg)) if msg.contains("GAP:")
        ));

        let multi = SvmData {
            num_records: 2,
            inputs: base_inputs,
            target: Target::Multiclass {
                name: "c".into(),
                labels: vec!["a".into(), "b".into(), "c".into()],
                codes: vec![Some(0), Some(1)],
            },
        };
        assert!(matches!(
            train(&multi, &Settings::default()),
            Err(AlgoError::Unsupported(msg)) if msg.contains("GAP:")
        ));
    }

    #[test]
    fn svm_never_crash_hostile_and_degenerate_inputs() {
        let settings = Settings::default();

        // 1. Empty data
        let empty = SvmData {
            num_records: 0,
            inputs: vec![InputColumn::Numeric {
                name: "x".into(),
                values: vec![],
            }],
            target: Target::Binary {
                name: "y".into(),
                positive_label: "1".into(),
                negative_label: "0".into(),
                values: vec![],
            },
        };
        assert!(matches!(train(&empty, &settings), Err(AlgoError::Data(_))));

        // 2. One record
        let one_rec = SvmData {
            num_records: 1,
            inputs: vec![InputColumn::Numeric {
                name: "x".into(),
                values: vec![Some(3.0)],
            }],
            target: Target::Binary {
                name: "y".into(),
                positive_label: "1".into(),
                negative_label: "0".into(),
                values: vec![Some(true)],
            },
        };
        assert!(matches!(
            train(&one_rec, &settings),
            Err(AlgoError::Data(_))
        ));

        // 3. One class only
        let one_class = SvmData {
            num_records: 4,
            inputs: vec![InputColumn::Numeric {
                name: "x".into(),
                values: vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0)],
            }],
            target: Target::Binary {
                name: "y".into(),
                positive_label: "1".into(),
                negative_label: "0".into(),
                values: vec![Some(true), Some(true), Some(true), Some(true)],
            },
        };
        assert!(matches!(
            train(&one_class, &settings),
            Err(AlgoError::Data(_))
        ));

        // 4. Constant column (must not divide by zero, must train and score cleanly)
        let constant_col = SvmData {
            num_records: 4,
            inputs: vec![InputColumn::Numeric {
                name: "const".into(),
                values: vec![Some(5.0), Some(5.0), Some(5.0), Some(5.0)],
            }],
            target: Target::Binary {
                name: "y".into(),
                positive_label: "1".into(),
                negative_label: "0".into(),
                values: vec![Some(true), Some(true), Some(false), Some(false)],
            },
        };
        let const_model = train(&constant_col, &settings);
        assert!(
            const_model.is_ok(),
            "constant column failed: {const_model:?}"
        );
        if let Ok(m) = const_model {
            let p = m.probability(&[InputValue::Numeric(Some(5.0))]);
            assert!(matches!(p, Ok(Some(prob)) if prob.is_finite() && (0.0..=1.0).contains(&prob)));
        }

        // 5. Huge C (beyond MAX_C is Settings error; large valid C trains cleanly)
        let data = tiny_linear_data();
        let huge_c_over = Settings {
            c: 1e300,
            ..Settings::default()
        };
        assert!(matches!(
            train(&data, &huge_c_over),
            Err(AlgoError::Settings(_))
        ));
        let large_c_ok = Settings {
            c: 1e9,
            kernel: Kernel::Linear,
            ..Settings::default()
        };
        assert!(train(&data, &large_c_ok).is_ok());

        // 6. Nominal field with thousands of categories (exceeds MAX_FEATURES -> Data error)
        let thousands_labels: Vec<String> = (0..5_000).map(|i| format!("cat_{i}")).collect();
        let wide_nominal = SvmData {
            num_records: 2,
            inputs: vec![InputColumn::Nominal {
                name: "wide".into(),
                labels: thousands_labels,
                codes: vec![Some(0), Some(1)],
            }],
            target: Target::Binary {
                name: "y".into(),
                positive_label: "1".into(),
                negative_label: "0".into(),
                values: vec![Some(false), Some(true)],
            },
        };
        assert!(matches!(
            train(&wide_nominal, &settings),
            Err(AlgoError::Data(_))
        ));

        // 7. NaN / Inf numeric input -> Data error
        let nan_data = SvmData {
            num_records: 2,
            inputs: vec![InputColumn::Numeric {
                name: "x".into(),
                values: vec![Some(f64::NAN), Some(1.0)],
            }],
            target: Target::Binary {
                name: "y".into(),
                positive_label: "1".into(),
                negative_label: "0".into(),
                values: vec![Some(false), Some(true)],
            },
        };
        assert!(matches!(
            train(&nan_data, &settings),
            Err(AlgoError::Data(_))
        ));

        // 8. Out-of-range nominal code -> Data error
        let bad_code = SvmData {
            num_records: 2,
            inputs: vec![InputColumn::Nominal {
                name: "c".into(),
                labels: vec!["a".into(), "b".into()],
                codes: vec![Some(0), Some(99)],
            }],
            target: Target::Binary {
                name: "y".into(),
                positive_label: "1".into(),
                negative_label: "0".into(),
                values: vec![Some(false), Some(true)],
            },
        };
        assert!(matches!(
            train(&bad_code, &settings),
            Err(AlgoError::Data(_))
        ));

        // 9. Support vector cap & iteration cap -> clean Data error
        let sv_cap = Settings {
            max_support_vectors: 1,
            kernel: Kernel::Linear,
            ..Settings::default()
        };
        assert!(matches!(train(&data, &sv_cap), Err(AlgoError::Data(_))));

        let iter_cap = Settings {
            max_iterations: 1,
            stopping_criteria: 1e-12,
            kernel: Kernel::Linear,
            ..Settings::default()
        };
        assert!(matches!(train(&data, &iter_cap), Err(AlgoError::Data(_))));

        // 10. Invalid settings & hostile scoring inputs
        assert!(matches!(
            train(
                &data,
                &Settings {
                    c: 0.0,
                    ..Settings::default()
                }
            ),
            Err(AlgoError::Settings(_))
        ));
        assert!(matches!(
            train(
                &data,
                &Settings {
                    stopping_criteria: f64::NAN,
                    ..Settings::default()
                }
            ),
            Err(AlgoError::Settings(_))
        ));
        assert!(matches!(
            train(
                &data,
                &Settings {
                    kernel: Kernel::Polynomial {
                        gamma: 1.0,
                        coef0: 0.0,
                        degree: 0
                    },
                    ..Settings::default()
                }
            ),
            Err(AlgoError::Settings(_))
        ));
        assert!(matches!(
            train(
                &data,
                &Settings {
                    kernel: Kernel::Rbf { gamma: -0.1 },
                    ..Settings::default()
                }
            ),
            Err(AlgoError::Settings(_))
        ));
        if let Ok(m) = train(
            &data,
            &Settings {
                kernel: Kernel::Linear,
                ..Settings::default()
            },
        ) {
            assert!(matches!(m.decision(&[]), Err(AlgoError::Data(_))));
            assert!(matches!(
                m.decision(&[InputValue::Nominal(Some(0))]),
                Err(AlgoError::Data(_))
            ));
            assert!(matches!(
                m.decision(&[InputValue::Numeric(Some(f64::INFINITY))]),
                Err(AlgoError::Data(_))
            ));
        }

        // 11. Exceeding MAX_RECORDS returns GAP:
        let too_many_n = MAX_RECORDS + 1;
        let mut vals = Vec::with_capacity(too_many_n);
        let mut tvals = Vec::with_capacity(too_many_n);
        for i in 0..too_many_n {
            vals.push(Some(i as f64));
            tvals.push(Some(i % 2 == 0));
        }
        let huge_rows = SvmData {
            num_records: too_many_n,
            inputs: vec![InputColumn::Numeric {
                name: "x".into(),
                values: vals,
            }],
            target: Target::Binary {
                name: "y".into(),
                positive_label: "1".into(),
                negative_label: "0".into(),
                values: tvals,
            },
        };
        assert!(matches!(
            train(&huge_rows, &settings),
            Err(AlgoError::Unsupported(msg)) if msg.contains("GAP:")
        ));
    }
}
