//! QUEST (Algorithms Guide, "QUEST Algorithms", pp. 315-326; Loh & Shih 1997): a binary tree
//! whose field selection and split-point selection are separate. The field with the smallest
//! Bonferroni-adjusted p-value splits the node (ANOVA F for continuous and ordinal fields,
//! Pearson chi-square for nominal ones); the split point comes from quadratic discriminant
//! analysis, nominal fields through their discriminant coordinates (CRIMCOORD). Missing values
//! follow surrogate splits; cost-complexity pruning cuts the grown tree.
//!
//! The tree comes out the way Modeler writes it (oracle `quest_nofs/{quest_nofs_default,
//! quest_nofs_depth3, quest_nofs_unpruned, quest_nofs_binary}`; every case trains on all 48
//! records (`train_pct` 0) and scores 91 records: 48 train + 43 grid):
//! - Field selection: the ANOVA F p-value of a continuous or ordinal field (computed on the
//!   records valid for it, classes present at the node) or the Pearson chi-square p-value of a
//!   nominal one; the smallest p below `alpha/m` (`m` inputs) splits, else growth stops.
//!   `quest_nofs_default`: the root takes `z` (Modeler's node stats print F 20.0811574562907,
//!   adjP 3.17936284410791e-06 = p x 5), node 1 the two-category `flag` (chi-square
//!   12.5238095238095, df (2-1)(3-1) = 2), node 2 the four-category `cat` (chi-square 12, df
//!   (4-1)(2-1) = 3 over the classes present there). GAP: Modeler prints the F test's
//!   denominator df as N - J(t) - 1 (roots: 44 with 3 classes, 45 with 2) where the Guide says
//!   Nf(t) - J(t); the Guide formula is kept - every decision in the four oracles is the same
//!   under both readings (selected p values are orders of magnitude below alpha_B).
//! - Levene's fallback when no p is below alpha_B (ANOVA F of the absolute deviations from the
//!   class means, alpha_L = alpha/(m + m_c)) is computed per the Guide but refused with a GAP
//!   when it would select the field: no oracle exercises it.
//! - Split point, numeric primary: the target's classes grouped into two superclasses by
//!   two-means clustering of the class means (initial centers at the two most extreme means; a
//!   binary target skips the grouping), then the QDA root nearer the first (lower-variance)
//!   superclass's mean (default root `z <= 2.3611080332804`, binary root `z <= 1.69031361110306`),
//!   degenerate variances next to the point mass (epsilon = 1e-12).
//! - Nominal primary: CRIMCOORD scores (the dummy-variable scatter whitened by SVD, the
//!   between-class scatter's principal eigenvector) split by the same QDA; the categories
//!   present at the node go left or right (default node 1 `flag`: T vs F; node 2 `cat`:
//!   {amber, blue} vs {green, red}; binary node 1 `cat`: {amber, blue, green} vs {red}).
//! - Ordinal fields enter selection through the ANOVA F on their codes (the Guide's ordinal
//!   fields use the F test) but appear only in surrogate predicates in the oracles; a selected
//!   ordinal primary split is refused with a GAP.
//! - Surrogates: the predictive measure of association; ordered by association, ties by their
//!   own improvement then input order (the C&RT conventions). Default root: `x <= 4` beside
//!   `z <= 2.3611080332804`; default node 1: `x <= 0.5`, `z <= 0.625`, `cat` in
//!   {blue, green, red}; binary root: four surrogates `x <= 2.5`, `cat` in {amber, blue}, `ord`
//!   in {low, middle}, `flag` = T (the saved limit is 5).
//! - Stopping: purity, maximum depth (`quest_nofs_depth3`), minimum parent and child sizes (2%
//!   and 1% of the training records), no field below alpha_B. Pruning keeps the smallest tree of
//!   the weakest-link sequence (`quest_nofs_default` pruned against `quest_nofs_unpruned`: the
//!   same 7-node tree, growth had already stopped and the full tree carries the minimum risk).
//! - Missing values: records with a missing target do not train (none occur in the oracles);
//!   GAP: missing predictor values during growth. At scoring a blank primary follows the
//!   surrogates, then the default child, with the Laplace confidence (n_j + 1)/(N + J) reduced
//!   by 0.9 per split taken through a surrogate (`quest_nofs_*` grid records 10041-10043 with
//!   blank `x`, `z` and `cat`).
//! - PMML: nodes numbered breadth first, the default child the larger one, child predicates
//!   `surrogate` compounds as in C&RT, one `ScoreDistribution` per category with probability =
//!   the node's share. Modeler's `X-Node`/`X-risk` extensions and predictor importances are not
//!   generated and not compared.
//!
//! Supported (the oracles' settings): a nominal or flag target, at most 32 categories per field,
//! `alpha` 0.05, maximum depth 3 or 5, minimum sizes 2%/1% in percentage mode, at most 5
//! surrogates, pruning on or off, empirical priors, no frequency field, `train_pct` 0, at most
//! 100000 records and 64 inputs. Everything else returns [`AlgoError::Unsupported`] with a
//! `GAP:` - misclassification costs, profits, equal or user-specified priors, the standard error
//! rule, the overfit prevention set (`train_pct` > 0: `oracle/models_next/quest_*` keep failing;
//! Modeler's hold-out draw uses an undisclosed random generator), regression targets, a
//! Levene-selected or ordinal primary split, tied selection p-values, tied CRIMCOORD
//! eigenvalues, missing predictor values in training, and the more-than-16-category surrogate
//! subset search shared with C&RT. See `target/quest/REPORT.md` for the full list.

use crate::common::{
    class_summary, data_dictionary, mining_schema, regression_summary, split_text, value_range,
};
use crate::{AlgoError, Dataset, categories};
use dms_pmml::{
    BooleanOperator, MiningFunction, MissingValueStrategy, Model, ModelHeader, NoTrueChildStrategy,
    Node, Operator, Pmml, Predicate, ScoreDistribution, TreeModel,
};
use dms_types::{Measurement, Storage, Value};
use std::cmp::Ordering;
use std::collections::VecDeque;

fn gap(message: &str) -> AlgoError {
    AlgoError::Unsupported(format!("GAP: QUEST {message}"))
}

fn validate(dataset: &Dataset<'_>, settings: &QuestSettings) -> Result<(), AlgoError> {
    if ![3, 5].contains(&settings.max_depth)
        || settings.min_parent != MinSize::Percent(2.0)
        || settings.min_child != MinSize::Percent(1.0)
        || settings.max_surrogates != 5
        || settings.std_err_rule.is_some()
        || settings.alpha != 0.05
    {
        return Err(gap("settings outside quest_nofs coverage"));
    }
    let fields = dataset.table.model().fields();
    let target = fields
        .get(dataset.target)
        .ok_or_else(|| gap("target index"))?;
    if !matches!(target.measurement, Measurement::Nominal | Measurement::Flag) {
        return Err(gap("target must be nominal or flag"));
    }
    if dataset.table.rows().len() > 100_000 || dataset.inputs.len() > 64 {
        return Err(gap("training resource limit (100000 rows, 64 inputs)"));
    }
    if dataset.inputs.is_empty() || dataset.inputs.contains(&dataset.target) {
        return Err(gap("empty inputs or target used as input"));
    }
    for &position in dataset
        .inputs
        .iter()
        .chain(std::iter::once(&dataset.target))
    {
        let field = fields.get(position).ok_or_else(|| gap("field index"))?;
        if categories(dataset, position).len() > 32 && field.measurement != Measurement::Continuous
        {
            return Err(gap("more than 32 categories"));
        }
        for row in dataset.table.rows() {
            let value = row.get(position).ok_or_else(|| gap("short record"))?;
            if value
                .as_f64()
                .is_some_and(|v| !v.is_finite() || v.abs() > 1e100)
            {
                return Err(gap("nonfinite or excessive numeric magnitude"));
            }
        }
    }
    Ok(())
}

/// ANOVA uses available records, including their class counts (Guide p. 317).
fn anova(values: &[(f64, usize)], classes: usize) -> f64 {
    let mut moments = vec![Moments::default(); classes];
    for &(x, class) in values {
        if let Some(m) = moments.get_mut(class) {
            m.add(x);
        }
    }
    let groups: Vec<_> = moments.iter().filter(|m| m.n > 0.0).collect();
    let n = values.len() as f64;
    let k = groups.len() as f64;
    if k < 2.0 || n <= k {
        return 1.0;
    }
    let mean = groups.iter().map(|m| m.sum).sum::<f64>() / n;
    let between = groups
        .iter()
        .map(|m| m.n * (m.sum / m.n - mean).powi(2))
        .sum::<f64>();
    let within = groups.iter().map(|m| m.sse().max(0.0)).sum::<f64>();
    if within == 0.0 {
        return if between > 0.0 { 0.0 } else { 1.0 };
    }
    crate::stats::f_p((between / (k - 1.0)) / (within / (n - k)), k - 1.0, n - k)
}

fn pearson(
    codes: &[Option<usize>],
    labels: usize,
    target: &[usize],
    classes: usize,
    rows: &[usize],
) -> f64 {
    let mut cells = vec![vec![0.0; classes]; labels];
    let mut counts = vec![0.0; labels];
    let mut totals = vec![0.0; classes];
    for &r in rows {
        if let (Some(Some(c)), Some(j)) = (codes.get(r), target.get(r)) {
            if let Some(cell) = cells.get_mut(*c).and_then(|v| v.get_mut(*j)) {
                *cell += 1.0;
            }
            if let Some(count) = counts.get_mut(*c) {
                *count += 1.0;
            }
            if let Some(total) = totals.get_mut(*j) {
                *total += 1.0;
            }
        }
    }
    let n: f64 = totals.iter().sum();
    let i = counts.iter().filter(|&&v| v > 0.0).count();
    let j = totals.iter().filter(|&&v| v > 0.0).count();
    if i < 2 || j < 2 {
        return 1.0;
    }
    let mut statistic = 0.0;
    for (row, count) in cells.iter().zip(counts) {
        for (observed, total) in row.iter().zip(&totals) {
            let expected = count * total / n;
            if expected > 0.0 {
                statistic += (observed - expected).powi(2) / expected;
            }
        }
    }
    crate::stats::chi_square_p(statistic, ((i - 1) * (j - 1)) as f64)
}

fn numeric_values(column: &Column, codes: &[usize], rows: &[usize]) -> Vec<(f64, usize)> {
    rows.iter()
        .filter_map(|&r| {
            let x = match column {
                Column::Numeric { values, .. } => values.get(r).copied().flatten(),
                Column::Categorical { codes, .. } => {
                    codes.get(r).copied().flatten().map(|c| c as f64)
                }
            }?;
            Some((x, *codes.get(r)?))
        })
        .collect()
}

fn select_split(
    data: &Data,
    settings: &QuestSettings,
    rows: &[usize],
) -> Result<Option<(usize, Rule)>, AlgoError> {
    let Target::Classes { codes, labels } = &data.target else {
        return Err(gap("regression"));
    };
    let mut tests = Vec::new();
    for (input, column) in data.columns.iter().enumerate() {
        let p = match column {
            Column::Categorical {
                codes: x,
                labels: categories,
                ordinal: false,
            } => pearson(x, categories.len(), codes, labels.len(), rows),
            _ => anova(&numeric_values(column, codes, rows), labels.len()),
        };
        tests.push((input, p));
    }
    tests.sort_by(|a, b| a.1.total_cmp(&b.1));
    let Some(&(mut input, mut p)) = tests.first() else {
        return Ok(None);
    };
    if p >= settings.alpha / data.columns.len() as f64 {
        tests.clear();
        for (input, column) in data.columns.iter().enumerate() {
            if !matches!(column, Column::Numeric { ordinal: false, .. }) {
                continue;
            }
            let values = numeric_values(column, codes, rows);
            let mut groups = vec![Moments::default(); labels.len()];
            for &(x, j) in &values {
                if let Some(g) = groups.get_mut(j) {
                    g.add(x);
                }
            }
            let deviations: Vec<_> = values
                .iter()
                .filter_map(|&(x, j)| groups.get(j).map(|g| ((x - g.sum / g.n).abs(), j)))
                .collect();
            tests.push((input, anova(&deviations, labels.len())));
        }
        let threshold = settings.alpha / (data.columns.len() + tests.len()) as f64;
        tests.sort_by(|a, b| a.1.total_cmp(&b.1));
        let Some(&(candidate, significance)) = tests.first() else {
            return Ok(None);
        };
        if significance >= threshold {
            return Ok(None);
        }
        input = candidate;
        p = significance;
        return Err(gap(&format!(
            "Levene-selected split (field {input}, p={p}) has no oracle"
        )));
    }
    if tests
        .get(1)
        .is_some_and(|other| (other.1 - p).abs() <= p.abs() * 1e-12)
    {
        return Err(gap("tied variable-selection p values"));
    }
    let column = data
        .columns
        .get(input)
        .ok_or_else(|| gap("selected input"))?;
    let (values, coordinates) = match column {
        Column::Numeric { .. } => (numeric_values(column, codes, rows), None),
        Column::Categorical { ordinal: true, .. } => return Err(gap("ordinal primary split")),
        Column::Categorical {
            codes: x,
            labels: categories,
            ..
        } => {
            let coordinates = crimcoord(x, categories.len(), codes, labels.len(), rows)?;
            let values = rows
                .iter()
                .filter_map(|&r| {
                    Some((
                        *coordinates.get(x.get(r).copied().flatten()?)?,
                        *codes.get(r)?,
                    ))
                })
                .collect();
            (values, Some(coordinates))
        }
    };
    let threshold = qda(&values, codes, labels.len(), rows)?;
    let rule = match (column, coordinates) {
        (Column::Categorical { codes, labels, .. }, Some(coordinates)) => {
            let present = present_categories(codes, labels.len(), rows);
            let left = present
                .iter()
                .copied()
                .filter(|&c| coordinates.get(c).is_some_and(|&x| x <= threshold))
                .collect();
            categories_rule(&present, left)
        }
        _ => Rule::Threshold(threshold),
    };
    Ok(Some((input, rule)))
}

fn qda(
    values: &[(f64, usize)],
    target: &[usize],
    classes: usize,
    rows: &[usize],
) -> Result<f64, AlgoError> {
    let mut groups = vec![Moments::default(); classes];
    for &(x, j) in values {
        if let Some(g) = groups.get_mut(j) {
            g.add(x);
        }
    }
    let present: Vec<_> = groups
        .iter()
        .enumerate()
        .filter(|(_, g)| g.n > 0.0)
        .map(|(j, g)| (j, g.sum / g.n))
        .collect();
    if present.len() < 2 {
        return Err(gap("fewer than two observed classes on split field"));
    }
    let mut low = present
        .iter()
        .map(|(_, x)| *x)
        .fold(f64::INFINITY, f64::min);
    let mut high = present
        .iter()
        .map(|(_, x)| *x)
        .fold(f64::NEG_INFINITY, f64::max);
    if low == high {
        return Err(gap("equal class means"));
    }
    let mut assignment = vec![false; classes];
    let mut converged = false;
    for _ in 0..100 {
        let mut centers = [Moments::default(); 2];
        let mut changed = false;
        for &(j, x) in &present {
            if (x - low).abs() == (x - high).abs() {
                return Err(gap("two-means assignment tie"));
            }
            let side = (x - high).abs() < (x - low).abs();
            if let Some(a) = assignment.get_mut(j) {
                changed |= *a != side;
                *a = side;
            }
            if let Some(center) = centers.get_mut(usize::from(side)) {
                center.add(x);
            }
        }
        if !changed {
            converged = true;
            break;
        }
        low = centers
            .first()
            .filter(|m| m.n > 0.0)
            .map(|m| m.sum / m.n)
            .ok_or_else(|| gap("empty superclass"))?;
        high = centers
            .get(1)
            .filter(|m| m.n > 0.0)
            .map(|m| m.sum / m.n)
            .ok_or_else(|| gap("empty superclass"))?;
    }
    if !converged {
        return Err(gap("two-means iteration limit"));
    }
    let mut moments = [Moments::default(); 2];
    let mut mass = [0.0_f64; 2];
    for &(x, j) in values {
        if let Some(m) = moments.get_mut(usize::from(assignment.get(j).copied().unwrap_or(false))) {
            m.add(x);
        }
    }
    for &r in rows {
        if let Some(j) = target.get(r)
            && let Some(m) = mass.get_mut(usize::from(assignment.get(*j).copied().unwrap_or(false)))
        {
            *m += 1.0;
        }
    }
    let [a, b] = moments;
    if a.n < 2.0 || b.n < 2.0 {
        return Err(gap("singleton QDA superclass"));
    }
    let (ma, mb) = (a.sum / a.n, b.sum / b.n);
    let (va, vb) = (
        (a.sse() / (a.n - 1.0)).max(0.0),
        (b.sse() / (b.n - 1.0)).max(0.0),
    );
    let cut = if va.min(vb) < 1e-12 {
        // Guide p.319: degenerate distributions are separated next to the point mass.
        if va.max(vb) < 1e-12 {
            (ma + mb) / 2.0
        } else if va < vb {
            ma + 1e-12 * (mb - ma).signum()
        } else {
            mb + 1e-12 * (ma - mb).signum()
        }
    } else {
        let ratio = mass[0] / mass[1];
        let aa = va - vb;
        let bb = 2.0 * (ma * vb - mb * va);
        let cc = mb * mb * va - ma * ma * vb + 2.0 * va * vb * (ratio * (vb / va).sqrt()).ln();
        if aa.abs() < 1e-14 * va.max(vb) {
            (ma + mb) / 2.0 - va * (ratio.ln()) / (ma - mb)
        } else {
            let disc = bb * bb - 4.0 * aa * cc;
            if disc < 0.0 {
                return Err(gap("QDA has no real roots"));
            }
            let d1 = (-bb - disc.sqrt()) / (2.0 * aa);
            let d2 = (-bb + disc.sqrt()) / (2.0 * aa);
            let first = if va <= vb { ma } else { mb };
            if (d1 - first).abs() < (d2 - first).abs() {
                d1
            } else {
                d2
            }
        }
    };
    if !cut.is_finite() {
        return Err(gap("nonfinite QDA cut"));
    }
    Ok(cut)
}

/// Whiten the dummy-variable total scatter, then diagonalize between-class scatter.
fn crimcoord(
    codes: &[Option<usize>],
    categories: usize,
    target: &[usize],
    classes: usize,
    rows: &[usize],
) -> Result<Vec<f64>, AlgoError> {
    let mut counts = vec![0.0; categories];
    let mut cells = vec![vec![0.0; categories]; classes];
    let mut totals = vec![0.0; classes];
    for &r in rows {
        if let (Some(Some(c)), Some(j)) = (codes.get(r), target.get(r)) {
            if let Some(n) = counts.get_mut(*c) {
                *n += 1.0;
            }
            if let Some(n) = totals.get_mut(*j) {
                *n += 1.0;
            }
            if let Some(n) = cells.get_mut(*j).and_then(|v| v.get_mut(*c)) {
                *n += 1.0;
            }
        }
    }
    let n: f64 = counts.iter().sum();
    let mut total = vec![vec![0.0; categories]; categories];
    let mut between = total.clone();
    for (i, row) in total.iter_mut().enumerate() {
        for (j, value) in row.iter_mut().enumerate() {
            *value = if i == j { counts[i] } else { 0.0 } - counts[i] * counts[j] / n;
            between[i][j] = cells
                .iter()
                .zip(&totals)
                .filter(|(_, t)| **t > 0.0)
                .map(|(c, t)| c[i] * c[j] / t)
                .sum::<f64>()
                - counts[i] * counts[j] / n;
        }
    }
    let (eigen, vectors) = jacobi_eigen(total)?;
    let maximum = eigen.first().copied().unwrap_or(0.0);
    let rank = eigen.iter().filter(|&&e| e > maximum * 1e-12).count();
    if rank == 0 {
        return Err(gap("constant categorical primary"));
    }
    let whitening: Vec<Vec<f64>> = vectors
        .iter()
        .map(|row| {
            row.iter()
                .zip(&eigen)
                .take(rank)
                .map(|(v, e)| v / e.sqrt())
                .collect()
        })
        .collect();
    let mut reduced = vec![vec![0.0; rank]; rank];
    for (i, row) in reduced.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            for (r, brow) in between.iter().enumerate() {
                for (c, b) in brow.iter().enumerate() {
                    *v += whitening[r][i] * b * whitening[c][j];
                }
            }
        }
    }
    let (eigen, vectors) = jacobi_eigen(reduced)?;
    if eigen.get(1).is_some_and(|e| (eigen[0] - e).abs() <= 1e-12) {
        return Err(gap("tied CRIMCOORD eigenvalues"));
    }
    Ok(whitening
        .iter()
        .map(|row| row.iter().zip(&vectors).map(|(w, v)| w * v[0]).sum())
        .collect())
}

use crate::cart::{Impurity, MinSize};

/// Settings of a QUEST build node (Modeler's defaults in [`Default`]).
#[derive(Debug, Clone, PartialEq)]
pub struct QuestSettings {
    /// Bonferroni nominal significance level.
    pub alpha: f64,
    /// `max_depth` (default 5): splits below the root.
    pub max_depth: usize,
    /// `min_parent_records_pc` / `_abs` (default 2%).
    pub min_parent: MinSize,
    /// `min_child_records_pc` / `_abs` (default 1%).
    pub min_child: MinSize,
    /// `min_impurity` (default 0.0001).
    pub min_impurity_change: f64,
    /// `impurity_measure` (default Gini).
    pub impurity: Impurity,
    /// `max_surrogates` (default 5).
    pub max_surrogates: usize,
    /// `prune_tree` (default true).
    pub prune: bool,
    /// `use_std_err_rule` with `std_err_multiplier`: `Some(multiplier)` when on (default off).
    pub std_err_rule: Option<f64>,
}

impl Default for QuestSettings {
    fn default() -> Self {
        QuestSettings {
            alpha: 0.05,
            max_depth: 5,
            min_parent: MinSize::Percent(2.0),
            min_child: MinSize::Percent(1.0),
            min_impurity_change: 0.0001,
            impurity: Impurity::Gini,
            max_surrogates: 5,
            prune: true,
            std_err_rule: None,
        }
    }
}

/// Trains a QUEST model.
pub fn train(dataset: &Dataset<'_>, settings: &QuestSettings) -> Result<Pmml, AlgoError> {
    if dataset.frequency.is_some() {
        return Err(AlgoError::Unsupported(
            "QUEST with a frequency field".into(),
        ));
    }
    validate(dataset, settings)?;
    let data = Data::read(dataset)?;
    if data.rows.is_empty() {
        return Err(AlgoError::Data("no records with a valid target".into()));
    }
    if data.is_regression() && settings.std_err_rule.is_some() {
        return Err(AlgoError::Unsupported(
            "the standard error rule for regression trees".into(),
        ));
    }
    let mut tree = grow(&data, settings)?;
    if settings.prune {
        prune(&mut tree, &data, settings);
    }
    let root = pmml_node(&tree, 0, &data, Predicate::True);
    let mut schema = mining_schema(dataset);
    let mut used = Vec::new();
    fn predicate_fields(predicate: &Predicate, used: &mut Vec<String>) {
        match predicate {
            Predicate::Simple { field, .. } if !used.contains(field) => used.push(field.clone()),
            Predicate::Compound { predicates, .. } => {
                for p in predicates {
                    predicate_fields(p, used);
                }
            }
            _ => {}
        }
    }
    let mut queue = VecDeque::from([&root]);
    while let Some(node) = queue.pop_front() {
        predicate_fields(&node.predicate, &mut used);
        queue.extend(&node.children);
    }
    schema.sort_by_key(|f| used.iter().position(|s| s == &f.name).unwrap_or(usize::MAX));
    for field in &mut schema {
        field.invalid_treatment = dms_pmml::InvalidTreatment::ReturnInvalid;
    }
    let header = ModelHeader {
        name: None,
        function: if data.is_regression() {
            MiningFunction::Regression
        } else {
            MiningFunction::Classification
        },
        algorithm: Some("Quest".into()),
        mining_schema: schema,
        output: Vec::new(),
        local_transformations: Vec::new(),
    };
    Ok(Pmml {
        version: "4.3".into(),
        data_dictionary: data_dictionary(dataset),
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Tree(TreeModel {
            header,
            missing_value_strategy: MissingValueStrategy::None,
            no_true_child_strategy: NoTrueChildStrategy::ReturnNullPrediction,
            root,
        }),
    })
}

// ------------------------------------------------------------------------------------------
// Training data

/// An input field's values on the training records.
enum Column {
    /// Continuous (`ordinal == false`) or numeric ordinal values.
    Numeric {
        values: Vec<Option<f64>>,
        ordinal: bool,
    },
    /// Category codes into `labels`.
    Categorical {
        codes: Vec<Option<usize>>,
        labels: Vec<String>,
        ordinal: bool,
    },
}

impl Column {
    fn is_valid(&self, row: usize) -> bool {
        match self {
            Column::Numeric { values, .. } => values[row].is_some(),
            Column::Categorical { codes, .. } => codes[row].is_some(),
        }
    }
}

enum Target {
    Classes {
        codes: Vec<usize>,
        labels: Vec<String>,
    },
    Numbers(Vec<f64>),
}

struct Data {
    names: Vec<String>,
    columns: Vec<Column>,
    target: Target,
    /// Training records (valid target), as indices into the columns.
    rows: Vec<usize>,
    /// Empirical priors per class.
    priors: Vec<f64>,
    /// Training records per class.
    class_totals: Vec<f64>,
}

impl Data {
    fn read(dataset: &Dataset<'_>) -> Result<Data, AlgoError> {
        let fields = dataset.table.model().fields();
        let field = |position: usize| {
            fields
                .get(position)
                .ok_or_else(|| AlgoError::Data(format!("no field at position {position}")))
        };
        let table_rows = dataset.table.rows();

        let target_field = field(dataset.target)?;
        let target_value = |row: &[Value]| {
            row.get(dataset.target)
                .filter(|value| !target_field.is_missing(value))
                .cloned()
        };
        let mut rows = Vec::new();
        let target = if target_field.measurement == Measurement::Continuous {
            let mut numbers = vec![0.0; table_rows.len()];
            for (i, row) in table_rows.iter().enumerate() {
                if let Some(x) = target_value(row)
                    .and_then(|v| v.as_f64())
                    .filter(|x| x.is_finite())
                {
                    numbers[i] = x;
                    rows.push(i);
                }
            }
            Target::Numbers(numbers)
        } else {
            let mut labels = categories(dataset, dataset.target);
            let mut codes = vec![0; table_rows.len()];
            for (i, row) in table_rows.iter().enumerate() {
                if let Some(value) = target_value(row) {
                    codes[i] = code_of(&mut labels, value.to_string());
                    rows.push(i);
                }
            }
            Target::Classes { codes, labels }
        };

        let mut names = Vec::new();
        let mut columns = Vec::new();
        for &position in dataset.inputs {
            let input = field(position)?;
            let numeric = matches!(input.storage, Storage::Integer | Storage::Real);
            let cell = |row: &[Value]| {
                row.get(position)
                    .filter(|value| {
                        !input.is_missing(value)
                            && !matches!(value, Value::String(s) if s.trim().is_empty())
                    })
                    .cloned()
            };
            let column = match input.measurement {
                Measurement::Continuous | Measurement::Ordinal if numeric => Column::Numeric {
                    values: table_rows
                        .iter()
                        .map(|row| cell(row).and_then(|v| v.as_f64()).filter(|x| x.is_finite()))
                        .collect(),
                    ordinal: input.measurement == Measurement::Ordinal,
                },
                Measurement::Continuous => {
                    return Err(AlgoError::Unsupported(format!(
                        "QUEST input {} with non-numeric storage",
                        input.name
                    )));
                }
                _ => {
                    let mut labels = categories(dataset, position);
                    let codes = table_rows
                        .iter()
                        .map(|row| cell(row).map(|v| code_of(&mut labels, v.to_string())))
                        .collect();
                    Column::Categorical {
                        codes,
                        labels,
                        ordinal: input.measurement == Measurement::Ordinal,
                    }
                }
            };
            names.push(input.name.clone());
            columns.push(column);
        }

        let (priors, class_totals) = match &target {
            Target::Classes { codes, labels } => {
                let mut totals = vec![0.0; labels.len()];
                for &row in &rows {
                    totals[codes[row]] += 1.0;
                }
                let n = rows.len() as f64;
                (totals.iter().map(|t| t / n).collect(), totals)
            }
            Target::Numbers(_) => (Vec::new(), Vec::new()),
        };
        Ok(Data {
            names,
            columns,
            target,
            rows,
            priors,
            class_totals,
        })
    }

    fn is_regression(&self) -> bool {
        matches!(self.target, Target::Numbers(_))
    }
}

fn code_of(labels: &mut Vec<String>, text: String) -> usize {
    match labels.iter().position(|label| *label == text) {
        Some(code) => code,
        None => {
            labels.push(text);
            labels.len() - 1
        }
    }
}

// ------------------------------------------------------------------------------------------
// Impurity

/// The prior-weighted class terms `��(j) N_j(t) / N_j` and their sum, the node mass `p(t)`
/// (Algorithms Guide, Gini), with `N_j` the root counts of records valid for the predictor.
fn mass_terms(counts: &[f64], priors: &[f64], root: &[f64]) -> (Vec<f64>, f64) {
    let mut terms = vec![0.0; counts.len()];
    let mut mass = 0.0;
    for j in 0..counts.len() {
        if root[j] > 0.0 && counts[j] > 0.0 {
            terms[j] = priors[j] * counts[j] / root[j];
            mass += terms[j];
        }
    }
    (terms, mass)
}

fn gini(counts: &[f64], priors: &[f64], root: &[f64]) -> f64 {
    let (terms, mass) = mass_terms(counts, priors, root);
    if mass <= 0.0 {
        return 0.0;
    }
    1.0 - terms.iter().map(|t| (t / mass) * (t / mass)).sum::<f64>()
}

/// The improvement of a classification split with the given child counts.
fn class_improvement(
    impurity: Impurity,
    left: &[f64],
    right: &[f64],
    priors: &[f64],
    root: &[f64],
) -> f64 {
    let (left_terms, left_mass) = mass_terms(left, priors, root);
    let (right_terms, right_mass) = mass_terms(right, priors, root);
    let mass = left_mass + right_mass;
    if mass <= 0.0 {
        return 0.0;
    }
    match impurity {
        Impurity::Gini => {
            let parent: Vec<f64> = left.iter().zip(right).map(|(l, r)| l + r).collect();
            gini(&parent, priors, root)
                - left_mass / mass * gini(left, priors, root)
                - right_mass / mass * gini(right, priors, root)
        }
        Impurity::Twoing => {
            if left_mass <= 0.0 || right_mass <= 0.0 {
                return 0.0;
            }
            let difference: f64 = left_terms
                .iter()
                .zip(&right_terms)
                .map(|(l, r)| (l / left_mass - r / right_mass).abs())
                .sum();
            let (p_left, p_right) = (left_mass / mass, right_mass / mass);
            p_left * p_right / 4.0 * difference * difference
        }
    }
}

/// Count, sum and sum of squares of target values.
#[derive(Debug, Clone, Copy, Default)]
struct Moments {
    n: f64,
    sum: f64,
    squares: f64,
}

impl Moments {
    fn add(&mut self, y: f64) {
        self.n += 1.0;
        self.sum += y;
        self.squares += y * y;
    }

    fn sse(&self) -> f64 {
        if self.n <= 0.0 {
            0.0
        } else {
            self.squares - self.sum * self.sum / self.n
        }
    }
}

/// The LSD improvement `(SSE(t) - SSE(tL) - SSE(tR)) / N(t)`.
fn lsd_improvement(total: &Moments, left: &Moments) -> f64 {
    let right = Moments {
        n: total.n - left.n,
        sum: total.sum - left.sum,
        squares: total.squares - left.squares,
    };
    (total.sse() - left.sse() - right.sse()) / total.n
}

// ------------------------------------------------------------------------------------------
// Splits

#[derive(Debug, Clone, PartialEq)]
enum Rule {
    /// `x <= threshold` goes left.
    Threshold(f64),
    /// Category codes that go left and right: the categories present where the split was made
    /// (`cart_no_surrogates` node 24 names only `NORMAL` because `HIGH` is absent there). Other
    /// categories satisfy neither side.
    Categories { left: Vec<usize>, right: Vec<usize> },
}

impl Rule {
    fn goes_left(&self, column: &Column, row: usize) -> Option<bool> {
        match (self, column) {
            (Rule::Threshold(t), Column::Numeric { values, .. }) => values[row].map(|x| x <= *t),
            (Rule::Categories { left, right }, Column::Categorical { codes, .. }) => codes[row]
                .and_then(|code| {
                    if left.contains(&code) {
                        Some(true)
                    } else if right.contains(&code) {
                        Some(false)
                    } else {
                        None
                    }
                }),
            _ => None,
        }
    }
}

/// The present categories split into `set` and the rest.
fn categories_rule(present: &[usize], set: Vec<usize>) -> Rule {
    let right = present
        .iter()
        .copied()
        .filter(|c| !set.contains(c))
        .collect();
    Rule::Categories { left: set, right }
}

/// Root class counts of the records valid for a predictor (the `N_j` of the Gini formula).
fn root_counts(
    data: &Data,
    valid: impl Fn(usize) -> bool,
    codes: &[usize],
    classes: usize,
) -> Vec<f64> {
    let mut counts = vec![0.0; classes];
    for &row in &data.rows {
        if valid(row) {
            counts[codes[row]] += 1.0;
        }
    }
    counts
}

fn present_categories(codes: &[Option<usize>], categories: usize, rows: &[usize]) -> Vec<usize> {
    let mut present = vec![false; categories];
    for &row in rows {
        if let Some(code) = codes[row] {
            present[code] = true;
        }
    }
    (0..categories).filter(|&c| present[c]).collect()
}

/// Subsets of the present categories that contain the first one and not all of them, in binary
/// count order of the other categories.
fn subsets(present: &[usize]) -> Option<Vec<Vec<usize>>> {
    let k = present.len();
    // GAP: Modeler's search over more than 16 categories is not known; such fields are skipped.
    if !(2..=16).contains(&k) {
        return None;
    }
    Some(
        (0u32..(1 << (k - 1)) - 1)
            .map(|mask| {
                std::iter::once(present[0])
                    .chain(
                        (0..k - 1)
                            .filter(|b| mask & (1 << b) != 0)
                            .map(|b| present[b + 1]),
                    )
                    .collect()
            })
            .collect(),
    )
}

struct Surrogate {
    input: usize,
    rule: Rule,
    /// Whether the records satisfying `rule` go to the primary's left child.
    rule_left: bool,
    association: f64,
    /// The surrogate split's own improvement on the target, which orders surrogates of equal
    /// association.
    improvement: f64,
}

/// The best surrogate on `input` for the records of `labeled` (`(row, goes left)`): the split
/// that sends the most of them where the primary does, scored by the predictive measure of
/// association `(agree - majority) / (n - majority)`.
fn best_surrogate(
    data: &Data,
    settings: &QuestSettings,
    primary: usize,
    primary_rule: &Rule,
    input: usize,
    labeled: &[(usize, bool)],
) -> Option<Surrogate> {
    let column = &data.columns[input];
    let Target::Classes { codes, labels } = &data.target else {
        return None;
    };
    let both: Vec<_> = labeled
        .iter()
        .copied()
        .filter(|(r, _)| column.is_valid(*r))
        .collect();
    if both.len() < 2 {
        return None;
    }
    let root = root_counts(
        data,
        |r| column.is_valid(r) && data.columns[primary].is_valid(r),
        codes,
        labels.len(),
    );
    // Sum by target category, matching the Guide's prior-weighted probability formula.
    let probability = |rule: Option<(&Rule, bool)>| {
        let mut counts = vec![0.0; labels.len()];
        for &(r, left) in &both {
            if rule.is_none_or(|(rule, direction)| {
                rule.goes_left(column, r)
                    .is_some_and(|hit| (hit == direction) == left)
            }) {
                counts[codes[r]] += 1.0;
            }
        }
        counts
            .iter()
            .enumerate()
            .map(|(j, n)| {
                if root[j] > 0.0 {
                    data.priors[j] * n / root[j]
                } else {
                    0.0
                }
            })
            .sum::<f64>()
    };
    let total = probability(None);
    let primary_root = root_counts(
        data,
        |r| data.columns[primary].is_valid(r),
        codes,
        labels.len(),
    );
    let mut left_counts = vec![0.0; labels.len()];
    let mut right_counts = left_counts.clone();
    for &(r, _) in labeled {
        if primary_rule.goes_left(&data.columns[primary], r) == Some(true) {
            left_counts[codes[r]] += 1.0;
        } else {
            right_counts[codes[r]] += 1.0;
        }
    }
    let (_, left_mass) = mass_terms(&left_counts, &data.priors, &primary_root);
    let (_, right_mass) = mass_terms(&right_counts, &data.priors, &primary_root);
    let baseline = left_mass.max(right_mass) / (left_mass + right_mass);
    if total <= 0.0 || baseline >= 1.0 {
        return None;
    }
    let mut candidates = Vec::new();
    match column {
        Column::Numeric { values, ordinal } => {
            let mut sorted: Vec<_> = both.iter().filter_map(|(r, _)| values[*r]).collect();
            sorted.sort_by(f64::total_cmp);
            sorted.dedup();
            for pair in sorted.windows(2) {
                if let [low, high] = pair {
                    candidates.push(Rule::Threshold(if *ordinal {
                        *low
                    } else {
                        low / 2.0 + high / 2.0
                    }));
                }
            }
        }
        Column::Categorical {
            codes,
            labels,
            ordinal,
        } => {
            let rows: Vec<_> = both.iter().map(|(r, _)| *r).collect();
            let present = present_categories(codes, labels.len(), &rows);
            if *ordinal {
                for set in subsets(&present)? {
                    candidates.push(categories_rule(&present, set));
                }
            } else {
                let mut left = vec![0usize; labels.len()];
                let mut right = left.clone();
                for &(r, side) in &both {
                    if let Some(code) = codes.get(r).copied().flatten()
                        && let Some(count) = if side {
                            left.get_mut(code)
                        } else {
                            right.get_mut(code)
                        }
                    {
                        *count += 1;
                    }
                }
                let set: Vec<_> = present
                    .iter()
                    .copied()
                    .filter(|&c| left[c] >= right[c])
                    .collect();
                if !set.is_empty() && set.len() < present.len() {
                    candidates.push(categories_rule(&present, set));
                }
            }
        }
    }
    let mut best: Option<(usize, f64, Rule, bool)> = None;
    for rule in candidates {
        let mut counts = [[0.0_f64; 2]; 2];
        for &(r, left) in &both {
            if let Some(hit) = rule.goes_left(column, r) {
                counts[usize::from(hit)][usize::from(left)] += 1.0;
            }
        }
        let direct = (counts[0][0] + counts[1][1]) as usize;
        let reverse = both.len() - direct;
        let (agree, direction) = if direct >= reverse {
            (direct, true)
        } else {
            (reverse, false)
        };
        let impurity: f64 = counts
            .iter()
            .map(|c| {
                let n = c[0] + c[1];
                if n > 0.0 { c[0] * c[1] / n } else { 0.0 }
            })
            .sum();
        let replace = best
            .as_ref()
            .is_none_or(|(b, g, _, _)| agree > *b || (agree == *b && impurity < *g));
        if replace {
            best = Some((agree, impurity, rule, direction));
        }
    }
    let (_, _, rule, rule_left) = best?;
    let agree = probability(Some((&rule, rule_left)));
    let association = (agree / total - baseline) / (1.0 - baseline);
    if association <= 0.0 {
        return None;
    }
    let sides: Vec<_> = labeled
        .iter()
        .filter_map(|&(r, _)| rule.goes_left(column, r).map(|left| (r, left)))
        .collect();
    Some(Surrogate {
        input,
        improvement: split_improvement(data, settings, column, &sides),
        rule,
        rule_left,
        association,
    })
}

/// The improvement of the split that sends the `sides` records left (`true`) or right.
fn split_improvement(
    data: &Data,
    settings: &QuestSettings,
    column: &Column,
    sides: &[(usize, bool)],
) -> f64 {
    match &data.target {
        Target::Classes { codes, labels } => {
            let root = root_counts(data, |r| column.is_valid(r), codes, labels.len());
            let mut left = vec![0.0; labels.len()];
            let mut right = vec![0.0; labels.len()];
            for &(row, goes_left) in sides {
                if goes_left {
                    left[codes[row]] += 1.0;
                } else {
                    right[codes[row]] += 1.0;
                }
            }
            class_improvement(settings.impurity, &left, &right, &data.priors, &root)
        }
        Target::Numbers(ys) => {
            let mut total = Moments::default();
            let mut left = Moments::default();
            for &(row, goes_left) in sides {
                total.add(ys[row]);
                if goes_left {
                    left.add(ys[row]);
                }
            }
            lsd_improvement(&total, &left)
        }
    }
}

fn surrogates(
    data: &Data,
    settings: &QuestSettings,
    primary: usize,
    rule: &Rule,
    rows: &[usize],
) -> Vec<Surrogate> {
    let max = settings.max_surrogates;
    if max == 0 {
        return Vec::new();
    }
    let labeled: Vec<(usize, bool)> = rows
        .iter()
        .filter_map(|&row| {
            rule.goes_left(&data.columns[primary], row)
                .map(|left| (row, left))
        })
        .collect();
    let mut found: Vec<Surrogate> = (0..data.columns.len())
        .filter(|&input| input != primary)
        .filter_map(|input| best_surrogate(data, settings, primary, rule, input, &labeled))
        .collect();
    // Higher association first; on ties the larger improvement, then the earlier input
    // (`cart_unpruned` nodes 6, 13, 15; `cart_depth3` node 5; `cart_twoing` nodes 0, 13, 21).
    let descending = |a: f64, b: f64| {
        if (a - b).abs() <= 1e-12 {
            Ordering::Equal
        } else {
            b.partial_cmp(&a).unwrap_or(Ordering::Equal)
        }
    };
    found.sort_by(|a, b| {
        descending(a.association, b.association)
            .then(descending(a.improvement, b.improvement))
            .then(a.input.cmp(&b.input))
    });
    found.truncate(max);
    found
}

// ------------------------------------------------------------------------------------------
// Growing

struct Split {
    input: usize,
    rule: Rule,
    surrogates: Vec<Surrogate>,
    /// Whether the left child is the default child.
    default_left: bool,
    /// Indices of the children in the tree's node list.
    left: usize,
    right: usize,
}

struct GrownNode {
    id: usize,
    rows: Vec<usize>,
    parent: Option<usize>,
    split: Option<Split>,
}

struct Tree {
    nodes: Vec<GrownNode>,
}

fn grow(data: &Data, settings: &QuestSettings) -> Result<Tree, AlgoError> {
    let total = data.rows.len();
    let min_parent = settings.min_parent.records(total);
    let min_child = settings.min_child.records(total);
    let mut nodes = vec![GrownNode {
        id: 0,
        rows: data.rows.clone(),
        parent: None,
        split: None,
    }];
    let mut next_id = 1;
    let mut queue = VecDeque::from([(0usize, 0usize)]);
    while let Some((index, depth)) = queue.pop_front() {
        let rows = std::mem::take(&mut nodes[index].rows);
        let stop =
            depth >= settings.max_depth || is_pure(data, &rows) || (rows.len() as f64) < min_parent;
        let split = if stop {
            None
        } else {
            let made = split_node(data, settings, &rows, min_child)?;
            if made.is_none() {
                // The ids of the children the node did not get are used up.
                next_id += 2;
            }
            made
        };
        if let Some((input, rule, surrogates, default_left, left_rows, right_rows)) = split {
            let (left, right) = (nodes.len(), nodes.len() + 1);
            for (offset, child_rows) in [left_rows, right_rows].into_iter().enumerate() {
                nodes.push(GrownNode {
                    id: next_id + offset,
                    rows: child_rows,
                    parent: Some(index),
                    split: None,
                });
            }
            next_id += 2;
            nodes[index].split = Some(Split {
                input,
                rule,
                surrogates,
                default_left,
                left,
                right,
            });
            queue.push_back((left, depth + 1));
            queue.push_back((right, depth + 1));
        }
        nodes[index].rows = rows;
    }
    Ok(Tree { nodes })
}

type MadeSplit = (usize, Rule, Vec<Surrogate>, bool, Vec<usize>, Vec<usize>);

/// The node's split, or `None` when its best split fails the stopping rules.
fn split_node(
    data: &Data,
    settings: &QuestSettings,
    rows: &[usize],
    min_child: f64,
) -> Result<Option<MadeSplit>, AlgoError> {
    let Some((input, rule)) = select_split(data, settings, rows)? else {
        return Ok(None);
    };
    let column = &data.columns[input];
    let left_valid = rows
        .iter()
        .filter(|&&r| rule.goes_left(column, r) == Some(true))
        .count();
    let right_valid = rows
        .iter()
        .filter(|&&r| rule.goes_left(column, r) == Some(false))
        .count();
    if left_valid == 0
        || right_valid == 0
        || (left_valid as f64) < min_child
        || (right_valid as f64) < min_child
    {
        return Ok(None);
    }
    let column = &data.columns[input];
    let surrogates = surrogates(data, settings, input, &rule, rows);
    let default_left = left_valid >= right_valid;
    let (mut left_rows, mut right_rows) = (Vec::new(), Vec::new());
    for &row in rows {
        let left = rule.goes_left(column, row).or_else(|| {
            surrogates.iter().find_map(|s| {
                s.rule
                    .goes_left(&data.columns[s.input], row)
                    .map(|hit| hit == s.rule_left)
            })
        });
        if left.unwrap_or(default_left) {
            left_rows.push(row);
        } else {
            right_rows.push(row);
        }
    }
    Ok(Some((
        input,
        rule,
        surrogates,
        default_left,
        left_rows,
        right_rows,
    )))
}

fn is_pure(data: &Data, rows: &[usize]) -> bool {
    let Some(&first) = rows.first() else {
        return true;
    };
    match &data.target {
        Target::Classes { codes, .. } => rows.iter().all(|&r| codes[r] == codes[first]),
        Target::Numbers(ys) => rows.iter().all(|&r| ys[r] == ys[first]),
    }
}

// ------------------------------------------------------------------------------------------
// Pruning

/// The resubstitution risk of a node as a leaf: its misclassified records, or its sum of squared
/// deviations, over all training records.
fn node_risk(data: &Data, rows: &[usize]) -> f64 {
    let total = data.rows.len() as f64;
    match &data.target {
        Target::Classes { codes, labels } => {
            let mut counts = vec![0usize; labels.len()];
            for &row in rows {
                counts[codes[row]] += 1;
            }
            let majority = counts.iter().copied().max().unwrap_or(0);
            (rows.len() - majority) as f64 / total
        }
        Target::Numbers(ys) => {
            let mut moments = Moments::default();
            for &row in rows {
                moments.add(ys[row]);
            }
            moments.sse() / total
        }
    }
}

/// Weakest-link pruning (Algorithms Guide, "Pruning"): collapse the splits with the smallest
/// `g(t) = (R(t) - R(T_t)) / (|T_t| - 1)` until only the root is left, then keep the smallest tree
/// of that sequence whose risk is within the chosen margin of the minimum.
fn prune(tree: &mut Tree, data: &Data, settings: &QuestSettings) {
    let risk: Vec<f64> = tree
        .nodes
        .iter()
        .map(|n| node_risk(data, &n.rows))
        .collect();
    let mut kept: Vec<bool> = tree.nodes.iter().map(|n| n.split.is_some()).collect();
    let mut sequence = vec![(kept.clone(), subtree(tree, &kept, &risk, 0).0)];
    while kept[0] {
        let links: Vec<(usize, f64)> = (0..tree.nodes.len())
            .filter(|&index| kept[index])
            .map(|index| {
                let (branch_risk, leaves) = subtree(tree, &kept, &risk, index);
                (index, (risk[index] - branch_risk) / (leaves as f64 - 1.0))
            })
            .collect();
        let weakest = links.iter().map(|(_, g)| *g).fold(f64::INFINITY, f64::min);
        for (index, g) in links {
            if g <= weakest + 1e-12 {
                kept[index] = false;
            }
        }
        // Splits below a collapsed node go with it.
        for index in 0..tree.nodes.len() {
            if kept[index] && !ancestors_kept(tree, &kept, index) {
                kept[index] = false;
            }
        }
        let tree_risk = subtree(tree, &kept, &risk, 0).0;
        sequence.push((kept.clone(), tree_risk));
    }
    let minimum = sequence
        .iter()
        .map(|(_, r)| *r)
        .fold(f64::INFINITY, f64::min);
    let margin = settings.std_err_rule.map_or(0.0, |multiplier| {
        let n = data.rows.len() as f64;
        multiplier * (minimum * (1.0 - minimum) / n).max(0.0).sqrt()
    });
    if let Some((chosen, _)) = sequence
        .iter()
        .rev()
        .find(|(_, r)| *r <= minimum + margin + 1e-12)
    {
        for (node, keep) in tree.nodes.iter_mut().zip(chosen) {
            if !keep {
                node.split = None;
            }
        }
    }
}

/// Risk and leaf count of the subtree under `index` with the `kept` splits.
fn subtree(tree: &Tree, kept: &[bool], risk: &[f64], index: usize) -> (f64, usize) {
    match &tree.nodes[index].split {
        Some(split) if kept[index] => {
            let (left_risk, left_leaves) = subtree(tree, kept, risk, split.left);
            let (right_risk, right_leaves) = subtree(tree, kept, risk, split.right);
            (left_risk + right_risk, left_leaves + right_leaves)
        }
        _ => (risk[index], 1),
    }
}

fn ancestors_kept(tree: &Tree, kept: &[bool], index: usize) -> bool {
    let mut current = tree.nodes[index].parent;
    while let Some(parent) = current {
        if !kept[parent] {
            return false;
        }
        current = tree.nodes[parent].parent;
    }
    true
}

// ------------------------------------------------------------------------------------------
// PMML

fn pmml_node(tree: &Tree, index: usize, data: &Data, predicate: Predicate) -> Node {
    let node = &tree.nodes[index];
    let (score, distribution) = summary(data, &node.rows);
    let mut out = Node {
        id: Some(node.id.to_string()),
        score: Some(score),
        record_count: Some(node.rows.len() as f64),
        default_child: None,
        predicate,
        distribution,
        children: Vec::new(),
    };
    if let Some(split) = &node.split {
        let (left, right) = (&tree.nodes[split.left], &tree.nodes[split.right]);
        let default = if split.default_left {
            left.id
        } else {
            right.id
        };
        out.default_child = Some(default.to_string());
        out.children = vec![
            pmml_node(tree, split.left, data, child_predicate(data, split, true)),
            pmml_node(tree, split.right, data, child_predicate(data, split, false)),
        ];
    }
    out
}

fn child_predicate(data: &Data, split: &Split, left: bool) -> Predicate {
    let mut parts = vec![rule_predicate(data, split.input, &split.rule, left)];
    for s in &split.surrogates {
        parts.push(rule_predicate(data, s.input, &s.rule, left == s.rule_left));
    }
    parts.push(if split.default_left == left {
        Predicate::True
    } else {
        Predicate::False
    });
    Predicate::Compound {
        operator: BooleanOperator::Surrogate,
        predicates: parts,
    }
}

/// The predicate of the records that satisfy `rule` (`satisfied`) or do not.
fn rule_predicate(data: &Data, input: usize, rule: &Rule, satisfied: bool) -> Predicate {
    let field = data.names[input].clone();
    match (rule, &data.columns[input]) {
        (Rule::Threshold(t), _) => {
            let simple = Predicate::Simple {
                field,
                operator: if satisfied {
                    Operator::LessOrEqual
                } else {
                    Operator::GreaterThan
                },
                value: Some(split_text(*t)),
            };
            let predicates = if satisfied {
                vec![Predicate::True, simple]
            } else {
                vec![simple, Predicate::True]
            };
            Predicate::Compound {
                operator: BooleanOperator::And,
                predicates,
            }
        }
        (Rule::Categories { left, right }, Column::Categorical { labels, .. }) => {
            let chosen = if satisfied { left } else { right };
            let mut predicates: Vec<Predicate> = chosen
                .iter()
                .map(|&c| Predicate::Simple {
                    field: field.clone(),
                    operator: Operator::Equal,
                    value: Some(labels[c].clone()),
                })
                .collect();
            if predicates.len() < 2 {
                predicates.push(Predicate::False);
            }
            Predicate::Compound {
                operator: BooleanOperator::Or,
                predicates,
            }
        }
        (Rule::Categories { .. }, Column::Numeric { .. }) => Predicate::False,
    }
}

/// A node's prediction and score distribution.
fn summary(data: &Data, rows: &[usize]) -> (String, Vec<ScoreDistribution>) {
    match &data.target {
        Target::Classes { codes, labels } => {
            let mut counts = vec![0.0; labels.len()];
            for &row in rows {
                counts[codes[row]] += 1.0;
            }
            class_summary(labels, &counts, &data.priors, &data.class_totals)
        }
        Target::Numbers(ys) => regression_summary(
            rows.iter().map(|&r| ys[r]),
            value_range(data.rows.iter().map(|&r| ys[r])),
        ),
    }
}

fn jacobi_eigen(mut a: Vec<Vec<f64>>) -> Result<(Vec<f64>, Vec<Vec<f64>>), AlgoError> {
    let n = a.len();
    let mut v = (0..n)
        .map(|i| (0..n).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
        .collect::<Vec<Vec<f64>>>();
    for _ in 0..100 {
        let off: f64 = (0..n)
            .flat_map(|i| (0..n).filter(move |&j| j != i).map(move |j| (i, j)))
            .map(|(i, j)| a[i][j] * a[i][j])
            .sum();
        if off < 1e-30 {
            break;
        }
        for i in 0..n {
            for j in i + 1..n {
                if a[i][j].abs() < 1e-300 {
                    continue;
                }
                let theta = (a[j][j] - a[i][i]) / (2.0 * a[i][j]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for row in a.iter_mut() {
                    let (ari, arj) = (row[i], row[j]);
                    row[i] = c * ari - s * arj;
                    row[j] = s * ari + c * arj;
                }
                let (low, high) = a.split_at_mut(j);
                if let (Some(row_i), Some(row_j)) = (low.get_mut(i), high.first_mut()) {
                    for (x, y) in row_i.iter_mut().zip(row_j.iter_mut()) {
                        let (air, ajr) = (*x, *y);
                        *x = c * air - s * ajr;
                        *y = s * air + c * ajr;
                    }
                }
                for row in v.iter_mut() {
                    let (vri, vrj) = (row[i], row[j]);
                    row[i] = c * vri - s * vrj;
                    row[j] = s * vri + c * vrj;
                }
            }
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&x, &y| a[y][y].total_cmp(&a[x][x]));
    let values: Vec<f64> = order.iter().map(|&i| a[i][i]).collect();
    if values.iter().any(|v| !v.is_finite()) {
        return Err(AlgoError::Data("QUEST eigen decomposition failed".into()));
    }
    let vectors = (0..n)
        .map(|r| order.iter().map(|&c| v[r][c]).collect())
        .collect();
    Ok((values, vectors))
}
