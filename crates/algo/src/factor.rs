//! PCA/Factor (`BuildPCANode`): principal components of the correlation matrix, exported as
//! Modeler does: one PMML `GeneralRegressionModel` per factor whose coefficients turn raw input
//! values into factor scores (`oracle/models_next/factor_*`).
//!
//! Evidence (`models_next`, 36 records of five numeric inputs, x2 = 2 x1 + 1 so the
//! correlation matrix is singular):
//! - Records with a missing input are left out (`includeCompleteRecs`); correlations use
//!   standard deviations with n - 1.
//! - Components are the eigenvectors of the correlation matrix by decreasing eigenvalue; a
//!   component's sign makes its loadings sum positive (`factor_nfactors`: factor 1 has all
//!   positive coefficients).
//! - `limitFactors` `By Eigenvalue` keeps eigenvalues above `eigenvalueLimit` (`factor_varimax`,
//!   2 of 5); `By Maximum` keeps `maxFactorLimit` components (`factor_nfactors`, 2). GAP: in
//!   that case the eigenvalue rule also gives 2, so whether `By Maximum` is a cap on the
//!   eigenvalue rule or an exact count is not settled; we keep exactly that many.
//! - Simple mode (`expertMode` false) keeps every component (`models_next2/factor_default`:
//!   5 of 5).
//! - `rotationMethod` `Varimax` (`factor_varimax`): Kaiser-normalized varimax by pairwise
//!   rotations, at most `maxIterations` sweeps, until no angle exceeds 1e-5.
//! - Regression factor scores without inverting the (possibly singular) correlation matrix:
//!   B = V Λ^(-1/2) T (T the rotation, identity without one); a factor's coefficient on input j
//!   is B_jk / sd_j and its constant -Σ_j B_jk mean_j / sd_j (both cases to about 1e-14).
//!
//! Everything else (other extraction methods, covariance matrices, other rotations or score
//! methods, pairwise deletion) is rejected.

use crate::AlgoError;
use dms_pmml::{
    GeneralModelType, GeneralRegressionModel, MiningFunction, MiningModel, MiningSegment, Model,
    ModelHeader, PCell, PPCell, Pmml, Predicate,
};
use dms_types::{Table, Value};

/// Upper bounds against degenerate inputs.
const MAX_INPUTS: usize = 256;
const MAX_ROWS: usize = 1_000_000;

#[derive(Debug, Clone, PartialEq)]
pub enum FactorCount {
    /// Keep components whose eigenvalue exceeds the limit.
    Eigenvalue(f64),
    /// Keep this many components.
    Maximum(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FactorSettings {
    pub count: FactorCount,
    /// Varimax rotation with this many sweeps at most; `None` for no rotation.
    pub varimax: Option<usize>,
    pub name: Option<String>,
}

pub fn train(
    table: &Table,
    inputs: &[usize],
    settings: &FactorSettings,
) -> Result<Pmml, AlgoError> {
    if inputs.is_empty() || inputs.len() > MAX_INPUTS {
        return Err(AlgoError::Data(
            "PCA/Factor needs between 1 and 256 numeric inputs".into(),
        ));
    }
    let fields = table.model().fields();
    let mut data: Vec<Vec<f64>> = Vec::new();
    'rows: for row in table.rows().iter().take(MAX_ROWS) {
        let mut values = Vec::with_capacity(inputs.len());
        for &p in inputs {
            let field = fields
                .get(p)
                .ok_or_else(|| AlgoError::Data("PCA/Factor input out of range".into()))?;
            let value = row.get(p).unwrap_or(&Value::Null);
            if field.is_missing(value) {
                continue 'rows;
            }
            match value {
                Value::Integer(i) => values.push(*i as f64),
                Value::Real(r) if r.is_finite() => values.push(*r),
                _ => {
                    return Err(AlgoError::Data(format!(
                        "PCA/Factor input {} is not numeric",
                        field.name
                    )));
                }
            }
        }
        data.push(values);
    }
    let n = data.len();
    let p = inputs.len();
    if n < 2 {
        return Err(AlgoError::Data(
            "PCA/Factor needs at least two complete records".into(),
        ));
    }
    let nf = n as f64;
    let mean: Vec<f64> = (0..p)
        .map(|j| data.iter().map(|r| r[j]).sum::<f64>() / nf)
        .collect();
    let sd: Vec<f64> = (0..p)
        .map(|j| (data.iter().map(|r| (r[j] - mean[j]).powi(2)).sum::<f64>() / (nf - 1.0)).sqrt())
        .collect();
    if sd.iter().any(|s| !s.is_finite() || *s <= 0.0) {
        return Err(AlgoError::Data("PCA/Factor input has no variance".into()));
    }
    let mut corr = vec![vec![0.0; p]; p];
    for a in 0..p {
        for b in a..p {
            let c = data
                .iter()
                .map(|r| (r[a] - mean[a]) * (r[b] - mean[b]))
                .sum::<f64>()
                / ((nf - 1.0) * sd[a] * sd[b]);
            corr[a][b] = c;
            corr[b][a] = c;
        }
    }
    let (values, vectors) = jacobi_eigen(corr)?;
    let k = match settings.count {
        FactorCount::Eigenvalue(limit) => values.iter().filter(|v| **v > limit).count(),
        FactorCount::Maximum(max) => max.min(p),
    };
    if k == 0 {
        return Err(AlgoError::Data("PCA/Factor extracted no factors".into()));
    }
    // Columns of V for the kept components, signed so each column sums positive.
    let mut v: Vec<Vec<f64>> = (0..k)
        .map(|c| (0..p).map(|j| vectors[j][c]).collect())
        .collect();
    for column in &mut v {
        if column.iter().sum::<f64>() < 0.0 {
            for x in column.iter_mut() {
                *x = -*x;
            }
        }
    }
    let rotation = match settings.varimax {
        Some(sweeps) if k > 1 => {
            let loadings: Vec<Vec<f64>> = (0..p)
                .map(|j| (0..k).map(|c| v[c][j] * values[c].sqrt()).collect())
                .collect();
            varimax(&loadings, sweeps)
        }
        _ => identity(k),
    };
    // B = V Λ^(-1/2) T.
    let b: Vec<Vec<f64>> = (0..p)
        .map(|j| {
            (0..k)
                .map(|f| {
                    (0..k)
                        .map(|c| v[c][j] / values[c].sqrt() * rotation[c][f])
                        .sum()
                })
                .collect()
        })
        .collect();
    let names: Vec<String> = inputs
        .iter()
        .filter_map(|&i| fields.get(i).map(|f| f.name.clone()))
        .collect();
    let mining_schema: Vec<_> = inputs
        .iter()
        .filter_map(|&i| crate::common::mining_field(table, i, dms_pmml::Usage::Active))
        .collect();
    let segments = (0..k)
        .map(|f| {
            let constant = -(0..p).map(|j| b[j][f] * mean[j] / sd[j]).sum::<f64>();
            let mut param_matrix = vec![PCell {
                target_category: None,
                parameter: "(Constant)".into(),
                beta: constant,
            }];
            let mut pp_matrix = Vec::new();
            for (j, name) in names.iter().enumerate() {
                param_matrix.push(PCell {
                    target_category: None,
                    parameter: name.clone(),
                    beta: b[j][f] / sd[j],
                });
                pp_matrix.push(PPCell {
                    value: "1".into(),
                    predictor: name.clone(),
                    parameter: name.clone(),
                });
            }
            MiningSegment {
                id: None,
                predicate: Predicate::True,
                model: Box::new(Model::GeneralRegression(GeneralRegressionModel {
                    header: ModelHeader {
                        name: settings.name.clone(),
                        function: MiningFunction::Regression,
                        algorithm: None,
                        mining_schema: mining_schema.clone(),
                        output: Vec::new(),
                        local_transformations: Vec::new(),
                    },
                    model_type: GeneralModelType::Regression,
                    target_reference_category: None,
                    link_function: None,
                    link_parameter: None,
                    parameters: std::iter::once("(Constant)".to_string())
                        .chain(names.iter().cloned())
                        .collect(),
                    factors: Vec::new(),
                    covariates: names.clone(),
                    pp_matrix,
                    param_matrix,
                })),
            }
        })
        .collect();
    Ok(Pmml {
        version: "4.2".into(),
        data_dictionary: inputs
            .iter()
            .filter_map(|&i| crate::common::data_field(table, i))
            .collect(),
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Mining(MiningModel {
            header: ModelHeader {
                name: settings.name.clone(),
                function: MiningFunction::Regression,
                algorithm: Some("PCA".into()),
                mining_schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            method: dms_pmml::FACTOR_SCORES.into(),
            segments,
        }),
    })
}

fn identity(k: usize) -> Vec<Vec<f64>> {
    (0..k)
        .map(|i| (0..k).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
        .collect()
}

/// Eigenvalues (decreasing) and eigenvectors (columns) of a symmetric matrix, cyclic Jacobi.
fn jacobi_eigen(mut a: Vec<Vec<f64>>) -> Result<(Vec<f64>, Vec<Vec<f64>>), AlgoError> {
    let n = a.len();
    let mut v = identity(n);
    for _ in 0..100 {
        let off: f64 = (0..n)
            .flat_map(|i| (0..n).filter(move |&j| j != i).map(move |j| (i, j)))
            .map(|(i, j)| a[i][j] * a[i][j])
            .sum();
        if off < 1e-30 {
            break;
        }
        for i in 0..n {
            for j in i + 1..n {
                if a[i][j].abs() < 1e-300 {
                    continue;
                }
                let theta = (a[j][j] - a[i][i]) / (2.0 * a[i][j]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for row in a.iter_mut() {
                    let (ari, arj) = (row[i], row[j]);
                    row[i] = c * ari - s * arj;
                    row[j] = s * ari + c * arj;
                }
                let (low, high) = a.split_at_mut(j);
                if let (Some(row_i), Some(row_j)) = (low.get_mut(i), high.first_mut()) {
                    for (x, y) in row_i.iter_mut().zip(row_j.iter_mut()) {
                        let (air, ajr) = (*x, *y);
                        *x = c * air - s * ajr;
                        *y = s * air + c * ajr;
                    }
                }
                for row in v.iter_mut() {
                    let (vri, vrj) = (row[i], row[j]);
                    row[i] = c * vri - s * vrj;
                    row[j] = s * vri + c * vrj;
                }
            }
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&x, &y| a[y][y].total_cmp(&a[x][x]));
    let values: Vec<f64> = order.iter().map(|&i| a[i][i]).collect();
    if values.iter().any(|v| !v.is_finite()) {
        return Err(AlgoError::Data(
            "PCA/Factor eigen decomposition failed".into(),
        ));
    }
    let vectors = (0..n)
        .map(|r| order.iter().map(|&c| v[r][c]).collect())
        .collect();
    Ok((values, vectors))
}

/// Kaiser-normalized varimax by pairwise rotations; returns the k x k rotation matrix.
fn varimax(loadings: &[Vec<f64>], sweeps: usize) -> Vec<Vec<f64>> {
    let p = loadings.len();
    let k = loadings.first().map_or(0, Vec::len);
    let h: Vec<f64> = loadings
        .iter()
        .map(|row| row.iter().map(|x| x * x).sum::<f64>().sqrt())
        .collect();
    let mut a: Vec<Vec<f64>> = loadings
        .iter()
        .zip(&h)
        .map(|(row, h)| {
            row.iter()
                .map(|x| if *h > 0.0 { x / h } else { 0.0 })
                .collect()
        })
        .collect();
    let mut t = identity(k);
    let pf = p as f64;
    for _ in 0..sweeps {
        let mut changed = false;
        for i in 0..k {
            for j in i + 1..k {
                let (mut sa, mut sb, mut sc, mut sd) = (0.0, 0.0, 0.0, 0.0);
                for row in &a {
                    let (x, y) = (row[i], row[j]);
                    let u = x * x - y * y;
                    let w = 2.0 * x * y;
                    sa += u;
                    sb += w;
                    sc += u * u - w * w;
                    sd += 2.0 * u * w;
                }
                let num = sd - 2.0 * sa * sb / pf;
                let den = sc - (sa * sa - sb * sb) / pf;
                let phi = num.atan2(den) / 4.0;
                if phi.abs() > 1e-5 {
                    changed = true;
                    let (cs, sn) = (phi.cos(), phi.sin());
                    for row in &mut a {
                        let (x, y) = (row[i], row[j]);
                        row[i] = cs * x + sn * y;
                        row[j] = -sn * x + cs * y;
                    }
                    for row in &mut t {
                        let (x, y) = (row[i], row[j]);
                        row[i] = cs * x + sn * y;
                        row[j] = -sn * x + cs * y;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    t
}
