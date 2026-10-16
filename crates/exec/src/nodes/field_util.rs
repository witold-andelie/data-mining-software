//! Helpers shared by the Field Operations nodes (Derive, Filler, Reclassify, Filter, Partition):
//! expression parsing and evaluation with node-level errors, storage inference for computed
//! columns, and building the output table.

use crate::error::OperatorError;
use crate::settings;
use dms_clem::{Bindings, Compiled, Datum, Expr, compile, compile_condition, parse};
use dms_strfmt::fastinfoset::Element;
use dms_types::{DataModel, Field, Measurement, Storage, Table, Value};

/// Parses a CLEM setting; an empty (or blank) setting is `None`.
pub(crate) fn parse_optional(
    setting: &str,
    text: Option<String>,
) -> Result<Option<Expr>, OperatorError> {
    match text.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(source) => parse(source)
            .map(Some)
            .map_err(|error| OperatorError::Settings(format!("{setting}: {error}"))),
    }
}

/// Parses a CLEM setting that must not be empty.
pub(crate) fn parse_required(setting: &str, text: Option<String>) -> Result<Expr, OperatorError> {
    parse_optional(setting, text)?
        .ok_or_else(|| OperatorError::Settings(format!("{setting}: expression is empty")))
}

/// Binds an expression to the input's data model.
pub(crate) fn bind(
    setting: &str,
    expr: &Expr,
    model: &DataModel,
    current_field: Option<usize>,
) -> Result<Compiled, OperatorError> {
    compile(
        expr,
        &Bindings {
            model,
            current_field,
        },
    )
    .map_err(|error| OperatorError::Data(format!("{setting}: {error}")))
}

/// Binds an expression used as a condition: a flag field stands for its truth value.
pub(crate) fn bind_condition(
    setting: &str,
    expr: &Expr,
    model: &DataModel,
    current_field: Option<usize>,
) -> Result<Compiled, OperatorError> {
    compile_condition(
        expr,
        &Bindings {
            model,
            current_field,
        },
    )
    .map_err(|error| OperatorError::Data(format!("{setting}: {error}")))
}

/// The value an expression stores in a field. A condition is no field value: Modeler rejects
/// a formula with a Boolean result (`Invalid type for field: BOOLEAN`, oracle rule 32).
pub(crate) fn field_value(setting: &str, datum: Datum, index: u64) -> Result<Value, OperatorError> {
    match datum {
        Datum::Value(value) => Ok(value),
        Datum::Bool(_) => Err(OperatorError::Data(format!(
            "{setting}, record {index}: a condition cannot be stored in a field; use a flag \
             Derive or `if ... then ... else ... endif`"
        ))),
    }
}

/// Evaluates on one record; failures name the record (1-based).
pub(crate) fn eval(
    setting: &str,
    compiled: &Compiled,
    row: &[Value],
    index: u64,
) -> Result<Datum, OperatorError> {
    compiled
        .eval(row, index)
        .map_err(|error| OperatorError::Data(format!("{setting}, record {index}: {error}")))
}

/// Evaluates a condition on one record; null counts as false.
pub(crate) fn eval_condition(
    setting: &str,
    compiled: &Compiled,
    row: &[Value],
    index: u64,
) -> Result<bool, OperatorError> {
    let datum = eval(setting, compiled, row, index)?;
    datum
        .truth()
        .map(|truth| truth == Some(true))
        .map_err(|error| OperatorError::Data(format!("{setting}, record {index}: {error}")))
}

/// 1-based record number for `@INDEX`.
pub(crate) fn record_number(position: usize) -> u64 {
    u64::try_from(position).map_or(u64::MAX, |p| p.saturating_add(1))
}

/// The `<field>` names listed at `path`.
pub(crate) fn field_list(settings: Option<&Element>, path: &str) -> Vec<String> {
    settings::children(settings, path, "field")
        .into_iter()
        .map(|field| field.text())
        .collect()
}

/// Name of a field derived from `base` with an extension (multiple-field modes).
pub(crate) fn extended_name(base: &str, extension: &str, prefix: bool) -> String {
    if prefix {
        format!("{extension}{base}")
    } else {
        format!("{base}{extension}")
    }
}

/// The storage that holds all `values` (nulls ignored): integers with reals give real; mixtures
/// of other kinds have none. `Unknown` when every value is null.
pub(crate) fn infer_storage(values: &[Value]) -> Option<Storage> {
    let mut found: Option<Storage> = None;
    for value in values {
        let Some(kind) = value.storage() else {
            continue;
        };
        found = match (found, kind) {
            (None, kind) => Some(kind),
            (Some(a), b) if a == b => Some(a),
            (Some(Storage::Integer), Storage::Real) | (Some(Storage::Real), Storage::Integer) => {
                Some(Storage::Real)
            }
            _ => return None,
        };
    }
    Some(found.unwrap_or(Storage::Unknown))
}

/// Integers become reals when the column storage is real.
pub(crate) fn coerce_column(values: &mut [Value], storage: Storage) {
    if storage == Storage::Real {
        for value in values.iter_mut() {
            if let Value::Integer(i) = value {
                *value = Value::Real(*i as f64);
            }
        }
    }
}

/// Storage and values of literal settings text (set values, flag values): all integers give
/// integers, all numbers reals, anything else strings.
pub(crate) fn literal_values(texts: &[&str]) -> (Storage, Vec<Value>) {
    let integers: Option<Vec<i64>> = texts.iter().map(|t| t.trim().parse().ok()).collect();
    if let Some(integers) = integers {
        return (
            Storage::Integer,
            integers.into_iter().map(Value::Integer).collect(),
        );
    }
    let reals: Option<Vec<f64>> = texts
        .iter()
        .map(|t| t.trim().parse::<f64>().ok().filter(|r| r.is_finite()))
        .collect();
    if let Some(reals) = reals {
        return (Storage::Real, reals.into_iter().map(Value::Real).collect());
    }
    (
        Storage::String,
        texts.iter().map(|t| Value::string(*t)).collect(),
    )
}

/// A computed column to add to (or replace in) a table.
pub(crate) struct Column {
    pub field: Field,
    pub values: Vec<Value>,
}

impl Column {
    /// Infers storage from the values; mixtures of unrelated kinds are a data error.
    pub fn inferred(
        name: &str,
        measurement: Measurement,
        mut values: Vec<Value>,
    ) -> Result<Column, OperatorError> {
        let storage = infer_storage(&values).ok_or_else(|| {
            OperatorError::Data(format!(
                "{name}: the expression produces values of different kinds"
            ))
        })?;
        coerce_column(&mut values, storage);
        let mut field = Field::new(name, storage);
        field.measurement = measurement;
        Ok(Column { field, values })
    }
}

/// Adds the columns after the existing fields; a column named like an existing field replaces
/// that field in place.
pub(crate) fn with_columns(input: &Table, columns: Vec<Column>) -> Result<Table, OperatorError> {
    let mut fields: Vec<Field> = input.model().fields().to_vec();
    let mut targets = Vec::with_capacity(columns.len());
    for column in &columns {
        match fields.iter().position(|f| f.name == column.field.name) {
            Some(position) => {
                if let Some(slot) = fields.get_mut(position) {
                    *slot = column.field.clone();
                }
                targets.push(position);
            }
            None => {
                fields.push(column.field.clone());
                targets.push(fields.len().saturating_sub(1));
            }
        }
    }
    let width = fields.len();
    let rows = input
        .rows()
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let mut out = row.clone();
            out.resize(width, Value::Null);
            for (column, target) in columns.iter().zip(&targets) {
                if let (Some(slot), Some(value)) = (out.get_mut(*target), column.values.get(index))
                {
                    *slot = value.clone();
                }
            }
            out
        })
        .collect();
    let model = DataModel::new(fields).map_err(|e| OperatorError::Data(e.to_string()))?;
    Table::new(model, rows).map_err(|e| OperatorError::Data(e.to_string()))
}

/// Measurement level from the saved `...FieldMeasure` text.
pub(crate) fn measurement(text: Option<&str>) -> Measurement {
    match text.map(str::trim) {
        Some("range") => Measurement::Continuous,
        Some("flag") => Measurement::Flag,
        Some("set") => Measurement::Nominal,
        Some("orderedSet") => Measurement::Ordinal,
        Some("typeless") => Measurement::Typeless,
        Some("discrete") => Measurement::Categorical,
        Some("collection") => Measurement::Collection,
        Some("geospatial") => Measurement::Geospatial,
        _ => Measurement::Default,
    }
}

/// Position of a named input field.
pub(crate) fn position(model: &DataModel, name: &str) -> Result<usize, OperatorError> {
    model
        .position(name)
        .ok_or_else(|| OperatorError::Data(format!("input has no field {name:?}")))
}
