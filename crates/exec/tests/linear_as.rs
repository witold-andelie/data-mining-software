//! Synthetic checks of the Guide's selection and data edge cases, without corpus files.
#![allow(clippy::unwrap_used, clippy::panic, reason = "test assertions")]

use dms_algo::{AlgoError, Dataset, linear_as};
use dms_pmml::Model;
use dms_types::{DataModel, Field, Measurement, Storage, Table, Value, ValueSpec};

fn numeric_table(names: &[&str], rows: Vec<Vec<Value>>) -> Table {
    let fields = names
        .iter()
        .map(|name| {
            let mut f = Field::new(*name, Storage::Real);
            f.measurement = Measurement::Continuous;
            f
        })
        .collect();
    Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
}

#[test]
fn stepwise_removes_the_initial_proxy_and_listwise_deletes_missing_rows() {
    let mut rows: Vec<_> = (0..64)
        .map(|i| {
            let bit = |b| if i & (1 << b) == 0 { -1.0 } else { 1.0 };
            let (b, c, d, e) = (bit(0), bit(1), bit(2), bit(3));
            vec![
                Value::Real(b + c + 0.5 * d),
                Value::Real(b),
                Value::Real(c),
                Value::Real(4.0 + b + c + 0.1 * e),
            ]
        })
        .collect();
    rows.push(vec![
        Value::Null,
        Value::Real(1e10),
        Value::Real(1e10),
        Value::Real(1e10),
    ]);
    rows.push(vec![
        Value::Real(1e10),
        Value::Real(1e10),
        Value::Real(1e10),
        Value::Null,
    ]);
    let table = numeric_table(&["proxy", "b", "c", "y"], rows);
    let model = linear_as::train(&Dataset {
        table: &table,
        target: 3,
        inputs: &[0, 1, 2],
        frequency: None,
    })
    .unwrap();
    let labels: Vec<_> = model.parameters.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, ["Intercept", "b", "c"]);
    let Model::GeneralRegression(m) = model.pmml.model else {
        panic!("wrong model");
    };
    for (c, beta) in m.param_matrix.iter().zip([4.0, 1.0, 1.0]) {
        assert!((c.beta - beta).abs() < 1e-12, "{} vs {beta}", c.beta);
    }
}

#[test]
fn duplicate_and_constant_covariates_keep_the_first_candidate() {
    let table = numeric_table(
        &["first", "duplicate", "constant", "y"],
        (-10..11)
            .map(|x| {
                vec![
                    Value::Real(x as f64),
                    Value::Real(x as f64),
                    Value::Real(1.0),
                    Value::Real(2.0 + 3.0 * x as f64),
                ]
            })
            .collect(),
    );
    let model = linear_as::train(&Dataset {
        table: &table,
        target: 3,
        inputs: &[0, 1, 2],
        frequency: None,
    })
    .unwrap();
    assert_eq!(model.parameters[1].label, "first");
    assert!(
        model
            .parameters
            .iter()
            .all(|p| p.label != "duplicate" && p.label != "constant")
    );
}

fn factor_table(slopes: bool, varying_x: bool) -> Table {
    let mut f = Field::new("g<&\"", Storage::String);
    f.measurement = Measurement::Nominal;
    // Include a level with no records; preserve declared order, including escaping.
    f.values = ValueSpec::Categories(
        ["a<&\"", "b", "unused"]
            .into_iter()
            .map(Value::string)
            .collect(),
    );
    let mut x = Field::new("x", Storage::Real);
    x.measurement = Measurement::Continuous;
    let mut y = Field::new("y", Storage::Real);
    y.measurement = Measurement::Continuous;
    Table::new(
        DataModel::new(vec![f, x, y]).unwrap(),
        (0..20)
            .flat_map(|i| {
                [false, true].map(move |second| {
                    let x = if varying_x { i as f64 } else { 2.0 };
                    let slope = if second && slopes { 3.0 } else { 1.0 };
                    vec![
                        Value::string(if second { "b" } else { "a<&\"" }),
                        Value::Real(x),
                        Value::Real(2.0 + f64::from(second) + slope * x),
                    ]
                })
            })
            .collect(),
    )
    .unwrap()
}

#[test]
fn zero_residual_interaction_is_significant_only_when_slopes_differ() {
    for (slopes, varying_x, want_interaction) in [
        (true, true, true),
        (false, true, false),
        (true, false, false),
    ] {
        let table = factor_table(slopes, varying_x);
        let model = linear_as::train(&Dataset {
            table: &table,
            target: 2,
            inputs: &[0, 1],
            frequency: None,
        })
        .unwrap();
        let has_interaction = model.parameters.iter().any(|p| p.label.contains("] * x"));
        assert_eq!(
            has_interaction, want_interaction,
            "slopes={slopes}, varying_x={varying_x}"
        );
        if slopes && varying_x {
            let reference = model
                .parameters
                .iter()
                .find(|p| p.label == "[g<&\"=unused] * x")
                .unwrap();
            assert_eq!(reference.df, 0);
        }
    }
}

#[test]
fn invalid_data_and_unverified_pair_search_return_errors() {
    let table = numeric_table(
        &["x", "y"],
        vec![
            vec![Value::Real(0.0), Value::Real(0.0)],
            vec![Value::Real(1.0), Value::Real(1.0)],
        ],
    );
    let data = Dataset {
        table: &table,
        target: 1,
        inputs: &[0],
        frequency: None,
    };
    for bad in [
        Dataset {
            target: usize::MAX,
            ..data
        },
        Dataset {
            inputs: &[usize::MAX],
            ..data
        },
        Dataset {
            inputs: &[0, 0],
            ..data
        },
        Dataset {
            inputs: &[1],
            ..data
        },
        Dataset {
            inputs: &[0; 51],
            ..data
        },
        Dataset {
            frequency: Some(0),
            ..data
        },
    ] {
        assert!(linear_as::train(&bad).is_err());
    }
    for value in [
        Value::Real(f64::NAN),
        Value::Real(f64::INFINITY),
        Value::Real(1e200),
        Value::string("bad"),
        Value::Null,
    ] {
        let bad = numeric_table(
            &["x", "y"],
            vec![
                vec![value, Value::Real(1.0)],
                vec![Value::Real(2.0), Value::Real(3.0)],
            ],
        );
        assert!(
            linear_as::train(&Dataset {
                table: &bad,
                ..data
            })
            .is_err()
        );
    }
    let factor = factor_table(false, true);
    let (mut schema, rows) = factor.into_parts();
    let mut fields = schema.fields().to_vec();
    fields[1].measurement = Measurement::Nominal;
    schema = DataModel::new(fields).unwrap();
    let two = Table::new(schema, rows).unwrap();
    assert!(
        matches!(linear_as::train(&Dataset { table:&two,target:2,inputs:&[0,1],frequency:None }),Err(AlgoError::Unsupported(s)) if s.contains("factor-by-factor"))
    );
}
