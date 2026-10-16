//! Type (Field Operations): passes record data through unchanged and updates field metadata
//! (measurement level, role, known values/ranges/flags, missing-value "blank" definitions, field
//! labels, value labels, check mode).
//!
//! Implemented: `typeTable/defaultValueMode` (`read`, `pass`, `computed`), per-field rows
//! `typeTable/rows/row` (`name`, `direction`, `measure`, `valueMode`, `augment`, `type/measure`,
//! `type/values/value`, `type/valueLabels`, `missingEnabled`, `missing/{enabled,nullIncluded,
//! whitespaceIncluded,rangeIncluded,values/value,rangeLowerBound,rangeUpperBound}`, `annotation`,
//! `check` when `none`). Partition fields (`Role::Partition`) always store known values as
//! [`ValueSpec::Categories`] so downstream `@PARTITION_FIELD`, `@TRAINING_PARTITION`,
//! `@TESTING_PARTITION` and `@VALIDATION_PARTITION` expressions resolve.
//!
//! Not implemented: non-`None` `check` modes (`nullify`, `coerce`, `discard`, `warn`, `abort`),
//! which make `build` return [`OperatorError::Settings`]; `formattingTable` display settings are
//! ignored.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::calendar;
use dms_types::{
    CheckMode, DataModel, Field, Measurement, MissingSpec, Role, Storage, Table, Value, ValueSpec,
};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefaultValueMode {
    Read,
    Pass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowValueMode {
    Inherit { augment: bool },
    Read { augment: bool },
    Pass,
    Local,
    Computed { augment: bool },
}

#[derive(Debug, Clone)]
struct RawTypedValue {
    type_tag: Option<String>,
    text: String,
}

#[derive(Debug, Clone)]
struct RawMissing {
    null_is_blank: bool,
    whitespace_is_blank: bool,
    values: Vec<RawTypedValue>,
    range: Option<(Option<RawTypedValue>, Option<RawTypedValue>)>,
}

#[derive(Debug, Clone)]
struct RowSpec {
    role: Option<Role>,
    measure: Option<Measurement>,
    type_measure: Option<Measurement>,
    value_mode: RowValueMode,
    values: Vec<RawTypedValue>,
    value_labels: Vec<(RawTypedValue, String)>,
    missing: Option<RawMissing>,
    check: CheckMode,
    label: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct TypeTable {
    default_value_mode: DefaultValueMode,
    rows: HashMap<String, RowSpec>,
}

#[derive(Debug, Clone)]
struct FilterRow {
    drop: bool,
    rename: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SourceTables {
    default_drop: bool,
    filter_rows: HashMap<String, FilterRow>,
    type_table: TypeTable,
}

struct TypeOp {
    table: TypeTable,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let table = parse_type_table_with_default(node.settings.as_ref(), DefaultValueMode::Read)?;
    Ok(Box::new(TypeOp { table }))
}

impl Operator for TypeOp {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Type needs one input".into()))?;
        apply_type_table(&self.table, input)
    }
}

pub(crate) fn parse_source_tables(s: Option<&Element>) -> Result<SourceTables, OperatorError> {
    let default_drop = settings::flag(s, "filterTable/defaultDrop")?.unwrap_or(false);
    let mut filter_rows = HashMap::new();
    for row in settings::children(s, "filterTable/rows", "row") {
        let name = row.child("name").map(|e| e.text()).unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let drop = settings::flag(Some(row), "drop")?.unwrap_or(false);
        let rename = settings::text(Some(row), "rename").filter(|r| !r.trim().is_empty());
        filter_rows.insert(name, FilterRow { drop, rename });
    }
    let type_table = parse_type_table_with_default(s, DefaultValueMode::Pass)?;
    Ok(SourceTables {
        default_drop,
        filter_rows,
        type_table,
    })
}

pub(crate) fn apply_source_tables(
    tables: &SourceTables,
    input: Table,
) -> Result<Table, OperatorError> {
    let filtered = if !tables.default_drop && tables.filter_rows.is_empty() {
        input
    } else {
        let mut kept = Vec::new();
        let mut fields = Vec::new();
        for (position, field) in input.model().fields().iter().enumerate() {
            let row = tables.filter_rows.get(&field.name);
            let drop = row.map_or(tables.default_drop, |r| r.drop);
            if drop {
                continue;
            }
            let mut field = field.clone();
            if let Some(rename) = row.and_then(|r| r.rename.as_ref()) {
                field.name = rename.clone();
            }
            kept.push(position);
            fields.push(field);
        }
        let model = DataModel::new(fields).map_err(|e| OperatorError::Data(e.to_string()))?;
        let rows = input
            .rows()
            .iter()
            .map(|row| {
                kept.iter()
                    .map(|p| row.get(*p).cloned().unwrap_or(Value::Null))
                    .collect()
            })
            .collect();
        Table::new(model, rows).map_err(|e| OperatorError::Data(e.to_string()))?
    };
    if tables.type_table.default_value_mode == DefaultValueMode::Pass
        && tables.type_table.rows.is_empty()
    {
        Ok(filtered)
    } else {
        apply_type_table(&tables.type_table, &filtered)
    }
}

fn parse_type_table_with_default(
    s: Option<&Element>,
    fallback: DefaultValueMode,
) -> Result<TypeTable, OperatorError> {
    let default_value_mode = match settings::text(s, "typeTable/defaultValueMode")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") => fallback,
        Some("read") | Some("computed") => DefaultValueMode::Read,
        Some("pass") => DefaultValueMode::Pass,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "typeTable/defaultValueMode: unknown mode {other:?}"
            )));
        }
    };
    let mut rows = HashMap::new();
    for row in settings::children(s, "typeTable/rows", "row") {
        let name = row.child("name").map(|e| e.text()).unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let check = match settings::text(Some(row), "check").as_deref().map(str::trim) {
            None | Some("") | Some("none") => CheckMode::None,
            Some(other) => {
                return Err(OperatorError::Settings(format!(
                    "Type check mode {other:?} on field {name:?} is not supported yet"
                )));
            }
        };
        let role = match settings::text(Some(row), "direction")
            .as_deref()
            .map(str::trim)
        {
            None | Some("") => None,
            Some("in") => Some(Role::Input),
            Some("out") => Some(Role::Target),
            Some("both") => Some(Role::Both),
            Some("none") => Some(Role::None),
            Some("partition") => Some(Role::Partition),
            Some("split") => Some(Role::Split),
            Some("freqWeight") => Some(Role::Frequency),
            Some("record_id") => Some(Role::RecordId),
            Some(other) => {
                return Err(OperatorError::Settings(format!(
                    "Type direction {other:?} on field {name:?} is unknown"
                )));
            }
        };
        let measure = settings::text(Some(row), "measure")
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(parse_measurement)
            .transpose()?;
        let type_measure = settings::text(Some(row), "type/measure")
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(parse_measurement)
            .transpose()?;
        let augment = settings::flag(Some(row), "augment")?.unwrap_or(false);
        let value_mode = match settings::text(Some(row), "valueMode")
            .as_deref()
            .map(str::trim)
        {
            None | Some("") => RowValueMode::Inherit { augment },
            Some("read") => RowValueMode::Read { augment },
            Some("pass") => RowValueMode::Pass,
            Some("local") => RowValueMode::Local,
            Some("computed") => RowValueMode::Computed { augment },
            Some(other) => {
                return Err(OperatorError::Settings(format!(
                    "Type valueMode {other:?} on field {name:?} is unknown"
                )));
            }
        };
        let values = settings::children(Some(row), "type/values", "value")
            .into_iter()
            .map(raw_typed_value)
            .collect();
        let value_labels = parse_raw_value_labels(row);
        let missing = parse_raw_missing(row)?;
        let label = settings::text(Some(row), "annotation").filter(|a| !a.is_empty());
        rows.insert(
            name,
            RowSpec {
                role,
                measure,
                type_measure,
                value_mode,
                values,
                value_labels,
                missing,
                check,
                label,
            },
        );
    }
    Ok(TypeTable {
        default_value_mode,
        rows,
    })
}

fn parse_measurement(text: &str) -> Result<Measurement, OperatorError> {
    match text {
        "range" => Ok(Measurement::Continuous),
        "discrete" => Ok(Measurement::Categorical),
        "flag" => Ok(Measurement::Flag),
        "set" => Ok(Measurement::Nominal),
        "orderedSet" => Ok(Measurement::Ordinal),
        "typeless" => Ok(Measurement::Typeless),
        "auto" => Ok(Measurement::Default),
        other => Err(OperatorError::Settings(format!(
            "Type measure {other:?} is not supported"
        ))),
    }
}

fn raw_typed_value(el: &Element) -> RawTypedValue {
    RawTypedValue {
        type_tag: el.attribute("type").map(str::to_owned),
        text: el.text(),
    }
}

fn parse_raw_value_labels(row: &Element) -> Vec<(RawTypedValue, String)> {
    let Some(container) = settings::element(Some(row), "type/valueLabels") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut pending_key: Option<RawTypedValue> = None;
    for child in container.child_elements() {
        match &*child.name.local {
            "valueKey" => pending_key = Some(raw_typed_value(child)),
            "valueLabel" => {
                if let Some(key) = pending_key.take() {
                    out.push((key, child.text()));
                }
            }
            _ => {}
        }
    }
    out
}

fn parse_raw_missing(row: &Element) -> Result<Option<RawMissing>, OperatorError> {
    let row_enabled = settings::flag(Some(row), "missingEnabled")?;
    let Some(missing_el) = row.child("missing") else {
        if row_enabled == Some(true) {
            return Ok(Some(RawMissing {
                null_is_blank: true,
                whitespace_is_blank: true,
                values: Vec::new(),
                range: None,
            }));
        }
        return Ok(None);
    };
    let enabled = settings::flag(Some(missing_el), "enabled")?;
    let null_is_blank = settings::flag(Some(missing_el), "nullIncluded")?.unwrap_or(true);
    let whitespace_is_blank =
        settings::flag(Some(missing_el), "whitespaceIncluded")?.unwrap_or(true);
    let range_included = settings::flag(Some(missing_el), "rangeIncluded")?.unwrap_or(false);
    let values: Vec<RawTypedValue> = settings::children(Some(missing_el), "values", "value")
        .into_iter()
        .map(raw_typed_value)
        .collect();
    let range = if range_included {
        let low = missing_el.child("rangeLowerBound").map(raw_typed_value);
        let high = missing_el.child("rangeUpperBound").map(raw_typed_value);
        Some((low, high))
    } else {
        None
    };
    let is_active =
        enabled != Some(false) || row_enabled == Some(true) || !values.is_empty() || range_included;
    if !is_active {
        return Ok(None);
    }
    Ok(Some(RawMissing {
        null_is_blank,
        whitespace_is_blank,
        values,
        range,
    }))
}

pub(crate) fn apply_type_table(
    table_spec: &TypeTable,
    input: &Table,
) -> Result<Table, OperatorError> {
    let mut fields = Vec::with_capacity(input.model().len());
    for (col_idx, input_field) in input.model().fields().iter().enumerate() {
        let mut field = input_field.clone();
        let row = table_spec.rows.get(&field.name);
        if let Some(row) = row {
            if let Some(role) = row.role {
                field.role = role;
            }
            field.check = row.check;
            if let Some(label) = &row.label {
                field.label = Some(label.clone());
            }
            if !row.value_labels.is_empty() {
                field.value_labels = row
                    .value_labels
                    .iter()
                    .filter_map(|(k, v)| {
                        parse_typed_value(k, field.storage).map(|val| (val, v.clone()))
                    })
                    .collect();
            }
            if let Some(raw_missing) = &row.missing {
                field.missing = build_missing_spec(raw_missing, field.storage);
            }
        }

        let explicit_discrete = row.and_then(|r| r.measure) == Some(Measurement::Categorical);
        if let Some(row) = row {
            match row.measure {
                Some(Measurement::Default) => {
                    if let Some(tm) = row.type_measure
                        && tm != Measurement::Default
                    {
                        field.measurement = tm;
                    }
                }
                Some(m) => field.measurement = m,
                None => {
                    if let Some(tm) = row.type_measure
                        && tm != Measurement::Default
                    {
                        field.measurement = tm;
                    }
                }
            }
        }
        if field.storage == Storage::String && field.measurement == Measurement::Continuous {
            field.measurement = Measurement::Categorical;
        }

        let effective_mode = match row.map(|r| r.value_mode) {
            Some(RowValueMode::Pass) => RowValueMode::Pass,
            Some(RowValueMode::Local) => RowValueMode::Local,
            Some(RowValueMode::Computed { augment }) => RowValueMode::Computed { augment },
            Some(RowValueMode::Read { augment }) => RowValueMode::Read { augment },
            Some(RowValueMode::Inherit { augment }) => match table_spec.default_value_mode {
                DefaultValueMode::Pass => RowValueMode::Pass,
                DefaultValueMode::Read => RowValueMode::Read { augment },
            },
            None => match table_spec.default_value_mode {
                DefaultValueMode::Pass => RowValueMode::Pass,
                DefaultValueMode::Read => RowValueMode::Read { augment: false },
            },
        };

        if field.measurement == Measurement::Typeless {
            field.values = ValueSpec::Unknown;
            fields.push(field);
            continue;
        }

        match effective_mode {
            RowValueMode::Pass => {
                if field.role == Role::Partition && matches!(field.values, ValueSpec::Categories(_))
                {
                    // Preserve upstream partition values (e.g. from PartitionNode).
                } else if row.is_some_and(|r| matches!(r.value_mode, RowValueMode::Pass)) {
                    field.values = ValueSpec::Unknown;
                }
            }
            RowValueMode::Local | RowValueMode::Computed { .. } | RowValueMode::Read { .. } => {
                let augment = matches!(
                    effective_mode,
                    RowValueMode::Computed { augment: true } | RowValueMode::Read { augment: true }
                );
                let saved: Vec<Value> = row
                    .map(|r| {
                        r.values
                            .iter()
                            .filter_map(|v| parse_typed_value(v, field.storage))
                            .collect()
                    })
                    .unwrap_or_default();
                let use_saved_only = matches!(
                    effective_mode,
                    RowValueMode::Local | RowValueMode::Computed { augment: false }
                ) && !saved.is_empty();

                if field.measurement == Measurement::Default
                    || (field.measurement == Measurement::Categorical && !explicit_discrete)
                {
                    if field.role == Role::Partition {
                        field.measurement = Measurement::Nominal;
                    } else if matches!(
                        field.storage,
                        Storage::Integer
                            | Storage::Real
                            | Storage::Date
                            | Storage::Time
                            | Storage::Timestamp
                    ) && field.measurement == Measurement::Default
                    {
                        field.measurement = Measurement::Continuous;
                    } else {
                        let distinct = if use_saved_only {
                            saved.clone()
                        } else {
                            distinct_column_values(input, col_idx, &field, &saved, augment)
                        };
                        field.measurement = if distinct.len() == 2 {
                            Measurement::Flag
                        } else {
                            Measurement::Nominal
                        };
                    }
                }

                field.values =
                    compute_value_spec(input, col_idx, &field, &saved, use_saved_only, augment);
            }
            RowValueMode::Inherit { .. } => {}
        }

        fields.push(field);
    }
    let model = DataModel::new(fields).map_err(|e| OperatorError::Data(e.to_string()))?;
    Table::new(model, input.rows().to_vec()).map_err(|e| OperatorError::Data(e.to_string()))
}

fn compute_value_spec(
    input: &Table,
    col_idx: usize,
    field: &Field,
    saved: &[Value],
    use_saved_only: bool,
    augment: bool,
) -> ValueSpec {
    if field.role == Role::Partition {
        let cats = if use_saved_only {
            saved.to_vec()
        } else {
            distinct_column_values(input, col_idx, field, saved, augment)
        };
        return if cats.is_empty() {
            match &field.values {
                ValueSpec::Categories(existing) if !existing.is_empty() => {
                    ValueSpec::Categories(existing.clone())
                }
                _ => ValueSpec::Unknown,
            }
        } else {
            ValueSpec::Categories(cats)
        };
    }

    match field.measurement {
        Measurement::Continuous => {
            if use_saved_only {
                match (saved.first(), saved.last()) {
                    (Some(low), Some(high)) => ValueSpec::Range {
                        low: low.clone(),
                        high: high.clone(),
                    },
                    _ => ValueSpec::Unknown,
                }
            } else {
                let mut bounds: Option<(Value, Value)> = if augment {
                    match &field.values {
                        ValueSpec::Range { low, high } => Some((low.clone(), high.clone())),
                        _ => saved
                            .first()
                            .zip(saved.last())
                            .map(|(a, b)| (a.clone(), b.clone())),
                    }
                } else {
                    None
                };
                for row in input.rows() {
                    let Some(val) = row.get(col_idx) else {
                        continue;
                    };
                    if val.is_null() || field.missing.is_blank(val) {
                        continue;
                    }
                    bounds = Some(match bounds {
                        None => (val.clone(), val.clone()),
                        Some((min, max)) => {
                            let new_min = if val.total_cmp(&min).is_lt() {
                                val.clone()
                            } else {
                                min
                            };
                            let new_max = if val.total_cmp(&max).is_gt() {
                                val.clone()
                            } else {
                                max
                            };
                            (new_min, new_max)
                        }
                    });
                }
                if bounds.is_none()
                    && let (Some(min), Some(max)) = (saved.first(), saved.last())
                {
                    bounds = Some((min.clone(), max.clone()));
                }
                bounds.map_or(ValueSpec::Unknown, |(low, high)| ValueSpec::Range {
                    low,
                    high,
                })
            }
        }
        Measurement::Flag => {
            let candidates = if use_saved_only {
                saved.to_vec()
            } else {
                distinct_column_values(input, col_idx, field, saved, augment)
            };
            match (candidates.first(), candidates.get(1)) {
                (Some(t), Some(f)) => ValueSpec::Flag {
                    true_value: t.clone(),
                    false_value: f.clone(),
                },
                (Some(only), None) => ValueSpec::Flag {
                    true_value: only.clone(),
                    false_value: only.clone(),
                },
                _ => ValueSpec::Unknown,
            }
        }
        Measurement::Nominal | Measurement::Ordinal => {
            let cats = if use_saved_only {
                saved.to_vec()
            } else {
                distinct_column_values(input, col_idx, field, saved, augment)
            };
            ValueSpec::Categories(cats)
        }
        Measurement::Categorical => {
            if !saved.is_empty() {
                ValueSpec::Categories(saved.to_vec())
            } else {
                ValueSpec::Unknown
            }
        }
        Measurement::Typeless
        | Measurement::Default
        | Measurement::Collection
        | Measurement::Geospatial => ValueSpec::Unknown,
    }
}

fn distinct_column_values(
    input: &Table,
    col_idx: usize,
    field: &Field,
    saved: &[Value],
    augment: bool,
) -> Vec<Value> {
    let mut values = Vec::new();
    if augment {
        for v in saved {
            if !v.is_null() && !field.missing.is_blank(v) {
                values.push(v.clone());
            }
        }
        if let ValueSpec::Categories(existing) = &field.values {
            for v in existing {
                if !v.is_null() && !field.missing.is_blank(v) {
                    values.push(v.clone());
                }
            }
        }
    }
    for row in input.rows() {
        let Some(val) = row.get(col_idx) else {
            continue;
        };
        if val.is_null() || field.missing.is_blank(val) {
            continue;
        }
        values.push(val.clone());
    }
    if values.is_empty() && !saved.is_empty() {
        return saved.to_vec();
    }
    values.sort_by(Value::total_cmp);
    values.dedup_by(|a, b| a.total_cmp(b).is_eq());
    values
}

fn build_missing_spec(raw: &RawMissing, storage: Storage) -> MissingSpec {
    let values = raw
        .values
        .iter()
        .filter_map(|v| parse_typed_value(v, storage))
        .collect();
    let mut ranges = Vec::new();
    if let Some((low_raw, high_raw)) = &raw.range {
        let low = low_raw
            .as_ref()
            .and_then(|v| parse_typed_value(v, storage))
            .or_else(|| default_bound(storage, false));
        let high = high_raw
            .as_ref()
            .and_then(|v| parse_typed_value(v, storage))
            .or_else(|| default_bound(storage, true));
        if let (Some(l), Some(h)) = (low, high) {
            ranges.push((l, h));
        }
    }
    MissingSpec {
        values,
        ranges,
        null_is_blank: raw.null_is_blank,
        whitespace_is_blank: raw.whitespace_is_blank,
    }
}

fn default_bound(storage: Storage, upper: bool) -> Option<Value> {
    match storage {
        Storage::Integer => Some(Value::Integer(if upper { i64::MAX } else { i64::MIN })),
        Storage::Real => Some(Value::Real(if upper {
            f64::INFINITY
        } else {
            f64::NEG_INFINITY
        })),
        _ => None,
    }
}

fn parse_typed_value(raw: &RawTypedValue, storage: Storage) -> Option<Value> {
    let text = raw.text.as_str();
    match storage {
        Storage::Integer => text
            .trim()
            .parse::<i64>()
            .ok()
            .or_else(|| {
                text.trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|f| f.is_finite() && *f >= i64::MIN as f64 && *f <= i64::MAX as f64)
                    .map(|f| {
                        #[allow(clippy::cast_possible_truncation)]
                        let n = f as i64;
                        n
                    })
            })
            .map(Value::Integer),
        Storage::Real => text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|f| f.is_finite())
            .map(Value::real),
        Storage::String => Some(Value::string(text)),
        Storage::Date => parse_iso_date(text.trim()).map(Value::Date),
        Storage::Time => parse_iso_time(text.trim()).map(Value::Time),
        Storage::Timestamp => parse_iso_timestamp(text.trim()).map(Value::Timestamp),
        Storage::Unknown | Storage::List => match raw.type_tag.as_deref() {
            Some("I" | "J") => text.trim().parse::<i64>().ok().map(Value::Integer),
            Some("D") => text
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|f| f.is_finite())
                .map(Value::real),
            Some("A") => Some(Value::string(text)),
            _ => text
                .trim()
                .parse::<i64>()
                .ok()
                .map(Value::Integer)
                .or_else(|| {
                    text.trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|f| f.is_finite())
                        .map(Value::real)
                })
                .or_else(|| Some(Value::string(text))),
        },
    }
}

fn parse_iso_date(text: &str) -> Option<i64> {
    let mut parts = text.split('-');
    let y: i64 = parts.next()?.trim().parse().ok()?;
    let m: u32 = parts.next()?.trim().parse().ok()?;
    let d: u32 = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    calendar::days_from_civil(y, m, d)
}

fn parse_iso_time(text: &str) -> Option<i64> {
    let (clock, frac) = match text.split_once('.') {
        Some((c, f)) => (c, Some(f)),
        None => (text, None),
    };
    let mut parts = clock.split(':');
    let h: u32 = parts.next()?.trim().parse().ok()?;
    let m: u32 = parts.next()?.trim().parse().ok()?;
    let s: u32 = parts.next().map_or(Some(0), |s| s.trim().parse().ok())?;
    if parts.next().is_some() {
        return None;
    }
    let micros = match frac {
        None => 0,
        Some(digits) => {
            let kept = digits.get(..digits.len().min(6))?;
            let n: u32 = kept.parse().ok()?;
            n * 10u32.pow(u32::try_from(6 - kept.len()).ok()?)
        }
    };
    calendar::micros_from_time(h, m, s, micros)
}

fn parse_iso_timestamp(text: &str) -> Option<i64> {
    let (d, t) = text.split_once([' ', 'T'])?;
    let days = parse_iso_date(d.trim())?;
    let micros = parse_iso_time(t.trim())?;
    days.checked_mul(calendar::MICROS_PER_DAY)?
        .checked_add(micros)
}
