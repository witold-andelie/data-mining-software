//! Partition (Field Operations): adds a field that assigns each record to the training, testing
//! (and optionally validation) partition.
//!
//! Implemented: `partitionFieldName`, `trainingPartitionSize`/`testingPartitionSize`/
//! `validationPartitionSize` (percentages; the validation partition exists when
//! `createValidationPartition` is true),
//! `partitionRandomSeed` (with `partitionUseRandomSeed`; without it the seed comes from the clock),
//! the partition labels and `partitionValueMode` (`system_label` gives `1_Training`, `label`
//! gives `Training`, `system` gives the strings `1`, `2`, `3`: `oracle/random/part_*`). The output field has `Role::Partition` and its
//! `ValueSpec::Categories` lists training, testing and validation values in that order, which is
//! what `@PARTITION_FIELD`, `@TRAINING_PARTITION` etc. use.
//!
//! Assignment (`crate::random`): one `java.util.Random(seed).nextInt(100)` draw per record; the
//! record goes to the first partition whose cumulative percentage exceeds the draw
//! (`oracle/random/part_*`, every record of seven cases). GAP: sizes that do not add up to 100
//! are scaled to 100 (no oracle), and without `partitionUseRandomSeed` Modeler seeds from the
//! clock, which we cannot reproduce.
//! Not implemented (error): `usePartitionHashColumn` (SQL generation by a unique field).

use super::field_util::with_columns;
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::random::JavaRandom;
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Field, Measurement, Role, Storage, Table, Value, ValueSpec};
use std::sync::Arc;

struct Partition {
    name: String,
    /// Values of the partitions in order: training, testing, (validation).
    values: Vec<Value>,
    storage: Storage,
    /// Cumulative upper bounds in percent for each partition.
    bounds: Vec<f64>,
    seed: Option<i64>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let text = |path: &str, default: &str| {
        settings::text(s, path)
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| default.to_string())
    };
    if settings::flag(s, "usePartitionHashColumn")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "usePartitionHashColumn (partition by a unique field) not supported yet".into(),
        ));
    }
    let name = text("partitionFieldName", "Partition");
    let size = |path: &str, default: i64| -> Result<f64, OperatorError> {
        Ok(settings::integer(s, path)?.unwrap_or(default).clamp(0, 100) as f64)
    };
    let with_validation = settings::flag(s, "createValidationPartition")?.unwrap_or(false);
    let mut sizes = vec![
        size("trainingPartitionSize", 50)?,
        size("testingPartitionSize", 50)?,
    ];
    let mut labels = vec![
        text("trainingPartitionLabel", "Training"),
        text("testingPartitionLabel", "Testing"),
    ];
    if with_validation {
        sizes.push(size("validationPartitionSize", 0)?);
        labels.push(text("validationPartitionLabel", "Validation"));
    }
    let total: f64 = sizes.iter().sum();
    if total <= 0.0 {
        return Err(OperatorError::Settings(
            "partition sizes: at least one size must be positive".into(),
        ));
    }
    let mut running = 0.0;
    let bounds = sizes
        .iter()
        .map(|size| {
            running += size;
            running * 100.0 / total
        })
        .collect();
    let (values, storage) = match settings::text(s, "partitionValueMode")
        .as_deref()
        .map(str::trim)
    {
        Some("label") => (
            labels.iter().map(|l| Value::string(l.as_str())).collect(),
            Storage::String,
        ),
        Some("system") => (
            (1..=labels.len())
                .map(|n| Value::string(n.to_string()))
                .collect(),
            Storage::String,
        ),
        _ => (
            labels
                .iter()
                .enumerate()
                .map(|(i, l)| Value::string(format!("{}_{l}", i + 1)))
                .collect(),
            Storage::String,
        ),
    };
    let seed = if settings::flag(s, "partitionUseRandomSeed")?.unwrap_or(true) {
        Some(settings::integer(s, "partitionRandomSeed")?.unwrap_or(1_234_567))
    } else {
        None
    };
    Ok(Box::new(Partition {
        name,
        values,
        storage,
        bounds,
        seed,
    }))
}

impl Operator for Partition {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Partition needs one input".into()))?;
        let seed = self.seed.unwrap_or_else(clock_seed);
        let mut random = JavaRandom::new(seed);
        let column = (0..input.row_count())
            .map(|_| {
                let draw = f64::from(random.next_int(100));
                let index = self
                    .bounds
                    .iter()
                    .position(|bound| draw < *bound)
                    .unwrap_or(self.bounds.len().saturating_sub(1));
                self.values.get(index).cloned().unwrap_or(Value::Null)
            })
            .collect();
        let mut field = Field::new(&self.name, self.storage);
        field.role = Role::Partition;
        field.measurement = Measurement::Nominal;
        field.values = ValueSpec::Categories(self.values.clone());
        with_columns(
            input,
            vec![super::field_util::Column {
                field,
                values: column,
            }],
        )
    }
}

fn clock_seed() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::from(d.subsec_nanos()) ^ d.as_secs() as i64)
}
