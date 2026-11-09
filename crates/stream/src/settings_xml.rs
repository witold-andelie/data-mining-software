//! Lossless XML text when possible; only adjacent text chunks may be normalized.
use dms_strfmt::fastinfoset::{Attribute, Element, NamespaceDeclaration, Node, QualifiedName};
use std::sync::Arc;

pub fn element_to_xml(element: &Element) -> String {
    element.to_xml()
}

pub fn element_from_xml(text: &str) -> Result<Element, String> {
    let depth = check_xml_depth(text)?;
    if depth > 64 {
        crate::native::bounded_stack(|| parse_xml(text).map_err(crate::DocError::Format))
            .map_err(|e| e.to_string())
    } else {
        parse_xml(text)
    }
}
fn parse_xml(text: &str) -> Result<Element, String> {
    if text.len() > crate::doc::MAX_BYTES {
        return Err("XML exceeds 1 GiB".into());
    }
    let doc = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 2_000_000,
        },
    )
    .map_err(|e| e.to_string())?;
    // Check before constructing any recursive tree.
    for node in doc.descendants().filter(|n| n.is_element()) {
        if node.ancestors().filter(|n| n.is_element()).count() > 512 {
            return Err("XML nesting exceeds 512".into());
        }
    }
    fn shell(node: roxmltree::Node<'_, '_>, text: &str) -> Element {
        let qname = |range: std::ops::Range<usize>, local: &str, namespace: Option<&str>| {
            let raw = text.get(range).unwrap_or_default();
            let attr_name = raw
                .split(['=', ' ', '\t', '\r', '\n'])
                .next()
                .unwrap_or_default();
            let prefix = attr_name.split_once(':').map(|(p, _)| Arc::from(p));
            QualifiedName {
                prefix,
                namespace: namespace.map(Arc::from),
                local: Arc::from(local),
            }
        };
        let start = text
            .get(node.range())
            .unwrap_or_default()
            .strip_prefix('<')
            .unwrap_or_default();
        let tag = start
            .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .next()
            .unwrap_or_default();
        let name = QualifiedName {
            prefix: tag.split_once(':').map(|(p, _)| Arc::from(p)),
            namespace: node.tag_name().namespace().map(Arc::from),
            local: Arc::from(node.tag_name().name()),
        };
        // roxmltree exposes in-scope namespaces. Keep only declarations physically present
        // on this element, in lexical order, including redundant declarations.
        let mut declarations = Vec::new();
        let mut cursor = start.strip_prefix(tag).unwrap_or_default();
        loop {
            cursor = cursor.trim_start();
            if cursor.starts_with(['/', '>']) || cursor.is_empty() {
                break;
            }
            let Some(eq) = cursor.find('=') else {
                break;
            };
            let attr = cursor.get(..eq).unwrap_or_default().trim();
            cursor = cursor
                .get(eq.saturating_add(1)..)
                .unwrap_or_default()
                .trim_start();
            let Some(quote @ ('\'' | '"')) = cursor.chars().next() else {
                break;
            };
            cursor = cursor.get(1..).unwrap_or_default();
            let Some(end) = cursor.find(quote) else {
                break;
            };
            if attr == "xmlns" || attr.starts_with("xmlns:") {
                let prefix = attr.strip_prefix("xmlns:");
                let ns = node
                    .namespaces()
                    .find(|n| n.name() == prefix)
                    .map(|n| n.uri())
                    .filter(|n| !n.is_empty());
                declarations.push(NamespaceDeclaration {
                    prefix: prefix.map(Arc::from),
                    namespace: ns.map(Arc::from),
                });
            }
            cursor = cursor.get(end.saturating_add(1)..).unwrap_or_default();
        }
        Element {
            name,
            namespace_declarations: declarations,
            attributes: node
                .attributes()
                .map(|a| Attribute {
                    name: qname(a.range(), a.name(), a.namespace()),
                    value: Arc::from(a.value()),
                })
                .collect(),
            children: Vec::new(),
        }
    }
    let root = doc.root_element();
    let mut stack = vec![(root.children(), shell(root, text))];
    loop {
        match stack.last_mut().and_then(|(children, _)| children.next()) {
            Some(child) if child.is_element() => stack.push((child.children(), shell(child, text))),
            Some(child) => {
                let leaf = match child.node_type() {
                    roxmltree::NodeType::Text => {
                        Some(Node::Text(Arc::from(child.text().unwrap_or_default())))
                    }
                    roxmltree::NodeType::Comment => {
                        Some(Node::Comment(Arc::from(child.text().unwrap_or_default())))
                    }
                    roxmltree::NodeType::PI => child.pi().map(|pi| Node::ProcessingInstruction {
                        target: Arc::from(pi.target),
                        content: Arc::from(pi.value.unwrap_or_default()),
                    }),
                    _ => None,
                };
                if let (Some(leaf), Some((_, parent))) = (leaf, stack.last_mut()) {
                    parent.children.push(leaf);
                }
            }
            None => {
                if let Some((_, complete)) = stack.pop() {
                    if let Some((_, parent)) = stack.last_mut() {
                        parent.children.push(Node::Element(complete));
                    } else {
                        return Ok(normalize(&complete));
                    }
                }
            }
        }
    }
}

pub fn xml_roundtrips(element: &Element) -> bool {
    let Ok(parsed) = element_from_xml(&element_to_xml(element)) else {
        return false;
    };
    let normalized = normalize(element);
    let mut pending = vec![(&parsed, &normalized)];
    while let Some((a, b)) = pending.pop() {
        if a.name != b.name
            || a.attributes != b.attributes
            || a.namespace_declarations != b.namespace_declarations
            || a.children.len() != b.children.len()
        {
            return false;
        }
        for (a, b) in a.children.iter().zip(&b.children) {
            match (a, b) {
                (Node::Element(a), Node::Element(b)) => pending.push((a, b)),
                _ if a == b => {}
                _ => return false,
            }
        }
    }
    true
}

/// Iterative, including for caller-created trees deeper than the parser limit.
pub fn normalize(element: &Element) -> Element {
    fn shell(e: &Element) -> Element {
        Element {
            name: e.name.clone(),
            namespace_declarations: e.namespace_declarations.clone(),
            attributes: e.attributes.clone(),
            children: Vec::new(),
        }
    }
    fn flush(parent: &mut Element, text: &mut Option<String>) {
        if let Some(text) = text.take() {
            parent.children.push(Node::Text(text.into()));
        }
    }
    let mut stack = vec![(element.children.iter(), shell(element), None::<String>)];
    loop {
        let next = stack
            .last_mut()
            .and_then(|(children, _, _)| children.next());
        match next {
            Some(Node::Element(e)) => {
                if let Some((_, parent, text)) = stack.last_mut() {
                    flush(parent, text);
                }
                stack.push((e.children.iter(), shell(e), None));
            }
            Some(Node::Text(text)) => {
                if let Some((_, _, pending)) = stack.last_mut() {
                    pending.get_or_insert_with(String::new).push_str(text);
                }
            }
            Some(node) => {
                if let Some((_, parent, text)) = stack.last_mut() {
                    flush(parent, text);
                    parent.children.push(node.clone());
                }
            }
            None => {
                if let Some((_, mut complete, mut text)) = stack.pop() {
                    flush(&mut complete, &mut text);
                    if let Some((_, parent, _)) = stack.last_mut() {
                        parent.children.push(Node::Element(complete));
                    } else {
                        return complete;
                    }
                }
            }
        }
    }
}

// Bound before roxmltree's recursive tokenizer and capacity estimates. Quotes, comments,
// processing instructions and CDATA must not count as markup nesting.
fn check_xml_depth(text: &str) -> Result<usize, String> {
    if text.len() > crate::doc::MAX_BYTES {
        return Err("XML exceeds 1 GiB".into());
    }
    if text
        .bytes()
        .filter(|&b| b == b'<' || b == b'=')
        .take(2_000_001)
        .count()
        > 2_000_000
    {
        return Err("XML item count exceeds limit".into());
    }
    let mut at = 0usize;
    let mut depth = 0usize;
    let mut maximum = 0usize;
    while let Some(tail) = text.get(at..) {
        let Some(offset) = tail.find('<') else {
            break;
        };
        at = at.saturating_add(offset);
        let tail = text.get(at..).ok_or("invalid XML offset")?;
        let terminator = if tail.starts_with("<!--") {
            Some("-->")
        } else if tail.starts_with("<![CDATA[") {
            Some("]]>")
        } else if tail.starts_with("<?") {
            Some("?>")
        } else {
            None
        };
        if let Some(end) = terminator {
            at = at
                .saturating_add(tail.find(end).ok_or("unterminated XML token")?)
                .saturating_add(end.len());
            continue;
        }
        if tail.starts_with("<!") {
            return Err("XML declarations are unsupported inside settings".into());
        }
        let closing = tail.starts_with("</");
        let mut quote = None;
        let mut last = b'<';
        let mut found = false;
        for (offset, b) in tail.bytes().enumerate().skip(1) {
            if let Some(q) = quote {
                if b == q {
                    quote = None;
                }
            } else if b == b'\'' || b == b'"' {
                quote = Some(b);
            } else if b == b'>' {
                if closing {
                    depth = depth.saturating_sub(1);
                } else {
                    maximum = maximum.max(depth.saturating_add(1));
                    if maximum > 512 {
                        return Err("XML nesting exceeds 512".into());
                    }
                    if last != b'/' {
                        depth = depth.saturating_add(1);
                    }
                }
                at = at.saturating_add(offset).saturating_add(1);
                found = true;
                break;
            }
            if !b.is_ascii_whitespace() {
                last = b;
            }
        }
        if !found {
            return Err("unterminated XML tag".into());
        }
    }
    Ok(maximum)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xml_infoset_and_only_text_normalization() {
        for xml in [
            "<p a=\"&amp;&#13;&#10;&#9;\">a<![CDATA[b]]>c<!--x--><?pi y?><q/></p>",
            "<a:r xmlns:a=\"urn:a\" xmlns=\"urn:b\"><c xmlns:a=\"urn:a\" a:x=\"v\"/><d xmlns=\"\"/></a:r>",
        ] {
            let e = element_from_xml(xml).unwrap();
            assert!(xml_roundtrips(&e), "{xml}");
        }
        assert!(element_from_xml("<!DOCTYPE p [<!ENTITY x 'y'>]><p>&x;</p>").is_err());
        assert!(element_from_xml(&format!("{}{}", "<p>".repeat(513), "</p>".repeat(513))).is_err());
        let e = element_from_xml(&format!("{}{}", "<p>".repeat(512), "</p>".repeat(512))).unwrap();
        assert!(xml_roundtrips(&e));
    }
}
