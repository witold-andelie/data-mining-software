//! Data files (L2): delimited text, SPSS Statistics files, Excel.
//!
//! Readers turn file contents into [`dms_types::Table`]s following the settings of Modeler's
//! source nodes; writers produce files other tools (and the Modeler oracle comparison) read.

pub mod csv;
pub mod delimited;
pub mod statistics;
