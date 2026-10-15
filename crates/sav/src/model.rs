//! Public data model.

use crate::bytes::Endian;

/// Limits that bound memory and work for untrusted input.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Maximum number of variable records (physical, including long-string continuations).
    pub max_variables: usize,
    /// Maximum number of cells (cases x variables) materialised.
    pub max_cells: u64,
    /// Maximum total decompressed size of ZLIB data, in bytes.
    pub max_decompressed_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_variables: 200_000,
            max_cells: 200_000_000,
            max_decompressed_bytes: 4 << 30,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Bytecode,
    Zlib,
}

#[derive(Debug, Clone)]
pub struct Header {
    pub product: String,
    pub layout_code: i32,
    pub endianness: Endian,
    pub compression: Compression,
    /// Raw `nominal_case_size`; unreliable in practice.
    pub nominal_case_size: i32,
    /// Index into [`SavFile::variables`] of the weight variable.
    pub weight_variable: Option<usize>,
    /// Case count from the header (or the extended-number-of-cases record), if known.
    pub case_count: Option<u64>,
    pub bias: f64,
    pub creation_date: String,
    pub creation_time: String,
    pub file_label: String,
}

/// A cell value. System-missing is kept distinct; user-missing values stay `Number`/`String`
/// (see [`Variable::missing`]).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Number(f64),
    SystemMissing,
    String(String),
}

/// An output format (`print`/`write`): type code, field width, decimals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    pub type_code: u8,
    pub width: u8,
    pub decimals: u8,
}

impl Format {
    pub(crate) fn from_raw(raw: i32) -> Self {
        let b = raw.to_le_bytes();
        Self {
            decimals: b[0],
            width: b[1],
            type_code: b[2],
        }
    }

    /// Date/time formats whose values are seconds since 1582-10-14 00:00:00 (DATE, TIME,
    /// DATETIME, ADATE, JDATE, DTIME, MOYR, QYR, WKYR, EDATE, SDATE, MTIME, YMDHMS).
    pub fn is_date_time(&self) -> bool {
        matches!(self.type_code, 20..=25 | 28..=30 | 38..=41)
    }

    /// WKDAY and MONTH: calendar-part formats (values are day-of-week / month numbers, not
    /// seconds; this is inferred, not verified).
    pub fn is_calendar_part(&self) -> bool {
        matches!(self.type_code, 26 | 27)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measure {
    Unknown,
    Nominal,
    Ordinal,
    Scale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Right,
    Centre,
}

/// User-missing specification. Range bounds are raw (LOWEST/HIGHEST appear as the file's
/// sentinel values, normally -f64::MAX / f64::MAX).
#[derive(Debug, Clone, PartialEq)]
pub enum MissingValues {
    None,
    Discrete(Vec<Value>),
    Range { low: f64, high: f64 },
    RangeAndDiscrete { low: f64, high: f64, value: f64 },
}

#[derive(Debug, Clone)]
pub struct Variable {
    /// Long name (subtype 13) or the short name when there is none.
    pub name: String,
    pub short_name: String,
    pub label: Option<String>,
    /// 0 = numeric, otherwise the string width in bytes (up to 32767 for very long strings).
    pub width: usize,
    pub print_format: Format,
    pub write_format: Format,
    pub measure: Measure,
    pub alignment: Option<Alignment>,
    pub display_width: Option<u32>,
    pub missing: MissingValues,
}

impl Variable {
    pub fn is_string(&self) -> bool {
        self.width > 0
    }
    pub fn is_date_time(&self) -> bool {
        self.print_format.is_date_time()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValueLabel {
    pub value: Value,
    pub label: String,
}

/// A set of value labels applied to one or more variables.
#[derive(Debug, Clone, PartialEq)]
pub struct ValueLabelSet {
    /// Indices into [`SavFile::variables`].
    pub variables: Vec<usize>,
    pub labels: Vec<ValueLabel>,
}

/// An extension record kept uninterpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawExtension {
    pub subtype: i32,
    pub size: i32,
    pub count: i32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SavFile {
    pub header: Header,
    pub variables: Vec<Variable>,
    pub value_labels: Vec<ValueLabelSet>,
    pub documents: Vec<String>,
    /// Encoding name declared by the file (subtype 20), if present.
    pub encoding: Option<String>,
    /// Name of the encoding actually used to decode strings.
    pub encoding_used: String,
    pub unknown_extensions: Vec<RawExtension>,
    /// Recoverable oddities (ignored malformed extensions, unknown encodings, ...).
    pub warnings: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

impl SavFile {
    /// Value label sets that apply to variable `index`.
    pub fn labels_for(&self, index: usize) -> impl Iterator<Item = &ValueLabelSet> {
        self.value_labels
            .iter()
            .filter(move |set| set.variables.contains(&index))
    }
}
