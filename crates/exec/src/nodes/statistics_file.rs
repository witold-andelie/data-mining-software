//! Statistics File (Sources palette, `SPSSInputNode`): reads an IBM SPSS Statistics `.sav` file
//! into a [`Table`] via [`dms_sav`] and [`dms_io::statistics`].
//!
//! Implemented: `inputFileName` (resolved via [`ExecContext::files`]), `readVariableNames`
//! (`namesAndLabels`, `labelsAsNames`), `readValues` (`dataAndLabels`), storage/measurement/role
//! mapping per `oracle/probes/sav_to_csv/*.fields.json` (SPSS dates/times/timestamps converted to
//! `Value::Date`/`Time`/`Timestamp`, system missing to `Value::Null`, user-missing values to
//! [`dms_types::MissingSpec`], variable and value labels attached to [`dms_types::Field`]), and
//! embedded `filterTable`/`typeTable` rules.
//!
//! `useFieldFormatForStorage` follows `dms_io::statistics`. Not implemented:
//! `readValues = labelsAsData`, `fileIsPasswordEncrypted = true`, and non-`None` `check` modes in `typeTable`.

use super::r#type::{SourceTables, apply_source_tables, parse_source_tables};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_io::statistics::{NameMode, StatisticsOptions, read_table};
use dms_stream::StreamNode;
use dms_types::Table;
use std::sync::Arc;

struct StatisticsFile {
    file_name: String,
    options: StatisticsOptions,
    tables: SourceTables,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let file_name = settings::text(s, "inputFileName")
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| {
            OperatorError::Settings("Statistics File: inputFileName is required".into())
        })?;

    let field_format_storage = settings::flag(s, "useFieldFormatForStorage")?.unwrap_or(false);
    if settings::flag(s, "fileIsPasswordEncrypted")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "Statistics File: password-encrypted files are not supported yet".into(),
        ));
    }

    let names = match settings::text(s, "readVariableNames")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("namesAndLabels") => NameMode::NamesAndLabels,
        Some("labelsAsNames") => NameMode::LabelsAsNames,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Statistics File: unknown readVariableNames {other:?}"
            )));
        }
    };

    match settings::text(s, "readValues").as_deref().map(str::trim) {
        None | Some("") | Some("dataAndLabels") => {}
        Some("labelsAsData") => {
            return Err(OperatorError::Settings(
                "Statistics File: readValues=labelsAsData is not supported yet".into(),
            ));
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Statistics File: unknown readValues {other:?}"
            )));
        }
    }

    let tables = parse_source_tables(s)?;
    Ok(Box::new(StatisticsFile {
        file_name,
        options: StatisticsOptions {
            names,
            field_format_storage,
        },
        tables,
    }))
}

impl Operator for StatisticsFile {
    fn arity(&self) -> Arity {
        Arity::Exactly(0)
    }

    fn run(&self, _inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let resolved = context.files.resolve(&self.file_name).ok_or_else(|| {
            OperatorError::Io(format!("cannot find data file {:?}", self.file_name))
        })?;
        let bytes = std::fs::read(resolved.path())
            .map_err(|e| OperatorError::Io(format!("{}: {e}", resolved.path().display())))?;
        let table = read_table(&bytes, self.options)
            .map_err(|e| OperatorError::Data(format!("{}: {e}", resolved.path().display())))?;
        apply_source_tables(&self.tables, table)
    }
}
