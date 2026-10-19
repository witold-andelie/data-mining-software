//! Data files (L2): delimited and fixed-width text, JSON, SAS, SPSS Statistics files, Excel.
//!
//! Readers turn file contents into [`dms_types::Table`]s following the settings of Modeler's
//! source nodes; writers produce files other tools (and the Modeler oracle comparison) read.

pub mod csv;
pub mod delimited;
pub mod fixed;
pub mod json;
pub mod odbc;
pub mod sas;
pub mod sav_export;
pub mod statistics;
pub mod text;
pub mod xlsx;
