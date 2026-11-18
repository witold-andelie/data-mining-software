//! Extension nodes with Python syntax: Extension Import (Sources), Extension Transform (Record
//! Ops), Extension Output (Output) and Extension Export (Export). Each runs the node's script in
//! the user's own Python through `dms-python`.
//!
//! Settings (this application's native names; Modeler's own `.str` names for these nodes have not
//! been observed, so streams imported from Modeler keep their extension nodes as saved):
//! `syntaxType` (`python`, the default; `r` is not supported) and `pythonSyntax` (the script).
//!
//! Script API: `modelerData` is a pandas DataFrame of the input (None for Import) and
//! `modelerDataModel` lists its fields (`name`, `storage`, `measurement`, `role`). Import and
//! Transform scripts leave their result in `modelerData`. What a script prints is the
//! Output node's document.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput};
use crate::settings;
use dms_python::{BridgeError, Mode, ScriptRequest};
use dms_stream::StreamNode;
use dms_types::Table;
use std::sync::Arc;

/// Lines of printed output shown in an Output node's document.
const MAX_LINES: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Import,
    Transform,
    Output,
    Export,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Import => "Extension Import",
            Kind::Transform => "Extension Transform",
            Kind::Output => "Extension Output",
            Kind::Export => "Extension Export",
        }
    }
}

struct Extension {
    kind: Kind,
    script: String,
}

fn build(node: &StreamNode, kind: Kind) -> Result<Box<dyn Operator>, OperatorError> {
    let settings = node.settings.as_ref();
    match settings::text(settings, "syntaxType")
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("") | Some("python") => {}
        Some("r") => {
            return Err(OperatorError::Settings(format!(
                "{}: R scripts are not supported; rewrite the script in Python",
                kind.label()
            )));
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "syntaxType: expected python, found {other:?}"
            )));
        }
    }
    let script = settings::text(settings, "pythonSyntax").unwrap_or_default();
    if script.trim().is_empty() {
        return Err(OperatorError::Settings(format!(
            "{}: the Python script is empty",
            kind.label()
        )));
    }
    Ok(Box::new(Extension { kind, script }))
}

pub fn build_import(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    build(node, Kind::Import)
}

pub fn build_transform(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    build(node, Kind::Transform)
}

pub fn build_output(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    build(node, Kind::Output)
}

pub fn build_export(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    build(node, Kind::Export)
}

fn error(kind: Kind, error: BridgeError) -> OperatorError {
    match error {
        BridgeError::Cancelled => OperatorError::Cancelled,
        BridgeError::Input(_) => OperatorError::Data(error.to_string()),
        BridgeError::Script { .. } | BridgeError::Output(_) => {
            OperatorError::Data(format!("{}: {error}", kind.label()))
        }
        BridgeError::NotFound { .. }
        | BridgeError::TooOld { .. }
        | BridgeError::MissingPackages { .. } => OperatorError::Settings(error.to_string()),
        BridgeError::Process(_) | BridgeError::Io(_) | BridgeError::Timeout(_) => {
            OperatorError::Io(format!("{}: {error}", kind.label()))
        }
    }
}

impl Extension {
    fn execute(
        &self,
        inputs: &[Arc<Table>],
        context: &ExecContext,
    ) -> Result<dms_python::ScriptResult, OperatorError> {
        let python = dms_python::resolve(context.python.as_deref(), &context.cancel)
            .map_err(|e| error(self.kind, e))?;
        let input = match self.kind {
            Kind::Import => None,
            _ => Some(inputs.first().ok_or_else(|| {
                OperatorError::Inputs(format!("{} needs one input", self.kind.label()))
            })?),
        };
        let mode = match self.kind {
            Kind::Import => Mode::Import,
            Kind::Transform => Mode::Transform,
            Kind::Output | Kind::Export => Mode::Consume,
        };
        dms_python::run_script(
            &ScriptRequest::new(&python, &self.script, input.map(Arc::as_ref), mode),
            &context.cancel,
        )
        .map_err(|e| error(self.kind, e))
    }
}

impl Operator for Extension {
    fn arity(&self) -> Arity {
        match self.kind {
            Kind::Import => Arity::Exactly(0),
            _ => Arity::Exactly(1),
        }
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let result = self.execute(inputs, context)?;
        match (self.kind, result.table) {
            (Kind::Import | Kind::Transform, Some(table)) => Ok(table),
            // Export passes its input on (like the other export nodes).
            (Kind::Export, _) => inputs
                .first()
                .map(|t| t.as_ref().clone())
                .ok_or_else(|| OperatorError::Inputs("Extension Export needs one input".into())),
            _ => Err(OperatorError::Inputs(format!(
                "{} produced no data",
                self.kind.label()
            ))),
        }
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        model: Option<Arc<dms_pmml::Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        if self.kind != Kind::Output {
            return self
                .run(inputs, context)
                .map(|table| NodeOutput::Data(Arc::new(table)));
        }
        let _ = model;
        let result = self.execute(inputs, context)?;
        let mut rows: Vec<Vec<String>> = result
            .text
            .lines()
            .take(MAX_LINES)
            .map(|line| vec![line.to_string()])
            .collect();
        if result.truncated || result.text.lines().nth(MAX_LINES).is_some() {
            rows.push(vec!["(output truncated)".into()]);
        }
        Ok(NodeOutput::Document(Arc::new(Document {
            title: "Extension Output".into(),
            tables: vec![DocumentTable {
                title: None,
                header: Vec::new(),
                rows,
            }],
        })))
    }
}
