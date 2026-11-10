//! Reference executor (L3): plans a stream's diagram and runs its nodes on in-memory tables.
//!
//! Written for obvious correctness: row-based tables, each node evaluated at most once, failures
//! kept with the node that failed. A faster columnar executor is checked against this one.

pub mod context;
pub mod error;
pub mod nodes;
pub mod operator;
pub mod output;
pub mod plan;
mod random;
pub mod registry;
pub mod schema;
pub mod settings;

pub use context::{ExecContext, FileResolver, Resolved};
pub use error::{ErrorKind, NodeError, OperatorError};
pub use operator::{Arity, Factory, Operator};
pub use output::{Document, DocumentTable, NodeOutput, format_real};
pub use plan::{EvalEvent, Execution, NodeResult, Plan};
