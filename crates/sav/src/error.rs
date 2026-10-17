//! Error type: a byte offset into the input plus a kind.

use std::fmt;

/// What went wrong while reading or writing a system file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// The input ended inside the named structure.
    Truncated(&'static str),
    /// The first four bytes are not `$FL2` / `$FL3`.
    BadMagic,
    /// The layout code is neither 2 nor 3 in either byte order.
    BadLayoutCode(i32),
    /// A valid but unsupported variant (EBCDIC, IBM/VAX floats, ...).
    Unsupported(&'static str),
    /// A structure violates the format.
    Malformed(&'static str),
    /// A [`crate::Limits`] bound was exceeded.
    LimitExceeded(&'static str),
    /// ZLIB data could not be decompressed or does not match its trailer.
    Decompress(String),
    /// The writer refused the input.
    Write(String),
}

/// A read failure at `offset` (bytes from the start of the input).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub offset: usize,
    pub kind: ErrorKind,
}

impl Error {
    pub(crate) fn new(offset: usize, kind: ErrorKind) -> Self {
        Self { offset, kind }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sav: at byte {}: ", self.offset)?;
        match &self.kind {
            ErrorKind::Truncated(what) => write!(f, "input ends inside {what}"),
            ErrorKind::BadMagic => write!(f, "not a system file (bad record type tag)"),
            ErrorKind::BadLayoutCode(code) => write!(f, "unrecognised layout code {code}"),
            ErrorKind::Unsupported(what) => write!(f, "unsupported: {what}"),
            ErrorKind::Malformed(what) => write!(f, "malformed: {what}"),
            ErrorKind::LimitExceeded(what) => write!(f, "limit exceeded: {what}"),
            ErrorKind::Decompress(what) => write!(f, "decompression failed: {what}"),
            ErrorKind::Write(what) => write!(f, "{what}"),
        }
    }
}

impl std::error::Error for Error {}
