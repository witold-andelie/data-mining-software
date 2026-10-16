//! What a node produces: data for the nodes downstream, a model (build nodes) or a document
//! (output nodes such as Data Audit, Statistics, Matrix).

use dms_pmml::Pmml;
use dms_types::Table;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum NodeOutput {
    Data(Arc<Table>),
    /// A trained model; the model nuggets linked to the build node score with it.
    Model(Arc<Pmml>),
    Document(Arc<Document>),
}

impl NodeOutput {
    pub fn data(&self) -> Option<&Arc<Table>> {
        match self {
            NodeOutput::Data(table) => Some(table),
            _ => None,
        }
    }

    pub fn model(&self) -> Option<&Arc<Pmml>> {
        match self {
            NodeOutput::Model(model) => Some(model),
            _ => None,
        }
    }

    pub fn document(&self) -> Option<&Arc<Document>> {
        match self {
            NodeOutput::Document(document) => Some(document),
            _ => None,
        }
    }
}

/// A real as Modeler's documents print it with `decimals` places: the shortest text that reads
/// back to the same double, rounded half-to-even (Java's `DecimalFormat`). Rounding the shortest
/// text instead of the exact binary value matters at 15 places: Modeler prints the variance
/// 1485.566997852143 as `1485.566997852143000`, not `1485.566997852142975` (`team/cluster_analysis`
/// Statistics), and the standard deviation 3.3988100979788345 as `3.398810097978834`.
/// Non-finite values print as `$null$`.
pub fn format_real(value: f64, decimals: usize) -> String {
    if !value.is_finite() {
        return "$null$".to_string();
    }
    // Rust's `Display` for f64 is the shortest round-trip text, never in exponent form.
    let text = format!("{}", value.abs());
    let (int_part, frac_part) = text.split_once('.').unwrap_or((text.as_str(), ""));
    let mut digits: Vec<u8> = int_part.bytes().collect();
    let int_len = digits.len();
    let mut frac: Vec<u8> = frac_part.bytes().collect();
    if frac.len() > decimals {
        let rest = frac.split_off(decimals);
        digits.extend_from_slice(&frac);
        let first = rest.first().copied().unwrap_or(b'0');
        let beyond = rest.iter().skip(1).any(|&d| d != b'0');
        let last_odd = digits.last().is_some_and(|&d| (d - b'0') % 2 == 1);
        if first > b'5' || (first == b'5' && (beyond || last_odd)) {
            // Add one unit in the last kept place, carrying leftwards.
            let mut i = digits.len();
            loop {
                if i == 0 {
                    digits.insert(0, b'1');
                    break;
                }
                i -= 1;
                if digits[i] == b'9' {
                    digits[i] = b'0';
                } else {
                    digits[i] += 1;
                    break;
                }
            }
        }
    } else {
        digits.extend_from_slice(&frac);
        digits.resize(int_len + decimals, b'0');
    }
    let split = digits.len() - decimals;
    let mut out = String::with_capacity(digits.len() + 2);
    if value < 0.0 && digits.iter().any(|&d| d != b'0') {
        out.push('-');
    }
    out.extend(digits[..split].iter().map(|&d| char::from(d)));
    if decimals > 0 {
        out.push('.');
        out.extend(digits[split..].iter().map(|&d| char::from(d)));
    }
    out
}

/// An output node's result, as text the way Modeler shows and exports it, so it compares with
/// the oracle's `outputs/` files (`docs/oracle.md`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    pub title: String,
    pub tables: Vec<DocumentTable>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentTable {
    /// The section the table belongs to (`Statistics`, `Pearson Correlations`, ...).
    pub title: Option<String>,
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::format_real;

    #[test]
    fn reals_round_their_shortest_text_half_to_even() {
        assert_eq!(format_real(1485.566997852143, 15), "1485.566997852143000");
        assert_eq!(format_real(3.3988100979788345, 15), "3.398810097978834");
        assert_eq!(format_real(0.4166666666666667, 3), "0.417");
        assert_eq!(format_real(2.5, 0), "2");
        assert_eq!(format_real(9.9995, 3), "10.000");
        assert_eq!(format_real(-0.25, 3), "-0.250");
        assert_eq!(format_real(-0.0001, 3), "0.000");
        assert_eq!(format_real(f64::NAN, 3), "$null$");
    }
}
