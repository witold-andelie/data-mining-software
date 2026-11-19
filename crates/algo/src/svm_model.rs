//! The SVM build node's model: a [`Dataset`] in, Modeler's SVM nugget out.
//!
//! The trainer ([`crate::svm`]) works on plain vectors. This module reads them from the table and
//! assembles the nugget around the trained machine, the way the saved `svm_cancer` models are laid
//! out: the support vectors live in the space of derived fields (`<field>_scaled`, linear norm
//! from the training range to `[0, 1]`; `<field>=<category>`, a 0/1 indicator per category), the
//! coefficients are the signed duals, and the Platt parameters turn the decision value into a
//! probability.
//!
//! - The target must be a flag: its true value is the response (`y = +1`).
//! - A continuous input is scaled; every other input is one-hot coded with the Type node's
//!   categories.
//!
//! GAP: a target that is not a flag, frequency fields, and the trainer's own (regression,
//! one-class and multiclass SVM).

use crate::common::{data_field, field_categories, mining_field};
use crate::svm::{self, InputColumn, Settings, SvmData, Target};
use crate::{AlgoError, Dataset};
use dms_pmml::{
    DataType, DerivedField, Expr, Kernel as PmmlKernel, MiningFunction, Model, ModelHeader, OpType,
    Outliers, Pmml, SvmModel as PmmlSvm, Usage,
};
use dms_types::{Measurement, Storage, ValueSpec};

fn gap(what: &str) -> AlgoError {
    AlgoError::Unsupported(format!("GAP: SVM {what}"))
}

/// Trains an SVM on `dataset` (target: a flag, inputs: the predictors).
pub fn train(dataset: &Dataset<'_>, settings: &Settings) -> Result<Pmml, AlgoError> {
    crate::check_cancelled()?;
    if dataset.frequency.is_some() {
        return Err(gap("with a frequency field"));
    }
    let table = dataset.table;
    let fields = table.model().fields();
    let target_field = fields
        .get(dataset.target)
        .ok_or_else(|| AlgoError::Data("the target is not in the table".into()))?;
    let ValueSpec::Flag {
        true_value,
        false_value,
    } = &target_field.values
    else {
        return Err(gap(&format!(
            "with a target that is not a flag ({:?}): the response category is unknown",
            target_field.name
        )));
    };
    let positive_label = true_value.to_string();
    let negative_label = false_value.to_string();
    let target_values = table
        .rows()
        .iter()
        .map(|row| {
            row.get(dataset.target)
                .filter(|v| !target_field.is_missing(v))
                .map(ToString::to_string)
                .and_then(|text| {
                    if text == positive_label {
                        Some(true)
                    } else if text == negative_label {
                        Some(false)
                    } else {
                        None
                    }
                })
        })
        .collect();

    let mut inputs = Vec::new();
    let mut positions = Vec::new();
    for &position in dataset.inputs {
        crate::check_cancelled()?;
        if position == dataset.target || positions.contains(&position) {
            continue;
        }
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Data("an input field is not in the table".into()))?;
        let column = if field.measurement == Measurement::Continuous {
            if !matches!(field.storage, Storage::Integer | Storage::Real) {
                return Err(gap(&format!(
                    "with the date, time or text input {:?}",
                    field.name
                )));
            }
            InputColumn::Numeric {
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
            }
        } else {
            let labels = field_categories(table, position);
            if labels.is_empty() {
                continue;
            }
            InputColumn::Nominal {
                name: field.name.clone(),
                codes: table
                    .rows()
                    .iter()
                    .map(|row| {
                        let text = row
                            .get(position)
                            .filter(|v| !field.is_missing(v))?
                            .to_string();
                        labels.iter().position(|l| *l == text)
                    })
                    .collect(),
                labels,
            }
        };
        positions.push(position);
        inputs.push(column);
    }
    if inputs.is_empty() {
        return Err(AlgoError::Data("SVM has no usable input".into()));
    }
    let data = SvmData {
        num_records: table.rows().len(),
        inputs,
        target: Target::Binary {
            name: target_field.name.clone(),
            positive_label: positive_label.clone(),
            negative_label: negative_label.clone(),
            values: target_values,
        },
    };
    let model = svm::train(&data, settings)?;

    // the derived fields, in the order of the encoded features
    let mut derived: Vec<(usize, DerivedField)> = Vec::new();
    for transform in &model.transforms {
        match transform {
            svm::InputTransform::Numeric {
                name,
                min,
                max,
                feature_index,
            } => {
                // a constant column maps to 0
                let top = if max > min { *max } else { min + 1.0 };
                derived.push((
                    *feature_index,
                    DerivedField {
                        name: format!("{name}_scaled"),
                        data_type: DataType::Double,
                        optype: OpType::Continuous,
                        expr: Expr::NormContinuous {
                            field: name.clone(),
                            points: vec![(*min, 0.0), (top, if max > min { 1.0 } else { 0.0 })],
                            outliers: Outliers::AsIs,
                            map_missing_to: None,
                        },
                    },
                ));
            }
            svm::InputTransform::Nominal {
                name,
                labels,
                start_index,
            } => {
                for (k, label) in labels.iter().enumerate() {
                    derived.push((
                        start_index + k,
                        DerivedField {
                            name: format!("{name}={label}"),
                            data_type: DataType::Double,
                            optype: OpType::Continuous,
                            expr: Expr::NormDiscrete {
                                field: name.clone(),
                                value: label.clone(),
                                map_missing_to: None,
                            },
                        },
                    ));
                }
            }
        }
    }
    derived.sort_by_key(|(index, _)| *index);
    let feature_names: Vec<String> = derived.iter().map(|(_, d)| d.name.clone()).collect();
    if feature_names.len() != model.num_features {
        return Err(AlgoError::Data(
            "the SVM encoding does not match its feature count".into(),
        ));
    }

    let mut dictionary = Vec::new();
    dictionary.extend(data_field(table, dataset.target));
    let mut schema = Vec::new();
    for &position in &positions {
        dictionary.extend(data_field(table, position));
        schema.extend(mining_field(table, position, Usage::Active));
    }
    schema.extend(mining_field(table, dataset.target, Usage::Target));
    let header = ModelHeader {
        name: Some("C-SVC".into()),
        function: MiningFunction::Classification,
        algorithm: None,
        mining_schema: schema,
        output: Vec::new(),
        local_transformations: Vec::new(),
    };
    let kernel = match model.kernel {
        svm::Kernel::Linear => PmmlKernel::Linear,
        svm::Kernel::Polynomial {
            gamma,
            coef0,
            degree,
        } => PmmlKernel::Polynomial {
            gamma,
            coef0,
            degree: f64::from(degree),
        },
        svm::Kernel::Rbf { gamma } => PmmlKernel::RadialBasis { gamma },
        svm::Kernel::Sigmoid { gamma, coef0 } => PmmlKernel::Sigmoid { gamma, coef0 },
    };
    Ok(Pmml {
        version: "3.2".into(),
        data_dictionary: dictionary,
        transformations: derived.into_iter().map(|(_, d)| d).collect(),
        functions: Vec::new(),
        model: Model::Svm(PmmlSvm {
            header,
            target: target_field.name.clone(),
            kernel,
            fields: feature_names,
            vectors: model
                .support_vectors
                .iter()
                .map(|sv| sv.vector.clone())
                .collect(),
            coefficients: model
                .support_vectors
                .iter()
                .map(|sv| sv.coefficient)
                .collect(),
            constant: model.constant,
            target_category: positive_label,
            alternate_category: negative_label,
            platt: Some((model.platt.param_a, model.platt.param_b)),
        }),
    })
}
