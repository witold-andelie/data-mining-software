//! Field metadata: storage, measurement level, role, values and missing-value definitions.

use crate::value::Value;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

/// How values are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Storage {
    String,
    Integer,
    Real,
    Date,
    Time,
    Timestamp,
    List,
    /// Not yet known (for example an empty column read without type hints).
    Unknown,
}

/// What the values mean (Modeler's "measurement level" or "type").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Measurement {
    /// Not yet instantiated.
    Default,
    Continuous,
    /// Categorical whose values are not yet read (`Nominal`, `Ordinal` or `Flag` once known).
    Categorical,
    Flag,
    Nominal,
    Ordinal,
    Typeless,
    Collection,
    Geospatial,
}

/// How modelling nodes use a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Input,
    Target,
    Both,
    None,
    Partition,
    Split,
    Frequency,
    RecordId,
}

/// Known values of a field, once instantiated.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ValueSpec {
    #[default]
    Unknown,
    /// Continuous fields: observed or declared bounds.
    Range { low: Value, high: Value },
    /// Nominal and ordinal fields, in order.
    Categories(Vec<Value>),
    Flag {
        true_value: Value,
        false_value: Value,
    },
}

/// What a Type node does with values outside the declared values (`check` option).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CheckMode {
    #[default]
    None,
    Nullify,
    Coerce,
    Discard,
    Warn,
    Abort,
}

/// User-defined missing values ("blanks"). System nulls are always missing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MissingSpec {
    pub values: Vec<Value>,
    /// Inclusive ranges.
    pub ranges: Vec<(Value, Value)>,
    /// Treat the system null as blank.
    pub null_is_blank: bool,
    /// Treat empty and white-space-only strings as blank.
    pub whitespace_is_blank: bool,
}

impl MissingSpec {
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
            && self.ranges.is_empty()
            && !self.null_is_blank
            && !self.whitespace_is_blank
    }

    /// Whether `value` is a blank under this definition (CLEM `@BLANK`).
    pub fn is_blank(&self, value: &Value) -> bool {
        match value {
            Value::Null => return self.null_is_blank,
            Value::String(text) if self.whitespace_is_blank && text.trim().is_empty() => {
                return true;
            }
            _ => {}
        }
        self.values
            .iter()
            .any(|blank| blank.total_cmp(value) == Ordering::Equal)
            || self.ranges.iter().any(|(low, high)| {
                low.total_cmp(value) != Ordering::Greater
                    && value.total_cmp(high) != Ordering::Greater
                    && same_kind(low, value)
            })
    }
}

/// Range checks only apply to values of the range's own kind (numbers with numbers, ...).
fn same_kind(a: &Value, b: &Value) -> bool {
    matches!(
        (a, b),
        (
            Value::Integer(_) | Value::Real(_),
            Value::Integer(_) | Value::Real(_)
        ) | (Value::String(_), Value::String(_))
            | (Value::Date(_), Value::Date(_))
            | (Value::Time(_), Value::Time(_))
            | (Value::Timestamp(_), Value::Timestamp(_))
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub storage: Storage,
    pub measurement: Measurement,
    pub role: Role,
    pub values: ValueSpec,
    pub missing: MissingSpec,
    pub check: CheckMode,
    /// Field label (description).
    pub label: Option<String>,
    /// Value labels, in declaration order.
    pub value_labels: Vec<(Value, String)>,
    /// A field-level display format overriding the plain rendering of its values.
    pub display: Option<DisplayFormat>,
}

/// How Modeler prints a field's values in data exports when a node sets the field's format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayFormat {
    /// `MON YYYY` dates (`Dec 2019`): Time Intervals month labels (`misc_ti_months`).
    MonthYear,
    /// Timestamps with a dotted time, `YYYY-MM-DD HH.MM.SS` (Modeler `format10`;
    /// `misc_ti_workseconds`: `2020-02-28 08.05.47`).
    DottedTimestamp,
}

impl DisplayFormat {
    /// The value as Modeler prints it, or `None` to fall back to the plain rendering.
    pub fn render(self, value: &Value) -> Option<String> {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        match (self, value) {
            (DisplayFormat::MonthYear, Value::Date(days)) => {
                let (year, month, _) = crate::calendar::civil_from_days(*days)?;
                let name = MONTHS.get(usize::try_from(month).ok()?.checked_sub(1)?)?;
                Some(format!("{name} {year}"))
            }
            (DisplayFormat::DottedTimestamp, Value::Timestamp(micros)) => {
                let (days, time) = crate::calendar::split_timestamp(*micros);
                let (year, month, day) = crate::calendar::civil_from_days(days)?;
                let (h, m, s, us) = crate::calendar::time_from_micros(time)?;
                if us != 0 {
                    return None;
                }
                Some(format!(
                    "{year:04}-{month:02}-{day:02} {h:02}.{m:02}.{s:02}"
                ))
            }
            _ => None,
        }
    }
}

impl Field {
    /// A field with only a name and storage; everything else is unknown or default.
    pub fn new(name: impl Into<String>, storage: Storage) -> Field {
        Field {
            name: name.into(),
            storage,
            measurement: Measurement::Default,
            role: Role::Input,
            values: ValueSpec::Unknown,
            missing: MissingSpec::default(),
            check: CheckMode::None,
            label: None,
            value_labels: Vec::new(),
            display: None,
        }
    }

    /// System null or a blank.
    pub fn is_missing(&self, value: &Value) -> bool {
        value.is_null() || self.missing.is_blank(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    DuplicateField(String),
    UnknownField(String),
    RowWidth {
        row: usize,
        expected: usize,
        found: usize,
    },
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::DuplicateField(name) => write!(f, "duplicate field name {name:?}"),
            ModelError::UnknownField(name) => write!(f, "unknown field {name:?}"),
            ModelError::RowWidth {
                row,
                expected,
                found,
            } => write!(f, "row {row} has {found} values, expected {expected}"),
        }
    }
}

impl std::error::Error for ModelError {}

/// The ordered fields of a data set, with unique names.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DataModel {
    fields: Vec<Field>,
    index: HashMap<String, usize>,
}

impl DataModel {
    pub fn new(fields: Vec<Field>) -> Result<DataModel, ModelError> {
        let mut index = HashMap::with_capacity(fields.len());
        for (position, field) in fields.iter().enumerate() {
            if index.insert(field.name.clone(), position).is_some() {
                return Err(ModelError::DuplicateField(field.name.clone()));
            }
        }
        Ok(DataModel { fields, index })
    }

    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn position(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }

    pub fn field(&self, name: &str) -> Option<&Field> {
        self.position(name)
            .and_then(|position| self.fields.get(position))
    }

    /// Replaces the metadata of an existing field (the name must stay the same).
    pub fn update(&mut self, field: Field) -> Result<(), ModelError> {
        let position = self
            .position(&field.name)
            .ok_or_else(|| ModelError::UnknownField(field.name.clone()))?;
        if let Some(slot) = self.fields.get_mut(position) {
            *slot = field;
        }
        Ok(())
    }

    pub fn into_fields(self) -> Vec<Field> {
        self.fields
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blanks_by_value_range_and_whitespace() {
        let spec = MissingSpec {
            values: vec![Value::Integer(-99)],
            ranges: vec![(Value::Integer(900), Value::Integer(999))],
            null_is_blank: false,
            whitespace_is_blank: true,
        };
        assert!(spec.is_blank(&Value::Integer(-99)));
        assert!(spec.is_blank(&Value::Real(-99.0)));
        assert!(spec.is_blank(&Value::Integer(950)));
        assert!(!spec.is_blank(&Value::Integer(1000)));
        assert!(spec.is_blank(&Value::string("  ")));
        assert!(!spec.is_blank(&Value::string("x")));
        assert!(!spec.is_blank(&Value::string("950")));
        assert!(!spec.is_blank(&Value::Null));
        assert!(
            MissingSpec {
                null_is_blank: true,
                ..MissingSpec::default()
            }
            .is_blank(&Value::Null)
        );
        let field = Field {
            missing: spec,
            ..Field::new("f", Storage::Integer)
        };
        assert!(field.is_missing(&Value::Null));
    }

    #[test]
    fn data_model_rejects_duplicates_and_finds_fields() {
        let model = DataModel::new(vec![
            Field::new("a", Storage::Integer),
            Field::new("b", Storage::String),
        ])
        .unwrap();
        assert_eq!(model.position("b"), Some(1));
        assert_eq!(model.field("a").map(|f| f.storage), Some(Storage::Integer));
        assert_eq!(
            DataModel::new(vec![
                Field::new("a", Storage::Integer),
                Field::new("a", Storage::Real)
            ])
            .unwrap_err(),
            ModelError::DuplicateField("a".into())
        );
    }
}
