//! Field Reorder (Field Operations): permutes fields. Values and field metadata are copied.
//!
//! Implemented settings (saved names; the scripting names are in parentheses):
//! - `autoOrderType` (`mode`): `auto` or `custom`.
//! - `nameTypeStorage` (`sort_by`), used when `autoOrderType` is `auto`: `Name` or `Type`.
//!   `Storage` is rejected (`OperatorError::Settings`); no probe sorts by storage.
//! - `ascendingDescending` (`ascending`): `true` (ascending, the default) or `false`.
//! - `fieldReorderTable` (`start_fields` / `end_fields`), used when `autoOrderType` is `custom`:
//!   `fieldName` children in order, split by `dividerIndex`. Names before the divider are the
//!   start fields; names at and after it are the end fields. A missing or negative divider
//!   (the auto-mode file writes `-1`) means there is no start section.
//!
//! Display and bookkeeping settings are ignored: `annotation`, `keywords`, `label`, `autoLabel`,
//! `toolTipText`, `cacheEnabled`, `enabled`, `name`, `position`, `doReadValues`,
//! `fieldMappingEssentialFields`, and the repository / save-date elements.
//!
//! Rules read from the oracle (`corpus/fields/fields_main.csv` through a Type node):
//! - `reorder_custom`: start fields in listed order (`note`, `y`, `flg`), then every field that
//!   is not listed, in input order (`grp`, `cat`, `x`), then end fields in listed order
//!   (`row_name`, `id`).
//! - `reorder_name_asc`: code-point order of the field name (`cat`, `flg`, `grp`, `id`, `note`,
//!   `row_name`, `x`, `y`). `reorder_name_desc` is that order reversed.
//! - `reorder_by_type`: measurement level, then name. The case shows Continuous (`id`, `x`, `y`),
//!   then Flag (`flg`), then Nominal (`cat`, `grp`, `note`, `row_name`). Other levels use the
//!   same alphabetical order of their English names (Categorical, Collection, Continuous,
//!   Default, Flag, Geospatial, Nominal, Ordinal, Typeless). No probe sorts those other levels.
//!
//! A name in the custom list that is not in the input is skipped. A repeated name is placed
//! once, at its first listed position. Neither case is in the probes.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{DataModel, Field, Measurement, Table, Value};
use std::cmp::Ordering;
use std::sync::Arc;

enum Criterion {
    Name,
    Type,
}

enum Mode {
    Auto {
        criterion: Criterion,
        ascending: bool,
    },
    Custom {
        start: Vec<String>,
        end: Vec<String>,
    },
}

struct FieldReorder {
    mode: Mode,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let settings_element = node.settings.as_ref();
    let mode = match settings::text(settings_element, "autoOrderType")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("auto") => {
            let criterion = match settings::text(settings_element, "nameTypeStorage")
                .as_deref()
                .map(str::trim)
            {
                None | Some("") | Some("Name") => Criterion::Name,
                Some("Type") => Criterion::Type,
                Some("Storage") => {
                    return Err(OperatorError::Settings(
                        "nameTypeStorage: Storage is not supported yet".into(),
                    ));
                }
                Some(other) => {
                    return Err(OperatorError::Settings(format!(
                        "nameTypeStorage: unknown criterion {other:?}"
                    )));
                }
            };
            Mode::Auto {
                criterion,
                ascending: settings::flag(settings_element, "ascendingDescending")?.unwrap_or(true),
            }
        }
        Some("custom") => {
            let names: Vec<String> =
                settings::children(settings_element, "fieldReorderTable", "fieldName")
                    .into_iter()
                    .map(|field| field.text().trim().to_string())
                    .filter(|name| !name.is_empty())
                    .collect();
            let divider =
                settings::integer(settings_element, "fieldReorderTable/dividerIndex")?.unwrap_or(0);
            let split = usize::try_from(divider)
                .map(|index| index.min(names.len()))
                .unwrap_or(0);
            let (start, end) = names.split_at(split);
            Mode::Custom {
                start: start.to_vec(),
                end: end.to_vec(),
            }
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "autoOrderType: unknown mode {other:?}"
            )));
        }
    };
    Ok(Box::new(FieldReorder { mode }))
}

impl Operator for FieldReorder {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Field Reorder needs one input".into()))?;
        let order = self.order(input.model().fields());
        project(input, &order)
    }
}

impl FieldReorder {
    fn order(&self, fields: &[Field]) -> Vec<usize> {
        match &self.mode {
            Mode::Auto {
                criterion,
                ascending,
            } => auto_order(fields, criterion, *ascending),
            Mode::Custom { start, end } => custom_order(fields, start, end),
        }
    }
}

fn auto_order(fields: &[Field], criterion: &Criterion, ascending: bool) -> Vec<usize> {
    let mut order: Vec<usize> = (0..fields.len()).collect();
    order.sort_by(|&left, &right| {
        let (Some(left_field), Some(right_field)) = (fields.get(left), fields.get(right)) else {
            return Ordering::Equal;
        };
        let compared = match criterion {
            Criterion::Name => left_field.name.cmp(&right_field.name),
            Criterion::Type => type_rank(left_field.measurement)
                .cmp(&type_rank(right_field.measurement))
                .then(left_field.name.cmp(&right_field.name)),
        };
        if ascending {
            compared
        } else {
            compared.reverse()
        }
    });
    order
}

/// Alphabetical English measurement name. See the module docs for what the oracle shows.
fn type_rank(measurement: Measurement) -> u8 {
    match measurement {
        Measurement::Categorical => 0,
        Measurement::Collection => 1,
        Measurement::Continuous => 2,
        Measurement::Default => 3,
        Measurement::Flag => 4,
        Measurement::Geospatial => 5,
        Measurement::Nominal => 6,
        Measurement::Ordinal => 7,
        Measurement::Typeless => 8,
    }
}

fn custom_order(fields: &[Field], start: &[String], end: &[String]) -> Vec<usize> {
    let mut placed = vec![false; fields.len()];
    let mut order = Vec::with_capacity(fields.len());
    for name in start {
        place(fields, name, &mut placed, &mut order);
    }
    // Names reserved for the end stay out of the middle (`reorder_custom`).
    for index in 0..fields.len() {
        if placed.get(index).copied().unwrap_or(true) {
            continue;
        }
        let Some(field) = fields.get(index) else {
            continue;
        };
        if end.iter().any(|name| name == &field.name) {
            continue;
        }
        if let Some(slot) = placed.get_mut(index) {
            *slot = true;
        }
        order.push(index);
    }
    for name in end {
        place(fields, name, &mut placed, &mut order);
    }
    order
}

fn place(fields: &[Field], name: &str, placed: &mut [bool], order: &mut Vec<usize>) {
    let Some(index) = fields.iter().position(|field| field.name == name) else {
        return;
    };
    let Some(slot) = placed.get_mut(index) else {
        return;
    };
    if *slot {
        return;
    }
    *slot = true;
    order.push(index);
}

fn project(input: &Table, order: &[usize]) -> Result<Table, OperatorError> {
    let source = input.model().fields();
    let mut fields = Vec::with_capacity(order.len());
    for &index in order {
        let field = source.get(index).ok_or_else(|| {
            OperatorError::Data(format!("field index {index} is outside the input"))
        })?;
        fields.push(field.clone());
    }
    let model = DataModel::new(fields).map_err(|error| OperatorError::Data(error.to_string()))?;
    let rows = input
        .rows()
        .iter()
        .map(|row| {
            order
                .iter()
                .map(|index| row.get(*index).cloned().unwrap_or(Value::Null))
                .collect()
        })
        .collect();
    Table::new(model, rows).map_err(|error| OperatorError::Data(error.to_string()))
}
