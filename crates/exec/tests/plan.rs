//! Plan evaluation with substitute operators (no Modeler files needed).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_exec::{
    Arity, ErrorKind, ExecContext, Factory, FileResolver, Operator, OperatorError, Plan, Resolved,
};
use dms_stream::{Diagram, Link, NodeRef, Stream, StreamNode};
use dms_types::{DataModel, Field, Storage, Table, Value};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
        subdiagram: None,
    }
}

fn link(from: &str, to: &str) -> Link {
    Link {
        from: NodeRef::Uid(from.into()),
        to: NodeRef::Uid(to.into()),
    }
}

fn stream(nodes: Vec<StreamNode>, links: Vec<Link>) -> Stream {
    Stream {
        file_version: None,
        save_version: None,
        properties: None,
        diagram: Diagram {
            nodes,
            links,
            terminal_order: Vec::new(),
        },
    }
}

fn one_column(values: &[i64]) -> Table {
    let model = DataModel::new(vec![Field::new("x", Storage::Integer)]).unwrap();
    Table::new(
        model,
        values.iter().map(|v| vec![Value::Integer(*v)]).collect(),
    )
    .unwrap()
}

struct Source;
impl Operator for Source {
    fn arity(&self) -> Arity {
        Arity::Exactly(0)
    }
    fn run(&self, _: &[Arc<Table>], _: &ExecContext) -> Result<Table, OperatorError> {
        Ok(one_column(&[1, 2, 3]))
    }
}

/// Runs of `CountingSource`; only `evaluates_a_chain_and_shares_upstream_work` uses it, so
/// parallel tests cannot disturb the count.
static COUNTING_RUNS: AtomicUsize = AtomicUsize::new(0);

struct CountingSource;
impl Operator for CountingSource {
    fn arity(&self) -> Arity {
        Arity::Exactly(0)
    }
    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        COUNTING_RUNS.fetch_add(1, Ordering::SeqCst);
        Source.run(inputs, context)
    }
}

/// Keeps rows whose value is at least 2.
struct AtLeastTwo;
impl Operator for AtLeastTwo {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }
    fn run(&self, inputs: &[Arc<Table>], _: &ExecContext) -> Result<Table, OperatorError> {
        let input = &inputs[0];
        let rows = input
            .rows()
            .iter()
            .filter(|row| row[0].as_f64().is_some_and(|v| v >= 2.0))
            .cloned()
            .collect();
        Table::new(input.model().clone(), rows).map_err(|e| OperatorError::Data(e.to_string()))
    }
}

struct Failing;
impl Operator for Failing {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }
    fn run(&self, _: &[Arc<Table>], _: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Data("boom".into()))
    }
}

fn lookup(catalog_id: &str) -> Option<Factory> {
    match catalog_id {
        "var_file" => Some(|_| Ok(Box::new(Source))),
        "user_input" => Some(|_| Ok(Box::new(CountingSource))),
        "filter" => Some(|_| Ok(Box::new(AtLeastTwo))),
        "select" => Some(|_| Ok(Box::new(Failing))),
        // The real Table implementation.
        "table" => dms_exec::registry::factory("table"),
        _ => None,
    }
}

#[test]
fn evaluates_a_chain_and_shares_upstream_work() {
    let s = stream(
        vec![
            node("src", "UserInputNode"),
            node("f", "FilterNode"),
            node("t1", "TableNode"),
            node("t2", "TableNode"),
        ],
        vec![link("src", "f"), link("f", "t1"), link("src", "t2")],
    );
    let plan = Plan::with_factories(&s, lookup);
    assert_eq!(plan.terminals(), vec!["t1", "t2"]);
    let results = plan.evaluate(&["t1", "t2"], &ExecContext::default());
    assert_eq!(COUNTING_RUNS.load(Ordering::SeqCst), 1);
    assert_eq!(results["t1"].as_ref().unwrap().row_count(), 2);
    assert_eq!(results["t2"].as_ref().unwrap().row_count(), 3);
    assert_eq!(plan.inputs_of("t1"), vec!["f"]);
}

#[test]
fn failures_stay_with_their_node_and_propagate_downstream() {
    let s = stream(
        vec![
            node("src", "DelimitedFileImportNode"),
            node("bad", "SelectNode"),
            node("out", "TableNode"),
            node("d", "DeriveNode"),
        ],
        vec![link("src", "bad"), link("bad", "out"), link("src", "d")],
    );
    let plan = Plan::with_factories(&s, lookup);
    let results = plan.evaluate(&["out", "d"], &ExecContext::default());
    assert_eq!(
        results["bad"].as_ref().unwrap_err().kind,
        ErrorKind::Data("boom".into())
    );
    assert_eq!(
        results["out"].as_ref().unwrap_err().kind,
        ErrorKind::Upstream("bad".into())
    );
    assert_eq!(
        results["d"].as_ref().unwrap_err().kind,
        ErrorKind::Unsupported("derive".into())
    );
    assert!(results["src"].is_ok());
}

#[test]
fn cycles_and_wrong_input_counts_are_errors_not_hangs() {
    let s = stream(
        vec![
            node("a", "FilterNode"),
            node("b", "FilterNode"),
            node("src", "DelimitedFileImportNode"),
            node("two", "TableNode"),
        ],
        vec![
            link("a", "b"),
            link("b", "a"),
            link("src", "two"),
            link("src", "two"),
        ],
    );
    let plan = Plan::with_factories(&s, lookup);
    let results = plan.evaluate(&["a", "two"], &ExecContext::default());
    assert!(
        results
            .values()
            .any(|r| matches!(r, Err(e) if e.kind == ErrorKind::Cycle))
    );
    assert!(results["a"].is_err());
    assert!(matches!(
        results["two"].as_ref().unwrap_err().kind,
        ErrorKind::Inputs(_)
    ));
}

#[test]
fn resolver_relocates_foreign_paths_by_file_name() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("resolver");
    let nested = dir.join("sub");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("Data.CSV"), "x").unwrap();
    let resolver = FileResolver {
        data_dirs: vec![dir.clone()],
    };
    let resolved = resolver.resolve(r"C:\elsewhere\data.csv").unwrap();
    assert_eq!(resolved.path(), nested.join("Data.CSV"));
    assert!(matches!(resolved, Resolved::Relocated { .. }));
    assert_eq!(resolver.resolve(r"C:\elsewhere\missing.csv"), None);
    assert!(resolver.resolve("$CLEO_DEMOS/data.csv").is_some());
}

fn supernode(uid: &str, nodes: Vec<StreamNode>, links: Vec<Link>) -> StreamNode {
    let mut node = node(uid, "MutableSuperNode");
    node.subdiagram = Some(Diagram {
        nodes,
        links,
        terminal_order: Vec::new(),
    });
    node
}

fn input_connector(uid: &str) -> StreamNode {
    node(uid, "SuperNodeInputConnectorNode")
}

fn output_connector(uid: &str) -> StreamNode {
    node(uid, "SuperNodeOutputConnectorNode")
}

#[test]
fn supernodes_are_expanded_and_keep_their_uid() {
    // src -> S{in -> f -> out} -> t
    let s = stream(
        vec![
            node("src", "DelimitedFileImportNode"),
            supernode(
                "S",
                vec![
                    input_connector("in"),
                    node("f", "FilterNode"),
                    output_connector("out"),
                ],
                vec![link("in", "f"), link("f", "out")],
            ),
            node("t", "TableNode"),
        ],
        vec![link("src", "S"), link("S", "t")],
    );
    let plan = Plan::with_factories(&s, lookup);
    assert_eq!(plan.terminals(), vec!["t"]);
    assert_eq!(plan.inputs_of("t"), vec!["S"]);
    assert_eq!(plan.inputs_of("f"), vec!["in"]);
    let results = plan.evaluate(&["S"], &ExecContext::default());
    assert_eq!(results["S"].as_ref().unwrap().row_count(), 2);
    assert_eq!(results["f"].as_ref().unwrap().row_count(), 2);
}

#[test]
fn nested_source_and_terminal_supernodes() {
    // S1{src -> out1} -> S2{in2 -> S3{in3 -> f -> out3} -> t_inner}
    let s = stream(
        vec![
            supernode(
                "S1",
                vec![
                    node("src", "DelimitedFileImportNode"),
                    output_connector("out1"),
                ],
                vec![link("src", "out1")],
            ),
            supernode(
                "S2",
                vec![
                    input_connector("in2"),
                    supernode(
                        "S3",
                        vec![
                            input_connector("in3"),
                            node("f", "FilterNode"),
                            output_connector("out3"),
                        ],
                        vec![link("in3", "f"), link("f", "out3")],
                    ),
                    node("t_inner", "TableNode"),
                ],
                vec![link("in2", "S3"), link("S3", "t_inner")],
            ),
        ],
        vec![link("S1", "S2")],
    );
    let plan = Plan::with_factories(&s, lookup);
    assert_eq!(plan.terminals(), vec!["t_inner"]);
    assert_eq!(plan.inputs_of("t_inner"), vec!["S3"]);
    let results = plan.evaluate(&["S3", "t_inner"], &ExecContext::default());
    assert_eq!(results["S3"].as_ref().unwrap().row_count(), 2);
    assert_eq!(results["t_inner"].as_ref().unwrap().row_count(), 2);
}

#[test]
fn an_unconnected_supernode_input_fails_its_reader() {
    // S{in -> f -> out} with nothing linked into S.
    let s = stream(
        vec![
            supernode(
                "S",
                vec![
                    input_connector("in"),
                    node("f", "FilterNode"),
                    output_connector("out"),
                ],
                vec![link("in", "f"), link("f", "out")],
            ),
            node("t", "TableNode"),
        ],
        vec![link("S", "t")],
    );
    let plan = Plan::with_factories(&s, lookup);
    let results = plan.evaluate(&["S"], &ExecContext::default());
    let error = results["S"].as_ref().unwrap_err();
    assert!(matches!(error.kind, ErrorKind::Inputs(_)), "{error:?}");
}
