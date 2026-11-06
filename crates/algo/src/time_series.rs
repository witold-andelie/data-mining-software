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
//! Phase B evidence: `ts_more` (30 saved streams), 28 satisfy optimizer rule 3
//! in `exec/tests/time_series_more.rs`: converged SSE/likelihood no worse than
//! saved Modeler parameters (1e-9 relative), all scored points, bounds and
//! residuals within 0.01 target sample SD (worst accepted: 0.008378 SD).
//! Supports d=0, seasonal ARIMA products, simple/Holt/Brown/damped/seasonal/
//! Winters smoothing, log/sqrt, and transfer functions with numerator and denominator
//! (`transfer`, whose docs give the denominator start-up evidence).
//! Expert refits recorded ARIMA/AllModels structure including omitted AR lags, and
//! selects untransformed Exponential Smoothing models by minimum NBIC = ln(SSE/(n-k)) +
//! k*ln(n)/n; automatic ARIMA/AllModels transform/order selection remains GAP (the guide
//! omits decision thresholds).
//! The four keyed ARIMA transform recordings have different saved and rebuilt
//! specifications: detached-model refits are tested separately from linked scores.
//! Two cases stay outside rule 3 because Modeler's estimate is not the optimum of the
//! documented exact likelihood; ours is better and is kept:
//! - `ts_tf_static`: an independent exact AR(1) likelihood (SciPy) has its optimum at our
//!   (81.1180, 0.15312, -0.16418); Modeler's saved (81.0875, 0.14865, -0.15936) has
//!   gradient (0.0003, -0.056, 0.21) and a 6.3e-4 worse -loglik; scores differ 1.64 % SD.
//! - `ts_sarlog_noconst`: the saved builder lacks the scripted keyed transform, so Modeler's
//!   own rebuild (which the oracle's tables show) is untransformed, like ours. Its fitted
//!   values give phi 0.988174, Phi 0.524296 exactly (residual 2e-15); ours 0.988068,
//!   0.524038 has a 7.3e-5 better -loglik on an independent exact likelihood. Near the unit
//!   root that moves the six-step interval bound by 11.7 % SD.
//!
//! Passing the objective alone is not acceptance.
//! Other limits: <=512 fitting observations, monthly regular dates, bounded
//! orders; no outlier detection, missing values, aggregation or automatic Expert.

use crate::{AlgoError, arima, smooth};
use arima::{ArimaSpec, Transform};
use smooth::SmoothSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Simple,
    Holt,
    Smooth {
        kind: smooth::Kind,
        transform: Transform,
        period: usize,
    },
    ExpertSmooth {
        period: usize,
    },
    SeasonalArima(ArimaOrder),
    /// Zero coefficients in a recorded subset ARIMA denote omitted lags.
    SparseArima {
        order: ArimaOrder,
        zero_ar: u64,
        zero_ma: u64,
    },
    Arima {
        p: usize,
        d: usize,
        q: usize,
        transform: Transform,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArimaOrder {
    pub p: usize,
    pub d: usize,
    pub q: usize,
    pub sp: usize,
    pub sd: usize,
    pub sq: usize,
    pub period: usize,
    pub constant: bool,
    pub transform: Transform,
}

#[derive(Debug, Clone)]
pub enum FittedSpec {
    Transfer(crate::transfer::TransferSpec),
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
        Method::SeasonalArima(order) => fit_seasonal_arima(y, order, 0, 0),
        Method::SparseArima {
            order,
            zero_ar,
            zero_ma,
        } => fit_seasonal_arima(y, order, zero_ar, zero_ma),
        Method::Smooth {
            kind,
            transform,
            period,
        } => fit_general_smooth(y, kind, transform, period),
        Method::ExpertSmooth { period } => fit_expert_smooth(y, period),
        Method::Simple | Method::Holt => fit_smooth(y, method == Method::Holt),
        Method::Arima { p, d, q, transform } => {
            if d > 1 || p > 1 || q > 1 {
                return Err(AlgoError::Unsupported(
                    "GAP: ARIMA estimation orders outside (0,1,0)/(1,1,1)".into(),
                ));
            }
            fit_arima(y, p, d, q, transform)
        }
    }
}

/// Objective at arbitrary saved coefficients, on exactly the same observations as training.
pub fn objective(spec: &FittedSpec, y: &[f64]) -> Result<f64, AlgoError> {
    validate(y)?;
    match spec {
        FittedSpec::Transfer(_) => Err(data("transfer objective requires predictor observations")),
        FittedSpec::Smooth(spec) => {
            if let SmoothSpec::General(g) = spec {
                let z = arima::transform(g.transform, y).map_err(data)?;
                let (p, _) = g.run(&z).map_err(data)?;
                return Ok(p.iter().zip(z).map(|(a, b)| (a - b).powi(2)).sum());
            }
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
        SmoothSpec::General(_) => return Err(data("general smoothing dispatch")),
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

fn general_initial(
    z: &[f64],
    kind: smooth::Kind,
    transform: Transform,
    period: usize,
    x: &[f64],
) -> Result<smooth::GeneralSpec, AlgoError> {
    use smooth::{GeneralSpec, Kind, State};
    if kind.seasonal() && (!(2..=64).contains(&period) || z.len() <= period) {
        return Err(data(
            "seasonal smoothing needs observations exceeding one period",
        ));
    }
    let n = count(z.len());
    let mean = z.iter().sum::<f64>() / n;
    let mid = (n - 1.0) / 2.0;
    let mut slope = if kind.trend() {
        z.iter()
            .enumerate()
            .map(|(i, y)| (count(i) - mid) * (y - mean))
            .sum::<f64>()
            / (n * (n * n - 1.0) / 12.0)
    } else {
        0.0
    };
    let mut seasons = Vec::new();
    if kind.seasonal() {
        let mut groups = Vec::new();
        let mut cross = 0.0;
        let mut squares = 0.0;
        for j in 0..period {
            let group: Vec<_> = z
                .iter()
                .enumerate()
                .skip(j)
                .step_by(period)
                .map(|(i, y)| (count(i), *y))
                .collect();
            let size = count(group.len());
            let t = group.iter().map(|(t, _)| t).sum::<f64>() / size;
            let v = group.iter().map(|(_, v)| v).sum::<f64>() / size;
            let xx = group.iter().map(|(a, _)| (a - t).powi(2)).sum::<f64>();
            let xy = group.iter().map(|(a, b)| (a - t) * (b - v)).sum::<f64>();
            cross += xy;
            squares += xx;
            groups.push((t, v, if xx > 0.0 { xy / xx } else { 0.0 }));
        }
        if kind == Kind::WintersAdd {
            slope = if squares > 0.0 { cross / squares } else { 0.0 };
        }
        if kind == Kind::WintersMul {
            slope = groups.iter().map(|(_, _, s)| s).sum::<f64>() / count(period);
        }
        seasons = groups
            .iter()
            .map(|(t, v, s)| match kind {
                Kind::Seasonal => *v,
                Kind::WintersAdd => v - slope * t,
                _ => v - s * t,
            })
            .collect();
        // Algorithms Guide p. 381: simple seasonal subtracts the sample mean;
        // Winters additive/multiplicative centers/normalizes across the m seasonal indices.
        let center = if kind == Kind::Seasonal {
            mean
        } else {
            seasons.iter().sum::<f64>() / count(period)
        };
        for v in &mut seasons {
            if kind == Kind::WintersMul {
                *v /= center;
            } else {
                *v -= center;
            }
        }
    }
    let alpha = x.first().copied().ok_or_else(|| data("missing alpha"))?;
    let mut at = 1;
    let gamma = if kind.trend() && kind != Kind::Brown {
        at += 1;
        x.get(1).copied().unwrap_or(0.0)
    } else {
        0.0
    };
    let phi = if kind == Kind::Damped {
        at += 1;
        x.get(at - 1).copied().unwrap_or(1.0)
    } else {
        1.0
    };
    let delta = if kind.seasonal() {
        x.get(at).copied().unwrap_or(0.0)
    } else {
        0.0
    };
    let mut g = GeneralSpec {
        kind,
        transform,
        alpha,
        gamma,
        phi,
        delta,
        initial: State {
            level: z.last().copied().ok_or_else(|| data("empty history"))?,
            trend: -slope,
            seasons,
        },
        sigma2: 0.0,
        df: n - count(kind.parameters()),
    };
    let mut state = g.initial.clone();
    // Include t=n, then predict/assimilate t=0 with zero innovation. Reversing
    // time changes the trend sign, after the damped/Brown state update at zero.
    for (i, y) in z.iter().enumerate().rev() {
        g.step(&mut state, Some(*y), i).map_err(data)?;
    }
    let zero = if kind.seasonal() { period - 1 } else { 0 };
    g.step(&mut state, None, zero).map_err(data)?;
    state.trend = -state.trend;
    g.initial = state;
    Ok(g)
}

fn fit_general_smooth(
    y: &[f64],
    kind: smooth::Kind,
    transform: Transform,
    period: usize,
) -> Result<Fit, AlgoError> {
    let z = arima::transform(transform, y).map_err(data)?;
    let bounds = vec![(1e-7, 1.0 - 1e-7); kind.parameters()];
    let residuals = |x: &[f64]| {
        let g = general_initial(&z, kind, transform, period, x)?;
        let (p, _) = g.run(&z).map_err(data)?;
        Ok(p.iter().zip(&z).map(|(a, b)| a - b).collect::<Vec<_>>())
    };
    let mut best = None;
    let mut starts: Vec<Vec<f64>> = [0.01, 0.1, 0.25, 0.5, 0.8]
        .into_iter()
        .map(|s| {
            let mut initial = vec![s; kind.parameters()];
            if kind == smooth::Kind::Damped
                && let Some(v) = initial.get_mut(2)
            {
                *v = 0.9;
            }
            initial
        })
        .collect();
    if kind.parameters() >= 2 {
        let mut low_other = vec![0.001; kind.parameters()];
        low_other[0] = 0.1;
        if kind == smooth::Kind::Damped
            && let Some(v) = low_other.get_mut(2)
        {
            *v = 0.9;
        }
        starts.push(low_other);
    }
    for initial in starts {
        if let Ok((x, iterations)) = lm(initial, &bounds, residuals) {
            let r = residuals(&x)?;
            let value = r.iter().map(|v| v * v).sum::<f64>();
            if best.as_ref().is_none_or(|(_, _, v)| value < *v) {
                best = Some((x, iterations, value));
            }
        }
    }
    let (x, iterations, objective) =
        best.ok_or_else(|| data("smoothing optimizer did not converge"))?;
    let mut g = general_initial(&z, kind, transform, period, &x)?;
    g.sigma2 = objective / g.df;
    let (_, state) = g.run(&z).map_err(data)?;
    let rmse = g.sigma2.sqrt();
    Ok(Fit {
        spec: FittedSpec::Smooth(SmoothSpec::General(g)),
        objective,
        iterations,
        final_level: state.level,
        final_trend: state.trend,
        rmse,
    })
}

fn fit_expert_smooth(y: &[f64], period: usize) -> Result<Fit, AlgoError> {
    use smooth::Kind;
    let mut candidates = vec![Kind::Simple, Kind::Holt, Kind::Brown, Kind::Damped];
    if (2..=64).contains(&period) && y.len() > period {
        candidates.push(Kind::Seasonal);
        candidates.push(Kind::WintersAdd);
        if y.iter().all(|v| *v > 0.0) {
            candidates.push(Kind::WintersMul);
        }
    }
    let n = count(y.len());
    let mut best: Option<(f64, Fit)> = None;
    for kind in candidates {
        let k = count(kind.parameters());
        if n <= k {
            continue;
        }
        if let Ok(fit) = fit_general_smooth(y, kind, Transform::None, period) {
            let sigma2 = (fit.objective / (n - k)).max(1e-300);
            let nbic = sigma2.ln() + k * n.ln() / n;
            if best.as_ref().is_none_or(|(b, _)| nbic < *b) {
                best = Some((nbic, fit));
            }
        }
    }
    best.map(|(_, fit)| fit)
        .ok_or_else(|| data("Expert exponential smoothing found no admissible candidate"))
}

fn fit_seasonal_arima(
    y: &[f64],
    o: ArimaOrder,
    zero_ar: u64,
    zero_ma: u64,
) -> Result<Fit, AlgoError> {
    if o.p > 4
        || o.q > 4
        || o.sp > 1
        || o.sq > 1
        || o.d > 1
        || o.sd > 1
        || o.period > 24
        || o.period == 0
    {
        return Err(AlgoError::Unsupported(
            "GAP: estimation ARIMA factor orders exceed one".into(),
        ));
    }
    let degree = o.d + o.sd * o.period;
    if y.len() <= degree + 5 {
        return Err(data("insufficient differenced history"));
    }
    let z = arima::transform(o.transform, y).map_err(data)?;
    let n = count(y.len() - degree);
    let active = |order: usize, zeros: u64| (0..order).filter(|i| zeros & (1u64 << i) == 0).count();
    let k = usize::from(o.constant) + active(o.p, zero_ar) + active(o.q, zero_ma) + o.sp + o.sq;
    let make = |x: &[f64]| {
        let mut i = usize::from(o.constant);
        let mut take = |len: usize, zeros: u64| {
            (0..len)
                .map(|lag| {
                    if zeros & (1u64 << lag) != 0 {
                        0.0
                    } else {
                        let value = x.get(i).copied().unwrap_or(0.0);
                        i += 1;
                        value
                    }
                })
                .collect()
        };
        ArimaSpec {
            constant: if o.constant {
                x.first().copied().unwrap_or(0.0)
            } else {
                0.0
            },
            transform: o.transform,
            d: o.d,
            period: o.period,
            seasonal_d: o.sd,
            ar: take(o.p, zero_ar),
            ma: take(o.q, zero_ma),
            seasonal_ar: take(o.sp, 0),
            seasonal_ma: take(o.sq, 0),
            sigma2: 1.0,
            df: n - count(k),
        }
    };
    let residuals = |x: &[f64]| {
        let (r, ld) = arima::likelihood_residuals(&make(x), y).map_err(data)?;
        let scale = (ld / (2.0 * n)).exp();
        Ok(r.into_iter().map(|v| v * scale).collect::<Vec<_>>())
    };
    let mut bounds = vec![(-1.0 + 1e-8, 1.0 - 1e-8); k];
    if o.constant
        && let Some(v) = bounds.first_mut()
    {
        *v = (f64::NEG_INFINITY, f64::INFINITY);
    }
    let mut best = None;
    for start in [-0.3, 0.1, 0.5] {
        let mut x = vec![start; k];
        if o.constant
            && let Some(v) = x.first_mut()
        {
            *v = if degree == 0 {
                z.iter().sum::<f64>() / count(z.len())
            } else {
                0.0
            };
        }
        if let Ok((x, iterations)) = lm(x, &bounds, residuals) {
            let r = residuals(&x)?;
            let value = r.iter().map(|v| v * v).sum::<f64>();
            if best.as_ref().is_none_or(|(_, _, v)| value < *v) {
                best = Some((x, iterations, value));
            }
        }
    }
    let (x, iterations, _) =
        best.ok_or_else(|| data("seasonal ARIMA optimizer did not converge"))?;
    let mut spec = make(&x);
    let (r, _) = arima::likelihood_residuals(&spec, y).map_err(data)?;
    spec.sigma2 = r.iter().map(|v| v * v).sum::<f64>() / spec.df;
    let objective = objective(&FittedSpec::Arima(spec.clone()), y)?;
    let rmse = spec.sigma2.sqrt();
    Ok(Fit {
        spec: FittedSpec::Arima(spec),
        objective,
        iterations,
        final_level: 0.0,
        final_trend: 0.0,
        rmse,
    })
}

fn fit_arima(
    y: &[f64],
    p: usize,
    d: usize,
    q: usize,
    transform: Transform,
) -> Result<Fit, AlgoError> {
    let w = arima::difference(&arima::transform(transform, y).map_err(data)?, d);
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
        initial.push(if q == 0 {
            r1.clamp(-0.95, 0.95)
        } else {
            phi.clamp(-0.95, 0.95)
        });
        if q == 1 {
            initial.push((lambda / variance).clamp(-0.95, 0.95));
        } // internal MA sign is plus
    }
    if p == 0 && q == 1 {
        initial.push(0.1);
    }
    let make = |x: &[f64]| ArimaSpec {
        seasonal_d: 0,
        period: 1,
        seasonal_ar: vec![],
        seasonal_ma: vec![],
        transform,
        constant: x.first().copied().unwrap_or(0.0),
        d,
        ar: if p == 0 {
            vec![]
        } else {
            vec![x.get(1).copied().unwrap_or(0.0)]
        },
        ma: if q == 0 {
            vec![]
        } else {
            vec![x.get(1 + p).copied().unwrap_or(0.0)]
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
pub(crate) fn lm(
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
        for (i, ((v, (lo, hi)), g)) in x.iter().zip(bounds).zip(&mut gradient).enumerate() {
            if (*v <= *lo && *g > 0.0) || (*v >= *hi && *g < 0.0) {
                *g = 0.0;
                for (j, row) in matrix.iter_mut().enumerate() {
                    if i == j {
                        for cell in row.iter_mut() {
                            *cell = 0.0;
                        }
                        if let Some(diag) = row.get_mut(i) {
                            *diag = 1.0;
                        }
                    } else if let Some(cell) = row.get_mut(i) {
                        *cell = 0.0;
                    }
                }
            }
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

pub(crate) fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Result<Vec<f64>, AlgoError> {
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
        if let FittedSpec::Transfer(spec) = &self.spec {
            let mut plain = self.clone();
            plain.spec = FittedSpec::Arima(spec.noise.clone());
            return plain
                .to_xml(target, n)
                .replace("</ARIMA>", &format!("{}</ARIMA>", spec.effects_xml()));
        }
        let target = target
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('"', "&quot;")
            .replace('\r', "&#13;")
            .replace('\n', "&#10;")
            .replace('\t', "&#9;");
        let (algorithm, body, variance, df) = match &self.spec {
            FittedSpec::Transfer(_) => return String::new(),
            FittedSpec::Smooth(SmoothSpec::General(g)) => {
                let transform = match g.transform {
                    Transform::None => "none",
                    Transform::Log => "logarithmic",
                    Transform::Sqrt => "squareroot",
                };
                let trend = match g.kind {
                    smooth::Kind::Brown => "polynomial_exponential",
                    smooth::Kind::Damped => "damped_additive",
                    _ => "additive",
                };
                let trend_xml = if g.kind.trend() {
                    format!(
                        "<Trend_ExpoSmooth trend=\"{trend}\" gamma=\"{}\" phi=\"{}\"/>",
                        g.gamma, g.phi
                    )
                } else {
                    String::new()
                };
                let seasonal = if g.kind.seasonal() {
                    format!(
                        "<Seasonality_ExpoSmooth type=\"{}\" period=\"{}\" delta=\"{}\"/>",
                        if g.kind == smooth::Kind::WintersMul {
                            "multiplicative"
                        } else {
                            "additive"
                        },
                        g.initial.seasons.len(),
                        g.delta
                    )
                } else {
                    String::new()
                };
                let array = g
                    .initial
                    .seasons
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" ");
                let body = format!(
                    "<ExponentialSmoothing transformation=\"{transform}\"><Level alpha=\"{}\"/>{trend_xml}{seasonal}</ExponentialSmoothing><Extension name=\"initLevel\" value=\"{}\"/><Extension name=\"initTrend\" value=\"{}\"/><Extension name=\"initSeason\"><Array>{array}</Array></Extension>",
                    g.alpha, g.initial.level, g.initial.trend
                );
                ("ExponentialSmoothing", body, g.sigma2, g.df)
            }
            FittedSpec::Smooth(spec) => {
                let (alpha, gamma, level, trend, variance, df) = match spec {
                    SmoothSpec::General(_) => (0.0, None, 0.0, 0.0, 0.0, 0.0),
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
                let transform = match spec.transform {
                    Transform::None => "none",
                    Transform::Log => "logarithmic",
                    Transform::Sqrt => "squareRoot",
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
                    "<Extension><ARIMA transformation=\"{transform}\" constantTerm=\"{}\" seasonLength=\"{}\"><NonseasonalComponent p=\"{}\" d=\"{}\" q=\"{}\">{}{}</NonseasonalComponent><SeasonalComponent seasonalP=\"{}\" seasonalD=\"{}\" seasonalQ=\"{}\" period=\"{}\">{}{}</SeasonalComponent></ARIMA></Extension>",
                    spec.constant,
                    spec.period,
                    spec.ar.len(),
                    spec.d,
                    spec.ma.len(),
                    array("AR", &spec.ar, 1.0),
                    array("MA", &spec.ma, -1.0),
                    spec.seasonal_ar.len(),
                    spec.seasonal_d,
                    spec.seasonal_ma.len(),
                    spec.period,
                    array("AR", &spec.seasonal_ar, 1.0),
                    array("MA", &spec.seasonal_ma, -1.0)
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

    #[test]
    fn expert_smooth_selects_simple_holt_and_short_seasonal() {
        let constant = vec![50.0; 48];
        let fit_const = fit(&constant, Method::ExpertSmooth { period: 1 }).unwrap();
        assert!(matches!(
            fit_const.spec,
            FittedSpec::Smooth(SmoothSpec::General(smooth::GeneralSpec {
                kind: smooth::Kind::Simple,
                ..
            }))
        ));
        let seasonal_18: Vec<f64> = (0..18)
            .map(|i| 80.0 + 10.0 * (2.0 * std::f64::consts::PI * f64::from(i % 12) / 12.0).sin())
            .collect();
        let fit_seas = fit(&seasonal_18, Method::ExpertSmooth { period: 12 }).unwrap();
        assert!(matches!(
            fit_seas.spec,
            FittedSpec::Smooth(SmoothSpec::General(smooth::GeneralSpec {
                kind: smooth::Kind::Seasonal,
                ..
            }))
        ));
    }
}
