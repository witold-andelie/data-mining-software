//! Restructure (Field Operations): turns selected nominal/flag field values into new columns
//! filled with indicator flags (`0`/`1`) or with the values of other fields.
//!
//! Implemented: `createRestructuredFields/persistentObjectListMember/{srcFieldName,srcFieldValue,
//! includeFieldNames}`, `restructureIncludeFieldNames` (fallback when a member omits
//! `includeFieldNames`; default true), `restructureValuesMode` (`OtherFields` default | `Flags`),
//! and `restructureValueFields/field[n]`. Evidence: `docs/formats/nodes/restructure.md`,
//! `oracle/records/restructure_flags`, `oracle/records/restructure_values`, and
//! `oracle/team/stream_2_M`.
//!
//! Rules (settled by `oracle/records/restructure_flags`, `restructure_values`, and
//! `oracle/team/stream_2_M`):
//! - All input fields are preserved in their original order, followed by the newly created fields.
//! - Each member `(srcFieldName, srcFieldValue)` selects a value of `srcFieldName` (excluding
//!   missing/null records, which never match).
//! - When `srcFieldValue == "all"`, the member expands to the field's known values
//!   (`ValueSpec::Categories` or `ValueSpec::Flag`), or if not instantiated, the distinct
//!   non-missing values observed in the input in first-seen order (GAP: no oracle stream uses
//!   `all`; names and order follow the explicit-value rules below).
//! - In `Flags` mode (`restructureValuesMode=Flags`), one new field is added per expanded member:
//!   named `<src>_<val>` (`restructure_flags`: `grp_A`, `grp_B`; `team/stream_2_M`:
//!   `ACCOUNT_CURRENT`, `ACCOUNT_SAVE`), or `<val>` when `includeFieldNames` is false. Each
//!   flag column has `Storage::Integer`, `Measurement::Flag`, `ValueSpec::Flag { true_value: 1,
//!   false_value: 0 }`, and holds `1` on matching records and `0` on all others (including nulls).
//! - In `OtherFields` mode (`restructureValuesMode=OtherFields`), for each value field `vf` in
//!   `restructureValueFields` (outer loop) and each expanded member `(src, val)` (inner loop), one
//!   new field is added: named `<src>_<val>_<vf>` when `includeFieldNames` is true
//!   (`restructure_values`: `grp_A_iv`, `grp_B_iv`, `grp_A_rv`, `grp_B_rv`) or `<val>_<vf>` when
//!   false. The new field inherits `vf`'s storage and measurement, and holds `vf`'s value on
//!   matching records and `Value::Null` otherwise.
//!
//! GAP: value matching for non-string source fields compares numbers by numeric value and other
//! types by their plain text form.

use super::field_util::{Column, field_list, position, with_columns};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Field, Measurement, Storage, Table, Value, ValueSpec};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueMode {
    OtherFields,
    Flags,
}

struct MemberSpec {
    src_field: String,
    src_value: String,
    include_field_name: bool,
}

struct Restructure {
    members: Vec<MemberSpec>,
    value_mode: ValueMode,
    value_fields: Vec<String>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let default_include = settings::flag(s, "restructureIncludeFieldNames")?.unwrap_or(true);
    let value_mode = match settings::text(s, "restructureValuesMode")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("OtherFields") => ValueMode::OtherFields,
        Some("Flags") => ValueMode::Flags,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "restructureValuesMode: expected OtherFields or Flags, found {other:?}"
            )));
        }
    };
    let mut members = Vec::new();
    for m in settings::children(s, "createRestructuredFields", "persistentObjectListMember") {
        let src_field = settings::text(Some(m), "srcFieldName").unwrap_or_default();
        if src_field.trim().is_empty() {
            return Err(OperatorError::Settings(
                "createRestructuredFields: member has empty srcFieldName".into(),
            ));
        }
        let src_value = settings::text(Some(m), "srcFieldValue").unwrap_or_default();
        if src_value.is_empty() {
            return Err(OperatorError::Settings(
                "createRestructuredFields: member has empty srcFieldValue".into(),
            ));
        }
        let include_field_name =
            settings::flag(Some(m), "includeFieldNames")?.unwrap_or(default_include);
        members.push(MemberSpec {
            src_field,
            src_value,
            include_field_name,
        });
    }
    if members.is_empty() {
        let fallback = settings::text(s, "restructureField").unwrap_or_default();
        if !fallback.trim().is_empty() {
            members.push(MemberSpec {
                src_field: fallback,
                src_value: "all".into(),
                include_field_name: default_include,
            });
        } else {
            return Err(OperatorError::Settings(
                "createRestructuredFields: no fields selected to restructure".into(),
            ));
        }
    }
    let value_fields = field_list(s, "restructureValueFields");
    if value_mode == ValueMode::OtherFields && value_fields.is_empty() {
        return Err(OperatorError::Settings(
            "restructureValueFields: no value fields selected for OtherFields mode".into(),
        ));
    }
    Ok(Box::new(Restructure {
        members,
        value_mode,
        value_fields,
    }))
}

struct ExpandedMember {
    src_pos: usize,
    src_field: String,
    label: String,
    matcher: ValueMatcher,
    include_field_name: bool,
}

enum ValueMatcher {
    Value(Value),
    Number(f64),
    Text(String),
}

impl ValueMatcher {
    fn from_value(val: &Value) -> ValueMatcher {
        ValueMatcher::Value(val.clone())
    }

    fn from_text(text: &str, storage: Storage) -> ValueMatcher {
        match storage {
            Storage::Integer | Storage::Real => text
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .map_or_else(
                    || ValueMatcher::Text(text.to_string()),
                    ValueMatcher::Number,
                ),
            _ => ValueMatcher::Text(text.to_string()),
        }
    }

    fn matches(&self, cell: &Value) -> bool {
        if cell.is_null() {
            return false;
        }
        match (self, cell) {
            (ValueMatcher::Value(target), other) => target.total_cmp(other).is_eq(),
            (ValueMatcher::Number(n), Value::Integer(_) | Value::Real(_)) => {
                cell.as_f64().is_some_and(|v| v == *n)
            }
            (ValueMatcher::Text(t), Value::String(s)) => t.as_str() == &**s,
            (ValueMatcher::Text(t), other) => *t == other.to_string(),
            _ => false,
        }
    }
}

impl Operator for Restructure {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Restructure needs one input".into()))?;
        let model = input.model();
        let rows = input.rows();

        let mut expanded = Vec::new();
        for member in &self.members {
            let src_pos = position(model, &member.src_field)?;
            let field = model
                .fields()
                .get(src_pos)
                .ok_or_else(|| OperatorError::Data(format!("no field {:?}", member.src_field)))?;
            if member.src_value == "all" {
                let values = expand_all_values(field, src_pos, rows);
                for val in values {
                    expanded.push(ExpandedMember {
                        src_pos,
                        src_field: member.src_field.clone(),
                        label: val.to_string(),
                        matcher: ValueMatcher::from_value(&val),
                        include_field_name: member.include_field_name,
                    });
                }
            } else {
                expanded.push(ExpandedMember {
                    src_pos,
                    src_field: member.src_field.clone(),
                    label: member.src_value.clone(),
                    matcher: ValueMatcher::from_text(&member.src_value, field.storage),
                    include_field_name: member.include_field_name,
                });
            }
        }

        let mut columns = Vec::new();
        match self.value_mode {
            ValueMode::Flags => {
                for item in &expanded {
                    let base = if item.include_field_name {
                        format!("{}_{}", item.src_field, item.label)
                    } else {
                        item.label.clone()
                    };
                    let name = base;
                    let mut values = Vec::with_capacity(rows.len());
                    for row in rows {
                        let cell = row.get(item.src_pos).unwrap_or(&Value::Null);
                        let flag = i64::from(item.matcher.matches(cell));
                        values.push(Value::Integer(flag));
                    }
                    let mut field = Field::new(name, Storage::Integer);
                    field.measurement = Measurement::Flag;
                    field.values = ValueSpec::Flag {
                        true_value: Value::Integer(1),
                        false_value: Value::Integer(0),
                    };
                    columns.push(Column { field, values });
                }
            }
            ValueMode::OtherFields => {
                for vf_name in &self.value_fields {
                    let vf_pos = position(model, vf_name)?;
                    let vf_field = model
                        .fields()
                        .get(vf_pos)
                        .ok_or_else(|| OperatorError::Data(format!("no field {vf_name:?}")))?;
                    for item in &expanded {
                        let name = if item.include_field_name {
                            format!("{}_{}_{}", item.src_field, item.label, vf_name)
                        } else {
                            format!("{}_{}", item.label, vf_name)
                        };
                        let mut values = Vec::with_capacity(rows.len());
                        for row in rows {
                            let cell = row.get(item.src_pos).unwrap_or(&Value::Null);
                            if item.matcher.matches(cell) {
                                values.push(row.get(vf_pos).cloned().unwrap_or(Value::Null));
                            } else {
                                values.push(Value::Null);
                            }
                        }
                        let mut field = Field::new(name, vf_field.storage);
                        field.measurement = vf_field.measurement;
                        columns.push(Column { field, values });
                    }
                }
            }
        }

        with_columns(input, columns)
    }
}

fn expand_all_values(field: &Field, pos: usize, rows: &[Vec<Value>]) -> Vec<Value> {
    match &field.values {
        ValueSpec::Categories(cats) if !cats.is_empty() => return cats.clone(),
        ValueSpec::Flag {
            true_value,
            false_value,
        } => return vec![true_value.clone(), false_value.clone()],
        _ => {}
    }
    let mut seen: Vec<Value> = Vec::new();
    for row in rows {
        if let Some(val) = row.get(pos)
            && !field.is_missing(val)
            && !seen.iter().any(|v| v.total_cmp(val).is_eq())
        {
            seen.push(val.clone());
        }
    }
    seen
}
