//! Field Reorder (Field Operations). Not implemented yet; oracle probes in `corpus/fields/`.

use crate::error::OperatorError;
use crate::operator::Operator;
use dms_stream::StreamNode;

pub fn build(_node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    Err(OperatorError::Settings(
        "Field Reorder is not implemented yet".into(),
    ))
}
