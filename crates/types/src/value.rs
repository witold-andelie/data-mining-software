//! Cell values.

use crate::calendar;
use crate::field::Storage;
use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

/// One cell of a record. `Null` is Modeler's system-missing value (`$null$`); user-defined
/// missing values ("blanks") are ordinary values flagged by the field's [`crate::MissingSpec`].
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    /// Never NaN: constructors normalize NaN to `Null`.
    Real(f64),
    String(Arc<str>),
    /// Days since 1970-01-01.
    Date(i64),
    /// Microseconds since midnight.
    Time(i64),
    /// Microseconds since 1970-01-01T00:00:00.
    Timestamp(i64),
    List(Arc<[Value]>),
}

impl Value {
    /// A real value; NaN becomes `Null` (Modeler data has no NaN).
    pub fn real(value: f64) -> Value {
        if value.is_nan() {
            Value::Null
        } else {
            Value::Real(value)
        }
    }

    pub fn string(value: impl Into<Arc<str>>) -> Value {
        Value::String(value.into())
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// The storage this value naturally has; `None` for `Null`.
    pub fn storage(&self) -> Option<Storage> {
        Some(match self {
            Value::Null => return None,
            Value::Integer(_) => Storage::Integer,
            Value::Real(_) => Storage::Real,
            Value::String(_) => Storage::String,
            Value::Date(_) => Storage::Date,
            Value::Time(_) => Storage::Time,
            Value::Timestamp(_) => Storage::Timestamp,
            Value::List(_) => Storage::List,
        })
    }

    /// Numeric view: integers and reals.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Integer(i) => Some(*i as f64),
            Value::Real(r) => Some(*r),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// A total order for sorting and grouping: `Null` first, then numbers (integers and reals
    /// compared by value), strings (by code point), dates, times, timestamps and lists.
    pub fn total_cmp(&self, other: &Value) -> Ordering {
        fn rank(value: &Value) -> u8 {
            match value {
                Value::Null => 0,
                Value::Integer(_) | Value::Real(_) => 1,
                Value::String(_) => 2,
                Value::Date(_) => 3,
                Value::Time(_) => 4,
                Value::Timestamp(_) => 5,
                Value::List(_) => 6,
            }
        }
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
            (Value::Integer(_) | Value::Real(_), Value::Integer(_) | Value::Real(_)) => {
                let a = self.as_f64().unwrap_or(0.0);
                let b = other.as_f64().unwrap_or(0.0);
                a.total_cmp(&b)
            }
            (Value::String(a), Value::String(b)) => a.cmp(b),
            (Value::Date(a), Value::Date(b))
            | (Value::Time(a), Value::Time(b))
            | (Value::Timestamp(a), Value::Timestamp(b)) => a.cmp(b),
            (Value::List(a), Value::List(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    let ordering = x.total_cmp(y);
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                }
                a.len().cmp(&b.len())
            }
            _ => rank(self).cmp(&rank(other)),
        }
    }
}

/// Plain rendering: ISO 8601 dates and times, shortest round-trip reals.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => Ok(()),
            Value::Integer(i) => write!(f, "{i}"),
            Value::Real(r) => write!(f, "{r}"),
            Value::String(s) => f.write_str(s),
            Value::Date(days) => write_date(f, *days),
            Value::Time(micros) => write_time(f, *micros),
            Value::Timestamp(micros) => {
                let (days, time) = calendar::split_timestamp(*micros);
                write_date(f, days)?;
                f.write_str(" ")?;
                write_time(f, time)
            }
            Value::List(items) => {
                f.write_str("[")?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("]")
            }
        }
    }
}

fn write_date(f: &mut fmt::Formatter<'_>, days: i64) -> fmt::Result {
    match calendar::civil_from_days(days) {
        Some((y, m, d)) => write!(f, "{y:04}-{m:02}-{d:02}"),
        None => write!(f, "<day {days}>"),
    }
}

fn write_time(f: &mut fmt::Formatter<'_>, micros: i64) -> fmt::Result {
    match calendar::time_from_micros(micros) {
        Some((h, m, s, 0)) => write!(f, "{h:02}:{m:02}:{s:02}"),
        Some((h, m, s, us)) => write!(f, "{h:02}:{m:02}:{s:02}.{us:06}"),
        None => write!(f, "<time {micros}>"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nan_is_null() {
        assert_eq!(Value::real(f64::NAN), Value::Null);
        assert_eq!(Value::real(1.5), Value::Real(1.5));
    }

    #[test]
    fn total_order_mixes_integers_and_reals_and_puts_null_first() {
        let mut values = vec![
            Value::string("b"),
            Value::Real(2.5),
            Value::Null,
            Value::Integer(2),
            Value::Integer(3),
            Value::string("a"),
        ];
        values.sort_by(Value::total_cmp);
        assert_eq!(
            values,
            vec![
                Value::Null,
                Value::Integer(2),
                Value::Real(2.5),
                Value::Integer(3),
                Value::string("a"),
                Value::string("b"),
            ]
        );
    }

    #[test]
    fn displays_dates_times_and_lists() {
        assert_eq!(Value::Date(0).to_string(), "1970-01-01");
        assert_eq!(Value::Time(3_723_000_000).to_string(), "01:02:03");
        assert_eq!(
            Value::Timestamp(calendar::MICROS_PER_DAY + 1).to_string(),
            "1970-01-02 00:00:00.000001"
        );
        let list = Value::List(Arc::from(vec![Value::Integer(1), Value::string("x")]));
        assert_eq!(list.to_string(), "[1,x]");
        assert_eq!(Value::Null.to_string(), "");
    }
}
