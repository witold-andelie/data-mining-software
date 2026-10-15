//! Planning and evaluating a stream's top-level diagram.

use crate::context::ExecContext;
use crate::error::{ErrorKind, NodeError};
use crate::operator::{Factory, Operator};
use crate::registry;
use dms_catalog::Resolution;
use dms_stream::{NodeRef, Stream};
use dms_types::Table;
use std::collections::HashMap;
use std::sync::Arc;

pub type NodeResult = Result<Arc<Table>, NodeError>;

/// A node ready to evaluate: its inputs (in link order) and its operator, or why it has none.
pub struct PlannedNode {
    pub uid: String,
    pub catalog_id: Option<&'static str>,
    pub inputs: Vec<usize>,
    operator: Result<Box<dyn Operator>, NodeError>,
}

pub struct Plan {
    nodes: Vec<PlannedNode>,
    by_uid: HashMap<String, usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
    Pending,
    Expanded,
    Done,
}

impl Plan {
    /// Plans the top-level diagram with the built-in node implementations. Nodes whose type has
    /// no implementation still get planned; they fail with `Unsupported` only if evaluated.
    pub fn new(stream: &Stream) -> Plan {
        Plan::with_factories(stream, registry::factory)
    }

    /// Plans with a custom lookup from catalog id to implementation (used by tests).
    pub fn with_factories(stream: &Stream, lookup: impl Fn(&str) -> Option<Factory>) -> Plan {
        let diagram = &stream.diagram;
        let by_uid: HashMap<String, usize> = diagram
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.uid.clone(), index))
            .collect();
        let mut inputs: Vec<Vec<usize>> = vec![Vec::new(); diagram.nodes.len()];
        for link in &diagram.links {
            if let (NodeRef::Uid(from), NodeRef::Uid(to)) = (&link.from, &link.to)
                && let (Some(&from), Some(&to)) = (by_uid.get(from), by_uid.get(to))
                && let Some(list) = inputs.get_mut(to)
            {
                list.push(from);
            }
        }
        let nodes = diagram
            .nodes
            .iter()
            .zip(inputs)
            .map(|(node, inputs)| {
                let catalog_id = match dms_catalog::resolve(node) {
                    Resolution::Node(spec) => Some(spec.id),
                    Resolution::Structural | Resolution::Unmapped => None,
                };
                let operator = match catalog_id.and_then(&lookup) {
                    Some(factory) => factory(node).map_err(|error| error.at(&node.uid)),
                    None => Err(NodeError::new(
                        &node.uid,
                        ErrorKind::Unsupported(
                            catalog_id.map_or_else(|| node.type_identifier(), str::to_string),
                        ),
                    )),
                };
                PlannedNode {
                    uid: node.uid.clone(),
                    catalog_id,
                    inputs,
                    operator,
                }
            })
            .collect();
        Plan { nodes, by_uid }
    }

    pub fn nodes(&self) -> &[PlannedNode] {
        &self.nodes
    }

    /// Indexes of the nodes that feed `uid`.
    pub fn inputs_of(&self, uid: &str) -> Vec<&str> {
        self.by_uid
            .get(uid)
            .and_then(|&index| self.nodes.get(index))
            .map(|node| {
                node.inputs
                    .iter()
                    .filter_map(|&input| self.nodes.get(input).map(|n| n.uid.as_str()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Nodes no other node reads from.
    pub fn terminals(&self) -> Vec<&str> {
        let mut has_successor = vec![false; self.nodes.len()];
        for node in &self.nodes {
            for &input in &node.inputs {
                if let Some(flag) = has_successor.get_mut(input) {
                    *flag = true;
                }
            }
        }
        self.nodes
            .iter()
            .zip(has_successor)
            .filter(|(_, has)| !has)
            .map(|(node, _)| node.uid.as_str())
            .collect()
    }

    /// Evaluates the outputs of `targets` and everything they need, each node at most once.
    /// Failures stay attached to the node that failed; downstream nodes report `Upstream`.
    pub fn evaluate(&self, targets: &[&str], context: &ExecContext) -> HashMap<String, NodeResult> {
        let mut state = vec![Visit::Pending; self.nodes.len()];
        let mut results: Vec<Option<NodeResult>> = vec![None; self.nodes.len()];
        let mut stack: Vec<(usize, bool)> = targets
            .iter()
            .filter_map(|uid| self.by_uid.get(*uid).map(|&index| (index, false)))
            .collect();
        while let Some((index, expanded)) = stack.pop() {
            let Some(node) = self.nodes.get(index) else {
                continue;
            };
            match (state.get(index).copied(), expanded) {
                (Some(Visit::Done) | None, _) => {}
                (Some(Visit::Expanded), false) => {
                    // Reached again while its inputs are being evaluated: a cycle.
                    set(
                        &mut results,
                        index,
                        Err(NodeError::new(&node.uid, ErrorKind::Cycle)),
                    );
                    set_state(&mut state, index, Visit::Done);
                }
                (Some(Visit::Pending), _) => {
                    set_state(&mut state, index, Visit::Expanded);
                    stack.push((index, true));
                    for &input in node.inputs.iter().rev() {
                        if state.get(input) != Some(&Visit::Done) {
                            stack.push((input, false));
                        }
                    }
                }
                (Some(Visit::Expanded), true) => {
                    let result = self.run_node(node, &results, context);
                    set(&mut results, index, result);
                    set_state(&mut state, index, Visit::Done);
                }
            }
        }
        self.nodes
            .iter()
            .zip(results)
            .filter_map(|(node, result)| result.map(|result| (node.uid.clone(), result)))
            .collect()
    }

    fn run_node(
        &self,
        node: &PlannedNode,
        results: &[Option<NodeResult>],
        context: &ExecContext,
    ) -> NodeResult {
        let operator = node.operator.as_ref().map_err(Clone::clone)?;
        let mut inputs = Vec::with_capacity(node.inputs.len());
        for &input in &node.inputs {
            let upstream_uid = self
                .nodes
                .get(input)
                .map_or_else(String::new, |n| n.uid.clone());
            match results.get(input).and_then(Option::as_ref) {
                Some(Ok(table)) => inputs.push(Arc::clone(table)),
                _ => {
                    return Err(NodeError::new(&node.uid, ErrorKind::Upstream(upstream_uid)));
                }
            }
        }
        if !operator.arity().accepts(inputs.len()) {
            return Err(NodeError::new(
                &node.uid,
                ErrorKind::Inputs(format!("{} inputs not accepted", inputs.len())),
            ));
        }
        operator
            .run(&inputs, context)
            .map(Arc::new)
            .map_err(|error| error.at(&node.uid))
    }
}

fn set(results: &mut [Option<NodeResult>], index: usize, result: NodeResult) {
    if let Some(slot) = results.get_mut(index) {
        *slot = Some(result);
    }
}

fn set_state(state: &mut [Visit], index: usize, visit: Visit) {
    if let Some(slot) = state.get_mut(index) {
        *slot = visit;
    }
}
