//! The KNN build node's model: a table in, Modeler's KNN nugget out.
//!
//! The trainer ([`crate::knn`]) works on plain vectors. This module reads them from the table:
//! a continuous input is a numeric predictor (rescaled by the trainer), every other input a
//! nominal one with the Type node's categories. The target is optional: a model without one
//! identifies neighbours (the saved `car_sales_knn` model). The nugget keeps the model's
//! description ([`dms_pmml::KnnModel`]), not the stored cases.
//!
//! The case ids are the 1-based row numbers of the training table, including the rows left out
//! as incomplete (Modeler's `$casecount`).
//!
//! GAP: frequency fields, date and time predictors, and the trainer's own.

use crate::AlgoError;
use crate::common::{data_field, field_categories, mining_field};
use crate::knn::{self, DistanceMetric, KnnData, NominalColumn, NumericColumn, Settings, Target};
use dms_pmml::{KnnModel, MiningFunction, Model, ModelHeader, Pmml, Usage};
use dms_types::{Measurement, Storage, Table};

fn gap(what: &str) -> AlgoError {
    AlgoError::Unsupported(format!("GAP: KNN {what}"))
}

/// Trains a KNN model on `table`: the predictors at `inputs`, the target at `target` (none to
/// identify neighbours).
pub fn train(
    table: &Table,
    target: Option<usize>,
    inputs: &[usize],
    settings: &Settings,
) -> Result<Pmml, AlgoError> {
    crate::check_cancelled()?;
    let fields = table.model().fields();
    let mut numeric = Vec::new();
    let mut nominal = Vec::new();
    let mut positions = Vec::new();
    for &position in inputs {
        crate::check_cancelled()?;
        if Some(position) == target || positions.contains(&position) {
            continue;
        }
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Data("an input field is not in the table".into()))?;
        if field.measurement == Measurement::Continuous {
            if !matches!(field.storage, Storage::Integer | Storage::Real) {
                return Err(gap(&format!(
                    "with the date, time or text predictor {:?}",
                    field.name
                )));
            }
            numeric.push(NumericColumn {
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
            });
        } else {
            let labels = field_categories(table, position);
            if labels.is_empty() {
                continue;
            }
            nominal.push(NominalColumn {
                name: field.name.clone(),
                codes: table
                    .rows()
                    .iter()
                    .map(|row| {
                        let text = row
                            .get(position)
                            .filter(|v| !field.is_missing(v))?
                            .to_string();
                        labels
                            .iter()
                            .position(|l| *l == text)
                            .and_then(|k| u32::try_from(k).ok())
                    })
                    .collect(),
                labels,
            });
        }
        positions.push(position);
    }
    if positions.is_empty() {
        return Err(AlgoError::Data("KNN has no usable predictor".into()));
    }
    let target_column = match target {
        None => None,
        Some(position) => {
            let field = fields
                .get(position)
                .ok_or_else(|| AlgoError::Data("the target is not in the table".into()))?;
            if field.measurement == Measurement::Continuous {
                Some(Target::Continuous {
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
                })
            } else {
                let labels = field_categories(table, position);
                Some(Target::Categorical {
                    name: field.name.clone(),
                    codes: table
                        .rows()
                        .iter()
                        .map(|row| {
                            let text = row
                                .get(position)
                                .filter(|v| !field.is_missing(v))?
                                .to_string();
                            labels
                                .iter()
                                .position(|l| *l == text)
                                .and_then(|k| u32::try_from(k).ok())
                        })
                        .collect(),
                    labels,
                })
            }
        }
    };
    let data = KnnData {
        numeric,
        nominal,
        target: target_column,
        case_ids: None,
        folds: None,
    };
    let model = knn::train(&data, settings)?;

    let mut dictionary = Vec::new();
    let mut schema = Vec::new();
    if let Some(position) = target {
        dictionary.extend(data_field(table, position));
        schema.extend(mining_field(table, position, Usage::Target));
    }
    for &position in &positions {
        dictionary.extend(data_field(table, position));
        schema.extend(mining_field(table, position, Usage::Active));
    }
    let function = match target.and_then(|p| fields.get(p)) {
        None => MiningFunction::Other("neighbors".into()),
        Some(field) if field.measurement == Measurement::Continuous => MiningFunction::Regression,
        Some(_) => MiningFunction::Classification,
    };
    let header = ModelHeader {
        name: None,
        function,
        algorithm: None,
        mining_schema: schema,
        output: Vec::new(),
        local_transformations: Vec::new(),
    };
    Ok(Pmml {
        version: "3.2".into(),
        data_dictionary: dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Knn(KnnModel {
            header,
            target: target.and_then(|p| fields.get(p)).map(|f| f.name.clone()),
            k: model.k(),
            metric: match model.metric() {
                DistanceMetric::Euclidean => "Euclidean",
                DistanceMetric::CityBlock => "CityBlock",
            }
            .into(),
            rescale: model.rescale(),
            training_cases: model.n_training(),
        }),
    })
}
