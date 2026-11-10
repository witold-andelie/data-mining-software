//! Lossless XML attachment edits: splice only attribute values / list Value entries.
use super::*;
use roxmltree::{Document, Node, ParsingOptions};
use std::ops::Range;

const MAX_ATTACHMENT: usize = 256 << 20;

fn parse(bytes: &[u8]) -> Result<Document<'_>, String> {
    if bytes.len() > MAX_ATTACHMENT {
        return Err("Attachment exceeds 256 MiB".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "Attachment is not UTF-8 XML")?;
    // Bound nesting before invoking the XML tokenizer. Quotes, comments and CDATA are tokens.
    let mut at = 0usize;
    let mut depth = 0usize;
    let mut attributes = 0usize;
    while let Some(tail) = text.get(at..) {
        let Some(offset) = tail.find('<') else {
            break;
        };
        at = at.saturating_add(offset);
        let tail = text.get(at..).ok_or("Invalid XML offset")?;
        let end_token = if tail.starts_with("<!--") {
            Some("-->")
        } else if tail.starts_with("<![CDATA[") {
            Some("]]>")
        } else if tail.starts_with("<?") {
            Some("?>")
        } else {
            None
        };
        if let Some(token) = end_token {
            at = at
                .saturating_add(tail.find(token).ok_or("Unterminated XML token")?)
                .saturating_add(token.len());
            continue;
        }
        if tail.starts_with("<!") {
            return Err("XML declarations and entities are unsupported".into());
        }
        let mut quote = None;
        let mut last = b'<';
        let mut found = false;
        for (offset, byte) in tail.bytes().enumerate().skip(1) {
            if let Some(q) = quote {
                if byte == q {
                    quote = None;
                }
            } else if matches!(byte, b'\'' | b'"') {
                quote = Some(byte);
            } else if byte == b'=' {
                attributes = attributes.saturating_add(1);
                if attributes > MAX_ITEMS {
                    return Err("Attachment exceeds 100000 attributes".into());
                }
            } else if byte == b'>' {
                if tail.starts_with("</") {
                    depth = depth.saturating_sub(1);
                } else {
                    if depth >= MAX_DEPTH {
                        return Err("Attachment nesting exceeds 512".into());
                    }
                    if last != b'/' {
                        depth = depth.saturating_add(1);
                    }
                }
                at = at.saturating_add(offset).saturating_add(1);
                found = true;
                break;
            }
            if !byte.is_ascii_whitespace() {
                last = byte;
            }
        }
        if !found {
            return Err("Unterminated XML tag".into());
        }
    }
    let doc = Document::parse_with_options(
        text,
        ParsingOptions {
            nodes_limit: MAX_ITEMS as u32,
            ..Default::default()
        },
    )
    .map_err(|e| format!("Invalid attachment XML: {e}"))?;
    if doc
        .descendants()
        .map(|n| n.attributes().len())
        .fold(0usize, usize::saturating_add)
        > MAX_ITEMS
    {
        return Err("Attachment exceeds 100000 attributes".into());
    }
    Ok(doc)
}

fn parameter<'a, 'input>(
    doc: &'a Document<'input>,
    spec: &ParamSpec,
) -> Result<Option<Node<'a, 'input>>, String> {
    if spec.source != ParamSource::Attachment {
        return Err("This parameter comes from properties".into());
    }
    let mut matches = doc.descendants().filter(|n| {
        n.is_element()
            && n.tag_name().name() == "Parameter"
            && n.attribute("name") == Some(spec.path)
    });
    let result = matches.next();
    if matches.next().is_some() {
        return Err(format!("Duplicate attachment parameter {}", spec.path));
    }
    Ok(result)
}

fn read_node(node: Option<Node<'_, '_>>, spec: &ParamSpec) -> Result<ParamValue, String> {
    let Some(node) = node else {
        return spec
            .default
            .map_or(Ok(ParamValue::Unset), |s| scalar(&spec.kind, s));
    };
    if matches!(spec.kind, ParamKind::Fields { .. }) {
        if node.attribute("value").is_some() {
            return Err(format!("{} must be an XML list", spec.key));
        }
        let mut fields = Vec::new();
        let mut size = 0usize;
        for value in node
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "Value")
        {
            if fields.len() >= MAX_ROWS {
                return Err("List exceeds 10000 entries".into());
            }
            let text = value
                .attribute("value")
                .ok_or("List Value lacks its value attribute")?;
            check_text(text)?;
            size = size.saturating_add(text.len());
            if size > MAX_TEXT {
                return Err("List text exceeds 1 MiB".into());
            }
            fields.push(text.into());
        }
        return Ok(ParamValue::Fields(fields));
    }
    match node.attribute("value") {
        Some(text) => scalar(&spec.kind, text),
        // Modeler saves inactive optional numeric values as an empty ListValue.
        None if !node
            .descendants()
            .any(|n| n.is_element() && n.tag_name().name() == "Value") =>
        {
            Ok(ParamValue::Unset)
        }
        None => Err(format!("{} is a list, expected a scalar", spec.key)),
    }
}

pub fn read_attachment(attachment: Option<&[u8]>, spec: &ParamSpec) -> Result<ParamValue, String> {
    let doc = parse(attachment.ok_or("The node has no parameters attachment")?)?;
    let node = parameter(&doc, spec)?;
    if node.is_none()
        && !doc.descendants().any(|n| {
            n.is_element() && n.tag_name().name() == "Parameter" && n.parent_element().is_some()
        })
    {
        return Err("Attachment has no Parameter parent to append to".into());
    }
    read_node(node, spec)
}

fn escape(text: &str) -> String {
    let mut result = String::new();
    for c in text.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&apos;"),
            '\r' => result.push_str("&#13;"),
            '\n' => result.push_str("&#10;"),
            '\t' => result.push_str("&#9;"),
            _ => result.push(c),
        }
    }
    result
}

fn qualified(node: Node<'_, '_>, local: &str) -> String {
    node.tag_name()
        .namespace()
        .and_then(|ns| node.lookup_prefix(ns))
        .map_or_else(|| local.into(), |prefix| format!("{prefix}:{local}"))
}

fn attr_edit(
    node: Node<'_, '_>,
    text: &str,
    replacement: &str,
) -> Result<(Range<usize>, String), String> {
    if let Some(attr) = node
        .attributes()
        .find(|a| a.name() == "value" && a.namespace().is_none())
    {
        // Do not use range_value(): its compressed offsets truncate very long '=' whitespace.
        let range = attr.range();
        let raw = text
            .get(range.clone())
            .ok_or("Invalid XML attribute span")?;
        let eq = raw.find('=').ok_or("Missing attribute equals")?;
        let after = raw
            .get(eq.saturating_add(1)..)
            .ok_or("Invalid attribute offset")?;
        let quoted = after.trim_start();
        let prefix = raw.len().saturating_sub(quoted.len()).saturating_add(1);
        let start = range
            .start
            .checked_add(prefix)
            .ok_or("XML offset overflow")?;
        let end = range.end.checked_sub(1).ok_or("XML offset underflow")?;
        if start > end {
            return Err("Invalid XML attribute value span".into());
        }
        return Ok((start..end, escape(replacement)));
    }
    let raw = text.get(node.range()).ok_or("Invalid XML node span")?;
    let end = opening_end(raw)?;
    let before = raw.get(..end).ok_or("Invalid XML opening span")?.trim_end();
    let position = node.range().start.saturating_add(if before.ends_with('/') {
        before.len().saturating_sub(1)
    } else {
        end
    });
    Ok((
        position..position,
        format!(" value=\"{}\"", escape(replacement)),
    ))
}

fn opening_end(raw: &str) -> Result<usize, String> {
    let mut quote = None;
    for (i, b) in raw.bytes().enumerate() {
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else if matches!(b, b'\'' | b'"') {
            quote = Some(b);
        } else if b == b'>' {
            return Ok(i);
        }
    }
    Err("Unterminated XML opening tag".into())
}

fn opening_name(raw: &str) -> Result<&str, String> {
    let tail = raw.get(1..).ok_or("Missing XML opening name")?;
    let end = tail
        .find(|c: char| c.is_whitespace() || matches!(c, '/' | '>'))
        .ok_or("Missing XML name terminator")?;
    tail.get(..end)
        .ok_or_else(|| "Invalid XML name span".into())
}

fn append_edit(
    parent: Node<'_, '_>,
    text: &str,
    addition: String,
) -> Result<(Range<usize>, String), String> {
    let range = parent.range();
    let raw = text.get(range.clone()).ok_or("Invalid XML parent span")?;
    let opening = opening_end(raw)?;
    let before = raw
        .get(..opening)
        .ok_or("Invalid XML opening span")?
        .trim_end();
    if before.ends_with('/') {
        let start = range.start.saturating_add(before.len().saturating_sub(1));
        let end = range.start.saturating_add(opening).saturating_add(1);
        Ok((start..end, format!(">{addition}</{}>", opening_name(raw)?)))
    } else {
        let close = raw.rfind("</").ok_or("Missing XML closing tag")?;
        let position = range.start.saturating_add(close);
        Ok((position..position, addition))
    }
}

pub fn write_attachment(
    attachment: &[u8],
    spec: &ParamSpec,
    value: &ParamValue,
) -> Result<Vec<u8>, String> {
    let doc = parse(attachment)?;
    let node = parameter(&doc, spec)?;
    if same_value(&read_node(node, spec)?, value) {
        return Ok(attachment.to_vec());
    }
    validate(spec, value)?;
    let text = std::str::from_utf8(attachment).map_err(|_| "Attachment is not UTF-8")?;
    let mut edits = Vec::new();
    if matches!(value, ParamValue::Unset) {
        if let Some(node) = node {
            edits.push((node.range(), String::new()));
        }
    } else if let Some(node) = node {
        if let ParamValue::Fields(fields) = value {
            let values: Vec<_> = node
                .descendants()
                .filter(|n| n.is_element() && n.tag_name().name() == "Value")
                .collect();
            for (i, old) in values.iter().enumerate() {
                match fields.get(i) {
                    Some(v) if old.attribute("value") != Some(v.as_str()) => {
                        edits.push(attr_edit(*old, text, v)?)
                    }
                    None => edits.push((old.range(), String::new())),
                    _ => {}
                }
            }
            if fields.len() > values.len() {
                let list = node
                    .children()
                    .find(|n| n.is_element() && n.tag_name().name() == "ListValue");
                let mut addition = String::new();
                for v in fields.iter().skip(values.len()) {
                    addition.push_str(&format!(
                        "<{} value=\"{}\"/>",
                        qualified(node, "Value"),
                        escape(v)
                    ));
                }
                let (parent, addition) = match list {
                    Some(list) => (list, addition),
                    None => (
                        node,
                        format!(
                            "<{}>{addition}</{}>",
                            qualified(node, "ListValue"),
                            qualified(node, "ListValue")
                        ),
                    ),
                };
                edits.push(append_edit(parent, text, addition)?);
            }
        } else {
            edits.push(attr_edit(node, text, &file_text(value)?)?);
        }
    } else {
        let sibling = doc
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "Parameter")
            .ok_or("Attachment has no Parameter parent to append to")?;
        let parent = sibling
            .parent_element()
            .ok_or("Parameter has no parent element")?;
        // The first Parameter may declare its prefix locally. Reuse only a namespace
        // binding available at the insertion parent; otherwise declare one on the new node.
        let namespace = sibling.tag_name().namespace();
        let prefix = namespace.and_then(|ns| parent.lookup_prefix(ns));
        let tag = prefix.map_or_else(|| "Parameter".to_string(), |p| format!("{p}:Parameter"));
        let declaration = match namespace {
            Some(ns) if prefix.is_none() && parent.default_namespace() != Some(ns) => {
                format!(" xmlns=\"{}\"", escape(ns))
            }
            None if parent.default_namespace().is_some() => " xmlns=\"\"".into(),
            _ => String::new(),
        };
        let list_tag = prefix.map_or_else(|| "ListValue".to_string(), |p| format!("{p}:ListValue"));
        let value_tag = prefix.map_or_else(|| "Value".to_string(), |p| format!("{p}:Value"));
        let addition = if let ParamValue::Fields(fields) = value {
            let mut inner = String::new();
            for field in fields {
                inner.push_str(&format!("<{} value=\"{}\"/>", value_tag, escape(field)));
            }
            format!(
                "<{tag}{declaration} name=\"{}\"><{list_tag}>{inner}</{list_tag}></{tag}>",
                escape(spec.path)
            )
        } else {
            format!(
                "<{tag}{declaration} name=\"{}\" value=\"{}\"/>",
                escape(spec.path),
                escape(&file_text(value)?)
            )
        };
        edits.push(append_edit(parent, text, addition)?);
    }
    edits.sort_by_key(|(range, _)| range.start);
    let added = edits
        .iter()
        .fold(0usize, |n, (_, s)| n.saturating_add(s.len()));
    if attachment.len().saturating_add(added) > MAX_ATTACHMENT {
        return Err("Edited attachment exceeds 256 MiB".into());
    }
    let mut result = String::new();
    let mut previous = 0usize;
    for (range, replacement) in edits {
        if range.start < previous {
            return Err("Overlapping XML edits".into());
        }
        result.push_str(
            text.get(previous..range.start)
                .ok_or("Invalid XML edit start")?,
        );
        result.push_str(&replacement);
        previous = range.end;
    }
    result.push_str(text.get(previous..).ok_or("Invalid XML edit end")?);
    let result = result.into_bytes();
    // Verify namespace scope, escaped text and structural changes before returning any bytes.
    let checked = parse(&result)?;
    let verified = parameter(&checked, spec)?;
    if if matches!(value, ParamValue::Unset) {
        verified.is_some()
    } else {
        !same_value(&read_node(verified, spec)?, value)
    } {
        return Err("Edited attachment did not retain the requested value".into());
    }
    Ok(result)
}
