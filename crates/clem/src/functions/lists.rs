//! Multi-field and list CLEM functions (`[f1, f2, ...]`).

use super::{Args, Function, Returns, compare_values, f, integer};
use crate::types::Ty;
use crate::{ClemError, Datum};
use dms_types::Value;
use std::cmp::Ordering;
use std::sync::Arc;

pub(super) const FUNCTIONS: &[Function] = &[
    // member(ITEM, LIST)
    f("member", 2, |a| {
        let (Some(item), Some(Datum::Value(Value::List(list)))) = (a.first(), a.get(1)) else {
            return Err(ClemError::Type("member needs an item and a list".into()));
        };
        if item.is_null() {
            return Ok(Datum::NULL);
        }
        let item = item.clone().into_value()?;
        Ok(Datum::Bool(list.iter().any(|v| v.total_cmp(&item).is_eq())))
    })
    .returns(Returns::Fixed(Ty::Bool))
    .args(Args::Uniform(1)),
    f("sum_n", 1, sum_n)
        .returns(Returns::Items(0))
        .args(Args::Fields(0)),
    f("mean_n", 1, mean_n)
        .returns(Returns::Fixed(Ty::Real))
        .args(Args::Fields(0)),
    // Oracle rule 30: sample standard deviation (N - 1 denominator).
    f("sdev_n", 1, sdev_n)
        .returns(Returns::Fixed(Ty::Real))
        .args(Args::Fields(0)),
    f("min_n", 1, |a| min_max_n(a, false))
        .returns(Returns::Items(0))
        .args(Args::Fields(0)),
    f("max_n", 1, |a| min_max_n(a, true))
        .returns(Returns::Items(0))
        .args(Args::Fields(0)),
    f("count_nulls", 1, |a| {
        let list = list_arg(a, 0)?;
        let count = list.iter().filter(|v| matches!(v, Value::Null)).count();
        Ok(Datum::Value(Value::Integer(
            i64::try_from(count).unwrap_or(i64::MAX),
        )))
    })
    .returns(Returns::Fixed(Ty::Integer))
    .args(Args::Fields(0)),
    f("count_non_nulls", 1, |a| {
        let list = list_arg(a, 0)?;
        let count = list.iter().filter(|v| !matches!(v, Value::Null)).count();
        Ok(Datum::Value(Value::Integer(
            i64::try_from(count).unwrap_or(i64::MAX),
        )))
    })
    .returns(Returns::Fixed(Ty::Integer))
    .args(Args::Fields(0)),
    f("first_non_null", 1, |a| {
        let list = list_arg(a, 0)?;
        Ok(Datum::Value(
            list.iter()
                .find(|v| !matches!(v, Value::Null))
                .cloned()
                .unwrap_or(Value::Null),
        ))
    })
    .returns(Returns::Items(0))
    .args(Args::Fields(0)),
    f("last_non_null", 1, |a| {
        let list = list_arg(a, 0)?;
        Ok(Datum::Value(
            list.iter()
                .rev()
                .find(|v| !matches!(v, Value::Null))
                .cloned()
                .unwrap_or(Value::Null),
        ))
    })
    .returns(Returns::Items(0))
    .args(Args::Fields(0)),
    // Oracle rule 30: 1-based index, or 0 (not null) when all elements are null.
    f("first_non_null_index", 1, |a| {
        let list = list_arg(a, 0)?;
        let idx = list
            .iter()
            .position(|v| !matches!(v, Value::Null))
            .map_or(0, |i| i64::try_from(i).unwrap_or(0).saturating_add(1));
        Ok(Datum::Value(Value::Integer(idx)))
    })
    .returns(Returns::Fixed(Ty::Integer))
    .args(Args::Fields(0)),
    f("last_non_null_index", 1, |a| {
        let list = list_arg(a, 0)?;
        let idx = list
            .iter()
            .rposition(|v| !matches!(v, Value::Null))
            .map_or(0, |i| i64::try_from(i).unwrap_or(0).saturating_add(1));
        Ok(Datum::Value(Value::Integer(idx)))
    })
    .returns(Returns::Fixed(Ty::Integer))
    .args(Args::Fields(0)),
    // Oracle rule 30: 1-based index (leftmost on ties); 0 when all elements are null.
    f("max_index", 1, |a| extremum_index(a, Ordering::Greater))
        .returns(Returns::Fixed(Ty::Integer))
        .args(Args::Fields(0)),
    f("min_index", 1, |a| extremum_index(a, Ordering::Less))
        .returns(Returns::Fixed(Ty::Integer))
        .args(Args::Fields(0)),
    // Oracle rule 30: 1-based index into list; out of 1..=len returns null.
    f("value_at", 2, |a| {
        let Some(index) = a.first() else {
            return Err(ClemError::Type("value_at needs an index and a list".into()));
        };
        let list = list_arg(a, 1)?;
        if index.is_null() {
            return Ok(Datum::NULL);
        }
        let idx = integer(index)?;
        if idx < 1 {
            return Ok(Datum::NULL);
        }
        let pos = usize::try_from(idx - 1).ok();
        Ok(Datum::Value(
            pos.and_then(|i| list.get(i).cloned())
                .unwrap_or(Value::Null),
        ))
    })
    .returns(Returns::Items(1))
    .args(Args::Fields(1)),
    // Oracle rule 30: null if ITEM1 is null; null elements in LIST do not match.
    f("count_equal", 2, |a| {
        count_cmp(a, |ord| ord == Ordering::Equal)
    })
    .returns(Returns::Fixed(Ty::Integer))
    .args(Args::Fields(1)),
    f("count_not_equal", 2, |a| {
        count_cmp(a, |ord| ord != Ordering::Equal)
    })
    .returns(Returns::Fixed(Ty::Integer))
    .args(Args::Fields(1)),
    f("count_greater_than", 2, |a| {
        count_cmp(a, |ord| ord == Ordering::Greater)
    })
    .returns(Returns::Fixed(Ty::Integer))
    .args(Args::Fields(1)),
    f("count_less_than", 2, |a| {
        count_cmp(a, |ord| ord == Ordering::Less)
    })
    .returns(Returns::Fixed(Ty::Integer))
    .args(Args::Fields(1)),
    f("first_index", 2, |a| find_index(a, false))
        .returns(Returns::Fixed(Ty::Integer))
        .args(Args::Fields(1)),
    f("last_index", 2, |a| find_index(a, true))
        .returns(Returns::Fixed(Ty::Integer))
        .args(Args::Fields(1)),
];

fn list_arg(args: &[Datum], pos: usize) -> Result<&Arc<[Value]>, ClemError> {
    match args.get(pos) {
        Some(Datum::Value(Value::List(list))) => Ok(list),
        other => Err(ClemError::Type(format!("expected a list, found {other:?}"))),
    }
}

fn sum_n(args: &[Datum]) -> Result<Datum, ClemError> {
    let list = list_arg(args, 0)?;
    let mut int_sum: i64 = 0;
    let mut real_sum: f64 = 0.0;
    let mut any_non_null = false;
    let mut any_real = false;
    for value in list.iter() {
        match value {
            Value::Null => {}
            Value::Integer(i) => {
                any_non_null = true;
                int_sum = int_sum.wrapping_add(*i);
                real_sum += *i as f64;
            }
            Value::Real(r) => {
                any_non_null = true;
                any_real = true;
                real_sum += *r;
            }
            other => {
                return Err(ClemError::Type(format!(
                    "expected a number in list, found {other:?}"
                )));
            }
        }
    }
    if !any_non_null {
        Ok(Datum::NULL)
    } else if any_real {
        Ok(Datum::Value(Value::real(real_sum)))
    } else {
        Ok(Datum::Value(Value::Integer(int_sum)))
    }
}

fn mean_n(args: &[Datum]) -> Result<Datum, ClemError> {
    let list = list_arg(args, 0)?;
    let mut sum = 0.0;
    let mut count: usize = 0;
    for value in list.iter() {
        match value {
            Value::Null => {}
            Value::Integer(i) => {
                sum += *i as f64;
                count = count.saturating_add(1);
            }
            Value::Real(r) => {
                sum += *r;
                count = count.saturating_add(1);
            }
            other => {
                return Err(ClemError::Type(format!(
                    "expected a number in list, found {other:?}"
                )));
            }
        }
    }
    if count == 0 {
        Ok(Datum::NULL)
    } else {
        Ok(Datum::Value(Value::real(sum / (count as f64))))
    }
}

fn sdev_n(args: &[Datum]) -> Result<Datum, ClemError> {
    let list = list_arg(args, 0)?;
    let mut numbers = Vec::new();
    for value in list.iter() {
        match value {
            Value::Null => {}
            Value::Integer(i) => numbers.push(*i as f64),
            Value::Real(r) => numbers.push(*r),
            other => {
                return Err(ClemError::Type(format!(
                    "expected a number in list, found {other:?}"
                )));
            }
        }
    }
    // GAP: sample standard deviation needs at least 2 non-null values; < 2 returns null.
    if numbers.len() < 2 {
        return Ok(Datum::NULL);
    }
    let n = numbers.len() as f64;
    let mean = numbers.iter().sum::<f64>() / n;
    let var = numbers
        .iter()
        .map(|x| {
            let d = *x - mean;
            d * d
        })
        .sum::<f64>()
        / (n - 1.0);
    Ok(Datum::Value(Value::real(var.sqrt())))
}

fn min_max_n(args: &[Datum], pick_max: bool) -> Result<Datum, ClemError> {
    let list = list_arg(args, 0)?;
    let mut any_real = false;
    let mut best_int: Option<i64> = None;
    let mut best_real: Option<f64> = None;
    let better_int = |a: i64, b: i64| if pick_max { a > b } else { a < b };
    let better_real = |a: f64, b: f64| {
        if pick_max {
            a.total_cmp(&b) == Ordering::Greater
        } else {
            a.total_cmp(&b) == Ordering::Less
        }
    };
    for value in list.iter() {
        match value {
            Value::Null => {}
            Value::Integer(i) => {
                best_int = Some(match best_int {
                    Some(prev) if !better_int(*i, prev) => prev,
                    _ => *i,
                });
                let r = *i as f64;
                best_real = Some(match best_real {
                    Some(prev) if !better_real(r, prev) => prev,
                    _ => r,
                });
            }
            Value::Real(r) => {
                any_real = true;
                best_real = Some(match best_real {
                    Some(prev) if !better_real(*r, prev) => prev,
                    _ => *r,
                });
            }
            other => {
                return Err(ClemError::Type(format!(
                    "expected a number in list, found {other:?}"
                )));
            }
        }
    }
    match (any_real, best_int, best_real) {
        (true, _, Some(r)) => Ok(Datum::Value(Value::real(r))),
        (false, Some(i), _) => Ok(Datum::Value(Value::Integer(i))),
        _ => Ok(Datum::NULL),
    }
}

fn extremum_index(args: &[Datum], wanted: Ordering) -> Result<Datum, ClemError> {
    let list = list_arg(args, 0)?;
    let mut best: Option<(usize, &Value)> = None;
    for (idx, value) in list.iter().enumerate() {
        if matches!(value, Value::Null) {
            continue;
        }
        match best {
            None => best = Some((idx, value)),
            Some((_, prev)) => {
                // Strictly better updates; ties keep the leftmost index.
                if compare_values(value, prev)? == wanted {
                    best = Some((idx, value));
                }
            }
        }
    }
    let result = best.map_or(0, |(idx, _)| {
        i64::try_from(idx).unwrap_or(0).saturating_add(1)
    });
    Ok(Datum::Value(Value::Integer(result)))
}

fn count_cmp(args: &[Datum], predicate: fn(Ordering) -> bool) -> Result<Datum, ClemError> {
    let Some(target) = args.first() else {
        return Err(ClemError::Type("missing item argument".into()));
    };
    let list = list_arg(args, 1)?;
    if target.is_null() {
        return Ok(Datum::NULL);
    }
    let target = target.clone().into_value()?;
    let mut count: i64 = 0;
    for value in list.iter() {
        if matches!(value, Value::Null) {
            continue;
        }
        if predicate(compare_values(value, &target)?) {
            count = count.saturating_add(1);
        }
    }
    Ok(Datum::Value(Value::Integer(count)))
}

fn find_index(args: &[Datum], from_end: bool) -> Result<Datum, ClemError> {
    let Some(target) = args.first() else {
        return Err(ClemError::Type("missing item argument".into()));
    };
    let list = list_arg(args, 1)?;
    if target.is_null() {
        return Ok(Datum::NULL);
    }
    let target = target.clone().into_value()?;
    let mut found: i64 = 0;
    for (idx, value) in list.iter().enumerate() {
        if matches!(value, Value::Null) {
            continue;
        }
        if compare_values(value, &target)? == Ordering::Equal {
            found = i64::try_from(idx).unwrap_or(0).saturating_add(1);
            if !from_end {
                break;
            }
        }
    }
    Ok(Datum::Value(Value::Integer(found)))
}
