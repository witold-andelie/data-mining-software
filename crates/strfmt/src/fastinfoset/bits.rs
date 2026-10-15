//! Bit-level reader. X.891 Annex C defines every encoding bit by bit, most significant bit
//! first, with each construct starting on a stated bit of an octet; reading bit fields keeps the
//! decoder a direct transcription of those clauses.

use super::{Error, ErrorKind};

pub(crate) struct Bits<'a> {
    data: &'a [u8],
    /// Absolute position in bits from the start of `data`.
    pos: usize,
    /// Offset of `data` within the caller's input, for error positions.
    base: usize,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8], base: usize) -> Self {
        Self { data, pos: 0, base }
    }

    /// Bit position within the current octet as X.891 counts it (1 = first bit).
    pub fn bit_in_octet(&self) -> usize {
        self.pos % 8 + 1
    }

    pub fn is_at_end(&self) -> bool {
        self.pos >= self.data.len().saturating_mul(8)
    }

    fn remaining_bits(&self) -> usize {
        self.data.len().saturating_mul(8).saturating_sub(self.pos)
    }

    pub fn remaining_octets(&self) -> usize {
        self.remaining_bits() / 8
    }

    pub fn error(&self, kind: ErrorKind) -> Error {
        self.error_at(self.pos, kind)
    }

    /// The current position, to report an error at the start of a construct.
    pub fn mark(&self) -> usize {
        self.pos
    }

    pub fn error_at(&self, mark: usize, kind: ErrorKind) -> Error {
        Error {
            offset: self.base.saturating_add(mark / 8),
            bit: mark % 8 + 1,
            kind,
        }
    }

    /// The next `n` (at most 32) bits as an unsigned integer, without consuming them.
    pub fn peek(&self, n: u32) -> Result<u32, Error> {
        if n > 32 {
            return Err(self.error(ErrorKind::Internal("bit field wider than 32 bits")));
        }
        if n as usize > self.remaining_bits() {
            return Err(self.error(ErrorKind::UnexpectedEnd));
        }
        let mut value = 0u32;
        for i in 0..n as usize {
            let index = self.pos + i;
            let octet = self.data.get(index / 8).copied().unwrap_or(0);
            let bit = (octet >> (7 - index % 8)) & 1;
            value = (value << 1) | u32::from(bit);
        }
        Ok(value)
    }

    pub fn read(&mut self, n: u32) -> Result<u32, Error> {
        let value = self.peek(n)?;
        self.pos += n as usize;
        Ok(value)
    }

    pub fn read_flag(&mut self) -> Result<bool, Error> {
        Ok(self.read(1)? == 1)
    }

    /// `n` padding bits, which X.891 requires to be zero.
    pub fn padding(&mut self, n: u32) -> Result<(), Error> {
        let start = self.error(ErrorKind::InvalidPadding);
        if self.read(n)? == 0 {
            Ok(())
        } else {
            Err(start)
        }
    }

    /// `len` whole octets; the reader must be at the start of an octet.
    pub fn octets(&mut self, len: u64) -> Result<&'a [u8], Error> {
        if !self.pos.is_multiple_of(8) {
            return Err(self.error(ErrorKind::Internal("octet string not octet-aligned")));
        }
        let len = usize::try_from(len).map_err(|_| self.error(ErrorKind::UnexpectedEnd))?;
        if len > self.remaining_octets() {
            return Err(self.error(ErrorKind::UnexpectedEnd));
        }
        let start = self.pos / 8;
        let slice = self
            .data
            .get(start..start + len)
            .ok_or_else(|| self.error(ErrorKind::UnexpectedEnd))?;
        self.pos += len * 8;
        Ok(slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_fields_across_octets() {
        let mut bits = Bits::new(&[0b1010_1100, 0b0101_0000], 0);
        assert_eq!(bits.read(1).unwrap(), 1);
        assert_eq!(bits.read(4).unwrap(), 0b0101);
        assert_eq!(bits.bit_in_octet(), 6);
        assert_eq!(bits.read(6).unwrap(), 0b100_010);
        assert_eq!(bits.read(5).unwrap(), 0b1_0000);
        assert!(bits.is_at_end());
        assert_eq!(bits.read(1).unwrap_err().kind, ErrorKind::UnexpectedEnd);
    }

    #[test]
    fn rejects_nonzero_padding_and_unaligned_octets() {
        let mut bits = Bits::new(&[0b0100_0000, 7], 10);
        assert!(bits.padding(1).is_ok());
        let err = bits.padding(1).unwrap_err();
        assert_eq!((err.offset, err.kind), (10, ErrorKind::InvalidPadding));
        assert_eq!(
            bits.octets(1).unwrap_err().kind,
            ErrorKind::Internal("octet string not octet-aligned")
        );
    }

    #[test]
    fn octet_lengths_beyond_input_fail() {
        let mut bits = Bits::new(&[1, 2], 0);
        assert_eq!(bits.octets(3).unwrap_err().kind, ErrorKind::UnexpectedEnd);
        assert_eq!(
            bits.octets(u64::MAX).unwrap_err().kind,
            ErrorKind::UnexpectedEnd
        );
        assert_eq!(bits.octets(2).unwrap(), &[1, 2]);
    }
}
