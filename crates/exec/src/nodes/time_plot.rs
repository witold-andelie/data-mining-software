//! Time Plot (Graphs): fields plotted against the record index. The output is the data behind
//! the graph, as Modeler's VIZML export holds it (`nodes::graph_data`;
//! `oracle/graphs/timeplot_na_k`).
//!
//! Settings: `timePlotSeriesFields`, `graphNormalizeData` (default true), `timePlotSeriesToPlot`
//! (`Series` only), `timePlotUseRecordsLimit` with `timePlotRecordsLimit` (default on, 2000).
//! `timePlotPanel`, lines, points and smoothers only change the drawing. A custom x field
//! (`timePlotUseCustomXField`), overlays and inputs beyond the records limit are rejected.
//!
//! Rules: one row per record: `@INDEX` (1, 2, ... as doubles) and each series field, rescaled
//! to `(v - min) / (max - min)` over its present values when normalising (`timeplot_na_k`: Na
//! of the first record 0.738508715870943); nulls are empty cells.
//!
//! GAP: which records Modeler keeps beyond the limit (no probe).

use super::graph_data::java_double;
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Table, Value};
use std::sync::Arc;

struct TimePlot {
    series: Vec<String>,
    normalize: bool,
    limit: Option<usize>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    if settings::flag(s, "timePlotUseCustomXField")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "Time Plot with a custom x field is not supported yet".into(),
        ));
    }
    match settings::text(s, "timePlotSeriesToPlot")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("Series") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Time Plot of {other:?} is not supported yet"
            )));
        }
    }
    for unsupported in ["graphColorField", "graphAnimationField"] {
        if settings::text(s, unsupported).is_some_and(|f| !f.trim().is_empty()) {
            return Err(OperatorError::Settings(format!(
                "Time Plot setting {unsupported} is not supported yet"
            )));
        }
    }
    let series: Vec<String> = settings::children(s, "timePlotSeriesFields", "field")
        .into_iter()
        .map(|f| f.text().trim().to_string())
        .filter(|f| !f.is_empty())
        .collect();
    if series.is_empty() {
        return Err(OperatorError::Settings(
            "Time Plot: no series fields (timePlotSeriesFields)".into(),
        ));
    }
    let limit = if settings::flag(s, "timePlotUseRecordsLimit")?.unwrap_or(true) {
        Some(
            settings::integer(s, "timePlotRecordsLimit")?
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(2000),
        )
    } else {
        None
    };
    Ok(Box::new(TimePlot {
        series,
        normalize: settings::flag(s, "graphNormalizeData")?.unwrap_or(true),
        limit,
    }))
}

impl Operator for TimePlot {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Time Plot is a graph node and produces a Document, not a Table".into(),
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
            .ok_or_else(|| OperatorError::Inputs("Time Plot needs one input".into()))?;
        if let Some(limit) = self.limit
            && input.row_count() > limit
        {
            return Err(OperatorError::Settings(format!(
                "Time Plot of more than {limit} records is not supported yet"
            )));
        }
        let positions = self
            .series
            .iter()
            .map(|name| {
                input
                    .model()
                    .position(name)
                    .ok_or_else(|| OperatorError::Data(format!("no field {name}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let fields = input.model().fields();
        let number = |row: &[Value], at: usize| -> Option<f64> {
            let value = row.get(at)?;
            let field = fields.get(at)?;
            (!value.is_null() && !field.is_missing(value))
                .then(|| value.as_f64())
                .flatten()
        };
        let ranges: Vec<(f64, f64)> = positions
            .iter()
            .map(|&at| {
                input
                    .rows()
                    .iter()
                    .filter_map(|row| number(row, at))
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), v| {
                        (l.min(v), h.max(v))
                    })
            })
            .collect();
        let rows = input
            .rows()
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let mut cells = vec![java_double((index + 1) as f64)];
                for (&at, &(low, high)) in positions.iter().zip(&ranges) {
                    cells.push(match number(row, at) {
                        None => String::new(),
                        Some(v) if self.normalize => {
                            let span = high - low;
                            java_double(if span > 0.0 { (v - low) / span } else { 0.0 })
                        }
                        Some(v) => java_double(v),
                    });
                }
                cells
            })
            .collect();
        let mut header = vec!["@INDEX".to_string()];
        header.extend(self.series.iter().cloned());
        Ok(NodeOutput::Document(Arc::new(Document {
            title: "Time Plot".into(),
            tables: vec![DocumentTable {
                title: None,
                header,
                rows,
            }],
        })))
    }
}
