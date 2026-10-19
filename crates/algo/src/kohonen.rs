//! Kohonen (Algorithms Guide: "Kohonen Algorithms"): **statistical parity** with Modeler 19
//! (`docs/statistical-parity.md`, rule 2). Modeler starts the network from random weights and
//! presents the training records in a random order drawn from an undisclosed generator, so its
//! weights and map cannot be reproduced bit for bit. We draw both from our own SplitMix64 stream
//! seeded with the node's `randomSeed`; the oracle tests require the map size exactly and hold
//! Modeler's mean quantization error and non-empty unit count to the band of 100 of our seeds
//! widened by 10 % of its width, on 17 oracles (`models_next/kohonen_*`, `kohonen_size/*`).
//! One is a known outlier: on `kohonen_size_n20_in2` Modeler's map has 2 empty units and a mean
//! quantization error of 0.1779, just above our 100 runs' widened band (0.1556..0.1756 + 10 %).
//! Across the other 16, Modeler's error ranks near the middle of ours (mean rank 41 of 100).
//!
//! Encoding matches the scorer and the oracle PMML. A numeric continuous field is scaled to
//! `[0, 1]` by its declared `ValueSpec::Range` (the Type node's bounds, which Modeler saves as
//! `MiningField@lowValue`/`@highValue`); values outside the range are coerced to it, and a missing
//! value becomes 0.5. Without a declared range the observed minimum and maximum are used. A flag
//! is 1 for its true value, 0 for the false one, 0.5 when missing. Any other categorical field
//! uses one indicator per category, 1.0 for the matching category and 0.0 for the rest, all 0.0
//! when missing (the guide's indicator coding; unlike K-Means there is no `sqrt(1/2)` weight).
//!
//! Training follows the guide: the winning unit is the nearest center in Euclidean distance, the
//! winner and every unit within the Chebyshev neighbourhood are moved by `eta * (x - w)`, and
//! records are reshuffled at the start of every cycle. The two phases run neighbourhood / eta /
//! cycles 2 / 0.3 / 20 and 1 / 0.1 / 150. Linear decay reaches the lesser of the current and
//! following phase's eta at the end of the phase; the last phase has no following phase and keeps
//! its eta. Streams also save a third phase (1 / 0.001 / 1) that the node's dialog does not show;
//! Modeler does not use it: decaying phase 2 towards it puts Modeler's error above the median of
//! our runs on 14 of 17 oracles (mean rank 19.6 of 30), keeping phase 2 at 0.1 centres it.
//! Presenting the records in one fixed order, or drawing them with replacement, fits worse.
//! Exponential decay (geometric decrease, floor 0.0001) is implemented from the guide's
//! description but has no oracle, so the builder rejects it.
//!
//! The model has the shape Modeler saves: `LocalTransformations` with the encoded fields
//! `cluster0..`, the raw and derived `ClusteringField`s, and one `Cluster` per map unit named
//! `X=<x>, Y=<y>` with its `KohonenMap` position and center array. It scores through
//! `dms-pmml` and `nodes::apply`, which writes `$KX-`, `$KY-` and `$KXY-`.
//!
//! In simple mode Modeler ignores the saved output layer dimensions (10 x 7) and trains a 4 x 3
//! map whatever the data: `kohonen_size/*` (5, 10, 20, 36, 50, 75, 100, 150, 200, 300, 500 and
//! 1000 records; 1, 2 and 5 inputs) all save 12 units. Expert mode uses the saved dimensions
//! exactly (3 x 5 in `kohonen_map`).

use crate::AlgoError;
use crate::common::{data_field, field_categories, mining_field};
use dms_pmml::{
    Cluster, ClusteringField, ClusteringModel, CompareFunction, ComparisonMeasure, DataType,
    DerivedField, Expr, MeasureKind, Metric, MiningFunction, Model, ModelClass, ModelHeader,
    OpType, Outliers, Pmml, Usage,
};
use dms_types::{Measurement, Storage, Table, Value, ValueSpec};

/// Eta decay between cycles (`learningRateDecayMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decay {
    Linear,
    Exponential,
}

/// One training phase: the Chebyshev neighbourhood radius, its starting eta and its cycles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Phase {
    pub neighbourhood: usize,
    pub eta: f64,
    pub cycles: usize,
}

/// Settings of a Kohonen build node (Modeler's defaults in [`Default`]).
#[derive(Debug, Clone, PartialEq)]
pub struct KohonenSettings {
    /// `expertMode`: use the saved map dimensions instead of the auto-size rule.
    pub expert: bool,
    /// `outputLayerWidth` (map units along X) in expert mode; ignored in simple mode.
    pub width: usize,
    /// `outputLayerLength` (map units along Y) in expert mode; ignored in simple mode.
    pub length: usize,
    /// `learningPhase1..2` neighbourhood, eta and cycles.
    pub phases: [Phase; 2],
    /// `learningRateDecayMode`.
    pub decay: Decay,
    /// `randomSeed`: seeds our generator for the initial weights and every cycle's order.
    pub seed: u64,
    /// `modelName`.
    pub name: Option<String>,
}

impl Default for KohonenSettings {
    fn default() -> Self {
        KohonenSettings {
            expert: false,
            width: 10,
            length: 7,
            phases: [
                Phase {
                    neighbourhood: 2,
                    eta: 0.3,
                    cycles: 20,
                },
                Phase {
                    neighbourhood: 1,
                    eta: 0.1,
                    cycles: 150,
                },
            ],
            decay: Decay::Linear,
            seed: 123,
            name: None,
        }
    }
}

/// How one input field becomes encoded columns.
enum Encoding {
    Range { low: f64, high: f64 },
    Flag { true_value: String },
    Set { categories: Vec<String> },
}

/// Trains a Kohonen map on the `inputs` of `table`.
pub fn train(
    table: &Table,
    inputs: &[usize],
    settings: &KohonenSettings,
) -> Result<Pmml, AlgoError> {
    validate(settings)?;
    let fields = table.model().fields();
    let rows = table.rows();
    if rows.is_empty() {
        return Err(AlgoError::Data("no records to map".into()));
    }
    if inputs.is_empty() {
        return Err(AlgoError::Data("no input fields to map".into()));
    }

    let mut encodings = Vec::with_capacity(inputs.len());
    for &position in inputs {
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Data(format!("no field at position {position}")))?;
        let numeric = matches!(field.storage, Storage::Integer | Storage::Real);
        let encoding = match field.measurement {
            Measurement::Continuous if numeric => {
                let declared = match &field.values {
                    ValueSpec::Range { low, high } => match (low.as_f64(), high.as_f64()) {
                        (Some(low), Some(high))
                            if low.is_finite() && high.is_finite() && high > low =>
                        {
                            Some((low, high))
                        }
                        _ => None,
                    },
                    _ => None,
                };
                let (low, high) = match declared {
                    Some(range) => range,
                    None => observed_range(rows, position, field).ok_or_else(|| {
                        AlgoError::Data(format!("{} has no numeric values", field.name))
                    })?,
                };
                Encoding::Range { low, high }
            }
            Measurement::Continuous => {
                return Err(AlgoError::Unsupported(format!(
                    "Kohonen input {} with non-numeric storage",
                    field.name
                )));
            }
            Measurement::Flag => match &field.values {
                ValueSpec::Flag { true_value, .. } => Encoding::Flag {
                    true_value: true_value.to_string(),
                },
                _ => Encoding::Set {
                    categories: field_categories(table, position),
                },
            },
            _ => Encoding::Set {
                categories: field_categories(table, position),
            },
        };
        encodings.push(encoding);
    }

    // Encoded records. A missing value is the field type's neutral value; an invalid continuous
    // cell that is not a declared blank is treated like a blank (0.5).
    let encoded: Vec<Vec<f64>> = rows
        .iter()
        .map(|row| {
            let mut point = Vec::new();
            for (encoding, &position) in encodings.iter().zip(inputs) {
                let field = fields.get(position);
                let value = row
                    .get(position)
                    .filter(|value| field.is_some_and(|f| !f.is_missing(value)));
                match encoding {
                    Encoding::Range { low, high } => {
                        point.push(
                            value
                                .and_then(Value::as_f64)
                                .map_or(0.5, |x| scale(x.clamp(*low, *high), *low, *high)),
                        );
                    }
                    Encoding::Flag { true_value } => point.push(value.map_or(0.5, |value| {
                        if value.to_string() == *true_value {
                            1.0
                        } else {
                            0.0
                        }
                    })),
                    Encoding::Set { categories } => {
                        let text = value.map(Value::to_string);
                        point.extend(categories.iter().map(|category| {
                            if text.as_deref() == Some(category.as_str()) {
                                1.0
                            } else {
                                0.0
                            }
                        }));
                    }
                }
            }
            point
        })
        .collect();
    let dimensions = encoded.first().map_or(0, Vec::len);
    if dimensions == 0 {
        return Err(AlgoError::Data(
            "the inputs encode to no numeric columns".into(),
        ));
    }

    let (width, length) = map_size(settings);
    let units = width
        .checked_mul(length)
        .ok_or_else(|| AlgoError::Data("the map is too large".into()))?;
    if units
        .checked_mul(dimensions)
        .is_none_or(|cells| cells > 1_000_000)
    {
        return Err(AlgoError::Data(
            "the map and its inputs are too large to train".into(),
        ));
    }

    let mut rng = SplitMix64::new(settings.seed);
    // "Small random weights": the guide does not give a distribution. Uniform [0, 1) is the
    // usual choice for inputs already scaled to that interval; the seed band accepts the result.
    let mut weights: Vec<Vec<f64>> = (0..units)
        .map(|_| (0..dimensions).map(|_| rng.next_f64()).collect())
        .collect();
    let mut order: Vec<usize> = (0..rows.len()).collect();

    let last_phase = settings.phases.len().saturating_sub(1);
    for (phase_index, phase) in settings.phases.iter().enumerate() {
        let cycles = phase.cycles.max(1);
        let low_eta = settings
            .phases
            .get(phase_index + 1)
            .map_or(phase.eta, |next| phase.eta.min(next.eta));
        let mut eta = phase.eta;
        for cycle in 0..cycles {
            // "Input records are presented to the network in random order": a fresh shuffle
            // every cycle, from the same generator that drew the initial weights.
            shuffle(&mut order, &mut rng);
            for &record in &order {
                let Some(point) = encoded.get(record) else {
                    continue;
                };
                let winner = nearest(point, &weights);
                let wx = winner / length;
                let wy = winner % length;
                for ux in 0..width {
                    if ux.abs_diff(wx) > phase.neighbourhood {
                        continue;
                    }
                    for uy in 0..length {
                        if uy.abs_diff(wy) > phase.neighbourhood {
                            continue;
                        }
                        let Some(unit) = weights.get_mut(ux * length + uy) else {
                            continue;
                        };
                        for (weight, x) in unit.iter_mut().zip(point) {
                            *weight += eta * (x - *weight);
                        }
                    }
                }
            }
            eta = match settings.decay {
                Decay::Linear if phase_index < last_phase => {
                    phase.eta - (phase.eta - low_eta) * (cycle + 1) as f64 / cycles as f64
                }
                Decay::Exponential if phase_index < last_phase => {
                    exponential_eta(phase.eta, low_eta, cycle + 1, cycles)
                }
                _ => phase.eta,
            };
        }
    }

    // Cluster sizes: the training records' winning units after the final weights.
    let mut sizes = vec![0usize; units];
    for point in &encoded {
        if let Some(size) = sizes.get_mut(nearest(point, &weights)) {
            *size += 1;
        }
    }

    // PMML: derived (encoded) fields, then the raw fields and the derived ones as clustering
    // fields, then one cluster per map position in X-major, Y-minor order (Modeler's order).
    let mut derived = Vec::new();
    for (encoding, &position) in encodings.iter().zip(inputs) {
        let name = fields[position].name.clone();
        let mut push = |expr: Expr| {
            derived.push(DerivedField {
                name: format!("cluster{}", derived.len()),
                data_type: DataType::Double,
                optype: OpType::Continuous,
                expr,
            });
        };
        match encoding {
            Encoding::Range { low, high } => push(Expr::NormContinuous {
                field: name,
                points: if *high > *low {
                    vec![(*low, 0.0), (*high, 1.0)]
                } else {
                    vec![(*low, 0.0), (*low + 1.0, 0.0)]
                },
                // The guide coerces values outside the type-defined range; at scoring time the
                // encoded value must clamp to 0/1 the same way, not extrapolate.
                outliers: Outliers::AsExtremeValues,
                map_missing_to: Some(0.5),
            }),
            Encoding::Flag { true_value } => push(Expr::NormDiscrete {
                field: name,
                value: true_value.clone(),
                map_missing_to: Some(0.5),
            }),
            Encoding::Set { categories } => {
                for category in categories {
                    push(Expr::NormDiscrete {
                        field: name.clone(),
                        value: category.clone(),
                        // All indicators are 0 for a missing set value.
                        map_missing_to: Some(0.0),
                    });
                }
            }
        }
    }
    let mut clustering_fields: Vec<ClusteringField> = inputs
        .iter()
        .filter_map(|&position| fields.get(position))
        .map(|field| ClusteringField {
            field: field.name.clone(),
            weight: 1.0,
            compare: Some(CompareFunction::AbsDiff),
            is_center_field: false,
        })
        .collect();
    clustering_fields.extend(derived.iter().map(|field| ClusteringField {
        field: field.name.clone(),
        weight: 1.0,
        compare: Some(CompareFunction::AbsDiff),
        is_center_field: true,
    }));
    let clusters = weights
        .iter()
        .zip(sizes)
        .enumerate()
        .map(|(index, (center, size))| {
            let x = index / length;
            let y = index % length;
            Cluster {
                id: None,
                name: Some(format!("X={x}, Y={y}")),
                size: Some(size as f64),
                center: center.clone(),
                variances: Vec::new(),
                category_counts: Vec::new(),
                mean_distance: None,
                map_position: Some((x as f64, y as f64)),
            }
        })
        .collect();
    let header = ModelHeader {
        name: settings.name.clone(),
        function: MiningFunction::Clustering,
        algorithm: Some("Kohonen".into()),
        mining_schema: inputs
            .iter()
            .filter_map(|&position| mining_field(table, position, Usage::Active))
            .collect(),
        output: Vec::new(),
        local_transformations: derived,
    };
    Ok(Pmml {
        version: "4.2".into(),
        data_dictionary: inputs
            .iter()
            .filter_map(|&position| data_field(table, position))
            .collect(),
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Clustering(ClusteringModel {
            header,
            model_class: ModelClass::CenterBased,
            measure: ComparisonMeasure {
                kind: MeasureKind::Distance,
                metric: Metric::SquaredEuclidean,
                compare: CompareFunction::AbsDiff,
                log_likelihood: false,
            },
            fields: clustering_fields,
            clusters,
            // Modeler writes no MissingValueWeights; the PMML default (1 per center field) is
            // what its nugget uses.
            missing_weights: Vec::new(),
            anomaly_cutoff: None,
        }),
    })
}

fn validate(settings: &KohonenSettings) -> Result<(), AlgoError> {
    if settings.width == 0 || settings.length == 0 {
        // Simple mode ignores the saved dimensions, but a zero is still a bad default.
        return Err(AlgoError::Settings(
            "Kohonen map dimensions must be positive".into(),
        ));
    }
    for (index, phase) in settings.phases.iter().enumerate() {
        if phase.neighbourhood == 0 {
            return Err(AlgoError::Settings(format!(
                "Kohonen phase {} needs a positive neighbourhood",
                index + 1
            )));
        }
        if !phase.eta.is_finite() || !(0.0..=1.0).contains(&phase.eta) {
            return Err(AlgoError::Settings(format!(
                "Kohonen phase {} eta must be from 0 to 1, found {}",
                index + 1,
                phase.eta
            )));
        }
        if phase.cycles == 0 {
            return Err(AlgoError::Settings(format!(
                "Kohonen phase {} needs at least one cycle",
                index + 1
            )));
        }
    }
    Ok(())
}

/// The map dimensions: expert mode uses the saved ones; simple mode is always 4 x 3.
fn map_size(settings: &KohonenSettings) -> (usize, usize) {
    if settings.expert {
        (settings.width, settings.length)
    } else {
        SIMPLE_MAP
    }
}

/// Simple mode's map (`kohonen_size/*`: 5 to 1000 records, 1 to 5 inputs, all 4 x 3).
const SIMPLE_MAP: (usize, usize) = (4, 3);

fn observed_range(
    rows: &[Vec<Value>],
    position: usize,
    field: &dms_types::Field,
) -> Option<(f64, f64)> {
    let (low, high) = rows
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), row| {
            let Some(value) = row
                .get(position)
                .filter(|value| !field.is_missing(value))
                .and_then(Value::as_f64)
            else {
                return (low, high);
            };
            (low.min(value), high.max(value))
        });
    // A constant field is a valid range of width zero (every value then encodes to 0).
    (low.is_finite() && high.is_finite()).then_some((low, high))
}

fn scale(x: f64, low: f64, high: f64) -> f64 {
    if high > low {
        (x - low) / (high - low)
    } else {
        0.0
    }
}

/// The geometric eta after `cycle` of `cycles`, reaching `low` at the end and never below the
/// guide's 0.0001 floor. The guide gives the formula as an image; this is its stated analogue:
/// "decreasing by a fixed proportion each cycle".
fn exponential_eta(start: f64, low: f64, cycle: usize, cycles: usize) -> f64 {
    if start <= 0.0 {
        return start;
    }
    let ratio = (low / start).max(0.0);
    let fraction = cycle as f64 / cycles.max(1) as f64;
    (start * ratio.powf(fraction)).max(0.0001)
}

fn nearest(point: &[f64], weights: &[Vec<f64>]) -> usize {
    let mut best = 0;
    let mut best_distance = f64::INFINITY;
    for (index, weight) in weights.iter().enumerate() {
        let distance: f64 = point
            .iter()
            .zip(weight)
            .map(|(x, y)| {
                let difference = x - y;
                difference * difference
            })
            .sum();
        if distance < best_distance {
            best = index;
            best_distance = distance;
        }
    }
    best
}

fn shuffle(order: &mut [usize], rng: &mut SplitMix64) {
    for index in (1..order.len()).rev() {
        let other = rng.below(index + 1);
        order.swap(index, other);
    }
}

/// SplitMix64, a small seedable generator (Steele, Lea and Flood 2014). Modeler's generator is
/// undisclosed; this one gives reproducible maps from `randomSeed` and is only used for training.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    fn next_f64(&mut self) -> f64 {
        // 53 random bits, the mantissa width, so every result is exactly representable.
        (self.next_u64() >> 11) as f64 * (1.0 / (1_u64 << 53) as f64)
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        (self.next_u64() % bound as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dms_types::{DataModel, Field, Role};

    fn field(name: &str, low: f64, high: f64) -> Field {
        let mut field = Field::new(name, Storage::Real);
        field.measurement = Measurement::Continuous;
        field.role = Role::Both;
        field.values = ValueSpec::Range {
            low: Value::Real(low),
            high: Value::Real(high),
        };
        field
    }

    fn table() -> Table {
        let fields = vec![field("x", 0.0, 10.0), field("y", 0.0, 20.0)];
        let rows = (0..24)
            .map(|index| {
                let x = f64::from(index % 12);
                vec![Value::Real(x), Value::Real(2.0 * x)]
            })
            .collect();
        Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
    }

    fn settings(seed: u64) -> KohonenSettings {
        KohonenSettings {
            expert: true,
            width: 3,
            length: 2,
            seed,
            ..KohonenSettings::default()
        }
    }

    fn clusters(pmml: &Pmml) -> &[Cluster] {
        match &pmml.model {
            Model::Clustering(model) => &model.clusters,
            _ => panic!("expected a clustering model"),
        }
    }

    #[test]
    fn the_map_has_one_cluster_per_position_in_x_major_order() {
        let pmml = train(&table(), &[0, 1], &settings(7)).unwrap();
        let clusters = clusters(&pmml);
        assert_eq!(clusters.len(), 6);
        let positions: Vec<_> = clusters
            .iter()
            .map(|cluster| cluster.map_position)
            .collect();
        assert_eq!(
            positions,
            vec![
                Some((0.0, 0.0)),
                Some((0.0, 1.0)),
                Some((1.0, 0.0)),
                Some((1.0, 1.0)),
                Some((2.0, 0.0)),
                Some((2.0, 1.0)),
            ]
        );
        assert_eq!(clusters[4].name.as_deref(), Some("X=2, Y=0"));
        assert_eq!(
            clusters.iter().map(|c| c.size.unwrap_or(0.0)).sum::<f64>(),
            24.0
        );
    }

    #[test]
    fn training_is_reproducible_from_the_seed() {
        let table = table();
        let first = train(&table, &[0, 1], &settings(42)).unwrap();
        let second = train(&table, &[0, 1], &settings(42)).unwrap();
        let other = train(&table, &[0, 1], &settings(43)).unwrap();
        assert_eq!(first, second);
        assert_ne!(first, other);
        for cluster in clusters(&first) {
            assert_eq!(cluster.center.len(), 2);
            assert!(cluster.center.iter().all(|w| w.is_finite()));
            assert!(cluster.center.iter().all(|w| (0.0..=1.0).contains(w)));
        }
    }

    #[test]
    fn encoding_matches_the_scorer() {
        use dms_pmml::Value as PmmlValue;

        let pmml = train(&table(), &[0, 1], &settings(1)).unwrap();
        let names: Vec<&str> = pmml
            .data_dictionary
            .iter()
            .map(|field| field.name.as_str())
            .collect();
        let scorer = pmml.bind(&names).unwrap();
        let score = |x: f64, y: f64| {
            scorer
                .score(&[PmmlValue::Number(x), PmmlValue::Number(y)])
                .unwrap()
        };
        // Declared range 0..10 / 0..20: out-of-range values are coerced to the limits.
        assert_eq!(score(20.0, 20.0), score(10.0, 20.0));
        assert_eq!(score(-5.0, 0.0), score(0.0, 0.0));
        // A missing value is 0.5, the range midpoint here.
        assert_eq!(
            scorer
                .score(&[PmmlValue::Missing, PmmlValue::Missing])
                .unwrap(),
            score(5.0, 10.0)
        );
    }

    #[test]
    fn invalid_settings_are_rejected() {
        let table = table();
        let mut bad = settings(1);
        bad.phases[0].eta = 1.5;
        assert!(matches!(
            train(&table, &[0], &bad),
            Err(AlgoError::Settings(_))
        ));
        let mut bad = settings(1);
        bad.phases[1].cycles = 0;
        assert!(matches!(
            train(&table, &[0], &bad),
            Err(AlgoError::Settings(_))
        ));
        let mut bad = settings(1);
        bad.length = 0;
        assert!(matches!(
            train(&table, &[0], &bad),
            Err(AlgoError::Settings(_))
        ));
    }

    #[test]
    fn no_records_are_a_data_error() {
        let model = DataModel::new(vec![field("x", 0.0, 1.0)]).unwrap();
        let empty = Table::new(model, Vec::new()).unwrap();
        assert!(matches!(
            train(&empty, &[0], &settings(1)),
            Err(AlgoError::Data(_))
        ));
    }
}
