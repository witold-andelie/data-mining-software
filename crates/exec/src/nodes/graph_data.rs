//! Helpers the graph nodes share. A graph node's output is the data behind the graph, one
//! [`DocumentTable`](crate::DocumentTable) whose header and rows are those of the `embeddedSource`
//! in Modeler's VIZML export of the graph (`xtask/oracle/modeler_oracle.py`; `oracle/graphs/*`).
//! VIZML writes numbers as Java's `Double.toString` does, so the cells use [`java_double`].

/// `x` as Java's `Double.toString`: the shortest text that reads back as `x`, with at least one
/// fractional digit, in scientific notation (`1.0E-4`, `1.2E7`) outside [10^-3, 10^7).
pub(crate) fn java_double(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    let magnitude = x.abs();
    if (1e-3..1e7).contains(&magnitude) {
        return format!("{x:?}");
    }
    // Shortest round-trip digits in Rust's `{:e}` form, e.g. "1.234e-5", then Java's layout.
    let text = format!("{x:e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let mantissa = if mantissa.contains('.') {
        mantissa.to_string()
    } else {
        format!("{mantissa}.0")
    };
    format!("{mantissa}E{exponent}")
}

/// Bins drawn at most (a number of bins, or the bins a width yields).
pub(crate) const MAX_BINS: usize = 10_000;

/// How a graph bins a continuous x field (Histogram, Collection): `graphNumBins` (default 25),
/// `graphBinByWidth` with `graphBinWidth`, `automaticRangeX` (default true) or
/// `userDefinedRangeXMin`/`Max`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BinSettings {
    pub bins: usize,
    pub width: Option<f64>,
    pub range: Option<(f64, f64)>,
}

impl BinSettings {
    pub(crate) fn read(
        s: Option<&dms_strfmt::fastinfoset::Element>,
        node: &str,
    ) -> Result<BinSettings, crate::error::OperatorError> {
        use crate::error::OperatorError;
        use crate::settings;
        let bins = settings::integer(s, "graphNumBins")?
            .and_then(|n| usize::try_from(n).ok())
            .filter(|&n| (1..=MAX_BINS).contains(&n))
            .unwrap_or(25);
        let width = if settings::flag(s, "graphBinByWidth")?.unwrap_or(false) {
            let width = settings::real(s, "graphBinWidth")?.unwrap_or(1.0);
            if !(width.is_finite() && width > 0.0) {
                return Err(OperatorError::Settings(format!(
                    "{node} bin width {width} must be positive"
                )));
            }
            Some(width)
        } else {
            None
        };
        let range = if settings::flag(s, "automaticRangeX")?.unwrap_or(true) {
            None
        } else {
            let low = settings::real(s, "userDefinedRangeXMin")?.unwrap_or(0.0);
            let high = settings::real(s, "userDefinedRangeXMax")?.unwrap_or(0.0);
            if !(low.is_finite() && high.is_finite() && low < high) {
                return Err(OperatorError::Settings(format!(
                    "{node} range {low}..{high} is empty"
                )));
            }
            Some((low, high))
        };
        Ok(BinSettings { bins, width, range })
    }

    /// The bins for values in `low..=high` (the data's or the user range).
    pub(crate) fn bins(&self, low: f64, high: f64) -> Result<Bins, crate::error::OperatorError> {
        match self.width {
            Some(width) => {
                let start = (low / width).floor() * width;
                let count = ((high - start) / width).floor() + 1.0;
                if !(count.is_finite() && count <= MAX_BINS as f64) {
                    return Err(crate::error::OperatorError::Data(format!(
                        "bin width {width} gives too many bins"
                    )));
                }
                Ok(Bins {
                    start,
                    width,
                    count: count as usize,
                    high,
                    by_width: true,
                })
            }
            None => Ok(Bins {
                start: low,
                width: (low + (high - low) / self.bins as f64) - low,
                count: self.bins,
                high,
                by_width: false,
            }),
        }
    }
}

/// Equal-width bins (see `nodes::histogram` for the rules and the cases behind them).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Bins {
    pub start: f64,
    pub width: f64,
    pub count: usize,
    high: f64,
    by_width: bool,
}

impl Bins {
    /// The bin of `x`: `floor((x - start) / width)`, the maximum in the last bin.
    pub(crate) fn index(&self, x: f64) -> usize {
        if self.width > 0.0 {
            (((x - self.start) / self.width).floor().max(0.0) as usize)
                .min(self.count.saturating_sub(1))
        } else {
            0
        }
    }

    /// Bin centres: midpoints of accumulated edges, the last edge the maximum when binning by
    /// number.
    pub(crate) fn centres(&self) -> Vec<f64> {
        let mut edge = self.start;
        (0..self.count)
            .map(|bin| {
                let mut next = edge + self.width;
                if !self.by_width && bin + 1 == self.count {
                    next = self.high;
                }
                let centre = (edge + next) / 2.0;
                edge = next;
                centre
            })
            .collect()
    }
}

/// A category as VIZML writes it: reals as Java doubles (`team/stream_1_M`: churn `1.0`),
/// integers and anything else as their text (`team/decision_trees_heart`: heart disease `2`).
pub(crate) fn category_text(value: &dms_types::Value) -> String {
    match value {
        dms_types::Value::Integer(i) => i.to_string(),
        dms_types::Value::Real(r) => java_double(*r),
        other => other.to_string(),
    }
}

/// A count or percentage as the tooltips show it: at most three decimals, no trailing zeros
/// (`77`, `49.351`, `25`).
pub(crate) fn tooltip_number(x: f64) -> String {
    let text = crate::output::format_real(x, 3);
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    };
    if text == "-0" { "0".into() } else { text }
}

#[cfg(test)]
mod tests {
    use super::{java_double, tooltip_number};

    #[test]
    fn numbers_print_like_java() {
        assert_eq!(java_double(77.0), "77.0");
        assert_eq!(java_double(23.259999999999998), "23.259999999999998");
        assert_eq!(java_double(0.50808674), "0.50808674");
        assert_eq!(java_double(0.0001), "1.0E-4");
        assert_eq!(java_double(12_345_678.0), "1.2345678E7");
        assert_eq!(tooltip_number(49.35064935064935), "49.351");
        assert_eq!(tooltip_number(25.0), "25");
        assert_eq!(tooltip_number(77.0), "77");
    }
}
