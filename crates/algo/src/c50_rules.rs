//! C5.0 rule sets (`modelFormat` `ruleSet`). Sources: IBM SPSS Modeler 19 Algorithms
//! Guide p.51 (scoring, default-rule confidence) and p.12 (tree-derived default class);
//! Scripting Guide pp.226–227 (`output_type` RuleSet); Quinlan, *C4.5: Programs for
//! Machine Learning* (1993), ch.5 (public tree-to-rule simplification). No See5/C5.0
//! implementation or Modeler executable was consulted.
//!
//! Evidence (`models_ml/c50_rules_voting` and `c50_rules_firsthit`, identical PMML):
//! - Paths of the pruned tree become candidate rules (`c50_rules_voting`: 13 leaves,
//!   the same tree as `c50_default`). Each path is generalized by dropping conditions
//!   while the pessimistic error *rate* (the tree-pruning bound divided by coverage)
//!   does not increase: `flag = T and x > 9 → A` (71/72) drops `cat` and the redundant
//!   `x > 3`; `cat = amber and 3 < x ≤ 9 → B` (28/36) drops `flag = T`. Comparing
//!   estimated *counts* would keep `flag = T`, which the oracle does not.
//! - Duplicate generalized rules collapse (`c50_rules_voting`: four `x > 9, flag = T`
//!   A-leaves become one rule). Compound conditions are sorted by field name, then
//!   operator (`cat`, `flag`, `x`; `x ≤ 9` before `x > 3`).
//! - Every unique simplified rule that still covers a case of its class is kept
//!   (`c50_rules_voting`: 10 rules). They are grouped by target-category order
//!   (A, B, C), then by confidence descending, then by first tree-path index.
//! - Rule confidence and weight are the binary Laplace ratio
//!   `(nbCorrect + 1) / (recordCount + 2)` (`c50_rules_voting`: 32/36 B → 33/38).
//!   The Algorithms Guide's OCR'd direct-rule `correct/total` and tree-derived
//!   `correct/(total + n_categories)` do not match the exported PMML.
//! - Default class is the training mode (`c50_rules_voting`: A, 183 of 384); ties
//!   take the first target category. Default confidence is 0.5 (Algorithms Guide
//!   p.12). Both streams export `weightedSum`; `ruleset_evaluation` FirstHit does
//!   not change the PMML (`c50_rules_firsthit`).
//!
//! GAP: a fixture that discards a generalized rule (MDL / unique-cover selection)
//! is not in the captured oracles; extra simplified rules currently stay. C4.5
//! extracts from the unpruned tree; this fixture's extra unpruned splits are not
//! in the exported rules, so leaves of the pruned tree are used. Weighted
//! covering, costs, winnowing, boosting (`c50_boost_rules`) and missing training
//! predictors stay rejected.

use crate::c50::{self, C50Settings};
use crate::{AlgoError, Dataset, categories, data_dictionary, mining_schema};
use dms_pmml::{
    BooleanOperator, MiningFunction, Model, ModelHeader, Operator, Pmml, Predicate, RuleSetModel,
    SimpleRule,
};
use dms_types::{Field, Value};

const MAX_RULES: usize = 10_000;
const MAX_CONDITIONS: usize = 64;
const MAX_DEPTH: usize = 32;

struct TrainRow {
    index: usize,
    class: usize,
}

struct Train<'a> {
    dataset: &'a Dataset<'a>,
    fields: &'a [Field],
    classes: Vec<String>,
    rows: Vec<TrainRow>,
}

pub fn train(dataset: &Dataset<'_>, settings: &C50Settings) -> Result<Pmml, AlgoError> {
    let tree = c50::rule_tree(dataset, settings)?;
    let train = Train::read(dataset)?;
    let cf = (100.0 - settings.pruning_severity) / 100.0;
    let mut drafts = Vec::new();
    collect_paths(&tree, &mut Vec::new(), &train, cf, &mut drafts, 0)?;
    let drafts = unique_rules(drafts);
    let rules = emit_rules(&drafts, &train)?;
    let default = default_class(&train)?;
    Ok(Pmml {
        version: "4.3".into(),
        data_dictionary: data_dictionary(dataset),
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::RuleSet(RuleSetModel {
            header: ModelHeader {
                name: None,
                function: MiningFunction::Classification,
                algorithm: Some("RuleSet".into()),
                mining_schema: mining_schema(dataset),
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            selection: "weightedSum".into(),
            default_score: Some(default),
            default_confidence: Some(0.5),
            rules,
        }),
    })
}

impl<'a> Train<'a> {
    fn read(dataset: &'a Dataset<'a>) -> Result<Self, AlgoError> {
        let fields = dataset.table.model().fields();
        let target = fields
            .get(dataset.target)
            .ok_or_else(|| AlgoError::Data("invalid C5.0 target index".into()))?;
        let classes = categories(dataset, dataset.target);
        if classes.is_empty() {
            return Err(AlgoError::Data("C5.0 target has no categories".into()));
        }
        let mut rows = Vec::new();
        for (index, row) in dataset.table.rows().iter().enumerate() {
            let Some(value) = row.get(dataset.target).filter(|v| !target.is_missing(v)) else {
                continue;
            };
            let text = value.to_string();
            let class = classes
                .iter()
                .position(|c| c == &text)
                .ok_or_else(|| AlgoError::Data(format!("C5.0 unknown target {text:?}")))?;
            rows.push(TrainRow { index, class });
            if rows.len() > 100_000 {
                return Err(AlgoError::Unsupported(
                    "C5.0 is limited to 100000 records".into(),
                ));
            }
        }
        if rows.is_empty() {
            return Err(AlgoError::Data("C5.0 has no valid targets".into()));
        }
        Ok(Self {
            dataset,
            fields,
            classes,
            rows,
        })
    }

    fn coverage(&self, conditions: &[Predicate], class: usize) -> Result<(f64, f64), AlgoError> {
        let mut total = 0.0;
        let mut correct = 0.0;
        for row in &self.rows {
            if matches_all(self, row.index, conditions)? {
                total += 1.0;
                if row.class == class {
                    correct += 1.0;
                }
            }
        }
        Ok((total, correct))
    }
}

struct Draft {
    conditions: Vec<Predicate>,
    class: usize,
    origin: usize,
    total: f64,
    correct: f64,
}

fn collect_paths(
    node: &dms_pmml::Node,
    prefix: &mut Vec<Predicate>,
    train: &Train<'_>,
    cf: f64,
    out: &mut Vec<Draft>,
    depth: usize,
) -> Result<(), AlgoError> {
    if depth > MAX_DEPTH || out.len() >= MAX_RULES {
        return Err(AlgoError::Unsupported(
            "C5.0 rule depth/rule limit exceeded".into(),
        ));
    }
    if node.children.is_empty() {
        let score = node
            .score
            .as_ref()
            .ok_or_else(|| AlgoError::Data("C5.0 leaf has no class".into()))?;
        let class = train
            .classes
            .iter()
            .position(|c| c == score)
            .ok_or_else(|| AlgoError::Data(format!("C5.0 unknown leaf class {score:?}")))?;
        let mut conditions = Vec::new();
        for pred in prefix.iter() {
            push_atoms(pred, &mut conditions, 0)?;
        }
        simplify(&mut conditions, train, class, cf)?;
        let (total, correct) = train.coverage(&conditions, class)?;
        out.push(Draft {
            conditions,
            class,
            origin: out.len(),
            total,
            correct,
        });
        return Ok(());
    }
    for child in &node.children {
        if prefix.len() >= MAX_CONDITIONS {
            return Err(AlgoError::Unsupported(
                "C5.0 rule condition limit exceeded".into(),
            ));
        }
        prefix.push(child.predicate.clone());
        collect_paths(child, prefix, train, cf, out, depth.saturating_add(1))?;
        prefix.pop();
    }
    Ok(())
}

fn push_atoms(pred: &Predicate, out: &mut Vec<Predicate>, depth: usize) -> Result<(), AlgoError> {
    if depth > MAX_DEPTH || out.len() >= MAX_CONDITIONS {
        return Err(AlgoError::Unsupported(
            "C5.0 rule condition limit exceeded".into(),
        ));
    }
    match pred {
        Predicate::True => Ok(()),
        Predicate::Simple { .. } | Predicate::SimpleSet { .. } => {
            out.push(pred.clone());
            Ok(())
        }
        Predicate::Compound {
            operator: BooleanOperator::And,
            predicates,
        } => {
            for p in predicates {
                push_atoms(p, out, depth.saturating_add(1))?;
            }
            Ok(())
        }
        _ => Err(AlgoError::Unsupported(
            "C5.0 rule predicate that is not a conjunction of simple tests".into(),
        )),
    }
}

fn simplify(
    conditions: &mut Vec<Predicate>,
    train: &Train<'_>,
    class: usize,
    cf: f64,
) -> Result<(), AlgoError> {
    for _ in 0..MAX_CONDITIONS {
        if conditions.is_empty() {
            break;
        }
        let (total, correct) = train.coverage(conditions, class)?;
        let current = pessimistic_rate(total, total - correct, cf);
        let mut best: Option<(usize, f64, f64)> = None;
        for i in 0..conditions.len() {
            let mut trial = conditions.clone();
            if i >= trial.len() {
                continue;
            }
            trial.remove(i);
            let (n, c) = train.coverage(&trial, class)?;
            let rate = pessimistic_rate(n, n - c, cf);
            let better = best.as_ref().is_none_or(|(_, r, n_best)| {
                rate < *r - 1e-12 || ((rate - *r).abs() <= 1e-12 && n > *n_best)
            });
            if rate <= current + 1e-12 && better {
                best = Some((i, rate, n));
            }
        }
        let Some((i, _, _)) = best else {
            break;
        };
        if i < conditions.len() {
            conditions.remove(i);
        }
    }
    Ok(())
}

fn pessimistic_rate(total: f64, errors: f64, cf: f64) -> f64 {
    if total <= 0.0 {
        return 1.0;
    }
    let errors = if errors.is_finite() && errors > 0.0 {
        errors
    } else {
        0.0
    };
    c50::error_bound(total, errors, cf) / total
}

fn unique_rules(drafts: Vec<Draft>) -> Vec<Draft> {
    let mut kept = Vec::new();
    for draft in drafts {
        if draft.correct <= 0.0 {
            continue;
        }
        let key = condition_key(&draft.conditions);
        if kept
            .iter()
            .any(|k: &Draft| k.class == draft.class && condition_key(&k.conditions) == key)
        {
            continue;
        }
        kept.push(draft);
        if kept.len() >= MAX_RULES {
            break;
        }
    }
    kept
}

fn condition_key(conditions: &[Predicate]) -> Vec<(String, u8, String)> {
    let mut keys: Vec<_> = conditions.iter().map(atom_key).collect();
    keys.sort();
    keys
}

fn atom_key(pred: &Predicate) -> (String, u8, String) {
    match pred {
        Predicate::Simple {
            field,
            operator,
            value,
        } => (
            field.clone(),
            *operator as u8,
            value.clone().unwrap_or_default(),
        ),
        Predicate::SimpleSet {
            field,
            is_in,
            values,
        } => (
            field.clone(),
            if *is_in { 100 } else { 101 },
            values.join("\u{1f}"),
        ),
        _ => (String::new(), 255, String::new()),
    }
}

fn emit_rules(drafts: &[Draft], train: &Train<'_>) -> Result<Vec<SimpleRule>, AlgoError> {
    let mut order: Vec<usize> = (0..drafts.len()).collect();
    order.sort_by(|&a, &b| {
        let (Some(left), Some(right)) = (drafts.get(a), drafts.get(b)) else {
            return a.cmp(&b);
        };
        let conf_a = laplace(left.correct, left.total);
        let conf_b = laplace(right.correct, right.total);
        left.class
            .cmp(&right.class)
            .then(conf_b.total_cmp(&conf_a))
            .then(left.origin.cmp(&right.origin))
    });
    let mut rules = Vec::new();
    for (id, &index) in order.iter().enumerate() {
        let Some(draft) = drafts.get(index) else {
            continue;
        };
        let score = train.classes.get(draft.class).ok_or_else(|| {
            AlgoError::Data("C5.0 rule class is missing from the target categories".into())
        })?;
        let confidence = laplace(draft.correct, draft.total);
        rules.push(SimpleRule {
            id: Some(id.to_string()),
            predicate: emit_predicate(&draft.conditions),
            score: score.clone(),
            confidence,
            weight: confidence,
            record_count: Some(draft.total),
            correct: Some(draft.correct),
            disabled: false,
        });
        if rules.len() >= MAX_RULES {
            return Err(AlgoError::Unsupported("C5.0 rule limit exceeded".into()));
        }
    }
    Ok(rules)
}

fn laplace(correct: f64, total: f64) -> f64 {
    if total < 0.0 || !total.is_finite() || !correct.is_finite() {
        return 0.5;
    }
    (correct + 1.0) / (total + 2.0)
}

fn emit_predicate(conditions: &[Predicate]) -> Predicate {
    let mut atoms = conditions.to_vec();
    atoms.sort_by_key(atom_key);
    match atoms.len() {
        0 => Predicate::True,
        1 => atoms.into_iter().next().unwrap_or(Predicate::True),
        _ => Predicate::Compound {
            operator: BooleanOperator::And,
            predicates: atoms,
        },
    }
}

fn default_class(train: &Train<'_>) -> Result<String, AlgoError> {
    let mut counts = vec![0.0; train.classes.len()];
    for row in &train.rows {
        if let Some(slot) = counts.get_mut(row.class) {
            *slot += 1.0;
        }
    }
    let max = counts.iter().copied().fold(0.0, f64::max);
    let index = counts
        .iter()
        .position(|c| *c == max)
        .ok_or_else(|| AlgoError::Data("C5.0 has no default class".into()))?;
    train
        .classes
        .get(index)
        .cloned()
        .ok_or_else(|| AlgoError::Data("C5.0 has no default class".into()))
}

fn matches_all(
    train: &Train<'_>,
    row_index: usize,
    conditions: &[Predicate],
) -> Result<bool, AlgoError> {
    let Some(row) = train.dataset.table.rows().get(row_index) else {
        return Ok(false);
    };
    for pred in conditions {
        if !matches_atom(train.fields, row, pred)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn matches_atom(fields: &[Field], row: &[Value], pred: &Predicate) -> Result<bool, AlgoError> {
    match pred {
        Predicate::True => Ok(true),
        Predicate::Simple {
            field,
            operator,
            value,
        } => {
            let pos = field_pos(fields, field)?;
            let Some(field_def) = fields.get(pos) else {
                return Ok(false);
            };
            let Some(cell) = row.get(pos) else {
                return Ok(false);
            };
            if field_def.is_missing(cell) {
                return Ok(false);
            }
            match operator {
                Operator::Equal | Operator::NotEqual => {
                    let rhs = value.as_deref().unwrap_or("");
                    let eq = cell.to_string() == rhs;
                    Ok(if *operator == Operator::Equal {
                        eq
                    } else {
                        !eq
                    })
                }
                Operator::LessThan
                | Operator::LessOrEqual
                | Operator::GreaterThan
                | Operator::GreaterOrEqual => {
                    let Some(lhs) = cell.as_f64().filter(|n| n.is_finite()) else {
                        return Ok(false);
                    };
                    let rhs = value
                        .as_deref()
                        .unwrap_or("")
                        .parse::<f64>()
                        .ok()
                        .filter(|n| n.is_finite())
                        .ok_or_else(|| {
                            AlgoError::Data(format!("C5.0 rule cut {value:?} is not numeric"))
                        })?;
                    Ok(match operator {
                        Operator::LessThan => lhs < rhs,
                        Operator::LessOrEqual => lhs <= rhs,
                        Operator::GreaterThan => lhs > rhs,
                        _ => lhs >= rhs,
                    })
                }
                Operator::IsMissing => Ok(false),
                Operator::IsNotMissing => Ok(true),
            }
        }
        Predicate::SimpleSet {
            field,
            is_in,
            values,
        } => {
            let pos = field_pos(fields, field)?;
            let Some(field_def) = fields.get(pos) else {
                return Ok(false);
            };
            let Some(cell) = row.get(pos) else {
                return Ok(false);
            };
            if field_def.is_missing(cell) {
                return Ok(false);
            }
            let text = cell.to_string();
            let found = values.iter().any(|v| v == &text);
            Ok(if *is_in { found } else { !found })
        }
        Predicate::Compound {
            operator: BooleanOperator::And,
            predicates,
        } => {
            for p in predicates {
                if !matches_atom(fields, row, p)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Err(AlgoError::Unsupported(
            "C5.0 rule predicate that is not a conjunction of simple tests".into(),
        )),
    }
}

fn field_pos(fields: &[Field], name: &str) -> Result<usize, AlgoError> {
    fields
        .iter()
        .position(|f| f.name == name)
        .ok_or_else(|| AlgoError::Data(format!("C5.0 rule refers to unknown field {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c50::C50Settings;
    use dms_types::{DataModel, Measurement, Storage, Table, ValueSpec};

    fn sample() -> Table {
        let mut x = dms_types::Field::new("x", Storage::Real);
        x.measurement = Measurement::Continuous;
        let mut y = dms_types::Field::new("y", Storage::String);
        y.measurement = Measurement::Nominal;
        y.values = ValueSpec::Categories(vec![Value::string("a"), Value::string("b")]);
        Table::new(
            DataModel::new(vec![x, y]).unwrap(),
            (0..20)
                .map(|i| {
                    vec![
                        Value::Real(i as f64),
                        Value::string(if i < 10 { "a" } else { "b" }),
                    ]
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn laplace_confidence_default_and_weighted_sum() {
        let table = sample();
        let dataset = Dataset {
            table: &table,
            target: 1,
            inputs: &[0],
            frequency: None,
        };
        let pmml = crate::c50::train(
            &dataset,
            &C50Settings {
                rules: true,
                ..C50Settings::default()
            },
        )
        .unwrap();
        let Model::RuleSet(set) = pmml.model else {
            panic!("expected a rule set");
        };
        assert_eq!(set.selection, "weightedSum");
        assert_eq!(set.default_score.as_deref(), Some("a"));
        assert_eq!(set.default_confidence, Some(0.5));
        assert_eq!(set.rules.len(), 2);
        for rule in &set.rules {
            let total = rule.record_count.unwrap();
            let correct = rule.correct.unwrap();
            assert!((rule.confidence - (correct + 1.0) / (total + 2.0)).abs() < 1e-12);
            assert_eq!(rule.weight, rule.confidence);
        }
        assert_eq!(set.rules[0].score, "a");
        assert_eq!(set.rules[1].score, "b");
    }
}
