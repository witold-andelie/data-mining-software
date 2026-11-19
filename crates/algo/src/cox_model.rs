//! The Cox regression build node's model: a [`Dataset`] and the survival-time field in,
//! Modeler's Cox nugget out.
//!
//! The trainer ([`crate::cox`]) works on plain vectors. This module reads them from the table:
//!
//! - The target is the status field and must be a flag: its true value means the event happened,
//!   its false value means the record is censored. (Modeler asks for the event value in a
//!   dialog the saved streams do not record.)
//! - The survival time is a numeric field; the inputs are the covariates. A continuous input is
//!   a covariate, any other input a factor with the Type node's categories, the last one the
//!   reference. The time and status fields are never covariates.
//! - A record with a missing time or status, or a negative time, is left out (a missing
//!   covariate is handled by the trainer: Guide, "Blank Handling").
//!
//! GAP: strata, frequency weights, custom terms, time-dependent covariates (the trainer's own).

use crate::common::{data_field, field_categories, mining_field};
use crate::cox::{self, Covariate, CoxData, Settings};
use crate::{AlgoError, Dataset};
use dms_pmml::{CoxModel, CoxTerm, MiningFunction, Model, ModelHeader, Pmml, Usage};
use dms_types::{Measurement, Storage, ValueSpec};

fn gap(what: &str) -> AlgoError {
    AlgoError::Unsupported(format!("GAP: Cox regression {what}"))
}

/// Trains a Cox model on `dataset` (target: status, inputs: covariates) with the survival times
/// in the field at position `time`.
pub fn train(dataset: &Dataset<'_>, time: usize, settings: &Settings) -> Result<Pmml, AlgoError> {
    crate::check_cancelled()?;
    if dataset.frequency.is_some() {
        return Err(gap("with a frequency field"));
    }
    let table = dataset.table;
    let fields = table.model().fields();
    let target_field = fields
        .get(dataset.target)
        .ok_or_else(|| AlgoError::Data("the status field is not in the table".into()))?;
    let time_field = fields
        .get(time)
        .ok_or_else(|| AlgoError::Data("the survival time field is not in the table".into()))?;
    if !matches!(time_field.storage, Storage::Integer | Storage::Real) {
        return Err(AlgoError::Data(format!(
            "the survival time {:?} is not numeric",
            time_field.name
        )));
    }
    let ValueSpec::Flag {
        true_value,
        false_value,
    } = &target_field.values
    else {
        return Err(gap(&format!(
            "with a status field that is not a flag ({:?}): the event value is unknown",
            target_field.name
        )));
    };
    let event_value = true_value.to_string();
    let censored_value = false_value.to_string();

    // the records that have a time and a status
    let mut kept: Vec<(usize, f64, bool)> = Vec::new();
    for (index, row) in table.rows().iter().enumerate() {
        crate::check_cancelled()?;
        let Some(t) = row
            .get(time)
            .filter(|v| !time_field.is_missing(v))
            .and_then(dms_types::Value::as_f64)
            .filter(|t| t.is_finite() && *t >= 0.0)
        else {
            continue;
        };
        let Some(status) = row
            .get(dataset.target)
            .filter(|v| !target_field.is_missing(v))
            .map(ToString::to_string)
        else {
            continue;
        };
        if status == event_value {
            kept.push((index, t, true));
        } else if status == censored_value {
            kept.push((index, t, false));
        }
    }
    if kept.is_empty() {
        return Err(AlgoError::Data(
            "Cox regression has no record with a time and a status".into(),
        ));
    }

    let mut covariates = Vec::new();
    let mut positions = Vec::new();
    for &position in dataset.inputs {
        crate::check_cancelled()?;
        if position == dataset.target || position == time || positions.contains(&position) {
            continue;
        }
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Data("an input field is not in the table".into()))?;
        let covariate = if field.measurement == Measurement::Continuous {
            if !matches!(field.storage, Storage::Integer | Storage::Real) {
                return Err(gap(&format!(
                    "with the date, time or text covariate {:?}",
                    field.name
                )));
            }
            Covariate::Numeric {
                name: field.name.clone(),
                values: kept
                    .iter()
                    .map(|(index, _, _)| {
                        table
                            .rows()
                            .get(*index)
                            .and_then(|row| row.get(position))
                            .filter(|v| !field.is_missing(v))
                            .and_then(dms_types::Value::as_f64)
                            .filter(|x| x.is_finite())
                    })
                    .collect(),
            }
        } else {
            let levels = field_categories(table, position);
            if levels.is_empty() {
                continue;
            }
            Covariate::Factor {
                name: field.name.clone(),
                codes: kept
                    .iter()
                    .map(|(index, _, _)| {
                        let text = table
                            .rows()
                            .get(*index)
                            .and_then(|row| row.get(position))
                            .filter(|v| !field.is_missing(v))?
                            .to_string();
                        levels
                            .iter()
                            .position(|l| *l == text)
                            .and_then(|k| u32::try_from(k).ok())
                    })
                    .collect(),
                levels,
            }
        };
        positions.push(position);
        covariates.push(covariate);
    }
    if covariates.is_empty() {
        return Err(AlgoError::Data("Cox regression has no usable input".into()));
    }

    let data = CoxData {
        time: kept.iter().map(|(_, t, _)| *t).collect(),
        event: kept.iter().map(|(_, _, e)| *e).collect(),
        strata: None,
        weights: None,
        covariates,
    };
    let fit = cox::train(&data, settings)?;

    let mut terms = Vec::with_capacity(fit.parameters.len());
    for (k, parameter) in fit.parameters.iter().enumerate() {
        terms.push(CoxTerm {
            field: parameter.covariate_name.clone(),
            level: parameter.level_label.clone(),
            beta: parameter.beta,
            reference: fit.reference_point.get(k).copied().unwrap_or(0.0),
        });
    }
    let baseline = match fit.baselines.as_slice() {
        [one] => one
            .points
            .iter()
            .map(|p| (p.time, p.cum_hazard))
            .collect::<Vec<_>>(),
        _ => return Err(gap("with strata")),
    };

    let mut dictionary = Vec::new();
    let mut schema = Vec::new();
    schema.extend(mining_field(table, dataset.target, Usage::Target));
    schema.extend(mining_field(table, time, Usage::Active));
    dictionary.extend(data_field(table, dataset.target));
    dictionary.extend(data_field(table, time));
    for &position in &positions {
        dictionary.extend(data_field(table, position));
        schema.extend(mining_field(table, position, Usage::Active));
    }
    let header = ModelHeader {
        name: Some("COXREG".into()),
        function: MiningFunction::Regression,
        algorithm: None,
        mining_schema: schema,
        output: Vec::new(),
        local_transformations: Vec::new(),
    };
    Ok(Pmml {
        version: "3.1".into(),
        data_dictionary: dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Cox(CoxModel {
            header,
            time_field: time_field.name.clone(),
            event_field: target_field.name.clone(),
            event_value,
            terms,
            baseline,
        }),
    })
}
