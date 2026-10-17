//! RFM Aggregate, Modeler 19.0 (public Scripting and Automation Guide, table 70).
//!
//! Saved settings implemented: `rfmAggregate{IdField,DateField,ValueField,ContiguousIds,
//! RecencyDateMode,RecencyDate,DiscardRecordsBelow,DiscardRecordsBelowValue,
//! IncludeRecentTransactions,SpecifyTransactionDate,TransactionDateAfter,
//! TransactionWithinLast,TransactionsScale,SaveDate2,SaveDate3,NameExtension,
//! ExtensionPosition,RecencyOutputFieldName,FrequencyOutputFieldName,
//! MonetaryOutputFieldName,R2OutputFieldName,R3OutputFieldName}`. The saved
//! `IdOutputFieldName=Id` is unused by Modeler: the input ID name survives (`misc_ra_fixed`).
//! Nondefault ID renaming is rejected. Display/repository metadata has no effect.
//!
//! Oracle evidence (all under `oracle/misc/`):
//! - `misc_ra_fixed`: first surviving appearance order; frequency counts records; monetary
//!   sums values; recency is reference calendar day minus latest transaction calendar day.
//!   Future dates give negative recency. `misc_ra_timestamp` discards time of day.
//! - `misc_ra_default_today` actually saves `Fixed` with epoch milliseconds: no clock is read.
//!   Saved dates are instants; their day is in the stream's `timeZone`
//!   ([`ExecContext::day_of_instant`]: `server` is this machine's zone).
//! - `misc_ra_contiguous`: contiguous ID groups give the same totals on presorted input.
//! - `misc_ra_prefix`, `misc_ra_suffix`: extension concatenation adds no separator.
//! - `misc_ra_min_value`: discard amounts strictly below the threshold (equality survives).
//! - `misc_ra_date_after`: strictly after the absolute cutoff, with no upper date bound.
//! - `misc_ra_last_days`, `misc_ra_last_weeks`, `misc_ra_last_years`: relative windows are
//!   lower-exclusive, reference-inclusive. `misc_ra_last_months`: March 31 minus one month
//!   has no matching calendar day, giving an empty result rather than clamping to February 29.
//! - `misc_ra_second`, `misc_ra_third`: duplicate dates count separately; insufficient dates
//!   give null R2/R3, appended after Monetary. `misc_ra_missing`: null dates/amounts do not
//!   remove a record, nulls are ignored by max/sum, all-null sums are null, empty IDs group.
//!
//! GAP: live Today mode has no execution-date context and is rejected. Moving a server-zone
//! stream to a machine in another zone changes the days of saved instants, as moving it to
//! another Modeler server does.
//! GAP: relative calendar months/years beyond the observed valid-day/invalid-day behavior
//! need additional probes. Explicit user-missing definitions and repeated noncontiguous IDs
//! in ContiguousIds mode are rejected pending oracle evidence.

use super::field_util::{GroupKey, position};
use super::rfm_settings::{reject_unknown, required_text};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::calendar::{MICROS_PER_DAY, civil_from_days, days_from_civil};
use dms_types::{DataModel, Field, Storage, Table, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

const PREFIX: &str = "rfmAggregate";
const SETTINGS: &[&str] = &[
    "IdField",
    "DateField",
    "ValueField",
    "ContiguousIds",
    "RecencyDateMode",
    "RecencyDate",
    "DiscardRecordsBelow",
    "DiscardRecordsBelowValue",
    "IncludeRecentTransactions",
    "SpecifyTransactionDate",
    "TransactionDateAfter",
    "TransactionWithinLast",
    "TransactionsScale",
    "SaveDate2",
    "SaveDate3",
    "NameExtension",
    "ExtensionPosition",
    "IdOutputFieldName",
    "RecencyOutputFieldName",
    "FrequencyOutputFieldName",
    "MonetaryOutputFieldName",
    "R2OutputFieldName",
    "R3OutputFieldName",
];

struct RfmAggregate {
    id: String,
    date: String,
    value: String,
    reference: i64,
    contiguous: bool,
    minimum: Option<f64>,
    window: Window,
    names: Vec<String>,
}

enum Window {
    All,
    After(i64),
    Relative(i64, String),
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    reject_unknown(s, PREFIX, SETTINGS)?;
    let text = |key: &str| settings::text(s, &format!("{PREFIX}{key}"));
    let flag = |key: &str| settings::flag(s, &format!("{PREFIX}{key}"));
    if text("RecencyDateMode").as_deref() != Some("Fixed") {
        return Err(bad(
            "unsupported live Today recency; save a Fixed reference date",
        ));
    }
    let reference = required_integer(s, "RecencyDate")?;
    let minimum = if flag("DiscardRecordsBelow")?.unwrap_or(false) {
        Some(
            settings::real(s, "rfmAggregateDiscardRecordsBelowValue")?
                .ok_or_else(|| bad("missing DiscardRecordsBelowValue"))?,
        )
    } else {
        None
    };
    let window = if !flag("IncludeRecentTransactions")?.unwrap_or(false) {
        Window::All
    } else if flag("SpecifyTransactionDate")?.unwrap_or(true) {
        Window::After(required_integer(s, "TransactionDateAfter")?)
    } else {
        let count = required_integer(s, "TransactionWithinLast")?;
        if !(0..=1_000_000).contains(&count) {
            return Err(bad("TransactionWithinLast must be between 0 and 1000000"));
        }
        let scale = text("TransactionsScale").unwrap_or_else(|| "Months".into());
        if !["Days", "Weeks", "Months", "Years"].contains(&scale.as_str()) {
            return Err(bad(format!("unsupported TransactionsScale {scale:?}")));
        }
        Window::Relative(count, scale)
    };
    let second = flag("SaveDate2")?.unwrap_or(false);
    let third = flag("SaveDate3")?.unwrap_or(false);
    if third && !second {
        return Err(bad("SaveDate3 requires SaveDate2"));
    }
    if text("IdOutputFieldName").is_some_and(|name| name != "Id") {
        return Err(bad("unsupported custom IdOutputFieldName"));
    }
    let prefix = match text("ExtensionPosition").as_deref() {
        None | Some("Prefix") => true,
        Some("Suffix") => false,
        Some(other) => return Err(bad(format!("unsupported ExtensionPosition {other:?}"))),
    };
    let extension = text("NameExtension").unwrap_or_default();
    let mut names = Vec::new();
    for (key, default, enabled) in [
        ("RecencyOutputFieldName", "Recency", true),
        ("FrequencyOutputFieldName", "Frequency", true),
        ("MonetaryOutputFieldName", "Monetary", true),
        ("R2OutputFieldName", "R2", second),
        ("R3OutputFieldName", "R3", third),
    ] {
        if enabled {
            let name = text(key).unwrap_or_else(|| default.into());
            if name.trim().is_empty() {
                return Err(bad(format!("empty {key}")));
            }
            names.push(if prefix {
                format!("{extension}{name}")
            } else {
                format!("{name}{extension}")
            });
        }
    }
    Ok(Box::new(RfmAggregate {
        id: required_text(s, "rfmAggregateIdField")?,
        date: required_text(s, "rfmAggregateDateField")?,
        value: required_text(s, "rfmAggregateValueField")?,
        reference,
        contiguous: flag("ContiguousIds")?.unwrap_or(false),
        minimum,
        window,
        names,
    }))
}

#[derive(Default)]
struct Group {
    id: Option<Value>,
    count: i64,
    sum: Option<f64>,
    dates: Vec<i64>,
}

impl Operator for RfmAggregate {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("RFM Aggregate needs one input".into()))?;
        let model = input.model();
        let id = position(model, &self.id)?;
        let date = position(model, &self.date)?;
        let value = position(model, &self.value)?;
        let id_field = model
            .fields()
            .get(id)
            .ok_or_else(|| bad("missing ID metadata"))?;
        for p in [id, date, value] {
            if model
                .fields()
                .get(p)
                .is_some_and(|f| !f.missing.values.is_empty() || !f.missing.ranges.is_empty())
            {
                return Err(bad("unsupported user-missing definitions"));
            }
        }
        for (p, allowed) in [
            (date, &[Storage::Date, Storage::Timestamp][..]),
            (value, &[Storage::Integer, Storage::Real][..]),
        ] {
            if !model
                .fields()
                .get(p)
                .is_some_and(|f| allowed.contains(&f.storage))
            {
                return Err(bad(
                    "DateField must be date/timestamp; ValueField must be numeric",
                ));
            }
        }
        if !matches!(
            id_field.storage,
            Storage::String | Storage::Integer | Storage::Real
        ) {
            return Err(bad("unsupported ID storage"));
        }
        let reference = context.day_of_instant(self.reference)?;
        let (cutoff, bounded) = match &self.window {
            Window::All => (None, false),
            Window::After(millis) => (Some(context.day_of_instant(*millis)?), false),
            Window::Relative(count, scale) => (relative_day(reference, *count, scale), true),
        };
        let mut groups: Vec<Group> = Vec::new();
        let mut seen = BTreeMap::new();
        for row in input.rows() {
            let amount = match row.get(value).unwrap_or(&Value::Null) {
                Value::Null => None,
                v => Some(v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| {
                    OperatorError::Data("RFM Aggregate: nonfinite or nonnumeric amount".into())
                })?),
            };
            if self
                .minimum
                .is_some_and(|min| amount.is_none_or(|v| v < min))
            {
                continue;
            }
            let day = transaction_day(row.get(date).unwrap_or(&Value::Null))?;
            if !matches!(self.window, Window::All)
                && !day.is_some_and(|day| {
                    cutoff.is_some_and(|lower| day > lower) && (!bounded || day <= reference)
                })
            {
                continue;
            }
            let customer = row.get(id).cloned().unwrap_or(Value::Null);
            let valid_id = match &customer {
                Value::Null => true,
                Value::String(_) => id_field.storage == Storage::String,
                Value::Integer(_) => matches!(id_field.storage, Storage::Integer | Storage::Real),
                Value::Real(v) => id_field.storage == Storage::Real && v.is_finite(),
                _ => false,
            };
            if !valid_id {
                return Err(OperatorError::Data(
                    "RFM Aggregate: nonfinite ID or value inconsistent with ID storage".into(),
                ));
            }
            let slot = if self.contiguous {
                if groups
                    .last()
                    .and_then(|g| g.id.as_ref())
                    .is_some_and(|v| v.total_cmp(&customer).is_eq())
                {
                    groups.len().saturating_sub(1)
                } else {
                    if seen
                        .insert(GroupKey(vec![customer.clone()]), groups.len())
                        .is_some()
                    {
                        return Err(bad(
                            "unsupported repeated noncontiguous ID with ContiguousIds",
                        ));
                    }
                    groups.len()
                }
            } else {
                *seen
                    .entry(GroupKey(vec![customer.clone()]))
                    .or_insert(groups.len())
            };
            if slot == groups.len() {
                groups.push(Group {
                    id: Some(customer),
                    ..Group::default()
                });
            }
            let group = groups
                .get_mut(slot)
                .ok_or_else(|| OperatorError::Data("RFM Aggregate: invalid group".into()))?;
            group.count = group
                .count
                .checked_add(1)
                .ok_or_else(|| OperatorError::Data("RFM Aggregate: frequency overflow".into()))?;
            if let Some(amount) = amount {
                let sum = group.sum.unwrap_or(0.0) + amount;
                if !sum.is_finite() {
                    return Err(OperatorError::Data(
                        "RFM Aggregate: monetary overflow".into(),
                    ));
                }
                group.sum = Some(sum);
            }
            if let Some(day) = day {
                // Only three dates are needed, including repeats; never retain a customer's history.
                group.dates.push(day);
                group.dates.sort_unstable_by(|a, b| b.cmp(a));
                group.dates.truncate(3);
            }
        }
        let mut fields = vec![id_field.clone()];
        fields.extend(self.names.iter().enumerate().map(|(i, name)| {
            Field::new(
                name,
                if i == 2 {
                    Storage::Real
                } else {
                    Storage::Integer
                },
            )
        }));
        let output_model = DataModel::new(fields).map_err(|e| bad(e.to_string()))?;
        let mut rows = Vec::with_capacity(groups.len());
        for group in groups {
            let recency = |index| -> Result<Value, OperatorError> {
                group
                    .dates
                    .get(index)
                    .map(|day| {
                        reference
                            .checked_sub(*day)
                            .map(Value::Integer)
                            .ok_or_else(|| {
                                OperatorError::Data("RFM Aggregate: recency overflow".into())
                            })
                    })
                    .transpose()
                    .map(|v| v.unwrap_or(Value::Null))
            };
            let mut row = vec![
                group.id.clone().unwrap_or(Value::Null),
                recency(0)?,
                Value::Integer(group.count),
                group.sum.map_or(Value::Null, Value::Real),
            ];
            for index in 1..self.names.len().saturating_sub(2) {
                row.push(recency(index)?);
            }
            rows.push(row);
        }
        Table::new(output_model, rows).map_err(|e| OperatorError::Data(e.to_string()))
    }
}

fn transaction_day(value: &Value) -> Result<Option<i64>, OperatorError> {
    let day = match value {
        Value::Null => return Ok(None),
        Value::Date(day) => *day,
        Value::Timestamp(micros) => micros.div_euclid(MICROS_PER_DAY),
        _ => {
            return Err(OperatorError::Data(
                "RFM Aggregate: invalid transaction date".into(),
            ));
        }
    };
    if civil_from_days(day).is_none() {
        return Err(OperatorError::Data(
            "RFM Aggregate: transaction date out of range".into(),
        ));
    }
    Ok(Some(day))
}

fn relative_day(reference: i64, count: i64, scale: &str) -> Option<i64> {
    match scale {
        "Days" => reference.checked_sub(count),
        "Weeks" => reference.checked_sub(count.checked_mul(7)?),
        "Months" | "Years" => {
            let (year, month, day) = civil_from_days(reference)?;
            let months = year
                .checked_mul(12)?
                .checked_add(i64::from(month).checked_sub(1)?)?
                .checked_sub(count.checked_mul(if scale == "Years" { 12 } else { 1 })?)?;
            days_from_civil(
                months.div_euclid(12),
                u32::try_from(months.rem_euclid(12).checked_add(1)?).ok()?,
                day,
            )
        }
        _ => None,
    }
}

fn required_integer(s: Option<&Element>, key: &str) -> Result<i64, OperatorError> {
    settings::integer(s, &format!("{PREFIX}{key}"))?.ok_or_else(|| bad(format!("missing {key}")))
}

fn bad(message: impl Into<String>) -> OperatorError {
    OperatorError::Settings(format!("RFM Aggregate: {}", message.into()))
}
