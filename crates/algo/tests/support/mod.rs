//! Helpers the algorithm oracle tests share: the `models/*` cases, Modeler's CSV dialect, the
//! training table typed by the oracle's data dictionary, tree comparison, and scoring against
//! the nugget's recorded output.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly; each test uses a subset"
)]

use dms_pmml::{DataType, Model, Node, OpType, Pmml, Predicate, Value as PmmlValue};
use dms_types::{DataModel, Field, Measurement, Storage, Table, Value, ValueSpec};
use std::path::{Path, PathBuf};

/// The PMML of a `models/<case>` oracle.
pub fn oracle_model(case: &str) -> Pmml {
    let xml = std::fs::read_dir(case_dir(case).join("models"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "xml"))
        .unwrap();
    Pmml::parse(&std::fs::read_to_string(xml).unwrap()).unwrap()
}

pub fn case_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../oracle/models")
        .join(name)
}

pub fn data_file(dir: &Path, marker: &str) -> PathBuf {
    std::fs::read_dir(dir.join("data"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.file_name().unwrap().to_string_lossy().contains(marker))
        .unwrap_or_else(|| panic!("no {marker} file in {}", dir.display()))
}

pub struct Cell {
    pub text: String,
    pub quoted: bool,
}

/// Modeler's CSV export: quotes inside quoted cells are not escaped, so a quote closes a cell
/// only before `,`, a line break or the end.
pub fn parse_modeler_csv(text: &str) -> Vec<Vec<Cell>> {
    let text = text.trim_start_matches('\u{feff}');
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let (mut quoted, mut in_quotes) = (false, false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            let next = chars.peek().copied();
            if c == '"' && next.is_some_and(|n| !matches!(n, ',' | '\r' | '\n')) {
                cell.push('"');
            } else if c == '"' {
                in_quotes = false;
            } else {
                cell.push(c);
            }
            continue;
        }
        match c {
            '"' => {
                in_quotes = true;
                quoted = true;
            }
            ',' => row.push(Cell {
                text: std::mem::take(&mut cell),
                quoted: std::mem::take(&mut quoted),
            }),
            '\r' => {}
            '\n' => {
                row.push(Cell {
                    text: std::mem::take(&mut cell),
                    quoted: std::mem::take(&mut quoted),
                });
                rows.push(std::mem::take(&mut row));
            }
            _ => cell.push(c),
        }
    }
    if !cell.is_empty() || quoted || !row.is_empty() {
        row.push(Cell { text: cell, quoted });
        rows.push(row);
    }
    rows
}

pub fn is_missing(cell: &Cell) -> bool {
    (!cell.quoted && cell.text.is_empty()) || cell.text == "$null$"
}

/// The build node's records as a table typed by the oracle's data dictionary.
pub fn training_table(oracle: &Pmml, csv: &str) -> Table {
    let rows = parse_modeler_csv(csv);
    let header: Vec<&str> = rows[0].iter().map(|c| c.text.as_str()).collect();
    // (CSV column, field) for the columns the data dictionary types.
    let mut columns: Vec<(usize, Field)> = Vec::new();
    for (i, name) in header.iter().enumerate() {
        let Some(dd) = oracle.data_dictionary.iter().find(|f| f.name == *name) else {
            continue;
        };
        let storage = match dd.data_type {
            DataType::Double | DataType::Float => Storage::Real,
            DataType::Integer => Storage::Integer,
            _ => Storage::String,
        };
        let mut field = Field::new(*name, storage);
        field.measurement = match dd.optype {
            OpType::Continuous => Measurement::Continuous,
            OpType::Ordinal => Measurement::Ordinal,
            OpType::Categorical => Measurement::Nominal,
        };
        if dd.optype != OpType::Continuous {
            field.values =
                ValueSpec::Categories(dd.values.iter().map(|v| typed(storage, v)).collect());
        }
        columns.push((i, field));
    }
    let model_rows = rows[1..]
        .iter()
        .filter(|r| r.len() == header.len())
        .map(|r| {
            columns
                .iter()
                .map(|(i, field)| {
                    let cell = &r[*i];
                    if is_missing(cell) {
                        Value::Null
                    } else {
                        typed(field.storage, &cell.text)
                    }
                })
                .collect()
        })
        .collect();
    let fields = columns.into_iter().map(|(_, field)| field).collect();
    Table::new(DataModel::new(fields).unwrap(), model_rows).unwrap()
}

pub fn typed(storage: Storage, text: &str) -> Value {
    match storage {
        Storage::Real => Value::Real(text.trim().parse().unwrap()),
        Storage::Integer => Value::Integer(text.trim().parse().unwrap()),
        _ => Value::string(text),
    }
}

pub fn tree(pmml: &Pmml) -> &Node {
    match &pmml.model {
        Model::Tree(tree) => &tree.root,
        other => panic!("not a tree: {other:?}"),
    }
}

pub fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

pub fn numbers_equal(a: &str, b: &str) -> bool {
    a == b || matches!((a.parse::<f64>(), b.parse::<f64>()), (Ok(x), Ok(y)) if close(x, y))
}

/// The first difference between two trees, depth first.
pub fn first_difference(ours: &Node, theirs: &Node) -> Option<String> {
    let at = format!("node {}", theirs.id.as_deref().unwrap_or("?"));
    if ours.id != theirs.id {
        return Some(format!("{at}: id {:?}", ours.id));
    }
    if ours.record_count != theirs.record_count {
        return Some(format!(
            "{at}: records {:?} vs {:?}",
            ours.record_count, theirs.record_count
        ));
    }
    let scores = (ours.score.as_deref(), theirs.score.as_deref());
    if !matches!(scores, (Some(a), Some(b)) if numbers_equal(a, b)) && scores.0 != scores.1 {
        return Some(format!("{at}: score {:?} vs {:?}", scores.0, scores.1));
    }
    if ours.default_child != theirs.default_child {
        return Some(format!(
            "{at}: default child {:?} vs {:?}",
            ours.default_child, theirs.default_child
        ));
    }
    if !predicates_equal(&ours.predicate, &theirs.predicate) {
        return Some(format!(
            "{at}: predicate\n      ours   {}\n      oracle {}",
            show(&ours.predicate),
            show(&theirs.predicate)
        ));
    }
    if ours.distribution.len() != theirs.distribution.len()
        || ours
            .distribution
            .iter()
            .zip(&theirs.distribution)
            .any(|(a, b)| {
                a.value != b.value
                    || a.record_count != b.record_count
                    || !option_close(a.probability, b.probability)
                    || !option_close(a.confidence, b.confidence)
            })
    {
        return Some(format!(
            "{at}: distribution {:?} vs {:?}",
            ours.distribution, theirs.distribution
        ));
    }
    if ours.children.len() != theirs.children.len() {
        return Some(format!(
            "{at}: {} children vs {}",
            ours.children.len(),
            theirs.children.len()
        ));
    }
    ours.children
        .iter()
        .zip(&theirs.children)
        .find_map(|(a, b)| first_difference(a, b))
}

pub fn option_close(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => close(x, y),
        (None, None) => true,
        _ => false,
    }
}

pub fn predicates_equal(a: &Predicate, b: &Predicate) -> bool {
    match (a, b) {
        (
            Predicate::Simple {
                field: f1,
                operator: o1,
                value: v1,
            },
            Predicate::Simple {
                field: f2,
                operator: o2,
                value: v2,
            },
        ) => {
            f1 == f2
                && o1 == o2
                && match (v1, v2) {
                    (Some(x), Some(y)) => numbers_equal(x, y),
                    (x, y) => x == y,
                }
        }
        (
            Predicate::Compound {
                operator: o1,
                predicates: p1,
            },
            Predicate::Compound {
                operator: o2,
                predicates: p2,
            },
        ) => {
            o1 == o2
                && p1.len() == p2.len()
                && p1.iter().zip(p2).all(|(x, y)| predicates_equal(x, y))
        }
        (x, y) => x == y,
    }
}

pub fn show(p: &Predicate) -> String {
    match p {
        Predicate::True => "True".into(),
        Predicate::False => "False".into(),
        Predicate::Simple {
            field,
            operator,
            value,
        } => format!("{field} {operator:?} {}", value.as_deref().unwrap_or("")),
        Predicate::SimpleSet { field, values, .. } => format!("{field} in {values:?}"),
        Predicate::Compound {
            operator,
            predicates,
        } => format!(
            "{operator:?}({})",
            predicates.iter().map(show).collect::<Vec<_>>().join(" | ")
        ),
    }
}

/// Scores the nugget's records with our model and compares with what Modeler's nugget output
/// (`$R-<target>` and, for classification, `$RC-<target>`): (matching rows, rows).
pub fn score_both(ours: &Pmml, target: &str, csv: &str, inputs: &[&str]) -> (usize, usize) {
    let rows = parse_modeler_csv(csv);
    let header: Vec<&str> = rows[0].iter().map(|c| c.text.as_str()).collect();
    let column = |name: &str| header.iter().position(|h| *h == name);
    let positions: Vec<usize> = inputs.iter().map(|name| column(name).unwrap()).collect();
    let predicted = column(&format!("$R-{target}")).unwrap();
    let confidence = column(&format!("$RC-{target}"));
    let ours = ours.bind(inputs).unwrap();
    let mut matched = 0;
    let mut total = 0;
    for row in rows[1..].iter().filter(|r| r.len() == header.len()) {
        let record: Vec<PmmlValue> = positions
            .iter()
            .map(|&i| {
                let cell = &row[i];
                if is_missing(cell) {
                    PmmlValue::Missing
                } else if !cell.quoted
                    && let Ok(x) = cell.text.trim().parse::<f64>()
                {
                    PmmlValue::Number(x)
                } else {
                    PmmlValue::Text(cell.text.clone())
                }
            })
            .collect();
        let prediction = ours.score(&record).unwrap();
        total += 1;
        let expected = &row[predicted].text;
        let same_value = match &prediction.value {
            PmmlValue::Number(x) => expected.parse::<f64>().is_ok_and(|y| close(*x, y)),
            PmmlValue::Text(t) => {
                t == expected
                    || matches!((t.parse::<f64>(), expected.parse::<f64>()), (Ok(a), Ok(b)) if a == b)
            }
            PmmlValue::Missing => is_missing(&row[predicted]),
        };
        let same_confidence =
            confidence.is_none_or(
                |i| match (prediction.confidence, row[i].text.parse::<f64>()) {
                    (Some(a), Ok(b)) => close(a, b),
                    (None, _) => is_missing(&row[i]),
                    _ => false,
                },
            );
        if same_value && same_confidence {
            matched += 1;
        }
    }
    (matched, total)
}
