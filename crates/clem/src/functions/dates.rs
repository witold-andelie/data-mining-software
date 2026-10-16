//! Date, time and timestamp CLEM functions (docs/clem/oracle-results.md rules 50-55).

use super::{Function, Returns, f, first, integer, kind, number};
use crate::types::Ty;
use crate::{ClemError, Datum};
use dms_types::{
    Value,
    calendar::{
        MICROS_PER_DAY, MICROS_PER_SECOND, civil_from_days, days_from_civil, days_in_month,
        micros_from_time, split_timestamp, time_from_micros,
    },
};

/// CLEM's baseline date is `1900-01-01` (oracle rule 50), which is `-25_567` days from the
/// `1970-01-01` epoch used by `dms_types::Value::Date`.
const CLEM_EPOCH_DAYS: i64 = -25_567;
const DAYS_PER_MONTH: f64 = 30.4375;
const DAYS_PER_YEAR: f64 = 365.25;
const DAYS_PER_WEEK: f64 = 7.0;
const SECONDS_PER_DAY: i64 = 86_400;

pub(super) const FUNCTIONS: &[Function] = &[
    // Oracle rules 51 & 55: `to_date` rejects numeric arguments (`to_date(0)` is a type error)
    // and parses strings with 4-digit year first (`YYYY-MM-DD`) or day first (`DD/MM/YYYY`,
    // `DD-MM-YY` with 2-digit rollover `< 30 -> 20xx`, `>= 30 -> 19xx`).
    f("to_date", 1, to_date).returns(Returns::Fixed(Ty::Date)),
    f("to_time", 1, to_time).returns(Returns::Fixed(Ty::Time)),
    f("to_timestamp", 1, to_timestamp).returns(Returns::Fixed(Ty::Timestamp)),
    f("to_datetime", 1, to_datetime).returns(Returns::Fixed(Ty::Timestamp)),
    // Oracle rule 50: `datetime_date` takes 1 argument (seconds since 1900-01-01, string, date,
    // or timestamp) or 3 arguments `(YEAR, MONTH, DAY)`.
    f("datetime_date", 1, datetime_date)
        .returns(Returns::Fixed(Ty::Date))
        .arities(&[1, 3]),
    f("datetime_time", 1, datetime_time)
        .returns(Returns::Fixed(Ty::Time))
        .arities(&[1, 3]),
    f("datetime_timestamp", 1, datetime_timestamp)
        .returns(Returns::Fixed(Ty::Timestamp))
        .arities(&[1, 2, 6]),
    // Oracle rule 52: difference from DATE1 to DATE2 (`DATE2 - DATE1`).
    f("date_days_difference", 2, |a| {
        let Some((d1, d2)) = date_pair(a)? else {
            return Ok(Datum::NULL);
        };
        Ok(Datum::Value(Value::Integer(d2.saturating_sub(d1))))
    })
    .returns(Returns::Fixed(Ty::Integer)),
    f("date_months_difference", 2, |a| {
        date_diff_real(a, DAYS_PER_MONTH)
    })
    .returns(Returns::Fixed(Ty::Real)),
    f("date_weeks_difference", 2, |a| {
        date_diff_real(a, DAYS_PER_WEEK)
    })
    .returns(Returns::Fixed(Ty::Real)),
    f("date_years_difference", 2, |a| {
        date_diff_real(a, DAYS_PER_YEAR)
    })
    .returns(Returns::Fixed(Ty::Real)),
    // Oracle rules 50 & 52: elapsed units since 1900-01-01.
    f("date_in_days", 1, |a| {
        let Some(days) = date_days1(a)? else {
            return Ok(Datum::NULL);
        };
        Ok(Datum::Value(Value::Integer(days - CLEM_EPOCH_DAYS)))
    })
    .returns(Returns::Fixed(Ty::Integer)),
    f("date_in_months", 1, |a| date_in_real(a, DAYS_PER_MONTH)).returns(Returns::Fixed(Ty::Real)),
    f("date_in_weeks", 1, |a| date_in_real(a, DAYS_PER_WEEK)).returns(Returns::Fixed(Ty::Real)),
    f("date_in_years", 1, |a| date_in_real(a, DAYS_PER_YEAR)).returns(Returns::Fixed(Ty::Real)),
    f("datetime_in_seconds", 1, datetime_in_seconds).returns(Returns::Fixed(Ty::Real)),
    // Oracle rule 53: `date_add_months` clamps to month end (`2020-01-31 + 1 month = 2020-02-29`),
    // whereas `date_add_years` returns null when Feb 29 lands on a non-leap year.
    f("date_add_days", 2, |a| date_add_fixed(a, 1)).returns(Returns::Arg(1)),
    f("date_add_weeks", 2, |a| date_add_fixed(a, 7)).returns(Returns::Arg(1)),
    f("date_add_months", 2, date_add_months).returns(Returns::Arg(1)),
    f("date_add_years", 2, date_add_years).returns(Returns::Arg(1)),
    f("datetime_year", 1, |a| {
        let Some(days) = date_days1(a)? else {
            return Ok(Datum::NULL);
        };
        Ok(match civil_from_days(days) {
            Some((y, _, _)) => Datum::Value(Value::Integer(y)),
            None => Datum::NULL,
        })
    })
    .returns(Returns::Fixed(Ty::Integer)),
    f("datetime_month", 1, |a| {
        let Some(days) = date_days1(a)? else {
            return Ok(Datum::NULL);
        };
        Ok(match civil_from_days(days) {
            Some((_, m, _)) => Datum::Value(Value::Integer(i64::from(m))),
            None => Datum::NULL,
        })
    })
    .returns(Returns::Fixed(Ty::Integer)),
    f("datetime_day", 1, |a| {
        let Some(days) = date_days1(a)? else {
            return Ok(Datum::NULL);
        };
        Ok(match civil_from_days(days) {
            Some((_, _, d)) => Datum::Value(Value::Integer(i64::from(d))),
            None => Datum::NULL,
        })
    })
    .returns(Returns::Fixed(Ty::Integer)),
    // Oracle rule 54: 1 = Sunday .. 7 = Saturday (1970-01-01 was Thursday = 5).
    f("datetime_weekday", 1, |a| {
        let Some(days) = date_days1(a)? else {
            return Ok(Datum::NULL);
        };
        let weekday = (days + 4).rem_euclid(7) + 1;
        Ok(Datum::Value(Value::Integer(weekday)))
    })
    .returns(Returns::Fixed(Ty::Integer)),
    f("datetime_day_name", 1, |a| {
        name_from_index(
            a,
            &[
                "Sunday",
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
            ],
        )
    })
    .returns(Returns::Fixed(Ty::String)),
    f("datetime_day_short_name", 1, |a| {
        name_from_index(a, &["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"])
    })
    .returns(Returns::Fixed(Ty::String)),
    f("datetime_month_name", 1, |a| {
        name_from_index(
            a,
            &[
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ],
        )
    })
    .returns(Returns::Fixed(Ty::String)),
    f("datetime_month_short_name", 1, |a| {
        name_from_index(
            a,
            &[
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ],
        )
    })
    .returns(Returns::Fixed(Ty::String)),
    f("datetime_hour", 1, |a| time_part(a, |h, _, _| h)).returns(Returns::Fixed(Ty::Integer)),
    f("datetime_minute", 1, |a| time_part(a, |_, m, _| m)).returns(Returns::Fixed(Ty::Integer)),
    f("datetime_second", 1, |a| time_part(a, |_, _, s| s)).returns(Returns::Fixed(Ty::Integer)),
    f("date_iso_day", 1, |a| {
        let Some(days) = date_days1(a)? else {
            return Ok(Datum::NULL);
        };
        // ISO-8601: 1 = Monday .. 7 = Sunday (1970-01-01 was Thursday = 4).
        Ok(Datum::Value(Value::Integer((days + 3).rem_euclid(7) + 1)))
    })
    .returns(Returns::Fixed(Ty::Integer)),
    f("date_iso_week", 1, |a| {
        let Some(days) = date_days1(a)? else {
            return Ok(Datum::NULL);
        };
        Ok(match iso_year_week(days) {
            Some((_, w)) => Datum::Value(Value::Integer(w)),
            None => Datum::NULL,
        })
    })
    .returns(Returns::Fixed(Ty::Integer)),
    f("date_iso_year", 1, |a| {
        let Some(days) = date_days1(a)? else {
            return Ok(Datum::NULL);
        };
        Ok(match iso_year_week(days) {
            Some((y, _)) => Datum::Value(Value::Integer(y)),
            None => Datum::NULL,
        })
    })
    .returns(Returns::Fixed(Ty::Integer)),
    f("date_from_ywd", 3, |a| {
        let (Some(y), Some(w), Some(d)) = (a.first(), a.get(1), a.get(2)) else {
            return Err(ClemError::Type(
                "date_from_ywd needs three arguments".into(),
            ));
        };
        if y.is_null() || w.is_null() || d.is_null() {
            return Ok(Datum::NULL);
        }
        let (y, w, d) = (integer(y)?, integer(w)?, integer(d)?);
        Ok(match days_from_iso_ywd(y, w, d) {
            Some(days) => Datum::Value(Value::Date(days)),
            None => Datum::NULL,
        })
    })
    .returns(Returns::Fixed(Ty::Date)),
    f("date_before", 2, |a| {
        let Some((d1, d2)) = date_pair(a)? else {
            return Ok(Datum::NULL);
        };
        Ok(Datum::Bool(d1 < d2))
    })
    .returns(Returns::Fixed(Ty::Bool)),
    f("time_before", 2, |a| {
        let Some((t1, t2)) = time_pair(a)? else {
            return Ok(Datum::NULL);
        };
        Ok(Datum::Bool(t1 < t2))
    })
    .returns(Returns::Fixed(Ty::Bool)),
    f("is_date", 1, |a| kind(a, |v| matches!(v, Value::Date(_))))
        .returns(Returns::Fixed(Ty::Bool))
        .type_test(|t| matches!(t, Ty::Date)),
    f("is_time", 1, |a| kind(a, |v| matches!(v, Value::Time(_))))
        .returns(Returns::Fixed(Ty::Bool))
        .type_test(|t| matches!(t, Ty::Time)),
    f("is_timestamp", 1, |a| {
        kind(a, |v| matches!(v, Value::Timestamp(_)))
    })
    .returns(Returns::Fixed(Ty::Bool))
    .type_test(|t| matches!(t, Ty::Timestamp)),
    f("is_datetime", 1, |a| {
        kind(a, |v| {
            matches!(v, Value::Date(_) | Value::Time(_) | Value::Timestamp(_))
        })
    })
    .returns(Returns::Fixed(Ty::Bool))
    .type_test(|t| matches!(t, Ty::Date | Ty::Time | Ty::Timestamp)),
    f("time_in_secs", 1, |a| {
        let Some(micros) = time_micros1(a)? else {
            return Ok(Datum::NULL);
        };
        Ok(Datum::Value(Value::Integer(micros / MICROS_PER_SECOND)))
    })
    .returns(Returns::Fixed(Ty::Integer)),
    f("time_in_mins", 1, |a| {
        let Some(micros) = time_micros1(a)? else {
            return Ok(Datum::NULL);
        };
        Ok(Datum::Value(Value::real(
            (micros as f64) / ((60 * MICROS_PER_SECOND) as f64),
        )))
    })
    .returns(Returns::Fixed(Ty::Real)),
    f("time_in_hours", 1, |a| {
        let Some(micros) = time_micros1(a)? else {
            return Ok(Datum::NULL);
        };
        Ok(Datum::Value(Value::real(
            (micros as f64) / ((3600 * MICROS_PER_SECOND) as f64),
        )))
    })
    .returns(Returns::Fixed(Ty::Real)),
    f("time_secs_difference", 2, |a| {
        let Some((t1, t2)) = time_pair(a)? else {
            return Ok(Datum::NULL);
        };
        Ok(Datum::Value(Value::Integer((t2 - t1) / MICROS_PER_SECOND)))
    })
    .returns(Returns::Fixed(Ty::Integer)),
    f("time_mins_difference", 2, |a| {
        let Some((t1, t2)) = time_pair(a)? else {
            return Ok(Datum::NULL);
        };
        Ok(Datum::Value(Value::real(
            ((t2 - t1) as f64) / ((60 * MICROS_PER_SECOND) as f64),
        )))
    })
    .returns(Returns::Fixed(Ty::Real)),
    f("time_hours_difference", 2, |a| {
        let Some((t1, t2)) = time_pair(a)? else {
            return Ok(Datum::NULL);
        };
        Ok(Datum::Value(Value::real(
            ((t2 - t1) as f64) / ((3600 * MICROS_PER_SECOND) as f64),
        )))
    })
    .returns(Returns::Fixed(Ty::Real)),
    f("time_add_seconds", 2, |a| time_add(a, MICROS_PER_SECOND)).returns(Returns::Arg(1)),
    f("time_add_minutes", 2, |a| {
        time_add(a, 60 * MICROS_PER_SECOND)
    })
    .returns(Returns::Arg(1)),
    f("time_add_hours", 2, |a| {
        time_add(a, 3600 * MICROS_PER_SECOND)
    })
    .returns(Returns::Arg(1)),
];

fn join_timestamp(days: i64, micros: i64) -> Option<i64> {
    days.checked_mul(MICROS_PER_DAY)?.checked_add(micros)
}

fn to_date(args: &[Datum]) -> Result<Datum, ClemError> {
    match first(args)? {
        Datum::Value(Value::Null) => Ok(Datum::NULL),
        Datum::Value(Value::Date(d)) => Ok(Datum::Value(Value::Date(d))),
        Datum::Value(Value::Timestamp(ts)) => Ok(Datum::Value(Value::Date(split_timestamp(ts).0))),
        Datum::Value(Value::String(s)) => Ok(Datum::Value(
            parse_date_str(&s).map_or(Value::Null, Value::Date),
        )),
        other => Err(ClemError::Type(format!(
            "cannot convert {other:?} to a date"
        ))),
    }
}

fn to_time(args: &[Datum]) -> Result<Datum, ClemError> {
    match first(args)? {
        Datum::Value(Value::Null) => Ok(Datum::NULL),
        Datum::Value(Value::Time(t)) => Ok(Datum::Value(Value::Time(t))),
        Datum::Value(Value::Timestamp(ts)) => Ok(Datum::Value(Value::Time(split_timestamp(ts).1))),
        Datum::Value(Value::String(s)) => Ok(Datum::Value(
            parse_time_str(&s).map_or(Value::Null, Value::Time),
        )),
        other => Err(ClemError::Type(format!(
            "cannot convert {other:?} to a time"
        ))),
    }
}

fn to_timestamp(args: &[Datum]) -> Result<Datum, ClemError> {
    match first(args)? {
        Datum::Value(Value::Null) => Ok(Datum::NULL),
        Datum::Value(Value::Timestamp(ts)) => Ok(Datum::Value(Value::Timestamp(ts))),
        Datum::Value(Value::Date(d)) => Ok(Datum::Value(
            join_timestamp(d, 0).map_or(Value::Null, Value::Timestamp),
        )),
        Datum::Value(Value::String(s)) => Ok(Datum::Value(
            parse_timestamp_str(&s).map_or(Value::Null, Value::Timestamp),
        )),
        other => Err(ClemError::Type(format!(
            "cannot convert {other:?} to a timestamp"
        ))),
    }
}

fn to_datetime(args: &[Datum]) -> Result<Datum, ClemError> {
    match first(args)? {
        Datum::Value(Value::Null) => Ok(Datum::NULL),
        Datum::Value(v @ (Value::Date(_) | Value::Time(_) | Value::Timestamp(_))) => {
            Ok(Datum::Value(v))
        }
        Datum::Value(Value::String(s)) => {
            if let Some(ts) = parse_timestamp_str(&s) {
                Ok(Datum::Value(Value::Timestamp(ts)))
            } else if let Some(d) = parse_date_str(&s) {
                Ok(Datum::Value(Value::Date(d)))
            } else if let Some(t) = parse_time_str(&s) {
                Ok(Datum::Value(Value::Time(t)))
            } else {
                Ok(Datum::NULL)
            }
        }
        other => Err(ClemError::Type(format!(
            "cannot convert {other:?} to a datetime"
        ))),
    }
}

/// Integer arguments of the multi-part constructors; `None` when one is null.
fn integer_parts(args: &[Datum]) -> Result<Option<Vec<i64>>, ClemError> {
    if args.iter().any(Datum::is_null) {
        return Ok(None);
    }
    args.iter().map(integer).collect::<Result<_, _>>().map(Some)
}

fn datetime_date(args: &[Datum]) -> Result<Datum, ClemError> {
    // `datetime_date(YEAR, MONTH, DAY)`.
    if args.len() == 3 {
        let Some(&[y, m, d]) = integer_parts(args)?.as_deref() else {
            return Ok(Datum::NULL);
        };
        let days = u32::try_from(m)
            .ok()
            .zip(u32::try_from(d).ok())
            .and_then(|(m, d)| days_from_civil(y, m, d));
        return Ok(Datum::Value(days.map_or(Value::Null, Value::Date)));
    }
    match first(args)? {
        Datum::Value(Value::Null) => Ok(Datum::NULL),
        Datum::Value(Value::Integer(secs)) => {
            let days = CLEM_EPOCH_DAYS.saturating_add(secs.div_euclid(SECONDS_PER_DAY));
            Ok(Datum::Value(Value::Date(days)))
        }
        Datum::Value(Value::Real(secs)) => {
            let days_offset = (secs / (SECONDS_PER_DAY as f64)).floor() as i64;
            Ok(Datum::Value(Value::Date(
                CLEM_EPOCH_DAYS.saturating_add(days_offset),
            )))
        }
        other => to_date(&[other]),
    }
}

fn datetime_time(args: &[Datum]) -> Result<Datum, ClemError> {
    // `datetime_time(HOUR, MINUTE, SECOND)`.
    if args.len() == 3 {
        let Some(&[h, m, s]) = integer_parts(args)?.as_deref() else {
            return Ok(Datum::NULL);
        };
        let micros = match (u32::try_from(h), u32::try_from(m), u32::try_from(s)) {
            (Ok(h), Ok(m), Ok(s)) => micros_from_time(h, m, s, 0),
            _ => None,
        };
        return Ok(Datum::Value(micros.map_or(Value::Null, Value::Time)));
    }
    match first(args)? {
        Datum::Value(Value::Null) => Ok(Datum::NULL),
        Datum::Value(Value::Integer(secs)) => {
            let micros = secs.saturating_mul(MICROS_PER_SECOND);
            Ok(Datum::Value(if (0..MICROS_PER_DAY).contains(&micros) {
                Value::Time(micros)
            } else {
                Value::Null
            }))
        }
        Datum::Value(Value::Real(secs)) => {
            let micros = (secs * (MICROS_PER_SECOND as f64)).round() as i64;
            Ok(Datum::Value(if (0..MICROS_PER_DAY).contains(&micros) {
                Value::Time(micros)
            } else {
                Value::Null
            }))
        }
        other => to_time(&[other]),
    }
}

fn datetime_timestamp(args: &[Datum]) -> Result<Datum, ClemError> {
    // `datetime_timestamp(DATE, TIME)`.
    if let [date, time] = args {
        return match (date, time) {
            _ if date.is_null() || time.is_null() => Ok(Datum::NULL),
            (Datum::Value(Value::Date(d)), Datum::Value(Value::Time(t))) => Ok(Datum::Value(
                join_timestamp(*d, *t).map_or(Value::Null, Value::Timestamp),
            )),
            _ => Err(ClemError::Type(
                "datetime_timestamp(date, time) needs a date and a time".into(),
            )),
        };
    }
    // `datetime_timestamp(YEAR, MONTH, DAY, HOUR, MINUTE, SECOND)`.
    if args.len() == 6 {
        let Some(&[y, mo, d, h, mi, s]) = integer_parts(args)?.as_deref() else {
            return Ok(Datum::NULL);
        };
        let days = u32::try_from(mo)
            .ok()
            .zip(u32::try_from(d).ok())
            .and_then(|(mo, d)| days_from_civil(y, mo, d));
        let time = match (u32::try_from(h), u32::try_from(mi), u32::try_from(s)) {
            (Ok(h), Ok(mi), Ok(s)) => micros_from_time(h, mi, s, 0),
            _ => None,
        };
        let ts = days.zip(time).and_then(|(d, t)| join_timestamp(d, t));
        return Ok(Datum::Value(ts.map_or(Value::Null, Value::Timestamp)));
    }
    match first(args)? {
        Datum::Value(Value::Null) => Ok(Datum::NULL),
        Datum::Value(Value::Integer(secs)) => {
            let unix_secs = CLEM_EPOCH_DAYS
                .saturating_mul(SECONDS_PER_DAY)
                .saturating_add(secs);
            Ok(Datum::Value(
                unix_secs
                    .checked_mul(MICROS_PER_SECOND)
                    .map_or(Value::Null, Value::Timestamp),
            ))
        }
        Datum::Value(Value::Real(secs)) => {
            let base = (CLEM_EPOCH_DAYS * SECONDS_PER_DAY) as f64;
            let micros = ((base + secs) * (MICROS_PER_SECOND as f64)).round() as i64;
            Ok(Datum::Value(Value::Timestamp(micros)))
        }
        other => to_timestamp(&[other]),
    }
}

/// Parses Modeler's accepted date strings (oracle rule 55):
/// - `YYYY-MM-DD` or `YYYY/MM/DD` (4-digit year first)
/// - `DD-MM-YYYY` or `DD/MM/YYYY` (4-digit year last)
/// - `DD-MM-YY` or `DD/MM/YY` (2-digit year last; `< 30 -> 2000 + yy`, `>= 30 -> 1900 + yy`)
fn parse_date_str(text: &str) -> Option<i64> {
    let text = text.trim();
    let sep = if text.contains('-') && !text.contains('/') {
        '-'
    } else if text.contains('/') && !text.contains('-') {
        '/'
    } else {
        return None;
    };
    let parts: Vec<&str> = text.split(sep).collect();
    if parts.len() != 3
        || !parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    let (p0, p1, p2) = (*parts.first()?, *parts.get(1)?, *parts.get(2)?);
    let (year, month, day) = if p0.len() == 4 && p1.len() <= 2 && p2.len() <= 2 {
        (
            p0.parse::<i64>().ok()?,
            p1.parse::<u32>().ok()?,
            p2.parse::<u32>().ok()?,
        )
    } else if p0.len() <= 2 && p1.len() <= 2 && p2.len() == 4 {
        (
            p2.parse::<i64>().ok()?,
            p1.parse::<u32>().ok()?,
            p0.parse::<u32>().ok()?,
        )
    } else if p0.len() <= 2 && p1.len() <= 2 && p2.len() == 2 {
        let yy = p2.parse::<i64>().ok()?;
        let year = if yy < 30 { 2000 + yy } else { 1900 + yy };
        (year, p1.parse::<u32>().ok()?, p0.parse::<u32>().ok()?)
    } else {
        return None;
    };
    days_from_civil(year, month, day)
}

fn parse_time_str(text: &str) -> Option<i64> {
    let text = text.trim();
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h = parts.first()?.parse::<u32>().ok()?;
    let m = parts.get(1)?.parse::<u32>().ok()?;
    let sec_str = *parts.get(2)?;
    let (s, micros) = match sec_str.split_once('.') {
        Some((s_part, frac_part)) => {
            if frac_part.is_empty() || !frac_part.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let s = s_part.parse::<u32>().ok()?;
            let mut padded: String = frac_part.chars().take(6).collect();
            while padded.len() < 6 {
                padded.push('0');
            }
            (s, padded.parse::<u32>().ok()?)
        }
        None => (sec_str.parse::<u32>().ok()?, 0),
    };
    micros_from_time(h, m, s, micros)
}

fn parse_timestamp_str(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date_part, time_part) = text.split_once(' ').or_else(|| text.split_once('T'))?;
    let days = parse_date_str(date_part)?;
    let micros = parse_time_str(time_part)?;
    join_timestamp(days, micros)
}

fn date_days(datum: &Datum) -> Result<Option<i64>, ClemError> {
    match datum {
        Datum::Value(Value::Null) => Ok(None),
        Datum::Value(Value::Date(d)) => Ok(Some(*d)),
        Datum::Value(Value::Timestamp(ts)) => Ok(Some(split_timestamp(*ts).0)),
        other => Err(ClemError::Type(format!("expected a date, found {other:?}"))),
    }
}

fn date_days1(args: &[Datum]) -> Result<Option<i64>, ClemError> {
    date_days(&first(args)?)
}

fn date_pair(args: &[Datum]) -> Result<Option<(i64, i64)>, ClemError> {
    let (Some(a), Some(b)) = (args.first(), args.get(1)) else {
        return Err(ClemError::Type("needs two date arguments".into()));
    };
    Ok(date_days(a)?.zip(date_days(b)?))
}

fn date_diff_real(args: &[Datum], divisor: f64) -> Result<Datum, ClemError> {
    let Some((d1, d2)) = date_pair(args)? else {
        return Ok(Datum::NULL);
    };
    Ok(Datum::Value(Value::real(((d2 - d1) as f64) / divisor)))
}

fn date_in_real(args: &[Datum], divisor: f64) -> Result<Datum, ClemError> {
    let Some(days) = date_days1(args)? else {
        return Ok(Datum::NULL);
    };
    Ok(Datum::Value(Value::real(
        ((days - CLEM_EPOCH_DAYS) as f64) / divisor,
    )))
}

fn datetime_in_seconds(args: &[Datum]) -> Result<Datum, ClemError> {
    match first(args)? {
        Datum::Value(Value::Null) => Ok(Datum::NULL),
        Datum::Value(Value::Date(d)) => {
            let secs = ((d - CLEM_EPOCH_DAYS) as f64) * (SECONDS_PER_DAY as f64);
            Ok(Datum::Value(Value::real(secs)))
        }
        Datum::Value(Value::Time(t)) => Ok(Datum::Value(Value::real(
            (t as f64) / (MICROS_PER_SECOND as f64),
        ))),
        Datum::Value(Value::Timestamp(ts)) => {
            let (days, micros) = split_timestamp(ts);
            let secs = ((days - CLEM_EPOCH_DAYS) as f64) * (SECONDS_PER_DAY as f64)
                + (micros as f64) / (MICROS_PER_SECOND as f64);
            Ok(Datum::Value(Value::real(secs)))
        }
        other => Err(ClemError::Type(format!(
            "datetime_in_seconds needs a date, time or timestamp, found {other:?}"
        ))),
    }
}

fn date_add_fixed(args: &[Datum], scale_days: i64) -> Result<Datum, ClemError> {
    let (Some(num), Some(target)) = (args.first(), args.get(1)) else {
        return Err(ClemError::Type("needs two arguments".into()));
    };
    if num.is_null() || target.is_null() {
        return Ok(Datum::NULL);
    }
    let delta_days = integer(num)?.saturating_mul(scale_days);
    match target {
        Datum::Value(Value::Date(d)) => Ok(Datum::Value(Value::Date(d.saturating_add(delta_days)))),
        Datum::Value(Value::Timestamp(ts)) => {
            let (days, micros) = split_timestamp(*ts);
            Ok(Datum::Value(
                join_timestamp(days.saturating_add(delta_days), micros)
                    .map_or(Value::Null, Value::Timestamp),
            ))
        }
        other => Err(ClemError::Type(format!(
            "expected a date or timestamp, found {other:?}"
        ))),
    }
}

fn date_add_months(args: &[Datum]) -> Result<Datum, ClemError> {
    let (Some(num), Some(target)) = (args.first(), args.get(1)) else {
        return Err(ClemError::Type(
            "date_add_months needs two arguments".into(),
        ));
    };
    if num.is_null() || target.is_null() {
        return Ok(Datum::NULL);
    }
    let delta = integer(num)?;
    let add_to_days = |days: i64| -> Option<i64> {
        let (y, m, d) = civil_from_days(days)?;
        let total = y
            .checked_mul(12)?
            .checked_add(i64::from(m) - 1)?
            .checked_add(delta)?;
        let new_y = total.div_euclid(12);
        let new_m = u32::try_from(total.rem_euclid(12) + 1).ok()?;
        let clamped_d = d.min(days_in_month(new_y, new_m));
        days_from_civil(new_y, new_m, clamped_d)
    };
    match target {
        Datum::Value(Value::Date(d)) => Ok(Datum::Value(
            add_to_days(*d).map_or(Value::Null, Value::Date),
        )),
        Datum::Value(Value::Timestamp(ts)) => {
            let (days, micros) = split_timestamp(*ts);
            let ts = add_to_days(days).and_then(|d| join_timestamp(d, micros));
            Ok(Datum::Value(ts.map_or(Value::Null, Value::Timestamp)))
        }
        other => Err(ClemError::Type(format!(
            "expected a date or timestamp, found {other:?}"
        ))),
    }
}

fn date_add_years(args: &[Datum]) -> Result<Datum, ClemError> {
    let (Some(num), Some(target)) = (args.first(), args.get(1)) else {
        return Err(ClemError::Type("date_add_years needs two arguments".into()));
    };
    if num.is_null() || target.is_null() {
        return Ok(Datum::NULL);
    }
    let delta = integer(num)?;
    let add_to_days = |days: i64| -> Option<i64> {
        let (y, m, d) = civil_from_days(days)?;
        // Oracle rule 53: does NOT clamp Feb 29 on non-leap target years; returns null.
        days_from_civil(y.checked_add(delta)?, m, d)
    };
    match target {
        Datum::Value(Value::Date(d)) => Ok(Datum::Value(
            add_to_days(*d).map_or(Value::Null, Value::Date),
        )),
        Datum::Value(Value::Timestamp(ts)) => {
            let (days, micros) = split_timestamp(*ts);
            let ts = add_to_days(days).and_then(|d| join_timestamp(d, micros));
            Ok(Datum::Value(ts.map_or(Value::Null, Value::Timestamp)))
        }
        other => Err(ClemError::Type(format!(
            "expected a date or timestamp, found {other:?}"
        ))),
    }
}

fn name_from_index(args: &[Datum], names: &[&str]) -> Result<Datum, ClemError> {
    let value = first(args)?;
    if value.is_null() {
        return Ok(Datum::NULL);
    }
    let idx = integer(&value)?;
    if idx < 1 {
        return Ok(Datum::NULL);
    }
    let pos = usize::try_from(idx - 1).ok();
    Ok(match pos.and_then(|i| names.get(i)) {
        Some(&name) => Datum::Value(Value::string(name)),
        None => Datum::NULL,
    })
}

fn time_micros(datum: &Datum) -> Result<Option<i64>, ClemError> {
    match datum {
        Datum::Value(Value::Null) => Ok(None),
        Datum::Value(Value::Time(t)) => Ok(Some(*t)),
        Datum::Value(Value::Timestamp(ts)) => Ok(Some(split_timestamp(*ts).1)),
        other => Err(ClemError::Type(format!("expected a time, found {other:?}"))),
    }
}

fn time_micros1(args: &[Datum]) -> Result<Option<i64>, ClemError> {
    time_micros(&first(args)?)
}

fn time_pair(args: &[Datum]) -> Result<Option<(i64, i64)>, ClemError> {
    let (Some(a), Some(b)) = (args.first(), args.get(1)) else {
        return Err(ClemError::Type("needs two time arguments".into()));
    };
    Ok(time_micros(a)?.zip(time_micros(b)?))
}

fn time_part(args: &[Datum], pick: fn(u32, u32, u32) -> u32) -> Result<Datum, ClemError> {
    let Some(micros) = time_micros1(args)? else {
        return Ok(Datum::NULL);
    };
    Ok(match time_from_micros(micros) {
        Some((h, m, s, _)) => Datum::Value(Value::Integer(i64::from(pick(h, m, s)))),
        None => Datum::NULL,
    })
}

fn time_add(args: &[Datum], unit_micros: i64) -> Result<Datum, ClemError> {
    let (Some(num), Some(target)) = (args.first(), args.get(1)) else {
        return Err(ClemError::Type("needs two arguments".into()));
    };
    if num.is_null() || target.is_null() {
        return Ok(Datum::NULL);
    }
    let delta = (number(num)? * (unit_micros as f64)).round() as i64;
    match target {
        Datum::Value(Value::Time(t)) => Ok(Datum::Value(Value::Time(
            t.saturating_add(delta).rem_euclid(MICROS_PER_DAY),
        ))),
        Datum::Value(Value::Timestamp(ts)) => {
            Ok(Datum::Value(Value::Timestamp(ts.saturating_add(delta))))
        }
        other => Err(ClemError::Type(format!(
            "expected a time or timestamp, found {other:?}"
        ))),
    }
}

/// ISO-8601 `(iso_year, iso_week)` for a day number relative to 1970-01-01.
fn iso_year_week(days: i64) -> Option<(i64, i64)> {
    // ISO weeks start on Monday; the Thursday of the same week determines its ISO year.
    let iso_weekday = (days + 3).rem_euclid(7) + 1; // 1 = Mon .. 7 = Sun
    let thursday = days.checked_add(4 - iso_weekday)?;
    let (iso_year, _, _) = civil_from_days(thursday)?;
    let jan4 = days_from_civil(iso_year, 1, 4)?;
    let jan4_weekday = (jan4 + 3).rem_euclid(7) + 1;
    let week1_monday = jan4 - (jan4_weekday - 1);
    let week = (days - week1_monday).div_euclid(7) + 1;
    Some((iso_year, week))
}

fn days_from_iso_ywd(year: i64, week: i64, weekday: i64) -> Option<i64> {
    if !(1..=53).contains(&week) || !(1..=7).contains(&weekday) {
        return None;
    }
    let jan4 = days_from_civil(year, 1, 4)?;
    let jan4_weekday = (jan4 + 3).rem_euclid(7) + 1;
    let week1_monday = jan4 - (jan4_weekday - 1);
    let days = week1_monday + (week - 1) * 7 + (weekday - 1);
    let (check_year, check_week) = iso_year_week(days)?;
    (check_year == year && check_week == week).then_some(days)
}
