//! Bayesian Network learning and posterior inference (IBM SPSS Modeler 19 Algorithms Guide,
//! "Bayesian Networks Algorithms", pp. 41-52): Tree Augmented Naive Bayes (TAN) and Markov Blanket
//! structure/parameter learning, BFS Markov-blanket feature selection, and posterior scoring.
//!
//! The Bayes Net node is licence-blocked in the oracle install, so there is no oracle run. The
//! stream `ibm-demos/bayes_bankloan` and `ibm-demos/bayes_churn_retrain` do embed models Modeler
//! trained earlier, and those are the evidence: rebuilding their training tables with the
//! discretisation each model records and training here gives the same structure and the same
//! tables (to 1e-12) for the TAN models of both streams and for the Markov blanket models of
//! `bayes_bankloan`, with and without feature selection (`tests/bayes_net_oracle.rs`, local
//! corpus). Evidence covers two data sets for TAN and one for Markov blanket; the decisions below
//! say which choices that evidence fixed and which are still open.
//!
//! # Decisions
//!
//! Fixed by the saved Modeler models (where the Guide's text alone gives a different result):
//!
//! A. **TAN edge weight.** The Guide weighs an edge by the conditional mutual information. On
//!    `bayes_bankloan` that picks `ageBins - ed` where Modeler's tree has `incomeBins - ed`. The
//!    significance of the test of `X_i _|_ X_j | Y` (smaller p-value is the stronger edge; equal
//!    p-values by the excess of the statistic over its degrees of freedom) reproduces the trees of
//!    both streams, as does the excess `statistic - df` alone; the two cannot be told apart by
//!    the evidence and this implementation uses the p-value. Pure mutual information stays wrong
//!    on that one edge.
//! B. **Degrees of freedom.** The Guide's `nu = (|X| - 1)(|Y| - 1)|S|` is read with the categories
//!    that occur in each observed stratum of `S`: `nu = sum over observed strata of
//!    (rows - 1)(columns - 1)`. With the full cardinality product the literal `RecognizeMB` drops
//!    `employBins` from the blanket of `default` on `bayes_bankloan` (p = 0.75 against 4e-7);
//!    Modeler keeps it.
//! C. **Small-cell adjustment.** `N^0_ijk = 2 / (r_i q_i)` with `q_i` the number of parent
//!    configurations that occur in the data (all 11 tables of the churn model fit with residual
//!    1e-23; the full product `r_pi |Y|` does not).
//! D. **v-structure order.** Rule 1 visits the non-adjacent pairs `(X_i, X_k)` in index order and,
//!    for each pair, the common neighbours `X_j`; an arc that an earlier v-structure already
//!    oriented is not reversed. Visiting by middle node instead orients `default -> debtincBins`
//!    on `bayes_bankloan`; Modeler has `debtincBins -> default`.
//!
//! Defaults and everything else (not fixed by the evidence):
//!
//! 1. **Defaults (`Settings::default`)**: the declared defaults of the node as the saved streams
//!    record them: `structure_type = Tan`, `use_feature_selection = false`,
//!    `parameter_learning_method = Likelihood`, `independence = Likelihood` (`G^2`),
//!    `significance_level = 0.01`, `maximal_conditioning_set = 5`, `maximum_number_inputs = 10`.
//!    The node's `missing_values` default is pairwise; here it equals complete records unless a
//!    value is missing, in which case it is a `GAP:`.
//! 2. **Prim's starting node and tie-breaking (TAN, Guide p. 46)**: The Guide says "marks a
//!    variable at random" and then roots the directed tree at `X_1`. We deterministically start
//!    Prim's algorithm at the first selected predictor (`X_1`) and break equal-weight ties by
//!    lowest `(marked_var_index, unmarked_var_index)` so training is deterministic.
//! 3. **Superseded**: the mutual information is no longer the TAN weight (decision A).
//! 4. **Zero-parent-count fallback during posterior scoring (Guide pp. 47, 50-51)**: The Guide
//!    sets `theta_ijk = 0` when `N_ij = 0` under MLE. If every target category gets unnormalized
//!    score `0` during `posterior()`, we fall back to the target marginal `theta_Y` (or uniform
//!    `1 / |Y|` if all zeros) so `posterior` always returns a valid distribution summing to `1`.
//! 5. **Missing or unseen evidence during `posterior()` (Guide pp. 51-52)**: The Guide's scoring
//!    equations assume complete cases. When `evidence` has `None` or an out-of-range category code
//!    for a predictor, TAN performs exact sum-product marginalization along the predictor tree
//!    conditioned on `Y`, and Markov Blanket marginalizes unobserved variables over `MB_Y` (falling
//!    back to marginal/observed-factor evaluation if unobserved state space exceeds
//!    `MAX_MB_EXACT_MARGINAL_STATES`); `evidence[target]` is ignored when computing `P(Y | evidence)`.
//! 6. **`RecognizePC` early-break vs `cutSetSize` advance (Guide p. 44)**: The Guide's
//!    `RecognizePC` pseudocode literally writes `if (|NonPC| > 0) { ... cutSetSize += 1; } else break;`,
//!    which stops before larger conditioning sets if no variable is eliminated at the current
//!    `cutSetSize`. We follow the Guide's pseudocode literally.
//! 7. **`RecognizeMB` spouse candidate iteration & missing `S_{Y,Z}` (Guide p. 43)**: Step 3
//!    iterates `X_i in MB` while adding `Z_i` to `MB`; we snapshot `MB` before iterating so newly
//!    added spouses (which have no `CanSP_{Z_i}`) are not re-iterated. If `Z_i` was removed as a
//!    false positive in step 2 rather than in `RecognizePC(Y)`, `S_{Y,Z_i}` falls back to
//!    `S_{Z_i,Y}` or empty set.
//! 8. **Markov Blanket CI test zero-expected cells & degrees of freedom (Guide p. 48)**: When
//!    `N(s_k) = 0`, `N(x_i, s_k) = 0`, or `N(y_j, s_k) = 0`, the expected cell count is `0` and
//!    observed count is `0`; that term contributes `0` to `chi^2` and `G^2` (avoiding `0/0`).
//!    Degrees of freedom: decision B; when `nu == 0`, `p = 1.0` (independent).
//! 9. **Markov Blanket skeleton search symmetry (Guide p. 49)**: For each edge `X_i - X_j` at
//!    conditioning set size `k = 1..=maximal_conditioning_set`, we search subsets of
//!    `ADJ_{X_i} \ {X_j}` and, if still adjacent, `ADJ_{X_j} \ {X_i}` in deterministic index
//!    order, deleting the edge on the smallest separating set found.
//! 10. **Markov Blanket arc orientation rules (Guide pp. 49-50)**:
//!     - Rule 1 (v-structures): decision D.
//!     - Rule 2 (`X_i -> X_j - X_k` -> `X_j -> X_k`) requires `X_i` and `X_k` to be non-adjacent
//!       (Meek R1) to prevent spurious orientations in triangles.
//!     - Rules 2 and 4 (plus cycle-avoidance `X_i -> ... -> X_j` with `X_i - X_j` -> `X_i -> X_j`)
//!       run to a fixed point before Rule 3 orients one remaining undirected edge `X_i - X_j` (in
//!       index order, avoiding cycles) and loops back to step 2 as stated on p. 50.
//! 11. **Markov Blanket parameter estimation for `Y` under `Bayes` (Guide pp. 47, 50-51)**: The
//!     Guide writes `N^0_{ijk} = 2 / (r_i * q_i)` for predictors; we apply the same uninformative
//!     Dirichlet prior `2 / (r_Y * q_Y)` to `Y` given its parents `pi_Y` (which equals `2 / |Y|`
//!     when `pi_Y` is empty, matching TAN p. 47).
//! 12. **`inputs_always_selected` with `use_feature_selection` (Scripting Guide)**: When feature
//!     selection is enabled, any indices in `inputs_always_selected` are guaranteed to remain in
//!     the selected predictor set after `RecognizeMB`.
//! 13. **Explicit `GAP:` errors**:
//!     - `MissingValueHandling::Pairwise` with a missing value: Guide p. 51 only describes "two
//!       fields in question" and does not define 3+ variable CI tests, multi-parent CPT counts, or
//!       normalization across varying valid record counts.
//!     - `maximum_number_inputs` when fewer than the candidate/selected predictors are allowed: the
//!       Guide does not specify how inputs are ranked or pruned to `maximum_number_inputs`.

use crate::AlgoError;
use std::fmt::Write as _;

const MAX_VARIABLES: usize = 512;
const MAX_CATEGORIES: usize = 1_000;
const MAX_RECORDS: usize = 1_000_000;
const MAX_CONDITIONING_SET: usize = 20;
const MAX_CPT_CELLS: usize = 1_000_000;
const MAX_CI_CELLS: usize = 1_000_000;
const MAX_CI_TESTS: usize = 2_000_000;
const MAX_ORIENTATION_STEPS: usize = 100_000;
const MAX_MB_EXACT_MARGINAL_STATES: usize = 4_096;

/// Error returned by Bayesian network training and scoring.
pub type BayesNetError = AlgoError;

fn gap(what: &str) -> BayesNetError {
    AlgoError::Unsupported(format!("GAP: BayesNet {what}"))
}

/// A categorical variable with its name and ordered category labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    /// Variable name.
    pub name: String,
    /// Category labels in code order (`0..categories.len()`).
    pub categories: Vec<String>,
}

/// Pre-categorized input table for Bayesian network training.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoricalData {
    /// Variables in column order (predictors and target).
    pub variables: Vec<Variable>,
    /// Records, each with one `Option<u32>` category code per variable.
    pub rows: Vec<Vec<Option<u32>>>,
    /// Column index of the target variable in `variables`.
    pub target: usize,
}

/// Network structure learning method (`structure_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureType {
    /// Tree Augmented Naive Bayes (TAN).
    Tan,
    /// Markov Blanket structure around the target.
    MarkovBlanket,
}

/// Parameter estimation method (`parameter_learning_method`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterLearningMethod {
    /// Maximum likelihood estimation (`N_ijk / N_ij`).
    Likelihood,
    /// Posterior estimation with uninformative Dirichlet priors (`2 / (r_i * q_i)`).
    Bayes,
}

/// Conditional independence test statistic (`independence`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndependenceTest {
    /// Log likelihood ratio (`G^2`).
    Likelihood,
    /// Pearson chi-square (`chi^2`).
    Pearson,
}

/// Missing-value treatment ("Blank Handling" in the Guide).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingValueHandling {
    /// Exclude records with any missing value across active fields (Guide default).
    CompleteRecordsOnly,
    /// Pairwise exclusion ("Use only complete records" deselected). Identical to complete
    /// records when no value is missing; with missing values it returns `GAP:`.
    Pairwise,
}

/// Settings of a Bayesian Network build node.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Structure type (`structure_type`).
    pub structure_type: StructureType,
    /// Whether to run BFS Markov-blanket feature selection preprocessing (`use_feature_selection`).
    pub use_feature_selection: bool,
    /// CPT estimation method (`parameter_learning_method`).
    pub parameter_learning_method: ParameterLearningMethod,
    /// Conditional independence test (`independence`).
    pub independence: IndependenceTest,
    /// Significance cutoff `alpha` for CI tests (`significance_level`).
    pub significance_level: f64,
    /// Maximum conditioning set size (`maximal_conditioning_set`).
    pub maximal_conditioning_set: usize,
    /// Column indices of predictors always included (`inputs_always_selected`).
    pub inputs_always_selected: Vec<usize>,
    /// Optional cap on the number of predictors (`maximum_number_inputs`).
    pub maximum_number_inputs: Option<usize>,
    /// Missing-value handling (`missing_values`).
    pub missing_values: MissingValueHandling,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            structure_type: StructureType::Tan,
            use_feature_selection: false,
            parameter_learning_method: ParameterLearningMethod::Likelihood,
            independence: IndependenceTest::Likelihood,
            significance_level: 0.01,
            maximal_conditioning_set: 5,
            inputs_always_selected: Vec::new(),
            maximum_number_inputs: None,
            missing_values: MissingValueHandling::CompleteRecordsOnly,
        }
    }
}

/// Conditional probability table for one node given its parent variables.
#[derive(Debug, Clone, PartialEq)]
pub struct ConditionalProbabilityTable {
    /// Column index of this variable in `Network::variables`.
    pub variable: usize,
    /// Parent column indices in `Network::variables` (empty for root).
    pub parents: Vec<usize>,
    /// Cardinality of this variable (`r_i`).
    pub cardinality: usize,
    /// Number of parent configurations (`q_i = product of parent cardinalities`, `1` if no parents).
    pub parent_cardinality: usize,
    /// Row-major table of length `parent_cardinality * cardinality`:
    /// entry `j * cardinality + k` is `P(X_i = k | parents = j)`.
    /// Parent index `j` uses mixed-radix encoding with `parents[0]` most significant.
    pub probabilities: Vec<f64>,
}

/// A trained Bayesian network (TAN or Markov Blanket).
#[derive(Debug, Clone, PartialEq)]
pub struct Network {
    /// Structure type used to build this network.
    pub structure_type: StructureType,
    /// Variables in original column order.
    pub variables: Vec<Variable>,
    /// Target variable column index.
    pub target: usize,
    /// Active variables included in the network (`target` + selected predictors or `MB_Y`).
    pub active_variables: Vec<usize>,
    /// Conditional probability tables, one per node in `active_variables`, in matching order.
    pub cpts: Vec<ConditionalProbabilityTable>,
    /// Number of non-redundant parameters `K`.
    pub parameter_count: usize,
}

impl Network {
    /// Returns the CPT for variable column index `var`, if present in the network.
    pub fn cpt(&self, var: usize) -> Option<&ConditionalProbabilityTable> {
        self.cpts.iter().find(|c| c.variable == var)
    }

    /// Computes `P(target = each category | evidence)`, where `evidence` has one `Option<u32>`
    /// per variable in `self.variables`.
    pub fn posterior(&self, evidence: &[Option<u32>]) -> Result<Vec<f64>, BayesNetError> {
        if evidence.len() != self.variables.len() {
            return Err(BayesNetError::Data(format!(
                "evidence length {} does not match variable count {}",
                evidence.len(),
                self.variables.len()
            )));
        }
        let target_cpt = self
            .cpt(self.target)
            .ok_or_else(|| BayesNetError::Data("network missing target CPT".into()))?;
        let r_y = target_cpt.cardinality;
        if r_y == 0 {
            return Err(BayesNetError::Data("target has zero categories".into()));
        }

        let raw_scores = match self.structure_type {
            StructureType::Tan => self.tan_log_scores(target_cpt, evidence)?,
            StructureType::MarkovBlanket => self.mb_log_scores(target_cpt, evidence)?,
        };

        normalize_log_scores(&raw_scores, &target_cpt.probabilities)
    }

    fn mb_log_scores(
        &self,
        target_cpt: &ConditionalProbabilityTable,
        evidence: &[Option<u32>],
    ) -> Result<Vec<f64>, BayesNetError> {
        let r_y = target_cpt.cardinality;
        // Identify children of Y (CPTs in MB_Y whose parents contain `self.target`).
        let child_cpts: Vec<&ConditionalProbabilityTable> = self
            .cpts
            .iter()
            .filter(|c| c.variable != self.target && c.parents.contains(&self.target))
            .collect();

        // Check if any variable in MB_Y \ {Y} that affects Y or its children is unobserved.
        let mut relevant_unobs: Vec<usize> = Vec::new();
        let mut is_observed = vec![false; self.variables.len()];
        for (idx, var) in self.variables.iter().enumerate() {
            if let Some(Some(code)) = evidence.get(idx).copied()
                && (code as usize) < var.categories.len()
                && let Some(slot) = is_observed.get_mut(idx)
            {
                *slot = true;
            }
        }
        for cpt in &self.cpts {
            if cpt.variable != self.target
                && !is_observed.get(cpt.variable).copied().unwrap_or(false)
            {
                relevant_unobs.push(cpt.variable);
            }
        }

        if relevant_unobs.is_empty() {
            // Complete evidence: Guide p. 51 scoring equation directly.
            let mut log_scores = Vec::with_capacity(r_y);
            for yl in 0..r_y {
                let py = cpt_prob(target_cpt, &self.variables, self.target, yl, yl, evidence);
                if py <= 0.0 {
                    log_scores.push(f64::NEG_INFINITY);
                    continue;
                }
                let mut log_p = py.ln();
                let mut possible = true;
                for &child_cpt in &child_cpts {
                    let xv = evidence
                        .get(child_cpt.variable)
                        .copied()
                        .flatten()
                        .unwrap_or(0) as usize;
                    let pc = cpt_prob(child_cpt, &self.variables, self.target, yl, xv, evidence);
                    if pc <= 0.0 {
                        possible = false;
                        break;
                    }
                    log_p += pc.ln();
                }
                log_scores.push(if possible { log_p } else { f64::NEG_INFINITY });
            }
            return Ok(log_scores);
        }

        // Compute unobserved state space size over MB_Y \ {Y}.
        let mut total_states = 1usize;
        let mut exact_possible = true;
        for &u in &relevant_unobs {
            let ru = self
                .variables
                .get(u)
                .map_or(1, |v| v.categories.len().max(1));
            match total_states.checked_mul(ru) {
                Some(prod) if prod <= MAX_MB_EXACT_MARGINAL_STATES => total_states = prod,
                _ => {
                    exact_possible = false;
                    break;
                }
            }
        }

        if exact_possible {
            // Exact marginalization over unobserved nodes in MB_Y using full MB_Y DAG joint.
            let mut full_evidence = evidence.to_vec();
            let mut log_scores = Vec::with_capacity(r_y);
            let mut assignments = vec![0usize; relevant_unobs.len()];

            for yl in 0..r_y {
                if let Some(slot) = full_evidence.get_mut(self.target) {
                    *slot = Some(yl as u32);
                }
                let mut max_log = f64::NEG_INFINITY;
                let mut term_logs = Vec::with_capacity(total_states);

                assignments.fill(0);
                for _state in 0..total_states {
                    for (pos, &u) in relevant_unobs.iter().enumerate() {
                        if let Some(slot) = full_evidence.get_mut(u) {
                            *slot = Some(assignments.get(pos).copied().unwrap_or(0) as u32);
                        }
                    }
                    let mut log_joint = 0.0f64;
                    let mut nonzero = true;
                    for cpt in &self.cpts {
                        let val = full_evidence
                            .get(cpt.variable)
                            .copied()
                            .flatten()
                            .unwrap_or(0) as usize;
                        let p =
                            cpt_prob(cpt, &self.variables, self.target, yl, val, &full_evidence);
                        if p <= 0.0 {
                            nonzero = false;
                            break;
                        }
                        log_joint += p.ln();
                    }
                    if nonzero {
                        if log_joint > max_log {
                            max_log = log_joint;
                        }
                        term_logs.push(log_joint);
                    }
                    // Advance mixed-radix odometer.
                    for pos in (0..relevant_unobs.len()).rev() {
                        let u = relevant_unobs.get(pos).copied().unwrap_or(0);
                        let ru = self.variables.get(u).map_or(1, |v| v.categories.len());
                        if let Some(slot) = assignments.get_mut(pos) {
                            *slot += 1;
                            if *slot < ru {
                                break;
                            }
                            *slot = 0;
                        }
                    }
                }
                if max_log.is_finite() {
                    let sum_exp: f64 = term_logs.iter().map(|&l| (l - max_log).exp()).sum();
                    log_scores.push(max_log + sum_exp.ln());
                } else {
                    log_scores.push(f64::NEG_INFINITY);
                }
            }
            return Ok(log_scores);
        }

        // Large missing-evidence fallback: average uniformly over missing parent configs of Y
        // and include only fully-observed child factors (marginalizing missing children to 1).
        let mut log_scores = Vec::with_capacity(r_y);
        for yl in 0..r_y {
            let py =
                cpt_prob_avg_missing(target_cpt, &self.variables, self.target, yl, yl, evidence);
            if py <= 0.0 {
                log_scores.push(f64::NEG_INFINITY);
                continue;
            }
            let mut log_p = py.ln();
            let mut possible = true;
            for &child_cpt in &child_cpts {
                if let Some(Some(xv)) = evidence.get(child_cpt.variable).copied()
                    && (xv as usize) < child_cpt.cardinality
                {
                    let pc = cpt_prob_avg_missing(
                        child_cpt,
                        &self.variables,
                        self.target,
                        yl,
                        xv as usize,
                        evidence,
                    );
                    if pc <= 0.0 {
                        possible = false;
                        break;
                    }
                    log_p += pc.ln();
                }
            }
            log_scores.push(if possible { log_p } else { f64::NEG_INFINITY });
        }
        Ok(log_scores)
    }

    fn tan_log_scores(
        &self,
        target_cpt: &ConditionalProbabilityTable,
        evidence: &[Option<u32>],
    ) -> Result<Vec<f64>, BayesNetError> {
        let r_y = target_cpt.cardinality;
        let pred_cpts: Vec<&ConditionalProbabilityTable> = self
            .cpts
            .iter()
            .filter(|c| c.variable != self.target)
            .collect();
        let m = pred_cpts.len();

        // Map variable index -> position in `pred_cpts`.
        let mut var_to_pos = vec![None; self.variables.len()];
        for (pos, cpt) in pred_cpts.iter().enumerate() {
            if let Some(slot) = var_to_pos.get_mut(cpt.variable) {
                *slot = Some(pos);
            }
        }

        let mut parent_pos: Vec<Option<usize>> = vec![None; m];
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); m];
        for (pos, cpt) in pred_cpts.iter().enumerate() {
            if cpt.parents.len() == 2 {
                let p_var = cpt.parents.first().copied().unwrap_or(0);
                if let Some(Some(p)) = var_to_pos.get(p_var).copied() {
                    if let Some(slot) = parent_pos.get_mut(pos) {
                        *slot = Some(p);
                    }
                    if let Some(ch) = children.get_mut(p) {
                        ch.push(pos);
                    }
                }
            }
        }

        // BFS topological order from roots.
        let mut order = Vec::with_capacity(m);
        let mut queue = std::collections::VecDeque::new();
        for (pos, p) in parent_pos.iter().enumerate() {
            if p.is_none() {
                queue.push_back(pos);
            }
        }
        while let Some(u) = queue.pop_front() {
            order.push(u);
            if let Some(ch) = children.get(u) {
                for &v in ch {
                    queue.push_back(v);
                }
            }
        }

        let mut log_scores = Vec::with_capacity(r_y);
        for yl in 0..r_y {
            let p_y = target_cpt.probabilities.get(yl).copied().unwrap_or(0.0);
            if p_y <= 0.0 {
                log_scores.push(f64::NEG_INFINITY);
                continue;
            }
            let mut log_joint = p_y.ln();
            let mut lambdas: Vec<Vec<f64>> = pred_cpts
                .iter()
                .map(|cpt| {
                    let r = cpt.cardinality;
                    match evidence.get(cpt.variable).copied().flatten() {
                        Some(code) if (code as usize) < r => {
                            let mut v = vec![0.0; r];
                            if let Some(slot) = v.get_mut(code as usize) {
                                *slot = 1.0;
                            }
                            v
                        }
                        _ => vec![1.0; r],
                    }
                })
                .collect();

            // Bottom-up sum-product over the predictor tree.
            for &pos in order.iter().rev() {
                let Some(cpt) = pred_cpts.get(pos) else {
                    continue;
                };
                let r = cpt.cardinality;
                let mut cur_lambda = lambdas.get(pos).cloned().unwrap_or_default();

                // Rescale to prevent underflow.
                let max_val = cur_lambda.iter().copied().fold(0.0f64, f64::max);
                if max_val <= 0.0 {
                    log_joint = f64::NEG_INFINITY;
                    break;
                }
                if max_val != 1.0 {
                    for x in &mut cur_lambda {
                        *x /= max_val;
                    }
                    log_joint += max_val.ln();
                }

                if let Some(p_idx) = parent_pos.get(pos).copied().flatten() {
                    let Some(parent_cpt) = pred_cpts.get(p_idx) else {
                        continue;
                    };
                    let rp = parent_cpt.cardinality;
                    let mut msg = vec![0.0f64; rp];
                    for (u, msg_u) in msg.iter_mut().enumerate() {
                        let cfg = u * r_y + yl;
                        let base = cfg * r;
                        let mut sum = 0.0;
                        for (v, &lam_v) in cur_lambda.iter().enumerate() {
                            if lam_v > 0.0 {
                                let prob = cpt.probabilities.get(base + v).copied().unwrap_or(0.0);
                                sum += prob * lam_v;
                            }
                        }
                        *msg_u = sum;
                    }
                    if let Some(p_lam) = lambdas.get_mut(p_idx) {
                        for (dst, mval) in p_lam.iter_mut().zip(msg.iter()) {
                            *dst *= *mval;
                        }
                    }
                } else {
                    // Root predictor (parents = [target]).
                    let base = yl * r;
                    let mut root_sum = 0.0;
                    for (u, &lam_u) in cur_lambda.iter().enumerate() {
                        if lam_u > 0.0 {
                            let prob = cpt.probabilities.get(base + u).copied().unwrap_or(0.0);
                            root_sum += prob * lam_u;
                        }
                    }
                    if root_sum <= 0.0 {
                        log_joint = f64::NEG_INFINITY;
                        break;
                    }
                    log_joint += root_sum.ln();
                }
            }
            log_scores.push(log_joint);
        }
        Ok(log_scores)
    }

    /// Plain-text representation of the structure (parents per variable) and every CPT.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let kind = match self.structure_type {
            StructureType::Tan => "TAN",
            StructureType::MarkovBlanket => "MarkovBlanket",
        };
        let target_name = self
            .variables
            .get(self.target)
            .map_or("?", |v| v.name.as_str());
        let _ = writeln!(
            out,
            "BayesianNetwork(type={kind}, target={target_name}, K={})",
            self.parameter_count
        );
        for cpt in &self.cpts {
            let var_name = self
                .variables
                .get(cpt.variable)
                .map_or("?", |v| v.name.as_str());
            let parent_names: Vec<&str> = cpt
                .parents
                .iter()
                .map(|&p| self.variables.get(p).map_or("?", |v| v.name.as_str()))
                .collect();
            let _ = writeln!(
                out,
                "Node {var_name} | parents=[{}]",
                parent_names.join(", ")
            );
            for j in 0..cpt.parent_cardinality {
                let start = j * cpt.cardinality;
                let end = start + cpt.cardinality;
                let row = cpt.probabilities.get(start..end).unwrap_or(&[]);
                let vals: Vec<String> = row.iter().map(|p| format!("{p:.6}")).collect();
                let _ = writeln!(out, "  parent_cfg[{j}]: [{}]", vals.join(", "));
            }
        }
        out
    }
}

fn cpt_prob(
    cpt: &ConditionalProbabilityTable,
    variables: &[Variable],
    target: usize,
    yl: usize,
    val: usize,
    evidence: &[Option<u32>],
) -> f64 {
    let mut j = 0usize;
    for &p in &cpt.parents {
        let rp = variables.get(p).map_or(1, |v| v.categories.len().max(1));
        let vp = if p == target {
            yl
        } else {
            evidence.get(p).copied().flatten().unwrap_or(0) as usize
        };
        if vp >= rp {
            return 0.0;
        }
        j = j * rp + vp;
    }
    cpt.probabilities
        .get(j * cpt.cardinality + val)
        .copied()
        .unwrap_or(0.0)
}

fn cpt_prob_avg_missing(
    cpt: &ConditionalProbabilityTable,
    variables: &[Variable],
    target: usize,
    yl: usize,
    val: usize,
    evidence: &[Option<u32>],
) -> f64 {
    let mut indices = vec![0usize];
    for &p in &cpt.parents {
        let rp = variables.get(p).map_or(1, |v| v.categories.len().max(1));
        if p == target {
            for idx in &mut indices {
                *idx = *idx * rp + yl;
            }
        } else if let Some(Some(vp)) = evidence.get(p).copied()
            && (vp as usize) < rp
        {
            for idx in &mut indices {
                *idx = *idx * rp + (vp as usize);
            }
        } else {
            let mut next = Vec::with_capacity(
                indices
                    .len()
                    .saturating_mul(rp)
                    .min(MAX_MB_EXACT_MARGINAL_STATES),
            );
            for &idx in &indices {
                for v in 0..rp {
                    if next.len() >= MAX_MB_EXACT_MARGINAL_STATES {
                        break;
                    }
                    next.push(idx * rp + v);
                }
            }
            indices = next;
        }
    }
    if indices.is_empty() {
        return 0.0;
    }
    let sum: f64 = indices
        .iter()
        .map(|&j| {
            cpt.probabilities
                .get(j * cpt.cardinality + val)
                .copied()
                .unwrap_or(0.0)
        })
        .sum();
    sum / (indices.len() as f64)
}

fn normalize_log_scores(log_scores: &[f64], prior: &[f64]) -> Result<Vec<f64>, BayesNetError> {
    let k = log_scores.len();
    if k == 0 {
        return Err(BayesNetError::Data("target has zero categories".into()));
    }
    let max_log = log_scores
        .iter()
        .copied()
        .filter(|x| x.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);
    if max_log.is_finite() {
        let mut probs: Vec<f64> = log_scores
            .iter()
            .map(|&l| {
                if l.is_finite() {
                    (l - max_log).exp()
                } else {
                    0.0
                }
            })
            .collect();
        let sum: f64 = probs.iter().sum();
        if sum > 0.0 && sum.is_finite() {
            for p in &mut probs {
                *p /= sum;
            }
            return Ok(probs);
        }
    }
    // Zero-score fallback: target marginal prior, else uniform.
    let prior_sum: f64 = prior
        .iter()
        .copied()
        .filter(|x| *x > 0.0 && x.is_finite())
        .sum();
    if prior.len() == k && prior_sum > 0.0 {
        return Ok(prior.iter().map(|&p| p.max(0.0) / prior_sum).collect());
    }
    Ok(vec![1.0 / (k as f64); k])
}

/// Validated, dense complete-case matrix of category indices.
struct CleanData {
    cardinalities: Vec<usize>,
    /// Row-major `n_rows * n_vars` category codes (`usize`).
    records: Vec<usize>,
    n_rows: usize,
    n_vars: usize,
}

impl CleanData {
    #[inline]
    fn get(&self, row: usize, col: usize) -> Option<usize> {
        self.records.get(row * self.n_vars + col).copied()
    }
}

/// Trains a Bayesian network (TAN or Markov Blanket) on `data` according to `settings`.
pub fn train(data: &CategoricalData, settings: &Settings) -> Result<Network, BayesNetError> {
    crate::check_cancelled()?;
    validate_inputs(data, settings)?;
    let clean = extract_complete_records(data)?;
    let mut test_counter = 0usize;
    let mut selected_predictors: Vec<usize> = if settings.use_feature_selection {
        let mut mb = recognize_mb(
            &clean,
            data.target,
            settings.independence,
            settings.significance_level,
            settings.maximal_conditioning_set,
            &mut test_counter,
        )?;
        for &forced in &settings.inputs_always_selected {
            if !mb.contains(&forced) {
                mb.push(forced);
            }
        }
        mb
    } else {
        (0..data.variables.len())
            .filter(|&i| i != data.target)
            .collect()
    };

    if let Some(max_inputs) = settings.maximum_number_inputs
        && selected_predictors.len() > max_inputs
    {
        return Err(gap(&format!(
            "maximum_number_inputs ({max_inputs}) is smaller than the {} selected predictors, \
             and the Algorithms Guide does not specify how inputs are ranked or pruned",
            selected_predictors.len()
        )));
    }
    selected_predictors.sort_unstable();
    selected_predictors.dedup();

    match settings.structure_type {
        StructureType::Tan => train_tan(data, &clean, &selected_predictors, settings),
        StructureType::MarkovBlanket => train_markov_blanket(
            data,
            &clean,
            &selected_predictors,
            settings,
            &mut test_counter,
        ),
    }
}

fn validate_inputs(data: &CategoricalData, settings: &Settings) -> Result<(), BayesNetError> {
    let n_vars = data.variables.len();
    if !(2..=MAX_VARIABLES).contains(&n_vars) {
        return Err(BayesNetError::Data(format!(
            "Bayesian network requires 2..={MAX_VARIABLES} variables, got {n_vars}"
        )));
    }
    if data.target >= n_vars {
        return Err(BayesNetError::Data(format!(
            "target index {} out of range for {n_vars} variables",
            data.target
        )));
    }
    for (idx, var) in data.variables.iter().enumerate() {
        let cats = var.categories.len();
        if cats == 0 || cats > MAX_CATEGORIES {
            return Err(BayesNetError::Data(format!(
                "variable {} ({}) must have 1..={MAX_CATEGORIES} categories, got {cats}",
                idx, var.name
            )));
        }
    }
    if data.rows.is_empty() || data.rows.len() > MAX_RECORDS {
        return Err(BayesNetError::Data(format!(
            "Bayesian network requires 1..={MAX_RECORDS} rows, got {}",
            data.rows.len()
        )));
    }
    if !settings.significance_level.is_finite()
        || settings.significance_level <= 0.0
        || settings.significance_level >= 1.0
    {
        return Err(BayesNetError::Settings(format!(
            "significance_level must be strictly between 0 and 1, got {}",
            settings.significance_level
        )));
    }
    if settings.maximal_conditioning_set > MAX_CONDITIONING_SET {
        return Err(BayesNetError::Settings(format!(
            "maximal_conditioning_set ({}) exceeds limit {MAX_CONDITIONING_SET}",
            settings.maximal_conditioning_set
        )));
    }
    for &idx in &settings.inputs_always_selected {
        if idx >= n_vars || idx == data.target {
            return Err(BayesNetError::Settings(format!(
                "inputs_always_selected contains invalid predictor index {idx}"
            )));
        }
    }
    if settings.missing_values == MissingValueHandling::Pairwise
        && data.rows.iter().any(|row| row.iter().any(Option::is_none))
    {
        return Err(gap(
            "pairwise missing-value handling ('Use only complete records' deselected) is not \
             defined in the Algorithms Guide for 3+ variable tests or CPT estimation",
        ));
    }
    Ok(())
}

fn extract_complete_records(data: &CategoricalData) -> Result<CleanData, BayesNetError> {
    let n_vars = data.variables.len();
    let cardinalities: Vec<usize> = data.variables.iter().map(|v| v.categories.len()).collect();
    let mut records = Vec::new();
    let mut n_rows = 0usize;

    for (row_idx, row) in data.rows.iter().enumerate() {
        if row.len() != n_vars {
            return Err(BayesNetError::Data(format!(
                "row {row_idx} has {} columns, expected {n_vars}",
                row.len()
            )));
        }
        let mut complete = true;
        for (col_idx, cell) in row.iter().enumerate() {
            match cell {
                None => {
                    complete = false;
                }
                Some(code) => {
                    let c = *code as usize;
                    let card = cardinalities.get(col_idx).copied().unwrap_or(0);
                    if c >= card {
                        return Err(BayesNetError::Data(format!(
                            "row {row_idx} col {col_idx} has category code {c} >= cardinality {card}"
                        )));
                    }
                }
            }
        }
        if complete {
            for code in row.iter().flatten() {
                records.push(*code as usize);
            }
            n_rows += 1;
        }
    }

    if n_rows == 0 {
        return Err(BayesNetError::Data(
            "no complete records remain after missing-value exclusion".into(),
        ));
    }

    Ok(CleanData {
        cardinalities,
        records,
        n_rows,
        n_vars,
    })
}

/// Weight of a candidate tree edge: the significance of the test of `X_i _|_ X_j | Y`.
///
/// The stronger edge is the one with the smaller p-value; edges whose p-values are equal (all
/// underflow to 0 for large samples) are ordered by the excess of the statistic over its degrees
/// of freedom.
#[derive(Debug, Clone, Copy, Default)]
struct TanWeight {
    p_value: f64,
    excess: f64,
}

impl TanWeight {
    fn new(stat: f64, df: f64) -> Self {
        let p_value = if df > 0.0 && df.is_finite() {
            crate::stats::chi_square_p(stat.max(0.0), df)
        } else {
            1.0
        };
        Self {
            p_value,
            excess: stat - df,
        }
    }

    fn stronger_than(&self, other: &Self) -> bool {
        if self.p_value < other.p_value {
            return true;
        }
        self.p_value <= other.p_value && self.excess > other.excess
    }
}

fn train_tan(
    data: &CategoricalData,
    clean: &CleanData,
    predictors: &[usize],
    settings: &Settings,
) -> Result<Network, BayesNetError> {
    let m = predictors.len();
    // TAN parent besides Y for each predictor index in `0..m`: `Option<usize>` (index in `predictors`).
    let mut pred_parent: Vec<Option<usize>> = vec![None; m];

    if m >= 2 {
        // Edge weight of every predictor pair: the conditional test of X_i, X_j given Y.
        let mut weights = vec![TanWeight::default(); m * m];
        let mut test_counter = 0usize;
        for i in 0..m {
            crate::check_cancelled()?;
            let vi = predictors.get(i).copied().unwrap_or(0);
            for j in (i + 1)..m {
                let vj = predictors.get(j).copied().unwrap_or(0);
                let (stat, df) = ci_statistic(
                    clean,
                    vi,
                    vj,
                    &[data.target],
                    settings.independence,
                    &mut test_counter,
                )?;
                let w = TanWeight::new(stat, df);
                if let Some(slot) = weights.get_mut(i * m + j) {
                    *slot = w;
                }
                if let Some(slot) = weights.get_mut(j * m + i) {
                    *slot = w;
                }
            }
        }

        // Prim's MWST starting deterministically at index 0 (`X_1`), building undirected adjacency.
        let mut marked = vec![false; m];
        let mut tree_adj: Vec<Vec<usize>> = vec![Vec::new(); m];
        if let Some(first) = marked.get_mut(0) {
            *first = true;
        }
        for _step in 1..m {
            let mut best: Option<(TanWeight, usize, usize)> = None;
            for u in 0..m {
                if !marked.get(u).copied().unwrap_or(false) {
                    continue;
                }
                for v in 0..m {
                    if marked.get(v).copied().unwrap_or(true) {
                        continue;
                    }
                    let w = weights.get(u * m + v).copied().unwrap_or_default();
                    let replace = match best {
                        None => true,
                        Some((bw, _, _)) => w.stronger_than(&bw),
                    };
                    if replace {
                        best = Some((w, u, v));
                    }
                }
            }
            if let Some((_, u, v)) = best {
                if let Some(slot) = marked.get_mut(v) {
                    *slot = true;
                }
                if let Some(adj_u) = tree_adj.get_mut(u) {
                    adj_u.push(v);
                }
                if let Some(adj_v) = tree_adj.get_mut(v) {
                    adj_v.push(u);
                }
            }
        }

        // Orient outward from root `X_1` (index 0) via BFS.
        let mut visited = vec![false; m];
        let mut queue = std::collections::VecDeque::new();
        if let Some(v0) = visited.get_mut(0) {
            *v0 = true;
        }
        queue.push_back(0usize);
        while let Some(u) = queue.pop_front() {
            let neighbors = tree_adj.get(u).cloned().unwrap_or_default();
            for v in neighbors {
                if !visited.get(v).copied().unwrap_or(true) {
                    if let Some(vis) = visited.get_mut(v) {
                        *vis = true;
                    }
                    if let Some(par) = pred_parent.get_mut(v) {
                        *par = Some(u);
                    }
                    queue.push_back(v);
                }
            }
        }
    }

    // Build CPTs: target first, then predictors in order.
    let mut active_variables = Vec::with_capacity(1 + m);
    active_variables.push(data.target);
    active_variables.extend_from_slice(predictors);

    let mut cpts = Vec::with_capacity(1 + m);
    let target_cpt = estimate_cpt(clean, data.target, &[], settings.parameter_learning_method)?;
    let mut parameter_count = target_cpt.cardinality.saturating_sub(1);
    cpts.push(target_cpt);

    for (pos, &var) in predictors.iter().enumerate() {
        let mut parents = Vec::with_capacity(2);
        if let Some(Some(parent_pos)) = pred_parent.get(pos)
            && let Some(&parent_var) = predictors.get(*parent_pos)
        {
            parents.push(parent_var);
        }
        parents.push(data.target);
        let cpt = estimate_cpt(clean, var, &parents, settings.parameter_learning_method)?;
        parameter_count += cpt.cardinality.saturating_sub(1) * cpt.parent_cardinality;
        cpts.push(cpt);
    }

    Ok(Network {
        structure_type: StructureType::Tan,
        variables: data.variables.clone(),
        target: data.target,
        active_variables,
        cpts,
        parameter_count,
    })
}

fn estimate_cpt(
    clean: &CleanData,
    variable: usize,
    parents: &[usize],
    method: ParameterLearningMethod,
) -> Result<ConditionalProbabilityTable, BayesNetError> {
    let r = clean.cardinalities.get(variable).copied().unwrap_or(0);
    if r == 0 {
        return Err(BayesNetError::Data("variable has zero categories".into()));
    }
    let mut q = 1usize;
    for &p in parents {
        let rp = clean.cardinalities.get(p).copied().unwrap_or(0);
        q = q.checked_mul(rp).ok_or_else(|| {
            BayesNetError::Data("parent configuration cardinality overflow".into())
        })?;
    }
    let total_cells = q
        .checked_mul(r)
        .ok_or_else(|| BayesNetError::Data("CPT cell count overflow".into()))?;
    if total_cells > MAX_CPT_CELLS {
        return Err(BayesNetError::Data(format!(
            "CPT cell count {total_cells} exceeds limit {MAX_CPT_CELLS}"
        )));
    }

    let mut counts_ij = vec![0usize; q];
    let mut counts_ijk = vec![0usize; total_cells];

    for row in 0..clean.n_rows {
        let mut j = 0usize;
        for &p in parents {
            let rp = clean.cardinalities.get(p).copied().unwrap_or(1);
            let vp = clean.get(row, p).unwrap_or(0);
            j = j * rp + vp;
        }
        let k = clean.get(row, variable).unwrap_or(0);
        if let Some(slot) = counts_ij.get_mut(j) {
            *slot += 1;
        }
        if let Some(slot) = counts_ijk.get_mut(j * r + k) {
            *slot += 1;
        }
    }

    let mut probabilities = vec![0.0f64; total_cells];
    match method {
        ParameterLearningMethod::Likelihood => {
            for j in 0..q {
                let nij = counts_ij.get(j).copied().unwrap_or(0);
                if nij == 0 {
                    // Guide pp. 47, 50: "Note that if N_ij = 0, then theta_ijk = 0."
                    continue;
                }
                let denom = nij as f64;
                for k in 0..r {
                    let nijk = counts_ijk.get(j * r + k).copied().unwrap_or(0);
                    if let Some(slot) = probabilities.get_mut(j * r + k) {
                        *slot = (nijk as f64) / denom;
                    }
                }
            }
        }
        ParameterLearningMethod::Bayes => {
            // Guide pp. 47, 51: N^0_ijk = 2 / (r_i * q_i), so N^0_ij = 2 / q_i, where q_i counts
            // the parent configurations that occur in the data.
            let observed = counts_ij.iter().filter(|&&c| c > 0).count().max(1);
            let prior_cell = 2.0 / ((r as f64) * (observed as f64));
            let prior_row = 2.0 / (observed as f64);
            for j in 0..q {
                let nij = counts_ij.get(j).copied().unwrap_or(0) as f64;
                let denom = nij + prior_row;
                for k in 0..r {
                    let nijk = counts_ijk.get(j * r + k).copied().unwrap_or(0) as f64;
                    if let Some(slot) = probabilities.get_mut(j * r + k) {
                        *slot = (nijk + prior_cell) / denom;
                    }
                }
            }
        }
    }

    Ok(ConditionalProbabilityTable {
        variable,
        parents: parents.to_vec(),
        cardinality: r,
        parent_cardinality: q,
        probabilities,
    })
}

/// Conditional independence test returning p-value `I(X, Y | cond)` (Guide pp. 48-49).
fn ci_test(
    clean: &CleanData,
    x: usize,
    y: usize,
    cond: &[usize],
    method: IndependenceTest,
    test_counter: &mut usize,
) -> Result<f64, BayesNetError> {
    let (stat, df) = ci_statistic(clean, x, y, cond, method, test_counter)?;
    if df <= 0.0 || !df.is_finite() {
        return Ok(1.0);
    }
    Ok(crate::stats::chi_square_p(stat.max(0.0), df))
}

/// The test statistic (`chi^2` or `G^2`) of `X _|_ Y | cond` and its degrees of freedom, summed
/// over the observed strata of `cond` (see the decision list: the Guide's `|S|` is the number of
/// observed configurations).
fn ci_statistic(
    clean: &CleanData,
    x: usize,
    y: usize,
    cond: &[usize],
    method: IndependenceTest,
    test_counter: &mut usize,
) -> Result<(f64, f64), BayesNetError> {
    *test_counter += 1;
    if *test_counter > MAX_CI_TESTS {
        return Err(BayesNetError::Data(format!(
            "CI test count exceeded limit {MAX_CI_TESTS}"
        )));
    }
    if (*test_counter).is_multiple_of(1024) {
        crate::check_cancelled()?;
    }

    let rx = clean.cardinalities.get(x).copied().unwrap_or(0);
    let ry = clean.cardinalities.get(y).copied().unwrap_or(0);
    if rx <= 1 || ry <= 1 {
        return Ok((0.0, 0.0));
    }

    let mut s_card_u128 = 1u128;
    for &z in cond {
        let rz = clean.cardinalities.get(z).copied().unwrap_or(0) as u128;
        s_card_u128 = s_card_u128
            .checked_mul(rz)
            .ok_or_else(|| BayesNetError::Data("conditioning set cardinality overflow".into()))?;
    }
    let xy_cells = rx
        .checked_mul(ry)
        .ok_or_else(|| BayesNetError::Data("CI contingency table overflow".into()))?;
    if xy_cells > MAX_CI_CELLS {
        return Err(BayesNetError::Data(format!(
            "CI table size {xy_cells} exceeds limit {MAX_CI_CELLS}"
        )));
    }

    let mut stat = 0.0f64;
    let mut df = 0.0f64;

    if s_card_u128 <= 4_096 && (s_card_u128 as usize).saturating_mul(xy_cells) <= 65_536 {
        let s_card = s_card_u128 as usize;
        let mut counts_xys = vec![0usize; s_card * xy_cells];
        let mut counts_xs = vec![0usize; s_card * rx];
        let mut counts_ys = vec![0usize; s_card * ry];
        let mut counts_s = vec![0usize; s_card];

        for row in 0..clean.n_rows {
            let mut s_idx = 0usize;
            for &z in cond {
                let rz = clean.cardinalities.get(z).copied().unwrap_or(1);
                let vz = clean.get(row, z).unwrap_or(0);
                s_idx = s_idx * rz + vz;
            }
            let vx = clean.get(row, x).unwrap_or(0);
            let vy = clean.get(row, y).unwrap_or(0);
            if let Some(slot) = counts_xys.get_mut(s_idx * xy_cells + vx * ry + vy) {
                *slot += 1;
            }
            if let Some(slot) = counts_xs.get_mut(s_idx * rx + vx) {
                *slot += 1;
            }
            if let Some(slot) = counts_ys.get_mut(s_idx * ry + vy) {
                *slot += 1;
            }
            if let Some(slot) = counts_s.get_mut(s_idx) {
                *slot += 1;
            }
        }

        for s_idx in 0..s_card {
            let ns = counts_s.get(s_idx).copied().unwrap_or(0);
            if ns == 0 {
                continue;
            }
            let xys = counts_xys
                .get(s_idx * xy_cells..(s_idx + 1) * xy_cells)
                .unwrap_or(&[]);
            let xs = counts_xs.get(s_idx * rx..(s_idx + 1) * rx).unwrap_or(&[]);
            let ys = counts_ys.get(s_idx * ry..(s_idx + 1) * ry).unwrap_or(&[]);
            let (st, d) = stratum_stat(xys, xs, ys, ns, rx, ry, method);
            stat += st;
            df += d;
        }
    } else {
        // Sparse stratum grouping for large conditioning sets.
        let mut row_keys: Vec<(Vec<usize>, usize, usize)> = Vec::with_capacity(clean.n_rows);
        for row in 0..clean.n_rows {
            let key: Vec<usize> = cond
                .iter()
                .map(|&z| clean.get(row, z).unwrap_or(0))
                .collect();
            let vx = clean.get(row, x).unwrap_or(0);
            let vy = clean.get(row, y).unwrap_or(0);
            row_keys.push((key, vx, vy));
        }
        row_keys.sort_unstable_by(|a, b| a.0.cmp(&b.0));

        let mut xys = vec![0usize; xy_cells];
        let mut xs = vec![0usize; rx];
        let mut ys = vec![0usize; ry];
        let mut idx = 0usize;
        while idx < row_keys.len() {
            let key = &row_keys[idx].0;
            let mut end = idx + 1;
            while end < row_keys.len() && &row_keys[end].0 == key {
                end += 1;
            }
            xys.fill(0);
            xs.fill(0);
            ys.fill(0);
            let ns = end - idx;
            for item in &row_keys[idx..end] {
                let vx = item.1;
                let vy = item.2;
                if let Some(slot) = xys.get_mut(vx * ry + vy) {
                    *slot += 1;
                }
                if let Some(slot) = xs.get_mut(vx) {
                    *slot += 1;
                }
                if let Some(slot) = ys.get_mut(vy) {
                    *slot += 1;
                }
            }
            let (st, d) = stratum_stat(&xys, &xs, &ys, ns, rx, ry, method);
            stat += st;
            df += d;
            idx = end;
        }
    }

    Ok((stat, df))
}

fn stratum_stat(
    xys: &[usize],
    xs: &[usize],
    ys: &[usize],
    ns: usize,
    rx: usize,
    ry: usize,
    method: IndependenceTest,
) -> (f64, f64) {
    let ns_f = ns as f64;
    let mut stat = 0.0f64;
    match method {
        IndependenceTest::Pearson => {
            for vx in 0..rx {
                let nxs = xs.get(vx).copied().unwrap_or(0);
                if nxs == 0 {
                    continue;
                }
                for vy in 0..ry {
                    let nys = ys.get(vy).copied().unwrap_or(0);
                    if nys == 0 {
                        continue;
                    }
                    let nxys = xys.get(vx * ry + vy).copied().unwrap_or(0) as f64;
                    let expected = (nxs as f64) * (nys as f64) / ns_f;
                    let diff = nxys - expected;
                    stat += (diff * diff) / expected;
                }
            }
        }
        IndependenceTest::Likelihood => {
            for vx in 0..rx {
                let nxs = xs.get(vx).copied().unwrap_or(0);
                if nxs == 0 {
                    continue;
                }
                for vy in 0..ry {
                    let nxys = xys.get(vx * ry + vy).copied().unwrap_or(0);
                    if nxys == 0 {
                        continue;
                    }
                    let nys = ys.get(vy).copied().unwrap_or(0);
                    if nys == 0 {
                        continue;
                    }
                    let ratio = ((nxys as f64) * ns_f) / ((nxs as f64) * (nys as f64));
                    if ratio > 0.0 {
                        stat += 2.0 * (nxys as f64) * ratio.ln();
                    }
                }
            }
        }
    }
    // Degrees of freedom of the observed table: categories that occur in the stratum.
    let rows = xs.iter().filter(|&&c| c > 0).count();
    let cols = ys.iter().filter(|&&c| c > 0).count();
    let df = (rows.saturating_sub(1) * cols.saturating_sub(1)) as f64;
    (stat, df)
}

/// Calls `f` for every `k`-element subset of `pool` in lexicographical order until `f` returns
/// `Ok(true)` (early stop) or all subsets are visited.
fn for_each_subset<F>(pool: &[usize], k: usize, mut f: F) -> Result<bool, BayesNetError>
where
    F: FnMut(&[usize]) -> Result<bool, BayesNetError>,
{
    let n = pool.len();
    if k > n {
        return Ok(false);
    }
    if k == 0 {
        return f(&[]);
    }
    let mut indices: Vec<usize> = (0..k).collect();
    let mut subset = vec![0usize; k];
    loop {
        for (dst, &idx) in subset.iter_mut().zip(indices.iter()) {
            *dst = pool.get(idx).copied().unwrap_or(0);
        }
        if f(&subset)? {
            return Ok(true);
        }
        // Advance rightmost incrementable index.
        let mut pos = k;
        while pos > 0 {
            pos -= 1;
            let cur = indices.get(pos).copied().unwrap_or(0);
            if cur < n - (k - pos) {
                let next = cur + 1;
                for offset in 0..(k - pos) {
                    if let Some(slot) = indices.get_mut(pos + offset) {
                        *slot = next + offset;
                    }
                }
                break;
            }
            if pos == 0 {
                return Ok(false);
            }
        }
    }
}

struct PcResult {
    adj: Vec<usize>,
    sep_sets: Vec<Option<Vec<usize>>>,
}

/// BFS `RecognizePC` (Guide p. 44). Returns `PC(target)` and `sep_sets` where `sep_sets[v]` stores
/// the separating set `S_{target, v}` if `v` was eliminated.
fn recognize_pc(
    clean: &CleanData,
    target: usize,
    cand_adj: Vec<usize>,
    independence: IndependenceTest,
    alpha: f64,
    max_set_size: usize,
    test_counter: &mut usize,
) -> Result<PcResult, BayesNetError> {
    let mut adj = cand_adj;
    let mut sep_sets: Vec<Option<Vec<usize>>> = vec![None; clean.n_vars];
    let mut cut_size = 0usize;

    loop {
        let mut non_pc = Vec::new();
        for &x_i in &adj {
            let others: Vec<usize> = adj.iter().copied().filter(|&v| v != x_i).collect();
            if others.len() < cut_size {
                continue;
            }
            let mut found_sep: Option<Vec<usize>> = None;
            let _ = for_each_subset(&others, cut_size, |s| {
                let p = ci_test(clean, x_i, target, s, independence, test_counter)?;
                if p > alpha {
                    found_sep = Some(s.to_vec());
                    return Ok(true);
                }
                Ok(false)
            })?;
            if let Some(sep) = found_sep {
                non_pc.push(x_i);
                if let Some(slot) = sep_sets.get_mut(x_i) {
                    *slot = Some(sep);
                }
            }
        }

        if !non_pc.is_empty() {
            adj.retain(|v| !non_pc.contains(v));
            cut_size += 1;
        } else {
            break;
        }
        if adj.len() <= cut_size || cut_size > max_set_size {
            break;
        }
    }

    Ok(PcResult { adj, sep_sets })
}

/// BFS `RecognizeMB` feature selection (Guide p. 43).
fn recognize_mb(
    clean: &CleanData,
    target: usize,
    independence: IndependenceTest,
    alpha: f64,
    max_set_size: usize,
    test_counter: &mut usize,
) -> Result<Vec<usize>, BayesNetError> {
    let can_adj_y: Vec<usize> = (0..clean.n_vars).filter(|&v| v != target).collect();
    let PcResult {
        adj: pc,
        sep_sets: sep_y,
    } = recognize_pc(
        clean,
        target,
        can_adj_y,
        independence,
        alpha,
        max_set_size,
        test_counter,
    )?;

    let mut in_mb = vec![false; clean.n_vars];
    for &x_i in &pc {
        if let Some(slot) = in_mb.get_mut(x_i) {
            *slot = true;
        }
    }

    let mut can_sp: Vec<Option<Vec<usize>>> = vec![None; clean.n_vars];
    let mut sep_from_x: Vec<Option<Vec<usize>>> = vec![None; clean.n_vars];
    for &x_i in &pc {
        let can_adj_xi: Vec<usize> = (0..clean.n_vars).filter(|&v| v != x_i).collect();
        let PcResult {
            adj: sp_xi,
            sep_sets: sep_xi,
        } = recognize_pc(
            clean,
            x_i,
            can_adj_xi,
            independence,
            alpha,
            max_set_size,
            test_counter,
        )?;
        if !sp_xi.contains(&target) {
            if let Some(slot) = in_mb.get_mut(x_i) {
                *slot = false;
            }
            if let Some(slot) = sep_from_x.get_mut(x_i) {
                *slot = sep_xi.get(target).cloned().flatten();
            }
        }
        if let Some(slot) = can_sp.get_mut(x_i) {
            *slot = Some(sp_xi);
        }
    }

    let snapshot_mb: Vec<usize> = (0..clean.n_vars)
        .filter(|&v| in_mb.get(v).copied().unwrap_or(false))
        .collect();
    for x_i in snapshot_mb {
        if let Some(Some(sp_xi)) = can_sp.get(x_i) {
            for &z_i in sp_xi {
                if z_i == target || in_mb.get(z_i).copied().unwrap_or(false) {
                    continue;
                }
                let mut cond = sep_y
                    .get(z_i)
                    .cloned()
                    .flatten()
                    .or_else(|| sep_from_x.get(z_i).cloned().flatten())
                    .unwrap_or_default();
                if !cond.contains(&x_i) {
                    cond.push(x_i);
                }
                let p = ci_test(clean, target, z_i, &cond, independence, test_counter)?;
                if p <= alpha
                    && let Some(slot) = in_mb.get_mut(z_i)
                {
                    *slot = true;
                }
            }
        }
    }

    Ok((0..clean.n_vars)
        .filter(|&v| in_mb.get(v).copied().unwrap_or(false))
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeState {
    None,
    Undirected,
    /// Directed edge `row -> col`.
    Forward,
    /// Directed edge `col -> row`.
    Backward,
}

fn creates_cycle(adj: &[Vec<EdgeState>], from: usize, to: usize) -> bool {
    let m = adj.len();
    let mut visited = vec![false; m];
    let mut stack = vec![to];
    while let Some(cur) = stack.pop() {
        if cur == from {
            return true;
        }
        if visited.get(cur).copied().unwrap_or(true) {
            continue;
        }
        if let Some(slot) = visited.get_mut(cur) {
            *slot = true;
        }
        if let Some(row) = adj.get(cur) {
            for (next, &edge) in row.iter().enumerate() {
                if edge == EdgeState::Forward && !visited.get(next).copied().unwrap_or(true) {
                    stack.push(next);
                }
            }
        }
    }
    false
}

fn orient_edge(adj: &mut [Vec<EdgeState>], from: usize, to: usize) -> bool {
    let cur = adj
        .get(from)
        .and_then(|r| r.get(to))
        .copied()
        .unwrap_or(EdgeState::None);
    if cur != EdgeState::Undirected || creates_cycle(adj, from, to) {
        return false;
    }
    if let Some(slot) = adj.get_mut(from).and_then(|r| r.get_mut(to)) {
        *slot = EdgeState::Forward;
    }
    if let Some(slot) = adj.get_mut(to).and_then(|r| r.get_mut(from)) {
        *slot = EdgeState::Backward;
    }
    true
}

fn train_markov_blanket(
    data: &CategoricalData,
    clean: &CleanData,
    predictors: &[usize],
    settings: &Settings,
    test_counter: &mut usize,
) -> Result<Network, BayesNetError> {
    let mut nodes = Vec::with_capacity(1 + predictors.len());
    nodes.push(data.target);
    nodes.extend_from_slice(predictors);
    nodes.sort_unstable();
    nodes.dedup();
    let m = nodes.len();
    let y_pos = nodes
        .iter()
        .position(|&v| v == data.target)
        .ok_or_else(|| BayesNetError::Data("target missing from node list".into()))?;

    let mut adj = vec![vec![EdgeState::Undirected; m]; m];
    for (u, row) in adj.iter_mut().enumerate() {
        if let Some(slot) = row.get_mut(u) {
            *slot = EdgeState::None;
        }
    }
    let mut sep_set: Vec<Vec<Option<Vec<usize>>>> = vec![vec![None; m]; m];

    // Step 1: unconditional independence tests (|S| = 0).
    for u in 0..m {
        for v in (u + 1)..m {
            let p = ci_test(
                clean,
                nodes[u],
                nodes[v],
                &[],
                settings.independence,
                test_counter,
            )?;
            if p > settings.significance_level {
                adj[u][v] = EdgeState::None;
                adj[v][u] = EdgeState::None;
                sep_set[u][v] = Some(Vec::new());
                sep_set[v][u] = Some(Vec::new());
            }
        }
    }

    // Step 2: exhaustive conditional independence search for k = 1..=maximal_conditioning_set.
    for k in 1..=settings.maximal_conditioning_set {
        let mut edges = Vec::new();
        for (u, row) in adj.iter().enumerate() {
            for (v, &edge) in row.iter().enumerate().skip(u + 1) {
                if edge != EdgeState::None {
                    edges.push((u, v));
                }
            }
        }
        let mut any_possible = false;
        for (u, v) in edges {
            if adj[u][v] == EdgeState::None {
                continue;
            }
            let mut found_sep: Option<Vec<usize>> = None;
            for (src, other) in [(u, v), (v, u)] {
                let neighbors: Vec<usize> = (0..m)
                    .filter(|&w| w != other && adj[src][w] != EdgeState::None)
                    .map(|w| nodes[w])
                    .collect();
                if neighbors.len() >= k {
                    any_possible = true;
                    let _ = for_each_subset(&neighbors, k, |s| {
                        let p = ci_test(
                            clean,
                            nodes[u],
                            nodes[v],
                            s,
                            settings.independence,
                            test_counter,
                        )?;
                        if p > settings.significance_level {
                            found_sep = Some(s.to_vec());
                            return Ok(true);
                        }
                        Ok(false)
                    })?;
                    if found_sep.is_some() {
                        break;
                    }
                }
            }
            if let Some(sep) = found_sep {
                adj[u][v] = EdgeState::None;
                adj[v][u] = EdgeState::None;
                sep_set[u][v] = Some(sep.clone());
                sep_set[v][u] = Some(sep);
            }
        }
        if !any_possible {
            break;
        }
    }

    // Arc Orientation Rules (Guide pp. 49-50).
    // Rule 1: X_i - X_j - X_k or X_i -> X_j - X_k -> X_i -> X_j <- X_k if X_j not in S_{X_i, X_k}.
    for i in 0..m {
        for k in (i + 1)..m {
            if adj[i][k] != EdgeState::None {
                continue;
            }
            for j in 0..m {
                if i == j || k == j {
                    continue;
                }
                let ij = adj[i][j];
                let kj = adj[k][j];
                if (ij != EdgeState::Undirected && ij != EdgeState::Forward)
                    || (kj != EdgeState::Undirected && kj != EdgeState::Forward)
                {
                    continue;
                }
                if let Some(Some(sep)) = sep_set.get(i).and_then(|r| r.get(k))
                    && !sep.contains(&nodes[j])
                {
                    let _ = orient_edge(&mut adj, i, j);
                    let _ = orient_edge(&mut adj, k, j);
                }
            }
        }
    }

    // Iterative Rules 2, 4, cycle avoidance, and Rule 3 fallback until all arcs are oriented.
    for _step in 0..MAX_ORIENTATION_STEPS {
        loop {
            let mut changed = false;
            // Rule 2: X_i -> X_j - X_k (with X_i, X_k non-adjacent) -> X_j -> X_k.
            for j in 0..m {
                for i in 0..m {
                    if i == j || adj[i][j] != EdgeState::Forward {
                        continue;
                    }
                    for k in 0..m {
                        if k == i || k == j || adj[i][k] != EdgeState::None {
                            continue;
                        }
                        if adj[j][k] == EdgeState::Undirected && orient_edge(&mut adj, j, k) {
                            changed = true;
                        }
                    }
                }
            }
            // Rule 4 (p. 50): X_i - X_j - X_k, X_i, X_k non-adjacent, X_i -> X_l <- X_k, X_j - X_l -> X_j -> X_l.
            for j in 0..m {
                for l in 0..m {
                    if l == j || adj[j][l] != EdgeState::Undirected {
                        continue;
                    }
                    let mut triggered = false;
                    for i in 0..m {
                        if i == j
                            || i == l
                            || adj[i][j] != EdgeState::Undirected
                            || adj[i][l] != EdgeState::Forward
                        {
                            continue;
                        }
                        for k in (i + 1)..m {
                            if k == j
                                || k == l
                                || adj[i][k] != EdgeState::None
                                || adj[k][j] != EdgeState::Undirected
                                || adj[k][l] != EdgeState::Forward
                            {
                                continue;
                            }
                            if orient_edge(&mut adj, j, l) {
                                changed = true;
                                triggered = true;
                                break;
                            }
                        }
                        if triggered {
                            break;
                        }
                    }
                }
            }
            // Cycle avoidance: if i - j and j -> ... -> i exists, orient j -> i.
            for i in 0..m {
                for j in (i + 1)..m {
                    if adj[i][j] == EdgeState::Undirected
                        && ((creates_cycle(&adj, i, j) && orient_edge(&mut adj, j, i))
                            || (creates_cycle(&adj, j, i) && orient_edge(&mut adj, i, j)))
                    {
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }

        // Rule 3: orient one remaining undirected edge X_u - X_v -> X_u -> X_v (or X_v -> X_u if cycle)
        // and return to step 2.
        let first_undirected = adj.iter().enumerate().find_map(|(u, row)| {
            row.iter()
                .enumerate()
                .skip(u + 1)
                .find_map(|(v, &edge)| (edge == EdgeState::Undirected).then_some((u, v)))
        });
        match first_undirected {
            None => break,
            Some((u, v)) => {
                if !orient_edge(&mut adj, u, v) {
                    let _ = orient_edge(&mut adj, v, u);
                }
            }
        }
    }

    // Derive Markov Blanket MB_Y = pi_Y U {Y} U X_Ch U parents(X_Ch) (Guide p. 50).
    let mut in_mb = vec![false; m];
    in_mb[y_pos] = true;
    for u in 0..m {
        if adj[u][y_pos] == EdgeState::Forward {
            in_mb[u] = true;
        }
        if adj[y_pos][u] == EdgeState::Forward {
            in_mb[u] = true;
            for p in 0..m {
                if adj[p][u] == EdgeState::Forward {
                    in_mb[p] = true;
                }
            }
        }
    }

    // Order active_variables with target first, then predictors in ascending index order.
    let mut mb_indices: Vec<usize> = (0..m).filter(|&u| u != y_pos && in_mb[u]).collect();
    mb_indices.sort_unstable();
    let mut ordered_pos = Vec::with_capacity(1 + mb_indices.len());
    ordered_pos.push(y_pos);
    ordered_pos.extend(mb_indices);

    let mut active_variables = Vec::with_capacity(ordered_pos.len());
    let mut cpts = Vec::with_capacity(ordered_pos.len());
    let mut parameter_count = 0usize;

    for pos in ordered_pos {
        let var = nodes[pos];
        active_variables.push(var);
        let parents: Vec<usize> = (0..m)
            .filter(|&p| in_mb[p] && adj[p][pos] == EdgeState::Forward)
            .map(|p| nodes[p])
            .collect();
        let cpt = estimate_cpt(clean, var, &parents, settings.parameter_learning_method)?;
        parameter_count += cpt.cardinality.saturating_sub(1) * cpt.parent_cardinality;
        cpts.push(cpt);
    }

    Ok(Network {
        structure_type: StructureType::MarkovBlanket,
        variables: data.variables.clone(),
        target: data.target,
        active_variables,
        cpts,
        parameter_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(name: &str, cats: &[&str]) -> Variable {
        Variable {
            name: name.to_string(),
            categories: cats.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn bayes_net_tan_known_answer_mle_and_bayes() -> Result<(), BayesNetError> {
        // 3 variables: Y (target, 2 cats), X0 (2 cats), X1 (2 cats).
        // 8 records:
        // Y=0: (X0=0,X1=0)x2, (X0=1,X1=1)x2  -> X0 and X1 perfectly coupled given Y=0
        // Y=1: (X0=0,X1=1)x2, (X0=1,X1=0)x2  -> X0 and X1 perfectly coupled given Y=1
        // Hand calculation:
        // P(Y=0) = 4/8 = 0.5, P(Y=1) = 0.5.
        // Given Y=0: P(X0=0|Y=0)=0.5, P(X0=1|Y=0)=0.5; P(X1=0|Y=0)=0.5, P(X1=1|Y=0)=0.5.
        // P(X0=0,X1=0|Y=0)=0.5, P(X0=1,X1=1|Y=0)=0.5 -> I(X0,X1|Y=0) = ln(2).
        // Total I(X0,X1|Y) = 0.5*ln(2) + 0.5*ln(2) = ln(2).
        // Prim's starts at X0 (root), adds X0 -> X1.
        // So X0 has parents [Y], X1 has parents [X0, Y].
        // Under MLE:
        //   P(Y) = [0.5, 0.5]
        //   P(X0 | Y=0) = [0.5, 0.5], P(X0 | Y=1) = [0.5, 0.5]
        //   P(X1 | X0=0, Y=0) = [1.0, 0.0], P(X1 | X0=0, Y=1) = [0.0, 1.0]
        //   P(X1 | X0=1, Y=0) = [0.0, 1.0], P(X1 | X0=1, Y=1) = [1.0, 0.0]
        //   Posterior for (X0=0, X1=0): Y=0 score = 0.5*0.5*1.0 = 0.25, Y=1 score = 0 -> [1.0, 0.0].
        // K = (2-1) + (2-1)*2 + (2-1)*4 = 7.
        // Under Bayes (N^0_Y = 2/2 = 1 -> (4+1)/(8+2)=0.5;
        //   X0: r=2, q=2 -> N^0_ijk = 2/4 = 0.5, N^0_ij = 1.0 -> (2+0.5)/(4+1) = 0.5;
        //   X1: r=2, q=4 -> N^0_ijk = 2/8 = 0.25, N^0_ij = 0.5 -> (2+0.25)/(2+0.5) = 0.9, (0+0.25)/2.5 = 0.1).
        //   Posterior for (X0=0, X1=0):
        //     Y=0 unnorm = 0.5 * 0.5 * 0.9 = 0.225; Y=1 unnorm = 0.5 * 0.5 * 0.1 = 0.025 -> [0.9, 0.1].
        let data = CategoricalData {
            variables: vec![
                var("X0", &["0", "1"]),
                var("X1", &["0", "1"]),
                var("Y", &["0", "1"]),
            ],
            rows: vec![
                vec![Some(0), Some(0), Some(0)],
                vec![Some(0), Some(0), Some(0)],
                vec![Some(1), Some(1), Some(0)],
                vec![Some(1), Some(1), Some(0)],
                vec![Some(0), Some(1), Some(1)],
                vec![Some(0), Some(1), Some(1)],
                vec![Some(1), Some(0), Some(1)],
                vec![Some(1), Some(0), Some(1)],
            ],
            target: 2,
        };

        let mle_settings = Settings {
            parameter_learning_method: ParameterLearningMethod::Likelihood,
            ..Settings::default()
        };
        let net_mle = train(&data, &mle_settings)?;
        assert_eq!(net_mle.parameter_count, 7);
        let x0_cpt = net_mle
            .cpt(0)
            .ok_or_else(|| BayesNetError::Data("missing X0".into()))?;
        assert_eq!(x0_cpt.parents, vec![2]);
        let x1_cpt = net_mle
            .cpt(1)
            .ok_or_else(|| BayesNetError::Data("missing X1".into()))?;
        assert_eq!(x1_cpt.parents, vec![0, 2]);
        assert_eq!(
            x1_cpt.probabilities,
            vec![1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0]
        );
        let post_mle = net_mle.posterior(&[Some(0), Some(0), None])?;
        assert!((post_mle[0] - 1.0).abs() < 1e-12);
        assert!(post_mle[1].abs() < 1e-12);

        let bayes_settings = Settings {
            parameter_learning_method: ParameterLearningMethod::Bayes,
            ..Settings::default()
        };
        let net_bayes = train(&data, &bayes_settings)?;
        let x1_bayes = net_bayes
            .cpt(1)
            .ok_or_else(|| BayesNetError::Data("missing X1".into()))?;
        for (got, exp) in x1_bayes
            .probabilities
            .iter()
            .zip([0.9, 0.1, 0.1, 0.9, 0.1, 0.9, 0.9, 0.1])
        {
            assert!((got - exp).abs() < 1e-12, "got {got}, expected {exp}");
        }
        let post_bayes = net_bayes.posterior(&[Some(0), Some(0), None])?;
        assert!((post_bayes[0] - 0.9).abs() < 1e-12);
        assert!((post_bayes[1] - 0.1).abs() < 1e-12);

        // Missing evidence -> marginal [0.5, 0.5]
        let post_missing = net_bayes.posterior(&[None, None, None])?;
        assert!((post_missing[0] - 0.5).abs() < 1e-12);
        assert!((post_missing[1] - 0.5).abs() < 1e-12);
        Ok(())
    }

    #[test]
    fn bayes_net_tan_known_answer_4var_tree() -> Result<(), BayesNetError> {
        // 4 variables: X0, X1, X2, Y (target at col 3), 32 records (16 per Y):
        // Markov chain X0 -> X1 -> X2 given Y:
        //   P(X1 = X0 | Y) = 6/8 = 3/4, P(X2 = X1 | Y) = 7/8, P(X2 = X0 | Y) = 5/8.
        // Hand calculation of CMI:
        //   I(X1, X2 | Y) = (14/32)*ln(14/8) + (2/32)*ln(2/8) ... = 0.316377...
        //   I(X0, X1 | Y) = (12/32)*ln(1.5) + (4/32)*ln(0.5) ... = 0.130812...
        //   I(X0, X2 | Y) = (10/32)*ln(1.25) + (6/32)*ln(0.75) ... = 0.031584...
        // Prim's starts at X0, picks max edge (X0, X1), then max edge (X1, X2).
        // Outward orientation from X0 gives X0 -> X1 -> X2:
        //   parents(X0) = [Y], parents(X1) = [X0, Y], parents(X2) = [X1, Y].
        // MLE P(X2=0 | X1=0, Y=0) = 7/8 = 0.875.
        let mut rows = Vec::with_capacity(32);
        for y in [0u32, 1] {
            for _ in 0..5 {
                rows.push(vec![Some(0), Some(0), Some(0), Some(y)]);
                rows.push(vec![Some(1), Some(1), Some(1), Some(y)]);
            }
            rows.push(vec![Some(0), Some(0), Some(1), Some(y)]);
            rows.push(vec![Some(1), Some(1), Some(0), Some(y)]);
            for _ in 0..2 {
                rows.push(vec![Some(0), Some(1), Some(1), Some(y)]);
                rows.push(vec![Some(1), Some(0), Some(0), Some(y)]);
            }
        }
        let data = CategoricalData {
            variables: vec![
                var("X0", &["0", "1"]),
                var("X1", &["0", "1"]),
                var("X2", &["0", "1"]),
                var("Y", &["0", "1"]),
            ],
            rows,
            target: 3,
        };
        let settings = Settings {
            parameter_learning_method: ParameterLearningMethod::Likelihood,
            ..Settings::default()
        };
        let net = train(&data, &settings)?;
        assert_eq!(
            net.cpt(0).map(|c| c.parents.as_slice()),
            Some([3].as_slice())
        );
        assert_eq!(
            net.cpt(1).map(|c| c.parents.as_slice()),
            Some([0, 3].as_slice())
        );
        assert_eq!(
            net.cpt(2).map(|c| c.parents.as_slice()),
            Some([1, 3].as_slice())
        );
        let c2 = net
            .cpt(2)
            .ok_or_else(|| BayesNetError::Data("missing X2 CPT".into()))?;
        // j=0 is (X1=0, Y=0): P(X2=0 | X1=0, Y=0) = 7/8 = 0.875.
        assert!((c2.probabilities[0] - 0.875).abs() < 1e-12);
        assert!((c2.probabilities[1] - 0.125).abs() < 1e-12);
        Ok(())
    }

    #[test]
    fn bayes_net_mb_known_answer_v_structure() -> Result<(), BayesNetError> {
        // 4 variables: A (0), B (1), Y (2, target), Z (3).
        // 32 rows: Y = A AND B; Z independent of (A, B, Y).
        // Hand calculation:
        //   (A, B): each cell 8 -> chi2 = 0, G2 = 0, p = 1.0 -> edge A-B deleted, S_{A,B} = {}.
        //   (A, Z), (B, Z), (Y, Z): each cell balanced -> p = 1.0 -> deleted.
        //   (A, Y): A=0 -> (Y=0:16, Y=1:0), A=1 -> (Y=0:8, Y=1:8):
        //     chi2 = 32/3 = 10.666667, df=1, p = 0.001091 < 0.05.
        //     G2 = 13.808739, df=1, p = 0.000202 < 0.05.
        //   Same for (B, Y). Skeleton: A - Y - B.
        //   Rule 1: Y not in S_{A,B}={} -> orients A -> Y <- B.
        //   MB_Y = {Y, A, B} (Z excluded).
        //   Under Bayes (r_Y=2, q_Y=4, N^0_ijk=0.25, N^0_ij=0.5):
        //     N(A=1,B=1)=8, N(Y=1,A=1,B=1)=8 -> P(Y=1 | A=1, B=1) = 8.25 / 8.5 = 33/34.
        let mut rows = Vec::with_capacity(32);
        for a in 0..=1u32 {
            for b in 0..=1u32 {
                let y = u32::from(a == 1 && b == 1);
                for z in 0..=1u32 {
                    for _ in 0..4 {
                        rows.push(vec![Some(a), Some(b), Some(y), Some(z)]);
                    }
                }
            }
        }
        let data = CategoricalData {
            variables: vec![
                var("A", &["0", "1"]),
                var("B", &["0", "1"]),
                var("Y", &["0", "1"]),
                var("Z", &["0", "1"]),
            ],
            rows,
            target: 2,
        };

        for ind in [IndependenceTest::Pearson, IndependenceTest::Likelihood] {
            let settings = Settings {
                structure_type: StructureType::MarkovBlanket,
                independence: ind,
                parameter_learning_method: ParameterLearningMethod::Bayes,
                ..Settings::default()
            };
            let net = train(&data, &settings)?;
            assert_eq!(net.active_variables, vec![2, 0, 1]);
            let y_cpt = net
                .cpt(2)
                .ok_or_else(|| BayesNetError::Data("missing Y CPT".into()))?;
            assert_eq!(y_cpt.parents, vec![0, 1]);
            let post = net.posterior(&[Some(1), Some(1), None, Some(0)])?;
            assert!((post[0] - (1.0 / 34.0)).abs() < 1e-12);
            assert!((post[1] - (33.0 / 34.0)).abs() < 1e-12);
        }
        Ok(())
    }

    #[test]
    fn bayes_net_mb_known_answer_spouse() -> Result<(), BayesNetError> {
        // 4 variables: Y (0, target), C (1), S (2), Z (3).
        // 32 rows: C = Y AND S; Z independent.
        // Hand calculation:
        //   Marginally: Y and S are independent (chi2 = 0, p = 1.0 -> edge Y-S deleted, S_{Y,S} = {}).
        //   Conditioned on C: chi2(Y, S | C=0) = 6.0, chi2(Y, S | C=1) = 0 -> total chi2 = 6.0,
        //   df = (2-1)*(2-1)*2 = 2 -> p = exp(-3) = 0.049787 < 0.05 (so Y and S are dependent | C).
        //   Rule 1 on Y - C - S with C not in S_{Y,S}={} -> Y -> C <- S.
        //   MB_Y = {Y, C, S} (Z excluded, S included as spouse!).
        //   Posterior given (C=0, S=1) under Bayes:
        //     P(Y) = [0.5, 0.5]; P(C=0 | Y=0, S=1) = 8.25/8.5 = 33/34, P(C=0 | Y=1, S=1) = 0.25/8.5 = 1/34
        //     -> P(Y=0 | C=0, S=1) = 33/34, P(Y=1 | C=0, S=1) = 1/34.
        let mut rows = Vec::with_capacity(32);
        for y in 0..=1u32 {
            for s in 0..=1u32 {
                let c = u32::from(y == 1 && s == 1);
                for z in 0..=1u32 {
                    for _ in 0..4 {
                        rows.push(vec![Some(y), Some(c), Some(s), Some(z)]);
                    }
                }
            }
        }
        let data = CategoricalData {
            variables: vec![
                var("Y", &["0", "1"]),
                var("C", &["0", "1"]),
                var("S", &["0", "1"]),
                var("Z", &["0", "1"]),
            ],
            rows,
            target: 0,
        };
        let settings = Settings {
            structure_type: StructureType::MarkovBlanket,
            independence: IndependenceTest::Pearson,
            parameter_learning_method: ParameterLearningMethod::Bayes,
            ..Settings::default()
        };
        let net = train(&data, &settings)?;
        assert_eq!(net.active_variables, vec![0, 1, 2]);
        let c_cpt = net
            .cpt(1)
            .ok_or_else(|| BayesNetError::Data("missing C CPT".into()))?;
        assert_eq!(c_cpt.parents, vec![0, 2]);
        let post = net.posterior(&[None, Some(0), Some(1), Some(0)])?;
        assert!((post[0] - (33.0 / 34.0)).abs() < 1e-12);
        assert!((post[1] - (1.0 / 34.0)).abs() < 1e-12);
        Ok(())
    }

    /// Fixed SplitMix64 generator for deterministic synthetic sampling tests.
    struct TestRng(u64);
    impl TestRng {
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        fn bernoulli(&mut self, p: f64) -> u32 {
            let u = ((self.next_u64() >> 11) as f64) / ((1u64 << 53) as f64);
            u32::from(u < p)
        }
    }

    #[test]
    fn bayes_net_tan_recovery_4pred_tree() -> Result<(), BayesNetError> {
        // True TAN structure: Y (col 4) -> all X0..X3, and predictor chain X0 -> X1 -> X2 -> X3.
        // Conditional couplings given Y:
        //   P(X0=1 | Y) = 0.25 if Y=0 else 0.75
        //   P(X_{i}=X_{i-1} | Y) = 0.85 (so I(X_{i-1}, X_i | Y) >= 0.27 nats, while non-adjacent
        //   pairs have CMI <= 0.08 nats, and gap > 0.19 nats).
        // With N = 2000, SE of empirical CMI is O(1/sqrt(N)) ~ 0.022 nats (~8.5x smaller than
        // the gap), so Prim's MWST recovers the exact chain X0 -> X1 -> X2 -> X3.
        let mut rng = TestRng(42);
        let n = 2_000;
        let mut rows = Vec::with_capacity(n);
        for _ in 0..n {
            let y = rng.bernoulli(0.5);
            let x0 = rng.bernoulli(if y == 0 { 0.25 } else { 0.75 });
            let flip1 = rng.bernoulli(0.15);
            let x1 = x0 ^ flip1;
            let flip2 = rng.bernoulli(0.15);
            let x2 = x1 ^ flip2;
            let flip3 = rng.bernoulli(0.15);
            let x3 = x2 ^ flip3;
            rows.push(vec![Some(x0), Some(x1), Some(x2), Some(x3), Some(y)]);
        }
        let data = CategoricalData {
            variables: vec![
                var("X0", &["0", "1"]),
                var("X1", &["0", "1"]),
                var("X2", &["0", "1"]),
                var("X3", &["0", "1"]),
                var("Y", &["0", "1"]),
            ],
            rows,
            target: 4,
        };
        let net = train(&data, &Settings::default())?;
        assert_eq!(
            net.cpt(0).map(|c| c.parents.as_slice()),
            Some([4].as_slice())
        );
        assert_eq!(
            net.cpt(1).map(|c| c.parents.as_slice()),
            Some([0, 4].as_slice())
        );
        assert_eq!(
            net.cpt(2).map(|c| c.parents.as_slice()),
            Some([1, 4].as_slice())
        );
        assert_eq!(
            net.cpt(3).map(|c| c.parents.as_slice()),
            Some([2, 4].as_slice())
        );
        Ok(())
    }

    #[test]
    fn bayes_net_mb_recovery_parents_children_spouses() -> Result<(), BayesNetError> {
        // True 6-variable DAG: D -> P -> Y -> C <- S, and disconnected U.
        // True Markov blanket of Y: {P, Y, C, S} (excludes grandparent D and disconnected U).
        // Couplings:
        //   D ~ Bern(0.5), P = D with p=0.85, Y = P with p=0.85,
        //   S ~ Bern(0.5), C = (Y & S) with p=0.90 (and 0.10 when !(Y & S)),
        //   U ~ Bern(0.5).
        // Sample size N = 4000: gives non-centrality lambda > 40 for conditional spouse test
        // I(Y, S | C) (chi2 ~ 100+, p << 0.001), while conditional independence D _|_ Y | P
        // and marginal independence of U have expected p > 0.05 (use alpha = 0.01 to avoid
        // type-I false edges across 15 pairs).
        let mut rng = TestRng(123_456);
        let n = 4_000;
        let mut rows = Vec::with_capacity(n);
        for _ in 0..n {
            let d = rng.bernoulli(0.5);
            let p = d ^ rng.bernoulli(0.15);
            let y = p ^ rng.bernoulli(0.15);
            let s = rng.bernoulli(0.5);
            let c = if y == 1 && s == 1 {
                rng.bernoulli(0.90)
            } else {
                rng.bernoulli(0.10)
            };
            let u = rng.bernoulli(0.5);
            rows.push(vec![Some(d), Some(p), Some(y), Some(c), Some(s), Some(u)]);
        }
        let data = CategoricalData {
            variables: vec![
                var("D", &["0", "1"]),
                var("P", &["0", "1"]),
                var("Y", &["0", "1"]),
                var("C", &["0", "1"]),
                var("S", &["0", "1"]),
                var("U", &["0", "1"]),
            ],
            rows,
            target: 2,
        };
        let settings = Settings {
            structure_type: StructureType::MarkovBlanket,
            significance_level: 0.01,
            independence: IndependenceTest::Likelihood,
            ..Settings::default()
        };
        let net = train(&data, &settings)?;
        // active_variables has Y (2) first, then sorted MB predictors: P (1), C (3), S (4).
        assert_eq!(net.active_variables, vec![2, 1, 3, 4]);

        // Also verify BFS feature selection (use_feature_selection = true) recovers {P, C, S}
        // on the same dataset (here U is removed at cutSetSize = 0 so cutSetSize = 1 runs
        // and removes D conditioned on P).
        let fs_settings = Settings {
            use_feature_selection: true,
            ..settings
        };
        let net_fs = train(&data, &fs_settings)?;
        assert_eq!(net_fs.active_variables, vec![2, 1, 3, 4]);
        Ok(())
    }

    #[test]
    fn bayes_net_properties_valid_probs_deterministic_unseen_missing() -> Result<(), BayesNetError>
    {
        let mut rows = Vec::new();
        for a in 0..=1u32 {
            for b in 0..=1u32 {
                let y = u32::from(a == 1 && b == 1);
                for _ in 0..6 {
                    rows.push(vec![Some(a), Some(b), Some(y)]);
                }
            }
        }
        let data = CategoricalData {
            variables: vec![
                var("A", &["a0", "a1"]),
                var("B", &["b0", "b1"]),
                var("Y", &["no", "yes"]),
            ],
            rows,
            target: 2,
        };

        for st in [StructureType::Tan, StructureType::MarkovBlanket] {
            for pl in [
                ParameterLearningMethod::Likelihood,
                ParameterLearningMethod::Bayes,
            ] {
                let settings = Settings {
                    structure_type: st,
                    parameter_learning_method: pl,
                    ..Settings::default()
                };
                let net1 = train(&data, &settings)?;
                let net2 = train(&data, &settings)?;
                assert_eq!(net1, net2);
                assert_eq!(net1.to_text(), net2.to_text());
                assert!(net1.to_text().contains("Node Y"));

                for cpt in &net1.cpts {
                    for &p in &cpt.probabilities {
                        assert!(p.is_finite() && (0.0..=1.0).contains(&p));
                    }
                    for j in 0..cpt.parent_cardinality {
                        let row_sum: f64 = cpt.probabilities
                            [j * cpt.cardinality..(j + 1) * cpt.cardinality]
                            .iter()
                            .sum();
                        if pl == ParameterLearningMethod::Bayes {
                            assert!((row_sum - 1.0).abs() < 1e-12);
                        } else {
                            assert!((row_sum - 1.0).abs() < 1e-12 || row_sum == 0.0);
                        }
                    }
                }

                for ev in [
                    [Some(0), Some(0), None],
                    [Some(1), Some(1), None],
                    [Some(1), None, None],
                    [None, None, None],
                    [Some(999), Some(1), None],
                    [Some(999), Some(999), Some(42)],
                ] {
                    let post = net1.posterior(&ev)?;
                    assert_eq!(post.len(), 2);
                    for &p in &post {
                        assert!(p.is_finite() && (0.0..=1.0).contains(&p));
                    }
                    let sum: f64 = post.iter().sum();
                    assert!((sum - 1.0).abs() < 1e-12, "sum = {sum}");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn bayes_net_never_crash_and_gaps() -> Result<(), BayesNetError> {
        let base_vars = vec![var("X", &["0", "1"]), var("Y", &["0", "1"])];

        // 1. Empty table -> Data error
        let empty = CategoricalData {
            variables: base_vars.clone(),
            rows: Vec::new(),
            target: 1,
        };
        assert!(matches!(
            train(&empty, &Settings::default()),
            Err(BayesNetError::Data(_))
        ));

        // 2. All rows incomplete -> Data error
        let all_missing = CategoricalData {
            variables: base_vars.clone(),
            rows: vec![vec![None, Some(0)], vec![Some(1), None]],
            target: 1,
        };
        assert!(matches!(
            train(&all_missing, &Settings::default()),
            Err(BayesNetError::Data(_))
        ));

        // 3. One record -> Ok for TAN and MB
        let one_row = CategoricalData {
            variables: base_vars.clone(),
            rows: vec![vec![Some(0), Some(1)]],
            target: 1,
        };
        for st in [StructureType::Tan, StructureType::MarkovBlanket] {
            let net = train(
                &one_row,
                &Settings {
                    structure_type: st,
                    ..Settings::default()
                },
            )?;
            let post = net.posterior(&[Some(0), None])?;
            assert!((post.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        }

        // 4. Single-category target -> Ok, posterior = [1.0]
        let single_target = CategoricalData {
            variables: vec![var("X", &["0", "1"]), var("Y", &["only"])],
            rows: vec![vec![Some(0), Some(0)], vec![Some(1), Some(0)]],
            target: 1,
        };
        for st in [StructureType::Tan, StructureType::MarkovBlanket] {
            let net = train(
                &single_target,
                &Settings {
                    structure_type: st,
                    ..Settings::default()
                },
            )?;
            let post = net.posterior(&[Some(0), None])?;
            assert_eq!(post.len(), 1);
            assert!((post[0] - 1.0).abs() < 1e-12);
        }

        // 5. Constant variables (both 1-category and multi-category with single observed value)
        let constant_vars = CategoricalData {
            variables: vec![
                var("C1", &["a"]),
                var("C3", &["a", "b", "c"]),
                var("Y", &["0", "1"]),
            ],
            rows: vec![
                vec![Some(0), Some(0), Some(0)],
                vec![Some(0), Some(0), Some(1)],
            ],
            target: 2,
        };
        for st in [StructureType::Tan, StructureType::MarkovBlanket] {
            for ind in [IndependenceTest::Pearson, IndependenceTest::Likelihood] {
                let net = train(
                    &constant_vars,
                    &Settings {
                        structure_type: st,
                        independence: ind,
                        ..Settings::default()
                    },
                )?;
                let post = net.posterior(&[Some(0), Some(1), None])?;
                assert!((post.iter().sum::<f64>() - 1.0).abs() < 1e-12);
            }
        }

        // 6. Thousands of categories -> Data error (capped at MAX_CATEGORIES)
        let many_cats: Vec<String> = (0..2_500).map(|i| format!("c{i}")).collect();
        let huge_card = CategoricalData {
            variables: vec![
                Variable {
                    name: "Big".into(),
                    categories: many_cats,
                },
                var("Y", &["0", "1"]),
            ],
            rows: vec![vec![Some(0), Some(0)]],
            target: 1,
        };
        assert!(matches!(
            train(&huge_card, &Settings::default()),
            Err(BayesNetError::Data(_))
        ));

        // 7. Conditioning-set limit of 0 -> Ok for MB and feature selection
        let mb_zero = train(
            &one_row,
            &Settings {
                structure_type: StructureType::MarkovBlanket,
                use_feature_selection: true,
                maximal_conditioning_set: 0,
                ..Settings::default()
            },
        )?;
        assert!(!mb_zero.active_variables.is_empty());

        // 8. GAP errors: Pairwise missing handling & maximum_number_inputs pruning
        let pairwise = Settings {
            missing_values: MissingValueHandling::Pairwise,
            ..Settings::default()
        };
        // Without a missing value pairwise deletion is complete records.
        assert_eq!(
            train(&one_row, &pairwise)?,
            train(&one_row, &Settings::default())?
        );
        let with_missing = CategoricalData {
            variables: base_vars.clone(),
            rows: vec![vec![Some(0), Some(1)], vec![None, Some(0)]],
            target: 1,
        };
        assert!(matches!(
            train(&with_missing, &pairwise),
            Err(BayesNetError::Unsupported(ref msg)) if msg.contains("GAP:")
        ));

        let two_preds = CategoricalData {
            variables: vec![
                var("X0", &["0", "1"]),
                var("X1", &["0", "1"]),
                var("Y", &["0", "1"]),
            ],
            rows: vec![vec![Some(0), Some(0), Some(0)]],
            target: 2,
        };
        let gap_max_inputs = train(
            &two_preds,
            &Settings {
                maximum_number_inputs: Some(1),
                ..Settings::default()
            },
        );
        assert!(matches!(
            gap_max_inputs,
            Err(BayesNetError::Unsupported(ref msg)) if msg.contains("GAP:")
        ));

        // 9. Corrupt category code & wrong evidence length -> error, never panic
        let bad_code = CategoricalData {
            variables: base_vars,
            rows: vec![vec![Some(5), Some(0)]],
            target: 1,
        };
        assert!(matches!(
            train(&bad_code, &Settings::default()),
            Err(BayesNetError::Data(_))
        ));
        let net_ok = train(&one_row, &Settings::default())?;
        assert!(matches!(
            net_ok.posterior(&[None]),
            Err(BayesNetError::Data(_))
        ));

        Ok(())
    }

    #[test]
    fn bayes_net_inputs_always_selected_and_large_mb_fallback() -> Result<(), BayesNetError> {
        // Verify inputs_always_selected retains an independent variable even when feature
        // selection drops it, and verify the large-MB missing-evidence fallback path.
        let mut rows = Vec::with_capacity(32);
        for a in 0..=1u32 {
            for b in 0..=1u32 {
                let y = u32::from(a == 1 && b == 1);
                for z in 0..=1u32 {
                    for _ in 0..4 {
                        rows.push(vec![Some(a), Some(b), Some(y), Some(z)]);
                    }
                }
            }
        }
        let data = CategoricalData {
            variables: vec![
                var("A", &["0", "1"]),
                var("B", &["0", "1"]),
                var("Y", &["0", "1"]),
                var("Z", &["0", "1"]),
            ],
            rows,
            target: 2,
        };
        let settings = Settings {
            structure_type: StructureType::Tan,
            use_feature_selection: true,
            inputs_always_selected: vec![3],
            ..Settings::default()
        };
        let net = train(&data, &settings)?;
        // Z (3) was independent of Y, so feature selection alone would drop it,
        // but inputs_always_selected keeps it in active_variables.
        assert!(net.active_variables.contains(&3));

        // Construct a synthetic MarkovBlanket network with 13 binary children of Y (2^13 = 8192 > 4096)
        // to exercise the large missing-evidence fallback in mb_log_scores without overflow/panic.
        let mut vars = vec![var("Y", &["0", "1"])];
        for i in 0..13 {
            vars.push(var(&format!("C{i}"), &["0", "1"]));
        }
        let mut cpts = vec![ConditionalProbabilityTable {
            variable: 0,
            parents: Vec::new(),
            cardinality: 2,
            parent_cardinality: 1,
            probabilities: vec![0.4, 0.6],
        }];
        for i in 1..=13 {
            cpts.push(ConditionalProbabilityTable {
                variable: i,
                parents: vec![0],
                cardinality: 2,
                parent_cardinality: 2,
                probabilities: vec![0.8, 0.2, 0.3, 0.7],
            });
        }
        let big_mb = Network {
            structure_type: StructureType::MarkovBlanket,
            variables: vars,
            target: 0,
            active_variables: (0..14).collect(),
            cpts,
            parameter_count: 1 + 13 * 2,
        };
        // All 13 children missing -> fallback path triggers (2^13 > 4096) and returns [0.4, 0.6].
        let post_all_missing = big_mb.posterior(&[None; 14])?;
        assert!((post_all_missing[0] - 0.4).abs() < 1e-12);
        assert!((post_all_missing[1] - 0.6).abs() < 1e-12);
        // Observe C0 = 1 while C1..C12 missing -> updates P(Y) using C0 only.
        let mut ev = vec![None; 14];
        ev[1] = Some(1);
        let post_c0 = big_mb.posterior(&ev)?;
        // P(Y=0) * P(C0=1|Y=0) = 0.4 * 0.2 = 0.08; P(Y=1) * P(C0=1|Y=1) = 0.6 * 0.7 = 0.42
        assert!((post_c0[0] - 0.08 / 0.50).abs() < 1e-12);
        assert!((post_c0[1] - 0.42 / 0.50).abs() < 1e-12);
        Ok(())
    }
}
