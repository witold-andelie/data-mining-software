//! Distribution (Graphs): the categories of a field and their counts, optionally broken down by
//! an overlay (colour) field. The output is the data behind the graph, as Modeler's VIZML export
//! holds it (`nodes::graph_data`; `oracle/graphs/dist_*`, `xtask/oracle/probes/graph_streams.py`).
//!
//! Settings: `graphXField`, `graphColorField`, `graphNormalizeData`; `distributionSortType`
//! (`alphabetic`, `byCount`) and `distributionProportionalScale` only change the drawing.
//! `distributionSelectionMode` other than `selectedFields` (all flags) is rejected.
//!
//! Rules:
//! - One row per category, or per (category, overlay value) pair, in order of first appearance
//!   in the data; the sort setting does not reorder the data (`dist_drug` and
//!   `dist_drug_by_occurrence` both list drugY, drugC, drugX, drugA, drugB; `dist_bp_color`).
//! - Columns: the field, `__count__`, the overlay field if any, `__tooltip__`. The tooltip is
//!   `<field> = <value>`, `<overlay> = <value>`, `Count = <n>` on separate lines.
//! - With `graphNormalizeData` the `__count__` column is the percentage of the category's records
//!   (`dist_bp_color_norm`: HIGH/drugY 38 of 77 is 49.35064935064935) and the tooltip adds
//!   `Percent = <p>` with at most three decimals.
//!
//! - Real categories are written as Java doubles (`team/stream_1_M`: churn `1.0`), integer ones
//!   as integers (`team/decision_trees_heart`: `2`).
//!
//! GAP: null and blank values (no probe has them) are left out.

use super::graph_data::{category_text, java_double, tooltip_number};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Table, Value};
use std::sync::Arc;

struct Distribution {
    field: String,
    overlay: Option<String>,
    normalize: bool,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    match settings::text(s, "distributionSelectionMode")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("selectedFields") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Distribution of {other:?} is not supported yet"
            )));
        }
    }
    let field = settings::text(s, "graphXField")
        .filter(|f| !f.trim().is_empty())
        .ok_or_else(|| OperatorError::Settings("Distribution: no field (graphXField)".into()))?;
    Ok(Box::new(Distribution {
        field,
        overlay: settings::text(s, "graphColorField").filter(|f| !f.trim().is_empty()),
        normalize: settings::flag(s, "graphNormalizeData")?.unwrap_or(false),
    }))
}

impl Operator for Distribution {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Distribution is a graph node and produces a Document, not a Table".into(),
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
            .ok_or_else(|| OperatorError::Inputs("Distribution needs one input".into()))?;
        let position = |name: &str| {
            input
                .model()
                .position(name)
                .ok_or_else(|| OperatorError::Data(format!("no field {name}")))
        };
        let x = position(&self.field)?;
        let overlay = self.overlay.as_deref().map(position).transpose()?;
        let text = |row: &[Value], at: usize| -> Option<String> {
            let value = row.get(at)?;
            let field = input.model().fields().get(at)?;
            (!value.is_null() && !field.is_missing(value)).then(|| category_text(value))
        };

        // (category, overlay value) -> count, in order of first appearance.
        let mut cells: Vec<(String, Option<String>, f64)> = Vec::new();
        for row in input.rows() {
            let Some(category) = text(row, x) else {
                continue;
            };
            let colour = match overlay {
                Some(at) => match text(row, at) {
                    Some(c) => Some(c),
                    None => continue,
                },
                None => None,
            };
            match cells
                .iter_mut()
                .find(|(c, o, _)| *c == category && *o == colour)
            {
                Some(cell) => cell.2 += 1.0,
                None => cells.push((category, colour, 1.0)),
            }
        }
        let total = |category: &str| -> f64 {
            cells
                .iter()
                .filter(|(c, _, _)| c == category)
                .map(|(_, _, n)| n)
                .sum()
        };

        let mut header = vec![self.field.clone(), "__count__".to_string()];
        if let Some(name) = &self.overlay {
            header.push(name.clone());
        }
        header.push("__tooltip__".to_string());
        let rows = cells
            .iter()
            .map(|(category, colour, count)| {
                let percent = count / total(category) * 100.0;
                let value = if self.normalize { percent } else { *count };
                let mut tooltip = format!("{} = {category}", self.field);
                let mut row = vec![category.clone(), java_double(value)];
                if let (Some(name), Some(colour)) = (&self.overlay, colour) {
                    tooltip.push_str(&format!("\n{name} = {colour}"));
                    row.push(colour.clone());
                }
                tooltip.push_str(&format!("\nCount = {}", tooltip_number(*count)));
                if self.normalize {
                    tooltip.push_str(&format!("\nPercent = {}", tooltip_number(percent)));
                }
                row.push(tooltip);
                row
            })
            .collect();
        Ok(NodeOutput::Document(Arc::new(Document {
            title: format!("Distribution of {}", self.field),
            tables: vec![DocumentTable {
                title: None,
                header,
                rows,
            }],
        })))
    }
}
