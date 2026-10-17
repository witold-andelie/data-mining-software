//! User Input (Sources palette, `UserInputNode`): records typed into the node
//! (`oracle/sources/user_*`).
//!
//! Settings: `userFieldTable/fields/field` (`name`, `storage`, `values`), `dataMode` (`combine`
//! default, `order`), embedded `filterTable`/`typeTable` rules. At run time the stream's
//! `dateFormat` and `timeFormat` read dates and times (`user_temporal`).
//!
//! Rules:
//! - Values are separated by spaces; double quotes group a value with spaces or commas, and `""`
//!   is the empty string (`user_strings`: `"  "`, `"has,comma"`).
//! - In an integer or real field, `start,end,step` is the values from `start` up to `end`
//!   inclusive (`user_integer_ranges`: `10,20,2`, `-3,3,2`), computed as `start + i * step`, so
//!   a real end the steps miss is left out (`user_real_ranges`: `0,0.3,0.1` gives 0, 0.1, 0.2). A
//!   step that is not positive is refused ("Invalid value specified for field 'x': -2",
//!   `user_descending_range`), as is any value that does not read in the field's storage.
//! - `combine`: every combination of the fields' values, the last field changing fastest, with
//!   duplicates kept (`user_default`, `user_combined`: 3 x 3 records). A field with no values
//!   counts as one null (`user_combined_blank`).
//! - `order`: record i has the i-th value of each field, null where a field has fewer
//!   (`user_ordered`, `user_unequal`).
//! - Without a storage (`unknown`), a field whose values are all integers is integer, all
//!   numbers real; any other is refused ("Storage not determined for field: mixed",
//!   `user_inferred`).
//!
//! GAPs: a decreasing range with a positive step, ranges of two parts, and `unknown` fields
//! with no values or with date-like values are refused (no probe settles them); ranges are
//! accumulated as `start + i * step` (no probe tells it from repeated addition).

use super::r#type::{SourceTables, apply_source_tables, parse_source_tables};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_io::text::{DateFormat, TextOptions, TimeFormat, parse};
use dms_stream::StreamNode;
use dms_types::{DataModel, Field, Measurement, Storage, Table, Value};
use std::sync::Arc;

/// Values one range may produce.
const MAX_RANGE_VALUES: usize = 1_000_000;

struct UserField {
    name: String,
    /// `None`: inferred from the values.
    storage: Option<Storage>,
    tokens: Vec<String>,
}

struct UserInput {
    fields: Vec<UserField>,
    combine: bool,
    tables: SourceTables,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let bad = |what: String| OperatorError::Settings(format!("User Input: {what}"));
    let combine = match settings::text(s, "dataMode").as_deref().map(str::trim) {
        None | Some("") | Some("combine") => true,
        Some("order") => false,
        Some(other) => return Err(bad(format!("unknown dataMode {other:?}"))),
    };
    let mut fields = Vec::new();
    for field in settings::children(s, "userFieldTable/fields", "field") {
        let field = Some(field);
        let name = settings::text(field, "name").unwrap_or_default();
        if name.is_empty() {
            return Err(bad("a field without a name".into()));
        }
        let storage = match settings::text(field, "storage").as_deref().map(str::trim) {
            None | Some("") | Some("unknown") => None,
            Some("string") => Some(Storage::String),
            Some("integer") => Some(Storage::Integer),
            Some("real") => Some(Storage::Real),
            Some("date") => Some(Storage::Date),
            Some("time") => Some(Storage::Time),
            Some("timestamp") => Some(Storage::Timestamp),
            Some(other) => {
                return Err(bad(format!(
                    "storage {other:?} of field {name:?} is not supported yet"
                )));
            }
        };
        let text = settings::text(field, "values").unwrap_or_default();
        fields.push(UserField {
            tokens: tokens(&text).map_err(|e| bad(format!("field {name:?}: {e}")))?,
            name,
            storage,
        });
    }
    if fields.is_empty() {
        return Err(bad("no fields".into()));
    }
    Ok(Box::new(UserInput {
        fields,
        combine,
        tables: parse_source_tables(s)?,
    }))
}

/// The values typed for a field: separated by white space, double quotes grouping.
fn tokens(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let mut token = String::new();
        if c == '"' {
            chars.next();
            loop {
                match chars.next() {
                    Some('"') => break,
                    Some(c) => token.push(c),
                    None => return Err("unclosed quote".into()),
                }
            }
            if chars.peek().is_some_and(|c| !c.is_whitespace()) {
                return Err("text right after a closing quote".into());
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                token.push(c);
                chars.next();
            }
        }
        out.push(token);
    }
    Ok(out)
}

impl UserField {
    fn invalid(&self, token: &str) -> OperatorError {
        OperatorError::Data(format!(
            "Invalid value specified for field '{}': {token}",
            self.name
        ))
    }

    /// The storage, inferring a missing one from the values.
    fn storage(&self, options: &TextOptions) -> Result<Storage, OperatorError> {
        if let Some(storage) = self.storage {
            return Ok(storage);
        }
        let parts = || self.tokens.iter().flat_map(|t| t.split(','));
        let not_determined =
            || OperatorError::Data(format!("Storage not determined for field: {}", self.name));
        if self.tokens.is_empty() {
            return Err(not_determined());
        }
        for storage in [Storage::Integer, Storage::Real] {
            if parts().all(|p| parse(p.trim(), storage, options).is_some()) {
                return Ok(storage);
            }
        }
        Err(not_determined())
    }

    fn values(&self, storage: Storage, options: &TextOptions) -> Result<Vec<Value>, OperatorError> {
        let mut values = Vec::new();
        for token in &self.tokens {
            let numeric = matches!(storage, Storage::Integer | Storage::Real);
            if numeric && token.contains(',') {
                self.range(token, storage, options, &mut values)?;
                continue;
            }
            let value = match storage {
                Storage::String => Value::string(token.as_str()),
                _ => parse(token.trim(), storage, options).ok_or_else(|| self.invalid(token))?,
            };
            values.push(value);
        }
        Ok(values)
    }

    fn range(
        &self,
        token: &str,
        storage: Storage,
        options: &TextOptions,
        values: &mut Vec<Value>,
    ) -> Result<(), OperatorError> {
        let parts: Vec<&str> = token.split(',').map(str::trim).collect();
        let [start, end, step] = parts.as_slice() else {
            return Err(self.invalid(token));
        };
        let number = |text: &str| {
            parse(text, storage, options)
                .and_then(|v| v.as_f64())
                .ok_or_else(|| self.invalid(text))
        };
        let (start, end, step) = (number(start)?, number(end)?, number(step)?);
        if step <= 0.0 || !step.is_finite() {
            return Err(self.invalid(parts[2]));
        }
        if start > end {
            return Err(OperatorError::Settings(format!(
                "User Input: field '{}': a decreasing range {token} is not supported yet",
                self.name
            )));
        }
        for i in 0..MAX_RANGE_VALUES {
            let value = start + i as f64 * step;
            if value > end {
                return Ok(());
            }
            values.push(match storage {
                Storage::Integer => Value::Integer(value as i64),
                _ => Value::real(value),
            });
        }
        Err(OperatorError::Settings(format!(
            "User Input: field '{}': the range {token} has more than {MAX_RANGE_VALUES} values",
            self.name
        )))
    }
}

impl Operator for UserInput {
    fn arity(&self) -> Arity {
        Arity::Exactly(0)
    }

    fn run(&self, _inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let sp = context.stream_properties.as_deref();
        let mut options = TextOptions {
            recognize_dates: false,
            ..TextOptions::default()
        };
        if let Some(id) = settings::text(sp, "dateFormat") {
            options.date_format = DateFormat::from_modeler_id(&id).ok_or_else(|| {
                OperatorError::Settings(format!(
                    "the stream date format {id:?} is not supported yet"
                ))
            })?;
        }
        options.time_format = Some(
            TimeFormat::from_modeler_id(
                settings::text(sp, "timeFormat")
                    .as_deref()
                    .unwrap_or("format4"),
            )
            .ok_or_else(|| {
                OperatorError::Settings("the stream time format is not supported yet".into())
            })?,
        );

        let mut model_fields = Vec::with_capacity(self.fields.len());
        let mut columns = Vec::with_capacity(self.fields.len());
        for field in &self.fields {
            let storage = field.storage(&options)?;
            columns.push(field.values(storage, &options)?);
            let mut model_field = Field::new(field.name.clone(), storage);
            model_field.measurement = match storage {
                Storage::String => Measurement::Categorical,
                _ => Measurement::Continuous,
            };
            model_fields.push(model_field);
        }
        let rows = if self.combine {
            combinations(&columns)?
        } else {
            let count = columns.iter().map(Vec::len).max().unwrap_or(0);
            (0..count)
                .map(|i| {
                    columns
                        .iter()
                        .map(|c| c.get(i).cloned().unwrap_or(Value::Null))
                        .collect()
                })
                .collect()
        };
        let model = DataModel::new(model_fields).map_err(|e| OperatorError::Data(e.to_string()))?;
        let table = Table::new(model, rows).map_err(|e| OperatorError::Data(e.to_string()))?;
        apply_source_tables(&self.tables, table)
    }
}

/// Every combination, the last column changing fastest; an empty column is one null.
fn combinations(columns: &[Vec<Value>]) -> Result<Vec<Vec<Value>>, OperatorError> {
    let null = [Value::Null];
    let columns: Vec<&[Value]> = columns
        .iter()
        .map(|c| {
            if c.is_empty() {
                &null[..]
            } else {
                c.as_slice()
            }
        })
        .collect();
    let count = columns
        .iter()
        .try_fold(1usize, |n, c| n.checked_mul(c.len()))
        .filter(|&n| n <= MAX_RANGE_VALUES * 10)
        .ok_or_else(|| OperatorError::Settings("User Input: too many combinations".into()))?;
    let mut rows = Vec::with_capacity(count);
    for mut index in 0..count {
        let mut row = vec![Value::Null; columns.len()];
        for (at, column) in columns.iter().enumerate().rev() {
            row[at] = column[index % column.len()].clone();
            index /= column.len();
        }
        rows.push(row);
    }
    Ok(rows)
}
