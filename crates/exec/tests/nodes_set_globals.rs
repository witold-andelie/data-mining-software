//! Set Globals on synthetic tables, with explicit stream-run ordering and isolated state.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use dms_clem::{GlobalStatistic as Stat, GlobalValues};
use dms_exec::{Arity, ExecContext, Factory, NodeOutput, Operator, OperatorError, Plan, registry};
use dms_stream::{Diagram, Link, NodeRef, Stream, StreamNode};
use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
use dms_types::{DataModel, Field, MissingSpec, Storage, Table, Value};
use std::sync::Arc;

fn e(name: &str, children: Vec<Node>) -> Node {
    Node::Element(Element {
        name: QualifiedName {
            prefix: None,
            namespace: None,
            local: Arc::from(name),
        },
        namespace_declarations: Vec::new(),
        attributes: Vec::new(),
        children,
    })
}
fn p(name: &str, text: &str) -> Node {
    e(name, vec![Node::Text(Arc::from(text))])
}
fn node(uid: &str, class: &str, settings: Vec<Node>) -> StreamNode {
    let Node::Element(settings) = e("properties", settings) else {
        panic!("element")
    };
    StreamNode {
        uid: uid.into(),
        class: class.into(),
        extension: None,
        model_builder: None,
        name: None,
        label: None,
        position: None,
        properties_attachment: None,
        containers: Vec::new(),
        settings: Some(settings),
        binning_output: None,
        result_items: Vec::new(),
        subdiagram: None,
    }
}
fn config(clear: bool, fields: &[(&str, &[Stat])]) -> Vec<Node> {
    vec![
        p(
            "setGlobalsClearGlobals",
            if clear { "true" } else { "false" },
        ),
        e(
            "setGlobalsTable",
            vec![e(
                "rows",
                fields
                    .iter()
                    .map(|(name, stats)| {
                        let mut values = vec![p("name", name)];
                        values.extend(stats.iter().map(|s| p(s.setting(), "true")));
                        e("row", values)
                    })
                    .collect(),
            )],
        ),
    ]
}
fn table(storage: Storage, values: Vec<Value>) -> Table {
    Table::new(
        DataModel::new(vec![Field::new("x", storage)]).unwrap(),
        values.into_iter().map(|v| vec![v]).collect(),
    )
    .unwrap()
}
fn ints(values: &[i64]) -> Table {
    table(
        Storage::Integer,
        values.iter().copied().map(Value::Integer).collect(),
    )
}
fn set(
    globals: &mut GlobalValues,
    clear: bool,
    stats: &[Stat],
    input: Table,
) -> Result<NodeOutput, OperatorError> {
    dms_exec::nodes::set_globals::build(&node(
        "g",
        "SetGlobalsNode",
        config(clear, &[("x", stats)]),
    ))?
    .produce_with_globals(&[Arc::new(input)], None, &ExecContext::default(), globals)
}
fn value(globals: &GlobalValues, stat: Stat) -> Value {
    globals.get("x", stat).unwrap().clone()
}
fn near(actual: Value, expected: f64) {
    assert!(
        (actual.as_f64().unwrap() - expected).abs() < 1e-12,
        "{actual:?} != {expected}"
    );
}

#[test]
fn numeric_statistics_count_duplicates_and_preserve_integer_storage() {
    let mut globals = GlobalValues::default();
    set(&mut globals, true, &Stat::ALL, ints(&[-4, -2, -1, 0, 0, 7])).unwrap();
    assert_eq!(value(&globals, Stat::Sum), Value::Integer(0));
    assert_eq!(value(&globals, Stat::Min), Value::Integer(-4));
    assert_eq!(value(&globals, Stat::Max), Value::Integer(7));
    assert_eq!(value(&globals, Stat::Mean), Value::Real(0.0));
    near(value(&globals, Stat::Sdev), (70.0f64 / 5.0).sqrt());
}

#[test]
fn nulls_and_user_missing_values_are_excluded() {
    let mut field = Field::new("x", Storage::Real);
    field.missing = MissingSpec {
        values: vec![Value::Real(0.0)],
        ..MissingSpec::default()
    };
    let input = Table::new(
        DataModel::new(vec![field]).unwrap(),
        [
            Value::Null,
            Value::Real(0.0),
            Value::Real(-4.0),
            Value::Real(-2.5),
        ]
        .into_iter()
        .map(|v| vec![v])
        .collect(),
    )
    .unwrap();
    let mut globals = GlobalValues::default();
    set(&mut globals, true, &Stat::ALL, input).unwrap();
    assert_eq!(value(&globals, Stat::Sum), Value::Real(-6.5));
    assert_eq!(value(&globals, Stat::Mean), Value::Real(-3.25));
    near(value(&globals, Stat::Sdev), 1.060660171779821);
}

#[test]
fn constants_singletons_and_all_null_follow_oracle_conventions() {
    let mut globals = GlobalValues::default();
    set(&mut globals, true, &Stat::ALL, ints(&[7, 7, 7])).unwrap();
    assert_eq!(value(&globals, Stat::Sdev), Value::Real(0.0));
    set(&mut globals, true, &Stat::ALL, ints(&[7])).unwrap();
    assert_eq!(value(&globals, Stat::Sum), Value::Integer(7));
    assert_eq!(value(&globals, Stat::Mean), Value::Real(7.0));
    assert!(globals.get("x", Stat::Sdev).is_err());
    set(
        &mut globals,
        true,
        &Stat::ALL,
        table(Storage::Real, vec![Value::Null, Value::Null]),
    )
    .unwrap();
    for stat in Stat::ALL {
        assert!(globals.get("x", stat).is_err());
    }
}

#[test]
fn extrema_keep_empty_strings_spaces_dates_and_timestamps() {
    for (storage, values, min, max) in [
        (
            Storage::String,
            vec![
                Value::Null,
                Value::string(" "),
                Value::string("zebra"),
                Value::string(""),
                Value::string("Apple"),
            ],
            Value::string(""),
            Value::string("zebra"),
        ),
        (
            Storage::Date,
            vec![Value::Date(3), Value::Null, Value::Date(-2)],
            Value::Date(-2),
            Value::Date(3),
        ),
        (
            Storage::Timestamp,
            vec![
                Value::Timestamp(2_000_001),
                Value::Timestamp(-1),
                Value::Null,
            ],
            Value::Timestamp(-1),
            Value::Timestamp(2_000_001),
        ),
    ] {
        let mut globals = GlobalValues::default();
        set(
            &mut globals,
            true,
            &[Stat::Min, Stat::Max],
            table(storage, values),
        )
        .unwrap();
        assert_eq!(value(&globals, Stat::Min), min);
        assert_eq!(value(&globals, Stat::Max), max);
    }
}

#[test]
fn retain_replaces_only_selected_keys_and_clear_removes_everything() {
    let mut globals = GlobalValues::default();
    globals.set("other", Stat::Sum, Value::Integer(55)).unwrap();
    set(&mut globals, false, &Stat::ALL, ints(&[1, 2])).unwrap();
    set(
        &mut globals,
        false,
        &[Stat::Sum, Stat::Mean],
        ints(&[10, 20]),
    )
    .unwrap();
    assert_eq!(value(&globals, Stat::Sum), Value::Integer(30));
    assert_eq!(value(&globals, Stat::Mean), Value::Real(15.0));
    assert_eq!(value(&globals, Stat::Max), Value::Integer(2));
    assert_eq!(
        globals.get("other", Stat::Sum).unwrap(),
        &Value::Integer(55)
    );
    set(&mut globals, true, &[Stat::Sum], ints(&[5])).unwrap();
    assert_eq!(value(&globals, Stat::Sum), Value::Integer(5));
    assert!(globals.get("other", Stat::Sum).is_err());
    assert!(globals.get("x", Stat::Mean).is_err());
}

#[test]
fn errors_are_transactional_and_unsettled_behaviors_are_rejected() {
    let mut globals = GlobalValues::default();
    set(&mut globals, true, &Stat::ALL, ints(&[1, 2])).unwrap();
    let before = globals.clone();
    for input in [
        ints(&[]),
        ints(&[i64::MAX, 1]),
        table(Storage::String, vec![Value::string("abc")]),
        table(Storage::Real, vec![Value::Real(f64::INFINITY)]),
    ] {
        assert!(set(&mut globals, true, &Stat::ALL, input).is_err());
        assert_eq!(globals, before);
    }
    assert!(set(&mut globals, false, &[Stat::Sdev], ints(&[1])).is_err());
    assert_eq!(globals, before);
    let bad = dms_exec::nodes::set_globals::build(&node(
        "bad",
        "SetGlobalsNode",
        config(true, &[("missing", &[Stat::Sum])]),
    ))
    .unwrap();
    assert!(
        bad.produce_with_globals(
            &[Arc::new(ints(&[1]))],
            None,
            &ExecContext::default(),
            &mut globals
        )
        .is_err()
    );
    assert_eq!(globals, before);
}

#[test]
fn unknown_settings_statistics_and_invalid_flags_fail() {
    for settings in [
        vec![p("setGlobalsClearGlobals", "yes")],
        vec![p("setGlobalsShowResults", "yes")],
        vec![p("setGlobalsUnsupported", "true")],
        vec![p("doReadValues", "true")],
        vec![e(
            "setGlobalsDefaultOperations",
            vec![p("setElement", "median")],
        )],
        vec![e(
            "setGlobalsTable",
            vec![e(
                "rows",
                vec![e("row", vec![p("name", "x"), p("median", "true")])],
            )],
        )],
        config(true, &[("x", &[Stat::Sum]), ("x", &[Stat::Mean])]),
    ] {
        assert!(
            dms_exec::nodes::set_globals::build(&node("g", "SetGlobalsNode", settings)).is_err()
        );
    }
    assert!(registry::factory("set_globals").is_some());
}

struct Source;
impl Operator for Source {
    fn arity(&self) -> Arity {
        Arity::Exactly(0)
    }
    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Ok(ints(&[1, 2, 3]))
    }
}
fn lookup(id: &str) -> Option<Factory> {
    if id == "var_file" {
        Some(|_| Ok(Box::new(Source)))
    } else {
        registry::factory(id)
    }
}
fn link(from: &str, to: &str) -> Link {
    Link {
        from: NodeRef::Uid(from.into()),
        to: NodeRef::Uid(to.into()),
    }
}
fn synthetic_stream() -> Stream {
    Stream {
        file_version: None,
        save_version: None,
        properties: None,
        diagram: Diagram {
            nodes: vec![
                node("source", "DelimitedFileImportNode", Vec::new()),
                node(
                    "reader",
                    "DeriveNode",
                    vec![
                        p("deriveSingleFieldName", "g"),
                        p("deriveFormulaExpression", "@GLOBAL_SUM(x)"),
                    ],
                ),
                node(
                    "prime",
                    "SetGlobalsNode",
                    config(true, &[("x", &Stat::ALL)]),
                ),
            ],
            links: vec![link("source", "prime"), link("source", "reader")],
            terminal_order: vec![NodeRef::Uid("prime".into()), NodeRef::Uid("reader".into())],
        },
    }
}

#[test]
fn explicit_runs_share_globals_but_fresh_executions_are_isolated() {
    let plan = Plan::with_factories(&synthetic_stream(), lookup);
    let context = ExecContext::default();
    let mut execution = plan.execution().unwrap();
    assert!(execution.evaluate(&["reader"], &context)["reader"].is_err());
    assert!(execution.evaluate(&["prime"], &context)["prime"].is_ok());
    let results = execution.evaluate(&["reader"], &context);
    let reader = results["reader"].as_ref().unwrap().data().unwrap();
    assert_eq!(
        reader.column("g").unwrap().cloned().collect::<Vec<_>>(),
        vec![Value::Integer(6); 3]
    );
    assert_eq!(
        execution.globals().get("x", Stat::Sum).unwrap(),
        &Value::Integer(6)
    );
    assert!(plan.evaluate(&["reader"], &context)["reader"].is_err());
    assert_eq!(plan.terminals_in_run_order(), vec!["prime", "reader"]);
}

#[test]
fn multiple_targets_execute_in_the_requested_order() {
    let plan = Plan::with_factories(&synthetic_stream(), lookup);
    let context = ExecContext::default();
    assert!(plan.evaluate(&["prime", "reader"], &context)["reader"].is_ok());
    assert!(plan.evaluate(&["reader", "prime"], &context)["reader"].is_err());
}

#[test]
fn reader_recomputes_after_another_global_update() {
    let mut stream = synthetic_stream();
    stream.diagram.nodes.push(node(
        "small",
        "SelectNode",
        vec![p("selectCondition", "x < 3")],
    ));
    stream.diagram.nodes.push(node(
        "replace",
        "SetGlobalsNode",
        config(true, &[("x", &[Stat::Sum])]),
    ));
    stream
        .diagram
        .links
        .extend([link("source", "small"), link("small", "replace")]);
    let plan = Plan::with_factories(&stream, lookup);
    let mut execution = plan.execution().unwrap();
    let context = ExecContext::default();
    execution.evaluate(&["prime"], &context);
    let before = execution.evaluate(&["reader"], &context);
    execution.evaluate(&["replace"], &context);
    let after = execution.evaluate(&["reader"], &context);
    assert_eq!(
        before["reader"].as_ref().unwrap().data().unwrap().rows()[0][1],
        Value::Integer(6)
    );
    assert_eq!(
        after["reader"].as_ref().unwrap().data().unwrap().rows()[0][1],
        Value::Integer(3)
    );
}

#[test]
fn saved_globals_seed_each_execution_independently() {
    let mut stream = synthetic_stream();
    let mut value = match p("value", "12") {
        Node::Element(e) => e,
        _ => panic!("element"),
    };
    value.attributes.push(dms_strfmt::fastinfoset::Attribute {
        name: QualifiedName {
            prefix: None,
            namespace: None,
            local: Arc::from("type"),
        },
        value: Arc::from("J"),
    });
    let Node::Element(properties) = e(
        "properties",
        vec![e(
            "globalsTable",
            vec![e(
                "rows",
                vec![e(
                    "row",
                    vec![p("name", "x"), e("sum", vec![Node::Element(value)])],
                )],
            )],
        )],
    ) else {
        panic!("element")
    };
    stream.properties = Some(properties);
    let plan = Plan::with_factories(&stream, lookup);
    let context = ExecContext::default();
    let mut first = plan.execution().unwrap();
    first.evaluate(&["prime"], &context);
    assert_eq!(
        first.globals().get("x", Stat::Sum).unwrap(),
        &Value::Integer(6)
    );
    assert_eq!(
        plan.execution()
            .unwrap()
            .globals()
            .get("x", Stat::Sum)
            .unwrap(),
        &Value::Integer(12)
    );
    assert_eq!(
        plan.evaluate(&["reader"], &context)["reader"]
            .as_ref()
            .unwrap()
            .data()
            .unwrap()
            .rows()[0][1],
        Value::Integer(12)
    );
}

#[test]
fn every_clem_node_reads_the_stream_globals() {
    let input = Arc::new(ints(&[1, 2, 3]));
    let mut globals = GlobalValues::default();
    set(&mut globals, true, &Stat::ALL, ints(&[1, 2, 3])).unwrap();
    let context = ExecContext::default();
    let select = dms_exec::nodes::select::build(&node(
        "s",
        "SelectNode",
        vec![p("selectCondition", "x > @GLOBAL_MEAN(x)")],
    ))
    .unwrap();
    assert_eq!(
        select
            .produce_with_globals(&[Arc::clone(&input)], None, &context, &mut globals)
            .unwrap()
            .data()
            .unwrap()
            .rows(),
        &[vec![Value::Integer(3)]]
    );
    let filler = dms_exec::nodes::filler::build(&node(
        "f",
        "FillerNode",
        vec![
            e("fillerSelectedFields", vec![p("field", "x")]),
            p("fillerReplaceMode", "conditional"),
            p("fillerCondition", "x < @GLOBAL_MAX(x)"),
            p("fillerReplaceExpression", "@GLOBAL_SUM(@FIELD)"),
        ],
    ))
    .unwrap();
    assert_eq!(
        filler
            .produce_with_globals(&[Arc::clone(&input)], None, &context, &mut globals)
            .unwrap()
            .data()
            .unwrap()
            .rows(),
        &[
            vec![Value::Integer(6)],
            vec![Value::Integer(6)],
            vec![Value::Integer(3)]
        ]
    );
    let balance = dms_exec::nodes::balance::build(&node(
        "b",
        "BalanceNode",
        vec![e(
            "balanceDirectives",
            vec![e(
                "entries",
                vec![e(
                    "entry",
                    vec![p("condition", "x > @GLOBAL_MEAN(x)"), p("factor", "2")],
                )],
            )],
        )],
    ))
    .unwrap();
    assert_eq!(
        balance
            .produce_with_globals(&[Arc::clone(&input)], None, &context, &mut globals)
            .unwrap()
            .data()
            .unwrap()
            .row_count(),
        4
    );
    let merge = dms_exec::nodes::merge::build(&node(
        "m",
        "MergeNode",
        vec![
            p("mergeMode", "condition"),
            p("mergeCondition", "x > @GLOBAL_MEAN(x)"),
        ],
    ))
    .unwrap();
    assert_eq!(
        merge
            .produce_with_globals(&[Arc::clone(&input)], None, &context, &mut globals)
            .unwrap()
            .data()
            .unwrap()
            .rows(),
        &[vec![Value::Integer(3)]]
    );
    let report = dms_exec::nodes::report::build(&node(
        "r",
        "ReportNode",
        vec![
            p(
                "reportText",
                "#WHERE x > @GLOBAL_MEAN(x)\n[@GLOBAL_SUM(x)]\n#",
            ),
            p("reportHighlightInserted", "false"),
        ],
    ))
    .unwrap();
    let result = report
        .produce_with_globals(&[input], None, &context, &mut globals)
        .unwrap();
    assert_eq!(
        result.document().unwrap().tables[0].rows,
        vec![vec!["6".to_string()]]
    );
}

#[cfg(feature = "corpus")]
mod corpus {
    use super::*;
    use dms_exec::{Document, DocumentTable, FileResolver, format_real};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    #[path = "../support/mod.rs"]
    mod support;

    fn document(table: &Table, null_text: &str) -> Document {
        Document {
            title: String::new(),
            tables: vec![DocumentTable {
                title: None,
                header: table
                    .model()
                    .fields()
                    .iter()
                    .map(|f| f.name.clone())
                    .collect(),
                rows: table
                    .rows()
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|v| match v {
                                Value::Real(r) => format_real(*r, 15),
                                Value::Null => null_text.to_string(),
                                value => value.to_string(),
                            })
                            .collect()
                    })
                    .collect(),
            }],
        }
    }

    fn compare(table: &Table, path: PathBuf, null_text: &str) {
        let expected = support::parse_csv_document(&std::fs::read_to_string(&path).unwrap());
        let actual = document(table, null_text);
        // The Table DocumentOutput CSV does not quote embedded commas (misc_main's
        // has,comma becomes two CSV cells). Compare its serialized dialect faithfully;
        // data/ exports, in contrast, use quoted Output File CSV.
        let actual = if null_text == "$null$" {
            let table = &actual.tables[0];
            let mut lines = vec![table.header.join(",")];
            lines.extend(table.rows.iter().map(|row| row.join(",")));
            support::parse_csv_document(&lines.join("\n"))
        } else {
            actual
        };
        let comparison = support::compare_documents(&actual, &expected);
        assert_eq!(
            comparison.matched,
            comparison.total,
            "{}: {:?}",
            path.display(),
            comparison.diffs
        );
        assert!(comparison.diffs.is_empty());
    }

    #[test]
    fn all_nine_recorded_run_orders_table_outputs_data_and_errors_match() {
        let root = support::repo_root();
        let mut runs_checked = 0;
        let mut outputs_checked = 0;
        let mut data_checked = 0;
        for case in [
            "default",
            "all_null",
            "clear",
            "constant",
            "empty",
            "missing",
            "retain",
            "singleton",
            "types",
        ] {
            let stem = format!("misc_gl_{case}");
            let oracle = root.join("oracle/misc").join(&stem);
            let manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(oracle.join("manifest.json")).unwrap())
                    .unwrap();
            let archive = dms_strfmt::read_archive(
                &std::fs::read(root.join("corpus/misc").join(format!("{stem}.str"))).unwrap(),
                &dms_strfmt::ArchiveLimits::default(),
            )
            .unwrap();
            let stream = Stream::from_document(&archive.document).unwrap();
            let context = ExecContext {
                files: FileResolver {
                    data_dirs: vec![root.join("corpus/misc")],
                },
                stream_properties: stream.properties.clone().map(Arc::new),
                ..ExecContext::default()
            };
            let plan = Plan::new(&stream);
            let mut execution = plan.execution().unwrap();
            let mut terminal_results = HashMap::new();
            for run in manifest["runs"].as_array().unwrap() {
                let uid = run["node"]["id"].as_str().unwrap();
                let results = execution.evaluate(&[uid], &context);
                let result = results[uid].clone();
                assert_eq!(
                    result.is_ok(),
                    run["ok"].as_bool().unwrap(),
                    "{stem}: {uid}: {result:?}"
                );
                terminal_results.insert(uid.to_string(), result);
                runs_checked += 1;
            }
            for output in manifest["outputs"].as_array().unwrap() {
                let uid = output["id"].as_str().unwrap();
                let path = oracle.join("outputs").join(
                    Path::new(output["file"].as_str().unwrap())
                        .file_name()
                        .unwrap(),
                );
                compare(
                    terminal_results[uid].as_ref().unwrap().data().unwrap(),
                    path,
                    "$null$",
                );
                outputs_checked += 1;
            }
            for entry in manifest["data"].as_array().unwrap() {
                let uid = entry["id"].as_str().unwrap();
                let results = execution.evaluate(&[uid], &context);
                let path = oracle.join("data").join(
                    Path::new(entry["file"].as_str().unwrap())
                        .file_name()
                        .unwrap(),
                );
                compare(results[uid].as_ref().unwrap().data().unwrap(), path, "");
                data_checked += 1;
            }
            for error in manifest["errors"].as_array().unwrap() {
                let uid = error["node"]["id"].as_str().unwrap();
                let result = if error["stage"] == "run_terminal" {
                    terminal_results[uid].clone()
                } else {
                    execution.evaluate(&[uid], &context)[uid].clone()
                };
                assert!(result.is_err(), "{stem}: expected {error:?}");
                // Upstream attribution is separate from the compiler's precise undefined-global
                // message; synthetic/CLEM tests verify that message directly.
            }
        }
        assert_eq!((runs_checked, outputs_checked, data_checked), (20, 7, 18));
        eprintln!(
            "Set Globals oracle: 20 runs, 7 Table outputs, 18 data exports, 4 expected errors match"
        );
    }
}
