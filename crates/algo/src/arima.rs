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

use crate::stats::student_t_quantile;
use std::fmt;

const MAX_HISTORY: usize = 100_000;
const MAX_HORIZON: usize = 10_000;
const MAX_ORDER: usize = 64;
const MAX_PSI: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    None,
    Log,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArimaSpec {
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
    if z.len() <= spec.d {
        return Err(ArimaError::Data(
            "Time Series series is shorter than the differencing order".into(),
        ));
    }
    let differenced = difference(&z, spec.d);
    let w: Vec<f64> = differenced.iter().map(|v| v - spec.constant).collect();
    let gamma_lags = w.len().saturating_add(horizon).saturating_add(2).max(2);
    let gamma = arma_gamma(&spec.ar, &spec.ma, gamma_lags)?;
    let (hats, v) = innovations(&w, &gamma)?;
    let psi = integrated_psi(spec, horizon.saturating_add(1))?;

    let mut fitted = vec![None; z.len()];
    for (t, slot) in fitted.iter_mut().enumerate().skip(spec.d) {
        let w_index = t.saturating_sub(spec.d);
        let what = *hats.get(w_index).ok_or_else(|| {
            ArimaError::Data("Time Series innovations is short of the history".into())
        })?;
        let zhat = undifference_one(spec.d, &z, t, spec.constant, what)?;
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
    let w_forecasts = future_w(spec, &w, &hats, horizon)?;
    let mut last_z = *z
        .last()
        .ok_or_else(|| ArimaError::Data("Time Series series is empty".into()))?;
    for (h, what) in w_forecasts.iter().enumerate() {
        let zhat = last_z + spec.constant + what;
        last_z = zhat;
        let sum_psi2: f64 = psi.iter().take(h.saturating_add(1)).map(|p| p * p).sum();
        let se = (spec.sigma2 * sum_psi2).sqrt();
        forecasts.push(back_transform(spec.transform, zhat, se, tcrit)?);
    }
    Ok(SeriesScore { fitted, forecasts })
}

impl ArimaSpec {
    fn check(&self) -> Result<(), ArimaError> {
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

pub(crate) fn transform(kind: Transform, y: &[f64]) -> Result<Vec<f64>, ArimaError> {
    match kind {
        Transform::None => Ok(y.to_vec()),
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

fn undifference_one(
    d: usize,
    z: &[f64],
    t: usize,
    constant: f64,
    what: f64,
) -> Result<f64, ArimaError> {
    if d != 1 {
        return Err(ArimaError::Settings(format!(
            "GAP: ARIMA differencing order {d} is not implemented (oracle cases use d=1)"
        )));
    }
    let prev = z.get(t.saturating_sub(1)).copied().ok_or_else(|| {
        ArimaError::Data("Time Series differencing needs the previous observation".into())
    })?;
    Ok(prev + constant + what)
}

fn back_transform(kind: Transform, zhat: f64, se: f64, tcrit: f64) -> Result<Forecast, ArimaError> {
    if !zhat.is_finite() || !se.is_finite() || se < 0.0 {
        return Err(ArimaError::Data(
            "Time Series prediction is not finite".into(),
        ));
    }
    match kind {
        Transform::None => Ok(Forecast {
            point: zhat,
            lower: zhat - tcrit * se,
            upper: zhat + tcrit * se,
        }),
        Transform::Log => {
            let point = (zhat + se * se / 2.0).exp();
            Ok(Forecast {
                point,
                lower: (zhat - tcrit * se).exp(),
                upper: (zhat + tcrit * se).exp(),
            })
        }
    }
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
    let extra = 400usize;
    let psi_len = lags
        .saturating_add(extra)
        .min(MAX_PSI)
        .max(lags.saturating_add(1));
    let psi = arma_psi(phi, theta, psi_len)?;
    let mut gamma = Vec::with_capacity(lags);
    for k in 0..lags {
        let mut g = 0.0;
        let last = psi.len().saturating_sub(k);
        for j in 0..last {
            let a = psi.get(j).copied().unwrap_or(0.0);
            let b = psi.get(j.saturating_add(k)).copied().unwrap_or(0.0);
            g += a * b;
        }
        if !g.is_finite() {
            return Err(ArimaError::Data("ARMA autocovariance is not finite".into()));
        }
        gamma.push(g);
    }
    Ok(gamma)
}

/// Exact concentrated Gaussian likelihood ingredients using the scoring recursion.
/// Returns standardized innovations and log determinant (innovation variance at sigma²=1).
pub fn likelihood_residuals(spec: &ArimaSpec, y: &[f64]) -> Result<(Vec<f64>, f64), ArimaError> {
    spec.check()?;
    if y.len() < 5 || y.len() > 512 || y.iter().any(|v| !v.is_finite()) {
        return Err(ArimaError::Data(
            "ARIMA estimation requires 5..=512 finite observations".into(),
        ));
    }
    if spec.d != 1 || spec.ar.len() > 1 || spec.ma.len() > 1 {
        return Err(ArimaError::Settings(
            "GAP: estimation supports d=1, p/q <= 1".into(),
        ));
    }
    let z = transform(spec.transform, y)?;
    let w: Vec<f64> = difference(&z, spec.d)
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

/// Innovations algorithm. `hats[i]` predicts `w[i]` (`hats[0] = 0`); `hats[n]`
/// is the one-step predictor beyond the sample. `v[i]` is its MSE with σ² = 1.
fn innovations(w: &[f64], gamma: &[f64]) -> Result<(Vec<f64>, Vec<f64>), ArimaError> {
    let n = w.len();
    let mut v = vec![0.0; n.saturating_add(1)];
    let g0 = *gamma
        .first()
        .ok_or_else(|| ArimaError::Data("ARMA autocovariance is empty".into()))?;
    if let Some(slot) = v.first_mut() {
        *slot = g0;
    }
    let mut hats = vec![0.0; n.saturating_add(1)];
    let mut thetas: Vec<Vec<f64>> = Vec::with_capacity(n);
    for n_obs in 1..=n {
        let mut th = vec![0.0; n_obs.saturating_add(1)];
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
        thetas.push(th.clone());
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
    Ok((hats, v))
}

fn integrated_psi(spec: &ArimaSpec, hmax: usize) -> Result<Vec<f64>, ArimaError> {
    let hmax = hmax.clamp(1, MAX_PSI);
    let mut ar_poly = vec![1.0];
    for coef in &spec.ar {
        ar_poly.push(-coef);
    }
    for _ in 0..spec.d {
        let mut next = vec![0.0; ar_poly.len().saturating_add(1)];
        for (i, a) in ar_poly.iter().enumerate() {
            if let Some(slot) = next.get_mut(i) {
                *slot += a;
            }
            if let Some(slot) = next.get_mut(i.saturating_add(1)) {
                *slot -= a;
            }
        }
        ar_poly = next;
    }
    let mut ma_poly = vec![1.0];
    ma_poly.extend_from_slice(&spec.ma);
    let mut psi = vec![0.0; hmax];
    if let Some(slot) = psi.first_mut() {
        *slot = 1.0;
    }
    for k in 1..hmax {
        let mut acc = ma_poly.get(k).copied().unwrap_or(0.0);
        let last = k.min(ar_poly.len().saturating_sub(1));
        for i in 1..=last {
            let ar_i = ar_poly.get(i).copied().unwrap_or(0.0);
            let psi_prev = psi.get(k.saturating_sub(i)).copied().unwrap_or(0.0);
            acc -= ar_i * psi_prev;
        }
        if let Some(slot) = psi.get_mut(k) {
            *slot = acc;
        }
    }
    Ok(psi)
}

fn future_w(
    spec: &ArimaSpec,
    w: &[f64],
    hats: &[f64],
    horizon: usize,
) -> Result<Vec<f64>, ArimaError> {
    let mut out = Vec::with_capacity(horizon);
    let residuals: Vec<f64> = w
        .iter()
        .enumerate()
        .map(|(i, value)| value - hats.get(i).copied().unwrap_or(0.0))
        .collect();
    for h in 1..=horizon {
        let wf = if h == 1 {
            hats.get(w.len()).copied().unwrap_or(0.0)
        } else {
            let mut acc = 0.0;
            for (i, phi) in spec.ar.iter().enumerate() {
                let idx = w
                    .len()
                    .saturating_add(h)
                    .saturating_sub(1)
                    .saturating_sub(i.saturating_add(1));
                let val = if idx < w.len() {
                    w.get(idx).copied().unwrap_or(0.0)
                } else {
                    out.get(idx.saturating_sub(w.len())).copied().unwrap_or(0.0)
                };
                acc += phi * val;
            }
            for j in h..=spec.ma.len() {
                let idx = w
                    .len()
                    .saturating_add(h)
                    .saturating_sub(1)
                    .saturating_sub(j);
                if let Some(a) = residuals.get(idx) {
                    let theta = spec.ma.get(j.saturating_sub(1)).copied().unwrap_or(0.0);
                    acc += theta * a;
                }
            }
            acc
        };
        out.push(wf);
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::excessive_precision, reason = "oracle CSV prints 15 decimals")]
mod tests {
    use super::*;

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
}
