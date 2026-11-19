//! The Bayes Net build node's model: a [`Dataset`] in, Modeler's Bayes Net nugget out.
//!
//! The learner ([`crate::bayes_net`]) works on categorical variables. This module turns the
//! training table into them and assembles the nugget's PMML around the learned network:
//!
//! - A target must be categorical. A predictor with a continuous measurement is discretised into
//!   five equal-width bins between its smallest and largest value (the range the Type node
//!   states, else the records' own); a bin without records is
//!   merged into its neighbours (Algorithms Guide, "Handling of Continuous Predictors"). An empty
//!   bin between two occupied ones is shared at its midpoint, which reproduces the cut points of
//!   the saved `bayes_bankloan` model; a run of empty bins at the end joins the last occupied
//!   bin. The bins are `Discretize` derived fields named `<field>Bins`, labelled `0`, `1`, ...
//! - Every other predictor keeps its categories. A value outside the category list counts as
//!   missing.
//! - Records with a missing value in any used field are left out (the default of the node; the
//!   learner returns a `GAP:` for pairwise deletion when a value is missing).
//!
//! GAP: frequency fields, date and time predictors, and the variable importance the nugget
//! reports.

use crate::bayes_net::{self, CategoricalData, Network, Settings, Variable};
use crate::common::{self, data_field, field_categories, mining_field};
use crate::{AlgoError, Dataset};
use dms_pmml::{
    DataType, DerivedField, DiscretizeBin, Expr, InvalidTreatment, MiningFunction, Model,
    ModelHeader, OpType, Pmml, Usage, Value as PmmlValue,
};
use dms_types::{Measurement, Storage};

/// Number of equal-width bins of a continuous predictor.
const BIN_COUNT: usize = 5;

fn gap(what: &str) -> AlgoError {
    AlgoError::Unsupported(format!("GAP: Bayes Net {what}"))
}

/// How one predictor becomes a categorical variable.
enum Source {
    /// Categories of the field as they are.
    Categories(Vec<String>),
    /// Cut points (ascending); category `k` is `(cut[k-1], cut[k]]`, the ends unbounded.
    Bins(Vec<f64>),
}

struct Predictor {
    /// Position in the training table.
    position: usize,
    /// The variable's name in the network.
    name: String,
    source: Source,
}

impl Predictor {
    fn labels(&self) -> Vec<String> {
        match &self.source {
            Source::Categories(labels) => labels.clone(),
            Source::Bins(cuts) => (0..=cuts.len()).map(|k| k.to_string()).collect(),
        }
    }

    /// The category index of a record's value; `None` when it is missing or not a category.
    fn code(&self, value: &dms_types::Value, field: &dms_types::Field) -> Option<u32> {
        if field.is_missing(value) {
            return None;
        }
        match &self.source {
            Source::Categories(labels) => {
                let text = value.to_string();
                labels.iter().position(|l| *l == text).map(|k| k as u32)
            }
            Source::Bins(cuts) => {
                let x = value.as_f64()?;
                Some(cuts.iter().take_while(|&&c| x > c).count() as u32)
            }
        }
    }
}

/// The cut points of the five equal-width bins of `range` after the bins `values` leave empty are
/// merged.
pub fn bin_cuts(range: (f64, f64), values: &[f64]) -> Vec<f64> {
    let (low, high) = range;
    if !(low.is_finite() && high.is_finite()) || high <= low {
        return Vec::new();
    }
    let width = (high - low) / BIN_COUNT as f64;
    let edges: Vec<f64> = (1..BIN_COUNT).map(|k| low + width * k as f64).collect();
    let mut counts = [0usize; BIN_COUNT];
    for &x in values {
        let k = edges.iter().take_while(|&&c| x > c).count();
        if let Some(slot) = counts.get_mut(k) {
            *slot += 1;
        }
    }
    // `edges[k]` separates bin `k` from bin `k + 1`. A run of empty bins `i..=j` removes the
    // edges `i-1..=j`; between two occupied bins one edge at the middle of the run's span stays.
    let mut keep: Vec<f64> = Vec::new();
    let mut k = 0;
    while k < BIN_COUNT {
        if counts.get(k).copied().unwrap_or(0) > 0 {
            if let Some(&edge) = edges.get(k) {
                // the edge after an occupied bin: kept unless an empty run follows
                if counts.get(k + 1).copied().unwrap_or(1) > 0 {
                    keep.push(edge);
                }
            }
            k += 1;
            continue;
        }
        let start = k;
        while k < BIN_COUNT && counts.get(k).copied().unwrap_or(0) == 0 {
            k += 1;
        }
        // the run is `start..k`; an interior run keeps one edge in the middle of its span
        if start > 0
            && k < BIN_COUNT
            && let (Some(&first), Some(&last)) = (edges.get(start - 1), edges.get(k - 1))
        {
            keep.push((first + last) / 2.0);
        }
    }
    keep
}

/// Trains a Bayes Net on `dataset` and returns the nugget's PMML.
pub fn train(dataset: &Dataset<'_>, settings: &Settings) -> Result<Pmml, AlgoError> {
    crate::check_cancelled()?;
    if dataset.frequency.is_some() {
        return Err(gap("with a frequency field"));
    }
    let table = dataset.table;
    let fields = table.model().fields();
    let target_labels = field_categories(table, dataset.target);
    if target_labels.is_empty() {
        return Err(AlgoError::Data(
            "Bayes Net needs a categorical target with values".into(),
        ));
    }

    let mut predictors: Vec<Predictor> = Vec::new();
    for &position in dataset.inputs {
        crate::check_cancelled()?;
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Data("an input field is not in the table".into()))?;
        if position == dataset.target || predictors.iter().any(|p| p.position == position) {
            continue;
        }
        let source = if field.measurement == Measurement::Continuous {
            if !matches!(field.storage, Storage::Integer | Storage::Real) {
                return Err(gap(&format!(
                    "with the date, time or text predictor {:?}",
                    field.name
                )));
            }
            let values: Vec<f64> = table
                .rows()
                .iter()
                .filter_map(|row| row.get(position))
                .filter(|v| !field.is_missing(v))
                .filter_map(dms_types::Value::as_f64)
                .collect();
            if values.is_empty() {
                continue;
            }
            // The range the Type node states (the whole data when it instantiated), else the
            // records' own: the saved `bayes_bankloan` bins span the Type range of 850 records
            // although 700 reach the node.
            let range = match &field.values {
                dms_types::ValueSpec::Range { low, high } => low.as_f64().zip(high.as_f64()),
                _ => None,
            }
            .unwrap_or_else(|| common::value_range(values.iter().copied()));
            Source::Bins(bin_cuts(range, &values))
        } else {
            let labels = field_categories(table, position);
            if labels.is_empty() {
                continue;
            }
            Source::Categories(labels)
        };
        let name = match source {
            Source::Bins(_) => format!("{}Bins", field.name),
            Source::Categories(_) => field.name.clone(),
        };
        if fields
            .iter()
            .any(|f| f.name == name && f.name != field.name)
        {
            return Err(AlgoError::Settings(format!(
                "the discretised name {name:?} is already a field"
            )));
        }
        predictors.push(Predictor {
            position,
            name,
            source,
        });
    }
    if predictors.is_empty() {
        return Err(AlgoError::Data("Bayes Net has no usable predictor".into()));
    }

    let target_field = fields
        .get(dataset.target)
        .ok_or_else(|| AlgoError::Data("the target is not in the table".into()))?;
    let mut variables: Vec<Variable> = predictors
        .iter()
        .map(|p| Variable {
            name: p.name.clone(),
            categories: p.labels(),
        })
        .collect();
    variables.push(Variable {
        name: target_field.name.clone(),
        categories: target_labels.clone(),
    });
    let target_index = variables.len() - 1;
    let mut rows = Vec::with_capacity(table.rows().len());
    for row in table.rows() {
        let mut codes: Vec<Option<u32>> = Vec::with_capacity(variables.len());
        for p in &predictors {
            let field = fields
                .get(p.position)
                .ok_or_else(|| AlgoError::Data("an input field is not in the table".into()))?;
            codes.push(row.get(p.position).and_then(|v| p.code(v, field)));
        }
        let target_value = row.get(dataset.target);
        codes.push(
            target_value
                .filter(|v| !target_field.is_missing(v))
                .and_then(|v| {
                    let text = v.to_string();
                    target_labels
                        .iter()
                        .position(|l| *l == text)
                        .map(|k| k as u32)
                }),
        );
        rows.push(codes);
    }
    let data = CategoricalData {
        variables,
        rows,
        target: target_index,
    };
    let network = bayes_net::train(&data, settings)?;
    assemble(dataset, &predictors, &target_labels, &network)
}

fn assemble(
    dataset: &Dataset<'_>,
    predictors: &[Predictor],
    target_labels: &[String],
    network: &Network,
) -> Result<Pmml, AlgoError> {
    let table = dataset.table;
    let mut dictionary = Vec::new();
    for position in std::iter::once(dataset.target).chain(predictors.iter().map(|p| p.position)) {
        if let Some(mut entry) = data_field(table, position) {
            if position == dataset.target {
                entry.values = target_labels.to_vec();
            }
            dictionary.push(entry);
        }
    }
    let mut transformations = Vec::new();
    for p in predictors {
        let Source::Bins(cuts) = &p.source else {
            continue;
        };
        let field = table
            .model()
            .fields()
            .get(p.position)
            .ok_or_else(|| AlgoError::Data("an input field is not in the table".into()))?;
        let bins = (0..=cuts.len())
            .map(|k| DiscretizeBin {
                value: PmmlValue::Number(k as f64),
                left: k.checked_sub(1).and_then(|i| cuts.get(i)).copied(),
                right: cuts.get(k).copied(),
                left_closed: false,
                right_closed: true,
            })
            .collect();
        transformations.push(DerivedField {
            name: p.name.clone(),
            data_type: DataType::Integer,
            optype: OpType::Categorical,
            expr: Expr::Discretize {
                field: field.name.clone(),
                bins,
                default: None,
                map_missing_to: None,
            },
        });
    }
    let mut schema: Vec<_> = predictors
        .iter()
        .filter_map(|p| mining_field(table, p.position, Usage::Active))
        .collect();
    for field in &mut schema {
        // a category the training data never had gives no evidence
        field.invalid_treatment = InvalidTreatment::AsMissing;
    }
    schema.extend(mining_field(table, dataset.target, Usage::Target));
    let header = ModelHeader {
        name: None,
        function: MiningFunction::Classification,
        algorithm: None,
        mining_schema: schema,
        output: Vec::new(),
        local_transformations: Vec::new(),
    };
    Ok(Pmml {
        version: "4.1".into(),
        data_dictionary: dictionary,
        transformations,
        functions: Vec::new(),
        model: Model::BayesNet(network.to_model(header)?),
    })
}

#[cfg(test)]
mod tests {
    use super::bin_cuts;

    fn close(cuts: &[f64], expected: &[f64]) {
        assert_eq!(cuts.len(), expected.len(), "{cuts:?} vs {expected:?}");
        for (c, e) in cuts.iter().zip(expected) {
            assert!((c - e).abs() < 1e-9, "{cuts:?} vs {expected:?}");
        }
    }

    #[test]
    fn bayes_net_bins_are_equal_width_when_every_bin_is_occupied() {
        let values: Vec<f64> = (0..=10).map(f64::from).collect();
        close(&bin_cuts((0.0, 10.0), &values), &[2.0, 4.0, 6.0, 8.0]);
    }

    #[test]
    fn bayes_net_bins_share_an_empty_bin_at_its_midpoint() {
        // The saved `incomeBins` of `bayes_bankloan`: range 13 to 446, the fourth of five bins
        // (272.8, 359.4] empty; Modeler's cuts are 99.6, 186.2 and 316.1.
        let values = [13.0, 50.0, 120.0, 150.0, 200.0, 250.0, 446.0];
        close(&bin_cuts((13.0, 446.0), &values), &[99.6, 186.2, 316.1]);
    }

    #[test]
    fn bayes_net_bins_merge_a_run_of_empty_bins_into_one_cut() {
        // Only the first and the last bin hold records: one cut at the middle of the range.
        close(&bin_cuts((0.0, 10.0), &[0.0, 0.5, 10.0]), &[5.0]);
    }

    #[test]
    fn bayes_net_bins_of_a_constant_or_empty_field_have_no_cut() {
        assert!(bin_cuts((3.0, 3.0), &[3.0, 3.0, 3.0]).is_empty());
        assert!(bin_cuts((f64::INFINITY, f64::NEG_INFINITY), &[]).is_empty());
        assert!(bin_cuts((f64::NAN, 1.0), &[1.0]).is_empty());
    }
}
