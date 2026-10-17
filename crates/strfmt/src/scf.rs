//! `SCF\x01` model attachments: zlib-wrapped XML.
//!
//! Neural Net nuggets store PMML this way (`corpus/models_ml/nn_fixed_a.str`, member
//! `data/0002.dat`). The same wrapper holds StatXML and SaveXML members of that stream; this
//! module returns the XML and does not interpret it. The trailer after the compressed block
//! (a second small zlib, then `00 00 00 00 FF FF`) is not part of the document.
//!
//! Layout, big-endian, verified on `nn_fixed_a.str` (`data/0002.dat` PMML, `data/0003.dat`
//! StatXML, `data/0028.dat` SaveXML):
//!
//! - `0..4` magic `SCF\x01`
//! - `4..8` u32 `0`
//! - `8..12` u32 `20 + compressed_len`
//! - `12..16` u32 compressed length
//! - `16..20` u32 uncompressed length
//! - `20 .. 20 + compressed_len` zlib (`78 9C`). Modeler omits the Adler32 trailer: inflating
//!   that slice yields exactly `uncompressed_len` bytes and then `BufError`. A stream that
//!   still carries Adler32 (`StreamEnd`) is accepted too.
//!
//! Uncompressed payload: four zero bytes, `0x03`, `0x00`, one length byte, that many bytes of
//! `mimeType=...` text, then the XML. On `nn_fixed_a` the PMML metadata is 56 bytes (`0x38`)
//! and the document starts at offset 63.

use flate2::{Decompress, FlushDecompress, Status};
use std::fmt;

/// Uncompressed XML larger than this is rejected. PMML parsing uses the same 64 MiB cap.
const MAX_UNCOMPRESSED: usize = 64 * 1024 * 1024;

const MAGIC: &[u8] = b"SCF\x01";

/// The model XML inside an `SCF\x01` attachment.
pub fn read_model_xml(bytes: &[u8]) -> Result<String, ScfError> {
    if bytes.len() < 20 || !bytes.starts_with(MAGIC) {
        return Err(ScfError::new("not an SCF model"));
    }
    let size_field = read_u32(bytes, 8)?;
    let compressed_len_u = read_u32(bytes, 12)?;
    let uncompressed_len_u = read_u32(bytes, 16)?;
    let expected_size = compressed_len_u
        .checked_add(20)
        .ok_or_else(|| ScfError::new("SCF size overflows"))?;
    if size_field != expected_size {
        return Err(ScfError::new(
            "SCF size field does not match the compressed length",
        ));
    }
    let uncompressed_len = usize::try_from(uncompressed_len_u)
        .map_err(|_| ScfError::new("SCF uncompressed length does not fit"))?;
    if uncompressed_len == 0 {
        return Err(ScfError::new("SCF document is empty"));
    }
    if uncompressed_len > MAX_UNCOMPRESSED {
        return Err(ScfError::new("SCF uncompressed document exceeds 64 MiB"));
    }
    let compressed_len = usize::try_from(compressed_len_u)
        .map_err(|_| ScfError::new("SCF compressed length does not fit"))?;
    let end = 20usize
        .checked_add(compressed_len)
        .ok_or_else(|| ScfError::new("SCF compressed range overflows"))?;
    let payload = bytes
        .get(20..end)
        .ok_or_else(|| ScfError::new("SCF compressed bytes are truncated"))?;

    let mut out = vec![0u8; uncompressed_len];
    let mut decoder = Decompress::new(true);
    let status = decoder
        .decompress(payload, &mut out, FlushDecompress::Finish)
        .map_err(|_| ScfError::new("SCF zlib data is corrupt"))?;
    match status {
        Status::StreamEnd | Status::BufError => {}
        Status::Ok => return Err(ScfError::new("SCF zlib stream is incomplete")),
    }
    if decoder.total_out() != uncompressed_len as u64 || decoder.total_in() != payload.len() as u64
    {
        return Err(ScfError::new(
            "SCF uncompressed length does not match the zlib output",
        ));
    }
    extract_xml(&out)
}

fn extract_xml(payload: &[u8]) -> Result<String, ScfError> {
    let prefix = payload
        .get(..6)
        .ok_or_else(|| ScfError::new("SCF payload is shorter than its header"))?;
    if prefix != [0, 0, 0, 0, 3, 0] {
        return Err(ScfError::new("SCF payload is not a wrapped XML document"));
    }
    let meta_len = usize::from(
        payload
            .get(6)
            .copied()
            .ok_or_else(|| ScfError::new("SCF payload is shorter than its header"))?,
    );
    let xml_at = 7usize
        .checked_add(meta_len)
        .ok_or_else(|| ScfError::new("SCF metadata length overflows"))?;
    let xml = payload
        .get(xml_at..)
        .ok_or_else(|| ScfError::new("SCF metadata length overruns the payload"))?;
    if !xml.starts_with(b"<") {
        return Err(ScfError::new("SCF payload is not XML"));
    }
    let text = std::str::from_utf8(xml).map_err(|_| ScfError::new("SCF XML is not UTF-8"))?;
    Ok(text.to_string())
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, ScfError> {
    let b0 = byte_at(bytes, at)?;
    let b1 = byte_at(
        bytes,
        at.checked_add(1)
            .ok_or_else(|| ScfError::new("SCF header is truncated"))?,
    )?;
    let b2 = byte_at(
        bytes,
        at.checked_add(2)
            .ok_or_else(|| ScfError::new("SCF header is truncated"))?,
    )?;
    let b3 = byte_at(
        bytes,
        at.checked_add(3)
            .ok_or_else(|| ScfError::new("SCF header is truncated"))?,
    )?;
    Ok(u32::from_be_bytes([b0, b1, b2, b3]))
}

fn byte_at(bytes: &[u8], at: usize) -> Result<u8, ScfError> {
    bytes
        .get(at)
        .copied()
        .ok_or_else(|| ScfError::new("SCF header is truncated"))
}

/// Failure to read an `SCF\x01` model attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScfError {
    message: String,
}

impl ScfError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ScfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SCF model: {}", self.message)
    }
}

impl std::error::Error for ScfError {}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;

    fn wrap(xml: &str, strip_adler: bool) -> Vec<u8> {
        let metadata = b"mimeType=application/xml\ntypeId=pmml";
        let mut payload = Vec::new();
        payload.extend_from_slice(&[0, 0, 0, 0, 3, 0]);
        payload.push(u8::try_from(metadata.len()).unwrap());
        payload.extend_from_slice(metadata);
        payload.extend_from_slice(xml.as_bytes());
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&payload).unwrap();
        let mut compressed = encoder.finish().unwrap();
        if strip_adler {
            compressed.truncate(compressed.len().saturating_sub(4));
        }
        let compressed_len = u32::try_from(compressed.len()).unwrap();
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&(20 + compressed_len).to_be_bytes());
        out.extend_from_slice(&compressed_len.to_be_bytes());
        out.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
        out.extend_from_slice(&compressed);
        out.extend_from_slice(&[0, 0, 0, 0, 0xFF, 0xFF]);
        out
    }

    #[test]
    fn reads_xml_with_and_without_adler32() {
        let xml = "<PMML version=\"4.3\"/>";
        assert_eq!(read_model_xml(&wrap(xml, false)).unwrap(), xml);
        assert_eq!(read_model_xml(&wrap(xml, true)).unwrap(), xml);
    }

    #[test]
    fn rejects_a_bad_header_and_corrupt_deflate() {
        assert!(read_model_xml(b"not scf").is_err());
        assert!(read_model_xml(b"SCF\x01").is_err());
        let mut bytes = wrap("<PMML/>", true);
        bytes[11] = bytes.get(11).copied().unwrap_or(0).wrapping_add(1);
        assert!(read_model_xml(&bytes).is_err());
        let mut corrupt = wrap("<PMML/>", false);
        if let Some(slot) = corrupt.get_mut(24) {
            *slot ^= 0xFF;
        }
        assert!(read_model_xml(&corrupt).is_err());
    }

    #[cfg(feature = "corpus")]
    #[test]
    fn unwraps_a_saved_neural_net_member() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .unwrap()
            .to_path_buf();
        let bytes = std::fs::read(root.join("corpus/models_ml/nn_fixed_a.str")).unwrap();
        let archive = crate::read_archive(&bytes, &crate::ArchiveLimits::default()).unwrap();
        let pmml = archive
            .attachments
            .iter()
            .filter(|attachment| attachment.bytes.starts_with(MAGIC))
            .filter_map(|attachment| read_model_xml(&attachment.bytes).ok())
            .find(|xml| xml.contains("<NeuralNetwork"));
        let pmml = pmml.expect("neural PMML member");
        assert!(pmml.contains("activationFunction=\"tanh\""));
        assert!(pmml.trim_end().ends_with("</PMML>"));
    }
}
