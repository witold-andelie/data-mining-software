//! Decoding of fast infoset documents (X.891 Annex C). Each function names the clause it
//! implements; "from bit k" is the bit within an octet (1 = first) at which the encoding starts.

use super::algorithms;
use super::bits::Bits;
use super::{
    AdditionalDatum, Attribute, Document, Element, Error, ErrorKind, Limits, NamespaceDeclaration,
    Node, Notation, QualifiedName, Table, UnparsedEntity,
};
use std::sync::Arc;

/// Maximum entries per vocabulary table ("one-meg", X.891 7.2.1).
const ONE_MEG: usize = 1 << 20;
const XML_PREFIX: &str = "xml";
const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
/// First vocabulary table index of restricted alphabets and encoding algorithms declared in an
/// initial vocabulary (X.891 7.2.19, 7.2.20).
const FIRST_USER_ALPHABET: u32 = 16;
const FIRST_USER_ALGORITHM: u32 = 32;
const LAST_BUILT_IN_ALGORITHM: u32 = 10;
/// Built-in restricted alphabets (X.891 clause 9).
const NUMERIC_ALPHABET: &str = "0123456789-+.e ";
const DATE_TIME_ALPHABET: &str = "0123456789-:TZ ";

/// Decodes a fast infoset document.
pub fn decode(input: &[u8], limits: &Limits) -> Result<Document, Error> {
    let (xml_declaration, body_start) = xml_declaration(input);
    let body = input.get(body_start..).unwrap_or_default();
    let mut decoder = Decoder {
        bits: Bits::new(body, body_start),
        vocabulary: Vocabulary::new(),
        limits: *limits,
    };
    let mut document = decoder.document()?;
    document.xml_declaration = xml_declaration;
    Ok(document)
}

/// An XML declaration may precede the binary header (X.891 12.3); it is short and ASCII.
fn xml_declaration(input: &[u8]) -> (Option<String>, usize) {
    if !input.starts_with(b"<?xml") {
        return (None, 0);
    }
    let window = input.get(..input.len().min(128)).unwrap_or_default();
    let Some(end) = window.windows(2).position(|pair| pair == b"?>") else {
        return (None, 0);
    };
    let declaration = window.get(..end + 2).unwrap_or_default();
    (
        Some(String::from_utf8_lossy(declaration).into_owned()),
        end + 2,
    )
}

/// The dynamic vocabulary tables of X.891 clause 8. Index `i` refers to element `i - 1`.
struct Vocabulary {
    prefixes: Vec<Arc<str>>,
    namespace_names: Vec<Arc<str>>,
    local_names: Vec<Arc<str>>,
    other_ncnames: Vec<Arc<str>>,
    other_uris: Vec<Arc<str>>,
    attribute_values: Vec<Arc<str>>,
    content_character_chunks: Vec<Arc<str>>,
    other_strings: Vec<Arc<str>>,
    element_names: Vec<QualifiedName>,
    attribute_names: Vec<QualifiedName>,
    /// Restricted alphabets from the initial vocabulary (indexes from 16).
    alphabets: Vec<Vec<char>>,
    /// User-defined encoding algorithms declared in the initial vocabulary (indexes from 32);
    /// only their URIs are known.
    user_algorithms: usize,
    empty: Arc<str>,
}

impl Vocabulary {
    fn new() -> Self {
        Self {
            prefixes: vec![Arc::from(XML_PREFIX)],
            namespace_names: vec![Arc::from(XML_NAMESPACE)],
            local_names: Vec::new(),
            other_ncnames: Vec::new(),
            other_uris: Vec::new(),
            attribute_values: Vec::new(),
            content_character_chunks: Vec::new(),
            other_strings: Vec::new(),
            element_names: Vec::new(),
            attribute_names: Vec::new(),
            alphabets: Vec::new(),
            user_algorithms: 0,
            empty: Arc::from(""),
        }
    }

    fn strings(&mut self, table: Table) -> Option<&mut Vec<Arc<str>>> {
        match table {
            Table::Prefix => Some(&mut self.prefixes),
            Table::NamespaceName => Some(&mut self.namespace_names),
            Table::LocalName => Some(&mut self.local_names),
            Table::OtherNcName => Some(&mut self.other_ncnames),
            Table::OtherUri => Some(&mut self.other_uris),
            Table::AttributeValue => Some(&mut self.attribute_values),
            Table::ContentCharacterChunk => Some(&mut self.content_character_chunks),
            Table::OtherString => Some(&mut self.other_strings),
            Table::ElementName
            | Table::AttributeName
            | Table::RestrictedAlphabet
            | Table::EncodingAlgorithm => None,
        }
    }

    fn names(&mut self, table: Table) -> Option<&mut Vec<QualifiedName>> {
        match table {
            Table::ElementName => Some(&mut self.element_names),
            Table::AttributeName => Some(&mut self.attribute_names),
            _ => None,
        }
    }
}

fn entry<T: Clone>(entries: &[T], index: u32) -> Option<T> {
    let position = usize::try_from(index).ok()?.checked_sub(1)?;
    entries.get(position).cloned()
}

enum Format {
    Utf8,
    Utf16,
    RestrictedAlphabet(u32),
    EncodingAlgorithm(u32),
}

struct Decoder<'a> {
    bits: Bits<'a>,
    vocabulary: Vocabulary,
    limits: Limits,
}

impl<'a> Decoder<'a> {
    fn error(&self, kind: ErrorKind) -> Error {
        self.bits.error(kind)
    }

    /// C.1 header and C.2 Document.
    fn document(&mut self) -> Result<Document, Error> {
        let not_fast_infoset = self.error(ErrorKind::NotFastInfoset);
        let identification = self.bits.read(16).map_err(|_| not_fast_infoset.clone())?;
        if identification != 0b1110_0000_0000_0000 {
            return Err(not_fast_infoset);
        }
        if self.bits.read(16)? != 1 {
            return Err(self.error(ErrorKind::Unsupported("fast infoset version other than 1")));
        }
        self.bits.padding(1)?;
        let flags = self.bits.read(7)?;
        let has = |component: u32| flags & (1 << (6 - component)) != 0;
        let mut document = Document::default();
        if has(0) {
            document.additional_data = self.additional_data()?;
        }
        if has(1) {
            self.initial_vocabulary()?;
        }
        if has(2) {
            document.notations = self.notations()?;
        }
        if has(3) {
            document.unparsed_entities = self.unparsed_entities()?;
        }
        if has(4) {
            self.bits.padding(1)?;
            let octets = self.octets_from_bit2()?;
            document.character_encoding_scheme = Some(self.utf8(octets)?);
        }
        if has(5) {
            self.bits.padding(7)?;
            document.standalone = Some(self.bits.read_flag()?);
        }
        if has(6) {
            document.version = Some(self.non_identifying_from_bit1(Table::OtherString)?);
        }
        document.children = self.children()?;
        // C.1.3: an encoding that ends on the fourth bit is padded to the octet boundary.
        if self.bits.bit_in_octet() == 5 {
            self.bits.padding(4)?;
        }
        if !self.bits.is_at_end() {
            return Err(self.error(ErrorKind::TrailingData));
        }
        Ok(document)
    }

    /// C.2.4
    fn additional_data(&mut self) -> Result<Vec<AdditionalDatum>, Error> {
        let count = self.sequence_length()?;
        let mut data = Vec::new();
        for _ in 0..count {
            self.bits.padding(1)?;
            let id_octets = self.octets_from_bit2()?;
            let id = self.utf8(id_octets)?;
            self.bits.padding(1)?;
            let data_octets = self.octets_from_bit2()?;
            data.push(AdditionalDatum {
                id,
                data: data_octets.to_vec(),
            });
        }
        Ok(data)
    }

    /// C.2.5
    fn initial_vocabulary(&mut self) -> Result<(), Error> {
        self.bits.padding(3)?;
        let flags = self.bits.read(13)?;
        let has = |component: u32| flags & (1 << (12 - component)) != 0;
        if has(0) {
            return Err(self.error(ErrorKind::Unsupported("external vocabulary")));
        }
        if has(1) {
            for _ in 0..self.sequence_length()? {
                self.bits.padding(1)?;
                let octets = self.octets_from_bit2()?;
                let alphabet: Vec<char> = self.utf8(octets)?.chars().collect();
                if alphabet.len() < 2 || alphabet.len() > ONE_MEG {
                    return Err(self.error(ErrorKind::InvalidRestrictedAlphabetData));
                }
                if self.vocabulary.alphabets.len() >= 256 {
                    return Err(self.error(ErrorKind::LimitExceeded("restricted alphabet table")));
                }
                self.vocabulary.alphabets.push(alphabet);
            }
        }
        if has(2) {
            for _ in 0..self.sequence_length()? {
                self.bits.padding(1)?;
                self.octets_from_bit2()?;
                self.vocabulary.user_algorithms += 1;
            }
        }
        let identifying = [
            (3, Table::Prefix),
            (4, Table::NamespaceName),
            (5, Table::LocalName),
            (6, Table::OtherNcName),
            (7, Table::OtherUri),
        ];
        for (component, table) in identifying {
            if has(component) {
                for _ in 0..self.sequence_length()? {
                    self.bits.padding(1)?;
                    let octets = self.octets_from_bit2()?;
                    let string = self.utf8(octets)?;
                    self.add_string(table, string)?;
                }
            }
        }
        let non_identifying = [
            (8, Table::AttributeValue),
            (9, Table::ContentCharacterChunk),
            (10, Table::OtherString),
        ];
        for (component, table) in non_identifying {
            if has(component) {
                for _ in 0..self.sequence_length()? {
                    self.bits.padding(2)?;
                    let string = self.encoded_string_from_bit3()?;
                    self.add_string(table, string)?;
                }
            }
        }
        for (component, table) in [(11, Table::ElementName), (12, Table::AttributeName)] {
            if has(component) {
                for _ in 0..self.sequence_length()? {
                    self.bits.padding(6)?;
                    let name = self.name_surrogate()?;
                    self.add_name(table, name)?;
                }
            }
        }
        Ok(())
    }

    /// C.2.6 and C.11
    fn notations(&mut self) -> Result<Vec<Notation>, Error> {
        let mut notations = Vec::new();
        loop {
            if self.bits.peek(4)? == 0b1111 {
                self.bits.read(4)?;
                self.bits.padding(4)?;
                return Ok(notations);
            }
            let identification = self.bits.read(6)?;
            if identification != 0b11_0000 {
                return Err(self.error(ErrorKind::InvalidIdentification(identification)));
            }
            let has_system = self.bits.read_flag()?;
            let has_public = self.bits.read_flag()?;
            let name = self.identifying(Table::OtherNcName)?;
            let system_identifier = self.optional_identifying(has_system, Table::OtherUri)?;
            let public_identifier = self.optional_identifying(has_public, Table::OtherUri)?;
            notations.push(Notation {
                name,
                system_identifier,
                public_identifier,
            });
        }
    }

    /// C.2.7 and C.10
    fn unparsed_entities(&mut self) -> Result<Vec<UnparsedEntity>, Error> {
        let mut entities = Vec::new();
        loop {
            if self.bits.peek(4)? == 0b1111 {
                self.bits.read(4)?;
                self.bits.padding(4)?;
                return Ok(entities);
            }
            let identification = self.bits.read(7)?;
            if identification != 0b110_1000 {
                return Err(self.error(ErrorKind::InvalidIdentification(identification)));
            }
            let has_public = self.bits.read_flag()?;
            let name = self.identifying(Table::OtherNcName)?;
            let system_identifier = self.identifying(Table::OtherUri)?;
            let public_identifier = self.optional_identifying(has_public, Table::OtherUri)?;
            let notation_name = self.identifying(Table::OtherNcName)?;
            entities.push(UnparsedEntity {
                name,
                system_identifier,
                public_identifier,
                notation_name,
            });
        }
    }

    /// Children of the document (C.2.11) and of every element (C.3.7), decoded iteratively with
    /// an explicit stack of open elements so nesting depth never grows the call stack.
    fn children(&mut self) -> Result<Vec<Node>, Error> {
        let mut open: Vec<Element> = Vec::new();
        let mut document_children = Vec::new();
        loop {
            let at_document = open.is_empty();
            // After a construct ending on the fourth bit, four padding bits precede the next item,
            // or the terminator of the enclosing item follows directly.
            if self.bits.bit_in_octet() == 5 {
                let padding_error = self.error(ErrorKind::InvalidPadding);
                match self.bits.read(4)? {
                    0b1111 => {
                        if close(&mut open, &mut document_children) {
                            return Ok(document_children);
                        }
                        continue;
                    }
                    0 => {}
                    _ => return Err(padding_error),
                }
            }
            if self.bits.bit_in_octet() != 1 {
                return Err(self.error(ErrorKind::Internal("item does not start on an octet")));
            }
            if self.bits.peek(4)? == 0b1111 {
                self.bits.read(4)?;
                if close(&mut open, &mut document_children) {
                    return Ok(document_children);
                }
                continue;
            }
            if self.bits.peek(1)? == 0 {
                self.bits.read(1)?;
                if open.len() >= self.limits.max_depth {
                    return Err(self.error(ErrorKind::LimitExceeded("element nesting depth")));
                }
                let element = self.element_start()?;
                open.push(element);
                continue;
            }
            let node = if !at_document && self.bits.peek(2)? == 0b10 {
                self.bits.read(2)?;
                Node::Text(self.non_identifying_from_bit3(Table::ContentCharacterChunk)?)
            } else {
                let identification = self.bits.peek(8)?;
                match identification {
                    0b1110_0001 => {
                        self.bits.read(8)?;
                        self.processing_instruction()?
                    }
                    0b1110_0010 => {
                        self.bits.read(8)?;
                        Node::Comment(self.non_identifying_from_bit1(Table::OtherString)?)
                    }
                    _ if !at_document && identification >> 2 == 0b11_0010 => {
                        self.bits.read(6)?;
                        self.unexpanded_entity_reference()?
                    }
                    _ if at_document && identification >> 2 == 0b11_0001 => {
                        self.bits.read(6)?;
                        self.document_type_declaration()?
                    }
                    _ => return Err(self.error(ErrorKind::InvalidIdentification(identification))),
                }
            };
            match open.last_mut() {
                Some(parent) => parent.children.push(node),
                None => document_children.push(node),
            }
        }
    }

    /// C.3.3 to C.3.6: everything of an element before its children, from bit 2.
    fn element_start(&mut self) -> Result<Element, Error> {
        let has_attributes = self.bits.read_flag()?;
        let mut namespace_declarations = Vec::new();
        if self.bits.peek(4)? == 0b1110 {
            self.bits.read(4)?;
            self.bits.padding(2)?;
            loop {
                if self.bits.peek(4)? == 0b1111 {
                    self.bits.read(4)?;
                    self.bits.padding(6)?;
                    break;
                }
                let identification = self.bits.read(6)?;
                if identification != 0b11_0011 {
                    return Err(self.error(ErrorKind::InvalidIdentification(identification)));
                }
                // C.12, from bit 7.
                let has_prefix = self.bits.read_flag()?;
                let has_namespace = self.bits.read_flag()?;
                let prefix = self.optional_identifying(has_prefix, Table::Prefix)?;
                let namespace = self.optional_identifying(has_namespace, Table::NamespaceName)?;
                namespace_declarations.push(NamespaceDeclaration { prefix, namespace });
            }
        }
        let name = self.qualified_name_from_bit3()?;
        let mut attributes = Vec::new();
        if has_attributes {
            loop {
                if self.bits.peek(4)? == 0b1111 {
                    self.bits.read(4)?;
                    break;
                }
                let identification = self.bits.read(1)?;
                if identification != 0 {
                    return Err(self.error(ErrorKind::InvalidIdentification(identification)));
                }
                let name = self.qualified_name_from_bit2()?;
                let value = self.non_identifying_from_bit1(Table::AttributeValue)?;
                attributes.push(Attribute { name, value });
            }
        }
        Ok(Element {
            name,
            namespace_declarations,
            attributes,
            children: Vec::new(),
        })
    }

    /// C.5, from bit 1 (after the identification octet).
    fn processing_instruction(&mut self) -> Result<Node, Error> {
        let target = self.identifying(Table::OtherNcName)?;
        let content = self.non_identifying_from_bit1(Table::OtherString)?;
        Ok(Node::ProcessingInstruction { target, content })
    }

    /// C.6, from bit 7.
    fn unexpanded_entity_reference(&mut self) -> Result<Node, Error> {
        let has_system = self.bits.read_flag()?;
        let has_public = self.bits.read_flag()?;
        let name = self.identifying(Table::OtherNcName)?;
        let system_identifier = self.optional_identifying(has_system, Table::OtherUri)?;
        let public_identifier = self.optional_identifying(has_public, Table::OtherUri)?;
        Ok(Node::UnexpandedEntityReference {
            name,
            system_identifier,
            public_identifier,
        })
    }

    /// C.9, from bit 7; ends on the fourth bit after its terminator.
    fn document_type_declaration(&mut self) -> Result<Node, Error> {
        let has_system = self.bits.read_flag()?;
        let has_public = self.bits.read_flag()?;
        let system_identifier = self.optional_identifying(has_system, Table::OtherUri)?;
        let public_identifier = self.optional_identifying(has_public, Table::OtherUri)?;
        let mut processing_instructions = Vec::new();
        loop {
            if self.bits.peek(4)? == 0b1111 {
                self.bits.read(4)?;
                break;
            }
            let identification = self.bits.read(8)?;
            if identification != 0b1110_0001 {
                return Err(self.error(ErrorKind::InvalidIdentification(identification)));
            }
            let target = self.identifying(Table::OtherNcName)?;
            let content = self.non_identifying_from_bit1(Table::OtherString)?;
            processing_instructions.push((target, content));
        }
        Ok(Node::DocumentTypeDeclaration {
            system_identifier,
            public_identifier,
            processing_instructions,
        })
    }

    /// C.16, from bit 7.
    fn name_surrogate(&mut self) -> Result<QualifiedName, Error> {
        let has_prefix = self.bits.read_flag()?;
        let has_namespace = self.bits.read_flag()?;
        if has_prefix && !has_namespace {
            return Err(self.error(ErrorKind::PrefixWithoutNamespace));
        }
        let prefix = if has_prefix {
            self.bits.padding(1)?;
            let mark = self.bits.mark();
            let index = self.integer_from_bit2()?;
            Some(self.lookup_string(Table::Prefix, index, mark)?)
        } else {
            None
        };
        let namespace = if has_namespace {
            self.bits.padding(1)?;
            let mark = self.bits.mark();
            let index = self.integer_from_bit2()?;
            Some(self.lookup_string(Table::NamespaceName, index, mark)?)
        } else {
            None
        };
        self.bits.padding(1)?;
        let mark = self.bits.mark();
        let index = self.integer_from_bit2()?;
        let local = self.lookup_string(Table::LocalName, index, mark)?;
        Ok(QualifiedName {
            prefix,
            namespace,
            local,
        })
    }

    /// C.17: an attribute name, from bit 2.
    fn qualified_name_from_bit2(&mut self) -> Result<QualifiedName, Error> {
        if self.bits.peek(4)? == 0b1111 {
            self.bits.read(4)?;
            self.bits.padding(1)?;
            self.literal_qualified_name(Table::AttributeName)
        } else {
            let mark = self.bits.mark();
            let index = self.integer_from_bit2()?;
            self.lookup_name(Table::AttributeName, index, mark)
        }
    }

    /// C.18: an element name, from bit 3.
    fn qualified_name_from_bit3(&mut self) -> Result<QualifiedName, Error> {
        if self.bits.peek(4)? == 0b1111 {
            self.bits.read(4)?;
            self.literal_qualified_name(Table::ElementName)
        } else {
            let mark = self.bits.mark();
            let index = self.integer_from_bit3()?;
            self.lookup_name(Table::ElementName, index, mark)
        }
    }

    /// C.17.3 / C.18.3: presence bits, then prefix, namespace name and local name (C.13). The
    /// name surrogate is added to the element or attribute name table (X.891 7.16.7).
    fn literal_qualified_name(&mut self, table: Table) -> Result<QualifiedName, Error> {
        let has_prefix = self.bits.read_flag()?;
        let has_namespace = self.bits.read_flag()?;
        if has_prefix && !has_namespace {
            return Err(self.error(ErrorKind::PrefixWithoutNamespace));
        }
        let prefix = self.optional_identifying(has_prefix, Table::Prefix)?;
        let namespace = self.optional_identifying(has_namespace, Table::NamespaceName)?;
        let local = self.identifying(Table::LocalName)?;
        let name = QualifiedName {
            prefix,
            namespace,
            local,
        };
        self.add_name(table, name.clone())?;
        Ok(name)
    }

    fn optional_identifying(
        &mut self,
        present: bool,
        table: Table,
    ) -> Result<Option<Arc<str>>, Error> {
        if present {
            self.identifying(table).map(Some)
        } else {
            Ok(None)
        }
    }

    /// C.13: a literal is always added to its table (X.891 7.13.7).
    fn identifying(&mut self, table: Table) -> Result<Arc<str>, Error> {
        if self.bits.read_flag()? {
            let mark = self.bits.mark();
            let index = self.integer_from_bit2()?;
            self.lookup_string(table, index, mark)
        } else {
            let octets = self.octets_from_bit2()?;
            let string = self.utf8(octets)?;
            self.add_string(table, string.clone())?;
            Ok(string)
        }
    }

    /// C.14; index zero denotes the empty string (X.891 7.14.6).
    fn non_identifying_from_bit1(&mut self, table: Table) -> Result<Arc<str>, Error> {
        if self.bits.read_flag()? {
            if self.bits.peek(7)? == 0b111_1111 {
                self.bits.read(7)?;
                return Ok(self.vocabulary.empty.clone());
            }
            let mark = self.bits.mark();
            let index = self.integer_from_bit2()?;
            self.lookup_string(table, index, mark)
        } else {
            let add_to_table = self.bits.read_flag()?;
            let string = self.encoded_string_from_bit3()?;
            if add_to_table {
                self.add_string(table, string.clone())?;
            }
            Ok(string)
        }
    }

    /// C.15
    fn non_identifying_from_bit3(&mut self, table: Table) -> Result<Arc<str>, Error> {
        if self.bits.read_flag()? {
            let mark = self.bits.mark();
            let index = self.integer_from_bit4()?;
            self.lookup_string(table, index, mark)
        } else {
            let add_to_table = self.bits.read_flag()?;
            let string = self.encoded_string_from_bit5()?;
            if add_to_table {
                self.add_string(table, string.clone())?;
            }
            Ok(string)
        }
    }

    /// C.19
    fn encoded_string_from_bit3(&mut self) -> Result<Arc<str>, Error> {
        let format = self.encoding_format()?;
        let octets = self.octets_from_bit5()?;
        self.decode_string(format, octets)
    }

    /// C.20
    fn encoded_string_from_bit5(&mut self) -> Result<Arc<str>, Error> {
        let format = self.encoding_format()?;
        let octets = self.octets_from_bit7()?;
        self.decode_string(format, octets)
    }

    /// C.19.3 / C.20.3, with C.29 for table indexes.
    fn encoding_format(&mut self) -> Result<Format, Error> {
        Ok(match self.bits.read(2)? {
            0b00 => Format::Utf8,
            0b01 => Format::Utf16,
            0b10 => Format::RestrictedAlphabet(self.bits.read(8)? + 1),
            _ => Format::EncodingAlgorithm(self.bits.read(8)? + 1),
        })
    }

    /// X.891 7.17
    fn decode_string(&self, format: Format, octets: &[u8]) -> Result<Arc<str>, Error> {
        match format {
            Format::Utf8 => self.utf8(octets),
            Format::Utf16 => {
                if !octets.len().is_multiple_of(2) {
                    return Err(self.error(ErrorKind::InvalidUtf16));
                }
                let units: Vec<u16> = octets
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .collect();
                String::from_utf16(&units)
                    .map(Arc::from)
                    .map_err(|_| self.error(ErrorKind::InvalidUtf16))
            }
            Format::RestrictedAlphabet(index) => {
                let built_in: Vec<char>;
                let alphabet: &[char] = match index {
                    1 => {
                        built_in = NUMERIC_ALPHABET.chars().collect();
                        &built_in
                    }
                    2 => {
                        built_in = DATE_TIME_ALPHABET.chars().collect();
                        &built_in
                    }
                    _ => index
                        .checked_sub(FIRST_USER_ALPHABET)
                        .and_then(|i| self.vocabulary.alphabets.get(i as usize))
                        .ok_or_else(|| {
                            self.error(ErrorKind::IndexOutOfRange {
                                table: Table::RestrictedAlphabet,
                                index,
                            })
                        })?,
                };
                restricted_alphabet(alphabet, octets)
                    .map(Arc::from)
                    .ok_or_else(|| self.error(ErrorKind::InvalidRestrictedAlphabetData))
            }
            Format::EncodingAlgorithm(index) if index <= LAST_BUILT_IN_ALGORITHM => {
                algorithms::decode(index, octets)
                    .map(Arc::from)
                    .map_err(|reason| {
                        self.error(ErrorKind::InvalidAlgorithmData {
                            algorithm: index,
                            reason,
                        })
                    })
            }
            Format::EncodingAlgorithm(index) => {
                let declared = index
                    .checked_sub(FIRST_USER_ALGORITHM)
                    .is_some_and(|i| (i as usize) < self.vocabulary.user_algorithms);
                Err(self.error(if declared {
                    ErrorKind::Unsupported("user-defined encoding algorithm")
                } else {
                    ErrorKind::IndexOutOfRange {
                        table: Table::EncodingAlgorithm,
                        index,
                    }
                }))
            }
        }
    }

    fn utf8(&self, octets: &[u8]) -> Result<Arc<str>, Error> {
        std::str::from_utf8(octets)
            .map(Arc::from)
            .map_err(|_| self.error(ErrorKind::InvalidUtf8))
    }

    fn add_string(&mut self, table: Table, string: Arc<str>) -> Result<(), Error> {
        let full = self.error(ErrorKind::LimitExceeded("vocabulary table full"));
        let internal = self.error(ErrorKind::Internal("not a string table"));
        let entries = self.vocabulary.strings(table).ok_or(internal)?;
        if entries.len() >= ONE_MEG {
            return Err(full);
        }
        entries.push(string);
        Ok(())
    }

    fn add_name(&mut self, table: Table, name: QualifiedName) -> Result<(), Error> {
        let full = self.error(ErrorKind::LimitExceeded("name table full"));
        let internal = self.error(ErrorKind::Internal("not a name table"));
        let entries = self.vocabulary.names(table).ok_or(internal)?;
        if entries.len() >= ONE_MEG {
            return Err(full);
        }
        entries.push(name);
        Ok(())
    }

    /// `mark` is the position of the index, where a missing entry is reported.
    fn lookup_string(&mut self, table: Table, index: u32, mark: usize) -> Result<Arc<str>, Error> {
        let missing = self
            .bits
            .error_at(mark, ErrorKind::IndexOutOfRange { table, index });
        let internal = self.error(ErrorKind::Internal("not a string table"));
        let entries = self.vocabulary.strings(table).ok_or(internal)?;
        entry(entries, index).ok_or(missing)
    }

    /// `mark` is the position of the index, where a missing entry is reported.
    fn lookup_name(
        &mut self,
        table: Table,
        index: u32,
        mark: usize,
    ) -> Result<QualifiedName, Error> {
        let missing = self
            .bits
            .error_at(mark, ErrorKind::IndexOutOfRange { table, index });
        let internal = self.error(ErrorKind::Internal("not a name table"));
        let entries = self.vocabulary.names(table).ok_or(internal)?;
        entry(entries, index).ok_or(missing)
    }

    /// C.21
    fn sequence_length(&mut self) -> Result<u32, Error> {
        if self.bits.read(1)? == 0 {
            Ok(self.bits.read(7)? + 1)
        } else {
            self.bits.padding(3)?;
            Ok(self.bits.read(20)? + 129)
        }
    }

    /// C.22
    fn octets_from_bit2(&mut self) -> Result<&'a [u8], Error> {
        let length = if self.bits.read(1)? == 0 {
            u64::from(self.bits.read(6)?) + 1
        } else if self.bits.read(1)? == 0 {
            self.bits.padding(5)?;
            u64::from(self.bits.read(8)?) + 65
        } else {
            self.bits.padding(5)?;
            u64::from(self.bits.read(32)?) + 321
        };
        self.bits.octets(length)
    }

    /// C.23
    fn octets_from_bit5(&mut self) -> Result<&'a [u8], Error> {
        let length = if self.bits.read(1)? == 0 {
            u64::from(self.bits.read(3)?) + 1
        } else if self.bits.read(1)? == 0 {
            self.bits.padding(2)?;
            u64::from(self.bits.read(8)?) + 9
        } else {
            self.bits.padding(2)?;
            u64::from(self.bits.read(32)?) + 265
        };
        self.bits.octets(length)
    }

    /// C.24
    fn octets_from_bit7(&mut self) -> Result<&'a [u8], Error> {
        let length = if self.bits.read(1)? == 0 {
            u64::from(self.bits.read(1)?) + 1
        } else if self.bits.read(1)? == 0 {
            u64::from(self.bits.read(8)?) + 3
        } else {
            u64::from(self.bits.read(32)?) + 259
        };
        self.bits.octets(length)
    }

    /// C.25
    fn integer_from_bit2(&mut self) -> Result<u32, Error> {
        if self.bits.read(1)? == 0 {
            Ok(self.bits.read(6)? + 1)
        } else if self.bits.read(1)? == 0 {
            Ok(self.bits.read(13)? + 65)
        } else {
            self.bits.padding(1)?;
            Ok(self.bits.read(20)? + 8257)
        }
    }

    /// C.27
    fn integer_from_bit3(&mut self) -> Result<u32, Error> {
        if self.bits.read(1)? == 0 {
            return Ok(self.bits.read(5)? + 1);
        }
        match self.bits.read(2)? {
            0b00 => Ok(self.bits.read(11)? + 33),
            0b01 => Ok(self.bits.read(19)? + 2081),
            0b10 => {
                self.bits.padding(7)?;
                Ok(self.bits.read(20)? + 526_369)
            }
            other => Err(self.error(ErrorKind::InvalidIdentification(0b100 | other))),
        }
    }

    /// C.28
    fn integer_from_bit4(&mut self) -> Result<u32, Error> {
        if self.bits.read(1)? == 0 {
            return Ok(self.bits.read(4)? + 1);
        }
        match self.bits.read(2)? {
            0b00 => Ok(self.bits.read(10)? + 17),
            0b01 => Ok(self.bits.read(18)? + 1041),
            0b10 => {
                self.bits.padding(6)?;
                Ok(self.bits.read(20)? + 263_185)
            }
            other => Err(self.error(ErrorKind::InvalidIdentification(0b100 | other))),
        }
    }
}

/// Closes the innermost open element into its parent; true when the terminator ends the document.
fn close(open: &mut Vec<Element>, document_children: &mut Vec<Node>) -> bool {
    let Some(element) = open.pop() else {
        return true;
    };
    match open.last_mut() {
        Some(parent) => parent.children.push(Node::Element(element)),
        None => document_children.push(Node::Element(element)),
    }
    false
}

/// X.891 7.17.6: fixed-width character indexes; an all-ones field (and trailing one bits) ends the
/// string.
fn restricted_alphabet(alphabet: &[char], octets: &[u8]) -> Option<String> {
    let width = (usize::BITS - alphabet.len().leading_zeros()) as usize;
    if width == 0 || width > 20 {
        return None;
    }
    let terminator = (1usize << width) - 1;
    let total_bits = octets.len() * 8;
    let mut out = String::new();
    let mut position = 0;
    while position + width <= total_bits {
        let mut field = 0usize;
        for offset in 0..width {
            let index = position + offset;
            let octet = *octets.get(index / 8)?;
            field = (field << 1) | usize::from((octet >> (7 - index % 8)) & 1);
        }
        position += width;
        if field == terminator {
            return trailing_ones(octets, position).then_some(out);
        }
        out.push(*alphabet.get(field)?);
    }
    trailing_ones(octets, position).then_some(out)
}

fn trailing_ones(octets: &[u8], from_bit: usize) -> bool {
    (from_bit..octets.len() * 8).all(|index| {
        octets
            .get(index / 8)
            .is_some_and(|octet| (octet >> (7 - index % 8)) & 1 == 1)
    })
}
