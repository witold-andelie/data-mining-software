//! XML text serialization of a decoded infoset, for inspection and tests.

use super::{Document, Element, Node, QualifiedName};
use std::fmt::Write as _;

pub fn write_element(element: &Element) -> String {
    let mut out = String::new();
    write_one_element(&mut out, element);
    out
}

fn write_one_element(out: &mut String, element: &Element) {
    open_tag(out, element);
    if element.children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    let mut stack: Vec<Frame<'_>> =
        vec![Frame::Close(element), Frame::Nodes(element.children.iter())];
    while let Some(frame) = stack.last_mut() {
        match frame {
            Frame::Close(element) => {
                let _ = write!(out, "</{}>", name(&element.name));
                stack.pop();
            }
            Frame::Nodes(nodes) => match nodes.next() {
                None => {
                    stack.pop();
                }
                Some(Node::Element(element)) => {
                    open_tag(out, element);
                    if element.children.is_empty() {
                        out.push_str("/>");
                    } else {
                        out.push('>');
                        stack.push(Frame::Close(element));
                        stack.push(Frame::Nodes(element.children.iter()));
                    }
                }
                Some(node) => write_leaf(out, node),
            },
        }
    }
}

pub fn write_document(document: &Document) -> String {
    let mut out = String::new();
    // Writing to a String cannot fail.
    let _ = write!(
        out,
        "<?xml version=\"{}\"",
        document.version.as_deref().unwrap_or("1.0")
    );
    if let Some(encoding) = &document.character_encoding_scheme {
        let _ = write!(out, " encoding=\"{}\"", escape(encoding, true));
    }
    if let Some(standalone) = document.standalone {
        let _ = write!(
            out,
            " standalone=\"{}\"",
            if standalone { "yes" } else { "no" }
        );
    }
    out.push_str("?>");
    let mut stack: Vec<Frame<'_>> = vec![Frame::Nodes(document.children.iter())];
    while let Some(frame) = stack.last_mut() {
        match frame {
            Frame::Close(element) => {
                let _ = write!(out, "</{}>", name(&element.name));
                stack.pop();
            }
            Frame::Nodes(nodes) => match nodes.next() {
                None => {
                    stack.pop();
                }
                Some(Node::Element(element)) => {
                    open_tag(&mut out, element);
                    if element.children.is_empty() {
                        out.push_str("/>");
                    } else {
                        out.push('>');
                        stack.push(Frame::Close(element));
                        stack.push(Frame::Nodes(element.children.iter()));
                    }
                }
                Some(node) => write_leaf(&mut out, node),
            },
        }
    }
    out
}

enum Frame<'a> {
    Nodes(std::slice::Iter<'a, Node>),
    Close(&'a Element),
}

fn open_tag(out: &mut String, element: &Element) {
    let _ = write!(out, "<{}", name(&element.name));
    for declaration in &element.namespace_declarations {
        let namespace = declaration.namespace.as_deref().unwrap_or("");
        match &declaration.prefix {
            Some(prefix) => {
                let _ = write!(out, " xmlns:{prefix}=\"{}\"", escape(namespace, true));
            }
            None => {
                let _ = write!(out, " xmlns=\"{}\"", escape(namespace, true));
            }
        }
    }
    for attribute in &element.attributes {
        let _ = write!(
            out,
            " {}=\"{}\"",
            name(&attribute.name),
            escape(&attribute.value, true)
        );
    }
}

fn write_leaf(out: &mut String, node: &Node) {
    match node {
        Node::Element(_) => {}
        Node::Text(text) => out.push_str(&escape(text, false)),
        Node::Comment(text) => {
            let _ = write!(out, "<!--{text}-->");
        }
        Node::ProcessingInstruction { target, content } => {
            let _ = write!(out, "<?{target} {content}?>");
        }
        Node::UnexpandedEntityReference { name, .. } => {
            let _ = write!(out, "&{name};");
        }
        Node::DocumentTypeDeclaration { .. } => {}
    }
}

fn name(name: &QualifiedName) -> String {
    name.to_string()
}

fn escape(text: &str, attribute: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            '\n' if attribute => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' if attribute => out.push_str("&#9;"),
            _ => out.push(c),
        }
    }
    out
}
