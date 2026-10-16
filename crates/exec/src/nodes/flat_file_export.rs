//! Flat File export (Export). Not implemented yet; oracle probes in `corpus/exports/`.

use crate::error::OperatorError;
use crate::operator::Operator;
use dms_stream::StreamNode;

pub fn build(_node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    Err(OperatorError::Settings(
        "Flat File export is not implemented yet".into(),
    ))
}
