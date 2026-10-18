//! Local-header ZIP reader for attachments that Python's `zipfile` rejects.
//!
//! Time Series nuggets (`corpus/models_next/ts_arima.str`, `data/0003.dat`) store a
//! 4-byte big-endian length followed by a ZIP whose local extra fields are IBM
//! length-prefixed metadata (`00 00 00 00`, or `mime-type` + `application/xml`),
//! not ZIP extra records (id + size). Flag bit 3 is set, so sizes live in a data
//! descriptor after the compressed bytes. Walking local headers and inflating raw
//! deflate recovers `.meta`, `0.xml`, `1.xml`, `datamodel.xml`, `TSDPOutput.json`.
//!
//! Rule (`ts_arima` `data/0003.dat`): skip `xlen` extra bytes; do not parse them
//! as ZIP extras. `zipfile.ZipFile` reports `Corrupt extra field 0000 (size=256)`.

use crate::container::ArchiveLimits;
use flate2::read::DeflateDecoder;
use std::fmt;
use std::io::Read;

/// One member recovered from a local-header walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipMember {
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZipWalkError {
    NotZip,
    Truncated,
    TooManyMembers(usize),
    MemberTooLarge { name: String, limit: u64 },
    ArchiveTooLarge { limit: u64 },
    NameTooLong(usize),
    ExtraTooLong(usize),
    UnsupportedMethod(u16),
    Inflate(String),
    Name(String),
}

impl fmt::Display for ZipWalkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZipWalkError::NotZip => f.write_str("not a ZIP local-header archive"),
            ZipWalkError::Truncated => f.write_str("ZIP local header is truncated"),
            ZipWalkError::TooManyMembers(count) => {
                write!(f, "{count} ZIP members exceed the limit")
            }
            ZipWalkError::MemberTooLarge { name, limit } => {
                write!(f, "{name} is larger than {limit} bytes")
            }
            ZipWalkError::ArchiveTooLarge { limit } => {
                write!(f, "ZIP members total more than {limit} bytes")
            }
            ZipWalkError::NameTooLong(len) => write!(f, "ZIP name length {len} exceeds the limit"),
            ZipWalkError::ExtraTooLong(len) => {
                write!(f, "ZIP extra field length {len} exceeds the limit")
            }
            ZipWalkError::UnsupportedMethod(method) => {
                write!(f, "ZIP compression method {method} is not supported")
            }
            ZipWalkError::Inflate(message) => write!(f, "ZIP deflate data is corrupt: {message}"),
            ZipWalkError::Name(message) => write!(f, "ZIP member name is not UTF-8: {message}"),
        }
    }
}

impl std::error::Error for ZipWalkError {}

const LOCAL_SIG: &[u8; 4] = b"PK\x03\x04";
const MAX_NAME: usize = 4 * 1024;
const MAX_EXTRA: usize = 64 * 1024;
const FLAG_DATA_DESCRIPTOR: u16 = 0x0008;

/// Bytes after a 4-byte big-endian length prefix, when that payload is a ZIP.
///
/// `ts_arima.str` `data/0003.dat` is `00 00 12 21` then `PK\x03\x04`.
pub fn zip_payload(bytes: &[u8]) -> Result<&[u8], ZipWalkError> {
    if bytes.starts_with(LOCAL_SIG) {
        return Ok(bytes);
    }
    let rest = bytes.get(4..).ok_or(ZipWalkError::NotZip)?;
    if rest.starts_with(LOCAL_SIG) {
        return Ok(rest);
    }
    Err(ZipWalkError::NotZip)
}

/// Members of a ZIP that may be length-prefixed and may use data descriptors.
pub fn read_length_prefixed_zip(
    bytes: &[u8],
    limits: &ArchiveLimits,
) -> Result<Vec<ZipMember>, ZipWalkError> {
    read_local_zip(zip_payload(bytes)?, limits)
}

/// Walks `PK\x03\x04` local headers and inflates each member.
pub fn read_local_zip(
    bytes: &[u8],
    limits: &ArchiveLimits,
) -> Result<Vec<ZipMember>, ZipWalkError> {
    let mut members = Vec::new();
    let mut total = 0u64;
    let mut offset = 0usize;
    while let Some(at) = find_sig(bytes, offset) {
        if members.len() >= limits.max_members {
            return Err(ZipWalkError::TooManyMembers(
                members.len().saturating_add(1),
            ));
        }
        let (member, consumed) = parse_local(bytes, at, limits)?;
        total =
            total
                .checked_add(member.bytes.len() as u64)
                .ok_or(ZipWalkError::ArchiveTooLarge {
                    limit: limits.max_total_bytes,
                })?;
        if total > limits.max_total_bytes {
            return Err(ZipWalkError::ArchiveTooLarge {
                limit: limits.max_total_bytes,
            });
        }
        let next = at.checked_add(consumed).ok_or(ZipWalkError::Truncated)?;
        members.push(member);
        offset = next;
    }
    if members.is_empty() {
        return Err(ZipWalkError::NotZip);
    }
    Ok(members)
}

fn find_sig(bytes: &[u8], from: usize) -> Option<usize> {
    let haystack = bytes.get(from..)?;
    haystack
        .windows(4)
        .position(|window| window == LOCAL_SIG)
        .and_then(|rel| from.checked_add(rel))
}

fn parse_local(
    bytes: &[u8],
    at: usize,
    limits: &ArchiveLimits,
) -> Result<(ZipMember, usize), ZipWalkError> {
    let header = bytes.get(at..).ok_or(ZipWalkError::Truncated)?;
    if header.len() < 30 {
        return Err(ZipWalkError::Truncated);
    }
    let flag = u16_le(header, 6)?;
    let method = u16_le(header, 8)?;
    let csize = u32_le(header, 18)? as usize;
    let nlen = u16_le(header, 26)? as usize;
    let xlen = u16_le(header, 28)? as usize;
    if nlen > MAX_NAME {
        return Err(ZipWalkError::NameTooLong(nlen));
    }
    if xlen > MAX_EXTRA {
        return Err(ZipWalkError::ExtraTooLong(xlen));
    }
    let name_at = 30usize;
    let extra_at = name_at.checked_add(nlen).ok_or(ZipWalkError::Truncated)?;
    let data_at = extra_at.checked_add(xlen).ok_or(ZipWalkError::Truncated)?;
    let name_bytes = header
        .get(name_at..extra_at)
        .ok_or(ZipWalkError::Truncated)?;
    let name = std::str::from_utf8(name_bytes)
        .map_err(|error| ZipWalkError::Name(error.to_string()))?
        .to_string();
    let _extra = header
        .get(extra_at..data_at)
        .ok_or(ZipWalkError::Truncated)?;
    let payload = header.get(data_at..).ok_or(ZipWalkError::Truncated)?;
    let max_out = usize::try_from(limits.max_member_bytes).unwrap_or(usize::MAX);
    let (out, consumed) = match method {
        0 => stored(payload, csize, flag, &name, max_out)?,
        8 => deflate(payload, csize, flag, &name, max_out)?,
        other => return Err(ZipWalkError::UnsupportedMethod(other)),
    };
    if (out.len() as u64) > limits.max_member_bytes {
        return Err(ZipWalkError::MemberTooLarge {
            name,
            limit: limits.max_member_bytes,
        });
    }
    let total = data_at
        .checked_add(consumed)
        .ok_or(ZipWalkError::Truncated)?;
    Ok((ZipMember { name, bytes: out }, total))
}

fn stored(
    payload: &[u8],
    csize: usize,
    flag: u16,
    name: &str,
    max_out: usize,
) -> Result<(Vec<u8>, usize), ZipWalkError> {
    if flag & FLAG_DATA_DESCRIPTOR != 0 && csize == 0 {
        return Err(ZipWalkError::Inflate(format!(
            "{name}: stored member has a data descriptor and no size"
        )));
    }
    if csize > max_out {
        return Err(ZipWalkError::MemberTooLarge {
            name: name.to_string(),
            limit: max_out as u64,
        });
    }
    let data = payload.get(..csize).ok_or(ZipWalkError::Truncated)?;
    Ok((data.to_vec(), csize))
}

fn deflate(
    payload: &[u8],
    csize: usize,
    flag: u16,
    name: &str,
    max_out: usize,
) -> Result<(Vec<u8>, usize), ZipWalkError> {
    let known_size = csize > 0 && flag & FLAG_DATA_DESCRIPTOR == 0;
    let input = if known_size {
        payload.get(..csize).ok_or(ZipWalkError::Truncated)?
    } else {
        payload
    };
    let (out, consumed) = inflate_raw(input, max_out)
        .map_err(|message| ZipWalkError::Inflate(format!("{name}: {message}")))?;
    let advanced = if known_size { csize } else { consumed };
    Ok((out, advanced))
}

fn inflate_raw(input: &[u8], max_out: usize) -> Result<(Vec<u8>, usize), String> {
    let mut decoder = DeflateDecoder::new(input);
    let mut out = Vec::new();
    decoder
        .by_ref()
        .take(max_out as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|error| error.to_string())?;
    if out.len() > max_out {
        return Err("uncompressed member exceeds the limit".into());
    }
    let consumed = usize::try_from(decoder.total_in())
        .map_err(|_| "compressed length does not fit".to_string())?;
    if consumed > input.len() {
        return Err("deflate consumed past the input".into());
    }
    Ok((out, consumed))
}

fn u16_le(bytes: &[u8], at: usize) -> Result<u16, ZipWalkError> {
    let end = at.checked_add(2).ok_or(ZipWalkError::Truncated)?;
    let slice = bytes.get(at..end).ok_or(ZipWalkError::Truncated)?;
    let arr: [u8; 2] = slice.try_into().map_err(|_| ZipWalkError::Truncated)?;
    Ok(u16::from_le_bytes(arr))
}

fn u32_le(bytes: &[u8], at: usize) -> Result<u32, ZipWalkError> {
    let end = at.checked_add(4).ok_or(ZipWalkError::Truncated)?;
    let slice = bytes.get(at..end).ok_or(ZipWalkError::Truncated)?;
    let arr: [u8; 4] = slice.try_into().map_err(|_| ZipWalkError::Truncated)?;
    Ok(u32::from_le_bytes(arr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compress, Compression, FlushCompress, Status};

    fn limits() -> ArchiveLimits {
        ArchiveLimits::default()
    }

    fn raw_deflate(data: &[u8]) -> Vec<u8> {
        let mut encoder = Compress::new(Compression::default(), false);
        let mut out = vec![0u8; data.len().saturating_add(64)];
        let status = encoder
            .compress(data, &mut out, FlushCompress::Finish)
            .expect("compress");
        assert!(matches!(status, Status::StreamEnd));
        out.truncate(encoder.total_out() as usize);
        out
    }

    fn local(name: &[u8], extra: &[u8], flag: u16, method: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(LOCAL_SIG);
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&flag.to_le_bytes());
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        let csize = if flag & FLAG_DATA_DESCRIPTOR == 0 {
            payload.len() as u32
        } else {
            0
        };
        out.extend_from_slice(&csize.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(extra);
        out.extend_from_slice(payload);
        out
    }

    fn prefixed(inner: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        out.extend_from_slice(inner);
        out
    }

    #[test]
    fn inflates_data_descriptor_and_ibm_extra() {
        // `ts_arima` `.meta` extra is four zero bytes; Python zipfile calls that corrupt.
        let body = b"<PMML version=\"4.3\"><TimeSeriesModel/></PMML>";
        let compressed = raw_deflate(body);
        let extra = [0u8, 0, 0, 0];
        let zip = local(
            b"0.xml",
            &extra,
            FLAG_DATA_DESCRIPTOR | 0x0800,
            8,
            &compressed,
        );
        let wrapped = prefixed(&zip);
        let members = read_length_prefixed_zip(&wrapped, &limits()).expect("walk");
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].name, "0.xml");
        assert_eq!(members[0].bytes, body);
    }

    #[test]
    fn reads_two_deflated_members() {
        let a = raw_deflate(b"alpha");
        let b = raw_deflate(b"beta");
        let mut zip = local(b"0.xml", b"", FLAG_DATA_DESCRIPTOR, 8, &a);
        zip.extend(local(b"1.xml", b"", FLAG_DATA_DESCRIPTOR, 8, &b));
        let members = read_local_zip(&zip, &limits()).expect("walk");
        assert_eq!(members[0].name, "0.xml");
        assert_eq!(members[0].bytes, b"alpha");
        assert_eq!(members[1].name, "1.xml");
        assert_eq!(members[1].bytes, b"beta");
    }

    #[test]
    fn stored_member_without_descriptor() {
        let zip = local(b"plain.txt", b"", 0, 0, b"hello");
        let members = read_local_zip(&zip, &limits()).expect("walk");
        assert_eq!(members[0].bytes, b"hello");
    }

    #[test]
    fn truncated_header_is_an_error() {
        assert!(matches!(
            read_local_zip(b"PK\x03\x04\x00", &limits()),
            Err(ZipWalkError::Truncated | ZipWalkError::NotZip)
        ));
    }

    #[test]
    fn rejects_non_zip_payload() {
        assert!(matches!(
            read_length_prefixed_zip(b"\0\0\0\x03abc", &limits()),
            Err(ZipWalkError::NotZip)
        ));
    }
}
