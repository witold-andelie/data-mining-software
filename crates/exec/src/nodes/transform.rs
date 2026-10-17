//! Transform (Output palette, `TransformNode`): a table of the chosen fields against the
//! transformations to preview (`oracle/misc_next/transform_*`). Modeler fills the cells with
//! distribution graphs, which its table export leaves empty; the document keeps that table.
//!
//! Settings: `transformFields` (rows, in the listed order: `pos`, `rv` in `transform_all`),
//! `allTransformations` (`all`, or `select` with `includeInverse`, `includeLogN`,
//! `includeLog10`, `includeExponential`, `includeSquareRoot`), `outputMode` (`screen`).
//! The columns are `Field`, `Selected Transform`, `Current Distribution`, then the
//! transformations in that fixed order (`transform_select` with all five equals
//! `transform_all`; `transform_html` with only the square root has that one column). The
//! offsets (`inverseOffset`, ...) only shape the graphs and the Derive nodes Modeler generates
//! from the output.
//!
//! GAP: file output (`outputMode` `file`, `transform_html`) writes the graphs as images and is
//! rejected.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::Table;
use std::sync::Arc;

/// The transformations in column order, with their settings.
const TRANSFORMATIONS: [(&str, &str); 5] = [
    ("Inverse", "includeInverse"),
    ("LogN", "includeLogN"),
    ("Log10", "includeLog10"),
    ("Exponential", "includeExponential"),
    ("Square Root", "includeSquareRoot"),
];

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    match settings::text(s, "outputMode").as_deref().map(str::trim) {
        None | Some("") | Some("screen") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Transform outputMode {other:?} (graph images) is not supported"
            )));
        }
    }
    let columns = match settings::text(s, "allTransformations")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("all") => TRANSFORMATIONS.iter().map(|(name, _)| *name).collect(),
        Some("select") => {
            let mut columns = Vec::new();
            for (name, setting) in TRANSFORMATIONS {
                if settings::flag(s, setting)?.unwrap_or(false) {
                    columns.push(name);
                }
            }
            columns
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Transform allTransformations {other:?} is unknown"
            )));
        }
    };
    Ok(Box::new(Transform {
        fields: settings::children(s, "transformFields", "field")
            .into_iter()
            .map(|f| f.text().trim().to_string())
            .filter(|name| !name.is_empty())
            .collect(),
        columns,
    }))
}

struct Transform {
    fields: Vec<String>,
    columns: Vec<&'static str>,
}

impl Operator for Transform {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Transform is a terminal output node and produces a Document, not a Table".into(),
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
            .ok_or_else(|| OperatorError::Inputs("Transform needs one input".into()))?;
        for name in &self.fields {
            if input.model().position(name).is_none() {
                return Err(OperatorError::Data(format!(
                    "Transform field {name} is not in the input"
                )));
            }
        }
        let header: Vec<String> = ["Field", "Selected Transform", "Current Distribution"]
            .into_iter()
            .chain(self.columns.iter().copied())
            .map(str::to_string)
            .collect();
        let rows = self
            .fields
            .iter()
            .map(|name| {
                std::iter::once(name.clone())
                    .chain(std::iter::repeat_n(String::new(), header.len() - 1))
                    .collect()
            })
            .collect();
        Ok(NodeOutput::Document(Arc::new(Document {
            title: String::new(),
            tables: vec![DocumentTable {
                title: None,
                header,
                rows,
            }],
        })))
    }
}
