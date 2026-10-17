//! Collection (Graphs): a field's values summarised over the bins of another, continuous field.
//! The output is the data behind the graph, as Modeler's VIZML export holds it
//! (`nodes::graph_data`; `oracle/graphs/collect_*`).
//!
//! Settings: `graphXField` (the "over" field, binned exactly as by the Histogram node: the same
//! `graphNumBins`, `graphBinByWidth`, range settings and bin centres), `graphYField` (the
//! collected field), `graphOperationType` (`SUM` default, `AVE`, `MIN`, `MAX`, `SDEV`). Overlay,
//! panel and animation fields are rejected.
//!
//! Rules: one row per bin, its centre and the operation over the collected values of the
//! records in it, summed in input order (`collect_na_over_age`: 4.782584999999999 for the first
//! Age bin; `collect_na_over_age_mean`: 0.6832264285714285). A record needs both values.
//!
//! GAP: an empty bin (no probe has one) prints an empty cell; `SDEV` is the sample standard
//! deviation (no probe).

use super::graph_data::{BinSettings, java_double};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Table, Value};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    Sum,
    Mean,
    Min,
    Max,
    SDev,
}

struct Collection {
    over: String,
    collect: String,
    operation: Operation,
    binning: BinSettings,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let field = |name: &str| settings::text(s, name).filter(|f| !f.trim().is_empty());
    for unsupported in ["graphColorField", "graphPanelField", "graphAnimationField"] {
        if field(unsupported).is_some() {
            return Err(OperatorError::Settings(format!(
                "Collection setting {unsupported} is not supported yet"
            )));
        }
    }
    let operation = match settings::text(s, "graphOperationType")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("SUM") => Operation::Sum,
        Some("AVE") => Operation::Mean,
        Some("MIN") => Operation::Min,
        Some("MAX") => Operation::Max,
        Some("SDEV") => Operation::SDev,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Collection operation {other:?} is not supported"
            )));
        }
    };
    Ok(Box::new(Collection {
        over: field("graphXField")
            .ok_or_else(|| OperatorError::Settings("Collection: no over field".into()))?,
        collect: field("graphYField")
            .ok_or_else(|| OperatorError::Settings("Collection: no collect field".into()))?,
        operation,
        binning: BinSettings::read(s, "Collection")?,
    }))
}

impl Operator for Collection {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Collection is a graph node and produces a Document, not a Table".into(),
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
            .ok_or_else(|| OperatorError::Inputs("Collection needs one input".into()))?;
        let position = |name: &str| {
            input
                .model()
                .position(name)
                .ok_or_else(|| OperatorError::Data(format!("no field {name}")))
        };
        let (x, y) = (position(&self.over)?, position(&self.collect)?);
        let fields = input.model().fields();
        let number = |row: &[Value], at: usize| -> Option<f64> {
            let value = row.get(at)?;
            let field = fields.get(at)?;
            (!value.is_null() && !field.is_missing(value))
                .then(|| value.as_f64())
                .flatten()
        };
        let mut points: Vec<(f64, f64)> = input
            .rows()
            .iter()
            .filter_map(|row| Some((number(row, x)?, number(row, y)?)))
            .collect();
        let (low, high) = match self.binning.range {
            Some(range) => range,
            None => points
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), (v, _)| {
                    (l.min(*v), h.max(*v))
                }),
        };
        points.retain(|(v, _)| *v >= low && *v <= high);
        let mut rows = Vec::new();
        if !points.is_empty() && low.is_finite() {
            let bins = self.binning.bins(low, high)?;
            let mut groups: Vec<Vec<f64>> = vec![Vec::new(); bins.count];
            for (over, collected) in &points {
                if let Some(group) = groups.get_mut(bins.index(*over)) {
                    group.push(*collected);
                }
            }
            for (centre, values) in bins.centres().into_iter().zip(&groups) {
                rows.push(vec![java_double(centre), summarise(self.operation, values)]);
            }
        }
        Ok(NodeOutput::Document(Arc::new(Document {
            title: format!("Collection of {} over {}", self.collect, self.over),
            tables: vec![DocumentTable {
                title: None,
                header: vec![self.over.clone(), self.collect.clone()],
                rows,
            }],
        })))
    }
}

fn summarise(operation: Operation, values: &[f64]) -> String {
    if values.is_empty() {
        return String::new();
    }
    let n = values.len() as f64;
    let sum: f64 = values.iter().sum();
    let value = match operation {
        Operation::Sum => sum,
        Operation::Mean => sum / n,
        Operation::Min => values.iter().copied().fold(f64::INFINITY, f64::min),
        Operation::Max => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        Operation::SDev => {
            if values.len() < 2 {
                return String::new();
            }
            let mean = sum / n;
            (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
        }
    };
    java_double(value)
}
