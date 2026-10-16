//! Oracle and unit tests for the Statistics, Means and Matrix output nodes.

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions and corpus setup fail loudly"
)]

use dms_exec::nodes::{matrix, means, statistics};
use dms_exec::{
    Document, DocumentTable, ExecContext, Factory, FileResolver, NodeOutput, OperatorError, Plan,
};
use dms_stream::{Stream, StreamNode};
use dms_strfmt::ArchiveLimits;
use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
use dms_types::{DataModel, Field, Measurement, MissingSpec, Storage, Table, Value, ValueSpec};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

/// The node's document, or the evaluation error (a team stream may need a node we lack).
fn evaluate_stream_output(
    label: &str,
    stream_path: &Path,
    node_uid: &str,
) -> Result<Arc<Document>, String> {
    let root = repo_root();
    let bytes = std::fs::read(stream_path).unwrap();
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();

    let mut data_dirs = Vec::new();
    let label_dir = root.join("corpus").join(label);
    if label_dir.is_dir() {
        data_dirs.push(label_dir);
    }
    let demo_dir = root.join("corpus").join("ibm-demo-data");
    if demo_dir.is_dir() {
        data_dirs.push(demo_dir);
    }

    let attachments: HashMap<String, Arc<[u8]>> = archive
        .attachments
        .into_iter()
        .map(|a| (a.name, a.bytes.into()))
        .collect();

    let context = ExecContext {
        files: FileResolver { data_dirs },
        stream_properties: stream.properties.clone().map(Arc::new),
        attachments: Arc::new(attachments),
        decimal_places: Some(15),
    };

    let plan = Plan::new(&stream);
    let results = plan.evaluate(&[node_uid], &context);
    match results.get(node_uid).unwrap() {
        Ok(NodeOutput::Document(doc)) => Ok(Arc::clone(doc)),
        Ok(other) => panic!("expected NodeOutput::Document for {node_uid}, got {other:?}"),
        Err(error) => Err(format!("{error:?}")),
    }
}

/// Parses a Modeler-dialect CSV (unescaped quotes inside quoted fields) into rows of strings.
fn parse_modeler_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() && rows.is_empty() {
            continue;
        }
        rows.push(parse_csv_line(line));
    }
    rows
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let mut cells = Vec::new();
    let mut i = 0;
    loop {
        if i >= bytes.len() {
            cells.push(String::new());
            break;
        }
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut end = start;
            let mut next = bytes.len();
            let mut j = start;
            while j < bytes.len() {
                if bytes[j] == b'"' && (j + 1 == bytes.len() || bytes[j + 1] == b',') {
                    end = j;
                    next = j + 1;
                    break;
                }
                j += 1;
            }
            cells.push(line[start..end].to_string());
            if next < bytes.len() && bytes[next] == b',' {
                i = next + 1;
            } else {
                break;
            }
        } else {
            let start = i;
            while i < bytes.len() && bytes[i] != b',' {
                i += 1;
            }
            cells.push(line[start..i].to_string());
            if i < bytes.len() && bytes[i] == b',' {
                i += 1;
            } else {
                break;
            }
        }
    }
    cells
}

/// Parses a Modeler Statistics JSON export into `Document` tables.
fn parse_statistics_json(text: &str) -> Document {
    let root: JsonValue = serde_json::from_str(text).unwrap();
    let mut tables = Vec::new();
    let sections = root.get("data").and_then(JsonValue::as_array).unwrap();
    for section in sections {
        let title = section
            .get("title")
            .and_then(JsonValue::as_str)
            .map(str::to_owned);
        let inner_data = section.get("data").and_then(JsonValue::as_array).unwrap();
        for item in inner_data {
            if let Some(format) = item.get("format").and_then(JsonValue::as_str) {
                let raw_rows = item.get("table").and_then(JsonValue::as_array).unwrap();
                let parsed_rows: Vec<Vec<String>> = raw_rows
                    .iter()
                    .map(|r| {
                        r.as_array()
                            .unwrap()
                            .iter()
                            .map(|c| c.as_str().unwrap().to_owned())
                            .collect()
                    })
                    .collect();
                if format == "FORMAT_ROW_AND_COLUMN_HEADER" {
                    let (header, rows) = parsed_rows
                        .split_first()
                        .map(|(h, rest)| (h.clone(), rest.to_vec()))
                        .unwrap_or_default();
                    tables.push(DocumentTable {
                        title: title.clone(),
                        header,
                        rows,
                    });
                } else {
                    tables.push(DocumentTable {
                        title: title.clone(),
                        header: Vec::new(),
                        rows: parsed_rows,
                    });
                }
            } else if let Some(sub_data) = item.get("data").and_then(JsonValue::as_array) {
                for sub_item in sub_data {
                    // A title-only item is the multiple-modes footnote; the `*` on the Mode
                    // value carries the same fact, so documents do not model footnotes.
                    let Some(raw_rows) = sub_item.get("table").and_then(JsonValue::as_array) else {
                        continue;
                    };
                    let parsed_rows: Vec<Vec<String>> = raw_rows
                        .iter()
                        .map(|r| {
                            r.as_array()
                                .unwrap()
                                .iter()
                                .map(|c| c.as_str().unwrap().to_owned())
                                .collect()
                        })
                        .collect();
                    tables.push(DocumentTable {
                        title: title.clone(),
                        header: Vec::new(),
                        rows: parsed_rows,
                    });
                }
            }
        }
    }
    Document {
        title: String::new(),
        tables,
    }
}

fn parse_csv_document(text: &str) -> Document {
    let all_rows = parse_modeler_csv(text);
    let (header, rows) = all_rows
        .split_first()
        .map(|(h, rest)| (h.clone(), rest.to_vec()))
        .unwrap_or_default();
    Document {
        title: String::new(),
        tables: vec![DocumentTable {
            title: None,
            header,
            rows,
        }],
    }
}

fn cells_equal(actual: &str, expected: &str) -> bool {
    if actual == expected {
        return true;
    }
    // Check numeric comparison for formatted reals or `<number>/<label>` cells.
    if let (Some((a_num, a_lbl)), Some((e_num, e_lbl))) =
        (actual.split_once('/'), expected.split_once('/'))
    {
        return a_lbl == e_lbl && numbers_close(a_num, e_num);
    }
    numbers_close(actual, expected)
}

/// Numeric cells printed with the same number of decimals that agree to 1e-12: Modeler sums in
/// an order we do not always reproduce, so the 15th decimal can differ by an ulp or two. This is
/// stricter than `xtask compare` (printed rounding plus a relative 1e-9). The report prints exact
/// text matches separately so the gap stays visible.
fn numbers_close(a: &str, b: &str) -> bool {
    let (Ok(fa), Ok(fb)) = (a.parse::<f64>(), b.parse::<f64>()) else {
        return false;
    };
    if !fa.is_finite() || !fb.is_finite() {
        return false;
    }
    // Only treat as matching if they have the same decimal precision formatting and agree to 1e-12.
    let a_decimals = a.split_once('.').map_or(0, |(_, frac)| frac.len());
    let b_decimals = b.split_once('.').map_or(0, |(_, frac)| frac.len());
    a_decimals == b_decimals && (fa - fb).abs() <= 1e-12
}

/// Cells compared: (matched within `cells_equal`, exactly equal text, total, differences).
fn compare_documents(actual: &Document, expected: &Document) -> (usize, usize, usize, Vec<String>) {
    let mut matched = 0usize;
    let mut exact = 0usize;
    let mut total = 0usize;
    let mut diffs = Vec::new();

    let max_tables = actual.tables.len().max(expected.tables.len());
    for t_idx in 0..max_tables {
        let a_tbl = actual.tables.get(t_idx);
        let e_tbl = expected.tables.get(t_idx);
        match (a_tbl, e_tbl) {
            (Some(a), Some(e)) => {
                if a.title != e.title {
                    diffs.push(format!(
                        "table {t_idx} title: actual={:?}, expected={:?}",
                        a.title, e.title
                    ));
                }
                if !a.header.is_empty() || !e.header.is_empty() {
                    let h_len = a.header.len().max(e.header.len());
                    for c in 0..h_len {
                        total += 1;
                        let ac = a.header.get(c).map(String::as_str);
                        let ec = e.header.get(c).map(String::as_str);
                        if ac == ec {
                            matched += 1;
                            exact += 1;
                        } else if diffs.len() < 10 {
                            diffs.push(format!(
                                "table {t_idx} header[{c}]: actual={ac:?}, expected={ec:?}"
                            ));
                        }
                    }
                }
                let r_len = a.rows.len().max(e.rows.len());
                for r in 0..r_len {
                    let a_row = a.rows.get(r);
                    let e_row = e.rows.get(r);
                    let c_len = a_row.map_or(0, Vec::len).max(e_row.map_or(0, Vec::len));
                    for c in 0..c_len {
                        total += 1;
                        let ac = a_row.and_then(|row| row.get(c)).map(String::as_str);
                        let ec = e_row.and_then(|row| row.get(c)).map(String::as_str);
                        if ac.is_some() && ac == ec {
                            matched += 1;
                            exact += 1;
                        } else if let (Some(av), Some(ev)) = (ac, ec)
                            && cells_equal(av, ev)
                        {
                            matched += 1;
                            if diffs.len() < 10 {
                                diffs.push(format!(
                                    "table {t_idx} ({:?}) row {r} col {c}: within 1e-12: actual={av:?}, expected={ev:?}",
                                    e.title
                                ));
                            }
                        } else if diffs.len() < 10 {
                            diffs.push(format!(
                                "table {t_idx} ({:?}) row {r} col {c}: actual={ac:?}, expected={ec:?}",
                                e.title
                            ));
                        }
                    }
                }
            }
            (None, Some(e)) => {
                let cells: usize = e.header.len() + e.rows.iter().map(Vec::len).sum::<usize>();
                total += cells;
                diffs.push(format!("missing table {t_idx} ({:?})", e.title));
            }
            (Some(a), None) => {
                let cells: usize = a.header.len() + a.rows.iter().map(Vec::len).sum::<usize>();
                total += cells;
                diffs.push(format!("extra table {t_idx} ({:?})", a.title));
            }
            (None, None) => {}
        }
    }

    (matched, exact, total, diffs)
}

#[derive(Debug)]
struct OracleCase {
    label: String,
    stream_name: String,
    stream_path: PathBuf,
    node_id: String,
    format: String,
    output_file: PathBuf,
    is_probe: bool,
}

fn collect_oracle_cases() -> Vec<OracleCase> {
    let root = repo_root();
    let mut cases = Vec::new();
    for (label, is_probe) in [("outputs", true), ("team", false)] {
        let dir = root.join("oracle").join(label);
        if !dir.is_dir() {
            continue;
        }
        let mut subdirs: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        subdirs.sort();

        for sub in subdirs {
            let manifest_path = sub.join("manifest.json");
            if !manifest_path.is_file() {
                continue;
            }
            let stream_name = sub.file_name().unwrap().to_str().unwrap().to_string();
            let text = std::fs::read_to_string(&manifest_path).unwrap();
            let manifest: JsonValue = serde_json::from_str(&text).unwrap();
            let stream_str = manifest.get("stream").and_then(JsonValue::as_str).unwrap();
            let stream_path = PathBuf::from(stream_str);
            let Some(outputs) = manifest.get("outputs").and_then(JsonValue::as_array) else {
                continue;
            };
            for entry in outputs {
                let node_type = entry.get("type").and_then(JsonValue::as_str).unwrap();
                if !matches!(node_type, "statistics" | "means" | "matrix") {
                    continue;
                }
                let node_id = entry
                    .get("id")
                    .and_then(JsonValue::as_str)
                    .unwrap()
                    .to_string();
                let format = entry
                    .get("format")
                    .and_then(JsonValue::as_str)
                    .unwrap()
                    .to_string();
                let file_str = entry.get("file").and_then(JsonValue::as_str).unwrap();
                cases.push(OracleCase {
                    label: label.to_string(),
                    stream_name: stream_name.clone(),
                    stream_path: stream_path.clone(),
                    node_id,
                    format,
                    output_file: PathBuf::from(file_str),
                    is_probe,
                });
            }
        }
    }
    cases
}

#[test]
fn statistics_means_and_matrix_match_modeler_oracles() {
    let cases = collect_oracle_cases();
    assert!(
        !cases.is_empty(),
        "expected oracle cases for statistics/means/matrix"
    );

    let mut failures = Vec::new();
    for case in &cases {
        let file_name = case.output_file.file_name().unwrap().to_str().unwrap();
        let actual = match evaluate_stream_output(&case.label, &case.stream_path, &case.node_id) {
            Ok(actual) => actual,
            Err(error) => {
                println!(
                    "{}/{}: not evaluated ({error})",
                    case.stream_name, file_name
                );
                if case.is_probe {
                    failures.push(format!("{}/{}: {error}", case.stream_name, file_name));
                }
                continue;
            }
        };
        let oracle_text = std::fs::read_to_string(&case.output_file).unwrap();
        let expected = if case.format == "json" {
            parse_statistics_json(&oracle_text)
        } else {
            parse_csv_document(&oracle_text)
        };

        let (matched, exact, total, diffs) = compare_documents(&actual, &expected);
        println!(
            "{}/{}: {}/{} cells ({} exact)",
            case.stream_name, file_name, matched, total, exact
        );
        for diff in &diffs {
            println!("    {diff}");
        }
        if matched != total && case.is_probe {
            failures.push(format!(
                "{}/{}: {}/{} cells matched; diffs: {:?}",
                case.stream_name, file_name, matched, total, diffs
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "probe oracle mismatches:
{}",
        failures.join(
            "
"
        )
    );
}

// ------------------------------------------------------------------ Unit tests for uncovered rules

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

fn p(name: &str, text: &str) -> Node {
    Node::Element(raw_element(name, vec![Node::Text(Arc::from(text))]))
}

fn g(name: &str, children: Vec<Node>) -> Node {
    Node::Element(raw_element(name, children))
}

fn make_node(children: Vec<Node>) -> StreamNode {
    StreamNode {
        uid: "u1".into(),
        class: "c".into(),
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

fn produce_doc(
    factory: Factory,
    settings: Vec<Node>,
    input: Table,
) -> Result<Arc<Document>, OperatorError> {
    let op = factory(&make_node(settings))?;
    let ctx = ExecContext {
        decimal_places: Some(3),
        ..ExecContext::default()
    };
    match op.produce(&[Arc::new(input)], None, &ctx)? {
        NodeOutput::Document(d) => Ok(d),
        other => panic!("expected Document, got {other:?}"),
    }
}

#[test]
fn statistics_absolute_strength_mode_unique_mode_and_settings_validation() {
    let fields = vec![
        Field::new("x", Storage::Integer),
        Field::new("y", Storage::Real),
    ];
    let rows = vec![
        vec![Value::Integer(5), Value::Real(1.0)],
        vec![Value::Integer(5), Value::Real(2.0)],
        vec![Value::Integer(10), Value::Real(3.0)],
    ];
    let table = Table::new(DataModel::new(fields).unwrap(), rows).unwrap();

    // Unique mode (5 appears twice, 10 once -> "5" without '*'), absolute correlation mode with
    // showCorrelationLabels=false.
    let doc = produce_doc(
        statistics::build,
        vec![
            g("examineFields", vec![p("field", "x")]),
            g(
                "statistics",
                vec![p("setElement", "mode"), p("setElement", "median")],
            ),
            g("correlationFields", vec![p("field", "x"), p("field", "y")]),
            p("correlationStrengthMode", "absolute"),
            p("showCorrelationLabels", "false"),
        ],
        table,
    )
    .unwrap();
    assert_eq!(
        doc.tables[0].rows,
        vec![
            vec!["Median".to_string(), "5".to_string()],
            vec!["Mode".to_string(), "5".to_string()],
        ]
    );
    assert_eq!(doc.tables[1].rows[0][1], "1.000");

    // Rejects unsupported outputMode, unknown stat, unknown strength mode, and invalid thresholds.
    assert!(statistics::build(&make_node(vec![p("outputMode", "file")])).is_err());
    assert!(
        statistics::build(&make_node(vec![g(
            "statistics",
            vec![p("setElement", "skewness")]
        )]))
        .is_err()
    );
    assert!(statistics::build(&make_node(vec![p("correlationStrengthMode", "rank")])).is_err());
    assert!(
        statistics::build(&make_node(vec![
            p("weakProbabilityThreshold", "0.98"),
            p("mediumProbabilityThreshold", "0.90"),
        ]))
        .is_err()
    );
}

#[test]
fn means_rejects_within_records_and_handles_degenerate_groups() {
    assert!(means::build(&make_node(vec![p("method", "withinRecords")])).is_err());
    assert!(means::build(&make_node(vec![])).is_err());
    assert!(means::build(&make_node(vec![p("groupingField", "g")])).is_err());

    // Single group -> F-Test, df1, df2, Importance are all "--".
    let fields = vec![
        Field::new("g", Storage::String),
        Field::new("x", Storage::Real),
    ];
    let rows = vec![
        vec![Value::string("A"), Value::Real(2.0)],
        vec![Value::string("A"), Value::Real(4.0)],
    ];
    let table = Table::new(DataModel::new(fields).unwrap(), rows).unwrap();
    let doc = produce_doc(
        means::build,
        vec![
            p("groupingField", "g"),
            g("meansGroupedFields", vec![p("field", "x")]),
        ],
        table,
    )
    .unwrap();
    let row = &doc.tables[0].rows[0];
    assert_eq!(&row[row.len() - 4..], &["--", "--", "--", "--"]);
}

#[test]
fn matrix_supports_sum_sdev_min_max_and_excludes_missing_values_when_configured() {
    let mut g_field = Field::new("g", Storage::String);
    g_field.measurement = Measurement::Nominal;
    g_field.values = ValueSpec::Categories(vec![Value::string("A"), Value::string("B")]);
    g_field.missing = MissingSpec {
        values: vec![Value::string("B")],
        ..MissingSpec::default()
    };
    let mut c_field = Field::new("c", Storage::String);
    c_field.measurement = Measurement::Nominal;
    c_field.values = ValueSpec::Categories(vec![Value::string("X")]);
    let v_field = Field::new("v", Storage::Real);

    let rows = vec![
        vec![Value::string("A"), Value::string("X"), Value::Real(2.0)],
        vec![Value::string("A"), Value::string("X"), Value::Real(6.0)],
        vec![Value::string("B"), Value::string("X"), Value::Real(100.0)],
        vec![Value::Null, Value::string("X"), Value::Real(50.0)],
    ];
    let table = Table::new(
        DataModel::new(vec![g_field, c_field, v_field]).unwrap(),
        rows,
    )
    .unwrap();

    for (fn_name, expected) in [
        ("sum", "8.000"),
        ("sdev", "2.828"),
        ("min", "2.000"),
        ("max", "6.000"),
    ] {
        let doc = produce_doc(
            matrix::build,
            vec![
                p("rowField", "g"),
                p("columnField", "c"),
                p("cellContents", "parameter"),
                p("parameterField", "v"),
                p("parameterFunction", fn_name),
                p("includeMissingValues", "false"),
            ],
            table.clone(),
        )
        .unwrap();
        // With includeMissingValues=false, both $null$ and blank "B" are excluded, leaving only "A".
        assert_eq!(
            doc.tables[0].rows,
            vec![vec!["A".to_string(), expected.to_string()]]
        );
    }

    assert!(matrix::build(&make_node(vec![p("rowField", "g")])).is_err());
    assert!(
        matrix::build(&make_node(vec![
            p("rowField", "g"),
            p("columnField", "c"),
            p("cellContents", "parameter"),
        ]))
        .is_err()
    );
}
