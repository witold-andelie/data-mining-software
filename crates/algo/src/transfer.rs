//! Transfer regression with stationary AR(1) noise (Algorithms Guide, Time Series).
//! `ts_more/ts_tf_static`, `ts_tf_num`, `ts_tf_den`, `ts_tf_delay` establish
//! the lost prefix and PMML numerator sign: coefficient zero is positive,
//! subsequent numerator coefficients are subtracted. Denominator initialization
//! remains GAP (`ts_tf_diff`, `ts_tf_two`); no invented prehistory is used.
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
                || x.len() < n
                || x.len() > 1024
                || x.iter().chain(&e.numerator).any(|v| !v.is_finite())
            {
                return Err(error("invalid transfer predictor/order/length"));
            }
            let prefix = e.difference + e.delay + e.numerator.len() - 1;
            start = start.max(prefix);
            let z = arima::difference(x, e.difference);
            for (i, out) in signal.iter_mut().enumerate().skip(prefix) {
                for (j, b) in e.numerator.iter().enumerate() {
                    let at = i - e.difference - e.delay - j;
                    *out += b * z
                        .get(at)
                        .copied()
                        .ok_or_else(|| error("transfer lag absent"))?;
                }
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
            format!("<PredicterEffect field=\"{field}\" transformation=\"none\" delay=\"{}\" nonSeasonalDiff=\"{}\" seasonalDiff=\"0\"><Numerator><NonseasonFactor maximumOrder=\"{}\"><Array n=\"{}\" type=\"real\">{coefs}</Array></NonseasonFactor></Numerator><Denominator><NonseasonFactor maximumOrder=\"0\"><Array n=\"0\" type=\"real\"/></NonseasonFactor></Denominator></PredicterEffect>",e.delay,e.difference,e.numerator.len(),e.numerator.len())
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
        .map(|e| e.numerator.len())
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
            for b in &mut e.numerator {
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
    let mut bounds = vec![(f64::NEG_INFINITY, f64::INFINITY); k];
    if let Some(v) = bounds.get_mut(1) {
        *v = (-1.0 + 1e-8, 1.0 - 1e-8);
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
