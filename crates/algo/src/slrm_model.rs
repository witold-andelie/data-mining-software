//! The SLRM build node's model: a table in, Modeler's SLRM nugget out.
//!
//! The trainer ([`crate::slrm`]) works on plain vectors. This module reads them from the table: a
//! continuous input is a numeric predictor, any other input a nominal one with the Type node's
//! categories; the target (the product) is categorical and the response a 0/1 field. The nugget
//! keeps, per product, the counts and cut points the saved `pm_selflearn` nugget keeps
//! ([`dms_pmml::SlrmModel`]).
//!
//! A record with a missing product or response is left out.
//!
//! GAP: continuing the training of an existing model (Modeler keeps its state in a format that
//! is not read), frequency fields, and the trainer's own (model assessment by simulation).

use crate::AlgoError;
use crate::common::{data_field, field_categories, mining_field};
use crate::slrm::{self, NominalColumn, NumericColumn, Settings, SlrmData};
use dms_pmml::{
    MiningFunction, Model, ModelHeader, Pmml, SlrmCell, SlrmInput, SlrmModel, SlrmProduct, Usage,
};
use dms_types::{Measurement, Storage, Table};

fn nominal_of(table: &Table, position: usize, name: &str) -> NominalColumn {
    let labels = field_categories(table, position);
    let field = table.model().fields().get(position);
    NominalColumn {
        name: name.to_string(),
        codes: table
            .rows()
            .iter()
            .map(|row| {
                let value = row.get(position)?;
                if field.is_some_and(|f| f.is_missing(value)) {
                    return None;
                }
                let text = value.to_string();
                labels
                    .iter()
                    .position(|l| *l == text)
                    .and_then(|k| u32::try_from(k).ok())
            })
            .collect(),
        labels,
    }
}

/// Trains an SLRM on `table`: the products in `target`, the 0/1 `response`, the predictors
/// `inputs` (field positions).
pub fn train(
    table: &Table,
    target: usize,
    response: usize,
    inputs: &[usize],
    settings: &Settings,
) -> Result<Pmml, AlgoError> {
    crate::check_cancelled()?;
    let fields = table.model().fields();
    let target_field = fields
        .get(target)
        .ok_or_else(|| AlgoError::Data("the target is not in the table".into()))?;
    let response_field = fields
        .get(response)
        .ok_or_else(|| AlgoError::Data("the response is not in the table".into()))?;
    let mut numeric = Vec::new();
    let mut nominal = Vec::new();
    let mut numeric_positions = Vec::new();
    let mut nominal_positions = Vec::new();
    for &position in inputs {
        crate::check_cancelled()?;
        if position == target || position == response {
            continue;
        }
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Data("an input field is not in the table".into()))?;
        if field.measurement == Measurement::Continuous {
            if !matches!(field.storage, Storage::Integer | Storage::Real) {
                return Err(AlgoError::Unsupported(format!(
                    "GAP: SLRM with the date, time or text predictor {:?}",
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
            numeric_positions.push(position);
        } else if !field_categories(table, position).is_empty() {
            nominal.push(nominal_of(table, position, &field.name));
            nominal_positions.push(position);
        }
    }
    let data = SlrmData {
        numeric,
        nominal,
        target: nominal_of(table, target, &target_field.name),
        response: table
            .rows()
            .iter()
            .map(|row| {
                let value = row.get(response)?;
                if response_field.is_missing(value) {
                    return None;
                }
                match value.as_f64()? {
                    1.0 => Some(true),
                    0.0 => Some(false),
                    _ => None,
                }
            })
            .collect(),
        frequency: None,
    };
    let model = slrm::train(&data, settings)?;

    let mut products = Vec::new();
    for product in 0..model.n_products() {
        let name = model.product_name(product).unwrap_or_default().to_string();
        let (n0, n1) = model.target_counts(product).unwrap_or((0, 0));
        let mut product_inputs = Vec::new();
        for (j, field) in model.numeric_names().iter().enumerate() {
            product_inputs.push(SlrmInput {
                field: field.clone(),
                cuts: Some(model.numeric_cuts(product, j)?),
                cells: model
                    .numeric_pair_counts(product, j)?
                    .into_iter()
                    .map(|c| SlrmCell {
                        value: c.value,
                        n0: c.n0,
                        n1: c.n1,
                    })
                    .collect(),
            });
        }
        for (j, field) in model.nominal_names().iter().enumerate() {
            product_inputs.push(SlrmInput {
                field: field.clone(),
                cuts: None,
                cells: model
                    .nominal_pair_counts(product, j)?
                    .into_iter()
                    .map(|c| SlrmCell {
                        value: c.value,
                        n0: c.n0,
                        n1: c.n1,
                    })
                    .collect(),
            });
        }
        products.push(SlrmProduct {
            name,
            threshold: model.threshold(product).unwrap_or(0.0),
            target_counts: [n0, n1],
            inputs: product_inputs,
        });
    }
    let mut dictionary = Vec::new();
    dictionary.extend(data_field(table, target));
    dictionary.extend(data_field(table, response));
    let mut schema = Vec::new();
    for &position in numeric_positions.iter().chain(&nominal_positions) {
        dictionary.extend(data_field(table, position));
        schema.extend(mining_field(table, position, Usage::Active));
    }
    schema.extend(mining_field(table, target, Usage::Target));
    let mut nugget = SlrmModel::new(target_field.name.clone(), products);
    nugget.header = ModelHeader {
        name: None,
        function: MiningFunction::Classification,
        algorithm: Some("SelfLearning".into()),
        mining_schema: schema,
        output: Vec::new(),
        local_transformations: Vec::new(),
    };
    Ok(Pmml {
        version: "3.2".into(),
        data_dictionary: dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Slrm(nugget),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slrm::{Query, SlrmData};
    use dms_pmml::Value as PValue;
    use dms_types::{DataModel, Field, Role, Value, ValueSpec};

    fn field(name: &str, storage: Storage, measurement: Measurement, role: Role) -> Field {
        let mut field = Field::new(name, storage);
        field.measurement = measurement;
        field.role = role;
        field
    }

    /// Two products with both responses, a continuous and a nominal predictor.
    fn table() -> Table {
        let mut product = field(
            "product",
            Storage::String,
            Measurement::Nominal,
            Role::Target,
        );
        product.values = ValueSpec::Categories(vec![Value::string("A"), Value::string("B")]);
        let mut grade = field("grade", Storage::String, Measurement::Nominal, Role::Input);
        grade.values = ValueSpec::Categories(vec![Value::string("x"), Value::string("y")]);
        let fields = vec![
            product,
            field(
                "response",
                Storage::Integer,
                Measurement::Flag,
                Role::Target,
            ),
            field("age", Storage::Real, Measurement::Continuous, Role::Input),
            grade,
        ];
        let mut rows = Vec::new();
        for i in 0..80_i64 {
            let age = 18.0 + (i * 7 % 50) as f64;
            let product = if i % 3 == 0 { "A" } else { "B" };
            let grade = if i % 5 < 2 { "x" } else { "y" };
            let response =
                i64::from((i * 11 + age as i64) % 4 == 0 || (grade == "x" && i % 2 == 0));
            rows.push(vec![
                Value::string(product),
                Value::Integer(response),
                Value::Real(age),
                Value::string(grade),
            ]);
        }
        Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
    }

    #[test]
    fn slrm_nugget_scores_like_the_trainer() {
        let table = table();
        let settings = Settings::default();
        let pmml = train(&table, 0, 1, &[2, 3], &settings).unwrap();
        let Model::Slrm(nugget) = &pmml.model else {
            panic!("not an SLRM model");
        };
        // the trainer's own model on the same vectors
        let data = SlrmData {
            numeric: vec![NumericColumn {
                name: "age".into(),
                values: table
                    .rows()
                    .iter()
                    .map(|r| r.get(2).and_then(dms_types::Value::as_f64))
                    .collect(),
            }],
            nominal: vec![nominal_of(&table, 3, "grade")],
            target: nominal_of(&table, 0, "product"),
            response: table
                .rows()
                .iter()
                .map(|r| {
                    r.get(1)
                        .and_then(dms_types::Value::as_f64)
                        .map(|x| x == 1.0)
                })
                .collect(),
            frequency: None,
        };
        let model = slrm::train(&data, &settings).unwrap();
        for row in table.rows().iter().take(40) {
            let age = row.get(2).and_then(dms_types::Value::as_f64).unwrap();
            let grade = row.get(3).unwrap().to_string();
            let grade_code = data.nominal[0]
                .labels
                .iter()
                .position(|l| *l == grade)
                .and_then(|k| u32::try_from(k).ok());
            let query = Query {
                numeric: vec![Some(age)],
                nominal: vec![grade_code],
            };
            let age_value = PValue::Number(age);
            let grade_value = PValue::Text(grade);
            let scores = nugget.scores(
                |name| match name {
                    "age" => Some(&age_value),
                    "grade" => Some(&grade_value),
                    _ => None,
                },
                true,
            );
            for (product, score) in scores.iter().enumerate() {
                let expected = model.response_probability(product, &query).unwrap();
                assert!(
                    (score.probability - expected).abs() < 1e-12,
                    "product {product}: {} vs {expected}",
                    score.probability
                );
            }
        }
    }
}
