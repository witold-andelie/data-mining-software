//! The interface every node implementation provides.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::output::NodeOutput;
use dms_pmml::Pmml;
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

    /// Produces output in a stream execution. The execution owns globals and passes them to
    /// each terminal run; ordinary nodes keep their existing produce implementation.
    fn produce_with_globals(
        &self,
        inputs: &[Arc<Table>],
        model: Option<Arc<Pmml>>,
        context: &ExecContext,
        globals: &mut dms_clem::GlobalValues,
    ) -> Result<NodeOutput, OperatorError> {
        let _ = globals;
        self.produce(inputs, model, context)
    }

    /// Produces whatever the node produces. Data nodes keep the default; build nodes return a
    /// model and output nodes a document. `model` is the model of the build node linked to a
    /// model nugget, when that build node ran in this evaluation.
    fn produce(
        &self,
        inputs: &[Arc<Table>],
        model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        let _ = model;
        self.run(inputs, context)
            .map(|table| NodeOutput::Data(Arc::new(table)))
    }
}

/// Builds an operator from a stream node's saved settings.
pub type Factory = fn(&StreamNode) -> Result<Box<dyn Operator>, OperatorError>;
