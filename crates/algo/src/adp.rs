//! Automatic preparation as used by Linear/ALM, IBM SPSS Modeler 19 Algorithms Guide,
//! "Automated Data Preparation Algorithms" pp. 13–28; public oracle cases under `linear/`.
//!
//! Rules observed in `lin_adp_none_drug/goods` and `lin_default_drug/goods`:
//! - Work on the build node's training records (including its partition filter).
//! - Continuous inputs get `<name>_transformed`, with clipping at mean +/- 3 sample SD.
//!   The six cases have no tails requiring robust estimation; those estimates equal the
//!   ordinary sample moments. `team/linear_regression/01_increase.xml` confirms partition moments.
//! - No predictor rescaling, target Box-Cox/rescaling, feature construction or univariate
//!   feature selection, although the saved standalone `adp.*` defaults request them.
//! - Merge the most similar nominal categories while their pooled-variance F-test p > .1.
//!   Drug merges BP HIGH/LOW, and all Cholesterol categories; Sex and goods Class stay intact.
//!   Sex's p=.06859953366 demonstrates ALM overrides saved adp.supervisedMergingPValue=.05.
//!   The .1 threshold is documented in IBM Modeler 18.6 Modeling Nodes, "Basics", p.153:
//!   https://www.ibm.com/docs/en/SS3RA7_18.6.0/pdf/ModelerModelingNodes.pdf
//! - Only changed predictors are recoded; merged groups are numbered by increasing count,
//!   with lexical original category as tie-breaker (drug BP: NORMAL=0, HIGH/LOW=1).
//!   A predictor reduced to one group is excluded (drug Cholesterol).
//! - Transformations use standard PMML NormContinuous and MapValues and score on raw fields.
//!
//! Robust moments follow the interval trimming formulas recovered from the public Guide PDF,
//! pp.18–19: eight raw-SD bins, discard the smaller tail while cumulative mass is <5%, then
//! estimate mean/sample SD on the retained cases. GAP: outlier and sparse-tail oracle coverage;
//! need a one-outlier/sparse-tail probe with PMML robust means/SDs to verify that branch.
//! GAP: missing-value imputation/order, recasting, temporal inputs and zero-count categories
//! are not exercised here; reject those data rather than claiming standalone ADP support.

use crate::{AlgoError, Dataset};
use dms_pmml::{DataType, DerivedField, Expr, OpType, Outliers, Value as PmmlValue};
use dms_types::{DataModel, Measurement, Storage, Table, Value, ValueSpec};

#[derive(Debug, Clone, PartialEq)]
pub struct AdpSettings {
    pub outliers: bool,
    pub cutoff: f64,
    pub merge: bool,
    pub merge_alpha: f64,
    pub suffix: String,
}

impl Default for AdpSettings {
    fn default() -> Self {
        Self {
            outliers: true,
            cutoff: 3.0,
            merge: true,
            merge_alpha: 0.1,
            suffix: "_transformed".into(),
        }
    }
}

pub(crate) struct Prepared {
    pub table: Table,
    pub inputs: Vec<usize>,
    pub transformations: Vec<DerivedField>,
}

pub(crate) fn prepare(dataset: &Dataset<'_>, s: &AdpSettings) -> Result<Prepared, AlgoError> {
    let table = dataset.table;
    if dataset.inputs.len() > 256
        || table.row_count() > 1_000_000
        || table.row_count().saturating_mul(table.model().len()) > 16_000_000
    {
        return Err(AlgoError::Data(
            "Linear ADP limit: 256 inputs / 1000000 rows / 16000000 cells".into(),
        ));
    }
    if !s.cutoff.is_finite()
        || s.cutoff <= 0.0
        || !s.merge_alpha.is_finite()
        || !(0.0..=1.0).contains(&s.merge_alpha)
        || s.suffix.is_empty()
        || s.suffix.len() > 256
    {
        return Err(AlgoError::Settings(
            "invalid Linear ADP cutoff, alpha or suffix".into(),
        ));
    }
    let target = table
        .model()
        .fields()
        .get(dataset.target)
        .ok_or_else(|| AlgoError::Data("Linear ADP target position out of range".into()))?;
    let mut fields = table.model().fields().to_vec();
    let mut rows: Vec<Vec<Value>> = table
        .rows()
        .iter()
        .filter(|row| {
            row.get(dataset.target)
                .is_some_and(|v| !matches!(v, Value::Null) && !target.is_missing(v))
        })
        .cloned()
        .collect();
    if rows.len() < 2 {
        return Err(AlgoError::Data("Linear ADP needs two valid targets".into()));
    }
    let mut inputs = Vec::new();
    let mut transformations = Vec::new();
    for &position in dataset.inputs {
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Data("Linear ADP input position out of range".into()))?
            .clone();
        if matches!(
            field.storage,
            Storage::Date | Storage::Time | Storage::Timestamp | Storage::List | Storage::Unknown
        ) || !matches!(
            field.measurement,
            Measurement::Continuous | Measurement::Nominal | Measurement::Flag
        ) {
            return Err(AlgoError::Unsupported(format!(
                "Linear ADP measurement/storage for {}",
                field.name
            )));
        }
        if rows.iter().any(|r| {
            r.get(position)
                .is_none_or(|v| matches!(v, Value::Null) || field.is_missing(v))
        }) {
            return Err(AlgoError::Unsupported(format!(
                "Linear ADP missing input {} (needs imputation oracle)",
                field.name
            )));
        }
        let name = format!("{}{}", field.name, s.suffix);
        if fields.iter().any(|f| f.name == name) {
            return Err(AlgoError::Settings(format!(
                "Linear ADP derived name collision: {name}"
            )));
        }
        if field.measurement == Measurement::Continuous {
            let values: Vec<f64> = rows
                .iter()
                .map(|r| {
                    r.get(position)
                        .and_then(Value::as_f64)
                        .filter(|v| v.is_finite())
                        .ok_or_else(|| {
                            AlgoError::Data(format!(
                                "Linear ADP non-finite numeric input {}",
                                field.name
                            ))
                        })
                })
                .collect::<Result<_, _>>()?;
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            let sd = (values.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
                / (values.len() - 1) as f64)
                .sqrt();
            if !mean.is_finite() || !sd.is_finite() {
                return Err(AlgoError::Data("Linear ADP moments overflow".into()));
            }
            if sd == 0.0 {
                continue;
            }
            let mut distinct = values.clone();
            distinct.sort_by(f64::total_cmp);
            distinct.dedup();
            if distinct.len() < 5 {
                return Err(AlgoError::Unsupported(format!(
                    "Linear ADP scale-to-ordinal recasting for {}",
                    field.name
                )));
            }
            if !s.outliers {
                inputs.push(position);
                continue;
            }
            let (mean, sd) = robust_moments(&values, mean, sd)?;
            let (lo, hi) = (mean - s.cutoff * sd, mean + s.cutoff * sd);
            if !lo.is_finite() || !hi.is_finite() || lo >= hi {
                return Err(AlgoError::Data("Linear ADP cutoff overflow".into()));
            }
            for (row, value) in rows.iter_mut().zip(values) {
                if let Some(v) = row.get_mut(position) {
                    *v = Value::Real(value.clamp(lo, hi));
                }
            }
            // Values outside the training range are invalid: the transformed field is missing,
            // so the prediction is null (`team/linear_regression` row 161: Meat's model uses
            // Cost_transformed, and Cost 104.68 is above Meat's training maximum 100.15; Modeler
            // saves this range as the field's `Interval closedClosed`).
            let (min, max) = (distinct[0], distinct[distinct.len() - 1]);
            let bound = |function: &str, limit: f64| Expr::Apply {
                function: function.into(),
                args: vec![
                    Expr::FieldRef {
                        field: field.name.clone(),
                        map_missing_to: None,
                    },
                    Expr::Constant(PmmlValue::Number(limit)),
                ],
                map_missing_to: None,
            };
            transformations.push(DerivedField {
                name: name.clone(),
                data_type: DataType::Double,
                optype: OpType::Continuous,
                expr: Expr::Apply {
                    function: "if".into(),
                    args: vec![
                        Expr::Apply {
                            function: "and".into(),
                            args: vec![bound("greaterOrEqual", min), bound("lessOrEqual", max)],
                            map_missing_to: None,
                        },
                        Expr::NormContinuous {
                            field: field.name.clone(),
                            points: vec![(lo, lo), (hi, hi)],
                            outliers: Outliers::AsExtremeValues,
                            map_missing_to: None,
                        },
                    ],
                    map_missing_to: None,
                },
            });
            if let Some(f) = fields.get_mut(position) {
                f.name = name;
                f.storage = Storage::Real;
            }
        } else if s.merge {
            let categories = crate::linear::bounded_categories(table, position, 100)?;
            if categories.len() > 100 {
                continue;
            }
            let mut groups: Vec<Group> = categories
                .iter()
                .map(|c| Group {
                    categories: vec![c.clone()],
                    n: 0,
                    mean: 0.0,
                    ss: 0.0,
                })
                .collect();
            for row in &rows {
                let category = row
                    .get(position)
                    .map(ToString::to_string)
                    .unwrap_or_default();
                let y = row
                    .get(dataset.target)
                    .and_then(Value::as_f64)
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| {
                        AlgoError::Data("Linear ADP target is not finite numeric".into())
                    })?;
                let group = groups
                    .iter_mut()
                    .find(|g| g.categories.contains(&category))
                    .ok_or_else(|| {
                        AlgoError::Data(format!(
                            "Linear ADP undeclared category for {}",
                            field.name
                        ))
                    })?;
                group.n += 1;
                let d = y - group.mean;
                group.mean += d / group.n as f64;
                group.ss += d * (y - group.mean);
            }
            if groups.iter().any(|g| g.n == 0) {
                return Err(AlgoError::Unsupported(format!(
                    "Linear ADP zero-count category for {}",
                    field.name
                )));
            }
            let original = groups.len();
            merge_groups(&mut groups, s.merge_alpha);
            if groups.len() <= 1 {
                continue;
            }
            if groups.len() != original {
                groups.sort_by(|a, b| {
                    a.n.cmp(&b.n)
                        .then_with(|| a.categories.iter().min().cmp(&b.categories.iter().min()))
                });
                let mappings: Vec<(String, String)> = groups
                    .iter()
                    .enumerate()
                    .flat_map(|(i, g)| g.categories.iter().map(move |c| (c.clone(), i.to_string())))
                    .collect();
                for row in &mut rows {
                    let original = row
                        .get(position)
                        .map(ToString::to_string)
                        .unwrap_or_default();
                    let mapped = mappings
                        .iter()
                        .find(|(a, _)| *a == original)
                        .map(|(_, b)| b.clone())
                        .ok_or_else(|| AlgoError::Data("Linear ADP merge lost category".into()))?;
                    if let Some(v) = row.get_mut(position) {
                        *v = Value::string(mapped);
                    }
                }
                transformations.push(DerivedField {
                    name: name.clone(),
                    data_type: DataType::String,
                    optype: OpType::Categorical,
                    expr: Expr::MapValues {
                        pairs: vec![(field.name.clone(), "input".into())],
                        output: "output".into(),
                        rows: mappings
                            .into_iter()
                            .map(|(a, b)| vec![("input".into(), a), ("output".into(), b)])
                            .collect(),
                        default: None,
                        map_missing_to: None,
                    },
                });
                if let Some(f) = fields.get_mut(position) {
                    f.name = name;
                    f.storage = Storage::String;
                    f.measurement = Measurement::Nominal;
                    f.values = ValueSpec::Categories(
                        (0..groups.len())
                            .map(|i| Value::string(i.to_string()))
                            .collect(),
                    );
                }
            }
        }
        inputs.push(position);
    }
    if inputs.is_empty() {
        return Err(AlgoError::Data("Linear ADP excluded every input".into()));
    }
    let model = DataModel::new(fields).map_err(|e| AlgoError::Data(e.to_string()))?;
    Ok(Prepared {
        table: Table::new(model, rows).map_err(|e| AlgoError::Data(e.to_string()))?,
        inputs,
        transformations,
    })
}

struct Group {
    categories: Vec<String>,
    n: usize,
    mean: f64,
    ss: f64,
}

fn robust_moments(values: &[f64], mean: f64, sd: f64) -> Result<(f64, f64), AlgoError> {
    // I_-3=(-inf,mean-3sd], I_i=(mean+(i-1)sd,mean+i*sd], I_4=(mean+3sd,inf).
    let bin = |x: f64| {
        (-3..=3)
            .position(|k| x <= mean + k as f64 * sd)
            .unwrap_or(7)
    };
    let mut counts = [0usize; 8];
    for &x in values {
        if let Some(n) = counts.get_mut(bin(x)) {
            *n += 1;
        }
    }
    let (mut lo, mut hi, mut removed) = (0usize, 7usize, 0usize);
    while lo < hi {
        let (Some(&left), Some(&right)) = (counts.get(lo), counts.get(hi)) else {
            break;
        };
        let next = removed.saturating_add(left.min(right));
        if next as f64 / values.len().max(1) as f64 >= 0.05 {
            break;
        }
        removed = next;
        if left <= right {
            lo += 1;
        } else {
            hi = hi.saturating_sub(1);
        }
    }
    let retained: Vec<f64> = values
        .iter()
        .copied()
        .filter(|x| (lo..=hi).contains(&bin(*x)))
        .collect();
    if retained.len() < 2 {
        return Err(AlgoError::Data(
            "Linear ADP has fewer than two retained values".into(),
        ));
    }
    let mean = retained.iter().sum::<f64>() / retained.len() as f64;
    let sd = (retained.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
        / (retained.len() - 1) as f64)
        .sqrt();
    if !mean.is_finite() || !sd.is_finite() {
        return Err(AlgoError::Data("Linear ADP robust moments overflow".into()));
    }
    Ok((mean, sd))
}

fn merge_groups(groups: &mut Vec<Group>, alpha: f64) {
    while groups.len() > 1 {
        let mut best = None;
        for (i, a) in groups.iter().enumerate() {
            for (j, b) in groups.iter().enumerate().skip(i + 1) {
                let n = a.n.saturating_add(b.n);
                let between = (a.mean - b.mean).powi(2) * a.n as f64 * b.n as f64 / n.max(1) as f64;
                let within = a.ss + b.ss;
                let p = if between == 0.0 {
                    1.0
                } else if within == 0.0 || n <= 2 {
                    0.0
                } else {
                    crate::stats::f_p(between / (within / (n - 2) as f64), 1.0, (n - 2) as f64)
                };
                if best.is_none_or(|(_, _, v)| p > v) {
                    best = Some((i, j, p));
                }
            }
        }
        let Some((i, j, p)) = best else {
            break;
        };
        if p <= alpha {
            break;
        }
        // Indices are from the bounded group enumeration above; j > i.
        let b = groups.remove(j);
        if let Some(a) = groups.get_mut(i) {
            let n = a.n.saturating_add(b.n);
            a.ss += b.ss + (a.mean - b.mean).powi(2) * a.n as f64 * b.n as f64 / n.max(1) as f64;
            a.mean += (b.mean - a.mean) * b.n as f64 / n.max(1) as f64;
            a.n = n;
            a.categories.extend(b.categories);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robust_moments_trim_less_than_five_percent_using_raw_sd_bins() {
        let mut values: Vec<f64> = (0..100).map(|i| i as f64 - 49.5).collect();
        values.push(1000.0);
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let sd = (values.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
            / (values.len() - 1) as f64)
            .sqrt();
        let (center, spread) = robust_moments(&values, mean, sd).unwrap();
        assert!(center.abs() < 1e-12);
        assert!((spread - (84166.66666666667_f64 / 100.0).sqrt()).abs() < 1e-12);
    }
}
