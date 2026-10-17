//! Balance (Record Ops): duplicates or drops records by condition-specific factors.
//!
//! Settings: `balanceDirectives/entries/entry[n]/{condition,factor}` (CLEM condition, real
//! factor), `balanceTrainingPartitionOnly` (default true; only applies when the input has a
//! partition field: records outside the training partition then pass unchanged).
//! Evidence: `docs/formats/nodes/balance.md` and the team streams (`'heart disease' = 2` factor
//! 1.25; `default = 0.0` factor 1.0 where `default` is an ordinary data field).
//!
//! Rules (settled by `oracle/records/balance_int_factors`, `balance_multi_match`):
//! - A record matching a directive gets `floor(factor)` copies plus one more with probability
//!   `frac(factor)` (`factor = 0.0` drops the record, `1.0` keeps 1 copy, `2.0` keeps 2, `3.0`
//!   keeps 3); a record matching no directive keeps one copy (`balance_int_factors`).
//! - Copies of a record stay adjacent to the original record in input order
//!   (`balance_int_factors`, `balance_multi_match`).
//! - A condition that evaluates to `$null$` does not match (`balance_int_factors` row 15 has
//!   `iv = null` under `iv = 10`; `balance_multi_match` rows 11, 15, 16 have `iv = null` under
//!   `iv >= 0`).
//! - Directives are evaluated in order and the **first** matching directive wins; subsequent
//!   matching directives are ignored (`balance_multi_match` with `[2.0, grp = "A"]` then
//!   `[3.0, iv >= 0]` gives 2 copies—not 6—to rows 4, 6, 10, 14 where both conditions hold).
//!
//! GAP: Balance has no seed and Modeler's own runs keep different records for fractional factors
//! (`oracle/random/balance_boost`, `balance_reduce` rerun), so those draws cannot be reproduced;
//! we draw `java.util.Random(SEED).nextDouble()` (`crate::random`). Integral factors (0.0, 1.0,
//! 2.0, ...) are exact.

use super::select::Condition;
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::random::JavaRandom;
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Role, Table, ValueSpec};
use std::sync::Arc;

const SEED: i64 = 1_234_567;

struct Directive {
    condition: Condition,
    factor: f64,
}

struct Balance {
    directives: Vec<Directive>,
    training_only: bool,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let training_only = settings::flag(s, "balanceTrainingPartitionOnly")?.unwrap_or(true);
    let mut directives = Vec::new();
    for entry in settings::children(s, "balanceDirectives/entries", "entry") {
        let condition = entry.child("condition").map(|e| e.text());
        let factor_text = entry.child("factor").map(|e| e.text()).unwrap_or_default();
        let factor: f64 = factor_text
            .trim()
            .parse()
            .ok()
            .filter(|f: &f64| f.is_finite() && (0.0..=1000.0).contains(f))
            .ok_or_else(|| {
                OperatorError::Settings(format!("balance factor {factor_text:?} is not valid"))
            })?;
        directives.push(Directive {
            condition: Condition::parse(condition, "balance condition")?,
            factor,
        });
    }
    Ok(Box::new(Balance {
        directives,
        training_only,
    }))
}

impl Operator for Balance {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        self.run_globals(inputs, &dms_clem::GlobalValues::default())
    }

    fn produce_with_globals(
        &self,
        inputs: &[Arc<Table>],
        _model: Option<Arc<dms_pmml::Pmml>>,
        _context: &ExecContext,
        globals: &mut dms_clem::GlobalValues,
    ) -> Result<crate::output::NodeOutput, OperatorError> {
        self.run_globals(inputs, globals)
            .map(|table| crate::output::NodeOutput::Data(Arc::new(table)))
    }
}

impl Balance {
    fn run_globals(
        &self,
        inputs: &[Arc<Table>],
        globals: &dms_clem::GlobalValues,
    ) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Balance needs one input".into()))?;
        let model = input.model();
        let bound = self
            .directives
            .iter()
            .map(|d| {
                let truths = d.condition.bind(model, globals)?.truths(input.rows())?;
                Ok((truths, d.factor))
            })
            .collect::<Result<Vec<_>, OperatorError>>()?;
        // (position of the partition field, its training value)
        let training = if self.training_only {
            match model
                .fields()
                .iter()
                .position(|f| f.role == Role::Partition)
            {
                Some(position) => {
                    let value = model
                        .fields()
                        .get(position)
                        .and_then(|f| match &f.values {
                            ValueSpec::Categories(values) => values.first().cloned(),
                            _ => None,
                        })
                        .ok_or_else(|| {
                            OperatorError::Data("partition values are not known".into())
                        })?;
                    Some((position, value))
                }
                None => None,
            }
        } else {
            None
        };
        let mut rng = JavaRandom::new(SEED);
        let mut out = Vec::new();
        for (index, row) in input.rows().iter().enumerate() {
            let in_scope = match &training {
                Some((position, value)) => row
                    .get(*position)
                    .is_some_and(|v| v.total_cmp(value).is_eq() && !v.is_null()),
                None => true,
            };
            let mut factor = 1.0_f64;
            if in_scope {
                for (truths, f) in &bound {
                    if truths.get(index) == Some(&Some(true)) {
                        factor = *f;
                        break;
                    }
                }
            }
            let whole = factor.floor();
            let mut copies = whole as usize;
            let frac = factor - whole;
            if frac > 0.0 && rng.next_double() < frac {
                copies = copies.saturating_add(1);
            }
            for _ in 0..copies {
                out.push(row.clone());
            }
        }
        Table::new(model.clone(), out).map_err(|error| OperatorError::Data(error.to_string()))
    }
}
