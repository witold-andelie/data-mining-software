//! Text cells as typed values: the decoding, trimming, storage inference and value parsing that
//! Modeler's text sources share (Var. File through [`crate::delimited`], Fixed File through
//! [`crate::fixed`]).
//!
//! Rules (Var. File: `DRUG1n`, `HEART.csv`, `acct97.txt`, `customer.dat`, `fraud_info.csv`,
//! `customer_bank_data.csv`, `goods.dat`; Fixed File: `oracle/sources/fixed_*`):
//! - An empty field is the empty string in a string column and `Null` otherwise
//!   (`oracle/probes/clem_semantics`, `oracle/records/*`).
//! - Storage inference inspects the first `scan_lines` (default 50, `scanLineCount`) records:
//!   a column is `Integer` when every non-empty scanned value is a whole number that fits 64 bits,
//!   `Real` when every one is a finite decimal number, `Date`/`Time`/`Timestamp` when date
//!   recognition is on and every scanned value matches the configured date/time format (or ISO
//!   `YYYY-MM-DD` / `DD/MM/YYYY` when auto-recognising), else `String`. A column with no values in
//!   the scanned window is `String`. Values after the scanned window that fail to parse for the
//!   inferred storage become `Null` (e.g. `30/2/1997` at row 58 of `acct97.txt`; `late` in
//!   `fixed_scan_one`).
//! - A storage override (`textImportTable`) fixes the storage; its `dateFormat` and `timeFormat`
//!   read that field's dates and times (`fixed_custom_storage`: `DD/MM/YYYY`, `HH.MM.SS`).
//!   Values that do not parse are `Null`.
//! - Numeric columns are range (`Continuous`), the others categorical.

use dms_types::calendar;
use dms_types::{DataModel, Field, Measurement, ModelError, Storage, Table, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Trim {
    #[default]
    None,
    Left,
    Right,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecimalSymbol {
    #[default]
    Period,
    Comma,
}

/// Date format from `stream/properties/dateFormat` or `textImportTable/rows/row/dateFormat`
/// (`docs/formats/stream-properties.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DateFormat {
    /// `format1`: `DDMMYY`
    DdMmYy,
    /// `format2`: `MMDDYY`
    MmDdYy,
    /// `format3`: `YYMMDD`
    YyMmDd,
    /// `format4`: `YYYYMMDD`
    YyyyMmDd,
    /// `format5`: `DD/MM/YY`
    DdMmYySlash,
    /// `format6`: `DD/MM/YYYY`
    DdMmYyyySlash,
    /// `format7`: `MM/DD/YY`
    MmDdYySlash,
    /// `format8`: `MM/DD/YYYY`
    MmDdYyyySlash,
    /// `format9`: `DD-MM-YY`
    DdMmYyDash,
    /// `format10`: `DD-MM-YYYY`
    DdMmYyyyDash,
    /// `format11`: `MM-DD-YY`
    MmDdYyDash,
    /// `format12`: `MM-DD-YYYY`
    MmDdYyyyDash,
    /// `format13`: `DD.MM.YY`
    DdMmYyDot,
    /// `format14`: `DD.MM.YYYY`
    DdMmYyyyDot,
    /// `format15`: `MM.DD.YY`
    MmDdYyDot,
    /// `format16`: `MM.DD.YYYY`
    MmDdYyyyDot,
    /// `format17`: `DD-MON-YY`
    DdMonYyDash,
    /// `format18`: `DD/MON/YY`
    DdMonYySlash,
    /// `format19`: `DD.MON.YY`
    DdMonYyDot,
    /// `format20`: `DD-MON-YYYY`
    DdMonYyyyDash,
    /// `format21`: `DD/MON/YYYY`
    DdMonYyyySlash,
    /// `format22`: `DD.MON.YYYY`
    DdMonYyyyDot,
    /// `format23`: `YYYY-MM-DD`
    #[default]
    YyyyMmDdDash,
    /// `format24`: `YYYYDDD`
    YyyyDdd,
}

impl DateFormat {
    /// Parses a Modeler saved format id (`format1`..`format24`) or scripting name.
    pub fn from_modeler_id(id: &str) -> Option<Self> {
        match id.trim() {
            "format1" | "DDMMYY" => Some(DateFormat::DdMmYy),
            "format2" | "MMDDYY" => Some(DateFormat::MmDdYy),
            "format3" | "YYMMDD" => Some(DateFormat::YyMmDd),
            "format4" | "YYYYMMDD" => Some(DateFormat::YyyyMmDd),
            "format5" | "DD/MM/YY" => Some(DateFormat::DdMmYySlash),
            "format6" | "DD/MM/YYYY" => Some(DateFormat::DdMmYyyySlash),
            "format7" | "MM/DD/YY" => Some(DateFormat::MmDdYySlash),
            "format8" | "MM/DD/YYYY" => Some(DateFormat::MmDdYyyySlash),
            "format9" | "DD-MM-YY" => Some(DateFormat::DdMmYyDash),
            "format10" | "DD-MM-YYYY" => Some(DateFormat::DdMmYyyyDash),
            "format11" | "MM-DD-YY" => Some(DateFormat::MmDdYyDash),
            "format12" | "MM-DD-YYYY" => Some(DateFormat::MmDdYyyyDash),
            "format13" | "DD.MM.YY" => Some(DateFormat::DdMmYyDot),
            "format14" | "DD.MM.YYYY" => Some(DateFormat::DdMmYyyyDot),
            "format15" | "MM.DD.YY" => Some(DateFormat::MmDdYyDot),
            "format16" | "MM.DD.YYYY" => Some(DateFormat::MmDdYyyyDot),
            "format17" | "DD-MON-YY" => Some(DateFormat::DdMonYyDash),
            "format18" | "DD/MON/YY" => Some(DateFormat::DdMonYySlash),
            "format19" | "DD.MON.YY" => Some(DateFormat::DdMonYyDot),
            "format20" | "DD-MON-YYYY" => Some(DateFormat::DdMonYyyyDash),
            "format21" | "DD/MON/YYYY" => Some(DateFormat::DdMonYyyySlash),
            "format22" | "DD.MON.YYYY" => Some(DateFormat::DdMonYyyyDot),
            "format23" | "YYYY-MM-DD" => Some(DateFormat::YyyyMmDdDash),
            "format24" | "YYYYDDD" => Some(DateFormat::YyyyDdd),
            _ => None,
        }
    }
}

/// How bytes become text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TextEncoding {
    /// UTF-8 when valid, else Windows-1252; a byte-order mark selects its encoding.
    #[default]
    Auto,
    /// A WHATWG encoding label (`UTF-8`, `GBK`, `windows-1252`, ...).
    Label(String),
}

impl TextEncoding {
    /// Whether `label` is a known WHATWG encoding label.
    pub fn is_valid_label(label: &str) -> bool {
        encoding_rs::Encoding::for_label(label.trim().as_bytes()).is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum InvalidCharacters {
    #[default]
    Discard,
    Replace(String),
}

/// A user-chosen storage for one field (`textImportTable` row with `override`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageOverride {
    pub field: String,
    pub storage: Storage,
    /// The row's `dateFormat`, for date and timestamp storage.
    pub date_format: Option<DateFormat>,
    /// The row's `timeFormat`, for time and timestamp storage.
    pub time_format: Option<TimeFormat>,
}

/// Time format from `stream/properties/timeFormat` or `textImportTable/rows/row/timeFormat`
/// (`docs/formats/stream-properties.md`: `format1`..`format15` in the guide's order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeFormat {
    parts: ClockParts,
    /// `None` for the compact forms (`HHMMSS`).
    separator: Option<char>,
    /// One or two digits per part (`(H)H:(M)M:(S)S`) instead of exactly two.
    loose: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClockParts {
    HourMinuteSecond,
    HourMinute,
    MinuteSecond,
}

impl TimeFormat {
    /// Parses a Modeler saved format id (`format1`..`format15`) or scripting name.
    pub fn from_modeler_id(id: &str) -> Option<Self> {
        use ClockParts::{HourMinute, HourMinuteSecond, MinuteSecond};
        let (parts, separator, loose) = match id.trim() {
            "format1" | "HHMMSS" => (HourMinuteSecond, None, false),
            "format2" | "HHMM" => (HourMinute, None, false),
            "format3" | "MMSS" => (MinuteSecond, None, false),
            "format4" | "HH:MM:SS" => (HourMinuteSecond, Some(':'), false),
            "format5" | "HH:MM" => (HourMinute, Some(':'), false),
            "format6" | "MM:SS" => (MinuteSecond, Some(':'), false),
            "format7" | "(H)H:(M)M:(S)S" => (HourMinuteSecond, Some(':'), true),
            "format8" | "(H)H:(M)M" => (HourMinute, Some(':'), true),
            "format9" | "(M)M:(S)S" => (MinuteSecond, Some(':'), true),
            "format10" | "HH.MM.SS" => (HourMinuteSecond, Some('.'), false),
            "format11" | "HH.MM" => (HourMinute, Some('.'), false),
            "format12" | "MM.SS" => (MinuteSecond, Some('.'), false),
            "format13" | "(H)H.(M)M.(S)S" => (HourMinuteSecond, Some('.'), true),
            "format14" | "(H)H.(M)M" => (HourMinute, Some('.'), true),
            "format15" | "(M)M.(S)S" => (MinuteSecond, Some('.'), true),
            _ => return None,
        };
        Some(TimeFormat {
            parts,
            separator,
            loose,
        })
    }
}

/// Settings shared by the text sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextOptions {
    pub encoding: TextEncoding,
    pub invalid: InvalidCharacters,
    /// Lines skipped before the first record (or the header).
    pub skip_lines: usize,
    /// Trimming of every field's text.
    pub trim: Trim,
    /// Number of records inspected when inferring storage (`scanLineCount`, default 50).
    pub scan_lines: usize,
    pub decimal: DecimalSymbol,
    pub recognize_dates: bool,
    pub date_format: DateFormat,
    /// `None`: one or two hour digits, two minute digits, optional seconds and fraction.
    pub time_format: Option<TimeFormat>,
    /// Start of the 100-year window for two-digit years (`date2DigitBaseline`, default 1930).
    pub two_digit_year_baseline: i32,
    pub overrides: Vec<StorageOverride>,
}

impl Default for TextOptions {
    fn default() -> Self {
        TextOptions {
            encoding: TextEncoding::Auto,
            invalid: InvalidCharacters::Discard,
            skip_lines: 0,
            trim: Trim::None,
            scan_lines: 50,
            decimal: DecimalSymbol::Period,
            recognize_dates: true,
            date_format: DateFormat::YyyyMmDdDash,
            time_format: None,
            two_digit_year_baseline: 1930,
            overrides: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextError {
    Encoding(String),
    TooManyColumns(usize),
    Model(ModelError),
}

impl std::fmt::Display for TextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TextError::Encoding(label) => write!(f, "unknown text encoding {label:?}"),
            TextError::TooManyColumns(count) => {
                write!(
                    f,
                    "{count} columns exceed the limit of {}",
                    crate::delimited::MAX_COLUMNS
                )
            }
            TextError::Model(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for TextError {}

/// The file's text: decoded, without a leading byte-order mark, and with every character for
/// which `invalid` holds discarded or replaced. Undecodable bytes are always invalid.
pub(crate) fn decode(
    bytes: &[u8],
    options: &TextOptions,
    invalid: impl Fn(char) -> bool,
) -> Result<String, TextError> {
    let encoding = match &options.encoding {
        TextEncoding::Label(label) => encoding_rs::Encoding::for_label(label.trim().as_bytes())
            .ok_or_else(|| TextError::Encoding(label.clone()))?,
        TextEncoding::Auto => match encoding_rs::Encoding::for_bom(bytes) {
            Some((encoding, _)) => encoding,
            None if std::str::from_utf8(bytes).is_ok() => encoding_rs::UTF_8,
            None => encoding_rs::WINDOWS_1252,
        },
    };
    let (text, _, _) = encoding.decode(bytes);
    let text = text.trim_start_matches('\u{feff}');
    let is_invalid = |c: char| c == '\u{fffd}' || invalid(c);
    if !text.chars().any(is_invalid) {
        return Ok(text.to_string());
    }
    let replacement = match &options.invalid {
        InvalidCharacters::Discard => "",
        InvalidCharacters::Replace(text) => text,
    };
    let mut clean = String::with_capacity(text.len());
    for c in text.chars() {
        if is_invalid(c) {
            clean.push_str(replacement);
        } else {
            clean.push(c);
        }
    }
    Ok(clean)
}

/// The table of records of field texts (already trimmed), one field per name. A field missing
/// from a short record is `Null`.
pub(crate) fn build_table(
    names: &[String],
    records: &[Vec<String>],
    options: &TextOptions,
) -> Result<Table, TextError> {
    let mut fields = Vec::with_capacity(names.len());
    let mut columns: Vec<Vec<Value>> = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        let cells: Vec<Option<&str>> = records
            .iter()
            .map(|record| record.get(index).map(String::as_str))
            .collect();
        let raw: Vec<Option<&str>> = cells
            .iter()
            .map(|cell| cell.filter(|value| !value.is_empty()))
            .collect();
        let choice = options
            .overrides
            .iter()
            .find(|choice| choice.field == *name);
        let formatted;
        let column_options = match choice {
            Some(choice) if choice.date_format.is_some() || choice.time_format.is_some() => {
                formatted = TextOptions {
                    date_format: choice.date_format.unwrap_or(options.date_format),
                    time_format: choice.time_format.or(options.time_format),
                    overrides: Vec::new(),
                    ..options.clone()
                };
                &formatted
            }
            _ => options,
        };
        let storage = choice
            .map(|choice| choice.storage)
            .filter(|storage| *storage != Storage::Unknown)
            .unwrap_or_else(|| infer_storage(&raw, column_options));
        let values = cells
            .iter()
            .map(|cell| match cell {
                Some(text) if !text.is_empty() || storage == Storage::String => {
                    convert(text, storage, column_options)
                }
                _ => Value::Null,
            })
            .collect();
        let mut field = Field::new(name.clone(), storage);
        field.measurement = match storage {
            Storage::String => Measurement::Categorical,
            _ => Measurement::Continuous,
        };
        fields.push(field);
        columns.push(values);
    }
    let rows = (0..records.len())
        .map(|row| {
            columns
                .iter()
                .map(|column| column.get(row).cloned().unwrap_or(Value::Null))
                .collect()
        })
        .collect();
    let model = DataModel::new(fields).map_err(TextError::Model)?;
    Table::new(model, rows).map_err(TextError::Model)
}

pub(crate) fn finish(field: String, trim: Trim) -> String {
    let trimmed = match trim {
        Trim::None => return field,
        Trim::Left => field.trim_start(),
        Trim::Right => field.trim_end(),
        Trim::Both => field.trim(),
    };
    trimmed.to_string()
}

fn infer_storage(values: &[Option<&str>], options: &TextOptions) -> Storage {
    let limit = values.len().min(options.scan_lines.max(1));
    let scanned = values.get(..limit).unwrap_or(values);
    let present: Vec<&str> = scanned.iter().flatten().copied().collect();
    if present.is_empty() {
        return Storage::String;
    }
    let all = |kind: Storage| {
        present
            .iter()
            .all(|text| parse(text.trim(), kind, options).is_some())
    };
    if all(Storage::Integer) {
        return Storage::Integer;
    }
    if all(Storage::Real) {
        return Storage::Real;
    }
    if options.recognize_dates {
        for kind in [Storage::Date, Storage::Time, Storage::Timestamp] {
            if all(kind) {
                return kind;
            }
        }
    }
    Storage::String
}

fn convert(text: &str, storage: Storage, options: &TextOptions) -> Value {
    match storage {
        Storage::String | Storage::Unknown | Storage::List => Value::string(text),
        _ => parse(text.trim(), storage, options).unwrap_or(Value::Null),
    }
}

fn parse(text: &str, storage: Storage, options: &TextOptions) -> Option<Value> {
    match storage {
        Storage::Integer => parse_integer(text).map(Value::Integer),
        Storage::Real => parse_real(text, options.decimal).map(Value::real),
        Storage::Date => parse_date(text, options).map(Value::Date),
        Storage::Time => parse_clock(text, options).map(Value::Time),
        Storage::Timestamp => parse_timestamp(text, options).map(Value::Timestamp),
        Storage::String | Storage::Unknown | Storage::List => Some(Value::string(text)),
    }
}

fn parse_integer(text: &str) -> Option<i64> {
    let digits = text.strip_prefix(['-', '+']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.strip_prefix('+').unwrap_or(text).parse().ok()
}

/// Plain decimal numbers with optional exponent; no `inf`/`nan`/hex forms.
fn parse_real(text: &str, decimal: DecimalSymbol) -> Option<f64> {
    if text.is_empty()
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b',' | b'e' | b'E'))
        || !text.bytes().any(|b| b.is_ascii_digit())
    {
        return None;
    }
    let normalised = match decimal {
        DecimalSymbol::Period if text.contains(',') => return None,
        DecimalSymbol::Period => text.to_string(),
        DecimalSymbol::Comma if text.contains('.') => return None,
        DecimalSymbol::Comma => text.replace(',', "."),
    };
    normalised
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn number(text: &str, digits: std::ops::RangeInclusive<usize>) -> Option<u32> {
    (digits.contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

fn parse_date(text: &str, options: &TextOptions) -> Option<i64> {
    let baseline = options.two_digit_year_baseline;
    if let Some(days) = parse_date_format(text, options.date_format, baseline) {
        return Some(days);
    }
    if options.recognize_dates {
        for fallback in [DateFormat::YyyyMmDdDash, DateFormat::DdMmYyyySlash] {
            if fallback != options.date_format
                && let Some(days) = parse_date_format(text, fallback, baseline)
            {
                return Some(days);
            }
        }
    }
    None
}

fn expand_two_digit_year(yy: u32, baseline: i32) -> i64 {
    let base = i64::from(baseline);
    let century = base.div_euclid(100) * 100;
    let candidate = century + i64::from(yy);
    if candidate < base {
        candidate + 100
    } else {
        candidate
    }
}

fn split3(text: &str, sep: char) -> Option<(&str, &str, &str)> {
    let mut parts = text.split(sep);
    let a = parts.next()?.trim();
    let b = parts.next()?.trim();
    let c = parts.next()?.trim();
    if parts.next().is_some() {
        return None;
    }
    Some((a, b, c))
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

fn parse_date_format(text: &str, format: DateFormat, baseline: i32) -> Option<i64> {
    let (year, month, day) = match format {
        DateFormat::DdMmYy => (
            expand_two_digit_year(number(text.get(4..6)?, 2..=2)?, baseline),
            number(text.get(2..4)?, 2..=2)?,
            number(text.get(0..2)?, 2..=2)?,
        ),
        DateFormat::MmDdYy => (
            expand_two_digit_year(number(text.get(4..6)?, 2..=2)?, baseline),
            number(text.get(0..2)?, 2..=2)?,
            number(text.get(2..4)?, 2..=2)?,
        ),
        DateFormat::YyMmDd => (
            expand_two_digit_year(number(text.get(0..2)?, 2..=2)?, baseline),
            number(text.get(2..4)?, 2..=2)?,
            number(text.get(4..6)?, 2..=2)?,
        ),
        DateFormat::YyyyMmDd => {
            if text.len() != 8 {
                return None;
            }
            (
                i64::from(number(text.get(0..4)?, 4..=4)?),
                number(text.get(4..6)?, 2..=2)?,
                number(text.get(6..8)?, 2..=2)?,
            )
        }
        DateFormat::YyyyDdd => {
            if text.len() != 7 {
                return None;
            }
            let year = i64::from(number(text.get(0..4)?, 4..=4)?);
            let doy = number(text.get(4..7)?, 3..=3)?;
            let max_doy = if calendar::is_leap_year(year) {
                366
            } else {
                365
            };
            if !(1..=max_doy).contains(&doy) {
                return None;
            }
            return calendar::days_from_civil(year, 1, 1)?.checked_add(i64::from(doy - 1));
        }
        DateFormat::DdMmYySlash | DateFormat::DdMmYyDash | DateFormat::DdMmYyDot => {
            let sep = match format {
                DateFormat::DdMmYySlash => '/',
                DateFormat::DdMmYyDash => '-',
                _ => '.',
            };
            let (d, m, y) = split3(text, sep)?;
            (
                expand_two_digit_year(number(y, 2..=2)?, baseline),
                number(m, 1..=2)?,
                number(d, 1..=2)?,
            )
        }
        DateFormat::DdMmYyyySlash | DateFormat::DdMmYyyyDash | DateFormat::DdMmYyyyDot => {
            let sep = match format {
                DateFormat::DdMmYyyySlash => '/',
                DateFormat::DdMmYyyyDash => '-',
                _ => '.',
            };
            let (d, m, y) = split3(text, sep)?;
            (
                i64::from(number(y, 4..=4)?),
                number(m, 1..=2)?,
                number(d, 1..=2)?,
            )
        }
        DateFormat::MmDdYySlash | DateFormat::MmDdYyDash | DateFormat::MmDdYyDot => {
            let sep = match format {
                DateFormat::MmDdYySlash => '/',
                DateFormat::MmDdYyDash => '-',
                _ => '.',
            };
            let (m, d, y) = split3(text, sep)?;
            (
                expand_two_digit_year(number(y, 2..=2)?, baseline),
                number(m, 1..=2)?,
                number(d, 1..=2)?,
            )
        }
        DateFormat::MmDdYyyySlash | DateFormat::MmDdYyyyDash | DateFormat::MmDdYyyyDot => {
            let sep = match format {
                DateFormat::MmDdYyyySlash => '/',
                DateFormat::MmDdYyyyDash => '-',
                _ => '.',
            };
            let (m, d, y) = split3(text, sep)?;
            (
                i64::from(number(y, 4..=4)?),
                number(m, 1..=2)?,
                number(d, 1..=2)?,
            )
        }
        DateFormat::DdMonYyDash | DateFormat::DdMonYySlash | DateFormat::DdMonYyDot => {
            let sep = match format {
                DateFormat::DdMonYyDash => '-',
                DateFormat::DdMonYySlash => '/',
                _ => '.',
            };
            let (d, mon, y) = split3(text, sep)?;
            (
                expand_two_digit_year(number(y, 2..=2)?, baseline),
                month_from_abbrev(mon)?,
                number(d, 1..=2)?,
            )
        }
        DateFormat::DdMonYyyyDash | DateFormat::DdMonYyyySlash | DateFormat::DdMonYyyyDot => {
            let sep = match format {
                DateFormat::DdMonYyyyDash => '-',
                DateFormat::DdMonYyyySlash => '/',
                _ => '.',
            };
            let (d, mon, y) = split3(text, sep)?;
            (
                i64::from(number(y, 4..=4)?),
                month_from_abbrev(mon)?,
                number(d, 1..=2)?,
            )
        }
        DateFormat::YyyyMmDdDash => {
            let (y, m, d) = split3(text, '-')?;
            (
                i64::from(number(y, 4..=4)?),
                number(m, 1..=2)?,
                number(d, 1..=2)?,
            )
        }
    };
    if matches!(
        format,
        DateFormat::DdMmYy | DateFormat::MmDdYy | DateFormat::YyMmDd
    ) && text.len() != 6
    {
        return None;
    }
    calendar::days_from_civil(year, month, day)
}

fn parse_time(text: &str) -> Option<i64> {
    let (clock, fraction) = match text.split_once('.') {
        Some((clock, fraction)) => (clock, Some(fraction)),
        None => (text, None),
    };
    let mut parts = clock.split(':');
    let hour = number(parts.next()?, 1..=2)?;
    let minute = number(parts.next()?, 2..=2)?;
    let second = parts.next().map_or(Some(0), |s| number(s, 2..=2))?;
    if parts.next().is_some() {
        return None;
    }
    let micros = match fraction {
        None => 0,
        Some(digits) => {
            let kept = digits.get(..digits.len().min(6))?;
            number(kept, 1..=6)? * 10u32.pow(u32::try_from(6 - kept.len()).ok()?)
        }
    };
    calendar::micros_from_time(hour, minute, second, micros)
}

fn parse_timestamp(text: &str, options: &TextOptions) -> Option<i64> {
    let (date, time) = text.split_once('T').or_else(|| text.rsplit_once(' '))?;
    let days = parse_date(date.trim(), options)?;
    let micros = parse_clock(time.trim(), options)?;
    days.checked_mul(calendar::MICROS_PER_DAY)?
        .checked_add(micros)
}

fn parse_clock(text: &str, options: &TextOptions) -> Option<i64> {
    match options.time_format {
        Some(format) => parse_time_format(text, format),
        None => parse_time(text),
    }
}

fn parse_time_format(text: &str, format: TimeFormat) -> Option<i64> {
    let count = match format.parts {
        ClockParts::HourMinuteSecond => 3,
        ClockParts::HourMinute | ClockParts::MinuteSecond => 2,
    };
    let values: Vec<u32> = match format.separator {
        None => {
            if text.len() != 2 * count {
                return None;
            }
            (0..count)
                .map(|i| number(text.get(2 * i..2 * i + 2)?, 2..=2))
                .collect::<Option<_>>()?
        }
        Some(separator) => {
            let digits = if format.loose { 1..=2 } else { 2..=2 };
            let parts: Vec<&str> = text.split(separator).collect();
            if parts.len() != count {
                return None;
            }
            parts
                .iter()
                .map(|part| number(part, digits.clone()))
                .collect::<Option<_>>()?
        }
    };
    let (hour, minute, second) = match (format.parts, values.as_slice()) {
        (ClockParts::HourMinuteSecond, &[h, m, s]) => (h, m, s),
        (ClockParts::HourMinute, &[h, m]) => (h, m, 0),
        (ClockParts::MinuteSecond, &[m, s]) => (0, m, s),
        _ => return None,
    };
    calendar::micros_from_time(hour, minute, second, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_formats() {
        let time = |text: &str, id: &str| {
            parse_time_format(text, TimeFormat::from_modeler_id(id).unwrap())
        };
        let micros = |h, m, s| calendar::micros_from_time(h, m, s, 0);
        assert_eq!(time("06.07.08", "format10"), micros(6, 7, 8));
        assert_eq!(time("6.7.8", "format10"), None);
        assert_eq!(time("6.7.8", "HH.MM.SS"), None);
        assert_eq!(time("6.7.8", "format13"), micros(6, 7, 8));
        assert_eq!(time("060708", "format1"), micros(6, 7, 8));
        assert_eq!(time("0607", "format3"), micros(0, 6, 7));
        assert_eq!(time("23:59", "format5"), micros(23, 59, 0));
        assert_eq!(time("24:00:00", "format4"), None);
        assert_eq!(time("06:07:08", "format10"), None);
        assert!(TimeFormat::from_modeler_id("format16").is_none());
    }

    #[test]
    fn decode_replaces_invalid_characters() {
        let replace = TextOptions {
            invalid: InvalidCharacters::Replace("?".into()),
            ..TextOptions::default()
        };
        assert_eq!(
            decode(b"a\0b\xff", &replace, |c| c == '\0').unwrap(),
            "a?b\u{ff}"
        );
        assert_eq!(
            decode(b"\xef\xbb\xbfa\0b", &TextOptions::default(), |c| c == '\0').unwrap(),
            "ab"
        );
        assert_eq!(
            decode(b"a\0b", &TextOptions::default(), |_| false).unwrap(),
            "a\0b"
        );
    }
}
