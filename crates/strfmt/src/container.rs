//! The `.str` container: a ZIP archive holding one fast infoset document (the stream XML) and
//! optional attachments under `data/`. Attachments are classified by their leading bytes and kept
//! verbatim; their contents (model nuggets, outputs, ...) are decoded elsewhere.

use crate::fastinfoset::{self, Document};
use std::fmt;
use std::io::{Cursor, Read};

/// Resource bounds for reading untrusted archives.
#[derive(Debug, Clone, Copy)]
pub struct ArchiveLimits {
    pub max_member_bytes: u64,
    pub max_total_bytes: u64,
    pub max_members: usize,
    pub fast_infoset: fastinfoset::Limits,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_member_bytes: 512 << 20,
            max_total_bytes: 1 << 30,
            max_members: 10_000,
            fast_infoset: fastinfoset::Limits::default(),
        }
    }
}

/// A decoded `.str` file.
#[derive(Debug, Clone)]
pub struct StreamArchive {
    /// ZIP member name of the stream document (for example `druglearn.xml`).
    pub document_member: String,
    pub document: Document,
    /// Members other than the stream document, in archive order.
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub kind: AttachmentKind,
    pub bytes: Vec<u8>,
}

/// What an attachment's leading bytes show it to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentKind {
    Empty,
    XmlText,
    FastInfoset,
    Zip,
    Gzip,
    Json,
    /// A 4-byte big-endian length followed by a payload of that kind.
    LengthPrefixed {
        declared_length: u32,
        /// Whether the declared length equals the bytes that follow it.
        length_matches: bool,
        payload: PayloadKind,
    },
    /// Four leading bytes that match none of the above.
    Unrecognized([u8; 4]),
    /// Fewer than four bytes that match none of the above.
    Short,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadKind {
    Empty,
    XmlText,
    FastInfoset,
    Zip,
    Gzip,
    Json,
    Unrecognized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveError {
    Zip(String),
    MissingDocument,
    MultipleDocuments(Vec<String>),
    TooManyMembers(usize),
    MemberTooLarge {
        name: String,
        limit: u64,
    },
    ArchiveTooLarge {
        limit: u64,
    },
    Read {
        name: String,
        message: String,
    },
    Document {
        name: String,
        error: fastinfoset::Error,
    },
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArchiveError::Zip(message) => write!(f, "not a readable ZIP archive: {message}"),
            ArchiveError::MissingDocument => {
                f.write_str("no stream document (*.xml outside data/)")
            }
            ArchiveError::MultipleDocuments(names) => {
                write!(f, "several stream documents: {}", names.join(", "))
            }
            ArchiveError::TooManyMembers(count) => write!(f, "{count} members exceed the limit"),
            ArchiveError::MemberTooLarge { name, limit } => {
                write!(f, "{name} is larger than {limit} bytes")
            }
            ArchiveError::ArchiveTooLarge { limit } => {
                write!(f, "members total more than {limit} bytes")
            }
            ArchiveError::Read { name, message } => write!(f, "cannot read {name}: {message}"),
            ArchiveError::Document { name, error } => write!(f, "{name}: {error}"),
        }
    }
}

impl std::error::Error for ArchiveError {}

/// Reads and decodes a `.str` file from its bytes.
pub fn read_archive(bytes: &[u8], limits: &ArchiveLimits) -> Result<StreamArchive, ArchiveError> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| ArchiveError::Zip(e.to_string()))?;
    if archive.len() > limits.max_members {
        return Err(ArchiveError::TooManyMembers(archive.len()));
    }
    let mut members = Vec::with_capacity(archive.len());
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|e| ArchiveError::Zip(e.to_string()))?;
        if file.is_dir() {
            continue;
        }
        let name = file
            .name()
            .map_err(|e| ArchiveError::Zip(e.to_string()))?
            .to_string();
        let mut data = Vec::new();
        file.by_ref()
            .take(limits.max_member_bytes.saturating_add(1))
            .read_to_end(&mut data)
            .map_err(|e| ArchiveError::Read {
                name: name.clone(),
                message: e.to_string(),
            })?;
        let length = data.len() as u64;
        if length > limits.max_member_bytes {
            return Err(ArchiveError::MemberTooLarge {
                name,
                limit: limits.max_member_bytes,
            });
        }
        total = total.saturating_add(length);
        if total > limits.max_total_bytes {
            return Err(ArchiveError::ArchiveTooLarge {
                limit: limits.max_total_bytes,
            });
        }
        members.push((name, data));
    }

    let documents: Vec<usize> = members
        .iter()
        .enumerate()
        .filter(|(_, (name, _))| is_document_member(name))
        .map(|(index, _)| index)
        .collect();
    let document_index = match documents.as_slice() {
        [] => return Err(ArchiveError::MissingDocument),
        [index] => *index,
        _ => {
            return Err(ArchiveError::MultipleDocuments(
                documents
                    .iter()
                    .filter_map(|index| members.get(*index).map(|(name, _)| name.clone()))
                    .collect(),
            ));
        }
    };
    let (document_member, document_bytes) = members.remove(document_index);
    let document = fastinfoset::decode(&document_bytes, &limits.fast_infoset).map_err(|error| {
        ArchiveError::Document {
            name: document_member.clone(),
            error,
        }
    })?;
    let attachments = members
        .into_iter()
        .map(|(name, bytes)| Attachment {
            kind: classify(&bytes),
            name,
            bytes,
        })
        .collect();
    Ok(StreamArchive {
        document_member,
        document,
        attachments,
    })
}

/// The stream document is the `.xml` member outside `data/`.
fn is_document_member(name: &str) -> bool {
    !name.starts_with("data/") && name.to_ascii_lowercase().ends_with(".xml")
}

/// Classifies an attachment by its leading bytes.
pub fn classify(bytes: &[u8]) -> AttachmentKind {
    match payload_kind(bytes) {
        PayloadKind::Empty => return AttachmentKind::Empty,
        PayloadKind::XmlText => return AttachmentKind::XmlText,
        PayloadKind::FastInfoset => return AttachmentKind::FastInfoset,
        PayloadKind::Zip => return AttachmentKind::Zip,
        PayloadKind::Gzip => return AttachmentKind::Gzip,
        PayloadKind::Json => return AttachmentKind::Json,
        PayloadKind::Unrecognized => {}
    }
    let Some(prefix) = bytes.get(..4) else {
        return AttachmentKind::Short;
    };
    let mut head = [0u8; 4];
    head.copy_from_slice(prefix);
    let rest = bytes.get(4..).unwrap_or_default();
    let payload = payload_kind(rest);
    if payload != PayloadKind::Unrecognized && payload != PayloadKind::Empty {
        let declared_length = u32::from_be_bytes(head);
        return AttachmentKind::LengthPrefixed {
            declared_length,
            length_matches: u64::from(declared_length) == rest.len() as u64,
            payload,
        };
    }
    AttachmentKind::Unrecognized(head)
}

fn payload_kind(bytes: &[u8]) -> PayloadKind {
    let trimmed = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .and_then(|start| bytes.get(start..))
        .unwrap_or_default();
    if bytes.is_empty() {
        PayloadKind::Empty
    } else if bytes.starts_with(&[0xE0, 0x00, 0x00, 0x01]) {
        PayloadKind::FastInfoset
    } else if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        PayloadKind::Zip
    } else if bytes.starts_with(&[0x1F, 0x8B]) {
        PayloadKind::Gzip
    } else if trimmed.starts_with(b"<") {
        PayloadKind::XmlText
    } else if trimmed.starts_with(b"{") || trimmed.starts_with(b"[") {
        PayloadKind::Json
    } else {
        PayloadKind::Unrecognized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_attachment_prefixes() {
        assert_eq!(classify(b""), AttachmentKind::Empty);
        assert_eq!(
            classify(b"<?xml version=\"1.0\"?><a/>"),
            AttachmentKind::XmlText
        );
        assert_eq!(classify(b"PK\x03\x04rest"), AttachmentKind::Zip);
        assert_eq!(classify(&[0x1F, 0x8B, 8, 0]), AttachmentKind::Gzip);
        assert_eq!(
            classify(b"\0\0\0\x05<a/>!"),
            AttachmentKind::LengthPrefixed {
                declared_length: 5,
                length_matches: true,
                payload: PayloadKind::XmlText,
            }
        );
        assert_eq!(
            classify(b"\0\0\0\x09{\"t\":1}"),
            AttachmentKind::LengthPrefixed {
                declared_length: 9,
                length_matches: false,
                payload: PayloadKind::Json,
            }
        );
        assert_eq!(
            classify(b"SCF\x01\0\0\0\0"),
            AttachmentKind::Unrecognized(*b"SCF\x01")
        );
        assert_eq!(classify(b"\0\0"), AttachmentKind::Short);
    }

    #[test]
    fn rejects_non_zip_input() {
        assert!(matches!(
            read_archive(b"not a zip", &ArchiveLimits::default()),
            Err(ArchiveError::Zip(_))
        ));
    }

    #[test]
    fn identifies_the_document_member() {
        assert!(is_document_member("druglearn.xml"));
        assert!(is_document_member("clementineStream.XML"));
        assert!(!is_document_member("data/0001.xml"));
        assert!(!is_document_member("data/0001.dat"));
    }
}
