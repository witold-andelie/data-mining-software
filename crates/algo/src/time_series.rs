//! Time Series estimation: statistical parity (optimizer).
//!
//! Evidence: IBM SPSS Modeler Algorithms Guide (1994–2016 edition), Time Series
//! pp. 379–386, 392–394; `models_next/ts_arima`, `ts_expert`, and
//! `models_next2/ts_simple`, `ts_holt`. Smoothing minimizes one-step SSE after
//! back-casting from the last observation (negative regression slope for Holt).
//! ARIMA minimizes exact concentrated Gaussian negative log likelihood using
//! the innovations in `arima`, with admissibility-constrained Levenberg–Marquardt.
//! The geometric-mean variance multiplier turns the concentrated likelihood
//! into a monotone least-squares objective. Variance is S/(n-k), excluding sigma²
//! from k. All recursions belong to `smooth` / `arima`, also used by the scorer.
//!
//! Only simple/Holt (no transform) and constant ARIMA(1,1,1) / (0,1,0)
//! (none/log) are evidenced. Expert automatic transformation/order selection is
//! GAP: the guide's flowchart does not specify its thresholds. The executor can
//! refit an Expert model's recorded selected specification, never its coefficients.
//! Modeler's coefficients are reported, not asserted equal: the smoothing data
//! are exact lines, with a boundary optimum for simple and nonunique Holt weights.

use crate::{AlgoError, arima, smooth};
use arima::{ArimaSpec, Transform};
use smooth::SmoothSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Simple,
    Holt,
    Arima {
        p: usize,
        d: usize,
        q: usize,
        transform: Transform,
    },
}

#[derive(Debug, Clone)]
pub enum FittedSpec {
    Smooth(SmoothSpec),
    Arima(ArimaSpec),
}

#[derive(Debug, Clone)]
pub struct Fit {
    pub spec: FittedSpec,
    /// SSE for smoothing, concentrated Gaussian negative log likelihood for ARIMA.
    pub objective: f64,
    pub iterations: usize,
    pub final_level: f64,
    pub final_trend: f64,
    pub rmse: f64,
}

fn data(message: impl ToString) -> AlgoError {
    AlgoError::Data(message.to_string())
}

fn validate(y: &[f64]) -> Result<(), AlgoError> {
    if !(5..=512).contains(&y.len()) || y.iter().any(|v| !v.is_finite()) {
        return Err(data(
            "Time Series estimation requires 5..=512 finite observations",
        ));
    }
    Ok(())
}

fn count(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Estimate from observations alone; no saved coefficients or oracle values enter this API.
pub fn fit(y: &[f64], method: Method) -> Result<Fit, AlgoError> {
    validate(y)?;
    match method {
        Method::Simple | Method::Holt => fit_smooth(y, method == Method::Holt),
        Method::Arima { p, d, q, transform } => {
            if d != 1 || !matches!((p, q), (0, 0) | (1, 1)) {
                return Err(AlgoError::Unsupported(
                    "GAP: ARIMA estimation orders outside (0,1,0)/(1,1,1)".into(),
                ));
            }
            fit_arima(y, p, q, transform)
        }
    }
}

/// Objective at arbitrary saved coefficients, on exactly the same observations as training.
pub fn objective(spec: &FittedSpec, y: &[f64]) -> Result<f64, AlgoError> {
    validate(y)?;
    match spec {
        FittedSpec::Smooth(spec) => {
            let (points, _, _) = smooth::recurse(spec, y).map_err(data)?;
            Ok(points.iter().zip(y).map(|(a, b)| (a - b).powi(2)).sum())
        }
        FittedSpec::Arima(spec) => {
            let (r, log_det) = arima::likelihood_residuals(spec, y).map_err(data)?;
            let n = count(r.len());
            let s: f64 = r.iter().map(|v| v * v).sum();
            if s <= 0.0 || !s.is_finite() {
                return Err(data("ARIMA likelihood has zero or nonfinite variance"));
            }
            Ok(0.5 * (n * ((2.0 * std::f64::consts::PI).ln() + 1.0 + (s / n).ln()) + log_det))
        }
    }
}

fn smooth_spec(y: &[f64], holt: bool, weights: &[f64]) -> Result<SmoothSpec, AlgoError> {
    let alpha = weights
        .first()
        .copied()
        .ok_or_else(|| data("missing alpha"))?;
    let gamma = weights.get(1).copied().unwrap_or(0.0);
    let n = count(y.len());
    let mean = y.iter().sum::<f64>() / n;
    let mid = (n - 1.0) / 2.0;
    let slope = y
        .iter()
        .enumerate()
        .map(|(i, v)| (count(i) - mid) * (v - mean))
        .sum::<f64>()
        / (n * (n * n - 1.0) / 12.0);
    let last = y.last().copied().ok_or_else(|| data("empty series"))?;
    let df = n - if holt { 2.0 } else { 1.0 };
    let make = |level, trend| {
        if holt {
            SmoothSpec::Holt {
                alpha,
                gamma,
                init_level: level,
                init_trend: trend,
                sigma2: 0.0,
                df,
            }
        } else {
            SmoothSpec::Simple {
                alpha,
                init_level: level,
                sigma2: 0.0,
                df,
            }
        }
    };
    // The state at n is y_n. Assimilate n-1,...,1, then predict back to 0.
    let reverse: Vec<f64> = y.iter().rev().skip(1).copied().collect();
    let (_, level, trend) = smooth::recurse(&make(last, -slope), &reverse).map_err(data)?;
    Ok(make(if holt { level + trend } else { level }, -trend))
}

fn fit_smooth(y: &[f64], holt: bool) -> Result<Fit, AlgoError> {
    let initial = if holt { vec![0.8, 0.4] } else { vec![0.8] };
    let bounds = vec![(0.0, 1.0); initial.len()];
    let residuals = |x: &[f64]| {
        let spec = smooth_spec(y, holt, x)?;
        let (points, _, _) = smooth::recurse(&spec, y).map_err(data)?;
        Ok(points.iter().zip(y).map(|(a, b)| a - b).collect::<Vec<_>>())
    };
    let (weights, iterations) = lm(initial, &bounds, residuals)?;
    let mut spec = smooth_spec(y, holt, &weights)?;
    let sse = objective(&FittedSpec::Smooth(spec.clone()), y)?;
    let df = count(y.len()) - if holt { 2.0 } else { 1.0 };
    match &mut spec {
        SmoothSpec::Simple { sigma2, .. } | SmoothSpec::Holt { sigma2, .. } => *sigma2 = sse / df,
    }
    let (_, final_level, final_trend) = smooth::recurse(&spec, y).map_err(data)?;
    Ok(Fit {
        spec: FittedSpec::Smooth(spec),
        objective: sse,
        iterations,
        final_level,
        final_trend,
        rmse: (sse / df).sqrt(),
    })
}

fn fit_arima(y: &[f64], p: usize, q: usize, transform: Transform) -> Result<Fit, AlgoError> {
    let w = arima::difference(&arima::transform(transform, y).map_err(data)?, 1);
    let n = count(w.len());
    let mean = w.iter().sum::<f64>() / n;
    let mut initial = vec![mean];
    if p == 1 {
        // Guide pp.385–386: Yule–Walker AR(p) and AR(p+q) approximations,
        // followed by the u_(t+l),a_t cross covariance for MA initialization.
        let c0 = w.iter().map(|v| (v - mean).powi(2)).sum::<f64>();
        if c0 <= 0.0 {
            return Err(data("ARIMA needs nonzero differenced variance"));
        }
        let rho = |lag: usize| {
            w.iter()
                .skip(lag)
                .zip(&w)
                .map(|(a, b)| (a - mean) * (b - mean))
                .sum::<f64>()
                / c0
        };
        let r1 = rho(1);
        let r2 = rho(2);
        // Modified Yule–Walker at lag q+1: rho(2) = phi*rho(1).
        let phi = if r1.abs() > 1e-12 {
            (r2 / r1).clamp(-0.95, 0.95)
        } else {
            0.0
        };
        let a2 = (r2 - r1 * r1) / (1.0 - r1 * r1);
        let a1 = r1 * (1.0 - a2);
        let autocorr = |lag: usize| match lag {
            0 => 1.0,
            1 => r1,
            2 => r2,
            _ => 0.0,
        };
        let ar2 = [-1.0, a1, a2];
        let variance: f64 = ar2
            .iter()
            .enumerate()
            .flat_map(|(i, a)| {
                ar2.iter()
                    .enumerate()
                    .map(move |(j, b)| a * b * autocorr(i.abs_diff(j)))
            })
            .sum();
        let lambda = r1 - a1 * r2 - phi + phi * (a1 * r1 + a2 * r2);
        initial.push(phi.clamp(-0.95, 0.95));
        initial.push((lambda / variance).clamp(-0.95, 0.95)); // internal MA sign is plus
    }
    let make = |x: &[f64]| ArimaSpec {
        transform,
        constant: x.first().copied().unwrap_or(0.0),
        d: 1,
        ar: if p == 0 {
            vec![]
        } else {
            vec![x.get(1).copied().unwrap_or(0.0)]
        },
        ma: if q == 0 {
            vec![]
        } else {
            vec![x.get(2).copied().unwrap_or(0.0)]
        },
        sigma2: 1.0,
        df: n - count(1 + p + q),
    };
    let mut bounds = vec![(f64::NEG_INFINITY, f64::INFINITY)];
    bounds.extend(vec![(-1.0 + 1e-10, 1.0 - 1e-10); p + q]);
    let residuals = |x: &[f64]| {
        let (r, log_det) = arima::likelihood_residuals(&make(x), y).map_err(data)?;
        let scale = (log_det / (2.0 * n)).exp();
        Ok(r.iter().map(|v| v * scale).collect::<Vec<_>>())
    };
    let (parameters, iterations) = lm(initial, &bounds, residuals)?;
    let mut spec = make(&parameters);
    let (r, _) = arima::likelihood_residuals(&spec, y).map_err(data)?;
    spec.sigma2 = r.iter().map(|v| v * v).sum::<f64>() / spec.df;
    let objective = objective(&FittedSpec::Arima(spec.clone()), y)?;
    let scores = arima::score(&spec, y, 0, 95.0).map_err(data)?;
    let sse: f64 = scores
        .fitted
        .iter()
        .zip(y)
        .filter_map(|(f, v)| f.map(|f| (f.point - v).powi(2)))
        .sum();
    let rmse = (sse / spec.df).sqrt();
    Ok(Fit {
        spec: FittedSpec::Arima(spec),
        objective,
        iterations,
        final_level: 0.0,
        final_trend: 0.0,
        rmse,
    })
}

/// Small, bounded Levenberg–Marquardt solver. Reject nonfinite/inadmissible steps,
/// use central numerical derivatives, and explicitly fail instead of returning a
/// silently unconverged model when the work budget is exhausted.
fn lm(
    mut x: Vec<f64>,
    bounds: &[(f64, f64)],
    raw_residuals: impl Fn(&[f64]) -> Result<Vec<f64>, AlgoError>,
) -> Result<(Vec<f64>, usize), AlgoError> {
    let mut lambda: f64 = 1e-3;
    let norm = |r: &[f64]| r.iter().map(|v| v * v).sum::<f64>();
    let initial = raw_residuals(&x)?;
    let scale = norm(&initial).sqrt();
    if scale == 0.0 {
        return Ok((x, 0));
    }
    if !scale.is_finite() {
        return Err(data("nonfinite Time Series objective"));
    }
    // A fixed residual scale preserves the minimizer and makes the convergence
    // tolerances independent of the units of the observed series.
    let residuals =
        |x: &[f64]| raw_residuals(x).map(|r| r.into_iter().map(|v| v / scale).collect::<Vec<_>>());
    let mut r: Vec<_> = initial.iter().map(|v| v / scale).collect();
    let mut value = norm(&r);
    for iteration in 0..400 {
        if !value.is_finite() {
            return Err(data("nonfinite Time Series objective"));
        }
        let mut jac = Vec::new();
        for (j, (lo, hi)) in bounds.iter().enumerate() {
            let v = x
                .get(j)
                .copied()
                .ok_or_else(|| data("optimizer dimension"))?;
            let h = 1e-5 * (1.0 + v.abs());
            let mut plus = x.clone();
            let mut minus = x.clone();
            let a = (v + h).min(*hi);
            let b = (v - h).max(*lo);
            if let Some(slot) = plus.get_mut(j) {
                *slot = a;
            }
            if let Some(slot) = minus.get_mut(j) {
                *slot = b;
            }
            let rp = residuals(&plus)?;
            let rm = residuals(&minus)?;
            jac.push(
                rp.iter()
                    .zip(rm)
                    .map(|(a0, b0)| (a0 - b0) / (a - b))
                    .collect::<Vec<_>>(),
            );
        }
        let size = x.len();
        let mut matrix = vec![vec![0.0; size]; size];
        let mut gradient = vec![0.0; size];
        for (i, ji) in jac.iter().enumerate() {
            if let Some(g) = gradient.get_mut(i) {
                *g = ji.iter().zip(&r).map(|(a, b)| a * b).sum();
            }
            for (j, jj) in jac.iter().enumerate() {
                if let Some(cell) = matrix.get_mut(i).and_then(|row| row.get_mut(j)) {
                    *cell = ji.iter().zip(jj).map(|(a, b)| a * b).sum();
                }
            }
        }
        let projected = gradient
            .iter()
            .zip(&x)
            .zip(bounds)
            .map(|((g, v), (lo, hi))| {
                if (*v <= *lo && *g > 0.0) || (*v >= *hi && *g < 0.0) {
                    0.0
                } else {
                    g.abs()
                }
            })
            .fold(0.0, f64::max);
        if projected <= 1e-9 * (1.0 + value) {
            return Ok((x, iteration));
        }
        for (i, row) in matrix.iter_mut().enumerate() {
            if let Some(v) = row.get_mut(i) {
                *v += lambda * v.abs().max(1e-10);
            }
        }
        let step = solve(matrix, gradient)?;
        let next: Vec<f64> = x
            .iter()
            .zip(step)
            .zip(bounds)
            .map(|((v, s), (lo, hi))| (v - s).clamp(*lo, *hi))
            .collect();
        let candidate = residuals(&next);
        if let Ok(rr) = candidate {
            let new_value = norm(&rr);
            if new_value < value {
                let improvement = value - new_value;
                x = next;
                r = rr;
                value = new_value;
                lambda = (lambda / 3.0).max(1e-12);
                if improvement < 1e-13 * (1.0 + value) {
                    return Ok((x, iteration + 1));
                }
                continue;
            }
        }
        lambda *= 10.0;
        if lambda > 1e15 {
            if projected < 1e-6 * (1.0 + value) {
                return Ok((x, iteration + 1));
            }
            return Err(data("Time Series optimizer stalled before convergence"));
        }
    }
    Err(data("Time Series optimizer exceeded 400 iterations"))
}

fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Result<Vec<f64>, AlgoError> {
    let n = b.len();
    for i in 0..n {
        let pivot = (i..n)
            .max_by(|j, k| {
                let at = |j: usize| {
                    a.get(j)
                        .and_then(|r| r.get(i))
                        .copied()
                        .unwrap_or(0.0)
                        .abs()
                };
                at(*j).total_cmp(&at(*k))
            })
            .ok_or_else(|| data("empty normal equation"))?;
        a.swap(i, pivot);
        b.swap(i, pivot);
        let row = a
            .get(i)
            .cloned()
            .ok_or_else(|| data("normal equation row"))?;
        let divisor = row.get(i).copied().unwrap_or(0.0);
        if divisor.abs() < 1e-30 || !divisor.is_finite() {
            return Err(data("singular normal equation"));
        }
        let rhs = b.get(i).copied().unwrap_or(0.0) / divisor;
        for j in 0..n {
            if j == i {
                continue;
            }
            let scale = a.get(j).and_then(|r| r.get(i)).copied().unwrap_or(0.0) / divisor;
            if let Some(other) = a.get_mut(j) {
                for (v, p) in other.iter_mut().zip(&row) {
                    *v -= scale * p;
                }
            }
            if let Some(v) = b.get_mut(j) {
                *v -= scale * rhs * divisor;
            }
        }
    }
    b.iter()
        .enumerate()
        .map(|(i, v)| {
            let d = a.get(i).and_then(|r| r.get(i)).copied().unwrap_or(0.0);
            let x = v / d;
            if x.is_finite() {
                Ok(x)
            } else {
                Err(data("nonfinite optimizer step"))
            }
        })
        .collect()
}

impl Fit {
    /// One PMML TimeSeriesModel, consumed by the same parser as a saved nugget.
    pub fn to_xml(&self, target: &str, n: usize) -> String {
        let target = target
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('"', "&quot;")
            .replace('\r', "&#13;")
            .replace('\n', "&#10;")
            .replace('\t', "&#9;");
        let (algorithm, body, variance, df) = match &self.spec {
            FittedSpec::Smooth(spec) => {
                let (alpha, gamma, level, trend, variance, df) = match spec {
                    SmoothSpec::Simple {
                        alpha,
                        init_level,
                        sigma2,
                        df,
                    } => (*alpha, None, *init_level, 0.0, *sigma2, *df),
                    SmoothSpec::Holt {
                        alpha,
                        gamma,
                        init_level,
                        init_trend,
                        sigma2,
                        df,
                    } => (*alpha, Some(*gamma), *init_level, *init_trend, *sigma2, *df),
                };
                let trend_xml=gamma.map(|g|format!("<Trend_ExpoSmooth trend=\"additive\" gamma=\"{g}\" smoothedValue=\"{}\"/>",self.final_trend)).unwrap_or_default();
                let initial_trend_xml = gamma
                    .map(|_| format!("<Extension name=\"initTrend\" value=\"{trend}\"/>"))
                    .unwrap_or_default();
                let body = format!(
                    "<ExponentialSmoothing transformation=\"none\" RMSE=\"{}\"><Level alpha=\"{alpha}\" smoothedValue=\"{}\"/>{trend_xml}</ExponentialSmoothing><Extension name=\"initLevel\" value=\"{level}\"/>{initial_trend_xml}",
                    self.rmse, self.final_level
                );
                ("ExponentialSmoothing", body, variance, df)
            }
            FittedSpec::Arima(spec) => {
                let transform = if spec.transform == Transform::Log {
                    "logarithmic"
                } else {
                    "none"
                };
                let array = |tag: &str, v: &[f64], sign: f64| {
                    format!(
                        "<{tag}><Array n=\"{}\" type=\"real\">{}</Array></{tag}>",
                        v.len(),
                        v.iter()
                            .map(|v| (v * sign).to_string())
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                };
                let body = format!(
                    "<Extension><ARIMA RMSE=\"{}\" transformation=\"{transform}\" constantTerm=\"{}\" seasonLength=\"1\"><NonseasonalComponent p=\"{}\" d=\"{}\" q=\"{}\">{}{}</NonseasonalComponent><SeasonalComponent p=\"0\" d=\"0\" q=\"0\"/></ARIMA></Extension>",
                    self.rmse,
                    spec.constant,
                    spec.ar.len(),
                    spec.d,
                    spec.ma.len(),
                    array("AR", &spec.ar, 1.0),
                    array("MA", &spec.ma, -1.0)
                );
                ("ARIMA", body, spec.sigma2, spec.df)
            }
        };
        format!(
            "<TimeSeriesModel modelName=\"{target}\" functionName=\"timeSeries\" algorithmName=\"{algorithm}\" bestFit=\"{algorithm}\"><MiningSchema><MiningField name=\"{target}\" usageType=\"target\"/></MiningSchema>{body}<Extension name=\"errVariance\" value=\"{variance}\"/><Extension name=\"degreesOfFreedom\" value=\"{df}\"/><Extension name=\"periodStartIndex\" value=\"0\"/><Extension name=\"periodLength\" value=\"{n}\"/></TimeSeriesModel>"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backcast_line_and_boundary_optimum() {
        let y: Vec<_> = (1..=48).map(|i| 20.0 + 0.5 * f64::from(i)).collect();
        let simple = fit(&y, Method::Simple).unwrap();
        assert!((simple.objective - 11.75).abs() < 1e-10);
        let holt = fit(&y, Method::Holt).unwrap();
        assert!(holt.objective < 1e-24);
        assert!((holt.final_trend - 0.5).abs() < 1e-12);
    }
    #[test]
    fn invalid_data_and_orders_fail() {
        for y in [vec![], vec![f64::NAN; 10], vec![1.0; 513]] {
            assert!(fit(&y, Method::Simple).is_err());
        }
        assert!(
            fit(
                &[1.0; 10],
                Method::Arima {
                    p: 999,
                    d: 1,
                    q: 1,
                    transform: Transform::None
                }
            )
            .is_err()
        );
        assert!(
            fit(
                &[-1.0; 10],
                Method::Arima {
                    p: 0,
                    d: 1,
                    q: 0,
                    transform: Transform::Log
                }
            )
            .is_err()
        );
    }

    #[test]
    fn smoothing_convergence_is_independent_of_units() {
        for scale in [1e-10, 1.0, 1e10] {
            let y: Vec<_> = (1..=48)
                .map(|i| scale * (20.0 + 0.5 * f64::from(i)))
                .collect();
            let fit = fit(&y, Method::Simple).unwrap();
            let FittedSpec::Smooth(SmoothSpec::Simple { alpha, .. }) = fit.spec else {
                panic!("simple")
            };
            assert_eq!(alpha, 1.0);
            assert!((fit.objective / (scale * scale) - 11.75).abs() < 1e-10);
        }
    }
}
