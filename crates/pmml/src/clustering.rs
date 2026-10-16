//! Scoring of PMML 4.3 center-based `ClusteringModel` (distance/similarity metrics, compare
//! functions, field weights, and missing-value weight adjustments).

use crate::expr::as_f64;
use crate::score::{Env, empty_prediction};
use crate::{
    ClusteringModel, CompareFunction, MeasureKind, Metric, ModelClass, Pmml, PmmlError, Prediction,
    Value,
};

pub(crate) fn score(
    _pmml: &Pmml,
    model: &ClusteringModel,
    env: &Env<'_>,
) -> Result<Prediction, PmmlError> {
    if model.model_class == ModelClass::DistributionBased {
        return Err(PmmlError::Unsupported(
            "distributionBased ClusteringModel".into(),
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
