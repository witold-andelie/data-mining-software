//! Planning and evaluating a stream.
//!
//! Supernodes are expanded: their inner nodes join the plan, and the supernode and its
//! connectors become aliases. A node reading a supernode reads the node linked to the
//! supernode's output connector; a node reading an input connector reads what is linked to the
//! supernode from outside. Nested supernodes resolve the same way. Aliases keep their uids, so
//! callers can still ask for a supernode's output by its uid.

use crate::context::ExecContext;
use crate::error::{ErrorKind, NodeError};
use crate::operator::{Factory, Operator};
use crate::output::NodeOutput;
use crate::registry;
use dms_catalog::Resolution;
use dms_stream::{Diagram, NodeRef, Stream, StreamNode};
use std::collections::HashMap;
use std::sync::Arc;

pub type NodeResult = Result<NodeOutput, NodeError>;

/// What happens while a stream is evaluated, in evaluation order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalEvent<'a> {
    /// A real node is about to run. Indices start at zero; total counts the nodes reachable
    /// through inputs and model builders, once each, before evaluation starts.
    NodeStarted {
        uid: &'a str,
        index: usize,
        total: usize,
    },
    /// The node finished, successfully or with an error.
    NodeFinished { uid: &'a str, ok: bool },
}

/// A node ready to evaluate: its inputs (in link order) and its operator, or why it has none.
pub struct PlannedNode {
    pub uid: String,
    pub catalog_id: Option<&'static str>,
    /// The nodes that feed this one, with supernodes and connectors resolved.
    pub inputs: Vec<usize>,
    /// The uids this node is linked to, as saved (a supernode stays a supernode).
    linked_inputs: Vec<String>,
    /// For a model nugget: the build node it is linked to (`builderNodeId`), when it is in the
    /// stream. It runs first, and its model replaces the nugget's saved one, as in Modeler.
    pub model_from: Option<usize>,
    operator: Result<Box<dyn Operator>, NodeError>,
}

pub struct Plan {
    nodes: Vec<PlannedNode>,
    /// Real nodes and the aliases (supernodes, connectors) that resolve to them.
    by_uid: HashMap<String, usize>,
    initial_globals: Result<dms_clem::GlobalValues, crate::error::OperatorError>,
    terminal_order: Vec<String>,
}

/// One execution of a stream. Saved globals seed this state once; subsequent terminal runs
/// and exports share it. Each evaluate call recomputes data, so a cached CLEM reader cannot
/// hide a later Set Globals update. A new execution never inherits another one's mutations.
pub struct Execution<'a> {
    plan: &'a Plan,
    globals: dms_clem::GlobalValues,
}

impl Execution<'_> {
    pub fn globals(&self) -> &dms_clem::GlobalValues {
        &self.globals
    }

    pub fn evaluate(
        &mut self,
        targets: &[&str],
        context: &ExecContext,
    ) -> HashMap<String, NodeResult> {
        self.evaluate_observed(targets, context, &mut |_| {})
    }

    /// Evaluates as usual, reporting only real `run_node` calls to the observer.
    pub fn evaluate_observed(
        &mut self,
        targets: &[&str],
        context: &ExecContext,
        observer: &mut dyn FnMut(EvalEvent<'_>),
    ) -> HashMap<String, NodeResult> {
        self.plan
            .evaluate_with_globals(targets, context, &mut self.globals, observer)
    }
}

/// Supernodes nested deeper than this, or alias chains longer than this, are rejected.
const MAX_ALIAS_DEPTH: usize = 256;

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
        let mut walk = Walk::default();
        walk.diagram(&stream.diagram, 0);
        let by_real: HashMap<&str, usize> = walk
            .real
            .iter()
            .enumerate()
            .map(|(index, node)| (node.uid.as_str(), index))
            .collect();
        let resolve = |uid: &str| -> Option<usize> {
            let mut current = uid;
            for _ in 0..MAX_ALIAS_DEPTH {
                if let Some(&index) = by_real.get(current) {
                    return Some(index);
                }
                current = walk.aliases.get(current)?.as_str();
            }
            None
        };
        let mut by_uid: HashMap<String, usize> = HashMap::new();
        for uid in by_real
            .keys()
            .copied()
            .chain(walk.aliases.keys().map(String::as_str))
        {
            if let Some(index) = resolve(uid) {
                by_uid.insert(uid.to_string(), index);
            }
        }
        let nodes = walk
            .real
            .iter()
            .map(|node| {
                let linked_inputs = walk.inputs.get(&node.uid).cloned().unwrap_or_default();
                let resolved: Vec<Option<usize>> =
                    linked_inputs.iter().map(|uid| resolve(uid)).collect();
                let catalog_id = match dms_catalog::resolve(node) {
                    Resolution::Node(spec) => Some(spec.id),
                    Resolution::Structural | Resolution::Unmapped => None,
                };
                let operator = if let Some(dangling) = linked_inputs
                    .iter()
                    .zip(&resolved)
                    .find_map(|(uid, index)| index.is_none().then_some(uid))
                {
                    Err(NodeError::new(
                        &node.uid,
                        ErrorKind::Inputs(format!(
                            "the input {dangling} leads to no node (an unconnected supernode \
                             connector?)"
                        )),
                    ))
                } else {
                    match catalog_id.and_then(&lookup) {
                        Some(factory) => factory(node).map_err(|error| error.at(&node.uid)),
                        None => Err(NodeError::new(
                            &node.uid,
                            ErrorKind::Unsupported(
                                catalog_id.map_or_else(|| node.type_identifier(), str::to_string),
                            ),
                        )),
                    }
                };
                let model_from = crate::settings::text(node.settings.as_ref(), "builderNodeId")
                    .and_then(|builder| by_real.get(builder.trim()).copied());
                PlannedNode {
                    uid: node.uid.clone(),
                    catalog_id,
                    inputs: resolved.into_iter().flatten().collect(),
                    linked_inputs,
                    model_from,
                    operator,
                }
            })
            .collect();
        Plan {
            nodes,
            by_uid,
            initial_globals: crate::nodes::set_globals::saved_globals(stream.properties.as_ref()),
            terminal_order: stream
                .diagram
                .terminal_order
                .iter()
                .filter_map(|node| match node {
                    NodeRef::Uid(uid) => Some(uid.clone()),
                    NodeRef::Unresolved(_) => None,
                })
                .collect(),
        }
    }

    pub fn nodes(&self) -> &[PlannedNode] {
        &self.nodes
    }

    pub fn execution(&self) -> Result<Execution<'_>, crate::error::OperatorError> {
        Ok(Execution {
            plan: self,
            globals: self.initial_globals.clone()?,
        })
    }

    /// The uids `uid` is linked to, as saved: a supernode input stays the supernode, so its
    /// output can be asked for (and exported) under that uid.
    pub fn inputs_of(&self, uid: &str) -> Vec<&str> {
        self.by_uid
            .get(uid)
            .and_then(|&index| self.nodes.get(index))
            .map(|node| node.linked_inputs.iter().map(String::as_str).collect())
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

    /// Saved terminalNodeOrder, followed by any terminals absent from that list. Modeler's
    /// public stream.iterator order is a separate order; callers can explicitly run targets
    /// one at a time in their recorded iterator order using one Execution.
    pub fn terminals_in_run_order(&self) -> Vec<&str> {
        let terminals = self.terminals();
        let mut ordered = Vec::new();
        for uid in &self.terminal_order {
            if terminals.contains(&uid.as_str()) && !ordered.contains(&uid.as_str()) {
                ordered.push(uid.as_str());
            }
        }
        for uid in terminals {
            if !ordered.contains(&uid) {
                ordered.push(uid);
            }
        }
        ordered
    }

    /// Evaluates the outputs of `targets` and everything they need, each node at most once.
    /// Failures stay attached to the node that failed; downstream nodes report `Upstream`. A
    /// target that is a supernode or connector gets the result of the node it resolves to.
    pub fn evaluate(&self, targets: &[&str], context: &ExecContext) -> HashMap<String, NodeResult> {
        match self.execution() {
            Ok(mut execution) => execution.evaluate(targets, context),
            Err(error) => targets
                .iter()
                .map(|uid| ((*uid).to_string(), Err(error.clone().at(uid))))
                .collect(),
        }
    }

    fn evaluate_with_globals(
        &self,
        targets: &[&str],
        context: &ExecContext,
        globals: &mut dms_clem::GlobalValues,
        observer: &mut dyn FnMut(EvalEvent<'_>),
    ) -> HashMap<String, NodeResult> {
        let total = self.reachable_count(targets);
        let mut started = 0usize;
        let mut state = vec![Visit::Pending; self.nodes.len()];
        let mut results: Vec<Option<NodeResult>> = vec![None; self.nodes.len()];
        let mut stack: Vec<(usize, bool)> = targets
            .iter()
            .rev()
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
                    for &input in node.inputs.iter().chain(&node.model_from).rev() {
                        if state.get(input) != Some(&Visit::Done) {
                            stack.push((input, false));
                        }
                    }
                }
                (Some(Visit::Expanded), true) => {
                    observer(EvalEvent::NodeStarted {
                        uid: &node.uid,
                        index: started,
                        total,
                    });
                    started = started.saturating_add(1);
                    let result = self.run_node(node, &results, context, globals);
                    observer(EvalEvent::NodeFinished {
                        uid: &node.uid,
                        ok: result.is_ok(),
                    });
                    set(&mut results, index, result);
                    set_state(&mut state, index, Visit::Done);
                }
            }
        }
        let mut out: HashMap<String, NodeResult> = self
            .nodes
            .iter()
            .zip(&results)
            .filter_map(|(node, result)| result.clone().map(|result| (node.uid.clone(), result)))
            .collect();
        for target in targets {
            if !out.contains_key(*target)
                && let Some(result) = self
                    .by_uid
                    .get(*target)
                    .and_then(|&index| results.get(index))
                    .and_then(Option::clone)
            {
                out.insert((*target).to_string(), result);
            }
        }
        out
    }

    fn reachable_count(&self, targets: &[&str]) -> usize {
        let mut seen = vec![false; self.nodes.len()];
        let mut stack: Vec<usize> = targets
            .iter()
            .filter_map(|uid| self.by_uid.get(*uid).copied())
            .collect();
        let mut count = 0usize;
        while let Some(index) = stack.pop() {
            let Some(visited) = seen.get_mut(index) else {
                continue;
            };
            if *visited {
                continue;
            }
            *visited = true;
            count = count.saturating_add(1);
            if let Some(node) = self.nodes.get(index) {
                stack.extend(node.inputs.iter().chain(&node.model_from).copied());
            }
        }
        count
    }

    fn run_node(
        &self,
        node: &PlannedNode,
        results: &[Option<NodeResult>],
        context: &ExecContext,
        globals: &mut dms_clem::GlobalValues,
    ) -> NodeResult {
        let operator = node.operator.as_ref().map_err(Clone::clone)?;
        let mut inputs = Vec::with_capacity(node.inputs.len());
        for &input in &node.inputs {
            let upstream_uid = self
                .nodes
                .get(input)
                .map_or_else(String::new, |n| n.uid.clone());
            match results.get(input).and_then(Option::as_ref) {
                Some(Ok(NodeOutput::Data(table))) => inputs.push(Arc::clone(table)),
                Some(Ok(_)) => {
                    return Err(NodeError::new(
                        &node.uid,
                        ErrorKind::Inputs(format!("{upstream_uid} produces no data")),
                    ));
                }
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
        // A build node that failed or is not implemented leaves the nugget its saved model.
        let model = node
            .model_from
            .and_then(|builder| results.get(builder))
            .and_then(Option::as_ref)
            .and_then(|result| result.as_ref().ok())
            .and_then(NodeOutput::model)
            .cloned();
        operator
            .produce_with_globals(&inputs, model, context, globals)
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

/// Collects the real nodes of a stream, the links into them and the aliases of supernodes and
/// their connectors, across nested diagrams.
#[derive(Default)]
struct Walk<'a> {
    real: Vec<&'a StreamNode>,
    /// Linked input uids of each real node, in link order.
    inputs: HashMap<String, Vec<String>>,
    /// Alias uid -> the uid it stands for.
    aliases: HashMap<String, String>,
}

impl<'a> Walk<'a> {
    fn diagram(&mut self, diagram: &'a Diagram, depth: usize) {
        for node in &diagram.nodes {
            match (&node.subdiagram, connector(node)) {
                (Some(inner), _) => {
                    // The supernode stands for what its output connector receives; its input
                    // connector stands for what the supernode receives.
                    for inner_node in &inner.nodes {
                        let from_inside = |kind: Connector| connector(inner_node) == Some(kind);
                        if from_inside(Connector::Output)
                            && let Some(source) = links_to(inner, &inner_node.uid).first()
                        {
                            self.aliases.insert(node.uid.clone(), source.clone());
                        }
                        if from_inside(Connector::Input)
                            && let Some(source) = links_to(diagram, &node.uid).first()
                        {
                            self.aliases.insert(inner_node.uid.clone(), source.clone());
                        }
                    }
                    if depth < MAX_ALIAS_DEPTH {
                        self.diagram(inner, depth + 1);
                    }
                }
                (None, Some(_)) => {}
                (None, None) => {
                    self.real.push(node);
                    self.inputs
                        .insert(node.uid.clone(), links_to(diagram, &node.uid));
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Connector {
    Input,
    Output,
}

fn connector(node: &StreamNode) -> Option<Connector> {
    match node.type_identifier().as_str() {
        "SuperNodeInputConnectorNode" => Some(Connector::Input),
        "SuperNodeOutputConnectorNode" => Some(Connector::Output),
        _ => None,
    }
}

/// The uids linked into `to` within one diagram, in link order.
fn links_to(diagram: &Diagram, to: &str) -> Vec<String> {
    diagram
        .links
        .iter()
        .filter_map(|link| match (&link.from, &link.to) {
            (NodeRef::Uid(from), NodeRef::Uid(target)) if target == to => Some(from.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Arity, OperatorError};
    use dms_stream::Link;
    use dms_types::{DataModel, Field, Storage, Table, Value};

    fn node(uid: &str, class: &str) -> StreamNode {
        StreamNode {
            uid: uid.into(),
            class: format!("com.example.{class}"),
            extension: None,
            model_builder: None,
            name: None,
            label: None,
            position: None,
            properties_attachment: None,
            containers: Vec::new(),
            settings: None,
            binning_output: None,
            result_items: Vec::new(),
            subdiagram: None,
        }
    }

    fn stream(nodes: Vec<StreamNode>, links: &[(&str, &str)]) -> Stream {
        Stream {
            file_version: None,
            save_version: None,
            properties: None,
            diagram: Diagram {
                nodes,
                links: links
                    .iter()
                    .map(|(from, to)| Link {
                        from: NodeRef::Uid((*from).into()),
                        to: NodeRef::Uid((*to).into()),
                    })
                    .collect(),
                terminal_order: Vec::new(),
            },
        }
    }

    struct Source;
    impl Operator for Source {
        fn arity(&self) -> Arity {
            Arity::Exactly(0)
        }
        fn run(&self, _: &[Arc<Table>], _: &ExecContext) -> Result<Table, OperatorError> {
            Ok(Table::new(
                DataModel::new(vec![Field::new("x", Storage::Integer)]).unwrap(),
                vec![vec![Value::Integer(42)]],
            )
            .unwrap())
        }
    }

    fn lookup(id: &str) -> Option<Factory> {
        match id {
            "user_input" => Some(|_| Ok(Box::new(Source))),
            _ => registry::factory(id),
        }
    }

    fn comparable(
        results: HashMap<String, NodeResult>,
    ) -> HashMap<String, Result<Table, NodeError>> {
        results
            .into_iter()
            .map(|(uid, result)| {
                let result = result.map(|output| output.data().unwrap().as_ref().clone());
                (uid, result)
            })
            .collect()
    }

    type RecordedEvent = (String, bool, usize, usize);

    fn observed(
        plan: &Plan,
        targets: &[&str],
    ) -> (HashMap<String, NodeResult>, Vec<RecordedEvent>) {
        let mut events = Vec::new();
        let results = plan.execution().unwrap().evaluate_observed(
            targets,
            &ExecContext::default(),
            &mut |event| match event {
                EvalEvent::NodeStarted { uid, index, total } => {
                    events.push((uid.into(), true, index, total))
                }
                EvalEvent::NodeFinished { uid, ok } => {
                    events.push((uid.into(), false, usize::from(ok), 0))
                }
            },
        );
        (results, events)
    }

    #[test]
    fn observed_results_and_shared_node_events_match_normal_evaluation() {
        let stream = stream(
            vec![
                node("source", "UserInputNode"),
                node("left", "TableNode"),
                node("right", "TableNode"),
            ],
            &[("source", "left"), ("source", "right")],
        );
        let plan = Plan::with_factories(&stream, lookup);
        let targets = ["left", "right", "left", "missing"];
        let (results, events) = observed(&plan, &targets);
        assert_eq!(
            comparable(results),
            comparable(plan.evaluate(&targets, &ExecContext::default()))
        );
        assert_eq!(events.len(), 6);
        for (index, (pair, uid)) in events
            .chunks_exact(2)
            .zip(["source", "left", "right"])
            .enumerate()
        {
            assert_eq!(pair[0], (uid.into(), true, index, 3));
            assert_eq!(pair[1], (uid.into(), false, 1, 0));
        }
    }

    #[test]
    fn observed_failures_cycles_and_unknown_targets_preserve_results() {
        let stream = stream(
            vec![
                node("mystery", "Mystery"),
                node("out", "TableNode"),
                node("a", "TableNode"),
                node("b", "TableNode"),
            ],
            &[("mystery", "out"), ("a", "b"), ("b", "a")],
        );
        let plan = Plan::new(&stream);
        let targets = ["out", "a", "missing"];
        let (results, events) = observed(&plan, &targets);
        assert_eq!(
            comparable(results),
            comparable(plan.evaluate(&targets, &ExecContext::default()))
        );
        assert_eq!(events.len(), 6); // The cycle's repeated node never calls run_node.
        for (index, pair) in events.chunks_exact(2).enumerate() {
            assert_eq!(pair[0].2, index);
            assert_eq!(pair[0].3, 4); // Reachable nodes include the cycle error.
            assert_eq!(pair[0].0, pair[1].0);
            assert_eq!(pair[1].2, 0);
        }
        assert!(observed(&plan, &["missing"]).1.is_empty());
    }

    #[test]
    fn observed_aliases_and_model_builders_are_counted_once() {
        let mut model = node("model", "TableNode");
        model.settings = Some(dms_strfmt::fastinfoset::Element {
            name: dms_strfmt::fastinfoset::QualifiedName {
                prefix: None,
                namespace: None,
                local: Arc::from("properties"),
            },
            namespace_declarations: Vec::new(),
            attributes: Vec::new(),
            children: vec![dms_strfmt::fastinfoset::Node::Element(
                dms_strfmt::fastinfoset::Element {
                    name: dms_strfmt::fastinfoset::QualifiedName {
                        prefix: None,
                        namespace: None,
                        local: Arc::from("builderNodeId"),
                    },
                    namespace_declarations: Vec::new(),
                    attributes: Vec::new(),
                    children: vec![dms_strfmt::fastinfoset::Node::Text(Arc::from("builder"))],
                },
            )],
        });
        let mut alias = node("alias", "MutableSuperNode");
        alias.subdiagram = Some(
            stream(
                vec![
                    node("source", "UserInputNode"),
                    node("connector", "SuperNodeOutputConnectorNode"),
                ],
                &[("source", "connector")],
            )
            .diagram,
        );
        let stream = stream(
            vec![alias, node("builder", "UserInputNode"), model],
            &[("alias", "model")],
        );
        let plan = Plan::with_factories(&stream, lookup);
        let targets = ["alias", "model", "builder"];
        let (results, events) = observed(&plan, &targets);
        assert_eq!(
            comparable(results),
            comparable(plan.evaluate(&targets, &ExecContext::default()))
        );
        let starts: Vec<_> = events
            .iter()
            .filter(|event| event.1)
            .map(|event| (event.0.as_str(), event.2, event.3))
            .collect();
        assert_eq!(
            starts,
            [("source", 0, 3), ("builder", 1, 3), ("model", 2, 3)]
        );
    }
}
