//! Clean-room C5.0 classification trees. Sources: IBM SPSS Modeler 19 Scripting
//! and Automation Guide pp.226–227; Algorithms Guide pp.1,51; PMML 4.3 TreeModel.
//! No See5/C5.0 implementation or Modeler executable was consulted.
//!
//! Evidence (`models_ml/` unless qualified):
//! - `c50_default`, `c50_expert`: information gain ratio, numeric midpoint cuts,
//!   multiway symbols; depth-first ids; flags true before false; largest default
//!   child (first on ties); parent prediction resolves tied class counts.
//! - `models/c50_default`: numeric cut description-length correction, statistical
//!   leaf pruning. Numeric thresholds floor the binary64 midpoint to eight decimal
//!   places, only if finite and partition-preserving; see [`numeric_cut`]. This
//!   retains K = 0.0373315 but also the seven-digit `drughalf` Na = 0.7350505
//!   and `drugtwice` K = 0.04003799, refuting six significant digits.
//! - Numeric description length is log2(min(cuts, range/gap))/N. `cuts` counts
//!   distinct adjacent values with two sufficiently large children, not records;
//!   range and best-cut gap are local to the node. In `c50_presets/drughalf`
//!   (case names prefixed `c50_`, suffixed `_expert`), the six-record node 13 has
//!   identical Na/K partitions, raw gain 1 and three eligible cuts. K's range/gap
//!   is 2.1778846154, so corrected gain 0.8128454130 beats Na's 0.7358395832.
//!   In `drugtwice` node 31 (28 records, 14 distinct numeric values), Na/K have
//!   equal uncorrected gain and class counts but DIFFERENT record partitions.
//!   Both have 13 cuts; K's range/gap = 6.3636650869 gives ratio 0.2248254785
//!   versus Na's 0.1626179637. This is not an input-order tie.
//! - `c50_min_child`: at least two branches must reach min_child_records.
//! - Of equal gain ratios the field first in input order wins (`models/c50_default`:
//!   `Sex` over `Cholesterol`, mirror-image partitions; `c50_weight`: `flag` over
//!   `noise`).
//! - Without `groupSymbolicValues` every symbol is its own branch while splits are
//!   evaluated; after pruning, sibling leaves of a symbolic split that predict the same
//!   class merge into one set branch (`c50_costs`: `code in {2, 3}`; `c50_partition`;
//!   `c50_weighted/c50_x4_unit`: `cat in {green, red}`). Merging pure symbols before
//!   evaluation picks `cat` over `code` in `c50_weighted/c50_w2_loose`.
//! - An ordinal field splits in two at a cut over its values in sorted order, costing
//!   log2(cuts)/total of gain like a numeric threshold (`c50_weighted`: `ord in {high,
//!   low}` versus `{middle}`; `c50_costs`: `code` beats the uncorrected `ord` cut).
//! - P (positive-gain gate): average only tests with positive local corrected
//!   gain (numerical positivity tolerance 1e-10), require that same gain to reach
//!   the average, then maximize gain / split information. Zero/negative gains do
//!   not dilute the average; there is no separate "examined gain". Costs retain
//!   the recorded gate bypass (`c50_costs` node 5).
//!   Public basis: Quinlan, "Improved Use of Continuous Attributes in C4.5",
//!   JAIR 4 (1996), pp.78–80: average-gain restriction (p.79), selecting numeric
//!   thresholds by gain and charging description length (p.80). Its literal
//!   distinct-value correction differs from our recorded local range/gap rule;
//!   positive-only membership is fixture evidence, not specified there.
//! - `c50_costs` (B->C 8, C->B 2): minimize sum(count[actual]*cost[actual][predicted]),
//!   retaining the parent's class on ties. Counts stay raw. Bound the number of
//!   misclassified cases, then multiply by their mean cost; for zero observed
//!   errors retain the ordinary bound. Under amber/flag=T/x<=9 this gives leaf
//!   8.75954415583688 versus subtree 7.567663573829303, so the split stays.
//!   Growth uses raw counts even with costs.
//! - Case weights (`C50Settings::weight_field`; the node still rejects `useWeight`):
//!   each weight keeps six significant digits, then all are scaled to sum to the
//!   record count. Probe `c50_weighted_streams` (weights of boosting trial 2, subsets,
//!   pruning settings): five weighted fixtures compare exactly; remaining GAPs below.
//! - Pruning (probe `c50_globalprune_streams`, 110 trees exact; `c50_prune`, 6 trees exact;
//!   `auto_train/ac_*`, 16 saved component trees exact): local pruning collapses a subtree
//!   whose error bound is not lower than its leaf's by more than 0.1, or raises the largest
//!   internal child by original case count when its redistributed bound is within 0.1 of the
//!   subtree's (`c50_prune/auto44`, `auto_noglobal`), regrowing the raised child's branches
//!   and then pruning the regrown candidate with both replacement and raising (without a
//!   second regrowth pass). In `c50_prune/wn23` and `wn100_noglobal` (`c3 = F / c2 = T`,
//!   23 rows), raising `c1 = T`'s child `c4` and pruning its regrown branches collapses
//!   `c4 = T` (12 rows) to a leaf while keeping `d <= 47.605` under `c4 = F` (11 rows,
//!   3-leaf bound 10.149265); raising that remaining internal child `d <= 47.605` onto all
//!   23 rows reduces the bound to 9.186232. Global pruning ranks subtrees by the training
//!   errors added by collapsing them, counted in records (costs under misclassification
//!   costs), and breaks equal-error ties by largest leaf count (`auto_train/ac_*`: at 41
//!   errors against limit 45.962848, collapsing the 4-leaf `member = N` split adds 2 errors
//!   before the 2-leaf `income <= 28` split under `region = east`, exposing `income > 59.5`
//!   as another 2-error, 2-leaf candidate; tied at `(2, 2)`, both together would reach
//!   47 > 45.962848 and are rejected), see [`global_prune`]. Plain multiway splits have a
//!   branch for every declared value, and a nominal field with too many values is never used
//!   (see [`MANY_VALUES`]). GAP `c50_weight`, `c50_cw_sev50`: global pruning there
//!   removes `red, code = 3, flag = T`'s `x <= 9` split but keeps `code = 1, x > 11`'s
//!   flag split, which have equal error increases; likely tied to the zero distributions.
//! - GAP zero distributions: with case weights Modeler exports some leaves with all
//!   class counts 0 but their record count kept, sums them into their ancestors, and
//!   scores them with confidence 0. Smallest case `c50_weighted/c50_rc1_x6_10` (12
//!   rows, x in 6..10, two weight-6 rows): `x > 9` is zero, `x <= 9` is not. It does not
//!   depend on record order or pruning settings, and it is not a rule of the subtree
//!   alone: `x <= 9` under `x > 5` is zero in `c50_weight` but not in `c50_rc1` (its
//!   red, code 1 rows), and adding the code 2 or 3 rows (`c50_rc12`, `c50_rc13`)
//!   removes every zero while `c50_red` zeroes all of `code = 1, x <= 11`.
//!
//! Zero-distribution investigation (recorded oracles only, 2026-10-17):
//! - No general rule established; do not zero counts heuristically or enable weights.
//!   The 22 weighted `c50_weighted` fixtures plus `models_ml/c50_weight` contain
//!   240 nonempty leaves, 18 with zero distributions (repeated settings included).
//!   Only the five fixtures in the exact weighted test currently match completely.
//! - Direct positive-weight class sums cannot explain the zeros. In the smallest
//!   case the right leaf's true A/B/C counts are 6/11, 72/11, 6/11. Sequentially
//!   summing its class counts and its case weights gives the same total in both
//!   f64 and f32; subtracting the left branch from its parent leaves three positive
//!   counts. Wholesale f32 weights/counts also lose correct exports: the nonzero
//!   `c50_rc1_x6` leaf `x > 9, x <= 11` has A=0.66666667 (one case of weight 2/3),
//!   whereas f32 would round it to 0.66666669 at eight significant digits.
//! - A negative class residual on the path is not sufficient or necessary. With
//!   f64 weights/sums and each child obtained by subtracting all excluded records
//!   from its inherited parent counts, it flags 4/18 zero leaves and 142/222
//!   nonzero leaves. With f32 weights/sums: 5/18 and 115/222. Assigning only the
//!   last child the residual flags 0/18 and 65/222 (f64), 0/18 and 45/222 (f32).
//!   A full x sweep flagging negative residuals at any ancestor gives 13/18 and
//!   193/222 (f64), 11/18 and 161/222 (f32). These are simulated arithmetic orders,
//!   not claims about Modeler's implementation.
//! - Collapsing a grown subtree is only a partial correlate: 15/18 zero leaves
//!   were internal nodes in our grown trees. Exceptions: `c50_code1_loose` nodes
//!   5 and 10, `c50_red_loose` node 5. Conversely the nonzero `x <= 9` leaf in
//!   `c50_rc1_x6_10` was also split. Its zero right leaf's child-total excess is
//!   8.881784197001252e-16; the nonzero `x > 9, x <= 11` leaf in `c50_rc1_x6`
//!   has a larger excess, 1.7763568394002505e-15. Neither sign nor a discrepancy
//!   threshold establishes a zero rule. The categorical-only weighted global-prune
//!   grids add four zero leaves each, none internal in our grown trees; a numeric
//!   threshold sweep or collapse cannot be required.
//! - Additional exact-comparison GAPs, independent of zero distributions:
//!   `c50_rc1_w2` has no zeros but Modeler globally collapses `x > 3, flag = T,
//!   x <= 11` and we retain its x=9 split. (Ordinal cuts now keep values absent from
//!   the node in their sorted range, matching `c50_rc12_loose` and `c50_rc13_loose`
//!   up to last-digit totals.) `c50_rc1_x6` root A exports
//!   4.6666666 (children summed), ours 4.6666667 (own count rounded). `c50_rc1g_loose`
//!   exports leaf total 6.0952381 versus our 6.0952380999999995. Zeroing the right
//!   leaves alone cannot make every weighted fixture exact or settle global pruning.
//!   The ignored `c50_all_weighted_trees_match_exactly` contract checks all fixtures;
//!   the enabled inventory test records the zeros without treating them as a rule.
//!
//! - `c50_group_symbols`: greedy gain-ratio grouping of symbolic categories.
//! - `c50_prune90`: pruning confidence is (100 - severity)/100.
//! - `c50_no_global_prune`: local pruning, including R5, without global pruning.
//! - R5: recursively prune children; choose the largest remaining internal
//!   child by original case count (first on ties), route all current cases
//!   through its fixed tests and recompute scores/distributions.
//!   Compare leaf L, kept subtree S, and fixed candidate R: choose the leaf if
//!   L<=S+0.1 and L<=R+0.1, otherwise raise if R<=S+0.1. Only after selection,
//!   retain the promoted test and regrow its descendants, then apply replacement
//!   pruning without more raising on that candidate. The phase-1 scope is the
//!   first twelve levels; growth and regrowth share the depth/node resource cap.
//!   Public basis for comparing leaf/subtree/largest branch and summing binomial
//!   upper estimates: Esposito, Malerba & Semeraro, "A Comparative Analysis of
//!   Methods for Pruning Decision Trees", IEEE TPAMI 19 (1997), pp.481–482.
//!   The precise R5 selection/regrowth schedule is an empirical convention.
//!   Phase-4 evidence (`c50_prune`, 2026-10-28): excluding leaf children before
//!   choosing the largest candidate makes auto44 and auto_noglobal exact,
//!   preserves wn11/auto8 and all 173 previously exact tree controls. This is
//!   an empirical extension of the paper's internal-node graft restriction,
//!   not a published C5.0 specification. On Auto44, north (15 cases) is a leaf;
//!   west (14) supplies age<=32, then regrowth supplies income<=90. The fixed
//!   candidate estimate 17.190266 beats the retained subtree's 17.871095.
//!   Whole Auto Classifier components improve from 14 to 2 prediction-value
//!   disagreements and 52 to 17 value/confidence disagreements per 360 rows,
//!   but still differ after global pruning. No candidate is chosen by dataset,
//!   field name, global-pruning setting or a fitted numerical threshold.
//!   Row routing binds each grown predicate's column and numeric cut once per
//!   branch partition, rather than looking up/parsing them for every record.
//!   This removes repeated work without changing predicates or arithmetic.
//! - `c50_discriminate` (15 complete trees): g01–g04 isolate positive-only gate
//!   membership, g05–g07 gain comparison across numeric/flag tests, g08/g09 local
//!   penalty scope; r02/r03 require descendants to regrow (cut 2.5 -> 2.75).
//!   P+R5 matches every node attribute and score, including g02_zero's b-then-a
//!   structure, which scores alone cannot distinguish from a-then-b. Previously
//!   exact contracts and `c50_presets/c50_wn_expert` stay exact.
//! - Tree confidence is (class_count+1)/(node_count+target_categories), with zero
//!   for absent classes in the exported distribution; raw binary propensity uses
//!   confidence and its complement, including absent classes (`c50_partition`).
//! - Rule sets (`modelFormat` `ruleSet`): [`crate::c50_rules`].
//!
//! Simple presets and winnowing investigation (Modeler 19, `c50_presets_streams`,
//! recorded oracles only, 2026-10-17):
//! - All 24 Simple TreeModels (exception grid, noisy grid, DRUG1n; accuracy noise
//!   0/5/10/20/30/50, generality noise 0/20) are identical within each dataset,
//!   including predicates, counts, distributions and MiningSchema. Our Expert
//!   defaults reproduce all 24 exactly. The older `models_ml/c50_generality` and
//!   `c50_noise20` also equal `c50_default`. The node accepts these recorded presets
//!   as training no-ops. This is an evidence-based mapping, not proof about all
//!   Simple settings: other noise values/combinations and nondefault presets with
//!   rules/boosting remain unsupported (no corresponding oracle).
//! - Baseline follow-up (recorded oracles only, 2026-10-18): drughalf and drugtwice
//!   now compare exactly, including 200/800 scored rows respectively, alongside
//!   all 110 unweighted global-prune trees and the previously exact controls.
//!   In drughalf, collapsing the Age subtree adds two errors, versus three for
//!   the Na/K subtree. Dividing by removed leaves reverses that ranking (2 vs
//!   1.5), wrongly rejects both under the 5 + sqrt(5*95/100) = 7.179449 budget.
//!   Ranking by added errors collapses Age alone, ending at seven errors.
//! - GAP baseline learner after P+R5 (2026-10-20): wn100 has 16 training errors
//!   versus Modeler's 22. Two extra subtrees remain: b<=7.3 at c3=T / b>2.1 /
//!   c1=F / c2=F, and c4 (with d<=47.605 below it) at c3=F / c2=T.
//!   The oracle is not a fixed point of CF=.25 leaf-versus-subtree pruning: at
//!   c3=T / b>2.1 (46 records, 36/10), leaf bound 12.536923 is below the retained
//!   subtree's 13.788497. No data-specific budget or pruning exception is used.
//! - `auto_train/ac_*`: all 16 saved C5.0 component trees still differ: 45/360
//!   errors here versus Modeler's 43/360. The largest-internal-child rule fixes
//!   the 44-record age<=32 / income<=90 replacement and eight-record collapse.
//!   Global pruning now also merges east+north at income<=46.5 / age>37.5 /
//!   member=Y, removing Modeler's six-record income<=28 refinement. The two
//!   additional errors and 17 differing confidences remain a GAP.
//! - `c50_prune/wn23` and wn100_noglobal export a d<=47.605 stump on the
//!   23-record witness, outside every phase-3 frozen prediction; our c4 subtree
//!   has the same six training errors but different partitions/distributions.
//!   wn11 stays split in isolation and with global pruning off, whereas the
//!   global-on wn100 oracle collapses it. Further local raising/re-estimation
//!   and global selection remain unresolved; see target/c50d4/REPORT.md.
//! - Raising evidence: wn's root c1 gain ratio is 0.0909106225, whereas b's best
//!   direct cut is 3.95, ratio 0.0645687711. The oracle's b=3.85 exists below c1=T
//!   with endpoints 3.7/4.0. R5 promotes this test and regrows its descendants on
//!   all rows (800 exact scores). Redistributing counts alone keeps b=5.6 with 17
//!   rows instead of 5.94999999 with 21. The 15 discriminating trees select P+R5
//!   from the frozen 70-cell family; wn100 and Auto Classifier still need more.
//!   Earlier investigations: `target/c50wn/REPORT.md`, `target/c50d2/REPORT.md`.
//! - A global budget based on summed leaf error bounds (27.754211 before global
//!   pruning in the raising experiment, oracle 32.059979) overprunes wn100 and
//!   breaks `c50_globalprune/c50_gf16_58a`, `models_ml/c50_min_child`, and drug.
//!   Counting all distinct numeric gaps, including ineligible child sizes, or
//!   using max(min_child, min(25, .1*N/classes)) does not resolve wn100 either.
//!   These variants are not retained. All nine datasets' stream inputs equal
//!   exported Type records numerically; loading different rows is not the cause.
//! - GAP winnowing: examined all 19 presets winnowing trees plus models_ml/c50_winnow.
//!   Exhaustively retraining every nonempty input subset with each case's pruning
//!   settings gives 2,900 trials. Only 12/20 oracles admit any exact subset. None
//!   matches drugtwice (all 3 settings), wn (all 3), or wn100 (default/noglobal).
//!   Therefore a field-elimination rule alone cannot reproduce every oracle with
//!   this learner. Keep winnowing rejected; do not install a data-specific selector.
//! - Dropping only fields unused in the final unwinnowed tree is insufficient:
//!   drug uses Sex but winnowing removes it; wn100 uses no a before winnowing but
//!   does afterwards (11 vs 20 nodes). PMML active fields describe final splits,
//!   not the full candidate set surviving selection. For example all eight subsets
//!   with/without e1/e2/e3 and without Sex reproduce drugnoise, so those exports
//!   cannot establish whether the three noise fields were eliminated.
//! - A monotone threshold on a field's collapsed-subtree error contribution, even
//!   relative to N, fails: in the 100-row drughalf ORACLE, collapsing Age adds 9
//!   training errors (CF=.25 bound increase 7.216168), versus Cholesterol's 5
//!   (3.999582), yet winnowing removes Age and retains Cholesterol. wn100 retains
//!   c1 although collapsing it decreases that bound by 1.251574. These bounds sum
//!   leaf estimates before/after collapsing every split on the indicated field.
//! - Best tested partial rule: independently omit each field, retrain, then remove
//!   fields whose omission does not increase total training error (or the sum of
//!   CF=.25 leaf error bounds). Both reproduce only 8/20 exact trees: drug,
//!   drugnoise at all three settings, grid, noisy, gw, and models_ml/c50_winnow.
//!   In our drughalf baseline, dropping Age increases errors 5->8 (bound
//!   17.851101->21.062915), Na 5->11 (21.268205); the oracle drops both. In our
//!   wn100 baseline, dropping a improves errors 19->16 (bound 30.017739->28.721417),
//!   yet the winnowed oracle uses a. These retraining numbers describe OUR learner,
//!   whose remaining baseline GAPs must be resolved before inferring a selection rule.
//! - Partial subset evidence, not an algorithm: excluding Sex exactly reproduces
//!   drug and drugnoise; retaining K/BP/Cholesterol (with or without unused Sex)
//!   reproduces all three drughalf winnowing settings. Pruning variants do not
//!   disambiguate those sets; wn100 severity 90 admits 12 different exact subsets.
//!   Sampling, a different preliminary tree, and selection-specific pruning remain
//!   unsettled. Recorded final classifiers cannot identify those internal choices.
//!
//! Rejected settings (never silently substituted): boosting/trials
//! (`c50_boost_tree`, `c50_boost_rules`: reweighting settled, weighted trees GAP; see `c50_boost`),
//! costs combined with weights/rules/boosting (no combined oracle), case/frequency
//! weights (`c50_weight`: GAP pruning and internal distributions), winnowing
//! (`c50_presets`: GAP elimination criterion and wn100 baseline),
//! unrecorded Simple presets/combinations (scope above), adjusted-propensity
//! training (the nugget scores its saved test-partition tree, `exec::nodes::c50`;
//! the builder does not yet write one), cross-validation, split-field training
//! and missing/unknown training predictors (no captured c50_missing oracle). PMML
//! scoring supports rules and boosts independently.
//!
//! Follow-up probes (specifications only; none run here):
//! - Copy the 384 rows of models_ml/tree_train.csv. Keep inputs x,cat,flag,ord,code,
//!   constant,noise; y nominal A/B/C; disable partitioning and boosting; expert
//!   min-child=2, severity=75. Costs default=1, B->C=8, C->B=2. Export one contrast
//!   at a time removing constant, adding 1/4/16 constant copies, then removing ord.
//!   This separates gain-denominator hypotheses without changing the records.
//! - Same data/settings, costs off: swap ord/code input order; then change only
//!   ord's measurement to nominal. This distinguishes tie preference/penalty.
//! - Same 384 rows, costs off, weightField=weight, useWeight=true: toggle only
//!   global pruning; repeat with weights multiplied by 10, then all weights=1.
//!   Export both PMML and training scores; inspect nodes by path, not numeric id.
//!   Also isolate the 12 red/code=3/flag=T rows with x in {4,6,8,10,12,14}:
//!   two rows per x, labels C/(A,C)/C/A/A/A respectively, weight=384/494 each.
//!   Compare this alone and with the remaining 372 rows restored to distinguish
//!   local pruning from a tree-wide error budget. Re-export the full weighted tree
//!   with row order reversed to test the zero-distribution/rounding anomalies.
//! - For numeric output precision, x={1.0000001,1.0000002}, ten A then ten B,
//!   y nominal, minimum child=2, costs/weights/boosting off; inspect threshold.

use crate::{AlgoError, Dataset, categories, data_dictionary, mining_schema};
use dms_pmml::{
    MiningFunction, MissingValueStrategy, Model, ModelHeader, NoTrueChildStrategy, Node, Operator,
    Pmml, Predicate, ScoreDistribution, TreeModel,
};
use dms_types::{Measurement, Value};

const MAX_ROWS: usize = 100_000;
const MAX_INPUTS: usize = 256;
const MAX_CATEGORIES: usize = 1024;
const MAX_NODES: usize = 10_000;
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct C50Settings {
    pub min_child_records: usize,
    pub pruning_severity: f64,
    pub global_pruning: bool,
    pub group_symbolics: bool,
    /// Diagnostic case weights; the exec node rejects these until the GAPs above close.
    pub weight_field: Option<usize>,
    /// Explicit actual/predicted costs; absent off-diagonal entries use default_cost.
    pub costs: Option<MisclassificationCosts>,
    /// A rule set instead of a tree (`modelFormat` `ruleSet`): [`crate::c50_rules`].
    pub rules: bool,
    /// Boosting trials (`useBoosting` with `boostingTrials`): [`crate::c50_boost`].
    pub boost_trials: Option<usize>,
}

impl Default for C50Settings {
    fn default() -> Self {
        Self {
            min_child_records: 2,
            pruning_severity: 75.0,
            global_pruning: true,
            group_symbolics: false,
            weight_field: None,
            costs: None,
            rules: false,
            boost_trials: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MisclassificationCosts {
    pub default_cost: f64,
    pub entries: Vec<(String, String, f64)>,
}

impl MisclassificationCosts {
    fn matrix(&self, classes: &[String]) -> Result<Vec<Vec<f64>>, AlgoError> {
        if !self.default_cost.is_finite() || self.default_cost <= 0.0 {
            return Err(AlgoError::Settings(
                "C5.0 default cost must be finite and positive".into(),
            ));
        }
        if self.entries.len() > MAX_NODES {
            return Err(AlgoError::Unsupported("C5.0 cost entry limit".into()));
        }
        let mut matrix = vec![vec![self.default_cost; classes.len()]; classes.len()];
        for (i, row) in matrix.iter_mut().enumerate() {
            if let Some(diagonal) = row.get_mut(i) {
                *diagonal = 0.0;
            }
        }
        let mut seen = std::collections::HashSet::new();
        for (actual, predicted, cost) in &self.entries {
            if !cost.is_finite() || *cost < 0.0 || (actual == predicted && *cost != 0.0) {
                return Err(AlgoError::Settings(
                    "C5.0 costs must be finite, nonnegative, and zero on the diagonal".into(),
                ));
            }
            let from = classes.iter().position(|s| s == actual);
            let to = classes.iter().position(|s| s == predicted);
            let (Some(from), Some(to)) = (from, to) else {
                return Err(AlgoError::Settings(
                    "C5.0 cost refers to an unknown target category".into(),
                ));
            };
            if !seen.insert((from, to)) {
                return Err(AlgoError::Settings("C5.0 duplicate cost entry".into()));
            }
            if let Some(cell) = matrix.get_mut(from).and_then(|r| r.get_mut(to)) {
                *cell = *cost;
            }
        }
        if matrix
            .iter()
            .flatten()
            .any(|c| *c > f64::MAX / (MAX_ROWS as f64 * MAX_CATEGORIES as f64))
        {
            return Err(AlgoError::Settings("C5.0 costs are too large".into()));
        }
        Ok(matrix)
    }
}

enum Column {
    Numeric {
        name: String,
        values: Vec<f64>,
    },
    Symbolic {
        name: String,
        values: Vec<usize>,
        labels: Vec<String>,
        flag: bool,
        ordinal: bool,
    },
}

struct Data {
    columns: Vec<Column>,
    classes: Vec<String>,
    targets: Vec<usize>,
    /// Case weights scaled to sum to the number of records (all 1 without a weight field).
    weights: Vec<f64>,
    costs: Option<Vec<Vec<f64>>>,
}

impl Data {
    fn read(dataset: &Dataset<'_>, weight_field: Option<usize>) -> Result<Self, AlgoError> {
        let fields = dataset.table.model().fields();
        let target = fields
            .get(dataset.target)
            .ok_or_else(|| AlgoError::Data("invalid C5.0 target index".into()))?;
        if target.measurement == Measurement::Continuous {
            return Err(AlgoError::Data("C5.0 needs a categorical target".into()));
        }
        if dataset.table.row_count() > MAX_ROWS || dataset.inputs.len() > MAX_INPUTS {
            return Err(AlgoError::Unsupported(format!(
                "C5.0 is limited to {MAX_ROWS} records and {MAX_INPUTS} inputs"
            )));
        }
        let classes = categories(dataset, dataset.target);
        if classes.is_empty() || classes.len() > MAX_CATEGORIES {
            return Err(AlgoError::Data(
                "C5.0 target has no categories or too many categories".into(),
            ));
        }
        let mut row_indices = Vec::new();
        let mut targets = Vec::new();
        for (i, row) in dataset.table.rows().iter().enumerate() {
            let Some(value) = row.get(dataset.target).filter(|v| !target.is_missing(v)) else {
                continue;
            };
            let text = value.to_string();
            let class = classes
                .iter()
                .position(|c| c == &text)
                .ok_or_else(|| AlgoError::Data(format!("C5.0 unknown target {text:?}")))?;
            row_indices.push(i);
            targets.push(class);
        }
        if targets.is_empty() {
            return Err(AlgoError::Data("C5.0 has no valid targets".into()));
        }
        let weights = match weight_field {
            None => vec![1.0; targets.len()],
            Some(position) => {
                let raw = row_indices
                    .iter()
                    .map(
                        |&i| match dataset.table.rows().get(i).and_then(|r| r.get(position)) {
                            Some(Value::Integer(n)) if *n > 0 => Ok(*n as f64),
                            Some(Value::Real(w)) if w.is_finite() && *w > 0.0 => Ok(*w),
                            _ => Err(AlgoError::Unsupported(
                                "C5.0 case weights that are missing or not positive".into(),
                            )),
                        },
                    )
                    .collect::<Result<Vec<f64>, _>>()?;
                // `c50_weighted/c50_w2_default`: weights keep six significant digits
                // (3.9285714 -> 3.92857) before scaling; a leaf of one such case
                // exports 3.9285701.
                let raw: Vec<f64> = raw
                    .into_iter()
                    .map(|w| format!("{w:.5e}").parse::<f64>().unwrap_or(w))
                    .collect();
                let total = raw.iter().sum::<f64>();
                if !total.is_finite() {
                    return Err(AlgoError::Data("C5.0 case-weight sum overflow".into()));
                }
                let scale = raw.len() as f64 / total;
                let scaled: Vec<f64> = raw.into_iter().map(|w| w * scale).collect();
                if scaled.iter().any(|w| !w.is_finite() || *w <= 0.0) {
                    return Err(AlgoError::Data(
                        "C5.0 case-weight normalization overflow or underflow".into(),
                    ));
                }
                scaled
            }
        };
        let mut columns = Vec::new();
        for &position in dataset.inputs {
            if position == dataset.target {
                return Err(AlgoError::Settings("C5.0 target is also an input".into()));
            }
            let field = fields
                .get(position)
                .ok_or_else(|| AlgoError::Data("invalid C5.0 input index".into()))?;
            let values = row_indices
                .iter()
                .map(|&i| {
                    dataset
                        .table
                        .rows()
                        .get(i)
                        .and_then(|r| r.get(position))
                        .filter(|v| !field.is_missing(v))
                        .ok_or_else(|| {
                            AlgoError::Unsupported(format!(
                                "C5.0 missing training predictor {}",
                                field.name
                            ))
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if field.measurement == Measurement::Continuous {
                let numbers = values
                    .into_iter()
                    .map(|v| match v {
                        Value::Integer(n) => Ok(*n as f64),
                        Value::Real(n) if n.is_finite() => Ok(*n),
                        _ => Err(AlgoError::Data(format!(
                            "C5.0 input {} is not finite numeric data",
                            field.name
                        ))),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                columns.push(Column::Numeric {
                    name: field.name.clone(),
                    values: numbers,
                });
            } else {
                let mut labels = categories(dataset, position);
                if labels.len() > MAX_CATEGORIES {
                    return Err(AlgoError::Unsupported(
                        "C5.0 predictor category limit".into(),
                    ));
                }
                let flag = field.measurement == Measurement::Flag;
                if flag {
                    labels.reverse();
                }
                let codes = values
                    .iter()
                    .map(|v| {
                        labels
                            .iter()
                            .position(|s| s == &v.to_string())
                            .ok_or_else(|| {
                                AlgoError::Unsupported(format!(
                                    "C5.0 unknown training category in {}",
                                    field.name
                                ))
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                columns.push(Column::Symbolic {
                    name: field.name.clone(),
                    values: codes,
                    labels,
                    flag,
                    ordinal: field.measurement == Measurement::Ordinal,
                });
            }
        }
        Ok(Self {
            columns,
            classes,
            targets,
            weights,
            costs: None,
        })
    }

    /// The weighted number of records in `rows`.
    fn weight(&self, rows: &[usize]) -> f64 {
        // Not `sum()`: an empty branch must weigh 0, not -0.
        rows.iter()
            .filter_map(|&r| self.weights.get(r))
            .fold(0.0, |a, b| a + b)
    }

    fn counts(&self, rows: &[usize]) -> Vec<f64> {
        let mut counts = vec![0.0; self.classes.len()];
        for &row in rows {
            if let Some(c) = self.targets.get(row).and_then(|&i| counts.get_mut(i)) {
                *c += self.weights.get(row).copied().unwrap_or(0.0);
            }
        }
        counts
    }
}

fn entropy(counts: &[f64]) -> f64 {
    let total: f64 = counts.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    counts
        .iter()
        .filter(|&&c| c > 0.0)
        .map(|c| {
            let p = c / total;
            -p * p.log2()
        })
        .sum()
}

/// Recorded PMML uses eight fractional decimal places, floored rather than rounded.
/// Binary64 scaling matters: drugtwice's midpoint 0.040038 exports 0.04003799.
/// Of 312 numeric predicates in the five recorded tree families, this reproduces
/// 308 from the final node's endpoints. Candidate endpoints 3.7/4.0 also explain
/// the four wn roots (final endpoints 3.8/3.9); the unwinnowed root is raised by R5.
/// Overflow when scaling and closely spaced inputs must not change the partition.
fn numeric_cut(lower: f64, upper: f64) -> f64 {
    let midpoint = lower / 2.0 + upper / 2.0;
    let cut = if midpoint >= lower && midpoint < upper {
        midpoint
    } else {
        lower
    };
    let rounded = (cut * 1e8).floor() / 1e8;
    if rounded.is_finite() && rounded >= lower && rounded < upper {
        rounded
    } else {
        cut
    }
}

struct Split {
    /// Local corrected gain, used by both the average-gain gate and gain ratio.
    gain: f64,
    ratio: f64,
    groups: Vec<(Predicate, Vec<usize>)>,
}

fn candidate(
    data: &Data,
    rows: &[usize],
    column: &Column,
    min_child: usize,
    grouping: bool,
) -> Option<Split> {
    let parent_counts = data.counts(rows);
    let base = entropy(&parent_counts);
    let total = data.weight(rows);
    let min_child = min_child as f64;
    match column {
        Column::Numeric { name, values } => {
            let mut sorted: Vec<(f64, usize, f64)> = rows
                .iter()
                .filter_map(|&r| {
                    let w = data.weights.get(r).copied()?;
                    values
                        .get(r)
                        .zip(data.targets.get(r))
                        .map(|(&v, &c)| (v, c, w))
                })
                .collect();
            sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut left = vec![0.0; data.classes.len()];
            let mut right = parent_counts;
            let mut best: Option<(f64, f64, f64, f64)> = None;
            let mut cuts = 0usize;
            let mut n = 0.0;
            for pair in sorted.windows(2) {
                let [(value, class, weight), (next, _, _)] = pair else {
                    continue;
                };
                if let Some(c) = left.get_mut(*class) {
                    *c += weight;
                }
                if let Some(c) = right.get_mut(*class) {
                    *c -= weight;
                }
                n += weight;
                if value == next || n < min_child || total - n < min_child {
                    continue;
                }
                cuts = cuts.saturating_add(1);
                let gain =
                    base - n / total * entropy(&left) - (total - n) / total * entropy(&right);
                if best.is_none_or(|(g, _, _, _)| gain > g + 1e-12) {
                    let cut = numeric_cut(*value, *next);
                    best = Some((gain, cut, n, next - value));
                }
            }
            let (raw, cut, n, gap) = best?;
            // Encoding the threshold within its observed range can be cheaper
            // than identifying one of every eligible gap (drughalf/drugtwice).
            let range = sorted.last()?.0 - sorted.first()?.0;
            let trials = (cuts.max(1) as f64).min(range / gap).max(1.0);
            let gain = raw - trials.log2() / total;
            let split_info = entropy(&[n, total - n]);
            if split_info <= 0.0 {
                return None;
            }
            let (left, right): (Vec<_>, Vec<_>) = rows
                .iter()
                .copied()
                .partition(|&r| values.get(r).is_some_and(|v| *v <= cut));
            Some(Split {
                gain,
                ratio: gain / split_info,
                groups: vec![
                    (
                        Predicate::Simple {
                            field: name.clone(),
                            operator: Operator::LessOrEqual,
                            value: Some(cut.to_string()),
                        },
                        left,
                    ),
                    (
                        Predicate::Simple {
                            field: name.clone(),
                            operator: Operator::GreaterThan,
                            value: Some(cut.to_string()),
                        },
                        right,
                    ),
                ],
            })
        }
        Column::Symbolic {
            name,
            values,
            labels,
            flag,
            ordinal,
        } => {
            let mut groups: Vec<(Vec<usize>, Vec<usize>)> = labels
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    (
                        vec![i],
                        rows.iter()
                            .copied()
                            .filter(|&r| values.get(r) == Some(&i))
                            .collect(),
                    )
                })
                .collect();
            // A plain multiway split has a branch for every declared value, an empty one
            // predicting the parent's class (`c50_globalprune/c50_gp_copies1_small`:
            // `g in {g02, g21}` with no records).
            if *flag || grouping {
                groups.retain(|(_, rs)| !rs.is_empty());
            } else if labels.len() as f64 > MANY_VALUES * (data.targets.len() as f64 - 4.0) {
                return None;
            }
            if *ordinal && !flag {
                return ordinal_cut(data, name, labels, groups, base, total, min_child);
            }
            // Without grouping every symbol is its own branch; same-class leaves merge only
            // after pruning (`merge_leaves`).
            if grouping && !flag {
                merge_groups(data, &mut groups, grouping);
            }
            if groups
                .iter()
                .filter(|(_, rs)| data.weight(rs) >= min_child)
                .count()
                < 2
            {
                return None;
            }
            let (gain, ratio) = group_quality(data, &groups, base, total);
            let sets = !flag && (grouping || groups.iter().any(|(labels, _)| labels.len() > 1));
            let groups = groups
                .into_iter()
                .map(|(codes, rs)| {
                    let symbols: Vec<String> = codes
                        .iter()
                        .filter_map(|&c| labels.get(c).cloned())
                        .collect();
                    let pred = if sets {
                        Predicate::SimpleSet {
                            field: name.clone(),
                            is_in: true,
                            values: symbols,
                        }
                    } else {
                        Predicate::Simple {
                            field: name.clone(),
                            operator: Operator::Equal,
                            value: symbols.first().cloned(),
                        }
                    };
                    (pred, rs)
                })
                .collect();
            Some(Split {
                gain,
                ratio,
                groups,
            })
        }
    }
}

/// An ordinal field splits in two at the cut of highest gain over its values in sorted
/// order (`c50_weighted`, `c50_boost`: `ord in {high, low}` versus `{middle}`). Like a
/// numeric threshold the cut costs log2(cuts) / total of gain (`c50_costs`: `code` beats
/// the uncorrected `ord` cut).
fn ordinal_cut(
    data: &Data,
    name: &str,
    labels: &[String],
    mut groups: Vec<(Vec<usize>, Vec<usize>)>,
    base: f64,
    total: f64,
    min_child: f64,
) -> Option<Split> {
    groups.sort_by(|(a, _), (b, _)| {
        let label = |codes: &Vec<usize>| codes.first().and_then(|&c| labels.get(c));
        label(a).cmp(&label(b))
    });
    let side = |part: &[(Vec<usize>, Vec<usize>)]| -> (Vec<usize>, Vec<usize>) {
        (
            part.iter().flat_map(|(c, _)| c.iter().copied()).collect(),
            part.iter().flat_map(|(_, r)| r.iter().copied()).collect(),
        )
    };
    let mut best: Option<(f64, f64, usize)> = None;
    let mut cuts = 0usize;
    for cut in 1..groups.len() {
        // Values absent from the node stay in the cut's sorted range, a cut following
        // a present value (`c50_weighted/c50_rc12_loose`: `{high, low}` with no `high`;
        // `c50_rc13_loose`: `{low, middle}` with no `middle`).
        if groups.get(cut - 1).is_none_or(|(_, rs)| rs.is_empty()) {
            continue;
        }
        let (low, high) = groups.split_at(cut);
        let two = [side(low), side(high)];
        if two.iter().any(|(_, rs)| data.weight(rs) < min_child) {
            continue;
        }
        cuts = cuts.saturating_add(1);
        let (gain, _) = group_quality(data, &two, base, total);
        if best.is_none_or(|(g, _, _)| gain > g + 1e-12) {
            let info = entropy(
                &two.iter()
                    .map(|(_, rs)| data.weight(rs))
                    .collect::<Vec<_>>(),
            );
            best = Some((gain, info, cut));
        }
    }
    let (raw, info, cut) = best?;
    let trials = cuts.max(1) as f64;
    let gain = raw - trials.log2() / total;
    if info <= 0.0 {
        return None;
    }
    let (low, high) = groups.split_at(cut);
    let branch = |part: &[(Vec<usize>, Vec<usize>)]| {
        let (codes, rows) = side(part);
        let values = codes
            .iter()
            .filter_map(|&i| labels.get(i).cloned())
            .collect();
        let predicate = Predicate::SimpleSet {
            field: name.to_string(),
            is_in: true,
            values,
        };
        (predicate, rows)
    };
    Some(Split {
        gain,
        ratio: gain / info,
        groups: vec![branch(low), branch(high)],
    })
}

fn group_quality(
    data: &Data,
    groups: &[(Vec<usize>, Vec<usize>)],
    base: f64,
    total: f64,
) -> (f64, f64) {
    let gain = base
        - groups
            .iter()
            .map(|(_, rs)| data.weight(rs) / total * entropy(&data.counts(rs)))
            .sum::<f64>();
    let info = entropy(
        &groups
            .iter()
            .map(|(_, rs)| data.weight(rs))
            .collect::<Vec<_>>(),
    );
    (gain, if info > 0.0 { gain / info } else { 0.0 })
}

fn merge_groups(data: &Data, groups: &mut Vec<(Vec<usize>, Vec<usize>)>, grouping: bool) {
    if groups.len() > 64 {
        return;
    }
    let total: f64 = groups.iter().map(|(_, rs)| data.weight(rs)).sum();
    let mut all = Vec::new();
    for (_, rows) in groups.iter() {
        all.extend(rows);
    }
    let base = entropy(&data.counts(&all));
    for _ in 0..MAX_CATEGORIES {
        if groups.len() <= 2 {
            break;
        }
        let current = group_quality(data, groups, base, total).1;
        let mut best = None;
        for i in 0..groups.len() {
            for j in i.saturating_add(1)..groups.len() {
                let Some((_, a)) = groups.get(i) else {
                    continue;
                };
                let Some((_, b)) = groups.get(j) else {
                    continue;
                };
                if !grouping {
                    let ca = data.counts(a);
                    let cb = data.counts(b);
                    let (wa, wb) = (data.weight(a), data.weight(b));
                    let pure_a = ca.iter().position(|c| *c == wa);
                    let pure_b = cb.iter().position(|c| *c == wb);
                    if pure_a.is_none() || pure_a != pure_b {
                        continue;
                    }
                }
                let mut trial = groups.clone();
                let (codes, rows) = trial.remove(j);
                if let Some((a, b)) = trial.get_mut(i) {
                    a.extend(codes);
                    b.extend(rows);
                }
                let quality = group_quality(data, &trial, base, total).1;
                if quality > current + 1e-12
                    && best.as_ref().is_none_or(|(_, _, q)| quality > *q + 1e-12)
                {
                    best = Some((i, j, quality));
                }
            }
        }
        let Some((i, j, _)) = best else {
            break;
        };
        let (codes, rows) = groups.remove(j);
        if let Some((a, b)) = groups.get_mut(i) {
            a.extend(codes);
            a.sort_unstable();
            b.extend(rows);
        }
    }
}

fn winner(counts: &[f64], parent: usize) -> usize {
    let max = counts.iter().copied().fold(0.0, f64::max);
    if counts.get(parent) == Some(&max) {
        parent
    } else {
        counts.iter().position(|c| *c == max).unwrap_or(0)
    }
}

fn cost_winner(counts: &[f64], parent: usize, costs: Option<&[Vec<f64>]>) -> usize {
    let Some(costs) = costs else {
        return winner(counts, parent);
    };
    let losses: Vec<f64> = (0..counts.len())
        .map(|predicted| {
            counts
                .iter()
                .enumerate()
                .map(|(actual, count)| {
                    count
                        * costs
                            .get(actual)
                            .and_then(|r| r.get(predicted))
                            .copied()
                            .unwrap_or(0.0)
                })
                .sum()
        })
        .collect();
    let best = losses.iter().copied().fold(f64::INFINITY, f64::min);
    if losses.get(parent) == Some(&best) {
        parent
    } else {
        losses.iter().position(|c| *c == best).unwrap_or(0)
    }
}

fn grow(
    data: &Data,
    rows: &[usize],
    settings: &C50Settings,
    parent: usize,
    depth: usize,
    budget: &mut usize,
) -> Result<Node, AlgoError> {
    if depth > MAX_DEPTH || *budget >= MAX_NODES {
        return Err(AlgoError::Unsupported(
            "C5.0 tree depth/node limit exceeded".into(),
        ));
    }
    *budget = budget.saturating_add(1);
    let counts = data.counts(rows);
    let class = cost_winner(&counts, parent, data.costs.as_deref());
    let total = data.weight(rows);
    let distribution = data
        .classes
        .iter()
        .zip(&counts)
        .map(|(value, &count)| ScoreDistribution {
            value: value.clone(),
            record_count: count,
            probability: Some(if total > 0.0 { count / total } else { 0.0 }),
            confidence: Some(if count > 0.0 {
                (count + 1.0) / (total + data.classes.len() as f64)
            } else {
                0.0
            }),
        })
        .collect();
    let mut node = Node {
        id: None,
        score: data.classes.get(class).cloned(),
        record_count: Some(total),
        default_child: None,
        predicate: Predicate::True,
        distribution,
        children: Vec::new(),
    };
    if counts.iter().filter(|&&c| c > 0.0).count() < 2
        || total < settings.min_child_records as f64 * 2.0
    {
        return Ok(node);
    }
    // Of equal gain ratios the field first in input order wins.
    let candidates: Vec<Split> = data
        .columns
        .iter()
        .filter_map(|col| {
            candidate(
                data,
                rows,
                col,
                settings.min_child_records,
                settings.group_symbolics,
            )
        })
        .collect();
    // P: only positive corrected gains enter the mean; compare that same gain
    // with the mean before maximizing gain ratio. Costs retain their recorded
    // bypass (`c50_costs` node 5 grows `x`).
    let average = if data.costs.is_some() {
        f64::NEG_INFINITY
    } else {
        let mut positive = 0usize;
        let mut sum = 0.0;
        for split in &candidates {
            if split.gain > 1e-10 {
                positive = positive.saturating_add(1);
                sum += split.gain;
            }
        }
        if positive == 0 {
            f64::INFINITY
        } else {
            sum / positive as f64
        }
    };
    let mut best: Option<Split> = None;
    for split in candidates {
        if split.gain > 1e-10
            && split.gain >= average - 1e-9
            && best.as_ref().is_none_or(|s| split.ratio > s.ratio + 1e-12)
        {
            best = Some(split);
        }
    }
    if let Some(split) = best {
        for (predicate, group) in split.groups {
            let mut child = grow(data, &group, settings, class, depth + 1, budget)?;
            child.predicate = predicate;
            node.children.push(child);
        }
    }
    Ok(node)
}

fn validate_settings(dataset: &Dataset<'_>, settings: &C50Settings) -> Result<(), AlgoError> {
    if settings.costs.is_some()
        && (settings.rules || settings.boost_trials.is_some() || settings.weight_field.is_some())
    {
        return Err(AlgoError::Unsupported(
            "C5.0 costs combined with rules, boosting, or case weights".into(),
        ));
    }
    if dataset.frequency.is_some() {
        return Err(AlgoError::Unsupported("C5.0 frequency weights".into()));
    }
    if settings.min_child_records == 0
        || settings.min_child_records > MAX_ROWS
        || !settings.pruning_severity.is_finite()
        || !(1.0..100.0).contains(&settings.pruning_severity)
    {
        return Err(AlgoError::Settings(
            "C5.0 minimum child records or pruning severity out of range".into(),
        ));
    }
    Ok(())
}

/// Pruned tree whose leaves become C5.0 rules. Growth and pruning match
/// [`train_tree`] (`c50_rules_voting`: 13 leaves, the same as `c50_default`).
/// GAP: C4.5 extracts from the unpruned tree; extra unpruned splits on this
/// fixture are not in the exported rules.
pub(crate) fn rule_tree(dataset: &Dataset<'_>, settings: &C50Settings) -> Result<Node, AlgoError> {
    validate_settings(dataset, settings)?;
    let data = Data::read(dataset, settings.weight_field)?;
    let rows: Vec<usize> = (0..data.targets.len()).collect();
    let mut budget = 0;
    let mut root = grow(&data, &rows, settings, 0, 0, &mut budget)?;
    prune(&data, &rows, &mut root, settings, 0, &mut budget)?;
    Ok(root)
}

/// One-sided Wilson upper bound with continuity correction; zero-error bound is
/// the exact binomial bound. Numerical inversion avoids an inverse-normal dependency.
/// Tree pruning and rule-condition dropping share this estimate (`c50_rules_voting`).
pub(crate) fn error_bound(total: f64, errors: f64, confidence: f64) -> f64 {
    if total <= 0.0 {
        return 0.0;
    }
    if errors <= 0.0 {
        return total * (1.0 - confidence.powf(1.0 / total));
    }
    if errors + 0.5 >= total {
        return total;
    }
    let mut low = 0.0;
    let mut high = 8.0;
    for _ in 0..64 {
        let z = (low + high) / 2.0;
        if normal_cdf(z) < 1.0 - confidence {
            low = z;
        } else {
            high = z;
        }
    }
    let z = (low + high) / 2.0;
    let p = (errors + 0.5) / total;
    total
        * (p + z * z / (2.0 * total)
            + z * (p * (1.0 - p) / total + z * z / (4.0 * total * total)).sqrt())
        / (1.0 + z * z / total)
}

fn normal_cdf(z: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.2316419 * z.abs());
    let tail = (-z * z / 2.0).exp() / (2.0 * std::f64::consts::PI).sqrt()
        * t
        * (0.319381530
            + t * (-0.356563782 + t * (1.781477937 + t * (-1.821255978 + t * 1.330274429))));
    if z >= 0.0 { 1.0 - tail } else { tail }
}

fn leaf_estimate(node: &Node, settings: &C50Settings, costs: Option<&[Vec<f64>]>) -> f64 {
    let total = node.record_count.unwrap_or(0.0);
    let correct = node
        .distribution
        .iter()
        .find(|d| Some(&d.value) == node.score.as_ref())
        .map_or(0.0, |d| d.record_count);
    let mut leaf = error_bound(
        total,
        total - correct,
        (100.0 - settings.pruning_severity) / 100.0,
    );
    if let Some(costs) = costs {
        let predicted = node
            .distribution
            .iter()
            .position(|d| Some(&d.value) == node.score.as_ref())
            .unwrap_or(0);
        let loss: f64 = node
            .distribution
            .iter()
            .enumerate()
            .map(|(actual, d)| {
                d.record_count
                    * costs
                        .get(actual)
                        .and_then(|r| r.get(predicted))
                        .copied()
                        .unwrap_or(0.0)
            })
            .sum();
        if total > correct {
            leaf *= loss / (total - correct);
        }
    }
    leaf
}

/// Estimate a fixed structure without pruning it (R5's selection estimate).
fn subtree_estimate(node: &Node, settings: &C50Settings, costs: Option<&[Vec<f64>]>) -> f64 {
    if node.children.is_empty() {
        leaf_estimate(node, settings, costs)
    } else {
        node.children
            .iter()
            .map(|child| subtree_estimate(child, settings, costs))
            .sum()
    }
}

/// The selected regrown candidate receives replacement and raising pruning,
/// without another regrowth pass. Iterating regrowth can recreate removed tests,
/// whereas raising without regrowth strictly shrinks the candidate tree.
fn prune_regrown(
    data: &Data,
    rows: &[usize],
    node: &mut Node,
    settings: &C50Settings,
    depth: usize,
) -> Result<f64, AlgoError> {
    if depth > MAX_DEPTH {
        return Err(AlgoError::Unsupported("C5.0 pruning depth limit".into()));
    }
    let leaf = leaf_estimate(node, settings, data.costs.as_deref());
    if node.children.is_empty() {
        return Ok(leaf);
    }
    let groups = branch_rows(data, rows, node);
    let mut subtree = 0.0;
    let mut largest = None;
    let mut largest_cases = 0;
    for (index, (child, group)) in node.children.iter_mut().zip(&groups).enumerate() {
        subtree += prune_regrown(data, group, child, settings, depth.saturating_add(1))?;
        if !child.children.is_empty() && (largest.is_none() || group.len() > largest_cases) {
            largest = Some(index);
            largest_cases = group.len();
        }
    }
    let mut raised = largest
        .and_then(|index| node.children.get(index))
        .filter(|_| depth < RAISING_DEPTH)
        .cloned();
    let parent = data
        .classes
        .iter()
        .position(|c| Some(c) == node.score.as_ref())
        .unwrap_or(0);
    let mut estimate = f64::INFINITY;
    if let Some(candidate) = &mut raised {
        redistribute(data, rows, candidate, parent, depth)?;
        estimate = subtree_estimate(candidate, settings, data.costs.as_deref());
    }
    if leaf <= subtree + LOCAL_SLACK && leaf <= estimate + LOCAL_SLACK {
        node.children.clear();
        return Ok(leaf);
    }
    if estimate <= subtree + LOCAL_SLACK
        && let Some(mut candidate) = raised
    {
        let result = prune_regrown(data, rows, &mut candidate, settings, depth)?;
        candidate.predicate = node.predicate.clone();
        *node = candidate;
        return Ok(result);
    }
    Ok(subtree)
}

fn branch_rows(data: &Data, rows: &[usize], node: &Node) -> Vec<Vec<usize>> {
    node.children
        .iter()
        .map(|child| {
            rows.iter()
                .copied()
                .filter(row_matcher(data, &child.predicate))
                .collect()
        })
        .collect()
}

/// Route all cases through inherited tests and recompute every node's prediction
/// and distribution. Ties retain the re-estimated parent's prediction.
fn redistribute(
    data: &Data,
    rows: &[usize],
    node: &mut Node,
    parent: usize,
    depth: usize,
) -> Result<(), AlgoError> {
    if depth > MAX_DEPTH {
        return Err(AlgoError::Unsupported(
            "C5.0 redistribution depth limit".into(),
        ));
    }
    let counts = data.counts(rows);
    let class = cost_winner(&counts, parent, data.costs.as_deref());
    node.score = data.classes.get(class).cloned();
    node.record_count = Some(data.weight(rows));
    for (distribution, count) in node.distribution.iter_mut().zip(counts) {
        distribution.record_count = count;
    }
    refresh_distribution(node);
    let groups = branch_rows(data, rows, node);
    for (child, group) in node.children.iter_mut().zip(groups) {
        redistribute(data, &group, child, class, depth.saturating_add(1))?;
    }
    Ok(())
}

/// Phase-1 R5 is bounded to the first twelve levels. This is a conservative
/// experiment scope limit, not a published claim about Modeler's maximum depth.
const RAISING_DEPTH: usize = 12;

fn prune(
    data: &Data,
    rows: &[usize],
    node: &mut Node,
    settings: &C50Settings,
    depth: usize,
    budget: &mut usize,
) -> Result<f64, AlgoError> {
    if depth > MAX_DEPTH {
        return Err(AlgoError::Unsupported("C5.0 pruning depth limit".into()));
    }
    let leaf = leaf_estimate(node, settings, data.costs.as_deref());
    if node.children.is_empty() {
        return Ok(leaf);
    }
    let groups = branch_rows(data, rows, node);
    let mut subtree = 0.0;
    let mut largest = None;
    let mut largest_cases = 0;
    for (index, (child, group)) in node.children.iter_mut().zip(&groups).enumerate() {
        subtree += prune(
            data,
            group,
            child,
            settings,
            depth.saturating_add(1),
            budget,
        )?;
        // Only internal children can supply a test to promote. Choose the
        // largest eligible original branch, retaining the first on ties.
        if !child.children.is_empty() && (largest.is_none() || group.len() > largest_cases) {
            largest = Some(index);
            largest_cases = group.len();
        }
    }
    let mut raised = largest
        .and_then(|index| node.children.get(index))
        .filter(|_| depth < RAISING_DEPTH)
        .cloned();
    let parent = data
        .classes
        .iter()
        .position(|c| Some(c) == node.score.as_ref())
        .unwrap_or(0);
    let mut estimate = f64::INFINITY;
    if let Some(candidate) = &mut raised {
        redistribute(data, rows, candidate, parent, depth)?;
        estimate = subtree_estimate(candidate, settings, data.costs.as_deref());
    }
    if leaf <= subtree + LOCAL_SLACK && leaf <= estimate + LOCAL_SLACK {
        node.children.clear();
        return Ok(leaf);
    }
    if estimate <= subtree + LOCAL_SLACK
        && let Some(mut candidate) = raised
    {
        let class = data
            .classes
            .iter()
            .position(|c| Some(c) == candidate.score.as_ref())
            .unwrap_or(parent);
        let promoted = branch_rows(data, rows, &candidate);
        for (child, group) in candidate.children.iter_mut().zip(promoted) {
            let predicate = child.predicate.clone();
            *child = grow(
                data,
                &group,
                settings,
                class,
                depth.saturating_add(1),
                budget,
            )?;
            child.predicate = predicate;
        }
        let result = prune_regrown(data, rows, &mut candidate, settings, depth)?;
        candidate.predicate = node.predicate.clone();
        *node = candidate;
        return Ok(result);
    }
    Ok(subtree)
}

/// A plain nominal field with more declared values than 0.3 x (training records - 4) is
/// never considered (`c50_globalprune/c50_gf*`: g with 8, 16, 32, 64 values first splits
/// at 31, 58, 112, 224 records, not at 30, 56, 108, 208, whatever the gain; `c50_gsub16_*`:
/// the training total counts, not the node's; `c50_gmin5_16_*`: minimum child 5 does not
/// move it). GAP: other linear fits of those four bounds remain possible.
const MANY_VALUES: f64 = 0.3;

/// Local pruning collapses a subtree whose error bound is not lower than its leaf's by
/// more than this (`c50_no_global_prune`, `c50_globalprune/*_noglobal`).
const LOCAL_SLACK: f64 = 0.1;

/// Bind a grown test once when partitioning rows. Field lookup and numeric
/// parsing are invariant across the records of a branch, including raised tests.
fn row_matcher<'a>(data: &'a Data, predicate: &'a Predicate) -> impl Fn(&usize) -> bool + 'a {
    let field = match predicate {
        Predicate::Simple { field, .. } | Predicate::SimpleSet { field, .. } => Some(field),
        _ => None,
    };
    let column = field.and_then(|field| {
        data.columns.iter().find(|c| match c {
            Column::Numeric { name, .. } | Column::Symbolic { name, .. } => name == field,
        })
    });
    let cut = match predicate {
        Predicate::Simple {
            operator: Operator::LessOrEqual | Operator::GreaterThan,
            value,
            ..
        } => value.as_deref().and_then(|v| v.parse::<f64>().ok()),
        _ => None,
    };
    move |&row| {
        let label = || match column {
            Some(Column::Symbolic { values, labels, .. }) => {
                values.get(row).and_then(|&code| labels.get(code))
            }
            _ => None,
        };
        match predicate {
            Predicate::True => true,
            Predicate::Simple {
                operator, value, ..
            } => match operator {
                Operator::LessOrEqual | Operator::GreaterThan => {
                    let x = match column {
                        Some(Column::Numeric { values, .. }) => values.get(row).copied(),
                        _ => None,
                    };
                    match (x, cut) {
                        (Some(x), Some(cut)) if *operator == Operator::LessOrEqual => x <= cut,
                        (Some(x), Some(cut)) => x > cut,
                        _ => false,
                    }
                }
                Operator::Equal => label().is_some_and(|l| Some(l) == value.as_ref()),
                _ => false,
            },
            Predicate::SimpleSet { is_in, values, .. } => {
                label().is_some_and(|l| values.contains(l)) == *is_in
            }
            _ => false,
        }
    }
}

/// Records of `rows` misclassified by `node` as a leaf, by count (not weight); under
/// misclassification costs each counts its cost (`c50_costs` keeps node 5's subtree).
fn record_errors(data: &Data, node: &Node, rows: &[usize]) -> f64 {
    let class = data
        .classes
        .iter()
        .position(|c| node.score.as_ref() == Some(c));
    rows.iter()
        .filter_map(|&r| data.targets.get(r))
        .map(|&actual| match (class, data.costs.as_deref()) {
            (Some(p), _) if p == actual => 0.0,
            (Some(p), Some(costs)) => costs
                .get(actual)
                .and_then(|row| row.get(p))
                .copied()
                .unwrap_or(1.0),
            _ => 1.0,
        })
        .sum()
}

/// (errors as a leaf, errors of its leaves, number of leaves), by record count.
fn subtree_errors(data: &Data, node: &Node, rows: &[usize]) -> (f64, f64, usize) {
    let leaf = record_errors(data, node, rows);
    if node.children.is_empty() {
        return (leaf, leaf, 1);
    }
    let mut errors = 0.0;
    let mut leaves = 0usize;
    for child in &node.children {
        let sub: Vec<usize> = rows
            .iter()
            .copied()
            .filter(row_matcher(data, &child.predicate))
            .collect();
        let (_, e, l) = subtree_errors(data, child, &sub);
        errors += e;
        leaves = leaves.saturating_add(l);
    }
    (leaf, errors, leaves)
}

/// The smallest total error increase from collapsing an internal node; equal
/// error increases prefer the subtree with more leaves (`auto_train/ac_*`).
fn weakest_subtree(data: &Data, node: &Node, rows: &[usize]) -> Option<(f64, usize)> {
    if node.children.is_empty() {
        return None;
    }
    let (leaf, sub, leaves) = subtree_errors(data, node, rows);
    let mut best = Some((leaf - sub, leaves));
    for child in &node.children {
        let sub: Vec<usize> = rows
            .iter()
            .copied()
            .filter(row_matcher(data, &child.predicate))
            .collect();
        if let Some(a) = weakest_subtree(data, child, &sub) {
            best = best.map(|b| {
                if a.0 < b.0 - 1e-9 || ((a.0 - b.0).abs() <= 1e-9 && a.1 > b.1) {
                    a
                } else {
                    b
                }
            });
        }
    }
    best
}

/// Collapse every subtree matching `(extra, target_leaves)`, outermost first.
fn collapse_subtrees(data: &Data, node: &mut Node, rows: &[usize], step: (f64, usize)) {
    if node.children.is_empty() {
        return;
    }
    let (leaf, sub, leaves) = subtree_errors(data, node, rows);
    if (leaf - sub - step.0).abs() <= 1e-9 && leaves == step.1 {
        node.children.clear();
        return;
    }
    for child in &mut node.children {
        let sub: Vec<usize> = rows
            .iter()
            .copied()
            .filter(row_matcher(data, &child.predicate))
            .collect();
        collapse_subtrees(data, child, &sub, step);
    }
}

/// Global pruning ranks whole subtrees by added training errors, without dividing
/// by removed leaves (`c50_presets/c50_drughalf_expert`: Age adds 2, Na/K adds 3),
/// and breaks equal-error ties by largest leaf count (`auto_train/ac_*`: the
/// four-branch `member = N` split goes before the two-branch `income <= 28` split).
/// Equal `(added_errors, leaves)` are tried together; cumulative errors must stay
/// within one standard error of the locally pruned tree's (`c50_globalprune`:
/// exception blocks correcting one record go, two stay, at every severity and with case
/// weights; `models/c50_default`: `BP = HIGH` goes; `c50_min_child`: the equal increases
/// `green`, `red` would reach 87 > 79 + 7.92 together and stay).
fn global_prune(data: &Data, root: &mut Node) {
    let rows: Vec<usize> = (0..data.targets.len()).collect();
    let n = rows.len() as f64;
    let start = subtree_errors(data, root, &rows).1;
    let limit = start + (start * (n - start) / n).max(0.0).sqrt();
    let mut trial = root.clone();
    let mut steps = Vec::new();
    while let Some(step) = weakest_subtree(data, &trial, &rows) {
        collapse_subtrees(data, &mut trial, &rows, step);
        if subtree_errors(data, &trial, &rows).1 > limit + 1e-9 {
            break;
        }
        steps.push(step);
    }
    for step in steps {
        collapse_subtrees(data, root, &rows, step);
    }
}

fn number(node: &mut Node, next: &mut usize) {
    node.id = Some(next.to_string());
    *next = next.saturating_add(1);
    let mut largest: Option<(f64, String)> = None;
    for child in &mut node.children {
        number(child, next);
        let count = child.record_count.unwrap_or(0.0);
        if largest.as_ref().is_none_or(|(n, _)| count > *n) {
            largest = child.id.as_ref().map(|id| (count, id.clone()));
        }
    }
    node.default_child = largest.map(|(_, id)| id);
}

fn merge_leaves(node: &mut Node) {
    for child in &mut node.children {
        merge_leaves(child);
    }
    let mut merged = false;
    let mut i = 0;
    while i < node.children.len() {
        let mut j = i.saturating_add(1);
        while j < node.children.len() {
            let pair = node.children.get(i).zip(node.children.get(j));
            let values = pair.and_then(|(a, b)| {
                // Empty branches merge only with each other (`c50_gp_copies1_small`: the
                // empty `g02`, `g21` stay apart from `g20`, which predicts the same).
                let empty = |n: &Node| n.record_count.unwrap_or(0.0) <= 0.0;
                if !a.children.is_empty()
                    || !b.children.is_empty()
                    || a.score != b.score
                    || empty(a) != empty(b)
                {
                    return None;
                }
                let (af, mut av) = symbolic_branch(&a.predicate)?;
                let (bf, bv) = symbolic_branch(&b.predicate)?;
                if af != bf {
                    return None;
                }
                av.extend(bv);
                Some((af, av))
            });
            if let Some((field, values)) = values {
                let b = node.children.remove(j);
                if let Some(a) = node.children.get_mut(i) {
                    a.predicate = Predicate::SimpleSet {
                        field,
                        is_in: true,
                        values,
                    };
                    let total = a.record_count.unwrap_or(0.0) + b.record_count.unwrap_or(0.0);
                    a.record_count = Some(total);
                    for (ad, bd) in a.distribution.iter_mut().zip(&b.distribution) {
                        ad.record_count += bd.record_count;
                    }
                    refresh_distribution(a);
                }
                merged = true;
            } else {
                j = j.saturating_add(1);
            }
        }
        i = i.saturating_add(1);
    }
    if merged {
        for child in &mut node.children {
            if let Some((field, values)) = symbolic_branch(&child.predicate) {
                child.predicate = Predicate::SimpleSet {
                    field,
                    is_in: true,
                    values,
                };
            }
        }
    }
}

fn symbolic_branch(predicate: &Predicate) -> Option<(String, Vec<String>)> {
    match predicate {
        Predicate::Simple {
            field,
            operator: Operator::Equal,
            value: Some(value),
        } => Some((field.clone(), vec![value.clone()])),
        Predicate::SimpleSet {
            field,
            is_in: true,
            values,
        } => Some((field.clone(), values.clone())),
        _ => None,
    }
}

fn refresh_distribution(node: &mut Node) {
    let total = node.record_count.unwrap_or(0.0);
    let classes = node.distribution.len() as f64;
    for d in &mut node.distribution {
        d.probability = Some(if total > 0.0 {
            d.record_count / total
        } else {
            0.0
        });
        d.confidence = Some(if d.record_count > 0.0 {
            (d.record_count + 1.0) / (total + classes)
        } else {
            0.0
        });
    }
}

/// Weighted exports: every distribution keeps eight significant digits of its own weighted
/// count (`c50_w2_default`: A 117.5032, not the children's 117.503197), a leaf's total is
/// the sum of those, an internal total sums its children's (`c50_weighted/*`: exact).
/// GAP: `c50_w2_loose` totals `red` (two leaves) 7.77688522, the class-by-class sum,
/// where the children's totals give 7.7768852200000005. `c50_weight`: the zero-
/// distribution subtree under `red, code = 1, x <= 11, x > 5` and node 28 (`code = 2`,
/// whose C count is the sum of its children's) are not reproduced.
fn finalize_weighted_counts(node: &mut Node) -> Result<(), AlgoError> {
    for d in &mut node.distribution {
        d.record_count = format!("{:.7e}", d.record_count)
            .parse::<f64>()
            .map_err(|e| AlgoError::Data(format!("C5.0 weighted count formatting: {e}")))?;
    }
    if node.children.is_empty() {
        node.record_count = Some(node.distribution.iter().map(|d| d.record_count).sum());
    } else {
        for child in &mut node.children {
            finalize_weighted_counts(child)?;
        }
        node.record_count = Some(node.children.iter().filter_map(|c| c.record_count).sum());
    }
    refresh_distribution(node);
    Ok(())
}

pub fn train(dataset: &Dataset<'_>, settings: &C50Settings) -> Result<Pmml, AlgoError> {
    if let Some(trials) = settings.boost_trials {
        return crate::c50_boost::train(dataset, settings, trials);
    }
    if settings.rules {
        return crate::c50_rules::train(dataset, settings);
    }
    train_tree(dataset, settings)
}

/// One decision tree (no boosting, no rule set). The exec node still rejects
/// `useWeight`: see the module GAPs.
pub fn train_tree(dataset: &Dataset<'_>, settings: &C50Settings) -> Result<Pmml, AlgoError> {
    validate_settings(dataset, settings)?;
    let mut data = Data::read(dataset, settings.weight_field)?;
    data.costs = settings
        .costs
        .as_ref()
        .map(|c| c.matrix(&data.classes))
        .transpose()?;
    let rows: Vec<usize> = (0..data.targets.len()).collect();
    let mut budget = 0;
    let mut root = grow(&data, &rows, settings, 0, 0, &mut budget)?;
    prune(&data, &rows, &mut root, settings, 0, &mut budget)?;
    if settings.global_pruning {
        global_prune(&data, &mut root);
    }
    merge_leaves(&mut root);
    if settings.weight_field.is_some() {
        finalize_weighted_counts(&mut root)?;
    }
    number(&mut root, &mut 0);
    Ok(Pmml {
        version: "4.3".into(),
        data_dictionary: data_dictionary(dataset),
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Tree(TreeModel {
            header: ModelHeader {
                name: None,
                function: MiningFunction::Classification,
                algorithm: Some("C5".into()),
                mining_schema: mining_schema(dataset),
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            missing_value_strategy: MissingValueStrategy::WeightedConfidence,
            no_true_child_strategy: NoTrueChildStrategy::ReturnNullPrediction,
            root,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dms_types::{DataModel, Field, Storage, Table, ValueSpec};

    fn sample() -> Table {
        let mut x = Field::new("x", Storage::Real);
        x.measurement = Measurement::Continuous;
        let mut y = Field::new("y", Storage::String);
        y.measurement = Measurement::Nominal;
        y.values = ValueSpec::Categories(vec![Value::string("a"), Value::string("b")]);
        Table::new(
            DataModel::new(vec![x, y]).unwrap(),
            (0..20)
                .map(|i| {
                    vec![
                        Value::Real(i as f64),
                        Value::string(if i < 10 { "a" } else { "b" }),
                    ]
                })
                .collect(),
        )
        .unwrap()
    }
    #[test]
    fn numeric_threshold_precision_preserves_partitions() {
        // Recorded endpoints: models/default, presets/drughalf and drugtwice.
        for (lower, upper, expected) in [
            (0.037124, 0.037539, 0.0373315),
            (0.722286, 0.747815, 0.7350505),
            (0.038171, 0.041905, 0.04003799),
        ] {
            assert_eq!(numeric_cut(lower, upper), expected);
        }
        // Scaling may overflow, or flooring may fall below the lower endpoint.
        // Even adjacent floats must remain separated, including subnormals.
        for (lower, upper) in [
            (1.0_f64.next_up(), 1.0_f64.next_up().next_up()),
            (f64::MAX.next_down(), f64::MAX),
            (-f64::MAX, -f64::MAX.next_down()),
            (-f64::MAX, f64::MAX),
            (f64::from_bits(1), f64::from_bits(2)),
            (-f64::from_bits(2), -f64::from_bits(1)),
        ] {
            let cut = numeric_cut(lower, upper);
            assert!(cut.is_finite() && cut >= lower && cut < upper);
        }
    }

    #[test]
    fn numeric_midpoint_and_smoothed_confidence() {
        let table = sample();
        let dataset = Dataset {
            table: &table,
            target: 1,
            inputs: &[0],
            frequency: None,
        };
        let pmml = train(&dataset, &C50Settings::default()).unwrap();
        let s = pmml.bind(&["x"]).unwrap();
        let p = s.score(&[dms_pmml::Value::Number(9.5)]).unwrap();
        assert_eq!(p.value, dms_pmml::Value::Text("a".into()));
        assert_eq!(p.confidence, Some(11.0 / 12.0));
        assert_eq!(
            s.score(&[dms_pmml::Value::Number(9.6)]).unwrap().value,
            dms_pmml::Value::Text("b".into())
        );
    }

    #[test]
    fn cost_direction_parent_ties_and_raw_counts() {
        let classes = vec!["A".into(), "B".into(), "C".into()];
        let costs = MisclassificationCosts {
            default_cost: 1.0,
            entries: vec![("B".into(), "C".into(), 8.0), ("C".into(), "B".into(), 2.0)],
        }
        .matrix(&classes)
        .unwrap();
        assert_eq!(
            costs,
            vec![
                vec![0.0, 1.0, 1.0],
                vec![1.0, 0.0, 8.0],
                vec![1.0, 2.0, 0.0]
            ]
        );
        assert_eq!(cost_winner(&[2.0, 2.0, 2.0], 1, Some(&costs)), 0);
        assert_eq!(cost_winner(&[2.0, 0.0, 2.0], 2, Some(&costs)), 2);
        assert_eq!(cost_winner(&[2.0, 0.0, 2.0], 1, Some(&costs)), 0);

        // A cheaper prediction may differ from the majority without changing
        // record counts or smoothed confidence.
        let table = sample();
        let pmml = train_tree(
            &Dataset {
                table: &table,
                target: 1,
                inputs: &[],
                frequency: None,
            },
            &C50Settings {
                costs: Some(MisclassificationCosts {
                    default_cost: 1.0,
                    entries: vec![("a".into(), "b".into(), 0.25)],
                }),
                ..C50Settings::default()
            },
        )
        .unwrap();
        let Model::Tree(tree) = pmml.model else {
            panic!("tree");
        };
        assert_eq!(tree.root.score.as_deref(), Some("b"));
        assert_eq!(tree.root.record_count, Some(20.0));
        assert_eq!(
            tree.root
                .distribution
                .iter()
                .map(|d| d.record_count)
                .collect::<Vec<_>>(),
            [10.0, 10.0]
        );
        assert_eq!(tree.root.distribution[1].confidence, Some(0.5));
    }

    #[test]
    fn malformed_costs_and_unverified_combinations_are_errors() {
        let table = sample();
        let dataset = Dataset {
            table: &table,
            target: 1,
            inputs: &[0],
            frequency: None,
        };
        for costs in [
            MisclassificationCosts {
                default_cost: f64::NAN,
                entries: vec![],
            },
            MisclassificationCosts {
                default_cost: 0.0,
                entries: vec![],
            },
            MisclassificationCosts {
                default_cost: 1.0,
                entries: vec![("a".into(), "b".into(), f64::INFINITY)],
            },
            MisclassificationCosts {
                default_cost: 1.0,
                entries: vec![("a".into(), "b".into(), -1.0)],
            },
            MisclassificationCosts {
                default_cost: 1.0,
                entries: vec![("a".into(), "b".into(), f64::MAX)],
            },
            MisclassificationCosts {
                default_cost: 1.0,
                entries: vec![("absent".into(), "b".into(), 1.0)],
            },
            MisclassificationCosts {
                default_cost: 1.0,
                entries: vec![("a".into(), "a".into(), 1.0)],
            },
            MisclassificationCosts {
                default_cost: 1.0,
                entries: vec![("a".into(), "b".into(), 2.0); 2],
            },
            MisclassificationCosts {
                default_cost: 1.0,
                entries: vec![("a".into(), "b".into(), 2.0); MAX_NODES + 1],
            },
        ] {
            assert!(
                train_tree(
                    &dataset,
                    &C50Settings {
                        costs: Some(costs),
                        ..C50Settings::default()
                    }
                )
                .is_err()
            );
        }
        for mut settings in [
            C50Settings {
                rules: true,
                ..C50Settings::default()
            },
            C50Settings {
                boost_trials: Some(2),
                ..C50Settings::default()
            },
            C50Settings {
                weight_field: Some(0),
                ..C50Settings::default()
            },
        ] {
            settings.costs = Some(MisclassificationCosts {
                default_cost: 1.0,
                entries: vec![],
            });
            assert!(matches!(
                train_tree(&dataset, &settings),
                Err(AlgoError::Unsupported(_))
            ));
        }
    }

    #[test]
    fn unrepresentable_case_weight_totals_return_errors() {
        let original = sample();
        let mut fields = original.model().fields().to_vec();
        fields.push(Field::new("weight", Storage::Real));
        for weight in [f64::MAX, f64::MIN_POSITIVE / 1000.0] {
            let rows = original
                .rows()
                .iter()
                .map(|r| {
                    let mut row = r.clone();
                    row.push(Value::Real(weight));
                    row
                })
                .collect();
            let table = Table::new(DataModel::new(fields.clone()).unwrap(), rows).unwrap();
            assert!(
                train_tree(
                    &Dataset {
                        table: &table,
                        target: 1,
                        inputs: &[0],
                        frequency: None
                    },
                    &C50Settings {
                        weight_field: Some(2),
                        ..C50Settings::default()
                    }
                )
                .is_err()
            );
        }
    }
    #[test]
    fn malformed_settings_and_indices_return_errors() {
        let table = sample();
        for target in [2, usize::MAX] {
            assert!(
                train(
                    &Dataset {
                        table: &table,
                        target,
                        inputs: &[0],
                        frequency: None
                    },
                    &C50Settings::default()
                )
                .is_err()
            );
        }
        for severity in [f64::NAN, f64::INFINITY, -1.0, 100.0] {
            assert!(
                train(
                    &Dataset {
                        table: &table,
                        target: 1,
                        inputs: &[0],
                        frequency: None
                    },
                    &C50Settings {
                        pruning_severity: severity,
                        ..C50Settings::default()
                    }
                )
                .is_err()
            );
        }
        assert!(
            train(
                &Dataset {
                    table: &table,
                    target: 1,
                    inputs: &[usize::MAX],
                    frequency: None
                },
                &C50Settings::default()
            )
            .is_err()
        );
    }
}
