//! TwoStep cluster (Algorithms Guide: "TwoStep Cluster Algorithms"; Chiu et al. 2001). Records
//! are pre-clustered into the leaf entries of a cluster-feature (CF) tree, the leaf entries are
//! merged agglomeratively by the log-likelihood distance, and the number of clusters is fixed or
//! chosen from BIC/AIC and the merge distances.
//!
//! Rules settled against `oracle/models/twostep_default` and `oracle/twostep/*`
//! (`xtask/oracle/probes/twostep_streams.py`):
//! - Records with a missing input are left out. A categorical input with fewer than two
//!   categories among the remaining records is dropped (`ts_drug8`: `Cholesterol` is constant).
//! - A cluster v of N_v records has xi_v = -N_v (sum_k 1/2 ln(D_k + s2_vk) + sum_k E_vk), with
//!   s2_vk the maximum-likelihood variance of continuous field k in the cluster, E_vk the entropy
//!   of categorical field k, and D_k the field's sample variance over the training records when
//!   `standardize` is set (the guide's standardized form, in original units), else 1 (`ts_nostd`).
//!   The distance between clusters is xi_i + xi_j - xi_<i,j>; the 15 merge distances of every
//!   oracle (`X-Distances`) match to 1e-12 with each distinct record its own leaf entry, as the
//!   initial threshold 0 gives.
//! - BIC(J) = -2 sum_j xi_j + m_J ln N and AIC(J) = -2 sum_j xi_j + 2 m_J, with
//!   m_J = J (2 K_A + sum_k (L_k - 1)) (`X-CriterionIndex`).
//! - Automatic number of clusters: with dC(J) = C(J) - C(J + 1), one cluster when dC(1) < 0;
//!   otherwise the initial estimate is the smallest J with dC(J) / dC(1) < 0.04 (the largest
//!   number when none is), and R2(J) = d_J / d_(J+1) over J from the minimum to the smaller of the
//!   estimate and the maximum, where d_J is the distance of the merge that leaves J - 1 clusters.
//!   Of the two largest ratios the larger J wins unless the largest ratio exceeds the second by a
//!   factor over 1.15 (`twostep_default`: 5, `ts_aic`: 12, `ts_range3_6`: 5, `ts_nostd`: 4,
//!   `ts_drug50`: 3, `ts_drug_cont`: 4).
//! - The model saves the clusters' sizes, means, sample variances and category counts from the
//!   hierarchical merge; scoring then assigns each record to its nearest cluster, which can differ
//!   from the merge (`ts_drug50`: 17/24/9 in the model, 19/22/9 scored).
//! - Categories are ordered true then false for flags, as the Type lists them otherwise
//!   (`twostep_default`: `M F`, `NORMAL HIGH`). Clusters are named `<label>-<n>`, or `n` with
//!   numeric labels (`ts_label_prefix`, `ts_label_number`).
//!
//! GAP: cluster numbering. Clusters are numbered by their first leaf entry in CF-tree order. The
//! clusters themselves match Modeler's in all 42 oracle streams (`crates/exec/tests/
//! models_twostep.rs`), and the names in 12 of them: up to 8 records (one leaf, in record order,
//! `ts_prefix3`..`ts_prefix8`) and where few splits keep the first entries in place (`ts_drug9`,
//! `ts_drug20`, `ts_drug50`). After a leaf split Modeler's entries are not in record order
//! (`ts_prefix9`, one cluster per record, lists the split leaves as `0 5 3 6` and `2 7 1 4 8`
//! where record order gives `0 3 5 6` and `1 2 4 7 8`), and no ordering of the split we tried
//! (seeds first, distance to the seed, in-place partitions, insertion next to the closest entry)
//! reproduces it, so larger data get the same clusters under other numbers.
//! Outlier handling, the Euclidean distance and CF-tree rebuilds (a tree deeper than three
//! levels) are not implemented.

use crate::AlgoError;
use crate::common::{data_field, field_categories, mining_field};
use dms_pmml::{
    Cluster, ClusteringField, ClusteringModel, CompareFunction, ComparisonMeasure, MeasureKind,
    Metric, MiningFunction, Model, ModelClass, ModelHeader, OpType, Pmml, Usage,
};
use dms_types::{Measurement, Storage, Table, Value, ValueSpec};

/// The criterion that chooses the number of clusters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Criterion {
    Bic,
    Aic,
}

/// Settings of a TwoStep build node (Modeler's defaults in [`Default`]).
#[derive(Debug, Clone, PartialEq)]
pub struct TwoStepSettings {
    /// `standardize` (default true).
    pub standardize: bool,
    /// `cluster_num_auto` (default true): choose the number of clusters.
    pub auto_clusters: bool,
    /// `min_num_clusters`, `max_num_clusters` (2, 15): the automatic range.
    pub min_clusters: usize,
    pub max_clusters: usize,
    /// `num_clusters` (10): the fixed number.
    pub clusters: usize,
    pub criterion: Criterion,
    /// `label_prefix` (default `cluster`).
    pub label: String,
    /// `cluster_label` `Number`: clusters named `1`, `2`, ...
    pub numeric_labels: bool,
    /// The model name (`modelName`).
    pub name: Option<String>,
}

impl Default for TwoStepSettings {
    fn default() -> Self {
        TwoStepSettings {
            standardize: true,
            auto_clusters: true,
            min_clusters: 2,
            max_clusters: 15,
            clusters: 10,
            criterion: Criterion::Bic,
            label: "cluster".into(),
            numeric_labels: false,
            name: None,
        }
    }
}

/// Leaf entries and inner nodes hold at most this many entries (`maxBranch`).
const MAX_BRANCH: usize = 8;
/// Levels of the CF tree, leaves included (`maxLevel`).
const MAX_LEVEL: usize = 3;

/// One input: continuous, or categorical with its categories.
enum Input {
    Continuous {
        position: usize,
    },
    Categorical {
        position: usize,
        categories: Vec<String>,
    },
}

/// The sufficient statistics of a set of records.
#[derive(Debug, Clone)]
struct Cf {
    n: f64,
    sum: Vec<f64>,
    sum_sq: Vec<f64>,
    counts: Vec<Vec<f64>>,
}

impl Cf {
    fn add(&self, other: &Cf) -> Cf {
        let zip = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| x + y).collect();
        Cf {
            n: self.n + other.n,
            sum: zip(&self.sum, &other.sum),
            sum_sq: zip(&self.sum_sq, &other.sum_sq),
            counts: self
                .counts
                .iter()
                .zip(&other.counts)
                .map(|(a, b)| zip(a, b))
                .collect(),
        }
    }

    /// xi = -n (sum_k 1/2 ln(D_k + var_k) + sum_k entropy_k).
    fn log_likelihood(&self, deltas: &[f64]) -> f64 {
        let mut total = 0.0;
        for ((delta, sum), sum_sq) in deltas.iter().zip(&self.sum).zip(&self.sum_sq) {
            let mean = sum / self.n;
            let var = (sum_sq / self.n - mean * mean).max(0.0);
            total += 0.5 * (delta + var).ln();
        }
        for counts in &self.counts {
            for &count in counts {
                if count > 0.0 {
                    let p = count / self.n;
                    total -= p * p.ln();
                }
            }
        }
        -self.n * total
    }
}

/// The log-likelihood distance and its parts.
struct Measure<'a> {
    deltas: &'a [f64],
}

impl Measure<'_> {
    fn distance(&self, a: &Cf, b: &Cf) -> f64 {
        a.log_likelihood(self.deltas) + b.log_likelihood(self.deltas)
            - a.add(b).log_likelihood(self.deltas)
    }
}

enum Node {
    Leaf(Vec<Cf>),
    Inner(Vec<(Cf, Node)>),
}

impl Node {
    fn summary(&self) -> Option<Cf> {
        let mut cfs: Box<dyn Iterator<Item = &Cf>> = match self {
            Node::Leaf(entries) => Box::new(entries.iter()),
            Node::Inner(children) => Box::new(children.iter().map(|(cf, _)| cf)),
        };
        let first = cfs.next()?.clone();
        Some(cfs.fold(first, |acc, cf| acc.add(cf)))
    }

    fn len(&self) -> usize {
        match self {
            Node::Leaf(entries) => entries.len(),
            Node::Inner(children) => children.len(),
        }
    }

    /// Inserts a record; returns the two nodes replacing this one when it splits.
    fn insert(&mut self, record: Cf, measure: &Measure<'_>) -> Option<(Node, Node)> {
        match self {
            Node::Leaf(entries) => {
                let closest = closest(entries.iter(), &record, measure);
                match closest {
                    // Threshold 0: only an identical record joins an entry.
                    Some((k, d)) if d <= 0.0 => entries[k] = entries[k].add(&record),
                    _ => entries.push(record),
                }
            }
            Node::Inner(children) => {
                let (k, _) = closest(children.iter().map(|(cf, _)| cf), &record, measure)?;
                let split = children[k].1.insert(record, measure);
                match split {
                    None => {
                        if let Some(cf) = children[k].1.summary() {
                            children[k].0 = cf;
                        }
                    }
                    Some((left, right)) => {
                        let (Some(l), Some(r)) = (left.summary(), right.summary()) else {
                            return None;
                        };
                        children.splice(k..=k, [(l, left), (r, right)]);
                    }
                }
            }
        }
        (self.len() > MAX_BRANCH).then(|| self.split(measure))
    }

    /// Splits around the farthest pair of entries: each entry joins the nearer seed (the first
    /// on a tie), keeping its order.
    fn split(&mut self, measure: &Measure<'_>) -> (Node, Node) {
        match self {
            Node::Leaf(entries) => {
                let (a, b) = farthest(entries, measure);
                let (left, right) = partition(std::mem::take(entries), a, b, |cf| cf, measure);
                (Node::Leaf(left), Node::Leaf(right))
            }
            Node::Inner(children) => {
                let cfs: Vec<Cf> = children.iter().map(|(cf, _)| cf.clone()).collect();
                let (a, b) = farthest(&cfs, measure);
                let (left, right) =
                    partition(std::mem::take(children), a, b, |(cf, _)| cf, measure);
                (Node::Inner(left), Node::Inner(right))
            }
        }
    }

    fn leaves(self, out: &mut Vec<Cf>) {
        match self {
            Node::Leaf(entries) => out.extend(entries),
            Node::Inner(children) => {
                for (_, child) in children {
                    child.leaves(out);
                }
            }
        }
    }
}

fn closest<'a>(
    cfs: impl Iterator<Item = &'a Cf>,
    record: &Cf,
    measure: &Measure<'_>,
) -> Option<(usize, f64)> {
    let mut best: Option<(usize, f64)> = None;
    for (k, cf) in cfs.enumerate() {
        let d = measure.distance(cf, record);
        if best.is_none_or(|(_, b)| d < b) {
            best = Some((k, d));
        }
    }
    best
}

fn farthest(cfs: &[Cf], measure: &Measure<'_>) -> (usize, usize) {
    let mut best = (0, 1, f64::NEG_INFINITY);
    for a in 0..cfs.len() {
        for b in a + 1..cfs.len() {
            let d = measure.distance(&cfs[a], &cfs[b]);
            if d > best.2 {
                best = (a, b, d);
            }
        }
    }
    (best.0, best.1)
}

fn partition<T>(
    items: Vec<T>,
    a: usize,
    b: usize,
    cf: impl Fn(&T) -> &Cf,
    measure: &Measure<'_>,
) -> (Vec<T>, Vec<T>) {
    let seed_a = cf(&items[a]).clone();
    let seed_b = cf(&items[b]).clone();
    let mut left = Vec::new();
    let mut right = Vec::new();
    for (k, item) in items.into_iter().enumerate() {
        let to_left = k == a
            || (k != b
                && measure.distance(cf(&item), &seed_a) <= measure.distance(cf(&item), &seed_b));
        if to_left {
            left.push(item);
        } else {
            right.push(item);
        }
    }
    (left, right)
}

/// Trains a TwoStep model on the `inputs` of `table`.
pub fn train(
    table: &Table,
    inputs: &[usize],
    settings: &TwoStepSettings,
) -> Result<Pmml, AlgoError> {
    let fields = table.model().fields();
    let missing = |position: usize, value: Option<&Value>| match value {
        None | Some(Value::Null) => true,
        Some(v) => fields.get(position).is_none_or(|f| f.is_missing(v)),
    };
    let rows: Vec<&Vec<Value>> = table
        .rows()
        .iter()
        .filter(|row| inputs.iter().all(|&p| !missing(p, row.get(p))))
        .collect();
    if rows.is_empty() {
        return Err(AlgoError::Data("no records without missing inputs".into()));
    }
    let n = rows.len() as f64;

    let mut columns = Vec::new();
    for &position in inputs {
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Data(format!("no field at position {position}")))?;
        if field.measurement == Measurement::Continuous {
            if !matches!(field.storage, Storage::Integer | Storage::Real) {
                return Err(AlgoError::Unsupported(format!(
                    "TwoStep input {} with non-numeric storage",
                    field.name
                )));
            }
            columns.push(Input::Continuous { position });
            continue;
        }
        let mut categories = field_categories(table, position);
        if matches!(field.values, ValueSpec::Flag { .. }) {
            categories.reverse();
        }
        for row in &rows {
            if let Some(value) = row.get(position).map(Value::to_string)
                && !categories.contains(&value)
            {
                categories.push(value);
            }
        }
        let seen = categories
            .iter()
            .filter(|c| {
                rows.iter()
                    .any(|row| row.get(position).is_some_and(|v| v.to_string() == **c))
            })
            .count();
        if seen >= 2 {
            columns.push(Input::Categorical {
                position,
                categories,
            });
        }
    }

    // Records as CFs, the continuous deltas.
    let record_cf = |row: &Vec<Value>| -> Cf {
        let mut sum = Vec::new();
        let mut counts = Vec::new();
        for column in &columns {
            match column {
                Input::Continuous { position } => {
                    sum.push(row.get(*position).and_then(Value::as_f64).unwrap_or(0.0));
                }
                Input::Categorical {
                    position,
                    categories,
                } => {
                    let text = row.get(*position).map(Value::to_string);
                    counts.push(
                        categories
                            .iter()
                            .map(|c| if text.as_deref() == Some(c) { 1.0 } else { 0.0 })
                            .collect(),
                    );
                }
            }
        }
        let sum_sq = sum.iter().map(|x| x * x).collect();
        Cf {
            n: 1.0,
            sum,
            sum_sq,
            counts,
        }
    };
    let records: Vec<Cf> = rows.iter().map(|row| record_cf(row)).collect();
    let mut deltas = Vec::new();
    for k in 0..records[0].sum.len() {
        if !settings.standardize {
            deltas.push(1.0);
            continue;
        }
        let mean = records.iter().map(|r| r.sum[k]).sum::<f64>() / n;
        let var = records
            .iter()
            .map(|r| (r.sum[k] - mean).powi(2))
            .sum::<f64>()
            / (n - 1.0);
        if var.is_nan() || var <= 0.0 {
            return Err(AlgoError::Unsupported(
                "TwoStep with a constant continuous input".into(),
            ));
        }
        deltas.push(var);
    }
    let measure = Measure { deltas: &deltas };

    // Step 1: the CF tree.
    let mut root = Node::Leaf(Vec::new());
    let mut levels = 1;
    for record in records {
        if let Some((left, right)) = root.insert(record, &measure) {
            levels += 1;
            if levels > MAX_LEVEL {
                return Err(AlgoError::Unsupported(
                    "TwoStep data that need a CF-tree rebuild (more than three levels)".into(),
                ));
            }
            let (Some(l), Some(r)) = (left.summary(), right.summary()) else {
                return Err(AlgoError::Data("an empty CF-tree node".into()));
            };
            root = Node::Inner(vec![(l, left), (r, right)]);
        }
    }
    let mut leaves = Vec::new();
    root.leaves(&mut leaves);

    // Step 2: agglomerative clustering of the leaf entries.
    let keep = if settings.auto_clusters {
        settings.max_clusters.max(1)
    } else {
        settings.clusters.max(1)
    };
    let (solutions, distances) = agglomerate(leaves, &measure, keep);
    let k_a = columns
        .iter()
        .filter(|c| matches!(c, Input::Continuous { .. }))
        .count();
    let levels_sum: usize = columns
        .iter()
        .map(|c| match c {
            Input::Categorical {
                categories,
                position,
            } => rows
                .iter()
                .filter_map(|row| row.get(*position).map(Value::to_string))
                .fold(Vec::<String>::new(), |mut seen, v| {
                    if !seen.contains(&v) && categories.contains(&v) {
                        seen.push(v);
                    }
                    seen
                })
                .len()
                .saturating_sub(1),
            Input::Continuous { .. } => 0,
        })
        .sum();
    let parameters = (2 * k_a + levels_sum) as f64;
    let criterion = |j: usize| -> Option<f64> {
        let clusters = solutions.get(&j)?;
        let fit: f64 = clusters.iter().map(|c| c.log_likelihood(&deltas)).sum();
        let penalty = match settings.criterion {
            Criterion::Bic => j as f64 * parameters * n.ln(),
            Criterion::Aic => 2.0 * j as f64 * parameters,
        };
        Some(-2.0 * fit + penalty)
    };
    let count = if settings.auto_clusters {
        choose(settings, &criterion, &distances)
    } else {
        settings.clusters
    };
    let clusters = solutions.get(&count).cloned().ok_or_else(|| {
        AlgoError::Data(format!(
            "cannot build {count} clusters from {} distinct records",
            distances.len()
        ))
    })?;
    Ok(pmml(table, &columns, &deltas, clusters, settings))
}

/// Merges the leaf entries down to one cluster. Returns the solutions with at most `keep`
/// clusters by size, and `distances[j]`: the distance of the merge that leaves `j` clusters.
fn agglomerate(
    leaves: Vec<Cf>,
    measure: &Measure<'_>,
    keep: usize,
) -> (std::collections::BTreeMap<usize, Vec<Cf>>, Vec<f64>) {
    let mut solutions = std::collections::BTreeMap::new();
    let mut clusters = leaves;
    let mut distances = vec![0.0; clusters.len()];
    let mut matrix: Vec<Vec<f64>> = (0..clusters.len())
        .map(|i| {
            (0..clusters.len())
                .map(|j| {
                    if j > i {
                        measure.distance(&clusters[i], &clusters[j])
                    } else {
                        0.0
                    }
                })
                .collect()
        })
        .collect();
    if clusters.len() <= keep {
        solutions.insert(clusters.len(), clusters.clone());
    }
    while clusters.len() > 1 {
        let mut best = (0, 1, f64::INFINITY);
        for (i, row) in matrix.iter().enumerate() {
            for (j, &d) in row.iter().enumerate().skip(i + 1) {
                if d < best.2 {
                    best = (i, j, d);
                }
            }
        }
        let (a, b, d) = best;
        clusters[a] = clusters[a].add(&clusters[b]);
        clusters.remove(b);
        matrix.remove(b);
        for row in &mut matrix {
            row.remove(b);
        }
        for j in 0..clusters.len() {
            if j != a {
                let (lo, hi) = (a.min(j), a.max(j));
                matrix[lo][hi] = measure.distance(&clusters[lo], &clusters[hi]);
            }
        }
        distances[clusters.len()] = d;
        if clusters.len() <= keep {
            solutions.insert(clusters.len(), clusters.clone());
        }
    }
    (solutions, distances)
}

/// The automatic number of clusters (see the module docs).
fn choose(
    settings: &TwoStepSettings,
    criterion: &dyn Fn(usize) -> Option<f64>,
    distances: &[f64],
) -> usize {
    let max = settings.max_clusters.max(1);
    let min = settings.min_clusters.clamp(1, max);
    let values: Vec<f64> = (1..=max).map_while(criterion).collect();
    let n = values.len();
    if n < 2 {
        return min.min(n.max(1));
    }
    let change = |j: usize| values[j - 1] - values[j];
    let first = change(1);
    if first < 0.0 {
        return 1;
    }
    let estimate = (1..n).find(|&j| change(j) / first < 0.04).unwrap_or(n);
    // d_j: the merge that leaves j - 1 clusters.
    let d = |j: usize| distances.get(j - 1).copied().unwrap_or(0.0);
    let mut ratios: Vec<(usize, f64)> = (min.max(2)..=estimate.min(max))
        .filter(|&j| j < max && j < distances.len() && d(j + 1) > 0.0)
        .map(|j| (j, d(j) / d(j + 1)))
        .collect();
    ratios.sort_by(|x, y| y.1.total_cmp(&x.1));
    match ratios.as_slice() {
        [] => min,
        [(j, _)] => *j,
        [(j1, r1), (j2, r2), ..] => {
            if r1 / r2 > 1.15 {
                *j1
            } else {
                (*j1).max(*j2)
            }
        }
    }
}

fn pmml(
    table: &Table,
    columns: &[Input],
    deltas: &[f64],
    clusters: Vec<Cf>,
    settings: &TwoStepSettings,
) -> Pmml {
    let fields = table.model().fields();
    let mut data_dictionary = Vec::new();
    let mut clustering_fields = Vec::new();
    let mut continuous = 0;
    for column in columns {
        let (position, values) = match column {
            Input::Continuous { position } => (*position, Vec::new()),
            Input::Categorical {
                position,
                categories,
            } => (*position, categories.clone()),
        };
        if let Some(mut field) = data_field(table, position) {
            if let Input::Categorical { .. } = column {
                field.values = values;
                field.optype = OpType::Categorical;
            } else {
                field.optype = OpType::Continuous;
            }
            data_dictionary.push(field);
        }
        let name = fields[position].name.clone();
        match column {
            Input::Continuous { .. } => {
                clustering_fields.push(ClusteringField {
                    field: name,
                    weight: deltas[continuous],
                    compare: None,
                    is_center_field: true,
                });
                continuous += 1;
            }
            Input::Categorical { .. } => clustering_fields.push(ClusteringField {
                field: name,
                weight: 1.0,
                compare: None,
                is_center_field: false,
            }),
        }
    }
    let categorical_names: Vec<String> = columns
        .iter()
        .filter_map(|c| match c {
            Input::Categorical { position, .. } => Some(fields[*position].name.clone()),
            Input::Continuous { .. } => None,
        })
        .collect();
    let clusters = clusters
        .into_iter()
        .enumerate()
        .map(|(index, cf)| {
            let name = if settings.numeric_labels {
                (index + 1).to_string()
            } else {
                format!("{}-{}", settings.label, index + 1)
            };
            let center = cf.sum.iter().map(|s| s / cf.n).collect();
            let variances = cf
                .sum
                .iter()
                .zip(&cf.sum_sq)
                .map(|(s, ss)| {
                    if cf.n > 1.0 {
                        ((ss - s * s / cf.n) / (cf.n - 1.0)).max(0.0)
                    } else {
                        0.0
                    }
                })
                .collect();
            Cluster {
                id: None,
                name: Some(name),
                size: Some(cf.n),
                center,
                variances,
                category_counts: categorical_names.iter().cloned().zip(cf.counts).collect(),
            }
        })
        .collect();
    let header = ModelHeader {
        name: settings.name.clone(),
        function: MiningFunction::Clustering,
        algorithm: Some("TwoStep".into()),
        mining_schema: columns
            .iter()
            .filter_map(|c| match c {
                Input::Continuous { position } | Input::Categorical { position, .. } => {
                    mining_field(table, *position, Usage::Active)
                }
            })
            .collect(),
        output: Vec::new(),
        local_transformations: Vec::new(),
    };
    Pmml {
        version: "4.3".into(),
        data_dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Clustering(ClusteringModel {
            header,
            model_class: ModelClass::DistributionBased,
            measure: ComparisonMeasure {
                kind: MeasureKind::Distance,
                metric: Metric::Euclidean,
                compare: CompareFunction::AbsDiff,
                log_likelihood: true,
            },
            fields: clustering_fields,
            clusters,
            missing_weights: Vec::new(),
        }),
    }
}
