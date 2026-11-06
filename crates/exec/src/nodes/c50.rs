//! C5.0 build and nugget adapters. Kept separate from other builders/nuggets.
//! Unsupported training options are explicit errors; see dms_algo::c50 GAPs.

use super::field_util::{Column, with_columns};
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use crate::{ExecContext, NodeOutput};
use dms_algo::{
    Dataset,
    c50::{self, C50Settings, MisclassificationCosts},
};
use dms_pmml::{Model, Pmml, Prediction, Usage, Value as PValue};
use dms_stream::StreamNode;
use dms_types::{Field, Measurement, Role, Storage, Table, Value, ValueSpec};
use std::sync::Arc;

fn unsupported(what: &str) -> OperatorError {
    OperatorError::Settings(format!(
        "C5.0 unsupported: {what}; see crates/algo/src/c50.rs GAPs"
    ))
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let expert = settings::flag(s, "expertMode")?.unwrap_or(false);
    for (key, description) in [
        (
            "useWeight",
            "case weights (c50_weight: unresolved pruning, zero distributions, and internal count aggregation)",
        ),
        ("useFrequency", "frequency weights"),
        ("useCrossValidation", "cross-validation"),
        (
            "calculateAdjustedPropensities",
            "adjusted-propensity training (GAP: the builder does not write the test-partition tree that the nugget scores)",
        ),
    ] {
        if settings::flag(s, key)?.unwrap_or(false) {
            return Err(unsupported(description));
        }
    }
    let rules = match settings::text(s, "modelFormat").as_deref() {
        None | Some("decisionTree") => false,
        Some("ruleSet") => true,
        Some(other) => return Err(unsupported(&format!("modelFormat {other:?}"))),
    };
    let boost_trials = if settings::flag(s, "useBoosting")?.unwrap_or(false) {
        let trials = settings::integer(s, "boostingTrials")?.unwrap_or(10);
        Some(
            usize::try_from(trials)
                .ok()
                .filter(|t| (1..=1000).contains(t))
                .ok_or_else(|| {
                    OperatorError::Settings("C5.0 boostingTrials must be 1..1000".into())
                })?,
        )
    } else {
        None
    };
    let winnow = expert && settings::flag(s, "use_winnowing")?.unwrap_or(false);
    if winnow {
        return Err(unsupported(
            "winnowing (GAP: c50::winnow_columns follows the C5.0 specification and reproduces 15 of the 16 recorded winnowing streams; c50_winnow2/w03 differs, so the node keeps its saved models)",
        ));
    }
    if !expert {
        let generality = match settings::text(s, "modellingBias").as_deref() {
            None | Some("accuracy") => false,
            Some("generality") => true,
            Some(other) => return Err(unsupported(&format!("Simple modellingBias {other:?}"))),
        };
        let noise = settings::integer(s, "expectedNoise")?.unwrap_or(0);
        // c50_presets: these Simple tree presets reproduce Expert defaults exactly.
        // Unrecorded values/combinations remain explicit GAPs, not interpolated presets.
        if !matches!(noise, 0 | 5 | 10 | 20 | 30 | 50)
            || (generality && !matches!(noise, 0 | 20))
            || ((generality || noise != 0) && (rules || boost_trials.is_some()))
        {
            return Err(unsupported(
                "Simple preset (GAP c50_presets: recorded unboosted trees cover accuracy noise 0/5/10/20/30/50 and generality noise 0/20 only)",
            ));
        }
    }
    let min = settings::integer(s, "minRecordsPerBranch")?.unwrap_or(2);
    let severity = settings::real(s, "pruningSeverity")?.unwrap_or(75.0);
    let min_child_records = usize::try_from(min)
        .ok()
        .filter(|&m| (1..=100_000).contains(&m))
        .ok_or_else(|| {
            OperatorError::Settings("C5.0 minRecordsPerBranch must be 1..100000".into())
        })?;
    if !(1.0..100.0).contains(&severity) {
        return Err(OperatorError::Settings(
            "C5.0 pruningSeverity must be in [1,100)".into(),
        ));
    }
    let custom = !settings::flag(s, "autoModelFieldSelection")?.unwrap_or(true);
    let target = custom
        .then(|| settings::text(s, "modelTargetFieldSelection"))
        .flatten()
        .filter(|s| !s.is_empty());
    let inputs = custom.then(|| {
        settings::children(s, "modelInputFieldSelection", "field")
            .iter()
            .map(|f| f.text())
            .collect()
    });
    Ok(Box::new(Build {
        settings: C50Settings {
            min_child_records: if expert { min_child_records } else { 2 },
            pruning_severity: if expert { severity } else { 75.0 },
            global_pruning: !expert || settings::flag(s, "useGlobalPruning")?.unwrap_or(true),
            group_symbolics: settings::flag(s, "groupSymbolicValues")?.unwrap_or(false),
            weight_field: None,
            costs: if settings::flag(s, "useMisclassificationCosts")?.unwrap_or(false) {
                let entries: Vec<_> = settings::element(s, "misclassificationCosts/entries")
                    .into_iter()
                    .flat_map(|e| e.child_elements())
                    .filter(|e| &*e.name.local == "entry")
                    .take(10_001)
                    .collect();
                if entries.len() > 10_000 {
                    return Err(unsupported("more than 10000 cost entries"));
                }
                let entries = entries
                    .into_iter()
                    .map(|entry| {
                        let actual = settings::text(Some(entry), "trueValue").ok_or_else(|| {
                            OperatorError::Settings("C5.0 cost entry needs trueValue".into())
                        })?;
                        let predicted =
                            settings::text(Some(entry), "predictedValue").ok_or_else(|| {
                                OperatorError::Settings(
                                    "C5.0 cost entry needs predictedValue".into(),
                                )
                            })?;
                        let cost = settings::real(Some(entry), "cost")?.ok_or_else(|| {
                            OperatorError::Settings("C5.0 cost entry needs cost".into())
                        })?;
                        Ok((actual, predicted, cost))
                    })
                    .collect::<Result<Vec<_>, OperatorError>>()?;
                Some(MisclassificationCosts {
                    default_cost: settings::real(s, "misclassificationCosts/defaultCost")?
                        .unwrap_or(1.0),
                    entries,
                })
            } else {
                None
            },
            rules,
            boost_trials,
            winnow,
            growth_threshold: true,
        },
        target,
        inputs,
        use_partition: settings::flag(s, "usePartitionedData")?.unwrap_or(true),
        weight: if settings::flag(s, "useWeight")?.unwrap_or(false) {
            Some(
                settings::text(s, "weightField")
                    .filter(|f| !f.trim().is_empty())
                    .ok_or_else(|| {
                        OperatorError::Settings("C5.0 useWeight needs weightField".into())
                    })?,
            )
        } else {
            None
        },
    }))
}

struct Build {
    settings: C50Settings,
    target: Option<String>,
    inputs: Option<Vec<String>>,
    use_partition: bool,
    /// The case-weight field (`useWeight`, `weightField`).
    weight: Option<String>,
}

impl Operator for Build {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }
    fn run(&self, _: &[Arc<Table>], _: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "C5.0 builder produces a model".into(),
        ))
    }
    fn produce(
        &self,
        inputs: &[Arc<Table>],
        _: Option<Arc<Pmml>>,
        _: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("C5.0 needs one input".into()))?;
        let fields = input.model().fields();
        if fields.iter().any(|f| f.role == Role::Split) {
            return Err(unsupported("split-field training"));
        }
        if fields.iter().any(|f| f.role == Role::Frequency) {
            return Err(unsupported("Type frequency role"));
        }
        let target = if let Some(name) = &self.target {
            input
                .model()
                .position(name)
                .ok_or_else(|| OperatorError::Data(format!("C5.0 missing target {name}")))?
        } else {
            let targets: Vec<usize> = fields
                .iter()
                .enumerate()
                .filter(|(_, f)| matches!(f.role, Role::Target | Role::Both))
                .map(|(i, _)| i)
                .collect();
            let [target] = targets.as_slice() else {
                return Err(OperatorError::Settings(
                    "C5.0 needs exactly one target".into(),
                ));
            };
            *target
        };
        let active = if let Some(names) = &self.inputs {
            if names.is_empty() {
                return Err(OperatorError::Settings(
                    "C5.0 custom inputs are empty".into(),
                ));
            }
            names
                .iter()
                .map(|n| {
                    input
                        .model()
                        .position(n)
                        .ok_or_else(|| OperatorError::Data(format!("C5.0 missing input {n}")))
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            fields
                .iter()
                .enumerate()
                .filter(|(i, f)| *i != target && matches!(f.role, Role::Input | Role::Both))
                .map(|(i, _)| i)
                .collect()
        };
        let weight_field =
            self.weight
                .as_ref()
                .map(|name| {
                    input.model().position(name).ok_or_else(|| {
                        OperatorError::Data(format!("C5.0 missing weight field {name}"))
                    })
                })
                .transpose()?;
        let active: Vec<usize> = active
            .into_iter()
            .filter(|&i| Some(i) != weight_field)
            .collect();
        let settings = C50Settings {
            weight_field,
            ..self.settings.clone()
        };
        let table = training_partition(input, self.use_partition)?;
        let pmml = c50::train(
            &Dataset {
                table: &table,
                target,
                inputs: &active,
                frequency: None,
            },
            &settings,
        )
        .map_err(|e| OperatorError::Data(format!("C5.0: {e}")))?;
        Ok(NodeOutput::Model(Arc::new(pmml)))
    }
}

fn training_partition(input: &Table, enabled: bool) -> Result<Table, OperatorError> {
    let position = enabled
        .then(|| {
            input
                .model()
                .fields()
                .iter()
                .position(|f| f.role == Role::Partition)
        })
        .flatten();
    let Some(position) = position else {
        return Ok(input.clone());
    };
    let field = input
        .model()
        .fields()
        .get(position)
        .ok_or_else(|| OperatorError::Data("C5.0 invalid partition field".into()))?;
    let training = match &field.values {
        ValueSpec::Categories(values) => values.first(),
        _ => None,
    }
    .ok_or_else(|| {
        OperatorError::Data("C5.0 partition field does not declare training value".into())
    })?;
    let rows = input
        .rows()
        .iter()
        .filter(|r| r.get(position) == Some(training))
        .cloned()
        .collect();
    Table::new(input.model().clone(), rows)
        .map_err(|e| OperatorError::Data(format!("C5.0 partition: {e}")))
}

pub fn apply(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    if settings::flag(s, "displayRuleIdField")?.unwrap_or(false) {
        return Err(unsupported("rule-id output"));
    }
    // The adjusted-propensity model is a second tree PMML (`propensityModel@pmml`): the same
    // splits, with each node's test-partition counts and probabilities (`c50_partition`).
    let adjusted = if settings::flag(s, "calculateAdjustedPropensities")?.unwrap_or(false) {
        Some(
            settings::element(s, "propensityModel")
                .and_then(|m| m.attribute("pmml"))
                .map(str::to_string)
                .ok_or_else(|| {
                    OperatorError::Settings(
                        "C5.0 adjusted propensities need the saved propensityModel".into(),
                    )
                })?,
        )
    } else {
        None
    };
    Ok(Box::new(Apply {
        member: settings::element(s, "model")
            .and_then(|m| m.attribute("pmml"))
            .map(str::to_string),
        adjusted,
        confidence: settings::flag(s, "calculateConfidences")?.unwrap_or(true),
        raw: settings::flag(s, "calculateRawPropensities")?.unwrap_or(false),
    }))
}

struct Apply {
    member: Option<String>,
    /// Attachment of the adjusted-propensity tree, when `calculateAdjustedPropensities` is set.
    adjusted: Option<String>,
    confidence: bool,
    raw: bool,
}

fn attachment_pmml(context: &ExecContext, member: &str) -> Result<Pmml, OperatorError> {
    let bytes = context
        .attachments
        .get(member)
        .ok_or_else(|| OperatorError::Io(format!("C5.0 missing attachment {member}")))?;
    let xml = std::str::from_utf8(bytes)
        .map_err(|e| OperatorError::Data(format!("C5.0 {member}: {e}")))?;
    Pmml::parse(xml).map_err(|e| OperatorError::Data(format!("C5.0 {member}: {e}")))
}

fn find_node<'a>(node: &'a dms_pmml::Node, id: &str) -> Option<&'a dms_pmml::Node> {
    if node.id.as_deref() == Some(id) {
        return Some(node);
    }
    node.children.iter().find_map(|child| find_node(child, id))
}

impl Operator for Apply {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }
    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        self.score(inputs, None, context)
    }
    fn produce(
        &self,
        inputs: &[Arc<Table>],
        model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        self.score(inputs, model, context)
            .map(|t| NodeOutput::Data(Arc::new(t)))
    }
}

fn model_header(model: &Model) -> Result<&dms_pmml::ModelHeader, OperatorError> {
    match model {
        Model::Tree(m) => Ok(&m.header),
        Model::RuleSet(m) => Ok(&m.header),
        Model::Mining(m) => Ok(&m.header),
        _ => Err(unsupported("non-C5.0 nugget model")),
    }
}

impl Apply {
    fn score(
        &self,
        inputs: &[Arc<Table>],
        model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("C5.0 nugget needs one input".into()))?;
        let pmml = if let Some(model) = model {
            model
        } else {
            let member = self
                .member
                .as_deref()
                .ok_or_else(|| OperatorError::Data("C5.0 nugget has no saved model".into()))?;
            Arc::new(attachment_pmml(context, member)?)
        };
        let header = model_header(&pmml.model)?;
        let target = header
            .mining_schema
            .iter()
            .find(|f| f.usage == Usage::Target)
            .map(|f| &f.name)
            .ok_or_else(|| OperatorError::Data("C5.0 model has no target".into()))?;
        let names: Vec<&str> = input
            .model()
            .fields()
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        let scorer = pmml
            .bind(&names)
            .map_err(|e| OperatorError::Data(format!("C5.0: {e}")))?;
        let rows: Vec<Vec<PValue>> = input
            .rows()
            .iter()
            .map(|row| {
                input
                    .model()
                    .fields()
                    .iter()
                    .zip(row)
                    .map(|(f, v)| {
                        if f.is_missing(v) {
                            return PValue::Missing;
                        }
                        match v {
                            Value::Integer(n) => PValue::Number(*n as f64),
                            Value::Real(n) => PValue::Number(*n),
                            Value::String(s) => PValue::Text(s.to_string()),
                            _ => PValue::Missing,
                        }
                    })
                    .collect()
            })
            .collect();
        let predictions = rows
            .iter()
            .map(|values| {
                scorer
                    .score(values)
                    .map_err(|e| OperatorError::Data(format!("C5.0: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let index = (0..=input.model().fields().len())
            .find(|&i| {
                let suffix = if i == 0 { String::new() } else { i.to_string() };
                ["$C", "$CC", "$CRP", "$CAP"].iter().all(|prefix| {
                    input
                        .model()
                        .position(&format!("{prefix}{suffix}-{target}"))
                        .is_none()
                })
            })
            .ok_or_else(|| OperatorError::Data("C5.0 no free score field name".into()))?;
        let suffix = if index == 0 {
            String::new()
        } else {
            index.to_string()
        };
        let dd = pmml.data_dictionary.iter().find(|d| &d.name == target);
        let storage = match dd.map(|d| &d.data_type) {
            Some(dms_pmml::DataType::Integer) => Storage::Integer,
            Some(dms_pmml::DataType::Double | dms_pmml::DataType::Float) => Storage::Real,
            _ => Storage::String,
        };
        let mut field = Field::new(format!("$C{suffix}-{target}"), storage);
        field.measurement = Measurement::Categorical;
        let values = predictions
            .iter()
            .map(|p| match (&p.value, storage) {
                (PValue::Missing, _) => Value::Null,
                (PValue::Text(s), Storage::Integer) => {
                    s.parse().map_or(Value::Null, Value::Integer)
                }
                (PValue::Text(s), Storage::Real) => s.parse().map_or(Value::Null, Value::Real),
                (PValue::Text(s), _) => Value::string(s.as_str()),
                (PValue::Number(n), _) => Value::Real(*n),
            })
            .collect();
        let mut columns = vec![Column { field, values }];
        if self.confidence {
            columns.push(real_column(
                &format!("$CC{suffix}-{target}"),
                predictions.iter().map(|p| p.confidence),
            ));
        }
        let categories = dd.map(|d| d.values.as_slice()).unwrap_or(&[]);
        if self.raw {
            let [_, positive] = categories else {
                return Err(unsupported("raw propensity needs a binary target"));
            };
            columns.push(real_column(
                &format!("$CRP{suffix}-{target}"),
                predictions.iter().map(|p| raw_propensity(p, positive)),
            ));
        }
        if let Some(member) = &self.adjusted {
            let [_, positive] = categories else {
                return Err(unsupported("adjusted propensity needs a binary target"));
            };
            let adjusted = attachment_pmml(context, member)?;
            let values = adjusted_propensities(&adjusted, &names, &rows, positive)?;
            columns.push(real_column(
                &format!("$CAP{suffix}-{target}"),
                values.into_iter(),
            ));
        }
        with_columns(input, columns)
    }
}

/// Adjusted propensity from the test-partition tree (`c50_partition`, 529 scored records): the
/// record's leaf has `recordCount` N and, for its predicted class c, the stored
/// `ScoreDistribution@probability` p (six significant digits, so 4/6 is 0.666667). The adjusted
/// confidence is C5.0's Laplace estimate (N*p + 1)/(N + 2); the propensity is that confidence
/// when c is the positive category and its complement otherwise (4/6 gives 0.62500025, 10/12
/// gives 0.785714). Negative leaves were seen only with 0/N in both partitions; a negative leaf
/// with mixed test counts follows the same formula untested.
/// GAP: a record that reaches no single leaf (missing values spread over several children).
fn adjusted_propensities(
    pmml: &Pmml,
    names: &[&str],
    rows: &[Vec<PValue>],
    positive: &str,
) -> Result<Vec<Option<f64>>, OperatorError> {
    let Model::Tree(tree) = &pmml.model else {
        return Err(unsupported("adjusted-propensity model that is not a tree"));
    };
    let error =
        |message: String| OperatorError::Data(format!("C5.0 adjusted propensity: {message}"));
    let scorer = pmml.bind(names).map_err(|e| error(e.to_string()))?;
    rows.iter()
        .map(|values| {
            let prediction = scorer.score(values).map_err(|e| error(e.to_string()))?;
            let (PValue::Text(class), Some(id)) = (&prediction.value, &prediction.entity_id) else {
                return Err(unsupported(
                    "adjusted propensity for a record without a single leaf",
                ));
            };
            let node = find_node(&tree.root, id).ok_or_else(|| error(format!("no node {id}")))?;
            let count = node
                .record_count
                .ok_or_else(|| error(format!("node {id} has no recordCount")))?;
            let probability = node
                .distribution
                .iter()
                .find(|d| &d.value == class)
                .and_then(|d| d.probability)
                .ok_or_else(|| error(format!("node {id} has no probability for {class}")))?;
            let confidence = (count * probability + 1.0) / (count + 2.0);
            Ok(Some(if class == positive {
                confidence
            } else {
                1.0 - confidence
            }))
        })
        .collect()
}

fn raw_propensity(p: &Prediction, positive: &str) -> Option<f64> {
    match (&p.value, p.confidence) {
        (PValue::Text(value), Some(confidence)) => Some(if value == positive {
            confidence
        } else {
            1.0 - confidence
        }),
        _ => None,
    }
}

fn real_column(name: &str, values: impl Iterator<Item = Option<f64>>) -> Column {
    let mut field = Field::new(name, Storage::Real);
    field.measurement = Measurement::Continuous;
    Column {
        field,
        values: values
            .map(|v| v.filter(|n| n.is_finite()).map_or(Value::Null, Value::Real))
            .collect(),
    }
}
