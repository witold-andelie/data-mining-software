//! Table (Output palette): displays its input. The executor's result is the displayed data.
//! Not yet implemented: output to file (formatted, delimited, HTML) and highlighting.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use dms_stream::StreamNode;
use dms_types::Table;
use std::sync::Arc;

pub fn build(_node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    Ok(Box::new(TableOutput))
}

struct TableOutput;

impl Operator for TableOutput {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        inputs
            .first()
            .map(|input| Table::clone(input))
            .ok_or_else(|| OperatorError::Inputs("Table needs one input".into()))
    }
}
