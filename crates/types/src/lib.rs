//! Data model (L0): storage types, measurement levels, roles, values, fields and tables.
//!
//! Shared vocabulary of every layer that reads, transforms or models data. Values are plain
//! Rust data; interpretation that depends on stream settings (date baselines, formats) happens in
//! the layers that own those settings.

pub mod calendar;
pub mod cancel;
pub mod field;
pub mod table;
pub mod value;

pub use cancel::{CancelToken, Cancelled};

pub use field::{
    CheckMode, DataModel, DisplayFormat, Field, Measurement, MissingSpec, ModelError, Role,
    Storage, ValueSpec,
};
pub use table::Table;
pub use value::Value;
