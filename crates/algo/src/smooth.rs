//! Phase B (`ts_more`): Brown double smoothing, damped trend, additive seasons,
//! Winters additive/multiplicative and log/sqrt transforms use component
//! recursions and reverse-time initialization from the Algorithms Guide
//! (1994–2016, pp.379–383). All ten explicit smoothing cases pass rule 3;
//! largest point/bound discrepancy is Winters multiplicative, 0.001535 SD.
//! Additive forecast variances propagate independent unit innovations;
//! multiplicative seasons use local derivatives (validated at six steps only;
//! long-horizon variance parity remains GAP). Intervals use Student t and n-k.
//!
//! Exponential smoothing scoring for the Time Series nugget.
//!
//! Simple model (`ts_simple` sales, PMML `Level alpha="0.9999889207657683"` and
//! `initLevel="20.50000553967849"`): the one-step prediction starts at the initial
//! level, then `ℓ_t = α y_t + (1-α) ℓ_{t-1}`. Record 1 is 20.50000553967849; record 2
//! is 20.500000000061373. Forecasts stay at the final level (43.9999944603215), which
//! is also the saved `smoothedValue`. `ts_simple` cost uses the same recursion
//! (`initLevel` 8.250002769839247, final level 19.999997230160755).
//!
//! Holt linear trend (`ts_holt` sales, `trend="additive"`, `gamma="0.4"`,
//! `initLevel="20"`, `initTrend="0.4999999999999999"`):
//! `ŷ_t = ℓ_{t-1} + b_{t-1}`, `ℓ_t = α y_t + (1-α)(ℓ_{t-1}+b_{t-1})`,
//! `b_t = γ (ℓ_t − ℓ_{t-1}) + (1-γ) b_{t-1}`. Record 1 is 20.5. The series is an
//! exact line and `errVariance` is 0, so every history point equals the observation
//! and forecast `h` is `44 + 0.5 h` (h=6 is 47). Cost is `8 + 0.25 t` extended
//! (h=6 is 21.5). A short non-linear series in the unit test locks the same update
//! when the residual is not zero.
//!
//! Intervals use the saved `errVariance` as `σ²` and `degreesOfFreedom` for a Student
//! t quantile (not the normal quantile, and not `RMSE`²). `ts_simple` sales is 95%
//! with df 47: record 1 is 19.49412437553557 .. 21.50588670382141, half-width
//! `t_{0.975,47} √σ²`. Every history row uses that one-step width. Forecast step `h`
//! widens with `v_h = 1 + (h−1) α²` (h=2 is that width times `√(1+α²)`).
//! Holt's factor is the ETS(A,A,N) sum of squared ψ weights,
//! `v_h = 1 + Σ_{j=1}^{h-1} (α + j α γ)²`, with PMML `gamma` the component-form
//! weight γ. `ts_holt` has `σ² = 0`, so every bound equals the point.

use crate::stats::student_t_quantile;
use std::fmt;

const MAX_HISTORY: usize = 100_000;
const MAX_HORIZON: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Simple,
    Holt,
    Brown,
    Damped,
    Seasonal,
    WintersAdd,
    WintersMul,
}

impl Kind {
    pub fn trend(self) -> bool {
        !matches!(self, Self::Simple | Self::Seasonal)
    }
    pub fn seasonal(self) -> bool {
        matches!(self, Self::Seasonal | Self::WintersAdd | Self::WintersMul)
    }
    pub fn parameters(self) -> usize {
        1 + usize::from(self.trend() && self != Self::Brown)
            + usize::from(self == Self::Damped)
            + usize::from(self.seasonal())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub level: f64,
    pub trend: f64,
    pub seasons: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeneralSpec {
    pub kind: Kind,
    pub transform: crate::arima::Transform,
    pub alpha: f64,
    pub gamma: f64,
    pub phi: f64,
    pub delta: f64,
    pub initial: State,
    pub sigma2: f64,
    pub df: f64,
}

impl GeneralSpec {
    fn check(&self) -> Result<(), SmoothError> {
        let s = &self.initial;
        if [
            self.alpha,
            self.gamma,
            self.phi,
            self.delta,
            self.sigma2,
            self.df,
            s.level,
            s.trend,
        ]
        .iter()
        .any(|v| !v.is_finite())
            || s.seasons.iter().any(|v| !v.is_finite())
            || self.df <= 0.0
            || self.sigma2 < 0.0
            || !(0.0..=1.0).contains(&self.alpha)
            || !(0.0..=1.0).contains(&self.gamma)
            || !(0.0..=1.0).contains(&self.phi)
            || !(0.0..=1.0).contains(&self.delta)
            || (self.kind == Kind::Brown && (self.alpha <= 0.0 || self.alpha >= 1.0))
            || (self.kind.seasonal() && !(2..=64).contains(&s.seasons.len()))
        {
            return Err(SmoothError::Settings(
                "invalid smoothing parameters/state".into(),
            ));
        }
        Ok(())
    }

    /// Component recursions; Brown stores the first smoothed level and its implied trend.
    pub(crate) fn step(
        &self,
        state: &mut State,
        obs: Option<f64>,
        index: usize,
    ) -> Result<f64, SmoothError> {
        let seasonal = self.kind.seasonal();
        let j = index.checked_rem(state.seasons.len()).unwrap_or(0);
        let season = if seasonal {
            *state
                .seasons
                .get(j)
                .ok_or_else(|| SmoothError::Data("missing seasonal state".into()))?
        } else {
            0.0
        };
        let base = state.level
            + if self.kind == Kind::Brown {
                state.trend / self.alpha
            } else if self.kind.trend() {
                self.phi * state.trend
            } else {
                0.0
            };
        let prediction = if self.kind == Kind::WintersMul {
            base * season
        } else {
            base + season
        };
        let y = obs.unwrap_or(prediction);
        let old = state.level;
        if self.kind == Kind::Brown {
            let second = old - (1.0 - self.alpha) / self.alpha * state.trend;
            state.level = self.alpha * y + (1.0 - self.alpha) * old;
            let next_second = self.alpha * state.level + (1.0 - self.alpha) * second;
            state.trend = self.alpha / (1.0 - self.alpha) * (state.level - next_second);
        } else {
            let deseason = if self.kind == Kind::WintersMul {
                if season == 0.0 {
                    return Err(SmoothError::Data("zero multiplicative season".into()));
                }
                y / season
            } else {
                y - season
            };
            state.level = self.alpha * deseason + (1.0 - self.alpha) * base;
            if self.kind.trend() {
                state.trend =
                    self.gamma * (state.level - old) + (1.0 - self.gamma) * self.phi * state.trend;
            }
            if seasonal {
                let new = if self.kind == Kind::WintersMul {
                    if state.level == 0.0 {
                        return Err(SmoothError::Data("zero multiplicative level".into()));
                    }
                    self.delta * y / state.level + (1.0 - self.delta) * season
                } else {
                    self.delta * (y - state.level) + (1.0 - self.delta) * season
                };
                if let Some(slot) = state.seasons.get_mut(j) {
                    *slot = new;
                }
            }
        }
        if !prediction.is_finite()
            || !state.level.is_finite()
            || !state.trend.is_finite()
            || state.seasons.iter().any(|v| !v.is_finite())
        {
            return Err(SmoothError::Data("nonfinite smoothing recurrence".into()));
        }
        Ok(prediction)
    }

    pub(crate) fn run(&self, z: &[f64]) -> Result<(Vec<f64>, State), SmoothError> {
        self.check()?;
        if z.len() > MAX_HISTORY || z.iter().any(|v| !v.is_finite()) {
            return Err(SmoothError::Data("invalid smoothing history".into()));
        }
        let mut state = self.initial.clone();
        let points = z
            .iter()
            .enumerate()
            .map(|(i, y)| self.step(&mut state, Some(*y), i))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((points, state))
    }

    pub fn score(
        &self,
        y: &[f64],
        horizon: usize,
        confidence: f64,
    ) -> Result<SeriesScore, SmoothError> {
        crate::check_cancelled()?;
        if y.is_empty() || horizon > 512 {
            return Err(SmoothError::Data(
                "smoothing requires history and horizon <=512".into(),
            ));
        }
        let z = crate::arima::transform(self.transform, y)
            .map_err(|e| SmoothError::Data(e.to_string()))?;
        let (points, final_state) = self.run(&z)?;
        let t = student_t_quantile((1.0 + confidence / 100.0) / 2.0, self.df)
            .ok_or_else(|| SmoothError::Settings("invalid confidence/df".into()))?;
        let back = |p: f64, v: f64| -> Result<Forecast, SmoothError> {
            let f = crate::arima::back_transform(self.transform, p, v.sqrt(), t)
                .map_err(|e| SmoothError::Data(e.to_string()))?;
            if [f.point, f.lower, f.upper].iter().any(|v| !v.is_finite()) {
                return Err(SmoothError::Data("nonfinite inverse transform".into()));
            }
            Ok(Forecast {
                point: f.point,
                lower: f.lower,
                upper: f.upper,
            })
        };
        let fitted = points
            .into_iter()
            .map(|p| back(p, self.sigma2))
            .collect::<Result<Vec<_>, _>>()?;
        let mut state = final_state.clone();
        let mut means = Vec::new();
        for h in 0..horizon {
            crate::check_cancelled()?;
            means.push(self.step(&mut state, None, z.len().saturating_add(h))?);
        }
        // Impulse propagation is exact for additive component models. Multiplicative
        // seasonality uses the local derivative; longer-horizon variance remains GAP.
        let mut variance = vec![self.sigma2; horizon];
        let eps = 1e-5 * final_state.level.abs().max(1.0);
        for j in 0..horizon {
            crate::check_cancelled()?;
            let mut state = final_state.clone();
            for (h, mean) in means.iter().enumerate() {
                let pred = self.step(&mut state, None, z.len().saturating_add(h))?;
                if h == j {
                    let mut before = final_state.clone();
                    for k in 0..j {
                        self.step(&mut before, None, z.len().saturating_add(k))?;
                    }
                    self.step(&mut before, Some(*mean + eps), z.len().saturating_add(h))?;
                    state = before;
                } else if h > j
                    && let Some(v) = variance.get_mut(h)
                {
                    *v += self.sigma2 * ((pred - mean) / eps).powi(2);
                }
            }
        }
        let forecasts = means
            .into_iter()
            .zip(variance)
            .map(|(p, v)| back(p, v))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SeriesScore { fitted, forecasts })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SmoothSpec {
    General(GeneralSpec),
    /// No trend element (`ts_simple`).
    Simple {
        alpha: f64,
        init_level: f64,
        sigma2: f64,
        df: f64,
    },
    /// Additive trend (`ts_holt`). `gamma` is `Trend_ExpoSmooth/@gamma`.
    Holt {
        alpha: f64,
        gamma: f64,
        init_level: f64,
        init_trend: f64,
        sigma2: f64,
        df: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Forecast {
    pub point: f64,
    pub lower: f64,
    pub upper: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeriesScore {
    /// One prediction per historical observation, including the first
    /// (`ts_simple` / `ts_holt` row 1).
    pub fitted: Vec<Forecast>,
    pub forecasts: Vec<Forecast>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmoothError {
    Cancelled,
    Data(String),
    Settings(String),
}

impl fmt::Display for SmoothError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SmoothError::Cancelled => f.write_str("cancelled"),
            SmoothError::Data(message) | SmoothError::Settings(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SmoothError {}

/// One-step smoothing over `y`, then `horizon` forecasts from the final state.
pub fn score(
    spec: &SmoothSpec,
    y: &[f64],
    horizon: usize,
    conf_percent: f64,
) -> Result<SeriesScore, SmoothError> {
    crate::check_cancelled()?;
    if let SmoothSpec::General(g) = spec {
        return g.score(y, horizon, conf_percent);
    }
    spec.check()?;
    if y.is_empty() {
        return Err(SmoothError::Data("Time Series series is empty".into()));
    }
    if y.len() > MAX_HISTORY {
        return Err(SmoothError::Data(format!(
            "Time Series history of {} exceeds {MAX_HISTORY}",
            y.len()
        )));
    }
    if horizon > MAX_HORIZON {
        return Err(SmoothError::Settings(format!(
            "GAP: forecast span {horizon} exceeds {MAX_HORIZON}"
        )));
    }
    if y.iter().any(|value| !value.is_finite()) {
        return Err(SmoothError::Data(
            "Time Series target has a non-finite value".into(),
        ));
    }
    let p = (1.0 + conf_percent / 100.0) / 2.0;
    let tcrit = student_t_quantile(p, spec.df()).ok_or_else(|| {
        SmoothError::Settings(format!(
            "GAP: confidence level {conf_percent} with df {} has no t quantile",
            spec.df()
        ))
    })?;
    let (points, level, trend) = recurse(spec, y)?;
    let mut fitted = Vec::with_capacity(points.len());
    let history_scale = se(spec, 1)?;
    for point in points {
        crate::check_cancelled()?;
        fitted.push(interval(point, history_scale, tcrit)?);
    }
    let mut forecasts = Vec::with_capacity(horizon);
    for step in 1..=horizon {
        crate::check_cancelled()?;
        let point = match spec {
            SmoothSpec::General(_) => {
                return Err(SmoothError::Settings("general smoothing dispatch".into()));
            }
            SmoothSpec::Simple { .. } => level,
            SmoothSpec::Holt { .. } => {
                let hf = as_f64(step)?;
                level + hf * trend
            }
        };
        let scale = se(spec, step)?;
        forecasts.push(interval(point, scale, tcrit)?);
    }
    Ok(SeriesScore { fitted, forecasts })
}

impl SmoothSpec {
    fn df(&self) -> f64 {
        match self {
            SmoothSpec::General(g) => g.df,
            SmoothSpec::Simple { df, .. } | SmoothSpec::Holt { df, .. } => *df,
        }
    }

    fn sigma2(&self) -> f64 {
        match self {
            SmoothSpec::General(g) => g.sigma2,
            SmoothSpec::Simple { sigma2, .. } | SmoothSpec::Holt { sigma2, .. } => *sigma2,
        }
    }

    fn check(&self) -> Result<(), SmoothError> {
        let (alpha, gamma, init_level, init_trend, sigma2, df) = match self {
            SmoothSpec::General(g) => return g.check(),
            SmoothSpec::Simple {
                alpha,
                init_level,
                sigma2,
                df,
            } => (*alpha, 0.0, *init_level, 0.0, *sigma2, *df),
            SmoothSpec::Holt {
                alpha,
                gamma,
                init_level,
                init_trend,
                sigma2,
                df,
            } => (*alpha, *gamma, *init_level, *init_trend, *sigma2, *df),
        };
        if !alpha.is_finite()
            || !gamma.is_finite()
            || !init_level.is_finite()
            || !init_trend.is_finite()
        {
            return Err(SmoothError::Settings(
                "exponential smoothing parameter is not finite".into(),
            ));
        }
        if !sigma2.is_finite() || sigma2 < 0.0 {
            return Err(SmoothError::Settings(
                "exponential smoothing errVariance must be zero or positive".into(),
            ));
        }
        if !df.is_finite() || df <= 0.0 {
            return Err(SmoothError::Settings(
                "exponential smoothing degreesOfFreedom must be positive".into(),
            ));
        }
        Ok(())
    }
}

pub(crate) fn recurse(spec: &SmoothSpec, y: &[f64]) -> Result<(Vec<f64>, f64, f64), SmoothError> {
    match spec {
        SmoothSpec::General(g) => {
            let (p, state) = g.run(y)?;
            Ok((p, state.level, state.trend))
        }
        SmoothSpec::Simple {
            alpha, init_level, ..
        } => {
            let mut level = *init_level;
            let mut points = Vec::with_capacity(y.len());
            for value in y {
                crate::check_cancelled()?;
                points.push(level);
                level = alpha * value + (1.0 - alpha) * level;
                if !level.is_finite() {
                    return Err(SmoothError::Data(
                        "exponential smoothing level is not finite".into(),
                    ));
                }
            }
            Ok((points, level, 0.0))
        }
        SmoothSpec::Holt {
            alpha,
            gamma,
            init_level,
            init_trend,
            ..
        } => {
            let mut level = *init_level;
            let mut trend = *init_trend;
            let mut points = Vec::with_capacity(y.len());
            for value in y {
                crate::check_cancelled()?;
                let point = level + trend;
                if !point.is_finite() {
                    return Err(SmoothError::Data(
                        "exponential smoothing prediction is not finite".into(),
                    ));
                }
                points.push(point);
                let next_level = alpha * value + (1.0 - alpha) * point;
                let next_trend = gamma * (next_level - level) + (1.0 - gamma) * trend;
                if !next_level.is_finite() || !next_trend.is_finite() {
                    return Err(SmoothError::Data(
                        "exponential smoothing state is not finite".into(),
                    ));
                }
                level = next_level;
                trend = next_trend;
            }
            Ok((points, level, trend))
        }
    }
}

/// Standard deviation of the h-step forecast error, `h` starting at 1.
fn se(spec: &SmoothSpec, h: usize) -> Result<f64, SmoothError> {
    let factor = variance_factor(spec, h)?;
    let variance = spec.sigma2() * factor;
    if !variance.is_finite() || variance < 0.0 {
        return Err(SmoothError::Data(
            "exponential smoothing error variance is not finite".into(),
        ));
    }
    Ok(variance.sqrt())
}

/// `v_h` in `Var = σ² v_h`. Simple: `1 + (h-1) α²` (`ts_simple`).
/// Holt: `1 + Σ_{j=1}^{h-1} (α + j αγ)²`.
fn variance_factor(spec: &SmoothSpec, h: usize) -> Result<f64, SmoothError> {
    if h == 0 {
        return Err(SmoothError::Data(
            "exponential smoothing forecast step starts at 1".into(),
        ));
    }
    let hf = as_f64(h)?;
    let factor = match spec {
        SmoothSpec::General(_) => {
            return Err(SmoothError::Settings("general variance dispatch".into()));
        }
        SmoothSpec::Simple { alpha, .. } => 1.0 + (hf - 1.0) * alpha * alpha,
        SmoothSpec::Holt { alpha, gamma, .. } => {
            let beta = alpha * gamma;
            let steps = hf - 1.0;
            let inside =
                alpha * alpha + alpha * beta * hf + beta * beta * hf * (2.0 * hf - 1.0) / 6.0;
            1.0 + steps * inside
        }
    };
    if !factor.is_finite() || factor < 0.0 {
        return Err(SmoothError::Data(
            "exponential smoothing variance factor is not usable".into(),
        ));
    }
    Ok(factor)
}

fn interval(point: f64, se: f64, tcrit: f64) -> Result<Forecast, SmoothError> {
    let lower = point - tcrit * se;
    let upper = point + tcrit * se;
    if !point.is_finite() || !lower.is_finite() || !upper.is_finite() {
        return Err(SmoothError::Data(
            "Time Series prediction is not finite".into(),
        ));
    }
    Ok(Forecast {
        point,
        lower,
        upper,
    })
}

fn as_f64(value: usize) -> Result<f64, SmoothError> {
    u32::try_from(value).map(f64::from).map_err(|_| {
        SmoothError::Settings(format!(
            "GAP: forecast step {value} exceeds the supported maximum"
        ))
    })
}

impl From<dms_types::Cancelled> for SmoothError {
    fn from(_: dms_types::Cancelled) -> Self {
        Self::Cancelled
    }
}

#[cfg(test)]
#[allow(clippy::excessive_precision, reason = "oracle CSV prints 15 decimals")]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, abs: f64) -> bool {
        (a - b).abs() <= abs
    }

    fn sales() -> Vec<f64> {
        (1u32..=48).map(|t| 20.0 + 0.5 * f64::from(t)).collect()
    }

    fn cost() -> Vec<f64> {
        (1u32..=48).map(|t| 8.0 + 0.25 * f64::from(t)).collect()
    }

    fn simple_sales() -> SmoothSpec {
        SmoothSpec::Simple {
            alpha: 0.9999889207657683,
            init_level: 20.50000553967849,
            sigma2: 0.25000542184106844,
            df: 47.0,
        }
    }

    fn simple_cost() -> SmoothSpec {
        SmoothSpec::Simple {
            alpha: 0.9999889207657625,
            init_level: 8.250002769839247,
            sigma2: 0.06250135546026765,
            df: 47.0,
        }
    }

    fn holt_sales() -> SmoothSpec {
        SmoothSpec::Holt {
            alpha: 0.7999999999999999,
            gamma: 0.4,
            init_level: 20.0,
            init_trend: 0.4999999999999999,
            sigma2: 0.0,
            df: 46.0,
        }
    }

    fn holt_cost() -> SmoothSpec {
        SmoothSpec::Holt {
            alpha: 0.7999999999999999,
            gamma: 0.4,
            init_level: 8.0,
            init_trend: 0.24999999999999992,
            sigma2: 0.0,
            df: 46.0,
        }
    }

    #[test]
    fn simple_matches_ts_simple_sales_and_cost() {
        let sales = score(&simple_sales(), &sales(), 6, 95.0).unwrap();
        let first = sales.fitted[0];
        assert!(close(first.point, 20.500005539678490, 1e-12));
        assert!(close(first.lower, 19.494124375535570, 1e-12));
        assert!(close(first.upper, 21.505886703821410, 1e-12));
        let second = sales.fitted[1];
        assert!(close(second.point, 20.500000000061373, 1e-12));
        assert!(close(second.lower, 19.494118835918453, 1e-12));
        let third = sales.fitted[2];
        assert!(close(third.point, 20.999994460382887, 1e-12));
        let h1 = sales.forecasts[0];
        assert!(close(h1.point, 43.999994460321503, 1e-12));
        assert!(close(h1.lower, 42.994113296178583, 1e-12));
        assert!(close(h1.upper, 45.005875624464423, 1e-12));
        let h2 = sales.forecasts[1];
        assert!(close(h2.point, 43.999994460321503, 1e-12));
        assert!(close(h2.lower, 42.577471556109003, 1e-12));
        assert!(close(h2.upper, 45.422517364534002, 1e-12));
        let h6 = sales.forecasts[5];
        assert!(close(h6.lower, 41.536121614670620, 1e-12));
        assert!(close(h6.upper, 46.463867305972386, 1e-12));
        let width1 = first.upper - first.lower;
        let width2 = h2.upper - h2.lower;
        let alpha: f64 = 0.9999889207657683;
        assert!(close(width2 / width1, (1.0 + alpha * alpha).sqrt(), 1e-12));

        let cost = score(&simple_cost(), &cost(), 6, 95.0).unwrap();
        let cost1 = cost.fitted[0];
        assert!(close(cost1.point, 8.250002769839247, 1e-12));
        assert!(close(cost1.lower, 7.747062187767784, 1e-12));
        assert!(close(cost1.upper, 8.752943351910709, 1e-12));
        let cost6 = cost.forecasts[5];
        assert!(close(cost6.point, 19.999997230160755, 1e-12));
        assert!(close(cost6.lower, 18.768060807335313, 1e-12));
        assert!(close(cost6.upper, 21.231933652986196, 1e-12));
    }

    #[test]
    fn holt_matches_ts_holt_exact_line() {
        let sales = score(&holt_sales(), &sales(), 6, 95.0).unwrap();
        assert_eq!(sales.fitted.len(), 48);
        let first = sales.fitted[0];
        assert!(close(first.point, 20.5, 1e-12));
        assert_eq!(first.lower, first.point);
        assert_eq!(first.upper, first.point);
        let last = sales.fitted[47];
        assert!(close(last.point, 44.0, 1e-12));
        assert!(close(sales.forecasts[0].point, 44.5, 1e-12));
        assert!(close(sales.forecasts[5].point, 47.0, 1e-12));
        assert_eq!(sales.forecasts[5].lower, sales.forecasts[5].point);
        assert_eq!(sales.forecasts[5].upper, sales.forecasts[5].point);

        let cost = score(&holt_cost(), &cost(), 6, 95.0).unwrap();
        assert!(close(cost.fitted[0].point, 8.25, 1e-12));
        assert!(close(cost.forecasts[0].point, 20.25, 1e-12));
        assert!(close(cost.forecasts[5].point, 21.5, 1e-12));
        assert_eq!(cost.forecasts[5].lower, cost.forecasts[5].point);
    }

    #[test]
    fn holt_update_on_a_series_with_residuals() {
        // Independent of the zero-residual oracle line. alpha=0.8, gamma=0.4,
        // level0=1, trend0=0.5, y = 1, 3, 2, 5.
        let spec = SmoothSpec::Holt {
            alpha: 0.8,
            gamma: 0.4,
            init_level: 1.0,
            init_trend: 0.5,
            sigma2: 1.0,
            df: 2.0,
        };
        let scored = score(&spec, &[1.0, 3.0, 2.0, 5.0], 2, 95.0).unwrap();
        let points: Vec<f64> = scored.fitted.iter().map(|f| f.point).collect();
        assert!(close(points[0], 1.5, 1e-12));
        assert!(close(points[1], 1.44, 1e-12));
        assert!(close(points[2], 3.5272, 1e-12));
        assert!(close(points[3], 2.655936, 1e-12));
        assert!(close(scored.forecasts[0].point, 5.63178368, 1e-12));
        assert!(close(scored.forecasts[1].point, 6.73238016, 1e-12));
    }

    #[test]
    fn holt_variance_matches_psi_weights() {
        let spec = SmoothSpec::Holt {
            alpha: 0.8,
            gamma: 0.4,
            init_level: 1.0,
            init_trend: 0.0,
            sigma2: 4.0,
            df: 46.0,
        };
        let alpha = 0.8;
        let beta = alpha * 0.4;
        for h in 1..=12 {
            let mut sum = 1.0;
            for j in 1..h {
                let psi = alpha + f64::from(u32::try_from(j).unwrap()) * beta;
                sum += psi * psi;
            }
            let factor = variance_factor(&spec, h).unwrap();
            assert!(close(factor, sum, 1e-12), "h={h} factor={factor} sum={sum}");
        }
        let simple = SmoothSpec::Simple {
            alpha: 0.3,
            init_level: 0.0,
            sigma2: 1.0,
            df: 47.0,
        };
        assert!(close(variance_factor(&simple, 1).unwrap(), 1.0, 0.0));
        assert!(close(
            variance_factor(&simple, 4).unwrap(),
            1.0 + 3.0 * 0.09,
            1e-15
        ));
    }

    #[test]
    fn rejects_empty_series_and_negative_variance() {
        let err = score(&simple_sales(), &[], 1, 95.0).unwrap_err();
        assert!(matches!(err, SmoothError::Data(_)));
        let mut bad = simple_sales();
        if let SmoothSpec::Simple { sigma2, .. } = &mut bad {
            *sigma2 = -1.0;
        }
        let err = score(&bad, &[1.0], 1, 95.0).unwrap_err();
        assert!(matches!(err, SmoothError::Settings(_)));
    }
}
