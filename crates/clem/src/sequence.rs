//! Sequence functions over the records of a table, in record order (docs/clem/oracle-results.md
//! rules 59-62). Each function here turns the per-record values of its argument into one result
//! per record; `eval` evaluates the arguments.

use crate::{ClemError, Datum};
use dms_types::{MissingSpec, Value};

/// The running statistics `@MEAN`/`@AVE`, `@SUM`, `@MIN`, `@MAX` and `@SDEV`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Statistic {
    Mean,
    Sum,
    Min,
    Max,
    Sdev,
}

/// A statistic of `values` over all records so far, or over the last `window[i]` records
/// (the current one included) when a window is given. Nulls are skipped; a statistic over no
/// value is null (oracle: `@MEAN(a, 1)` on a null `a`). `@SDEV` needs two values cumulatively
/// but gives 0 for one value in a window (oracle rule 61).
pub fn statistic(
    statistic: Statistic,
    values: &[Datum],
    windows: Option<&[Datum]>,
) -> Result<Vec<Datum>, ClemError> {
    let mut running = Running::default();
    let mut out = Vec::with_capacity(values.len());
    for (i, value) in values.iter().enumerate() {
        match windows {
            None => {
                running.add(statistic, value)?;
                out.push(running.result(statistic, false));
            }
            Some(windows) => {
                let Some(size) = window_size(windows.get(i))? else {
                    out.push(Datum::NULL);
                    continue;
                };
                let start = (i + 1).saturating_sub(size);
                let mut window = Running::default();
                for value in values.get(start..=i).unwrap_or_default() {
                    window.add(statistic, value)?;
                }
                out.push(window.result(statistic, true));
            }
        }
    }
    Ok(out)
}

/// GAP: a null or non-positive window gives null.
fn window_size(window: Option<&Datum>) -> Result<Option<usize>, ClemError> {
    match window {
        None | Some(Datum::Value(Value::Null)) => Ok(None),
        Some(Datum::Value(Value::Integer(k))) => Ok(usize::try_from(*k).ok().filter(|k| *k > 0)),
        Some(other) => Err(ClemError::Type(format!(
            "a sequence window must be an integer, found {other:?}"
        ))),
    }
}

/// Accumulated values. Integers are summed exactly; the standard deviation uses the sums of
/// the values and of their squares, `sqrt((n Σx² - (Σx)²) / (n (n - 1)))`, which reproduces
/// every probe value to the last printed digit (a two-pass formula does too, but cannot run).
#[derive(Default)]
struct Running {
    count: u64,
    integer_sum: i64,
    all_integers: bool,
    sum: f64,
    squares: f64,
    min: Option<Value>,
    max: Option<Value>,
}

impl Running {
    fn add(&mut self, statistic: Statistic, datum: &Datum) -> Result<(), ClemError> {
        let value = match datum {
            Datum::Value(Value::Null) => return Ok(()),
            Datum::Value(value) => value,
            Datum::Bool(_) => {
                return Err(ClemError::Type(
                    "sequence statistics need values, not conditions".into(),
                ));
            }
        };
        if self.count == 0 {
            self.all_integers = true;
        }
        self.count += 1;
        match statistic {
            Statistic::Min | Statistic::Max => {
                let keep = |current: &Option<Value>, wanted: std::cmp::Ordering| {
                    current
                        .as_ref()
                        .is_none_or(|c| value.total_cmp(c) == wanted)
                };
                if keep(&self.min, std::cmp::Ordering::Less) {
                    self.min = Some(value.clone());
                }
                if keep(&self.max, std::cmp::Ordering::Greater) {
                    self.max = Some(value.clone());
                }
            }
            Statistic::Mean | Statistic::Sum | Statistic::Sdev => {
                let number = value.as_f64().ok_or_else(|| {
                    ClemError::Type(format!("sequence statistics need numbers, found {value:?}"))
                })?;
                match value {
                    Value::Integer(i) => self.integer_sum = self.integer_sum.wrapping_add(*i),
                    _ => self.all_integers = false,
                }
                self.sum += number;
                self.squares += number * number;
            }
        }
        Ok(())
    }

    fn result(&self, statistic: Statistic, windowed: bool) -> Datum {
        if self.count == 0 {
            return Datum::NULL;
        }
        let n = self.count as f64;
        let sum = if self.all_integers {
            self.integer_sum as f64
        } else {
            self.sum
        };
        Datum::Value(match statistic {
            Statistic::Mean => Value::real(sum / n),
            Statistic::Sum if self.all_integers => Value::Integer(self.integer_sum),
            Statistic::Sum => Value::real(self.sum),
            Statistic::Min => self.min.clone().unwrap_or(Value::Null),
            Statistic::Max => self.max.clone().unwrap_or(Value::Null),
            Statistic::Sdev if self.count == 1 => {
                if windowed {
                    Value::Real(0.0)
                } else {
                    Value::Null
                }
            }
            Statistic::Sdev => {
                let variance = (n * self.squares - sum * sum) / (n * (n - 1.0));
                Value::real(variance.max(0.0).sqrt())
            }
        })
    }
}

/// `@SINCE` (`include_current` false) and `@SINCE0`: how many records back the condition was
/// last true. Never true so far gives `@INDEX + 1` (oracle rule 62); a null condition is not
/// true. `first_index` is the `@INDEX` of the first record.
pub fn since(conditions: &[Option<bool>], include_current: bool, first_index: u64) -> Vec<Datum> {
    let mut last_true: Option<usize> = None;
    let mut out = Vec::with_capacity(conditions.len());
    for (i, condition) in conditions.iter().enumerate() {
        let hit_now = include_current && *condition == Some(true);
        let back = if hit_now {
            Some(0)
        } else {
            last_true.map(|last| i - last)
        };
        out.push(Datum::Value(Value::Integer(match back {
            Some(back) => i64::try_from(back).unwrap_or(i64::MAX),
            None => index(first_index, i).saturating_add(1),
        })));
        if *condition == Some(true) {
            last_true = Some(i);
        }
    }
    out
}

/// `@LAST_NON_BLANK(field)`: the latest value, the current one included, that is not blank under
/// the field's blank definition (oracle rule 58: without a definition nothing is blank, not even
/// null).
pub fn last_non_blank(values: &[Datum], missing: &MissingSpec) -> Vec<Datum> {
    let mut last = Datum::NULL;
    values
        .iter()
        .map(|datum| {
            if let Datum::Value(value) = datum
                && !missing.is_blank(value)
            {
                last = datum.clone();
            }
            last.clone()
        })
        .collect()
}

/// The 1-based `@INDEX` of record `i`.
pub fn index(first_index: u64, i: usize) -> i64 {
    i64::try_from(first_index)
        .unwrap_or(i64::MAX)
        .saturating_add(i64::try_from(i).unwrap_or(i64::MAX))
}
