//! Histogram (Graphs): the counts of a continuous field in equal-width bins, optionally broken
//! down by an overlay (colour) field. The output is the data behind the graph, as Modeler's
//! VIZML export holds it (`nodes::graph_data`; `oracle/graphs/hist_*`,
//! `xtask/oracle/probes/graph_streams.py`).
//!
//! Settings: `graphXField`, `graphNumBins` (default 25), `graphBinByWidth` with `graphBinWidth`,
//! `automaticRangeX` (default true) or `userDefinedRangeXMin`/`Max`, `graphColorField`,
//! `graphNormalizeData`. `histogramSeparateBands` is rejected; `graphNormalCurve` only draws.
//!
//! Rules:
//! - The range is the field's minimum and maximum (or the user range; values outside it are
//!   left out, both ends included). By number of bins the first bin starts at the minimum and the
//!   width is `(min + (max - min) / bins) - min`, the first edge minus the start, which differs
//!   from `(max - min) / bins` in the last bits (`hist_age`: 2.3599999999999994, `hist_age_range`:
//!   1.6000000000000014, `hist_na`, `hist_age_bins10`) and decides where values on an edge fall
//!   (`hist_age_range`: Age 36 and 52). By width, the first bin starts at
//!   `floor(min / width) * width` and there are as many bins as reach the maximum
//!   (`hist_age_width5`: 15 to 75, 12 bins).
//! - A value goes to bin `floor((x - start) / width)`, the maximum to the last bin.
//! - Rows: the bin centre and the count. Edges accumulate from the start (`edge += width`), the
//!   centre is the midpoint of a bin's edges, and by number of bins the last edge is the maximum
//!   (all 322 centres of the probe and team histograms, to the last bit); with an overlay, one row
//!   per bin and overlay value (in order of first appearance, zero counts included)
//!   (`hist_age_color`: 125 rows). `graphNormalizeData` gives the percentage of the bin's records,
//!   in a column named `__normalized__` (`hist_age_color_norm`); an empty bin's cells are empty
//!   (`team/stream_1_M`). Real overlay values print as Java doubles (`1.0`).
//!
//! GAP: null values (no probe has them) are left out.

use super::graph_data::{category_text, java_double};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Table, Value};
use std::sync::Arc;

/// Bins drawn at most (`graphNumBins` and the bins a width yields).
const MAX_BINS: usize = 10_000;

struct Histogram {
    field: String,
    overlay: Option<String>,
    normalize: bool,
    bins: usize,
    width: Option<f64>,
    range: Option<(f64, f64)>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    if settings::flag(s, "histogramSeparateBands")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "Histogram separate bands are not supported yet".into(),
        ));
    }
    let field = settings::text(s, "graphXField")
        .filter(|f| !f.trim().is_empty())
        .ok_or_else(|| OperatorError::Settings("Histogram: no field (graphXField)".into()))?;
    let bins = settings::integer(s, "graphNumBins")?
        .and_then(|n| usize::try_from(n).ok())
        .filter(|&n| (1..=MAX_BINS).contains(&n))
        .unwrap_or(25);
    let width = if settings::flag(s, "graphBinByWidth")?.unwrap_or(false) {
        let width = settings::real(s, "graphBinWidth")?.unwrap_or(1.0);
        if !(width.is_finite() && width > 0.0) {
            return Err(OperatorError::Settings(format!(
                "Histogram bin width {width} must be positive"
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
                "Histogram range {low}..{high} is empty"
            )));
        }
        Some((low, high))
    };
    Ok(Box::new(Histogram {
        field,
        overlay: settings::text(s, "graphColorField").filter(|f| !f.trim().is_empty()),
        normalize: settings::flag(s, "graphNormalizeData")?.unwrap_or(false),
        bins,
        width,
        range,
    }))
}

impl Operator for Histogram {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Histogram is a graph node and produces a Document, not a Table".into(),
        ))
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        _model: Option<Arc<dms_pmml::Pmml>>,
        _context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Histogram needs one input".into()))?;
        let position = |name: &str| {
            input
                .model()
                .position(name)
                .ok_or_else(|| OperatorError::Data(format!("no field {name}")))
        };
        let x = position(&self.field)?;
        let overlay = self.overlay.as_deref().map(position).transpose()?;
        let present = |row: &[Value], at: usize| -> Option<Value> {
            let value = row.get(at)?;
            let field = input.model().fields().get(at)?;
            (!value.is_null() && !field.is_missing(value)).then(|| value.clone())
        };

        // The records drawn: (x, overlay value).
        let mut points: Vec<(f64, Option<String>)> = Vec::new();
        for row in input.rows() {
            let Some(value) = present(row, x).and_then(|v| v.as_f64()) else {
                continue;
            };
            let colour = match overlay {
                Some(at) => match present(row, at) {
                    Some(c) => Some(category_text(&c)),
                    None => continue,
                },
                None => None,
            };
            points.push((value, colour));
        }
        let (low, high) = match self.range {
            Some(range) => range,
            None => points
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), (v, _)| {
                    (l.min(*v), h.max(*v))
                }),
        };
        points.retain(|(v, _)| *v >= low && *v <= high);

        let mut colours: Vec<String> = Vec::new();
        for (_, colour) in &points {
            if let Some(c) = colour
                && !colours.contains(c)
            {
                colours.push(c.clone());
            }
        }
        let (start, width, bins) = if points.is_empty() || !low.is_finite() {
            (0.0, 0.0, 0)
        } else {
            match self.width {
                Some(width) => {
                    let start = (low / width).floor() * width;
                    let bins = ((high - start) / width).floor() + 1.0;
                    if !(bins.is_finite() && bins <= MAX_BINS as f64) {
                        return Err(OperatorError::Data(format!(
                            "Histogram bin width {width} gives too many bins"
                        )));
                    }
                    (start, width, bins as usize)
                }
                None => {
                    let width = (low + (high - low) / self.bins as f64) - low;
                    (low, width, self.bins)
                }
            }
        };
        let slots = colours.len().max(1);
        let mut counts = vec![0.0; bins * slots];
        for (value, colour) in &points {
            let bin = if width > 0.0 {
                (((value - start) / width).floor().max(0.0) as usize).min(bins.saturating_sub(1))
            } else {
                0
            };
            let slot = colour
                .as_ref()
                .and_then(|c| colours.iter().position(|k| k == c))
                .unwrap_or(0);
            if let Some(cell) = counts.get_mut(bin * slots + slot) {
                *cell += 1.0;
            }
        }

        let value_column = if self.normalize && overlay.is_some() {
            "__normalized__"
        } else {
            "__count__"
        };
        let mut header = vec![self.field.clone(), value_column.to_string()];
        if let Some(name) = &self.overlay {
            header.push(name.clone());
        }
        let mut rows = Vec::with_capacity(counts.len());
        // Edges accumulate (`edge += width`); a bin's centre is the midpoint of its edges, and
        // with a number of bins the last edge is the maximum itself.
        let mut edge = start;
        for bin in 0..bins {
            let mut next = edge + width;
            if self.width.is_none() && bin + 1 == bins {
                next = high;
            }
            let centre = java_double((edge + next) / 2.0);
            edge = next;
            let cells = &counts[bin * slots..(bin + 1) * slots];
            let total: f64 = cells.iter().sum();
            if overlay.is_none() {
                rows.push(vec![centre, java_double(cells[0])]);
                continue;
            }
            for (colour, count) in colours.iter().zip(cells) {
                // An empty bin has no percentage: an empty cell (`team/stream_1_M`).
                let value = match (self.normalize, total > 0.0) {
                    (true, true) => java_double(count / total * 100.0),
                    (true, false) => String::new(),
                    (false, _) => java_double(*count),
                };
                rows.push(vec![centre.clone(), value, colour.clone()]);
            }
        }
        Ok(NodeOutput::Document(Arc::new(Document {
            title: format!("Histogram of {}", self.field),
            tables: vec![DocumentTable {
                title: None,
                header,
                rows,
            }],
        })))
    }
}
