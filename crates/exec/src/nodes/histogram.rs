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

use super::graph_data::{BinSettings, category_text, java_double};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Table, Value};
use std::sync::Arc;

struct Histogram {
    field: String,
    overlay: Option<String>,
    normalize: bool,
    binning: BinSettings,
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
    let binning = BinSettings::read(s, "Histogram")?;
    Ok(Box::new(Histogram {
        field,
        overlay: settings::text(s, "graphColorField").filter(|f| !f.trim().is_empty()),
        normalize: settings::flag(s, "graphNormalizeData")?.unwrap_or(false),
        binning,
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
        let (low, high) = match self.binning.range {
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
        let bins = if points.is_empty() || !low.is_finite() {
            None
        } else {
            Some(self.binning.bins(low, high)?)
        };
        let count = bins.map_or(0, |b| b.count);
        let slots = colours.len().max(1);
        let mut counts = vec![0.0; count * slots];
        for (value, colour) in &points {
            let bin = bins.map_or(0, |b| b.index(*value));
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
        let centres = bins.map(|b| b.centres()).unwrap_or_default();
        for (bin, centre) in centres.into_iter().enumerate() {
            let centre = java_double(centre);
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
