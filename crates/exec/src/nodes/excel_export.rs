//! Excel export (`ExcelNode`): writes the input table to an `.xlsx` workbook and passes that
//! table through unchanged.
//!
//! Implemented settings (element names under the node's `properties`; the scripting names are
//! the `excelexportnode` properties):
//!
//! | element | scripting property | accepted values |
//! |---|---|---|
//! | `excelFilePath` | `full_filename` | path; the file written is [`ExecContext::export_path`] |
//! | `excelFileType` | `excel_file_type` | `v2007` (the file form of `Excel2007`) |
//! | `excelExportType` | `export_mode` | `create` (the file form of `Create`) |
//! | `includeFieldNames` | `inc_field_names` | `true` (default when the element is absent) or `false` |
//! | `rangeStart` | `start_cell` | an `A1`-style cell; `A1` when the element is absent |
//! | `worksheetName` | `worksheet_name` | sheet tab, 1 to 31 characters; `Sheet1` when absent |
//! | `worksheetMode` | — | `index` |
//! | `worksheetIndex` | — | `0` (the single sheet this node creates) |
//! | `includeCellNotes` | `inc_labels_as_cell_notes` | `true` or `false` |
//! | `launchApplication` | `launch_application` | `true` or `false`; Excel is not launched |
//! | `executionMode` | — | `export_data` when it is saved |
//!
//! `excelExportType` other than `create`, `excelFileType` other than `v2007`, `worksheetMode`
//! other than `index`, `worksheetIndex` other than `0`, a start cell that is not on the sheet,
//! and any `properties` child that is not one of these or the shared node fields below is
//! rejected with [`OperatorError::Settings`].
//!
//! Shared node fields that do not change the workbook are ignored: `keywords`,
//! `autoGenerateImportNode`, `publishMetadata`, `lastSavedAs`, `enabled`,
//! `fieldMappingEssentialFields`, `repositoryPath`, `repositoryVersionMarker`, `doReadValues`,
//! `publishUseParameters`, `toolTipText`, `annotation`, `outputFileName`, `publishTag`,
//! `lastSavedBy`, `lastSavedDate`, `label`, `creationDate`, `repositoryLastModified`,
//! `publishParameters`, `cacheEnabled`, `scriptExecutionModeChange`, `autoLabel`, `createdBy`,
//! `name`, `position`.
//!
//! Rules, each one visible in a probe export under `oracle/exports/`:
//!
//! - Both probes save `rangeStart` `A1`, `worksheetName` `Sheet1`, `worksheetMode` `index`,
//!   `worksheetIndex` `0`, `excelFileType` `v2007`, `excelExportType` `create`,
//!   `launchApplication` `false` and `includeCellNotes` `true`. They differ only in
//!   `includeFieldNames`.
//! - `includeFieldNames` `true` writes the field names as inline strings on the first row
//!   (`excel_field_names`, dimension `A1:G19`). `false` starts with the first record
//!   (`excel_no_field_names`, dimension `A1:G18`).
//! - Integers use number format `0` (`iv` is `10`, `-10`, `0`, with no decimal point). Reals
//!   use General and Java `Double.toString` (`rv` is `-3.5`, `0.0`, `10.0`,
//!   `0.123456789012345`). The stream saves 15 decimal places; the real is still the full
//!   double text, not 15 fixed places.
//! - Dates are Excel serial days with format `d-mmm-yy` (`dt` `2020-01-15` is `43845.0`,
//!   `1999-12-31` is `36525.0`). 1970-01-01 is serial 25569. A missing date keeps that format
//!   and an empty value (rows `id` 7 and `id` 17).
//! - A missing number, a missing string and an empty string are all an empty numeric cell
//!   (`id` 6 field `s`, `id` 8 fields `grp` and `flg`, `id` 11 fields `iv`, `rv` and `s`).
//! - Strings are inline strings. Leading or trailing spaces set `xml:space="preserve"`
//!   (`id` 16, `  spaced  `). A tab is removed (`id` 9, the source text `tab` + tab + `here`
//!   is `tabhere`). Flag values stay the strings `T` and `F`, not Excel booleans.
//! - `includeCellNotes` is `true` in both probes and the Type node carries annotations
//!   (`Group`, `Real value`, `Text`, `When`). Neither workbook has a comments part, so this
//!   flag does not add cell notes for those annotations.
//! - `launchApplication` does not start a program. `excelExportType` `create` replaces the
//!   file.
//! - The probe source cells `say ""hi""` and `q ""and"" semi;colon,comma` are `say hi` and
//!   `q and semi;colon,comma` in both workbooks, because the variable-file import dropped
//!   those quote characters before the export ran. This node writes the string it receives.
//!
//! Time, timestamp and list values are rejected at run time: the probes do not contain them.
//! Dates before 1900-03-01 use Excel's serial with the false 1900 leap day removed; that shift
//! is inferred (the probes only contain 1999–2021).

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_io::xlsx::{self, WriteOptions, XlsxError};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::Table;
use std::path::Path;
use std::sync::Arc;

const MAX_COLUMN: u32 = 16_384;
const MAX_ROW: u32 = 1_048_576;

/// Settings this node reads, plus the shared fields every saved node carries.
const KNOWN: &[&str] = &[
    "rangeStart",
    "keywords",
    "autoGenerateImportNode",
    "publishMetadata",
    "lastSavedAs",
    "enabled",
    "fieldMappingEssentialFields",
    "repositoryPath",
    "worksheetMode",
    "excelFilePath",
    "repositoryVersionMarker",
    "doReadValues",
    "publishUseParameters",
    "toolTipText",
    "annotation",
    "outputFileName",
    "publishTag",
    "includeCellNotes",
    "lastSavedBy",
    "lastSavedDate",
    "executionMode",
    "label",
    "excelExportType",
    "creationDate",
    "repositoryLastModified",
    "excelFileType",
    "publishParameters",
    "cacheEnabled",
    "scriptExecutionModeChange",
    "autoLabel",
    "createdBy",
    "includeFieldNames",
    "name",
    "worksheetIndex",
    "launchApplication",
    "position",
    "worksheetName",
];

struct ExcelExport {
    path: String,
    sheet_name: String,
    start_column: u32,
    start_row: u32,
    field_names: bool,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    if let Some(settings) = s {
        reject_unknown(settings)?;
    }

    let path = settings::text(s, "excelFilePath")
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .ok_or_else(|| OperatorError::Settings("Excel: excelFilePath is missing".to_string()))?;

    match settings::text(s, "executionMode").as_deref().map(str::trim) {
        None | Some("") | Some("export_data") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Excel: executionMode {other:?} is not implemented"
            )));
        }
    }
    match settings::text(s, "excelFileType").as_deref().map(str::trim) {
        None | Some("") | Some("v2007") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Excel: excelFileType {other:?} is not implemented"
            )));
        }
    }
    match settings::text(s, "excelExportType")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("create") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Excel: excelExportType {other:?} is not implemented"
            )));
        }
    }
    match settings::text(s, "worksheetMode").as_deref().map(str::trim) {
        None | Some("") | Some("index") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Excel: worksheetMode {other:?} is not implemented"
            )));
        }
    }
    match settings::integer(s, "worksheetIndex")? {
        None | Some(0) => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Excel: worksheetIndex {other} is not implemented"
            )));
        }
    }
    // Accepted and recorded. `true` is what both probes save, and those workbooks have no
    // comments part. Launching Excel is a desktop action this process does not perform.
    settings::flag(s, "includeCellNotes")?;
    settings::flag(s, "launchApplication")?;

    let field_names = settings::flag(s, "includeFieldNames")?.unwrap_or(true);
    let start = settings::text(s, "rangeStart").unwrap_or_else(|| "A1".to_string());
    let (start_column, start_row) = parse_start_cell(start.trim())?;
    let sheet_name = match settings::text(s, "worksheetName") {
        None => "Sheet1".to_string(),
        Some(name) if name.is_empty() => {
            return Err(OperatorError::Settings(
                "Excel: worksheetName is empty".into(),
            ));
        }
        Some(name) => name,
    };
    validate_sheet_name(&sheet_name)?;

    Ok(Box::new(ExcelExport {
        path,
        sheet_name,
        start_column,
        start_row,
        field_names,
    }))
}

fn reject_unknown(settings: &Element) -> Result<(), OperatorError> {
    for child in settings.child_elements() {
        let name: &str = &child.name.local;
        if !KNOWN.contains(&name) {
            return Err(OperatorError::Settings(format!(
                "Excel: setting {name:?} is not implemented"
            )));
        }
    }
    Ok(())
}

fn validate_sheet_name(name: &str) -> Result<(), OperatorError> {
    let count = name.chars().count();
    if count == 0 || count > 31 {
        return Err(OperatorError::Settings(format!(
            "Excel: worksheetName {name:?} must be 1 to 31 characters"
        )));
    }
    if name
        .chars()
        .any(|c| c < ' ' || matches!(c, '\\' | '/' | '?' | '*' | '[' | ']' | ':'))
    {
        return Err(OperatorError::Settings(format!(
            "Excel: worksheetName {name:?} contains a character Excel rejects"
        )));
    }
    Ok(())
}

/// `A1` through `XFD1048576`. Column letters are case-insensitive.
fn parse_start_cell(text: &str) -> Result<(u32, u32), OperatorError> {
    let invalid = || OperatorError::Settings(format!("Excel: rangeStart {text:?} is not a cell"));
    let mut column: u32 = 0;
    let mut index = 0usize;
    let bytes = text.as_bytes();
    while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
        if index >= 3 {
            return Err(invalid());
        }
        let letter = bytes[index].to_ascii_uppercase();
        let value = u32::from(letter.saturating_sub(b'A')).saturating_add(1);
        column = column
            .checked_mul(26)
            .and_then(|next| next.checked_add(value))
            .filter(|next| *next <= MAX_COLUMN)
            .ok_or_else(invalid)?;
        index += 1;
    }
    if column == 0 {
        return Err(invalid());
    }
    let Some(rest) = text.get(index..) else {
        return Err(invalid());
    };
    if rest.is_empty() || rest.len() > 7 || !rest.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    let row: u32 = rest.parse().map_err(|_| invalid())?;
    if row == 0 || row > MAX_ROW {
        return Err(invalid());
    }
    Ok((column, row))
}

impl Operator for ExcelExport {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Excel export needs one input".into()))?;
        let bytes = xlsx::write_table(
            input,
            &WriteOptions {
                sheet_name: self.sheet_name.clone(),
                start_column: self.start_column,
                start_row: self.start_row,
                field_names: self.field_names,
            },
        )
        .map_err(map_xlsx)?;
        let path = context.export_path(&self.path);
        write_bytes(&path, &bytes)?;
        Ok(input.as_ref().clone())
    }
}

fn map_xlsx(error: XlsxError) -> OperatorError {
    let message = format!("Excel: {error}");
    match error {
        XlsxError::Zip(_) => OperatorError::Io(message),
        XlsxError::Value(_) | XlsxError::Grid(_) => OperatorError::Data(message),
        XlsxError::Sheet(_) => OperatorError::Settings(message),
    }
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), OperatorError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| OperatorError::Io(format!("{}: {error}", parent.display())))?;
    }
    std::fs::write(path, bytes)
        .map_err(|error| OperatorError::Io(format!("{}: {error}", path.display())))
}
