//! Scoring of PMML 4.3 `TreeModel` (predicates, missing-value strategies, `noTrueChildStrategy`,
//! `ScoreDistribution`).

use crate::expr::{as_f64, value_matches_str};
use crate::score::{Env, empty_prediction, target_categories};
use crate::{
    BooleanOperator, MiningFunction, MissingValueStrategy, NoTrueChildStrategy, Node, Operator,
    Pmml, PmmlError, Predicate, Prediction, TreeModel, Value,
};

const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Truth {
    True,
    False,
    Unknown,
}

pub(crate) fn score(pmml: &Pmml, tree: &TreeModel, env: &Env<'_>) -> Result<Prediction, PmmlError> {
    let root_truth = eval_predicate(&tree.root.predicate, env, 0)?;
    if root_truth != Truth::True {
        return Ok(empty_prediction());
    }
    eval_node(pmml, tree, &tree.root, env, 0)
}

fn eval_node(
    pmml: &Pmml,
    tree: &TreeModel,
    node: &Node,
    env: &Env<'_>,
    depth: usize,
) -> Result<Prediction, PmmlError> {
    if depth > MAX_DEPTH {
        return Err(PmmlError::Score(
            "TreeModel traversal depth exceeded".into(),
        ));
    }
    if node.children.is_empty() {
        return prediction_from_node(pmml, tree, node);
    }

    let mut unknown_children: Vec<&Node> = Vec::new();
    for child in &node.children {
        match eval_predicate(&child.predicate, env, 0)? {
            Truth::True => {
                let child_pred = eval_node(pmml, tree, child, env, depth + 1)?;
                if child_pred.value == Value::Missing
                    && tree.no_true_child_strategy == NoTrueChildStrategy::ReturnLastPrediction
                    && child.score.is_none()
                    && child.distribution.is_empty()
                {
                    return prediction_from_node(pmml, tree, node);
                }
                return Ok(child_pred);
            }
            Truth::False => {}
            Truth::Unknown => unknown_children.push(child),
        }
    }

    if !unknown_children.is_empty() {
        match tree.missing_value_strategy {
            MissingValueStrategy::LastPrediction => {
                // Tree-AS (`models_as/treeas_*`) keeps this node's score AND id;
                // `treeas_numeric` writes the id even for a continuous target.
                return prediction_from_node(pmml, tree, node);
            }
            MissingValueStrategy::NullPrediction => {
                return Ok(empty_prediction());
            }
            MissingValueStrategy::DefaultChild => {
                if let Some(def_id) = &node.default_child
                    && let Some(def_child) =
                        node.children.iter().find(|c| c.id.as_ref() == Some(def_id))
                {
                    return eval_node(pmml, tree, def_child, env, depth + 1);
                }
                return Ok(empty_prediction());
            }
            MissingValueStrategy::WeightedConfidence => {
                return score_weighted_confidence(pmml, tree, &unknown_children, env, depth + 1);
            }
            MissingValueStrategy::AggregateNodes => {
                return score_aggregate_nodes(pmml, tree, &unknown_children, env, depth + 1);
            }
            MissingValueStrategy::None => {}
        }
    }

    match tree.no_true_child_strategy {
        NoTrueChildStrategy::ReturnNullPrediction => Ok(empty_prediction()),
        NoTrueChildStrategy::ReturnLastPrediction => prediction_from_node(pmml, tree, node),
    }
}

fn score_weighted_confidence(
    pmml: &Pmml,
    tree: &TreeModel,
    children: &[&Node],
    env: &Env<'_>,
    depth: usize,
) -> Result<Prediction, PmmlError> {
    let total_records: f64 = children.iter().filter_map(|c| c.record_count).sum();
    if total_records <= 0.0 {
        return Ok(empty_prediction());
    }

    let mut categories: Vec<String> = target_categories(pmml, &tree.header)
        .into_iter()
        .map(str::to_string)
        .collect();
    let mut child_preds = Vec::with_capacity(children.len());

    for &child in children {
        let weight = child.record_count.unwrap_or(0.0) / total_records;
        let pred = eval_node(pmml, tree, child, env, depth)?;
        for (cat, _) in &pred.probabilities {
            if !categories.contains(cat) {
                categories.push(cat.clone());
            }
        }
        child_preds.push((weight, pred));
    }

    if categories.is_empty() {
        return Ok(empty_prediction());
    }

    let mut probabilities = Vec::with_capacity(categories.len());
    for cat in &categories {
        let mut weighted_prob = 0.0;
        for (weight, pred) in &child_preds {
            if let Some((_, p)) = pred.probabilities.iter().find(|(c, _)| c == cat) {
                weighted_prob += weight * p;
            }
        }
        probabilities.push((cat.clone(), weighted_prob));
    }

    let mut best_cat: Option<(String, f64)> = None;
    for (cat, prob) in &probabilities {
        if best_cat.as_ref().is_none_or(|(_, bp)| *prob > *bp) {
            best_cat = Some((cat.clone(), *prob));
        }
    }

    let Some((winner, confidence)) = best_cat else {
        return Ok(empty_prediction());
    };

    Ok(Prediction {
        value: convert_score_value(&winner, &tree.header.function)?,
        confidence: Some(confidence),
        probabilities,
        entity_id: None,
        affinity: None,
    })
}

fn score_aggregate_nodes(
    pmml: &Pmml,
    tree: &TreeModel,
    children: &[&Node],
    env: &Env<'_>,
    depth: usize,
) -> Result<Prediction, PmmlError> {
    let mut leaves = Vec::new();
    for &child in children {
        collect_aggregate_leaves(child, env, depth, &mut leaves)?;
    }
    if leaves.is_empty() {
        return Ok(empty_prediction());
    }

    let mut categories: Vec<String> = target_categories(pmml, &tree.header)
        .into_iter()
        .map(str::to_string)
        .collect();
    for leaf in &leaves {
        for dist in &leaf.distribution {
            if !categories.contains(&dist.value) {
                categories.push(dist.value.clone());
            }
        }
    }

    let mut counts: Vec<(String, f64)> = categories.into_iter().map(|c| (c, 0.0)).collect();
    for leaf in &leaves {
        for dist in &leaf.distribution {
            if let Some((_, c)) = counts.iter_mut().find(|(cat, _)| *cat == dist.value) {
                *c += dist.record_count;
            }
        }
    }

    let total: f64 = counts.iter().map(|(_, c)| *c).sum();
    if total <= 0.0 {
        return Ok(empty_prediction());
    }

    let probabilities: Vec<(String, f64)> = counts
        .into_iter()
        .map(|(cat, count)| (cat, count / total))
        .collect();

    let mut best_cat: Option<(String, f64)> = None;
    for (cat, prob) in &probabilities {
        if best_cat.as_ref().is_none_or(|(_, bp)| *prob > *bp) {
            best_cat = Some((cat.clone(), *prob));
        }
    }

    let Some((winner, confidence)) = best_cat else {
        return Ok(empty_prediction());
    };

    Ok(Prediction {
        value: convert_score_value(&winner, &tree.header.function)?,
        confidence: Some(confidence),
        probabilities,
        entity_id: None,
        affinity: None,
    })
}

fn collect_aggregate_leaves<'a>(
    node: &'a Node,
    env: &Env<'_>,
    depth: usize,
    leaves: &mut Vec<&'a Node>,
) -> Result<(), PmmlError> {
    if depth > MAX_DEPTH {
        return Err(PmmlError::Score(
            "TreeModel traversal depth exceeded".into(),
        ));
    }
    if node.children.is_empty() {
        leaves.push(node);
        return Ok(());
    }
    let mut unknowns = Vec::new();
    for child in &node.children {
        match eval_predicate(&child.predicate, env, 0)? {
            Truth::True => return collect_aggregate_leaves(child, env, depth + 1, leaves),
            Truth::False => {}
            Truth::Unknown => unknowns.push(child),
        }
    }
    if unknowns.is_empty() {
        leaves.push(node);
    } else {
        for u in unknowns {
            collect_aggregate_leaves(u, env, depth + 1, leaves)?;
        }
    }
    Ok(())
}

fn uses_laplace_smoothing(tree: &TreeModel) -> bool {
    matches!(
        tree.header.algorithm.as_deref(),
        Some("C&RT" | "C&R Tree" | "CHAID" | "QUEST")
    )
}

fn prediction_from_node(
    pmml: &Pmml,
    tree: &TreeModel,
    node: &Node,
) -> Result<Prediction, PmmlError> {
    let total_records: f64 = {
        let sum: f64 = node.distribution.iter().map(|d| d.record_count).sum();
        if sum > 0.0 {
            sum
        } else {
            node.record_count.unwrap_or(0.0)
        }
    };

    let target_cats = target_categories(pmml, &tree.header);
    let num_cats = if !target_cats.is_empty() {
        target_cats.len().max(node.distribution.len())
    } else {
        node.distribution.len()
    };
    let laplace = uses_laplace_smoothing(tree) && total_records > 0.0 && num_cats > 0;

    let prob_of = |d: &crate::ScoreDistribution| -> f64 {
        if laplace {
            (d.record_count + 1.0) / (total_records + num_cats as f64)
        } else if let Some(p) = d.probability {
            p
        } else if total_records > 0.0 {
            d.record_count / total_records
        } else {
            0.0
        }
    };

    let conf_of = |d: &crate::ScoreDistribution| -> f64 {
        if laplace {
            (d.record_count + 1.0) / (total_records + num_cats as f64)
        } else {
            d.confidence.unwrap_or_else(|| prob_of(d))
        }
    };

    let mut probabilities = Vec::new();
    if !node.distribution.is_empty() {
        if !target_cats.is_empty() {
            for &cat in &target_cats {
                let p = node
                    .distribution
                    .iter()
                    .find(|d| value_matches_str(&Value::Text(d.value.clone()), cat))
                    .map_or(
                        if laplace {
                            1.0 / (total_records + num_cats as f64)
                        } else {
                            0.0
                        },
                        prob_of,
                    );
                probabilities.push((cat.to_string(), p));
            }
            for d in &node.distribution {
                if !probabilities
                    .iter()
                    .any(|(c, _)| value_matches_str(&Value::Text(d.value.clone()), c))
                {
                    probabilities.push((d.value.clone(), prob_of(d)));
                }
            }
        } else {
            for d in &node.distribution {
                probabilities.push((d.value.clone(), prob_of(d)));
            }
        }
    }

    let winner: Option<String> = if let Some(s) = &node.score {
        Some(s.clone())
    } else {
        let mut best: Option<(&str, f64)> = None;
        for d in &node.distribution {
            let p = prob_of(d);
            if best.is_none_or(|(_, bp)| p > bp) {
                best = Some((d.value.as_str(), p));
            }
        }
        best.map(|(v, _)| v.to_string())
    };

    let Some(winner_str) = winner else {
        return Ok(empty_prediction());
    };

    let confidence = node
        .distribution
        .iter()
        .find(|d| value_matches_str(&Value::Text(d.value.clone()), &winner_str))
        .map(conf_of);

    let value = convert_score_value(&winner_str, &tree.header.function)?;

    Ok(Prediction {
        value,
        confidence,
        probabilities,
        entity_id: node.id.clone(),
        affinity: None,
    })
}

fn convert_score_value(score: &str, function: &MiningFunction) -> Result<Value, PmmlError> {
    if *function == MiningFunction::Regression {
        let n = score
            .trim()
            .parse::<f64>()
            .map_err(|_| PmmlError::Score(format!("invalid numeric regression score {score:?}")))?;
        Ok(Value::Number(n))
    } else {
        Ok(Value::Text(score.to_string()))
    }
}

pub(crate) fn eval_predicate(
    pred: &Predicate,
    env: &Env<'_>,
    depth: usize,
) -> Result<Truth, PmmlError> {
    if depth > MAX_DEPTH {
        return Err(PmmlError::Score(
            "predicate evaluation depth exceeded".into(),
        ));
    }
    match pred {
        Predicate::True => Ok(Truth::True),
        Predicate::False => Ok(Truth::False),
        Predicate::Simple {
            field,
            operator,
            value,
        } => {
            let val = env
                .get(field)
                .ok_or_else(|| PmmlError::UnknownField(field.clone()))?;
            match operator {
                Operator::IsMissing => Ok(if *val == Value::Missing {
                    Truth::True
                } else {
                    Truth::False
                }),
                Operator::IsNotMissing => Ok(if *val != Value::Missing {
                    Truth::True
                } else {
                    Truth::False
                }),
                Operator::Equal | Operator::NotEqual => {
                    if *val == Value::Missing {
                        return Ok(Truth::Unknown);
                    }
                    let rhs = value.as_deref().unwrap_or("");
                    let eq = value_matches_str(val, rhs);
                    let res = if *operator == Operator::Equal {
                        eq
                    } else {
                        !eq
                    };
                    Ok(if res { Truth::True } else { Truth::False })
                }
                Operator::LessThan
                | Operator::LessOrEqual
                | Operator::GreaterThan
                | Operator::GreaterOrEqual => {
                    if *val == Value::Missing {
                        return Ok(Truth::Unknown);
                    }
                    let rhs_str = value.as_deref().unwrap_or("");
                    let lhs_num = as_f64(val, field)?;
                    let rhs_num = rhs_str.trim().parse::<f64>().map_err(|_| {
                        PmmlError::Score(format!(
                            "non-numeric threshold {rhs_str:?} for field {field:?}"
                        ))
                    })?;
                    let res = match operator {
                        Operator::LessThan => lhs_num < rhs_num,
                        Operator::LessOrEqual => lhs_num <= rhs_num,
                        Operator::GreaterThan => lhs_num > rhs_num,
                        _ => lhs_num >= rhs_num,
                    };
                    Ok(if res { Truth::True } else { Truth::False })
                }
            }
        }
        Predicate::SimpleSet {
            field,
            is_in,
            values,
        } => {
            let val = env
                .get(field)
                .ok_or_else(|| PmmlError::UnknownField(field.clone()))?;
            if *val == Value::Missing {
                return Ok(Truth::Unknown);
            }
            let found = values.iter().any(|item| value_matches_str(val, item));
            let res = if *is_in { found } else { !found };
            Ok(if res { Truth::True } else { Truth::False })
        }
        Predicate::Compound {
            operator,
            predicates,
        } => match operator {
            BooleanOperator::And => {
                let mut saw_unknown = false;
                for p in predicates {
                    match eval_predicate(p, env, depth + 1)? {
                        Truth::False => return Ok(Truth::False),
                        Truth::True => {}
                        Truth::Unknown => saw_unknown = true,
                    }
                }
                Ok(if saw_unknown {
                    Truth::Unknown
                } else {
                    Truth::True
                })
            }
            BooleanOperator::Or => {
                let mut saw_unknown = false;
                for p in predicates {
                    match eval_predicate(p, env, depth + 1)? {
                        Truth::True => return Ok(Truth::True),
                        Truth::False => {}
                        Truth::Unknown => saw_unknown = true,
                    }
                }
                Ok(if saw_unknown {
                    Truth::Unknown
                } else {
                    Truth::False
                })
            }
            BooleanOperator::Xor => {
                let mut true_count = 0usize;
                for p in predicates {
                    match eval_predicate(p, env, depth + 1)? {
                        Truth::Unknown => return Ok(Truth::Unknown),
                        Truth::True => true_count += 1,
                        Truth::False => {}
                    }
                }
                Ok(if true_count % 2 == 1 {
                    Truth::True
                } else {
                    Truth::False
                })
            }
            BooleanOperator::Surrogate => {
                for p in predicates {
                    let t = eval_predicate(p, env, depth + 1)?;
                    if t != Truth::Unknown {
                        return Ok(t);
                    }
                }
                Ok(Truth::Unknown)
            }
        },
    }
}
