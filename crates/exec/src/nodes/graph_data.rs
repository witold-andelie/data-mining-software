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
