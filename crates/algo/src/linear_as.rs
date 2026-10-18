//! Linear-AS (`spss.linearEngine!linearEngine`), Modeler 19 Algorithms Guide,
//! sections 4, 5.2/5.3, 6.2 and Appendices A.2/B. Single mapper, unit weights.
//!
//! - Listwise deletion, continuous target; factors precede covariates, then squares,
//!   then significant covariate-by-factor pairs (Type category order).
//! - Within-level centered sums and the F test screen pairs at p < .05;
//!   `linearas/lin_factor` includes grp*x1 and excludes grp*x2.
//! - Provisional means in record order produce C, S=C/(N-1) and correlation R.
//!   Forward adjusted-R² selection sweeps grouped columns with tolerance 1e-4,
//!   rejects Condition b, tries removals, and stops on repeats or default limits.
//!   `linearas/lin_default` selects x1, x4, x3*x3; `lin_factor` selects grp,
//!   x2, grp*x1, with redundant [grp=c] beta/df zero.
//! - Unstandardize swept coefficients and recover the intercept from means.
//!   `models_as/linearas_default` verifies five main effects and scoring; its
//!   extra x4*x4 beta -1.329e-16 is floating-point noise, not a selection target.
//! - PMML 4.3 generalLinear/LE uses selected-effect order, complete factor levels,
//!   square exponents and interaction PPCells. Importance and ModelStats are not
//!   reproduced. [`TrainedModel`] carries the parameter labels and df, which the
//!   shared PMML type does not store.
//!
//! GAPs: nondefault settings, weights/frequency, factor-by-factor detection,
//! more than 50 main effects, and parameter counts reaching 5000 (pair search).

use crate::common::{data_field, mining_field};
use crate::linear::bounded_categories;
use crate::{AlgoError, Dataset};
use dms_pmml::{
    GeneralModelType, GeneralRegressionModel, MiningFunction, Model, ModelHeader, PCell, PPCell,
    Pmml, Usage,
};
use dms_types::{Measurement, Value};
use std::collections::HashSet;

const TOLERANCE: f64 = 1e-4;
const MAX_PARAMETERS: usize = 5000;
const MAX_CELLS: usize = 16_000_000;

/// Metadata omitted by the shared PMML scoring types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterInfo {
    pub label: String,
    pub df: u8,
}

#[derive(Debug, Clone)]
pub struct TrainedModel {
    pub pmml: Pmml,
    pub parameters: Vec<ParameterInfo>,
}

#[derive(Clone)]
struct Column {
    factor: Option<(usize, String)>,
    covariate: Option<usize>,
    square: bool,
}

impl Column {
    fn value(&self, row: &[Value]) -> Result<f64, AlgoError> {
        let indicator = self
            .factor
            .as_ref()
            .is_none_or(|(p, level)| row.get(*p).is_some_and(|v| v.to_string() == *level));
        if !indicator {
            return Ok(0.0);
        }
        let x = match self.covariate {
            Some(p) => row
                .get(p)
                .and_then(Value::as_f64)
                .ok_or_else(|| AlgoError::Data("Linear-AS covariate must be numeric".into()))?,
            None => 1.0,
        };
        Ok(if self.square { x * x } else { x })
    }
}

struct Effect {
    columns: std::ops::Range<usize>,
}

fn gap(what: &str) -> AlgoError {
    AlgoError::Unsupported(format!("GAP: Linear-AS {what}"))
}

/// Train only the verified forward-stepwise, adjusted-R² defaults.
pub fn train(data: &Dataset<'_>) -> Result<TrainedModel, AlgoError> {
    if data.frequency.is_some() {
        return Err(gap("frequency weights"));
    }
    if data.inputs.len() > 50 {
        return Err(gap("more than 50 main effects: pair search is unverified"));
    }
    let table = data.table;
    if table.row_count() > 1_000_000 {
        return Err(AlgoError::Data("Linear-AS limit: 1000000 records".into()));
    }
    let fields = table.model().fields();
    // The AS dictionary includes even unused fields; bound them before the common
    // dictionary helper discovers unknown category lists.
    if fields.len() > MAX_PARAMETERS {
        return Err(AlgoError::Data(
            "Linear-AS limit: 5000 dictionary fields".into(),
        ));
    }
    let mut dictionary_categories = 0usize;
    for (p, field) in fields.iter().enumerate() {
        if field.measurement != Measurement::Continuous {
            let count = bounded_categories(table, p, MAX_PARAMETERS)?.len();
            dictionary_categories = dictionary_categories.saturating_add(count);
            if count > MAX_PARAMETERS || dictionary_categories > 100_000 {
                return Err(AlgoError::Data(
                    "Linear-AS dictionary category limit".into(),
                ));
            }
        }
    }
    let target = fields
        .get(data.target)
        .ok_or_else(|| AlgoError::Settings("Linear-AS target position out of range".into()))?;
    if target.measurement != Measurement::Continuous {
        return Err(gap("target must be continuous"));
    }
    let mut unique = HashSet::new();
    let mut factors = Vec::new();
    let mut covariates = Vec::new();
    for &p in data.inputs {
        let field = fields
            .get(p)
            .ok_or_else(|| AlgoError::Settings("Linear-AS input position out of range".into()))?;
        if p == data.target || !unique.insert(p) {
            return Err(AlgoError::Settings(
                "Linear-AS inputs must be distinct and exclude target".into(),
            ));
        }
        if field.measurement == Measurement::Continuous {
            covariates.push(p);
        } else {
            factors.push(p);
        }
    }
    if factors.len() > 1 {
        return Err(gap("factor-by-factor interaction test is unverified"));
    }
    let rows: Vec<_> = table
        .rows()
        .iter()
        .filter(|row| {
            std::iter::once(data.target)
                .chain(data.inputs.iter().copied())
                .all(|p| {
                    row.get(p)
                        .is_some_and(|v| fields.get(p).is_some_and(|f| !f.is_missing(v)))
                })
        })
        .collect();
    if rows.len() < 2 {
        return Err(AlgoError::Data(
            "Linear-AS needs two complete records".into(),
        ));
    }
    for row in &rows {
        for p in std::iter::once(data.target).chain(covariates.iter().copied()) {
            if row
                .get(p)
                .and_then(Value::as_f64)
                .is_none_or(|x| !x.is_finite())
            {
                return Err(AlgoError::Data(
                    "Linear-AS requires finite numeric values".into(),
                ));
            }
        }
    }
    let mut columns = Vec::new();
    let mut effects = Vec::new();
    let mut levels = Vec::new();
    for &factor in &factors {
        levels = bounded_categories(table, factor, MAX_PARAMETERS)?;
        if levels
            .len()
            .saturating_add(covariates.len().saturating_mul(2))
            >= MAX_PARAMETERS
        {
            return Err(gap("parameter count reaches 5000"));
        }
        let start = columns.len();
        for level in &levels {
            columns.push(Column {
                factor: Some((factor, level.clone())),
                covariate: None,
                square: false,
            });
        }
        effects.push(Effect {
            columns: start..columns.len(),
        });
    }
    for square in [false, true] {
        for &p in &covariates {
            let start = columns.len();
            columns.push(Column {
                factor: None,
                covariate: Some(p),
                square,
            });
            effects.push(Effect {
                columns: start..columns.len(),
            });
        }
    }
    for &factor in &factors {
        for &covariate in &covariates {
            if interaction_p(&rows, data.target, factor, &levels, covariate)? < 0.05 {
                if columns.len().saturating_add(levels.len()) >= MAX_PARAMETERS {
                    return Err(gap("parameter count reaches 5000"));
                }
                let start = columns.len();
                for level in &levels {
                    columns.push(Column {
                        factor: Some((factor, level.clone())),
                        covariate: Some(covariate),
                        square: false,
                    });
                }
                effects.push(Effect {
                    columns: start..columns.len(),
                });
            }
        }
    }
    let width = columns.len().saturating_add(1);
    if width.saturating_mul(width) > MAX_CELLS {
        return Err(AlgoError::Data(
            "Linear-AS correlation matrix exceeds 16000000 cells".into(),
        ));
    }
    let mut moments = Moments::new(width);
    for row in &rows {
        let mut values = columns
            .iter()
            .map(|c| c.value(row))
            .collect::<Result<Vec<_>, _>>()?;
        values.push(
            row.get(data.target)
                .and_then(Value::as_f64)
                .ok_or_else(|| AlgoError::Data("Linear-AS target is not numeric".into()))?,
        );
        moments.add(&values)?;
    }
    let yi = columns.len();
    if moments.cross[yi][yi] <= 0.0 {
        return Err(AlgoError::Data(
            "Linear-AS target must be nonconstant".into(),
        ));
    }
    let correlation = moments.correlation();
    let selected = select(correlation, &effects, rows.len());
    make_model(data, &columns, &effects, &moments, &selected)
}

/// Appendix A.2: C += (1 - 1/N) (x-old_mean)(y-old_mean), then update means.
struct Moments {
    n: usize,
    mean: Vec<f64>,
    cross: Vec<Vec<f64>>,
}

impl Moments {
    fn new(width: usize) -> Self {
        Self {
            n: 0,
            mean: vec![0.0; width],
            cross: vec![vec![0.0; width]; width],
        }
    }

    fn add(&mut self, values: &[f64]) -> Result<(), AlgoError> {
        self.n = self.n.saturating_add(1);
        let n = self.n as f64;
        let delta: Vec<_> = values.iter().zip(&self.mean).map(|(x, m)| x - m).collect();
        for (i, row) in self.cross.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell += (1.0 - 1.0 / n) * delta[i] * delta[j];
                if !cell.is_finite() {
                    return Err(AlgoError::Data("Linear-AS centered sums overflow".into()));
                }
            }
        }
        for (mean, delta) in self.mean.iter_mut().zip(delta) {
            *mean += delta / n;
        }
        Ok(())
    }

    fn correlation(&self) -> Vec<Vec<f64>> {
        self.cross
            .iter()
            .enumerate()
            .map(|(i, row)| {
                row.iter()
                    .enumerate()
                    .map(|(j, c)| {
                        let denominator = self.cross[i][i].sqrt() * self.cross[j][j].sqrt();
                        if denominator > 0.0 {
                            c / denominator
                        } else {
                            0.0
                        }
                    })
                    .collect()
            })
            .collect()
    }
}

fn interaction_p(
    rows: &[&Vec<Value>],
    target: usize,
    factor: usize,
    levels: &[String],
    covariate: usize,
) -> Result<f64, AlgoError> {
    let mut groups: Vec<_> = levels.iter().map(|_| Moments::new(2)).collect();
    let mut total = Moments::new(1);
    for row in rows {
        let y = row
            .get(target)
            .and_then(Value::as_f64)
            .ok_or_else(|| AlgoError::Data("Linear-AS missing target".into()))?;
        let x = row
            .get(covariate)
            .and_then(Value::as_f64)
            .ok_or_else(|| AlgoError::Data("Linear-AS missing covariate".into()))?;
        let level = row
            .get(factor)
            .map(ToString::to_string)
            .ok_or_else(|| AlgoError::Data("Linear-AS missing factor".into()))?;
        let j = levels
            .iter()
            .position(|v| *v == level)
            .ok_or_else(|| AlgoError::Data("Linear-AS undeclared factor level".into()))?;
        if let Some(group) = groups.get_mut(j) {
            group.add(&[x, y])?;
        }
        total.add(&[y])?;
    }
    let (mut xx, mut xy, mut yy, mut full, mut s, mut rank) = (0.0, 0.0, 0.0, 0.0, 0usize, 0usize);
    for g in groups.iter().filter(|g| g.n > 0) {
        s += 1;
        rank += 1 + usize::from(g.cross[0][0] > 0.0);
        xx += g.cross[0][0];
        xy += g.cross[0][1];
        yy += g.cross[1][1];
        full += g.cross[1][1]
            - if g.cross[0][0] > 0.0 {
                g.cross[0][1] * g.cross[0][1] / g.cross[0][0]
            } else {
                0.0
            };
    }
    if s < 2 || rows.len() <= s.saturating_mul(2) || xx == 0.0 {
        return Ok(1.0);
    }
    let mut difference = yy - xy * xy / xx - full;
    let zero =
        |x: f64| x.abs() <= f64::EPSILON * rank as f64 * total.cross[0][0] && x.abs() <= 1e-8;
    if zero(difference) {
        difference = 0.0;
    }
    if zero(full) {
        full = 0.0;
    }
    if full == 0.0 {
        return Ok(if difference != 0.0 { 0.0 } else { 1.0 });
    }
    if full < 0.0 {
        return Err(AlgoError::Data(
            "Linear-AS negative interaction residual sum".into(),
        ));
    }
    let f = (difference / (s - 1) as f64) / (full / (rows.len() - 2 * s) as f64);
    Ok(crate::stats::f_p(
        f,
        (s - 1) as f64,
        (rows.len() - 2 * s) as f64,
    ))
}

#[derive(Clone)]
struct SelectionState {
    matrix: Vec<Vec<f64>>,
    effects: Vec<bool>,
    swept: Vec<bool>,
    score: f64,
}

/// Appendix B (asymmetric sweep); applying it twice unsweeps a column.
fn sweep(matrix: &mut [Vec<f64>], k: usize) {
    let row = matrix[k].clone();
    let column: Vec<_> = matrix.iter().map(|r| r[k]).collect();
    let d = row[k];
    for (i, r) in matrix.iter_mut().enumerate() {
        for (j, cell) in r.iter_mut().enumerate() {
            *cell -= column[i] * row[j] / d;
        }
        r[k] = -column[i] / d;
    }
    for (cell, value) in matrix[k].iter_mut().zip(row) {
        *cell = value / d;
    }
    matrix[k][k] = 1.0 / d;
}

fn adjusted(state: &SelectionState, n: usize) -> f64 {
    let p = 1 + state.swept.iter().filter(|v| **v).count();
    let yi = state.swept.len();
    if p >= n {
        f64::NEG_INFINITY
    } else {
        1.0 - state.matrix[yi][yi] * (n - 1) as f64 / (n - p) as f64
    }
}

fn candidate(
    current: &SelectionState,
    effect: &Effect,
    index: usize,
    n: usize,
    entering: bool,
) -> Option<SelectionState> {
    let mut next = current.clone();
    let mut count = 0;
    for k in effect.columns.clone() {
        if (entering && next.matrix[k][k] >= TOLERANCE) || (!entering && next.swept[k]) {
            sweep(&mut next.matrix, k);
            next.swept[k] = entering;
            count += 1;
        }
    }
    if count == 0
        || next.matrix.iter().flatten().any(|v| !v.is_finite())
        || (entering
            && current
                .swept
                .iter()
                .enumerate()
                .any(|(k, &s)| s && next.matrix[k][k] > 1.0 / TOLERANCE))
    {
        return None;
    }
    next.effects[index] = entering;
    next.score = adjusted(&next, n);
    Some(next)
}

fn select(matrix: Vec<Vec<f64>>, effects: &[Effect], n: usize) -> SelectionState {
    let width = matrix.len().saturating_sub(1);
    let mut current = SelectionState {
        matrix,
        effects: vec![false; effects.len()],
        swept: vec![false; width],
        score: 0.0,
    };
    current.score = adjusted(&current, n);
    let mut seen = HashSet::from([current.effects.clone()]);
    let mut steps = 0usize;
    let limit = effects.len().saturating_mul(3);
    while steps < limit && current.effects.iter().any(|v| !v) {
        let mut best: Option<SelectionState> = None;
        for (i, effect) in effects
            .iter()
            .enumerate()
            .filter(|(i, _)| !current.effects[*i])
        {
            if let Some(next) = candidate(&current, effect, i, n, true)
                && best.as_ref().is_none_or(|b| next.score > b.score)
            {
                best = Some(next);
            }
        }
        let Some(next) = best.filter(|b| b.score > current.score) else {
            break;
        };
        if !seen.insert(next.effects.clone()) {
            break;
        }
        current = next;
        steps += 1;
        while steps < limit {
            let mut best: Option<SelectionState> = None;
            for (i, effect) in effects
                .iter()
                .enumerate()
                .filter(|(i, _)| current.effects[*i])
            {
                if let Some(next) = candidate(&current, effect, i, n, false)
                    && best.as_ref().is_none_or(|b| next.score > b.score)
                {
                    best = Some(next);
                }
            }
            let Some(next) = best.filter(|b| b.score > current.score) else {
                break;
            };
            if !seen.insert(next.effects.clone()) {
                return current;
            }
            current = next;
            steps += 1;
        }
    }
    current
}

fn make_model(
    data: &Dataset<'_>,
    columns: &[Column],
    effects: &[Effect],
    moments: &Moments,
    state: &SelectionState,
) -> Result<TrainedModel, AlgoError> {
    let fields = data.table.model().fields();
    let name = |p: usize| {
        fields
            .get(p)
            .map(|f| f.name.clone())
            .ok_or_else(|| AlgoError::Data("Linear-AS field out of range".into()))
    };
    let yi = columns.len();
    let mut factors = Vec::new();
    let mut covariates = Vec::new();
    let mut parameters = vec![ParameterInfo {
        label: "Intercept".into(),
        df: 1,
    }];
    let mut names = vec!["P0000001".into()];
    let mut cells = Vec::new();
    let mut pp = Vec::new();
    let mut intercept = moments.mean[yi];
    for (_, effect) in effects
        .iter()
        .enumerate()
        .filter(|(i, _)| state.effects[*i])
    {
        for k in effect.columns.clone() {
            let c = &columns[k];
            let parameter = format!("P{:07}", names.len() + 1);
            let mut label = String::new();
            if let Some((p, level)) = &c.factor {
                let f = name(*p)?;
                label = format!("[{f}={level}]");
                pp.push(PPCell {
                    value: level.clone(),
                    predictor: f.clone(),
                    parameter: parameter.clone(),
                });
                if !factors.contains(p) {
                    factors.push(*p);
                }
            }
            if let Some(p) = c.covariate {
                let f = name(p)?;
                label = if c.square {
                    format!("{f} * {f}")
                } else if label.is_empty() {
                    f.clone()
                } else {
                    format!("{label} * {f}")
                };
                pp.push(PPCell {
                    value: if c.square { "2" } else { "1" }.into(),
                    predictor: f,
                    parameter: parameter.clone(),
                });
                if !covariates.contains(&p) {
                    covariates.push(p);
                }
            }
            let beta = if state.swept[k] {
                state.matrix[k][yi] * (moments.cross[yi][yi] / moments.cross[k][k]).sqrt()
            } else {
                0.0
            };
            intercept -= beta * moments.mean[k];
            names.push(parameter.clone());
            parameters.push(ParameterInfo {
                label,
                df: u8::from(state.swept[k]),
            });
            cells.push(PCell {
                target_category: None,
                parameter,
                beta,
            });
        }
    }
    cells.insert(
        0,
        PCell {
            target_category: None,
            parameter: "P0000001".into(),
            beta: intercept,
        },
    );
    if cells.iter().any(|c| !c.beta.is_finite()) {
        return Err(AlgoError::Data("Linear-AS coefficient overflow".into()));
    }
    let schema = factors
        .iter()
        .chain(&covariates)
        .filter_map(|&p| mining_field(data.table, p, Usage::Active))
        .chain(mining_field(data.table, data.target, Usage::Target))
        .map(|mut f| {
            f.invalid_treatment = dms_pmml::InvalidTreatment::ReturnInvalid;
            f
        })
        .collect();
    Ok(TrainedModel {
        parameters,
        pmml: Pmml {
            version: "4.3".into(),
            data_dictionary: (0..fields.len())
                .filter_map(|p| data_field(data.table, p))
                .collect(),
            transformations: Vec::new(),
            functions: Vec::new(),
            model: Model::GeneralRegression(GeneralRegressionModel {
                header: ModelHeader {
                    name: None,
                    function: MiningFunction::Regression,
                    algorithm: Some("LE".into()),
                    mining_schema: schema,
                    output: Vec::new(),
                    local_transformations: Vec::new(),
                },
                model_type: GeneralModelType::GeneralLinear,
                target_reference_category: None,
                link_function: None,
                link_parameter: None,
                parameters: names,
                factors: factors.into_iter().map(name).collect::<Result<_, _>>()?,
                covariates: covariates.into_iter().map(name).collect::<Result<_, _>>()?,
                pp_matrix: pp,
                param_matrix: cells,
            }),
        },
    })
}
