//! Excel source (`ExcelInputNode`, catalog `excel_import`): reads an `.xlsx` workbook into a
//! [`Table`] via [`dms_io::xlsx`], whose notes list the reading rules and their oracles
//! (`oracle/sources/excel_*`, `oracle/excel_more/*`).
//!
//! Implemented settings (element names under the node's `properties`; scripting names are the
//! `excelimportnode` properties from the probe `xtask/oracle/probes/source_streams.py`):
//!
//! | element | scripting property | accepted values |
//! |---|---|---|
//! | `inputFileName` | `full_filename` | path to `.xlsx`, resolved via [`ExecContext::files`] |
//! | `fieldNamesIncluded` | `read_field_names` | `true` (first row are headers, `excel_default`) / `false` (first row is data with `C1…` names, `excel_no_headers`) |
//! | `worksheetMode` | `worksheet_mode` | `index` (0-based `worksheetIndex`; `excel_default` reads index 0) / `name` (exact `worksheetName`, every `excel_more` case) |
//! | `worksheetIndex` | `worksheet_index` | non-negative, first sheet is 0 |
//! | `worksheetName` | `worksheet_name` | exact tab name when `worksheetMode` is `name` |
//! | `dataRange` | `data_range_mode` | `firstNonBlank` (bounding box of non-blank cells) / `explicitRange` (`rangeStart`..`rangeEnd`) |
//! | `rangeStart`, `rangeEnd` | `explicit_range_start`, `explicit_range_end` | A1 (`C4`, `E9`, `more_explicit_numeric`) or R1C1 (`R3C2`; parsed, no matching oracle) |
//! | `blankRows` | `blank_rows` | `stopReading` (end at the first blank row, `more_blank_noempty_stop`) / `returnBlankRows` (keep blank rows in place, `more_blank_noempty_return`) |
//! | `scanLineCount` | `lines_to_scan` | string-vs-numeric horizon, default 200 (`excel_scan_1` saved 200 vs `excel_scan_250` 250) |
//! | `excelFileType` | `excel_file_type` | `v2007` (`.xlsx`; `Excel2007` scripting spelling accepted) |
//! | `useNamedRange`, `namedRange` | `use_named_range`, `named_range` | only `false`/empty; `true` is a `GAP:` error (`excel_named_range` has no oracle data) |
//! | `filterTable`, `typeTable` | — | embedded Filter and Type rules via `parse_source_tables` |
//!
//! GAPs (explicit `GAP:` errors, never silently ignored):
//! - `useNamedRange = true` (`excel_named_range`; Modeler drops the server connection, no data).
//! - `.xls` files and `excelFileType` other than `v2007`.
//! - `doReadValues = true`.
//! - A sheet with a `<row>` element without cells (see [`dms_io::xlsx`]).
//!
//! Display-only settings are ignored: `refreshLinkState`, `keywords`, `lastSavedAs`,
//! `enabled`, `fieldMappingEssentialFields`, `repositoryPath`, `repositoryVersionMarker`,
//! `doReadValues` when false, `toolTipText`, `annotation`, `typeTable`/`filterTable` handled
//! separately, `publishTag`, `lastSavedBy`, `builderNodeId`, `lastSavedDate`, `label`,
//! `creationDate`, `repositoryLastModified`, `repalcebuilderNodeId` (saved typo),
//! `cacheEnabled`, `scriptExecutionModeChange`, `autoLabel`, `createdBy`, `name`, `position`.

use super::text_source::read_file;
use super::r#type::{SourceTables, apply_source_tables, parse_source_tables};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_io::xlsx::{BlankRows, ExcelOptions, RangeSelector, SheetSelector, parse_range_bound};
use dms_stream::StreamNode;
use dms_types::Table;
use std::sync::Arc;

struct ExcelImport {
    file_name: String,
    options: ExcelOptions,
    tables: SourceTables,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let bad = |what: String| OperatorError::Settings(format!("Excel: {what}"));
    let gap = |what: String| OperatorError::Settings(format!("Excel: GAP: {what}"));

    let file_name = settings::text(s, "inputFileName")
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .ok_or_else(|| bad("inputFileName is required".into()))?;
    if file_name.to_ascii_lowercase().ends_with(".xls")
        && !file_name.to_ascii_lowercase().ends_with(".xlsx")
    {
        return Err(gap(".xls files are not supported".into()));
    }
    if !file_name.to_ascii_lowercase().ends_with(".xlsx") {
        // Only the workbooks the oracles use are supported; anything else would need evidence.
        return Err(gap(format!("file {file_name:?} is not an .xlsx workbook")));
    }

    match settings::text(s, "excelFileType").as_deref().map(str::trim) {
        None | Some("") | Some("v2007") | Some("V2007") | Some("Excel2007") => {}
        Some(other) => {
            return Err(gap(format!("excelFileType {other:?} is not supported")));
        }
    }
    if settings::flag(s, "doReadValues")?.unwrap_or(false) {
        return Err(gap("doReadValues=true is not supported".into()));
    }
    if settings::flag(s, "useNamedRange")?.unwrap_or(false) {
        return Err(gap(
            "useNamedRange=true (named ranges) is not supported; excel_named_range has no oracle data".into(),
        ));
    }

    let field_names = settings::flag(s, "fieldNamesIncluded")?.unwrap_or(true);

    let worksheet_mode_raw =
        settings::text(s, "worksheetMode").unwrap_or_else(|| "index".to_string());
    let worksheet_mode = worksheet_mode_raw.trim();
    let sheet = if worksheet_mode.eq_ignore_ascii_case("index") {
        let raw = settings::integer(s, "worksheetIndex")?.unwrap_or(0);
        if raw < 0 {
            return Err(bad(format!(
                "worksheetIndex must be non-negative, got {raw}"
            )));
        }
        let index =
            usize::try_from(raw).map_err(|_| bad(format!("worksheetIndex {raw} does not fit")))?;
        SheetSelector::Index(index)
    } else if worksheet_mode.eq_ignore_ascii_case("name") {
        let name = settings::text(s, "worksheetName").unwrap_or_default();
        if name.is_empty() {
            return Err(bad(
                "worksheetName is required when worksheetMode is name".into()
            ));
        }
        SheetSelector::Name(name)
    } else {
        return Err(bad(format!("unknown worksheetMode {worksheet_mode:?}")));
    };

    let data_range_raw =
        settings::text(s, "dataRange").unwrap_or_else(|| "firstNonBlank".to_string());
    let data_range = data_range_raw.trim();
    let range_start = settings::text(s, "rangeStart").unwrap_or_default();
    let range_end = settings::text(s, "rangeEnd").unwrap_or_default();
    let range = if data_range.eq_ignore_ascii_case("firstNonBlank") {
        if !range_start.trim().is_empty() || !range_end.trim().is_empty() {
            return Err(bad(
                "rangeStart/rangeEnd must be empty when dataRange is firstNonBlank".into(),
            ));
        }
        RangeSelector::FirstNonBlank
    } else if data_range.eq_ignore_ascii_case("explicitRange") {
        if range_start.trim().is_empty() || range_end.trim().is_empty() {
            return Err(bad(
                "rangeStart and rangeEnd are required when dataRange is explicitRange".into(),
            ));
        }
        let start =
            parse_range_bound(range_start.trim()).map_err(|e| bad(format!("rangeStart: {e}")))?;
        let end = parse_range_bound(range_end.trim()).map_err(|e| bad(format!("rangeEnd: {e}")))?;
        RangeSelector::Explicit { start, end }
    } else {
        return Err(bad(format!("unknown dataRange {data_range:?}")));
    };

    let blank_rows = match settings::text(s, "blankRows").as_deref().map(str::trim) {
        None | Some("") | Some("stopReading") | Some("StopReading") => BlankRows::StopReading,
        Some("returnBlankRows") | Some("ReturnBlankRows") => BlankRows::ReturnBlankRows,
        Some(other) => return Err(bad(format!("unknown blankRows {other:?}"))),
    };

    let scan_raw = settings::integer(s, "scanLineCount")?.unwrap_or(200);
    if scan_raw < 0 {
        return Err(bad(format!(
            "scanLineCount must be non-negative, got {scan_raw}"
        )));
    }
    let scan_lines = usize::try_from(scan_raw).unwrap_or(200).max(1);

    Ok(Box::new(ExcelImport {
        file_name,
        options: ExcelOptions {
            sheet,
            field_names,
            range,
            blank_rows,
            scan_lines,
        },
        tables: parse_source_tables(s)?,
    }))
}

impl Operator for ExcelImport {
    fn arity(&self) -> Arity {
        Arity::Exactly(0)
    }

    fn run(&self, _inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let (path, bytes) = read_file(context, &self.file_name)?;
        let table = dms_io::xlsx::read_table(&bytes, &self.options)
            .map_err(|e| OperatorError::Data(format!("{}: {e}", path.display())))?;
        apply_source_tables(&self.tables, table)
    }
}
