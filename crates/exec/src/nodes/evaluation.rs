//! Evaluation (Graphs): gains, lift or response of model predictions by n-tile. The output is
//! the data behind the chart, as Modeler's VIZML export holds it (`nodes::graph_data`;
//! `oracle/graphs/eval_*`).
//!
//! Settings: `chartType` (`gains`, `lift`, `response`, `profit`, `roi`), `cumulativePlot` (default
//! true), `plotNTiles` (`percentiles` default, `quartiles`, `quintiles`, `deciles`, `vingtiles`,
//! `1000-tiles`), `fieldDetectionMethod` (`name`), `fixedCost` / `fixedRevenue` / `fixedWeight`
//! with `useFixedCosts` / `useFixedRevenue` / `useFixedWeight` (file names; the scripting
//! properties are `use_fixed_cost` / `cost_value` / `cost_field` and the same for revenue and
//! weight) and `variableCost` / `variableRevenue` / `variableWeight` (profit and ROI,
//! `oracle/outputs_more/evaluation_*`).
//! Rejected: user-defined hits or scores, business rules, metadata detection, a partition field
//! in the data (`splitByPartition` draws one chart per partition), a fixed weight other than 1
//! and non-cumulative profit or ROI plots (no oracle). Baselines and best lines only draw.
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
//!   Profit: cumulative `sum(hit ? revenue_i * weight_i : 0) - sum(cost_i)` where `revenue_i` is
//!   `fixedRevenue` or the `variableRevenue` field, `cost_i` is `fixedCost` or the `variableCost`
//!   field and `weight_i` is `fixedWeight` (1) or the `variableWeight` field; the weight does not
//!   multiply the cost (`outputs_more/evaluation_profit_weight` decile 1: `4.5 * 5 - 6 = 16.5`).
//!   ROI: that profit as a percentage of cumulative `sum(cost_i)`
//!   (`oracle/outputs_next/evaluation_profit`, `evaluation_roi`: decile 1
//!   has 4 records and 2 hits, profit 6, ROI 150; `outputs_more/evaluation_*_cost` decile 1:
//!   profit `-2`, cost sum `17`, ROI `-11.76470588235294`).
//! - Rows: the n-tile number (1.0, 2.0, ...) and each model's value.
//!
//! GAP: set targets and n-tiles without records (no probe) print 0. A missing or non-finite
//! cost, revenue or weight value has no oracle and is a `GAP:` error. The `.cou` output object
//! file and file-mode HTML with graph images are not written (no oracle for the images).

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
    /// `fixedCost` and `fixedRevenue` for profit and ROI, or a field name when the fixed flag
    /// is off (`variableCost` / `variableRevenue`).
    cost: f64,
    revenue: f64,
    cost_field: Option<String>,
    revenue_field: Option<String>,
    /// `variableWeight` when `useFixedWeight` is false; otherwise the weight is 1.
    weight_field: Option<String>,
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
    let (cost, cost_field, revenue, revenue_field, weight_field) = if money {
        let use_fixed_cost = settings::flag(s, "useFixedCosts")?.unwrap_or(true);
        let use_fixed_revenue = settings::flag(s, "useFixedRevenue")?.unwrap_or(true);
        let use_fixed_weight = settings::flag(s, "useFixedWeight")?.unwrap_or(true);
        if settings::real(s, "fixedWeight")?.unwrap_or(1.0) != 1.0 {
            return Err(unsupported("a fixedWeight other than 1"));
        }
        if !settings::flag(s, "cumulativePlot")?.unwrap_or(true) {
            return Err(unsupported("a non-cumulative profit or ROI plot"));
        }
        let cost_field = if use_fixed_cost {
            None
        } else {
            let name = settings::text(s, "variableCost")
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty());
            match name {
                Some(name) => Some(name),
                None => return Err(unsupported("a profit or ROI cost field that is empty")),
            }
        };
        let revenue_field = if use_fixed_revenue {
            None
        } else {
            let name = settings::text(s, "variableRevenue")
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty());
            match name {
                Some(name) => Some(name),
                None => return Err(unsupported("a profit or ROI revenue field that is empty")),
            }
        };
        let weight_field = if use_fixed_weight {
            None
        } else {
            let name = settings::text(s, "variableWeight")
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty());
            match name {
                Some(name) => Some(name),
                None => return Err(unsupported("a profit or ROI weight field that is empty")),
            }
        };
        (
            settings::real(s, "fixedCost")?.unwrap_or(0.0),
            cost_field,
            settings::real(s, "fixedRevenue")?.unwrap_or(0.0),
            revenue_field,
            weight_field,
        )
    } else {
        (
            settings::real(s, "fixedCost")?.unwrap_or(0.0),
            None,
            settings::real(s, "fixedRevenue")?.unwrap_or(0.0),
            None,
            None,
        )
    };
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
        cost,
        revenue,
        cost_field,
        revenue_field,
        weight_field,
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
        // Money fields, if any, resolved once so a missing column fails before scoring.
        let cost_pos = self
            .cost_field
            .as_deref()
            .map(|name| {
                model.position(name).ok_or_else(|| {
                    OperatorError::Data(format!("Evaluation needs cost field {name:?}"))
                })
            })
            .transpose()?;
        let revenue_pos = self
            .revenue_field
            .as_deref()
            .map(|name| {
                model.position(name).ok_or_else(|| {
                    OperatorError::Data(format!("Evaluation needs revenue field {name:?}"))
                })
            })
            .transpose()?;
        let weight_pos = self
            .weight_field
            .as_deref()
            .map(|name| {
                model.position(name).ok_or_else(|| {
                    OperatorError::Data(format!("Evaluation needs weight field {name:?}"))
                })
            })
            .transpose()?;
        let money = matches!(self.chart, Chart::Profit | Chart::Roi);
        let mut columns: Vec<Vec<String>> = Vec::new();
        for (_, predicted, confidence) in &models {
            if money {
                // (score, is hit, cost, revenue-if-hit, weight) per record.
                let mut scored: Vec<(f64, bool, f64, f64, f64)> = Vec::new();
                for row in input.rows() {
                    let actual = row.get(target_at).filter(|v| !v.is_null());
                    let prediction = row.get(*predicted).filter(|v| !v.is_null());
                    let conf = row.get(*confidence).and_then(Value::as_f64);
                    let (Some(actual), Some(prediction), Some(conf)) = (actual, prediction, conf)
                    else {
                        continue;
                    };
                    let cost_i = match cost_pos {
                        Some(pos) => {
                            let field = &fields[pos];
                            match row.get(pos) {
                                Some(v) if !v.is_null() && !field.is_missing(v) => {
                                    match v.as_f64() {
                                        Some(x) if x.is_finite() => x,
                                        _ => {
                                            return Err(OperatorError::Settings(
                                                "GAP: Evaluation: missing cost, revenue or weight has no oracle"
                                                    .into(),
                                            ));
                                        }
                                    }
                                }
                                _ => {
                                    return Err(OperatorError::Settings(
                                        "GAP: Evaluation: missing cost, revenue or weight has no oracle"
                                            .into(),
                                    ));
                                }
                            }
                        }
                        None => self.cost,
                    };
                    let revenue_i = match revenue_pos {
                        Some(pos) => {
                            let field = &fields[pos];
                            match row.get(pos) {
                                Some(v) if !v.is_null() && !field.is_missing(v) => {
                                    match v.as_f64() {
                                        Some(x) if x.is_finite() => x,
                                        _ => {
                                            return Err(OperatorError::Settings(
                                                "GAP: Evaluation: missing cost, revenue or weight has no oracle"
                                                    .into(),
                                            ));
                                        }
                                    }
                                }
                                _ => {
                                    return Err(OperatorError::Settings(
                                        "GAP: Evaluation: missing cost, revenue or weight has no oracle"
                                            .into(),
                                    ));
                                }
                            }
                        }
                        None => self.revenue,
                    };
                    let weight_i = match weight_pos {
                        Some(pos) => {
                            let field = &fields[pos];
                            match row.get(pos) {
                                Some(v) if !v.is_null() && !field.is_missing(v) => {
                                    match v.as_f64() {
                                        Some(x) if x.is_finite() => x,
                                        _ => {
                                            return Err(OperatorError::Settings(
                                                "GAP: Evaluation: missing cost, revenue or weight has no oracle"
                                                    .into(),
                                            ));
                                        }
                                    }
                                }
                                _ => {
                                    return Err(OperatorError::Settings(
                                        "GAP: Evaluation: missing cost, revenue or weight has no oracle"
                                            .into(),
                                    ));
                                }
                            }
                        }
                        None => 1.0,
                    };
                    let predicts_hit = prediction.to_string() == hit;
                    let score = if predicts_hit { conf } else { 1.0 - conf };
                    scored.push((
                        score,
                        actual.to_string() == hit,
                        cost_i,
                        revenue_i,
                        weight_i,
                    ));
                }
                scored.sort_by(|a, b| b.0.total_cmp(&a.0));
                let n = scored.len();
                let mut profit_tiles = vec![0.0; tiles];
                let mut cost_tiles = vec![0.0; tiles];
                for (rank, (_, is_hit, cost_i, revenue_i, weight_i)) in scored.iter().enumerate() {
                    if n == 0 {
                        continue;
                    }
                    let tile = ((rank + 1) as f64 / n as f64 * tiles as f64).ceil() as usize;
                    let tile = tile.clamp(1, tiles) - 1;
                    let profit_i = if *is_hit {
                        revenue_i * weight_i - cost_i
                    } else {
                        -cost_i
                    };
                    profit_tiles[tile] += profit_i;
                    cost_tiles[tile] += *cost_i;
                }
                let (mut cum_profit, mut cum_cost) = (0.0, 0.0);
                let values = (0..tiles)
                    .map(|t| {
                        cum_profit += profit_tiles[t];
                        cum_cost += cost_tiles[t];
                        let value = match self.chart {
                            Chart::Profit => cum_profit,
                            Chart::Roi if cum_cost != 0.0 => cum_profit / cum_cost * 100.0,
                            _ => 0.0,
                        };
                        java_double(value)
                    })
                    .collect();
                columns.push(values);
                continue;
            }
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
