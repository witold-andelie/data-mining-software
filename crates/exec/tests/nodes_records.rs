//! Record Operations nodes (Select, Sample, Sort, Aggregate, Append, Merge, Balance) on synthetic
//! tables.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_exec::nodes::{aggregate, append, balance, merge, sample, select, sort};
use dms_exec::{ExecContext, Factory, OperatorError};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Attribute, Element, Node, QualifiedName};
use dms_types::{DataModel, Field, Measurement, Role, Storage, Table, Value, ValueSpec};
use std::sync::Arc;

fn raw_element(name: &str, children: Vec<Node>) -> Element {
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

fn element(name: &str, children: Vec<Node>) -> Node {
    Node::Element(raw_element(name, children))
}

fn element_with_attr(name: &str, attr_name: &str, attr_val: &str, children: Vec<Node>) -> Node {
    Node::Element(Element {
        name: QualifiedName {
            prefix: None,
            namespace: None,
            local: Arc::from(name),
        },
        namespace_declarations: Vec::new(),
        attributes: vec![Attribute {
            name: QualifiedName {
                prefix: None,
                namespace: None,
                local: Arc::from(attr_name),
            },
            value: Arc::from(attr_val),
        }],
        children,
    })
}

fn p(name: &str, text: &str) -> Node {
    element(name, vec![Node::Text(Arc::from(text))])
}

fn g(name: &str, children: Vec<Node>) -> Node {
    element(name, children)
}

fn node(children: Vec<Node>) -> StreamNode {
    StreamNode {
        uid: "n1".into(),
        class: "x".into(),
        extension: None,
        model_builder: None,
        name: None,
        label: None,
        position: None,
        properties_attachment: None,
        containers: Vec::new(),
        settings: Some(raw_element("properties", children)),
        subdiagram: None,
    }
}

fn run(factory: Factory, settings: Vec<Node>, input: Table) -> Result<Table, OperatorError> {
    let operator = factory(&node(settings))?;
    operator.run(&[Arc::new(input)], &ExecContext::default())
}

fn run_many(
    factory: Factory,
    settings: Vec<Node>,
    inputs: Vec<Table>,
) -> Result<Table, OperatorError> {
    let operator = factory(&node(settings))?;
    let arcs: Vec<Arc<Table>> = inputs.into_iter().map(Arc::new).collect();
    operator.run(&arcs, &ExecContext::default())
}

fn col(table: &Table, name: &str) -> Vec<Value> {
    table.column(name).unwrap().cloned().collect()
}

fn sample_table() -> Table {
    let mut active = Field::new("active", Storage::String);
    active.measurement = Measurement::Flag;
    active.values = ValueSpec::Flag {
        true_value: Value::string("Y"),
        false_value: Value::string("N"),
    };
    let mut part = Field::new("Partition", Storage::String);
    part.role = Role::Partition;
    part.measurement = Measurement::Nominal;
    part.values = ValueSpec::Categories(vec![
        Value::string("1_Training"),
        Value::string("2_Testing"),
    ]);
    let fields = vec![
        Field::new("id", Storage::Integer),
        Field::new("age", Storage::Integer),
        active,
        part,
    ];
    let rows = vec![
        vec![
            Value::Integer(1),
            Value::Integer(20),
            Value::string("Y"),
            Value::string("1_Training"),
        ],
        vec![
            Value::Integer(2),
            Value::Integer(40),
            Value::string("N"),
            Value::string("2_Testing"),
        ],
        vec![
            Value::Integer(3),
            Value::Null,
            Value::Null,
            Value::string("1_Training"),
        ],
        vec![
            Value::Integer(4),
            Value::Integer(60),
            Value::string("Y"),
            Value::string("2_Testing"),
        ],
    ];
    Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
}

// ------------------------------------------------------------------ Select

#[test]
fn select_include_and_discard_drop_null_conditions_and_support_flag_fields() {
    // Include: age > 30 keeps id 2 (40) and id 4 (60); id 3 (null) is dropped.
    let included = run(
        select::build,
        vec![p("selectMode", "include"), p("selectCondition", "age > 30")],
        sample_table(),
    )
    .unwrap();
    assert_eq!(
        col(&included, "id"),
        vec![Value::Integer(2), Value::Integer(4)]
    );

    // Discard: age > 30 keeps id 1 (20); id 3 (null) is ALSO dropped (oracle rule 39).
    let discarded = run(
        select::build,
        vec![p("selectMode", "discard"), p("selectCondition", "age > 30")],
        sample_table(),
    )
    .unwrap();
    assert_eq!(col(&discarded, "id"), vec![Value::Integer(1)]);

    // Bare flag field as condition (oracle rule 33): null flag is dropped in both modes.
    let flag_in = run(
        select::build,
        vec![p("selectCondition", "active")],
        sample_table(),
    )
    .unwrap();
    assert_eq!(
        col(&flag_in, "id"),
        vec![Value::Integer(1), Value::Integer(4)]
    );

    let flag_out = run(
        select::build,
        vec![p("selectMode", "discard"), p("selectCondition", "active")],
        sample_table(),
    )
    .unwrap();
    assert_eq!(col(&flag_out, "id"), vec![Value::Integer(2)]);
}

#[test]
fn select_handles_empty_input_and_rejects_bad_settings() {
    let empty = Table::new(sample_table().model().clone(), Vec::new()).unwrap();
    let out = run(select::build, vec![p("selectCondition", "age > 0")], empty).unwrap();
    assert_eq!(out.row_count(), 0);

    assert!(select::build(&node(vec![])).is_err());
    assert!(select::build(&node(vec![p("selectCondition", "   ")])).is_err());
    assert!(
        select::build(&node(vec![
            p("selectMode", "bogus"),
            p("selectCondition", "age > 0"),
        ]))
        .is_err()
    );
    // Non-boolean expression in selectCondition fails at run time.
    assert!(
        run(
            select::build,
            vec![p("selectCondition", "age + 1")],
            sample_table(),
        )
        .is_err()
    );
}

// ------------------------------------------------------------------ Sample

#[test]
fn sample_first_n_one_in_n_max_size_and_discard_mode() {
    // First 2 records (pass mode).
    let first2 = run(
        sample::build,
        vec![p("sampleMethod", "firstN"), p("sampleFirstN", "2")],
        sample_table(),
    )
    .unwrap();
    assert_eq!(
        col(&first2, "id"),
        vec![Value::Integer(1), Value::Integer(2)]
    );

    // First 2 records in discard mode -> keeps records 3 and 4.
    let discard_first2 = run(
        sample::build,
        vec![
            p("sampleMode", "discard"),
            p("sampleMethod", "firstN"),
            p("sampleFirstN", "2"),
        ],
        sample_table(),
    )
    .unwrap();
    assert_eq!(
        col(&discard_first2, "id"),
        vec![Value::Integer(3), Value::Integer(4)]
    );

    // 1-in-2 keeps indices 0 and 2 -> ids 1 and 3; with max_size=1 keeps only id 1.
    let one_in_2 = run(
        sample::build,
        vec![p("sampleMethod", "oneInN"), p("sampleOneInN", "2")],
        sample_table(),
    )
    .unwrap();
    assert_eq!(
        col(&one_in_2, "id"),
        vec![Value::Integer(1), Value::Integer(3)]
    );

    let capped = run(
        sample::build,
        vec![
            p("sampleMethod", "oneInN"),
            p("sampleOneInN", "2"),
            p("sampleUseMaxSize", "true"),
            p("sampleMaxSize", "1"),
        ],
        sample_table(),
    )
    .unwrap();
    assert_eq!(col(&capped, "id"), vec![Value::Integer(1)]);
}

#[test]
fn sample_random_is_deterministic_and_rejects_bad_settings() {
    let settings = vec![
        p("sampleMethod", "random"),
        p("sampleRandom", "50.0"),
        p("sampleUseRandomSeed", "true"),
        p("sampleRandomSeed", "42"),
    ];
    let a = run(sample::build, settings.clone(), sample_table()).unwrap();
    let b = run(sample::build, settings, sample_table()).unwrap();
    assert_eq!(col(&a, "id"), col(&b, "id"));

    // 0% and 100% bounds.
    let zero = run(
        sample::build,
        vec![p("sampleMethod", "random"), p("sampleRandom", "0.0")],
        sample_table(),
    )
    .unwrap();
    assert_eq!(zero.row_count(), 0);

    let all = run(
        sample::build,
        vec![p("sampleMethod", "random"), p("sampleRandom", "100.0")],
        sample_table(),
    )
    .unwrap();
    assert_eq!(all.row_count(), 4);

    assert!(sample::build(&node(vec![p("sampleProcedure", "complex")])).is_err());
    assert!(sample::build(&node(vec![p("sampleMode", "other")])).is_err());
    assert!(sample::build(&node(vec![p("sampleMethod", "other")])).is_err());
    assert!(
        sample::build(&node(vec![
            p("sampleMethod", "oneInN"),
            p("sampleOneInN", "0"),
        ]))
        .is_err()
    );
    assert!(
        sample::build(&node(vec![
            p("sampleMethod", "random"),
            p("sampleRandom", "120.0"),
        ]))
        .is_err()
    );
}

// ------------------------------------------------------------------ Sort

#[test]
fn sort_orders_by_multiple_keys_and_places_nulls_first_in_ascending() {
    let fields = vec![
        Field::new("group", Storage::String),
        Field::new("val", Storage::Integer),
        Field::new("seq", Storage::Integer),
    ];
    let rows = vec![
        vec![Value::string("B"), Value::Integer(10), Value::Integer(1)],
        vec![Value::string("A"), Value::Integer(20), Value::Integer(2)],
        vec![Value::string("A"), Value::Null, Value::Integer(3)],
        vec![Value::string("A"), Value::Integer(20), Value::Integer(4)],
    ];
    let table = Table::new(DataModel::new(fields).unwrap(), rows).unwrap();

    // group Ascending (0), val Descending (true); ties preserve input order (seq 2 before 4).
    let sorted = run(
        sort::build,
        vec![g(
            "sortKeys",
            vec![g(
                "rows",
                vec![
                    g("row", vec![p("field", "group"), p("direction", "0")]),
                    g("row", vec![p("field", "val"), p("direction", "Descending")]),
                ],
            )],
        )],
        table,
    )
    .unwrap();
    assert_eq!(
        col(&sorted, "seq"),
        vec![
            Value::Integer(2),
            Value::Integer(4),
            Value::Integer(3),
            Value::Integer(1),
        ]
    );
}

#[test]
fn sort_rejects_bad_settings_and_missing_fields() {
    assert!(
        sort::build(&node(vec![g(
            "sortKeys",
            vec![g("rows", vec![g("row", vec![p("direction", "0")])])],
        )]))
        .is_err()
    );
    assert!(
        sort::build(&node(vec![g(
            "sortKeys",
            vec![g(
                "rows",
                vec![g(
                    "row",
                    vec![p("field", "age"), p("direction", "sideways")]
                )]
            )],
        )]))
        .is_err()
    );
    assert!(
        run(
            sort::build,
            vec![g(
                "sortKeys",
                vec![g("rows", vec![g("row", vec![p("field", "no_such_field")])])],
            )],
            sample_table(),
        )
        .is_err()
    );
}

// ------------------------------------------------------------------ Aggregate

#[test]
fn aggregate_groups_by_keys_computes_statistics_and_respects_contiguous_mode() {
    let fields = vec![
        Field::new("k", Storage::String),
        Field::new("x", Storage::Integer),
    ];
    let rows = vec![
        vec![Value::string("B"), Value::Integer(10)],
        vec![Value::string("A"), Value::Integer(2)],
        vec![Value::string("A"), Value::Integer(4)],
        vec![Value::string("A"), Value::Null],
        vec![Value::string("B"), Value::Integer(20)],
    ];
    let table = Table::new(DataModel::new(fields).unwrap(), rows).unwrap();

    let agg_row = g(
        "row",
        vec![
            p("field", "x"),
            p("sum", "true"),
            p("mean", "true"),
            p("min", "true"),
            p("max", "true"),
            p("stddev", "true"),
            p("median", "true"),
            p("count", "true"),
            p("variance", "true"),
            p("firstQuartile", "true"),
            p("thirdQuartile", "true"),
        ],
    );
    let out = run(
        aggregate::build,
        vec![
            g("aggregateKeyFields", vec![p("field", "k")]),
            g("aggregateFields", vec![g("rows", vec![agg_row.clone()])]),
            p("aggregateCountFieldName", "No_records"),
        ],
        table.clone(),
    )
    .unwrap();

    // Non-contiguous: groups are sorted by key ("A", then "B").
    assert_eq!(col(&out, "k"), vec![Value::string("A"), Value::string("B")]);
    assert_eq!(
        col(&out, "x_Sum"),
        vec![Value::Integer(6), Value::Integer(30)]
    );
    assert_eq!(
        col(&out, "x_Mean"),
        vec![Value::Real(3.0), Value::Real(15.0)]
    );
    assert_eq!(
        col(&out, "x_Min"),
        vec![Value::Integer(2), Value::Integer(10)]
    );
    assert_eq!(
        col(&out, "x_Max"),
        vec![Value::Integer(4), Value::Integer(20)]
    );
    assert_eq!(
        col(&out, "x_Count"),
        vec![Value::Integer(2), Value::Integer(2)]
    );
    assert_eq!(
        col(&out, "x_Var"),
        vec![Value::Real(2.0), Value::Real(50.0)]
    );
    assert_eq!(
        col(&out, "x_Median"),
        vec![Value::Real(3.0), Value::Real(15.0)]
    );
    // Group A has 3 records (one with null x); group B has 2 records.
    assert_eq!(
        col(&out, "No_records"),
        vec![Value::Integer(3), Value::Integer(2)]
    );

    // Contiguous keys: B, A, B stay 3 separate groups in stream order.
    let contig = run(
        aggregate::build,
        vec![
            g("aggregateKeyFields", vec![p("field", "k")]),
            p("aggregateContiguousKeys", "true"),
            p("aggregateIncludeCount", "false"),
            p("aggregateNameExtension", "agg_"),
            p("aggregateExtensionPosition", "Prefix"),
            g(
                "aggregateFields",
                vec![g(
                    "rows",
                    vec![g("row", vec![p("field", "x"), p("sum", "true")])],
                )],
            ),
        ],
        table,
    )
    .unwrap();
    assert_eq!(
        col(&contig, "k"),
        vec![Value::string("B"), Value::string("A"), Value::string("B")]
    );
    assert_eq!(
        col(&contig, "agg_x_Sum"),
        vec![Value::Integer(10), Value::Integer(6), Value::Integer(20)]
    );
    assert!(contig.model().field("Record_Count").is_none());
}

#[test]
fn aggregate_handles_empty_input_and_rejects_unsupported_settings() {
    let empty = Table::new(
        DataModel::new(vec![Field::new("x", Storage::Integer)]).unwrap(),
        Vec::new(),
    )
    .unwrap();
    // Without key fields, an empty input emits one summary row with count 0 and null stats.
    let no_keys = run(
        aggregate::build,
        vec![g(
            "aggregateFields",
            vec![g(
                "rows",
                vec![g("row", vec![p("field", "x"), p("sum", "true")])],
            )],
        )],
        empty.clone(),
    )
    .unwrap();
    assert_eq!(col(&no_keys, "x_Sum"), vec![Value::Null]);
    assert_eq!(col(&no_keys, "Record_Count"), vec![Value::Integer(0)]);

    // With key fields, an empty input emits 0 rows.
    let with_keys = run(
        aggregate::build,
        vec![g("aggregateKeyFields", vec![p("field", "x")])],
        empty,
    )
    .unwrap();
    assert_eq!(with_keys.row_count(), 0);

    assert!(aggregate::build(&node(vec![p("aggregateAllowApproximation", "true")])).is_err());
    assert!(
        aggregate::build(&node(vec![g(
            "aggregateExpressionsTable",
            vec![g(
                "rows",
                vec![g("row", vec![p("field", "m"), p("expression", "MAX('x')")])],
            )],
        )]))
        .is_err()
    );
}

// ------------------------------------------------------------------ Append

#[test]
fn append_matches_by_name_case_position_and_adds_tag_field() {
    let t1 = Table::new(
        DataModel::new(vec![
            Field::new("ID", Storage::Integer),
            Field::new("Score", Storage::Integer),
        ])
        .unwrap(),
        vec![vec![Value::Integer(1), Value::Integer(10)]],
    )
    .unwrap();
    let t2 = Table::new(
        DataModel::new(vec![
            Field::new("score", Storage::Real),
            Field::new("Extra", Storage::String),
        ])
        .unwrap(),
        vec![vec![Value::Real(20.5), Value::string("hi")]],
    )
    .unwrap();

    // Default: match by name, case-insensitive, main fields only, no tag.
    let main_only = run_many(append::build, vec![], vec![t1.clone(), t2.clone()]).unwrap();
    assert_eq!(main_only.model().len(), 2);
    assert_eq!(col(&main_only, "ID"), vec![Value::Integer(1), Value::Null]);
    // Integer + Real promotes to Real.
    assert_eq!(
        col(&main_only, "Score"),
        vec![Value::Real(10.0), Value::Real(20.5)]
    );

    // Include all fields + case-sensitive + tag field.
    let all_case = run_many(
        append::build,
        vec![
            p("appendMatchCase", "true"),
            p("appendIncludeAllFields", "true"),
            p("appendIncludeTag", "true"),
            p("appendTagField", "Src"),
        ],
        vec![t1.clone(), t2.clone()],
    )
    .unwrap();
    assert_eq!(all_case.model().len(), 5);
    assert_eq!(
        col(&all_case, "Score"),
        vec![Value::Integer(10), Value::Null]
    );
    assert_eq!(
        col(&all_case, "score"),
        vec![Value::Null, Value::Real(20.5)]
    );
    assert_eq!(
        col(&all_case, "Extra"),
        vec![Value::Null, Value::string("hi")]
    );
    assert_eq!(
        col(&all_case, "Src"),
        vec![Value::Integer(1), Value::Integer(2)]
    );

    // Match by position ( incompatible storage at position 1 fails; compatible succeeds).
    let t3 = Table::new(
        DataModel::new(vec![
            Field::new("OtherID", Storage::Integer),
            Field::new("OtherScore", Storage::Real),
        ])
        .unwrap(),
        vec![vec![Value::Integer(2), Value::Real(30.0)]],
    )
    .unwrap();
    let by_pos = run_many(
        append::build,
        vec![p("appendMode", "position")],
        vec![t1, t3],
    )
    .unwrap();
    assert_eq!(
        col(&by_pos, "ID"),
        vec![Value::Integer(1), Value::Integer(2)]
    );
    assert_eq!(
        col(&by_pos, "Score"),
        vec![Value::Real(10.0), Value::Real(30.0)]
    );

    assert!(append::build(&node(vec![p("appendMode", "bogus")])).is_err());
}

// ------------------------------------------------------------------ Merge

fn merge_inputs() -> (Table, Table) {
    let left = Table::new(
        DataModel::new(vec![
            Field::new("ID", Storage::Integer),
            Field::new("Name", Storage::String),
            Field::new("Shared", Storage::Integer),
        ])
        .unwrap(),
        vec![
            vec![
                Value::Integer(1),
                Value::string("Alice"),
                Value::Integer(10),
            ],
            vec![Value::Integer(2), Value::string("Bob"), Value::Integer(20)],
            vec![Value::Null, Value::string("NoKeyL"), Value::Integer(30)],
        ],
    )
    .unwrap();
    let right = Table::new(
        DataModel::new(vec![
            Field::new("ID", Storage::Integer),
            Field::new("Shared", Storage::Integer),
            Field::new("Score", Storage::Integer),
        ])
        .unwrap(),
        vec![
            vec![Value::Integer(2), Value::Integer(200), Value::Integer(95)],
            vec![Value::Integer(3), Value::Integer(300), Value::Integer(80)],
            vec![Value::Null, Value::Integer(400), Value::Integer(70)],
        ],
    )
    .unwrap();
    (left, right)
}

#[test]
fn merge_by_order_and_filter_table_group_drops_and_renames_duplicates() {
    let (left, right) = merge_inputs();
    // Default order merge renames second input's duplicate fields to 2_<name>.
    let ordered = run_many(merge::build, vec![], vec![left.clone(), right.clone()]).unwrap();
    let names: Vec<&str> = ordered
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["ID", "Name", "Shared", "2_ID", "2_Shared", "Score"]
    );
    assert_eq!(ordered.row_count(), 3);

    // filterTableGroup drops 2_ID and 2_Shared (like team/cluster_analysis.str) and renames Score.
    let filtered = run_many(
        merge::build,
        vec![g(
            "filterTableGroup",
            vec![g(
                "rows",
                vec![
                    g("row", vec![p("name", "2_ID"), p("drop", "true")]),
                    g("row", vec![p("name", "2_Shared"), p("drop", "true")]),
                    g("row", vec![p("name", "Score"), p("rename", "FinalScore")]),
                ],
            )],
        )],
        vec![left, right],
    )
    .unwrap();
    let names: Vec<&str> = filtered
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, vec!["ID", "Name", "Shared", "FinalScore"]);
}

#[test]
fn merge_by_key_inner_full_outer_partial_outer_and_anti() {
    let (left, right) = merge_inputs();

    // Inner join: only ID = 2 matches; key ID is coalesced, non-key Shared becomes Shared and 2_Shared.
    let inner = run_many(
        merge::build,
        vec![
            p("mergeMode", "key"),
            g("keyFields", vec![p("field", "ID")]),
            p("joinType", "inner"),
        ],
        vec![left.clone(), right.clone()],
    )
    .unwrap();
    assert_eq!(col(&inner, "ID"), vec![Value::Integer(2)]);
    assert_eq!(col(&inner, "Shared"), vec![Value::Integer(20)]);
    assert_eq!(col(&inner, "2_Shared"), vec![Value::Integer(200)]);

    // Partial outer join keeping tag "1": keeps null-key from 1, ID=1, and ID=2 (not ID=3 or null-key from 2).
    let partial = run_many(
        merge::build,
        vec![
            p("mergeMode", "key"),
            g("keyFields", vec![p("field", "ID")]),
            p("joinType", "partialOuter"),
            g(
                "tagTable",
                vec![g(
                    "tagNames",
                    vec![
                        element_with_attr("tag", "id", "1", vec![p("name", "1")]),
                        element_with_attr("tag", "id", "2", vec![p("name", "2")]),
                    ],
                )],
            ),
            g(
                "includeIncompleteRecords",
                vec![g(
                    "includeIncompleteRecordsList",
                    vec![g(
                        "includeIncompleteRecords",
                        vec![p("inputNodeTagName", "1")],
                    )],
                )],
            ),
        ],
        vec![left.clone(), right.clone()],
    )
    .unwrap();
    assert_eq!(
        col(&partial, "ID"),
        vec![Value::Null, Value::Integer(1), Value::Integer(2)]
    );
    assert_eq!(
        col(&partial, "Score"),
        vec![Value::Null, Value::Null, Value::Integer(95)]
    );

    // Full outer join: keeps both null keys (never matched), ID=1, ID=2, ID=3.
    let full = run_many(
        merge::build,
        vec![
            p("mergeMode", "key"),
            g("keyFields", vec![p("field", "ID")]),
            p("joinType", "fullOuter"),
        ],
        vec![left.clone(), right.clone()],
    )
    .unwrap();
    assert_eq!(
        col(&full, "ID"),
        vec![
            Value::Null,
            Value::Null,
            Value::Integer(1),
            Value::Integer(2),
            Value::Integer(3),
        ]
    );

    // Anti join: records in primary dataset (1) not matched in dataset 2 -> ID=1 and Null.
    let anti = run_many(
        merge::build,
        vec![
            p("mergeMode", "key"),
            g("keyFields", vec![p("field", "ID")]),
            p("joinType", "anti"),
        ],
        vec![left, right],
    )
    .unwrap();
    assert_eq!(col(&anti, "ID"), vec![Value::Integer(1), Value::Null]);
}

#[test]
fn merge_by_condition_and_settings_validation() {
    let (left, right) = merge_inputs();
    let cond = run_many(
        merge::build,
        vec![
            p("mergeMode", "condition"),
            p("mergeCondition", "Score > 90 and ID = 1"),
        ],
        vec![left, right],
    )
    .unwrap();
    assert_eq!(col(&cond, "ID"), vec![Value::Integer(1)]);
    assert_eq!(col(&cond, "Score"), vec![Value::Integer(95)]);

    assert!(merge::build(&node(vec![p("mergeMode", "key")])).is_err());
    assert!(
        merge::build(&node(vec![
            p("mergeMode", "key"),
            p("mergeCommonKeys", "false"),
            g("keyFields", vec![p("field", "ID")]),
        ]))
        .is_err()
    );
    assert!(merge::build(&node(vec![p("mergeMode", "condition")])).is_err());
    assert!(
        merge::build(&node(vec![
            p("mergeMode", "condition"),
            p("mergeCondition", "ID = 1"),
            p("joinType", "fullOuter"),
        ]))
        .is_err()
    );
}

// ------------------------------------------------------------------ Balance

#[test]
fn balance_applies_factors_and_respects_training_partition_only() {
    // Factor 2.0 on id = 1 (training) duplicates it; factor 0.0 on id = 4 (testing) leaves it
    // untouched when balanceTrainingPartitionOnly is true (default), and drops it when false.
    let directives = g(
        "balanceDirectives",
        vec![g(
            "entries",
            vec![
                g("entry", vec![p("factor", "2.0"), p("condition", "id = 1")]),
                g("entry", vec![p("factor", "0.0"), p("condition", "id = 4")]),
            ],
        )],
    );
    let train_only = run(balance::build, vec![directives.clone()], sample_table()).unwrap();
    assert_eq!(
        col(&train_only, "id"),
        vec![
            Value::Integer(1),
            Value::Integer(1),
            Value::Integer(2),
            Value::Integer(3),
            Value::Integer(4),
        ]
    );

    let all_partitions = run(
        balance::build,
        vec![p("balanceTrainingPartitionOnly", "false"), directives],
        sample_table(),
    )
    .unwrap();
    assert_eq!(
        col(&all_partitions, "id"),
        vec![
            Value::Integer(1),
            Value::Integer(1),
            Value::Integer(2),
            Value::Integer(3),
        ]
    );

    assert!(
        balance::build(&node(vec![g(
            "balanceDirectives",
            vec![g(
                "entries",
                vec![g(
                    "entry",
                    vec![p("factor", "-1.0"), p("condition", "id = 1")]
                )],
            )],
        )]))
        .is_err()
    );
}
