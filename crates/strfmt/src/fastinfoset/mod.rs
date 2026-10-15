//! Fast Infoset decoder, implemented from ITU-T Rec. X.891 (05/2005) | ISO/IEC 24824-1:2007.
//!
//! IBM SPSS Modeler stores the XML of a `.str` stream as a fast infoset document. This module
//! decodes any fast infoset document into an XML infoset tree, following Annex C (bit-level
//! encodings), clause 7 (vocabulary table indexes), clause 9 (built-in restricted alphabets) and
//! clause 10 (built-in encoding algorithms). External vocabularies and user-defined encoding
//! algorithms cannot be resolved from the document alone and are reported as unsupported.
//!
//! Strings are shared (`Arc<str>`): a vocabulary table reference reuses the table entry instead
//! of copying it, so the decoded tree stays proportional to the input size.

mod algorithms;
mod bits;
mod decoder;
mod xml;

use std::fmt;
use std::sync::Arc;

pub use decoder::decode;

/// Resource bounds for decoding untrusted input.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum element nesting depth.
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_depth: 1024 }
    }
}

/// A decoded fast infoset document (the XML infoset of the document information item).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document {
    /// The XML declaration that preceded the binary header, if any (X.891 12.3).
    pub xml_declaration: Option<String>,
    pub additional_data: Vec<AdditionalDatum>,
    pub notations: Vec<Notation>,
    pub unparsed_entities: Vec<UnparsedEntity>,
    pub character_encoding_scheme: Option<Arc<str>>,
    pub standalone: Option<bool>,
    pub version: Option<Arc<str>>,
    pub children: Vec<Node>,
}

impl Document {
    /// The document element (the first element child).
    pub fn root(&self) -> Option<&Element> {
        self.children.iter().find_map(Node::as_element)
    }

    /// Serializes the infoset as XML text (for inspection and tests).
    pub fn to_xml(&self) -> String {
        xml::write_document(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdditionalDatum {
    pub id: Arc<str>,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notation {
    pub name: Arc<str>,
    pub system_identifier: Option<Arc<str>>,
    pub public_identifier: Option<Arc<str>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnparsedEntity {
    pub name: Arc<str>,
    pub system_identifier: Arc<str>,
    pub public_identifier: Option<Arc<str>>,
    pub notation_name: Arc<str>,
}

/// A child information item of a document or element.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    /// A character chunk; consecutive chunks are kept as separate nodes.
    Text(Arc<str>),
    Comment(Arc<str>),
    ProcessingInstruction {
        target: Arc<str>,
        content: Arc<str>,
    },
    UnexpandedEntityReference {
        name: Arc<str>,
        system_identifier: Option<Arc<str>>,
        public_identifier: Option<Arc<str>>,
    },
    DocumentTypeDeclaration {
        system_identifier: Option<Arc<str>>,
        public_identifier: Option<Arc<str>>,
        /// Processing instructions inside the declaration: (target, content).
        processing_instructions: Vec<(Arc<str>, Arc<str>)>,
    },
}

impl Node {
    pub fn as_element(&self) -> Option<&Element> {
        match self {
            Node::Element(element) => Some(element),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QualifiedName {
    pub prefix: Option<Arc<str>>,
    pub namespace: Option<Arc<str>>,
    pub local: Arc<str>,
}

impl fmt::Display for QualifiedName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.prefix {
            Some(prefix) => write!(f, "{prefix}:{}", self.local),
            None => f.write_str(&self.local),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceDeclaration {
    pub prefix: Option<Arc<str>>,
    pub namespace: Option<Arc<str>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub name: QualifiedName,
    pub value: Arc<str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub name: QualifiedName,
    pub namespace_declarations: Vec<NamespaceDeclaration>,
    pub attributes: Vec<Attribute>,
    pub children: Vec<Node>,
}

impl Element {
    /// The value of the first attribute with this local name.
    pub fn attribute(&self, local: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| &*attribute.name.local == local)
            .map(|attribute| &*attribute.value)
    }

    pub fn child_elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(Node::as_element)
    }

    /// The first child element with this local name.
    pub fn child(&self, local: &str) -> Option<&Element> {
        self.child_elements()
            .find(|element| &*element.name.local == local)
    }

    /// Concatenated character chunks directly inside this element.
    pub fn text(&self) -> String {
        self.children
            .iter()
            .filter_map(|node| match node {
                Node::Text(text) => Some(&**text),
                _ => None,
            })
            .collect()
    }
}

/// Drops nested elements iteratively: the decoder accepts nesting up to `Limits::max_depth`, and
/// the default recursive drop of a deep tree would overflow the call stack.
impl Drop for Element {
    fn drop(&mut self) {
        let mut pending = std::mem::take(&mut self.children);
        while let Some(node) = pending.pop() {
            if let Node::Element(mut element) = node {
                pending.append(&mut element.children);
            }
        }
    }
}

/// A decoding failure at an input position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// Octet offset in the input where the failing construct was being read.
    pub offset: usize,
    /// Bit within that octet (1 = first bit), as X.891 counts bits.
    pub bit: usize,
    pub kind: ErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    UnexpectedEnd,
    /// The input does not start with the fast infoset identification and version.
    NotFastInfoset,
    InvalidPadding,
    /// Bits that do not identify any information item allowed at this position.
    InvalidIdentification(u32),
    IndexOutOfRange {
        table: Table,
        index: u32,
    },
    /// A name surrogate or qualified name with a prefix but no namespace name (X.891 7.16.6).
    PrefixWithoutNamespace,
    InvalidUtf8,
    InvalidUtf16,
    InvalidRestrictedAlphabetData,
    InvalidAlgorithmData {
        algorithm: u32,
        reason: &'static str,
    },
    Unsupported(&'static str),
    LimitExceeded(&'static str),
    TrailingData,
    Internal(&'static str),
}

/// The vocabulary tables of X.891 clause 8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table {
    Prefix,
    NamespaceName,
    LocalName,
    OtherNcName,
    OtherUri,
    AttributeValue,
    ContentCharacterChunk,
    OtherString,
    ElementName,
    AttributeName,
    RestrictedAlphabet,
    EncodingAlgorithm,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fast infoset octet {} bit {}: ", self.offset, self.bit)?;
        match &self.kind {
            ErrorKind::UnexpectedEnd => f.write_str("unexpected end of input"),
            ErrorKind::NotFastInfoset => f.write_str("not a fast infoset document"),
            ErrorKind::InvalidPadding => f.write_str("non-zero padding bits"),
            ErrorKind::InvalidIdentification(bits) => {
                write!(f, "unrecognized identification bits {bits:#b}")
            }
            ErrorKind::IndexOutOfRange { table, index } => {
                write!(f, "index {index} not in the {table:?} table")
            }
            ErrorKind::PrefixWithoutNamespace => f.write_str("prefix without a namespace name"),
            ErrorKind::InvalidUtf8 => f.write_str("invalid UTF-8"),
            ErrorKind::InvalidUtf16 => f.write_str("invalid UTF-16"),
            ErrorKind::InvalidRestrictedAlphabetData => {
                f.write_str("invalid restricted-alphabet data")
            }
            ErrorKind::InvalidAlgorithmData { algorithm, reason } => {
                write!(f, "encoding algorithm {algorithm}: {reason}")
            }
            ErrorKind::Unsupported(what) => write!(f, "unsupported: {what}"),
            ErrorKind::LimitExceeded(what) => write!(f, "limit exceeded: {what}"),
            ErrorKind::TrailingData => f.write_str("data after the end of the document"),
            ErrorKind::Internal(what) => write!(f, "internal decoder error: {what}"),
        }
    }
}

impl std::error::Error for Error {}
