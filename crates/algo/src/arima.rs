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
    Data(String),
    Settings(String),
}

impl fmt::Display for ArimaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
    let z = transform(spec.transform, y)?;
    let diff = spec.difference_polynomial()?;
    let degree = diff.len().saturating_sub(1);
    if z.len() <= degree {
        return Err(ArimaError::Data(
            "Time Series series is shorter than the differencing order".into(),
        ));
    }
    let differenced = filter_difference(&z, &diff);
    let w: Vec<f64> = differenced.iter().map(|v| v - spec.constant).collect();
    let gamma_lags = w.len().saturating_add(horizon).saturating_add(2).max(2);
    let gamma = arma_gamma(&spec.ar, &spec.ma, gamma_lags)?;
    let (hats, v, w_forecasts, var_factors) =
        innovations_with_forecasts(&spec.ar, spec.ma.len(), &w, &gamma, &diff, horizon)?;

    let mut fitted = vec![None; z.len()];
    for (t, slot) in fitted.iter_mut().enumerate().skip(degree) {
        let w_index = t.saturating_sub(degree);
        let what = *hats.get(w_index).ok_or_else(|| {
            ArimaError::Data("Time Series innovations is short of the history".into())
        })?;
        let zhat = invert_difference(&diff, &z, t, spec.constant + what)?;
        let vn = *v.get(w_index).ok_or_else(|| {
            ArimaError::Data("Time Series innovations variance is short of the history".into())
        })?;
        if vn < 0.0 || !vn.is_finite() {
            return Err(ArimaError::Data(
                "Time Series innovations variance is not positive".into(),
            ));
        }
        let se = (spec.sigma2 * vn).sqrt();
        *slot = Some(back_transform(spec.transform, zhat, se, tcrit)?);
    }

    let mut forecasts = Vec::with_capacity(horizon);
    let mut extended = z.clone();
    for (what, var_factor) in w_forecasts.iter().zip(&var_factors) {
        let zhat = invert_difference(&diff, &extended, extended.len(), spec.constant + what)?;
        extended.push(zhat);
        if *var_factor < 0.0 || !var_factor.is_finite() {
            return Err(ArimaError::Data(
                "Time Series forecast variance is not positive".into(),
            ));
        }
        let se = (spec.sigma2 * var_factor).sqrt();
        forecasts.push(back_transform(spec.transform, zhat, se, tcrit)?);
    }
    Ok(SeriesScore { fitted, forecasts })
}

impl ArimaSpec {
    fn difference_polynomial(&self) -> Result<Vec<f64>, ArimaError> {
        let mut poly = vec![1.0];
        for _ in 0..self.d {
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
                poly = polynomial_product(&poly, &seasonal);
            }
        }
        if poly.len() > 65 {
            return Err(ArimaError::Settings("difference degree exceeds 64".into()));
        }
        Ok(poly)
    }

    fn expanded(&self) -> Result<Self, ArimaError> {
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
    let expanded = spec.expanded()?;
    let spec = &expanded;
    if y.len() < 5 || y.len() > 512 || y.iter().any(|v| !v.is_finite()) {
        return Err(ArimaError::Data(
            "ARIMA estimation requires 5..=512 finite observations".into(),
        ));
    }
    if spec.d > 2 || spec.ar.len() > MAX_ORDER || spec.ma.len() > MAX_ORDER {
        return Err(ArimaError::Settings(
            "GAP: estimation supports d=1, p/q <= 1".into(),
        ));
    }
    let z = transform(spec.transform, y)?;
    let diff = spec.difference_polynomial()?;
    let w: Vec<f64> = filter_difference(&z, &diff)
        .iter()
        .map(|v| v - spec.constant)
        .collect();
    let gamma = arma_gamma(&spec.ar, &spec.ma, w.len().saturating_add(2))?;
    let (hats, variances) = innovations(&w, &gamma)?;
    let mut residuals = Vec::with_capacity(w.len());
    let mut log_det = 0.0;
    for ((value, hat), variance) in w.iter().zip(hats).zip(variances) {
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
