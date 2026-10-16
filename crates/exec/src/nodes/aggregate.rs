//! Aggregate (Record Ops): collapses records into one record per key group.
//!
//! Settings: `aggregateKeyFields/field[n]`, `aggregateContiguousKeys` (a new group starts
//! whenever the key changes between neighbouring records instead of grouping equal keys
//! anywhere), `aggregateFields/rows/row[i]/{field,sum,mean,min,max,stddev,median,count,variance,
//! firstQuartile,thirdQuartile}` (one flag per statistic), `aggregateNameExtension` (optional
//! custom extension added to every aggregated field name) and `aggregateExtensionPosition`
//! (`Suffix` default | `Prefix`), `aggregateIncludeCount` (default true) and
//! `aggregateCountFieldName` (default `Record_Count`). Evidence: `docs/formats/nodes/aggregate.md`
//! and oracle `team/stream_2_M` (key `ID`, `ACCOUNT_CURRENT`/`ACCOUNT_SAVE` max -> columns
//! `ACCOUNT_CURRENT_Max`, `ACCOUNT_SAVE_Max`, then `No_accounts`).
//!
//! Output fields: the key fields, then per aggregate row its statistics (order Sum, Mean, Min,
//! Max, SDev, Median, Count, Var, Q1, Q3), then the record count. Nulls are ignored by every
//! statistic; a statistic over no valid value is null.
//!
//! Not supported (`build` fails): `aggregateExpressionsTable` (expression aggregates),
//! `aggregateAllowApproximation`.
//!
//! GAP (unverified against Modeler): the suffixes of Sum, Mean, SDev, Median, Count, Var, Q1, Q3
//! (`_Max` is confirmed by `team/stream_2_M`) and how `aggregateNameExtension` combines with them;
//! the group order when `aggregateContiguousKeys` is false (we sort groups by key, which matches
//! the oracle stream); the sample (n-1) formulas for SDev and Var and null for fewer than two
//! values; Median of an even count is the mean of the two middle values; quartiles use position
//! p(n+1) with linear interpolation; Count counts valid values; an empty input without key fields
//! emits one summary row with count 0 (with key fields it emits no rows).

use super::field_util::extended_name;
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Field, Storage, Table, Value};
use std::cmp::Ordering;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stat {
    Sum,
    Mean,
    Min,
    Max,
    SDev,
    Median,
    Count,
    Var,
    Q1,
    Q3,
}

const STATS: [(Stat, &str, &str); 10] = [
    (Stat::Sum, "sum", "_Sum"),
    (Stat::Mean, "mean", "_Mean"),
    (Stat::Min, "min", "_Min"),
    (Stat::Max, "max", "_Max"),
    (Stat::SDev, "stddev", "_SDev"),
    (Stat::Median, "median", "_Median"),
    (Stat::Count, "count", "_Count"),
    (Stat::Var, "variance", "_Var"),
    (Stat::Q1, "firstQuartile", "_Q1"),
    (Stat::Q3, "thirdQuartile", "_Q3"),
];

struct Aggregate {
    keys: Vec<String>,
    contiguous: bool,
    fields: Vec<(String, Vec<Stat>)>,
    extension: String,
    prefix: bool,
    count_field: Option<String>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    if !settings::children(s, "aggregateExpressionsTable/rows", "row").is_empty() {
        return Err(OperatorError::Settings(
            "aggregateExpressionsTable not supported yet".into(),
        ));
    }
    if settings::flag(s, "aggregateAllowApproximation")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "aggregateAllowApproximation not supported yet".into(),
        ));
    }
    let extension = settings::text(s, "aggregateNameExtension").unwrap_or_default();
    let prefix = match settings::text(s, "aggregateExtensionPosition")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("Suffix") | Some("suffix") => false,
        Some("Prefix") | Some("prefix") => true,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "aggregateExtensionPosition: expected Prefix or Suffix, found {other:?}"
            )));
        }
    };
    let keys: Vec<String> = settings::children(s, "aggregateKeyFields", "field")
        .into_iter()
        .map(|e| e.text())
        .collect();
    if keys.iter().any(|k| k.trim().is_empty()) {
        return Err(OperatorError::Settings(
            "aggregateKeyFields: empty field name".into(),
        ));
    }
    let contiguous = settings::flag(s, "aggregateContiguousKeys")?.unwrap_or(false);
    let mut fields = Vec::new();
    for row in settings::children(s, "aggregateFields/rows", "row") {
        let name = row.child("field").map(|e| e.text()).unwrap_or_default();
        if name.trim().is_empty() {
            return Err(OperatorError::Settings(
                "aggregateFields: row without field name".into(),
            ));
        }
        let mut stats = Vec::new();
        for (stat, element, _) in STATS {
            let on = match row.child(element).map(|e| e.text()) {
                None => false,
                Some(text) => match text.trim() {
                    "true" => true,
                    "false" | "" => false,
                    other => {
                        return Err(OperatorError::Settings(format!(
                            "aggregateFields {name}.{element}: expected true or false, found {other:?}"
                        )));
                    }
                },
            };
            if on {
                stats.push(stat);
            }
        }
        if !stats.is_empty() {
            fields.push((name, stats));
        }
    }
    let count_field = if settings::flag(s, "aggregateIncludeCount")?.unwrap_or(true) {
        Some(
            settings::text(s, "aggregateCountFieldName")
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| "Record_Count".to_string()),
        )
    } else {
        None
    };
    Ok(Box::new(Aggregate {
        keys,
        contiguous,
        fields,
        extension,
        prefix,
        count_field,
    }))
}

impl Operator for Aggregate {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Aggregate needs one input".into()))?;
        let model = input.model();
        let rows = input.rows();
        let key_positions: Vec<(usize, bool)> = self
            .keys
            .iter()
            .map(|k| {
                model
                    .position(k)
                    .map(|p| (p, false))
                    .ok_or_else(|| OperatorError::Data(format!("unknown key field {k:?}")))
            })
            .collect::<Result<_, _>>()?;
        let mut value_positions = Vec::new();
        for (name, stats) in &self.fields {
            let position = model
                .position(name)
                .ok_or_else(|| OperatorError::Data(format!("unknown field {name:?}")))?;
            value_positions.push((position, name, stats));
        }

        // Groups as lists of row indexes.
        let mut order: Vec<usize> = (0..rows.len()).collect();
        let mut groups: Vec<Vec<usize>> = Vec::new();
        if !self.contiguous && !key_positions.is_empty() {
            order.sort_by(|&a, &b| match (rows.get(a), rows.get(b)) {
                (Some(x), Some(y)) => super::sort::compare_rows(&key_positions, x, y),
                _ => Ordering::Equal,
            });
        }
        let same_key = |a: usize, b: usize| match (rows.get(a), rows.get(b)) {
            (Some(x), Some(y)) => {
                super::sort::compare_rows(&key_positions, x, y) == Ordering::Equal
            }
            _ => false,
        };
        for &index in &order {
            match groups.last_mut() {
                Some(group) if group.last().is_some_and(|&last| same_key(last, index)) => {
                    group.push(index);
                }
                _ => groups.push(vec![index]),
            }
        }
        if groups.is_empty() && key_positions.is_empty() {
            groups.push(Vec::new());
        }

        // Output model.
        let mut fields: Vec<Field> = Vec::new();
        for &(position, _) in &key_positions {
            if let Some(field) = model.fields().get(position) {
                fields.push(field.clone());
            }
        }
        for &(position, name, stats) in &value_positions {
            let storage = model
                .fields()
                .get(position)
                .map_or(Storage::Unknown, |f| f.storage);
            let base = if self.extension.is_empty() {
                name.clone()
            } else {
                extended_name(name, &self.extension, self.prefix)
            };
            for &stat in stats {
                let suffix = STATS
                    .iter()
                    .find(|(s, _, _)| *s == stat)
                    .map_or("", |(_, _, suffix)| *suffix);
                fields.push(Field::new(
                    format!("{base}{suffix}"),
                    stat_storage(stat, storage),
                ));
            }
        }
        if let Some(name) = &self.count_field {
            fields.push(Field::new(name.clone(), Storage::Integer));
        }
        let out_model = dms_types::DataModel::new(fields)
            .map_err(|error| OperatorError::Data(error.to_string()))?;

        let mut out_rows = Vec::with_capacity(groups.len());
        for group in &groups {
            let mut out = Vec::with_capacity(out_model.len());
            let first = group.first().and_then(|&i| rows.get(i));
            for &(position, _) in &key_positions {
                out.push(
                    first
                        .and_then(|row| row.get(position))
                        .cloned()
                        .unwrap_or(Value::Null),
                );
            }
            for &(position, name, stats) in &value_positions {
                let values: Vec<&Value> = group
                    .iter()
                    .filter_map(|&i| rows.get(i).and_then(|row| row.get(position)))
                    .filter(|v| !v.is_null())
                    .collect();
                for &stat in stats {
                    out.push(compute(stat, &values, name)?);
                }
            }
            if self.count_field.is_some() {
                out.push(Value::Integer(
                    i64::try_from(group.len()).unwrap_or(i64::MAX),
                ));
            }
            out_rows.push(out);
        }
        Table::new(out_model, out_rows).map_err(|error| OperatorError::Data(error.to_string()))
    }
}

fn stat_storage(stat: Stat, storage: Storage) -> Storage {
    match stat {
        Stat::Count => Storage::Integer,
        Stat::Min | Stat::Max => storage,
        Stat::Sum if storage == Storage::Integer => Storage::Integer,
        _ => Storage::Real,
    }
}

fn compute(stat: Stat, values: &[&Value], field: &str) -> Result<Value, OperatorError> {
    match stat {
        Stat::Count => {
            return Ok(Value::Integer(
                i64::try_from(values.len()).unwrap_or(i64::MAX),
            ));
        }
        Stat::Min | Stat::Max => {
            let best = values.iter().copied().reduce(|a, b| {
                let ordering = a.total_cmp(b);
                if (stat == Stat::Min) == (ordering != Ordering::Greater) {
                    a
                } else {
                    b
                }
            });
            return Ok(best.cloned().unwrap_or(Value::Null));
        }
        _ => {}
    }
    let mut numbers = Vec::with_capacity(values.len());
    for value in values {
        numbers.push(value.as_f64().ok_or_else(|| {
            OperatorError::Data(format!("field {field:?} is not numeric; cannot aggregate"))
        })?);
    }
    if numbers.is_empty() {
        return Ok(Value::Null);
    }
    let n = numbers.len() as f64;
    let sum: f64 = numbers.iter().sum();
    Ok(match stat {
        Stat::Sum => {
            if values.iter().all(|v| matches!(v, Value::Integer(_))) {
                let total = values.iter().try_fold(0i64, |acc, v| match v {
                    Value::Integer(i) => acc.checked_add(*i),
                    _ => None,
                });
                total.map_or(Value::real(sum), Value::Integer)
            } else {
                Value::real(sum)
            }
        }
        Stat::Mean => Value::real(sum / n),
        Stat::Var | Stat::SDev => {
            if numbers.len() < 2 {
                Value::Null
            } else {
                let mean = sum / n;
                let var = numbers.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
                Value::real(if stat == Stat::SDev { var.sqrt() } else { var })
            }
        }
        Stat::Median | Stat::Q1 | Stat::Q3 => {
            numbers.sort_by(f64::total_cmp);
            let p = match stat {
                Stat::Q1 => 0.25,
                Stat::Q3 => 0.75,
                _ => 0.5,
            };
            Value::real(percentile(&numbers, p))
        }
        Stat::Count | Stat::Min | Stat::Max => Value::Null,
    })
}

/// Weighted-average percentile at position p(n+1) on sorted data.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    let at = |i: usize| sorted.get(i).copied().unwrap_or(f64::NAN);
    let position = p * (n as f64 + 1.0);
    if position < 1.0 {
        return at(0);
    }
    if position >= n as f64 {
        return at(n.saturating_sub(1));
    }
    let lower = position.floor();
    let fraction = position - lower;
    let i = lower as usize - 1;
    at(i) + fraction * (at(i + 1) - at(i))
}
