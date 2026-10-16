//! Aggregate (Record Ops): collapses records into one record per key group.
//!
//! Settings: `aggregateKeyFields/field[n]`, `aggregateContiguousKeys` (a new group starts
//! whenever the key changes between neighbouring records instead of grouping equal keys
//! anywhere), `aggregateFields/rows/row[i]/{field,sum,mean,min,max,stddev,median,count,variance,
//! firstQuartile,thirdQuartile}` (one flag per statistic), `aggregateNameExtension` (optional
//! custom extension added to every aggregated field name) and `aggregateExtensionPosition`
//! (`Suffix` default | `Prefix`), `aggregateIncludeCount` (default true) and
//! `aggregateCountFieldName` (default `Record_Count`). Evidence: `docs/formats/nodes/aggregate.md`
//! and `oracle/team/stream_2_M`.
//!
//! Rules (settled by `oracle/records/agg_all_stats`, `agg_ext_prefix`, `agg_ext_suffix`,
//! `agg_no_count`, `agg_contiguous`, `agg_no_keys`, and `oracle/team/stream_2_M`):
//! - Output fields: the key fields, then per aggregate row its statistics in the fixed order
//!   `_Sum`, `_Mean`, `_Min`, `_Max`, `_SDev`, `_Median`, `_Count`, `_Variance`,
//!   `_FirstQuartile`, `_ThirdQuartile` (`agg_all_stats`, `agg_no_keys`), then the record count
//!   field when `aggregateIncludeCount` is true (`agg_no_count` omits it).
//! - When `aggregateNameExtension` is non-empty, Modeler joins the extension to `{field}_{Stat}`
//!   with `_`: `{ext}_{field}_{Stat}` when `aggregateExtensionPosition=Prefix` (`agg_ext_prefix`:
//!   extension `PRE_` gives `PRE__iv_Sum`), and `{field}_{Stat}_{ext}` when `Suffix`
//!   (`agg_ext_suffix`: extension `_SUF` gives `iv_Sum__SUF`).
//! - When `aggregateContiguousKeys` is false (default) and key fields are present, groups are
//!   emitted in order of their first appearance in the input (`agg_all_stats`, `agg_ext_prefix`,
//!   `agg_ext_suffix`, `agg_no_count`, `team/stream_2_M`); key equality is exact (`"B"`, `"A"`,
//!   `"C"`, `""`, `"D"`). When `aggregateContiguousKeys` is true, a new group starts whenever a
//!   key value differs from the preceding record (`agg_contiguous`). With no key fields, all
//!   records collapse into a single summary row (`agg_no_keys`).
//! - Nulls are ignored by every statistic; `Count` counts non-null values (0 when all values in
//!   the group are null), while `Record_Count` counts all records in the group (`agg_all_stats`
//!   group `"D"`). Any other statistic over zero valid values is null.
//! - `Variance` and `SDev` use the sample denominator `n - 1` and return null when `n < 2`
//!   (`agg_all_stats` group `"C"` has `n = 1`).
//! - `Median` of an even count is the mean of the two middle values (`agg_all_stats` group `"A"`
//!   `rv` `[1.25, 2.5, 5.0, 10.0]` -> `3.75`).
//! - `FirstQuartile` and `ThirdQuartile` are Tukey hinges: the median of the lower half and upper
//!   half of the sorted non-null values, including the median element in both halves when `n` is
//!   odd (`agg_all_stats`, `agg_ext_prefix`, `agg_no_keys`: for `n = 13` sorted values, Q1 is row
//!   index 3 (`7.0` / `0.5`) and Q3 is row index 9 (`30.0` / `5.0`); for `n = 6` `rv` in group
//!   `"B"`, Q1 is `0.0` and Q3 is `9.0`; for `n = 4` `rv` in group `"A"`, Q1 is `1.875` and Q3 is
//!   `7.5`; for `n = 1`, Q1 = Q3 = the single value).
//!
//! Not supported (`build` fails): `aggregateExpressionsTable` (expression aggregates),
//! `aggregateAllowApproximation`.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Field, Storage, Table, Value};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;

/// A key of the non-contiguous grouping, ordered by `Value::total_cmp`.
struct GroupKey(Vec<Value>);

impl PartialEq for GroupKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for GroupKey {}

impl PartialOrd for GroupKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GroupKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .iter()
            .zip(&other.0)
            .map(|(a, b)| a.total_cmp(b))
            .find(|ordering| ordering.is_ne())
            .unwrap_or_else(|| self.0.len().cmp(&other.0.len()))
    }
}

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
    (Stat::Var, "variance", "_Variance"),
    (Stat::Q1, "firstQuartile", "_FirstQuartile"),
    (Stat::Q3, "thirdQuartile", "_ThirdQuartile"),
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
        let key_positions: Vec<usize> = self
            .keys
            .iter()
            .map(|k| {
                model
                    .position(k)
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

        // Groups as lists of row indexes, preserving first-appearance order.
        let same_key = |a: usize, b: usize| match (rows.get(a), rows.get(b)) {
            (Some(x), Some(y)) => key_positions.iter().all(|&p| {
                let vx = x.get(p).unwrap_or(&Value::Null);
                let vy = y.get(p).unwrap_or(&Value::Null);
                vx.total_cmp(vy) == Ordering::Equal
            }),
            _ => false,
        };
        let mut groups: Vec<Vec<usize>> = Vec::new();
        // Group of each key seen so far (non-contiguous mode).
        let mut group_of: BTreeMap<GroupKey, usize> = BTreeMap::new();
        for index in 0..rows.len() {
            if key_positions.is_empty() {
                if let Some(group) = groups.first_mut() {
                    group.push(index);
                } else {
                    groups.push(vec![index]);
                }
            } else if self.contiguous {
                match groups.last_mut() {
                    Some(group) if group.last().is_some_and(|&last| same_key(last, index)) => {
                        group.push(index);
                    }
                    _ => groups.push(vec![index]),
                }
            } else {
                let key = GroupKey(
                    key_positions
                        .iter()
                        .map(|&p| {
                            rows.get(index)
                                .and_then(|row| row.get(p))
                                .cloned()
                                .unwrap_or(Value::Null)
                        })
                        .collect(),
                );
                let next = groups.len();
                let slot = *group_of.entry(key).or_insert(next);
                match groups.get_mut(slot) {
                    Some(group) => group.push(index),
                    None => groups.push(vec![index]),
                }
            }
        }
        if groups.is_empty() && key_positions.is_empty() {
            groups.push(Vec::new());
        }

        // Output model.
        let mut fields: Vec<Field> = Vec::new();
        for &position in &key_positions {
            if let Some(field) = model.fields().get(position) {
                fields.push(field.clone());
            }
        }
        for &(position, name, stats) in &value_positions {
            let storage = model
                .fields()
                .get(position)
                .map_or(Storage::Unknown, |f| f.storage);
            for &stat in stats {
                let suffix = STATS
                    .iter()
                    .find(|(s, _, _)| *s == stat)
                    .map_or("", |(_, _, suffix)| *suffix);
                let default_name = format!("{name}{suffix}");
                let col_name = if self.extension.is_empty() {
                    default_name
                } else if self.prefix {
                    format!("{}_{default_name}", self.extension)
                } else {
                    format!("{default_name}_{}", self.extension)
                };
                fields.push(Field::new(col_name, stat_storage(stat, storage)));
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
            for &position in &key_positions {
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
            let len = numbers.len();
            let value = match stat {
                Stat::Median => median_sorted(&numbers),
                Stat::Q1 => {
                    let half = len.div_ceil(2);
                    median_sorted(numbers.get(..half).unwrap_or(&numbers))
                }
                _ => {
                    let start = len / 2;
                    median_sorted(numbers.get(start..).unwrap_or(&numbers))
                }
            };
            Value::real(value)
        }
        Stat::Count | Stat::Min | Stat::Max => Value::Null,
    })
}

/// Median of a non-empty sorted slice (mean of the two middle elements when `len` is even).
fn median_sorted(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    let at = |i: usize| sorted.get(i).copied().unwrap_or(f64::NAN);
    if n == 0 {
        return f64::NAN;
    }
    let mid = n / 2;
    if n.is_multiple_of(2) {
        (at(mid - 1) + at(mid)) / 2.0
    } else {
        at(mid)
    }
}
