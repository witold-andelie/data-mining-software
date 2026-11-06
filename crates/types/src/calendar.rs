//! Calendar arithmetic on the proleptic Gregorian calendar.
//!
//! Dates are day numbers relative to 1970-01-01 (day 0); times are microseconds since midnight;
//! timestamps are microseconds since 1970-01-01T00:00:00 (no time zone). Conversions to the
//! epochs other formats use (Modeler's date baseline, SPSS's 1582-10-14) happen at their
//! boundaries.

pub const MICROS_PER_SECOND: i64 = 1_000_000;
pub const MICROS_PER_DAY: i64 = 86_400 * MICROS_PER_SECOND;

/// Supported years; outside this range conversions return `None`.
const MIN_YEAR: i64 = -100_000;
const MAX_YEAR: i64 = 100_000;

/// Day number of `year-month-day`, or `None` for an invalid date.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    if !(MIN_YEAR..=MAX_YEAR).contains(&year) || !(1..=12).contains(&month) {
        return None;
    }
    if day == 0 || day > days_in_month(year, month) {
        return None;
    }
    // Days since 1970-01-01 (H. Hinnant's public-domain algorithm, March-based years).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y.rem_euclid(400);
    let m = i64::from(month);
    let day_of_year = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

/// `(year, month, day)` of a day number, or `None` outside the supported range.
pub fn civil_from_days(days: i64) -> Option<(i64, u32, u32)> {
    let z = days.checked_add(719_468)?;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = u32::try_from(day_of_year - (153 * mp + 2) / 5 + 1).ok()?;
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).ok()?;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (MIN_YEAR..=MAX_YEAR)
        .contains(&year)
        .then_some((year, month, day))
}

pub fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Microseconds since midnight, or `None` for an invalid time of day.
pub fn micros_from_time(hour: u32, minute: u32, second: u32, micros: u32) -> Option<i64> {
    if hour > 23 || minute > 59 || second > 59 || micros > 999_999 {
        return None;
    }
    Some(
        ((i64::from(hour) * 60 + i64::from(minute)) * 60 + i64::from(second)) * MICROS_PER_SECOND
            + i64::from(micros),
    )
}

/// `(hour, minute, second, microsecond)` of a time of day.
pub fn time_from_micros(micros: i64) -> Option<(u32, u32, u32, u32)> {
    if !(0..MICROS_PER_DAY).contains(&micros) {
        return None;
    }
    let seconds = micros / MICROS_PER_SECOND;
    Some((
        u32::try_from(seconds / 3600).ok()?,
        u32::try_from(seconds / 60 % 60).ok()?,
        u32::try_from(seconds % 60).ok()?,
        u32::try_from(micros % MICROS_PER_SECOND).ok()?,
    ))
}

/// `MON YYYY` (`Jan 1999`, `DEC 2019`): first day of that month. Case-insensitive 3-letter
/// English month abbreviation and a 4-digit year, separated by whitespace.
pub fn parse_month_year(text: &str) -> Option<i64> {
    let mut parts = text.split_whitespace();
    let month = month_from_abbrev(parts.next()?)?;
    let year = parts.next()?;
    if parts.next().is_some() || year.len() != 4 || !year.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    days_from_civil(year.parse().ok()?, month, 1)
}

fn month_from_abbrev(text: &str) -> Option<u32> {
    match text.to_ascii_uppercase().as_str() {
        "JAN" => Some(1),
        "FEB" => Some(2),
        "MAR" => Some(3),
        "APR" => Some(4),
        "MAY" => Some(5),
        "JUN" => Some(6),
        "JUL" => Some(7),
        "AUG" => Some(8),
        "SEP" => Some(9),
        "OCT" => Some(10),
        "NOV" => Some(11),
        "DEC" => Some(12),
        _ => None,
    }
}

/// Splits a timestamp into its day number and time of day.
pub fn split_timestamp(micros: i64) -> (i64, i64) {
    (
        micros.div_euclid(MICROS_PER_DAY),
        micros.rem_euclid(MICROS_PER_DAY),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_and_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
        assert_eq!(days_from_civil(2000, 3, 1), Some(11_017));
        assert_eq!(days_from_civil(1900, 1, 1), Some(-25_567));
        assert_eq!(days_from_civil(1582, 10, 14), Some(-141_428));
        assert_eq!(civil_from_days(-141_428), Some((1582, 10, 14)));
    }

    #[test]
    fn round_trips_every_day_over_a_long_range() {
        let start = days_from_civil(1500, 1, 1).unwrap();
        let end = days_from_civil(2600, 12, 31).unwrap();
        for days in start..=end {
            let (y, m, d) = civil_from_days(days).unwrap();
            assert_eq!(days_from_civil(y, m, d), Some(days));
        }
    }

    #[test]
    fn rejects_invalid_dates_and_times() {
        assert_eq!(days_from_civil(2023, 2, 29), None);
        assert!(days_from_civil(2024, 2, 29).is_some());
        assert_eq!(days_from_civil(1900, 13, 1), None);
        assert_eq!(days_from_civil(1900, 1, 0), None);
        assert_eq!(days_from_civil(i64::MAX, 1, 1), None);
        assert_eq!(civil_from_days(i64::MAX), None);
        assert_eq!(micros_from_time(24, 0, 0, 0), None);
        assert_eq!(time_from_micros(MICROS_PER_DAY), None);
    }

    #[test]
    fn times_and_timestamps() {
        let t = micros_from_time(13, 5, 9, 250_000).unwrap();
        assert_eq!(time_from_micros(t), Some((13, 5, 9, 250_000)));
        assert_eq!(split_timestamp(-1), (-1, MICROS_PER_DAY - 1));
    }

    #[test]
    fn month_year_strings() {
        assert_eq!(parse_month_year("Jan 1999"), days_from_civil(1999, 1, 1));
        assert_eq!(parse_month_year("JAN 1999"), days_from_civil(1999, 1, 1));
        assert_eq!(parse_month_year("Dec 2019"), days_from_civil(2019, 12, 1));
        assert_eq!(parse_month_year("Feb  2000"), days_from_civil(2000, 2, 1));
        assert_eq!(parse_month_year("Jan1999"), None);
        assert_eq!(parse_month_year("January 1999"), None);
        assert_eq!(parse_month_year("Jan 99"), None);
    }
}
