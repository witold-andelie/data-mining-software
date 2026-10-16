//! History (Field Operations): appends fields that hold values from earlier records.
//!
//! Implemented settings (children of `properties`; anything else fails
//! [`OperatorError::Settings`]):
//! - `historyFieldList/field`
//! - `historyOffset` (default 1), `historySpan` (default 1)
//! - `historyKeepUndef` (`discard` | `fill`; `leave` is rejected)
//! - `historyUndefValue` (the fill literal; default `undef`)
//!
//! Rules settled by the oracle (`corpus/fields/history_*.str`, `oracle/fields/history_*`):
//! - For each selected field, in list order, `historySpan` fields are appended. The name is
//!   `{field}__{k}` with `k` from 1 through the span (`history_xy_off1_fill`: `x__1`, `x__2`,
//!   `y__1`, `y__2`). `k` counts from the near end of the window, not from the absolute offset:
//!   `history_x_off3_fill` (offset 3, span 2) still names the fields `x__1` and `x__2`, and
//!   `x__1` is 3 records back, `x__2` is 4 records back. Record `id` 4 therefore has
//!   `x__1` = -10 (record 1) and `x__2` = -999.
//! - The original fields stay, in their original order, ahead of the new ones.
//! - `discard` drops every record that does not have the whole window
//!   (`history_x_off1_discard` and `history_xy_off1_discard` both start at `id` 3, the first
//!   record with two predecessors).
//! - `fill` keeps those records and writes `historyUndefValue`, coerced to the source field's
//!   storage (`-999` on real `x` is real). A null already stored in an earlier record stays
//!   null: `history_x_off3_fill` record `id` 11 has `x__1` null (record 8's `x`) and `x__2`
//!   = 2.5; `history_xy_off1_fill` record `id` 13 has `y__1` null.
//! - New fields keep the source field's storage and measurement. Their known values are cleared,
//!   because a fill can sit outside the source range.

use super::field_util::{field_list, position};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::{DataModel, Field, Storage, Table, Value, ValueSpec};
use std::sync::Arc;

/// New columns, across every selected field. The probes use a span of 2.
const MAX_NEW_FIELDS: usize = 10_000;

const KNOWN_SETTINGS: &[&str] = &[
    "annotation",
    "autoLabel",
    "cacheEnabled",
    "createdBy",
    "creationDate",
    "doReadValues",
    "enabled",
    "fieldMappingEssentialFields",
    "historyFieldList",
    "historyKeepUndef",
    "historyOffset",
    "historySpan",
    "historyUndefValue",
    "keywords",
    "label",
    "lastSavedAs",
    "lastSavedBy",
    "lastSavedDate",
    "name",
    "position",
    "repositoryLastModified",
    "repositoryPath",
    "repositoryVersionMarker",
    "scriptExecutionModeChange",
    "toolTipText",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Unavailable {
    Discard,
    Fill,
}

struct History {
    fields: Vec<String>,
    /// Records back to the nearest history field (`__1`).
    offset: u64,
    span: u64,
    unavailable: Unavailable,
    fill_with: String,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    reject_unknown(s)?;
    reject_unexpected_children(s, "historyFieldList", &["field"])?;
    let fields = field_list(s, "historyFieldList");
    if fields.is_empty() {
        return Err(OperatorError::Settings(
            "historyFieldList: no fields selected".into(),
        ));
    }
    if fields.iter().any(|name| name.trim().is_empty()) {
        return Err(OperatorError::Settings(
            "historyFieldList: empty field name".into(),
        ));
    }
    if let Some(name) = duplicate(&fields) {
        return Err(OperatorError::Settings(format!(
            "historyFieldList: duplicate field {name:?}"
        )));
    }
    let offset = positive(s, "historyOffset", 1)?;
    let span = positive(s, "historySpan", 1)?;
    let count = fields.len().saturating_mul(span_usize(span)?);
    if count > MAX_NEW_FIELDS {
        return Err(OperatorError::Settings(format!(
            "historySpan creates {count} fields; the limit is {MAX_NEW_FIELDS}"
        )));
    }
    let unavailable = match settings::text(s, "historyKeepUndef")
        .as_deref()
        .map(str::trim)
    {
        None | Some("" | "discard" | "Discard") => Unavailable::Discard,
        Some("fill" | "Fill") => Unavailable::Fill,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "historyKeepUndef: expected discard or fill, found {other:?}"
            )));
        }
    };
    let fill_with = settings::text(s, "historyUndefValue").unwrap_or_else(|| "undef".into());
    Ok(Box::new(History {
        fields,
        offset,
        span,
        unavailable,
        fill_with,
    }))
}

fn positive(settings: Option<&Element>, path: &str, default: i64) -> Result<u64, OperatorError> {
    let value = settings::integer(settings, path)?.unwrap_or(default);
    if value < 1 {
        return Err(OperatorError::Settings(format!(
            "{path}: expected a positive integer, found {value}"
        )));
    }
    u64::try_from(value).map_err(|_| {
        OperatorError::Settings(format!("{path}: {value} does not fit an unsigned integer"))
    })
}

fn span_usize(span: u64) -> Result<usize, OperatorError> {
    usize::try_from(span).map_err(|_| {
        OperatorError::Settings(format!("historySpan {span} does not fit a column count"))
    })
}

fn duplicate(names: &[String]) -> Option<&str> {
    for (index, name) in names.iter().enumerate() {
        if names.iter().take(index).any(|earlier| earlier == name) {
            return Some(name);
        }
    }
    None
}

fn reject_unknown(settings: Option<&Element>) -> Result<(), OperatorError> {
    let Some(settings) = settings else {
        return Err(OperatorError::Settings("History has no settings".into()));
    };
    for child in settings.child_elements() {
        let name = child.name.local.as_ref();
        if !KNOWN_SETTINGS.contains(&name) {
            return Err(OperatorError::Settings(format!(
                "History setting {name:?} is not supported"
            )));
        }
    }
    Ok(())
}

fn reject_unexpected_children(
    settings: Option<&Element>,
    path: &str,
    allowed: &[&str],
) -> Result<(), OperatorError> {
    let Some(parent) = settings::element(settings, path) else {
        return Ok(());
    };
    for child in parent.child_elements() {
        let name = child.name.local.as_ref();
        if !allowed.contains(&name) {
            return Err(OperatorError::Settings(format!(
                "{path} child {name:?} is not supported"
            )));
        }
    }
    Ok(())
}

/// The fill literal in the source field's storage. `undef` on a number is a data error.
fn coerce_fill(text: &str, storage: Storage) -> Result<Value, OperatorError> {
    let trimmed = text.trim();
    let value = match storage {
        Storage::Integer => parse_integer(trimmed)?,
        Storage::Real => parse_real(trimmed)?,
        Storage::String => Value::string(text),
        other => {
            return Err(OperatorError::Data(format!(
                "historyUndefValue cannot fill a {other:?} field"
            )));
        }
    };
    Ok(value)
}

fn parse_integer(text: &str) -> Result<Value, OperatorError> {
    if let Ok(integer) = text.parse::<i64>() {
        return Ok(Value::Integer(integer));
    }
    if let Ok(real) = text.parse::<f64>()
        && real.is_finite()
        && real.fract() == 0.0
        && (i64::MIN as f64..=i64::MAX as f64).contains(&real)
    {
        return Ok(Value::Integer(real as i64));
    }
    Err(OperatorError::Data(format!(
        "historyUndefValue {text:?} is not an integer"
    )))
}

fn parse_real(text: &str) -> Result<Value, OperatorError> {
    if let Ok(integer) = text.parse::<i64>() {
        return Ok(Value::Real(integer as f64));
    }
    text.parse::<f64>()
        .ok()
        .filter(|real| real.is_finite())
        .map(Value::Real)
        .ok_or_else(|| OperatorError::Data(format!("historyUndefValue {text:?} is not a number")))
}

struct Prepared {
    pos: usize,
    fill: Option<Value>,
    template: Field,
}

impl Operator for History {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("History needs one input".into()))?;
        let model = input.model();
        let rows = input.rows();
        let span = span_usize(self.span)?;
        let mut prepared = Vec::with_capacity(self.fields.len());
        let mut new_names = Vec::new();
        for name in &self.fields {
            let pos = position(model, name)?;
            let source = model
                .field(name)
                .ok_or_else(|| OperatorError::Data(format!("input has no field {name:?}")))?;
            let fill = if self.unavailable == Unavailable::Fill {
                Some(coerce_fill(&self.fill_with, source.storage)?)
            } else {
                None
            };
            for step in 1..=span {
                let generated = format!("{name}__{step}");
                if model.position(&generated).is_some()
                    || new_names.iter().any(|seen: &String| seen == &generated)
                {
                    return Err(OperatorError::Data(format!(
                        "History would replace existing field {generated:?}"
                    )));
                }
                new_names.push(generated);
            }
            let mut template = source.clone();
            template.values = ValueSpec::Unknown;
            template.label = None;
            template.value_labels.clear();
            prepared.push(Prepared {
                pos,
                fill,
                template,
            });
        }

        let mut fields = model.fields().to_vec();
        for (spec, name) in prepared.iter().zip(&self.fields) {
            for step in 1..=span {
                let mut field = spec.template.clone();
                field.name = format!("{name}__{step}");
                fields.push(field);
            }
        }

        // The far end of the window. A record with a smaller index has no such predecessor.
        let depth = self
            .offset
            .checked_add(self.span)
            .and_then(|sum| sum.checked_sub(1))
            .ok_or_else(|| {
                OperatorError::Settings("historyOffset and historySpan overflow".into())
            })?;
        let mut out_rows = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let index_u = u64::try_from(index).map_err(|_| {
                OperatorError::Data(format!("record {index} does not fit an offset"))
            })?;
            if self.unavailable == Unavailable::Discard && index_u < depth {
                continue;
            }
            let mut out = row.clone();
            for spec in &prepared {
                for step in 1..=span {
                    let step_u = u64::try_from(step).map_err(|_| {
                        OperatorError::Data(format!("history step {step} does not fit an offset"))
                    })?;
                    let lag = self
                        .offset
                        .checked_add(step_u.saturating_sub(1))
                        .ok_or_else(|| OperatorError::Data("history offset overflow".into()))?;
                    let value = if index_u >= lag {
                        let previous = usize::try_from(index_u - lag).map_err(|_| {
                            OperatorError::Data("history offset does not fit a record index".into())
                        })?;
                        rows.get(previous)
                            .and_then(|record| record.get(spec.pos))
                            .cloned()
                            .unwrap_or(Value::Null)
                    } else {
                        spec.fill.clone().ok_or_else(|| {
                            OperatorError::Data(
                                "history fill is missing for an unavailable record".into(),
                            )
                        })?
                    };
                    out.push(value);
                }
            }
            out_rows.push(out);
        }
        let model =
            DataModel::new(fields).map_err(|error| OperatorError::Data(error.to_string()))?;
        Table::new(model, out_rows).map_err(|error| OperatorError::Data(error.to_string()))
    }
}
