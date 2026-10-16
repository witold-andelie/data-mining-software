//! Sample (Record Ops), simple sampling only.
//!
//! Settings: `sampleProcedure` (`simple` default; `complex` is rejected), `sampleMode` (`pass` =
//! include the sample, default | `discard` = pass the records outside the sample), `sampleMethod`
//! (`firstN` default | `oneInN` | `random`), `sampleFirstN` (default 10000), `sampleOneInN`
//! (default 2, >= 1), `sampleRandom` (percent in 0..=100, default 50.0),
//! `sampleUseMaxSize`/`sampleMaxSize` (cap on the size of the sample, default 10000),
//! `sampleUseRandomSeed`/`sampleRandomSeed` (non-negative seed). Encodings per
//! `docs/formats/nodes/sample.md`.
//!
//! GAP: random sampling does NOT match Modeler. Modeler's random number generator is unknown and
//! not reproducible here, so we draw from a deterministic seeded generator (splitmix64; without a
//! saved seed a fixed constant). The same input and seed always give the same sample, but not
//! Modeler's sample.
//! GAP: for `oneInN` we select records 1, n+1, 2n+1, ... (0-based `index % n == 0`); which record
//! of each block Modeler selects is unverified.
//! Not supported (`build` fails): `sampleProcedure=complex` (strata, clusters, weights).

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::Table;
use std::sync::Arc;

/// Seed used when the node does not ask for a fixed one (deterministic by design).
const DEFAULT_SEED: u64 = 0x5EED_0000_D15C_0001;

/// Deterministic generator (splitmix64) shared with Balance. Not Modeler's generator.
pub(crate) struct SeededRng(u64);

impl SeededRng {
    pub(crate) fn new(seed: u64) -> SeededRng {
        SeededRng(seed)
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub(crate) fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

enum Method {
    First(usize),
    OneInN(usize),
    RandomPercent(f64),
}

struct Sample {
    discard: bool,
    method: Method,
    max_size: Option<usize>,
    seed: u64,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    match settings::text(s, "sampleProcedure")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("simple") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "sampleProcedure {other:?}: complex sampling not supported yet"
            )));
        }
    }
    let discard = match settings::text(s, "sampleMode").as_deref().map(str::trim) {
        None | Some("") | Some("pass") => false,
        Some("discard") => true,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "sampleMode: expected pass or discard, found {other:?}"
            )));
        }
    };
    let count = |path: &str, default: i64| -> Result<usize, OperatorError> {
        let value = settings::integer(s, path)?.unwrap_or(default);
        usize::try_from(value)
            .map_err(|_| OperatorError::Settings(format!("{path}: negative value {value}")))
    };
    let method = match settings::text(s, "sampleMethod").as_deref().map(str::trim) {
        None | Some("") | Some("firstN") => Method::First(count("sampleFirstN", 10000)?),
        Some("oneInN") => {
            let n = count("sampleOneInN", 2)?;
            if n == 0 {
                return Err(OperatorError::Settings(
                    "sampleOneInN: must be at least 1".into(),
                ));
            }
            Method::OneInN(n)
        }
        Some("random") => {
            let percent = settings::real(s, "sampleRandom")?.unwrap_or(50.0);
            if !(0.0..=100.0).contains(&percent) {
                return Err(OperatorError::Settings(format!(
                    "sampleRandom: {percent} is not a percentage"
                )));
            }
            Method::RandomPercent(percent)
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "sampleMethod: unknown value {other:?}"
            )));
        }
    };
    let max_size = if settings::flag(s, "sampleUseMaxSize")?.unwrap_or(false) {
        Some(count("sampleMaxSize", 10000)?)
    } else {
        None
    };
    let seed = if settings::flag(s, "sampleUseRandomSeed")?.unwrap_or(false) {
        let raw = settings::integer(s, "sampleRandomSeed")?.unwrap_or(0);
        u64::try_from(raw).map_err(|_| {
            OperatorError::Settings(format!("sampleRandomSeed: negative value {raw}"))
        })?
    } else {
        DEFAULT_SEED
    };
    Ok(Box::new(Sample {
        discard,
        method,
        max_size,
        seed,
    }))
}

impl Operator for Sample {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Sample needs one input".into()))?;
        let mut rng = SeededRng::new(self.seed);
        let mut selected_count = 0usize;
        let mut kept = Vec::new();
        for (index, row) in input.rows().iter().enumerate() {
            let mut selected = match self.method {
                Method::First(n) => index < n,
                Method::OneInN(n) => index % n == 0,
                Method::RandomPercent(percent) => rng.next_f64() * 100.0 < percent,
            };
            if selected {
                if self.max_size.is_some_and(|max| selected_count >= max) {
                    selected = false;
                } else {
                    selected_count += 1;
                }
            }
            if selected != self.discard {
                kept.push(row.clone());
            }
        }
        Table::new(input.model().clone(), kept)
            .map_err(|error| OperatorError::Data(error.to_string()))
    }
}
