//! CLEM, the expression language of Modeler (L1): parsing, binding to a data model and evaluation.
//!
//! Semantics follow `docs/clem/reference.md` (IBM's public user guide, in our words). Behaviour
//! IBM leaves unspecified is marked GAP in the code and decided by Modeler oracle probes.

mod eval;
mod functions;
mod lexer;
mod parser;

use dms_types::Value;
use std::fmt;

pub use eval::{Bindings, Compiled, compile};
pub use functions::names as function_names;
pub use parser::{BinaryOp, Expr, parse};

/// A CLEM result: a value (possibly `$null$`) or a truth value.
#[derive(Debug, Clone, PartialEq)]
pub enum Datum {
    Bool(bool),
    Value(Value),
}

impl Datum {
    pub const NULL: Datum = Datum::Value(Value::Null);

    pub fn is_null(&self) -> bool {
        matches!(self, Datum::Value(Value::Null))
    }

    /// Truth for conditions: `Some(bool)`, or `None` for null. Numbers count as true unless 0
    /// (the guide treats false and 0 alike).
    pub fn truth(&self) -> Result<Option<bool>, ClemError> {
        match self {
            Datum::Bool(b) => Ok(Some(*b)),
            Datum::Value(Value::Null) => Ok(None),
            Datum::Value(Value::Integer(i)) => Ok(Some(*i != 0)),
            Datum::Value(Value::Real(r)) => Ok(Some(*r != 0.0)),
            Datum::Value(other) => Err(ClemError::Type(format!("{other:?} is not a condition"))),
        }
    }

    /// The value to store; truth values are not values and must be mapped by the caller.
    pub fn into_value(self) -> Result<Value, ClemError> {
        match self {
            Datum::Value(value) => Ok(value),
            Datum::Bool(_) => Err(ClemError::Type(
                "a condition cannot be stored as a value here".into(),
            )),
        }
    }

    /// Text for `><` and `to_string`. GAP: Modeler's real formatting; we use the shortest
    /// round-trip form.
    pub fn to_text(&self) -> String {
        match self {
            Datum::Bool(true) => "true".into(),
            Datum::Bool(false) => "false".into(),
            Datum::Value(value) => value.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClemError {
    Syntax {
        position: usize,
        message: String,
    },
    UnknownField(String),
    UnknownFunction(String),
    Arity {
        function: String,
        expected: usize,
        found: usize,
    },
    /// Valid CLEM we do not implement yet.
    Unsupported(String),
    /// Operands of the wrong kind at evaluation time.
    Type(String),
}

impl fmt::Display for ClemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClemError::Syntax { position, message } => {
                write!(f, "syntax error at character {position}: {message}")
            }
            ClemError::UnknownField(name) => write!(f, "unknown field {name:?}"),
            ClemError::UnknownFunction(name) => write!(f, "unknown function {name}"),
            ClemError::Arity {
                function,
                expected,
                found,
            } => write!(f, "{function} takes {expected} arguments, {found} given"),
            ClemError::Unsupported(what) => write!(f, "not supported yet: {what}"),
            ClemError::Type(message) => write!(f, "type error: {message}"),
        }
    }
}

impl std::error::Error for ClemError {}
