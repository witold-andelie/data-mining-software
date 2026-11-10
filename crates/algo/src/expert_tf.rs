//! Transfer Function Expert, public Modeler 19 Algorithms Guide sections 6-7.
//! Identification is deliberately separate from saved-PMML scoring. No saved
//! coefficients or target names participate in selection. Estimation failures
//! return the step-1 univariate fit and an explicit diagnostic (section 6.9).
use crate::{
    AlgoError,
    arima::{self, ArimaSpec, Transform},
    expert,
    time_series::{self, ArimaOrder, Fit, FittedSpec, Method},
    transfer::{Effect, TransferSpec},
};

#[derive(Debug, Clone)]
pub struct Predictor {
    pub field: String,
    pub values: Vec<f64>,
    pub forced: bool,
    /// Events/interventions have a single coefficient, never a rational form.
    pub event: bool,
}

#[derive(Debug, Clone)]
pub struct Selection {
    pub fit: Fit,
    /// An estimation failure is distinct from a statistically empty model.
    pub fallback_reason: Option<String>,
}

fn error(s: impl ToString) -> AlgoError {
    AlgoError::Data(s.to_string())
}
fn diff(x: &[f64], d: usize, sd: usize, period: usize) -> Vec<f64> {
    let mut z = arima::difference(x, d);
    for _ in 0..sd {
        z = z.iter().skip(period).zip(&z).map(|(a, b)| a - b).collect();
    }
    z
}
fn varying(x: &[f64]) -> bool {
    x.first().is_some_and(|first| x.iter().any(|v| v != first))
}
fn significant_ccf(y: &[f64], x: &[f64]) -> bool {
    let n = y.len().min(x.len());
    if n < 3 {
        return false;
    }
    let y = y.get(y.len() - n..).unwrap_or(&[]);
    let x = x.get(x.len() - n..).unwrap_or(&[]);
    let ym = y.iter().sum::<f64>() / n as f64;
    let xm = x.iter().sum::<f64>() / n as f64;
    let yy = y.iter().map(|v| (v - ym).powi(2)).sum::<f64>();
    let xx = x.iter().map(|v| (v - xm).powi(2)).sum::<f64>();
    let scale = (xx * yy).sqrt();
    scale > 0.0
        && (0..=12.min(n - 1)).any(|lag| {
            let cov = y
                .iter()
                .skip(lag)
                .zip(x)
                .map(|(a, b)| (a - ym) * (b - xm))
                .sum::<f64>();
            (cov / scale * (n as f64).sqrt()).abs() > 2.0
        })
}

/// Fit the univariate structure selected by `expert_arima`, including sparse lags.
pub fn fit_univariate(y: &[f64], spec: &ArimaSpec) -> Result<Fit, AlgoError> {
    let mask = |v: &[f64]| {
        v.iter()
            .enumerate()
            .take(64)
            .fold(0, |m, (i, v)| if *v == 0.0 { m | (1u64 << i) } else { m })
    };
    let method = if spec.period == 1
        && spec.constant != 0.0
        && spec.ar.len() <= 1
        && spec.ma.len() <= 1
        && spec.d <= 1
    {
        Method::Arima {
            p: spec.ar.len(),
            d: spec.d,
            q: spec.ma.len(),
            transform: spec.transform,
        }
    } else {
        Method::SparseArima {
            order: ArimaOrder {
                p: spec.ar.len(),
                d: spec.d,
                q: spec.ma.len(),
                sp: spec.seasonal_ar.len(),
                sd: spec.seasonal_d,
                sq: spec.seasonal_ma.len(),
                period: spec.period,
                constant: spec.constant != 0.0,
                transform: spec.transform,
            },
            zero_ar: mask(&spec.ar),
            zero_ma: mask(&spec.ma),
        }
    };
    time_series::fit(y, method)
}

/// Bounded Guide selection; missing/constant inputs are discarded even if forced.
/// `all_models` compares smoothing only when no transfer predictor survives.
pub fn select(
    y: &[f64],
    predictors: &[Predictor],
    period: usize,
    all_models: bool,
) -> Result<Selection, AlgoError> {
    crate::check_cancelled()?;
    if !(5..=512).contains(&y.len())
        || y.iter().any(|v| !v.is_finite())
        || !(1..=24).contains(&period)
        || predictors.len() > 16
    {
        return Err(error(
            "Transfer Expert requires 5..=512 finite targets, period 1..=24 and <=16 inputs",
        ));
    }
    if predictors
        .iter()
        .any(|p| p.field.len() > 4096 || p.values.len() != y.len())
    {
        return Err(error("Transfer Expert predictor name/length invalid"));
    }
    let s = if y.len() < 20 || y.len() < 3 * period {
        1
    } else {
        period
    };
    let initial = arima::expert_arima(y, s).map_err(error)?;
    let univariate = fit_univariate(y, &initial);
    let fallback = |reason: Option<String>| -> Result<Selection, AlgoError> {
        let mut fit = univariate.clone()?;
        if all_models {
            let smooth = time_series::fit(y, Method::ExpertSmooth { period: s })?;
            if time_series::normalized_bic(&smooth.spec, y)?
                <= time_series::normalized_bic(&fit.spec, y)?
            {
                fit = smooth;
            }
        }
        Ok(Selection {
            fit,
            fallback_reason: reason,
        })
    };
    if y.len() <= 10 || univariate.as_ref().is_ok_and(|fit| fit.rmse == 0.0) {
        return fallback(None);
    }
    let season = if initial.seasonal_ar.is_empty()
        && initial.seasonal_ma.is_empty()
        && initial.seasonal_d == 0
    {
        1
    } else {
        s
    };
    let z = arima::transform(initial.transform, y).map_err(error)?;
    let w = diff(&z, initial.d, initial.seasonal_d, s);
    let mut inputs = Vec::new();
    for p in predictors {
        crate::check_cancelled()?;
        if p.values.iter().any(|v| !v.is_finite()) || !varying(&p.values) {
            continue;
        }
        let transform = if !p.event && p.values.iter().all(|v| *v > 0.0) {
            initial.transform
        } else {
            Transform::None
        };
        let x = arima::transform(transform, &p.values).map_err(error)?;
        let (mut d, mut sd) = (initial.d, initial.seasonal_d);
        let mut xd = diff(&x, d, sd, s);
        if !varying(&xd) {
            continue;
        }
        if !p.event && (d > 0 || sd > 0) && !significant_ccf(&w, &xd) {
            (d, sd) = arima::expert_differencing(&x, season);
            if d == 0 && sd == 0 {
                continue;
            }
            xd = diff(&x, d, sd, season);
            if !varying(&xd) || !significant_ccf(&w, &xd) {
                continue;
            }
        }
        inputs.push((
            p.clone(),
            Effect {
                field: p.field.clone(),
                transform,
                difference: d,
                seasonal_difference: sd,
                period: s,
                delay: 0,
                numerator: vec![0.0],
                denominator: vec![],
            },
        ));
    }
    if inputs.is_empty() {
        return fallback(None);
    }
    match identify(y, &w, &initial, inputs, season) {
        Ok(Some(fit)) => Ok(Selection {
            fit,
            fallback_reason: None,
        }),
        Ok(None) => fallback(None),
        Err(e) => fallback(Some(e.to_string())),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Term {
    Constant,
    Ar(usize),
    Ma(usize),
    Sar(usize),
    Sma(usize),
    Num(usize, usize),
    Den(usize, usize),
}
#[derive(Clone)]
struct Model {
    spec: TransferSpec,
    terms: Vec<Term>,
    inputs: Vec<Predictor>,
}
impl Model {
    fn value(&self, t: Term) -> f64 {
        match t {
            Term::Constant => self.spec.noise.constant,
            Term::Ar(i) => self.spec.noise.ar.get(i).copied().unwrap_or(0.0),
            Term::Ma(i) => self.spec.noise.ma.get(i).copied().unwrap_or(0.0),
            Term::Sar(i) => self.spec.noise.seasonal_ar.get(i).copied().unwrap_or(0.0),
            Term::Sma(i) => self.spec.noise.seasonal_ma.get(i).copied().unwrap_or(0.0),
            Term::Num(i, j) => self
                .spec
                .effects
                .get(i)
                .and_then(|e| e.numerator.get(j))
                .copied()
                .unwrap_or(0.0),
            Term::Den(i, j) => self
                .spec
                .effects
                .get(i)
                .and_then(|e| e.denominator.get(j))
                .copied()
                .unwrap_or(0.0),
        }
    }
    fn set(&mut self, t: Term, v: f64) {
        let slot = match t {
            Term::Constant => Some(&mut self.spec.noise.constant),
            Term::Ar(i) => self.spec.noise.ar.get_mut(i),
            Term::Ma(i) => self.spec.noise.ma.get_mut(i),
            Term::Sar(i) => self.spec.noise.seasonal_ar.get_mut(i),
            Term::Sma(i) => self.spec.noise.seasonal_ma.get_mut(i),
            Term::Num(i, j) => self
                .spec
                .effects
                .get_mut(i)
                .and_then(|e| e.numerator.get_mut(j)),
            Term::Den(i, j) => self
                .spec
                .effects
                .get_mut(i)
                .and_then(|e| e.denominator.get_mut(j)),
        };
        if let Some(slot) = slot {
            *slot = v;
        }
    }
    fn remove_effect(&mut self, index: usize) {
        if index >= self.inputs.len() || index >= self.spec.effects.len() {
            return;
        }
        self.inputs.remove(index);
        self.spec.effects.remove(index);
        self.terms = self
            .terms
            .iter()
            .filter_map(|&t| match t {
                Term::Num(i, _) | Term::Den(i, _) if i == index => None,
                Term::Num(i, j) if i > index => Some(Term::Num(i - 1, j)),
                Term::Den(i, j) if i > index => Some(Term::Den(i - 1, j)),
                _ => Some(t),
            })
            .collect();
    }
    fn columns(&self) -> Vec<Vec<f64>> {
        self.inputs.iter().map(|p| p.values.clone()).collect()
    }
}
fn noise_terms(s: &ArimaSpec) -> Vec<Term> {
    let mut t = vec![Term::Constant];
    for (v, kind) in [
        (&s.ar, Term::Ar as fn(usize) -> Term),
        (&s.ma, Term::Ma),
        (&s.seasonal_ar, Term::Sar),
        (&s.seasonal_ma, Term::Sma),
    ] {
        t.extend(
            v.iter()
                .enumerate()
                .filter(|(_, v)| **v != 0.0)
                .map(|(i, _)| kind(i)),
        );
    }
    t
}
fn initial_model(
    initial: &ArimaSpec,
    inputs: Vec<(Predictor, Effect)>,
    season: usize,
    n: usize,
) -> Model {
    let mut terms = noise_terms(initial);
    let events = inputs.iter().filter(|(p, _)| p.event).count();
    let series = inputs.len() - events;
    let mut m = if season == 1 { 8 } else { (season + 3).min(20) };
    while m > 0 && terms.len() + events + series * (m + 1) >= n / 2 {
        m -= 1;
    }
    let mut effects = Vec::new();
    let mut predictors = Vec::new();
    for (i, (p, mut e)) in inputs.into_iter().enumerate() {
        let order = if p.event { 0 } else { m };
        e.numerator = vec![0.0; order + 1];
        e.denominator.clear();
        e.delay = 0;
        terms.extend((0..=order).map(|j| Term::Num(i, j)));
        effects.push(e);
        predictors.push(p);
    }
    let mut noise = initial.clone();
    noise.constant = 0.0;
    for v in noise
        .ar
        .iter_mut()
        .chain(&mut noise.ma)
        .chain(&mut noise.seasonal_ar)
        .chain(&mut noise.seasonal_ma)
    {
        *v = if *v == 0.0 { 0.0 } else { 0.1 };
    }
    Model {
        spec: TransferSpec { noise, effects },
        terms,
        inputs: predictors,
    }
}
struct Estimate {
    t: Vec<f64>,
    residuals: Vec<f64>,
    raw: Vec<f64>,
    ld: f64,
    iterations: usize,
}
fn estimate(model: &mut Model, w: &[f64], n: usize, ml: bool) -> Result<Estimate, AlgoError> {
    if !ml {
        // Section 6.7.3: previous transfer estimates, default ARMA starts.
        let terms = model.terms.clone();
        for t in terms {
            crate::check_cancelled()?;
            if matches!(t, Term::Ar(_) | Term::Ma(_) | Term::Sar(_) | Term::Sma(_)) {
                model.set(t, 0.1);
            }
        }
    }
    let x = model.columns();
    let degree = model.spec.noise.d + model.spec.noise.seasonal_d * model.spec.noise.period;
    let (prefix, _) = model.spec.signal(&x, n)?;
    let start = prefix.max(degree);
    let count = n.saturating_sub(start);
    if count <= model.terms.len() || count < 5 {
        return Err(error(
            "Transfer Expert insufficient residual degrees of freedom",
        ));
    }
    let make = |p: &[f64]| {
        let mut m = model.clone();
        for (t, v) in model.terms.iter().zip(p) {
            m.set(*t, *v);
        }
        m
    };
    let raw = |p: &[f64]| -> Result<(Vec<f64>, f64), AlgoError> {
        let m = make(p);
        if m.spec
            .effects
            .iter()
            .any(|e| !expert::tf_admissible_denominator(&e.denominator))
        {
            return Err(error("unstable transfer denominator"));
        }
        let (_, signal) = m.spec.signal(&x, n)?;
        let adjusted: Vec<_> = w
            .iter()
            .skip(start - degree)
            .zip(signal.iter().skip(start))
            .map(|(a, b)| a - b)
            .collect();
        expert::tf_noise_residuals(&m.spec.noise, &adjusted, ml)
            .ok_or_else(|| error("invalid Transfer Expert noise likelihood"))
    };
    let residual = |p: &[f64]| -> Result<Vec<f64>, AlgoError> {
        let (r, ld) = raw(p)?;
        let scale = (ld / (2.0 * count as f64)).exp();
        Ok(r.iter().map(|v| v * scale).collect())
    };
    let mut initial: Vec<_> = model.terms.iter().map(|&t| model.value(t)).collect();
    // CLS can finish outside the stationary region. Contract only the noise
    // start, as the univariate ML optimizer does, before exact likelihood.
    if ml {
        for _ in 0..32 {
            crate::check_cancelled()?;
            if raw(&initial).is_ok() {
                break;
            }
            for (t, v) in model.terms.iter().zip(&mut initial) {
                if matches!(t, Term::Ar(_) | Term::Ma(_) | Term::Sar(_) | Term::Sma(_)) {
                    *v *= 0.5;
                }
            }
        }
    }
    let bounds: Vec<_> = model
        .terms
        .iter()
        .map(|t| match t {
            Term::Constant | Term::Num(_, _) => (f64::NEG_INFINITY, f64::INFINITY),
            _ => (-10.0, 10.0),
        })
        .collect();
    let (p, iterations) = time_series::lm(initial, &bounds, residual)?;
    let (r, ld) = raw(&p)?;
    let variance = r.iter().map(|v| v * v).sum::<f64>() / (count - model.terms.len()) as f64;
    let mut jac = Vec::new();
    for (j, v) in p.iter().enumerate() {
        crate::check_cancelled()?;
        let h = 1e-5 * (1.0 + v.abs());
        let mut plus = p.clone();
        let mut minus = p.clone();
        if let Some(c) = plus.get_mut(j) {
            *c += h;
        }
        if let Some(c) = minus.get_mut(j) {
            *c -= h;
        }
        let (rp, rm, width) = match (raw(&plus), raw(&minus)) {
            (Ok((rp, _)), Ok((rm, _))) => (rp, rm, 2.0 * h),
            (Ok((rp, _)), Err(_)) => (rp, r.clone(), h),
            (Err(_), Ok((rm, _))) => (r.clone(), rm, h),
            (Err(e), Err(_)) => return Err(e),
        };
        jac.push(
            rp.iter()
                .zip(rm)
                .map(|(a, b)| (a - b) / width)
                .collect::<Vec<_>>(),
        );
    }
    let matrix: Vec<Vec<f64>> = jac
        .iter()
        .map(|a| {
            jac.iter()
                .map(|b| a.iter().zip(b).map(|(a, b)| a * b).sum())
                .collect()
        })
        .collect();
    let mut t = Vec::new();
    for (i, value) in p.iter().enumerate() {
        crate::check_cancelled()?;
        let mut unit = vec![0.0; p.len()];
        if let Some(c) = unit.get_mut(i) {
            *c = 1.0;
        }
        let inverse = time_series::solve(matrix.clone(), unit)?;
        let diagonal = inverse.get(i).copied().unwrap_or(f64::INFINITY);
        if diagonal <= 0.0 || !diagonal.is_finite() {
            return Err(error("singular transfer covariance"));
        }
        t.push(value / (variance * diagonal).max(1e-300).sqrt());
    }
    let mut fitted = make(&p);
    fitted.spec.noise.df = (count - fitted.terms.len()) as f64;
    fitted.spec.noise.sigma2 = variance.max(1e-300);
    let diagnostic_raw = if ml {
        let (_, signal) = fitted.spec.signal(&x, n)?;
        let adjusted: Vec<_> = w
            .iter()
            .skip(start - degree)
            .zip(signal.iter().skip(start))
            .map(|(a, b)| a - b)
            .collect();
        expert::tf_raw_residuals(&fitted.spec.noise, &adjusted)
            .ok_or_else(|| error("invalid transfer diagnostic residuals"))?
    } else {
        r.clone()
    };
    *model = fitted;
    Ok(Estimate {
        t,
        residuals: r,
        raw: diagnostic_raw,
        ld,
        iterations,
    })
}
fn protected(model: &Model, term: Term) -> bool {
    match term {
        Term::Constant => model.spec.noise.d == 0 && model.spec.noise.seasonal_d == 0,
        Term::Num(i, _) => {
            model.inputs.get(i).is_some_and(|p| p.forced)
                && model
                    .terms
                    .iter()
                    .filter(|t| matches!(t,Term::Num(j,_) if *j==i))
                    .count()
                    <= 1
        }
        _ => false,
    }
}
fn prune(model: &mut Model, est: &Estimate, denominators: bool, numerators: bool) -> bool {
    let mut remove: Vec<_> = model
        .terms
        .iter()
        .zip(&est.t)
        .filter(|(t, v)| {
            v.abs() < 2.0
                && !protected(model, **t)
                && match t {
                    Term::Den(_, _) => denominators,
                    Term::Num(_, _) => numerators,
                    _ => true,
                }
        })
        .map(|(t, _)| *t)
        .collect();
    for (i, _) in model.inputs.iter().enumerate().filter(|(_, p)| p.forced) {
        let terms: Vec<_> = model
            .terms
            .iter()
            .zip(&est.t)
            .filter(|(t, _)| matches!(t,Term::Num(j,_) if *j==i))
            .collect();
        if terms.iter().all(|(t, _)| remove.contains(t))
            && let Some((keep, _)) = terms
                .iter()
                .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
        {
            remove.retain(|t| t != *keep);
        }
    }
    for t in &remove {
        model.set(*t, 0.0);
    }
    model.terms.retain(|t| !remove.contains(t));
    for i in (0..model.spec.effects.len()).rev() {
        if !model
            .terms
            .iter()
            .any(|t| matches!(t,Term::Num(j,_) if *j==i))
        {
            model.remove_effect(i);
        }
    }
    !remove.is_empty()
}
fn compact(model: &mut Model) {
    // Trimming only zero endpoints preserves internal sparse gaps.
    for i in 0..model.spec.effects.len() {
        let Some(e) = model.spec.effects.get_mut(i) else {
            continue;
        };
        while e.numerator.len() > 1 && e.numerator.last() == Some(&0.0) {
            e.numerator.pop();
        }
        while e.denominator.last() == Some(&0.0) {
            e.denominator.pop();
        }
        let leading = e.numerator.iter().position(|v| *v != 0.0).unwrap_or(0);
        if leading > 0 {
            e.numerator.drain(..leading);
            e.delay += leading;
        }
        model.terms = model
            .terms
            .iter()
            .filter_map(|&t| match t {
                Term::Num(j, k) if j == i => k.checked_sub(leading).map(|k| Term::Num(j, k)),
                _ => Some(t),
            })
            .collect();
    }
    for v in [
        &mut model.spec.noise.ar,
        &mut model.spec.noise.ma,
        &mut model.spec.noise.seasonal_ar,
        &mut model.spec.noise.seasonal_ma,
    ] {
        while v.last() == Some(&0.0) {
            v.pop();
        }
    }
}
fn identify(
    y: &[f64],
    w: &[f64],
    initial: &ArimaSpec,
    inputs: Vec<(Predictor, Effect)>,
    season: usize,
) -> Result<Option<Fit>, AlgoError> {
    let mut model = initial_model(initial, inputs, season, w.len());
    let mut est = estimate(&mut model, w, y.len(), false)?;
    // Whole-series deletion, last first; rebuild m after every deletion.
    for event in [false, true] {
        crate::check_cancelled()?;
        for _ in 0..16 {
            let drop = (0..model.inputs.len()).rev().find(|&i| {
                model
                    .inputs
                    .get(i)
                    .is_some_and(|p| p.event == event && !p.forced)
                    && !model
                        .terms
                        .iter()
                        .zip(&est.t)
                        .any(|(t, v)| matches!(t,Term::Num(j,_) if *j==i) && v.abs() >= 2.0)
            });
            let Some(i) = drop else {
                break;
            };
            model.remove_effect(i);
            if model.inputs.is_empty() {
                return Ok(None);
            }
            let inputs = model.inputs.into_iter().zip(model.spec.effects).collect();
            model = initial_model(initial, inputs, season, w.len());
            est = estimate(&mut model, w, y.len(), false)?;
        }
    }
    for _ in 0..2 {
        crate::check_cancelled()?;
        if prune(&mut model, &est, false, false) {
            est = estimate(&mut model, w, y.len(), false)?;
        }
    }
    // Replace FIR by sparse numerator or a quadratic rational response.
    let significance: Vec<_> = model
        .terms
        .iter()
        .copied()
        .zip(est.t.iter().copied())
        .collect();
    for i in 0..model.inputs.len() {
        crate::check_cancelled()?;
        if model.inputs.get(i).is_some_and(|p| p.event) {
            continue;
        }
        let mut sig: Vec<_> = significance
            .iter()
            .filter_map(|(t, v)| match t {
                Term::Num(j, k) if *j == i && v.abs() >= 2.0 => Some(*k),
                _ => None,
            })
            .collect();
        if sig.is_empty()
            && model.inputs.get(i).is_some_and(|p| p.forced)
            && let Some((Term::Num(_, k), _)) = significance
                .iter()
                .filter(|(t, _)| matches!(t,Term::Num(j,_) if *j==i))
                .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
        {
            sig.push(*k);
        }
        let Some(first) = sig.first().copied() else {
            continue;
        };
        let Some(e) = model.spec.effects.get_mut(i) else {
            continue;
        };
        let old = e.numerator.clone();
        e.delay = first;
        let rational = sig.len() > 2;
        let last = if rational {
            first + 2
        } else {
            sig.last().copied().unwrap_or(first)
        };
        e.numerator = (first..=last)
            .map(|k| {
                if rational || sig.contains(&k) {
                    old.get(k).copied().unwrap_or(0.0)
                } else {
                    0.0
                }
            })
            .collect();
        e.denominator = if rational { vec![0.0; 2] } else { vec![] };
        model
            .terms
            .retain(|t| !matches!(t,Term::Num(j,_)|Term::Den(j,_) if *j==i));
        if rational {
            model.terms.extend((0..3).map(|j| Term::Num(i, j)));
            model.terms.extend((0..2).map(|j| Term::Den(i, j)));
        } else {
            model
                .terms
                .extend(sig.iter().map(|k| Term::Num(i, k - first)));
        }
    }
    for (ml, den) in [(false, false), (false, true), (true, true)] {
        crate::check_cancelled()?;
        for _ in 0..128 {
            est = estimate(&mut model, w, y.len(), ml)?;
            if !prune(&mut model, &est, den, true) {
                break;
            }
            if model.inputs.is_empty() {
                return Ok(None);
            }
        }
    }
    compact(&mut model);
    est = estimate(&mut model, w, y.len(), true)?;
    let diagnostic = expert::tf_diagnostic(&model.spec.noise, &est.raw, season);
    let mut added = false;
    for term in noise_terms(&diagnostic)
        .into_iter()
        .filter(|t| *t != Term::Constant)
    {
        crate::check_cancelled()?;
        if !model.terms.contains(&term) {
            let v = match term {
                Term::Ar(i) => Some((&mut model.spec.noise.ar, i)),
                Term::Ma(i) => Some((&mut model.spec.noise.ma, i)),
                Term::Sar(i) => Some((&mut model.spec.noise.seasonal_ar, i)),
                Term::Sma(i) => Some((&mut model.spec.noise.seasonal_ma, i)),
                _ => None,
            };
            if let Some((v, i)) = v
                && i < 32
            {
                v.resize(v.len().max(i + 1), 0.0);
                model.terms.push(term);
                added = true;
            }
        }
    }
    if added {
        est = estimate(&mut model, w, y.len(), false)?;
        prune(&mut model, &est, true, true);
        if model.inputs.is_empty() {
            return Ok(None);
        }
        for _ in 0..128 {
            crate::check_cancelled()?;
            est = estimate(&mut model, w, y.len(), true)?;
            if !prune(&mut model, &est, true, true) {
                break;
            }
            if model.inputs.is_empty() {
                return Ok(None);
            }
        }
        compact(&mut model);
        est = estimate(&mut model, w, y.len(), true)?;
    }
    let n = est.residuals.len() as f64;
    let sse = est.residuals.iter().map(|v| v * v).sum::<f64>();
    let objective = 0.5 * (n * ((2.0 * std::f64::consts::PI).ln() + 1.0 + (sse / n).ln()) + est.ld);
    let rmse = model.spec.noise.sigma2.sqrt();
    Ok(Some(Fit {
        spec: FittedSpec::Transfer(model.spec),
        objective,
        iterations: est.iterations,
        final_level: 0.0,
        final_trend: 0.0,
        rmse,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn noise(seed: u64, n: usize) -> Vec<f64> {
        let mut state = seed;
        (0..n)
            .map(|_| {
                state = (1664525 * state + 1013904223) % 4294967296;
                12.0_f64.sqrt() * ((state as f64 + 0.5) / 4294967296.0 - 0.5)
            })
            .collect()
    }
    #[test]
    fn identifies_a_delayed_input_and_drops_an_irrelevant_candidate() {
        let x = noise(718, 160);
        let irrelevant = noise(116, 160);
        let e = noise(191, 160);
        let y: Vec<_> = (0usize..160)
            .map(|i| 100.0 + 3.0 * x.get(i.wrapping_sub(3)).copied().unwrap_or(0.0) + 0.1 * e[i])
            .collect();
        let result = select(
            &y,
            &[
                Predictor {
                    field: "x".into(),
                    values: x,
                    forced: false,
                    event: false,
                },
                Predictor {
                    field: "irrelevant".into(),
                    values: irrelevant,
                    forced: false,
                    event: false,
                },
            ],
            1,
            false,
        )
        .unwrap();
        let FittedSpec::Transfer(spec) = result.fit.spec else {
            panic!("{:?}", result.fallback_reason);
        };
        assert_eq!(spec.effects.len(), 1, "{spec:?}");
        assert_eq!(spec.effects[0].field, "x");
        assert_eq!(spec.effects[0].delay, 3);
        assert!(
            spec.effects[0]
                .numerator
                .iter()
                .skip(1)
                .all(|v| v.abs() < 0.03)
        );
        assert!((spec.effects[0].numerator[0] - 3.0).abs() < 0.03);
    }
    #[test]
    fn short_series_drops_even_forced_inputs_and_rejects_hostile_lengths() {
        let y: Vec<_> = noise(3, 10).iter().map(|v| v + 10.0).collect();
        let p = Predictor {
            field: "x".into(),
            values: noise(4, 10),
            forced: true,
            event: false,
        };
        assert!(matches!(
            select(&y, std::slice::from_ref(&p), 12, false)
                .unwrap()
                .fit
                .spec,
            FittedSpec::Arima(_)
        ));
        assert!(select(&y, &[p], usize::MAX, false).is_err());
        assert!(select(&[1.0; 513], &[], 1, false).is_err());
        assert!(select(&[f64::NAN; 20], &[], 1, false).is_err());
    }
    #[test]
    fn cleaning_discards_missing_and_constant_inputs() {
        let y: Vec<_> = noise(3, 60).iter().map(|v| v + 10.0).collect();
        let p = Predictor {
            field: "x".into(),
            values: vec![1.0; 60],
            forced: true,
            event: false,
        };
        let mut missing = p.clone();
        missing.values[4] = f64::NAN;
        let result = select(&y, &[p, missing], 1, false).unwrap();
        assert!(matches!(result.fit.spec, FittedSpec::Arima(_)));
        assert!(result.fallback_reason.is_none());
    }

    #[test]
    fn integrated_transfer_scores_on_the_differenced_target_scale() {
        for period in [1, 4] {
            let x: Vec<_> = (0..33).map(|i| 10.0 + (i as f64 * 1.13).sin()).collect();
            let mut y = Vec::new();
            for t in 0..32 {
                y.push(if t < period {
                    20.0 + 2.0 * x[t]
                } else {
                    y[t - period] + 2.0 * (x[t] - x[t - period]) + 0.1 * (t as f64).cos()
                });
            }
            let spec = TransferSpec {
                noise: ArimaSpec {
                    d: usize::from(period == 1),
                    seasonal_d: usize::from(period > 1),
                    period,
                    ar: vec![],
                    ma: vec![],
                    seasonal_ar: vec![],
                    seasonal_ma: vec![],
                    constant: 0.0,
                    transform: Transform::None,
                    sigma2: 0.01,
                    df: 20.0,
                },
                effects: vec![Effect {
                    field: "x".into(),
                    transform: Transform::None,
                    difference: usize::from(period == 1),
                    seasonal_difference: usize::from(period > 1),
                    period,
                    delay: 0,
                    numerator: vec![2.0],
                    denominator: vec![],
                }],
            };
            let scored = spec.score(&y, std::slice::from_ref(&x), 1, 95.0).unwrap();
            for t in 0..period {
                assert!(scored.fitted[t].is_none());
            }
            for t in period..32 {
                let expected = y[t - period] + 2.0 * (x[t] - x[t - period]);
                assert!((scored.fitted[t].as_ref().unwrap().point - expected).abs() < 1e-10);
            }
            assert!(
                (scored.forecasts[0].point - (y[32 - period] + 2.0 * (x[32] - x[32 - period])))
                    .abs()
                    < 1e-10
            );
            assert!(
                spec.objective(&y, std::slice::from_ref(&x))
                    .unwrap()
                    .is_finite()
            );
            let mut invalid = spec;
            invalid.noise.d = usize::MAX;
            assert!(invalid.score(&y, &[x], 1, 95.0).is_err());
        }
    }
}
