//! Clipboard TOML reuses the native serializer and bounded parser, including XML/base64.
use dms_stream::{DocMeta, Edit, Link, NodeRef, StreamDoc, StreamNode, native, new_uid};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const MAX_BYTES: usize = 16 << 20;
pub const MAX_NODES: usize = 10_000;
const FORMAT_KEY: &str = "__dms_clipboard_format";

// The native parser permits deep structured XML. Encoding and decoding also run on a
// bounded worker stack; deeply nested unknown TOML values are rejected separately.
thread_local! {
    static CLIPBOARD_WORKER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
fn bounded<T: Send>(work: impl FnOnce() -> Result<T, String> + Send) -> Result<T, String> {
    if CLIPBOARD_WORKER.with(|flag| flag.get()) {
        return work();
    }
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("dms-clipboard".into())
            .stack_size(64 << 20)
            .spawn_scoped(scope, move || {
                CLIPBOARD_WORKER.with(|flag| flag.set(true));
                work()
            })
            .map_err(|e| format!("Cannot start clipboard worker: {e}"))?
            .join()
            .map_err(|_| "Clipboard worker failed".to_string())?
    })
}
fn metadata_depth(doc: &StreamDoc) -> Result<(), String> {
    let mut pending: Vec<_> = doc.meta.extra.values().map(|v| (v, 0usize)).collect();
    while let Some((value, depth)) = pending.pop() {
        if depth > 64 {
            return Err("Clipboard metadata nesting exceeds 64".into());
        }
        if let Some(values) = value.as_array() {
            pending.extend(values.iter().map(|v| (v, depth.saturating_add(1))));
        }
        if let Some(values) = value.as_table() {
            pending.extend(values.values().map(|v| (v, depth.saturating_add(1))));
        }
    }
    Ok(())
}

/// The canonical native format uses a standalone root `format` assignment. Rename that key
/// and inject the native discriminator, then let the native TOML parser validate everything.
/// A fake assignment in a comment/XML string cannot bypass this: the real `format` would
/// collide with the injected one, or the renamed discriminator would not be a root key.
fn native_text(text: &str) -> Result<String, String> {
    if text.len() > MAX_BYTES || text.contains('\u{fffd}') {
        return Err(
            "Clipboard text exceeds 16 MiB or contains invalid UTF-8 replacement characters".into(),
        );
    }
    let mut converted = String::from("format = 'dms-stream'\n");
    let mut found = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let assignment = trimmed.split_once('=');
        if !found
            && assignment
                .is_some_and(|(key, _)| matches!(key.trim(), "format" | "\"format\"" | "'format'"))
        {
            let (_, value) = assignment.ok_or("Missing clipboard format")?;
            converted.push_str(FORMAT_KEY);
            converted.push_str(" =");
            converted.push_str(value);
            found = true;
        } else {
            converted.push_str(line);
        }
    }
    if !found {
        return Err("Expected format dms-clipboard, version 1".into());
    }
    Ok(converted)
}

pub fn decode(text: &str) -> Result<StreamDoc, String> {
    bounded(|| decode_inner(text))
}
fn decode_inner(text: &str) -> Result<StreamDoc, String> {
    let mut doc = native::from_toml(&native_text(text)?).map_err(|e| e.to_string())?;
    if doc
        .meta
        .extra
        .remove(FORMAT_KEY)
        .and_then(|v| v.as_str().map(str::to_owned))
        .as_deref()
        != Some("dms-clipboard")
    {
        return Err("Expected format dms-clipboard, version 1".into());
    }
    if doc.stream.all_nodes().len() > MAX_NODES {
        return Err("Clipboard exceeds 10000 nodes".into());
    }
    metadata_depth(&doc)?;
    // Native documents may retain unresolved imported references. Clipboard graphs must be
    // self-contained so every reference can be remapped without touching the destination.
    validate_links(&doc)?;
    Ok(doc)
}

fn validate_links(doc: &StreamDoc) -> Result<(), String> {
    let mut pending = vec![&doc.stream.diagram];
    while let Some(diagram) = pending.pop() {
        let uids: HashSet<_> = diagram.nodes.iter().map(|n| n.uid.as_str()).collect();
        for link in &diagram.links {
            for reference in [&link.from, &link.to] {
                if !matches!(reference, NodeRef::Uid(uid) if uids.contains(uid.as_str())) {
                    return Err("Clipboard contains a dangling link".into());
                }
            }
        }
        pending.extend(diagram.nodes.iter().filter_map(|n| n.subdiagram.as_ref()));
    }
    Ok(())
}

pub fn encode(doc: &StreamDoc, selection: &[String]) -> Result<String, String> {
    bounded(|| encode_inner(doc, selection))
}
fn encode_inner(doc: &StreamDoc, selection: &[String]) -> Result<String, String> {
    metadata_depth(doc)?;
    if selection.is_empty() || selection.len() > MAX_NODES {
        return Err("Select between 1 and 10000 nodes to copy".into());
    }
    let selected: HashSet<_> = selection.iter().map(String::as_str).collect();
    let mut part = StreamDoc::new();
    let top: HashSet<_> = doc
        .stream
        .diagram
        .nodes
        .iter()
        .map(|n| n.uid.as_str())
        .collect();
    for uid in selection {
        if !top.contains(uid.as_str()) {
            return Err(format!("Node {uid} is not on the top-level canvas"));
        }
    }
    part.stream.diagram.nodes = doc
        .stream
        .diagram
        .nodes
        .iter()
        .filter(|n| selected.contains(n.uid.as_str()))
        .cloned()
        .collect();
    if part.stream.all_nodes().len() > MAX_NODES {
        return Err("Clipboard exceeds 10000 nodes".into());
    }
    part.stream.diagram.links = doc
        .stream
        .diagram
        .links
        .iter()
        .filter(|link| {
            matches!((&link.from, &link.to), (NodeRef::Uid(a), NodeRef::Uid(b))
            if selected.contains(a.as_str()) && selected.contains(b.as_str()))
        })
        .cloned()
        .collect();
    part.stream.diagram.terminal_order = doc
        .stream
        .diagram
        .terminal_order
        .iter()
        .filter(|r| matches!(r, NodeRef::Uid(uid) if selected.contains(uid.as_str())))
        .cloned()
        .collect();
    let all = part.stream.all_nodes();
    let mut required = HashSet::new();
    for node in &all {
        required.extend(node.properties_attachment.iter().map(String::as_str));
        required.extend(node.containers.iter().map(|c| c.member.as_str()));
    }
    let mut bytes = 0usize;
    for name in required {
        let data = doc
            .attachments
            .get(name)
            .ok_or_else(|| format!("Missing attachment {name}"))?;
        bytes = bytes.saturating_add(data.len());
        if bytes > MAX_BYTES {
            return Err("Clipboard attachments exceed 16 MiB".into());
        }
        part.attachments.insert(name.into(), data.clone());
    }
    if let Some(extras) = doc.meta.extra.get("node") {
        let mut extras = extras.clone();
        let copied: HashSet<_> = all.iter().map(|n| n.uid.as_str()).collect();
        if let Some(table) = extras.as_table_mut() {
            table.retain(|uid, _| copied.contains(uid));
        }
        part.meta.extra.insert("node".into(), extras);
    }
    let text = native::to_toml(&part).map_err(|e| e.to_string())?.replacen(
        "format = \"dms-stream\"",
        "format = \"dms-clipboard\"",
        1,
    );
    if text.len() > MAX_BYTES {
        return Err("Clipboard text exceeds 16 MiB".into());
    }
    Ok(text)
}

pub(crate) struct Paste {
    pub edits: Vec<Edit>,
    pub selection: Vec<String>,
    pub meta: DocMeta,
}

pub(crate) fn paste(
    doc: &StreamDoc,
    text: &str,
    position: Option<(i64, i64)>,
    rng: &mut dyn FnMut() -> u64,
) -> Result<Paste, String> {
    let mut part = decode(text)?;
    let mut used: HashSet<String> = doc
        .stream
        .all_nodes()
        .iter()
        .map(|n| n.uid.clone())
        .collect();
    let mut mapping = HashMap::new();
    for node in part.stream.all_nodes() {
        let uid = new_uid(&|s| used.contains(s), rng);
        used.insert(uid.clone());
        mapping.insert(node.uid.clone(), uid);
    }
    let mut attachments = BTreeMap::new();
    let mut edits = Vec::new();
    for (name, bytes) in &part.attachments {
        let target = if doc.attachments.get(name).is_none_or(|old| old == bytes) {
            name.clone()
        } else {
            // Names of all incoming members are reserved too, so a generated name cannot
            // collide with a later member in this clipboard.
            let suffix = new_uid(
                &|s| {
                    let candidate = format!("{name}-{s}");
                    doc.attachments.contains_key(&candidate)
                        || part.attachments.contains_key(&candidate)
                        || attachments.values().any(|v| v == &candidate)
                },
                rng,
            );
            format!("{name}-{suffix}")
        };
        if !doc.attachments.contains_key(&target) {
            edits.push(Edit::PutAttachment {
                name: target.clone(),
                bytes: Some(bytes.clone()),
            });
        }
        attachments.insert(name.clone(), target);
    }
    let min_x = part
        .stream
        .diagram
        .nodes
        .iter()
        .filter_map(|n| n.position.map(|p| p.0))
        .min()
        .unwrap_or(0);
    let min_y = part
        .stream
        .diagram
        .nodes
        .iter()
        .filter_map(|n| n.position.map(|p| p.1))
        .min()
        .unwrap_or(0);
    let delta = match position {
        Some((x, y)) => (
            x.checked_sub(min_x).ok_or("Paste x overflow")?,
            y.checked_sub(min_y).ok_or("Paste y overflow")?,
        ),
        None => (30, 30),
    };
    for node in &mut part.stream.diagram.nodes {
        let (x, y) = node.position.unwrap_or((min_x, min_y));
        let x = x.checked_add(delta.0).ok_or("Paste x overflow")?;
        let y = y.checked_add(delta.1).ok_or("Paste y overflow")?;
        if !(-1_000_000..=1_000_000).contains(&x) || !(-1_000_000..=1_000_000).contains(&y) {
            return Err("Paste position exceeds +/-1000000".into());
        }
        node.position = Some((x, y));
    }
    let mut pending: Vec<&mut StreamNode> = part.stream.diagram.nodes.iter_mut().collect();
    while let Some(node) = pending.pop() {
        node.uid = mapping.get(&node.uid).ok_or("Missing uid mapping")?.clone();
        if let Some(name) = &mut node.properties_attachment {
            *name = attachments
                .get(name)
                .ok_or("Missing attachment mapping")?
                .clone();
        }
        for container in &mut node.containers {
            container.member = attachments
                .get(&container.member)
                .ok_or("Missing attachment mapping")?
                .clone();
        }
        if let Some(diagram) = &mut node.subdiagram {
            remap_links(&mut diagram.links, &mapping)?;
            remap_order(&mut diagram.terminal_order, &mapping)?;
            pending.extend(diagram.nodes.iter_mut());
        }
    }
    remap_links(&mut part.stream.diagram.links, &mapping)?;
    remap_order(&mut part.stream.diagram.terminal_order, &mapping)?;
    let mut meta = doc.meta.clone();
    if let Some(mut extras) = part.meta.extra.remove("node") {
        if let Some(table) = extras.as_table_mut() {
            let old = std::mem::take(table);
            for (uid, value) in old {
                table.insert(
                    mapping
                        .get(&uid)
                        .ok_or("Missing metadata uid mapping")?
                        .clone(),
                    value,
                );
            }
        }
        let target = meta
            .extra
            .entry("node".into())
            .or_insert_with(|| extras.clone());
        if let (Some(target), Some(source)) = (target.as_table_mut(), extras.as_table()) {
            target.extend(source.clone());
        }
    }
    let selection = part
        .stream
        .diagram
        .nodes
        .iter()
        .map(|n| n.uid.clone())
        .collect();
    let mut index = doc.stream.diagram.nodes.len();
    for node in part.stream.diagram.nodes {
        edits.push(Edit::AddNode { node, index });
        index = index.saturating_add(1);
    }
    let mut index = doc.stream.diagram.links.len();
    for link in part.stream.diagram.links {
        edits.push(Edit::Connect { link, index });
        index = index.saturating_add(1);
    }
    if !part.stream.diagram.terminal_order.is_empty() {
        let mut order = doc.stream.diagram.terminal_order.clone();
        order.extend(part.stream.diagram.terminal_order);
        edits.push(Edit::SetTerminalOrder { order });
    }
    Ok(Paste {
        edits,
        selection,
        meta,
    })
}

fn remap(reference: &mut NodeRef, mapping: &HashMap<String, String>) -> Result<(), String> {
    match reference {
        NodeRef::Uid(uid) => {
            *uid = mapping
                .get(uid)
                .ok_or("Clipboard contains a dangling node reference")?
                .clone()
        }
        NodeRef::Unresolved(_) => {
            return Err("Clipboard contains an unresolved node reference".into());
        }
    }
    Ok(())
}
fn remap_links(links: &mut [Link], mapping: &HashMap<String, String>) -> Result<(), String> {
    for link in links {
        remap(&mut link.from, mapping)?;
        remap(&mut link.to, mapping)?;
    }
    Ok(())
}
fn remap_order(order: &mut [NodeRef], mapping: &HashMap<String, String>) -> Result<(), String> {
    for reference in order {
        remap(reference, mapping)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        edit_session::tests::{session, text},
        session::Arg,
    };
    use dms_stream::{Container, Diagram};
    use std::sync::Arc;

    #[test]
    fn copies_only_internal_links_and_referenced_attachments_including_supernodes() {
        let mut session = session();
        let state = session.document_state.as_mut().unwrap();
        let mut nested = crate::run::tests::node("inner", "SelectNode");
        nested.properties_attachment = Some("settings".into());
        state.doc.stream.diagram.nodes[0].subdiagram = Some(Diagram {
            nodes: vec![nested],
            ..Default::default()
        });
        state.doc.stream.diagram.nodes[0]
            .containers
            .push(Container {
                name: None,
                member: "binary".into(),
            });
        state
            .doc
            .attachments
            .insert("settings".into(), Arc::from(b"hello".as_slice()));
        state
            .doc
            .attachments
            .insert("binary".into(), Arc::from([0u8, 255, 128]));
        state
            .doc
            .attachments
            .insert("unreferenced".into(), Arc::from(b"omit".as_slice()));
        let part = decode(&encode(&state.doc, &["a".into()]).unwrap()).unwrap();
        assert_eq!(part.stream.all_nodes().len(), 2);
        assert!(part.stream.diagram.links.is_empty());
        assert_eq!(part.attachments.len(), 2);
        assert!(!part.attachments.contains_key("unreferenced"));
        let before = text(&session);
        let encoded = encode(&session.document_state().unwrap().doc, &["a".into()]).unwrap();
        session
            .execute("edit.paste", &[Arg::Text(encoded)])
            .unwrap();
        let state = session.document_state().unwrap();
        let added = state.doc.node(&state.selection[0]).unwrap();
        assert_ne!(added.subdiagram.as_ref().unwrap().nodes[0].uid, "inner");
        assert_eq!(state.doc.attachments.len(), 3); // Same bytes share existing members.
        session.execute("edit.undo", &[]).unwrap();
        assert_eq!(text(&session), before);
    }

    #[test]
    fn attachment_collisions_and_unknown_node_fields_survive_undo_and_redo() {
        let source = "format='dms-clipboard'\nversion=1\n[[node]]\nuid='source'\nclass='SelectNode'\nposition=[0,0]\nfuture={ nested=[1,2] }\ncontainers=[{member='model'}]\n[[attachment]]\nname='model'\nbase64='AP+A'";
        let mut session = session();
        session
            .document_state
            .as_mut()
            .unwrap()
            .doc
            .attachments
            .insert("model".into(), Arc::from(b"existing".as_slice()));
        let before = text(&session);
        session
            .execute("edit.paste", &[Arg::Text(source.into())])
            .unwrap();
        let after = text(&session);
        let state = session.document_state().unwrap();
        let added = state.doc.node(&state.selection[0]).unwrap();
        let member = &added.containers[0].member;
        assert_ne!(member, "model");
        assert_eq!(state.doc.attachments[member].as_ref(), &[0, 255, 128]);
        assert_eq!(state.doc.attachments["model"].as_ref(), b"existing");
        assert!(after.contains("future = { nested = [1, 2] }"));
        session.execute("edit.undo", &[]).unwrap();
        assert_eq!(text(&session), before);
        session.execute("edit.redo", &[]).unwrap();
        assert_eq!(text(&session), after);
    }

    #[test]
    fn injectable_uid_generator_remaps_nested_links_and_terminal_orders() {
        let mut source = StreamDoc::new();
        let mut parent = crate::run::tests::node("outer", "SuperNode");
        parent.subdiagram = Some(Diagram {
            nodes: vec![
                crate::run::tests::node("inner1", "SelectNode"),
                crate::run::tests::node("inner2", "TableNode"),
            ],
            links: vec![Link {
                from: NodeRef::Uid("inner1".into()),
                to: NodeRef::Uid("inner2".into()),
            }],
            terminal_order: vec![NodeRef::Uid("inner2".into())],
        });
        source.stream.diagram.nodes.push(parent);
        let text = encode(&source, &["outer".into()]).unwrap();
        let mut target = StreamDoc::new();
        target
            .stream
            .diagram
            .nodes
            .push(crate::run::tests::node("idAAAAAAAAAAA", "SelectNode"));
        let paste = paste(&target, &text, None, &mut || 0).unwrap();
        target.apply_all(paste.edits).unwrap();
        let copied = target.node(&paste.selection[0]).unwrap();
        assert_eq!(copied.uid, "idAAAAAAAAAAB");
        let inner = copied.subdiagram.as_ref().unwrap();
        assert_eq!(
            inner.links[0].from,
            NodeRef::Uid(inner.nodes[0].uid.clone())
        );
        assert_eq!(inner.links[0].to, NodeRef::Uid(inner.nodes[1].uid.clone()));
        assert_eq!(
            inner.terminal_order,
            [NodeRef::Uid(inner.nodes[1].uid.clone())]
        );
    }

    #[test]
    fn clipboard_count_limit_and_discriminator_spoofs_are_errors() {
        let mut oversized = String::from("format='dms-clipboard'\nversion=1\n");
        for i in 0..10_001 {
            oversized.push_str(&format!("[[node]]\nuid='n{i}'\nclass='SelectNode'\n"));
        }
        assert!(decode(&oversized).is_err());
        let deeply_nested = format!(
            "format='dms-clipboard'\nversion=1\nx={}0{}",
            "[".repeat(512),
            "]".repeat(512)
        );
        assert!(decode(&deeply_nested).is_err());
        for bad in [
            "format='dms-stream'\nversion=1\nx='dms-clipboard'",
            "# format='dms-clipboard'\nformat='dms-stream'\nversion=1",
            "format='dms-stream'\nversion=1\nx='''\nformat='dms-clipboard'\n'''",
            "format='dms-clipboard'\nversion=1\n[[link]]\nfrom='missing'\nto='other'",
        ] {
            assert!(decode(bad).is_err(), "{bad}");
        }
        assert!(decode("# comment\n'format' = 'dms-clipboard' # discriminator\nversion=1").is_ok());
    }
}
