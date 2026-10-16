//! PMML parts every trained model shares: the data dictionary and the mining schema.

use crate::Dataset;
use dms_pmml::{
    DataField, DataType, InvalidTreatment, MiningField, OpType, Outliers, ScoreDistribution, Usage,
};
use dms_types::{Measurement, Storage, Table, Value, ValueSpec};

/// The categories of field `position` in PMML text form: the declared values of an
/// instantiated field, otherwise the distinct non-null values in order of appearance; blanks
/// (user-missing values) are not categories (`models/cart_surrogates`: `Credit_rating` declares
/// 0, 1 and the missing 9, the model lists 0 and 1). Empty for continuous fields. A flag lists
/// its false value first, the order the Type node saves and Modeler's `DataDictionary` uses
/// (`models/cart_default`: `Cholesterol` HIGH, NORMAL with NORMAL true); trees send the first
/// category of a subset left, so the order decides the children.
pub fn categories(dataset: &Dataset<'_>, position: usize) -> Vec<String> {
    field_categories(dataset.table, position)
}

/// [`categories`] of a table's field, for models without a target (clustering).
pub fn field_categories(table: &Table, position: usize) -> Vec<String> {
    let Some(field) = table.model().fields().get(position) else {
        return Vec::new();
    };
    let valid = |value: &&Value| !field.is_missing(value);
    match &field.values {
        ValueSpec::Categories(values) => values.iter().filter(valid).map(text).collect(),
        ValueSpec::Flag {
            true_value,
            false_value,
        } => [false_value, true_value]
            .into_iter()
            .filter(valid)
            .map(text)
            .collect(),
        _ if field.measurement == Measurement::Continuous => Vec::new(),
        _ => {
            let mut seen: Vec<String> = Vec::new();
            for row in table.rows() {
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
    std::iter::once(dataset.target)
        .chain(dataset.inputs.iter().copied())
        .filter_map(|position| data_field(dataset.table, position))
        .collect()
}

/// The `DataField` of a table's field: its optype, data type and categories.
pub fn data_field(table: &Table, position: usize) -> Option<DataField> {
    let field = table.model().fields().get(position)?;
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
            field_categories(table, position)
        },
    })
}

/// `MiningSchema`: the target as predicted, the inputs as active, with no treatments.
pub fn mining_schema(dataset: &Dataset<'_>) -> Vec<MiningField> {
    std::iter::once(mining_field(dataset.table, dataset.target, Usage::Target))
        .chain(
            dataset
                .inputs
                .iter()
                .map(|&p| mining_field(dataset.table, p, Usage::Active)),
        )
        .flatten()
        .collect()
}

/// A `MiningField` without treatments.
pub fn mining_field(table: &Table, position: usize, usage: Usage) -> Option<MiningField> {
    table.model().fields().get(position).map(|f| MiningField {
        name: f.name.clone(),
        usage,
        missing_replacement: None,
        outliers: Outliers::AsIs,
        low_value: None,
        high_value: None,
        invalid_treatment: InvalidTreatment::AsIs,
    })
}

/// A value as PMML writes it (integers without a decimal point, reals in shortest form).
fn text(value: &Value) -> String {
    value.to_string()
}

/// A classification node's prediction and distribution: the most probable class under the
/// priors (`π(j) N_j(t) / N_j`; the first on ties) and one `ScoreDistribution` per class with
/// probability = confidence = the class's share of the node (`models/cart_*`, `models/chaid_*`).
pub(crate) fn class_summary(
    labels: &[String],
    counts: &[f64],
    priors: &[f64],
    class_totals: &[f64],
) -> (String, Vec<ScoreDistribution>) {
    let n: f64 = counts.iter().sum();
    let weight = |j: usize| {
        if class_totals[j] > 0.0 {
            priors[j] * counts[j] / class_totals[j]
        } else {
            0.0
        }
    };
    // Equal shares can differ in the last bit through the priors (`chaid_default` node 10:
    // 15 drugC and 15 drugX predict drugC), so a later class must be clearly more probable.
    let mut best = 0;
    for j in 1..labels.len() {
        if weight(j) > weight(best) * (1.0 + 1e-12) {
            best = j;
        }
    }
    let distribution = labels
        .iter()
        .zip(counts)
        .map(|(label, &count)| {
            let share = if n > 0.0 { count / n } else { 0.0 };
            ScoreDistribution {
                value: label.clone(),
                record_count: count,
                probability: Some(share),
                confidence: Some(share),
            }
        })
        .collect();
    (labels.get(best).cloned().unwrap_or_default(), distribution)
}

/// The smallest and largest of the values.
pub(crate) fn value_range(values: impl Iterator<Item = f64>) -> (f64, f64) {
    values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), y| {
        (lo.min(y), hi.max(y))
    })
}

/// A regression node's prediction and distribution: the mean, and the counts of ten
/// equal-width bins of the target's training range `(low, high)` (values `0`..`9`).
pub(crate) fn regression_summary(
    values: impl Iterator<Item = f64>,
    (low, high): (f64, f64),
) -> (String, Vec<ScoreDistribution>) {
    let width = (high - low) / 10.0;
    let mut bins = [0.0; 10];
    let (mut n, mut sum) = (0.0, 0.0);
    for y in values {
        n += 1.0;
        sum += y;
        let bin = if width > 0.0 {
            (((y - low) / width).floor() as usize).min(9)
        } else {
            0
        };
        bins[bin] += 1.0;
    }
    let mean = if n > 0.0 { sum / n } else { 0.0 };
    let distribution = bins
        .iter()
        .enumerate()
        .map(|(bin, &count)| ScoreDistribution {
            value: bin.to_string(),
            record_count: count,
            probability: None,
            confidence: None,
        })
        .collect();
    (number_text(mean), distribution)
}

/// A split value in full precision (the shortest text that reads back to the same double).
/// Modeler's PMML prints 15 significant digits, but its nugget scores with the exact bound: a
/// record whose value is the bound itself must go where training sent it (`models/chaid_alpha`
/// row 1424: `Age` 35.446639274766213 at the bound printed as 35.4466392747662).
pub(crate) fn split_text(x: f64) -> String {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{x:.0}")
    } else {
        x.to_string()
    }
}

/// A number as Modeler writes predictions: 15 significant digits, trailing zeros dropped
/// (`0.0477765`, `62.9004775217086`, `1`).
pub(crate) fn number_text(x: f64) -> String {
    if !x.is_finite() {
        return x.to_string();
    }
    if x == 0.0 {
        return "0".into();
    }
    let scientific = format!("{x:.14e}");
    let exponent: i32 = scientific
        .split_once('e')
        .and_then(|(_, e)| e.parse().ok())
        .unwrap_or(0);
    if !(-5..15).contains(&exponent) {
        return scientific;
    }
    let decimals = usize::try_from(14 - exponent).unwrap_or(0);
    let mut text = format!("{x:.decimals$}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::number_text;

    #[test]
    fn numbers_have_fifteen_significant_digits() {
        assert_eq!(number_text(0.0477765), "0.0477765");
        assert_eq!(
            number_text((62.451230569719314 + 63.34972447369782) / 2.0),
            "62.9004775217086"
        );
        assert_eq!(number_text(1.0), "1");
        assert_eq!(number_text(44.0), "44");
        assert_eq!(number_text(0.684171809523809), "0.684171809523809");
    }
}
