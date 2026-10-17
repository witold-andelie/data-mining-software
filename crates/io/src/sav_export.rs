//! Tables to IBM SPSS Statistics system files.
//!
//! The byte rules below are what `oracle/exports/stat_default` and
//! `oracle/exports/stat_names_as_labels` show. Both files are bytecode-compressed UTF-8 system
//! files written by Modeler's Statistics Export node from `corpus/exports/export_main.csv`.
//!
//! - String width is `max(6, chars * 4)` and at most 32767 (`stat_default`: `grp` and `flg` are
//!   one character and width 6; `s` is 22 characters and width 88). Width is bytes, counted so a
//!   non-ASCII character can expand to 4 UTF-8 bytes. Empty and null strings are spaces, not
//!   system-missing.
//! - Display width is `min(string width, 50)` for strings and 8 otherwise (`stat_default`: `s` is
//!   50, `grp` is 6, `rv` is 8 even though its print format is F21.15). Strings are left-aligned;
//!   numbers and dates are right-aligned.
//! - Measurement: ordinal stays ordinal; nominal, flag, typeless and categorical are nominal;
//!   continuous is scale; a default level is scale for numbers and nominal for strings. Date,
//!   time and timestamp storage is scale (`stat_default`: `s` is typeless and nominal, `dt` is
//!   scale).
//! - Print and write formats are the same. Integers are F10.0 (`id`, `iv`). Reals are
//!   F{places+6}.{places} (`rv` is F21.15 when the stream has 15 decimal places). Strings are
//!   A{width}. A date whose stream `dateFormat` is `format23` (YYYY-MM-DD, also the default) is
//!   SDATE10 (`dt`). The SPSS value is seconds since 1582-10-14 (`2020-01-15` is
//!   `13798425600`).
//! - Names and labels (`spssExportFieldType` `names`, `stat_default`): the short name is the
//!   field name in ASCII upper case, truncated to 8, and the long name is the field name. The
//!   variable label is the field description, omitted when that description is empty (`id` has
//!   none, `grp` is "Group").
//! - Names as labels (`labels`, `stat_names_as_labels`): short and long names are `VAR00001`,
//!   `VAR00002`, ... and the variable label is the field name. The description is not used. The
//!   long-name record is still written when the two names are equal.
//! - Value labels, only when the field has none of its own. A string set gets a blank value
//!   first and then each category labelled with itself (`grp`: blank, A, B, C, D). A string flag
//!   gets the blank and the true value only (`flg`: blank and T, not F). Typeless and continuous
//!   fields get no value-label record (`s`, `id`, `rv`, `dt`). Strings wider than 8 bytes use
//!   extension subtype 21; no probe string with labels is that wide.
//! - User-missing values are written (up to three discrete values, one range, or a range plus one
//!   discrete value). `null_is_blank` and `whitespace_is_blank` are not written; both probes have
//!   no user-missing values. A string wider than 8 bytes stores them in subtype 22.
//! - Roles are subtype 18, `name:$@Role('N'\n)` joined by `/` with no trailing slash, using the
//!   long name (`stat_default`: `id` and `s` are 3, the others 0; `stat_names_as_labels` uses the
//!   `VAR0000n` names and the same codes). 0 input, 1 target, 2 both, 3 none, 4 partition,
//!   5 split. A typeless field is written as none even when its direction is input (`s` in
//!   `stat_default`, `VAR00005` in `stat_names_as_labels`). Frequency and record-id have no
//!   observed code.
//! - Whole numbers in [-99, 151] are bytecode integers, including 0 (`stat_default` rows 2, 6,
//!   12 and the `iv` value -10). Anything else numeric is raw little-endian f64, and system-missing
//!   is code 255. The file ends on the last element; there is no end-of-file code 252.
//!
//! Very long strings (width > 255) are not in either probe. Segments follow `n = (w + 251) / 252`
//! and the payload matches what [`dms_sav::read`] concatenates. Time, timestamp, list and unknown
//! storage are refused.

// `dms-exec` integration tests cannot name `dms-sav` (it is not a direct dependency).
// Re-export the reader they use to compare an export with the oracle.
use dms_sav::{
    Alignment, Format, Measure, MissingValues, OutVariable, Value as SavValue, ValueLabel,
    write_bytes,
};
pub use dms_sav::{Limits, SavFile, Value as SystemValue, read};
use dms_types::{Field, Measurement, Role, Storage, Table, Value, ValueSpec};
use std::collections::HashSet;

/// How field names are stored. The file values are `names` and `labels`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NameMode {
    /// Short name from the field name; the description becomes the variable label.
    #[default]
    NamesAndLabels,
    /// `VAR00001` style names; the field name becomes the variable label.
    NamesAsLabels,
}

/// Stream-level knobs that are not on the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportOptions {
    pub names: NameMode,
    /// Decimal places for real print formats. 15 writes F21.15.
    pub decimal_places: usize,
    /// Stream `dateFormat` id. Missing means `format23` (YYYY-MM-DD).
    pub date_format: Option<String>,
}

/// A table that cannot be written, or a format this exporter does not implement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavExportError {
    Settings(String),
    Data(String),
}

impl std::fmt::Display for SavExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SavExportError::Settings(message) | SavExportError::Data(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for SavExportError {}

/// Days from 1582-10-14 to 1970-01-01. `2020-01-15` is SPSS seconds `13798425600`.
const EPOCH_SHIFT_DAYS: i64 = 141_428;
const EXACT_INT: i64 = 1 << 53;

/// Writes `table` as a system file.
pub fn write_table(table: &Table, options: &ExportOptions) -> Result<Vec<u8>, SavExportError> {
    let mut variables = Vec::with_capacity(table.model().len());
    let mut used = HashSet::new();
    for (index, field) in table.model().fields().iter().enumerate() {
        variables.push(convert_field(table, index, field, options, &mut used)?);
    }
    let mut rows = Vec::with_capacity(table.row_count());
    for (row_index, row) in table.rows().iter().enumerate() {
        let mut out = Vec::with_capacity(variables.len());
        for (field, variable) in table.model().fields().iter().zip(&variables) {
            let value = row.get(out.len()).ok_or_else(|| {
                SavExportError::Data(format!("row {row_index} is shorter than the field list"))
            })?;
            out.push(convert_cell(field, variable.width, value)?);
        }
        rows.push(out);
    }
    write_bytes(&variables, &rows).map_err(|error| SavExportError::Data(error.to_string()))
}

fn convert_field(
    table: &Table,
    index: usize,
    field: &Field,
    options: &ExportOptions,
    used: &mut HashSet<String>,
) -> Result<OutVariable, SavExportError> {
    let width = match field.storage {
        Storage::String => string_width(table, index, &field.name)?,
        Storage::Integer | Storage::Real | Storage::Date => 0,
        Storage::Time => {
            return Err(SavExportError::Data(format!(
                "Statistics export: field {}: time storage is not implemented",
                field.name
            )));
        }
        Storage::Timestamp => {
            return Err(SavExportError::Data(format!(
                "Statistics export: field {}: timestamp storage is not implemented",
                field.name
            )));
        }
        Storage::List | Storage::Unknown => {
            return Err(SavExportError::Data(format!(
                "Statistics export: field {}: storage {:?} is not implemented",
                field.name, field.storage
            )));
        }
    };
    if field.storage == Storage::Date {
        date_format_ok(options)?;
    }
    let (short_name, long_name, label) = names_of(field, index, options.names, used)?;
    let print = print_format(field.storage, width, options)?;
    Ok(OutVariable {
        short_name,
        long_name,
        label,
        width,
        print_format: print,
        write_format: print,
        measure: measure_of(field),
        alignment: if field.storage == Storage::String {
            Alignment::Left
        } else {
            Alignment::Right
        },
        display_width: display_width(field.storage, width),
        missing: missing_of(field, width)?,
        value_labels: value_labels_of(field)?,
        role: role_code(field)?,
    })
}

fn names_of(
    field: &Field,
    index: usize,
    mode: NameMode,
    used: &mut HashSet<String>,
) -> Result<(String, String, Option<String>), SavExportError> {
    match mode {
        NameMode::NamesAsLabels => {
            let n = index.saturating_add(1);
            if n > 99_999 {
                return Err(SavExportError::Data(
                    "Statistics export: more than 99999 fields".into(),
                ));
            }
            let generated = format!("VAR{n:05}");
            if !used.insert(generated.clone()) {
                return Err(SavExportError::Data(format!(
                    "Statistics export: duplicate generated name {generated}"
                )));
            }
            let label = bounded_label(&field.name, &field.name)?;
            Ok((generated.clone(), generated, Some(label)))
        }
        NameMode::NamesAndLabels => {
            let short = allocate_short(&field.name, used)?;
            let long = long_name(&field.name)?;
            let label = field
                .label
                .as_deref()
                .filter(|text| !text.trim().is_empty())
                .map(|text| bounded_label(&field.name, text))
                .transpose()?;
            Ok((short, long, label))
        }
    }
}

fn allocate_short(name: &str, used: &mut HashSet<String>) -> Result<String, SavExportError> {
    if let Some(natural) = natural_short(name)
        && used.insert(natural.clone())
    {
        return Ok(natural);
    }
    for n in 1..=9_999_999u32 {
        let generated = format!("V{n:07}");
        if used.insert(generated.clone()) {
            return Ok(generated);
        }
    }
    Err(SavExportError::Data(format!(
        "Statistics export: no free short name for {name:?}"
    )))
}

fn natural_short(name: &str) -> Option<String> {
    if name.is_empty() || name.len() > 8 {
        return None;
    }
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        let upper = if ch.is_ascii_lowercase() {
            ch.to_ascii_uppercase()
        } else {
            ch
        };
        let byte = u8::try_from(u32::from(upper)).ok()?;
        let ok = byte.is_ascii_uppercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'@' | b'#' | b'$' | b'_' | b'.');
        if !ok {
            return None;
        }
        out.push(upper);
    }
    let first = out.as_bytes().first().copied()?;
    if first.is_ascii_uppercase() || first == b'@' {
        Some(out)
    } else {
        None
    }
}

fn long_name(name: &str) -> Result<String, SavExportError> {
    let text = truncate_bytes(name, 64);
    if text.is_empty() {
        return Err(SavExportError::Data(
            "Statistics export: a field name is empty".into(),
        ));
    }
    if text
        .bytes()
        .any(|byte| matches!(byte, b'\t' | b'=' | 0 | b'/' | b':' | b'\'' | b'\n'))
    {
        return Err(SavExportError::Data(format!(
            "Statistics export: field name {name:?} cannot be stored in a long-name or role record"
        )));
    }
    Ok(text.to_string())
}

fn bounded_label(field: &str, label: &str) -> Result<String, SavExportError> {
    let text = truncate_bytes(label, 255);
    if text.is_empty() {
        return Err(SavExportError::Data(format!(
            "Statistics export: field {field} has an empty label"
        )));
    }
    Ok(text.to_string())
}

fn truncate_bytes(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or("")
}

fn string_width(table: &Table, column: usize, name: &str) -> Result<usize, SavExportError> {
    let mut max_chars = 0usize;
    for (row_index, row) in table.rows().iter().enumerate() {
        let value = row.get(column).ok_or_else(|| {
            SavExportError::Data(format!("row {row_index} is shorter than the field list"))
        })?;
        match value {
            Value::Null => {}
            Value::String(text) => max_chars = max_chars.max(text.chars().count()),
            other => {
                return Err(SavExportError::Data(format!(
                    "Statistics export: field {name} row {row_index} is {other}, expected a string"
                )));
            }
        }
    }
    let expanded = max_chars.checked_mul(4).ok_or_else(|| {
        SavExportError::Data(format!("Statistics export: field {name} is too wide"))
    })?;
    if expanded > 32_767 {
        return Err(SavExportError::Data(format!(
            "Statistics export: field {name} needs {expanded} bytes; a system file holds 32767"
        )));
    }
    Ok(expanded.max(6))
}

fn print_format(
    storage: Storage,
    width: usize,
    options: &ExportOptions,
) -> Result<Format, SavExportError> {
    match storage {
        Storage::Integer => Ok(Format {
            type_code: 5,
            width: 10,
            decimals: 0,
        }),
        Storage::Real => {
            if options.decimal_places > 249 {
                return Err(SavExportError::Settings(format!(
                    "Statistics export: {} decimal places do not fit in an F format",
                    options.decimal_places
                )));
            }
            let format_width = options.decimal_places.saturating_add(6);
            let width = u8::try_from(format_width).map_err(|_| {
                SavExportError::Settings("Statistics export: real format width does not fit".into())
            })?;
            let decimals = u8::try_from(options.decimal_places).map_err(|_| {
                SavExportError::Settings(
                    "Statistics export: decimal places do not fit in a format".into(),
                )
            })?;
            Ok(Format {
                type_code: 5,
                width,
                decimals,
            })
        }
        Storage::String => {
            let stored = width.min(255);
            let format_width = u8::try_from(stored).map_err(|_| {
                SavExportError::Data("Statistics export: string format width does not fit".into())
            })?;
            Ok(Format {
                type_code: 1,
                width: format_width,
                decimals: 0,
            })
        }
        Storage::Date => Ok(Format {
            type_code: 39,
            width: 10,
            decimals: 0,
        }),
        _ => Err(SavExportError::Data(
            "Statistics export: no print format for this storage".into(),
        )),
    }
}

fn date_format_ok(options: &ExportOptions) -> Result<(), SavExportError> {
    match options.date_format.as_deref().map(str::trim) {
        None | Some("") | Some("format23") => Ok(()),
        Some(other) => Err(SavExportError::Settings(format!(
            "Statistics export: date format {other:?} is not implemented (only format23, YYYY-MM-DD)"
        ))),
    }
}

fn display_width(storage: Storage, width: usize) -> u32 {
    if storage == Storage::String {
        u32::try_from(width.min(50)).unwrap_or(50)
    } else {
        8
    }
}

fn measure_of(field: &Field) -> Measure {
    if matches!(
        field.storage,
        Storage::Date | Storage::Time | Storage::Timestamp
    ) {
        return Measure::Scale;
    }
    match field.measurement {
        Measurement::Ordinal => Measure::Ordinal,
        Measurement::Nominal
        | Measurement::Flag
        | Measurement::Typeless
        | Measurement::Categorical
        | Measurement::Collection => Measure::Nominal,
        Measurement::Continuous => Measure::Scale,
        Measurement::Default | Measurement::Geospatial => {
            if field.storage == Storage::String {
                Measure::Nominal
            } else {
                Measure::Scale
            }
        }
    }
}

fn role_code(field: &Field) -> Result<i32, SavExportError> {
    // `stat_default` sets `s` to Input, and the file still stores role 3 because `s` is typeless.
    if field.measurement == Measurement::Typeless {
        return Ok(3);
    }
    match field.role {
        Role::Input => Ok(0),
        Role::Target => Ok(1),
        Role::Both => Ok(2),
        Role::None => Ok(3),
        Role::Partition => Ok(4),
        Role::Split => Ok(5),
        Role::Frequency | Role::RecordId => Err(SavExportError::Data(format!(
            "Statistics export: field {} has role {:?}, which has no observed SPSS code",
            field.name, field.role
        ))),
    }
}

fn missing_of(field: &Field, width: usize) -> Result<MissingValues, SavExportError> {
    let spec = &field.missing;
    if spec.values.is_empty() && spec.ranges.is_empty() {
        return Ok(MissingValues::None);
    }
    if field.storage == Storage::String {
        if !spec.ranges.is_empty() {
            return Err(SavExportError::Data(format!(
                "Statistics export: field {}: a string missing range is not implemented",
                field.name
            )));
        }
        if width <= 8 && spec.values.len() > 3 {
            return Err(SavExportError::Data(format!(
                "Statistics export: field {}: more than 3 short-string missing values",
                field.name
            )));
        }
        if spec.values.len() > 255 {
            return Err(SavExportError::Data(format!(
                "Statistics export: field {}: more than 255 missing values",
                field.name
            )));
        }
        let mut values = Vec::with_capacity(spec.values.len());
        for value in &spec.values {
            match value {
                Value::String(text) => values.push(SavValue::String(text.to_string())),
                Value::Null => values.push(SavValue::String(String::new())),
                _ => {
                    return Err(SavExportError::Data(format!(
                        "Statistics export: field {}: a string missing value is not a string",
                        field.name
                    )));
                }
            }
        }
        return Ok(MissingValues::Discrete(values));
    }
    if spec.ranges.len() > 1 {
        return Err(SavExportError::Data(format!(
            "Statistics export: field {}: more than one missing range is not implemented",
            field.name
        )));
    }
    let numbers = |value: &Value| number_value(field, value);
    match (spec.ranges.as_slice(), spec.values.len()) {
        ([], 0) => Ok(MissingValues::None),
        ([], count) if count <= 3 => {
            let mut values = Vec::with_capacity(count);
            for value in &spec.values {
                values.push(SavValue::Number(numbers(value)?));
            }
            Ok(MissingValues::Discrete(values))
        }
        ([], _) => Err(SavExportError::Data(format!(
            "Statistics export: field {}: more than 3 discrete missing values",
            field.name
        ))),
        ([(low, high)], 0) => Ok(MissingValues::Range {
            low: numbers(low)?,
            high: numbers(high)?,
        }),
        ([(low, high)], 1) => {
            let value = spec.values.first().ok_or_else(|| {
                SavExportError::Data(format!(
                    "Statistics export: field {}: missing the discrete value beside the range",
                    field.name
                ))
            })?;
            Ok(MissingValues::RangeAndDiscrete {
                low: numbers(low)?,
                high: numbers(high)?,
                value: numbers(value)?,
            })
        }
        _ => Err(SavExportError::Data(format!(
            "Statistics export: field {}: a missing range plus more than one value is not implemented",
            field.name
        ))),
    }
}

fn value_labels_of(field: &Field) -> Result<Vec<ValueLabel>, SavExportError> {
    if !field.value_labels.is_empty() {
        let mut labels = Vec::with_capacity(field.value_labels.len());
        for (value, label) in &field.value_labels {
            if label.len() > 255 {
                return Err(SavExportError::Data(format!(
                    "Statistics export: field {}: a value label is longer than 255 bytes",
                    field.name
                )));
            }
            labels.push(ValueLabel {
                value: labelled_value(field, value)?,
                label: label.clone(),
            });
        }
        return Ok(labels);
    }
    if field.storage != Storage::String {
        return Ok(Vec::new());
    }
    match &field.values {
        ValueSpec::Flag { true_value, .. } => {
            let mut labels = vec![blank_label()];
            if let Value::String(text) = true_value
                && !text.is_empty()
            {
                labels.push(self_label(text));
            }
            Ok(labels)
        }
        ValueSpec::Categories(categories) => {
            let mut labels = vec![blank_label()];
            for category in categories {
                if let Value::String(text) = category
                    && !text.is_empty()
                {
                    labels.push(self_label(text));
                }
            }
            Ok(labels)
        }
        _ => Ok(Vec::new()),
    }
}

fn blank_label() -> ValueLabel {
    ValueLabel {
        value: SavValue::String(String::new()),
        label: String::new(),
    }
}

fn self_label(text: &str) -> ValueLabel {
    ValueLabel {
        value: SavValue::String(text.to_string()),
        label: text.to_string(),
    }
}

fn labelled_value(field: &Field, value: &Value) -> Result<SavValue, SavExportError> {
    if field.storage == Storage::String {
        return match value {
            Value::String(text) => Ok(SavValue::String(text.to_string())),
            Value::Null => Ok(SavValue::String(String::new())),
            _ => Err(SavExportError::Data(format!(
                "Statistics export: field {}: a value label key is not a string",
                field.name
            ))),
        };
    }
    match value {
        Value::Null => Ok(SavValue::SystemMissing),
        _ => Ok(SavValue::Number(number_value(field, value)?)),
    }
}

fn convert_cell(field: &Field, width: usize, value: &Value) -> Result<SavValue, SavExportError> {
    match (field.storage, value) {
        (_, Value::Null) if field.storage == Storage::String => Ok(SavValue::String(String::new())),
        (_, Value::Null) => Ok(SavValue::SystemMissing),
        (Storage::String, Value::String(text)) => {
            Ok(SavValue::String(truncate_bytes(text, width).to_string()))
        }
        (Storage::Integer | Storage::Real, Value::Integer(_) | Value::Real(_)) => {
            Ok(SavValue::Number(number_value(field, value)?))
        }
        (Storage::Date, Value::Date(days)) => Ok(SavValue::Number(spss_seconds(*days)?)),
        _ => Err(SavExportError::Data(format!(
            "Statistics export: field {} has a value that does not match its storage",
            field.name
        ))),
    }
}

fn number_value(field: &Field, value: &Value) -> Result<f64, SavExportError> {
    match value {
        Value::Integer(number) => exact_f64(*number, &field.name),
        Value::Real(number) => Ok(*number),
        Value::Date(days) => spss_seconds(*days),
        _ => Err(SavExportError::Data(format!(
            "Statistics export: field {}: expected a number, found {value}",
            field.name
        ))),
    }
}

fn exact_f64(value: i64, field: &str) -> Result<f64, SavExportError> {
    if !(-EXACT_INT..=EXACT_INT).contains(&value) {
        return Err(SavExportError::Data(format!(
            "Statistics export: field {field}: integer {value} is not an exact SPSS number"
        )));
    }
    Ok(value as f64)
}

fn spss_seconds(days: i64) -> Result<f64, SavExportError> {
    let shifted = days.checked_add(EPOCH_SHIFT_DAYS).ok_or_else(|| {
        SavExportError::Data("Statistics export: a date is outside the SPSS range".into())
    })?;
    let seconds = shifted.checked_mul(86_400).ok_or_else(|| {
        SavExportError::Data("Statistics export: a date is outside the SPSS range".into())
    })?;
    if !(-EXACT_INT..=EXACT_INT).contains(&seconds) {
        return Err(SavExportError::Data(
            "Statistics export: a date is outside the exact SPSS range".into(),
        ));
    }
    Ok(seconds as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dms_sav::{Limits, Measure};
    use dms_types::{DataModel, Field, Storage};

    fn field(name: &str, storage: Storage) -> Field {
        Field::new(name, storage)
    }

    fn options(names: NameMode) -> ExportOptions {
        ExportOptions {
            names,
            decimal_places: 15,
            date_format: Some("format23".into()),
        }
    }

    #[test]
    fn string_width_and_date_match_the_probe_rules() {
        let mut group = field("grp", Storage::String);
        group.measurement = Measurement::Nominal;
        group.values = ValueSpec::Categories(vec![Value::string("A"), Value::string("B")]);
        let mut text = field("s", Storage::String);
        text.measurement = Measurement::Typeless;
        text.label = Some("Text".into());
        let mut when = field("dt", Storage::Date);
        when.label = Some("When".into());
        when.role = Role::Input;
        let days = dms_types::calendar::days_from_civil(2020, 1, 15).unwrap();
        let model = DataModel::new(vec![group, text, when]).unwrap();
        let table = Table::new(
            model,
            vec![vec![
                Value::string("A"),
                Value::string("q and semi;colon,comma"),
                Value::Date(days),
            ]],
        )
        .unwrap();
        let bytes = write_table(&table, &options(NameMode::NamesAndLabels)).unwrap();
        let file = dms_sav::read(&bytes, &Limits::default()).unwrap();
        assert_eq!(file.variables[0].width, 6);
        assert_eq!(file.variables[1].width, 88);
        assert_eq!(file.variables[1].display_width, Some(50));
        assert_eq!(file.variables[1].measure, Measure::Nominal);
        assert_eq!(file.variables[1].label.as_deref(), Some("Text"));
        let roles = file
            .unknown_extensions
            .iter()
            .find(|record| record.subtype == 18)
            .map(|record| String::from_utf8_lossy(&record.data).into_owned())
            .unwrap_or_default();
        assert!(
            roles.contains("s:$@Role('3'\n)"),
            "typeless field should be role none, got {roles}"
        );
        assert_eq!(file.variables[2].print_format.type_code, 39);
        assert_eq!(file.rows[0][2], dms_sav::Value::Number(13_798_425_600.0));
        assert_eq!(file.variables[0].short_name, "GRP");
        assert_eq!(file.variables[0].name, "grp");
    }

    #[test]
    fn names_as_labels_uses_var0000n_and_the_field_name() {
        let model = DataModel::new(vec![field("id", Storage::Integer)]).unwrap();
        let table = Table::new(model, vec![vec![Value::Integer(1)]]).unwrap();
        let bytes = write_table(&table, &options(NameMode::NamesAsLabels)).unwrap();
        let file = dms_sav::read(&bytes, &Limits::default()).unwrap();
        assert_eq!(file.variables[0].short_name, "VAR00001");
        assert_eq!(file.variables[0].name, "VAR00001");
        assert_eq!(file.variables[0].label.as_deref(), Some("id"));
        assert_eq!(file.variables[0].print_format.width, 10);
        assert_eq!(file.variables[0].print_format.decimals, 0);
    }
}
