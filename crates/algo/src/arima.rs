//! Phase B (`ts_more`): d=0 starts at the stationary mean; seasonal factors
//! multiply lag polynomials, and both differencing polynomials are inverted for
//! history/forecasts. Covariances solve the finite Yule-Walker equations with
//! MA forcing, avoiding truncated impulse sums near unit roots. Sqrt forecasts
//! use `E[Z^2] = E[Z]^2 + Var[Z]`; log uses the lognormal mean. Converged fits of
//! ts_ar/ma/arima/airline and their no-constant airline counterpart pass rule 3;
//! saved log/sqrt models pass when refitted with their recorded transformation.
//! The no-constant seasonal log case's linked, untransformed rebuild still
//! differs by 0.116627 target SD and remains GAP, not accepted optimizer parity.
//! Public source: Modeler Algorithms Guide (1994–2016), Time Series pp.384–386.
//!
//! ARIMA scoring by the innovations algorithm (Brockwell & Davis; Algorithms Guide
//! "Maximum Likelihood (ML) Method").
//!
//! SPSS Time Series PMML (`ts_arima` sales): `<ARIMA transformation="none"
//! constantTerm="c">` with `<NonseasonalComponent p="1" d="1" q="1">`, MA stored
//! with the opposite sign of Brockwell & Davis (`+0.0254` → θ = `-0.0254`,
//! `FinalTheta` `-0.0254`). One-step predictions of `w_t = Δ^d y_t − c` use
//! innovations; t=2 is `y1 + c` (21.076748420698827); t=3 is 22.757645116819514.
//!
//! Intervals: `prediction ± t_{1-α/2, df} √(σ² r_n)` with `df` =
//! `degreesOfFreedom` (32) and `σ²` = `errVariance`. t=2: 18.531075468525607 ..
//! 23.622421372872047. Forecast rows use psi weights of the integrated model.
//!
//! Logarithmic transform (`ts_expert`): work on `ln y`, then the point is
//! `exp(μ + se²/2)` (lognormal mean) and the limits are `exp(μ ± t se)`
//! (no bias in the interval centre).
//!
//! Forecast rows (`ts_expert`, `ts_expert2`): future means and variances use
//! finite-history Gaussian conditioning from the innovations recursion extended
//! across the forecast horizon. For `pr_seastr_ar`, `pr_air_ar` and
//! `p2_seastr_ar`, first-step `Var/sigma2` is `v_n > 1` (e.g. `1.03731293244`
//! for `pr_seastr_ar` and `1.00000766665` for `pr_air_ar`), reproducing all
//! saved future points and bounds (including the lognormal mean `exp(mu + Var/2)`).
//! White noise with a constant and d=2 scoring are covered by the Expert corpus
//! regression in `dms-exec`; an omitted nonseasonal PMML component means d=p=q=0.

use crate::stats::student_t_quantile;
use std::fmt;

pub use crate::expert::expert_arima;

const MAX_HISTORY: usize = 100_000;
const MAX_HORIZON: usize = 10_000;
const MAX_INNOVATIONS: usize = 4_096;
const MAX_ORDER: usize = 64;
const MAX_PSI: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    None,
    Log,
    Sqrt,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArimaSpec {
    pub seasonal_d: usize,
    pub period: usize,
    pub seasonal_ar: Vec<f64>,
    pub seasonal_ma: Vec<f64>,
    pub transform: Transform,
    pub constant: f64,
    pub d: usize,
    /// φ of `(1 − φ1 B − …) w_t = (1 + θ1 B + …) a_t`.
    pub ar: Vec<f64>,
    /// θ in Brockwell & Davis sign (PMML MA negated).
    pub ma: Vec<f64>,
    pub sigma2: f64,
    pub df: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Forecast {
    pub point: f64,
    pub lower: f64,
    pub upper: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeriesScore {
    /// One per historical `y`; the first `d` (after transform) are `None`
    /// (`ts_arima` / `ts_expert` row 1).
    pub fitted: Vec<Option<Forecast>>,
    pub forecasts: Vec<Forecast>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArimaError {
    Cancelled,
    Data(String),
    Settings(String),
}

impl fmt::Display for ArimaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArimaError::Cancelled => f.write_str("cancelled"),
            ArimaError::Data(message) => f.write_str(message),
            ArimaError::Settings(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ArimaError {}

/// Scores the history with one-step innovations and `horizon` h-step forecasts.
pub fn score(
    spec: &ArimaSpec,
    y: &[f64],
    horizon: usize,
    conf_percent: f64,
) -> Result<SeriesScore, ArimaError> {
    crate::check_cancelled()?;
    score_with_signal(spec, y, horizon, conf_percent, &[])
}

/// A known additive signal on the transformed target scale. Subtract before
/// conditioning the noise, then restore before inverse transformation (including
/// the log/sqrt variance correction and confidence bounds).
pub(crate) fn score_with_signal(
    spec: &ArimaSpec,
    y: &[f64],
    horizon: usize,
    conf_percent: f64,
    signal: &[f64],
) -> Result<SeriesScore, ArimaError> {
    score_signal(spec, y, horizon, conf_percent, signal, false)
}

/// Transfer Expert models put the signal on the differenced target scale.
pub(crate) fn score_with_differenced_signal(
    spec: &ArimaSpec,
    y: &[f64],
    horizon: usize,
    conf_percent: f64,
    signal: &[f64],
) -> Result<SeriesScore, ArimaError> {
    score_signal(spec, y, horizon, conf_percent, signal, true)
}

fn score_signal(
    spec: &ArimaSpec,
    y: &[f64],
    horizon: usize,
    conf_percent: f64,
    signal: &[f64],
    differenced_signal: bool,
) -> Result<SeriesScore, ArimaError> {
    spec.check()?;
    let expanded = spec.expanded()?;
    let spec = &expanded;
    if y.len() > MAX_HISTORY {
        return Err(ArimaError::Data(format!(
            "Time Series history of {} exceeds {MAX_HISTORY}",
            y.len()
        )));
    }
    if horizon > MAX_HORIZON {
        return Err(ArimaError::Settings(format!(
            "GAP: forecast span {horizon} exceeds {MAX_HORIZON}"
        )));
    }
    if y.iter().any(|v| !v.is_finite()) {
        return Err(ArimaError::Data(
            "Time Series target has a non-finite value".into(),
        ));
    }
    let p = (1.0 + conf_percent / 100.0) / 2.0;
    let tcrit = student_t_quantile(p, spec.df).ok_or_else(|| {
        ArimaError::Settings(format!(
            "GAP: confidence level {conf_percent} with df {} has no t quantile",
            spec.df
        ))
    })?;
    if !signal.is_empty()
        && (signal.len() != y.len().saturating_add(horizon)
            || signal.iter().any(|v| !v.is_finite()))
    {
        return Err(ArimaError::Data(
            "invalid transfer signal length/value".into(),
        ));
    }
    let offset = |at: usize| signal.get(at).copied().unwrap_or(0.0);
    let z: Vec<_> = transform(spec.transform, y)?
        .into_iter()
        .enumerate()
        .map(|(i, v)| v - if differenced_signal { 0.0 } else { offset(i) })
        .collect();
    let diff = spec.difference_polynomial()?;
    let degree = diff.len().saturating_sub(1);
    if z.len() <= degree {
        return Err(ArimaError::Data(
            "Time Series series is shorter than the differencing order".into(),
        ));
    }
    let differenced = filter_difference(&z, &diff);
    let w: Vec<f64> = differenced
        .iter()
        .enumerate()
        .map(|(i, v)| {
            v - spec.constant
                - if differenced_signal {
                    offset(i + degree)
                } else {
                    0.0
                }
        })
        .collect();
    let gamma_lags = w.len().saturating_add(horizon).saturating_add(2).max(2);
    let gamma = arma_gamma(&spec.ar, &spec.ma, gamma_lags)?;
    let (hats, v, w_forecasts, var_factors) =
        innovations_with_forecasts(&spec.ar, spec.ma.len(), &w, &gamma, &diff, horizon)?;

    let mut fitted = vec![None; z.len()];
    for (t, slot) in fitted.iter_mut().enumerate().skip(degree) {
        crate::check_cancelled()?;
        let w_index = t.saturating_sub(degree);
        let what = *hats.get(w_index).ok_or_else(|| {
            ArimaError::Data("Time Series innovations is short of the history".into())
        })?;
        let zhat = invert_difference(
            &diff,
            &z,
            t,
            spec.constant + what + if differenced_signal { offset(t) } else { 0.0 },
        )?;
        let vn = *v.get(w_index).ok_or_else(|| {
            ArimaError::Data("Time Series innovations variance is short of the history".into())
        })?;
        if vn < 0.0 || !vn.is_finite() {
            return Err(ArimaError::Data(
                "Time Series innovations variance is not positive".into(),
            ));
        }
        let se = (spec.sigma2 * vn).sqrt();
        *slot = Some(back_transform(
            spec.transform,
            zhat + if differenced_signal { 0.0 } else { offset(t) },
            se,
            tcrit,
        )?);
    }

    let mut forecasts = Vec::with_capacity(horizon);
    let mut extended = z.clone();
    for (what, var_factor) in w_forecasts.iter().zip(&var_factors) {
        crate::check_cancelled()?;
        let zhat = invert_difference(
            &diff,
            &extended,
            extended.len(),
            spec.constant
                + what
                + if differenced_signal {
                    offset(extended.len())
                } else {
                    0.0
                },
        )?;
        extended.push(zhat);
        if *var_factor < 0.0 || !var_factor.is_finite() {
            return Err(ArimaError::Data(
                "Time Series forecast variance is not positive".into(),
            ));
        }
        let se = (spec.sigma2 * var_factor).sqrt();
        forecasts.push(back_transform(
            spec.transform,
            zhat + if differenced_signal {
                0.0
            } else {
                offset(extended.len().saturating_sub(1))
            },
            se,
            tcrit,
        )?);
    }
    Ok(SeriesScore { fitted, forecasts })
}

impl ArimaSpec {
    fn difference_polynomial(&self) -> Result<Vec<f64>, ArimaError> {
        let mut poly = vec![1.0];
        for _ in 0..self.d {
            crate::check_cancelled()?;
            poly = polynomial_product(&poly, &[1.0, -1.0]);
        }
        if self.seasonal_d > 0 {
            let mut seasonal = vec![0.0; self.period.saturating_add(1)];
            if let Some(v) = seasonal.first_mut() {
                *v = 1.0;
            }
            if let Some(v) = seasonal.last_mut() {
                *v = -1.0;
            }
            for _ in 0..self.seasonal_d {
                crate::check_cancelled()?;
                poly = polynomial_product(&poly, &seasonal);
            }
        }
        if poly.len() > 65 {
            return Err(ArimaError::Settings("difference degree exceeds 64".into()));
        }
        Ok(poly)
    }

    pub(crate) fn expanded(&self) -> Result<Self, ArimaError> {
        let mut spec = self.clone();
        spec.ar = expand_coefficients(&self.ar, &self.seasonal_ar, self.period, -1.0)?;
        spec.ma = expand_coefficients(&self.ma, &self.seasonal_ma, self.period, 1.0)?;
        spec.seasonal_ar.clear();
        spec.seasonal_ma.clear();
        Ok(spec)
    }

    fn check(&self) -> Result<(), ArimaError> {
        if self.period == 0
            || self.period > 64
            || self.seasonal_d > 2
            || self.d > 2
            || self.seasonal_ar.len() > 2
            || self.seasonal_ma.len() > 2
        {
            return Err(ArimaError::Settings(
                "unsupported seasonal ARIMA order".into(),
            ));
        }
        if self.ar.len() > MAX_ORDER || self.ma.len() > MAX_ORDER || self.d > MAX_ORDER {
            return Err(ArimaError::Settings(
                "GAP: ARIMA order exceeds the supported maximum".into(),
            ));
        }
        if !self.constant.is_finite() || !self.sigma2.is_finite() || !self.df.is_finite() {
            return Err(ArimaError::Settings(
                "ARIMA constant, variance or df is not finite".into(),
            ));
        }
        if self.sigma2 <= 0.0 {
            return Err(ArimaError::Settings(
                "ARIMA errVariance must be positive".into(),
            ));
        }
        if self.df <= 0.0 {
            return Err(ArimaError::Settings(
                "ARIMA degreesOfFreedom must be positive".into(),
            ));
        }
        if self.ar.iter().chain(&self.ma).any(|v| !v.is_finite()) {
            return Err(ArimaError::Settings(
                "ARIMA AR/MA coefficient is not finite".into(),
            ));
        }
        Ok(())
    }
}

fn polynomial_product(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; a.len().saturating_add(b.len()).saturating_sub(1)];
    for (i, a) in a.iter().enumerate() {
        for (j, b) in b.iter().enumerate() {
            if let Some(v) = out.get_mut(i.saturating_add(j)) {
                *v += a * b;
            }
        }
    }
    out
}

fn expand_coefficients(
    a: &[f64],
    season: &[f64],
    period: usize,
    sign: f64,
) -> Result<Vec<f64>, ArimaError> {
    let degree = season.len().saturating_mul(period).saturating_add(a.len());
    if degree > 64 {
        return Err(ArimaError::Settings("ARMA degree exceeds 64".into()));
    }
    let mut poly = vec![1.0];
    poly.extend(a.iter().map(|v| v * sign));
    let mut sp = vec![0.0; season.len().saturating_mul(period).saturating_add(1)];
    if let Some(v) = sp.first_mut() {
        *v = 1.0;
    }
    for (i, c) in season.iter().enumerate() {
        crate::check_cancelled()?;
        if let Some(v) = sp.get_mut((i + 1) * period) {
            *v = c * sign;
        }
    }
    Ok(polynomial_product(&poly, &sp)
        .into_iter()
        .skip(1)
        .map(|v| v * sign)
        .collect())
}

pub(crate) fn transform(kind: Transform, y: &[f64]) -> Result<Vec<f64>, ArimaError> {
    match kind {
        Transform::None => Ok(y.to_vec()),
        Transform::Sqrt => y
            .iter()
            .map(|v| {
                if *v < 0.0 {
                    Err(ArimaError::Data(
                        "square root transform needs nonnegative values".into(),
                    ))
                } else {
                    Ok(v.sqrt())
                }
            })
            .collect(),
        Transform::Log => y
            .iter()
            .map(|v| {
                if *v <= 0.0 {
                    Err(ArimaError::Data(
                        "logarithmic Time Series transform needs positive values".into(),
                    ))
                } else {
                    Ok(v.ln())
                }
            })
            .collect(),
    }
}

/// The Expert Modeler's transform for the series `y` with seasonal period `period` (1 when
/// nonseasonal), Guide appendix "Time Series Algorithm: Expert Modeler", Step 2. A high-order
/// AR is fitted by OLS on `y`, `ln y` and `sqrt y`. Each is scored by the log-likelihood of the
/// untransformed series, `-N/2 ln(sigma^2) + sum ln|dz/dy|`, with `sigma^2` the residual
/// variance on its degrees of freedom and `N` the number of cases. The best one is used when
/// both `(l_max - l_y)/N` and `|(l_max - l_y)/l_y|` exceed 4%. These likelihood conventions are
/// not printed in the Guide; they are the ones that reproduce all 95 recorded transform
/// decisions (`crates/exec/tests/time_series_expert.rs`).
pub fn expert_transform(y: &[f64], period: usize) -> Transform {
    let n = y.len();
    let lags: Vec<usize> = if period <= 3 {
        (1..=10).collect()
    } else if period <= 11 {
        // "AR(10) if there are not enough data"
        if n > 14 + 20 {
            (1..=14).collect()
        } else {
            (1..=10).collect()
        }
    } else {
        let s = period;
        (1..=6)
            .chain(s..=s + 3)
            .chain(2 * s..=2 * s + 2)
            .filter(|lag| n >= 50 || *lag < 2 * s)
            .collect()
    };
    let likelihood = |z: &[f64], jacobian: f64| -> Option<f64> {
        let sigma2 = ols_ar_variance(z, &lags)?;
        Some(-0.5 * n as f64 * sigma2.ln() + jacobian)
    };
    if y.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Transform::None;
    }
    let Some(l_y) = likelihood(y, 0.0) else {
        return Transform::None;
    };
    let mut best = (Transform::None, l_y);
    if y.iter().all(|v| *v > 0.0) {
        let logs: Vec<f64> = y.iter().map(|v| v.ln()).collect();
        let roots: Vec<f64> = y.iter().map(|v| v.sqrt()).collect();
        let jac_log = -logs.iter().sum::<f64>();
        let jac_sqrt = -logs
            .iter()
            .map(|l| std::f64::consts::LN_2 + 0.5 * l)
            .sum::<f64>();
        for (kind, z, jac) in [
            (Transform::Log, &logs, jac_log),
            (Transform::Sqrt, &roots, jac_sqrt),
        ] {
            if let Some(l) = likelihood(z, jac)
                && l > best.1
            {
                best = (kind, l);
            }
        }
    }
    let gain = best.1 - l_y;
    if best.0 != Transform::None && gain / n as f64 > 0.04 && (gain / l_y).abs() > 0.04 {
        best.0
    } else {
        Transform::None
    }
}

/// Residual variance of an OLS AR fit with intercept on the given lags, on `n - p - k` degrees
/// of freedom; `None` when there are not enough cases.
fn ols_ar_variance(z: &[f64], lags: &[usize]) -> Option<f64> {
    let p = *lags.iter().max()?;
    let rows = z.len().checked_sub(p)?;
    let k = lags.len() + 1;
    if rows <= k {
        return None;
    }
    let design = |t: usize| -> Vec<f64> {
        std::iter::once(1.0)
            .chain(lags.iter().map(|lag| z[t - lag]))
            .collect()
    };
    let mut xtx = vec![vec![0.0; k]; k];
    let mut xtz = vec![0.0; k];
    for (t, target) in z.iter().enumerate().skip(p) {
        if crate::check_cancelled().is_err() {
            return None;
        }
        let x = design(t);
        for (a, xa) in x.iter().enumerate() {
            xtz[a] += xa * target;
            for (b, xb) in x.iter().enumerate() {
                xtx[a][b] += xa * xb;
            }
        }
    }
    let beta = crate::time_series::solve(xtx, xtz).ok()?;
    let sse: f64 = z
        .iter()
        .enumerate()
        .skip(p)
        .map(|(t, target)| {
            let fit: f64 = design(t).iter().zip(&beta).map(|(x, b)| x * b).sum();
            (target - fit) * (target - fit)
        })
        .sum();
    let sigma2 = sse / (rows - k) as f64;
    (sigma2 > 0.0 && sigma2.is_finite()).then_some(sigma2)
}

/// The Expert Modeler's differencing orders `(d, D)` for the (transformed) series `z` with
/// seasonal period `period` (1 when nonseasonal or when `n < 3s`), Guide appendix "Time Series
/// Algorithm: Expert Modeler", Step 3.
///
/// - Step (a) runs OLS regressions against the simulated critical values of Tables 1-3,
///   interpolated in `1/n`.
/// - Step (b) repeatedly fits ARMA(1,1), or (1,1)(1,1) when `s > 1` and `n >= 3s`, by
///   conditional least squares with pre-sample deviations and innovations at zero. It
///   differences while `phi > 0.88` and `|phi - theta| > 0.12`, or while `phi` is within 0.03
///   below 0.88 and the first six ACF have `|t| > 1.6`.
///
/// The zero pre-sample start is not printed in the Guide. It is the start that reproduces all
/// 95 recorded `(d, D)` decisions (`crates/exec/tests/time_series_expert.rs`).
pub fn expert_differencing(z: &[f64], period: usize) -> (usize, usize) {
    let s = period.max(1);
    let (mut d, mut big_d) = difference_step_a(z, s).unwrap_or((0, 0));
    let mut w = z.to_vec();
    for _ in 0..d {
        w = difference(&w, 1);
    }
    for _ in 0..big_d {
        w = seasonal_difference(&w, s);
    }
    // Step (b), repeated until no difference is needed (bounded: d + D <= 3).
    while d + big_d < 3 && w.len() > 10 {
        if s == 1 || w.len() < 3 * s {
            let Some((phi, theta)) = cls_arma11(&w) else {
                break;
            };
            let near = phi < 0.88 && 0.88 - phi < 0.03 && first_six_acf_significant(&w);
            if (phi > 0.88 && (phi - theta).abs() > 0.12) || near {
                w = difference(&w, 1);
                d += 1;
                continue;
            }
            break;
        }
        let Some((p1, t1, p2, t2)) = cls_arma11_seasonal(&w, s) else {
            break;
        };
        let a1 = p1 > 0.88 && (p1 - t1).abs() > 0.12;
        let a2 = p2 > 0.88 && (p2 - t2).abs() > 0.12;
        if a1 && a2 {
            w = seasonal_difference(&difference(&w, 1), s);
            d += 1;
            big_d += 1;
            continue;
        }
        let near = p1 < 0.88 && 0.88 - p1 < 0.03 && first_six_acf_significant(&w);
        if (a1 || near) && !a2 {
            w = difference(&w, 1);
            d += 1;
            continue;
        }
        if a2 && !a1 {
            w = seasonal_difference(&w, s);
            big_d += 1;
            continue;
        }
        break;
    }
    (d, big_d)
}

fn seasonal_difference(w: &[f64], s: usize) -> Vec<f64> {
    w.iter().zip(w.iter().skip(s)).map(|(a, b)| b - a).collect()
}

/// Sample sizes of the Guide's critical-value Tables 1-3.
const CRITICAL_N: [f64; 4] = [50.0, 100.0, 200.0, 300.0];
/// Table 1 (s = 1): `C(1,1), C(1,2), C(2,1)`.
const CRITICAL_S1: [[f64; 3]; 4] = [
    [1.616, 0.617, 0.734],
    [1.807, 0.807, 0.863],
    [1.904, 0.904, 0.930],
    [1.937, 0.937, 0.954],
];
/// Table 2 (s = 4, used for 1 < s < 8): `C(1,1), C(1,2), C(2,1), C(4,1)`.
const CRITICAL_S4: [[f64; 4]; 4] = [
    [0.557, 0.823, 0.849, 0.734],
    [0.773, 0.911, 0.908, 0.921],
    [0.886, 0.947, 0.947, 0.930],
    [0.925, 0.961, 0.963, 0.954],
];
/// Table 3 (s = 12, used for s >= 8): `C(1,1), C(1,2), C(2,1), C(4,1)`.
const CRITICAL_S12: [[f64; 4]; 4] = [
    [0.494, 0.811, 0.851, 0.734],
    [0.759, 0.909, 0.907, 0.921],
    [0.882, 0.947, 0.946, 0.930],
    [0.922, 0.961, 0.961, 0.954],
];

/// Linear interpolation in `1/n` between the nearest tabulated sizes (one-sided outside
/// 50..300); `n` below 36 uses 36 and above 1000 uses 1000.
fn critical<const K: usize>(table: &[[f64; K]; 4], n: usize) -> [f64; K] {
    let n = (n as f64).clamp(36.0, 1000.0);
    let (lo, hi) = if n <= CRITICAL_N[0] {
        (0, 1)
    } else if n >= CRITICAL_N[3] {
        (2, 3)
    } else {
        let hi = CRITICAL_N.iter().position(|v| *v >= n).unwrap_or(3);
        (hi.saturating_sub(1), hi)
    };
    let (n1, n2) = (CRITICAL_N[lo], CRITICAL_N[hi]);
    let mut out = [0.0; K];
    for (k, slot) in out.iter_mut().enumerate() {
        let (c1, c2) = (table[lo][k], table[hi][k]);
        *slot = if n1 == n2 {
            c1
        } else {
            c1 + (c2 - c1) * (1.0 / n - 1.0 / n1) / (1.0 / n2 - 1.0 / n1)
        };
    }
    out
}

/// The lower 5% point of Student's t.
fn t_lower(df: usize) -> Option<f64> {
    crate::stats::student_t_quantile(0.95, df as f64).map(|q| -q)
}

/// OLS of `y` on an intercept and `cols`: coefficients and standard errors.
fn ols_se(y: &[f64], cols: &[Vec<f64>]) -> Option<(Vec<f64>, Vec<f64>)> {
    let n = y.len();
    let k = cols.len() + 1;
    if n <= k || cols.iter().any(|c| c.len() != n) {
        return None;
    }
    let row = |t: usize| -> Vec<f64> {
        std::iter::once(1.0)
            .chain(cols.iter().map(|c| c[t]))
            .collect()
    };
    let mut xtx = vec![vec![0.0; k]; k];
    let mut xty = vec![0.0; k];
    for (t, target) in y.iter().enumerate() {
        if crate::check_cancelled().is_err() {
            return None;
        }
        let x = row(t);
        for (a, xa) in x.iter().enumerate() {
            xty[a] += xa * target;
            for (b, xb) in x.iter().enumerate() {
                xtx[a][b] += xa * xb;
            }
        }
    }
    let beta = crate::time_series::solve(xtx.clone(), xty).ok()?;
    let sse: f64 = y
        .iter()
        .enumerate()
        .map(|(t, target)| {
            let fit: f64 = row(t).iter().zip(&beta).map(|(x, b)| x * b).sum();
            (target - fit) * (target - fit)
        })
        .sum();
    let sigma2 = sse / (n - k) as f64;
    let mut se = Vec::with_capacity(k);
    for i in 0..k {
        if crate::check_cancelled().is_err() {
            return None;
        }
        let unit: Vec<f64> = (0..k).map(|j| if i == j { 1.0 } else { 0.0 }).collect();
        let column = crate::time_series::solve(xtx.clone(), unit).ok()?;
        se.push((sigma2 * column.get(i).copied()?).max(0.0).sqrt());
    }
    Some((beta, se))
}

/// Step (a): `(d, D)` from the Guide's OLS regressions.
fn difference_step_a(y: &[f64], s: usize) -> Option<(usize, usize)> {
    let n = y.len();
    let unit_root = |phi: f64, se_phi: f64, tc: f64, c_flat: f64, c_drift: f64| {
        (tc.abs() < 2.0 && phi > c_flat) || (tc.abs() >= 2.0 && (phi - 1.0) / se_phi > c_drift)
    };
    let ar1 = |k: usize| -> Option<(f64, f64, f64)> {
        let (b, se) = ols_se(y.get(k..)?, &[y.get(..n.checked_sub(k)?)?.to_vec()])?;
        let tc = if se[0] > 0.0 { b[0] / se[0] } else { 0.0 };
        Some((b[1], se[1], tc))
    };
    if s == 1 {
        let [c11, c12, c21] = critical(&CRITICAL_S1, n);
        let c31 = t_lower(n.checked_sub(3)?)?;
        let (b, _) = ols_se(
            y.get(2..)?,
            &[y.get(1..n - 1)?.to_vec(), y.get(..n - 2)?.to_vec()],
        )?;
        if b[1] > c11 && -b[2] > c12 {
            return Some((2, 0));
        }
        let (phi, se_phi, tc) = ar1(1)?;
        return Some(if unit_root(phi, se_phi, tc, c21, c31) {
            (1, 0)
        } else {
            (0, 0)
        });
    }
    let table = if s < 8 { &CRITICAL_S4 } else { &CRITICAL_S12 };
    let [c11, c12, c21, c41] = critical(table, n);
    let c13 = c11 * c12;
    let c31 = t_lower(n.checked_sub(s + 2)?)?;
    let c51 = t_lower(n.checked_sub(3)?)?;
    let (b, _) = ols_se(
        y.get(s + 1..)?,
        &[
            y.get(s..n - 1)?.to_vec(),
            y.get(1..n - s)?.to_vec(),
            y.get(..n - s - 1)?.to_vec(),
        ],
    )?;
    if b[1] > c11 && b[2] > c12 && -b[3] > c13 {
        return Some((1, 1));
    }
    if b[1] <= b[2] {
        let (phi, se_phi, tc) = ar1(s)?;
        return Some(if unit_root(phi, se_phi, tc, c21, c31) {
            (0, 1)
        } else {
            (0, 0)
        });
    }
    let (phi, se_phi, tc) = ar1(1)?;
    Some(if unit_root(phi, se_phi, tc, c41, c51) {
        (1, 0)
    } else {
        (0, 0)
    })
}

/// All of the first six sample autocorrelations have Bartlett `|t| > 1.6`.
fn first_six_acf_significant(w: &[f64]) -> bool {
    let n = w.len();
    if n <= 7 {
        return false;
    }
    let mean = w.iter().sum::<f64>() / n as f64;
    let dev: Vec<f64> = w.iter().map(|v| v - mean).collect();
    let den: f64 = dev.iter().map(|v| v * v).sum();
    if den <= 0.0 {
        return false;
    }
    let mut cumulative = 0.0;
    (1..=6).all(|k| {
        let r = dev
            .iter()
            .zip(dev.iter().skip(k))
            .map(|(a, b)| a * b)
            .sum::<f64>()
            / den;
        let se = ((1.0 + 2.0 * cumulative) / n as f64).sqrt();
        cumulative += r * r;
        (r / se).abs() > 1.6
    })
}

/// CLS residuals of `(1 - phi B)(1 - Phi B^s)(w - mu) = (1 - theta B)(1 - Theta B^s) a`,
/// with pre-sample deviations and innovations at zero; `params` = mu, phi, theta[, Phi, Theta].
fn cls_residuals(w: &[f64], s: usize, params: &[f64]) -> Vec<f64> {
    let seasonal = params.len() == 5;
    let (mu, phi, theta) = (params[0], params[1], params[2]);
    let (big_phi, big_theta) = if seasonal {
        (params[3], params[4])
    } else {
        (0.0, 0.0)
    };
    let pad = if seasonal { s + 1 } else { 1 };
    let mut dev = vec![0.0; pad];
    dev.extend(w.iter().map(|v| v - mu));
    let mut a = vec![0.0; dev.len()];
    for t in pad..dev.len() {
        let mut ar = phi * dev[t - 1];
        let mut ma = theta * a[t - 1];
        if seasonal {
            ar += big_phi * dev[t - s] - phi * big_phi * dev[t - s - 1];
            ma += big_theta * a[t - s] - theta * big_theta * a[t - s - 1];
        }
        a[t] = dev[t] - ar + ma;
    }
    a.split_off(pad)
}

fn cls_fit(w: &[f64], s: usize, start: Vec<f64>) -> Option<Vec<f64>> {
    let bounds: Vec<(f64, f64)> = (0..start.len())
        .map(|i| {
            if i == 0 {
                (f64::MIN, f64::MAX)
            } else {
                (-10.0, 10.0)
            }
        })
        .collect();
    let (x, _) = crate::time_series::lm(start, &bounds, |p| Ok(cls_residuals(w, s, p))).ok()?;
    x.iter().all(|v| v.is_finite()).then_some(x)
}

fn cls_arma11(w: &[f64]) -> Option<(f64, f64)> {
    let mean = w.iter().sum::<f64>() / w.len() as f64;
    let x = cls_fit(w, 1, vec![mean, 0.5, 0.1])?;
    Some((x[1], x[2]))
}

fn cls_arma11_seasonal(w: &[f64], s: usize) -> Option<(f64, f64, f64, f64)> {
    let mean = w.iter().sum::<f64>() / w.len() as f64;
    let x = cls_fit(w, s, vec![mean, 0.5, 0.1, 0.5, 0.1])?;
    Some((x[1], x[2], x[3], x[4]))
}

/// Whether the Expert Modeler keeps the constant of a model with the ARMA part of `spec`:
/// always when the model has no AR or MA terms, otherwise when the constant's t statistic
/// ([`expert_constant_t`]) exceeds 1.96 in absolute value.
///
/// Inferred from Modeler's selections, not documented: all recorded ARIMA selections agree
/// (dropped constants have |t| <= 1.25 and all have ARMA terms; kept ones with ARMA terms have
/// |t| >= 2.76; `crates/exec/tests/time_series_expert.rs`).
pub fn expert_keeps_constant(spec: &ArimaSpec, y: &[f64]) -> Result<bool, ArimaError> {
    let terms = [&spec.ar, &spec.ma, &spec.seasonal_ar, &spec.seasonal_ma]
        .iter()
        .flat_map(|coefficients| coefficients.iter())
        .filter(|c| **c != 0.0)
        .count();
    Ok(terms == 0 || expert_constant_t(spec, y)?.abs() > 1.96)
}

/// The t statistic of the mean of the differenced (transformed) series, with the ARMA
/// coefficients of `spec` held fixed: the GLS estimate `1'Σ⁻¹w / 1'Σ⁻¹1` over its standard
/// error, the residual variance on `n − k` degrees of freedom (`k` counts the nonzero ARMA
/// coefficients and the constant). `spec.constant` is ignored.
pub fn expert_constant_t(spec: &ArimaSpec, y: &[f64]) -> Result<f64, ArimaError> {
    spec.check()?;
    let parameters = 1 + [&spec.ar, &spec.ma, &spec.seasonal_ar, &spec.seasonal_ma]
        .iter()
        .flat_map(|coefficients| coefficients.iter())
        .filter(|c| **c != 0.0)
        .count();
    let expanded = spec.expanded()?;
    if y.len() > 512 || y.iter().any(|v| !v.is_finite()) {
        return Err(ArimaError::Data(
            "ARIMA constant test requires at most 512 finite observations".into(),
        ));
    }
    let z = transform(expanded.transform, y)?;
    let w = filter_difference(&z, &expanded.difference_polynomial()?);
    let n = w.len();
    if n <= parameters {
        return Err(ArimaError::Data(
            "too few observations for the constant test".into(),
        ));
    }
    let gamma = arma_gamma(&expanded.ar, &expanded.ma, n.saturating_add(2))?;
    let standardized = |series: &[f64]| -> Result<Vec<f64>, ArimaError> {
        let (hats, variances) = innovations(series, &gamma)?;
        series
            .iter()
            .zip(hats)
            .zip(variances)
            .map(|((value, hat), variance)| {
                if variance > 0.0 && variance.is_finite() {
                    Ok((value - hat) / variance.sqrt())
                } else {
                    Err(ArimaError::Data(
                        "ARIMA innovation variance is not positive".into(),
                    ))
                }
            })
            .collect()
    };
    let e_w = standardized(&w)?;
    let e_one = standardized(&vec![1.0; n])?;
    let information: f64 = e_one.iter().map(|v| v * v).sum();
    if information <= 0.0 || !information.is_finite() {
        return Err(ArimaError::Data(
            "constant test information is not positive".into(),
        ));
    }
    let mean = e_w.iter().zip(&e_one).map(|(a, b)| a * b).sum::<f64>() / information;
    let sse: f64 = e_w
        .iter()
        .zip(&e_one)
        .map(|(a, b)| (a - mean * b) * (a - mean * b))
        .sum();
    let sigma2 = sse / (n - parameters) as f64;
    if sigma2 <= 0.0 || !sigma2.is_finite() {
        return Err(ArimaError::Data(
            "constant test residual variance is not positive".into(),
        ));
    }
    Ok(mean * (information / sigma2).sqrt())
}

pub(crate) fn difference(z: &[f64], d: usize) -> Vec<f64> {
    let mut series = z.to_vec();
    for _ in 0..d {
        if series.len() < 2 {
            return Vec::new();
        }
        series = series
            .windows(2)
            .filter_map(|pair| match pair {
                [a, b] => Some(b - a),
                _ => None,
            })
            .collect();
    }
    series
}

fn invert_difference(diff: &[f64], z: &[f64], t: usize, w: f64) -> Result<f64, ArimaError> {
    let mut value = w;
    for (lag, coef) in diff.iter().enumerate().skip(1) {
        crate::check_cancelled()?;
        let at = t
            .checked_sub(lag)
            .ok_or_else(|| ArimaError::Data("insufficient differencing history".into()))?;
        value -= coef
            * z.get(at)
                .copied()
                .ok_or_else(|| ArimaError::Data("missing differencing history".into()))?;
    }
    Ok(value)
}

fn filter_difference(z: &[f64], diff: &[f64]) -> Vec<f64> {
    z.iter()
        .enumerate()
        .skip(diff.len().saturating_sub(1))
        .map(|(i, _)| {
            diff.iter()
                .enumerate()
                .map(|(j, c)| c * z.get(i.saturating_sub(j)).copied().unwrap_or(0.0))
                .sum()
        })
        .collect()
}

pub(crate) fn back_transform(
    kind: Transform,
    zhat: f64,
    se: f64,
    tcrit: f64,
) -> Result<Forecast, ArimaError> {
    if !zhat.is_finite() || !se.is_finite() || se < 0.0 {
        return Err(ArimaError::Data(
            "Time Series prediction is not finite".into(),
        ));
    }
    let forecast = match kind {
        Transform::Sqrt => Forecast {
            point: zhat * zhat + se * se,
            lower: (zhat - tcrit * se).max(0.0).powi(2),
            upper: (zhat + tcrit * se).max(0.0).powi(2),
        },
        Transform::None => Forecast {
            point: zhat,
            lower: zhat - tcrit * se,
            upper: zhat + tcrit * se,
        },
        Transform::Log => {
            let point = (zhat + se * se / 2.0).exp();
            Forecast {
                point,
                lower: (zhat - tcrit * se).exp(),
                upper: (zhat + tcrit * se).exp(),
            }
        }
    };
    if [forecast.point, forecast.lower, forecast.upper]
        .iter()
        .any(|v| !v.is_finite())
    {
        return Err(ArimaError::Data("inverse transformation overflow".into()));
    }
    Ok(forecast)
}

fn arma_psi(phi: &[f64], theta: &[f64], n: usize) -> Result<Vec<f64>, ArimaError> {
    let n = n.min(MAX_PSI);
    let mut psi = Vec::with_capacity(n);
    psi.push(1.0);
    for j in 1..n {
        crate::check_cancelled()?;
        let mut v = theta.get(j - 1).copied().unwrap_or(0.0);
        for i in 0..phi.len().min(j) {
            let term =
                phi.get(i).copied().unwrap_or(0.0) * psi.get(j - 1 - i).copied().unwrap_or(0.0);
            v += term;
        }
        if !v.is_finite() {
            return Err(ArimaError::Data("ARMA psi weights are not finite".into()));
        }
        psi.push(v);
    }
    Ok(psi)
}

fn arma_gamma(phi: &[f64], theta: &[f64], lags: usize) -> Result<Vec<f64>, ArimaError> {
    // Exact stationary covariance for the estimation subset, including roots
    // close to the unit circle where truncating psi weights is inappropriate.
    if phi.len() <= 1 && theta.len() <= 1 {
        let p = phi.first().copied().unwrap_or(0.0);
        let t = theta.first().copied().unwrap_or(0.0);
        if p.abs() >= 1.0 {
            return Err(ArimaError::Settings("AR root is not stationary".into()));
        }
        let g0 = 1.0 + (p + t).powi(2) / (1.0 - p * p);
        let mut gamma = Vec::with_capacity(lags);
        let mut g = p * g0 + t;
        for k in 0..lags {
            crate::check_cancelled()?;
            gamma.push(if k == 0 { g0 } else { g });
            if k > 0 {
                g *= p;
            }
        }
        return Ok(gamma);
    }
    // Solve the finite Yule-Walker system, including the MA forcing terms.
    // Truncated impulse sums bias near-unit seasonal roots (ts_sarlog_noconst).
    let p = phi.len();
    let psi = arma_psi(phi, theta, theta.len().saturating_add(1))?;
    let forcing = |k: usize| -> f64 {
        let mut total = if k == 0 { 1.0 } else { 0.0 };
        for (j, t) in theta.iter().enumerate() {
            if let Some(lag) = (j + 1).checked_sub(k) {
                total += t * psi.get(lag).copied().unwrap_or(0.0);
            }
        }
        total
    };
    let mut matrix = vec![vec![0.0; p + 1]; p + 1];
    let mut rhs = Vec::with_capacity(p + 1);
    for (k, row) in matrix.iter_mut().enumerate() {
        crate::check_cancelled()?;
        if let Some(v) = row.get_mut(k) {
            *v = 1.0;
        }
        for (i, v) in phi.iter().enumerate() {
            if let Some(c) = row.get_mut(k.abs_diff(i + 1)) {
                *c -= v;
            }
        }
        rhs.push(forcing(k));
    }
    let mut gamma =
        crate::time_series::solve(matrix, rhs).map_err(|e| ArimaError::Data(e.to_string()))?;
    if gamma.first().is_none_or(|v| *v <= 0.0 || !v.is_finite()) {
        return Err(ArimaError::Settings("nonstationary covariance".into()));
    }
    for k in gamma.len()..lags {
        crate::check_cancelled()?;
        let g = forcing(k)
            + phi
                .iter()
                .enumerate()
                .map(|(i, v)| v * gamma.get(k - i - 1).copied().unwrap_or(0.0))
                .sum::<f64>();
        if !g.is_finite() {
            return Err(ArimaError::Data("ARMA covariance overflow".into()));
        }
        gamma.push(g);
    }
    gamma.truncate(lags);
    Ok(gamma)
}

/// Exact concentrated Gaussian likelihood ingredients using the scoring recursion.
/// Returns standardized innovations and log determinant (innovation variance at sigma²=1).
pub fn likelihood_residuals(spec: &ArimaSpec, y: &[f64]) -> Result<(Vec<f64>, f64), ArimaError> {
    spec.check()?;
    // Preserve the innovations arithmetic for all previously fitted structures.
    let extended = spec.d > 1 || spec.ar.len() > 4 || spec.ma.len() > 4;
    let expanded = spec.expanded()?;
    let spec = &expanded;
    if y.len() < 5 || y.len() > 512 || y.iter().any(|v| !v.is_finite()) {
        return Err(ArimaError::Data(
            "ARIMA estimation requires 5..=512 finite observations".into(),
        ));
    }
    if spec.d > 2 || spec.ar.len() > MAX_ORDER || spec.ma.len() > MAX_ORDER {
        return Err(ArimaError::Settings(
            "GAP: estimation supports d <= 2 and expanded p/q <= 64".into(),
        ));
    }
    let z = transform(spec.transform, y)?;
    let diff = spec.difference_polynomial()?;
    let w: Vec<f64> = filter_difference(&z, &diff)
        .iter()
        .map(|v| v - spec.constant)
        .collect();
    let gamma = arma_gamma(&spec.ar, &spec.ma, w.len().saturating_add(2))?;
    if extended {
        return toeplitz_likelihood(&w, &gamma);
    }
    let (hats, variances) = innovations(&w, &gamma)?;
    let mut residuals = Vec::with_capacity(w.len());
    let mut log_det = 0.0;
    for ((value, hat), variance) in w.iter().zip(hats).zip(variances) {
        crate::check_cancelled()?;
        if variance <= 0.0 || !variance.is_finite() {
            return Err(ArimaError::Data(
                "ARIMA innovation variance is not positive".into(),
            ));
        }
        residuals.push((value - hat) / variance.sqrt());
        log_det += variance.ln();
    }
    Ok((residuals, log_det))
}

/// Exact Gaussian innovations via Durbin-Levinson, O(n^2) time and O(n) space.
/// The stationary covariance is Toeplitz: the order-m predictor's reflection
/// coefficient is (gamma[m] - sum a[j] gamma[m-j]) / v[m-1]. This is the same
/// conditional Gaussian likelihood as `innovations`, without its cubic work.
fn toeplitz_likelihood(w: &[f64], gamma: &[f64]) -> Result<(Vec<f64>, f64), ArimaError> {
    let mut variance = gamma.first().copied().unwrap_or(0.0);
    let mut coefficients: Vec<f64> = Vec::new();
    let mut residuals = Vec::with_capacity(w.len());
    let mut log_det = 0.0;
    for (t, value) in w.iter().enumerate() {
        crate::check_cancelled()?;
        if variance <= 0.0 || !variance.is_finite() {
            return Err(ArimaError::Data(
                "ARIMA innovation variance is not positive".into(),
            ));
        }
        let hat = coefficients
            .iter()
            .enumerate()
            .map(|(j, a)| a * w.get(t - j - 1).copied().unwrap_or(0.0))
            .sum::<f64>();
        residuals.push((value - hat) / variance.sqrt());
        log_det += variance.ln();
        if t + 1 == w.len() {
            break;
        }
        let m = t + 1;
        let reflection = (gamma.get(m).copied().unwrap_or(0.0)
            - coefficients
                .iter()
                .enumerate()
                .map(|(j, a)| a * gamma.get(m - j - 1).copied().unwrap_or(0.0))
                .sum::<f64>())
            / variance;
        let mut next: Vec<_> = coefficients
            .iter()
            .zip(coefficients.iter().rev())
            .map(|(a, b)| a - reflection * b)
            .collect();
        next.push(reflection);
        coefficients = next;
        variance *= 1.0 - reflection * reflection;
    }
    Ok((residuals, log_det))
}

type ForecastInnovations = (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>);

/// Innovations algorithm. `hats[i]` predicts `w[i]` (`hats[0] = 0`); `hats[n]`
/// is the one-step predictor beyond the sample. `v[i]` is its MSE with σ² = 1.
fn innovations(w: &[f64], gamma: &[f64]) -> Result<(Vec<f64>, Vec<f64>), ArimaError> {
    let (hats, v, _, _) = innovations_with_forecasts(&[], 0, w, gamma, &[1.0], 0)?;
    Ok((hats, v))
}

/// The innovations recursion continued over `horizon` future steps: one-step predictors and
/// MSEs, then finite-history forecasts of `w` and their variance factors (σ² = 1) for the
/// expanded ARMA with AR coefficients `ar` and MA order `q_deg`, integrated by `diff`.
fn innovations_with_forecasts(
    ar: &[f64],
    q_deg: usize,
    w: &[f64],
    gamma: &[f64],
    diff: &[f64],
    horizon: usize,
) -> Result<ForecastInnovations, ArimaError> {
    let n = w.len();
    let total = n.saturating_add(horizon);
    if total > MAX_INNOVATIONS {
        return Err(ArimaError::Settings(format!(
            "GAP: ARIMA series + horizon {total} exceeds {MAX_INNOVATIONS} for exact innovations"
        )));
    }
    let mut v = vec![0.0; total.saturating_add(1)];
    let g0 = *gamma
        .first()
        .ok_or_else(|| ArimaError::Data("ARMA autocovariance is empty".into()))?;
    if let Some(slot) = v.first_mut() {
        *slot = g0;
    }
    let mut hats = vec![0.0; n.saturating_add(1)];
    let mut thetas: Vec<Vec<f64>> = Vec::with_capacity(total);
    for n_obs in 1..=total {
        crate::check_cancelled()?;
        let mut th = vec![0.0; n_obs.saturating_add(1)];
        if let Some(slot) = th.first_mut() {
            *slot = 1.0;
        }
        for k in 0..n_obs {
            let lag = n_obs.saturating_sub(k);
            let mut acc = gamma.get(lag).copied().unwrap_or(0.0);
            for j in 0..k {
                let th_k = thetas.get(k.saturating_sub(1));
                let lag_k = k.saturating_sub(j);
                let lag_n = n_obs.saturating_sub(j);
                let tk = th_k.and_then(|row| row.get(lag_k)).copied().unwrap_or(0.0);
                let tn = th.get(lag_n).copied().unwrap_or(0.0);
                let vj = v.get(j).copied().unwrap_or(0.0);
                acc -= tk * tn * vj;
            }
            let vk = v.get(k).copied().unwrap_or(0.0);
            if vk.abs() < 1e-300 {
                return Err(ArimaError::Data(
                    "Time Series innovations variance is zero".into(),
                ));
            }
            if let Some(slot) = th.get_mut(lag) {
                *slot = acc / vk;
            }
        }
        let mut vn = g0;
        for j in 0..n_obs {
            let thj = th.get(n_obs.saturating_sub(j)).copied().unwrap_or(0.0);
            let vj = v.get(j).copied().unwrap_or(0.0);
            vn -= thj * thj * vj;
        }
        if let Some(slot) = v.get_mut(n_obs) {
            *slot = vn;
        }
        if n_obs <= n {
            let mut hat = 0.0;
            for j in 1..=n_obs {
                let idx = n_obs.saturating_sub(j);
                let w_idx = w.get(idx).copied().unwrap_or(0.0);
                let h_idx = hats.get(idx).copied().unwrap_or(0.0);
                let thj = th.get(j).copied().unwrap_or(0.0);
                hat += thj * (w_idx - h_idx);
            }
            if let Some(slot) = hats.get_mut(n_obs) {
                *slot = hat;
            }
        }
        thetas.push(th);
    }

    let mut w_forecasts = Vec::with_capacity(horizon);
    for a in 0..horizon {
        crate::check_cancelled()?;
        let m = n.saturating_add(a);
        let mut pred = 0.0;
        for idx in 0..n {
            let lag = m.saturating_sub(idx);
            let th_val = if m == 0 {
                0.0
            } else {
                thetas
                    .get(m.saturating_sub(1))
                    .and_then(|row| row.get(lag))
                    .copied()
                    .unwrap_or(0.0)
            };
            let w_idx = w.get(idx).copied().unwrap_or(0.0);
            let h_idx = hats.get(idx).copied().unwrap_or(0.0);
            pred += th_val * (w_idx - h_idx);
        }
        w_forecasts.push(pred);
    }

    let mut ar_poly = Vec::with_capacity(ar.len().saturating_add(1));
    ar_poly.push(1.0);
    for coef in ar {
        crate::check_cancelled()?;
        ar_poly.push(-coef);
    }
    let ar_diff = polynomial_product(&ar_poly, diff);
    let psi_ad = difference_inverse(&ar_diff, horizon);
    let p_deg = ar.len();
    let theta_x = |m: usize, j: usize| -> f64 {
        if j == 0 {
            return 1.0;
        }
        let mut val = 0.0;
        for i in 0..=p_deg.min(j) {
            let lag = j.saturating_sub(i);
            let row_idx = m.saturating_sub(i);
            let th_w = if lag == 0 {
                1.0
            } else if row_idx == 0 {
                0.0
            } else {
                thetas
                    .get(row_idx.saturating_sub(1))
                    .and_then(|row| row.get(lag))
                    .copied()
                    .unwrap_or(0.0)
            };
            let ar_i = ar_poly.get(i).copied().unwrap_or(0.0);
            val += ar_i * th_w;
        }
        val
    };

    let mut var_factors = Vec::with_capacity(horizon);
    for a in 0..horizon {
        crate::check_cancelled()?;
        let mut acc_var = 0.0;
        for r_idx in 0..=a {
            let base_r = r_idx.max(a.saturating_sub(q_deg));
            let mut c_ar = 0.0;
            for j in 0..=a.saturating_sub(r_idx).min(q_deg) {
                let m_lookup = n.saturating_add(if j == 0 {
                    r_idx
                } else {
                    base_r.saturating_add(j)
                });
                let pd = psi_ad
                    .get(a.saturating_sub(r_idx).saturating_sub(j))
                    .copied()
                    .unwrap_or(0.0);
                c_ar += pd * theta_x(m_lookup, j);
            }
            let vr = v.get(n.saturating_add(r_idx)).copied().unwrap_or(0.0);
            acc_var += c_ar * c_ar * vr;
        }
        var_factors.push(acc_var);
    }

    Ok((hats, v, w_forecasts, var_factors))
}

fn difference_inverse(diff: &[f64], horizon: usize) -> Vec<f64> {
    let len = horizon.clamp(1, MAX_PSI);
    let mut psi = vec![0.0; len];
    if let Some(slot) = psi.first_mut() {
        *slot = 1.0;
    }
    let degree = diff.len().saturating_sub(1);
    for j in 1..len {
        let mut acc = 0.0;
        for i in 1..=j.min(degree) {
            let d_i = diff.get(i).copied().unwrap_or(0.0);
            let prev = psi.get(j.saturating_sub(i)).copied().unwrap_or(0.0);
            acc -= d_i * prev;
        }
        if let Some(slot) = psi.get_mut(j) {
            *slot = acc;
        }
    }
    psi
}

impl From<dms_types::Cancelled> for ArimaError {
    fn from(_: dms_types::Cancelled) -> Self {
        Self::Cancelled
    }
}

#[cfg(test)]
#[allow(clippy::excessive_precision, reason = "oracle CSV prints 15 decimals")]
mod tests {
    use super::*;

    #[test]
    fn seasonal_covariance_near_unit_root_is_not_truncated() {
        let mut ar = vec![0.0; 12];
        ar[11] = 0.999;
        let g = arma_gamma(&ar, &[], 25).unwrap();
        let variance = 1.0 / (1.0 - 0.999_f64.powi(2));
        assert!((g[0] - variance).abs() < 1e-9);
        assert!((g[12] - 0.999 * variance).abs() < 1e-9);
        assert_eq!(g[1], 0.0);
    }

    #[test]
    fn toeplitz_likelihood_matches_innovations() {
        let w: Vec<_> = (0..64).map(|i| (f64::from(i) * 1.731).sin()).collect();
        let mut sparse_ar = vec![0.0; 24];
        sparse_ar[0] = 0.2;
        sparse_ar[23] = -0.35;
        let mut sparse_ma = vec![0.0; 24];
        sparse_ma[4] = 0.25;
        sparse_ma[23] = -0.3;
        for (ar, ma) in [
            (vec![], vec![]),
            (vec![0.999], vec![-0.8]),
            (vec![], sparse_ma.clone()),
            (sparse_ar, sparse_ma),
        ] {
            let gamma = arma_gamma(&ar, &ma, w.len() + 2).unwrap();
            let (hats, variances) = innovations(&w, &gamma).unwrap();
            let (r, ld) = toeplitz_likelihood(&w, &gamma).unwrap();
            let expected_ld: f64 = variances.iter().take(w.len()).map(|v| v.ln()).sum();
            assert!((ld - expected_ld).abs() < 1e-9, "{ld} vs {expected_ld}");
            for (((value, hat), variance), r) in w.iter().zip(hats).zip(variances).zip(r) {
                let expected = (value - hat) / variance.sqrt();
                assert!((r - expected).abs() < 1e-9, "{r} vs {expected}");
            }
        }
    }

    #[test]
    fn inverse_transform_overflow_is_an_error() {
        for transform in [Transform::None, Transform::Log, Transform::Sqrt] {
            assert!(back_transform(transform, 1e308, 1e308, 2.0).is_err());
        }
    }

    fn close(a: f64, b: f64, abs: f64) -> bool {
        (a - b).abs() <= abs
    }

    fn sales() -> Vec<f64> {
        vec![
            20.5, 22.0, 23.5, 25.0, 24.5, 24.0, 23.5, 23.0, 22.5, 24.0, 25.5, 28.0, 26.5, 28.0,
            29.5, 31.0, 30.5, 30.0, 29.5, 29.0, 28.5, 30.0, 31.5, 34.0, 32.5, 34.0, 35.5, 37.0,
            36.5, 36.0, 35.5, 35.0, 34.5, 36.0, 37.5, 40.0,
        ]
    }

    fn arima_sales() -> ArimaSpec {
        ArimaSpec {
            seasonal_d: 0,
            period: 1,
            seasonal_ar: vec![],
            seasonal_ma: vec![],
            transform: Transform::None,
            constant: 0.5767484206988256,
            d: 1,
            ar: vec![0.22039078632882758],
            ma: vec![-0.025433442860370244],
            sigma2: 1.5018952362990103,
            df: 32.0,
        }
    }

    fn expert_sales() -> ArimaSpec {
        ArimaSpec {
            seasonal_d: 0,
            period: 1,
            seasonal_ar: vec![],
            seasonal_ma: vec![],
            transform: Transform::Log,
            constant: 0.01909870194198764,
            d: 1,
            ar: Vec::new(),
            ma: Vec::new(),
            sigma2: 0.0017348538310481068,
            df: 34.0,
        }
    }

    #[test]
    fn arima111_matches_ts_arima_sales() {
        let scored = score(&arima_sales(), &sales(), 6, 95.0).unwrap();
        assert!(scored.fitted[0].is_none());
        let t2 = scored.fitted[1].unwrap();
        // ts_arima oracle row 2: y1+c, interval from r_0.
        assert!(close(t2.point, 21.076748420698827, 1e-12));
        assert!(close(t2.lower, 18.531075468525607, 1e-12));
        assert!(close(t2.upper, 23.622421372872047, 1e-12));
        let t3 = scored.fitted[2].unwrap();
        assert!(close(t3.point, 22.757645116819514, 1e-12));
        let h1 = scored.forecasts[0];
        assert!(close(h1.point, 40.956391211360739, 1e-12));
        let h2 = scored.forecasts[1];
        assert!(close(h2.point, 41.616809405217616, 1e-12));
        assert!(close(h2.lower, 37.727127409704629, 1e-12));
    }

    #[test]
    fn log_arima010_matches_ts_expert_sales() {
        let scored = score(&expert_sales(), &sales(), 6, 95.0).unwrap();
        let t2 = scored.fitted[1].unwrap();
        assert!(close(t2.point, 20.913419097153703, 1e-12));
        assert!(close(t2.lower, 19.199368738510984, 1e-12));
        assert!(close(t2.upper, 22.741007121407240, 1e-12));
        let h1 = scored.forecasts[0];
        assert!(close(h1.point, 40.806671409080394, 1e-12));
        let h2 = scored.forecasts[1];
        assert!(close(h2.point, 41.629610787216457, 1e-12));
        assert!(close(h2.lower, 36.868925585806707, 1e-12));
    }

    #[test]
    fn finite_history_forecast_conditioning_uses_innovations_variance() {
        let spec = ArimaSpec {
            seasonal_d: 1,
            period: 12,
            seasonal_ar: vec![],
            seasonal_ma: vec![-0.8183351823458053],
            transform: Transform::None,
            constant: -0.014048492884527525,
            d: 1,
            ar: vec![],
            ma: vec![-0.9929555033480911],
            sigma2: 2.915957104462519,
            df: 80.0,
        };
        let y: Vec<f64> = (0..96).map(|t| 50.0 + 0.5 * f64::from(t)).collect();
        let expanded = spec.expanded().unwrap();
        let diff = expanded.difference_polynomial().unwrap();
        let w: Vec<f64> = filter_difference(&y, &diff)
            .iter()
            .map(|v| v - expanded.constant)
            .collect();
        let gamma = arma_gamma(&expanded.ar, &expanded.ma, w.len() + 8).unwrap();
        let (_, v, _, var_factors) =
            innovations_with_forecasts(&expanded.ar, expanded.ma.len(), &w, &gamma, &diff, 6)
                .unwrap();
        assert!(close(v[w.len()], 1.0373129324449755, 1e-12));
        assert!(close(var_factors[0], v[w.len()], 1e-15));
        assert!(var_factors[0] > 1.03);
    }
}
