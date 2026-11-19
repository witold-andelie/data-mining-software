//! Cox regression (`spss.coxreg_id!buildcoxreg` / `applycoxreg`): the build node's settings and
//! the nugget that scores survival.
//!
//! The build node saves its settings as inline `_...` properties (`ibm-demos/telco_coxreg`). They
//! map to [`dms_algo::cox::Settings`]; the training is [`dms_algo::cox_model`]. Strata (`_groups`),
//! custom terms and a non-flag status field are `GAP:`s. The Simple mode uses the node's defaults
//! for the estimation controls; only the method is read.
//!
//! The nugget scores each record at `_num_future_times` future times and writes, per time `k`,
//! `$C-<status>-k` (the more probable status), `$CP-<status>-k` (its probability) and, with
//! `_all_probabilities`, `$CP-<value>-k` for each status value; with `_cumulative_hazard`
//! also `$CH-<status>-k`. The field names come from the saved nugget's output list
//! (`ibm-demos/telco_coxreg`). What the values are is unverified: Modeler's oracle run does not
//! score this nugget. The event probability is `1 - S`, with `S` the survival over the interval
//! from the past survival time to the future time (Algorithms Guide, "Scoring"). A record with
//! a missing input, or whose total time passes the longest event time, scores `$null$`.

use super::field_util::{Column, with_columns};
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use crate::{ExecContext, NodeOutput};
use dms_algo::cox::{Method, RemovalCriterion, Settings};
use dms_pmml::{Model, Pmml, Value as PValue};
use dms_stream::StreamNode;
use dms_types::{Field, Measurement, Storage, Table, Value};
use std::sync::Arc;

/// Most future times one nugget scores.
const MAX_FUTURE_TIMES: i64 = 1_000;

fn gap(what: impl std::fmt::Display) -> OperatorError {
    OperatorError::Settings(format!("GAP: Cox regression {what}"))
}

/// What the build node needs: the trainer's settings and the survival-time field.
#[derive(Debug, Clone)]
pub(super) struct BuildSettings {
    pub settings: Settings,
    pub time_field: String,
}

pub(super) fn build_settings(node: &StreamNode) -> Result<BuildSettings, OperatorError> {
    let s = node.settings.as_ref();
    let text = |key: &str| settings::text(s, key).map(|v| v.trim().to_string());
    let time_field = text("_survival_time")
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            OperatorError::Settings("Cox regression needs a survival time field".into())
        })?;
    let method = match text("_method").as_deref() {
        None | Some("" | "Enter") => Method::Enter,
        Some("Stepwise") => Method::Stepwise,
        Some("BackwardsStepwise") => Method::BackwardsStepwise,
        Some(other) => return Err(gap(format!("_method={other:?}"))),
    };
    let mut out = Settings {
        method,
        ..Settings::default()
    };
    if text("_mode").as_deref() == Some("Expert") {
        if text("_groups").is_some_and(|g| !g.is_empty()) {
            return Err(gap("with strata (_groups)"));
        }
        if !matches!(
            text("_model_type").as_deref(),
            None | Some("" | "MainEffects")
        ) {
            return Err(gap("with custom terms (_model_type)"));
        }
        out.removal_criterion = match text("_removal_criterion").as_deref() {
            None | Some("" | "LR") => RemovalCriterion::Lr,
            Some("Wald") => RemovalCriterion::Wald,
            Some("Conditional") => RemovalCriterion::Conditional,
            Some(other) => return Err(gap(format!("_removal_criterion={other:?}"))),
        };
        if let Some(p) = settings::real(s, "_probability_entry")? {
            out.probability_entry = p;
        }
        if let Some(p) = settings::real(s, "_probability_removal")? {
            out.probability_removal = p;
        }
        if let Some(n) = settings::integer(s, "_max_iterations")? {
            out.max_iterations = usize::try_from(n)
                .map_err(|_| OperatorError::Settings("Cox _max_iterations is negative".into()))?;
        }
        if let Some(x) = settings::real(s, "_p_converge")? {
            out.parameter_convergence = x;
        }
        if let Some(x) = settings::real(s, "_l_converge")? {
            out.log_likelihood_convergence = x;
        }
    }
    Ok(BuildSettings {
        settings: out,
        time_field,
    })
}

/// Where the future times of a record come from.
#[derive(Debug, Clone)]
enum FutureTimes {
    /// `k * interval` for `k = 1..=count`.
    Intervals { count: usize, interval: f64 },
    /// The value of a field of the record.
    Field(String),
}

struct Apply {
    member: Option<String>,
    future: FutureTimes,
    /// The field holding the time the record already survived; none means 0.
    past: Option<String>,
    all_probabilities: bool,
    cumulative_hazard: bool,
}

pub(super) fn apply(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let text = |key: &str| settings::text(s, key).map(|v| v.trim().to_string());
    let future = match text("_future_time_as").as_deref() {
        None | Some("" | "Intervals") => {
            let count = settings::integer(s, "_num_future_times")?.unwrap_or(1);
            if !(1..=MAX_FUTURE_TIMES).contains(&count) {
                return Err(OperatorError::Settings(format!(
                    "Cox _num_future_times must be between 1 and {MAX_FUTURE_TIMES}, found {count}"
                )));
            }
            let interval = settings::real(s, "_time_interval")?.unwrap_or(1.0);
            if !(interval.is_finite() && interval > 0.0) {
                return Err(OperatorError::Settings(format!(
                    "Cox _time_interval must be positive, found {interval}"
                )));
            }
            FutureTimes::Intervals {
                count: usize::try_from(count).unwrap_or(1),
                interval,
            }
        }
        Some("Fields") => FutureTimes::Field(
            text("_time_field")
                .filter(|name| !name.is_empty())
                .ok_or_else(|| OperatorError::Settings("Cox nugget needs a time field".into()))?,
        ),
        Some(other) => return Err(gap(format!("_future_time_as={other:?}"))),
    };
    let member = settings::element(s, "model")
        .and_then(|model| model.attribute("pmml"))
        .map(str::to_string)
        .or_else(|| {
            node.containers
                .iter()
                .find(|c| matches!(c.name.as_deref(), Some("PMML" | "model")))
                .map(|c| c.member.clone())
        });
    Ok(Box::new(Apply {
        member,
        future,
        past: text("_past_survival_time").filter(|name| !name.is_empty()),
        all_probabilities: settings::flag(s, "_all_probabilities")?.unwrap_or(false),
        cumulative_hazard: settings::flag(s, "_cumulative_hazard")?.unwrap_or(false),
    }))
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
            .map(|table| NodeOutput::Data(Arc::new(table)))
    }
}

fn to_pmml(field: &Field, value: &Value) -> PValue {
    if field.is_missing(value) {
        return PValue::Missing;
    }
    match value {
        Value::Integer(n) => PValue::Number(*n as f64),
        Value::Real(x) => PValue::Number(*x),
        Value::String(text) => PValue::Text(text.to_string()),
        _ => PValue::Missing,
    }
}

fn column(name: &str, storage: Storage, values: Vec<Value>) -> Column {
    let mut field = Field::new(name, storage);
    field.measurement = match storage {
        Storage::Real | Storage::Integer => Measurement::Continuous,
        _ => Measurement::Categorical,
    };
    Column { field, values }
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
            .ok_or_else(|| OperatorError::Inputs("a Cox nugget needs one input".into()))?;
        let pmml = match model {
            Some(model) => model,
            None => {
                let member = self.member.as_deref().ok_or_else(|| {
                    OperatorError::Settings(
                        "the nugget saved no model and its build node did not run".into(),
                    )
                })?;
                let xml = super::apply::saved_model_xml(member, context)?;
                Arc::new(
                    Pmml::parse(&xml)
                        .map_err(|error| OperatorError::Data(format!("model: {error}")))?,
                )
            }
        };
        let Model::Cox(cox) = &pmml.model else {
            return Err(OperatorError::Data(
                "the nugget's model is not a Cox model".into(),
            ));
        };
        let fields = input.model().fields();
        let position = |name: &str| {
            input.model().position(name).ok_or_else(|| {
                OperatorError::Data(format!("model: the input has no field {name:?}"))
            })
        };
        let term_positions = cox
            .terms
            .iter()
            .map(|term| position(&term.field))
            .collect::<Result<Vec<_>, _>>()?;
        let past_at = self.past.as_deref().map(position).transpose()?;
        let time_at = match &self.future {
            FutureTimes::Field(name) => Some(position(name)?),
            FutureTimes::Intervals { .. } => None,
        };
        let count = match &self.future {
            FutureTimes::Intervals { count, .. } => *count,
            FutureTimes::Field(_) => 1,
        };

        // the two status values: the event value and the other one
        let categories: Vec<String> = pmml
            .data_dictionary
            .iter()
            .find(|f| f.name == cox.event_field)
            .map(|f| f.values.clone())
            .filter(|values| values.len() == 2)
            .or_else(|| (cox.event_value == "1").then(|| vec!["0".to_string(), "1".to_string()]))
            .ok_or_else(|| gap("nugget whose status values are unknown (not a 0/1 flag)"))?;
        let numeric = categories.iter().all(|c| c.trim().parse::<i64>().is_ok());
        let status_value = |label: &str| -> Value {
            if numeric {
                label
                    .trim()
                    .parse::<i64>()
                    .map_or(Value::Null, Value::Integer)
            } else {
                Value::string(label.to_string())
            }
        };
        let storage = if numeric {
            Storage::Integer
        } else {
            Storage::String
        };
        let event_at = categories.iter().position(|c| *c == cox.event_value);

        struct Scored {
            event: Option<f64>,
            hazard: Option<f64>,
        }
        let mut per_time: Vec<Vec<Scored>> = (0..count).map(|_| Vec::new()).collect();
        for row in input.rows() {
            let record: Vec<PValue> = fields
                .iter()
                .zip(row)
                .map(|(field, value)| to_pmml(field, value))
                .collect();
            let lp = cox.linear_predictor(|name| {
                term_positions
                    .iter()
                    .zip(&cox.terms)
                    .find(|(_, term)| term.field == name)
                    .and_then(|(at, _)| record.get(*at))
            });
            let number_at = |at: usize| match record.get(at) {
                Some(PValue::Number(x)) => Some(*x),
                _ => None,
            };
            let past = match past_at {
                None => Some(0.0),
                Some(at) => number_at(at),
            };
            for (k, scored) in per_time.iter_mut().enumerate() {
                let future = match (&self.future, time_at) {
                    (FutureTimes::Intervals { interval, .. }, _) => Some((k + 1) as f64 * interval),
                    (FutureTimes::Field(_), Some(at)) => number_at(at),
                    _ => None,
                };
                let result = lp
                    .zip(past)
                    .zip(future)
                    .and_then(|((lp, past), future)| cox.conditional(lp, past, future));
                scored.push(match result {
                    Some(score) => Scored {
                        event: Some(1.0 - score.survival),
                        hazard: Some(score.cumulative_hazard),
                    },
                    None => Scored {
                        event: None,
                        hazard: None,
                    },
                });
            }
        }

        let target = &cox.event_field;
        let mut columns = Vec::new();
        for (k, scored) in per_time.iter().enumerate() {
            let n = k + 1;
            let predicted = |s: &Scored| -> Option<(usize, f64)> {
                let event = s.event?;
                let other = 1.0 - event;
                let event_at = event_at?;
                // ties go to the status that did not happen
                Some(if event > other {
                    (event_at, event)
                } else {
                    (1 - event_at, other)
                })
            };
            columns.push(column(
                &format!("$C-{target}-{n}"),
                storage,
                scored
                    .iter()
                    .map(|s| {
                        predicted(s)
                            .and_then(|(at, _)| categories.get(at))
                            .map_or(Value::Null, |label| status_value(label))
                    })
                    .collect(),
            ));
            columns.push(column(
                &format!("$CP-{target}-{n}"),
                Storage::Real,
                scored
                    .iter()
                    .map(|s| predicted(s).map_or(Value::Null, |(_, p)| Value::real(p)))
                    .collect(),
            ));
            if self.all_probabilities {
                for (at, label) in categories.iter().enumerate() {
                    columns.push(column(
                        &format!("$CP-{label}-{n}"),
                        Storage::Real,
                        scored
                            .iter()
                            .map(|s| match s.event {
                                Some(event) if Some(at) == event_at => Value::real(event),
                                Some(event) => Value::real(1.0 - event),
                                None => Value::Null,
                            })
                            .collect(),
                    ));
                }
            }
            if self.cumulative_hazard {
                columns.push(column(
                    &format!("$CH-{target}-{n}"),
                    Storage::Real,
                    scored
                        .iter()
                        .map(|s| s.hazard.map_or(Value::Null, Value::real))
                        .collect(),
                ));
            }
        }
        with_columns(input, columns)
    }
}
