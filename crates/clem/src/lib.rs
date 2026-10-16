//! CLEM, the expression language of Modeler (L1): parsing, binding to a data model and evaluation.
//!
//! Semantics follow `docs/clem/reference.md` (IBM's public user guide, in our words). Behaviour
//! IBM leaves unspecified is marked GAP in the code and decided by Modeler oracle probes.

mod eval;
mod functions;
mod lexer;
mod parser;
mod types;

use dms_types::Value;
use std::fmt;

pub use eval::{Bindings, Compiled, compile, compile_condition};
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

    /// Truth for conditions: `Some(bool)`, or `None` for null. Numbers are no conditions
    /// (oracle: Modeler 19 rejects `if 1 then`, `1 and 1` and a Select on `1`, although the
    /// guide calls false and 0 alike); flag fields are mapped when binding.
    pub fn truth(&self) -> Result<Option<bool>, ClemError> {
        match self {
            Datum::Bool(b) => Ok(Some(*b)),
            Datum::Value(Value::Null) => Ok(None),
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

    /// Text for `><` and `to_string`: reals with six decimals whatever the stream's
    /// `decimal_places` (oracle rule 27; Modeler's constant folding prints `1.0 >< "u"` as
    /// `1u`, which only constant expressions show). Truth values have no text (oracle: type
    /// error). GAP: dates and times print as ISO here, not in the stream's formats.
    pub fn to_text(&self) -> Result<String, ClemError> {
        match self {
            Datum::Bool(_) => Err(ClemError::Type("a condition has no text".into())),
            Datum::Value(Value::Real(r)) => Ok(format!("{r:.6}")),
            Datum::Value(value) => Ok(value.to_string()),
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
        /// The accepted argument counts, e.g. `1 or 3`.
        expected: String,
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
