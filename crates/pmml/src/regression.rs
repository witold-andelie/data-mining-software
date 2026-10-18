//! Scoring of PMML 4.3 `GeneralRegressionModel` (`regression`, `generalLinear`,
//! `multinomialLogistic`, `generalizedLinear`) and `RegressionModel` (`NumericPredictor`,
//! `CategoricalPredictor`, `none`/`softmax`/`logit` normalization).

use crate::expr::{as_f64, value_matches_str};
use crate::score::{Env, empty_prediction, target_categories};
use crate::{
    GeneralModelType, GeneralRegressionModel, MiningFunction, OpType, Pmml, PmmlError, Prediction,
    RegressionModel, RegressionTable, Value,
};
use std::collections::HashMap;

pub(crate) fn score_general(
    pmml: &Pmml,
    model: &GeneralRegressionModel,
    env: &Env<'_>,
) -> Result<Prediction, PmmlError> {
    // Evaluate each parameter's predictor product `x_p` from PPMatrix.
    // Parameters with no PPCells in PPMatrix are intercepts (`x_p = 1.0`).
    let mut param_values: HashMap<&str, f64> = HashMap::with_capacity(model.parameters.len());
    for param in &model.parameters {
        param_values.insert(param.as_str(), 1.0);
    }
    for cell in &model.param_matrix {
        param_values.entry(cell.parameter.as_str()).or_insert(1.0);
    }

    for pp in &model.pp_matrix {
        let val = env
            .get(&pp.predictor)
            .ok_or_else(|| PmmlError::UnknownField(pp.predictor.clone()))?;
        if *val == Value::Missing {
            return Ok(empty_prediction());
        }

        let is_factor = if model.factors.iter().any(|f| f == &pp.predictor) {
            true
        } else if model.covariates.iter().any(|c| c == &pp.predictor) {
            false
        } else {
            pmml.data_dictionary
                .iter()
                .find(|df| df.name == pp.predictor)
                .is_some_and(|df| df.optype != OpType::Continuous)
        };

        let term = if is_factor {
            if value_matches_str(val, &pp.value) {
                1.0
            } else {
                0.0
            }
        } else {
            let x = as_f64(val, &pp.predictor)?;
            let exp = pp.value.trim().parse::<f64>().map_err(|_| {
                PmmlError::Score(format!(
                    "invalid covariate exponent {:?} for predictor {:?}",
                    pp.value, pp.predictor
                ))
            })?;
            if (exp - 1.0).abs() == 0.0 {
                x
            } else {
                x.powf(exp)
            }
        };

        let entry = param_values.entry(pp.parameter.as_str()).or_insert(1.0);
        *entry *= term;
    }

    match &model.model_type {
        GeneralModelType::Regression | GeneralModelType::GeneralLinear => {
            let mut y = 0.0;
            for pcell in &model.param_matrix {
                let x_p = param_values
                    .get(pcell.parameter.as_str())
                    .copied()
                    .unwrap_or(1.0);
                y += pcell.beta * x_p;
            }
            Ok(Prediction {
                value: Value::Number(y),
                confidence: None,
                probabilities: Vec::new(),
                entity_id: None,
                affinity: None,
            })
        }
        GeneralModelType::GeneralizedLinear => {
            let mut eta = 0.0;
            for pcell in &model.param_matrix {
                let x_p = param_values
                    .get(pcell.parameter.as_str())
                    .copied()
                    .unwrap_or(1.0);
                eta += pcell.beta * x_p;
            }
            let y = apply_link(
                eta,
                model.link_function.as_deref().unwrap_or("identity"),
                model.link_parameter,
            )?;
            Ok(Prediction {
                value: Value::Number(y),
                confidence: None,
                probabilities: Vec::new(),
                entity_id: None,
                affinity: None,
            })
        }
        GeneralModelType::MultinomialLogistic => {
            let mut categories: Vec<String> = target_categories(pmml, &model.header)
                .into_iter()
                .map(str::to_string)
                .collect();

            for pcell in &model.param_matrix {
                if let Some(cat) = &pcell.target_category
                    && !categories
                        .iter()
                        .any(|c| value_matches_str(&Value::Text(c.clone()), cat))
                {
                    categories.push(cat.clone());
                }
            }
            if let Some(ref_cat) = &model.target_reference_category
                && !categories
                    .iter()
                    .any(|c| value_matches_str(&Value::Text(c.clone()), ref_cat))
            {
                categories.push(ref_cat.clone());
            }

            if categories.is_empty() {
                return Err(PmmlError::Score(
                    "multinomialLogistic model has no target categories".into(),
                ));
            }

            // Compute linear predictor `r_k` for each category; reference category has `r_ref = 0.0`.
            let mut logits: Vec<(String, f64)> = Vec::with_capacity(categories.len());
            for cat in categories {
                let mut r = 0.0;
                for pcell in &model.param_matrix {
                    if let Some(pcat) = &pcell.target_category
                        && value_matches_str(&Value::Text(cat.clone()), pcat)
                    {
                        let x_p = param_values
                            .get(pcell.parameter.as_str())
                            .copied()
                            .unwrap_or(1.0);
                        r += pcell.beta * x_p;
                    }
                }
                logits.push((cat, r));
            }

            // Analytic Engine `linearSVM` exports the binary decision value as
            // the non-reference category's logit (`models_as/lsvm_default`,
            // `lsvm_l1`); the reference logit is zero. This same softmax gives
            // `$LC-respond` = logistic(abs(decision)), without a second transform.
            let probabilities = softmax(&logits);
            classification_prediction(probabilities)
        }
        GeneralModelType::OrdinalMultinomial => Err(PmmlError::Unsupported(
            "ordinalMultinomial GeneralRegressionModel".into(),
        )),
        GeneralModelType::Other(other) => Err(PmmlError::Unsupported(format!(
            "GeneralRegressionModel modelType {other:?}"
        ))),
    }
}

pub(crate) fn score_regression(
    pmml: &Pmml,
    model: &RegressionModel,
    env: &Env<'_>,
) -> Result<Prediction, PmmlError> {
    if model.header.function == MiningFunction::Regression {
        let Some(table) = model.tables.first() else {
            return Ok(empty_prediction());
        };
        let Some(raw) = eval_regression_table(table, env)? else {
            return Ok(empty_prediction());
        };
        let y = match model.normalization.as_str() {
            "" | "none" => raw,
            "logit" => 1.0 / (1.0 + (-raw).exp()),
            "exp" | "softmax" => raw.exp(),
            "cloglog" => 1.0 - (-raw.exp()).exp(),
            "loglog" => (-(-raw).exp()).exp(),
            other => {
                return Err(PmmlError::Unsupported(format!(
                    "RegressionModel normalizationMethod {other:?}"
                )));
            }
        };
        return Ok(Prediction {
            value: Value::Number(y),
            confidence: None,
            probabilities: Vec::new(),
            entity_id: None,
            affinity: None,
        });
    }

    // Classification RegressionModel.
    let mut table_vals: Vec<(String, f64, bool)> = Vec::with_capacity(model.tables.len());
    for table in &model.tables {
        let Some(y) = eval_regression_table(table, env)? else {
            return Ok(empty_prediction());
        };
        let cat = table.target_category.clone().unwrap_or_default();
        let is_empty_ref =
            table.numeric.is_empty() && table.categorical.is_empty() && table.intercept == 0.0;
        table_vals.push((cat, y, is_empty_ref));
    }

    // Include any reference category from DataField.values not listed in RegressionTable.
    for cat in target_categories(pmml, &model.header) {
        if !table_vals
            .iter()
            .any(|(c, _, _)| value_matches_str(&Value::Text(c.clone()), cat))
        {
            table_vals.push((cat.to_string(), 0.0, true));
        }
    }

    let raw_probs: Vec<(String, f64)> = match model.normalization.as_str() {
        "" | "none" => {
            if table_vals.len() == 2
                && let (Some(first), Some(second)) = (table_vals.first(), table_vals.get(1))
                && second.2
            {
                vec![
                    (first.0.clone(), first.1),
                    (second.0.clone(), 1.0 - first.1),
                ]
            } else {
                table_vals.into_iter().map(|(c, y, _)| (c, y)).collect()
            }
        }
        "softmax" => {
            let logits: Vec<(String, f64)> =
                table_vals.into_iter().map(|(c, y, _)| (c, y)).collect();
            softmax(&logits)
        }
        "logit" => {
            if table_vals.len() == 2
                && let (Some(first), Some(second)) = (table_vals.first(), table_vals.get(1))
                && second.2
            {
                let p0 = 1.0 / (1.0 + (-first.1).exp());
                vec![(first.0.clone(), p0), (second.0.clone(), 1.0 - p0)]
            } else {
                let mut probs: Vec<(String, f64)> = table_vals
                    .into_iter()
                    .map(|(c, y, _)| (c, 1.0 / (1.0 + (-y).exp())))
                    .collect();
                let sum: f64 = probs.iter().map(|(_, p)| *p).sum();
                if sum > 0.0 {
                    for (_, p) in &mut probs {
                        *p /= sum;
                    }
                }
                probs
            }
        }
        "simplemax" => {
            let sum: f64 = table_vals.iter().map(|(_, y, _)| *y).sum();
            table_vals
                .into_iter()
                .map(|(c, y, _)| (c, if sum > 0.0 { y / sum } else { 0.0 }))
                .collect()
        }
        other => {
            return Err(PmmlError::Unsupported(format!(
                "RegressionModel normalizationMethod {other:?}"
            )));
        }
    };

    // Order probabilities by target DataField values when present.
    let target_cats = target_categories(pmml, &model.header);
    let probabilities = if !target_cats.is_empty() {
        let mut ordered = Vec::with_capacity(raw_probs.len());
        for &tcat in &target_cats {
            if let Some((_, p)) = raw_probs
                .iter()
                .find(|(c, _)| value_matches_str(&Value::Text(c.clone()), tcat))
            {
                ordered.push((tcat.to_string(), *p));
            }
        }
        for (c, p) in raw_probs {
            if !ordered
                .iter()
                .any(|(oc, _)| value_matches_str(&Value::Text(oc.clone()), &c))
            {
                ordered.push((c, p));
            }
        }
        ordered
    } else {
        raw_probs
    };

    classification_prediction(probabilities)
}

fn eval_regression_table(table: &RegressionTable, env: &Env<'_>) -> Result<Option<f64>, PmmlError> {
    let mut y = table.intercept;
    for (field, exponent, coeff) in &table.numeric {
        let val = env
            .get(field)
            .ok_or_else(|| PmmlError::UnknownField(field.clone()))?;
        if *val == Value::Missing {
            return Ok(None);
        }
        let x = as_f64(val, field)?;
        let term = if *exponent == 1 { x } else { x.powi(*exponent) };
        y += coeff * term;
    }
    for (field, target_val, coeff) in &table.categorical {
        let val = env
            .get(field)
            .ok_or_else(|| PmmlError::UnknownField(field.clone()))?;
        if *val == Value::Missing {
            return Ok(None);
        }
        if value_matches_str(val, target_val) {
            y += coeff;
        }
    }
    Ok(Some(y))
}

fn softmax(logits: &[(String, f64)]) -> Vec<(String, f64)> {
    let max_r = logits
        .iter()
        .map(|(_, r)| *r)
        .fold(f64::NEG_INFINITY, f64::max);
    let weights: Vec<f64> = logits.iter().map(|(_, r)| (r - max_r).exp()).collect();
    let sum_w: f64 = weights.iter().sum();
    logits
        .iter()
        .zip(weights)
        .map(|((cat, _), w)| (cat.clone(), if sum_w > 0.0 { w / sum_w } else { 0.0 }))
        .collect()
}

fn classification_prediction(probabilities: Vec<(String, f64)>) -> Result<Prediction, PmmlError> {
    let mut best: Option<(String, f64)> = None;
    for (cat, prob) in &probabilities {
        if best.as_ref().is_none_or(|(_, bp)| *prob > *bp) {
            best = Some((cat.clone(), *prob));
        }
    }
    let Some((winner, confidence)) = best else {
        return Ok(empty_prediction());
    };
    Ok(Prediction {
        value: Value::Text(winner),
        confidence: Some(confidence),
        probabilities,
        entity_id: None,
        affinity: None,
    })
}

fn apply_link(eta: f64, link: &str, param: Option<f64>) -> Result<f64, PmmlError> {
    match link {
        "identity" => Ok(eta),
        "log" => Ok(eta.exp()),
        "logit" => Ok(1.0 / (1.0 + (-eta).exp())),
        "inverse" => {
            if eta == 0.0 {
                Err(PmmlError::Score(
                    "zero linear predictor for inverse link".into(),
                ))
            } else {
                Ok(1.0 / eta)
            }
        }
        "power" => {
            let p = param.unwrap_or(1.0);
            if p == 0.0 {
                Ok(eta.exp())
            } else {
                Ok(eta.powf(1.0 / p))
            }
        }
        "cloglog" => Ok(1.0 - (-eta.exp()).exp()),
        "loglog" => Ok((-(-eta).exp()).exp()),
        other => Err(PmmlError::Unsupported(format!(
            "GeneralRegressionModel linkFunction {other:?}"
        ))),
    }
}
