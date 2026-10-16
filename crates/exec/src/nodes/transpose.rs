//! Transpose (Field Operations), normal method: each selected field becomes one record, and
//! each input record becomes a field.
//!
//! Implemented settings (children of `properties`; anything else fails
//! [`OperatorError::Settings`]):
//! - `transposeMethod` (`normal` only; `casetovar` and `vartocase` are rejected)
//! - `transposeStorageTypeMode` (`numeric` | `string` | `custom`)
//! - `transposeCustomFieldList/field` when the mode is `custom`
//! - `transposeFieldNamesMode` (`use_prefix` | `read_from_field`)
//! - `transposeFieldNamePrefix`, `transposeNumberOfNewFields` (prefix names)
//! - `transposeReadFromField`, `transposeMaxNumberOfFields` (names read from a field)
//! - `transposeRowIdFieldName` (the field that holds the source field name)
//! - `transposeNewFieldNamesList/field` is accepted. It is the name cache Modeler wrote the last
//!   time the node ran; names are computed again from the input.
//!
//! The CASE to VAR / VAR to CASE lists (`transposeCaseToVar*`, `tranposeCaseToVarIdFieldList`,
//! `transposeVarToCase*`), `transposePYScript` and `transposeCaseToVarAggregateFunction` are
//! saved on every probe stream. A non-empty CASE/VAR list or script fails `build`. The aggregate
//! function is ignored while the method is `normal`.
//!
//! Rules settled by the oracle (`corpus/fields/transpose_*.str`, `oracle/fields/transpose_*`):
//! - `numeric` selects continuous fields in input order. `transpose_all_prefix` and
//!   `transpose_all_from_field` emit `id`, then `x`, then `y` (the flag and set fields are left
//!   out). `custom` uses `transposeCustomFieldList` in list order (`transpose_subset_prefix`,
//!   `transpose_subset_from_field`: `x` then `y` only).
//! - The first output field is `transposeRowIdFieldName` (`src_field`), a string, and each row's
//!   value is the source field's name.
//! - Prefix names are `{prefix}{n}` for `n` from 1 through `transposeNumberOfNewFields`
//!   (`col1`..`col20`, `v1`..`v20`). Records past that count are dropped. Columns past the last
//!   record are null (`transpose_all_prefix`: `col17`..`col20` are null; `x` is null in `col8`).
//! - Names read from a field are that field's values in record order, capped by
//!   `transposeMaxNumberOfFields`, with no extra null columns (`transpose_all_from_field`:
//!   `r01`..`r16` from `row_name`, not 20 columns).
//! - Every value column shares one storage. An integer field transposed with reals becomes real
//!   (`transpose_all_prefix`: `id` 1 is `1.0`). Nulls in the source stay null.

use super::field_util::{field_list, infer_storage, position};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::{DataModel, Field, Measurement, Storage, Table, Value, ValueSpec};
use std::sync::Arc;

/// Upper bound on generated columns. The probe streams ask for at most 20.
const MAX_COLUMNS: i64 = 10_000;

const KNOWN_SETTINGS: &[&str] = &[
    "annotation",
    "autoLabel",
    "cacheEnabled",
    "createdBy",
    "creationDate",
    "doReadValues",
    "enabled",
    "fieldMappingEssentialFields",
    "keywords",
    "label",
    "lastSavedAs",
    "lastSavedBy",
    "lastSavedDate",
    "name",
    "position",
    "repositoryLastModified",
    "repositoryPath",
    "repositoryVersionMarker",
    "scriptExecutionModeChange",
    "toolTipText",
    "tranposeCaseToVarIdFieldList",
    "transposeCaseToVarAggregateFunction",
    "transposeCaseToVarColumnFieldList",
    "transposeCaseToVarValueFieldList",
    "transposeCustomFieldList",
    "transposeFieldNamePrefix",
    "transposeFieldNamesMode",
    "transposeMaxNumberOfFields",
    "transposeMethod",
    "transposeNewFieldNamesList",
    "transposeNumberOfNewFields",
    "transposePYScript",
    "transposeReadFromField",
    "transposeRowIdFieldName",
    "transposeStorageTypeMode",
    "transposeVarToCaseIdFieldList",
    "transposeVarToCaseValFieldList",
];

/// Lists that belong to the CASE to VAR and VAR to CASE methods.
const OTHER_METHOD_LISTS: &[&str] = &[
    "transposeCaseToVarValueFieldList",
    "tranposeCaseToVarIdFieldList",
    "transposeCaseToVarColumnFieldList",
    "transposeVarToCaseValFieldList",
    "transposeVarToCaseIdFieldList",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum StorageMode {
    Numeric,
    String,
    Custom,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NameMode {
    Prefix,
    Read,
}

struct Transpose {
    storage_mode: StorageMode,
    custom_fields: Vec<String>,
    name_mode: NameMode,
    prefix: String,
    num_new_fields: usize,
    read_from: String,
    max_fields: usize,
    id_name: String,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    reject_unknown(s)?;
    match settings::text(s, "transposeMethod")
        .as_deref()
        .map(str::trim)
    {
        None | Some("" | "normal") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "transposeMethod {other:?} is not supported; only normal transpose is implemented"
            )));
        }
    }
    for path in OTHER_METHOD_LISTS {
        reject_if_used(s, path)?;
    }
    reject_if_used(s, "transposePYScript")?;
    // Saved on every probe. It applies to CASE to VAR, which `transposeMethod` already rejects.
    let _aggregate = settings::text(s, "transposeCaseToVarAggregateFunction");
    reject_unexpected_children(s, "transposeNewFieldNamesList", &["field"])?;
    reject_unexpected_children(s, "transposeCustomFieldList", &["field"])?;

    let storage_mode = match settings::text(s, "transposeStorageTypeMode")
        .as_deref()
        .map(str::trim)
    {
        None | Some("" | "numeric") => StorageMode::Numeric,
        Some("string") => StorageMode::String,
        Some("custom") => StorageMode::Custom,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "transposeStorageTypeMode: expected numeric, string or custom, found {other:?}"
            )));
        }
    };
    let custom_fields = field_list(s, "transposeCustomFieldList");
    if storage_mode == StorageMode::Custom && custom_fields.is_empty() {
        return Err(OperatorError::Settings(
            "transposeCustomFieldList: no fields selected".into(),
        ));
    }
    if custom_fields.iter().any(|name| name.trim().is_empty()) {
        return Err(OperatorError::Settings(
            "transposeCustomFieldList: empty field name".into(),
        ));
    }

    let name_mode = match settings::text(s, "transposeFieldNamesMode")
        .as_deref()
        .map(str::trim)
    {
        None | Some("" | "use_prefix") => NameMode::Prefix,
        Some("read_from_field") => NameMode::Read,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "transposeFieldNamesMode: expected use_prefix or read_from_field, found {other:?}"
            )));
        }
    };
    let prefix = settings::text(s, "transposeFieldNamePrefix").unwrap_or_else(|| "Field".into());
    let num_new_fields = column_count(s, "transposeNumberOfNewFields", 100)?;
    let max_fields = column_count(s, "transposeMaxNumberOfFields", 500)?;
    let read_from = settings::text(s, "transposeReadFromField").unwrap_or_default();
    if name_mode == NameMode::Read && read_from.trim().is_empty() {
        return Err(OperatorError::Settings(
            "transposeReadFromField: name field is empty".into(),
        ));
    }
    let id_name = settings::text(s, "transposeRowIdFieldName").unwrap_or_else(|| "ID".into());
    if id_name.trim().is_empty() {
        return Err(OperatorError::Settings(
            "transposeRowIdFieldName: the id field needs a name".into(),
        ));
    }

    Ok(Box::new(Transpose {
        storage_mode,
        custom_fields,
        name_mode,
        prefix,
        num_new_fields,
        read_from,
        max_fields,
        id_name,
    }))
}

fn column_count(
    settings: Option<&Element>,
    path: &str,
    default: i64,
) -> Result<usize, OperatorError> {
    let count = settings::integer(settings, path)?.unwrap_or(default);
    if !(1..=MAX_COLUMNS).contains(&count) {
        return Err(OperatorError::Settings(format!(
            "{path}: expected a column count from 1 to {MAX_COLUMNS}, found {count}"
        )));
    }
    usize::try_from(count).map_err(|_| {
        OperatorError::Settings(format!("{path}: {count} does not fit a column count"))
    })
}

fn reject_unknown(settings: Option<&Element>) -> Result<(), OperatorError> {
    let Some(settings) = settings else {
        return Err(OperatorError::Settings("Transpose has no settings".into()));
    };
    for child in settings.child_elements() {
        let name = child.name.local.as_ref();
        if !KNOWN_SETTINGS.contains(&name) {
            return Err(OperatorError::Settings(format!(
                "Transpose setting {name:?} is not supported"
            )));
        }
    }
    Ok(())
}

fn reject_unexpected_children(
    settings: Option<&Element>,
    path: &str,
    allowed: &[&str],
) -> Result<(), OperatorError> {
    let Some(parent) = settings::element(settings, path) else {
        return Ok(());
    };
    for child in parent.child_elements() {
        let name = child.name.local.as_ref();
        if !allowed.contains(&name) {
            return Err(OperatorError::Settings(format!(
                "{path} child {name:?} is not supported"
            )));
        }
    }
    Ok(())
}

/// An empty element is the default saved by the probes. Text or children mean the other
/// transpose method was configured.
fn reject_if_used(settings: Option<&Element>, path: &str) -> Result<(), OperatorError> {
    let Some(element) = settings::element(settings, path) else {
        return Ok(());
    };
    if element.child_elements().next().is_some() || !element.text().trim().is_empty() {
        return Err(OperatorError::Settings(format!(
            "{path} is not supported; only normal transpose is implemented"
        )));
    }
    Ok(())
}

fn name_from(cell: &Value) -> Result<String, OperatorError> {
    match cell {
        Value::Null => Err(OperatorError::Data("transpose name field is null".into())),
        Value::String(text) if text.is_empty() => {
            Err(OperatorError::Data("transpose name field is blank".into()))
        }
        Value::String(text) => Ok(text.to_string()),
        other => {
            let text = other.to_string();
            if text.is_empty() {
                Err(OperatorError::Data("transpose name field is blank".into()))
            } else {
                Ok(text)
            }
        }
    }
}

fn coerce(value: Value, storage: Storage) -> Value {
    if storage == Storage::Real
        && let Value::Integer(integer) = value
    {
        return Value::Real(integer as f64);
    }
    value
}

impl Operator for Transpose {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Transpose needs one input".into()))?;
        let model = input.model();
        let rows = input.rows();
        let sources = self.sources(model)?;
        let column_names = self.column_names(model, rows)?;
        if column_names.iter().any(|name| name == &self.id_name) {
            return Err(OperatorError::Data(format!(
                "transposed field {:?} has the same name as the id field {:?}",
                column_names
                    .iter()
                    .find(|name| *name == &self.id_name)
                    .map(String::as_str)
                    .unwrap_or(""),
                self.id_name
            )));
        }

        let width = column_names.len();
        let used = rows.len().min(width);
        let mut sample = Vec::new();
        for (pos, _) in &sources {
            for row in rows.iter().take(used) {
                if let Some(value) = row.get(*pos)
                    && !value.is_null()
                {
                    sample.push(value.clone());
                }
            }
        }
        let storage = infer_storage(&sample).ok_or_else(|| {
            OperatorError::Data(
                "transposed values are of different kinds and cannot share a storage".into(),
            )
        })?;

        let mut fields = Vec::with_capacity(width.saturating_add(1));
        let mut id_field = Field::new(&self.id_name, Storage::String);
        id_field.measurement = Measurement::Nominal;
        id_field.values = ValueSpec::Categories(
            sources
                .iter()
                .map(|(_, name)| Value::string(name.as_str()))
                .collect(),
        );
        fields.push(id_field);
        let measurement = match storage {
            Storage::Integer | Storage::Real => Measurement::Continuous,
            Storage::String => Measurement::Nominal,
            _ => Measurement::Default,
        };
        for name in &column_names {
            let mut field = Field::new(name, storage);
            field.measurement = measurement;
            fields.push(field);
        }

        let mut out_rows = Vec::with_capacity(sources.len());
        for (pos, name) in &sources {
            let mut row = Vec::with_capacity(width.saturating_add(1));
            row.push(Value::string(name.as_str()));
            for column in 0..width {
                let value = if column < rows.len() {
                    let cell = rows
                        .get(column)
                        .and_then(|record| record.get(*pos))
                        .cloned()
                        .unwrap_or(Value::Null);
                    coerce(cell, storage)
                } else {
                    Value::Null
                };
                row.push(value);
            }
            out_rows.push(row);
        }
        let model =
            DataModel::new(fields).map_err(|error| OperatorError::Data(error.to_string()))?;
        Table::new(model, out_rows).map_err(|error| OperatorError::Data(error.to_string()))
    }
}

impl Transpose {
    fn sources(&self, model: &DataModel) -> Result<Vec<(usize, String)>, OperatorError> {
        match self.storage_mode {
            StorageMode::Numeric => Ok(model
                .fields()
                .iter()
                .enumerate()
                .filter(|(_, field)| field.measurement == Measurement::Continuous)
                .map(|(pos, field)| (pos, field.name.clone()))
                .collect()),
            StorageMode::String => Ok(model
                .fields()
                .iter()
                .enumerate()
                .filter(|(_, field)| field.storage == Storage::String)
                .map(|(pos, field)| (pos, field.name.clone()))
                .collect()),
            StorageMode::Custom => {
                let mut sources = Vec::with_capacity(self.custom_fields.len());
                for name in &self.custom_fields {
                    sources.push((position(model, name)?, name.clone()));
                }
                Ok(sources)
            }
        }
    }

    fn column_names(
        &self,
        model: &DataModel,
        rows: &[Vec<Value>],
    ) -> Result<Vec<String>, OperatorError> {
        match self.name_mode {
            NameMode::Prefix => Ok((1..=self.num_new_fields)
                .map(|index| format!("{}{index}", self.prefix))
                .collect()),
            NameMode::Read => {
                let pos = position(model, &self.read_from)?;
                let mut names = Vec::new();
                for row in rows.iter().take(self.max_fields) {
                    let cell = row.get(pos).unwrap_or(&Value::Null);
                    let name = name_from(cell)?;
                    if names.iter().any(|seen| seen == &name) {
                        return Err(OperatorError::Data(format!(
                            "transpose name {name:?} is used more than once"
                        )));
                    }
                    names.push(name);
                }
                Ok(names)
            }
        }
    }
}
