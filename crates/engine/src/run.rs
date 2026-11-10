//! Stream execution on the calling worker thread, with node progress and attributed errors.

use crate::job::{JobContext, Progress};
use dms_exec::{ErrorKind, EvalEvent, ExecContext, FileResolver, NodeError, NodeResult, Plan};
use dms_stream::{NodeRef, Stream};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum RunTargets {
    Stream,
    Nodes(Vec<String>),
}

pub struct RunRequest {
    pub stream: Arc<Stream>,
    pub attachments: Arc<HashMap<String, Arc<[u8]>>>,
    pub data_dirs: Vec<PathBuf>,
    pub export_dir: PathBuf,
    pub targets: RunTargets,
}

#[derive(Debug)]
pub struct RunReport {
    pub cancelled: bool,
    pub targets: Vec<String>,
    pub results: HashMap<String, NodeResult>,
    /// Where the run broke (excluding `Upstream` and `Cancelled`), one per node: nodes that failed while
    /// running, in evaluation order, then errors set without running a node (cycles) by uid.
    pub root_causes: Vec<NodeError>,
    pub elapsed: Duration,
}

/// Plans and runs here so neither a plan nor its operators have to cross thread boundaries.
/// The nodes "Run Stream" runs: the terminal nodes in their saved run order, except nodes with
/// neither inputs nor a place in the saved order (an unconnected model nugget left on the
/// canvas cannot run, and running the stream does not ask it to).
fn stream_targets(stream: &Stream, plan: &Plan) -> Vec<String> {
    let saved: HashSet<&str> = stream
        .diagram
        .terminal_order
        .iter()
        .filter_map(|node| match node {
            NodeRef::Uid(uid) => Some(uid.as_str()),
            NodeRef::Unresolved(_) => None,
        })
        .collect();
    plan.terminals_in_run_order()
        .into_iter()
        .filter(|uid| saved.contains(uid) || !plan.inputs_of(uid).is_empty())
        .map(str::to_string)
        .collect()
}

pub fn run_stream(request: RunRequest, ctx: &JobContext) -> RunReport {
    let started = Instant::now();
    let plan = Plan::new(&request.stream);
    let (targets, sequential) = match request.targets {
        RunTargets::Stream => (
            stream_targets(&request.stream, &plan),
            plan.nodes()
                .iter()
                .any(|node| node.catalog_id == Some("set_globals")),
        ),
        RunTargets::Nodes(uids) => (uids, false),
    };
    let mut report = RunReport {
        cancelled: false,
        targets: Vec::new(),
        results: HashMap::new(),
        root_causes: Vec::new(),
        elapsed: Duration::ZERO,
    };
    let mut execution = match plan.execution() {
        Ok(execution) => execution,
        Err(error) => {
            report.cancelled = ctx.is_cancelled();
            report.targets = targets.clone();
            let mut seen = HashSet::new();
            for target in targets {
                let error = if report.cancelled {
                    dms_exec::OperatorError::Cancelled
                } else {
                    error.clone()
                }
                .at(&target);
                report.results.insert(target, Err(error.clone()));
                if error.kind != ErrorKind::Cancelled && seen.insert(error.uid.clone()) {
                    report.root_causes.push(error);
                }
            }
            report.elapsed = started.elapsed();
            return report;
        }
    };
    let context = ExecContext {
        cancel: ctx.cancel_token(),
        files: FileResolver {
            data_dirs: request.data_dirs,
        },
        stream_properties: request.stream.properties.clone().map(Arc::new),
        attachments: request.attachments,
        decimal_places: None,
        export_dir: Some(request.export_dir),
    };
    let batches = if sequential {
        targets.iter().map(|uid| vec![uid.as_str()]).collect()
    } else {
        vec![targets.iter().map(String::as_str).collect::<Vec<_>>()]
    };
    let mut done = 0usize;
    let mut total = 0usize;
    for batch in batches {
        let base = total;
        let completed = done;
        // Run failures in evaluation order; cancellation is not a root cause.
        let mut failed: Vec<String> = Vec::new();
        let results = execution.evaluate_observed(&batch, &context, &mut |event| {
            let current = match event {
                EvalEvent::NodeStarted {
                    uid,
                    index,
                    total: count,
                } => {
                    total = base.saturating_add(count);
                    done = completed.saturating_add(index);
                    Some(uid.to_string())
                }
                EvalEvent::NodeFinished { uid, ok } => {
                    if !ok {
                        failed.push(uid.to_string());
                    }
                    done = done.saturating_add(1);
                    None
                }
            };
            ctx.progress(Progress {
                done,
                total,
                current,
            });
        });
        for uid in &failed {
            if let Some(Err(error)) = results.get(uid)
                && !matches!(error.kind, ErrorKind::Upstream(_) | ErrorKind::Cancelled)
                && !report
                    .root_causes
                    .iter()
                    .any(|known| known.uid == error.uid)
            {
                report.root_causes.push(error.clone());
            }
        }
        // Errors set without running the node (a cycle) have no Finished event.
        let mut unreported: Vec<&NodeError> = results
            .values()
            .filter_map(|result| result.as_ref().err())
            .filter(|error| !matches!(error.kind, ErrorKind::Upstream(_) | ErrorKind::Cancelled))
            .filter(|error| {
                !report
                    .root_causes
                    .iter()
                    .any(|known| known.uid == error.uid)
            })
            .collect();
        unreported.sort_by(|a, b| a.uid.cmp(&b.uid));
        unreported.dedup_by(|a, b| a.uid == b.uid);
        report.root_causes.extend(unreported.into_iter().cloned());
        report.targets.extend(batch.into_iter().map(str::to_string));
        // Sequential terminal evaluations recompute shared nodes; retain their latest result.
        for (uid, result) in results {
            if matches!(&result, Err(error) if error.kind == ErrorKind::Cancelled) {
                report.results.entry(uid).or_insert(result);
            } else {
                report.results.insert(uid, result);
            }
        }
    }
    report.cancelled = ctx.is_cancelled();
    report.elapsed = started.elapsed();
    report
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::job::{JobEvent, JobManager, tests::wait_for_job};
    use dms_stream::{Diagram, Link, NodeRef, StreamNode};
    use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};

    pub(crate) fn node(uid: &str, class: &str) -> StreamNode {
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

    pub(crate) fn stream(nodes: Vec<StreamNode>, links: &[(&str, &str)], order: &[&str]) -> Stream {
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
                terminal_order: order
                    .iter()
                    .map(|uid| NodeRef::Uid((*uid).into()))
                    .collect(),
            },
        }
    }

    pub(crate) fn mystery_stream() -> Stream {
        stream(
            vec![node("mystery", "Mystery"), node("out", "TableNode")],
            &[("mystery", "out")],
            &[],
        )
    }

    fn element(name: &str, children: Vec<Node>) -> Element {
        Element {
            name: QualifiedName {
                prefix: None,
                namespace: None,
                local: Arc::from(name),
            },
            namespace_declarations: Vec::new(),
            attributes: Vec::new(),
            children,
        }
    }

    fn e(name: &str, children: Vec<Node>) -> Node {
        Node::Element(element(name, children))
    }

    fn p(name: &str, text: &str) -> Node {
        e(name, vec![Node::Text(Arc::from(text))])
    }

    fn request(stream: Stream, targets: RunTargets) -> RunRequest {
        RunRequest {
            stream: Arc::new(stream),
            attachments: Arc::new(HashMap::new()),
            data_dirs: Vec::new(),
            export_dir: std::env::temp_dir().join("dms-engine-run-tests"),
            targets,
        }
    }

    fn run(request: RunRequest) -> (RunReport, Vec<Progress>) {
        let mut jobs = JobManager::new(None);
        jobs.spawn(move |ctx| run_stream(request, ctx));
        let mut progress = Vec::new();
        for event in wait_for_job(&mut jobs) {
            match event {
                JobEvent::Progress {
                    progress: update, ..
                } => progress.push(update),
                JobEvent::Finished { output, .. } => return (output, progress),
                other => panic!("unexpected job event: {other:?}"),
            }
        }
        panic!("no report");
    }

    #[test]
    fn unsupported_root_is_attributed_and_downstream_only_reports_upstream() {
        let (report, progress) = run(request(mystery_stream(), RunTargets::Stream));
        assert_eq!(report.targets, ["out"]);
        assert_eq!(report.results.len(), 2);
        assert!(
            matches!(&report.results["mystery"], Err(NodeError { kind: ErrorKind::Unsupported(name), .. }) if name == "Mystery")
        );
        assert!(
            matches!(&report.results["out"], Err(NodeError { kind: ErrorKind::Upstream(uid), .. }) if uid == "mystery")
        );
        assert_eq!(report.root_causes.len(), 1);
        assert_eq!(report.root_causes[0].uid, "mystery");
        assert_eq!(
            progress,
            [
                Progress {
                    done: 0,
                    total: 2,
                    current: Some("mystery".into())
                },
                Progress {
                    done: 1,
                    total: 2,
                    current: None
                },
                Progress {
                    done: 1,
                    total: 2,
                    current: Some("out".into())
                },
                Progress {
                    done: 2,
                    total: 2,
                    current: None
                },
            ]
        );
    }

    #[test]
    fn stream_runs_skip_unconnected_nodes_unless_saved_as_terminals() {
        let s = stream(
            vec![
                node("mystery", "Mystery"),
                node("out", "TableNode"),
                node("lonely", "TableNode"),
            ],
            &[("mystery", "out")],
            &[],
        );
        let (report, _) = run(request(s.clone(), RunTargets::Stream));
        assert_eq!(report.targets, ["out"]);
        let mut saved = s;
        saved
            .diagram
            .terminal_order
            .push(NodeRef::Uid("lonely".into()));
        let (report, _) = run(request(saved, RunTargets::Stream));
        assert_eq!(report.targets, ["lonely", "out"]);
    }

    #[test]
    fn selected_nodes_keep_order_and_ignore_unknown_uids() {
        let (report, _) = run(request(
            mystery_stream(),
            RunTargets::Nodes(vec!["unknown".into(), "mystery".into()]),
        ));
        assert_eq!(report.targets, ["unknown", "mystery"]);
        assert_eq!(report.results.len(), 1);
        assert!(!report.results.contains_key("unknown"));
    }

    #[test]
    fn roots_list_run_failures_in_order_then_cycles() {
        let mut alias = node("alias", "MutableSuperNode");
        alias.subdiagram = Some(
            stream(
                vec![
                    node("inner", "Mystery"),
                    node("connector", "SuperNodeOutputConnectorNode"),
                ],
                &[("inner", "connector")],
                &[],
            )
            .diagram,
        );
        let stream = stream(
            vec![node("a", "TableNode"), node("b", "TableNode"), alias],
            &[("a", "b"), ("b", "a")],
            &[],
        );
        let (report, _) = run(request(
            stream,
            RunTargets::Nodes(vec!["a".into(), "alias".into()]),
        ));
        assert_eq!(
            report
                .root_causes
                .iter()
                .map(|e| e.uid.as_str())
                .collect::<Vec<_>>(),
            // Run failures in evaluation order, then errors set without running (cycles).
            ["inner", "a"]
        );
        assert!(matches!(report.root_causes[1].kind, ErrorKind::Cycle));
        assert!(report.results.contains_key("alias"));
    }

    #[test]
    fn successful_alias_targets_keep_their_failed_model_builder_as_a_root() {
        let mut source = node("source", "UserInputNode");
        source.settings = Some(element(
            "properties",
            vec![e(
                "userFieldTable",
                vec![e(
                    "fields",
                    vec![e(
                        "field",
                        vec![p("name", "x"), p("storage", "integer"), p("values", "1")],
                    )],
                )],
            )],
        ));
        let mut output = node("output", "TableNode");
        output.settings = Some(element("properties", vec![p("builderNodeId", "builder")]));
        let mut alias = node("alias", "MutableSuperNode");
        alias.subdiagram = Some(
            stream(
                vec![
                    source,
                    output,
                    node("connector", "SuperNodeOutputConnectorNode"),
                ],
                &[("source", "output"), ("output", "connector")],
                &[],
            )
            .diagram,
        );
        let s = stream(
            vec![
                node("cycle", "TableNode"),
                alias,
                node("builder", "Mystery"),
                node("later", "Mystery"),
            ],
            &[("cycle", "cycle")],
            &[],
        );
        let (report, _) = run(request(
            s,
            RunTargets::Nodes(vec!["cycle".into(), "alias".into(), "later".into()]),
        ));
        assert!(report.results["alias"].is_ok());
        assert_eq!(
            report
                .root_causes
                .iter()
                .map(|e| e.uid.as_str())
                .collect::<Vec<_>>(),
            ["builder", "later", "cycle"]
        );
    }

    #[test]
    fn sequential_runs_accumulate_progress_and_deduplicate_roots() {
        let s = stream(
            vec![
                node("mystery", "Mystery"),
                node("globals", "SetGlobalsNode"),
                node("out", "TableNode"),
            ],
            &[("mystery", "globals"), ("mystery", "out")],
            &["out", "globals"],
        );
        let (report, progress) = run(request(s, RunTargets::Stream));
        assert_eq!(report.targets, ["out", "globals"]);
        assert_eq!(report.results.len(), 3);
        assert_eq!(report.root_causes.len(), 1);
        assert_eq!(
            progress.last(),
            Some(&Progress {
                done: 4,
                total: 4,
                current: None
            })
        );
        assert_eq!(
            progress
                .iter()
                .filter(|p| p.current.as_deref() == Some("mystery"))
                .count(),
            2
        );
    }

    #[test]
    fn sequential_runs_share_mutated_globals() {
        let mut source = node("source", "UserInputNode");
        source.settings = Some(element(
            "properties",
            vec![e(
                "userFieldTable",
                vec![e(
                    "fields",
                    vec![e(
                        "field",
                        vec![p("name", "x"), p("storage", "integer"), p("values", "1 3")],
                    )],
                )],
            )],
        ));
        let mut set = node("set", "SetGlobalsNode");
        set.settings = Some(element(
            "properties",
            vec![e(
                "setGlobalsTable",
                vec![e(
                    "rows",
                    vec![e("row", vec![p("name", "x"), p("ave", "true")])],
                )],
            )],
        ));
        let mut reader = node("reader", "DeriveNode");
        reader.settings = Some(element(
            "properties",
            vec![
                p("deriveSingleFieldName", "mean"),
                p("deriveFormulaExpression", "@GLOBAL_MEAN(x)"),
            ],
        ));
        let s = stream(
            vec![source, set, reader],
            &[("source", "set"), ("source", "reader")],
            &["set", "reader"],
        );
        let (report, _) = run(request(s, RunTargets::Stream));
        assert!(report.root_causes.is_empty(), "{:?}", report.root_causes);
        assert_eq!(report.targets, ["set", "reader"]);
        let table = report.results["reader"].as_ref().unwrap().data().unwrap();
        assert_eq!(table.row_count(), 2);
        assert_eq!(table.rows()[0].last().unwrap().as_f64(), Some(2.0));
    }

    #[test]
    fn bad_initial_globals_attach_an_error_to_each_target() {
        let mut stream = mystery_stream();
        stream.properties = Some(element(
            "properties",
            vec![e(
                "globalsTable",
                vec![e("rows", vec![e("row", Vec::new())])],
            )],
        ));
        let (report, progress) = run(request(
            stream,
            RunTargets::Nodes(vec!["mystery".into(), "out".into()]),
        ));
        assert_eq!(report.results.len(), 2);
        assert_eq!(
            report
                .root_causes
                .iter()
                .map(|e| e.uid.as_str())
                .collect::<Vec<_>>(),
            ["mystery", "out"]
        );
        assert!(
            report
                .root_causes
                .iter()
                .all(|e| matches!(e.kind, ErrorKind::Settings(_)))
        );
        assert!(progress.is_empty());
    }

    #[test]
    fn cancellation_stops_before_the_next_sequential_terminal() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Mutex, mpsc};
        let s = stream(
            vec![
                node("mystery", "Mystery"),
                node("globals", "SetGlobalsNode"),
                node("out", "TableNode"),
            ],
            &[("mystery", "globals"), ("mystery", "out")],
            &["globals", "out"],
        );
        let (ready, arrived) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let (report_sender, report_receiver) = mpsc::channel();
        let gate = Mutex::new(gate);
        let calls = AtomicUsize::new(0);
        let mut jobs = JobManager::new(Some(Arc::new(move || {
            // Two nodes' started/finished updates: pause at the first terminal's finish.
            if calls.fetch_add(1, Ordering::SeqCst) == 3 {
                ready.send(()).unwrap();
                gate.lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap();
            }
        })));
        let id = jobs.spawn(move |ctx| {
            report_sender
                .send(run_stream(request(s, RunTargets::Stream), ctx))
                .unwrap();
        });
        arrived.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(jobs.cancel(id));
        release.send(()).unwrap();
        let report = report_receiver
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert_eq!(report.targets, ["globals", "out"]);
        assert_eq!(report.results.len(), 3);
        assert!(matches!(&report.results["out"], Err(error) if error.kind == ErrorKind::Cancelled));
        assert!(report.cancelled);
        assert!(
            report
                .root_causes
                .iter()
                .all(|error| error.kind != ErrorKind::Cancelled)
        );
        assert!(matches!(jobs.poll().as_slice(), [JobEvent::Cancelled { job }] if *job == id));
    }
}
