//! Execution errors, attributed to the node that failed.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeError {
    pub uid: String,
    pub kind: ErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// No implementation for this node type (catalog id, or the stream's type identifier).
    Unsupported(String),
    /// The node's settings could not be interpreted.
    Settings(String),
    /// Wrong number or kind of inputs.
    Inputs(String),
    /// An upstream node failed, so this one cannot run.
    Upstream(String),
    /// Reading or writing files.
    Io(String),
    /// The data does not satisfy what the node needs.
    Data(String),
    /// A cycle through this node.
    Cycle,
}

impl NodeError {
    pub fn new(uid: impl Into<String>, kind: ErrorKind) -> NodeError {
        NodeError {
            uid: uid.into(),
            kind,
        }
    }
}

impl fmt::Display for NodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "node {}: ", self.uid)?;
        match &self.kind {
            ErrorKind::Unsupported(what) => write!(f, "unsupported node type {what}"),
            ErrorKind::Settings(message) => write!(f, "settings: {message}"),
            ErrorKind::Inputs(message) => write!(f, "inputs: {message}"),
            ErrorKind::Upstream(uid) => write!(f, "upstream node {uid} failed"),
            ErrorKind::Io(message) => write!(f, "I/O: {message}"),
            ErrorKind::Data(message) => write!(f, "data: {message}"),
            ErrorKind::Cycle => f.write_str("part of a cycle"),
        }
    }
}

impl std::error::Error for NodeError {}

/// Errors from an operator, before the executor attributes them to a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatorError {
    Settings(String),
    Inputs(String),
    Io(String),
    Data(String),
}

impl OperatorError {
    pub fn at(self, uid: &str) -> NodeError {
        let kind = match self {
            OperatorError::Settings(message) => ErrorKind::Settings(message),
            OperatorError::Inputs(message) => ErrorKind::Inputs(message),
            OperatorError::Io(message) => ErrorKind::Io(message),
            OperatorError::Data(message) => ErrorKind::Data(message),
        };
        NodeError::new(uid, kind)
    }
}
