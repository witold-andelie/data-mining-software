//! Web (Graphs): links between the categories of several fields, weighted by how many records
//! have both. The output is the data behind the graph, as Modeler's VIZML export holds it: two
//! tables, the nodes and the links (`nodes::graph_data`; `oracle/graphs/web_bp_chol_drug`).
//!
//! Settings: `webFields`, `webTypeIsDirected` (false only), `webTrueFlags` (false only),
//! `webLinksFilter` `ShowMaximum` with `webMaxNumberOfLinks` (default 80);
//! `webDiscardIfVeryFewRecords`/`VeryManyRecords` must be off. The link thresholds for weak and
//! strong lines (`webWeakConnections`, `webStrongConnections`) only change the drawing.
//!
//! Rules:
//! - Nodes (`__nodename__;Field;Value`): every category of each field, fields in the order given,
//!   categories in the Type's order (sorted when the Type has none), named `<field>.<value>`.
//! - Links (`FROM;TO;Count`): each pair of categories of two different fields that occur together
//!   in at least one record, from the earlier field to the later one, with the number of such
//!   records (`web_bp_chol_drug`: 23 links, `BP.HIGH` to `Cholesterol.HIGH` 35). Records with a
//!   null in a field do not link that field.
//! - With more links than `webMaxNumberOfLinks`, the strongest are kept.
//!
//! GAP: Modeler lists the links in an order we do not reproduce (it looks like a hash order); we
//! list them by field, then category, and the oracle test compares them as a set.

use super::graph_data::{category_text, java_double};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Table, Value, ValueSpec};
use std::sync::Arc;

struct Web {
    fields: Vec<String>,
    max_links: usize,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let unsupported =
        |what: &str| OperatorError::Settings(format!("Web: {what} is not supported yet"));
    if settings::flag(s, "webTypeIsDirected")?.unwrap_or(false) {
        return Err(unsupported("a directed web"));
    }
    if settings::flag(s, "webTrueFlags")?.unwrap_or(false) {
        return Err(unsupported("true flags only"));
    }
    if settings::flag(s, "webDiscardIfVeryFewRecords")?.unwrap_or(false)
        || settings::flag(s, "webDiscardIfVeryManyRecords")?.unwrap_or(false)
    {
        return Err(unsupported("discarding links by record counts"));
    }
    match settings::text(s, "webLinksFilter")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("ShowMaximum") => {}
        Some(other) => return Err(unsupported(&format!("link filter {other}"))),
    }
    let fields: Vec<String> = settings::children(s, "webFields", "field")
        .into_iter()
        .map(|f| f.text().trim().to_string())
        .filter(|f| !f.is_empty())
        .collect();
    if fields.len() < 2 {
        return Err(OperatorError::Settings(
            "Web needs at least two fields".into(),
        ));
    }
    Ok(Box::new(Web {
        fields,
        max_links: settings::integer(s, "webMaxNumberOfLinks")?
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(80),
    }))
}

impl Operator for Web {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Web is a graph node and produces a Document, not a Table".into(),
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
            .ok_or_else(|| OperatorError::Inputs("Web needs one input".into()))?;
        let model = input.model();
        let positions = self
            .fields
            .iter()
            .map(|name| {
                model
                    .position(name)
                    .ok_or_else(|| OperatorError::Data(format!("no field {name}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let present = |row: &[Value], at: usize| -> Option<String> {
            let value = row.get(at)?;
            let field = model.fields().get(at)?;
            (!value.is_null() && !field.is_missing(value)).then(|| category_text(value))
        };

        // Categories per field: the Type's, else the values seen, sorted.
        let categories: Vec<Vec<String>> = positions
            .iter()
            .map(|&at| {
                let field = &model.fields()[at];
                match &field.values {
                    ValueSpec::Categories(values) if !values.is_empty() => {
                        values.iter().map(category_text).collect()
                    }
                    ValueSpec::Flag {
                        true_value,
                        false_value,
                    } => {
                        let mut both = vec![category_text(false_value), category_text(true_value)];
                        both.sort();
                        both
                    }
                    _ => {
                        let mut seen: Vec<String> = input
                            .rows()
                            .iter()
                            .filter_map(|row| present(row, at))
                            .collect();
                        seen.sort();
                        seen.dedup();
                        seen
                    }
                }
            })
            .collect();
        let mut node_rows = Vec::new();
        for (name, values) in self.fields.iter().zip(&categories) {
            for value in values {
                node_rows.push(vec![format!("{name}.{value}"), name.clone(), value.clone()]);
            }
        }

        // Co-occurrence counts for every pair of fields (earlier -> later).
        let mut links: Vec<(usize, usize, usize, usize, f64)> = Vec::new();
        for (a, &pa) in positions.iter().enumerate() {
            for (b, &pb) in positions.iter().enumerate().skip(a + 1) {
                for row in input.rows() {
                    let (Some(va), Some(vb)) = (present(row, pa), present(row, pb)) else {
                        continue;
                    };
                    let (Some(ia), Some(ib)) = (
                        categories[a].iter().position(|c| *c == va),
                        categories[b].iter().position(|c| *c == vb),
                    ) else {
                        continue;
                    };
                    match links
                        .iter_mut()
                        .find(|l| l.0 == a && l.1 == ia && l.2 == b && l.3 == ib)
                    {
                        Some(link) => link.4 += 1.0,
                        None => links.push((a, ia, b, ib, 1.0)),
                    }
                }
            }
        }
        links.sort_by_key(|l| (l.0, l.1, l.2, l.3));
        if links.len() > self.max_links {
            let mut by_strength: Vec<usize> = (0..links.len()).collect();
            by_strength.sort_by(|&i, &j| links[j].4.total_cmp(&links[i].4));
            let keep: std::collections::HashSet<usize> =
                by_strength.into_iter().take(self.max_links).collect();
            links = links
                .into_iter()
                .enumerate()
                .filter(|(i, _)| keep.contains(i))
                .map(|(_, l)| l)
                .collect();
        }
        let link_rows = links
            .iter()
            .map(|&(a, ia, b, ib, count)| {
                vec![
                    format!("{}.{}", self.fields[a], categories[a][ia]),
                    format!("{}.{}", self.fields[b], categories[b][ib]),
                    java_double(count),
                ]
            })
            .collect();
        Ok(NodeOutput::Document(Arc::new(Document {
            title: "Web".into(),
            tables: vec![
                DocumentTable {
                    title: None,
                    header: vec!["__nodename__".into(), "Field".into(), "Value".into()],
                    rows: node_rows,
                },
                DocumentTable {
                    title: None,
                    header: vec!["FROM".into(), "TO".into(), "Count".into()],
                    rows: link_rows,
                },
            ],
        })))
    }
}
