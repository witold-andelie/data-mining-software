//! Derive (Field Operations): adds a field computed from each record, in single-field or
//! multiple-field mode (`@FIELD` stands for each listed field; the new name is the field name
//! plus a prefix or suffix).
//!
//! Implemented: all six result types. `formula` (CLEM expression), `flag` (true/false value by
//! condition; a null condition gives the false value), `set` (the first row whose condition holds
//! gives its value, else the default; row order is the saved `index`), `state` (stateful on/off
//! switch, evaluated against the current state), `count` (running count with reset and
//! increment) and `conditional` (if/then/else). Output measurement comes from the saved
//! `derive*FieldMeasure`; `auto` leaves it uninstantiated. Storage is inferred from the produced
//! values (flag, set and state values from the literal texts: all integers give an integer
//! field, as the oracle shows for numeric set values).
//!
//! A formula with a Boolean result is an error, as in Modeler (docs/clem/oracle-results.md rule
//! 32); conditions treat flag fields as truth values and null as false.
//!
//! GAP: the order of reset and increment in `count` when both hold (reset wins); sequence
//! functions (`@OFFSET`, `@SINCE`, ...) are not in the CLEM
//! evaluator yet, so expressions using them fail with a data error at run time.
//! Not implemented: collection and geospatial result details (`deriveFormulaCollectionMeasure`,
//! `deriveFormulaGeoMeasure`) beyond the measurement level.

use super::field_util::{
    Column, bind, bind_condition, eval, eval_condition, extended_name, field_list, field_value,
    literal_values, measurement, parse_optional, parse_required, position, record_number,
    with_columns,
};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_clem::Expr;
use dms_stream::StreamNode;
use dms_types::{Field, Measurement, Storage, Table, Value, ValueSpec};
use std::sync::Arc;

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let text = |path: &str| settings::text(s, path);
    let or_default = |path: &str, default: &str| text(path).unwrap_or_else(|| default.to_string());

    let targets = match text("deriveMode").as_deref().map(str::trim) {
        None | Some("") | Some("single") => {
            let name = text("deriveSingleFieldName").unwrap_or_default();
            if name.trim().is_empty() {
                return Err(OperatorError::Settings(
                    "deriveSingleFieldName: the new field needs a name".into(),
                ));
            }
            Targets::Single(name)
        }
        Some("multiple") => {
            let fields = field_list(s, "deriveMultipleFieldList");
            if fields.is_empty() {
                return Err(OperatorError::Settings(
                    "deriveMultipleFieldList: no fields selected".into(),
                ));
            }
            Targets::Multiple {
                fields,
                extension: or_default("deriveMultipleExtensionName", "_Derive"),
                prefix: text("deriveMultipleExtensionPosition")
                    .as_deref()
                    .map(str::trim)
                    == Some("prefix"),
            }
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "deriveMode: unknown mode {other:?}"
            )));
        }
    };

    let (kind, measure_path) = match text("deriveType").as_deref().map(str::trim) {
        None | Some("") | Some("formula") => (
            Kind::Formula(parse_required(
                "deriveFormulaExpression",
                text("deriveFormulaExpression"),
            )?),
            "deriveFormulaFieldMeasure",
        ),
        Some("flag") => {
            let (_, values) = literal_values(&[
                &or_default("deriveFlagTrueValue", "T"),
                &or_default("deriveFlagFalseValue", "F"),
            ]);
            let mut values = values.into_iter();
            (
                Kind::Flag {
                    condition: parse_required(
                        "deriveFlagFieldCondition",
                        text("deriveFlagFieldCondition"),
                    )?,
                    true_value: values.next().unwrap_or(Value::Null),
                    false_value: values.next().unwrap_or(Value::Null),
                },
                "deriveFlagFieldMeasure",
            )
        }
        Some("set") => {
            let mut rows: Vec<(i64, String, Expr)> = Vec::new();
            for (ordinal, row) in settings::children(s, "deriveSetTable/rows", "row")
                .into_iter()
                .enumerate()
            {
                let name = row.child("name").map(|e| e.text()).unwrap_or_default();
                let index = settings::integer(Some(row), "index")?
                    .unwrap_or_else(|| i64::try_from(ordinal).unwrap_or(i64::MAX));
                let condition = parse_required(
                    "deriveSetTable/condition",
                    row.child("condition").map(|e| e.text()),
                )?;
                rows.push((index, name, condition));
            }
            rows.sort_by_key(|(index, _, _)| *index);
            let default = or_default("deriveSetDefaultValue", "default");
            let mut texts: Vec<&str> = rows.iter().map(|(_, name, _)| name.as_str()).collect();
            texts.push(&default);
            let (_, mut values) = literal_values(&texts);
            let default = values.pop().unwrap_or(Value::Null);
            (
                Kind::Set {
                    rows: rows
                        .into_iter()
                        .zip(values)
                        .map(|((_, _, condition), value)| (condition, value))
                        .collect(),
                    default,
                },
                "deriveSetFieldMeasure",
            )
        }
        Some("state") => {
            let (_, values) = literal_values(&[
                &or_default("deriveStateOnValue", "On"),
                &or_default("deriveStateOffValue", "Off"),
            ]);
            let mut values = values.into_iter();
            (
                Kind::State {
                    on_condition: parse_optional(
                        "deriveStateOnCondition",
                        text("deriveStateOnCondition"),
                    )?,
                    off_condition: parse_optional(
                        "deriveStateOffCondition",
                        text("deriveStateOffCondition"),
                    )?,
                    on_value: values.next().unwrap_or(Value::Null),
                    off_value: values.next().unwrap_or(Value::Null),
                    initially_on: text("deriveStateInitialState").as_deref().map(str::trim)
                        == Some("on"),
                },
                "deriveStateFieldMeasure",
            )
        }
        Some("count") => {
            let initial_text = or_default("deriveCountInitialValue", "0");
            let (_, mut initial) = literal_values(&[&initial_text]);
            let initial = match initial.pop() {
                Some(value @ (Value::Integer(_) | Value::Real(_))) => value,
                _ => {
                    return Err(OperatorError::Settings(format!(
                        "deriveCountInitialValue: expected a number, found {initial_text:?}"
                    )));
                }
            };
            (
                Kind::Count {
                    initial,
                    increment_condition: parse_optional(
                        "deriveCountIncrementCondition",
                        text("deriveCountIncrementCondition"),
                    )?,
                    increment_by: parse_required(
                        "deriveCountIncrementByExpression",
                        Some(or_default("deriveCountIncrementByExpression", "1")),
                    )?,
                    reset_condition: parse_optional(
                        "deriveCountResetCondition",
                        text("deriveCountResetCondition"),
                    )?,
                },
                "deriveCountFieldMeasure",
            )
        }
        Some("conditional") => (
            Kind::Conditional {
                condition: parse_required(
                    "deriveConditionalIfCondition",
                    text("deriveConditionalIfCondition"),
                )?,
                then: parse_required(
                    "deriveConditionalThenExpression",
                    text("deriveConditionalThenExpression"),
                )?,
                otherwise: parse_optional(
                    "deriveConditionalElseExpression",
                    text("deriveConditionalElseExpression"),
                )?,
            },
            "deriveConditionalFieldMeasure",
        ),
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "deriveType: unknown result type {other:?}"
            )));
        }
    };
    let measure = measurement(text(measure_path).as_deref());
    Ok(Box::new(Derive {
        targets,
        kind,
        measure,
    }))
}

enum Targets {
    Single(String),
    Multiple {
        fields: Vec<String>,
        extension: String,
        prefix: bool,
    },
}

enum Kind {
    Formula(Expr),
    Flag {
        condition: Expr,
        true_value: Value,
        false_value: Value,
    },
    Set {
        rows: Vec<(Expr, Value)>,
        default: Value,
    },
    State {
        on_condition: Option<Expr>,
        off_condition: Option<Expr>,
        on_value: Value,
        off_value: Value,
        initially_on: bool,
    },
    Count {
        initial: Value,
        increment_condition: Option<Expr>,
        increment_by: Expr,
        reset_condition: Option<Expr>,
    },
    Conditional {
        condition: Expr,
        then: Expr,
        otherwise: Option<Expr>,
    },
}

struct Derive {
    targets: Targets,
    kind: Kind,
    measure: Measurement,
}

impl Operator for Derive {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Derive needs one input".into()))?;
        let model = input.model();
        let mut columns = Vec::new();
        match &self.targets {
            Targets::Single(name) => columns.push(self.column(input, name, None)?),
            Targets::Multiple {
                fields,
                extension,
                prefix,
            } => {
                for field in fields {
                    let current = position(model, field)?;
                    let name = extended_name(field, extension, *prefix);
                    columns.push(self.column(input, &name, Some(current))?);
                }
            }
        }
        with_columns(input, columns)
    }
}

impl Derive {
    fn column(
        &self,
        input: &Table,
        name: &str,
        current: Option<usize>,
    ) -> Result<Column, OperatorError> {
        let model = input.model();
        let bound =
            |setting: &str, expr: &Expr| bind(&format!("{name}: {setting}"), expr, model, current);
        let condition = |setting: &str, expr: &Expr| {
            bind_condition(&format!("{name}: {setting}"), expr, model, current)
        };
        let rows = input.rows();
        let mut values = Vec::with_capacity(rows.len());
        let mut column = match &self.kind {
            Kind::Formula(expr) => {
                let compiled = bound("formula", expr)?;
                for (i, row) in rows.iter().enumerate() {
                    let index = record_number(i);
                    let datum = eval("formula", &compiled, row, index)?;
                    values.push(field_value(&format!("{name}: formula"), datum, index)?);
                }
                Column::inferred(name, self.measure, values)?
            }
            Kind::Flag {
                condition,
                true_value,
                false_value,
            } => {
                let compiled = bind_condition(
                    &format!("{name}: flag condition"),
                    condition,
                    model,
                    current,
                )?;
                for (i, row) in rows.iter().enumerate() {
                    let hit = eval_condition("flag condition", &compiled, row, record_number(i))?;
                    values.push(if hit { true_value } else { false_value }.clone());
                }
                let mut column = Column::inferred(name, self.measure, values)?;
                column.field.values = ValueSpec::Flag {
                    true_value: true_value.clone(),
                    false_value: false_value.clone(),
                };
                column.field.storage = storage_of(true_value, column.field.storage);
                column
            }
            Kind::Set {
                rows: cases,
                default,
            } => {
                let compiled = cases
                    .iter()
                    .map(|(test, value)| Ok((condition("set condition", test)?, value)))
                    .collect::<Result<Vec<_>, OperatorError>>()?;
                for (i, row) in rows.iter().enumerate() {
                    let mut chosen = default;
                    for (condition, value) in &compiled {
                        if eval_condition("set condition", condition, row, record_number(i))? {
                            chosen = value;
                            break;
                        }
                    }
                    values.push(chosen.clone());
                }
                self.literal_column(name, cases.iter().map(|(_, v)| v), default, values)
            }
            Kind::State {
                on_condition,
                off_condition,
                on_value,
                off_value,
                initially_on,
            } => {
                let on = on_condition
                    .as_ref()
                    .map(|expr| condition("state on condition", expr))
                    .transpose()?;
                let off = off_condition
                    .as_ref()
                    .map(|expr| condition("state off condition", expr))
                    .transpose()?;
                let mut is_on = *initially_on;
                for (i, row) in rows.iter().enumerate() {
                    let index = record_number(i);
                    let switch = if is_on { &off } else { &on };
                    if let Some(compiled) = switch
                        && eval_condition("state condition", compiled, row, index)?
                    {
                        is_on = !is_on;
                    }
                    values.push(if is_on { on_value } else { off_value }.clone());
                }
                self.literal_column(name, [on_value, off_value], on_value, values)
            }
            Kind::Count {
                initial,
                increment_condition,
                increment_by,
                reset_condition,
            } => {
                let increment = increment_condition
                    .as_ref()
                    .map(|expr| condition("count increment condition", expr))
                    .transpose()?;
                let amount = bound("count increment", increment_by)?;
                let reset = reset_condition
                    .as_ref()
                    .map(|expr| condition("count reset condition", expr))
                    .transpose()?;
                let mut count = initial.clone();
                for (i, row) in rows.iter().enumerate() {
                    let index = record_number(i);
                    let reset_now = match &reset {
                        Some(c) => eval_condition("count reset condition", c, row, index)?,
                        None => false,
                    };
                    if reset_now {
                        count = initial.clone();
                    } else {
                        let increment_now = match &increment {
                            Some(c) => eval_condition("count increment condition", c, row, index)?,
                            None => true,
                        };
                        if increment_now {
                            let by = field_value(
                                &format!("{name}: count increment"),
                                eval("count increment", &amount, row, index)?,
                                index,
                            )?;
                            count = add(&count, &by).ok_or_else(|| {
                                OperatorError::Data(format!(
                                    "{name}, record {index}: the count increment is not a number"
                                ))
                            })?;
                        }
                    }
                    values.push(count.clone());
                }
                Column::inferred(name, self.measure, values)?
            }
            Kind::Conditional {
                condition: test,
                then,
                otherwise,
            } => {
                let test = condition("if condition", test)?;
                let then = bound("then expression", then)?;
                let otherwise = otherwise
                    .as_ref()
                    .map(|expr| bound("else expression", expr))
                    .transpose()?;
                for (i, row) in rows.iter().enumerate() {
                    let index = record_number(i);
                    let chosen = if eval_condition("if condition", &test, row, index)? {
                        Some(&then)
                    } else {
                        otherwise.as_ref()
                    };
                    values.push(match chosen {
                        Some(compiled) => field_value(
                            &format!("{name}: conditional"),
                            eval("conditional", compiled, row, index)?,
                            index,
                        )?,
                        None => Value::Null,
                    });
                }
                Column::inferred(name, self.measure, values)?
            }
        };
        column.field.measurement = self.measure;
        if matches!(self.kind, Kind::Flag { .. }) && self.measure == Measurement::Default {
            column.field.measurement = Measurement::Flag;
        }
        Ok(column)
    }

    /// A column whose values all come from literal settings text: its storage follows the
    /// literals, even when some of them never occur.
    fn literal_column<'a>(
        &self,
        name: &str,
        literals: impl IntoIterator<Item = &'a Value>,
        default: &Value,
        values: Vec<Value>,
    ) -> Column {
        let mut storage = storage_of(default, Storage::Unknown);
        for literal in literals {
            let next = storage_of(literal, storage);
            storage = if storage == Storage::Unknown || next == storage {
                next
            } else if matches!(
                (storage, next),
                (Storage::Integer, Storage::Real) | (Storage::Real, Storage::Integer)
            ) {
                Storage::Real
            } else {
                Storage::String
            };
        }
        let mut field = Field::new(name, storage);
        field.measurement = self.measure;
        let values = values
            .into_iter()
            .map(|value| match (storage, value) {
                (Storage::Real, Value::Integer(i)) => Value::Real(i as f64),
                (Storage::String, value @ (Value::Integer(_) | Value::Real(_))) => {
                    Value::string(value.to_string())
                }
                (_, value) => value,
            })
            .collect();
        Column { field, values }
    }
}

fn storage_of(value: &Value, fallback: Storage) -> Storage {
    value.storage().unwrap_or(fallback)
}

/// Sum of two numbers, integer when both are integers.
fn add(a: &Value, b: &Value) -> Option<Value> {
    match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => Some(
            x.checked_add(*y)
                .map_or_else(|| Value::real(*x as f64 + *y as f64), Value::Integer),
        ),
        (Value::Null, _) | (_, Value::Null) => Some(a.clone()),
        _ => Some(Value::real(a.as_f64()? + b.as_f64()?)),
    }
}
