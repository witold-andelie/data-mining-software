//! C&R Tree rules the oracle cases do not exercise (their data has no missing values).

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_algo::Dataset;
use dms_algo::cart::{self, CartSettings, MinSize};
use dms_pmml::{Model, Node, Value as PmmlValue};
use dms_types::{DataModel, Field, Measurement, Storage, Table, Value, ValueSpec};

/// `x` separates the classes; `copy` follows `x` and stands in where `x` is missing.
fn table() -> Table {
    let mut x = Field::new("x", Storage::Real);
    x.measurement = Measurement::Continuous;
    let mut copy = Field::new("copy", Storage::Real);
    copy.measurement = Measurement::Continuous;
    let mut y = Field::new("y", Storage::String);
    y.measurement = Measurement::Nominal;
    y.values = ValueSpec::Categories(vec![Value::string("a"), Value::string("b")]);
    let rows = (0..20)
        .map(|i| {
            let value = f64::from(i);
            vec![
                // Record 3 (class a) has no x.
                if i == 3 {
                    Value::Null
                } else {
                    Value::Real(value)
                },
                Value::Real(value * 10.0),
                Value::string(if i < 10 { "a" } else { "b" }),
            ]
        })
        .collect();
    Table::new(DataModel::new(vec![x, copy, y]).unwrap(), rows).unwrap()
}

fn root(pmml: &dms_pmml::Pmml) -> &Node {
    match &pmml.model {
        Model::Tree(tree) => &tree.root,
        other => panic!("not a tree: {other:?}"),
    }
}

#[test]
fn a_missing_split_value_follows_the_surrogate() {
    let table = table();
    let dataset = Dataset {
        table: &table,
        target: 2,
        inputs: &[0, 1],
        frequency: None,
    };
    let settings = CartSettings {
        min_parent: MinSize::Records(2.0),
        min_child: MinSize::Records(1.0),
        ..CartSettings::default()
    };
    let pmml = cart::train(&dataset, &settings).unwrap();
    let root = root(&pmml);
    let counts: Vec<f64> = root
        .children
        .iter()
        .map(|c| c.record_count.unwrap())
        .collect();
    // The record without x goes left with the other class-a records through `copy`.
    assert_eq!(counts, [10.0, 10.0]);

    let scorer = pmml.bind(&["x", "copy"]).unwrap();
    let missing_x = scorer
        .score(&[PmmlValue::Missing, PmmlValue::Number(30.0)])
        .unwrap();
    assert_eq!(missing_x.value, PmmlValue::Text("a".into()));
    // With neither field the record takes the default child.
    let neither = scorer
        .score(&[PmmlValue::Missing, PmmlValue::Missing])
        .unwrap();
    assert_ne!(neither.value, PmmlValue::Missing);
}

#[test]
fn unsupported_settings_are_errors() {
    let table = table();
    let dataset = Dataset {
        table: &table,
        target: 2,
        inputs: &[0, 1],
        frequency: Some(1),
    };
    assert!(cart::train(&dataset, &CartSettings::default()).is_err());
}
