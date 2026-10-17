//! Training algorithms (L2): each trains on a [`Dataset`] and returns a PMML document
//! ([`dms_pmml::Pmml`]), the same representation models read from streams use, so a trained
//! model scores through `dms-pmml` and compares element by element with the PMML Modeler exports
//! (`docs/models.md`).
//!
//! Formulas follow IBM SPSS Modeler 19 Algorithms Guide (public;
//! `D:\spss-modeler\rea-work\ibm-docs\AlgorithmsGuide.txt` locally). Where the guide leaves a
//! choice open, the decision is marked `GAP:` and settled against the oracle.

pub mod adp;
pub mod cart;
pub mod chaid;
mod common;
pub mod feature_selection;
pub mod kmeans;
pub mod linear;
pub mod logistic;
pub mod stats;
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
            AlgoError::Settings(message) => write!(f, "invalid settings: {message}"),
            AlgoError::Data(message) => write!(f, "cannot train: {message}"),
            AlgoError::Unsupported(what) => write!(f, "not supported yet: {what}"),
        }
    }
}

impl std::error::Error for AlgoError {}
