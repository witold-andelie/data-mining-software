//! Sequence nugget (`ApplySequenceNode`, `oracle/models_next/sequence_*`): predictions from the
//! saved or freshly trained sequential rules, record by record along each id's history.
//!
//! The nugget keeps its rules in an attachment (`model/rules@containerData`): `rule` elements
//! whose antecedent and consequent are `SequenceCondition`s, a `then` list of `and` itemsets of
//! `field = value` conditions, with `confidence`. Rules are in the model's order.
//!
//! Scoring rules (`sequence_default`, `sequence_minsupp`; training and grid scores):
//! - Records of one id (`model/idField`) are contiguous; the history is the itemsets of its
//!   records so far, one per distinct time (`model/timeField`).
//! - A rule applies when its antecedent itemsets occur in the history in order, each in a
//!   later itemset than the previous one. Its consequent is predicted with the rule's
//!   confidence, also when the item is already in the history (`sequence_default` row 2 of
//!   customer 1: milk after bread, milk).
//! - Predictions are ordered by confidence, then rule order, one per item, at most
//!   `SequenceMaxPredictions`: `$S-<model>-<k>` and `$SC-<model>-<k>` (row 1: milk 0.6,
//!   eggs 0.6, jam 0.4, where butter 0.4 loses to jam by rule order).
//!
//! GAP: gaps (`SequenceUseGaps`), sequences without a time field, multi-item consequents and
//! non-contiguous ids have no oracle and are rejected.
//!
//! Scoring is statistical parity (tie order free): with the build node's rules every oracle
//! table matches (`crates/exec/tests/sequence.rs`). `produce` scores the model the linked build
//! node trained; `run` scores the rules saved in the attachment, so a stream without its builder
//! still applies its saved model.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::NodeOutput;
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Field, Measurement, Storage, Table, Value};
use std::sync::Arc;

use super::field_util::{Column, with_columns};

use dms_pmml::{SequenceItemset as Itemset, SequenceRule};

/// A rule as scoring uses it: a single-item consequent, read from a [`SequenceRule`].
struct Rule {
    antecedent: Vec<Itemset>,
    consequent: String,
    confidence: f64,
}

pub fn apply(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let unsupported =
        |what: &str| OperatorError::Settings(format!("Sequence {what} is not supported yet (GAP)"));
    if settings::flag(s, "SequenceUseGaps")?.unwrap_or(false) {
        return Err(unsupported("SequenceUseGaps"));
    }
    if !settings::flag(s, "SequenceUseTime")?.unwrap_or(true) {
        return Err(unsupported("scoring without a time field"));
    }
    let name = |path: &str| {
        settings::text(s, path)
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
    };
    let id_field = name("model/idField/name")
        .ok_or_else(|| OperatorError::Settings("the Sequence nugget names no id field".into()))?;
    let time_field = name("model/timeField/name")
        .ok_or_else(|| OperatorError::Settings("the Sequence nugget names no time field".into()))?;
    let model_name = name("model/name").unwrap_or_else(|| "Sequence".to_string());
    let rules_member = settings::element(s, "model/rules")
        .and_then(|rules| rules.attribute("containerData"))
        .map(str::to_string)
        .ok_or_else(|| OperatorError::Settings("the Sequence nugget saved no rules".into()))?;
    let max_predictions = match settings::integer(s, "SequenceMaxPredictions")? {
        Some(n) => n,
        None => settings::children(s, "buildSettings", "propertySetting")
            .into_iter()
            .find(|p| settings::text(Some(p), "name").as_deref() == Some("SequenceMaxPredictions"))
            .and_then(|p| settings::text(Some(p), "value"))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(3),
    };
    let max_predictions = usize::try_from(max_predictions)
        .map_err(|_| OperatorError::Settings("SequenceMaxPredictions is negative".into()))?;
    Ok(Box::new(Sequence {
        id_field,
        time_field,
        model_name,
        rules_member,
        max_predictions,
    }))
}

struct Sequence {
    id_field: String,
    time_field: String,
    model_name: String,
    rules_member: String,
    max_predictions: usize,
}

/// Whether `pattern`'s itemsets occur in `history` in order, each in a later itemset.
fn occurs(pattern: &[Itemset], history: &[Vec<(String, String)>]) -> bool {
    let mut next = 0;
    for set in pattern {
        match history[next..]
            .iter()
            .position(|transaction| set.iter().all(|item| transaction.contains(item)))
        {
            Some(offset) => next += offset + 1,
            None => return false,
        }
    }
    true
}

impl Operator for Sequence {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        self.predict(inputs, None, context)
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        model: Option<Arc<dms_pmml::Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        self.predict(inputs, model.as_deref(), context)
            .map(|table| NodeOutput::Data(Arc::new(table)))
    }
}

impl Sequence {
    /// Scores with the build node's freshly trained rules when the plan passes them (Modeler
    /// replaces the nugget's saved model on a build), else with the rules the stream saved.
    fn predict(
        &self,
        inputs: &[Arc<Table>],
        trained: Option<&dms_pmml::Pmml>,
        context: &ExecContext,
    ) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("a model nugget needs one input".into()))?;
        let rules = match trained {
            Some(pmml) => match &pmml.model {
                dms_pmml::Model::Sequence(model) => scoring_rules(&model.rules)?,
                _ => {
                    return Err(OperatorError::Data(
                        "the Sequence nugget received a model that is not a SequenceModel".into(),
                    ));
                }
            },
            None => {
                let bytes = context.attachments.get(&self.rules_member).ok_or_else(|| {
                    OperatorError::Io(format!(
                        "the stream file has no attachment {}",
                        self.rules_member
                    ))
                })?;
                let text = std::str::from_utf8(bytes)
                    .map_err(|_| OperatorError::Data("Sequence rules are not UTF-8".into()))?;
                let rules = dms_pmml::parse_sequence_rules(text)
                    .map_err(|error| OperatorError::Data(error.to_string()))?;
                scoring_rules(&rules)?
            }
        };
        let model = input.model();
        let position = |name: &str| {
            model
                .position(name)
                .ok_or_else(|| OperatorError::Data(format!("Sequence needs field {name:?}")))
        };
        let id_at = position(&self.id_field)?;
        let time_at = position(&self.time_field)?;
        let mut content: Vec<(String, usize)> = Vec::new();
        for rule in &rules {
            for (field, _) in rule.antecedent.iter().flatten() {
                if !content.iter().any(|(name, _)| name == field) {
                    content.push((field.clone(), position(field)?));
                }
            }
        }
        let mut seen_ids: Vec<Value> = Vec::new();
        let mut current: Option<Value> = None;
        let mut history: Vec<Vec<(String, String)>> = Vec::new();
        let mut last_time: Option<Value> = None;
        let mut predicted = Vec::with_capacity(input.rows().len());
        for row in input.rows() {
            let id = row.get(id_at).cloned().unwrap_or(Value::Null);
            if current.as_ref() != Some(&id) {
                if seen_ids.contains(&id) {
                    return Err(OperatorError::Data(
                        "Sequence ids are not contiguous (GAP: no oracle)".into(),
                    ));
                }
                seen_ids.push(id.clone());
                current = Some(id);
                history.clear();
                last_time = None;
            }
            let time = row.get(time_at).cloned().unwrap_or(Value::Null);
            if last_time.as_ref() != Some(&time) || history.is_empty() {
                history.push(Vec::new());
                last_time = Some(time);
            }
            if let Some(transaction) = history.last_mut() {
                for (field, at) in &content {
                    if let Some(value) = row.get(*at).filter(|v| !v.is_null()) {
                        transaction.push((field.clone(), value.to_string()));
                    }
                }
            }
            let mut hits: Vec<(&str, f64)> = Vec::new();
            for rule in &rules {
                if occurs(&rule.antecedent, &history)
                    && !hits.iter().any(|(item, _)| *item == rule.consequent)
                {
                    hits.push((rule.consequent.as_str(), rule.confidence));
                }
            }
            // Stable: equal confidences keep rule order.
            hits.sort_by(|a, b| b.1.total_cmp(&a.1));
            hits.truncate(self.max_predictions);
            predicted.push(
                hits.into_iter()
                    .map(|(item, confidence)| (item.to_string(), confidence))
                    .collect::<Vec<_>>(),
            );
        }
        let mut columns = Vec::new();
        for k in 0..self.max_predictions {
            let mut item = Field::new(format!("$S-{}-{}", self.model_name, k + 1), Storage::String);
            item.measurement = Measurement::Nominal;
            columns.push(Column {
                field: item,
                values: predicted
                    .iter()
                    .map(|p| {
                        p.get(k)
                            .map_or(Value::Null, |(i, _)| Value::string(i.as_str()))
                    })
                    .collect(),
            });
            columns.push(Column {
                field: Field::new(format!("$SC-{}-{}", self.model_name, k + 1), Storage::Real),
                values: predicted
                    .iter()
                    .map(|p| p.get(k).map_or(Value::Null, |(_, c)| Value::Real(*c)))
                    .collect(),
            });
        }
        with_columns(input, columns)
    }
}

/// The scoring view of rules from either channel (the build node's model or the saved rules
/// container). Consequents of several itemsets or items have no oracle.
fn scoring_rules(rules: &[SequenceRule]) -> Result<Vec<Rule>, OperatorError> {
    rules
        .iter()
        .map(|rule| {
            let [set] = rule.consequent.as_slice() else {
                return Err(OperatorError::Settings(
                    "Sequence consequents of several itemsets are not supported yet (GAP)".into(),
                ));
            };
            let [(_, value)] = set.as_slice() else {
                return Err(OperatorError::Settings(
                    "Sequence consequents of several items are not supported yet (GAP)".into(),
                ));
            };
            Ok(Rule {
                antecedent: rule.antecedent.clone(),
                consequent: value.clone(),
                confidence: rule.confidence,
            })
        })
        .collect()
}
