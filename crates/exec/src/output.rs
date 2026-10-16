//! What a node produces: data for the nodes downstream, a model (build nodes) or a document
//! (output nodes such as Data Audit, Statistics, Matrix).

use dms_pmml::Pmml;
use dms_types::Table;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum NodeOutput {
    Data(Arc<Table>),
    /// A trained model; the model nuggets linked to the build node score with it.
    Model(Arc<Pmml>),
    Document(Arc<Document>),
}

impl NodeOutput {
    pub fn data(&self) -> Option<&Arc<Table>> {
        match self {
            NodeOutput::Data(table) => Some(table),
            _ => None,
        }
    }

    pub fn model(&self) -> Option<&Arc<Pmml>> {
        match self {
            NodeOutput::Model(model) => Some(model),
            _ => None,
        }
    }

    pub fn document(&self) -> Option<&Arc<Document>> {
        match self {
            NodeOutput::Document(document) => Some(document),
            _ => None,
        }
    }
}

/// An output node's result, as text the way Modeler shows and exports it, so it compares with
/// the oracle's `outputs/` files (`docs/oracle.md`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    pub title: String,
    pub tables: Vec<DocumentTable>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentTable {
    /// The section the table belongs to (`Statistics`, `Pearson Correlations`, ...).
    pub title: Option<String>,
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}
