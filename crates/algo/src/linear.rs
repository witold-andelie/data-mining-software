//! Linear regression for the Linear node (`LinearRegression!spss.alm-extension`; Algorithms
//! Guide: "Linear modeling algorithms", pp.247–253): least squares, forward stepwise,
//! and Linear's automatic data preparation ([`crate::adp`]).
//!
//! The model is a PMML `GeneralRegressionModel` `generalLinear` shaped like Modeler's
//! (`oracle/models/linear_enter`, `oracle/team/linear_regression` `02_increase.xml`):
//! - Data dictionary: the target, then the inputs; mining schema: the inputs, then the target as
//!   predicted.
//! - Parameters `P0000001`, `P0000002`, ...: `Intercept` (with `linreg.Intercept`), then each
//!   factor in input order, then each covariate in input order: one parameter per covariate,
//!   a categorical
//!   input as one indicator `[<field>=<category>]` per category in the Type's order, the last one
//!   the reference with beta 0 (`team/linear_regression` `01_increase.xml`: `[Class=Meat]`
//!   beta 0, df 0).
//! - Coefficients minimise the residual sum of squares, solved by Householder QR of the design
//!   matrix (records with a missing target or input are left out, `linreg.ListwiseDeletion`);
//!   an aliased column gets beta 0.
//!
//! The classic Regression node (`BuildRegressionNode`, method `enter`) fits the same least
//! squares and writes `modelType` `regression` (`oracle/models/regression_enter`): parameters
//! named `(Constant)` and the covariate names, PMML 4.2. It takes continuous inputs only.
//!
//! Forward selection rules:
//! - Start with the intercept alone (or zero predictions without Intercept); add/remove whole
//!   input effects, not individual indicators. `lin_fwd_drug` selects Sex, BP, Age;
//!   `lin_fwd_goods` selects Class, Promotion. Parameters retain source factor/covariate order.
//! - Minimize AICc = n*ln(RSS/n)+2*n*r/(n-r-1), r the nonredundant parameter count including
//!   intercept. The oracle's reported AICc includes an extra n (constant across candidates).
//! - Public Guide alternatives: AdjustedRSquare maximizes adjusted R² (minimizes RSS/(n-r));
//!   Fstatistics uses the partial F test with rank-difference and residual degrees of freedom,
//!   strict p<Pin entry / p>Pout removal. Pin/Pout default .05/.1.
//! - After each entry, repeatedly try removal; stop on no improvement, a revisited subset,
//!   MaxEffects (default all), or MaxSteps (default 3*effects). Each entry/removal counts a step.
//!   Ties keep the first candidate in factor/covariate order.
//! - ADP runs before selection: `lin_default_drug` selects Sex, BP_transformed, Age_transformed;
//!   `lin_default_goods` selects Class, Promotion_transformed. No-selection ADP keeps all
//!   surviving effects (`lin_adp_none_drug/goods`). See adp's rule list and sources.
//!
//! GAP: Fstatistics/AdjustedRSquare, custom limits and tied/near-singular effect selection have
//! Guide formulas and synthetic tests, but no Modeler differential oracle. Near-singular
//! categorical candidates are conservatively excluded; need an aliased-factor selection probe.
//! Unsupported in the build node: ASE (needs overfit records), best subsets, custom interactions,
//! weights and the unverified ADP settings/data described in [`crate::adp`].

use crate::common::{data_field, mining_field};
use crate::{AlgoError, Dataset};
use dms_pmml::{
    GeneralModelType, GeneralRegressionModel, MiningFunction, Model, ModelHeader, PCell, PPCell,
    Pmml, Usage,
};
use dms_types::{Measurement, Table, Value, ValueSpec};

/// Forward-stepwise entry/removal criterion (Algorithms Guide, Linear modeling algorithms).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    None,
    Aicc,
    AdjustedRSquare,
    FStatistics,
}

/// Which node's model to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// The Linear node: `generalLinear`, parameters `P0000001`, ...
    Linear,
    /// The classic Regression node: `regression`, parameters named after their fields.
    Regression,
}

/// Settings of a Linear or Regression build node.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearSettings {
    pub style: Style,
    /// `linreg.Intercept` / `includeConstant` (default true).
    pub intercept: bool,
    pub selection: Selection,
    pub pin: f64,
    pub pout: f64,
    pub max_steps: Option<usize>,
    pub max_effects: Option<usize>,
    pub entry_tolerance: f64,
    pub preparation: Option<crate::adp::AdpSettings>,
    /// The model name.
    pub name: Option<String>,
}

impl Default for LinearSettings {
    fn default() -> Self {
        LinearSettings {
            style: Style::Linear,
            intercept: true,
            selection: Selection::None,
            pin: 0.05,
            pout: 0.1,
            max_steps: None,
            max_effects: None,
            entry_tolerance: 1e-4,
            preparation: None,
            name: None,
        }
    }
}

/// One PMML parameter and, unless it is a reference category, its design-matrix column.
#[derive(Clone)]
enum Parameter {
    Intercept,
    Covariate {
        position: usize,
    },
    Indicator {
        position: usize,
        category: String,
        reference: bool,
    },
}

impl Parameter {
    fn position(&self) -> Option<usize> {
        match self {
            Self::Intercept => None,
            Self::Covariate { position } | Self::Indicator { position, .. } => Some(*position),
        }
    }

    fn estimated(&self) -> bool {
        !matches!(
            self,
            Parameter::Indicator {
                reference: true,
                ..
            }
        )
    }

    fn value(&self, row: &[Value]) -> f64 {
        match self {
            Parameter::Intercept => 1.0,
            Parameter::Covariate { position } => {
                row.get(*position).and_then(Value::as_f64).unwrap_or(0.0)
            }
            Parameter::Indicator {
                position, category, ..
            } => {
                if row
                    .get(*position)
                    .is_some_and(|v| v.to_string() == *category)
                {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

/// Train a Linear or Regression model; the build node maps Linear's ADP/stepwise defaults.
pub fn train(dataset: &Dataset<'_>, settings: &LinearSettings) -> Result<Pmml, AlgoError> {
    validate(settings)?;
    if dataset.frequency.is_some() {
        return Err(AlgoError::Unsupported("Linear frequency weights".into()));
    }
    if dataset.inputs.len() > 256 || dataset.table.row_count() > 1_000_000 {
        return Err(AlgoError::Data(
            "Linear limit: 256 inputs / 1000000 records".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for &position in dataset.inputs {
        if dataset.table.model().fields().get(position).is_none()
            || position == dataset.target
            || !seen.insert(position)
        {
            return Err(AlgoError::Settings(
                "Linear input positions must be valid, distinct and exclude the target".into(),
            ));
        }
        if let Some(field) = dataset.table.model().fields().get(position)
            && field.measurement != Measurement::Continuous
            && (matches!(&field.values, ValueSpec::Categories(values) if values.len() > 512)
                || bounded_categories(dataset.table, position, 512)?.len() > 512)
        {
            return Err(AlgoError::Data(
                "Linear limit: 512 categories per input".into(),
            ));
        }
    }
    if let Some(adp) = &settings.preparation {
        let prepared = crate::adp::prepare(dataset, adp)?;
        let data = Dataset {
            table: &prepared.table,
            inputs: &prepared.inputs,
            ..*dataset
        };
        let mut plain = settings.clone();
        plain.preparation = None;
        let mut model = train(&data, &plain)?;
        model.data_dictionary = crate::common::data_dictionary(dataset);
        model.transformations = prepared.transformations;
        if let Model::GeneralRegression(m) = &mut model.model {
            m.header.mining_schema = dataset
                .inputs
                .iter()
                .filter_map(|&p| mining_field(dataset.table, p, Usage::Active))
                .chain(mining_field(dataset.table, dataset.target, Usage::Target))
                .collect();
        }
        return Ok(model);
    }
    let table = dataset.table;
    let fields = table.model().fields();
    let field = |position: usize| {
        fields
            .get(position)
            .ok_or_else(|| AlgoError::Data(format!("no field at position {position}")))
    };
    let target = field(dataset.target)?;
    if target.measurement != Measurement::Continuous {
        return Err(AlgoError::Settings(format!(
            "the Linear target {} is not continuous",
            target.name
        )));
    }
    let present = |position: usize, value: Option<&Value>| match value {
        None | Some(Value::Null) => false,
        Some(v) => fields.get(position).is_some_and(|f| !f.is_missing(v)),
    };
    let rows: Vec<&Vec<Value>> = table
        .rows()
        .iter()
        .filter(|row| {
            std::iter::once(dataset.target)
                .chain(dataset.inputs.iter().copied())
                .all(|p| present(p, row.get(p)))
        })
        .collect();
    for &position in std::iter::once(&dataset.target).chain(dataset.inputs.iter()) {
        if field(position)?.measurement == Measurement::Continuous
            && rows.iter().any(|r| {
                r.get(position)
                    .and_then(Value::as_f64)
                    .is_none_or(|v| !v.is_finite())
            })
        {
            return Err(AlgoError::Data(format!(
                "Linear field {} must contain finite numbers",
                field(position)?.name
            )));
        }
    }

    let mut parameters = Vec::new();
    if settings.intercept {
        parameters.push(Parameter::Intercept);
    }
    // ALM lists factors before covariates, in source order within each class (all six probes).
    let ordered: Vec<usize> = dataset
        .inputs
        .iter()
        .copied()
        .filter(|&p| {
            fields
                .get(p)
                .is_some_and(|f| f.measurement != Measurement::Continuous)
        })
        .chain(dataset.inputs.iter().copied().filter(|&p| {
            fields
                .get(p)
                .is_some_and(|f| f.measurement == Measurement::Continuous)
        }))
        .collect();
    for &position in &ordered {
        if field(position)?.measurement == Measurement::Continuous {
            parameters.push(Parameter::Covariate { position });
            continue;
        }
        if settings.style == Style::Regression {
            return Err(AlgoError::Unsupported(format!(
                "Regression input {} that is not continuous",
                field(position)?.name
            )));
        }
        let categories = bounded_categories(table, position, 512)?;
        if categories.len() > 512 || parameters.len().saturating_add(categories.len()) > 512 {
            return Err(AlgoError::Data("Linear limit: 512 parameters".into()));
        }
        let last = categories.len().saturating_sub(1);
        for (k, category) in categories.into_iter().enumerate() {
            parameters.push(Parameter::Indicator {
                position,
                category,
                reference: k == last,
            });
        }
    }
    if parameters.len() > 512 {
        return Err(AlgoError::Data("Linear limit: 512 parameters".into()));
    }
    let y: Vec<f64> = rows
        .iter()
        .map(|row| {
            row.get(dataset.target)
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        })
        .collect();
    if y.len() < 2 || y.iter().any(|x| !x.is_finite()) || y.iter().all(|x| *x == y[0]) {
        return Err(AlgoError::Data(
            "Linear needs a finite, nonconstant target and two records".into(),
        ));
    }
    if rows.len().saturating_mul(parameters.len()) > 16_000_000 {
        return Err(AlgoError::Data(
            "Linear design exceeds 16000000 cells".into(),
        ));
    }
    let selected = select(&parameters, &ordered, &rows, &y, settings)?;
    parameters.retain(|p| p.position().is_none_or(|pos| selected.contains(&pos)));
    let fit = fit(&parameters, &rows, &y)?;
    let mut beta = fit.beta.into_iter();

    let mut names = Vec::new();
    let mut pp_matrix = Vec::new();
    let mut param_matrix = Vec::new();
    for (index, parameter) in parameters.iter().enumerate() {
        let name = match (settings.style, parameter) {
            (Style::Linear, _) => format!("P{:07}", index + 1),
            (Style::Regression, Parameter::Intercept) => "(Constant)".to_string(),
            (Style::Regression, Parameter::Covariate { position }) => {
                fields[*position].name.clone()
            }
            (
                Style::Regression,
                Parameter::Indicator {
                    position, category, ..
                },
            ) => format!("[{}={category}]", fields[*position].name),
        };
        match parameter {
            Parameter::Intercept => {}
            Parameter::Covariate { position } => pp_matrix.push(PPCell {
                value: "1".into(),
                predictor: fields[*position].name.clone(),
                parameter: name.clone(),
            }),
            Parameter::Indicator {
                position, category, ..
            } => pp_matrix.push(PPCell {
                value: category.clone(),
                predictor: fields[*position].name.clone(),
                parameter: name.clone(),
            }),
        }
        let value = if parameter.estimated() {
            beta.next().unwrap_or(0.0)
        } else {
            0.0
        };
        param_matrix.push(PCell {
            target_category: None,
            parameter: name.clone(),
            beta: value,
        });
        names.push(name);
    }
    let (covariates, factors): (Vec<usize>, Vec<usize>) = selected
        .iter()
        .partition(|&&p| fields[p].measurement == Measurement::Continuous);
    let name_of = |positions: Vec<usize>| -> Vec<String> {
        positions
            .into_iter()
            .map(|p| fields[p].name.clone())
            .collect()
    };
    let mut mining_schema: Vec<_> = dataset
        .inputs
        .iter()
        .filter_map(|&p| mining_field(table, p, Usage::Active))
        .collect();
    mining_schema.extend(mining_field(table, dataset.target, Usage::Target));
    let (version, algorithm, model_type) = match settings.style {
        Style::Linear => (
            "4.3",
            Some("ALM".to_string()),
            GeneralModelType::GeneralLinear,
        ),
        Style::Regression => ("4.2", None, GeneralModelType::Regression),
    };
    Ok(Pmml {
        version: version.into(),
        data_dictionary: std::iter::once(dataset.target)
            .chain(dataset.inputs.iter().copied())
            .filter_map(|p| data_field(table, p))
            .collect(),
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::GeneralRegression(GeneralRegressionModel {
            header: ModelHeader {
                name: settings.name.clone(),
                function: MiningFunction::Regression,
                algorithm,
                mining_schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            model_type,
            target_reference_category: None,
            link_function: None,
            link_parameter: None,
            parameters: names,
            factors: name_of(factors),
            covariates: name_of(covariates),
            pp_matrix,
            param_matrix,
        }),
    })
}

/// Preserve Type category order, stopping after limit+1 values before allocating an
/// unbounded design. Unknown categories retain first-seen order without quadratic searches.
pub(crate) fn bounded_categories(
    table: &Table,
    position: usize,
    limit: usize,
) -> Result<Vec<String>, AlgoError> {
    let f = table
        .model()
        .fields()
        .get(position)
        .ok_or_else(|| AlgoError::Data("Linear category field out of range".into()))?;
    let mut values = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut add = |v: &Value| {
        if !f.is_missing(v) {
            let value = v.to_string();
            if seen.insert(value.clone()) {
                values.push(value);
            }
        }
        values.len() > limit
    };
    match &f.values {
        ValueSpec::Categories(cats) => {
            for v in cats {
                if add(v) {
                    break;
                }
            }
        }
        ValueSpec::Flag {
            true_value,
            false_value,
        } => {
            add(false_value);
            add(true_value);
        }
        _ => {
            for row in table.rows() {
                if row.get(position).is_some_and(&mut add) {
                    break;
                }
            }
        }
    }
    Ok(values)
}

fn validate(s: &LinearSettings) -> Result<(), AlgoError> {
    if !s.pin.is_finite()
        || !s.pout.is_finite()
        || !(0.0..=1.0).contains(&s.pin)
        || !(0.0..=1.0).contains(&s.pout)
        || s.pin > s.pout
        || !s.entry_tolerance.is_finite()
        || !(0.0..1.0).contains(&s.entry_tolerance)
        || s.max_steps.is_some_and(|n| n == 0 || n > 10000)
        || s.max_effects.is_some_and(|n| n == 0 || n > 256)
    {
        return Err(AlgoError::Settings(
            "Linear: invalid probabilities, tolerance or selection limits".into(),
        ));
    }
    Ok(())
}

struct Fit {
    beta: Vec<f64>,
    rss: f64,
    rank: usize,
}

fn fit(parameters: &[Parameter], rows: &[&Vec<Value>], y: &[f64]) -> Result<Fit, AlgoError> {
    let estimated: Vec<_> = parameters.iter().filter(|p| p.estimated()).collect();
    let design: Vec<Vec<f64>> = rows
        .iter()
        .map(|r| estimated.iter().map(|p| p.value(r)).collect())
        .collect();
    let (beta, rank) = least_squares(design.clone(), y.to_vec(), estimated.len())?;
    let rss: f64 = design
        .iter()
        .zip(y)
        .map(|(row, y)| {
            let error = y - row.iter().zip(&beta).map(|(x, b)| x * b).sum::<f64>();
            error * error
        })
        .sum();
    if !rss.is_finite() {
        return Err(AlgoError::Data(
            "Linear residual sum of squares overflow".into(),
        ));
    }
    Ok(Fit { beta, rank, rss })
}

fn subset(parameters: &[Parameter], selected: &[usize]) -> Vec<Parameter> {
    parameters
        .iter()
        .filter(|p| p.position().is_none_or(|pos| selected.contains(&pos)))
        .cloned()
        .collect()
}

/// Rank includes the intercept but excludes reference and aliased columns. AICc omits
/// the additive n constant: n*ln(RSS/n)+2*n*rank/(n-rank-1), confirmed by the oracle AICc
/// (drug ADP none: rank=5, n=200, RSS=2.65571320994146).
fn objective(f: &Fit, n: usize, criterion: Selection) -> f64 {
    let (n, rank) = (n as f64, f.rank as f64);
    match criterion {
        Selection::Aicc if n > rank + 1.0 => {
            n * (f.rss.max(f64::MIN_POSITIVE) / n).ln() + 2.0 * n * rank / (n - rank - 1.0)
        }
        // Minimizing residual mean square is equivalent to maximizing adjusted R squared.
        Selection::AdjustedRSquare if n > rank => f.rss / (n - rank),
        _ => f64::INFINITY,
    }
}

fn change_p(smaller: &Fit, larger: &Fit, n: usize) -> f64 {
    let df = larger.rank.saturating_sub(smaller.rank);
    let residual = n.saturating_sub(larger.rank);
    if df == 0 || residual == 0 {
        return 1.0;
    }
    if larger.rss == 0.0 {
        return if smaller.rss > 0.0 { 0.0 } else { 1.0 };
    }
    crate::stats::f_p(
        ((smaller.rss - larger.rss).max(0.0) / df as f64) / (larger.rss / residual as f64),
        df as f64,
        residual as f64,
    )
}

/// Reject entry that makes a continuous predictor's residual variance fraction smaller
/// than EffectEntryTolerance. GAP: near-singular multi-category effects require a dedicated
/// oracle; those candidates are rejected rather than assigning arbitrary indicator aliases.
fn eligible(
    parameters: &[Parameter],
    rows: &[&Vec<Value>],
    tolerance: f64,
) -> Result<bool, AlgoError> {
    let columns: Vec<_> = parameters.iter().filter(|p| p.estimated()).collect();
    for (j, column) in columns.iter().enumerate() {
        if matches!(column, Parameter::Intercept) {
            continue;
        }
        let y: Vec<f64> = rows.iter().map(|r| column.value(r)).collect();
        let mean = y.iter().sum::<f64>() / y.len().max(1) as f64;
        let total: f64 = y.iter().map(|v| (v - mean).powi(2)).sum();
        if total == 0.0 {
            return Ok(false);
        }
        let design: Vec<Vec<f64>> = rows
            .iter()
            .map(|r| {
                columns
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| *k != j)
                    .map(|(_, c)| c.value(r))
                    .collect()
            })
            .collect();
        let (beta, _) = least_squares(design.clone(), y.clone(), columns.len().saturating_sub(1))?;
        let rss: f64 = design
            .iter()
            .zip(y)
            .map(|(r, v)| (v - r.iter().zip(&beta).map(|(x, b)| x * b).sum::<f64>()).powi(2))
            .sum();
        if rss / total < tolerance {
            return Ok(false);
        }
    }
    Ok(true)
}

fn select(
    parameters: &[Parameter],
    effects: &[usize],
    rows: &[&Vec<Value>],
    y: &[f64],
    s: &LinearSettings,
) -> Result<Vec<usize>, AlgoError> {
    if s.selection == Selection::None {
        return Ok(effects.to_vec());
    }
    let mut selected = Vec::new();
    let mut current = fit(&subset(parameters, &selected), rows, y)?;
    let mut visited = std::collections::HashSet::new();
    visited.insert(selected.clone());
    let mut steps = 0;
    let limit = s.max_steps.unwrap_or(effects.len().saturating_mul(3));
    while steps < limit && selected.len() < s.max_effects.unwrap_or(effects.len()) {
        let mut best: Option<(Vec<usize>, Fit, f64)> = None;
        for &effect in effects.iter().filter(|p| !selected.contains(p)) {
            let candidate: Vec<usize> = effects
                .iter()
                .copied()
                .filter(|p| *p == effect || selected.contains(p))
                .collect();
            let params = subset(parameters, &candidate);
            if !eligible(&params, rows, s.entry_tolerance)? {
                continue;
            }
            let f = fit(&params, rows, y)?;
            if f.rank <= current.rank || f.rank >= rows.len() {
                continue;
            }
            let score = if s.selection == Selection::FStatistics {
                change_p(&current, &f, rows.len())
            } else {
                objective(&f, rows.len(), s.selection)
            };
            if best.as_ref().is_none_or(|b| score < b.2) {
                best = Some((candidate, f, score));
            }
        }
        let Some((candidate, f, score)) = best else {
            break;
        };
        let threshold = if s.selection == Selection::FStatistics {
            s.pin
        } else {
            objective(&current, rows.len(), s.selection)
        };
        if score >= threshold || !visited.insert(candidate.clone()) {
            break;
        }
        selected = candidate;
        current = f;
        steps += 1;
        while steps < limit {
            let mut best: Option<(Vec<usize>, Fit, f64)> = None;
            for &effect in &selected {
                let candidate: Vec<usize> =
                    selected.iter().copied().filter(|p| *p != effect).collect();
                let f = fit(&subset(parameters, &candidate), rows, y)?;
                let score = if s.selection == Selection::FStatistics {
                    -change_p(&f, &current, rows.len())
                } else {
                    objective(&f, rows.len(), s.selection)
                };
                if best.as_ref().is_none_or(|b| score < b.2) {
                    best = Some((candidate, f, score));
                }
            }
            let Some((candidate, f, score)) = best else {
                break;
            };
            let threshold = if s.selection == Selection::FStatistics {
                -s.pout
            } else {
                objective(&current, rows.len(), s.selection)
            };
            if score >= threshold {
                break;
            }
            if !visited.insert(candidate.clone()) {
                return Ok(selected);
            }
            selected = candidate;
            current = f;
            steps += 1;
        }
    }
    Ok(selected)
}

/// Least squares by Householder QR; an (almost) aliased column gets beta 0.
#[allow(
    clippy::needless_range_loop,
    reason = "column-wise Householder updates index rows and columns together"
)]
pub(crate) fn least_squares(
    mut a: Vec<Vec<f64>>,
    mut y: Vec<f64>,
    p: usize,
) -> Result<(Vec<f64>, usize), AlgoError> {
    let n = a.len();
    if n == 0
        || a.iter()
            .any(|r| r.len() != p || r.iter().any(|x| !x.is_finite()))
        || y.len() != n
    {
        return Err(AlgoError::Data(format!(
            "{n} complete records for {p} parameters"
        )));
    }
    let scale: Vec<f64> = (0..p)
        .map(|j| a.iter().map(|row| row[j] * row[j]).sum::<f64>().sqrt())
        .collect();
    let mut active = vec![true; p];
    let mut pivot = 0;
    let mut order = Vec::new();
    for j in 0..p {
        let norm = (pivot..n).map(|i| a[i][j] * a[i][j]).sum::<f64>().sqrt();
        if pivot >= n || norm <= 1e-6 * scale[j].max(f64::MIN_POSITIVE) {
            active[j] = false;
            continue;
        }
        let alpha = if a[pivot][j] > 0.0 { -norm } else { norm };
        let mut v: Vec<f64> = (pivot..n).map(|i| a[i][j]).collect();
        v[0] -= alpha;
        let vv: f64 = v.iter().map(|x| x * x).sum();
        for k in j..p {
            let dot: f64 = (pivot..n).map(|i| v[i - pivot] * a[i][k]).sum();
            let f = 2.0 * dot / vv;
            for i in pivot..n {
                a[i][k] -= f * v[i - pivot];
            }
        }
        let dot: f64 = (pivot..n).map(|i| v[i - pivot] * y[i]).sum();
        let f = 2.0 * dot / vv;
        for i in pivot..n {
            y[i] -= f * v[i - pivot];
        }
        order.push((pivot, j));
        pivot += 1;
    }
    let mut beta = vec![0.0; p];
    for &(row, j) in order.iter().rev() {
        let s: f64 = (j + 1..p)
            .filter(|&k| active[k])
            .map(|k| a[row][k] * beta[k])
            .sum();
        beta[j] = (y[row] - s) / a[row][j];
    }
    if beta.iter().any(|v| !v.is_finite()) {
        return Err(AlgoError::Data("Linear least squares overflow".into()));
    }
    Ok((beta, pivot))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_removes_a_proxy_and_counts_entry_and_removal_steps() {
        // Orthogonal bit contrasts: a is the best single predictor, but b+c replaces it.
        let records: Vec<Vec<Value>> = (0..64)
            .map(|i| {
                let contrast = |bit| if i & (1 << bit) == 0 { -1.0 } else { 1.0 };
                let (b, c, d, e) = (contrast(0), contrast(1), contrast(2), contrast(3));
                vec![
                    Value::Real(b + c + 0.5 * d),
                    Value::Real(b),
                    Value::Real(c),
                    Value::Real(4.0 + b + c + 0.1 * e),
                ]
            })
            .collect();
        let rows: Vec<_> = records.iter().collect();
        let y: Vec<_> = records.iter().map(|r| r[3].as_f64().unwrap()).collect();
        let parameters = vec![
            Parameter::Intercept,
            Parameter::Covariate { position: 0 },
            Parameter::Covariate { position: 1 },
            Parameter::Covariate { position: 2 },
        ];
        for criterion in [
            Selection::Aicc,
            Selection::AdjustedRSquare,
            Selection::FStatistics,
        ] {
            let mut s = LinearSettings {
                selection: criterion,
                ..LinearSettings::default()
            };
            assert_eq!(
                select(&parameters, &[0, 1, 2], &rows, &y, &s).unwrap(),
                [1, 2]
            );
            s.max_steps = Some(1);
            assert_eq!(select(&parameters, &[0, 1, 2], &rows, &y, &s).unwrap(), [0]);
            s.max_steps = Some(3);
            assert_eq!(
                select(&parameters, &[0, 1, 2], &rows, &y, &s).unwrap(),
                [0, 1, 2]
            );
            s.max_steps = Some(4);
            assert_eq!(
                select(&parameters, &[0, 1, 2], &rows, &y, &s).unwrap(),
                [1, 2]
            );
            s.max_steps = None;
            s.max_effects = Some(1);
            assert_eq!(select(&parameters, &[0, 1, 2], &rows, &y, &s).unwrap(), [0]);
        }
        let s = LinearSettings {
            selection: Selection::FStatistics,
            pin: 1e-100,
            ..LinearSettings::default()
        };
        assert!(
            select(&parameters, &[0, 1, 2], &rows, &y, &s)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn malformed_design_and_selection_settings_return_errors() {
        assert!(least_squares(vec![vec![]], vec![1.0], 1).is_err());
        assert!(least_squares(vec![vec![f64::NAN]], vec![1.0], 1).is_err());
        for s in [
            LinearSettings {
                pin: f64::NAN,
                ..LinearSettings::default()
            },
            LinearSettings {
                max_steps: Some(usize::MAX),
                ..LinearSettings::default()
            },
            LinearSettings {
                max_effects: Some(0),
                ..LinearSettings::default()
            },
        ] {
            assert!(validate(&s).is_err());
        }
        // A wide/aliased matrix exhausts row pivots without indexing beyond its rows.
        let (beta, rank) = least_squares(vec![vec![1.0, 2.0]], vec![3.0], 2).unwrap();
        assert_eq!(rank, 1);
        assert_eq!(beta, [3.0, 0.0]);
    }

    #[test]
    fn unknown_high_cardinality_input_is_bounded_before_adp_and_dictionary_generation() {
        let mut target = dms_types::Field::new("y", dms_types::Storage::Real);
        target.measurement = Measurement::Continuous;
        let mut input = dms_types::Field::new("x", dms_types::Storage::String);
        input.measurement = Measurement::Nominal;
        let table = Table::new(
            dms_types::DataModel::new(vec![target, input]).unwrap(),
            (0..1000)
                .map(|i| vec![Value::Real(i as f64), Value::string(i.to_string())])
                .collect(),
        )
        .unwrap();
        let data = Dataset {
            table: &table,
            target: 0,
            inputs: &[1],
            frequency: None,
        };
        for preparation in [None, Some(crate::adp::AdpSettings::default())] {
            assert!(matches!(
                train(
                    &data,
                    &LinearSettings {
                        preparation,
                        ..LinearSettings::default()
                    }
                ),
                Err(AlgoError::Data(_))
            ));
        }
    }
}
