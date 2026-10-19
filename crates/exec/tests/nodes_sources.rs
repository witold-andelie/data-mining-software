//! Source nodes (`var_file`, `statistics_file`) and the `type` node on synthetic files and tables.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_exec::nodes::{
    derive, filler, fixed_file, json_import, sas_import, statistics_file, r#type, user_input,
    var_file,
};
use dms_exec::{ExecContext, Factory, FileResolver, OperatorError};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::{Attribute, Element, Node, QualifiedName};
use dms_types::calendar;
use dms_types::{CheckMode, DataModel, Field, Measurement, Role, Storage, Table, Value, ValueSpec};
use std::path::PathBuf;
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

fn p_typed(name: &str, type_tag: &str, text: &str) -> Node {
    element_with_attr(name, "type", type_tag, vec![Node::Text(Arc::from(text))])
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
        binning_output: None,
        result_items: Vec::new(),
        subdiagram: None,
    }
}

fn stream_props(children: Vec<Node>) -> Arc<Element> {
    Arc::new(raw_element("properties", children))
}

fn run_source(
    factory: Factory,
    settings: Vec<Node>,
    context: &ExecContext,
) -> Result<Table, OperatorError> {
    let operator = factory(&node(settings))?;
    operator.run(&[], context)
}

fn run_unary(factory: Factory, settings: Vec<Node>, input: Table) -> Result<Table, OperatorError> {
    let operator = factory(&node(settings))?;
    operator.run(&[Arc::new(input)], &ExecContext::default())
}

fn col(table: &Table, name: &str) -> Vec<Value> {
    table.column(name).unwrap().cloned().collect()
}

fn tmp_dir(subdir: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(subdir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn ctx_with_dir(dir: PathBuf) -> ExecContext {
    ExecContext {
        files: FileResolver {
            data_dirs: vec![dir],
        },
        stream_properties: None,
        attachments: Default::default(),
        decimal_places: None,
        export_dir: None,
    }
}

// ---------------------------------------------------------------- var_file

#[test]
fn var_file_reads_csv_and_infers_storage_within_scan_lines() {
    let dir = tmp_dir("var_file_basic");
    let path = dir.join("sample.csv");
    std::fs::write(
        &path,
        "id,sex ,score,opened\r\n1,F,12.5,11/2/1997\r\n2,M,,20/5/1997\r\n3,F,8.0,30/2/1997\r\n",
    )
    .unwrap();

    let ctx = ctx_with_dir(dir);
    let out = run_source(
        var_file::build,
        vec![
            p("inputFileName", "sample.csv"),
            p("scanLineCount", "2"),
            p("autoRecognizeDates", "true"),
        ],
        &ctx,
    )
    .unwrap();

    let fields = out.model().fields();
    assert_eq!(fields.len(), 4);
    assert_eq!(fields[0].name, "id");
    assert_eq!(fields[0].storage, Storage::Integer);
    assert_eq!(fields[0].measurement, Measurement::Continuous);
    assert_eq!(fields[1].name, "sex ");
    assert_eq!(fields[1].storage, Storage::String);
    assert_eq!(fields[1].measurement, Measurement::Categorical);
    assert_eq!(fields[2].storage, Storage::Real);
    assert_eq!(fields[3].storage, Storage::Date);
    assert_eq!(
        col(&out, "opened"),
        vec![
            Value::Date(calendar::days_from_civil(1997, 2, 11).unwrap()),
            Value::Date(calendar::days_from_civil(1997, 5, 20).unwrap()),
            Value::Null,
        ]
    );
}

#[test]
fn var_file_delimiters_quotes_trim_decimal_and_stream_date_format() {
    let dir = tmp_dir("var_file_options");
    let path = dir.join("semi.txt");
    std::fs::write(
        &path,
        "skip me\n'a;1'; 3,5 ;05/01/95\n'b;2'; 4,25 ;05/01/25\n",
    )
    .unwrap();

    let mut ctx = ctx_with_dir(dir);
    ctx.stream_properties = Some(stream_props(vec![
        p("realDecimalSymbol", "comma"),
        p("dateFormat", "format7"),
        p("date2DigitBaseline", "1930"),
    ]));

    let out = run_source(
        var_file::build,
        vec![
            p("inputFileName", "C:/elsewhere/semi.txt"),
            p("skipHeaderCount", "1"),
            p("fieldNamesIncluded", "false"),
            p("delimitOnComma", "false"),
            p("useDelimiterChars", "true"),
            p("delimiterChars", ";"),
            p("singleQuoteMode", "pairDiscard"),
            p("trimMode", "both"),
            p("realDecimalSymbol", "default"),
            g(
                "textImportTable",
                vec![g(
                    "rows",
                    vec![g(
                        "row",
                        vec![
                            p("name", "field1"),
                            p("override", "true"),
                            p("storage", "string"),
                        ],
                    )],
                )],
            ),
        ],
        &ctx,
    )
    .unwrap();

    assert_eq!(
        col(&out, "field1"),
        vec![Value::string("a;1"), Value::string("b;2")]
    );
    assert_eq!(
        col(&out, "field2"),
        vec![Value::Real(3.5), Value::Real(4.25)]
    );
    assert_eq!(
        col(&out, "field3"),
        vec![
            Value::Date(calendar::days_from_civil(1995, 5, 1).unwrap()),
            Value::Date(calendar::days_from_civil(2025, 5, 1).unwrap()),
        ]
    );
}

#[test]
fn var_file_respects_import_datetime_as_string_and_embedded_filter_and_type() {
    let dir = tmp_dir("var_file_embedded");
    let path = dir.join("dates.csv");
    std::fs::write(&path, "d,x,drop_me\n2020-01-02,10,99\n2020-01-03,20,88\n").unwrap();

    let mut ctx = ctx_with_dir(dir);
    ctx.stream_properties = Some(stream_props(vec![p("importDatetimeAsString", "true")]));

    let out = run_source(
        var_file::build,
        vec![
            p("inputFileName", path.to_str().unwrap()),
            g(
                "filterTable",
                vec![g(
                    "rows",
                    vec![
                        g("row", vec![p("name", "drop_me"), p("drop", "true")]),
                        g("row", vec![p("name", "x"), p("rename", "score")]),
                    ],
                )],
            ),
            g(
                "typeTable",
                vec![
                    p("defaultValueMode", "pass"),
                    g(
                        "rows",
                        vec![g(
                            "row",
                            vec![
                                p("name", "score"),
                                p("direction", "out"),
                                p("annotation", "Target score"),
                            ],
                        )],
                    ),
                ],
            ),
        ],
        &ctx,
    )
    .unwrap();

    assert_eq!(out.model().len(), 2);
    assert_eq!(out.model().field("d").unwrap().storage, Storage::String);
    let score = out.model().field("score").unwrap();
    assert_eq!(score.role, Role::Target);
    assert_eq!(score.label.as_deref(), Some("Target score"));
}

#[test]
fn var_file_handles_empty_files_and_rejects_bad_settings() {
    let dir = tmp_dir("var_file_empty");
    let empty = dir.join("empty.csv");
    std::fs::write(&empty, b"").unwrap();
    let ctx = ctx_with_dir(dir);
    let out = run_source(var_file::build, vec![p("inputFileName", "empty.csv")], &ctx).unwrap();
    assert_eq!(out.row_count(), 0);
    assert_eq!(out.model().len(), 0);

    assert!(var_file::build(&node(vec![])).is_err());
    assert!(
        var_file::build(&node(vec![
            p("inputFileName", "a.csv"),
            p("multiBlankDelimiters", "true"),
        ]))
        .is_err()
    );
    assert!(
        var_file::build(&node(vec![
            p("inputFileName", "a.csv"),
            p("fileEncoding", "not-an-encoding"),
        ]))
        .is_err()
    );
    assert!(
        run_source(
            var_file::build,
            vec![p("inputFileName", "missing_file.csv")],
            &ctx,
        )
        .is_err()
    );
}

// ---------------------------------------------------------------- statistics_file

/// Writes a minimal uncompressed little-endian `.sav` file for testing `statistics_file`.
fn write_synthetic_sav() -> Vec<u8> {
    let mut out = Vec::new();
    let i32_le = |out: &mut Vec<u8>, v: i32| out.extend_from_slice(&v.to_le_bytes());
    let f64_le = |out: &mut Vec<u8>, v: f64| out.extend_from_slice(&v.to_le_bytes());
    let padded = |out: &mut Vec<u8>, text: &[u8], len: usize| {
        let mut buf = text.to_vec();
        buf.resize(len, b' ');
        out.extend_from_slice(&buf);
    };

    // Header: 3 variables (AGE numeric, SEX string width 4, BDATE date format 20).
    out.extend_from_slice(b"$FL2");
    padded(&mut out, b"@(#) SPSS DATA FILE test", 60);
    i32_le(&mut out, 2); // layout_code
    i32_le(&mut out, 3); // nominal_case_size (3 8-byte slots)
    i32_le(&mut out, 0); // compression = none
    i32_le(&mut out, 0); // weight_index = 0
    i32_le(&mut out, 2); // ncases = 2
    f64_le(&mut out, 100.0); // bias
    padded(&mut out, b"16 Oct 26", 9);
    padded(&mut out, b"12:00:00", 8);
    padded(&mut out, b"synthetic sav", 64);
    out.extend_from_slice(&[0, 0, 0]);

    // Var 1: AGE (numeric, label "Age in years", discrete missing -99.0, F8.2 = 0x00050802).
    i32_le(&mut out, 2);
    i32_le(&mut out, 0); // numeric
    i32_le(&mut out, 1); // has_var_label
    i32_le(&mut out, 1); // 1 discrete missing value
    i32_le(&mut out, 0x0005_0802);
    i32_le(&mut out, 0x0005_0802);
    padded(&mut out, b"AGE", 8);
    let label = b"Age in years";
    i32_le(&mut out, label.len() as i32);
    padded(&mut out, label, 12);
    f64_le(&mut out, -99.0);

    // Var 2: SEX (string width 4, A4 = 0x00010400).
    i32_le(&mut out, 2);
    i32_le(&mut out, 4);
    i32_le(&mut out, 0);
    i32_le(&mut out, 0);
    i32_le(&mut out, 0x0001_0400);
    i32_le(&mut out, 0x0001_0400);
    padded(&mut out, b"SEX", 8);

    // Var 3: BDATE (numeric with DATE format type_code 20 = 0x00140A00).
    i32_le(&mut out, 2);
    i32_le(&mut out, 0);
    i32_le(&mut out, 0);
    i32_le(&mut out, 0);
    i32_le(&mut out, 0x0014_0A00);
    i32_le(&mut out, 0x0014_0A00);
    padded(&mut out, b"BDATE", 8);

    // Value labels on SEX (var record 2): "F" -> "Female", "M" -> "Male".
    i32_le(&mut out, 3);
    i32_le(&mut out, 2);
    padded(&mut out, b"F", 8);
    out.push(6);
    padded(&mut out, b"Female", 7);
    padded(&mut out, b"M", 8);
    out.push(4);
    padded(&mut out, b"Male", 7);
    i32_le(&mut out, 4);
    i32_le(&mut out, 1);
    i32_le(&mut out, 2);

    // Extension 3 (integer info) and 4 (float info with sysmis = -f64::MAX).
    i32_le(&mut out, 7);
    i32_le(&mut out, 3);
    i32_le(&mut out, 4);
    i32_le(&mut out, 8);
    for v in [1, 0, 0, -1, 1, 1, 2, 65001] {
        i32_le(&mut out, v);
    }
    i32_le(&mut out, 7);
    i32_le(&mut out, 4);
    i32_le(&mut out, 8);
    i32_le(&mut out, 3);
    f64_le(&mut out, -f64::MAX);
    f64_le(&mut out, f64::MAX);
    f64_le(&mut out, -f64::MAX);

    // Extension 11 (display params: measure 3=Scale, 1=Nominal, 3=Scale).
    i32_le(&mut out, 7);
    i32_le(&mut out, 11);
    i32_le(&mut out, 4);
    i32_le(&mut out, 9);
    for v in [3, 8, 1, 1, 4, 0, 3, 10, 1] {
        i32_le(&mut out, v);
    }

    // Extension 18 ($@Role: SEX is Target = '1').
    let role_attr = b"SEX:$@Role('1'\n)";
    i32_le(&mut out, 7);
    i32_le(&mut out, 18);
    i32_le(&mut out, 1);
    i32_le(&mut out, role_attr.len() as i32);
    out.extend_from_slice(role_attr);

    // End of dictionary.
    i32_le(&mut out, 999);
    i32_le(&mut out, 0);

    // Row 1: AGE = 25.0, SEX = "F   ", BDATE = 1970-01-02 (12_219_379_200 + 86_400).
    f64_le(&mut out, 25.0);
    padded(&mut out, b"F", 8);
    f64_le(&mut out, 12_219_379_200.0 + 86_400.0);

    // Row 2: AGE = sysmis, SEX = "M   ", BDATE = -99.0 (valid date before 1582).
    f64_le(&mut out, -f64::MAX);
    padded(&mut out, b"M", 8);
    f64_le(&mut out, 12_219_379_200.0);

    out
}

#[test]
fn statistics_file_reads_sav_metadata_dates_missing_and_labels() {
    let dir = tmp_dir("statistics_file_basic");
    let path = dir.join("people.sav");
    std::fs::write(&path, write_synthetic_sav()).unwrap();

    let ctx = ctx_with_dir(dir);
    let out = run_source(
        statistics_file::build,
        vec![p("inputFileName", "people.sav")],
        &ctx,
    )
    .unwrap();

    let age = out.model().field("AGE").unwrap();
    assert_eq!(age.storage, Storage::Real);
    assert_eq!(age.measurement, Measurement::Continuous);
    assert_eq!(age.role, Role::Input);
    assert_eq!(age.label.as_deref(), Some("Age in years"));
    assert!(age.missing.is_blank(&Value::Real(-99.0)));
    assert!(age.missing.is_blank(&Value::Null));

    let sex = out.model().field("SEX").unwrap();
    assert_eq!(sex.storage, Storage::String);
    assert_eq!(sex.measurement, Measurement::Nominal);
    assert_eq!(sex.role, Role::Target);
    assert_eq!(
        sex.value_labels,
        vec![
            (Value::string("F"), "Female".into()),
            (Value::string("M"), "Male".into()),
        ]
    );

    let bdate = out.model().field("BDATE").unwrap();
    assert_eq!(bdate.storage, Storage::Date);

    assert_eq!(col(&out, "AGE"), vec![Value::Real(25.0), Value::Null]);
    assert_eq!(
        col(&out, "SEX"),
        vec![Value::string("F"), Value::string("M")]
    );
    assert_eq!(col(&out, "BDATE"), vec![Value::Date(1), Value::Date(0)]);

    // readVariableNames = labelsAsNames uses the variable label when present.
    let labelled = run_source(
        statistics_file::build,
        vec![
            p("inputFileName", "people.sav"),
            p("readVariableNames", "labelsAsNames"),
        ],
        &ctx,
    )
    .unwrap();
    assert!(labelled.model().field("Age in years").is_some());
}

#[test]
fn statistics_file_rejects_unsupported_and_missing_settings() {
    assert!(statistics_file::build(&node(vec![])).is_err());
    assert!(
        statistics_file::build(&node(vec![
            p("inputFileName", "x.sav"),
            p("fileIsPasswordEncrypted", "true"),
        ]))
        .is_err()
    );
    assert!(
        statistics_file::build(&node(vec![
            p("inputFileName", "x.sav"),
            p("readValues", "labelsAsData"),
        ]))
        .is_err()
    );
}

// ---------------------------------------------------------------- type

fn sample_table() -> Table {
    let fields = vec![
        Field::new("age", Storage::Integer),
        Field::new("bp", Storage::String),
        Field::new("sex", Storage::String),
        Field::new("part", Storage::String),
    ];
    let rows = vec![
        vec![
            Value::Integer(20),
            Value::string("LOW"),
            Value::string("F"),
            Value::string("1_Training"),
        ],
        vec![
            Value::Integer(40),
            Value::string("HIGH"),
            Value::string("M"),
            Value::string("2_Testing"),
        ],
        vec![
            Value::Integer(-99),
            Value::string("NORMAL"),
            Value::string("   "),
            Value::string("1_Training"),
        ],
        vec![
            Value::Null,
            Value::string("HIGH"),
            Value::string("F"),
            Value::string("2_Testing"),
        ],
    ];
    Table::new(DataModel::new(fields).unwrap(), rows).unwrap()
}

#[test]
fn type_node_reads_values_sets_roles_blanks_and_partition_categories() {
    let typed = run_unary(
        r#type::build,
        vec![g(
            "typeTable",
            vec![
                p("defaultValueMode", "read"),
                g(
                    "rows",
                    vec![
                        g(
                            "row",
                            vec![
                                p("name", "age"),
                                p("measure", "range"),
                                p("direction", "in"),
                                p("annotation", "Patient age"),
                                g(
                                    "missing",
                                    vec![
                                        p("enabled", "true"),
                                        p("nullIncluded", "true"),
                                        p("whitespaceIncluded", "false"),
                                        g("values", vec![p_typed("value", "I", "-99")]),
                                        p("rangeIncluded", "false"),
                                    ],
                                ),
                            ],
                        ),
                        g(
                            "row",
                            vec![
                                p("name", "bp"),
                                p("measure", "orderedSet"),
                                p("direction", "out"),
                                p("valueMode", "local"),
                                g(
                                    "type",
                                    vec![
                                        g(
                                            "values",
                                            vec![
                                                p_typed("value", "A", "LOW"),
                                                p_typed("value", "A", "NORMAL"),
                                                p_typed("value", "A", "HIGH"),
                                            ],
                                        ),
                                        g(
                                            "valueLabels",
                                            vec![
                                                p_typed("valueKey", "A", "HIGH"),
                                                p("valueLabel", "High BP"),
                                            ],
                                        ),
                                    ],
                                ),
                            ],
                        ),
                        g(
                            "row",
                            vec![
                                p("name", "sex"),
                                g(
                                    "missing",
                                    vec![
                                        p("enabled", "true"),
                                        p("nullIncluded", "true"),
                                        p("whitespaceIncluded", "true"),
                                    ],
                                ),
                            ],
                        ),
                        g("row", vec![p("name", "part"), p("direction", "partition")]),
                    ],
                ),
            ],
        )],
        sample_table(),
    )
    .unwrap();

    // age: -99 is a blank, so range is [20, 40].
    let age = typed.model().field("age").unwrap();
    assert_eq!(age.measurement, Measurement::Continuous);
    assert_eq!(age.role, Role::Input);
    assert_eq!(age.label.as_deref(), Some("Patient age"));
    assert_eq!(age.check, CheckMode::None);
    assert_eq!(
        age.values,
        ValueSpec::Range {
            low: Value::Integer(20),
            high: Value::Integer(40),
        }
    );
    assert!(age.missing.is_blank(&Value::Integer(-99)));
    assert!(age.missing.is_blank(&Value::Null));

    // bp: local orderedSet with value label.
    let bp = typed.model().field("bp").unwrap();
    assert_eq!(bp.measurement, Measurement::Ordinal);
    assert_eq!(bp.role, Role::Target);
    assert_eq!(
        bp.values,
        ValueSpec::Categories(vec![
            Value::string("LOW"),
            Value::string("NORMAL"),
            Value::string("HIGH"),
        ])
    );
    assert_eq!(
        bp.value_labels,
        vec![(Value::string("HIGH"), "High BP".into())]
    );

    // sex: whitespace "   " is a blank, leaving 2 distinct values ("F", "M") -> Flag; the
    // second value is true, as in Modeler.
    let sex = typed.model().field("sex").unwrap();
    assert_eq!(sex.measurement, Measurement::Flag);
    assert_eq!(
        sex.values,
        ValueSpec::Flag {
            true_value: Value::string("M"),
            false_value: Value::string("F"),
        }
    );

    // part: Role::Partition with ValueSpec::Categories, usable by @PARTITION_FIELD downstream.
    let part = typed.model().field("part").unwrap();
    assert_eq!(part.role, Role::Partition);
    assert_eq!(
        part.values,
        ValueSpec::Categories(vec![
            Value::string("1_Training"),
            Value::string("2_Testing"),
        ])
    );

    // Downstream Derive using @PARTITION_FIELD and @TESTING_PARTITION works.
    let derived = run_unary(
        derive::build,
        vec![
            p("deriveSingleFieldName", "is_test"),
            p("deriveType", "flag"),
            p(
                "deriveFlagFieldCondition",
                "@PARTITION_FIELD = @TESTING_PARTITION",
            ),
            p("deriveFlagTrueValue", "1"),
            p("deriveFlagFalseValue", "0"),
        ],
        typed.clone(),
    )
    .unwrap();
    assert_eq!(
        col(&derived, "is_test"),
        vec![
            Value::Integer(0),
            Value::Integer(1),
            Value::Integer(0),
            Value::Integer(1),
        ]
    );

    // Downstream Filler using @BLANK replaces -99 and Null on age.
    let filled = run_unary(
        filler::build,
        vec![
            g("fillerSelectedFields", vec![p("field", "age")]),
            p("fillerReplaceMode", "blanks"),
            p("fillerReplaceExpression", "0"),
        ],
        typed,
    )
    .unwrap();
    assert_eq!(
        col(&filled, "age"),
        vec![
            Value::Integer(20),
            Value::Integer(40),
            Value::Integer(0),
            Value::Integer(0),
        ]
    );
}

#[test]
fn type_node_handles_empty_tables_range_blanks_and_rejects_check_modes() {
    let empty = Table::new(
        DataModel::new(vec![Field::new("x", Storage::Integer)]).unwrap(),
        Vec::new(),
    )
    .unwrap();
    let out = run_unary(
        r#type::build,
        vec![g(
            "typeTable",
            vec![g(
                "rows",
                vec![g(
                    "row",
                    vec![
                        p("name", "x"),
                        g(
                            "missing",
                            vec![
                                p("enabled", "true"),
                                p("rangeIncluded", "true"),
                                p("rangeLowerBound", "0"),
                                p("rangeUpperBound", "10"),
                            ],
                        ),
                    ],
                )],
            )],
        )],
        empty,
    )
    .unwrap();
    assert_eq!(out.row_count(), 0);
    let x = out.model().field("x").unwrap();
    assert!(x.missing.is_blank(&Value::Integer(5)));
    assert!(!x.missing.is_blank(&Value::Integer(11)));

    // Non-None check modes return OperatorError::Settings at build time.
    for mode in ["nullify", "coerce", "discard", "warn", "abort"] {
        let err = r#type::build(&node(vec![g(
            "typeTable",
            vec![g(
                "rows",
                vec![g("row", vec![p("name", "x"), p("check", mode)])],
            )],
        )]));
        assert!(matches!(err, Err(OperatorError::Settings(_))));
    }
}

// ---------------------------------------------------------------- fixed_file

fn fixed_spec(name: &str, start: &str, length: &str) -> Node {
    g(
        "entry",
        vec![
            p("columnName", name),
            p("start", start),
            p("length", length),
        ],
    )
}

#[test]
fn fixed_file_cuts_fields_trims_both_ends_by_default_and_reads_overrides() {
    let dir = tmp_dir("fixed_file_basic");
    std::fs::write(dir.join("f.txt"), "01 ab  06.07.08\r\n2  c   23.59.59\r\n").unwrap();
    let specs = |entries: Vec<Node>| g("fixedFileColumnSpecs", vec![g("entries", entries)]);
    let settings = || {
        vec![
            p("inputFileName", "f.txt"),
            p("recordLength", "15"),
            specs(vec![
                fixed_spec("n", "1", "2"),
                fixed_spec("s", "3", "5"),
                fixed_spec("t", "8", "8"),
            ]),
            g(
                "textImportTable",
                vec![g(
                    "rows",
                    vec![g(
                        "row",
                        vec![
                            p("name", "t"),
                            p("override", "true"),
                            p("storage", "time"),
                            p("timeFormat", "format10"),
                        ],
                    )],
                )],
            ),
        ]
    };
    let ctx = ctx_with_dir(dir.clone());
    let table = run_source(fixed_file::build, settings(), &ctx).unwrap();
    assert_eq!(col(&table, "n"), [Value::Integer(1), Value::Integer(2)]);
    assert_eq!(col(&table, "s"), [Value::string("ab"), Value::string("c")]);
    assert_eq!(
        col(&table, "t"),
        [
            Value::Time(calendar::micros_from_time(6, 7, 8, 0).unwrap()),
            Value::Time(calendar::micros_from_time(23, 59, 59, 0).unwrap()),
        ]
    );

    // Out of order or overlapping fields: Modeler's "Bad format specified for file".
    let overlapping = vec![
        p("inputFileName", "f.txt"),
        specs(vec![fixed_spec("a", "3", "2"), fixed_spec("b", "1", "1")]),
    ];
    assert!(matches!(
        run_source(fixed_file::build, overlapping, &ctx),
        Err(OperatorError::Settings(_))
    ));
    let none = vec![p("inputFileName", "f.txt"), specs(vec![])];
    assert!(run_source(fixed_file::build, none, &ctx).is_err());
}

// ---------------------------------------------------------------- json_import

#[test]
fn json_import_reads_parameters_from_the_attachment() {
    let dir = tmp_dir("json_import_basic");
    std::fs::write(dir.join("r.json"), r#"[[1,"a",null],[2,"b",2.5]]"#).unwrap();
    let parameters = |format: &str, script: &str| {
        format!(
            r#"<Parameters xmlns="http://xml.spss.com/pasw/extension"><Parameter name="full_filename" value="r.json"/><Parameter name="stringFormat" value="{format}"/><Parameter name="pythonimport.import_syntax" value="{script}"/><Parameter name="pasw.annotation_label" value=""/></Parameters>"#
        )
    };
    let run = |text: String| {
        let mut ctx = ctx_with_dir(dir.clone());
        let attachments: std::collections::HashMap<String, Arc<[u8]>> =
            [("data/0001.dat".to_string(), Arc::from(text.into_bytes()))].into();
        ctx.attachments = Arc::new(attachments);
        let mut stream_node = node(Vec::new());
        stream_node.properties_attachment = Some("data/0001.dat".into());
        json_import::build(&stream_node)?.run(&[], &ctx)
    };
    let table = run(parameters(
        "values",
        "x.read_json(p, **{ 'orient' : 'values'})",
    ))
    .unwrap();
    let names: Vec<&str> = table
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, ["0", "1", "2"]);
    assert_eq!(col(&table, "0"), [Value::Integer(1), Value::Integer(2)]);
    assert_eq!(col(&table, "2"), [Value::Null, Value::Real(2.5)]);

    // The script's orient must match stringFormat; a custom script is rejected.
    assert!(
        run(parameters(
            "records",
            "read_json(p, **{ 'orient' : 'values'})"
        ))
        .is_err()
    );
    assert!(run(parameters("values", "my_reader()")).is_err());
}

// ---------------------------------------------------------------- user_input

fn user_field(name: &str, storage: &str, values: &str) -> Node {
    g(
        "field",
        vec![p("name", name), p("storage", storage), p("values", values)],
    )
}

#[test]
fn user_input_combines_orders_ranges_and_refuses_bad_values() {
    let run = |mode: &str, fields: Vec<Node>| {
        run_source(
            user_input::build,
            vec![
                p("dataMode", mode),
                g("userFieldTable", vec![g("fields", fields)]),
            ],
            &ExecContext::default(),
        )
    };
    let table = run(
        "combine",
        vec![
            user_field("n", "integer", "2 1"),
            user_field("blank", "string", ""),
            user_field("s", "string", r#"x "y z""#),
        ],
    )
    .unwrap();
    assert_eq!(col(&table, "n"), [2, 2, 1, 1].map(Value::Integer).to_vec());
    assert_eq!(col(&table, "blank"), vec![Value::Null; 4]);
    assert_eq!(col(&table, "s")[1], Value::string("y z"));

    let table = run(
        "order",
        vec![
            user_field("r", "integer", "1 10,14,2"),
            user_field("x", "real", "0,0.3,0.1"),
            user_field("u", "unknown", "1 2.5"),
        ],
    )
    .unwrap();
    assert_eq!(
        col(&table, "r"),
        [1, 10, 12, 14].map(Value::Integer).to_vec()
    );
    assert_eq!(
        col(&table, "x"),
        [
            Value::Real(0.0),
            Value::Real(0.1),
            Value::Real(0.2),
            Value::Null
        ]
    );
    assert_eq!(table.model().field("u").unwrap().storage, Storage::Real);

    for bad in [
        user_field("x", "integer", "5,1,-2"),
        user_field("x", "integer", "abc"),
        user_field("x", "unknown", "1 text"),
        user_field("x", "date", "31/12/1999"),
    ] {
        assert!(run("order", vec![bad]).is_err());
    }
}

// ---------------------------------------------------------------- sas_import

#[test]
fn sas_import_rejects_unsupported_and_missing_settings() {
    assert!(sas_import::build(&node(vec![])).is_err());
    assert!(matches!(
        sas_import::build(&node(vec![
            p("inputFileName", "sastest.sas7bdat"),
            p("readFormats", "true"),
        ])),
        Err(OperatorError::Settings(msg)) if msg.contains("GAP:")
    ));
    for unsupported_fmt in ["Windows", "UNIX", "SAS6", "SAS8"] {
        assert!(matches!(
            sas_import::build(&node(vec![
                p("inputFileName", "sastest.sas7bdat"),
                p("fileFormat", unsupported_fmt),
            ])),
            Err(OperatorError::Settings(msg)) if msg.contains("GAP:")
        ));
    }
    assert!(matches!(
        sas_import::build(&node(vec![
            p("inputFileName", "sastest.sas7bdat"),
            p("readVariableNames", "invalidMode"),
        ])),
        Err(OperatorError::Settings(_))
    ));
}

#[test]
fn sas_import_reads_corpus_files_and_runs_corpus_streams() {
    let corpus_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/sas");
    if !corpus_dir.join("sastest.sas7bdat").is_file() {
        return;
    }
    let ctx = ctx_with_dir(corpus_dir.clone());

    // Direct node execution: SAS7/SAS9 NamesAndLabels, LabelsAsNames, and Transport.
    let sas9_names = run_source(
        sas_import::build,
        vec![
            p("inputFileName", "sastest.sas7bdat"),
            p("fileFormat", "SAS7"),
            p("readVariableNames", "namesAndLabels"),
        ],
        &ctx,
    )
    .unwrap();
    let field_names: Vec<&str> = sas9_names
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(
        field_names,
        vec!["name", "grp", "joined", "amount", "id", "score"]
    );
    assert_eq!(
        sas9_names.model().field("name").unwrap().label.as_deref(),
        Some("Customer name")
    );
    assert_eq!(
        sas9_names.model().field("joined").unwrap().storage,
        Storage::Date
    );
    assert_eq!(sas9_names.row_count(), 5);

    let sas9_labels = run_source(
        sas_import::build,
        vec![
            p("inputFileName", "sastest.sas7bdat"),
            p("fileFormat", "SAS9"),
            p("readVariableNames", "labelsAsNames"),
        ],
        &ctx,
    )
    .unwrap();
    let label_names: Vec<&str> = sas9_labels
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(
        label_names,
        vec![
            "Customer name",
            "field2",
            "Join date",
            "Amount (EUR)",
            "field5",
            "field6",
        ]
    );

    let xpt = run_source(
        sas_import::build,
        vec![
            p("inputFileName", "sastest.xpt"),
            p("fileFormat", "Transport"),
            p("memberName", "SASTEST"),
            p("readVariableNames", "namesAndLabels"),
        ],
        &ctx,
    )
    .unwrap();
    let xpt_names: Vec<&str> = xpt
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(
        xpt_names,
        vec!["NAME", "GRP", "JOINED", "AMOUNT", "ID", "SCORE"]
    );
    assert_eq!(xpt.rows(), sas9_names.rows());

    // Execute the three saved .str streams from corpus/sas/ and verify their SASInputNode output.
    for (stream_file, expected_cols) in [
        (
            "sas9_names.str",
            vec!["name", "grp", "joined", "amount", "id", "score"],
        ),
        (
            "sas9_labels.str",
            vec![
                "Customer name",
                "field2",
                "Join date",
                "Amount (EUR)",
                "field5",
                "field6",
            ],
        ),
        (
            "sas_transport.str",
            vec!["NAME", "GRP", "JOINED", "AMOUNT", "ID", "SCORE"],
        ),
    ] {
        let str_path = corpus_dir.join(stream_file);
        if !str_path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&str_path).unwrap();
        let archive =
            dms_strfmt::read_archive(&bytes, &dms_strfmt::ArchiveLimits::default()).unwrap();
        let stream = dms_stream::Stream::from_document(&archive.document).unwrap();
        let sas_node = stream
            .all_nodes()
            .into_iter()
            .find(|n| n.type_identifier() == "SASInputNode")
            .unwrap();
        let props_debug: Vec<String> = sas_node
            .settings
            .as_ref()
            .map(|el| {
                el.child_elements()
                    .map(|c| format!("{}={:?}", c.name.local, c.text()))
                    .collect()
            })
            .unwrap_or_default();
        let plan = dms_exec::Plan::new(&stream);
        let results = plan.evaluate(&[sas_node.uid.as_str()], &ctx);
        let output = results
            .get(&sas_node.uid)
            .unwrap()
            .as_ref()
            .unwrap_or_else(|e| panic!("{stream_file} failed: {e} (props: {props_debug:?})"));
        let table = output.data().unwrap();
        let actual_cols: Vec<&str> = table
            .model()
            .fields()
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(
            actual_cols, expected_cols,
            "{stream_file} columns mismatch (props: {props_debug:?})"
        );
        assert_eq!(
            table.rows(),
            sas9_names.rows(),
            "{stream_file} rows mismatch"
        );
    }
}
