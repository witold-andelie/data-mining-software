//! Delimited text as tables, the way Modeler's Var. File node reads it.
//!
//! Implemented (settings of `DelimitedFileImportNode`): delimiters (comma, tab, space, other
//! characters, non-printing characters), header row, header lines to skip, quote handling
//! (`include`, `discard`, `pairDiscard`) for single and double quotes, trimming of string values,
//! decimal symbol, file encoding, invalid-character handling, storage inference per column and
//! storage overrides, ISO date/time recognition.
//!
//! Rules (checked against the oracle for `DRUG1n`, `HEART.csv`, `acct97.txt`, `customer.dat`,
//! `fraud_info.csv`, `customer_bank_data.csv` and `goods.dat`):
//! - Field names are kept exactly as written (no trimming: `"sex "` stays `sex `); a missing
//!   header gives `field1`, `field2`, ... (`oracle/probes/settings_variablefile/s090.str`).
//! - An empty field is the empty string in a string column and `Null` otherwise; a field missing
//!   from a short record is `Null` (`oracle/probes/clem_semantics`, `oracle/records/*`).
//! - Storage inference inspects the first `scan_lines` (default 50, `scanLineCount`) records:
//!   a column is `Integer` when every non-empty scanned value is a whole number that fits 64 bits,
//!   `Real` when every one is a finite decimal number, `Date`/`Time`/`Timestamp` when date
//!   recognition is on and every scanned value matches the configured date/time format (or ISO
//!   `YYYY-MM-DD` / `DD/MM/YYYY` when auto-recognising), else `String`. A column with no values in
//!   the scanned window is `String`. Values after the scanned window that fail to parse for the
//!   inferred storage become `Null` (e.g. `30/2/1997` at row 58 of `acct97.txt`).
//! - Numeric columns are range (`Continuous`), the others categorical.
//!
//! GAPs: records are lines (quoted fields may not span lines); `multiBlankDelimiters` and
//! end-of-line comments are not supported.

use dms_types::calendar;
use dms_types::{DataModel, Field, Measurement, ModelError, Storage, Table, Value};
use std::collections::HashSet;

/// Columns accepted from one file.
pub const MAX_COLUMNS: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuoteMode {
    /// The quote character is ordinary text.
    Include,
    /// Quote characters are removed wherever they occur.
    Discard,
    /// A field that starts with a quote ends at the matching quote; both are removed and the
    /// text between (delimiters included) is the value.
    #[default]
    PairAndDiscard,
}

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelimitedOptions {
    pub comma: bool,
    pub tab: bool,
    pub space: bool,
    pub non_printing: bool,
    /// Extra delimiter characters.
    pub other: Vec<char>,
    pub field_names_included: bool,
    /// Lines skipped before the header (or first record).
    pub skip_lines: usize,
    /// Fixed number of fields when `useManualFieldCount` is enabled.
    pub manual_field_count: Option<usize>,
    /// Number of data records inspected when inferring storage (`scanLineCount`, default 50).
    pub scan_lines: usize,
    pub single_quote: QuoteMode,
    pub double_quote: QuoteMode,
    pub trim: Trim,
    pub decimal: DecimalSymbol,
    pub encoding: TextEncoding,
    pub invalid: InvalidCharacters,
    pub recognize_dates: bool,
    pub date_format: DateFormat,
    /// Start of the 100-year window for two-digit years (`date2DigitBaseline`, default 1930).
    pub two_digit_year_baseline: i32,
    pub overrides: Vec<StorageOverride>,
}

impl Default for DelimitedOptions {
    fn default() -> Self {
        DelimitedOptions {
            comma: true,
            tab: false,
            space: false,
            non_printing: false,
            other: Vec::new(),
            field_names_included: true,
            skip_lines: 0,
            manual_field_count: None,
            scan_lines: 50,
            single_quote: QuoteMode::PairAndDiscard,
            double_quote: QuoteMode::PairAndDiscard,
            trim: Trim::None,
            decimal: DecimalSymbol::Period,
            encoding: TextEncoding::Auto,
            invalid: InvalidCharacters::Discard,
            recognize_dates: true,
            date_format: DateFormat::YyyyMmDdDash,
            two_digit_year_baseline: 1930,
            overrides: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DelimitedError {
    Encoding(String),
    TooManyColumns(usize),
    Model(ModelError),
}

impl std::fmt::Display for DelimitedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DelimitedError::Encoding(label) => write!(f, "unknown text encoding {label:?}"),
            DelimitedError::TooManyColumns(count) => {
                write!(f, "{count} columns exceed the limit of {MAX_COLUMNS}")
            }
            DelimitedError::Model(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for DelimitedError {}

/// Reads a delimited file.
pub fn read_table(bytes: &[u8], options: &DelimitedOptions) -> Result<Table, DelimitedError> {
    let text = decode(bytes, options)?;
    let mut records = parse_records(&text, options);
    let skipped = options.skip_lines.min(records.len());
    records.drain(..skipped);
    let header = if options.field_names_included && !records.is_empty() {
        Some(records.remove(0))
    } else {
        None
    };
    let width = options
        .manual_field_count
        .map(|n| n.max(1))
        .unwrap_or_else(|| {
            header
                .as_ref()
                .map_or_else(|| records.iter().map(Vec::len).max().unwrap_or(0), Vec::len)
        });
    if width > MAX_COLUMNS {
        return Err(DelimitedError::TooManyColumns(width));
    }
    let names = field_names(header.as_deref(), width);
    let mut fields = Vec::with_capacity(width);
    let mut columns: Vec<Vec<Value>> = Vec::with_capacity(width);
    for (index, name) in names.iter().enumerate() {
        let cells: Vec<Option<&str>> = records
            .iter()
            .map(|record| record.get(index).map(String::as_str))
            .collect();
        let raw: Vec<Option<&str>> = cells
            .iter()
            .map(|cell| cell.filter(|value| !value.is_empty()))
            .collect();
        let forced = options
            .overrides
            .iter()
            .find(|choice| choice.field == *name)
            .map(|choice| choice.storage)
            .filter(|storage| *storage != Storage::Unknown);
        let storage = forced.unwrap_or_else(|| infer_storage(&raw, options));
        let values = cells
            .iter()
            .map(|cell| match cell {
                Some(text) if !text.is_empty() || storage == Storage::String => {
                    convert(text, storage, options)
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
    let model = DataModel::new(fields).map_err(DelimitedError::Model)?;
    Table::new(model, rows).map_err(DelimitedError::Model)
}

fn field_names(header: Option<&[String]>, width: usize) -> Vec<String> {
    (0..width)
        .map(|index| match header.and_then(|names| names.get(index)) {
            Some(name) if !name.is_empty() => name.clone(),
            _ => format!("field{}", index + 1),
        })
        .collect()
}

fn decode(bytes: &[u8], options: &DelimitedOptions) -> Result<String, DelimitedError> {
    let encoding = match &options.encoding {
        TextEncoding::Label(label) => encoding_rs::Encoding::for_label(label.trim().as_bytes())
            .ok_or_else(|| DelimitedError::Encoding(label.clone()))?,
        TextEncoding::Auto => match encoding_rs::Encoding::for_bom(bytes) {
            Some((encoding, _)) => encoding,
            None if std::str::from_utf8(bytes).is_ok() => encoding_rs::UTF_8,
            None => encoding_rs::WINDOWS_1252,
        },
    };
    let (text, _, _) = encoding.decode(bytes);
    let replacement = match &options.invalid {
        InvalidCharacters::Discard => "",
        InvalidCharacters::Replace(text) => text,
    };
    Ok(if text.contains('\u{fffd}') {
        text.replace('\u{fffd}', replacement)
    } else {
        text.into_owned()
    })
}

struct Scanner<'a> {
    chars: Vec<char>,
    options: &'a DelimitedOptions,
    /// Quote characters for which no closing quote exists after some position.
    unclosed: HashSet<char>,
}

impl Scanner<'_> {
    fn is_delimiter(&self, c: char) -> bool {
        match c {
            ',' => self.options.comma,
            '\t' => self.options.tab,
            ' ' => self.options.space,
            '\n' | '\r' => false,
            c if c.is_control() => self.options.non_printing || self.options.other.contains(&c),
            c => self.options.other.contains(&c),
        }
    }

    fn quote_mode(&self, c: char) -> Option<QuoteMode> {
        match c {
            '"' => Some(self.options.double_quote),
            '\'' => Some(self.options.single_quote),
            _ => None,
        }
    }

    fn ends_field(&self, index: usize) -> bool {
        match self.chars.get(index) {
            None => true,
            Some(&c) => c == '\n' || c == '\r' || self.is_delimiter(c),
        }
    }

    /// The index of the quote that closes the one opened at `open`, if the text has one; a pair
    /// of quotes inside stands for one quote character.
    fn closing_quote(&mut self, open: usize, quote: char) -> Option<usize> {
        if self.unclosed.contains(&quote) {
            return None;
        }
        let mut index = open + 1;
        while let Some(&c) = self.chars.get(index) {
            if c == quote {
                if self.chars.get(index + 1) == Some(&quote) {
                    index += 2;
                    continue;
                }
                if self.ends_field(index + 1) {
                    return Some(index);
                }
            }
            index += 1;
        }
        self.unclosed.insert(quote);
        None
    }
}

/// Splits the text into records of raw field texts (quotes resolved, nothing trimmed yet except
/// by the trim setting).
fn parse_records(text: &str, options: &DelimitedOptions) -> Vec<Vec<String>> {
    let mut scanner = Scanner {
        chars: text.trim_start_matches('\u{feff}').chars().collect(),
        options,
        unclosed: HashSet::new(),
    };
    let mut records = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut at_field_start = true;
    let mut index = 0;
    while let Some(&c) = scanner.chars.get(index) {
        if at_field_start
            && let Some(mode) = scanner.quote_mode(c)
            && mode == QuoteMode::PairAndDiscard
            && let Some(close) = scanner.closing_quote(index, c)
        {
            let inner: String = scanner
                .chars
                .get(index + 1..close)
                .unwrap_or_default()
                .iter()
                .collect();
            field.push_str(&inner.replace(&format!("{c}{c}"), &c.to_string()));
            index = close + 1;
            at_field_start = false;
            continue;
        }
        match c {
            '\r' | '\n' => {
                if c == '\r' && scanner.chars.get(index + 1) == Some(&'\n') {
                    index += 1;
                }
                if !record.is_empty() || !field.is_empty() {
                    record.push(finish(std::mem::take(&mut field), options.trim));
                    records.push(std::mem::take(&mut record));
                }
                at_field_start = true;
            }
            c if scanner.is_delimiter(c) => {
                record.push(finish(std::mem::take(&mut field), options.trim));
                at_field_start = true;
            }
            c if scanner.quote_mode(c) == Some(QuoteMode::Discard) => {
                at_field_start = false;
            }
            c => {
                field.push(c);
                at_field_start = false;
            }
        }
        index += 1;
    }
    if !record.is_empty() || !field.is_empty() {
        record.push(finish(field, options.trim));
        records.push(record);
    }
    records
}

fn finish(field: String, trim: Trim) -> String {
    let trimmed = match trim {
        Trim::None => return field,
        Trim::Left => field.trim_start(),
        Trim::Right => field.trim_end(),
        Trim::Both => field.trim(),
    };
    trimmed.to_string()
}

fn infer_storage(values: &[Option<&str>], options: &DelimitedOptions) -> Storage {
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

fn convert(text: &str, storage: Storage, options: &DelimitedOptions) -> Value {
    match storage {
        Storage::String | Storage::Unknown | Storage::List => Value::string(text),
        _ => parse(text.trim(), storage, options).unwrap_or(Value::Null),
    }
}

fn parse(text: &str, storage: Storage, options: &DelimitedOptions) -> Option<Value> {
    match storage {
        Storage::Integer => parse_integer(text).map(Value::Integer),
        Storage::Real => parse_real(text, options.decimal).map(Value::real),
        Storage::Date => parse_date(text, options).map(Value::Date),
        Storage::Time => parse_time(text).map(Value::Time),
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

fn parse_date(text: &str, options: &DelimitedOptions) -> Option<i64> {
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

fn parse_timestamp(text: &str, options: &DelimitedOptions) -> Option<i64> {
    let (date, time) = text.split_once('T').or_else(|| text.rsplit_once(' '))?;
    let days = parse_date(date.trim(), options)?;
    let micros = parse_time(time.trim())?;
    days.checked_mul(calendar::MICROS_PER_DAY)?
        .checked_add(micros)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str, options: &DelimitedOptions) -> Table {
        read_table(text.as_bytes(), options).unwrap()
    }

    fn column(table: &Table, name: &str) -> Vec<Value> {
        table.column(name).unwrap().cloned().collect()
    }

    #[test]
    fn infers_storage_and_keeps_names() {
        let table = read(
            "Age,sex ,Na,Drug\r\n23,F,0.79,drugY\r\n47,,0.7,drugC\r\n",
            &DelimitedOptions::default(),
        );
        let storages: Vec<Storage> = table.model().fields().iter().map(|f| f.storage).collect();
        assert_eq!(
            storages,
            [
                Storage::Integer,
                Storage::String,
                Storage::Real,
                Storage::String
            ]
        );
        assert!(table.model().field("sex ").is_some());
        assert_eq!(
            column(&table, "sex "),
            [Value::string("F"), Value::string("")]
        );
        assert_eq!(
            column(&table, "Age"),
            [Value::Integer(23), Value::Integer(47)]
        );
    }

    #[test]
    fn headerless_files_get_field_names_and_skip_lines() {
        let options = DelimitedOptions {
            field_names_included: false,
            skip_lines: 1,
            ..DelimitedOptions::default()
        };
        let table = read("junk\n1,2\n3,4\n", &options);
        assert_eq!(table.model().fields()[1].name, "field2");
        assert_eq!(table.row_count(), 2);
    }

    #[test]
    fn quotes_by_mode() {
        let paired = read(
            "a,b\n\"x,y\",\"say \"\"hi\"\"\"\n",
            &DelimitedOptions::default(),
        );
        assert_eq!(column(&paired, "a"), [Value::string("x,y")]);
        assert_eq!(column(&paired, "b"), [Value::string("say \"hi\"")]);

        let include = DelimitedOptions {
            double_quote: QuoteMode::Include,
            ..DelimitedOptions::default()
        };
        let table = read("a,b\n\"x\",1\n", &include);
        assert_eq!(column(&table, "a"), [Value::string("\"x\"")]);

        let discard = DelimitedOptions {
            double_quote: QuoteMode::Discard,
            ..DelimitedOptions::default()
        };
        let table = read("a,b\n\"x,y\",1\n", &discard);
        assert_eq!(table.model().len(), 2);
        assert_eq!(column(&table, "a"), [Value::string("x")]);
    }

    #[test]
    fn apostrophes_inside_text_and_unclosed_quotes_are_text() {
        let table = read("a\nO'Brien\n'tis\n", &DelimitedOptions::default());
        assert_eq!(
            column(&table, "a"),
            [Value::string("O'Brien"), Value::string("'tis")]
        );
    }

    #[test]
    fn other_delimiters_trim_and_decimal_comma() {
        let options = DelimitedOptions {
            comma: false,
            other: vec![';'],
            trim: Trim::Both,
            decimal: DecimalSymbol::Comma,
            ..DelimitedOptions::default()
        };
        let table = read("a;b\n 1,5 ; x \n", &options);
        assert_eq!(column(&table, "a"), [Value::Real(1.5)]);
        assert_eq!(column(&table, "b"), [Value::string("x")]);
        let tab_space = DelimitedOptions {
            comma: false,
            tab: true,
            space: true,
            ..DelimitedOptions::default()
        };
        assert_eq!(read("a b\tc\n1 2\t3\n", &tab_space).model().len(), 3);
    }

    #[test]
    fn dates_times_and_overrides() {
        let table = read(
            "d,t,ts,n\n2020-02-29,13:05:09,2020-02-29 13:05:09,1\n,,,\n",
            &DelimitedOptions::default(),
        );
        let storages: Vec<Storage> = table.model().fields().iter().map(|f| f.storage).collect();
        assert_eq!(
            storages,
            [
                Storage::Date,
                Storage::Time,
                Storage::Timestamp,
                Storage::Integer
            ]
        );
        assert_eq!(column(&table, "d")[1], Value::Null);

        let off = DelimitedOptions {
            recognize_dates: false,
            ..DelimitedOptions::default()
        };
        assert_eq!(
            read("d\n2020-02-29\n", &off).model().fields()[0].storage,
            Storage::String
        );

        let forced = DelimitedOptions {
            overrides: vec![StorageOverride {
                field: "n".into(),
                storage: Storage::Real,
            }],
            ..DelimitedOptions::default()
        };
        let table = read("n\n1\nx\n", &forced);
        assert_eq!(column(&table, "n"), [Value::Real(1.0), Value::Null]);
    }

    #[test]
    fn big_integers_become_real_and_special_floats_are_text() {
        let table = read(
            "a,b\n99999999999999999999,nan\n1,inf\n",
            &DelimitedOptions::default(),
        );
        assert_eq!(table.model().fields()[0].storage, Storage::Real);
        assert_eq!(table.model().fields()[1].storage, Storage::String);
    }

    #[test]
    fn encodings() {
        let latin = b"a\n\xe9\n";
        let table = read_table(latin, &DelimitedOptions::default()).unwrap();
        assert_eq!(column(&table, "a"), [Value::string("\u{e9}")]);
        let options = DelimitedOptions {
            encoding: TextEncoding::Label("nonsense".into()),
            ..DelimitedOptions::default()
        };
        assert!(read_table(b"a\n", &options).is_err());
        let bom = read_table(b"\xef\xbb\xbfa\n1\n", &DelimitedOptions::default()).unwrap();
        assert_eq!(bom.model().fields()[0].name, "a");
    }

    #[test]
    fn empty_and_malformed_inputs_do_not_panic() {
        let options = DelimitedOptions::default();
        assert_eq!(read_table(b"", &options).unwrap().model().len(), 0);
        assert_eq!(read_table(b"a,b\n", &options).unwrap().row_count(), 0);
        assert!(read_table(b"a,a\n1,2\n", &options).is_err());
        let ragged = read_table(b"a,b\n1\n1,2,3\n", &options).unwrap();
        assert_eq!(column(&ragged, "b"), [Value::Null, Value::Integer(2)]);
        let _ = read_table(b"\"\"\"\n'\n\x00\xff", &options);
    }

    #[test]
    fn scan_lines_and_date_formats() {
        // Invalid date after scan_lines becomes Null while column stays Date (like acct97.txt).
        let options = DelimitedOptions {
            scan_lines: 2,
            ..DelimitedOptions::default()
        };
        let table = read("d\n 5/ 1/1995\n11/2/1997\n30/2/1997\n", &options);
        assert_eq!(table.model().fields()[0].storage, Storage::Date);
        assert_eq!(
            column(&table, "d"),
            [
                Value::Date(calendar::days_from_civil(1995, 1, 5).unwrap()),
                Value::Date(calendar::days_from_civil(1997, 2, 11).unwrap()),
                Value::Null,
            ]
        );

        // Stream date format MM/DD/YY (format7) with 2-digit year baseline 1930.
        let mdy = DelimitedOptions {
            date_format: DateFormat::from_modeler_id("format7").unwrap(),
            two_digit_year_baseline: 1930,
            ..DelimitedOptions::default()
        };
        let table = read("d\n05/01/95\n05/01/25\n", &mdy);
        assert_eq!(
            column(&table, "d"),
            [
                Value::Date(calendar::days_from_civil(1995, 5, 1).unwrap()),
                Value::Date(calendar::days_from_civil(2025, 5, 1).unwrap()),
            ]
        );

        // DD-MON-YYYY (format20).
        let mon = DelimitedOptions {
            date_format: DateFormat::from_modeler_id("format20").unwrap(),
            ..DelimitedOptions::default()
        };
        let table = read("d\n15-Feb-1997\n", &mon);
        assert_eq!(
            column(&table, "d"),
            [Value::Date(calendar::days_from_civil(1997, 2, 15).unwrap())]
        );
    }
}
