//! The Temporal Causal Model build node's model: a table in, Modeler's TCM nugget out.
//!
//! The trainer ([`crate::tcm`]) works on aligned series. This module reads them from the table:
//! the rows are the time points in table order (the time field only orders them: sort first),
//! each numeric field with the role Input, Target or Both is a series (an input only predicts,
//! a target only is predicted, a field with both roles does both) and the others are left out.
//! The nugget keeps each modelled series' intercept and lag coefficients
//! ([`dms_pmml::TcmModel`]).
//!
//! GAP: the trainer's own (outlier detection, root cause analysis, scenario analysis,
//! re-estimation).

use crate::AlgoError;
use crate::common::data_field;
use crate::tcm::{self, Series, SeriesAlgorithm, SeriesRole, Settings, TcmData};
use dms_pmml::{Model, Pmml, TcmModel, TcmPredictor, TcmTarget};
use dms_types::{Measurement, Role, Storage, Table};

/// Trains a TCM model on `table`. `exclude` is a field that is not a series (the time field).
pub fn train(
    table: &Table,
    exclude: Option<usize>,
    settings: &Settings,
) -> Result<Pmml, AlgoError> {
    crate::check_cancelled()?;
    let fields = table.model().fields();
    let mut series = Vec::new();
    let mut positions = Vec::new();
    for (position, field) in fields.iter().enumerate() {
        let role = match field.role {
            Role::Input => SeriesRole::InputOnly,
            Role::Target => SeriesRole::TargetOnly,
            Role::Both => SeriesRole::Both,
            _ => continue,
        };
        if Some(position) == exclude
            || field.measurement != Measurement::Continuous
            || !matches!(field.storage, Storage::Integer | Storage::Real)
        {
            continue;
        }
        series.push(Series {
            name: field.name.clone(),
            values: table
                .rows()
                .iter()
                .map(|row| {
                    row.get(position)
                        .filter(|v| !field.is_missing(v))
                        .and_then(dms_types::Value::as_f64)
                        .filter(|x| x.is_finite())
                })
                .collect(),
            role,
        });
        positions.push(position);
    }
    if series.is_empty() {
        return Err(AlgoError::Data("TCM has no series".into()));
    }
    let model = tcm::train(&TcmData::new(series), settings)?;
    let targets = model
        .models
        .iter()
        .map(|m| TcmTarget {
            series: m.target_index,
            ar_only: m.algorithm == SeriesAlgorithm::Ar,
            intercept: m.intercept.value,
            predictors: m
                .predictors
                .iter()
                .map(|p| TcmPredictor {
                    series: p.series_index,
                    coefficients: p.coefficients.clone(),
                })
                .collect(),
        })
        .collect();
    Ok(Pmml {
        version: "4.1".into(),
        data_dictionary: positions
            .iter()
            .filter_map(|&p| data_field(table, p))
            .collect(),
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Tcm(TcmModel::new(
            model.series_names.clone(),
            model.lag,
            targets,
        )),
    })
}
