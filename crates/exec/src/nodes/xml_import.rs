//! XML source (`xml_import!spss.xml_data`): reads records from an XML file, or from every XML
//! file of a directory, with the structure the node's dialog scanned
//! (`oracle/xml_scan/xml_*`).
//!
//! The node's settings are `Parameter` entries in the extension attachment (`node@properties`).
//! Modeler builds the fields only in the dialog; it saves them as `xmlReader.xpaths`
//! (`field_name`, `xpath`) and `xmlReader.storages` (`field_name`, `override`, `storage`), and a
//! scripted node without them fails with "Node -1 has no exports". The corpus streams carry a
//! dialog-style structure (`xtask/oracle/xml_structure.py`, from the user's dialog-saved
//! `corpus/sources/xml_template.str`).
//!
//! Reading rules, each shown by the oracles:
//!
//! - Records are the elements at the absolute path `xmlReader.records`, in document order
//!   (`xml_nested_records`: `/orders/order/line` takes the lines of every order in turn).
//! - A field's `xpath` is the record path plus `/child` (a child element) or `/@name` (an
//!   attribute). Output fields follow `xmlReader.storages`, not the xpath list.
//! - A child element's value is its text, entities and CDATA resolved (`xml_text_whitespace`:
//!   `caf\u{e9} & <tag>`, `a<b & c`); line ends are normalised as XML requires (CR LF becomes LF)
//!   and other whitespace is kept (`  spaced  `). The first of repeated children is taken
//!   (`xml_repeated_elements`: `red`, not `blue`).
//! - `string` storage: an empty or absent element or attribute is the empty string, not null
//!   (`xml_default` record 5 and record 6's absent `day`; `xml_text_whitespace` record 5).
//!   `integer` storage: the text as a whole number; empty or absent is null (`xml_dtd`,
//!   `xml_xsd`: record 5's `<qty/>`).
//! - `xmlReader.mode` `directory` reads the `.xml` files of `xmlReader.name` in name order, then,
//!   when `xmlReader.recurse` is true, each subdirectory in name order (`xml_directory`:
//!   a-first then z-last; `xml_directory_recursive`: then nested/m-middle). Other files are
//!   skipped (`ignore.txt`).
//! - The schema file (`full_schema_filename`, `xml_xsd`, `xml_dtd`) does not change the values:
//!   the storages carry the structure.
//!
//! GAP: storages other than `integer` and `string`, `override` true, `xmlReader.read` other than
//! `read`, a non-empty `xmlReader.fields`, namespaced names, and XPaths beyond a child element or
//! attribute have no oracle. A node with no scanned fields fails like Modeler's
//! (`xml_empty`, `xml_malformed_rejected`, `xml_namespace_rejected` have no fields).

use super::extension_params::{Param, flag, load_parameters, reject_unknown, text};
use super::r#type::{SourceTables, apply_source_tables, parse_source_tables};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use dms_stream::StreamNode;
use dms_types::{DataModel, Field, Measurement, Storage, Table, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const LABEL: &str = "XML source";

/// Bounds on what one node reads.
const MAX_FILES: usize = 10_000;
const MAX_DEPTH: usize = 32;
const MAX_FIELDS: usize = 4_096;

const KNOWN_PARAMETERS: &[&str] = &[
    "xmlReader.fields",
    "xmlReader.read",
    "xmlReader.selected_root",
    "xmlReader.full_schema_filename",
    "xmlReader.storages",
    "xmlReader.ui_storages",
    "xmlReader.xpaths",
    "xmlReader.recurse",
    "xmlReader.records",
    "xmlReader.catalog",
    "xmlReader.scanner_type",
    "xmlReader.name",
    "xmlReader.full_filename",
    "xmlReader.mode",
    "show_attributes",
    "autoLabel",
];

fn gap(message: impl std::fmt::Display) -> OperatorError {
    OperatorError::Settings(format!("GAP: {LABEL}: {message}"))
}

struct XmlImport {
    attachment: Option<String>,
    tables: SourceTables,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    Ok(Box::new(XmlImport {
        attachment: node.properties_attachment.clone(),
        tables: parse_source_tables(node.settings.as_ref())?,
    }))
}

impl Operator for XmlImport {
    fn arity(&self) -> Arity {
        Arity::Exactly(0)
    }

    fn run(&self, _inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let params = load_parameters(self.attachment.as_deref(), context, LABEL)?;
        let settings = Settings::parse(&params)?;
        let mut rows = Vec::new();
        for path in settings.files(context)? {
            let bytes = std::fs::read(&path)
                .map_err(|e| OperatorError::Io(format!("{}: {e}", path.display())))?;
            read_records(&path, &bytes, &settings, &mut rows)?;
        }
        let fields = settings
            .fields
            .iter()
            .map(|f| {
                let mut field = Field::new(f.name.clone(), f.storage);
                field.measurement = match f.storage {
                    Storage::String => Measurement::Categorical,
                    _ => Measurement::Continuous,
                };
                field
            })
            .collect();
        let model =
            DataModel::new(fields).map_err(|e| OperatorError::Data(format!("{LABEL}: {e}")))?;
        let table =
            Table::new(model, rows).map_err(|e| OperatorError::Data(format!("{LABEL}: {e}")))?;
        apply_source_tables(&self.tables, table)
    }
}

/// Where a field's value is in a record.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Item {
    Child(String),
    Attribute(String),
}

struct ScannedField {
    name: String,
    item: Item,
    storage: Storage,
}

enum Source {
    File(String),
    Directory { name: String, recurse: bool },
}

struct Settings {
    source: Source,
    records: Vec<String>,
    fields: Vec<ScannedField>,
}

fn structured<'a>(
    params: &'a HashMap<String, Param>,
    name: &str,
) -> Result<&'a [Vec<(String, String)>], OperatorError> {
    match params.get(name) {
        Some(Param::Structured(items)) => Ok(items),
        Some(Param::List(values)) if values.is_empty() => Ok(&[]),
        None => Ok(&[]),
        Some(_) => Err(gap(format!("{name} is not a list of structured values"))),
    }
}

fn attribute<'a>(item: &'a [(String, String)], name: &str) -> Option<&'a str> {
    item.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// An absolute path of plain element names (`/records/record`).
fn element_path(path: &str) -> Option<Vec<String>> {
    let rest = path.strip_prefix('/')?;
    let steps: Vec<String> = rest.split('/').map(str::to_string).collect();
    steps
        .iter()
        .all(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
        })
        .then_some(steps)
}

impl Settings {
    fn parse(params: &HashMap<String, Param>) -> Result<Settings, OperatorError> {
        reject_unknown(params, KNOWN_PARAMETERS, LABEL)?;
        if text(params, "xmlReader.read").is_some_and(|read| read != "read") {
            return Err(gap("only read = read (all scanned items) has an oracle"));
        }
        for name in ["xmlReader.fields"] {
            match params.get(name) {
                None => {}
                Some(Param::List(values)) if values.is_empty() => {}
                Some(_) => return Err(gap(format!("{name} (specified items) has no oracle"))),
            }
        }
        for name in [
            "xmlReader.selected_root",
            "xmlReader.catalog",
            "xmlReader.scanner_type",
        ] {
            if text(params, name).is_some_and(|value| !value.trim().is_empty()) {
                return Err(gap(format!("{name} has no oracle")));
            }
        }
        let source = match text(params, "xmlReader.mode").as_deref() {
            None | Some("single") => Source::File(
                text(params, "xmlReader.full_filename")
                    .filter(|name| !name.trim().is_empty())
                    .ok_or_else(|| {
                        OperatorError::Settings(format!("{LABEL}: full_filename is missing"))
                    })?,
            ),
            Some("directory") => Source::Directory {
                name: text(params, "xmlReader.name")
                    .filter(|name| !name.trim().is_empty())
                    .ok_or_else(|| {
                        OperatorError::Settings(format!("{LABEL}: directory_name is missing"))
                    })?,
                recurse: flag(params, "xmlReader.recurse")?.unwrap_or(false),
            },
            Some(other) => return Err(gap(format!("mode {other:?}"))),
        };
        let records_text = text(params, "xmlReader.records").unwrap_or_default();
        let records = element_path(&records_text).ok_or_else(|| {
            gap(format!(
                "record path {records_text:?} is not an absolute element path"
            ))
        })?;
        let xpaths = structured(params, "xmlReader.xpaths")?;
        let storages = structured(params, "xmlReader.storages")?;
        if storages.is_empty() {
            return Err(OperatorError::Data(format!(
                "{LABEL}: the node has no scanned fields (Modeler: \"Node has no exports\")"
            )));
        }
        if storages.len() > MAX_FIELDS {
            return Err(gap(format!("more than {MAX_FIELDS} fields")));
        }
        let mut fields = Vec::with_capacity(storages.len());
        for item in storages {
            let name = attribute(item, "field_name")
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    OperatorError::Data(format!("{LABEL}: a storage without field_name"))
                })?;
            if attribute(item, "override").is_some_and(|value| value != "false") {
                return Err(gap(format!("storage override for {name}")));
            }
            let storage = match attribute(item, "storage") {
                Some("integer") => Storage::Integer,
                Some("string") => Storage::String,
                other => return Err(gap(format!("storage {other:?} for {name}"))),
            };
            let xpath = xpaths
                .iter()
                .find(|x| attribute(x, "field_name") == Some(name))
                .and_then(|x| attribute(x, "xpath"))
                .ok_or_else(|| OperatorError::Data(format!("{LABEL}: no xpath for {name}")))?;
            let relative = xpath
                .strip_prefix(&records_text)
                .and_then(|rest| rest.strip_prefix('/'))
                .ok_or_else(|| gap(format!("xpath {xpath:?} outside the record path")))?;
            let item = match relative.strip_prefix('@') {
                Some(attr) => Item::Attribute(attr.to_string()),
                None => Item::Child(relative.to_string()),
            };
            let plain = match &item {
                Item::Attribute(n) | Item::Child(n) => {
                    element_path(&format!("/{n}")).is_some_and(|s| s.len() == 1)
                }
            };
            if !plain {
                return Err(gap(format!(
                    "xpath {xpath:?} is not a child element or attribute"
                )));
            }
            fields.push(ScannedField {
                name: name.to_string(),
                item,
                storage,
            });
        }
        Ok(Settings {
            source,
            records,
            fields,
        })
    }

    fn files(&self, context: &ExecContext) -> Result<Vec<PathBuf>, OperatorError> {
        match &self.source {
            Source::File(name) => Ok(vec![
                context
                    .files
                    .resolve(name)
                    .ok_or_else(|| OperatorError::Io(format!("cannot find data file {name:?}")))?
                    .path()
                    .to_path_buf(),
            ]),
            Source::Directory { name, recurse } => {
                let dir = context
                    .files
                    .resolve_dir(name)
                    .ok_or_else(|| OperatorError::Io(format!("cannot find directory {name:?}")))?;
                let mut files = Vec::new();
                collect(dir.path(), *recurse, 0, &mut files)?;
                Ok(files)
            }
        }
    }
}

/// The `.xml` files of `dir` in name order, then each subdirectory's when recursing.
fn collect(
    dir: &Path,
    recurse: bool,
    depth: usize,
    out: &mut Vec<PathBuf>,
) -> Result<(), OperatorError> {
    if depth > MAX_DEPTH {
        return Err(gap(format!("directories nested deeper than {MAX_DEPTH}")));
    }
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let entries =
        std::fs::read_dir(dir).map_err(|e| OperatorError::Io(format!("{}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| OperatorError::Io(format!("{}: {e}", dir.display())))?;
        let kind = entry
            .file_type()
            .map_err(|e| OperatorError::Io(format!("{}: {e}", entry.path().display())))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if kind.is_dir() {
            dirs.push((name, entry.path()));
        } else if kind.is_file() && name.to_ascii_lowercase().ends_with(".xml") {
            files.push((name, entry.path()));
        }
    }
    files.sort();
    dirs.sort();
    for (_, path) in files {
        if out.len() >= MAX_FILES {
            return Err(gap(format!("more than {MAX_FILES} files")));
        }
        out.push(path);
    }
    if recurse {
        for (_, path) in dirs {
            collect(&path, recurse, depth + 1, out)?;
        }
    }
    Ok(())
}

fn read_records(
    path: &Path,
    bytes: &[u8],
    settings: &Settings,
    rows: &mut Vec<Vec<Value>>,
) -> Result<(), OperatorError> {
    let bad =
        |message: String| OperatorError::Data(format!("{LABEL}: {}: {message}", path.display()));
    let text = std::str::from_utf8(bytes).map_err(|e| bad(format!("not UTF-8: {e}")))?;
    let document = roxmltree::Document::parse(text).map_err(|e| bad(e.to_string()))?;
    let root = document.root_element();
    let Some((first, rest)) = settings.records.split_first() else {
        return Ok(());
    };
    if root.tag_name().namespace().is_some() {
        return Err(gap("namespaced elements"));
    }
    if root.tag_name().name() != first {
        return Ok(());
    }
    let mut level = vec![root];
    for step in rest {
        level = level
            .iter()
            .flat_map(|node| node.children())
            .filter(|child| child.is_element() && child.tag_name().name() == step)
            .collect();
    }
    for record in level {
        let mut row = Vec::with_capacity(settings.fields.len());
        for field in &settings.fields {
            let raw: Option<String> = match &field.item {
                Item::Attribute(name) => record.attribute(name.as_str()).map(str::to_string),
                Item::Child(name) => record
                    .children()
                    .find(|child| child.is_element() && child.tag_name().name() == name)
                    .map(|child| {
                        child
                            .descendants()
                            .filter(|node| node.is_text())
                            .filter_map(|node| node.text())
                            .collect()
                    }),
            };
            let raw = raw.unwrap_or_default();
            row.push(match field.storage {
                Storage::Integer if raw.is_empty() => Value::Null,
                Storage::Integer => Value::Integer(raw.trim().parse::<i64>().map_err(|_| {
                    bad(format!(
                        "{:?} in integer field {} is not a whole number",
                        raw, field.name
                    ))
                })?),
                _ => Value::string(raw),
            });
        }
        rows.push(row);
    }
    Ok(())
}
