//! Statistics (Output palette): produces a document (`crate::output::Document`) laid out like the
//! table Modeler exports (`docs/oracle.md`, oracle `outputs/`). Not implemented yet.

use crate::error::OperatorError;
use crate::operator::Operator;
use dms_stream::StreamNode;

pub fn build(_node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    Err(OperatorError::Settings(
        "Statistics is not implemented yet".into(),
    ))
}
