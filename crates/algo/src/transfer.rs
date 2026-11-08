//! Transfer scoring with stationary ARMA noise (Algorithms Guide 19, Time Series).
//! Predictors are transformed, differenced, delayed, then filtered. Seasonal
//! multiplicative polynomials are expanded by the PMML parser, so scoring only
//! sees additive numerator and denominator recurrence coefficients.
//! `ts_more/ts_tf_static`, `ts_tf_num`, `ts_tf_den`, `ts_tf_delay` establish
//! the lost prefix and PMML numerator sign: coefficient zero is positive,
//! subsequent numerator coefficients are subtracted.
//!
//! Denominator (`ts_tf_diff`, `ts_tf_two`): the response follows
//! `v_t = sum_i delta_i v_{t-i} + numerator part`, with PMML's denominator values used as
//! stored. Before an effect's prefix every `v` is the steady state `omega(1) / delta(1)` times
//! the first (differenced) predictor value, as the Algorithms Guide's "Calculation of the
//! Transfer Function" describes. Evidence: with Modeler's saved `ts_tf_diff` parameters its
//! first fitted value (t = 4) implies `v_3 = 10.471991`, which is `1.6336 * z_2` to six
//! digits; zero or the raw first value do not fit. The scored prefix uses the
//! larger numerator/denominator degree (`ts_tf_diff`: 108 - 3 - 5 = 100).
use crate::{
    AlgoError, arima,
    time_series::{self, Fit, FittedSpec},
};

#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    pub field: String,
    pub transform: arima::Transform,
    pub difference: usize,
    pub seasonal_difference: usize,
    pub period: usize,
    pub delay: usize,
    /// All coefficients use the additive convention internally.
    pub numerator: Vec<f64>,
    /// Expanded recurrence coefficients: `v_t = sum_i delta_i v_{t-i} + ...`.
    pub denominator: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransferSpec {
    pub noise: arima::ArimaSpec,
    pub effects: Vec<Effect>,
}

fn error(message: &str) -> AlgoError {
    AlgoError::Data(message.into())
}

impl TransferSpec {
    fn signal(&self, x: &[Vec<f64>], n: usize) -> Result<(usize, Vec<f64>), AlgoError> {
        if n > 1024
            || self.effects.is_empty()
            || self.effects.len() > 16
            || x.len() != self.effects.len()
            || self.noise.d != 0
            || self.noise.seasonal_d != 0
        {
            return Err(error(
                "GAP: transfer requires <=16 predictors and undifferenced ARMA noise",
            ));
        }
        let mut signal = vec![0.0; n];
        let mut start = 0;
        for (e, x) in self.effects.iter().zip(x) {
            if e.difference > 2
                || e.seasonal_difference > 2
                || !(1..=64).contains(&e.period)
                || e.delay > 64
                || e.numerator.is_empty()
                || e.numerator.len() > 65
                || e.denominator.len() > 64
                || x.len() < n
                || x.len() > 1024
                || x.iter()
                    .chain(&e.numerator)
                    .chain(&e.denominator)
                    .any(|v| !v.is_finite())
            {
                return Err(error("invalid transfer predictor/order/length"));
            }
            let lost = e.difference + e.seasonal_difference * e.period;
            let prefix = lost + e.delay + e.numerator.len() - 1;
            start = start.max(lost + e.delay + (e.numerator.len() - 1).max(e.denominator.len()));
            let transformed =
                arima::transform(e.transform, x).map_err(|e| error(&e.to_string()))?;
            let mut z = arima::difference(&transformed, e.difference);
            for _ in 0..e.seasonal_difference {
                z = z
                    .iter()
                    .skip(e.period)
                    .zip(&z)
                    .map(|(a, b)| a - b)
                    .collect();
            }
            let mut response = vec![0.0; n];
            for (i, out) in response.iter_mut().enumerate().skip(prefix) {
                for (j, b) in e.numerator.iter().enumerate() {
                    let at = i - lost - e.delay - j;
                    *out += b * z
                        .get(at)
                        .copied()
                        .ok_or_else(|| error("transfer lag absent"))?;
                }
            }
            if !e.denominator.is_empty() {
                let level = 1.0 - e.denominator.iter().sum::<f64>();
                let first = z
                    .first()
                    .copied()
                    .ok_or_else(|| error("transfer lag absent"))?;
                if level.abs() < 1e-12 {
                    return Err(error("transfer denominator has a unit root"));
                }
                let before = e.numerator.iter().sum::<f64>() / level * first;
                for i in prefix..n {
                    let lagged: f64 = e
                        .denominator
                        .iter()
                        .enumerate()
                        .map(|(k, d)| {
                            let back = i.checked_sub(k + 1).filter(|&at| at >= prefix);
                            d * back.map_or(before, |at| response[at])
                        })
                        .sum();
                    response[i] += lagged;
                }
            }
            for (out, v) in signal.iter_mut().zip(&response).skip(prefix) {
                *out += v;
                if !out.is_finite() {
                    return Err(error("nonfinite transfer response"));
                }
            }
        }
        if start >= n {
            return Err(error("transfer prefix exceeds history"));
        }
        Ok((start, signal))
    }

    fn adjusted(&self, y: &[f64], x: &[Vec<f64>]) -> Result<Vec<f64>, AlgoError> {
        let (start, signal) = self.signal(x, y.len())?;
        let transformed =
            arima::transform(self.noise.transform, y).map_err(|e| error(&e.to_string()))?;
        Ok(transformed
            .iter()
            .zip(signal)
            .skip(start)
            .map(|(y, v)| y - v)
            .collect())
    }

    pub fn objective(&self, y: &[f64], x: &[Vec<f64>]) -> Result<f64, AlgoError> {
        let mut noise = self.noise.clone();
        noise.transform = arima::Transform::None;
        time_series::objective(&FittedSpec::Arima(noise), &self.adjusted(y, x)?)
    }

    pub fn score(
        &self,
        y: &[f64],
        x: &[Vec<f64>],
        horizon: usize,
        confidence: f64,
    ) -> Result<arima::SeriesScore, AlgoError> {
        if horizon > 512 || y.len() > 512 {
            return Err(error("transfer scoring history/horizon exceeds 512"));
        }
        let (start, signal) = self.signal(x, y.len().saturating_add(horizon))?;
        if start >= y.len() {
            return Err(error("transfer prefix exceeds history"));
        }
        let noise = arima::score_with_signal(
            &self.noise,
            &y[start..],
            horizon,
            confidence,
            &signal[start..],
        )
        .map_err(|e| error(&e.to_string()))?;
        let mut fitted = vec![None; start];
        fitted.extend(noise.fitted);
        let forecasts = noise.forecasts;
        Ok(arima::SeriesScore { fitted, forecasts })
    }

    pub(crate) fn effects_xml(&self) -> String {
        self.effects.iter().map(|e| {
            let field=e.field.replace('&',"&amp;").replace('<',"&lt;").replace('"',"&quot;");
            let coefs=e.numerator.iter().enumerate().map(|(i,v)|if i==0{*v}else{-v}).map(|v|v.to_string()).collect::<Vec<_>>().join(" ");
            let den=e.denominator.iter().map(f64::to_string).collect::<Vec<_>>().join(" ");
            let r=e.denominator.len();
            let transform = match e.transform {
                arima::Transform::None => "none",
                arima::Transform::Log => "logarithmic",
                arima::Transform::Sqrt => "squareroot",
            };
            format!("<PredicterEffect field=\"{field}\" transformation=\"{transform}\" delay=\"{}\" nonSeasonalDiff=\"{}\" seasonalDiff=\"{}\"><Numerator><NonseasonFactor maximumOrder=\"{}\"><Array n=\"{}\" type=\"real\">{coefs}</Array></NonseasonFactor></Numerator><Denominator><NonseasonFactor maximumOrder=\"{r}\"><Array n=\"{r}\" type=\"real\">{den}</Array></NonseasonFactor></Denominator></PredicterEffect>",e.delay,e.difference,e.seasonal_difference,e.numerator.len(),e.numerator.len())
        }).collect()
    }
}

/// Fit only the supplied orders; template coefficients and noise state are discarded.
pub fn fit(y: &[f64], x: &[Vec<f64>], template: &TransferSpec) -> Result<Fit, AlgoError> {
    if template.noise.transform != arima::Transform::None
        || template.noise.ar.len() != 1
        || !template.noise.ma.is_empty()
        || !template.noise.seasonal_ar.is_empty()
        || !template.noise.seasonal_ma.is_empty()
        || template.effects.iter().any(|e| {
            e.transform != arima::Transform::None
                || e.seasonal_difference != 0
                || e.difference > 1
                || e.numerator.len() > 5
                || e.denominator.len() > 4
        })
    {
        return Err(error("unsupported transfer fitting specification"));
    }
    if !(8..=512).contains(&y.len()) || y.iter().any(|v| !v.is_finite()) {
        return Err(error("invalid transfer training history"));
    }
    let (start, _) = template.signal(x, y.len())?;
    let k = 2 + template
        .effects
        .iter()
        .map(|e| e.numerator.len() + e.denominator.len())
        .sum::<usize>();
    let n = y.len() - start;
    if n <= k {
        return Err(error("insufficient transfer degrees of freedom"));
    }
    let nf = f64::from(u32::try_from(n).map_err(|_| error("transfer history limit"))?);
    let df = nf - f64::from(u32::try_from(k).map_err(|_| error("transfer parameter limit"))?);
    let make = |parameters: &[f64]| {
        let mut spec = template.clone();
        spec.noise.constant = parameters.first().copied().unwrap_or(0.0);
        spec.noise.ar = vec![parameters.get(1).copied().unwrap_or(0.0)];
        spec.noise.sigma2 = 1.0;
        spec.noise.df = df;
        let mut at = 2;
        for e in &mut spec.effects {
            for b in e.numerator.iter_mut().chain(&mut e.denominator) {
                *b = parameters.get(at).copied().unwrap_or(0.0);
                at += 1;
            }
        }
        spec
    };
    let residuals = |p: &[f64]| {
        let spec = make(p);
        let adjusted = spec.adjusted(y, x)?;
        let (r, ld) = arima::likelihood_residuals(&spec.noise, &adjusted)
            .map_err(|e| error(&e.to_string()))?;
        let scale = (ld / (2.0 * nf)).exp();
        Ok(r.into_iter().map(|v| v * scale).collect::<Vec<_>>())
    };
    let mut initial = vec![0.0; k];
    if let Some(v) = initial.first_mut() {
        *v = y.iter().sum::<f64>()
            / f64::from(u32::try_from(y.len()).map_err(|_| error("history limit"))?);
    }
    // The AR coefficient and every denominator coefficient stay inside the stationary range;
    // the Guide starts all denominator parameters at zero.
    let stationary = (-1.0 + 1e-8, 1.0 - 1e-8);
    let mut bounds = vec![(f64::NEG_INFINITY, f64::INFINITY); k];
    if let Some(v) = bounds.get_mut(1) {
        *v = stationary;
    }
    let mut at = 2;
    for e in &template.effects {
        at += e.numerator.len();
        for v in bounds.iter_mut().skip(at).take(e.denominator.len()) {
            *v = stationary;
        }
        at += e.denominator.len();
    }
    let (parameters, iterations) = time_series::lm(initial, &bounds, residuals)?;
    let mut spec = make(&parameters);
    let adjusted = spec.adjusted(y, x)?;
    let (r, _) =
        arima::likelihood_residuals(&spec.noise, &adjusted).map_err(|e| error(&e.to_string()))?;
    spec.noise.sigma2 = r.iter().map(|v| v * v).sum::<f64>() / df;
    let objective = spec.objective(y, x)?;
    let rmse = spec.noise.sigma2.sqrt();
    Ok(Fit {
        spec: FittedSpec::Transfer(spec),
        objective,
        iterations,
        final_level: 0.0,
        final_trend: 0.0,
        rmse,
    })
}
