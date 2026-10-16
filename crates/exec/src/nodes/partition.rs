//! Partition (Field Operations): adds a field that assigns each record to the training, testing
//! (and optionally validation) partition.
//!
//! Implemented: `partitionFieldName`, `trainingPartitionSize`/`testingPartitionSize`/
//! `validationPartitionSize` (percentages; the validation partition exists when
//! `createValidationPartition` is true; sizes are used as proportions of their sum),
//! `partitionRandomSeed` (with `partitionUseRandomSeed`; without it the seed comes from the clock),
//! the partition labels and `partitionValueMode` (`system_label` gives `1_Training`, `label`
//! gives `Training`, `system` gives 1, 2, 3). The output field has `Role::Partition` and its
//! `ValueSpec::Categories` lists training, testing and validation values in that order, which is
//! what `@PARTITION_FIELD`, `@TRAINING_PARTITION` etc. use.
//!
//! GAP: Modeler's random number generator and assignment procedure are undocumented, so
//! records are assigned by an independent, deterministic generator seeded from the setting
//! (one draw per record). The proportions hold on average but the individual assignment does NOT
//! match Modeler's, so streams that depend on exact partition membership will differ.
//! Not implemented (error): `usePartitionHashColumn` (SQL generation by a unique field).

use super::field_util::with_columns;
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Field, Measurement, Role, Storage, Table, Value, ValueSpec};
use std::sync::Arc;

struct Partition {
    name: String,
    /// Values of the partitions in order: training, testing, (validation).
    values: Vec<Value>,
    storage: Storage,
    /// Cumulative upper bounds in [0, 1] for each partition.
    bounds: Vec<f64>,
    seed: Option<u64>,
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
            running += size / total;
            running
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
                .map(|n| Value::Integer(n as i64))
                .collect(),
            Storage::Integer,
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
        let seed = settings::integer(s, "partitionRandomSeed")?.unwrap_or(1_234_567);
        Some(u64::try_from(seed).unwrap_or(0))
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
                let draw = random.next_double();
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

fn clock_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::from(d.subsec_nanos()) ^ d.as_secs())
}

/// A 48-bit linear congruential generator (the well-known `java.util.Random` recurrence, from
/// its public specification), used only as a reproducible source of uniform draws.
struct JavaRandom {
    state: u64,
}

const MASK: u64 = (1 << 48) - 1;

impl JavaRandom {
    fn new(seed: u64) -> JavaRandom {
        JavaRandom {
            state: (seed ^ 0x5DEE_CE66D) & MASK,
        }
    }

    fn next(&mut self, bits: u32) -> u64 {
        self.state = self.state.wrapping_mul(0x5DEE_CE66D).wrapping_add(0xB) & MASK;
        self.state >> (48 - bits)
    }

    fn next_double(&mut self) -> f64 {
        let high = self.next(26);
        let low = self.next(27);
        ((high << 27) + low) as f64 / (1u64 << 53) as f64
    }
}
