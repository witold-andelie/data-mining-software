//! Select (Record Ops): keeps (`include`) or drops (`discard`) the records for which a CLEM
//! condition is true.
//!
//! Settings: `selectMode` (`include` default | `discard`), `selectCondition` (CLEM text, stored
//! verbatim; an empty condition is an error). Evidence: `docs/formats/nodes/select.md`,
//! `docs/clem/oracle-results.md` rules 33 and 39, and the team streams (`'account YES/NO' = 1`,
//! `Record_Count > 1`, `@PARTITION_FIELD = @TESTING_PARTITION`).
//!
//! Conditions treat flag fields as truth values (rule 33). A condition that evaluates to `$null$`
//! drops the record in both `include` and `discard` modes (verified by Modeler 19 oracle, rule 39).

use super::field_util::{bind_condition, eval, parse_required, record_number};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_clem::{Compiled, Expr};
use dms_stream::StreamNode;
use dms_types::{DataModel, Table, Value};
use std::sync::Arc;

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let settings = node.settings.as_ref();
    let discard = match settings::text(settings, "selectMode")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("include") => false,
        Some("discard") => true,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "selectMode: expected include or discard, found {other:?}"
            )));
        }
    };
    let condition = Condition::parse(
        settings::text(settings, "selectCondition"),
        "selectCondition",
    )?;
    Ok(Box::new(Select { discard, condition }))
}

/// A parsed CLEM condition, bound to a data model when a node runs.
pub(crate) struct Condition {
    expr: Expr,
    setting: &'static str,
}

/// The truth of a condition on one record.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    True,
    False,
    Null,
}

impl Verdict {
    /// Oracle rule 39: a `$null$` condition excludes the record in both `include` and `discard`
    /// modes.
    fn keeps(self, discard: bool) -> bool {
        match self {
            Verdict::True => !discard,
            Verdict::False => discard,
            Verdict::Null => false,
        }
    }
}

pub(crate) struct BoundCondition {
    compiled: Compiled,
    setting: &'static str,
}

impl Condition {
    pub(crate) fn parse(
        text: Option<String>,
        setting: &'static str,
    ) -> Result<Condition, OperatorError> {
        let expr = parse_required(setting, text)?;
        Ok(Condition { expr, setting })
    }

    pub(crate) fn bind(&self, model: &DataModel) -> Result<BoundCondition, OperatorError> {
        let compiled = bind_condition(self.setting, &self.expr, model, None)?;
        Ok(BoundCondition {
            compiled,
            setting: self.setting,
        })
    }
}

impl BoundCondition {
    /// `index` is the 1-based record number.
    pub(crate) fn verdict(&self, row: &[Value], index: u64) -> Result<Verdict, OperatorError> {
        let datum = eval(self.setting, &self.compiled, row, index)?;
        let setting = self.setting;
        Ok(
            match datum.truth().map_err(|error| {
                OperatorError::Data(format!("{setting}, record {index}: {error}"))
            })? {
                Some(true) => Verdict::True,
                Some(false) => Verdict::False,
                None => Verdict::Null,
            },
        )
    }
}

struct Select {
    discard: bool,
    condition: Condition,
}

impl Operator for Select {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Select needs one input".into()))?;
        let bound = self.condition.bind(input.model())?;
        let mut kept = Vec::new();
        for (index, row) in input.rows().iter().enumerate() {
            if bound
                .verdict(row, record_number(index))?
                .keeps(self.discard)
            {
                kept.push(row.clone());
            }
        }
        Table::new(input.model().clone(), kept)
            .map_err(|error| OperatorError::Data(error.to_string()))
    }
}
