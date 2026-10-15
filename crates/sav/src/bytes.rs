//! Endian-aware bounds-checked byte cursor.

use crate::error::{Error, ErrorKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endian {
    Little,
    Big,
}

impl Endian {
    pub(crate) fn i32(self, b: [u8; 4]) -> i32 {
        match self {
            Endian::Little => i32::from_le_bytes(b),
            Endian::Big => i32::from_be_bytes(b),
        }
    }
    pub(crate) fn i64(self, b: [u8; 8]) -> i64 {
        match self {
            Endian::Little => i64::from_le_bytes(b),
            Endian::Big => i64::from_be_bytes(b),
        }
    }
    pub(crate) fn f64(self, b: [u8; 8]) -> f64 {
        match self {
            Endian::Little => f64::from_le_bytes(b),
            Endian::Big => f64::from_be_bytes(b),
        }
    }
    pub(crate) fn f64_bytes(self, v: f64) -> [u8; 8] {
        match self {
            Endian::Little => v.to_le_bytes(),
            Endian::Big => v.to_be_bytes(),
        }
    }
}

/// Cursor over `data`; error offsets are `base + position`.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    base: usize,
    endian: Endian,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], base: usize, endian: Endian) -> Self {
        Self {
            data,
            pos: 0,
            base,
            endian,
        }
    }

    pub fn offset(&self) -> usize {
        self.base.saturating_add(self.pos)
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn err(&self, kind: ErrorKind) -> Error {
        Error::new(self.offset(), kind)
    }

    pub fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n);
        match end.and_then(|end| self.data.get(self.pos..end)) {
            Some(slice) => {
                self.pos += n;
                Ok(slice)
            }
            None => Err(self.err(ErrorKind::Truncated(what))),
        }
    }

    pub fn array<const N: usize>(&mut self, what: &'static str) -> Result<[u8; N], Error> {
        let slice = self.take(N, what)?;
        <[u8; N]>::try_from(slice).map_err(|_| self.err(ErrorKind::Truncated(what)))
    }

    pub fn u8(&mut self, what: &'static str) -> Result<u8, Error> {
        Ok(self.array::<1>(what)?[0])
    }

    pub fn i32(&mut self, what: &'static str) -> Result<i32, Error> {
        let b = self.array::<4>(what)?;
        Ok(self.endian.i32(b))
    }

    pub fn i64(&mut self, what: &'static str) -> Result<i64, Error> {
        let b = self.array::<8>(what)?;
        Ok(self.endian.i64(b))
    }

    pub fn f64(&mut self, what: &'static str) -> Result<f64, Error> {
        let b = self.array::<8>(what)?;
        Ok(self.endian.f64(b))
    }

    /// An `int32` that must be a non-negative length or count.
    pub fn len(&mut self, what: &'static str) -> Result<usize, Error> {
        let at = self.offset();
        let v = self.i32(what)?;
        usize::try_from(v).map_err(|_| Error::new(at, ErrorKind::Malformed(what)))
    }
}
