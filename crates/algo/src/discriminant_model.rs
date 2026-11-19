//! The Discriminant build node's model: a table in, Modeler's Discriminant nugget out.
//!
//! The trainer ([`crate::discriminant`]) works on plain vectors. This module reads them from the
//! table: the target is categorical (its categories, in the Type node's order, are the groups) and
//! the predictors are the numeric inputs. The nugget keeps the canonical functions of the
//! predictors the analysis kept, the functions at the group centroids and the priors
//! ([`dms_pmml::DiscriminantModel`]), as Modeler's nugget does.
//!
//! GAP: separate-groups covariance (a quadratic classification, which the nugget's state cannot
//! express), non-numeric predictors, and the trainer's own.

use crate::AlgoError;
use crate::common::{data_field, field_categories, mining_field};
use crate::discriminant::{self, CovarianceMatrix, DiscriminantData, Predictor, Settings, Target};
use dms_pmml::{DiscriminantModel, Model, Pmml, Usage};
use dms_types::{Storage, Table};

/// Trains a discriminant model on `table`: the groups in `target`, the numeric predictors
/// `inputs` (field positions).
pub fn train(
    table: &Table,
    target: usize,
    inputs: &[usize],
    settings: &Settings,
) -> Result<Pmml, AlgoError> {
    crate::check_cancelled()?;
    if settings.covariance_matrix != CovarianceMatrix::WithinGroups {
        return Err(AlgoError::Unsupported(
            "GAP: Discriminant nugget with separate-groups covariance".into(),
        ));
    }
    let fields = table.model().fields();
    let target_field = fields
        .get(target)
        .ok_or_else(|| AlgoError::Data("the target is not in the table".into()))?;
    let labels = field_categories(table, target);
    if labels.len() < 2 {
        return Err(AlgoError::Data(
            "a discriminant model needs a target with at least two categories".into(),
        ));
    }
    let codes = table
        .rows()
        .iter()
        .map(|row| {
            let value = row.get(target)?;
            if target_field.is_missing(value) {
                return None;
            }
            let text = value.to_string();
            labels
                .iter()
                .position(|l| *l == text)
                .and_then(|k| i64::try_from(k).ok())
        })
        .collect();
    let mut predictors = Vec::new();
    let mut positions = Vec::new();
    for &position in inputs {
        crate::check_cancelled()?;
        if position == target || positions.contains(&position) {
            continue;
        }
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Data("an input field is not in the table".into()))?;
        if !matches!(field.storage, Storage::Integer | Storage::Real) {
            return Err(AlgoError::Unsupported(format!(
                "GAP: Discriminant with the non-numeric predictor {:?}",
                field.name
            )));
        }
        predictors.push(Predictor::new(
            field.name.clone(),
            table
                .rows()
                .iter()
                .map(|row| {
                    row.get(position)
                        .filter(|v| !field.is_missing(v))
                        .and_then(dms_types::Value::as_f64)
                        .filter(|x| x.is_finite())
                })
                .collect(),
        ));
        positions.push(position);
    }
    if predictors.is_empty() {
        return Err(AlgoError::Data(
            "Discriminant has no usable predictor".into(),
        ));
    }
    let data = DiscriminantData {
        target: Target::new(
            target_field.name.clone(),
            codes,
            labels
                .iter()
                .enumerate()
                .filter_map(|(k, l)| Some((i64::try_from(k).ok()?, l.clone())))
                .collect(),
        ),
        predictors,
        weights: None,
    };
    let model = discriminant::train(&data, settings)?;
    let kept: Vec<usize> = model.selected_indices.clone();
    let coefficients: Vec<Vec<f64>> = kept
        .iter()
        .enumerate()
        .map(|(i, _)| {
            model
                .unstandardized_coefficients
                .get(i)
                .cloned()
                .unwrap_or_default()
        })
        .collect();
    let nugget = DiscriminantModel::new(
        target_field.name.clone(),
        model.selected_names.clone(),
        model.groups.iter().map(|g| g.label.clone()).collect(),
        coefficients,
        model.canonical_constants.clone(),
        model.group_centroids.clone(),
        model.groups.iter().map(|g| g.prior).collect(),
    )
    .map_err(|error| AlgoError::Data(error.to_string()))?;
    let mut dictionary = Vec::new();
    dictionary.extend(data_field(table, target));
    let mut schema = Vec::new();
    for &index in &kept {
        if let Some(&position) = positions.get(index) {
            dictionary.extend(data_field(table, position));
            schema.extend(mining_field(table, position, Usage::Active));
        }
    }
    schema.extend(mining_field(table, target, Usage::Target));
    let mut nugget = nugget;
    nugget.header.mining_schema = schema;
    Ok(Pmml {
        version: "3.1".into(),
        data_dictionary: dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Discriminant(nugget),
    })
}
