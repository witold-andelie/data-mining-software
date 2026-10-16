//! Set to Flag (Field Operations): one flag field per saved category of a nominal field.
//!
//! Implemented settings (children of `properties`; anything else fails
//! [`OperatorError::Settings`]):
//! - `createFlagFields/persistentObjectListMember/{srcFieldName,srcFieldValue,flagNameExtension,prefixNotSuffix}`
//! - `trueValue` (default `T`), `falseValue` (default `F`)
//! - `useAggregateKeyFields`, `aggregateKeyFields/field`
//! - `fieldNameExtension`, `fieldNameExtensionEnabled`, `fieldNameExtensionPosition`
//!   (`prefix` | `suffix`), `setField` (must be empty)
//!
//! Common node properties (`name`, `label`, `position`, `autoLabel`, `cacheEnabled`, ...) are
//! accepted and ignored.
//!
//! Rules settled by the oracle (`corpus/fields/flag_*.str`, `oracle/fields/flag_*`):
//! - Each member becomes a flag named `{srcFieldName}_{srcFieldValue}`, appended after the input
//!   fields in member order (`flag_default`: `cat_red`, `cat_blue`, `cat_green`).
//! - A record's flag is the true value when that field equals the member value, and the false
//!   value otherwise. Null and any other value, including `""`, are false (`flag_default` row
//!   `id` 14, `cat` blank, all three flags `F`).
//! - `trueValue`/`falseValue` are literals: both integers (`flag_custom_values`: `1` and `0`,
//!   unquoted) give integer storage; both numbers give real; anything else gives strings
//!   (`flag_default`: `"T"` / `"F"`). Measurement is flag, with those two values.
//! - `fieldNameExtension` `f_` as a prefix (`flag_ext_prefix`) and `_f` as a suffix
//!   (`flag_ext_suffix`) do not rename the flags. The saved members have an empty
//!   `flagNameExtension`, and the names stay `cat_red`, `cat_blue`, `cat_green`. A non-empty
//!   per-member `flagNameExtension` is rejected.
//! - With `useAggregateKeyFields` (`flag_aggregate`), input fields other than the keys are
//!   dropped. One row is emitted per key in the order the key is first seen (`A`, `B`, `C`, then
//!   the empty `grp`). A flag is true when any record in the group matches.

use super::field_util::{Column, GroupKey, field_list, literal_values, position, with_columns};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::{Field, Measurement, Storage, Table, Value, ValueSpec};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Direct children of `properties` on the Set to Flag probe streams.
const KNOWN_SETTINGS: &[&str] = &[
    "aggregateKeyFields",
    "annotation",
    "autoLabel",
    "cacheEnabled",
    "createFlagFields",
    "createdBy",
    "creationDate",
    "doReadValues",
    "enabled",
    "falseValue",
    "fieldMappingEssentialFields",
    "fieldNameExtension",
    "fieldNameExtensionEnabled",
    "fieldNameExtensionPosition",
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
    "setField",
    "toolTipText",
    "trueValue",
    "useAggregateKeyFields",
];

const MEMBER_CHILDREN: &[&str] = &[
    "srcFieldName",
    "srcFieldValue",
    "flagNameExtension",
    "prefixNotSuffix",
];

struct Member {
    src_field: String,
    src_value: String,
    flag_name: String,
}

struct SetToFlag {
    members: Vec<Member>,
    true_value: Value,
    false_value: Value,
    storage: Storage,
    aggregate: bool,
    keys: Vec<String>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    reject_unknown(s)?;
    if let Some(text) = settings::text(s, "setField")
        && !text.trim().is_empty()
    {
        return Err(OperatorError::Settings(
            "setField is not supported; list values in createFlagFields".into(),
        ));
    }
    let _enabled = settings::flag(s, "fieldNameExtensionEnabled")?;
    match settings::text(s, "fieldNameExtensionPosition")
        .as_deref()
        .map(str::trim)
    {
        None | Some("" | "prefix" | "suffix") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "fieldNameExtensionPosition: expected prefix or suffix, found {other:?}"
            )));
        }
    }
    // Node-level extension is saved but does not rename flags; see the module docs.
    let _extension = settings::text(s, "fieldNameExtension");

    let members = parse_members(s)?;
    let true_text = settings::text(s, "trueValue").unwrap_or_else(|| "T".into());
    let false_text = settings::text(s, "falseValue").unwrap_or_else(|| "F".into());
    let (storage, values) = literal_values(&[&true_text, &false_text]);
    let mut values = values.into_iter();
    let true_value = values.next().unwrap_or(Value::Null);
    let false_value = values.next().unwrap_or(Value::Null);

    let aggregate = settings::flag(s, "useAggregateKeyFields")?.unwrap_or(false);
    reject_unexpected_children(s, "aggregateKeyFields", &["field"])?;
    let keys = field_list(s, "aggregateKeyFields");
    if keys.iter().any(|key| key.trim().is_empty()) {
        return Err(OperatorError::Settings(
            "aggregateKeyFields: empty field name".into(),
        ));
    }
    if let Some(name) = duplicate(&keys) {
        return Err(OperatorError::Settings(format!(
            "aggregateKeyFields: duplicate key {name:?}"
        )));
    }
    if aggregate {
        for member in &members {
            if keys.iter().any(|key| key == &member.flag_name) {
                return Err(OperatorError::Settings(format!(
                    "flag field {:?} has the same name as an aggregate key",
                    member.flag_name
                )));
            }
        }
    }

    Ok(Box::new(SetToFlag {
        members,
        true_value,
        false_value,
        storage,
        aggregate,
        keys,
    }))
}

fn parse_members(settings: Option<&Element>) -> Result<Vec<Member>, OperatorError> {
    reject_unexpected_children(
        settings,
        "createFlagFields",
        &["persistentObjectListMember"],
    )?;
    let mut members = Vec::new();
    let mut names = Vec::new();
    for member in settings::children(settings, "createFlagFields", "persistentObjectListMember") {
        for child in member.child_elements() {
            let name = child.name.local.as_ref();
            if !MEMBER_CHILDREN.contains(&name) {
                return Err(OperatorError::Settings(format!(
                    "createFlagFields member setting {name:?} is not supported"
                )));
            }
        }
        let src_field = settings::text(Some(member), "srcFieldName").unwrap_or_default();
        if src_field.trim().is_empty() {
            return Err(OperatorError::Settings(
                "createFlagFields: member has empty srcFieldName".into(),
            ));
        }
        let src_value = settings::text(Some(member), "srcFieldValue").unwrap_or_default();
        let extension = settings::text(Some(member), "flagNameExtension").unwrap_or_default();
        if !extension.trim().is_empty() {
            return Err(OperatorError::Settings(
                "createFlagFields flagNameExtension is not supported yet".into(),
            ));
        }
        // Empty extension: the flag is still false/true from prefixNotSuffix, which is unused.
        let _prefix = settings::flag(Some(member), "prefixNotSuffix")?;
        let flag_name = format!("{src_field}_{src_value}");
        if names.iter().any(|seen: &String| seen == &flag_name) {
            return Err(OperatorError::Settings(format!(
                "createFlagFields: duplicate flag field {flag_name:?}"
            )));
        }
        names.push(flag_name.clone());
        members.push(Member {
            src_field,
            src_value,
            flag_name,
        });
    }
    if members.is_empty() {
        return Err(OperatorError::Settings(
            "createFlagFields: no flag values selected".into(),
        ));
    }
    Ok(members)
}

fn reject_unknown(settings: Option<&Element>) -> Result<(), OperatorError> {
    let Some(settings) = settings else {
        return Err(OperatorError::Settings(
            "Set to Flag has no settings".into(),
        ));
    };
    for child in settings.child_elements() {
        let name = child.name.local.as_ref();
        if !KNOWN_SETTINGS.contains(&name) {
            return Err(OperatorError::Settings(format!(
                "Set to Flag setting {name:?} is not supported"
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

fn duplicate(names: &[String]) -> Option<&str> {
    for (index, name) in names.iter().enumerate() {
        if names.iter().take(index).any(|earlier| earlier == name) {
            return Some(name);
        }
    }
    None
}

/// Category text against one cell. Null never matches. Numbers match by value; other
/// storages match the category text exactly.
fn category_matches(cell: &Value, category: &str, storage: Storage) -> bool {
    if cell.is_null() {
        return false;
    }
    match storage {
        Storage::Integer | Storage::Real => category
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .map_or_else(
                || cell.to_string() == category,
                |number| cell.as_f64().is_some_and(|value| value == number),
            ),
        Storage::String => cell.as_str() == Some(category),
        _ => cell.to_string() == category,
    }
}

fn flag_field(name: &str, storage: Storage, true_value: &Value, false_value: &Value) -> Field {
    let mut field = Field::new(name, storage);
    field.measurement = Measurement::Flag;
    field.values = ValueSpec::Flag {
        true_value: true_value.clone(),
        false_value: false_value.clone(),
    };
    field
}

impl Operator for SetToFlag {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Set to Flag needs one input".into()))?;
        let model = input.model();
        for member in &self.members {
            if model.position(&member.flag_name).is_some() {
                return Err(OperatorError::Data(format!(
                    "Set to Flag would replace existing field {:?}",
                    member.flag_name
                )));
            }
        }
        let mut sources = Vec::with_capacity(self.members.len());
        for member in &self.members {
            let pos = position(model, &member.src_field)?;
            let storage = model
                .fields()
                .get(pos)
                .map(|field| field.storage)
                .unwrap_or(Storage::Unknown);
            sources.push((pos, storage));
        }
        let rows = input.rows();
        let mut columns: Vec<Vec<Value>> = self
            .members
            .iter()
            .map(|_| Vec::with_capacity(rows.len()))
            .collect();
        for row in rows {
            for (member_index, member) in self.members.iter().enumerate() {
                let (pos, storage) = sources
                    .get(member_index)
                    .copied()
                    .unwrap_or((0, Storage::Unknown));
                let cell = row.get(pos).unwrap_or(&Value::Null);
                let hit = category_matches(cell, &member.src_value, storage);
                let value = if hit {
                    self.true_value.clone()
                } else {
                    self.false_value.clone()
                };
                if let Some(column) = columns.get_mut(member_index) {
                    column.push(value);
                }
            }
        }

        if !self.aggregate {
            let built = self
                .members
                .iter()
                .zip(columns)
                .map(|(member, values)| Column {
                    field: flag_field(
                        &member.flag_name,
                        self.storage,
                        &self.true_value,
                        &self.false_value,
                    ),
                    values,
                })
                .collect();
            return with_columns(input, built);
        }

        let mut key_pos = Vec::with_capacity(self.keys.len());
        for key in &self.keys {
            key_pos.push(position(model, key)?);
        }
        // Groups in the order their key is first seen; the map finds a key's group.
        let mut groups: Vec<(Vec<Value>, Vec<bool>)> = Vec::new();
        let mut group_of: BTreeMap<GroupKey, usize> = BTreeMap::new();
        for (row_index, row) in rows.iter().enumerate() {
            let key: Vec<Value> = key_pos
                .iter()
                .map(|pos| row.get(*pos).cloned().unwrap_or(Value::Null))
                .collect();
            let hits: Vec<bool> = columns
                .iter()
                .map(|column| match column.get(row_index) {
                    Some(value) => value.total_cmp(&self.true_value).is_eq(),
                    None => false,
                })
                .collect();
            match group_of.get(&GroupKey(key.clone())) {
                Some(&index) => {
                    for (flag, hit) in groups[index].1.iter_mut().zip(hits) {
                        *flag = *flag || hit;
                    }
                }
                None => {
                    group_of.insert(GroupKey(key.clone()), groups.len());
                    groups.push((key, hits));
                }
            }
        }

        let mut fields = Vec::with_capacity(self.keys.len().saturating_add(self.members.len()));
        for key in &self.keys {
            let field = model
                .field(key)
                .cloned()
                .ok_or_else(|| OperatorError::Data(format!("input has no field {key:?}")))?;
            fields.push(field);
        }
        for member in &self.members {
            fields.push(flag_field(
                &member.flag_name,
                self.storage,
                &self.true_value,
                &self.false_value,
            ));
        }
        let out_rows = groups
            .into_iter()
            .map(|(key, hits)| {
                let mut row = key;
                for hit in hits {
                    row.push(if hit {
                        self.true_value.clone()
                    } else {
                        self.false_value.clone()
                    });
                }
                row
            })
            .collect();
        let model = dms_types::DataModel::new(fields)
            .map_err(|error| OperatorError::Data(error.to_string()))?;
        Table::new(model, out_rows).map_err(|error| OperatorError::Data(error.to_string()))
    }
}
