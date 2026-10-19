//! SAS Export (`SASExportNode`, Export palette, catalog id `sas_export`): writes the input
//! [`Table`] to a SAS dataset file (`.sas7bdat`, `.sd2`, or `.ssd`) using [`dms_io::sas_export`]
//! and passes the input table through unchanged (`oracle/sas_export/*`).
//!
//! Supported settings (both `.str` XML element names and Python scripting property names from the
//! Modeler 19 *Scripting and Automation Guide* `sasexportnode` specification):
//!
//! | `.str` / scripting property | Meaning | Verified values |
//! |---|---|---|
//! | `exportFileName` / `full_filename` | Output path, resolved via [`ExecContext::export_path`] | `sasx_sas8.sas7bdat`, `sasx_sas7.sas7bdat`, `sasx_names_as_labels.sas7bdat`, `sasx_windows.sd2`, `sasx_unix.ssd` |
//! | `sasExportVariant` / `format` | SAS file format | `SAS7` / `SAS8` (default, `.sas7bdat`; scripting `SAS8` saves as `SAS7`), `windows` / `Windows` (`.sd2`), `UNIX` (`.ssd`) |
//! | `sasExportFieldType` / `export_names` | Variable name / label mode | `names` / `NamesAndLabels` (default), `headings` / `NamesAsLabels` |
//! | `autoGenerateImportNode` / `generate_import` | Auto-generate import node on canvas | `true` or `false` (UI-only; does not affect exported file bytes) |
//! | `executionMode` | Stream execution mode | `export_data` (or absent) |
//! | `writeMode` | File write mode | `overwrite` (or absent) |

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_io::sas_export::{
    self, SasExportError, SasExportFormat, SasExportNameMode, SasExportOptions,
};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::Table;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

struct SasExport {
    path: String,
    format: SasExportFormat,
    names: SasExportNameMode,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    reject_unknown(s)?;

    let path = first_text(
        s,
        &[
            "exportFileName",
            "full_filename",
            "fileName",
            "fullFileName",
            "sasFileName",
            "sasExportFileName",
        ],
    )
    .or_else(|| fallback_file_name(s))
    .ok_or_else(|| OperatorError::Settings("SAS export: exportFileName is missing".into()))?;

    match settings::text(s, "executionMode").as_deref().map(str::trim) {
        None | Some("") | Some("export_data") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "SAS export: GAP: executionMode {other:?} is not supported"
            )));
        }
    }

    match settings::text(s, "writeMode").as_deref().map(str::trim) {
        None | Some("") | Some("overwrite") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "SAS export: GAP: writeMode {other:?} is not supported"
            )));
        }
    }

    let format_raw = first_text(
        s,
        &[
            "sasExportVariant",
            "sasExportFileType",
            "fileFormat",
            "format",
            "sasFormat",
            "exportFormat",
            "fileType",
            "sasFileType",
        ],
    )
    .or_else(|| fallback_format(s));

    let format = match format_raw.as_deref().map(str::trim) {
        None | Some("") => SasExportFormat::Sas8,
        Some(v)
            if v.eq_ignore_ascii_case("SAS8")
                || v.eq_ignore_ascii_case("v8")
                || v.eq_ignore_ascii_case("sas_8")
                || v.eq_ignore_ascii_case("8") =>
        {
            SasExportFormat::Sas8
        }
        Some(v)
            if v.eq_ignore_ascii_case("SAS7")
                || v.eq_ignore_ascii_case("v7")
                || v.eq_ignore_ascii_case("sas_7")
                || v.eq_ignore_ascii_case("7")
                || v.eq_ignore_ascii_case("sas7bdat") =>
        {
            SasExportFormat::Sas7
        }
        Some(v)
            if v.eq_ignore_ascii_case("Windows")
                || v.eq_ignore_ascii_case("win")
                || v.eq_ignore_ascii_case("sd2")
                || v.eq_ignore_ascii_case("sas6_windows")
                || v.eq_ignore_ascii_case("v6_windows") =>
        {
            SasExportFormat::WindowsSd2
        }
        Some(v)
            if v.eq_ignore_ascii_case("UNIX")
                || v.eq_ignore_ascii_case("ssd")
                || v.eq_ignore_ascii_case("ssd01")
                || v.eq_ignore_ascii_case("sas6_unix")
                || v.eq_ignore_ascii_case("v6_unix") =>
        {
            SasExportFormat::UnixSsd
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "SAS export: GAP: format {other:?} is not supported"
            )));
        }
    };

    let names_raw = first_text(
        s,
        &[
            "sasExportFieldType",
            "exportNames",
            "export_names",
            "writeVariableNames",
            "variableNamesMode",
            "nameMode",
            "namesMode",
        ],
    )
    .or_else(|| fallback_name_mode(s));

    let names = match names_raw.as_deref().map(str::trim) {
        None | Some("") => SasExportNameMode::NamesAndLabels,
        Some(v)
            if v.eq_ignore_ascii_case("NamesAndLabels")
                || v.eq_ignore_ascii_case("names_and_labels")
                || v.eq_ignore_ascii_case("names")
                || v.eq_ignore_ascii_case("true") =>
        {
            SasExportNameMode::NamesAndLabels
        }
        Some(v)
            if v.eq_ignore_ascii_case("NamesAsLabels")
                || v.eq_ignore_ascii_case("names_as_labels")
                || v.eq_ignore_ascii_case("headings")
                || v.eq_ignore_ascii_case("labels")
                || v.eq_ignore_ascii_case("false") =>
        {
            SasExportNameMode::NamesAsLabels
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "SAS export: GAP: export_names {other:?} is not supported"
            )));
        }
    };

    let _ = settings::flag(s, "launchApplication")?;
    let _ = settings::flag(s, "autoGenerateImportNode")?;
    let _ = settings::flag(s, "generate_import")?;
    let _ = settings::flag(s, "cacheEnabled")?;
    let _ = settings::flag(s, "autoLabel")?;
    let _ = settings::flag(s, "enabled")?;
    let _ = settings::flag(s, "doReadValues")?;
    let _ = settings::flag(s, "scriptExecutionModeChange")?;
    let _ = settings::flag(s, "publishMetadata")?;
    let _ = settings::flag(s, "publishUseParameters")?;

    Ok(Box::new(SasExport {
        path,
        format,
        names,
    }))
}

fn reject_unknown(settings_el: Option<&Element>) -> Result<(), OperatorError> {
    let Some(settings_el) = settings_el else {
        return Ok(());
    };
    let mut unknown = Vec::new();
    for child in settings_el.child_elements() {
        if !is_known(&child.name.local) {
            unknown.push(format!("{}={:?}", child.name.local, child.text()));
        }
    }
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(OperatorError::Settings(format!(
            "SAS export: GAP: unknown setting {}",
            unknown.join(", ")
        )))
    }
}

fn is_known(name: &str) -> bool {
    matches!(
        name,
        "name"
            | "label"
            | "autoLabel"
            | "toolTipText"
            | "annotation"
            | "keywords"
            | "cacheEnabled"
            | "position"
            | "exportFileName"
            | "full_filename"
            | "fileName"
            | "fullFileName"
            | "sasFileName"
            | "sasExportFileName"
            | "executionMode"
            | "sasExportVariant"
            | "sasExportFileType"
            | "fileFormat"
            | "format"
            | "sasFormat"
            | "exportFormat"
            | "fileType"
            | "sasFileType"
            | "sasExportFieldType"
            | "exportNames"
            | "export_names"
            | "writeVariableNames"
            | "variableNamesMode"
            | "nameMode"
            | "namesMode"
            | "writeMode"
            | "launchApplication"
            | "launchApplicationType"
            | "autoGenerateImportNode"
            | "generate_import"
            | "publishMetadata"
            | "publishUseParameters"
            | "publish"
            | "publishParameters"
            | "publishTag"
            | "outputFileName"
            | "enabled"
            | "createdBy"
            | "creationDate"
            | "lastSavedAs"
            | "lastSavedBy"
            | "lastSavedDate"
            | "doReadValues"
            | "fieldMappingEssentialFields"
            | "filterTable"
            | "repositoryPath"
            | "repositoryVersionMarker"
            | "repositoryLastModified"
            | "scriptExecutionModeChange"
    )
}

impl Operator for SasExport {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("SAS export needs one input".into()))?;
        let path = context.export_path(&self.path);
        let dataset_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("DATA")
            .to_string();
        let options = SasExportOptions {
            format: self.format,
            names: self.names,
            dataset_name,
        };
        let bytes = sas_export::write_table(input, &options).map_err(export_error)?;
        write_file(&path, &bytes)?;
        Ok(input.as_ref().clone())
    }
}

fn export_error(error: SasExportError) -> OperatorError {
    match error {
        SasExportError::Settings(message) => OperatorError::Settings(message),
        SasExportError::Data(message) => OperatorError::Data(message),
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), OperatorError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| OperatorError::Io(format!("{}: {error}", parent.display())))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|error| OperatorError::Io(format!("{}: {error}", path.display())))?;
    file.write_all(bytes)
        .map_err(|error| OperatorError::Io(format!("{}: {error}", path.display())))?;
    Ok(())
}

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

fn fallback_file_name(s: Option<&Element>) -> Option<String> {
    let s = s?;
    for child in s.child_elements() {
        if let Some(v) = child_value(child) {
            let lower = v.to_ascii_lowercase();
            if lower.ends_with(".sas7bdat") || lower.ends_with(".sd2") || lower.ends_with(".ssd") {
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
            && (v.eq_ignore_ascii_case("sas8")
                || v.eq_ignore_ascii_case("sas7")
                || v.eq_ignore_ascii_case("windows")
                || v.eq_ignore_ascii_case("unix"))
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
            && (v.eq_ignore_ascii_case("NamesAndLabels") || v.eq_ignore_ascii_case("NamesAsLabels"))
        {
            return Some(v);
        }
    }
    None
}
