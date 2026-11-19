//! The GLMM build node's model: a table in, Modeler's GLMM nugget out.
//!
//! The trainer ([`crate::glmm`]) works on plain vectors. This module reads them from the table:
//! the target, the fixed effects (a continuous field is a covariate, any other field a factor
//! with the Type node's categories) and the random-effect blocks (one subject key per record,
//! made of one or more fields), and assembles the nugget the way the saved `testscores_glmm`
//! model is laid out: a `GeneralRegressionModel` of the fixed effects only, which scores the
//! fixed part `x'beta` (the random effects are not part of the PMML).
//!
//! A record with a missing target, fixed effect or subject field is left out. The nugget is
//! written for a normal target with an identity link, the combination checked against Modeler;
//! other distributions train in [`crate::glmm`] but have no nugget here (`GAP:`).
//!
//! GAP: weights, offsets, trials and frequency fields, interactions between fixed effects, and
//! the trainer's own.

use crate::AlgoError;
use crate::common::{data_field, field_categories, mining_field};
use crate::glmm::{
    self, CovarianceType, Distribution, FactorColumn, FixedTerm, GlmmInput, GlmmSettings, Link,
    NumericColumn, RandomSpec,
};
use dms_pmml::{
    GeneralModelType, GeneralRegressionModel, MiningFunction, Model, ModelHeader, PCell, PPCell,
    Pmml, Usage,
};
use dms_types::{Measurement, Storage, Table};

fn gap(what: &str) -> AlgoError {
    AlgoError::Unsupported(format!("GAP: GLMM {what}"))
}

/// One random-effect block: the subject fields (positions in the table), whether it has a
/// random intercept, and its covariance type.
#[derive(Debug, Clone, PartialEq)]
pub struct RandomBlock {
    pub subjects: Vec<usize>,
    pub intercept: bool,
    pub covariance: CovarianceType,
}

/// Trains a GLMM: `target` and the `fixed` effects (field positions, in design order) of `table`.
pub fn train(
    table: &Table,
    target: usize,
    fixed: &[usize],
    random: &[RandomBlock],
    settings: &GlmmSettings,
) -> Result<Pmml, AlgoError> {
    crate::check_cancelled()?;
    if settings.distribution != Distribution::Normal || settings.link != Link::Identity {
        return Err(gap(
            "nugget for a target other than normal with an identity link (no Modeler run to check it)",
        ));
    }
    let fields = table.model().fields();
    let target_field = fields
        .get(target)
        .ok_or_else(|| AlgoError::Data("the target is not in the table".into()))?;
    if !matches!(target_field.storage, Storage::Integer | Storage::Real) {
        return Err(AlgoError::Data(format!(
            "the target {:?} is not numeric",
            target_field.name
        )));
    }

    // a record is used when every field it needs has a value
    let needed: Vec<usize> = std::iter::once(target)
        .chain(fixed.iter().copied())
        .chain(random.iter().flat_map(|b| b.subjects.iter().copied()))
        .collect();
    let rows: Vec<&Vec<dms_types::Value>> = table
        .rows()
        .iter()
        .filter(|row| {
            needed.iter().all(
                |&p| matches!((row.get(p), fields.get(p)), (Some(v), Some(f)) if !f.is_missing(v)),
            )
        })
        .collect();
    if rows.is_empty() {
        return Err(AlgoError::Data("GLMM has no complete record".into()));
    }
    let text = |row: &Vec<dms_types::Value>, p: usize| row.get(p).map(ToString::to_string);
    let y: Vec<f64> = rows
        .iter()
        .map(|row| {
            row.get(target)
                .and_then(dms_types::Value::as_f64)
                .unwrap_or(f64::NAN)
        })
        .collect();

    let mut terms = Vec::new();
    let mut declared: Vec<(usize, Vec<String>)> = Vec::new();
    for &position in fixed {
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Data("a fixed effect is not in the table".into()))?;
        if field.measurement == Measurement::Continuous {
            if !matches!(field.storage, Storage::Integer | Storage::Real) {
                return Err(gap(&format!(
                    "with the date, time or text covariate {:?}",
                    field.name
                )));
            }
            terms.push(FixedTerm::Covariate(NumericColumn {
                name: field.name.clone(),
                values: rows
                    .iter()
                    .map(|row| {
                        row.get(position)
                            .and_then(dms_types::Value::as_f64)
                            .unwrap_or(f64::NAN)
                    })
                    .collect(),
            }));
        } else {
            terms.push(FixedTerm::Factor(FactorColumn {
                name: field.name.clone(),
                levels: rows
                    .iter()
                    .map(|row| text(row, position).unwrap_or_default())
                    .collect(),
            }));
            declared.push((position, field_categories(table, position)));
        }
    }
    let random_specs = random
        .iter()
        .map(|block| RandomSpec {
            subject_names: block
                .subjects
                .iter()
                .filter_map(|&p| fields.get(p).map(|f| f.name.clone()))
                .collect(),
            subject_keys: rows
                .iter()
                .map(|row| {
                    block
                        .subjects
                        .iter()
                        .map(|&p| text(row, p).unwrap_or_default())
                        .collect()
                })
                .collect(),
            intercept: block.intercept,
            slopes: Vec::new(),
            covariance: block.covariance,
        })
        .collect();
    let input = GlmmInput {
        target_name: target_field.name.clone(),
        y,
        fixed: terms,
        random: random_specs,
        weight: None,
        frequency: None,
        offset: None,
        trials: None,
    };
    let fit = glmm::fit(&input, settings)?;

    // the PMML: parameters P0000001.. with their predictor and level
    let mut parameters = Vec::new();
    let mut pp_matrix = Vec::new();
    let mut param_matrix = Vec::new();
    let mut factor_names = Vec::new();
    let mut covariate_names = Vec::new();
    for parameter in &fit.parameters {
        parameters.push(parameter.name.clone());
        param_matrix.push(PCell {
            target_category: None,
            parameter: parameter.name.clone(),
            beta: parameter.estimate,
        });
        let label = parameter.label.as_str();
        if label == "Intercept" {
            continue;
        }
        if let Some(inner) = label.strip_prefix('[').and_then(|l| l.strip_suffix(']'))
            && let Some((name, level)) = inner.split_once('=')
        {
            if !factor_names.iter().any(|n: &String| n == name) {
                factor_names.push(name.to_string());
            }
            pp_matrix.push(PPCell {
                value: level.to_string(),
                predictor: name.to_string(),
                parameter: parameter.name.clone(),
            });
        } else {
            if !covariate_names.iter().any(|n: &String| n == label) {
                covariate_names.push(label.to_string());
            }
            pp_matrix.push(PPCell {
                value: "1".into(),
                predictor: label.to_string(),
                parameter: parameter.name.clone(),
            });
        }
    }
    let mut dictionary = Vec::new();
    dictionary.extend(data_field(table, target));
    let mut schema = Vec::new();
    for &position in fixed {
        dictionary.extend(data_field(table, position));
        schema.extend(mining_field(table, position, Usage::Active));
    }
    schema.extend(mining_field(table, target, Usage::Target));
    Ok(Pmml {
        version: "4.1".into(),
        data_dictionary: dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::GeneralRegression(GeneralRegressionModel {
            header: ModelHeader {
                name: None,
                function: MiningFunction::Regression,
                algorithm: Some("GLMM".into()),
                mining_schema: schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            model_type: GeneralModelType::GeneralizedLinear,
            target_reference_category: None,
            link_function: Some("identity".into()),
            link_parameter: None,
            parameters,
            factors: factor_names,
            covariates: covariate_names,
            pp_matrix,
            param_matrix,
        }),
    })
}
