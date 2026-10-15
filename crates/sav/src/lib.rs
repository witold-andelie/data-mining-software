//! IBM SPSS Statistics system files (`.sav`, `.zsav`) reader (L0, standalone).
//!
//! Implemented from the public GNU PSPP Developers Guide chapter "System File Format"
//! (see the crate README). Strings are decoded with the file's declared encoding; numbers,
//! dates and user-missing values are left raw (see [`Variable::missing`] and
//! [`Format::is_date_time`]).

mod assemble;
mod bytes;
mod data;
mod dict;
mod error;
mod model;

pub use bytes::Endian;
pub use error::{Error, ErrorKind};
pub use model::{
    Alignment, Compression, Format, Header, Limits, Measure, MissingValues, RawExtension, SavFile,
    Value, ValueLabel, ValueLabelSet, Variable,
};

/// Reads a whole system file. Never panics; malformed input yields an [`Error`].
pub fn read(bytes: &[u8], limits: &Limits) -> Result<SavFile, Error> {
    let raw = dict::parse(bytes, limits)?;
    let data_offset = raw.data_offset;
    let mut assembled = assemble::assemble(raw)?;
    let params = data::DataParams {
        compression: assembled.file.header.compression,
        endian: assembled.file.header.endianness,
        bias: assembled.file.header.bias,
        sysmis: assembled.sysmis,
        plan: &assembled.plan,
        encoding: assembled.encoding,
        case_count: assembled.file.header.case_count,
        offset: data_offset,
    };
    let rows = data::read_rows(bytes, &params, limits)?;
    assembled.file.rows = rows;
    Ok(assembled.file)
}
