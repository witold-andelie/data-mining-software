//! Tables to and from Arrow IPC files (the format both sides read without extra packages
//! beyond pyarrow). Storage maps one to one; measurement and role travel as field metadata and
//! come back from the input model, because pandas does not keep Arrow field metadata.

use crate::BridgeError;
use arrow_array::builder::{
    Date32Builder, Float64Builder, Int64Builder, StringBuilder, Time64MicrosecondBuilder,
    TimestampMicrosecondBuilder,
};
use arrow_array::{Array, ArrayRef, RecordBatch, cast::AsArray, types};
use arrow_ipc::reader::FileReader;
use arrow_ipc::writer::FileWriter;
use arrow_schema::{DataType, Field as ArrowField, Schema, TimeUnit};
use dms_types::{DataModel, Field, Storage, Table, Value};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::sync::Arc;

/// The most values a script may return (rows × columns), so a runaway script cannot exhaust
/// memory while its result is read.
pub const MAX_CELLS: usize = 100_000_000;

const MICROS_PER_DAY: i64 = 86_400_000_000;

fn storage_name(storage: Storage) -> &'static str {
    match storage {
        Storage::String => "string",
        Storage::Integer => "integer",
        Storage::Real => "real",
        Storage::Date => "date",
        Storage::Time => "time",
        Storage::Timestamp => "timestamp",
        Storage::List => "list",
        Storage::Unknown => "unknown",
    }
}

/// The storage a column is sent as: its own, or for unknown storage the kind of its first
/// non-null value (all null: string).
fn sent_storage(table: &Table, column: usize, storage: Storage) -> Storage {
    if storage != Storage::Unknown {
        return storage;
    }
    table
        .rows()
        .iter()
        .filter_map(|row| row.get(column))
        .find_map(|value| match value {
            Value::Null => None,
            Value::Integer(_) => Some(Storage::Integer),
            Value::Real(_) => Some(Storage::Real),
            Value::String(_) => Some(Storage::String),
            Value::Date(_) => Some(Storage::Date),
            Value::Time(_) => Some(Storage::Time),
            Value::Timestamp(_) => Some(Storage::Timestamp),
            Value::List(_) => Some(Storage::List),
        })
        .unwrap_or(Storage::String)
}

fn mismatch(field: &Field, value: &Value) -> BridgeError {
    BridgeError::Input(format!(
        "field {:?} has {} storage but holds {value:?}",
        field.name,
        storage_name(field.storage)
    ))
}

fn column(table: &Table, index: usize, field: &Field) -> Result<(DataType, ArrayRef), BridgeError> {
    let values = table
        .rows()
        .iter()
        .map(|row| row.get(index).unwrap_or(&Value::Null));
    let rows = table.row_count();
    Ok(match sent_storage(table, index, field.storage) {
        Storage::Integer => {
            let mut b = Int64Builder::with_capacity(rows);
            for v in values {
                match v {
                    Value::Null => b.append_null(),
                    Value::Integer(i) => b.append_value(*i),
                    other => return Err(mismatch(field, other)),
                }
            }
            (DataType::Int64, Arc::new(b.finish()))
        }
        Storage::Real => {
            let mut b = Float64Builder::with_capacity(rows);
            for v in values {
                match v {
                    Value::Null => b.append_null(),
                    Value::Real(r) => b.append_value(*r),
                    Value::Integer(i) => b.append_value(*i as f64),
                    other => return Err(mismatch(field, other)),
                }
            }
            (DataType::Float64, Arc::new(b.finish()))
        }
        Storage::String => {
            let mut b = StringBuilder::new();
            for v in values {
                match v {
                    Value::Null => b.append_null(),
                    Value::String(s) => b.append_value(s),
                    Value::Integer(i) => b.append_value(i.to_string()),
                    Value::Real(r) => b.append_value(r.to_string()),
                    other => return Err(mismatch(field, other)),
                }
            }
            (DataType::Utf8, Arc::new(b.finish()))
        }
        Storage::Date => {
            let mut b = Date32Builder::with_capacity(rows);
            for v in values {
                match v {
                    Value::Null => b.append_null(),
                    Value::Date(days) => b.append_value(i32::try_from(*days).map_err(|_| {
                        BridgeError::Input(format!(
                            "field {:?}: date {days} days from 1970 is out of range",
                            field.name
                        ))
                    })?),
                    other => return Err(mismatch(field, other)),
                }
            }
            (DataType::Date32, Arc::new(b.finish()))
        }
        Storage::Time => {
            let mut b = Time64MicrosecondBuilder::with_capacity(rows);
            for v in values {
                match v {
                    Value::Null => b.append_null(),
                    Value::Time(us) => b.append_value(*us),
                    other => return Err(mismatch(field, other)),
                }
            }
            (
                DataType::Time64(TimeUnit::Microsecond),
                Arc::new(b.finish()),
            )
        }
        Storage::Timestamp => {
            let mut b = TimestampMicrosecondBuilder::with_capacity(rows);
            for v in values {
                match v {
                    Value::Null => b.append_null(),
                    Value::Timestamp(us) => b.append_value(*us),
                    other => return Err(mismatch(field, other)),
                }
            }
            (
                DataType::Timestamp(TimeUnit::Microsecond, None),
                Arc::new(b.finish()),
            )
        }
        Storage::List | Storage::Unknown => {
            return Err(BridgeError::Input(format!(
                "field {:?} is a list (collection); list fields cannot be passed to Python yet. \
                 Filter it out before this node.",
                field.name
            )));
        }
    })
}

/// Writes `table` as an Arrow IPC file.
pub fn write_table(table: &Table, path: &Path) -> Result<(), BridgeError> {
    let mut fields = Vec::with_capacity(table.model().len());
    let mut arrays = Vec::with_capacity(table.model().len());
    for (index, field) in table.model().fields().iter().enumerate() {
        let (data_type, array) = column(table, index, field)?;
        let metadata = HashMap::from([
            (
                "dms.storage".to_string(),
                storage_name(field.storage).to_string(),
            ),
            (
                "dms.measurement".to_string(),
                format!("{:?}", field.measurement),
            ),
            ("dms.role".to_string(), format!("{:?}", field.role)),
        ]);
        fields.push(ArrowField::new(&field.name, data_type, true).with_metadata(metadata));
        arrays.push(array);
    }
    let schema = Arc::new(Schema::new(fields));
    let batch = if arrays.is_empty() {
        RecordBatch::new_empty(schema.clone())
    } else {
        RecordBatch::try_new(schema.clone(), arrays)
            .map_err(|e| BridgeError::Input(e.to_string()))?
    };
    let file = BufWriter::new(File::create(path)?);
    let mut writer =
        FileWriter::try_new(file, &schema).map_err(|e| BridgeError::Io(e.to_string()))?;
    writer
        .write(&batch)
        .and_then(|()| writer.finish())
        .map_err(|e| BridgeError::Io(e.to_string()))?;
    Ok(())
}

fn storage_of(name: &str, data_type: &DataType) -> Result<Storage, BridgeError> {
    Ok(match data_type {
        DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64
        | DataType::Boolean => Storage::Integer,
        DataType::Float16 | DataType::Float32 | DataType::Float64 => Storage::Real,
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View | DataType::Null => {
            Storage::String
        }
        DataType::Date32 | DataType::Date64 => Storage::Date,
        DataType::Time32(_) | DataType::Time64(_) => Storage::Time,
        DataType::Timestamp(_, _) => Storage::Timestamp,
        other => {
            return Err(BridgeError::Output(format!(
                "column {name:?} has type {other}, which has no matching field storage; convert \
                 it to numbers, text, dates or times"
            )));
        }
    })
}

fn integer(array: &dyn Array, row: usize) -> Result<Value, String> {
    use DataType as D;
    Ok(match array.data_type() {
        D::Int8 => Value::Integer(i64::from(
            array.as_primitive::<types::Int8Type>().value(row),
        )),
        D::Int16 => Value::Integer(i64::from(
            array.as_primitive::<types::Int16Type>().value(row),
        )),
        D::Int32 => Value::Integer(i64::from(
            array.as_primitive::<types::Int32Type>().value(row),
        )),
        D::Int64 => Value::Integer(array.as_primitive::<types::Int64Type>().value(row)),
        D::UInt8 => Value::Integer(i64::from(
            array.as_primitive::<types::UInt8Type>().value(row),
        )),
        D::UInt16 => Value::Integer(i64::from(
            array.as_primitive::<types::UInt16Type>().value(row),
        )),
        D::UInt32 => Value::Integer(i64::from(
            array.as_primitive::<types::UInt32Type>().value(row),
        )),
        D::UInt64 => {
            let v = array.as_primitive::<types::UInt64Type>().value(row);
            Value::Integer(i64::try_from(v).map_err(|_| format!("{v} is too large"))?)
        }
        D::Boolean => Value::Integer(i64::from(array.as_boolean().value(row))),
        _ => Value::Null,
    })
}

fn time_micros(unit: &TimeUnit, value: i64) -> i64 {
    match unit {
        TimeUnit::Second => value.saturating_mul(1_000_000),
        TimeUnit::Millisecond => value.saturating_mul(1_000),
        TimeUnit::Microsecond => value,
        TimeUnit::Nanosecond => value.div_euclid(1_000),
    }
}

fn value(array: &dyn Array, row: usize) -> Result<Value, String> {
    use DataType as D;
    if array.is_null(row) {
        return Ok(Value::Null);
    }
    Ok(match array.data_type() {
        D::Float16 => Value::real(f64::from(
            array
                .as_primitive::<types::Float16Type>()
                .value(row)
                .to_f32(),
        )),
        D::Float32 => Value::real(f64::from(
            array.as_primitive::<types::Float32Type>().value(row),
        )),
        D::Float64 => Value::real(array.as_primitive::<types::Float64Type>().value(row)),
        D::Utf8 => Value::string(array.as_string::<i32>().value(row)),
        D::LargeUtf8 => Value::string(array.as_string::<i64>().value(row)),
        D::Utf8View => Value::string(array.as_string_view().value(row)),
        D::Null => Value::Null,
        D::Date32 => Value::Date(i64::from(
            array.as_primitive::<types::Date32Type>().value(row),
        )),
        D::Date64 => Value::Date(
            array
                .as_primitive::<types::Date64Type>()
                .value(row)
                .div_euclid(86_400_000),
        ),
        D::Time32(unit) => {
            let v = match unit {
                TimeUnit::Second => {
                    i64::from(array.as_primitive::<types::Time32SecondType>().value(row))
                }
                _ => i64::from(
                    array
                        .as_primitive::<types::Time32MillisecondType>()
                        .value(row),
                ),
            };
            Value::Time(time_micros(unit, v))
        }
        D::Time64(unit) => {
            let v = match unit {
                TimeUnit::Nanosecond => array
                    .as_primitive::<types::Time64NanosecondType>()
                    .value(row),
                _ => array
                    .as_primitive::<types::Time64MicrosecondType>()
                    .value(row),
            };
            Value::Time(time_micros(unit, v).rem_euclid(MICROS_PER_DAY))
        }
        D::Timestamp(unit, _) => {
            let v = match unit {
                TimeUnit::Second => array
                    .as_primitive::<types::TimestampSecondType>()
                    .value(row),
                TimeUnit::Millisecond => array
                    .as_primitive::<types::TimestampMillisecondType>()
                    .value(row),
                TimeUnit::Microsecond => array
                    .as_primitive::<types::TimestampMicrosecondType>()
                    .value(row),
                TimeUnit::Nanosecond => array
                    .as_primitive::<types::TimestampNanosecondType>()
                    .value(row),
            };
            Value::Timestamp(time_micros(unit, v))
        }
        _ => integer(array, row)?,
    })
}

/// The field of a returned column: the input field of the same name and storage keeps its
/// measurement, role, labels and missing values; otherwise only the name and storage are known.
fn field_for(name: &str, storage: Storage, input: Option<&DataModel>) -> Field {
    match input.and_then(|model| model.field(name)) {
        Some(field) if field.storage == storage => field.clone(),
        Some(field) => {
            let mut new = Field::new(name, storage);
            new.role = field.role;
            new
        }
        None => Field::new(name, storage),
    }
}

/// Reads an Arrow IPC file written by the script runner into a table. `input` supplies the
/// metadata of fields that pass through unchanged.
pub fn read_table(path: &Path, input: Option<&DataModel>) -> Result<Table, BridgeError> {
    let file = BufReader::new(File::open(path)?);
    let reader = FileReader::try_new(file, None)
        .map_err(|e| BridgeError::Output(format!("unreadable data: {e}")))?;
    let schema = reader.schema();
    let mut fields = Vec::with_capacity(schema.fields().len());
    for field in schema.fields() {
        let storage = storage_of(field.name(), field.data_type())?;
        fields.push(field_for(field.name(), storage, input));
    }
    let model = DataModel::new(fields).map_err(|e| BridgeError::Output(e.to_string()))?;
    let width = model.len().max(1);
    let mut rows: Vec<Vec<Value>> = Vec::new();
    for batch in reader {
        let batch = batch.map_err(|e| BridgeError::Output(format!("unreadable data: {e}")))?;
        let cells = rows
            .len()
            .saturating_add(batch.num_rows())
            .saturating_mul(width);
        if cells > MAX_CELLS {
            return Err(BridgeError::Output(format!(
                "more than {MAX_CELLS} values returned; reduce the data before returning it"
            )));
        }
        for row in 0..batch.num_rows() {
            let mut out = Vec::with_capacity(batch.num_columns());
            for (column, field) in batch.columns().iter().zip(schema.fields()) {
                out.push(
                    value(column.as_ref(), row).map_err(|e| {
                        BridgeError::Output(format!("column {:?}: {e}", field.name()))
                    })?,
                );
            }
            rows.push(out);
        }
    }
    Table::new(model, rows).map_err(|e| BridgeError::Output(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dms_types::{Measurement, Role};

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("dms-python-{}-{name}", std::process::id()))
    }

    #[test]
    fn every_storage_round_trips_and_keeps_input_metadata() {
        let mut age = Field::new("age", Storage::Integer);
        age.measurement = Measurement::Continuous;
        age.role = Role::Input;
        let mut churn = Field::new("churn", Storage::String);
        churn.measurement = Measurement::Flag;
        churn.role = Role::Target;
        let model = DataModel::new(vec![
            age,
            Field::new("income", Storage::Real),
            churn,
            Field::new("day", Storage::Date),
            Field::new("at", Storage::Time),
            Field::new("when", Storage::Timestamp),
            Field::new("unknown", Storage::Unknown),
        ])
        .unwrap();
        let rows = vec![
            vec![
                Value::Integer(42),
                Value::Real(1.5),
                Value::string("T"),
                Value::Date(-1),
                Value::Time(3_600_000_000),
                Value::Timestamp(1_700_000_000_000_000),
                Value::Null,
            ],
            vec![Value::Null; 7],
        ];
        let table = Table::new(model.clone(), rows.clone()).unwrap();
        let path = temp("roundtrip.arrow");
        write_table(&table, &path).unwrap();
        let back = read_table(&path, Some(&model)).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back.rows(), rows.as_slice());
        let fields = back.model().fields();
        assert_eq!(fields[0].measurement, Measurement::Continuous);
        assert_eq!(fields[2].role, Role::Target);
        // Unknown storage with only nulls travels as text.
        assert_eq!(fields[6].storage, Storage::String);
    }

    #[test]
    fn lists_and_mismatched_values_are_input_errors_not_panics() {
        let model = DataModel::new(vec![Field::new("n", Storage::Integer)]).unwrap();
        let table = Table::new(model, vec![vec![Value::string("x")]]).unwrap();
        let path = temp("mismatch.arrow");
        assert!(matches!(
            write_table(&table, &path),
            Err(BridgeError::Input(_))
        ));
        let model = DataModel::new(vec![Field::new("l", Storage::List)]).unwrap();
        let table = Table::new(model, vec![]).unwrap();
        assert!(matches!(
            write_table(&table, &path),
            Err(BridgeError::Input(_))
        ));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn garbage_files_are_output_errors() {
        let path = temp("garbage.arrow");
        std::fs::write(&path, b"not arrow at all").unwrap();
        assert!(matches!(
            read_table(&path, None),
            Err(BridgeError::Output(_))
        ));
        let _ = std::fs::remove_file(&path);
    }
}
