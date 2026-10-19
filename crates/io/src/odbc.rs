//! ODBC 3 source and Create export, using odbc-api 29.2.0 (MIT).
//!
//! Evidence: Modeler 19 Scripting and Automation Guide tables 46/246 and
//! `xtask/oracle/probes/database_streams.py` (Microsoft Access Driver, DSN `dms_access`).
//!
//! Create column types are the ones Modeler chose on the only DBMS with an oracle, Microsoft
//! Access (`database/db_export`, read back by the driver's own column metadata): Integer
//! CURRENCY, String VARCHAR(255), Real DOUBLE, Date and Timestamp DATETIME. That matches the
//! driver's type list (no BIGINT; CURRENCY is its only exact numeric), but one driver cannot
//! settle a general SQLGetTypeInfo rule, and odbc-api has no safe SQLGetTypeInfo call, so any
//! other DBMS (`SQL_DBMS_NAME`) is a GAP. No statement timeout is set: Modeler sets none and
//! the Access driver rejects SQL_ATTR_QUERY_TIMEOUT (HYC00).

use dms_types::{DataModel, Field, Measurement, Storage, Table, Value};
use odbc_api::parameter::{InputParameter, VarWCharArray, WithDataType};
use odbc_api::{
    Connection, ConnectionOptions, Cursor, CursorRow, DataType, Environment, Nullable,
    ResultSetMetadata,
};
use std::fmt;
use std::num::NonZeroUsize;

const MAX_COLUMNS: usize = 1024;
const MAX_CELLS: usize = 1_000_000;
const MAX_SQL: usize = 1_048_576;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}

fn driver(error: odbc_api::Error) -> Error {
    // Driver diagnostics may echo SQL or connection attributes. Return SQLSTATE only;
    // never propagate a password or a cell's contents into logs/run.json.
    let state = match error {
        odbc_api::Error::Diagnostics { record, function } => {
            format!(" in {function} (SQLSTATE {:?})", record.state)
        }
        odbc_api::Error::NoDiagnostics { function } => {
            format!(" in {function} (driver returned no diagnostics)")
        }
        _ => String::new(),
    };
    Error(format!("ODBC operation failed{state}"))
}

pub fn validate_dsn(dsn: &str) -> Result<(), Error> {
    if dsn.is_empty()
        || dsn.len() > 128
        || dsn
            .chars()
            .any(|c| c.is_control() || matches!(c, ';' | '{' | '}'))
    {
        return Err(Error(
            "GAP: ODBC requires a plain DSN name, without connection attributes".into(),
        ));
    }
    Ok(())
}

fn connect<'a>(env: &'a Environment, dsn: &str) -> Result<Connection<'a>, Error> {
    validate_dsn(dsn)?;
    env.connect(dsn, "", "", ConnectionOptions { login_timeout_sec: Some(15), ..Default::default() })
        .map_err(|e| Error(format!("ODBC DSN connection failed: configure the named user/system DSN for this process's architecture and verify driver/database access (no credentials supported): {}", driver(e))))
}

fn quote(conn: &Connection<'_>, name: &str) -> Result<String, Error> {
    if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        return Err(Error(
            "GAP: ODBC identifier is empty, too long or contains control characters".into(),
        ));
    }
    let delimiter = conn
        .identifier_quote_char()
        .map_err(driver)?
        .ok_or_else(|| Error("GAP: driver has no identifier quoting".into()))?;
    let close = if delimiter == '[' { ']' } else { delimiter };
    Ok(format!(
        "{delimiter}{}{close}",
        name.replace(close, &format!("{close}{close}"))
    ))
}

/// Table order is the driver's SELECT * order; query order is its result-set order.
pub fn read(dsn: &str, table_or_query: &str, query: bool) -> Result<Table, Error> {
    guarded(|| read_inner(dsn, table_or_query, query))
}

// odbc-api can panic in Drop when SQLDisconnect/SQLFreeHandle fails. Contain Rust
// unwinding at the I/O boundary, including cleanup during an ordinary error return.
// Connection attributes/credentials are never supplied to driver diagnostics.
fn guarded<T>(action: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(action)).unwrap_or_else(|_| {
        Err(Error(
            "ODBC driver/library failed during operation or cleanup".into(),
        ))
    })
}

fn read_inner(dsn: &str, table_or_query: &str, query: bool) -> Result<Table, Error> {
    if table_or_query.len() > MAX_SQL || table_or_query.contains('\0') {
        return Err(Error(
            "GAP: ODBC SQL exceeds the size limit or contains NUL".into(),
        ));
    }
    let env = Environment::new().map_err(driver)?;
    let conn = connect(&env, dsn)?;
    let sql = if query {
        table_or_query.to_owned()
    } else {
        format!("SELECT * FROM {}", quote(&conn, table_or_query)?)
    };
    let mut cursor = conn
        .execute(&sql, (), None)
        .map_err(driver)?
        .ok_or_else(|| Error("GAP: Database source did not return a result set".into()))?;
    let count = usize::try_from(cursor.num_result_cols().map_err(driver)?)
        .ok()
        .filter(|n| (1..=MAX_COLUMNS).contains(n))
        .ok_or_else(|| Error("GAP: ODBC column count exceeds limit".into()))?;
    let mut fields = Vec::with_capacity(count);
    for column in 1..=count {
        let column =
            u16::try_from(column).map_err(|_| Error("ODBC column index overflow".into()))?;
        let storage = match cursor.col_data_type(column).map_err(driver)? {
            DataType::Integer | DataType::SmallInt | DataType::BigInt | DataType::TinyInt => {
                Storage::Integer
            }
            DataType::Double
            | DataType::Real
            | DataType::Float { .. }
            | DataType::Decimal { .. }
            | DataType::Numeric { .. } => Storage::Real,
            DataType::Varchar { .. }
            | DataType::WVarchar { .. }
            | DataType::Char { .. }
            | DataType::WChar { .. } => Storage::String,
            DataType::Timestamp { .. } => Storage::Timestamp,
            _ => {
                return Err(Error(
                    "GAP: Database source SQL storage type is not covered".into(),
                ));
            }
        };
        let mut field = Field::new(cursor.col_name(column).map_err(driver)?, storage);
        field.measurement = if storage == Storage::String {
            Measurement::Categorical
        } else {
            Measurement::Continuous
        };
        fields.push(field);
    }
    let mut rows = Vec::new();
    let mut text_budget = 0usize;
    while let Some(mut row) = cursor.next_row().map_err(driver)? {
        if rows.len() >= MAX_CELLS / count {
            return Err(Error("GAP: Database source cell limit exceeded".into()));
        }
        let mut values = Vec::with_capacity(count);
        for (offset, field) in fields.iter().enumerate() {
            let column = u16::try_from(offset.saturating_add(1))
                .map_err(|_| Error("ODBC column index overflow".into()))?;
            let value = match field.storage {
                Storage::Integer => {
                    let mut v = Nullable::<i64>::null();
                    row.get_data(column, &mut v).map_err(driver)?;
                    v.into_opt().map(Value::Integer).unwrap_or(Value::Null)
                }
                Storage::Real => {
                    let mut v = Nullable::<f64>::null();
                    row.get_data(column, &mut v).map_err(driver)?;
                    match v.into_opt() {
                        None => Value::Null,
                        Some(n) if n.is_finite() => Value::Real(n),
                        _ => return Err(Error("GAP: nonfinite ODBC number".into())),
                    }
                }
                Storage::String => match wide_text(&mut row, column)? {
                    None => Value::Null,
                    Some(s) => {
                        text_budget = text_budget.saturating_add(s.len());
                        if text_budget > 64 * 1024 * 1024 {
                            return Err(Error("GAP: ODBC text memory limit exceeded".into()));
                        }
                        Value::string(s)
                    }
                },
                Storage::Timestamp => {
                    let mut v = Nullable::<odbc_api::sys::Timestamp>::null();
                    row.get_data(column, &mut v).map_err(driver)?;
                    match v.into_opt() {
                        None => Value::Null,
                        Some(t) => timestamp(t)?,
                    }
                }
                _ => return Err(Error("GAP: ODBC storage not covered".into())),
            };
            values.push(value);
        }
        rows.push(values);
    }
    Table::new(
        DataModel::new(fields).map_err(|e| Error(e.to_string()))?,
        rows,
    )
    .map_err(|e| Error(e.to_string()))
}

fn wide_text(row: &mut CursorRow<'_>, column: u16) -> Result<Option<String>, Error> {
    let mut value = VarWCharArray::<4097>::NULL;
    row.get_data(column, &mut value).map_err(driver)?;
    if !value.is_complete() {
        return Err(Error("GAP: ODBC text exceeds 4096 UTF-16 units".into()));
    }
    value
        .as_slice()
        .map(String::from_utf16)
        .transpose()
        .map_err(|_| Error("ODBC invalid UTF-16".into()))
}

fn timestamp(t: odbc_api::sys::Timestamp) -> Result<Value, Error> {
    use dms_types::calendar::{MICROS_PER_DAY, days_from_civil, micros_from_time};
    let value = days_from_civil(i64::from(t.year), u32::from(t.month), u32::from(t.day))
        .and_then(|d| d.checked_mul(MICROS_PER_DAY))
        .zip(micros_from_time(
            u32::from(t.hour),
            u32::from(t.minute),
            u32::from(t.second),
            t.fraction / 1000,
        ))
        .and_then(|(d, time)| d.checked_add(time));
    if !t.fraction.is_multiple_of(1000) {
        return Err(Error("GAP: sub-microsecond ODBC timestamp".into()));
    }
    value
        .map(Value::Timestamp)
        .ok_or_else(|| Error("invalid ODBC timestamp".into()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub sql_type: i16,
    pub type_name: String,
    pub size: Option<i32>,
    pub scale: Option<i16>,
}

/// SQLColumns metadata, filtered by exact table name (ODBC treats '_' as a wildcard).
pub fn columns(dsn: &str, table: &str) -> Result<Vec<Column>, Error> {
    guarded(|| {
        let env = Environment::new().map_err(driver)?;
        let conn = connect(&env, dsn)?;
        columns_on(&conn, table)
    })
}

fn columns_on(conn: &Connection<'_>, table: &str) -> Result<Vec<Column>, Error> {
    let mut statement = conn.preallocate().map_err(driver)?;
    let mut cursor = statement
        .columns_cursor("", "", table, "%")
        .map_err(driver)?;
    let mut columns = Vec::new();
    while let Some(mut row) = cursor.next_row().map_err(driver)? {
        if columns.len() >= MAX_COLUMNS {
            return Err(Error("GAP: SQLColumns limit exceeded".into()));
        }
        let table_name = wide_text(&mut row, 3)?;
        let name =
            wide_text(&mut row, 4)?.ok_or_else(|| Error("SQLColumns missing name".into()))?;
        let mut sql_type = 0i16;
        row.get_data(5, &mut sql_type).map_err(driver)?;
        let type_name =
            wide_text(&mut row, 6)?.ok_or_else(|| Error("SQLColumns missing type name".into()))?;
        let mut size = Nullable::<i32>::null();
        row.get_data(7, &mut size).map_err(driver)?;
        let mut scale = Nullable::<i16>::null();
        row.get_data(9, &mut scale).map_err(driver)?;
        if table_name
            .as_deref()
            .is_some_and(|n| n.eq_ignore_ascii_case(table))
        {
            columns.push(Column {
                name,
                sql_type,
                type_name,
                size: size.into_opt(),
                scale: scale.into_opt(),
            });
        }
    }
    Ok(columns)
}

/// The Create column type for a field, by DBMS (`SQL_DBMS_NAME`; see the module docs).
pub fn select_type(dbms: &str, storage: Storage) -> Result<(String, DataType), Error> {
    if !dbms.eq_ignore_ascii_case("ACCESS") {
        return Err(Error(format!(
            "GAP: database export column types are verified for Microsoft Access only, not {dbms:?}"
        )));
    }
    Ok(match storage {
        Storage::Integer => (
            "CURRENCY".into(),
            DataType::Numeric {
                precision: 19,
                scale: 0,
            },
        ),
        Storage::String => (
            "VARCHAR(255)".into(),
            DataType::Varchar {
                length: NonZeroUsize::new(255),
            },
        ),
        Storage::Real => ("DOUBLE".into(), DataType::Double),
        Storage::Date | Storage::Timestamp => {
            ("DATETIME".into(), DataType::Timestamp { precision: 0 })
        }
        _ => return Err(Error("GAP: database export storage is not covered".into())),
    })
}

/// Only Create is supported. Uses a transaction and explicitly rolls back on errors.
/// The driver determines whether DDL participates in its transactions.
pub fn create(dsn: &str, name: &str, table: &Table, drop_existing: bool) -> Result<(), Error> {
    guarded(|| create_inner(dsn, name, table, drop_existing))
}

fn create_inner(dsn: &str, name: &str, table: &Table, drop_existing: bool) -> Result<(), Error> {
    if table.model().is_empty()
        || table.model().len() > MAX_COLUMNS
        || table.row_count().saturating_mul(table.model().len()) > MAX_CELLS
    {
        return Err(Error(
            "GAP: database export dimensions exceed limits".into(),
        ));
    }
    let env = Environment::new().map_err(driver)?;
    let conn = connect(&env, dsn)?;
    let dbms = conn.database_management_system_name().map_err(driver)?;
    let selected = table
        .model()
        .fields()
        .iter()
        .map(|f| select_type(&dbms, f.storage))
        .collect::<Result<Vec<_>, _>>()?;
    // Preflight every cell before destructive DDL, including storage/length checks.
    for row in table.rows() {
        for ((value, field), (_, sql_type)) in row.iter().zip(table.model().fields()).zip(&selected)
        {
            let _ = parameter(value, field.storage, *sql_type)?;
        }
    }
    let target = quote(&conn, name)?;
    let names = table
        .model()
        .fields()
        .iter()
        .map(|f| quote(&conn, &f.name))
        .collect::<Result<Vec<_>, _>>()?;
    let definitions = names
        .iter()
        .zip(&selected)
        .map(|(name, (ty, _))| format!("{name} {ty}"))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = vec!["?"; names.len()].join(", ");
    let insert = format!(
        "INSERT INTO {target} ({}) VALUES ({placeholders})",
        names.join(", ")
    );
    let exists = !columns_on(&conn, name)?.is_empty();
    if exists && !drop_existing {
        return Err(Error(
            "Database Create target already exists; dropExistingTable is false".into(),
        ));
    }
    conn.set_autocommit(false).map_err(driver)?;
    let result = (|| {
        if exists {
            conn.execute(&format!("DROP TABLE {target}"), (), None)
                .map_err(driver)?;
        }
        conn.execute(&format!("CREATE TABLE {target} ({definitions})"), (), None)
            .map_err(driver)?;
        let mut prepared = conn.prepare(&insert).map_err(driver)?;
        for row in table.rows() {
            let params = row
                .iter()
                .zip(table.model().fields())
                .zip(&selected)
                .map(|((value, field), (_, ty))| parameter(value, field.storage, *ty))
                .collect::<Result<Vec<_>, _>>()?;
            prepared.execute(params.as_slice()).map_err(driver)?;
        }
        conn.commit().map_err(driver)
    })();
    if result.is_err() {
        conn.rollback().map_err(driver)?;
    }
    conn.set_autocommit(true).map_err(driver)?;
    result
}

fn parameter(
    value: &Value,
    storage: Storage,
    sql_type: DataType,
) -> Result<Box<dyn InputParameter>, Error> {
    if value.storage().is_some_and(|s| s != storage) {
        return Err(Error("GAP: database export cell storage mismatch".into()));
    }
    let boxed: Box<dyn InputParameter> = match value {
        Value::Integer(n) => {
            // Fixed-scale numeric drivers (e.g. CURRENCY) may have a smaller integral range.
            // Bind exact decimal text rather than converting i64 to f64 or SQL_C_SBIGINT.
            Box::new(WithDataType::new(
                odbc_api::IntoParameter::into_parameter(n.to_string()),
                sql_type,
            ))
        }
        Value::Real(n) if n.is_finite() => Box::new(WithDataType::new(*n, sql_type)),
        Value::String(s) if s.encode_utf16().count() <= 255 && !s.contains('\0') => {
            Box::new(WithDataType::new(
                VarWCharArray::<256>::new(&s.encode_utf16().collect::<Vec<_>>()),
                sql_type,
            ))
        }
        Value::Date(_) | Value::Timestamp(_) => {
            use dms_types::calendar::{civil_from_days, split_timestamp, time_from_micros};
            let (day, micros) = match value {
                Value::Date(d) => (*d, 0),
                Value::Timestamp(t) => split_timestamp(*t),
                _ => return Err(Error("invalid timestamp".into())),
            };
            let (y, m, d) = civil_from_days(day)
                .ok_or_else(|| Error("GAP: export date out of range".into()))?;
            let (h, min, s, us) = time_from_micros(micros)
                .ok_or_else(|| Error("GAP: export time out of range".into()))?;
            if us != 0 {
                return Err(Error(
                    "GAP: database export subsecond timestamps are not oracle-covered".into(),
                ));
            }
            let t = odbc_api::sys::Timestamp {
                year: i16::try_from(y)
                    .map_err(|_| Error("GAP: export year out of range".into()))?,
                month: m as u16,
                day: d as u16,
                hour: h as u16,
                minute: min as u16,
                second: s as u16,
                fraction: us.saturating_mul(1000),
            };
            Box::new(WithDataType::new(t, sql_type))
        }
        Value::Null => Box::new(WithDataType::new(VarWCharArray::<1>::NULL, sql_type)),
        _ => {
            return Err(Error(
                "GAP: database export value is too long, nonfinite or unsupported".into(),
            ));
        }
    };
    Ok(boxed)
}
