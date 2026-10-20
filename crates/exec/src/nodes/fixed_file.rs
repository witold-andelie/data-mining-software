//! Fixed File (Sources palette, `FixedFileImportNode`): reads a fixed-width text file into a
//! [`Table`] using [`dms_io::fixed`] (`oracle/sources/fixed_*`).
//!
//! Implemented: `inputFileName` (resolved via [`ExecContext::files`]), `fixedFileColumnSpecs`
//! (`columnName`, 1-based `start`, `length`), `recordLength`, `lineOriented` (default true),
//! `skipHeaderLines`, the text settings of `super::text_source` (trimming both ends by
//! default), and embedded `filterTable`/`typeTable` rules. `fieldNamesIncluded` does not apply:
//! the layout names the fields.
//!
//! Rules: Modeler refuses a layout whose fields are not in file order without overlaps ("Bad
//! format specified for file", `fixed_fields_reordered`).
//!
//! GAP: `fixed_fields_reordered` has fields both out of order and overlapping, so which of the
//! two Modeler refuses is unknown; we refuse both. `breakCasebyNewline` is rejected.

use super::text_source::{TextSettings, read_file};
use super::r#type::{SourceTables, apply_source_tables, parse_source_tables};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_io::fixed::{FixedColumn, FixedOptions, read_table};
use dms_io::text::Trim;
use dms_stream::StreamNode;
use dms_types::Table;
use std::sync::Arc;

struct FixedFile {
    file_name: String,
    columns: Vec<FixedColumn>,
    record_length: usize,
    line_oriented: bool,
    text: TextSettings,
    tables: SourceTables,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let bad = |what: String| OperatorError::Settings(format!("Fixed File: {what}"));
    let file_name = settings::text(s, "inputFileName")
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| bad("inputFileName is required".into()))?;
    if settings::flag(s, "breakCasebyNewline")?.unwrap_or(false) {
        return Err(bad("breakCasebyNewline is not supported yet".into()));
    }
    let count = |key: &str, value: Option<i64>| {
        usize::try_from(value.unwrap_or(0))
            .map_err(|_| bad(format!("{key} must be non-negative, got {value:?}")))
    };
    let record_length = count("recordLength", settings::integer(s, "recordLength")?)?;
    let line_oriented = settings::flag(s, "lineOriented")?.unwrap_or(true);
    if !line_oriented && record_length == 0 {
        return Err(bad(
            "recordLength must be positive for records that are not lines".into(),
        ));
    }

    let mut columns: Vec<FixedColumn> = Vec::new();
    for entry in settings::children(s, "fixedFileColumnSpecs/entries", "entry") {
        let entry = Some(entry);
        let name = settings::text(entry, "columnName").unwrap_or_default();
        let start = count("start", settings::integer(entry, "start")?)?;
        let length = count("length", settings::integer(entry, "length")?)?;
        if name.is_empty() || start == 0 || length == 0 {
            return Err(bad(format!(
                "field {name:?} needs a name, a start from 1 and a positive length"
            )));
        }
        if let Some(previous) = columns.last()
            && start < previous.start + previous.length
        {
            return Err(bad(format!(
                "field {name:?} starts before field {:?} ends (bad format)",
                previous.name
            )));
        }
        columns.push(FixedColumn {
            name,
            start,
            length,
        });
    }
    if columns.is_empty() {
        return Err(bad("no fields (fixedFileColumnSpecs)".into()));
    }

    let mut text = TextSettings::read(s, "Fixed File", Trim::Both)?;
    text.skip_lines(count(
        "skipHeaderLines",
        settings::integer(s, "skipHeaderLines")?,
    )?);
    Ok(Box::new(FixedFile {
        file_name,
        columns,
        record_length,
        line_oriented,
        text,
        tables: parse_source_tables(s)?,
    }))
}

impl Operator for FixedFile {
    fn arity(&self) -> Arity {
        Arity::Exactly(0)
    }

    fn run(&self, _inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let (path, bytes) = read_file(context, &self.file_name)?;
        let options = FixedOptions {
            columns: self.columns.clone(),
            record_length: self.record_length,
            line_oriented: self.line_oriented,
            text: self.text.resolve(context)?,
        };
        let table = read_table(&bytes, &options)
            .map_err(|e| OperatorError::Data(format!("{}: {e}", path.display())))?;
        apply_source_tables(&self.tables, table)
    }
}
