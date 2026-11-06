//! Generalized Linear Engine Phase I (`spss.gle!gle_builder` / `spss.gle!gle_applier`).
//!
//! Formulas: IBM SPSS Modeler 19 Algorithms Guide, “Generalized Linear Engine (GLE)
//! Algorithm”, Phase I (from the public AlgorithmsGuide-19.0.txt). Saved stream
//! identifiers and `_` property names come from `corpus/gle/*.str`.
//!
//! Phase I: normal, binomial, Poisson and gamma; canonical links plus identity and
//! log; main effects and an intercept; default estimation; scoring of the mean
//! (and the binomial class / probability).
//!
//! Assumptions where the Guide leaves a choice:
//! - Hybrid estimation: one Fisher-scoring iteration then Newton–Raphson (GENLIN
//!   `max_fisher_iterations` default 1). If Fisher already converges, Newton
//!   continues until it converges too.
//! - M = 100 iterations, J = 5 step-halves, parameter convergence 1e-6 absolute,
//!   Hessian 1e-4 absolute after the other tests, log-likelihood test off.
//! - Starting values follow section 3.1.3.1. Non-binomial working response is η;
//!   binomial adds (y−μ)g′(μ).
//! - Scale φ is held at 1 while estimating β (scoring uses only the mean).
//! - SWEEP on XᵀWX in parameter order; a pivot ≤ 1e-8 of the original diagonal
//!   marks a redundant column with β = 0.
//! - Over-parameterised factor dummies in Type order; SWEEP zeros the dependent
//!   level (typically the last when an intercept is present).
//! - Binomial: last Type category is the reference (y = 0); first is success.
//! - Normal + identity + intercept: centre y, fit, add the mean back to the
//!   intercept after mapping β to the original scale (section 2.2.4).
//! - Covariate columns are centred and scaled as section 2.2.4; dummies are not.

use crate::common::{data_field, mining_field};
use crate::linear::bounded_categories;
use crate::{AlgoError, Dataset};
use dms_pmml::{
    GeneralModelType, GeneralRegressionModel, MiningFunction, Model, ModelHeader, PCell, PPCell,
    Pmml, Usage,
};
use dms_types::{Measurement, Storage, Value};

const MAX_RECORDS: usize = 1_000_000;
const MAX_PARAMETERS: usize = 1024;
const MAX_CATEGORIES: usize = 512;
const REL_EPS: f64 = 1e-6;
const ETA_CAP: f64 = 80.0;
const MU_FLOOR: f64 = 1e-12;

/// Target distribution (Phase I subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Distribution {
    Normal,
    Binomial,
    Poisson,
    Gamma,
}

/// Link function (Phase I subset).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Link {
    Identity,
    Log,
    Logit,
    /// Power link; Phase I accepts only α = −1 (gamma canonical inverse).
    Power {
        alpha: f64,
    },
}

impl Link {
    fn name(self) -> &'static str {
        match self {
            Link::Identity => "identity",
            Link::Log => "log",
            Link::Logit => "logit",
            Link::Power { .. } => "power",
        }
    }

    fn parameter(self) -> Option<f64> {
        match self {
            Link::Power { alpha } => Some(alpha),
            _ => None,
        }
    }
}

/// How the Hessian is formed (section 3.1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Estimation {
    FisherScoring,
    NewtonRaphson,
    Hybrid { fisher_iterations: usize },
}

/// Settings of a GLE build node (Modeler defaults in [`Default`]).
#[derive(Debug, Clone, PartialEq)]
pub struct GleSettings {
    pub distribution: Distribution,
    pub link: Link,
    pub intercept: bool,
    pub estimation: Estimation,
    pub max_iterations: usize,
    pub max_step_halving: usize,
    pub parameter_convergence: f64,
    pub parameter_relative: bool,
    pub likelihood_convergence: Option<f64>,
    pub likelihood_relative: bool,
    pub hessian_convergence: f64,
    pub hessian_relative: bool,
    pub singularity: f64,
    pub name: Option<String>,
}

impl Default for GleSettings {
    fn default() -> Self {
        GleSettings {
            distribution: Distribution::Normal,
            link: Link::Identity,
            intercept: true,
            estimation: Estimation::Hybrid {
                fisher_iterations: 1,
            },
            max_iterations: 100,
            max_step_halving: 5,
            parameter_convergence: 1e-6,
            parameter_relative: false,
            likelihood_convergence: None,
            likelihood_relative: false,
            hessian_convergence: 1e-4,
            hessian_relative: false,
            singularity: 1e-12,
            name: None,
        }
    }
}

/// Fitted Phase I GLE: original-scale coefficients and the PMML nugget.
#[derive(Debug, Clone)]
pub struct GleFit {
    pub pmml: Pmml,
    pub parameter_names: Vec<String>,
    pub coefficients: Vec<f64>,
    pub iterations: usize,
    pub log_likelihood: f64,
    pub converged: bool,
}

/// Train a Phase I GLE model.
pub fn train(dataset: &Dataset<'_>, settings: &GleSettings) -> Result<Pmml, AlgoError> {
    Ok(fit(dataset, settings)?.pmml)
}

/// Fit a Phase I GLE model and keep the coefficients for tests and predictions.
pub fn fit(dataset: &Dataset<'_>, settings: &GleSettings) -> Result<GleFit, AlgoError> {
    if dataset.frequency.is_some() {
        return Err(AlgoError::Unsupported("GAP: GLE frequency weights".into()));
    }
    validate_settings(settings)?;
    let data = Data::read(dataset, settings)?;
    let work = Work::from_data(&data, settings)?;
    let (beta_t, iterations, loglik, converged) = estimate(&work, settings)?;
    let beta = back_transform(&work, &beta_t);
    let pmml = to_pmml(dataset, &data, settings, &beta)?;
    Ok(GleFit {
        pmml,
        parameter_names: data
            .columns
            .iter()
            .map(|c| c.parameter_name.clone())
            .collect(),
        coefficients: beta,
        iterations,
        log_likelihood: loglik,
        converged,
    })
}

fn validate_settings(settings: &GleSettings) -> Result<(), AlgoError> {
    if !settings.parameter_convergence.is_finite() || settings.parameter_convergence < 0.0 {
        return Err(AlgoError::Settings(
            "GLE p_converge must be a non-negative finite number".into(),
        ));
    }
    if !settings.hessian_convergence.is_finite() || settings.hessian_convergence < 0.0 {
        return Err(AlgoError::Settings(
            "GLE h_converge must be a non-negative finite number".into(),
        ));
    }
    if !settings.singularity.is_finite() || settings.singularity < 0.0 {
        return Err(AlgoError::Settings(
            "GLE sing_tolerance must be a non-negative finite number".into(),
        ));
    }
    if settings.max_iterations > 10_000 {
        return Err(AlgoError::Settings(
            "GLE max_iterations is too large".into(),
        ));
    }
    if settings.max_step_halving > 64 {
        return Err(AlgoError::Settings(
            "GLE step-halving limit is too large".into(),
        ));
    }
    if let Some(eps) = settings.likelihood_convergence
        && (!eps.is_finite() || eps < 0.0)
    {
        return Err(AlgoError::Settings(
            "GLE l_converge must be a non-negative finite number".into(),
        ));
    }
    if let Link::Power { alpha } = settings.link
        && (alpha + 1.0).abs() > 1e-12
    {
        return Err(AlgoError::Unsupported(format!(
            "GAP: GLE power link with parameter {alpha} (Phase I allows -1 only)"
        )));
    }
    if !combination_ok(settings.distribution, settings.link) {
        return Err(AlgoError::Settings(format!(
            "GAP: GLE distribution {:?} with link {:?} is not a Phase I combination",
            settings.distribution, settings.link
        )));
    }
    Ok(())
}

fn combination_ok(distribution: Distribution, link: Link) -> bool {
    matches!(
        (distribution, link),
        (Distribution::Normal, Link::Identity | Link::Log)
            | (Distribution::Poisson, Link::Identity | Link::Log)
            | (
                Distribution::Gamma,
                Link::Identity | Link::Log | Link::Power { .. }
            )
            | (
                Distribution::Binomial,
                Link::Identity | Link::Log | Link::Logit
            )
    )
}

struct Column {
    kind: ColumnKind,
    parameter_name: String,
}

enum ColumnKind {
    Intercept,
    Covariate {
        position: usize,
        name: String,
    },
    Indicator {
        position: usize,
        name: String,
        category: String,
    },
}

struct Data {
    y: Vec<f64>,
    rows: Vec<Vec<f64>>,
    columns: Vec<Column>,
    success: Option<String>,
    reference: Option<String>,
    factors: Vec<String>,
    covariates: Vec<String>,
}

impl Data {
    fn read(dataset: &Dataset<'_>, settings: &GleSettings) -> Result<Data, AlgoError> {
        let table = dataset.table;
        let fields = table.model().fields();
        let target = fields
            .get(dataset.target)
            .ok_or_else(|| AlgoError::Data("GLE target field is out of range".into()))?;
        if dataset.inputs.len() > 256 || table.row_count() > MAX_RECORDS {
            return Err(AlgoError::Data(
                "GLE limit: 256 inputs / 1000000 records".into(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for &position in dataset.inputs {
            if fields.get(position).is_none()
                || position == dataset.target
                || !seen.insert(position)
            {
                return Err(AlgoError::Settings(
                    "GLE input positions must be valid, distinct and exclude the target".into(),
                ));
            }
        }

        let (labels, success_label, reference) = match settings.distribution {
            Distribution::Binomial => {
                if target.measurement == Measurement::Continuous {
                    return Err(AlgoError::Data(format!(
                        "GLE binomial target {} is continuous",
                        target.name
                    )));
                }
                let labels = crate::categories(dataset, dataset.target);
                if labels.len() != 2 {
                    return Err(AlgoError::Data(format!(
                        "The target variable has more than 2 levels. A binary target must have 2 levels ({} has {}).",
                        target.name,
                        labels.len()
                    )));
                }
                let reference = labels.last().cloned();
                let success = labels.first().cloned();
                (labels, success, reference)
            }
            _ => {
                if target.measurement != Measurement::Continuous
                    && target.measurement != Measurement::Default
                {
                    return Err(AlgoError::Data(format!(
                        "GLE {:?} target {} is not continuous",
                        settings.distribution, target.name
                    )));
                }
                if !matches!(target.storage, Storage::Integer | Storage::Real) {
                    return Err(AlgoError::Data(format!(
                        "GLE {:?} target {} is not numeric",
                        settings.distribution, target.name
                    )));
                }
                (Vec::new(), None, None)
            }
        };

        let mut columns = Vec::new();
        let mut next = 0usize;
        let mut push = |kind: ColumnKind| {
            let parameter_name = format!("P{next:07}");
            next = next.saturating_add(1);
            columns.push(Column {
                kind,
                parameter_name,
            });
        };
        if settings.intercept {
            push(ColumnKind::Intercept);
        }
        let mut factors = Vec::new();
        let mut covariates = Vec::new();
        for &position in dataset.inputs {
            let input = fields
                .get(position)
                .ok_or_else(|| AlgoError::Data("GLE input field is out of range".into()))?;
            match input.measurement {
                Measurement::Typeless | Measurement::Collection | Measurement::Geospatial => {
                    return Err(AlgoError::Unsupported(format!(
                        "GAP: GLE input {} with measurement {:?}",
                        input.name, input.measurement
                    )));
                }
                Measurement::Continuous | Measurement::Default
                    if matches!(input.storage, Storage::Integer | Storage::Real) =>
                {
                    covariates.push(input.name.clone());
                    push(ColumnKind::Covariate {
                        position,
                        name: input.name.clone(),
                    });
                }
                _ => {
                    // A blank string is missing, not a level (`gle/gle_normal_factor`: Modeler's
                    // model lists g = A, B, C over 20 records and scores the blank row $null$).
                    let mut cats = bounded_categories(table, position, MAX_CATEGORIES)?;
                    cats.retain(|category| !category.trim().is_empty());
                    if cats.is_empty() || cats.len() > MAX_CATEGORIES {
                        return Err(AlgoError::Data(format!(
                            "GLE factor {} has no usable categories",
                            input.name
                        )));
                    }
                    factors.push(input.name.clone());
                    for category in cats {
                        push(ColumnKind::Indicator {
                            position,
                            name: input.name.clone(),
                            category,
                        });
                    }
                }
            }
        }
        if columns.is_empty() {
            return Err(AlgoError::Settings(
                "GLE has no parameters to estimate".into(),
            ));
        }
        if columns.len() > MAX_PARAMETERS {
            return Err(AlgoError::Data(format!(
                "GLE would estimate {} parameters",
                columns.len()
            )));
        }

        let mut y = Vec::new();
        let mut rows = Vec::new();
        for row in table.rows() {
            let Some(target_value) = row.get(dataset.target).filter(|v| !target.is_missing(v))
            else {
                continue;
            };
            let Some(yi) = encode_target(
                target_value,
                settings.distribution,
                success_label.as_deref(),
                &labels,
            ) else {
                continue;
            };
            let Some(x) = design_row(row, fields, &columns) else {
                continue;
            };
            y.push(yi);
            rows.push(x);
            if y.len() > MAX_RECORDS {
                return Err(AlgoError::Data(
                    "GLE is limited to 1000000 complete records".into(),
                ));
            }
        }
        if y.is_empty() {
            return Err(AlgoError::Data("no complete records to train GLE".into()));
        }
        Ok(Data {
            y,
            rows,
            columns,
            success: success_label,
            reference,
            factors,
            covariates,
        })
    }
}

fn same_category(left: &str, right: &str) -> bool {
    left == right
        || matches!(
            (left.parse::<f64>(), right.parse::<f64>()),
            (Ok(a), Ok(b)) if a == b
        )
}

fn encode_target(
    value: &Value,
    distribution: Distribution,
    success: Option<&str>,
    labels: &[String],
) -> Option<f64> {
    match distribution {
        Distribution::Binomial => {
            let text = value.to_string();
            if !labels.iter().any(|label| same_category(label, &text)) {
                return None;
            }
            let is_success = success.is_some_and(|s| same_category(s, &text));
            Some(if is_success { 1.0 } else { 0.0 })
        }
        Distribution::Normal => value.as_f64().filter(|v| v.is_finite()),
        Distribution::Gamma => value.as_f64().filter(|v| v.is_finite() && *v > 0.0),
        Distribution::Poisson => {
            let v = value.as_f64()?;
            if !v.is_finite() || v < 0.0 {
                return None;
            }
            let rounded = v.round();
            if (v - rounded).abs() > 1e-9 {
                return None;
            }
            Some(rounded)
        }
    }
}

fn at<T: Copy>(values: &[T], index: usize) -> Result<T, AlgoError> {
    values
        .get(index)
        .copied()
        .ok_or_else(|| AlgoError::Data("GLE design cell is missing".into()))
}

fn slot<T>(values: &mut [T], index: usize) -> Result<&mut T, AlgoError> {
    values
        .get_mut(index)
        .ok_or_else(|| AlgoError::Data("GLE index is out of range".into()))
}

fn design_row(row: &[Value], fields: &[dms_types::Field], columns: &[Column]) -> Option<Vec<f64>> {
    let mut x = Vec::with_capacity(columns.len());
    for column in columns {
        match &column.kind {
            ColumnKind::Intercept => x.push(1.0),
            ColumnKind::Covariate { position, .. } => {
                let field = fields.get(*position)?;
                let value = row.get(*position).filter(|v| !field.is_missing(v))?;
                x.push(value.as_f64().filter(|n| n.is_finite())?);
            }
            ColumnKind::Indicator {
                position, category, ..
            } => {
                let field = fields.get(*position)?;
                let value = row.get(*position).filter(|v| {
                    !field.is_missing(v) && !matches!(v, Value::String(s) if s.trim().is_empty())
                })?;
                let text = value.to_string();
                let hit = text == *category
                    || matches!(
                        (category.parse::<f64>(), text.parse::<f64>()),
                        (Ok(a), Ok(b)) if a == b
                    );
                x.push(if hit { 1.0 } else { 0.0 });
            }
        }
    }
    Some(x)
}

struct Work {
    y: Vec<f64>,
    x: Vec<Vec<f64>>,
    center: Vec<f64>,
    scale: Vec<f64>,
    y_center: f64,
    intercept: bool,
    distribution: Distribution,
    link: Link,
    binomial: bool,
}

impl Work {
    fn from_data(data: &Data, settings: &GleSettings) -> Result<Work, AlgoError> {
        let n = data.rows.len();
        let p = data.columns.len();
        let mut center = vec![0.0; p];
        let mut scale = vec![1.0; p];
        let nf = n as f64;
        for (j, column) in data.columns.iter().enumerate() {
            match column.kind {
                ColumnKind::Intercept | ColumnKind::Indicator { .. } => {}
                ColumnKind::Covariate { .. } => {
                    let mut sum = 0.0;
                    for row in &data.rows {
                        sum += at(row, j)?;
                    }
                    let mean = sum / nf;
                    let mut ss = 0.0;
                    for row in &data.rows {
                        let d = at(row, j)? - mean;
                        ss += d * d;
                    }
                    let sd = if n > 1 { (ss / (nf - 1.0)).sqrt() } else { 0.0 };
                    if settings.intercept {
                        if sd.is_finite() && sd > 0.0 {
                            *slot(&mut center, j)? = mean;
                            *slot(&mut scale, j)? = sd;
                        } else if mean.abs() > 0.0 {
                            *slot(&mut scale, j)? = mean.abs();
                        }
                    } else {
                        let s = (mean * mean + sd * sd).sqrt();
                        *slot(&mut scale, j)? = if s.is_finite() && s > 0.0 { s } else { 1.0 };
                    }
                }
            }
        }
        let mut x = Vec::with_capacity(n);
        for row in &data.rows {
            let mut xt = Vec::with_capacity(p);
            for j in 0..p {
                let v = at(row, j)?;
                let c = at(&center, j)?;
                let s = at(&scale, j)?;
                if !s.is_finite() || s == 0.0 {
                    return Err(AlgoError::Data("GLE scale for a predictor is zero".into()));
                }
                xt.push((v - c) / s);
            }
            x.push(xt);
        }
        let y_center = if settings.distribution == Distribution::Normal
            && matches!(settings.link, Link::Identity)
            && settings.intercept
        {
            data.y.iter().sum::<f64>() / nf
        } else {
            0.0
        };
        let y: Vec<f64> = data.y.iter().map(|v| v - y_center).collect();
        Ok(Work {
            y,
            x,
            center,
            scale,
            y_center,
            intercept: settings.intercept,
            distribution: settings.distribution,
            link: settings.link,
            binomial: settings.distribution == Distribution::Binomial,
        })
    }
}

fn estimate(
    work: &Work,
    settings: &GleSettings,
) -> Result<(Vec<f64>, usize, f64, bool), AlgoError> {
    let p = work.scale.len();
    let mut beta = initial_beta(work, settings)?;
    if settings.max_iterations == 0 {
        let loglik = log_likelihood(work, &beta).unwrap_or(f64::NEG_INFINITY);
        return Ok((beta, 0, loglik, false));
    }
    let k_fisher = match settings.estimation {
        Estimation::NewtonRaphson => 0,
        Estimation::FisherScoring => settings.max_iterations.saturating_add(1),
        Estimation::Hybrid { fisher_iterations } => fisher_iterations,
    };
    // The first iteration is a Fisher step only when K >= 1 (Newton-only uses the observed
    // Hessian from the start: `gle4/g4_id_d_newton`).
    let Some((mut loglik, mut score, mut xtwx)) = derivatives(work, &beta, k_fisher >= 1) else {
        return Err(AlgoError::Data(
            "GLE starting log-likelihood is not finite".into(),
        ));
    };
    // Hybrid estimation stops one Fisher iteration earlier than the limit: with one Fisher
    // step Modeler's non-converging 3-cycle ends at iterate 99, Newton-only at 100
    // (`gle4/g4_id_d_*`, `gle3/g3_id_d_def`).
    let limit = match settings.estimation {
        Estimation::Hybrid { fisher_iterations } => settings
            .max_iterations
            .saturating_sub(fisher_iterations)
            .max(1),
        _ => settings.max_iterations,
    };
    let mut converged = false;
    let mut iterations = 0usize;
    let mut need_newton_confirm = false;
    let mut force_fisher = false;
    loop {
        if iterations >= limit {
            break;
        }
        iterations = iterations.saturating_add(1);
        let fisher_step = force_fisher || iterations <= k_fisher;
        let Some(delta) = sweep_solve(&xtwx, &score, p, settings.singularity) else {
            break;
        };
        match step_halve(work, &beta, &delta, loglik, settings.max_step_halving) {
            Some((new_beta, _)) => {
                // The Hessian computed here serves the NEXT iteration, which is a Fisher
                // step only while its number is at most K (Guide 3.1.3 step 6).
                let use_fisher = if need_newton_confirm {
                    false
                } else {
                    force_fisher || iterations.saturating_add(1) <= k_fisher
                };
                let Some((ll, s, a)) = derivatives(work, &new_beta, use_fisher) else {
                    break;
                };
                let param_ok = parameter_converged(&beta, &new_beta, settings);
                let like_ok = likelihood_converged(loglik, ll, settings);
                let hess_ok = hessian_converged(&s, &a, ll, p, settings);
                let separated = binomial_separated(work, &new_beta, &a, p);
                beta = new_beta;
                loglik = ll;
                score = s;
                xtwx = a;
                if separated {
                    break;
                }
                if param_ok && like_ok && hess_ok {
                    if matches!(settings.estimation, Estimation::Hybrid { .. })
                        && iterations <= k_fisher
                        && !need_newton_confirm
                        && !force_fisher
                    {
                        need_newton_confirm = true;
                        continue;
                    }
                    converged = true;
                    break;
                }
            }
            None if !fisher_step => {
                let Some((_, s, a)) = derivatives(work, &beta, true) else {
                    break;
                };
                score = s;
                xtwx = a;
                force_fisher = true;
                iterations = iterations.saturating_sub(1);
            }
            None => break,
        }
    }
    Ok((beta, iterations, loglik, converged))
}

fn initial_beta(work: &Work, settings: &GleSettings) -> Result<Vec<f64>, AlgoError> {
    let n = work.y.len();
    let p = work.scale.len();
    let mut xtwx = vec![
        0.0;
        p.checked_mul(p).ok_or_else(|| {
            AlgoError::Data("GLE parameter product overflow".into())
        })?
    ];
    let mut xtwz = vec![0.0; p];
    for i in 0..n {
        let y = at(&work.y, i)?;
        let x = work
            .x
            .get(i)
            .ok_or_else(|| AlgoError::Data("GLE row".into()))?;
        let mu = start_mu(y, work.distribution);
        let eta = g_of(mu, work.link).unwrap_or(1.0);
        let g1 = g1_of(mu, work.link).unwrap_or(0.0);
        let var = variance(mu, work.distribution).unwrap_or(0.0);
        let den = var * g1 * g1;
        let we = if den.is_finite() && den > 0.0 {
            1.0 / den
        } else {
            0.0
        };
        if we == 0.0 || !we.is_finite() {
            continue;
        }
        let z = if work.binomial {
            eta + (y - mu) * g1
        } else {
            eta
        };
        for a in 0..p {
            let xa = at(x, a)?;
            let wz = xtwz
                .get_mut(a)
                .ok_or_else(|| AlgoError::Data("GLE index".into()))?;
            *wz += we * xa * z;
            for b in 0..p {
                let xb = at(x, b)?;
                let idx = a.saturating_mul(p).saturating_add(b);
                let cell = xtwx
                    .get_mut(idx)
                    .ok_or_else(|| AlgoError::Data("GLE index".into()))?;
                *cell += we * xa * xb;
            }
        }
    }
    sweep_solve(&xtwx, &xtwz, p, settings.singularity)
        .ok_or_else(|| AlgoError::Data("GLE starting weighted least squares failed".into()))
}

fn start_mu(y: f64, distribution: Distribution) -> f64 {
    match distribution {
        Distribution::Binomial => (y + 0.5) / 2.0,
        _ => y,
    }
}

/// Guide 3.1.3: quasi-complete separation when every fitted mean is at a
/// boundary (`p*_min < 0.001`) and a non-redundant Hessian diagonal vanishes.
fn binomial_separated(work: &Work, beta: &[f64], xtwx: &[f64], p: usize) -> bool {
    if !work.binomial {
        return false;
    }
    let mut pstar = f64::INFINITY;
    for xrow in &work.x {
        let Some(eta) = dot(xrow, beta) else {
            continue;
        };
        let Some(mu) = mu_of(eta, work.link).and_then(|m| project_mu(m, work)) else {
            continue;
        };
        pstar = pstar.min(mu.min(1.0 - mu).abs());
    }
    if pstar >= 0.001 {
        return false;
    }
    (0..p).any(|i| {
        xtwx.get(i.saturating_mul(p).saturating_add(i))
            .copied()
            .unwrap_or(0.0)
            .abs()
            == 0.0
    })
}

fn step_halve(
    work: &Work,
    beta: &[f64],
    delta: &[f64],
    loglik: f64,
    max_halves: usize,
) -> Option<(Vec<f64>, f64)> {
    let mut xi = 1.0;
    let mut undefined_full = None;
    for j in 0..=max_halves {
        let mut trial = Vec::with_capacity(beta.len());
        for (b, d) in beta.iter().zip(delta) {
            trial.push(b + xi * d);
        }
        if trial.iter().all(|v| v.is_finite()) {
            match log_likelihood(work, &trial) {
                // An undefined likelihood on either side compares false, so Modeler
                // never halves: the full step stands (`gle3/g3_id_d`).
                Some(ll) if j == 0 && (ll.is_nan() || loglik.is_nan()) => {
                    return Some((trial, ll));
                }
                Some(ll) if ll.is_finite() && ll >= loglik - 1e-14 * (1.0 + loglik.abs()) => {
                    return Some((trial, ll));
                }
                None if j == 0 => {
                    // Identity/log binomial can leave (0, 1); the kernel is then
                    // undefined. IEEE comparisons with NaN do not trigger
                    // step-halving, so the full step is kept.
                    undefined_full = Some(trial);
                }
                _ => {}
            }
        }
        xi *= 0.5;
    }
    undefined_full.map(|trial| (trial, loglik))
}

fn parameter_converged(old: &[f64], new: &[f64], settings: &GleSettings) -> bool {
    let mut max_change = 0.0;
    for (a, b) in old.iter().zip(new) {
        let diff = (a - b).abs();
        let value = if settings.parameter_relative {
            diff / (a.abs() + REL_EPS)
        } else {
            diff
        };
        if value > max_change {
            max_change = value;
        }
    }
    max_change < settings.parameter_convergence
}

fn likelihood_converged(old: f64, new: f64, settings: &GleSettings) -> bool {
    let Some(eps) = settings.likelihood_convergence else {
        return true;
    };
    let diff = (new - old).abs();
    if settings.likelihood_relative {
        diff / (old.abs() + REL_EPS) < eps
    } else {
        diff < eps
    }
}

fn hessian_converged(
    score: &[f64],
    xtwx: &[f64],
    loglik: f64,
    p: usize,
    settings: &GleSettings,
) -> bool {
    let Some(delta) = sweep_solve(xtwx, score, p, settings.singularity) else {
        return false;
    };
    let mut quad = 0.0;
    for (s, d) in score.iter().zip(&delta) {
        quad += s * d;
    }
    let value = quad.abs();
    if settings.hessian_relative {
        value / (loglik.abs() + REL_EPS) < settings.hessian_convergence
    } else {
        value < settings.hessian_convergence
    }
}

fn derivatives(work: &Work, beta: &[f64], fisher: bool) -> Option<(f64, Vec<f64>, Vec<f64>)> {
    let n = work.y.len();
    let p = beta.len();
    let len = p.checked_mul(p)?;
    let mut loglik = 0.0;
    let mut score = vec![0.0; p];
    let mut xtwx = vec![0.0; len];
    let mut undefined = false;
    for i in 0..n {
        let y = *work.y.get(i)?;
        let x = work.x.get(i)?;
        let eta = dot(x, beta)?;
        let raw = mu_of(eta, work.link)?;
        if excluded(raw, work) {
            undefined |= undefined_likelihood(raw, work);
            continue;
        }
        let mu = project_mu(raw, work)?;
        let ll = kernel(y, mu, work.distribution)?;
        loglik += ll;
        let Some(g1) = g1_of(mu, work.link) else {
            continue;
        };
        let Some(var) = variance(mu, work.distribution) else {
            continue;
        };
        if !g1.is_finite() || !var.is_finite() || var == 0.0 {
            continue;
        }
        let v = (y - mu) / (var * g1);
        if !v.is_finite() {
            continue;
        }
        let we = 1.0 / (var * g1 * g1);
        let w = if fisher {
            we
        } else {
            match (g2_of(mu, work.link), vprime(mu, work.distribution)) {
                (Some(g2), Some(vp)) => {
                    let extra = (y - mu) * (vp * g1 + var * g2) / (var * var * g1 * g1 * g1);
                    let newton = we + extra;
                    if newton.is_finite() { newton } else { we }
                }
                _ => we,
            }
        };
        if !w.is_finite() {
            continue;
        }
        for a in 0..p {
            let xa = *x.get(a)?;
            *score.get_mut(a)? += v * xa;
            for b in 0..p {
                let xb = *x.get(b)?;
                *xtwx.get_mut(a.saturating_mul(p).saturating_add(b))? += w * xa * xb;
            }
        }
    }
    if undefined {
        loglik = f64::NAN;
    } else if !loglik.is_finite() {
        return None;
    }
    Some((loglik, score, xtwx))
}

fn log_likelihood(work: &Work, beta: &[f64]) -> Option<f64> {
    let mut loglik = 0.0;
    for (y, x) in work.y.iter().zip(&work.x) {
        let eta = dot(x, beta)?;
        let raw = mu_of(eta, work.link)?;
        if excluded(raw, work) {
            if undefined_likelihood(raw, work) {
                return Some(f64::NAN);
            }
            continue;
        }
        let mu = project_mu(raw, work)?;
        loglik += kernel(*y, mu, work.distribution)?;
    }
    if loglik.is_finite() {
        Some(loglik)
    } else {
        None
    }
}

fn dot(x: &[f64], beta: &[f64]) -> Option<f64> {
    if x.len() != beta.len() {
        return None;
    }
    let mut s = 0.0;
    for (a, b) in x.iter().zip(beta) {
        s += a * b;
    }
    if s.is_finite() { Some(s) } else { None }
}

/// A binomial record whose mean leaves (0, 1) under an identity or log link drops out of the
/// likelihood, score and Hessian for that iterate (Modeler's estimates are roots of the score
/// over the remaining records: `gle3/g3_id_b_*` exactly, `gle3/g3_log_a_it1` to 1e-6;
/// `gle-lab/NOTES.md`). The logit link cannot leave the range.
fn excluded(mu: f64, work: &Work) -> bool {
    work.distribution == Distribution::Binomial
        && !matches!(work.link, Link::Logit)
        && !(mu > 0.0 && mu < 1.0)
}

/// An excluded binomial mean at or below 0 makes the log-likelihood undefined (`ln mu`), so no
/// step-halving comparison can be made and the full step is taken. A mean at or above 1 is only
/// excluded. This asymmetry separates Modeler's converged `gle3/g3_id_b` (means above 1) from
/// its 3-cycle `gle3/g3_id_d` (means below 0); `gle-lab/NOTES.md`, rounds 3-5.
fn undefined_likelihood(mu: f64, work: &Work) -> bool {
    excluded(mu, work) && (mu <= 0.0 || mu.is_nan())
}

fn project_mu(mu: f64, work: &Work) -> Option<f64> {
    let mu = match work.distribution {
        Distribution::Binomial => mu,
        Distribution::Poisson | Distribution::Gamma => {
            if mu <= 0.0 {
                MU_FLOOR
            } else {
                mu
            }
        }
        Distribution::Normal => mu,
    };
    if mu.is_finite() { Some(mu) } else { None }
}

fn mu_of(eta: f64, link: Link) -> Option<f64> {
    let mu = match link {
        Link::Identity => eta,
        Link::Log => {
            if eta > ETA_CAP {
                ETA_CAP.exp()
            } else if eta < -ETA_CAP {
                (-ETA_CAP).exp()
            } else {
                eta.exp()
            }
        }
        Link::Logit => {
            let z = eta.clamp(-ETA_CAP, ETA_CAP);
            1.0 / (1.0 + (-z).exp())
        }
        Link::Power { alpha } => {
            if (alpha).abs() < 2.2e-16 {
                eta.exp()
            } else if eta == 0.0 {
                return None;
            } else {
                eta.powf(1.0 / alpha)
            }
        }
    };
    if mu.is_finite() { Some(mu) } else { None }
}

fn g_of(mu: f64, link: Link) -> Option<f64> {
    let eta = match link {
        Link::Identity => mu,
        Link::Log => {
            if mu <= 0.0 {
                return None;
            }
            mu.ln()
        }
        Link::Logit => {
            if mu <= 0.0 || mu >= 1.0 {
                return None;
            }
            (mu / (1.0 - mu)).ln()
        }
        Link::Power { alpha } => {
            if alpha.abs() < 2.2e-16 {
                if mu <= 0.0 {
                    return None;
                }
                mu.ln()
            } else {
                if mu <= 0.0 {
                    return None;
                }
                mu.powf(alpha)
            }
        }
    };
    if eta.is_finite() { Some(eta) } else { None }
}

fn g1_of(mu: f64, link: Link) -> Option<f64> {
    let d = match link {
        Link::Identity => 1.0,
        Link::Log => {
            if mu.abs() < MU_FLOOR {
                return None;
            }
            1.0 / mu
        }
        Link::Logit => {
            let den = mu * (1.0 - mu);
            if den.abs() < MU_FLOOR {
                return None;
            }
            1.0 / den
        }
        Link::Power { alpha } => {
            if alpha.abs() < 2.2e-16 {
                if mu.abs() < MU_FLOOR {
                    return None;
                }
                1.0 / mu
            } else {
                alpha * mu.powf(alpha - 1.0)
            }
        }
    };
    if d.is_finite() { Some(d) } else { None }
}

fn g2_of(mu: f64, link: Link) -> Option<f64> {
    let d = match link {
        Link::Identity => 0.0,
        Link::Log => {
            if mu.abs() < MU_FLOOR {
                return None;
            }
            -1.0 / (mu * mu)
        }
        Link::Logit => {
            let den = mu * mu * (1.0 - mu) * (1.0 - mu);
            if den.abs() < MU_FLOOR {
                return None;
            }
            (2.0 * mu - 1.0) / den
        }
        Link::Power { alpha } => {
            if alpha.abs() < 2.2e-16 {
                if mu.abs() < MU_FLOOR {
                    return None;
                }
                -1.0 / (mu * mu)
            } else {
                alpha * (alpha - 1.0) * mu.powf(alpha - 2.0)
            }
        }
    };
    if d.is_finite() { Some(d) } else { None }
}

fn variance(mu: f64, distribution: Distribution) -> Option<f64> {
    let v = match distribution {
        Distribution::Normal => 1.0,
        Distribution::Poisson => mu.max(MU_FLOOR),
        Distribution::Gamma => mu * mu,
        Distribution::Binomial => {
            let p = mu.clamp(MU_FLOOR, 1.0 - MU_FLOOR);
            p * (1.0 - p)
        }
    };
    if v.is_finite() && v > 0.0 {
        Some(v)
    } else {
        None
    }
}

fn vprime(mu: f64, distribution: Distribution) -> Option<f64> {
    let d = match distribution {
        Distribution::Normal => 0.0,
        Distribution::Poisson => 1.0,
        Distribution::Gamma => 2.0 * mu,
        Distribution::Binomial => 1.0 - 2.0 * mu,
    };
    if d.is_finite() { Some(d) } else { None }
}

fn kernel(y: f64, mu: f64, distribution: Distribution) -> Option<f64> {
    let v = match distribution {
        Distribution::Normal => {
            let e = y - mu;
            -0.5 * e * e
        }
        Distribution::Poisson => {
            let m = mu.max(MU_FLOOR);
            if y == 0.0 { -m } else { y * m.ln() - m }
        }
        Distribution::Gamma => {
            let m = mu.max(MU_FLOOR);
            -y / m - m.ln()
        }
        Distribution::Binomial => {
            let p = mu.clamp(MU_FLOOR, 1.0 - MU_FLOOR);
            y * p.ln() + (1.0 - y) * (1.0 - p).ln()
        }
    };
    if v.is_finite() { Some(v) } else { None }
}

fn sweep_solve(matrix: &[f64], rhs: &[f64], n: usize, tol: f64) -> Option<Vec<f64>> {
    let len = n.checked_mul(n)?;
    if matrix.len() < len || rhs.len() < n || n == 0 {
        return None;
    }
    let mut a = matrix[..len].to_vec();
    let original: Vec<f64> = (0..n)
        .map(|k| {
            a.get(k.saturating_mul(n).saturating_add(k))
                .copied()
                .unwrap_or(0.0)
        })
        .collect();
    let mut swept = vec![false; n];
    for k in 0..n {
        let pivot = *a.get(k.saturating_mul(n).saturating_add(k))?;
        let orig = original.get(k).copied().unwrap_or(0.0);
        if !pivot.is_finite() || pivot <= tol * orig.abs() || pivot <= 0.0 {
            continue;
        }
        *swept.get_mut(k)? = true;
        let row: Vec<f64> = (0..n)
            .filter_map(|j| a.get(k.saturating_mul(n).saturating_add(j)).copied())
            .collect();
        if row.len() != n {
            return None;
        }
        for i in (0..n).filter(|&i| i != k) {
            let factor = *a.get(i.saturating_mul(n).saturating_add(k))? / pivot;
            for j in (0..n).filter(|&j| j != k) {
                let idx = i.saturating_mul(n).saturating_add(j);
                *a.get_mut(idx)? -= factor * row.get(j)?;
            }
        }
        for j in (0..n).filter(|&j| j != k) {
            *a.get_mut(k.saturating_mul(n).saturating_add(j))? = row.get(j)? / pivot;
            *a.get_mut(j.saturating_mul(n).saturating_add(k))? /= pivot;
        }
        *a.get_mut(k.saturating_mul(n).saturating_add(k))? = -1.0 / pivot;
    }
    let mut x = vec![0.0; n];
    for i in (0..n).filter(|&i| swept.get(i) == Some(&true)) {
        let mut s = 0.0;
        for j in (0..n).filter(|&j| swept.get(j) == Some(&true)) {
            s += -a.get(i.saturating_mul(n).saturating_add(j))? * rhs.get(j)?;
        }
        *x.get_mut(i)? = s;
    }
    if x.iter().all(|v| v.is_finite()) {
        Some(x)
    } else {
        None
    }
}

fn back_transform(work: &Work, beta_t: &[f64]) -> Vec<f64> {
    let p = beta_t.len();
    let mut orig = vec![0.0; p];
    for j in 0..p {
        let bt = beta_t.get(j).copied().unwrap_or(0.0);
        let s = work.scale.get(j).copied().unwrap_or(1.0);
        let value = if s.abs() > 0.0 { bt / s } else { 0.0 };
        if let Some(slot) = orig.get_mut(j) {
            *slot = value;
        }
    }
    if work.intercept
        && let Some(intercept) = orig.first_mut()
    {
        let mut adj = 0.0;
        for j in 1..p {
            let bt = beta_t.get(j).copied().unwrap_or(0.0);
            let c = work.center.get(j).copied().unwrap_or(0.0);
            let s = work.scale.get(j).copied().unwrap_or(1.0);
            if s.abs() > 0.0 {
                adj += bt * c / s;
            }
        }
        *intercept = beta_t.first().copied().unwrap_or(0.0) - adj + work.y_center;
    }
    orig
}

fn to_pmml(
    dataset: &Dataset<'_>,
    data: &Data,
    settings: &GleSettings,
    beta: &[f64],
) -> Result<Pmml, AlgoError> {
    let table = dataset.table;
    let mut dictionary = Vec::new();
    let mut schema = Vec::new();
    if let Some(field) = data_field(table, dataset.target) {
        dictionary.push(field);
    }
    if let Some(field) = mining_field(table, dataset.target, Usage::Target) {
        schema.push(field);
    }
    for &position in dataset.inputs {
        if let Some(field) = data_field(table, position) {
            dictionary.push(field);
        }
        if let Some(field) = mining_field(table, position, Usage::Active) {
            schema.push(field);
        }
    }
    let parameters: Vec<String> = data
        .columns
        .iter()
        .map(|c| c.parameter_name.clone())
        .collect();
    let mut pp_matrix = Vec::new();
    for column in &data.columns {
        match &column.kind {
            ColumnKind::Covariate { name, .. } => pp_matrix.push(PPCell {
                value: "1".into(),
                predictor: name.clone(),
                parameter: column.parameter_name.clone(),
            }),
            ColumnKind::Indicator { name, category, .. } => pp_matrix.push(PPCell {
                value: category.clone(),
                predictor: name.clone(),
                parameter: column.parameter_name.clone(),
            }),
            ColumnKind::Intercept => {}
        }
    }
    let classification = settings.distribution == Distribution::Binomial;
    let mut param_matrix = Vec::new();
    for (column, value) in data.columns.iter().zip(beta) {
        param_matrix.push(PCell {
            target_category: if classification {
                data.success.clone()
            } else {
                None
            },
            parameter: column.parameter_name.clone(),
            beta: *value,
        });
    }
    let model_type = if classification && matches!(settings.link, Link::Logit) {
        GeneralModelType::MultinomialLogistic
    } else {
        GeneralModelType::GeneralizedLinear
    };
    Ok(Pmml {
        version: "4.3".into(),
        data_dictionary: dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::GeneralRegression(GeneralRegressionModel {
            header: ModelHeader {
                name: settings.name.clone(),
                function: if classification {
                    MiningFunction::Classification
                } else {
                    MiningFunction::Regression
                },
                algorithm: Some("GZLM".into()),
                mining_schema: schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            model_type,
            target_reference_category: data.reference.clone(),
            link_function: Some(settings.link.name().into()),
            link_parameter: settings.link.parameter(),
            parameters,
            factors: data.factors.clone(),
            covariates: data.covariates.clone(),
            pp_matrix,
            param_matrix,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Dataset;
    use dms_types::{DataModel, Field, Role, Table, ValueSpec};

    fn f64_close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * (1.0 + a.abs().max(b.abs()))
    }

    fn numeric(name: &str, role: Role) -> Field {
        let mut field = Field::new(name, Storage::Real);
        field.measurement = Measurement::Continuous;
        field.role = role;
        field
    }

    fn integer(name: &str, role: Role) -> Field {
        let mut field = Field::new(name, Storage::Integer);
        field.measurement = Measurement::Continuous;
        field.role = role;
        field
    }

    fn flag(name: &str) -> Field {
        let mut field = Field::new(name, Storage::String);
        field.measurement = Measurement::Flag;
        field.role = Role::Target;
        field.values = ValueSpec::Flag {
            true_value: Value::string("T"),
            false_value: Value::string("F"),
        };
        field
    }

    fn set_field(name: &str) -> Field {
        let mut field = Field::new(name, Storage::String);
        field.measurement = Measurement::Nominal;
        field.role = Role::Input;
        field.values = ValueSpec::Categories(vec![
            Value::string("A"),
            Value::string("B"),
            Value::string("C"),
        ]);
        field
    }

    fn table(fields: Vec<Field>, rows: Vec<Vec<Value>>) -> Table {
        Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
    }

    fn dataset<'a>(table: &'a Table, target: usize, inputs: &'a [usize]) -> Dataset<'a> {
        Dataset {
            table,
            target,
            inputs,
            frequency: None,
        }
    }

    #[test]
    fn normal_identity_recovers_exact_line() {
        let records: Vec<Vec<Value>> = (1..=4)
            .map(|i| vec![Value::Real(i as f64), Value::Real(1.0 + 2.0 * i as f64)])
            .collect();
        let table = table(
            vec![numeric("x", Role::Input), numeric("y", Role::Target)],
            records,
        );
        let inputs = [0usize];
        let fit = fit(&dataset(&table, 1, &inputs), &GleSettings::default()).unwrap();
        assert!(f64_close(fit.coefficients[0], 1.0, 1e-9));
        assert!(f64_close(fit.coefficients[1], 2.0, 1e-9));
        assert!(fit.converged);
    }

    #[test]
    fn normal_identity_without_intercept() {
        let records: Vec<Vec<Value>> = (1..=4)
            .map(|i| vec![Value::Real(i as f64), Value::Real(3.0 * i as f64)])
            .collect();
        let table = table(
            vec![numeric("x", Role::Input), numeric("y", Role::Target)],
            records,
        );
        let inputs = [0usize];
        let settings = GleSettings {
            intercept: false,
            ..GleSettings::default()
        };
        let fit = fit(&dataset(&table, 1, &inputs), &settings).unwrap();
        assert_eq!(fit.coefficients.len(), 1);
        assert!(f64_close(fit.coefficients[0], 3.0, 1e-9));
    }

    #[test]
    fn poisson_log_intercept_only_is_log_mean() {
        let records = vec![
            vec![Value::Integer(1)],
            vec![Value::Integer(2)],
            vec![Value::Integer(3)],
        ];
        let table = table(vec![integer("y", Role::Target)], records);
        let inputs: [usize; 0] = [];
        let settings = GleSettings {
            distribution: Distribution::Poisson,
            link: Link::Log,
            ..GleSettings::default()
        };
        let fit = fit(&dataset(&table, 0, &inputs), &settings).unwrap();
        assert!(f64_close(fit.coefficients[0], 2.0f64.ln(), 1e-8));
    }

    #[test]
    fn binomial_logit_balanced_intercept_is_zero() {
        let records = vec![
            vec![Value::string("T")],
            vec![Value::string("T")],
            vec![Value::string("F")],
            vec![Value::string("F")],
        ];
        let table = table(vec![flag("y")], records);
        let inputs: [usize; 0] = [];
        let settings = GleSettings {
            distribution: Distribution::Binomial,
            link: Link::Logit,
            ..GleSettings::default()
        };
        let fit = fit(&dataset(&table, 0, &inputs), &settings).unwrap();
        // Last category T is the reference, first F is success: two F and two T → p=0.5.
        assert!(f64_close(fit.coefficients[0], 0.0, 1e-8));
    }

    #[test]
    fn gamma_log_intercept_only_is_log_mean() {
        let records = vec![
            vec![Value::Real(1.0)],
            vec![Value::Real(2.0)],
            vec![Value::Real(3.0)],
        ];
        let table = table(vec![numeric("y", Role::Target)], records);
        let inputs: [usize; 0] = [];
        let settings = GleSettings {
            distribution: Distribution::Gamma,
            link: Link::Log,
            ..GleSettings::default()
        };
        let fit = fit(&dataset(&table, 0, &inputs), &settings).unwrap();
        assert!(f64_close(fit.coefficients[0], 2.0f64.ln(), 1e-6));
    }

    #[test]
    fn factor_plus_covariate_matches_overparameterised_ols() {
        // y = 10 + x + 2 I_B + 4 I_C; last dummy C is aliased with the intercept.
        let mut rows = Vec::new();
        for (g, x, add) in [
            ("A", 1.0, 0.0),
            ("A", 2.0, 0.0),
            ("B", 3.0, 2.0),
            ("B", 4.0, 2.0),
            ("C", 5.0, 4.0),
            ("C", 6.0, 4.0),
        ] {
            rows.push(vec![
                Value::Real(x),
                Value::string(g),
                Value::Real(10.0 + x + add),
            ]);
        }
        let table = table(
            vec![
                numeric("x", Role::Input),
                set_field("g"),
                numeric("y", Role::Target),
            ],
            rows,
        );
        let inputs = [0usize, 1];
        let fit = fit(&dataset(&table, 2, &inputs), &GleSettings::default()).unwrap();
        assert_eq!(fit.parameter_names.len(), 5);
        assert!(
            f64_close(fit.coefficients[0], 14.0, 1e-8),
            "{:?}",
            fit.coefficients
        );
        assert!(f64_close(fit.coefficients[1], 1.0, 1e-8));
        assert!(f64_close(fit.coefficients[2], -4.0, 1e-8));
        assert!(f64_close(fit.coefficients[3], -2.0, 1e-8));
        assert!(f64_close(fit.coefficients[4], 0.0, 1e-8));
    }

    #[test]
    fn missing_target_or_input_is_dropped() {
        let records = vec![
            vec![Value::Real(1.0), Value::Real(3.0)],
            vec![Value::Null, Value::Real(5.0)],
            vec![Value::Real(2.0), Value::Null],
            vec![Value::Real(3.0), Value::Real(7.0)],
        ];
        let table = table(
            vec![numeric("x", Role::Input), numeric("y", Role::Target)],
            records,
        );
        let inputs = [0usize];
        let fit = fit(&dataset(&table, 1, &inputs), &GleSettings::default()).unwrap();
        // Two complete points (1,3) and (3,7): y = 1 + 2x.
        assert!(f64_close(fit.coefficients[0], 1.0, 1e-8));
        assert!(f64_close(fit.coefficients[1], 2.0, 1e-8));
    }

    #[test]
    fn poisson_rejects_negative_and_keeps_integers() {
        let records = vec![
            vec![Value::Integer(1)],
            vec![Value::Integer(-1)],
            vec![Value::Real(2.4)],
            vec![Value::Integer(3)],
        ];
        let table = table(vec![integer("y", Role::Target)], records);
        let inputs: [usize; 0] = [];
        let settings = GleSettings {
            distribution: Distribution::Poisson,
            link: Link::Log,
            ..GleSettings::default()
        };
        let fit = fit(&dataset(&table, 0, &inputs), &settings).unwrap();
        let mean = 2.0f64;
        assert!(f64_close(fit.coefficients[0], mean.ln(), 1e-8));
    }

    #[test]
    fn binomial_three_levels_is_an_error() {
        let mut y = Field::new("y", Storage::String);
        y.measurement = Measurement::Nominal;
        y.role = Role::Target;
        y.values = ValueSpec::Categories(vec![
            Value::string("a"),
            Value::string("b"),
            Value::string("c"),
        ]);
        let table = table(
            vec![y],
            vec![
                vec![Value::string("a")],
                vec![Value::string("b")],
                vec![Value::string("c")],
            ],
        );
        let inputs: [usize; 0] = [];
        let settings = GleSettings {
            distribution: Distribution::Binomial,
            link: Link::Logit,
            ..GleSettings::default()
        };
        let err = fit(&dataset(&table, 0, &inputs), &settings).unwrap_err();
        assert!(format!("{err}").contains("2 levels"));
    }

    #[test]
    fn unsupported_link_is_a_gap() {
        let table = table(
            vec![numeric("y", Role::Target)],
            vec![vec![Value::Real(1.0)], vec![Value::Real(2.0)]],
        );
        let inputs: [usize; 0] = [];
        let settings = GleSettings {
            distribution: Distribution::Normal,
            link: Link::Logit,
            ..GleSettings::default()
        };
        let err = fit(&dataset(&table, 0, &inputs), &settings).unwrap_err();
        assert!(format!("{err}").contains("GAP"));
    }

    #[test]
    fn poisson_two_parameter_score_vanishes() {
        let records: Vec<Vec<Value>> = (0..12)
            .map(|i| {
                let x = i as f64;
                let mu = (0.5 + 0.2 * x).exp();
                vec![Value::Real(x), Value::Integer(mu.round() as i64)]
            })
            .collect();
        let table = table(
            vec![numeric("x", Role::Input), integer("y", Role::Target)],
            records,
        );
        let inputs = [0usize];
        let settings = GleSettings {
            distribution: Distribution::Poisson,
            link: Link::Log,
            ..GleSettings::default()
        };
        let data = Data::read(&dataset(&table, 1, &inputs), &settings).unwrap();
        let work = Work::from_data(&data, &settings).unwrap();
        let fit = fit(&dataset(&table, 1, &inputs), &settings).unwrap();
        let beta_t = {
            // Reconstruct transformed β from original via the inverse of back_transform
            // by re-estimating on the work matrix: score at the fitted original-scale
            // coefficients mapped back. Use the stored transformed path: fit again
            // internally through estimate.
            let (bt, _, _, _) = estimate(&work, &settings).unwrap();
            bt
        };
        let (ll, score, _) = derivatives(&work, &beta_t, false).unwrap();
        assert!(ll.is_finite());
        let max = score.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
        assert!(max < 1e-5, "score {score:?} fit {:?}", fit.coefficients);
    }

    fn noise(seed: u64) -> f64 {
        let state = 1664525u64.wrapping_mul(seed).wrapping_add(1013904223) % 4_294_967_296;
        12.0f64.sqrt() * ((state as f64 + 0.5) / 4_294_967_296.0 - 0.5)
    }

    pub(crate) fn probe_rows() -> Vec<Vec<Value>> {
        let mut rows = Vec::new();
        for i in 0..24 {
            let x = 0.5 + 0.5 * i as f64;
            let gi = i % 3;
            let g = ["A", "B", "C"][gi];
            let e = noise(1000 + i as u64);
            let g_n = [0.0, 1.5, -1.0][gi];
            let g_g = [0.0, 0.2, -0.15][gi];
            let g_p = [0.0, 0.3, -0.2][gi];
            let g_b = [0.0, 1.0, -0.5][gi];
            let y_normal = 4.0 + 0.8 * x + g_n + 0.15 * e;
            let y_gamma = (0.4 + 0.08 * x + g_g + 0.05 * e).exp();
            let lam = (0.2 + 0.12 * x + g_p).exp();
            let y_poisson = (lam + 0.5).floor() as i64;
            let logit = -2.0 + 0.25 * x + g_b;
            let y_flag = if logit > 0.0 { "T" } else { "F" };
            let xv = if i == 5 || i == 13 {
                Value::Null
            } else {
                Value::Real(x)
            };
            let gv = if i == 15 {
                Value::Null
            } else {
                Value::string(g)
            };
            let yn = if i == 7 {
                Value::Null
            } else {
                Value::Real(y_normal)
            };
            let yg = if i == 8 {
                Value::Null
            } else {
                Value::Real(y_gamma)
            };
            let yp = if i == 9 {
                Value::Null
            } else {
                Value::Integer(y_poisson)
            };
            let yf = if i == 10 {
                Value::Null
            } else {
                Value::string(y_flag)
            };
            rows.push(vec![Value::Integer(i as i64 + 1), xv, gv, yn, yg, yp, yf]);
        }
        rows
    }

    fn probe_table() -> Table {
        let mut id = Field::new("id", Storage::Integer);
        id.measurement = Measurement::Continuous;
        id.role = Role::None;
        table(
            vec![
                id,
                numeric("x", Role::Input),
                set_field("g"),
                numeric("y_normal", Role::Target),
                numeric("y_gamma", Role::None),
                integer("y_poisson", Role::None),
                {
                    let mut f = flag("y_flag");
                    f.role = Role::None;
                    f
                },
            ],
            probe_rows(),
        )
    }

    fn retarget(table: Table, target: &str, inputs: &[&str]) -> Table {
        let (model, rows) = table.into_parts();
        let mut fields = model.fields().to_vec();
        for field in &mut fields {
            if field.name == target {
                field.role = Role::Target;
            } else if inputs.contains(&field.name.as_str()) {
                field.role = Role::Input;
            } else {
                field.role = Role::None;
            }
        }
        Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
    }

    #[test]
    fn probe_case_fits_are_finite() {
        let specs: &[(&str, usize, &[usize], GleSettings)] = &[
            ("gle_normal_identity", 3, &[1], GleSettings::default()),
            (
                "gle_normal_log",
                3,
                &[1],
                GleSettings {
                    link: Link::Log,
                    ..GleSettings::default()
                },
            ),
            (
                "gle_normal_noint",
                3,
                &[1],
                GleSettings {
                    intercept: false,
                    ..GleSettings::default()
                },
            ),
            ("gle_normal_factor", 3, &[1, 2], GleSettings::default()),
            (
                "gle_poisson_log",
                5,
                &[1],
                GleSettings {
                    distribution: Distribution::Poisson,
                    link: Link::Log,
                    ..GleSettings::default()
                },
            ),
            (
                "gle_poisson_identity",
                5,
                &[1],
                GleSettings {
                    distribution: Distribution::Poisson,
                    link: Link::Identity,
                    ..GleSettings::default()
                },
            ),
            (
                "gle_poisson_noint",
                5,
                &[1],
                GleSettings {
                    distribution: Distribution::Poisson,
                    link: Link::Log,
                    intercept: false,
                    ..GleSettings::default()
                },
            ),
            (
                "gle_gamma_log",
                4,
                &[1],
                GleSettings {
                    distribution: Distribution::Gamma,
                    link: Link::Log,
                    ..GleSettings::default()
                },
            ),
            (
                "gle_gamma_identity",
                4,
                &[1],
                GleSettings {
                    distribution: Distribution::Gamma,
                    link: Link::Identity,
                    ..GleSettings::default()
                },
            ),
            (
                "gle_gamma_inverse",
                4,
                &[1],
                GleSettings {
                    distribution: Distribution::Gamma,
                    link: Link::Power { alpha: -1.0 },
                    ..GleSettings::default()
                },
            ),
            (
                "gle_binom_logit",
                6,
                &[1],
                GleSettings {
                    distribution: Distribution::Binomial,
                    link: Link::Logit,
                    ..GleSettings::default()
                },
            ),
            (
                "gle_binom_identity",
                6,
                &[1],
                GleSettings {
                    distribution: Distribution::Binomial,
                    link: Link::Identity,
                    ..GleSettings::default()
                },
            ),
            (
                "gle_binom_log",
                6,
                &[1],
                GleSettings {
                    distribution: Distribution::Binomial,
                    link: Link::Log,
                    ..GleSettings::default()
                },
            ),
            (
                "gle_binom_factor",
                6,
                &[1, 2],
                GleSettings {
                    distribution: Distribution::Binomial,
                    link: Link::Logit,
                    ..GleSettings::default()
                },
            ),
        ];
        let base = probe_table();
        for (name, target, inputs, settings) in specs {
            let names: Vec<&str> = base
                .model()
                .fields()
                .iter()
                .map(|f| f.name.as_str())
                .collect();
            let target_name = names[*target];
            let input_names: Vec<&str> = inputs.iter().map(|i| names[*i]).collect();
            let table = retarget(base.clone(), target_name, &input_names);
            let fit = fit(&dataset(&table, *target, inputs), settings)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(
                fit.coefficients.iter().all(|c| c.is_finite()),
                "{name} {:?}",
                fit.coefficients
            );
            let expected: &[f64] = match *name {
                "gle_normal_identity" => &[4.289936749693884, 0.7802265448428849],
                "gle_normal_log" => &[1.6431350434743892, 0.08457280955758528],
                "gle_normal_noint" => &[1.2871174205669236],
                "gle_normal_factor" => &[
                    3.0640336659934437,
                    0.8004027562966295,
                    1.0000000000000007,
                    2.5,
                    0.0,
                ],
                "gle_poisson_log" => &[0.29520863875648884, 0.11651209749261668],
                "gle_poisson_identity" => &[1.0222111117036419, 0.3224708641053594],
                "gle_poisson_noint" => &[0.14800317496399268],
                "gle_gamma_log" => &[0.49038464756711553, 0.07458560231833393],
                "gle_gamma_identity" => &[1.51367977268204, 0.18906477595138121],
                "gle_gamma_inverse" => &[0.5710022328477902, -0.027836844753388417],
                "gle_binom_logit" => &[22.429759271389315, -2.565075472490318],
                // Ours, not Modeler's: these two remain gaps (xtask/known-gaps.tsv).
                "gle_binom_identity" => &[-0.055134788184932226, 0.0636235591374231],
                "gle_binom_log" => &[39.119061036382845, -4.115935218805453],
                "gle_binom_factor" => &[
                    126.13034812751488,
                    -12.938444031206005,
                    -28.94161867161248,
                    -45.265088691668986,
                    0.0,
                ],
                _ => panic!("no frozen coefficients for {name}"),
            };
            assert_eq!(
                fit.coefficients.len(),
                expected.len(),
                "{name} {:?}",
                fit.coefficients
            );
            for (got, want) in fit.coefficients.iter().zip(expected) {
                assert!(
                    f64_close(*got, *want, 1e-10),
                    "{name}: {got} vs {want} ({:?})",
                    fit.coefficients
                );
            }
        }
    }
}
