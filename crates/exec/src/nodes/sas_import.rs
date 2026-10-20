//! SAS File (Sources palette, `SASInputNode`): reads a SAS dataset (`.sas7bdat` or SAS Transport
//! `XPORT v5` `.xpt`) into a [`Table`] using [`dms_io::sas`] (`oracle/sas/*`).
//!
//! Supported settings (both internal `.str` XML element names and Python scripting property names
//! from Table 54 of the Modeler 19 *Scripting and Automation Guide*):
//!
//! | `.str` / scripting property | Meaning | Verified values |
//! |---|---|---|
//! | `inputFileName` / `full_filename` | Path to the SAS file, resolved via [`ExecContext::files`] | `sastest.sas7bdat`, `sastest.xpt` (`sas9_names`, `sas9_labels`, `sas_transport`) |
//! | `fileFormat` / `format` | SAS file format | `SAS7` / `SAS9` (`.sas7bdat`, default), `Transport` (`XPORT v5` `.xpt`) |
//! | `memberName` / `member_name` | Member name inside a `Transport` file | `SASTEST` (`sas_transport`) |
//! | `readVariableNames` / `importNames` / `import_names` | Variable naming mode | `true` / `NamesAndLabels` (`sas9_names`, `sas_transport`), `false` / `LabelsAsNames` / `LabelsasNames` (`sas9_labels`) |
//! | `readFormats` / `read_formats` | Read value labels from a SAS format catalog | `false` (default); `true` returns a `GAP:` error |
//! | `filterTable`, `typeTable` | Embedded Filter and Type settings on the source node | Handled via `super::r#type::parse_source_tables` and `super::r#type::apply_source_tables` |
//!
//! Verified rules:
//! - `SAS9` and `SAS7` both select the 64-bit little-endian `sas7bdat` reader (`sas9_names` and
//!   `sas9_labels` set `format = "SAS9"` in scripting, which Modeler saves/reports as `SAS7`).
//! - `NamesAndLabels` uses SAS variable names as field names and attaches non-empty variable
//!   labels to [`dms_types::Field::label`] (`sas9_names`, `sas_transport`).
//! - `LabelsAsNames` uses non-empty variable labels as field names and names unlabeled variables
//!   `field<1-based column position>` (`field2`, `field5`, `field6` in `sas9_labels`).
//! - `Transport` reads the named member (`member_name = "SASTEST"` in `sas_transport`, or the
//!   first member when omitted) from an `XPORT v5` file.
//!
//! GAPs (returned as explicit `GAP:` errors):
//! - `read_formats = true` / `full_format_filename` (reading SAS format catalogs for value labels).
//! - Legacy SAS formats (`Windows`, `UNIX`, `SAS6`, `SAS8`) and `XPORT v8`.

use super::text_source::read_file;
use super::r#type::{SourceTables, apply_source_tables, parse_source_tables};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_io::sas::{NameMode, SasFormat, SasOptions, read_table};
use dms_strfmt::fastinfoset::Element;
use dms_types::Table;
use std::sync::Arc;

struct SasImport {
    file_name: String,
    options: SasOptions,
    tables: SourceTables,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let bad = |what: String| OperatorError::Settings(format!("SAS File: {what}"));

    let file_name = first_text(
        s,
        &[
            "inputFileName",
            "full_filename",
            "fileName",
            "fullFileName",
            "sasFileName",
        ],
    )
    .or_else(|| fallback_file_name(s))
    .ok_or_else(|| {
        bad(format!(
            "inputFileName / full_filename is required (children: {})",
            child_summary(s)
        ))
    })?;

    let read_formats_raw = first_text(
        s,
        &[
            "readFormats",
            "read_formats",
            "useFormatFile",
            "readFormatFile",
        ],
    );
    if let Some(val) = read_formats_raw.as_deref().map(str::trim) {
        if val.eq_ignore_ascii_case("true") {
            return Err(bad(
                "GAP: read_formats=true (reading SAS format catalogs) is not supported".into(),
            ));
        }
        if !val.is_empty() && !val.eq_ignore_ascii_case("false") {
            return Err(bad(format!("unknown read_formats value {val:?}")));
        }
    }

    let format_raw = first_text(
        s,
        &[
            "fileFormat",
            "format",
            "sasFormat",
            "fileType",
            "inputFileFormat",
            "sasFileType",
        ],
    )
    .or_else(|| fallback_format(s));

    let format = match format_raw.as_deref().map(str::trim) {
        None | Some("") => SasFormat::Sas7bdat,
        Some(val)
            if val.eq_ignore_ascii_case("sas7")
                || val.eq_ignore_ascii_case("sas9")
                || val.eq_ignore_ascii_case("sas7bdat") =>
        {
            SasFormat::Sas7bdat
        }
        Some(val)
            if val.eq_ignore_ascii_case("transport")
                || val.eq_ignore_ascii_case("xport")
                || val.eq_ignore_ascii_case("xpt") =>
        {
            SasFormat::Transport
        }
        Some(other) => {
            return Err(bad(format!(
                "GAP: SAS format {other:?} is not supported (only SAS7/SAS9 and Transport)"
            )));
        }
    };

    let member_name = first_text(
        s,
        &[
            "memberName",
            "member_name",
            "member",
            "datasetName",
            "tableName",
        ],
    )
    .or_else(|| fallback_member_name(s));

    let names = parse_name_mode(s, &bad)?;

    Ok(Box::new(SasImport {
        file_name,
        options: SasOptions {
            format,
            member_name,
            names,
        },
        tables: parse_source_tables(s)?,
    }))
}

impl Operator for SasImport {
    fn arity(&self) -> Arity {
        Arity::Exactly(0)
    }

    fn run(&self, _inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let (path, bytes) = read_file(context, &self.file_name)?;
        let table = read_table(&bytes, &self.options)
            .map_err(|e| OperatorError::Data(format!("{}: {e}", path.display())))?;
        apply_source_tables(&self.tables, table)
    }
}

use dms_stream::StreamNode;

fn first_text(s: Option<&Element>, keys: &[&str]) -> Option<String> {
    for &key in keys {
        if let Some(val) = settings::text(s, key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn parse_name_mode(
    s: Option<&Element>,
    bad: &dyn Fn(String) -> OperatorError,
) -> Result<NameMode, OperatorError> {
    if let Some(mode) = first_text(
        s,
        &[
            "readVariableNames",
            "importNames",
            "import_names",
            "variableNamesMode",
            "nameMode",
            "namesMode",
        ],
    )
    .or_else(|| fallback_name_mode(s))
    {
        if mode.eq_ignore_ascii_case("NamesAndLabels")
            || mode.eq_ignore_ascii_case("names_and_labels")
            || mode.eq_ignore_ascii_case("names")
            || mode.eq_ignore_ascii_case("true")
        {
            return Ok(NameMode::NamesAndLabels);
        }
        if mode.eq_ignore_ascii_case("LabelsAsNames")
            || mode.eq_ignore_ascii_case("labels_as_names")
            || mode.eq_ignore_ascii_case("labels")
            || mode.eq_ignore_ascii_case("false")
        {
            return Ok(NameMode::LabelsAsNames);
        }
        return Err(bad(format!(
            "unknown readVariableNames / import_names {mode:?}"
        )));
    }
    Ok(NameMode::NamesAndLabels)
}

fn child_value(child: &Element) -> Option<String> {
    let text = child.text();
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        return Some(trimmed.to_string());
    }
    child
        .attribute("value")
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn child_summary(s: Option<&Element>) -> String {
    let Some(s) = s else {
        return "<none>".to_string();
    };
    s.child_elements()
        .map(|c| format!("{}={:?}", c.name.local, c.text()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn fallback_file_name(s: Option<&Element>) -> Option<String> {
    let s = s?;
    for child in s.child_elements() {
        if let Some(v) = child_value(child) {
            let lower = v.to_ascii_lowercase();
            if lower.ends_with(".sas7bdat") || lower.ends_with(".xpt") || lower.ends_with(".sd2") {
                return Some(v);
            }
        }
    }
    None
}

fn fallback_format(s: Option<&Element>) -> Option<String> {
    let s = s?;
    for child in s.child_elements() {
        if let Some(v) = child_value(child)
            && (v.eq_ignore_ascii_case("transport")
                || v.eq_ignore_ascii_case("sas7")
                || v.eq_ignore_ascii_case("sas9")
                || v.eq_ignore_ascii_case("sas6")
                || v.eq_ignore_ascii_case("sas8")
                || v.eq_ignore_ascii_case("windows")
                || v.eq_ignore_ascii_case("unix"))
        {
            return Some(v);
        }
    }
    None
}

fn fallback_member_name(s: Option<&Element>) -> Option<String> {
    let s = s?;
    for child in s.child_elements() {
        let name_lower = child.name.local.to_ascii_lowercase();
        if name_lower.contains("member")
            && let Some(v) = child_value(child)
        {
            return Some(v);
        }
    }
    None
}

fn fallback_name_mode(s: Option<&Element>) -> Option<String> {
    let s = s?;
    for child in s.child_elements() {
        if let Some(v) = child_value(child)
            && (v.eq_ignore_ascii_case("NamesAndLabels") || v.eq_ignore_ascii_case("LabelsAsNames"))
        {
            return Some(v);
        }
    }
    None
}
