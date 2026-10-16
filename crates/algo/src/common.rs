//! PMML parts every trained model shares: the data dictionary and the mining schema.

use crate::Dataset;
use dms_pmml::{DataField, DataType, InvalidTreatment, MiningField, OpType, Outliers, Usage};
use dms_types::{Measurement, Storage, Value, ValueSpec};

/// The categories of field `position` in PMML text form: the declared values of an
/// instantiated field, otherwise the distinct non-null values in order of appearance; blanks
/// (user-missing values) are not categories (`models/cart_surrogates`: `Credit_rating` declares
/// 0, 1 and the missing 9, the model lists 0 and 1). Empty for continuous fields. A flag lists
/// its true value first, the order the Type node saves and Modeler's `DataDictionary` uses
/// (`models/cart_default`: `Cholesterol` HIGH, NORMAL); trees send the first category of a
/// subset left, so the order decides the children.
pub fn categories(dataset: &Dataset<'_>, position: usize) -> Vec<String> {
    let Some(field) = dataset.table.model().fields().get(position) else {
        return Vec::new();
    };
    let valid = |value: &&Value| !field.is_missing(value);
    match &field.values {
        ValueSpec::Categories(values) => values.iter().filter(valid).map(text).collect(),
        ValueSpec::Flag {
            true_value,
            false_value,
        } => [true_value, false_value]
            .into_iter()
            .filter(valid)
            .map(text)
            .collect(),
        _ if field.measurement == Measurement::Continuous => Vec::new(),
        _ => {
            let mut seen: Vec<String> = Vec::new();
            for row in dataset.table.rows() {
                if let Some(value) = row.get(position).filter(valid) {
                    let value = text(value);
                    if !seen.contains(&value) {
                        seen.push(value);
                    }
                }
            }
            seen
        }
    }
}

/// `DataDictionary`: the target, then the inputs.
pub fn data_dictionary(dataset: &Dataset<'_>) -> Vec<DataField> {
    let fields = dataset.table.model().fields();
    std::iter::once(dataset.target)
        .chain(dataset.inputs.iter().copied())
        .filter_map(|position| {
            let field = fields.get(position)?;
            let continuous = field.measurement == Measurement::Continuous;
            Some(DataField {
                name: field.name.clone(),
                optype: if continuous {
                    OpType::Continuous
                } else if field.measurement == Measurement::Ordinal {
                    OpType::Ordinal
                } else {
                    OpType::Categorical
                },
                data_type: match field.storage {
                    Storage::Integer => DataType::Integer,
                    Storage::Real => DataType::Double,
                    Storage::Date => DataType::Date,
                    Storage::Time => DataType::Time,
                    Storage::Timestamp => DataType::DateTime,
                    _ => DataType::String,
                },
                values: if continuous {
                    Vec::new()
                } else {
                    categories(dataset, position)
                },
            })
        })
        .collect()
}

/// `MiningSchema`: the target as predicted, the inputs as active, with no treatments.
pub fn mining_schema(dataset: &Dataset<'_>) -> Vec<MiningField> {
    let fields = dataset.table.model().fields();
    let field = |position: usize, usage: Usage| {
        fields.get(position).map(|f| MiningField {
            name: f.name.clone(),
            usage,
            missing_replacement: None,
            outliers: Outliers::AsIs,
            low_value: None,
            high_value: None,
            invalid_treatment: InvalidTreatment::AsIs,
        })
    };
    std::iter::once(field(dataset.target, Usage::Target))
        .chain(dataset.inputs.iter().map(|&p| field(p, Usage::Active)))
        .flatten()
        .collect()
}

/// A value as PMML writes it (integers without a decimal point, reals in shortest form).
fn text(value: &Value) -> String {
    value.to_string()
}
