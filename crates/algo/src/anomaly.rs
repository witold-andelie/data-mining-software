//! Anomaly Detection (`BuildAnomalyNode`; Algorithms Guide: "Anomaly Detection Algorithm"):
//! TwoStep clusters ("peer groups") of the inputs, then each record's log-likelihood distance
//! to its peer group relative to the group's mean distance (the anomaly index). Scoring is
//! `dms_pmml`'s `score_anomaly`.
//!
//! Evidence (`oracle/models_next/anomaly_default`: 48 records, inputs x, z continuous with
//! 2 and 1 missing values, cat with one empty string, flag, ord; default settings):
//! - Missing values are imputed: a continuous input takes its grand mean over valid values
//!   (saved as the `MiningField` replacement to 15 significant digits), and a missing or empty
//!   categorical value is one more category, the empty string.
//! - Categories are in order of first appearance in the data (`ord`: low middle high, while
//!   the Type lists high low middle). GAP: the empty category also comes last by first
//!   appearance here (record 10, after all four colours), so whether it is always last is not
//!   settled.
//! - Clustering is TwoStep's (CF tree with threshold 0, log-likelihood merges, BIC with the
//!   automatic number of clusters between `minPeerGroups` and `maxPeerGroups`), on the imputed
//!   records with D_k = the population variance of input k over `adjustmentCoeff` (the saved
//!   `fieldWeight`: 0.91253 for x, 0.13006 for z, from variances 5.4752 and 0.78036 over 6).
//!   The 15 merge distances (`X-Distances`) match to 1e-13, and BIC(1) to 1e-12.
//! - The merge leaves 33 and 15 records; the saved peer groups (32 and 16) are the training
//!   records nearest to each merged cluster by the log-likelihood distance (the guide's
//!   sufficient statistics "based on the cases in cluster h").
//! - The model lists the categorical inputs first, then the continuous ones, each group in
//!   input order; clusters save their size, means, sample variances and category counts.
//! - A cluster's `meanClusterDistance` is the mean deviation (log-likelihood distance to the
//!   nearest cluster) of the training records nearest to it, saved to 15 significant digits
//!   (Modeler's own sum differs in the last of them: 4.26301612350401 against 4.263016123503995).
//! - `perRecords` p: the ceil(p% of the records)-th largest training index, to six
//!   significant digits, is the cutoff (1% of 48: the largest, 1.372014362108979 -> 1.37201).
//!   GAP: only one case; whether the count is rounded up or to nearest with a minimum of one is
//!   not settled. `numRecords` n takes the n-th largest index the same way, and `indexLevel`
//!   is the cutoff itself (neither has an oracle case: GAP).
//!
//! Rejected: noise handling (`noiseLevel` > 0), records without imputation
//! (`imputeMissValues` false), frequency or analysis weights, and data that need a CF-tree
//! rebuild (TwoStep's limit).

use crate::AlgoError;
use crate::common::{data_field, mining_field};
use crate::twostep::{self, Cf, Criterion, TwoStepSettings};
use dms_pmml::{
    Cluster, ClusteringField, ClusteringModel, CompareFunction, ComparisonMeasure, MeasureKind,
    Metric, MiningFunction, Model, ModelClass, ModelHeader, OpType, Pmml, Usage,
};
use dms_types::{Measurement, Storage, Table, Value};

/// How the anomaly cutoff is chosen (`anomalyMethod`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnomalyCutoff {
    /// `perRecords`: this percentage of the training records are anomalies.
    Percent(f64),
    /// `numRecords`: this many training records are anomalies.
    Records(usize),
    /// `indexLevel`: records whose index exceeds this value.
    Index(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnomalySettings {
    pub cutoff: AnomalyCutoff,
    /// `autoPeerGroup` with `minPeerGroups`..`maxPeerGroups`, else `numPeerGroups`.
    pub auto_groups: bool,
    pub min_groups: usize,
    pub max_groups: usize,
    pub groups: usize,
    /// `adjustmentCoeff` (default 6).
    pub adjustment: f64,
    pub name: Option<String>,
}

const MAX_ROWS: usize = 1_000_000;

enum Input {
    Continuous {
        position: usize,
        mean: f64,
    },
    Categorical {
        position: usize,
        categories: Vec<String>,
    },
}

/// The category a categorical value counts as: missing, blank and empty values are the empty
/// category.
fn category(table: &Table, position: usize, value: Option<&Value>) -> String {
    let field = &table.model().fields()[position];
    match value {
        None => String::new(),
        Some(v) if field.is_missing(v) => String::new(),
        Some(v) => v.to_string(),
    }
}

/// Fifteen significant digits, as Modeler writes the model's own statistics.
fn digits(x: f64, significant: usize) -> f64 {
    format!("{:.*e}", significant.saturating_sub(1), x)
        .parse()
        .unwrap_or(x)
}

pub fn train(
    table: &Table,
    inputs: &[usize],
    settings: &AnomalySettings,
) -> Result<Pmml, AlgoError> {
    let fields = table.model().fields();
    let rows = table.rows();
    if rows.is_empty() {
        return Err(AlgoError::Data("no records to model".into()));
    }
    if rows.len() > MAX_ROWS {
        return Err(AlgoError::Unsupported(format!(
            "Anomaly Detection on more than {MAX_ROWS} records"
        )));
    }
    if !(settings.adjustment.is_finite() && settings.adjustment > 0.0) {
        return Err(AlgoError::Unsupported(
            "a non-positive adjustment coefficient".into(),
        ));
    }

    // Categorical inputs first, then continuous ones (the saved model's order).
    let mut categorical = Vec::new();
    let mut continuous = Vec::new();
    for &position in inputs {
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Data(format!("no field at position {position}")))?;
        if field.measurement == Measurement::Continuous {
            if !matches!(field.storage, Storage::Integer | Storage::Real) {
                return Err(AlgoError::Unsupported(format!(
                    "Anomaly Detection input {} with non-numeric storage",
                    field.name
                )));
            }
            let valid: Vec<f64> = rows
                .iter()
                .filter_map(|row| row.get(position))
                .filter(|v| !field.is_missing(v))
                .filter_map(Value::as_f64)
                .collect();
            if valid.is_empty() {
                continue;
            }
            let mean = digits(valid.iter().sum::<f64>() / valid.len() as f64, 15);
            continuous.push(Input::Continuous { position, mean });
        } else {
            let mut categories: Vec<String> = Vec::new();
            for row in rows {
                let value = category(table, position, row.get(position));
                if !categories.contains(&value) {
                    categories.push(value);
                }
            }
            categorical.push(Input::Categorical {
                position,
                categories,
            });
        }
    }
    let columns: Vec<Input> = categorical.into_iter().chain(continuous).collect();

    // Records as CFs, with imputed values.
    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        let mut sum = Vec::new();
        let mut counts = Vec::new();
        for column in &columns {
            match column {
                Input::Continuous { position, mean } => {
                    let x = row
                        .get(*position)
                        .filter(|v| !fields[*position].is_missing(v))
                        .and_then(Value::as_f64)
                        .unwrap_or(*mean);
                    if x.abs() > 1.0e150 {
                        return Err(AlgoError::Unsupported(
                            "inputs beyond 1e150 (the guide drops such records)".into(),
                        ));
                    }
                    sum.push(x);
                }
                Input::Categorical {
                    position,
                    categories,
                } => {
                    let value = category(table, *position, row.get(*position));
                    counts.push(
                        categories
                            .iter()
                            .map(|c| if *c == value { 1.0 } else { 0.0 })
                            .collect(),
                    );
                }
            }
        }
        let sum_sq = sum.iter().map(|x| x * x).collect();
        records.push(Cf {
            n: 1.0,
            sum,
            sum_sq,
            counts,
        });
    }
    // Inputs with a single value carry no information (the guide drops constant variables).
    let n = records.len() as f64;
    let mut deltas = Vec::new();
    for k in 0..records[0].sum.len() {
        let mean = records.iter().map(|r| r.sum[k]).sum::<f64>() / n;
        let var = records
            .iter()
            .map(|r| (r.sum[k] - mean).powi(2))
            .sum::<f64>()
            / n;
        if !var.is_finite() || var <= 0.0 {
            return Err(AlgoError::Unsupported(
                "Anomaly Detection with a constant continuous input".into(),
            ));
        }
        deltas.push(var / settings.adjustment);
    }
    for (k, column) in columns.iter().enumerate() {
        if let Input::Categorical { .. } = column {
            let seen = records
                .first()
                .map(|r| r.counts[k].len())
                .unwrap_or_default();
            let used = (0..seen)
                .filter(|&c| records.iter().any(|r| r.counts[k][c] > 0.0))
                .count();
            if used < 2 {
                return Err(AlgoError::Unsupported(
                    "Anomaly Detection with a constant categorical input".into(),
                ));
            }
        }
    }

    let levels: usize = columns
        .iter()
        .map(|c| match c {
            Input::Categorical { categories, .. } => categories.len().saturating_sub(1),
            Input::Continuous { .. } => 0,
        })
        .sum();
    let parameters = (2 * deltas.len() + levels) as f64;
    let clustering = TwoStepSettings {
        standardize: true,
        auto_clusters: settings.auto_groups,
        min_clusters: settings.min_groups,
        max_clusters: settings.max_groups,
        clusters: settings.groups,
        criterion: Criterion::Bic,
        label: String::new(),
        numeric_labels: true,
        name: settings.name.clone(),
    };
    let merged = twostep::cluster(records.clone(), &deltas, parameters, &clustering)?;
    // The peer groups are the training records nearest to each merged cluster.
    let mut clusters: Vec<Option<Cf>> = vec![None; merged.len()];
    for record in &records {
        let record_xi = record.log_likelihood(&deltas);
        let nearest = merged
            .iter()
            .map(|c| c.log_likelihood(&deltas) + record_xi - c.add(record).log_likelihood(&deltas))
            .enumerate()
            .fold(None, |best: Option<(usize, f64)>, (i, d)| match best {
                Some((_, b)) if b <= d => best,
                _ => Some((i, d)),
            })
            .map_or(0, |(i, _)| i);
        clusters[nearest] = Some(match clusters[nearest].take() {
            Some(cf) => cf.add(record),
            None => record.clone(),
        });
    }
    let clusters: Vec<Cf> = clusters.into_iter().flatten().collect();

    let mut pmml = model(table, &columns, &deltas, clusters, settings);
    // Each training record's deviation from its nearest cluster sets the clusters' mean
    // deviations, then the cutoff.
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    let scores = {
        let scorer = pmml
            .bind(&names)
            .map_err(|error| AlgoError::Data(format!("anomaly model: {error}")))?;
        rows.iter()
            .map(|row| {
                let record: Vec<dms_pmml::Value> = row
                    .iter()
                    .zip(fields)
                    .map(|(value, field)| {
                        if field.is_missing(value) {
                            dms_pmml::Value::Missing
                        } else if let Some(x) = value.as_f64() {
                            dms_pmml::Value::Number(x)
                        } else {
                            dms_pmml::Value::Text(value.to_string())
                        }
                    })
                    .collect();
                scorer
                    .score_anomaly(&record)
                    .map_err(|error| AlgoError::Data(format!("anomaly model: {error}")))?
                    .ok_or_else(|| AlgoError::Data("a training record was screened out".into()))
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let Model::Clustering(model) = &mut pmml.model else {
        return Err(AlgoError::Data("not a clustering model".into()));
    };
    for (index, cluster) in model.clusters.iter_mut().enumerate() {
        let deviations: Vec<f64> = scores
            .iter()
            .filter(|s| s.cluster == index)
            .map(|s| s.deviation)
            .collect();
        cluster.mean_distance = Some(if deviations.is_empty() {
            0.0
        } else {
            digits(deviations.iter().sum::<f64>() / deviations.len() as f64, 15)
        });
    }
    // The indices again, now against the saved mean deviations.
    let indices: Vec<f64> = scores
        .iter()
        .map(|s| {
            let mean = model.clusters[s.cluster].mean_distance.unwrap_or(0.0);
            if mean != 0.0 {
                s.deviation / mean
            } else {
                s.index
            }
        })
        .collect();
    let count = match settings.cutoff {
        AnomalyCutoff::Index(level) => {
            model.anomaly_cutoff = Some(level);
            return Ok(pmml);
        }
        AnomalyCutoff::Records(count) => count,
        AnomalyCutoff::Percent(percent) => (percent / 100.0 * n).ceil() as usize,
    }
    .clamp(1, indices.len());
    let mut sorted = indices;
    sorted.sort_by(|a, b| b.total_cmp(a));
    model.anomaly_cutoff = Some(digits(sorted[count - 1], 6));
    Ok(pmml)
}

fn model(
    table: &Table,
    columns: &[Input],
    deltas: &[f64],
    clusters: Vec<Cf>,
    settings: &AnomalySettings,
) -> Pmml {
    let fields = table.model().fields();
    let mut data_dictionary = Vec::new();
    let mut mining_schema = Vec::new();
    let mut clustering_fields = Vec::new();
    let mut continuous = 0;
    for column in columns {
        match column {
            Input::Continuous { position, mean } => {
                if let Some(mut field) = data_field(table, *position) {
                    field.optype = OpType::Continuous;
                    data_dictionary.push(field);
                }
                if let Some(mut field) = mining_field(table, *position, Usage::Active) {
                    field.missing_replacement = Some(mean.to_string());
                    mining_schema.push(field);
                }
                clustering_fields.push(ClusteringField {
                    field: fields[*position].name.clone(),
                    weight: deltas[continuous],
                    compare: None,
                    is_center_field: true,
                });
                continuous += 1;
            }
            Input::Categorical {
                position,
                categories,
            } => {
                if let Some(mut field) = data_field(table, *position) {
                    field.values = categories.clone();
                    field.optype = OpType::Categorical;
                    data_dictionary.push(field);
                }
                mining_schema.extend(mining_field(table, *position, Usage::Active));
                clustering_fields.push(ClusteringField {
                    field: fields[*position].name.clone(),
                    weight: 1.0,
                    compare: None,
                    is_center_field: true,
                });
            }
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
        .map(|(index, cf)| Cluster {
            id: None,
            name: Some(index.to_string()),
            size: Some(cf.n),
            center: cf.sum.iter().map(|s| s / cf.n).collect(),
            variances: cf
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
                .collect(),
            category_counts: categorical_names.iter().cloned().zip(cf.counts).collect(),
            mean_distance: None,
        })
        .collect();
    Pmml {
        version: "4.3".into(),
        data_dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Clustering(ClusteringModel {
            header: ModelHeader {
                name: settings.name.clone(),
                function: MiningFunction::Clustering,
                algorithm: Some("AnomalyDetection".into()),
                mining_schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
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
            anomaly_cutoff: None,
        }),
    }
}
