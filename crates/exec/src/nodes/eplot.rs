//! E-Plot (Beta) (`EPlotNode`, catalog id `eplot`, Graphs, scope later).
//!
//! Public source: IBM SPSS Modeler 19.0 Python Scripting and Automation Guide, Table 107
//! (`eplotnode Properties`). The node shows the relationship between numeric fields. It is
//! similar to Plot, with different options and a graphing interface of its own. The user guide
//! only lists it in the Analytic Server support table.
//!
//! ## Settings
//!
//! Element names as Modeler 19 saves them (`corpus/eplot/*.str`, written by
//! `xtask/oracle/probes/eplot_streams.py` through the documented scripting properties):
//!
//! - `x_field` → `graphXField`, `y_field` → `graphYField`
//! - `color_field` → `graphColorField`, `size_field` → `graphSizeField`,
//!   `shape_field` → `graphShapeField` (each optional; all three may be set together)
//! - `interested_fields` → child `field` elements of `graphInterestedFields`
//! - `records_limit` → `graphLargeDatasetReadThreshold` (default 2000; a negative value
//!   clamps to 0, matching Plot's saved encoding of `-1` → `0`)
//! - `if_over_limit` → `graphLargeDatasetOption`, `Sample` (the default) or `All`
//!
//! ## Records plotted
//!
//! - At most `records_limit` records when Sample is in force and the input is larger.
//! - Every input record when the input is no larger than the limit, or when Use all data
//!   is in force.
//! - Sample is a reservoir sample of exactly `min(n, records_limit)` records, then the kept
//!   rows in input order. The guide says “randomly samples … until it hits the
//!   records_limit” and names no RNG. OPEN: Modeler’s generator and seed. This crate uses
//!   `java.util.Random` with the Sample node’s default seed `1_217_380` so the subset is
//!   deterministic here.
//!
//! ## Output document
//!
//! Not verifiable against Modeler: in batch the node runs and returns a `DocumentOutput`, but
//! exporting it fails in every format (comma-delimited, JSON, text, VizML, HTML: IOException,
//! "Unsupported for windowed rendering"; `oracle/probes/eplot_streams/manifest.json`). The
//! `eplot` oracles therefore check only the records that reach the node. The table here is
//! Plot-shaped (`nodes::graph_data`; `oracle/graphs/plot_*`), an assumption:
//!
//! - One row per plotted record, in the order of those records in the input.
//! - Header: x, y, then colour / size / shape when set, then `interested_fields` in listed
//!   order. A name already in the header is not repeated.
//! - Integers and reals print as Java doubles; dates and strings use `Value`'s display
//!   (ISO dates). A null or missing value is an empty cell and the record stays.
//!
use super::graph_data::java_double;
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput};
use crate::random::JavaRandom;
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Field, Table, Value};
use std::sync::Arc;

/// Sample node’s default `sampleRandomSeed`. Used only because E-Plot documents no seed.
const SAMPLE_SEED: i64 = 1_217_380;
const DEFAULT_LIMIT: i64 = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverLimit {
    Sample,
    All,
}

struct EPlot {
    x: String,
    y: String,
    color: Option<String>,
    size: Option<String>,
    shape: Option<String>,
    interested: Vec<String>,
    limit: usize,
    over_limit: OverLimit,
}

fn named(s: Option<&dms_strfmt::fastinfoset::Element>, name: &str) -> Option<String> {
    settings::text(s, name)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn field_list(s: Option<&dms_strfmt::fastinfoset::Element>, parent: &str) -> Vec<String> {
    settings::children(s, parent, "field")
        .into_iter()
        .map(|field| field.text().trim().to_string())
        .filter(|field| !field.is_empty())
        .collect()
}

fn over_limit(s: Option<&dms_strfmt::fastinfoset::Element>) -> Result<OverLimit, OperatorError> {
    match named(s, "graphLargeDatasetOption").as_deref() {
        None | Some("Sample") => Ok(OverLimit::Sample),
        Some("All") => Ok(OverLimit::All),
        Some(other) => Err(OperatorError::Settings(format!(
            "E-Plot graphLargeDatasetOption={other:?} is not Sample or All"
        ))),
    }
}

fn records_limit(s: Option<&dms_strfmt::fastinfoset::Element>) -> Result<usize, OperatorError> {
    let raw = settings::integer(s, "graphLargeDatasetReadThreshold")?.unwrap_or(DEFAULT_LIMIT);
    if raw < 0 {
        return Ok(0);
    }
    usize::try_from(raw).map_err(|_| {
        OperatorError::Settings(format!(
            "E-Plot records_limit {raw} does not fit a row count"
        ))
    })
}

fn require_axis(
    s: Option<&dms_strfmt::fastinfoset::Element>,
    name: &str,
    label: &str,
) -> Result<String, OperatorError> {
    named(s, name)
        .ok_or_else(|| OperatorError::Settings(format!("E-Plot: no {label} field ({name})")))
}

/// Reservoir sample of `limit` indices from `0..n`, then sorted into input order.
/// `java.util.Random.nextInt(i + 1)` at each step past the fill (Algorithm R).
fn sample_indices(n: usize, limit: usize, seed: i64) -> Vec<usize> {
    if limit == 0 || n == 0 {
        return Vec::new();
    }
    if n <= limit {
        return (0..n).collect();
    }
    let mut chosen: Vec<usize> = (0..limit).collect();
    let mut random = JavaRandom::new(seed);
    for index in limit..n {
        let bound = match u32::try_from(index.saturating_add(1)) {
            Ok(bound) if bound > 0 => bound,
            _ => break,
        };
        let draw = random.next_int(bound) as usize;
        if let Some(slot) = chosen.get_mut(draw) {
            *slot = index;
        }
    }
    chosen.sort_unstable();
    chosen
}

fn header_columns(plot: &EPlot) -> Vec<String> {
    let mut header = vec![plot.x.clone(), plot.y.clone()];
    for extra in plot
        .color
        .iter()
        .chain(plot.size.iter())
        .chain(plot.shape.iter())
        .chain(plot.interested.iter())
    {
        if !header.iter().any(|name| name == extra) {
            header.push(extra.clone());
        }
    }
    header
}

fn cell(field: &Field, value: &Value) -> String {
    if value.is_null() || field.is_missing(value) {
        return String::new();
    }
    match value {
        Value::Integer(_) | Value::Real(_) => value
            .as_f64()
            .filter(|number| number.is_finite())
            .map(java_double)
            .unwrap_or_default(),
        other => other.to_string(),
    }
}

fn cell_at(input: &Table, row: &[Value], at: usize) -> String {
    let Some(field) = input.model().fields().get(at) else {
        return String::new();
    };
    cell(field, row.get(at).unwrap_or(&Value::Null))
}

/// The E-Plot (Beta) node (`EPlotNode`).
pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    Ok(Box::new(EPlot {
        x: require_axis(s, "graphXField", "x")?,
        y: require_axis(s, "graphYField", "y")?,
        color: named(s, "graphColorField"),
        size: named(s, "graphSizeField"),
        shape: named(s, "graphShapeField"),
        interested: field_list(s, "graphInterestedFields"),
        limit: records_limit(s)?,
        over_limit: over_limit(s)?,
    }))
}

impl Operator for EPlot {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "E-Plot is a graph node and produces a Document, not a Table".into(),
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
            .ok_or_else(|| OperatorError::Inputs("E-Plot needs one input".into()))?;
        let n = input.row_count();
        let indices = match self.over_limit {
            OverLimit::All => (0..n).collect(),
            OverLimit::Sample => sample_indices(n, self.limit, SAMPLE_SEED),
        };
        let header = header_columns(self);
        let positions = header
            .iter()
            .map(|name| {
                input
                    .model()
                    .position(name)
                    .ok_or_else(|| OperatorError::Data(format!("no field {name}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut rows = Vec::with_capacity(indices.len());
        for &row_index in &indices {
            let Some(row) = input.rows().get(row_index) else {
                continue;
            };
            let cells = positions
                .iter()
                .map(|&at| cell_at(input, row, at))
                .collect();
            rows.push(cells);
        }
        Ok(NodeOutput::Document(Arc::new(Document {
            title: format!("E-Plot of {}", self.x),
            tables: vec![DocumentTable {
                title: None,
                header,
                rows,
            }],
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::{SAMPLE_SEED, cell, over_limit, records_limit, sample_indices};
    use crate::context::ExecContext;
    use crate::output::NodeOutput;
    use dms_stream::StreamNode;
    use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
    use dms_types::{DataModel, Field, Storage, Table, Value};
    use std::sync::Arc;

    fn qname(local: &str) -> QualifiedName {
        QualifiedName {
            prefix: None,
            namespace: None,
            local: Arc::from(local),
        }
    }

    fn prop(name: &str, text: &str) -> Node {
        Node::Element(Element {
            name: qname(name),
            namespace_declarations: Vec::new(),
            attributes: Vec::new(),
            children: vec![Node::Text(Arc::from(text))],
        })
    }

    fn list_prop(name: &str, fields: &[&str]) -> Node {
        Node::Element(Element {
            name: qname(name),
            namespace_declarations: Vec::new(),
            attributes: Vec::new(),
            children: fields.iter().map(|field| prop("field", field)).collect(),
        })
    }

    fn properties(children: Vec<Node>) -> StreamNode {
        StreamNode {
            uid: "e".into(),
            class: "EPlotNode".into(),
            extension: None,
            model_builder: None,
            name: None,
            label: None,
            position: None,
            properties_attachment: None,
            containers: Vec::new(),
            settings: Some(Element {
                name: qname("properties"),
                namespace_declarations: Vec::new(),
                attributes: Vec::new(),
                children,
            }),
            binning_output: None,
            result_items: Vec::new(),
            subdiagram: None,
        }
    }

    fn table(names: &[(&str, Storage)], rows: Vec<Vec<Value>>) -> Table {
        let model = DataModel::new(
            names
                .iter()
                .map(|(name, storage)| Field::new(*name, *storage))
                .collect(),
        )
        .unwrap();
        Table::new(model, rows).unwrap()
    }

    fn run(
        settings_nodes: Vec<Node>,
        input: Table,
    ) -> Result<Arc<crate::Document>, crate::OperatorError> {
        let operator = super::build(&properties(settings_nodes))?;
        match operator.produce(&[Arc::new(input)], None, &ExecContext::default())? {
            NodeOutput::Document(document) => Ok(document),
            NodeOutput::Data(_) | NodeOutput::Model(_) => {
                panic!("eplot produced a non-document")
            }
        }
    }

    fn xy_input() -> Table {
        table(
            &[
                ("amount", Storage::Real),
                ("score", Storage::Real),
                ("weight", Storage::Real),
                ("region", Storage::String),
                ("channel", Storage::String),
                ("id", Storage::Integer),
                ("when", Storage::Date),
            ],
            vec![
                vec![
                    Value::real(1.5),
                    Value::real(0.5),
                    Value::real(1.1),
                    Value::string("North"),
                    Value::string("Store"),
                    Value::Integer(1),
                    Value::Date(0),
                ],
                vec![
                    Value::Null,
                    Value::real(1.0),
                    Value::real(1.2),
                    Value::string(""),
                    Value::string("Web"),
                    Value::Integer(2),
                    Value::Date(1),
                ],
                vec![
                    Value::real(3.0),
                    Value::Null,
                    Value::Null,
                    Value::string("South"),
                    Value::string("Store"),
                    Value::Integer(3),
                    Value::Date(2),
                ],
            ],
        )
    }

    fn many_rows(count: usize) -> Table {
        let rows = (0..count)
            .map(|i| {
                vec![
                    Value::real(i as f64),
                    Value::real((i as f64) + 0.5),
                    Value::string(if i % 2 == 0 { "A" } else { "B" }),
                ]
            })
            .collect();
        table(
            &[
                ("amount", Storage::Real),
                ("score", Storage::Real),
                ("region", Storage::String),
            ],
            rows,
        )
    }

    #[test]
    fn xy_only_keeps_missing_cells_and_prints_java_doubles() {
        let document = run(
            vec![prop("graphXField", "amount"), prop("graphYField", "score")],
            xy_input(),
        )
        .unwrap();
        let table = &document.tables[0];
        assert_eq!(table.header, vec!["amount", "score"]);
        assert_eq!(
            table.rows,
            vec![
                vec!["1.5".to_string(), "0.5".to_string()],
                vec![String::new(), "1.0".to_string()],
                vec!["3.0".to_string(), String::new()],
            ]
        );
        assert_eq!(document.title, "E-Plot of amount");
    }

    #[test]
    fn each_overlay_appends_one_column() {
        let color = run(
            vec![
                prop("graphXField", "amount"),
                prop("graphYField", "score"),
                prop("graphColorField", "region"),
            ],
            xy_input(),
        )
        .unwrap();
        assert_eq!(color.tables[0].header, vec!["amount", "score", "region"]);
        assert_eq!(color.tables[0].rows[1][2], "");

        let size = run(
            vec![
                prop("graphXField", "amount"),
                prop("graphYField", "score"),
                prop("graphSizeField", "weight"),
            ],
            xy_input(),
        )
        .unwrap();
        assert_eq!(size.tables[0].header, vec!["amount", "score", "weight"]);
        assert_eq!(size.tables[0].rows[2][2], "");

        let shape = run(
            vec![
                prop("graphXField", "amount"),
                prop("graphYField", "score"),
                prop("graphShapeField", "channel"),
            ],
            xy_input(),
        )
        .unwrap();
        assert_eq!(shape.tables[0].header, vec!["amount", "score", "channel"]);
        assert_eq!(shape.tables[0].rows[0][2], "Store");
    }

    #[test]
    fn several_overlays_and_interested_fields_share_one_header() {
        let document = run(
            vec![
                prop("graphXField", "amount"),
                prop("graphYField", "score"),
                prop("graphColorField", "region"),
                prop("graphSizeField", "weight"),
                prop("graphShapeField", "channel"),
                list_prop("graphInterestedFields", &["id", "when", "region"]),
            ],
            xy_input(),
        )
        .unwrap();
        assert_eq!(
            document.tables[0].header,
            vec![
                "amount", "score", "region", "weight", "channel", "id", "when"
            ]
        );
        assert_eq!(document.tables[0].rows[0][5], "1.0");
        assert_eq!(document.tables[0].rows[0][6], "1970-01-01");
        assert_eq!(document.tables[0].rows.len(), 3);
    }

    #[test]
    fn string_and_date_axes_print_as_text() {
        let string_x = run(
            vec![prop("graphXField", "region"), prop("graphYField", "score")],
            xy_input(),
        )
        .unwrap();
        assert_eq!(string_x.tables[0].rows[0][0], "North");
        assert_eq!(string_x.tables[0].rows[1][0], "");

        let date_x = run(
            vec![prop("graphXField", "when"), prop("graphYField", "score")],
            xy_input(),
        )
        .unwrap();
        assert_eq!(date_x.tables[0].rows[0][0], "1970-01-01");
        assert_eq!(date_x.tables[0].rows[1][0], "1970-01-02");
    }

    #[test]
    fn integer_cells_print_as_java_doubles() {
        let field = Field::new("id", Storage::Integer);
        assert_eq!(cell(&field, &Value::Integer(23)), "23.0");
    }

    #[test]
    fn missing_axis_is_a_settings_error() {
        let err = match super::build(&properties(vec![prop("graphXField", "amount")])) {
            Err(err) => err,
            Ok(_) => panic!("expected a settings error, built an operator"),
        };
        match err {
            crate::OperatorError::Settings(message) => {
                assert!(message.contains("y"), "{message}");
            }
            other => panic!("expected settings error, got {other:?}"),
        }
    }

    #[test]
    fn default_limit_is_2000_and_negatives_clamp_to_zero() {
        let empty = properties(vec![]);
        assert_eq!(records_limit(empty.settings.as_ref()).unwrap(), 2000);
        let negative = properties(vec![prop("graphLargeDatasetReadThreshold", "-1")]);
        assert_eq!(records_limit(negative.settings.as_ref()).unwrap(), 0);
        let ten = properties(vec![prop("graphLargeDatasetReadThreshold", "10")]);
        assert_eq!(records_limit(ten.settings.as_ref()).unwrap(), 10);
    }

    #[test]
    fn over_limit_encodings() {
        let missing = properties(vec![]);
        assert_eq!(
            over_limit(missing.settings.as_ref()).unwrap(),
            super::OverLimit::Sample
        );
        let sample = properties(vec![prop("graphLargeDatasetOption", "Sample")]);
        assert_eq!(
            over_limit(sample.settings.as_ref()).unwrap(),
            super::OverLimit::Sample
        );
        let all = properties(vec![prop("graphLargeDatasetOption", "All")]);
        assert_eq!(
            over_limit(all.settings.as_ref()).unwrap(),
            super::OverLimit::All
        );
        let bad = properties(vec![prop("graphLargeDatasetOption", "Bin")]);
        assert!(over_limit(bad.settings.as_ref()).is_err());
    }

    #[test]
    fn under_the_limit_plots_every_row() {
        let document = run(
            vec![
                prop("graphXField", "amount"),
                prop("graphYField", "score"),
                prop("graphLargeDatasetReadThreshold", "2000"),
            ],
            many_rows(80),
        )
        .unwrap();
        assert_eq!(document.tables[0].rows.len(), 80);
        assert_eq!(document.tables[0].rows[0][0], "0.0");
        assert_eq!(document.tables[0].rows[79][0], "79.0");
    }

    #[test]
    fn sample_over_the_limit_keeps_limit_rows_in_input_order() {
        let document = run(
            vec![
                prop("graphXField", "amount"),
                prop("graphYField", "score"),
                prop("graphLargeDatasetReadThreshold", "10"),
                prop("graphLargeDatasetOption", "Sample"),
            ],
            many_rows(80),
        )
        .unwrap();
        let rows = &document.tables[0].rows;
        assert_eq!(rows.len(), 10);
        let amounts: Vec<f64> = rows
            .iter()
            .map(|row| row[0].parse::<f64>().unwrap())
            .collect();
        let mut sorted = amounts.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(amounts, sorted);
        assert!(amounts.iter().all(|value| (0.0..80.0).contains(value)));
        let again = sample_indices(80, 10, SAMPLE_SEED);
        assert_eq!(again.len(), 10);
        let expected: Vec<String> = again
            .iter()
            .map(|i| super::java_double(*i as f64))
            .collect();
        let got: Vec<String> = rows.iter().map(|row| row[0].clone()).collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn use_all_ignores_the_records_limit() {
        let document = run(
            vec![
                prop("graphXField", "amount"),
                prop("graphYField", "score"),
                prop("graphLargeDatasetReadThreshold", "10"),
                prop("graphLargeDatasetOption", "All"),
            ],
            many_rows(80),
        )
        .unwrap();
        assert_eq!(document.tables[0].rows.len(), 80);
    }

    #[test]
    fn sample_of_zero_is_empty_and_under_limit_is_identity() {
        assert!(sample_indices(10, 0, SAMPLE_SEED).is_empty());
        assert_eq!(sample_indices(4, 10, SAMPLE_SEED), vec![0, 1, 2, 3]);
        let first = sample_indices(20, 7, SAMPLE_SEED);
        let second = sample_indices(20, 7, SAMPLE_SEED);
        assert_eq!(first, second);
        assert_eq!(first.len(), 7);
    }

    #[test]
    fn produce_rejects_a_missing_field() {
        let err = run(
            vec![
                prop("graphXField", "amount"),
                prop("graphYField", "missing"),
            ],
            xy_input(),
        )
        .unwrap_err();
        match err {
            crate::OperatorError::Data(message) => {
                assert!(message.contains("missing"), "{message}")
            }
            other => panic!("expected data error, got {other:?}"),
        }
    }

    #[test]
    fn run_does_not_yield_a_table() {
        assert!(crate::registry::is_implemented("eplot"));
        let operator = super::build(&properties(vec![
            prop("graphXField", "amount"),
            prop("graphYField", "score"),
        ]))
        .unwrap();
        let err = operator
            .run(&[Arc::new(xy_input())], &ExecContext::default())
            .unwrap_err();
        match err {
            crate::OperatorError::Inputs(message) => {
                assert!(message.contains("Document"), "{message}");
            }
            other => panic!("expected inputs error, got {other:?}"),
        }
    }
}
