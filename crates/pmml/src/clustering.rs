//! Scoring of PMML 4.3 center-based `ClusteringModel` (distance/similarity metrics, compare
//! functions, field weights, and missing-value weight adjustments), and of SPSS TwoStep's
//! distribution-based models with the log-likelihood distance (`log_likelihood`).
//!
//! TwoStep (Chiu et al. 2001; `models/twostep_default`, `team/cluster_analysis`): a cluster v
//! of size N_v has the log-likelihood
//! xi_v = -N_v * (sum_k 1/2 ln(s2_k + s2_vk) + sum_k E_vk)
//! over the continuous fields k (DataField optype), with s2_k the field's variance over the training data
//! (its `fieldWeight`) and s2_vk the cluster's variance as the PMML saves it (the sample
//! variance; training merges with the maximum-likelihood one, but Modeler's nugget scores with the
//! saved value: `twostep/ts_drug_cont` record 64 goes to cluster-4 by 0.92781 against 0.92820,
//! where the maximum-likelihood variance would choose cluster-2), and E_vk = -sum_l (N_vkl / N_v) ln(N_vkl / N_v) the
//! entropy of each categorical field's counts. A record joins the cluster j minimising
//! d(j, s) = xi_j + xi_s - xi_<j,s>, the first on a tie; the distance is its affinity.

use crate::expr::{as_f64, value_matches_str};
use crate::score::{Env, empty_prediction};
use crate::{
    ClusteringModel, CompareFunction, MeasureKind, Metric, ModelClass, Pmml, PmmlError, Prediction,
    Value,
};

pub(crate) fn score(
    pmml: &Pmml,
    model: &ClusteringModel,
    env: &Env<'_>,
) -> Result<Prediction, PmmlError> {
    if model.model_class == ModelClass::DistributionBased {
        if model.measure.log_likelihood {
            return score_log_likelihood(pmml, model, env);
        }
        return Err(PmmlError::Unsupported(
            "distributionBased ClusteringModel without the log-likelihood distance".into(),
        ));
    }

    let center_fields: Vec<&crate::ClusteringField> =
        model.fields.iter().filter(|f| f.is_center_field).collect();
    let m = center_fields.len();
    if m == 0 || model.clusters.is_empty() {
        return Ok(empty_prediction());
    }

    // Gather record coordinates and compute the PMML 4.3 missing-value adjustment factor:
    // adjust_M = (sum_{i=1}^m q_i) / (sum_{i=1}^m non_missing(x_i) * q_i).
    let mut coords: Vec<Option<f64>> = Vec::with_capacity(m);
    let mut total_q = 0.0;
    let mut present_q = 0.0;

    for (i, cf) in center_fields.iter().enumerate() {
        let q_i = model.missing_weights.get(i).copied().unwrap_or(1.0);
        total_q += q_i;
        let val = env
            .get(&cf.field)
            .ok_or_else(|| PmmlError::UnknownField(cf.field.clone()))?;
        if *val == Value::Missing {
            coords.push(None);
        } else {
            coords.push(Some(as_f64(val, &cf.field)?));
            present_q += q_i;
        }
    }

    if present_q <= 0.0 {
        return Ok(empty_prediction());
    }
    let adjust_m = total_q / present_q;

    let mut best: Option<(usize, &crate::Cluster, f64)> = None;

    for (idx, cluster) in model.clusters.iter().enumerate() {
        if cluster.center.len() != m {
            return Err(PmmlError::Score(format!(
                "cluster {} center length {} does not match {} center fields",
                idx + 1,
                cluster.center.len(),
                m
            )));
        }

        let affinity = compute_affinity(model, &center_fields, &coords, &cluster.center, adjust_m)?;

        let is_better = match best {
            None => true,
            Some((_, _, best_aff)) => match model.measure.kind {
                MeasureKind::Distance => affinity < best_aff,
                MeasureKind::Similarity => affinity > best_aff,
            },
        };
        if is_better {
            best = Some((idx, cluster, affinity));
        }
    }

    let Some((idx, cluster, affinity)) = best else {
        return Ok(empty_prediction());
    };

    let cluster_id = cluster
        .id
        .clone()
        .or_else(|| cluster.name.clone())
        .unwrap_or_else(|| (idx + 1).to_string());
    let cluster_val = cluster
        .name
        .clone()
        .or_else(|| cluster.id.clone())
        .unwrap_or_else(|| (idx + 1).to_string());

    Ok(Prediction {
        value: Value::Text(cluster_val),
        confidence: None,
        probabilities: Vec::new(),
        entity_id: Some(cluster_id),
        affinity: Some(affinity),
    })
}

fn compute_affinity(
    model: &ClusteringModel,
    center_fields: &[&crate::ClusteringField],
    coords: &[Option<f64>],
    center: &[f64],
    adjust_m: f64,
) -> Result<f64, PmmlError> {
    let mut sum = 0.0;
    let mut max_val = 0.0;

    for ((cf, &opt_x), &y) in center_fields.iter().zip(coords).zip(center) {
        let Some(x) = opt_x else {
            continue;
        };
        let cmp = cf.compare.unwrap_or(model.measure.compare);
        let d = eval_compare(cmp, x, y)?;
        let w = cf.weight;

        match model.measure.metric {
            Metric::Euclidean | Metric::SquaredEuclidean => {
                sum += w * d * d;
            }
            Metric::CityBlock => {
                sum += w * d;
            }
            Metric::Chebychev => {
                let wd = w * d;
                if wd > max_val {
                    max_val = wd;
                }
            }
            Metric::Minkowski(p) => {
                sum += w * d.powf(p);
            }
        }
    }

    let out = match model.measure.metric {
        Metric::SquaredEuclidean => sum * adjust_m,
        Metric::Euclidean => (sum * adjust_m).sqrt(),
        Metric::CityBlock => sum * adjust_m,
        Metric::Chebychev => max_val * adjust_m,
        Metric::Minkowski(p) => (sum * adjust_m).powf(1.0 / p),
    };

    Ok(out)
}

fn eval_compare(cmp: CompareFunction, x: f64, y: f64) -> Result<f64, PmmlError> {
    let diff = (x - y).abs();
    match cmp {
        CompareFunction::AbsDiff => Ok(diff),
        CompareFunction::GaussSim(s) => {
            let z = diff / s;
            Ok((-std::f64::consts::LN_2 * z * z).exp())
        }
        CompareFunction::Delta => Ok(if diff == 0.0 { 0.0 } else { 1.0 }),
        CompareFunction::Equal => Ok(if diff == 0.0 { 1.0 } else { 0.0 }),
        CompareFunction::Table => Err(PmmlError::Unsupported("table compareFunction".into())),
    }
}

/// Sufficient statistics of a TwoStep cluster: size, per continuous field the mean and the
/// maximum-likelihood variance, per categorical field the category counts.
struct Stats {
    n: f64,
    means: Vec<f64>,
    variances: Vec<f64>,
    counts: Vec<Vec<f64>>,
}

impl Stats {
    /// xi = -n * (sum_k 1/2 ln(total_k + var_k) + sum_k entropy_k).
    fn log_likelihood(&self, totals: &[f64]) -> f64 {
        let continuous: f64 = totals
            .iter()
            .zip(&self.variances)
            .map(|(total, var)| 0.5 * (total + var).ln())
            .sum();
        let entropy: f64 = self
            .counts
            .iter()
            .flat_map(|counts| counts.iter())
            .filter(|&&count| count > 0.0)
            .map(|&count| {
                let p = count / self.n;
                -p * p.ln()
            })
            .sum();
        -self.n * (continuous + entropy)
    }

    /// This cluster with one more record: `x` its continuous values, `categories` the index of
    /// its category per categorical field.
    fn with(&self, x: &[f64], categories: &[usize]) -> Stats {
        let n = self.n + 1.0;
        let mut means = Vec::with_capacity(x.len());
        let mut variances = Vec::with_capacity(x.len());
        for ((&mean, &var), &value) in self.means.iter().zip(&self.variances).zip(x) {
            let delta = value - mean;
            means.push(mean + delta / n);
            variances.push((self.n * var + self.n * delta * delta / n) / n);
        }
        let counts = self
            .counts
            .iter()
            .zip(categories)
            .map(|(counts, &category)| {
                let mut counts = counts.clone();
                if counts.len() <= category {
                    counts.resize(category + 1, 0.0);
                }
                counts[category] += 1.0;
                counts
            })
            .collect();
        Stats {
            n,
            means,
            variances,
            counts,
        }
    }
}

fn score_log_likelihood(
    pmml: &Pmml,
    model: &ClusteringModel,
    env: &Env<'_>,
) -> Result<Prediction, PmmlError> {
    // The continuous fields own the center coordinates (TwoStep leaves `isCenterField` unset,
    // so true, on its categorical fields); the DataField's optype tells them apart.
    let is_continuous = |field: &crate::ClusteringField| {
        pmml.data_dictionary
            .iter()
            .find(|f| f.name == field.field)
            .is_some_and(|f| f.optype == crate::OpType::Continuous)
    };
    let (continuous, categorical): (Vec<&crate::ClusteringField>, Vec<_>) =
        model.fields.iter().partition(|f| is_continuous(f));
    let totals: Vec<f64> = continuous.iter().map(|f| f.weight).collect();

    let mut x = Vec::with_capacity(continuous.len());
    for field in &continuous {
        let value = env
            .get(&field.field)
            .ok_or_else(|| PmmlError::UnknownField(field.field.clone()))?;
        if *value == Value::Missing {
            return Ok(empty_prediction());
        }
        x.push(as_f64(value, &field.field)?);
    }
    // Each categorical value as an index into the field's DataField values; an unseen category
    // gets the next index (a count of zero in every cluster).
    let mut categories = Vec::with_capacity(categorical.len());
    for field in &categorical {
        let value = env
            .get(&field.field)
            .ok_or_else(|| PmmlError::UnknownField(field.field.clone()))?;
        if *value == Value::Missing {
            return Ok(empty_prediction());
        }
        let values = pmml
            .data_dictionary
            .iter()
            .find(|f| f.name == field.field)
            .map(|f| f.values.as_slice())
            .unwrap_or_default();
        categories.push(
            values
                .iter()
                .position(|v| value_matches_str(value, v))
                .unwrap_or(values.len()),
        );
    }

    let record = Stats {
        n: 0.0,
        means: vec![0.0; x.len()],
        variances: vec![0.0; x.len()],
        counts: vec![Vec::new(); categories.len()],
    }
    .with(&x, &categories);
    let record_xi = record.log_likelihood(&totals);

    let mut best: Option<(usize, f64)> = None;
    for (idx, cluster) in model.clusters.iter().enumerate() {
        let n = cluster.size.unwrap_or(0.0);
        if cluster.center.len() != x.len() || cluster.variances.len() != x.len() {
            return Err(PmmlError::Score(format!(
                "cluster {} needs a center and a variance per continuous field",
                idx + 1
            )));
        }
        let counts = categorical
            .iter()
            .map(|field| {
                cluster
                    .category_counts
                    .iter()
                    .find(|(name, _)| *name == field.field)
                    .map(|(_, counts)| counts.clone())
                    .ok_or_else(|| {
                        PmmlError::Score(format!(
                            "cluster {} has no counts for {}",
                            idx + 1,
                            field.field
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let stats = Stats {
            n,
            means: cluster.center.clone(),
            variances: cluster.variances.clone(),
            counts,
        };
        let distance = stats.log_likelihood(&totals) + record_xi
            - stats.with(&x, &categories).log_likelihood(&totals);
        if best.is_none_or(|(_, d)| distance < d) {
            best = Some((idx, distance));
        }
    }

    let Some((idx, distance)) = best else {
        return Ok(empty_prediction());
    };
    let cluster = &model.clusters[idx];
    let name = cluster
        .name
        .clone()
        .or_else(|| cluster.id.clone())
        .unwrap_or_else(|| (idx + 1).to_string());
    Ok(Prediction {
        value: Value::Text(name.clone()),
        confidence: None,
        probabilities: Vec::new(),
        entity_id: Some(cluster.id.clone().unwrap_or(name)),
        affinity: Some(distance),
    })
}

/// The Anomaly Detection indices of one record (Algorithms Guide: "Anomaly Detection
/// Algorithm", scoring and reasoning stages; `models_next/anomaly_default`).
#[derive(Debug, Clone, PartialEq)]
pub struct AnomalyScore {
    /// The index of the nearest cluster (the peer group).
    pub cluster: usize,
    /// The group deviation index: the log-likelihood distance to the peer group.
    pub deviation: f64,
    /// The anomaly index: the deviation over the peer group's mean deviation.
    pub index: f64,
    /// Each input's variable contribution measure (its share of the deviation), in
    /// `ClusteringField` order.
    pub contributions: Vec<(String, f64)>,
}

/// Anomaly Detection scores with the TwoStep log-likelihood distance, except that a cluster's
/// saved sample variance is turned back into the maximum-likelihood one first (all 104 scored
/// records of `anomaly_default` to 1e-14; the TwoStep nugget scores with the sample variance).
/// A variable's deviation index is its own term of the distance: for a continuous field
/// -n/2 ln(D + s2) - 1/2 ln D + (n + 1)/2 ln(D + s2'), for a categorical one
/// -n E + (n + 1) E', where ' marks the cluster with the record added and D is the
/// `fieldWeight` (the field's population variance over 6, the adjustment coefficient).
/// Missing continuous values take the `MiningField` replacement (the grand mean); a missing
/// categorical value is the empty category.
pub(crate) fn score_anomaly(
    pmml: &Pmml,
    model: &ClusteringModel,
    env: &Env<'_>,
) -> Result<Option<AnomalyScore>, PmmlError> {
    enum Input {
        Continuous { x: f64, delta: f64, slot: usize },
        Categorical { category: usize },
    }
    let mut inputs = Vec::with_capacity(model.fields.len());
    let mut continuous = 0;
    for field in &model.fields {
        let value = env
            .get(&field.field)
            .ok_or_else(|| PmmlError::UnknownField(field.field.clone()))?;
        let data_field = pmml.data_dictionary.iter().find(|f| f.name == field.field);
        if data_field.is_some_and(|f| f.optype == crate::OpType::Continuous) {
            if *value == Value::Missing {
                return Ok(None);
            }
            inputs.push(Input::Continuous {
                x: as_f64(value, &field.field)?,
                delta: field.weight,
                slot: continuous,
            });
            continuous += 1;
        } else {
            let values = data_field.map(|f| f.values.as_slice()).unwrap_or_default();
            let position = if *value == Value::Missing {
                values.iter().position(String::is_empty)
            } else {
                values.iter().position(|v| value_matches_str(value, v))
            };
            let Some(category) = position else {
                return Ok(None);
            };
            inputs.push(Input::Categorical { category });
        }
    }

    let entropy = |counts: &[f64], n: f64| -> f64 {
        counts
            .iter()
            .filter(|&&c| c > 0.0)
            .map(|&c| -(c / n) * (c / n).ln())
            .sum()
    };
    let mut best: Option<(usize, f64, Vec<f64>)> = None;
    for (idx, cluster) in model.clusters.iter().enumerate() {
        let n = cluster.size.unwrap_or(0.0);
        let mut terms = Vec::with_capacity(inputs.len());
        for (input, field) in inputs.iter().zip(&model.fields) {
            match *input {
                Input::Continuous { x, delta, slot } => {
                    let (Some(&mean), Some(&sample)) =
                        (cluster.center.get(slot), cluster.variances.get(slot))
                    else {
                        return Err(PmmlError::Score(format!(
                            "cluster {} has no statistics for {}",
                            idx + 1,
                            field.field
                        )));
                    };
                    let var = if n > 1.0 { sample * (n - 1.0) / n } else { 0.0 };
                    let d = x - mean;
                    let merged = (n * var + n * d * d / (n + 1.0)) / (n + 1.0);
                    terms.push(
                        -n * 0.5 * (delta + var).ln() - 0.5 * delta.ln()
                            + (n + 1.0) * 0.5 * (delta + merged).ln(),
                    );
                }
                Input::Categorical { category } => {
                    let counts = cluster
                        .category_counts
                        .iter()
                        .find(|(name, _)| *name == field.field)
                        .map(|(_, counts)| counts.clone())
                        .ok_or_else(|| {
                            PmmlError::Score(format!(
                                "cluster {} has no counts for {}",
                                idx + 1,
                                field.field
                            ))
                        })?;
                    let mut added = counts.clone();
                    if added.len() <= category {
                        added.resize(category + 1, 0.0);
                    }
                    added[category] += 1.0;
                    terms.push(-n * entropy(&counts, n) + (n + 1.0) * entropy(&added, n + 1.0));
                }
            }
        }
        let distance: f64 = terms.iter().sum();
        if best.as_ref().is_none_or(|(_, d, _)| distance < *d) {
            best = Some((idx, distance, terms));
        }
    }
    let Some((cluster, deviation, terms)) = best else {
        return Ok(None);
    };

    // The guide's zero-denominator rules ("Odd Situations").
    let sizes: Vec<f64> = model
        .clusters
        .iter()
        .map(|c| c.size.unwrap_or(0.0))
        .collect();
    let size = sizes[cluster];
    let size_ratio = || {
        let mean_size = sizes.iter().sum::<f64>() / sizes.len() as f64;
        if size > 0.0 { mean_size / size } else { 0.0 }
    };
    let mean = model.clusters[cluster].mean_distance.unwrap_or(0.0);
    let index = if mean != 0.0 {
        deviation / mean
    } else if deviation == 0.0 {
        size_ratio()
    } else {
        let (weighted, others) = model
            .clusters
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != cluster)
            .fold((0.0, 0.0), |(w, s), (_, c)| {
                let n = c.size.unwrap_or(0.0);
                (w + n * c.mean_distance.unwrap_or(0.0), s + n)
            });
        if weighted != 0.0 && others > 0.0 {
            deviation / (weighted / others)
        } else {
            size_ratio()
        }
    };
    let share = |term: f64| {
        if deviation != 0.0 {
            term / deviation
        } else {
            1.0 / terms.len() as f64
        }
    };
    Ok(Some(AnomalyScore {
        cluster,
        deviation,
        index,
        contributions: model
            .fields
            .iter()
            .zip(&terms)
            .map(|(field, &term)| (field.field.clone(), share(term)))
            .collect(),
    }))
}
