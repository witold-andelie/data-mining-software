//! Statistics Export (`SPSSExportNode`): writes the input table to an IBM SPSS Statistics
//! `.sav` file and passes that table through unchanged.
//!
//! Implemented settings (element names under the node's `properties`, the file form of the
//! `statisticsexportnode` scripting properties):
//!
//! | element | scripting property | accepted values |
//! |---|---|---|
//! | `exportFileName` | `full_filename` | path; the file written is [`ExecContext::export_path`] |
//! | `spssExportFileType` | `file_type` | `sav` (default). `zsav` is rejected |
//! | `spssExportFieldType` | `export_names` | `names` / `NamesAndLabels` (default), `labels` / `NamesAsLabels` |
//! | `writeMode` | (not in the scripting table) | `overwrite` (default). Anything else is rejected |
//! | `executionMode` | | `export_data` when it is saved |
//! | `encryptFileWithPassword` | `encrypt_file` | missing or `false`. `true` is rejected |
//! | `password` | `password` | ignored while encryption is off. The probes save the element empty |
//! | `launchApplication` | `launch_application` | `true` or `false`. It does not change the file |
//! | `launchApplicationType` | | missing or `SPSS` |
//! | `autoGenerateImportNode` | `generate_import` | `true` or `false`. It does not change the file |
//!
//! Publish settings (`publish`, `publishMetadata`, `publishParameters`, `publishUseParameters`,
//! `publishTag`, `outputFileName`) and the common node properties (`name`, `label`, `autoLabel`,
//! `toolTipText`, `annotation`, `keywords`, `cacheEnabled`, `enabled`, `position`,
//! `createdBy`, `creationDate`, `lastSavedAs`, `lastSavedBy`, `lastSavedDate`,
//! `doReadValues`, `fieldMappingEssentialFields`, `filterTable`, `repositoryPath`,
//! `repositoryVersionMarker`, `repositoryLastModified`, `scriptExecutionModeChange`) do not
//! change the file. Any other child of `properties` is rejected, so an unimplemented setting
//! fails `build`.
//!
//! Decimal places come from [`ExecContext::decimal_places`] (the stream's `realDecimalPlaces`,
//! 15 in both probes). The date print format comes from the stream's `dateFormat`. Per-field
//! export places and date formats are not on the table.
//!
//! Rules, each one visible in a probe export under `oracle/exports/`. The conversion itself is
//! documented on [`dms_io::sav_export`].
//!
//! - `names` (`stat_default`) writes short names `ID`, `GRP`, `IV`, `RV`, `S`, `FLG`, `DT` and
//!   long names `id`, `grp`, `iv`, `rv`, `s`, `flg`, `dt`. Variable labels are the descriptions
//!   "Group", "Real value", "Text" and "When"; `id`, `iv` and `flg` have none.
//! - `labels` (`stat_names_as_labels`) writes `VAR00001`..`VAR00007` for both names and uses the
//!   field name as the variable label. The case data is the same file as `stat_default`.
//! - Both files are UTF-8, bytecode-compressed, bias 100, with the SPSS 31 / `spssio32.dll`
//!   machine-info records, extended case count `(1, 18)`, and no end-of-file bytecode.
//! - Integers are F10.0, the real is F21.15, strings are A6 / A6 / A88, and the date is SDATE10
//!   (`stat_default`). `2020-01-15` is the SPSS number `13798425600`.
//! - `grp` value labels are a blank and then A, B, C, D. `flg` value labels are a blank and T,
//!   not F (`stat_default`).
//! - Roles (`stat_default`): `id` and `s` are none (`3`); every other field is input (`0`).
//!   `s` is typeless, so it is none even though the stream direction is input.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_io::sav_export::{self, ExportOptions, NameMode, SavExportError};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::Table;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

struct StatisticsExport {
    path: String,
    names: NameMode,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let settings_el = node.settings.as_ref();
    reject_unknown(settings_el)?;

    let path = settings::text(settings_el, "exportFileName")
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            OperatorError::Settings("Statistics export: exportFileName is missing".into())
        })?;

    match settings::text(settings_el, "executionMode")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("export_data") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Statistics export: executionMode {other:?} is not implemented"
            )));
        }
    }

    match settings::text(settings_el, "spssExportFileType")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("sav") => {}
        Some("zsav") => {
            return Err(OperatorError::Settings(
                "Statistics export: spssExportFileType zsav is not implemented".into(),
            ));
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Statistics export: unknown spssExportFileType {other:?}"
            )));
        }
    }

    let names = match settings::text(settings_el, "spssExportFieldType")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("names") | Some("NamesAndLabels") => NameMode::NamesAndLabels,
        Some("labels") | Some("NamesAsLabels") => NameMode::NamesAsLabels,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Statistics export: unknown spssExportFieldType {other:?}"
            )));
        }
    };

    match settings::text(settings_el, "writeMode")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("overwrite") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Statistics export: writeMode {other:?} is not implemented"
            )));
        }
    }

    if settings::flag(settings_el, "encryptFileWithPassword")? == Some(true) {
        return Err(OperatorError::Settings(
            "Statistics export: encryptFileWithPassword is not implemented".into(),
        ));
    }
    // Present on both probes. None of these change the bytes of the file.
    let _ = settings::flag(settings_el, "launchApplication")?;
    let _ = settings::flag(settings_el, "autoGenerateImportNode")?;
    let _ = settings::flag(settings_el, "cacheEnabled")?;
    let _ = settings::flag(settings_el, "autoLabel")?;
    let _ = settings::flag(settings_el, "enabled")?;
    let _ = settings::flag(settings_el, "doReadValues")?;
    let _ = settings::flag(settings_el, "scriptExecutionModeChange")?;
    let _ = settings::flag(settings_el, "publishMetadata")?;
    let _ = settings::flag(settings_el, "publishUseParameters")?;

    match settings::text(settings_el, "launchApplicationType")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("SPSS") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Statistics export: launchApplicationType {other:?} is not implemented"
            )));
        }
    }

    Ok(Box::new(StatisticsExport { path, names }))
}

fn reject_unknown(settings_el: Option<&Element>) -> Result<(), OperatorError> {
    let Some(settings_el) = settings_el else {
        return Ok(());
    };
    let mut unknown = Vec::new();
    for child in settings_el.child_elements() {
        if !is_known(&child.name.local) {
            unknown.push(child.name.local.to_string());
        }
    }
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(OperatorError::Settings(format!(
            "Statistics export: unknown setting {}",
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
            | "executionMode"
            | "spssExportFileType"
            | "spssExportFieldType"
            | "writeMode"
            | "encryptFileWithPassword"
            | "password"
            | "launchApplication"
            | "launchApplicationType"
            | "autoGenerateImportNode"
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

impl Operator for StatisticsExport {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Statistics export needs one input".into()))?;
        let options = ExportOptions {
            names: self.names,
            decimal_places: context.decimal_places(),
            date_format: settings::text(context.stream_properties.as_deref(), "dateFormat"),
        };
        let bytes = sav_export::write_table(input, &options).map_err(export_error)?;
        let path = context.export_path(&self.path);
        write_file(&path, &bytes)?;
        Ok(input.as_ref().clone())
    }
}

fn export_error(error: SavExportError) -> OperatorError {
    match error {
        SavExportError::Settings(message) => OperatorError::Settings(message),
        SavExportError::Data(message) => OperatorError::Data(message),
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
