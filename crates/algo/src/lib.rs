//! Training algorithms (L2): each trains on a [`Dataset`] and returns a PMML document
//! ([`dms_pmml::Pmml`]), the same representation models read from streams use, so a trained
//! model scores through `dms-pmml` and compares element by element with the PMML Modeler exports
//! (`docs/models.md`).
//!
//! Formulas follow IBM SPSS Modeler 19 Algorithms Guide (public;
//! `D:\spss-modeler\rea-work\ibm-docs\AlgorithmsGuide.txt` locally). Where the guide leaves a
//! choice open, the decision is marked `GAP:` and settled against the oracle.

pub mod adp;
pub mod anomaly;
pub mod apriori;
pub mod arima;
pub mod c50;
pub mod c50_boost;
pub mod c50_rules;
pub mod carma;
pub mod cart;
pub mod chaid;
pub mod cluster_evaluation;
mod common;
pub mod decision_list;
pub mod expert;
pub mod expert_tf;
pub mod factor;
pub mod feature_selection;
pub mod gle;
pub mod kmeans;
pub mod kohonen;
pub mod linear;
pub mod linear_as;
pub mod logistic;
pub mod lsvm;
pub mod model_comparison;
pub mod neural_net;
pub mod quest;
pub mod random_trees;
pub mod sequence;
pub mod smooth;
pub mod stats;
pub mod time_series;
pub mod transfer;
pub mod tree_as;
pub mod twostep;

pub use common::{
    categories, data_dictionary, data_field, field_categories, mining_field, mining_schema,
};

use dms_types::Table;
use std::fmt;

/// The records a model trains on and the roles of their fields.
#[derive(Debug, Clone, Copy)]
pub struct Dataset<'a> {
    pub table: &'a Table,
    /// Position of the target field.
    pub target: usize,
    /// Positions of the input fields, in the order the model lists them.
    pub inputs: &'a [usize],
    /// Position of a frequency weight field (`Role::Frequency`), if any.
    pub frequency: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlgoError {
    Cancelled,
    /// Settings the algorithm cannot use.
    Settings(String),
    /// Data the algorithm cannot train on (no records, a target without values, ...).
    Data(String),
    /// Valid settings we do not implement yet.
    Unsupported(String),
}

impl fmt::Display for AlgoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AlgoError::Cancelled => f.write_str("cancelled"),
            AlgoError::Settings(message) => write!(f, "invalid settings: {message}"),
            AlgoError::Data(message) => write!(f, "cannot train: {message}"),
            AlgoError::Unsupported(what) => write!(f, "not supported yet: {what}"),
        }
    }
}

impl std::error::Error for AlgoError {}

impl From<dms_types::Cancelled> for AlgoError {
    fn from(_: dms_types::Cancelled) -> Self {
        Self::Cancelled
    }
}

thread_local! {
    // Training is synchronous on its worker. A scoped token also reaches numerical
    // callbacks and Auto/ensemble candidates without changing their numerical APIs.
    static CANCEL: std::cell::RefCell<Option<dms_types::CancelToken>> = const { std::cell::RefCell::new(None) };
}

/// Run synchronous training with a cancellation token. Scopes are thread-local and
/// nest safely; dropping a scope restores the previous token, including on unwind.
/// Callers outside an execution scope retain the original, never-cancelled API.
pub fn with_cancel<T, E: From<dms_types::Cancelled>>(
    cancel: &dms_types::CancelToken,
    work: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    cancel.check()?;
    let previous = CANCEL.with(|slot| {
        slot.try_borrow_mut()
            .map(|mut slot| slot.replace(cancel.clone()))
            .map_err(|_| dms_types::Cancelled)
    })?;
    struct Restore(Option<dms_types::CancelToken>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CANCEL.with(|slot| {
                if let Ok(mut slot) = slot.try_borrow_mut() {
                    *slot = self.0.take();
                }
            });
        }
    }
    let _restore = Restore(previous);
    let result = work();
    // Some candidate searches intentionally ignore failed candidates. Cancellation
    // still has to escape them, and must never publish an incomplete model.
    cancel.check()?;
    result
}

/// Cheap checkpoint for iterations and record batches in synchronous algorithms.
pub fn check_cancelled() -> Result<(), dms_types::Cancelled> {
    CANCEL.with(|slot| {
        let slot = slot.try_borrow().map_err(|_| dms_types::Cancelled)?;
        match slot.as_ref() {
            Some(cancel) => cancel.check(),
            None => Ok(()),
        }
    })
}
