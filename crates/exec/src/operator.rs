//! The interface every node implementation provides.

use crate::context::ExecContext;
use crate::error::OperatorError;
use dms_stream::StreamNode;
use dms_types::Table;
use std::sync::Arc;

/// How many inputs a node accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity {
    Exactly(usize),
    AtLeast(usize),
}

impl Arity {
    pub fn accepts(self, count: usize) -> bool {
        match self {
            Arity::Exactly(n) => count == n,
            Arity::AtLeast(n) => count >= n,
        }
    }
}

/// A configured node: settings already parsed, ready to run on its inputs.
pub trait Operator: Send + Sync {
    fn arity(&self) -> Arity;

    /// Produces the node's output data from its inputs (in link order).
    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError>;
}

/// Builds an operator from a stream node's saved settings.
pub type Factory = fn(&StreamNode) -> Result<Box<dyn Operator>, OperatorError>;
