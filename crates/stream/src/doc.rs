//! Editable document wrapper. Execution continues to use `Stream` directly.
use crate::{Diagram, Stream, StreamNode};
use std::{collections::BTreeMap, fmt, sync::Arc};

pub(crate) const MAX_NODES: usize = 100_000;
pub(crate) const MAX_LINKS: usize = 400_000;
pub(crate) const MAX_LABEL: usize = 1_000;
pub(crate) const MAX_ATTACHMENT: usize = 256 << 20;
pub(crate) const MAX_BYTES: usize = 1 << 30;
pub(crate) const MAX_ATTACHMENTS: usize = 10_000;

#[derive(Debug, Clone)]
pub struct StreamDoc {
    pub stream: Stream,
    /// Imported ZIP members, excluding the stream document itself.
    pub attachments: BTreeMap<String, Arc<[u8]>>,
    pub meta: DocMeta,
}

#[derive(Debug, Clone, Default)]
pub struct DocMeta {
    pub imported_from: Option<String>,
    /// Unknown top-level keys. The reserved `node` key holds unknown node fields,
    /// keyed by uid, including nodes in subdiagrams (uids are document-wide unique).
    pub extra: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocError {
    Read(String),
    Format(String),
    TooLarge(String),
    Version(u32),
}
impl fmt::Display for DocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(s) => write!(f, "document I/O: {s}"),
            Self::Format(s) => write!(f, "invalid document: {s}"),
            Self::TooLarge(s) => write!(f, "document limit exceeded: {s}"),
            Self::Version(v) => write!(f, "unsupported document version {v}"),
        }
    }
}
impl std::error::Error for DocError {}
impl Default for StreamDoc {
    fn default() -> Self {
        Self::new()
    }
}
impl StreamDoc {
    pub fn new() -> Self {
        Self {
            stream: Stream {
                file_version: None,
                save_version: None,
                properties: None,
                diagram: Diagram::default(),
            },
            attachments: BTreeMap::new(),
            meta: DocMeta::default(),
        }
    }
    pub fn import_str(bytes: &[u8], file_name: &str) -> Result<Self, DocError> {
        if bytes.len() > MAX_BYTES {
            return Err(DocError::TooLarge("input exceeds 1 GiB".into()));
        }
        crate::native::bounded_stack(|| Self::read_str(bytes, file_name))
    }
    fn read_str(bytes: &[u8], file_name: &str) -> Result<Self, DocError> {
        let limits = dms_strfmt::ArchiveLimits {
            max_member_bytes: MAX_ATTACHMENT as u64,
            ..Default::default()
        };
        let archive =
            dms_strfmt::read_archive(bytes, &limits).map_err(|e| DocError::Read(e.to_string()))?;
        let stream = Stream::from_document(&archive.document)
            .map_err(|e| DocError::Format(e.to_string()))?;
        let mut attachments = BTreeMap::new();
        for member in archive.attachments {
            if attachments
                .insert(member.name.clone(), member.bytes.into())
                .is_some()
            {
                return Err(DocError::Format(format!(
                    "duplicate attachment {}",
                    member.name
                )));
            }
        }
        let doc = Self {
            stream,
            attachments,
            meta: DocMeta {
                imported_from: Some(file_name.to_string()),
                ..Default::default()
            },
        };
        crate::native::validate(&doc)?;
        Ok(doc)
    }
    pub fn node(&self, uid: &str) -> Option<&StreamNode> {
        self.stream.diagram.nodes.iter().find(|n| n.uid == uid)
    }
}

// TOML permits NaN. Treat all NaNs as equal so the requested Eq contract is reflexive.
impl PartialEq for DocMeta {
    fn eq(&self, other: &Self) -> bool {
        if self.imported_from != other.imported_from || self.extra.len() != other.extra.len() {
            return false;
        }
        let mut pending = Vec::new();
        for (key, value) in &self.extra {
            let Some(other) = other.extra.get(key) else {
                return false;
            };
            pending.push((value, other));
        }
        while let Some((a, b)) = pending.pop() {
            match (a, b) {
                (toml::Value::Float(a), toml::Value::Float(b))
                    if a == b || (a.is_nan() && b.is_nan()) => {}
                (toml::Value::Array(a), toml::Value::Array(b)) if a.len() == b.len() => {
                    pending.extend(a.iter().zip(b))
                }
                (toml::Value::Table(a), toml::Value::Table(b)) if a.len() == b.len() => {
                    for (key, value) in a {
                        let Some(other) = b.get(key) else {
                            return false;
                        };
                        pending.push((value, other));
                    }
                }
                _ if a == b => {}
                _ => return false,
            }
        }
        true
    }
}
impl Eq for DocMeta {}
