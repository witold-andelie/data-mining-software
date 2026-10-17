//! Set Globals (`SetGlobalsNode`, catalog `set_globals`). Sources: Modeler 19.0 User's
//! Guide, Global Functions (table 35), Python Scripting and Automation Guide (table 237),
//! and the saved streams/oracle exports `misc_gl_*` (no Modeler implementation code).
//!
//! Settings and verified rules:
//! - `setGlobalsTable/rows/row/name` selects a field; Boolean `sum`, `ave`, `min`, `max`,
//!   `sdev` select independent statistics (`default`, `retain`, `clear`). Missing flags
//!   select nothing. Numeric duplicates count; system nulls and user-missing values are
//!   excluded (`default`, `missing`). Sum preserves integer storage; mean/SDev are real.
//! - SDev is sample SDev, denominator n-1 (`default`, `empty`, which actually has TWO rows).
//!   Constant SDev is zero (`constant`); singleton SDev and all-null statistics are undefined,
//!   and CLEM reads fail (`singleton`, `all_null`).
//! - Min/max preserve string/date/timestamp storage (`types`). Empty strings participate
//!   and sort before spaces; null strings do not participate.
//! - `setGlobalsClearGlobals` / `clear_first` defaults true. False replaces ONLY selected
//!   (field, statistic) keys and preserves all others (`retain`); true clears all keys first
//!   (`clear`). Updates are committed together after validation/computation succeeds.
//! - `setGlobalsShowResults` / `show_preview` defaults false (misc_manifest defaults); either
//!   flag is accepted. There
//!   is no extra document/file in the oracle (`clear` true, `retain` false).
//! - `setGlobalsDefaultOperations` holds UI defaults (`ave`, `sdev`, `min`, `max`, `sum`),
//!   not additional selected fields. `doReadValues=false` is inert; true is unsupported.
//!   Unknown statistics, malformed flags, duplicate fields and unknown setGlobals* settings
//!   are rejected rather than ignored.
//! - Saved `stream/properties/globalsTable` seeds a fresh execution, with J integer, D real,
//!   A string, U ISO date and W ISO timestamp values (`default`, `types`, `retain`, `clear`).
//!   `timestamp` is bookkeeping. Missing globalsTable means no saved globals (`all_null`).
//! - Runs are sequential and callers choose their target order; data exports recompute their
//!   inputs after terminal runs. In `clear` the Table reads the prime's old rv mean (-2.5),
//!   while its later data export reads the final mean (1.488...). No reader cache crosses runs.
//!
//! GAP: Modeler's stream.iterator order is not encoded by terminalNodeOrder or XML node
//! order (retain differs from both). The CLI exposes an explicit run order; its default uses
//! the saved terminalNodeOrder. A public-API probe must record all three orders before/after
//! save/reopen and node insertion to establish a portable iterator-order rule.
//! GAP: truly zero input rows (the historical `empty` fixture has two), nonnumeric sum/mean/
//! SDev, integer sum overflow, replacing an existing value with an undefined statistic,
//! clearGlobalsInStream=true and other saved-value encodings need probes; rejected explicitly.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::NodeOutput;
use crate::settings;
use dms_clem::{GlobalStatistic, GlobalValues};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::{Storage, Table, Value};
use std::collections::HashSet;
use std::sync::Arc;

fn unsupported(what: impl std::fmt::Display) -> OperatorError {
    OperatorError::Settings(format!("Set Globals: unsupported {what}"))
}

fn statistic(name: &str) -> Option<GlobalStatistic> {
    GlobalStatistic::ALL
        .into_iter()
        .find(|s| s.setting() == name)
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    if let Some(s) = s {
        for child in s.child_elements() {
            let name = child.name.local.as_ref();
            if name.starts_with("setGlobals")
                && !matches!(
                    name,
                    "setGlobalsTable"
                        | "setGlobalsClearGlobals"
                        | "setGlobalsShowResults"
                        | "setGlobalsDefaultOperations"
                )
            {
                return Err(unsupported(format!("setting {name}")));
            }
        }
    }
    if settings::flag(s, "doReadValues")? == Some(true) {
        return Err(unsupported("doReadValues=true"));
    }
    let clear = settings::flag(s, "setGlobalsClearGlobals")?.unwrap_or(true);
    let _preview = settings::flag(s, "setGlobalsShowResults")?.unwrap_or(false);
    for operation in settings::children(s, "setGlobalsDefaultOperations", "setElement") {
        if statistic(operation.text().trim()).is_none() {
            return Err(unsupported(format!(
                "default operation {:?}",
                operation.text()
            )));
        }
    }
    let mut fields = Vec::new();
    let mut seen = HashSet::new();
    for row in settings::children(s, "setGlobalsTable/rows", "row") {
        let name = settings::text(Some(row), "name").unwrap_or_default();
        if name.is_empty() || !seen.insert(name.clone()) {
            return Err(OperatorError::Settings(
                "Set Globals: empty or duplicate field name".into(),
            ));
        }
        for child in row.child_elements() {
            if child.name.local.as_ref() != "name" && statistic(&child.name.local).is_none() {
                return Err(unsupported(format!("statistic {}", child.name.local)));
            }
        }
        let mut selected = Vec::new();
        for stat in GlobalStatistic::ALL {
            if settings::flag(Some(row), stat.setting())? == Some(true) {
                selected.push(stat);
            }
        }
        fields.push((name, selected));
    }
    Ok(Box::new(SetGlobals { fields, clear }))
}

struct SetGlobals {
    fields: Vec<(String, Vec<GlobalStatistic>)>,
    clear: bool,
}

impl Operator for SetGlobals {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(unsupported(
            "running without a stream execution's global state",
        ))
    }

    fn produce_with_globals(
        &self,
        inputs: &[Arc<Table>],
        _model: Option<Arc<dms_pmml::Pmml>>,
        _context: &ExecContext,
        globals: &mut GlobalValues,
    ) -> Result<NodeOutput, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Set Globals needs one input".into()))?;
        if input.row_count() == 0 {
            return Err(unsupported("zero input rows (GAP)"));
        }
        let mut next = if self.clear {
            GlobalValues::default()
        } else {
            globals.clone()
        };
        for (name, selected) in &self.fields {
            let position = input.model().position(name).ok_or_else(|| {
                OperatorError::Data(format!("Set Globals: unknown field {name:?}"))
            })?;
            let field =
                input.model().fields().get(position).ok_or_else(|| {
                    OperatorError::Data("Set Globals: invalid field position".into())
                })?;
            let values: Vec<&Value> = input
                .rows()
                .iter()
                .filter_map(|row| row.get(position))
                .filter(|v| !v.is_null() && !field.missing.is_blank(v))
                .collect();
            for &stat in selected {
                let value = summarize(&values, stat)?;
                if !self.clear && value.is_null() && globals.get(name, stat).is_ok() {
                    return Err(unsupported(
                        "replacing an existing statistic with an undefined value (GAP)",
                    ));
                }
                next.set(name, stat, value)
                    .map_err(|e| OperatorError::Data(e.to_string()))?;
            }
        }
        *globals = next;
        Ok(NodeOutput::Data(Arc::clone(input)))
    }
}

fn summarize(values: &[&Value], stat: GlobalStatistic) -> Result<Value, OperatorError> {
    if values.is_empty() {
        return Ok(Value::Null);
    }
    if matches!(stat, GlobalStatistic::Min | GlobalStatistic::Max) {
        if values
            .iter()
            .any(|v| matches!(v, Value::List(_) | Value::Time(_)))
        {
            return Err(unsupported("extrema for collection/time storage (GAP)"));
        }
        let best = values.iter().copied().reduce(|a, b| {
            if (stat == GlobalStatistic::Min && a.total_cmp(b).is_gt())
                || (stat == GlobalStatistic::Max && a.total_cmp(b).is_lt())
            {
                b
            } else {
                a
            }
        });
        return Ok(best.cloned().unwrap_or(Value::Null));
    }
    let mut sum = 0.0;
    for value in values {
        let number = value
            .as_f64()
            .filter(|n| n.is_finite())
            .ok_or_else(|| unsupported("sum/mean/SDev of nonnumeric or nonfinite values (GAP)"))?;
        sum += number;
        if !sum.is_finite() {
            return Err(unsupported("nonfinite statistic"));
        }
    }
    let n = values.len() as f64;
    let mean = sum / n;
    Ok(match stat {
        GlobalStatistic::Sum if values.iter().all(|v| matches!(v, Value::Integer(_))) => {
            Value::Integer(
                values
                    .iter()
                    .try_fold(0i64, |sum, value| match value {
                        Value::Integer(value) => sum.checked_add(*value),
                        _ => None,
                    })
                    .ok_or_else(|| unsupported("integer sum overflow (GAP)"))?,
            )
        }
        GlobalStatistic::Sum => Value::Real(sum),
        GlobalStatistic::Mean => Value::Real(mean),
        GlobalStatistic::Sdev if values.len() < 2 => Value::Null,
        GlobalStatistic::Sdev => {
            let mut ss = 0.0;
            for value in values {
                if let Some(number) = value.as_f64() {
                    ss += (number - mean).powi(2);
                }
            }
            let sdev = (ss / (n - 1.0)).sqrt();
            if !sdev.is_finite() {
                return Err(unsupported("nonfinite SDev"));
            }
            Value::Real(sdev)
        }
        GlobalStatistic::Min | GlobalStatistic::Max => Value::Null,
    })
}

pub(crate) fn saved_globals(properties: Option<&Element>) -> Result<GlobalValues, OperatorError> {
    if settings::flag(properties, "clearGlobalsInStream")? == Some(true) {
        return Err(unsupported("clearGlobalsInStream=true (GAP)"));
    }
    let mut globals = GlobalValues::default();
    for row in settings::children(properties, "globalsTable/rows", "row") {
        let name = settings::text(Some(row), "name")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| OperatorError::Settings("globalsTable: missing field name".into()))?;
        for child in row.child_elements() {
            if child.name.local.as_ref() == "name" {
                continue;
            }
            let stat = statistic(&child.name.local)
                .ok_or_else(|| unsupported(format!("saved statistic {}", child.name.local)))?;
            let value = child
                .child("value")
                .ok_or_else(|| unsupported("saved global without value"))?;
            let text = value.text();
            let storage = match value.attribute("type") {
                Some("J") => Storage::Integer,
                Some("D") => Storage::Real,
                Some("A") => Storage::String,
                Some("U") => Storage::Date,
                Some("W") => Storage::Timestamp,
                other => return Err(unsupported(format!("saved value encoding {other:?} (GAP)"))),
            };
            let parsed = dms_io::text::parse(&text, storage, &dms_io::text::TextOptions::default())
                .filter(|v| !matches!(v, Value::Real(r) if !r.is_finite()))
                .ok_or_else(|| {
                    OperatorError::Settings(format!(
                        "globalsTable: invalid {storage:?} value {text:?}"
                    ))
                })?;
            globals
                .set(&name, stat, parsed)
                .map_err(|e| OperatorError::Data(e.to_string()))?;
        }
    }
    Ok(globals)
}
