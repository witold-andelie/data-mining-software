//! JSON source (`json_import!pasw.json`): reads a JSON file into a [`Table`] using
//! [`dms_io::json`] (`oracle/sources/json_*`).
//!
//! Like the JSON export, the node's settings are `Parameter` entries in the extension attachment
//! (`node@properties`), read when the node runs:
//!
//! | parameter | scripting property | accepted values |
//! |---|---|---|
//! | `full_filename` | `full_filename` | path, resolved via [`ExecContext::files`] |
//! | `stringFormat` | `string_format` | `records` (default when absent), `values` |
//! | `pythonimport.import_syntax`, `pythoninteractor.interactor_syntax` | — | the default `modelerJson.read_json` script whose `'orient'` matches `stringFormat` |
//! | `pythonimport.use_virtual_env`, `pythoninteractor.use_virtual_env` | — | `true` or `false`; the file is read here, not by a Python process |
//! | `pythoninteractor.syntaxCache`, `autoLabel` | — | ignored |
//!
//! The file is UTF-8. Embedded `filterTable`/`typeTable` rules on the node element apply.

use super::extension_params::{Param, flag, load_parameters, reject_lists, reject_unknown, text};
use super::text_source::read_file;
use super::r#type::{SourceTables, apply_source_tables, parse_source_tables};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use dms_io::json::{Orient, read_table};
use dms_stream::StreamNode;
use dms_types::Table;
use std::collections::HashMap;
use std::sync::Arc;

const LABEL: &str = "JSON source";

const KNOWN_PARAMETERS: &[&str] = &[
    "full_filename",
    "stringFormat",
    "pythonimport.import_syntax",
    "pythonimport.use_virtual_env",
    "pythoninteractor.interactor_syntax",
    "pythoninteractor.use_virtual_env",
    "pythoninteractor.syntaxCache",
    "autoLabel",
];

struct JsonImport {
    attachment: Option<String>,
    tables: SourceTables,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    Ok(Box::new(JsonImport {
        attachment: node.properties_attachment.clone(),
        tables: parse_source_tables(node.settings.as_ref())?,
    }))
}

impl Operator for JsonImport {
    fn arity(&self) -> Arity {
        Arity::Exactly(0)
    }

    fn run(&self, _inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let params = load_parameters(self.attachment.as_deref(), context, LABEL)?;
        let (file_name, orient) = json_settings(&params)?;
        let (path, bytes) = read_file(context, &file_name)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|e| OperatorError::Data(format!("{}: not UTF-8 text: {e}", path.display())))?;
        let table = read_table(text, orient)
            .map_err(|e| OperatorError::Data(format!("{}: {e}", path.display())))?;
        apply_source_tables(&self.tables, table)
    }
}

fn json_settings(params: &HashMap<String, Param>) -> Result<(String, Orient), OperatorError> {
    reject_unknown(params, KNOWN_PARAMETERS, LABEL)?;
    reject_lists(params, LABEL)?;
    let file_name = text(params, "full_filename")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OperatorError::Settings(format!("{LABEL}: full_filename is missing")))?;
    let (orient, orient_text) = match text(params, "stringFormat").as_deref().map(str::trim) {
        None | Some("records") => (Orient::Records, "'orient' : 'records'"),
        Some("values") => (Orient::Values, "'orient' : 'values'"),
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "{LABEL}: unknown stringFormat {other:?}"
            )));
        }
    };
    let _ = flag(params, "pythonimport.use_virtual_env")?;
    let _ = flag(params, "pythoninteractor.use_virtual_env")?;
    for name in [
        "pythonimport.import_syntax",
        "pythoninteractor.interactor_syntax",
    ] {
        if let Some(script) = text(params, name).filter(|script| !script.is_empty()) {
            if !script.contains("read_json") {
                return Err(OperatorError::Settings(format!(
                    "{LABEL}: a custom import script ({name}) is not implemented"
                )));
            }
            if !script.contains(orient_text) {
                return Err(OperatorError::Settings(format!(
                    "{LABEL}: {name} orient does not match stringFormat ({orient_text})"
                )));
            }
        }
    }
    Ok((file_name, orient))
}
