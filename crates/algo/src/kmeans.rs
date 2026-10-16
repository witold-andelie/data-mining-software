//! K-Means (Algorithms Guide: "K-Means Algorithm"): fields are encoded to `[0, 1]`, initial
//! centers are chosen by maximin from the first record, and records are reassigned to the nearest
//! center until no assignment changes, the centers move less than the tolerance, or the iteration
//! limit is reached.
//!
//! Encoding (the `LocalTransformations` of Modeler's model, `models/kmeans_categorical`):
//! continuous fields `(x - min) / (max - min)` over the training records, missing 0.5; flags 1
//! for the true value, 0 for the false one, missing 0.5; other categorical fields one indicator
//! per category scaled by the set encoding value (default 0.707107), missing all 0.
//!
//! The textbook procedure reproduces all three `models/kmeans_*` oracles exactly: the same
//! cluster sizes in the same order and the same centers (Modeler prints them to six digits).
//! Distance ties go to the earlier center; an empty cluster keeps its center. The model keeps
//! centers and the set encoding value to six significant digits, like Modeler's, whose nugget
//! computes distances (`$KMD-`) with them.
//!
//! The model scores through `dms-pmml`: continuous fields as `NormContinuous`, flags as
//! `NormDiscrete` of the true value, categories as `NormDiscrete` times the encoding value
//! (Modeler writes `MapValues`, which scores the same).

use crate::AlgoError;
use crate::common::{data_field, field_categories, mining_field};
use dms_pmml::{
    Cluster, ClusteringField, ClusteringModel, CompareFunction, ComparisonMeasure, DataType,
    DerivedField, Expr, MeasureKind, Metric, MiningFunction, Model, ModelClass, ModelHeader,
    OpType, Outliers, Pmml, Usage, Value as PmmlValue,
};
use dms_types::{Measurement, Storage, Table, Value, ValueSpec};

/// Settings of a K-Means build node (Modeler's defaults in [`Default`]).
#[derive(Debug, Clone, PartialEq)]
pub struct KMeansSettings {
    /// `num_clusters` (default 5).
    pub clusters: usize,
    /// `max_iterations` (default 20).
    pub max_iterations: usize,
    /// `tolerance` (default 1e-6): the largest center move that still continues.
    pub tolerance: f64,
    /// The weight of a category indicator (`encoding_value`, default 0.707107: Modeler's
    /// `sqrt(0.5)` to six digits, which it also trains with; `models/kmeans_categorical` centers
    /// such as 0.181828 = 0.707107 * 18 / 70 come out one digit off with the exact root).
    pub set_encoding: f64,
    /// The model name (`modelName`).
    pub name: Option<String>,
}

impl Default for KMeansSettings {
    fn default() -> Self {
        KMeansSettings {
            clusters: 5,
            max_iterations: 20,
            tolerance: 1e-6,
            set_encoding: six_digits(std::f64::consts::FRAC_1_SQRT_2),
            name: None,
        }
    }
}

/// How one input field becomes encoded columns.
enum Encoding {
    Range { low: f64, high: f64 },
    Flag { true_value: String },
    Set { categories: Vec<String> },
}

/// Trains a K-Means model on the `inputs` of `table`.
pub fn train(
    table: &Table,
    inputs: &[usize],
    settings: &KMeansSettings,
) -> Result<Pmml, AlgoError> {
    if settings.clusters == 0 {
        return Err(AlgoError::Settings(
            "K-Means needs at least one cluster".into(),
        ));
    }
    let fields = table.model().fields();
    let rows = table.rows();
    if rows.is_empty() {
        return Err(AlgoError::Data("no records to cluster".into()));
    }
    let mut encodings = Vec::new();
    for &position in inputs {
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Data(format!("no field at position {position}")))?;
        let numeric = matches!(field.storage, Storage::Integer | Storage::Real);
        let encoding = match (&field.values, field.measurement) {
            (_, Measurement::Continuous) if numeric => {
                let values = rows
                    .iter()
                    .filter_map(|row| row.get(position))
                    .filter(|v| !field.is_missing(v))
                    .filter_map(Value::as_f64);
                let (low, high) = values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
                    (lo.min(x), hi.max(x))
                });
                if !low.is_finite() {
                    return Err(AlgoError::Data(format!("{} has no values", field.name)));
                }
                Encoding::Range { low, high }
            }
            (_, Measurement::Continuous) => {
                return Err(AlgoError::Unsupported(format!(
                    "K-Means input {} with non-numeric storage",
                    field.name
                )));
            }
            (ValueSpec::Flag { true_value, .. }, Measurement::Flag) => Encoding::Flag {
                true_value: true_value.to_string(),
            },
            _ => Encoding::Set {
                categories: field_categories(table, position),
            },
        };
        encodings.push(encoding);
    }

    // Encoded records.
    let encoded: Vec<Vec<f64>> = rows
        .iter()
        .map(|row| {
            let mut point = Vec::new();
            for (encoding, &position) in encodings.iter().zip(inputs) {
                let value = row
                    .get(position)
                    .filter(|v| fields.get(position).is_some_and(|f| !f.is_missing(v)));
                match encoding {
                    Encoding::Range { low, high } => point.push(
                        value
                            .and_then(Value::as_f64)
                            .map_or(0.5, |x| scale(x, *low, *high)),
                    ),
                    Encoding::Flag { true_value } => point.push(value.map_or(0.5, |v| {
                        if v.to_string() == *true_value {
                            1.0
                        } else {
                            0.0
                        }
                    })),
                    Encoding::Set { categories } => {
                        let text = value.map(Value::to_string);
                        point.extend(categories.iter().map(|c| {
                            if text.as_deref() == Some(c.as_str()) {
                                settings.set_encoding
                            } else {
                                0.0
                            }
                        }));
                    }
                }
            }
            point
        })
        .collect();

    let (centers, sizes) = cluster(&encoded, settings);

    // PMML: derived (encoded) fields, then the centers.
    let mut derived = Vec::new();
    for (encoding, &position) in encodings.iter().zip(inputs) {
        let name = fields[position].name.clone();
        let mut push = |expr: Expr| {
            derived.push(DerivedField {
                name: format!("cluster{}", derived.len()),
                data_type: DataType::Double,
                optype: OpType::Continuous,
                expr,
            });
        };
        match encoding {
            Encoding::Range { low, high } => push(Expr::NormContinuous {
                field: name,
                points: if high > low {
                    vec![(*low, 0.0), (*high, 1.0)]
                } else {
                    vec![(*low, 0.0), (*low + 1.0, 0.0)]
                },
                outliers: Outliers::AsIs,
                map_missing_to: Some(0.5),
            }),
            Encoding::Flag { true_value } => push(Expr::NormDiscrete {
                field: name,
                value: true_value.clone(),
                map_missing_to: Some(0.5),
            }),
            Encoding::Set { categories } => {
                for category in categories {
                    push(Expr::Apply {
                        function: "*".into(),
                        args: vec![
                            Expr::NormDiscrete {
                                field: name.clone(),
                                value: category.clone(),
                                map_missing_to: Some(0.0),
                            },
                            Expr::Constant(PmmlValue::Number(six_digits(settings.set_encoding))),
                        ],
                        map_missing_to: None,
                    });
                }
            }
        }
    }
    let clustering_fields = derived
        .iter()
        .map(|d| ClusteringField {
            field: d.name.clone(),
            weight: 1.0,
            compare: Some(CompareFunction::AbsDiff),
            is_center_field: true,
        })
        .collect();
    let clusters = centers
        .into_iter()
        .zip(sizes)
        .enumerate()
        .map(|(index, (center, size))| Cluster {
            id: None,
            name: Some((index + 1).to_string()),
            size: Some(size as f64),
            center: center.into_iter().map(six_digits).collect(),
        })
        .collect();
    let header = ModelHeader {
        name: settings.name.clone(),
        function: MiningFunction::Clustering,
        algorithm: Some("KMeans".into()),
        mining_schema: inputs
            .iter()
            .filter_map(|&p| mining_field(table, p, Usage::Active))
            .collect(),
        output: Vec::new(),
        local_transformations: derived,
    };
    let missing_weights = vec![1.0; header.local_transformations.len()];
    Ok(Pmml {
        version: "4.2".into(),
        data_dictionary: inputs
            .iter()
            .filter_map(|&p| data_field(table, p))
            .collect(),
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Clustering(ClusteringModel {
            header,
            model_class: ModelClass::CenterBased,
            measure: ComparisonMeasure {
                kind: MeasureKind::Distance,
                metric: Metric::Euclidean,
                compare: CompareFunction::AbsDiff,
            },
            fields: clustering_fields,
            clusters,
            missing_weights,
        }),
    })
}

/// A number rounded to six significant digits, as Modeler saves K-Means centers and the set
/// encoding value; its nugget scores with them (`models/kmeans_3clusters` distances match the
/// rounded centers, not the exact ones).
fn six_digits(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    format!("{x:.5e}").parse().unwrap_or(x)
}

fn scale(x: f64, low: f64, high: f64) -> f64 {
    if high > low {
        (x - low) / (high - low)
    } else {
        0.0
    }
}

fn distance2(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
}

/// The nearest center, the earlier one on ties.
fn nearest(point: &[f64], centers: &[Vec<f64>]) -> usize {
    let mut best = 0;
    let mut best_distance = f64::INFINITY;
    for (j, center) in centers.iter().enumerate() {
        let d = distance2(point, center);
        if d < best_distance {
            best = j;
            best_distance = d;
        }
    }
    best
}

/// Maximin initial centers, then assign/update iterations: the centers and cluster sizes.
fn cluster(points: &[Vec<f64>], settings: &KMeansSettings) -> (Vec<Vec<f64>>, Vec<usize>) {
    let k = settings.clusters.min(points.len());
    let mut centers: Vec<Vec<f64>> = vec![points[0].clone()];
    // Each record's distance to its nearest center so far.
    let mut nearest_distance: Vec<f64> = points.iter().map(|p| distance2(p, &points[0])).collect();
    while centers.len() < k {
        let mut far = 0;
        for (i, &d) in nearest_distance.iter().enumerate() {
            if d > nearest_distance[far] {
                far = i;
            }
        }
        let center = points[far].clone();
        for (i, p) in points.iter().enumerate() {
            nearest_distance[i] = nearest_distance[i].min(distance2(p, &center));
        }
        centers.push(center);
    }

    let dimensions = points[0].len();
    let mut assignment: Vec<usize> = Vec::new();
    for _ in 0..settings.max_iterations.max(1) {
        let next: Vec<usize> = points.iter().map(|p| nearest(p, &centers)).collect();
        let mut sums = vec![vec![0.0; dimensions]; centers.len()];
        let mut counts = vec![0usize; centers.len()];
        for (p, &j) in points.iter().zip(&next) {
            counts[j] += 1;
            for (s, x) in sums[j].iter_mut().zip(p) {
                *s += x;
            }
        }
        let mut largest_move = 0.0_f64;
        for (j, center) in centers.iter_mut().enumerate() {
            if counts[j] == 0 {
                continue;
            }
            let updated: Vec<f64> = sums[j].iter().map(|s| s / counts[j] as f64).collect();
            largest_move = largest_move.max(distance2(center, &updated).sqrt());
            *center = updated;
        }
        let unchanged = next == assignment;
        assignment = next;
        if unchanged || largest_move < settings.tolerance {
            break;
        }
    }
    let mut sizes = vec![0usize; centers.len()];
    for &j in &assignment {
        sizes[j] += 1;
    }
    (centers, sizes)
}
