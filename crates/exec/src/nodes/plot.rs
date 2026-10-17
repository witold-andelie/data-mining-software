//! Plot (`ScatterPlotNode`, Graphs) and Multiplot (`MultiPlotNode`): the points of a scatter
//! plot, or of several y fields against one x field. The output is the data behind the graph, as
//! Modeler's VIZML export holds it (`nodes::graph_data`; `oracle/graphs/plot_*`, `multi_*`,
//! `team/linear_regression` plots).
//!
//! Plot settings: `graphXField`, `graphYField` and one overlay of `graphColorField`,
//! `graphSizeField`, `graphShapeField` or `graphTransparencyField`. Multiplot settings:
//! `graphXField`, `graphYMultiFields`, `graphNormalizeData`. Both: `graphLargeDatasetReadThreshold`
//! (default 2000), `graphPanelField`. Rejected: 3-D (`graphZField`), animation, several overlays,
//! and inputs
//! larger than the threshold (Modeler then bins or samples them; no probe covers it).
//!
//! Rules:
//! - One row per record, in input order: x, y (or the y fields), the overlay, then the panel
//!   field (`plot_na_k_color`: `Na;K;Drug`; `plot_age_na_size`: `Age;Na;K`;
//!   `team/linear_regression`: `increase;$L-increase;Class` with Class as panel).
//! - Numbers print as Java doubles whatever their storage (`plot_age_na`: Age `23.0`); a null
//!   or missing value is an empty cell and the record stays (`team/linear_regression` row 161:
//!   `1.42312147518035;;Meat`).
//! - Multiplot with `graphNormalizeData` rescales each y field to `(v - min) / (max - min)`
//!   over its present values; x is not rescaled (`multi_age_na_k_norm`: Na 0.792535 is
//!   0.738508715870943).
//!
//! GAP: a constant y field under normalisation (no probe) prints 0.

use super::graph_data::java_double;
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Table, Value};
use std::sync::Arc;

struct Plot {
    title: &'static str,
    x: String,
    ys: Vec<String>,
    overlay: Option<String>,
    /// `graphPanelField`: one more column after the overlay.
    panel: Option<String>,
    normalize: bool,
    threshold: usize,
}

fn field(s: Option<&dms_strfmt::fastinfoset::Element>, name: &str) -> Option<String> {
    settings::text(s, name).filter(|f| !f.trim().is_empty())
}

fn common_checks(s: Option<&dms_strfmt::fastinfoset::Element>) -> Result<usize, OperatorError> {
    for unsupported in ["graphAnimationField", "graphZField"] {
        if field(s, unsupported).is_some() {
            return Err(OperatorError::Settings(format!(
                "graph setting {unsupported} is not supported yet"
            )));
        }
    }
    Ok(settings::integer(s, "graphLargeDatasetReadThreshold")?
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(2000))
}

/// The Plot node (`ScatterPlotNode`).
pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let threshold = common_checks(s)?;
    let x = field(s, "graphXField")
        .ok_or_else(|| OperatorError::Settings("Plot: no x field (graphXField)".into()))?;
    let y = field(s, "graphYField")
        .ok_or_else(|| OperatorError::Settings("Plot: no y field (graphYField)".into()))?;
    let overlays: Vec<String> = [
        "graphColorField",
        "graphSizeField",
        "graphShapeField",
        "graphTransparencyField",
    ]
    .iter()
    .filter_map(|name| field(s, name))
    .collect();
    if overlays.len() > 1 {
        return Err(OperatorError::Settings(
            "Plot with several overlay fields is not supported yet".into(),
        ));
    }
    Ok(Box::new(Plot {
        title: "Plot",
        x,
        ys: vec![y],
        overlay: overlays.into_iter().next(),
        panel: field(s, "graphPanelField"),
        normalize: false,
        threshold,
    }))
}

/// The Multiplot node (`MultiPlotNode`).
pub fn build_multiplot(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let threshold = common_checks(s)?;
    let x = field(s, "graphXField")
        .ok_or_else(|| OperatorError::Settings("Multiplot: no x field (graphXField)".into()))?;
    let ys: Vec<String> = settings::children(s, "graphYMultiFields", "field")
        .into_iter()
        .map(|f| f.text().trim().to_string())
        .filter(|f| !f.is_empty())
        .collect();
    if ys.is_empty() {
        return Err(OperatorError::Settings(
            "Multiplot: no y fields (graphYMultiFields)".into(),
        ));
    }
    if settings::flag(s, "graphUseOverlayExpression")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "Multiplot overlay expressions are not supported yet".into(),
        ));
    }
    Ok(Box::new(Plot {
        title: "Multiplot",
        x,
        ys,
        overlay: None,
        panel: field(s, "graphPanelField"),
        normalize: settings::flag(s, "graphNormalizeData")?.unwrap_or(false),
        threshold,
    }))
}

impl Operator for Plot {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(format!(
            "{} is a graph node and produces a Document, not a Table",
            self.title
        )))
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        _model: Option<Arc<dms_pmml::Pmml>>,
        _context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs(format!("{} needs one input", self.title)))?;
        if input.row_count() > self.threshold {
            return Err(OperatorError::Settings(format!(
                "{} of more than {} records (Modeler bins or samples them) is not supported yet",
                self.title, self.threshold
            )));
        }
        let mut header = vec![self.x.clone()];
        header.extend(self.ys.iter().cloned());
        header.extend(self.overlay.iter().cloned());
        header.extend(self.panel.iter().cloned());
        let positions = header
            .iter()
            .map(|name| {
                input
                    .model()
                    .position(name)
                    .ok_or_else(|| OperatorError::Data(format!("no field {name}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let fields = input.model().fields();
        let present = |row: &[Value], at: usize| -> Option<Value> {
            let value = row.get(at)?;
            let field = fields.get(at)?;
            (!value.is_null() && !field.is_missing(value)).then(|| value.clone())
        };
        // Per y column (1..=ys), the (min, max) of its present numbers when normalising.
        let ranges: Vec<Option<(f64, f64)>> = positions
            .iter()
            .enumerate()
            .map(|(column, &at)| {
                if !self.normalize || column == 0 || column > self.ys.len() {
                    return None;
                }
                let values = input
                    .rows()
                    .iter()
                    .filter_map(|row| present(row, at).and_then(|v| v.as_f64()));
                let (low, high) = values.fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), v| {
                    (l.min(v), h.max(v))
                });
                low.is_finite().then_some((low, high))
            })
            .collect();
        let rows = input
            .rows()
            .iter()
            .map(|row| {
                positions
                    .iter()
                    .zip(&ranges)
                    .map(|(&at, range)| match present(row, at) {
                        None => String::new(),
                        Some(value) => match (value.as_f64(), range) {
                            (Some(v), Some((low, high))) => {
                                let span = high - low;
                                java_double(if span > 0.0 { (v - low) / span } else { 0.0 })
                            }
                            (Some(v), None)
                                if matches!(value, Value::Integer(_) | Value::Real(_)) =>
                            {
                                java_double(v)
                            }
                            _ => value.to_string(),
                        },
                    })
                    .collect()
            })
            .collect();
        Ok(NodeOutput::Document(Arc::new(Document {
            title: format!("{} of {}", self.title, self.x),
            tables: vec![DocumentTable {
                title: None,
                header,
                rows,
            }],
        })))
    }
}
