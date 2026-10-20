//! Evaluation (Graphs): gains, lift or response of model predictions by n-tile. The output is
//! the data behind the chart, as Modeler's VIZML export holds it (`nodes::graph_data`;
//! `oracle/graphs/eval_*`).
//!
//! Settings: `chartType` (`gains`, `lift`, `response`, `profit`, `roi`), `cumulativePlot` (default
//! true), `plotNTiles` (`percentiles` default, `quartiles`, `quintiles`, `deciles`, `vingtiles`,
//! `1000-tiles`), `fieldDetectionMethod` (`name`), `fixedCost` / `fixedRevenue` (profit and ROI).
//! Rejected: user-defined hits or scores, business rules, metadata detection, a partition field
//! in the data (`splitByPartition` draws one chart per partition), and for profit and ROI cost,
//! revenue or weight fields, a weight other than 1 and non-cumulative plots (no oracle).
//! Baselines and best lines only draw.
//!
//! Rules (`eval_gains`, `eval_lift`, `eval_gains_noncumulative`: all 300 values to the last bit):
//! - The target is the Type's target, a flag; a hit is its true value.
//! - Models are found by name: a prediction `$<P>-<target>` with its confidence
//!   `$<P>C-<target>`, one column per model in input order, named after the prediction.
//! - A record's score is its confidence when it predicts a hit, else 1 - confidence; records are
//!   ranked by descending score (input order on ties).
//! - The record at rank r (1-based) of n falls in n-tile `ceil(r / n * T)` with T n-tiles,
//!   computed in floating point (`eval_gains` percentile 7: rank 14 of 200 gives
//!   7.000000000000001, so percentile 8).
//! - Gains: hits up to (cumulative) or in the n-tile, as a percentage of all hits. Response: hits
//!   as a percentage of the records there. Lift: `(hits / records) / (all hits / n)` there.
//!   Profit: `hits * fixedRevenue - records * fixedCost`; ROI: that profit as a percentage of
//!   `records * fixedCost` (`oracle/outputs_next/evaluation_profit`, `evaluation_roi`: decile 1
//!   has 4 records and 2 hits, profit 6, ROI 150).
//! - Rows: the n-tile number (1.0, 2.0, ...) and each model's value.
//!
//! GAP: set targets and n-tiles without records (no probe) print 0.

use super::graph_data::java_double;
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Role, Table, Value, ValueSpec};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chart {
    Gains,
    Lift,
    Response,
    Profit,
    Roi,
}

struct Evaluation {
    chart: Chart,
    /// `fixedCost` and `fixedRevenue` for profit and ROI.
    cost: f64,
    revenue: f64,
    cumulative: bool,
    tiles: usize,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let unsupported =
        |what: &str| OperatorError::Settings(format!("Evaluation: {what} is not supported yet"));
    let chart = match settings::text(s, "chartType").as_deref().map(str::trim) {
        None | Some("") | Some("gains") => Chart::Gains,
        Some("lift") => Chart::Lift,
        Some("response") => Chart::Response,
        Some("profit") => Chart::Profit,
        Some("roi") => Chart::Roi,
        Some(other) => return Err(unsupported(&format!("chart type {other}"))),
    };
    let money = matches!(chart, Chart::Profit | Chart::Roi);
    if money {
        for key in ["useFixedCosts", "useFixedRevenue", "useFixedWeight"] {
            if !settings::flag(s, key)?.unwrap_or(true) {
                return Err(unsupported(&format!("{key} false")));
            }
        }
        if settings::real(s, "fixedWeight")?.unwrap_or(1.0) != 1.0 {
            return Err(unsupported("a fixedWeight other than 1"));
        }
        if !settings::flag(s, "cumulativePlot")?.unwrap_or(true) {
            return Err(unsupported("a non-cumulative profit or ROI plot"));
        }
    }
    for flag in [
        "userDefinedHit",
        "userDefinedScore",
        "includeBusinessRule",
        "useProfitCriteria",
    ] {
        if settings::flag(s, flag)?.unwrap_or(false) {
            return Err(unsupported(flag));
        }
    }
    match settings::text(s, "fieldDetectionMethod")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("name") => {}
        Some(other) => return Err(unsupported(&format!("field detection {other}"))),
    }
    let tiles = match settings::text(s, "plotNTiles").as_deref().map(str::trim) {
        None | Some("") | Some("percentiles") => 100,
        Some("quartiles") => 4,
        Some("quintiles") => 5,
        Some("deciles") => 10,
        Some("vingtiles") => 20,
        Some("1000-tiles") => 1000,
        Some(other) => return Err(unsupported(&format!("n-tiles {other}"))),
    };
    Ok(Box::new(Evaluation {
        chart,
        cost: settings::real(s, "fixedCost")?.unwrap_or(0.0),
        revenue: settings::real(s, "fixedRevenue")?.unwrap_or(0.0),
        cumulative: settings::flag(s, "cumulativePlot")?.unwrap_or(true),
        tiles,
    }))
}

impl Operator for Evaluation {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Evaluation is a graph node and produces a Document, not a Table".into(),
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
            .ok_or_else(|| OperatorError::Inputs("Evaluation needs one input".into()))?;
        let model = input.model();
        let fields = model.fields();
        if fields.iter().any(|f| f.role == Role::Partition) {
            return Err(OperatorError::Settings(
                "Evaluation by partition is not supported yet".into(),
            ));
        }
        let target_at = fields
            .iter()
            .position(|f| f.role == Role::Target)
            .ok_or_else(|| OperatorError::Data("Evaluation needs a target field".into()))?;
        let target = &fields[target_at];
        let ValueSpec::Flag { true_value, .. } = &target.values else {
            return Err(OperatorError::Settings(
                "Evaluation of a target that is not a flag is not supported yet".into(),
            ));
        };
        let hit = true_value.to_string();
        // Models: `$<P>-<target>` with `$<P>C-<target>`.
        let suffix = format!("-{}", target.name);
        let models: Vec<(String, usize, usize)> = fields
            .iter()
            .enumerate()
            .filter_map(|(at, f)| {
                let prefix = f.name.strip_suffix(&suffix)?.strip_prefix('$')?;
                if prefix.is_empty() || !prefix.chars().all(|c| c.is_ascii_uppercase()) {
                    return None;
                }
                let confidence = model.position(&format!("${prefix}C{suffix}"))?;
                Some((f.name.clone(), at, confidence))
            })
            .collect();
        if models.is_empty() {
            return Err(OperatorError::Data(
                "Could not find any appropriate predictions to analyse".into(),
            ));
        }

        let tiles = self.tiles;
        let mut columns: Vec<Vec<String>> = Vec::new();
        for (_, predicted, confidence) in &models {
            // (score, is hit) per record with a target, prediction and confidence.
            let mut scored: Vec<(f64, bool)> = input
                .rows()
                .iter()
                .filter_map(|row| {
                    let actual = row.get(target_at).filter(|v| !v.is_null())?;
                    let prediction = row.get(*predicted).filter(|v| !v.is_null())?;
                    let conf = row.get(*confidence).and_then(Value::as_f64)?;
                    let predicts_hit = prediction.to_string() == hit;
                    let score = if predicts_hit { conf } else { 1.0 - conf };
                    Some((score, actual.to_string() == hit))
                })
                .collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0));
            let n = scored.len();
            let all_hits = scored.iter().filter(|(_, h)| *h).count() as f64;
            let mut hits = vec![0.0; tiles];
            let mut counts = vec![0.0; tiles];
            for (rank, (_, is_hit)) in scored.iter().enumerate() {
                let tile = ((rank + 1) as f64 / n as f64 * tiles as f64).ceil() as usize;
                let tile = tile.clamp(1, tiles) - 1;
                counts[tile] += 1.0;
                if *is_hit {
                    hits[tile] += 1.0;
                }
            }
            let (mut cum_hits, mut cum_counts) = (0.0, 0.0);
            let values = (0..tiles)
                .map(|t| {
                    if self.cumulative {
                        cum_hits += hits[t];
                        cum_counts += counts[t];
                    } else {
                        cum_hits = hits[t];
                        cum_counts = counts[t];
                    }
                    let value = match self.chart {
                        Chart::Gains if all_hits > 0.0 => cum_hits / all_hits * 100.0,
                        Chart::Response if cum_counts > 0.0 => cum_hits / cum_counts * 100.0,
                        Chart::Lift if cum_counts > 0.0 && all_hits > 0.0 => {
                            (cum_hits / cum_counts) / (all_hits / n as f64)
                        }
                        Chart::Profit => cum_hits * self.revenue - cum_counts * self.cost,
                        Chart::Roi if cum_counts * self.cost != 0.0 => {
                            (cum_hits * self.revenue - cum_counts * self.cost)
                                / (cum_counts * self.cost)
                                * 100.0
                        }
                        _ => 0.0,
                    };
                    java_double(value)
                })
                .collect();
            columns.push(values);
        }
        let rows = (0..tiles)
            .map(|t| {
                std::iter::once(java_double((t + 1) as f64))
                    .chain(columns.iter().map(|c| c[t].clone()))
                    .collect()
            })
            .collect();
        let mut header = vec!["nTileColumnName".to_string()];
        header.extend(models.iter().map(|(name, _, _)| name.clone()));
        Ok(NodeOutput::Document(Arc::new(Document {
            title: "Evaluation".into(),
            tables: vec![DocumentTable {
                title: None,
                header,
                rows,
            }],
        })))
    }
}
