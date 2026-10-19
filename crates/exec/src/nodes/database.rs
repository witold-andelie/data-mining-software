//! Modeler 19 database/databaseexport: observed `database_streams.py` settings only.
//! Every computational setting is either consumed or checked against the saved default.
//! Password elements are validated without copying their contents into errors/operators.
//! CLI comparison uses ExecContext::export_dir: database tables are redirected to
//! `dms_<saved table>` so running `compare database db_export.str` preserves exp_create.
use crate::operator::{Arity, Operator};
use crate::settings;
use crate::{ExecContext, OperatorError};
use dms_io::odbc;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::Table;
use std::collections::HashSet;
use std::sync::Arc;

struct Database {
    dsn: String,
    target: String,
    query: bool,
    export: bool,
    drop_existing: bool,
}

fn gap(path: &str) -> OperatorError {
    OperatorError::Settings(format!(
        "GAP: Database setting {path} is not covered by the oracle"
    ))
}

pub fn build_import(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    build(node, false)
}
pub fn build_export(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    build(node, true)
}

fn build(node: &StreamNode, export: bool) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref().ok_or_else(|| gap("properties"))?;
    validate(s, export)?;
    let dsn = required(Some(s), "connection/datasource")?;
    odbc::validate_dsn(&dsn).map_err(|e| OperatorError::Settings(e.to_string()))?;
    let query = !export
        && match settings::text(Some(s), "importMode").as_deref() {
            None | Some("table") => false,
            Some("query") => true,
            _ => return Err(gap("importMode")),
        };
    let target = required(Some(s), if query { "queryText" } else { "tableName" })?;
    if !export {
        let inactive = if query { "tableName" } else { "queryText" };
        if settings::text(Some(s), inactive).is_some_and(|v| !v.is_empty()) {
            return Err(gap(inactive));
        }
    }
    Ok(Box::new(Database {
        dsn,
        target,
        query,
        export,
        drop_existing: settings::text(Some(s), "dropExistingTable").as_deref() == Some("true"),
    }))
}

fn required(s: Option<&Element>, path: &str) -> Result<String, OperatorError> {
    settings::text(s, path)
        .filter(|v| !v.trim().is_empty() && v.len() <= 1_048_576 && !v.contains('\0'))
        .ok_or_else(|| gap(path))
}

// Metadata that has no execution semantics. Nested computational settings do not enter here.
fn metadata(name: &str) -> bool {
    matches!(
        name,
        "keywords"
            | "lastSavedAs"
            | "fieldMappingEssentialFields"
            | "repositoryPath"
            | "repositoryVersionMarker"
            | "toolTipText"
            | "annotation"
            | "publishTag"
            | "lastSavedBy"
            | "lastSavedDate"
            | "label"
            | "creationDate"
            | "repositoryLastModified"
            | "createdBy"
            | "name"
            | "position"
            | "builderNodeId"
            | "repalcebuilderNodeId"
            | "publishParameters"
            | "outputFileName"
    )
}

fn validate(root: &Element, export: bool) -> Result<(), OperatorError> {
    let mut seen = HashSet::new();
    for child in root.child_elements() {
        let name = &*child.name.local;
        if !seen.insert(name) {
            return Err(gap("duplicate property"));
        }
        if metadata(name) {
            continue;
        }
        if name == "connection" {
            let mut connection_seen = HashSet::new();
            for value in child.child_elements() {
                let key = &*value.name.local;
                if !connection_seen.insert(key) {
                    return Err(gap("duplicate connection property"));
                }
                let expected = match key {
                    "type" => Some("odbc"),
                    "user" | "catalog" | "credential" | "displayName" | "epassword" => Some(""),
                    // Modeler's saved false is the no-password marker, not a credential.
                    "password" => Some("false"),
                    "useCredential" => Some("false"),
                    "datasource" => None,
                    _ => return Err(gap("connection property")),
                };
                if value.child_elements().next().is_some()
                    || expected.is_some_and(|v| value.text() != v)
                {
                    return Err(gap("connection credentials/options"));
                }
            }
            continue;
        }
        if !export && matches!(name, "typeTable" | "filterTable") {
            validate_source_table(child)?;
            continue;
        }
        let text = child.text();
        if child.child_elements().next().is_some() {
            return Err(gap(name));
        }
        let expected = match name {
            "tableName" => continue,
            "enabled" | "autoLabel" => "true",
            "cacheEnabled" | "scriptExecutionModeChange" | "doReadValues" => "false",
            "schemaName" => "",
            "delimitMode" => "asNeeded",
            "importMode" if !export && matches!(text.as_str(), "table" | "query") => continue,
            "queryText" if !export => continue,
            "datetimeMode" if !export => "native",
            "refreshLinkState" if !export => "NoReplaceLink",
            "trimMode" if !export => "none",
            "splitText" if !export => "never",
            "databaseExportType" if export => "create",
            "dropExistingTable" if export => "true",
            "defaultStringSize" if export => "255",
            "executionMode" if export => "export_data",
            "dbxBulkLoadOption" if export => "off",
            "DBMergeFromUI" | "useBatchCommit" | "dbxValidateTableSize" if export => "true",
            "DBMergeType" if export => "onlyExist",
            "dbxBatchSize" if export => "10000",
            "dbxBulkloadDelimiter" if export => "Tab",
            "dbxExportCompressionMode" if export => "NOCOMPRESS",
            "dbxOdbcBindType" if export => "Row",
            "dbxCreateTableTemplate" if export => "CREATE TABLE <table-name> <(table-columns)>",
            "dbxCreateIndexTemplate" if export => {
                "CREATE INDEX <index-name> ON <table-name><(index-columns)>"
            }
            "autoGenerateImportNode"
            | "dbxExportUseDPF"
            | "dbxUseBulkloadDatafilePath"
            | "dbxUseBulkloadLoaderPath"
            | "dbxUseCreateIndexTemplate"
            | "dbxUseCreateTableTemplate"
            | "dbxUseNotLoggedMode"
            | "dbxUseBulkloadLog"
            | "publishMetadata"
            | "publishUseParameters"
            | "deleteExistingRows"
            | "dbxExportUseCompression"
                if export =>
            {
                "false"
            }
            "keyColumns"
            | "dbxBulkloadDatafilePath"
            | "dbxExportTablespace"
            | "dbxExportDPFHashField"
            | "dbxBulkloadLoaderPath"
            | "dbxIndexList"
            | "DBMergeMappedItem"
            | "databaseCustomTypes"
            | "dbxBulkloadLogPath"
            | "dbxBulkloadDelimiterString"
            | "dbxExtraCmdlineOptions"
                if export =>
            {
                ""
            }
            _ => return Err(gap(name)),
        };
        if text != expected {
            return Err(gap(name));
        }
    }
    Ok(())
}

fn validate_source_table(table: &Element) -> Result<(), OperatorError> {
    let mut seen = HashSet::new();
    for child in table.child_elements() {
        let name = &*child.name.local;
        if !seen.insert(name) {
            return Err(gap("source table duplicate"));
        }
        let valid = match (&*table.name.local, name) {
            ("typeTable", "defaultValueMode") => {
                child.text() == "pass" && child.child_elements().next().is_none()
            }
            ("filterTable", "defaultDrop") => {
                child.text() == "false" && child.child_elements().next().is_none()
            }
            (_, "rows") => child.children.is_empty(),
            ("filterTable", "fieldGroups") => child.child_elements().all(|e| {
                matches!(&*e.name.local, "newNames" | "droppedNames" | "groups")
                    && e.children.is_empty()
            }),
            _ => false,
        };
        if !valid {
            return Err(gap("source type/filter override"));
        }
    }
    Ok(())
}

impl Operator for Database {
    fn arity(&self) -> Arity {
        Arity::Exactly(usize::from(self.export))
    }
    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        if !self.export {
            return odbc::read(&self.dsn, &self.target, self.query)
                .map_err(|e| OperatorError::Io(e.to_string()));
        }
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Database export needs one input".into()))?;
        let target = if context.export_dir.is_some() {
            format!("dms_{}", self.target)
        } else {
            self.target.clone()
        };
        odbc::create(&self.dsn, &target, input, self.drop_existing)
            .map_err(|e| OperatorError::Io(e.to_string()))?;
        Ok((**input).clone())
    }
}
