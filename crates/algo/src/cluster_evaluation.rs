//! Cluster evaluation (Algorithms Guide: "Cluster Evaluation Algorithms"; pp. 87-89).
//!
//! The Auto Cluster builder ranks trained components with the average silhouette coefficient.
//! The guide's data preparation is followed literally:
//!
//! - continuous variables are rescaled to `[-1, 1]` with `2 * (x - min) / (max - min) - 1`
//!   over the training records;
//! - categorical variables (including flags) use one-of-c indicator coding, in the ascending
//!   order of the field's categories (`crate::common::field_categories`);
//! - a case's distance to a centroid is Euclidean over the variables that are valid in that
//!   case. Missing values have weight 0, so they are left out of the distance; a cluster's
//!   centroid coordinate is the mean of the valid values that fall in the cluster, and stays
//!   unknown when the cluster has no valid value for that variable;
//! - `A` is the distance to the centroid of the case's own cluster, `B` the smallest distance
//!   to another non-empty cluster's centroid; the case coefficient is `(B - A) / max(A, B)`.
//!
//! Memberships are **the model's own scores** (`Pmml::bind` / `Scorer::score`), not a fresh
//! nearest-centroid pass. This matters: TwoStep's hierarchical clusters and its scoring
//! assignment differ (`numClusters` 2 but assignment counts 31/17), and K-Means scores in its
//! encoded `[0, 1]` space while the evaluation distances use `[-1, 1]`.
//!
//! Verified against `oracle/models_next2/acluster_default`: K-Means
//! `0.524354456082289`, TwoStep `0.419943931920704` to 1e-12, with assignment sizes
//! `[1, 13, 12, 11, 11]` and `[31, 17]`. The exact average includes cases with `A == 0`; skipping
//! them gives `0.514234338126593` and misses the oracle. A case whose `A` and `B` are both zero
//! contributes nothing (the coefficient is undefined), as does a case with no other non-empty
//! cluster. Categorical distance components beyond indicator coding and variable importance are
//! `GAP:` (no oracle): importance is reported as 0 when the builder does not calculate it.
//!
//! The remaining `trainPartitionStats` values are defined here too: cluster sizes are the
//! scoring-assignment counts in model order (non-empty clusters only), percentages are integer
//! floor `size * 100 / N`, the ratio is `smallest / largest`, and a component has one overlay
//! (`numOverlays="1"`).

use crate::AlgoError;
use crate::common::field_categories;
use dms_pmml::{Cluster, Model, Pmml, Value as PmmlValue};
use dms_types::{Measurement, Storage, Table, Value, ValueSpec};

/// One evaluated component, as the builder saves `trainPartitionStats`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClusterEvaluation {
    /// Average silhouette coefficient over the cases that have a defined coefficient.
    pub silhouette: f64,
    /// Number of cases in that average.
    pub cases: usize,
    /// The non-empty clusters, in model order.
    pub clusters: Vec<ClusterStats>,
    /// `trainPartitionStats/@numOverlays` (one trained component).
    pub num_overlays: usize,
    /// `trainPartitionStats/@importance` (0 when variable importance is not calculated).
    pub importance: f64,
}

/// The size of one non-empty cluster.
#[derive(Debug, Clone, PartialEq)]
pub struct ClusterStats {
    /// The model's cluster id or name (`1`, `cluster-1`, `X=0, Y=2`).
    pub label: String,
    /// The number of training records the model assigns to this cluster.
    pub size: usize,
    /// `floor(size * 100 / N)`.
    pub percent: u32,
}

impl ClusterEvaluation {
    pub fn num_clusters(&self) -> usize {
        self.clusters.len()
    }

    /// The smallest cluster (the first on a tie).
    pub fn smallest(&self) -> Option<&ClusterStats> {
        self.clusters.iter().min_by_key(|cluster| cluster.size)
    }

    /// The largest cluster (the first on a tie).
    pub fn largest(&self) -> Option<&ClusterStats> {
        self.clusters.iter().max_by_key(|cluster| cluster.size)
    }

    /// `trainPartitionStats/@clusterRatio`: the smallest size over the largest.
    pub fn cluster_ratio(&self) -> f64 {
        match (self.smallest(), self.largest()) {
            (Some(smallest), Some(largest)) if largest.size > 0 => {
                smallest.size as f64 / largest.size as f64
            }
            _ => 0.0,
        }
    }
}

/// Columns used by the frame-relative input indices.
#[derive(Debug, Clone, Copy)]
struct Position {
    /// The field's column in the table.
    table: usize,
}

/// One evaluated input variable and the columns it occupies.
struct Variable {
    position: Position,
    kind: Kind,
}

enum Kind {
    Continuous {
        low: f64,
        high: f64,
        /// First encoded column (one for a continuous variable).
        column: usize,
    },
    Categorical {
        categories: Vec<String>,
        column: usize,
    },
}

impl Variable {
    fn dimensions(&self) -> usize {
        match &self.kind {
            Kind::Continuous { .. } => 1,
            Kind::Categorical { categories, .. } => categories.len(),
        }
    }
}

/// A trained clustering model evaluated on the records it trained on.
///
/// `inputs` are table positions; they must cover every field the model needs. The result is
/// independent of the Pmml's own transformations: the evaluation frame is rebuilt from the raw
/// training values as the guide describes.
pub fn evaluate(
    model: &Pmml,
    table: &Table,
    inputs: &[usize],
) -> Result<ClusterEvaluation, AlgoError> {
    let clustering = match &model.model {
        Model::Clustering(clustering) => clustering,
        _ => {
            return Err(AlgoError::Unsupported(
                "cluster evaluation needs a clustering model".into(),
            ));
        }
    };
    if inputs.is_empty() {
        return Err(AlgoError::Settings(
            "cluster evaluation needs at least one input".into(),
        ));
    }
    let rows = table.rows();
    if rows.is_empty() {
        return Err(AlgoError::Data("no records to evaluate".into()));
    }
    let fields = table.model().fields();
    if inputs.len() > MAX_INPUTS || clustering.clusters.len() > MAX_CLUSTERS {
        return Err(AlgoError::Data(
            "cluster evaluation input or cluster limit exceeded".into(),
        ));
    }
    let dimensions = inputs.iter().try_fold(0usize, |total, &position| {
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Data(format!("no field at position {position}")))?;
        let width = variable_width(table, position, field)?;
        total
            .checked_add(width)
            .ok_or_else(|| AlgoError::Data("cluster evaluation has too many columns".into()))
    })?;
    let cells = rows
        .len()
        .checked_mul(dimensions)
        .ok_or_else(|| AlgoError::Data("cluster evaluation is too large".into()))?;
    if cells > MAX_CELLS {
        return Err(AlgoError::Data(
            "cluster evaluation training set is too large".into(),
        ));
    }

    let mut variables = Vec::with_capacity(inputs.len());
    let mut column = 0usize;
    for &position in inputs {
        crate::check_cancelled()?;
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Data(format!("no field at position {position}")))?;
        let kind = match &field.values {
            ValueSpec::Flag { .. } if field.measurement == Measurement::Flag => {
                let categories = field_categories(table, position);
                let start = column;
                column = column
                    .checked_add(categories.len())
                    .ok_or_else(|| AlgoError::Data("too many categories".into()))?;
                Kind::Categorical {
                    categories,
                    column: start,
                }
            }
            _ if field.measurement == Measurement::Continuous => {
                if !matches!(field.storage, Storage::Integer | Storage::Real) {
                    return Err(AlgoError::Unsupported(format!(
                        "cluster evaluation of continuous field {} with non-numeric storage",
                        field.name
                    )));
                }
                let (low, high) = observed_range(rows, position, field).ok_or_else(|| {
                    AlgoError::Data(format!("{} has no values to evaluate", field.name))
                })?;
                let start = column;
                column = column
                    .checked_add(1)
                    .ok_or_else(|| AlgoError::Data("too many columns".into()))?;
                Kind::Continuous {
                    low,
                    high,
                    column: start,
                }
            }
            _ => {
                let categories = field_categories(table, position);
                let start = column;
                column = column
                    .checked_add(categories.len())
                    .ok_or_else(|| AlgoError::Data("too many categories".into()))?;
                Kind::Categorical {
                    categories,
                    column: start,
                }
            }
        };
        variables.push(Variable {
            position: Position { table: position },
            kind,
        });
    }
    if column != dimensions {
        return Err(AlgoError::Data(
            "cluster evaluation column count changed while preparing".into(),
        ));
    }
    let names: Vec<&str> = inputs
        .iter()
        .filter_map(|&position| fields.get(position).map(|field| field.name.as_str()))
        .collect();
    let scorer = model.bind(&names).map_err(|error| {
        AlgoError::Data(format!(
            "cluster evaluation cannot score the model: {error}"
        ))
    })?;

    let cluster_identity: Vec<(Option<&str>, Option<&str>, String)> = clustering
        .clusters
        .iter()
        .enumerate()
        .map(|(index, cluster)| {
            (
                cluster.id.as_deref(),
                cluster.name.as_deref(),
                cluster_label(cluster, index),
            )
        })
        .collect();

    // Transform each record into the evaluation frame, score it, and remember its cluster.
    let mut points: Vec<Vec<f64>> = Vec::with_capacity(rows.len());
    let mut valid: Vec<Vec<bool>> = Vec::with_capacity(rows.len());
    let mut assignment: Vec<usize> = Vec::with_capacity(rows.len());
    for row in rows {
        crate::check_cancelled()?;
        let mut point = vec![0.0; dimensions];
        let mut valid_variables = Vec::with_capacity(variables.len());
        for variable in &variables {
            let field = fields.get(variable.position.table);
            let value = row
                .get(variable.position.table)
                .filter(|value| field.is_some_and(|field| !field.is_missing(value)));
            match &variable.kind {
                Kind::Continuous { low, high, column } => {
                    let number = value.and_then(Value::as_f64);
                    if let (Some(slot), Some(x)) = (point.get_mut(*column), number) {
                        *slot = if high > low {
                            2.0 * (x - low) / (high - low) - 1.0
                        } else {
                            0.0
                        };
                        valid_variables.push(true);
                    } else {
                        valid_variables.push(false);
                    }
                }
                Kind::Categorical { categories, column } => {
                    let text = value.map(Value::to_string);
                    for (offset, category) in categories.iter().enumerate() {
                        if let Some(slot) = point.get_mut(column.saturating_add(offset)) {
                            *slot = f64::from(u8::from(text.as_deref() == Some(category.as_str())));
                        }
                    }
                    valid_variables.push(text.is_some());
                }
            }
        }
        let record: Vec<PmmlValue> = inputs
            .iter()
            .map(|&position| {
                row.get(position)
                    .filter(|value| fields.get(position).is_some_and(|f| !f.is_missing(value)))
                    .map_or(PmmlValue::Missing, to_pmml)
            })
            .collect();
        let prediction = scorer.score(&record).map_err(|error| {
            AlgoError::Data(format!("cluster evaluation cannot score a record: {error}"))
        })?;
        let id = prediction.entity_id.as_deref();
        let value = match &prediction.value {
            PmmlValue::Text(text) => Some(text.as_str()),
            _ => None,
        };
        let cluster = cluster_identity
            .iter()
            .position(|(cluster_id, name, _)| {
                (id.is_some() && (*cluster_id == id || *name == id))
                    || (value.is_some() && (*cluster_id == value || *name == value))
            })
            .ok_or_else(|| {
                AlgoError::Data(format!(
                    "the model scored a cluster it does not define ({:?})",
                    id.or(value).unwrap_or("no label")
                ))
            })?;
        points.push(point);
        valid.push(valid_variables);
        assignment.push(cluster);
    }

    let mut sizes = vec![0usize; clustering.clusters.len()];
    for &cluster in &assignment {
        crate::check_cancelled()?;
        if let Some(size) = sizes.get_mut(cluster) {
            *size = size.saturating_add(1);
        }
    }
    let non_empty: Vec<usize> = sizes
        .iter()
        .enumerate()
        .filter_map(|(index, &size)| (size > 0).then_some(index))
        .collect();
    if non_empty.is_empty() {
        return Err(AlgoError::Data(
            "the model assigned no training record to a cluster".into(),
        ));
    }

    // Centroid coordinates: means of the valid transformed values per cluster.
    let mut sums = vec![vec![0.0f64; dimensions]; clustering.clusters.len()];
    let mut counts = vec![vec![0usize; dimensions]; clustering.clusters.len()];
    for ((point, valid_variables), &cluster) in points.iter().zip(&valid).zip(&assignment) {
        crate::check_cancelled()?;
        let Some(cluster_sums) = sums.get_mut(cluster) else {
            continue;
        };
        let Some(cluster_counts) = counts.get_mut(cluster) else {
            continue;
        };
        for (variable, &is_valid) in variables.iter().zip(valid_variables) {
            if !is_valid {
                continue;
            }
            match &variable.kind {
                Kind::Continuous { column, .. } => {
                    if let (Some(sum), Some(count), Some(&x)) = (
                        cluster_sums.get_mut(*column),
                        cluster_counts.get_mut(*column),
                        point.get(*column),
                    ) {
                        *sum += x;
                        *count = count.saturating_add(1);
                    }
                }
                Kind::Categorical { categories, column } => {
                    for offset in 0..categories.len() {
                        let at = column.saturating_add(offset);
                        if let (Some(sum), Some(count), Some(&x)) = (
                            cluster_sums.get_mut(at),
                            cluster_counts.get_mut(at),
                            point.get(at),
                        ) {
                            *sum += x;
                            *count = count.saturating_add(1);
                        }
                    }
                }
            }
        }
    }
    let centroids: Vec<Vec<Option<f64>>> = sums
        .iter()
        .zip(&counts)
        .map(|(sums, counts)| {
            sums.iter()
                .zip(counts)
                .map(|(&sum, &count)| (count > 0).then(|| sum / count as f64))
                .collect()
        })
        .collect();

    // Silhouette: A to the own centroid, B to the nearest other non-empty centroid.
    let mut total = 0.0f64;
    let mut cases = 0usize;
    for ((point, valid_variables), &own) in points.iter().zip(&valid).zip(&assignment) {
        crate::check_cancelled()?;
        let Some(own_centroid) = centroids.get(own) else {
            continue;
        };
        let Some(a) = distance(point, valid_variables, &variables, own_centroid) else {
            continue;
        };
        let mut b = f64::INFINITY;
        for &other in &non_empty {
            if other == own {
                continue;
            }
            let Some(other_centroid) = centroids.get(other) else {
                continue;
            };
            if let Some(distance) = distance(point, valid_variables, &variables, other_centroid) {
                b = b.min(distance);
            }
        }
        if !b.is_finite() {
            continue;
        }
        let largest = a.max(b);
        if largest > 0.0 {
            total += (b - a) / largest;
            cases = cases.saturating_add(1);
        }
    }
    let silhouette = if cases > 0 { total / cases as f64 } else { 0.0 };

    let n = rows.len();
    let clusters = non_empty
        .iter()
        .map(|&cluster| {
            let label = cluster_identity
                .get(cluster)
                .map_or_else(|| (cluster + 1).to_string(), |(_, _, label)| label.clone());
            let size = sizes.get(cluster).copied().unwrap_or(0);
            let percent = u32::try_from(size.saturating_mul(100) / n).unwrap_or(u32::MAX);
            ClusterStats {
                label,
                size,
                percent,
            }
        })
        .collect();

    Ok(ClusterEvaluation {
        silhouette,
        cases,
        clusters,
        num_overlays: 1,
        importance: 0.0,
    })
}

const MAX_INPUTS: usize = 4096;
const MAX_CLUSTERS: usize = 4096;
const MAX_CELLS: usize = 50_000_000;

fn variable_width(
    table: &Table,
    position: usize,
    field: &dms_types::Field,
) -> Result<usize, AlgoError> {
    match &field.values {
        ValueSpec::Flag { .. } if field.measurement == Measurement::Flag => {
            Ok(field_categories(table, position).len())
        }
        _ if field.measurement == Measurement::Continuous => {
            if matches!(field.storage, Storage::Integer | Storage::Real) {
                Ok(1)
            } else {
                Err(AlgoError::Unsupported(format!(
                    "cluster evaluation of continuous field {} with non-numeric storage",
                    field.name
                )))
            }
        }
        _ => Ok(field_categories(table, position).len()),
    }
}

fn observed_range(
    rows: &[Vec<Value>],
    position: usize,
    field: &dms_types::Field,
) -> Option<(f64, f64)> {
    rows.iter()
        .filter_map(|row| row.get(position))
        .filter(|value| !field.is_missing(value))
        .filter_map(Value::as_f64)
        .fold(None, |range, x| match range {
            None => Some((x, x)),
            Some((low, high)) => Some((low.min(x), high.max(x))),
        })
}

/// Euclid over the variables valid in both the case and the centroid; `None` when they share
/// no dimension.
fn distance(
    point: &[f64],
    valid: &[bool],
    variables: &[Variable],
    centroid: &[Option<f64>],
) -> Option<f64> {
    let mut sum = 0.0f64;
    let mut shared = false;
    let mut column = 0usize;
    for (variable, &is_valid) in variables.iter().zip(valid) {
        if crate::check_cancelled().is_err() {
            return None;
        }
        let dimensions = variable.dimensions();
        if is_valid {
            for offset in 0..dimensions {
                let at = column.saturating_add(offset);
                let (Some(&x), Some(Some(y))) = (point.get(at), centroid.get(at)) else {
                    continue;
                };
                let difference = x - y;
                sum += difference * difference;
                shared = true;
            }
        }
        column = column.saturating_add(dimensions);
    }
    shared.then(|| sum.sqrt())
}

fn cluster_label(cluster: &Cluster, index: usize) -> String {
    cluster
        .id
        .clone()
        .or_else(|| cluster.name.clone())
        .unwrap_or_else(|| (index + 1).to_string())
}

fn to_pmml(value: &Value) -> PmmlValue {
    match value {
        Value::Null => PmmlValue::Missing,
        Value::Integer(number) => PmmlValue::Number(*number as f64),
        Value::Real(number) => PmmlValue::Number(*number),
        Value::String(text) => PmmlValue::Text(text.to_string()),
        Value::Date(number) | Value::Time(number) | Value::Timestamp(number) => {
            PmmlValue::Number(*number as f64)
        }
        Value::List(_) => PmmlValue::Text(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, reason = "tests fail loudly")]

    use super::evaluate;
    use dms_pmml::{
        Cluster, ClusteringField, ClusteringModel, CompareFunction, ComparisonMeasure, MeasureKind,
        Metric, MiningFunction, Model, ModelClass, ModelHeader, Pmml, Value as PmmlValue,
    };
    use dms_types::{DataModel, Field, Measurement, Role, Storage, Table, Value};

    fn table(rows: Vec<[f64; 2]>) -> Table {
        let fields = vec![
            {
                let mut field = Field::new("a", Storage::Real);
                field.measurement = Measurement::Continuous;
                field.role = Role::Input;
                field
            },
            {
                let mut field = Field::new("b", Storage::Real);
                field.measurement = Measurement::Continuous;
                field.role = Role::Input;
                field
            },
        ];
        let rows = rows
            .into_iter()
            .map(|[a, b]| vec![Value::Real(a), Value::Real(b)])
            .collect();
        Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
    }

    fn two_cluster_model() -> Pmml {
        let clusters = vec![
            Cluster {
                id: None,
                name: Some("1".into()),
                size: Some(2.0),
                center: vec![1.0, 1.0],
                variances: Vec::new(),
                category_counts: Vec::new(),
                mean_distance: None,
                map_position: None,
            },
            Cluster {
                id: None,
                name: Some("2".into()),
                size: Some(2.0),
                center: vec![9.0, 9.0],
                variances: Vec::new(),
                category_counts: Vec::new(),
                mean_distance: None,
                map_position: None,
            },
        ];
        let fields = ["a", "b"]
            .into_iter()
            .map(|field| ClusteringField {
                field: field.into(),
                weight: 1.0,
                compare: Some(CompareFunction::AbsDiff),
                is_center_field: true,
            })
            .collect();
        Pmml {
            version: "4.3".into(),
            data_dictionary: Vec::new(),
            transformations: Vec::new(),
            functions: Vec::new(),
            model: Model::Clustering(ClusteringModel {
                header: ModelHeader {
                    name: None,
                    function: MiningFunction::Clustering,
                    algorithm: Some("KMeans".into()),
                    mining_schema: Vec::new(),
                    output: Vec::new(),
                    local_transformations: Vec::new(),
                },
                model_class: ModelClass::CenterBased,
                measure: ComparisonMeasure {
                    kind: MeasureKind::Distance,
                    metric: Metric::Euclidean,
                    compare: CompareFunction::AbsDiff,
                    log_likelihood: false,
                },
                fields,
                clusters,
                missing_weights: vec![1.0, 1.0],
                anomaly_cutoff: None,
            }),
        }
    }

    #[test]
    fn silhouette_uses_scaled_data_and_the_models_assignment() {
        // Points 0/0, 0/2 and 10/10, 10/12 form two obvious groups. Continuous rescaling maps
        // 0..10 to -1..1 (b 0..12 maps to -1..1), so the coefficient is the same as on the raw
        // points: each case is 0.058823... away from its own centroid and ~0.78 from the other.
        let table = table(vec![[0.0, 0.0], [0.0, 2.0], [10.0, 10.0], [10.0, 12.0]]);
        let evaluation = evaluate(&two_cluster_model(), &table, &[0, 1]).unwrap();
        assert_eq!(evaluation.num_clusters(), 2);
        assert_eq!(evaluation.clusters[0].size, 2);
        assert_eq!(evaluation.clusters[1].size, 2);
        assert_eq!(evaluation.clusters[0].percent, 50);
        assert_eq!(evaluation.cases, 4);
        assert!(evaluation.silhouette > 0.7 && evaluation.silhouette < 1.0);
        assert_eq!(evaluation.cluster_ratio(), 1.0);
        assert_eq!(evaluation.num_overlays, 1);
        assert_eq!(evaluation.importance, 0.0);
    }

    #[test]
    fn missing_values_have_weight_zero() {
        let mut table = table(vec![[0.0, 0.0], [0.0, 2.0], [10.0, 10.0], [10.0, 12.0]]);
        table = Table::new(
            table.model().clone(),
            table
                .rows()
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    if index == 0 {
                        vec![Value::Null, row[1].clone()]
                    } else {
                        row.clone()
                    }
                })
                .collect(),
        )
        .unwrap();
        let evaluation = evaluate(&two_cluster_model(), &table, &[0, 1]).unwrap();
        assert_eq!(evaluation.num_clusters(), 2);
        assert_eq!(evaluation.cases, 4);
        assert!(evaluation.silhouette.is_finite());
    }

    #[test]
    fn a_constant_field_does_not_divide_by_zero() {
        let table = table(vec![[1.0, 0.0], [1.0, 2.0], [1.0, 10.0], [1.0, 12.0]]);
        let evaluation = evaluate(&two_cluster_model(), &table, &[0, 1]).unwrap();
        assert!(evaluation.silhouette.is_finite());
        assert_eq!(evaluation.num_clusters(), 2);
    }

    #[test]
    fn a_non_clustering_model_is_unsupported() {
        let mut model = two_cluster_model();
        model.model = Model::Unsupported("not clustering".into());
        let error = evaluate(&model, &table(vec![[0.0, 0.0]]), &[0, 1]).unwrap_err();
        assert!(matches!(error, crate::AlgoError::Unsupported(_)));
    }

    #[test]
    fn a_non_clustering_value_is_not_scored() {
        let mut model = two_cluster_model();
        if let Model::Clustering(clustering) = &mut model.model {
            clustering.clusters[0].center = vec![0.0, 0.0];
            clustering.clusters[1].center = vec![0.0, 0.0];
        }
        let table = table(vec![[0.0, 0.0], [10.0, 10.0]]);
        let evaluation = evaluate(&model, &table, &[0, 1]).unwrap();
        // One cluster wins every case; the coefficient is undefined and the average is 0.
        assert_eq!(evaluation.num_clusters(), 1);
        assert_eq!(evaluation.cases, 0);
        assert_eq!(evaluation.silhouette, 0.0);
    }

    #[test]
    fn pmml_values_keep_integer_inputs() {
        let value = dms_types::Value::Integer(3);
        assert_eq!(super::to_pmml(&value), PmmlValue::Number(3.0));
    }
}
