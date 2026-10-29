//! Transfer regression with stationary AR(1) noise (Algorithms Guide, Time Series).
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
//! digits; zero or the raw first value do not fit. The prefix and the degrees of freedom do
//! not grow with the denominator (`ts_tf_diff`: 108 - 3 - 5 = 100).
use crate::{
    AlgoError, arima,
    time_series::{self, Fit, FittedSpec},
};

#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    pub field: String,
    pub difference: usize,
    pub delay: usize,
    /// All coefficients use the additive convention internally.
    pub numerator: Vec<f64>,
    /// `delta_1..delta_r` of `v_t = sum_i delta_i v_{t-i} + ...` (PMML's stored values).
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
            || self.noise.transform != arima::Transform::None
            || self.noise.d != 0
            || self.noise.seasonal_d != 0
            || self.noise.ar.len() != 1
            || !self.noise.ma.is_empty()
            || !self.noise.seasonal_ar.is_empty()
            || !self.noise.seasonal_ma.is_empty()
        {
            return Err(error(
                "GAP: transfer requires <=16 predictors and untransformed AR(1) noise",
            ));
        }
        let mut signal = vec![0.0; n];
        let mut start = 0;
        for (e, x) in self.effects.iter().zip(x) {
            if e.difference > 1
                || e.delay > 64
                || e.numerator.is_empty()
                || e.numerator.len() > 5
                || e.denominator.len() > 4
                || x.len() < n
                || x.len() > 1024
                || x.iter()
                    .chain(&e.numerator)
                    .chain(&e.denominator)
                    .any(|v| !v.is_finite())
            {
                return Err(error("invalid transfer predictor/order/length"));
            }
            let prefix = e.difference + e.delay + e.numerator.len() - 1;
            start = start.max(prefix);
            let z = arima::difference(x, e.difference);
            let mut response = vec![0.0; n];
            for (i, out) in response.iter_mut().enumerate().skip(prefix) {
                for (j, b) in e.numerator.iter().enumerate() {
                    let at = i - e.difference - e.delay - j;
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
        Ok(y.iter()
            .zip(signal)
            .skip(start)
            .map(|(y, v)| y - v)
            .collect())
    }

    pub fn objective(&self, y: &[f64], x: &[Vec<f64>]) -> Result<f64, AlgoError> {
        time_series::objective(
            &FittedSpec::Arima(self.noise.clone()),
            &self.adjusted(y, x)?,
        )
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
        let adjusted = self.adjusted(y, x)?;
        let noise = arima::score(&self.noise, &adjusted, horizon, confidence)
            .map_err(|e| error(&e.to_string()))?;
        let mut fitted = vec![None; start];
        let add = |f: arima::Forecast, s: f64| arima::Forecast {
            point: f.point + s,
            lower: f.lower + s,
            upper: f.upper + s,
        };
        fitted.extend(
            noise
                .fitted
                .into_iter()
                .zip(signal.iter().skip(start))
                .map(|(f, s)| f.map(|f| add(f, *s))),
        );
        let forecasts = noise
            .forecasts
            .into_iter()
            .zip(signal.iter().skip(y.len()))
            .map(|(f, s)| add(f, *s))
            .collect();
        Ok(arima::SeriesScore { fitted, forecasts })
    }

    pub(crate) fn effects_xml(&self) -> String {
        self.effects.iter().map(|e| {
            let field=e.field.replace('&',"&amp;").replace('<',"&lt;").replace('"',"&quot;");
            let coefs=e.numerator.iter().enumerate().map(|(i,v)|if i==0{*v}else{-v}).map(|v|v.to_string()).collect::<Vec<_>>().join(" ");
            let den=e.denominator.iter().map(f64::to_string).collect::<Vec<_>>().join(" ");
            let r=e.denominator.len();
            format!("<PredicterEffect field=\"{field}\" transformation=\"none\" delay=\"{}\" nonSeasonalDiff=\"{}\" seasonalDiff=\"0\"><Numerator><NonseasonFactor maximumOrder=\"{}\"><Array n=\"{}\" type=\"real\">{coefs}</Array></NonseasonFactor></Numerator><Denominator><NonseasonFactor maximumOrder=\"{r}\"><Array n=\"{r}\" type=\"real\">{den}</Array></NonseasonFactor></Denominator></PredicterEffect>",e.delay,e.difference,e.numerator.len(),e.numerator.len())
        }).collect()
    }
}

/// Fit only the supplied orders; template coefficients and noise state are discarded.
pub fn fit(y: &[f64], x: &[Vec<f64>], template: &TransferSpec) -> Result<Fit, AlgoError> {
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
