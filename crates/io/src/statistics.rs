//! SPSS Statistics files as tables, the way Modeler's Statistics File node presents them.
//!
//! Rules (observed with the oracle, `oracle/probes/sav_to_csv/*.fields.json`, 30 corpus files,
//! 1140 variables):
//!
//! - Numeric variables are `Real` storage whatever their format (including `F8.0`); only date
//!   formats change that: DATE/ADATE/JDATE/EDATE/SDATE/MOYR/QYR/WKYR -> `Date`, DATETIME/YMDHMS
//!   -> `Timestamp`, TIME/DTIME/MTIME -> `Time` (the last group is inferred, no corpus file has
//!   one). String variables are `String`.
//! - Measurement: SPSS scale -> range (`Continuous`), nominal -> set (`Nominal`), ordinal ->
//!   ordered set (`Ordinal`); an unknown level is range for numbers and set for strings.
//! - Role comes from the file's variable attribute `$@Role` (0 input, 1 target, 2 both, 3 none,
//!   4 partition, 5 split), input when absent.
//! - System-missing numbers are `Null`. User-missing values stay as read and become a
//!   [`MissingSpec`] on the field (null and, for strings, white space count as blank too).
//! - Values are left `Unknown` in the field: the Type node instantiates them.
//!
//! Not implemented: reading value labels as data, password-encrypted files, and storage from the
//! field format (`useFieldFormatForStorage`).

use dms_sav::{Measure, MissingValues, SavFile, Variable};
use dms_types::calendar::{MICROS_PER_DAY, MICROS_PER_SECOND};
use dms_types::{
    DataModel, Field, Measurement, MissingSpec, ModelError, Role, Storage, Table, Value,
};

/// `Import names` of the node: what field names are made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NameMode {
    /// Variable names, with variable labels as field labels.
    #[default]
    NamesAndLabels,
    /// Variable labels as names where a variable has one.
    LabelsAsNames,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatisticsOptions {
    pub names: NameMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatisticsError {
    Sav(String),
    Model(ModelError),
}

impl std::fmt::Display for StatisticsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StatisticsError::Sav(message) => write!(f, "{message}"),
            StatisticsError::Model(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for StatisticsError {}

/// Reads a `.sav`/`.zsav` file.
pub fn read_table(bytes: &[u8], options: StatisticsOptions) -> Result<Table, StatisticsError> {
    let file = dms_sav::read(bytes, &dms_sav::Limits::default())
        .map_err(|error| StatisticsError::Sav(error.to_string()))?;
    table_from_sav(&file, options)
}

/// Seconds between 1582-10-14 (the SPSS epoch) and 1970-01-01.
const EPOCH_SHIFT_DAYS: i64 = 141_428;

pub fn table_from_sav(
    file: &SavFile,
    options: StatisticsOptions,
) -> Result<Table, StatisticsError> {
    let roles = roles(file);
    let mut fields = Vec::with_capacity(file.variables.len());
    let mut storages = Vec::with_capacity(file.variables.len());
    for (index, variable) in file.variables.iter().enumerate() {
        let storage = storage_of(variable);
        storages.push(storage);
        let name = match (options.names, variable.label.as_deref()) {
            (NameMode::LabelsAsNames, Some(label)) if !label.trim().is_empty() => label.to_string(),
            _ => variable.name.clone(),
        };
        let mut field = Field::new(name, storage);
        field.measurement = measurement_of(variable);
        field.role = roles
            .get(&variable.name.to_lowercase())
            .copied()
            .unwrap_or(Role::Input);
        field.label = variable.label.clone().filter(|label| !label.is_empty());
        field.missing = missing_of(variable, storage);
        for set in file.labels_for(index) {
            for entry in &set.labels {
                field
                    .value_labels
                    .push((convert(&entry.value, storage), entry.label.clone()));
            }
        }
        fields.push(field);
    }
    let rows = file
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .zip(&storages)
                .map(|(value, storage)| convert(value, *storage))
                .collect()
        })
        .collect();
    let model = DataModel::new(fields).map_err(StatisticsError::Model)?;
    Table::new(model, rows).map_err(StatisticsError::Model)
}

fn storage_of(variable: &Variable) -> Storage {
    if variable.is_string() {
        return Storage::String;
    }
    let code = variable.print_format.type_code;
    match code {
        20 | 23 | 24 | 28..=30 | 38 | 39 => Storage::Date,
        22 | 41 => Storage::Timestamp,
        21 | 25 | 40 => Storage::Time,
        _ => Storage::Real,
    }
}

fn measurement_of(variable: &Variable) -> Measurement {
    match variable.measure {
        Measure::Scale => Measurement::Continuous,
        Measure::Nominal => Measurement::Nominal,
        Measure::Ordinal => Measurement::Ordinal,
        Measure::Unknown if variable.is_string() => Measurement::Nominal,
        Measure::Unknown => Measurement::Continuous,
    }
}

fn convert(value: &dms_sav::Value, storage: Storage) -> Value {
    match value {
        dms_sav::Value::SystemMissing => Value::Null,
        dms_sav::Value::String(text) => Value::string(text.as_str()),
        dms_sav::Value::Number(number) => {
            if !number.is_finite() {
                return Value::Null;
            }
            match storage {
                Storage::Date => date_value(*number),
                Storage::Timestamp => timestamp_value(*number),
                Storage::Time => time_value(*number),
                _ => Value::real(*number),
            }
        }
    }
}

/// Seconds since 1582-10-14 00:00:00 as whole days since 1970-01-01.
fn date_value(seconds: f64) -> Value {
    let days = (seconds / 86_400.0).floor();
    if !(-1.0e8..1.0e8).contains(&days) {
        return Value::Null;
    }
    // Bounded above, so the conversion cannot overflow.
    #[allow(clippy::cast_possible_truncation)]
    Value::Date(days as i64 - EPOCH_SHIFT_DAYS)
}

fn timestamp_value(seconds: f64) -> Value {
    if !(-1.0e13..1.0e13).contains(&seconds) {
        return Value::Null;
    }
    #[allow(clippy::cast_possible_truncation)]
    let micros = (seconds * 1_000_000.0).round() as i64;
    match micros.checked_sub(EPOCH_SHIFT_DAYS * MICROS_PER_DAY) {
        Some(shifted) => Value::Timestamp(shifted),
        None => Value::Null,
    }
}

/// TIME values are durations in seconds; only a time of day is representable.
fn time_value(seconds: f64) -> Value {
    if !(0.0..86_400.0).contains(&seconds) {
        return Value::Null;
    }
    #[allow(clippy::cast_possible_truncation)]
    let micros = (seconds * MICROS_PER_SECOND as f64).round() as i64;
    if micros >= MICROS_PER_DAY {
        return Value::Null;
    }
    Value::Time(micros)
}

fn missing_of(variable: &Variable, storage: Storage) -> MissingSpec {
    let numeric = |number: f64| -> Value {
        match storage {
            Storage::Date | Storage::Timestamp | Storage::Time => {
                convert(&dms_sav::Value::Number(number), storage)
            }
            _ => Value::real(number),
        }
    };
    let discrete = |value: &dms_sav::Value| convert(value, storage);
    let (values, ranges): (Vec<Value>, Vec<(Value, Value)>) = match &variable.missing {
        MissingValues::None => return MissingSpec::default(),
        MissingValues::Discrete(values) => (values.iter().map(discrete).collect(), Vec::new()),
        MissingValues::Range { low, high } => (Vec::new(), vec![(numeric(*low), numeric(*high))]),
        MissingValues::RangeAndDiscrete { low, high, value } => {
            (vec![numeric(*value)], vec![(numeric(*low), numeric(*high))])
        }
    };
    MissingSpec {
        values,
        ranges,
        null_is_blank: true,
        whitespace_is_blank: variable.is_string(),
    }
}

/// `$@Role` attributes (extension record 18): `name:$@Role('0'\n)/name2:...`, lower-cased name
/// to role.
fn roles(file: &SavFile) -> std::collections::HashMap<String, Role> {
    let mut roles = std::collections::HashMap::new();
    for extension in file.unknown_extensions.iter().filter(|e| e.subtype == 18) {
        let text = String::from_utf8_lossy(&extension.data);
        for record in text.split('/') {
            let Some((name, attributes)) = record.split_once(':') else {
                continue;
            };
            let Some(start) = attributes
                .find("$@Role('")
                .or_else(|| attributes.find("$Role('"))
            else {
                continue;
            };
            let value: String = attributes
                .get(start..)
                .and_then(|rest| rest.split_once('\''))
                .map(|(_, rest)| rest.chars().take_while(|c| c.is_ascii_digit()).collect())
                .unwrap_or_default();
            let role = match value.as_str() {
                "0" => Role::Input,
                "1" => Role::Target,
                "2" => Role::Both,
                "3" => Role::None,
                "4" => Role::Partition,
                "5" => Role::Split,
                _ => continue,
            };
            roles.insert(name.trim().to_lowercase(), role);
        }
    }
    roles
}

#[cfg(test)]
mod tests {
    use super::*;
    use dms_types::calendar;

    #[test]
    fn spss_epoch_matches_the_calendar() {
        assert_eq!(
            calendar::days_from_civil(1582, 10, 14),
            Some(-EPOCH_SHIFT_DAYS)
        );
        // 1970-01-01 is 12,219,379,200 s after the SPSS epoch.
        assert_eq!(date_value(12_219_379_200.0), Value::Date(0));
        assert_eq!(date_value(12_219_379_200.0 + 86_399.0), Value::Date(0));
        assert_eq!(
            timestamp_value(12_219_379_200.0 + 1.5),
            Value::Timestamp(1_500_000)
        );
    }

    #[test]
    fn times_are_times_of_day() {
        assert_eq!(time_value(3723.0), Value::Time(3_723_000_000));
        assert_eq!(time_value(-1.0), Value::Null);
        assert_eq!(time_value(86_400.0), Value::Null);
    }

    #[test]
    fn parses_role_attributes() {
        let file = SavFile {
            header: dms_sav::Header {
                product: String::new(),
                layout_code: 2,
                endianness: dms_sav::Endian::Little,
                compression: dms_sav::Compression::None,
                nominal_case_size: 0,
                weight_variable: None,
                case_count: None,
                bias: 100.0,
                creation_date: String::new(),
                creation_time: String::new(),
                file_label: String::new(),
            },
            variables: Vec::new(),
            value_labels: Vec::new(),
            documents: Vec::new(),
            encoding: None,
            encoding_used: String::new(),
            unknown_extensions: vec![dms_sav::RawExtension {
                subtype: 18,
                size: 1,
                count: 0,
                data: b"Age:$@Role('0'\n)/Id:$@Role('3'\n)/Part:$@Role('4'\n)".to_vec(),
            }],
            warnings: Vec::new(),
            rows: Vec::new(),
        };
        let found = roles(&file);
        assert_eq!(found.get("age"), Some(&Role::Input));
        assert_eq!(found.get("id"), Some(&Role::None));
        assert_eq!(found.get("part"), Some(&Role::Partition));
    }
}
