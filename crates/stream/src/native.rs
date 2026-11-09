//! Version 1 native TOML format and same-directory atomic persistence.
use crate::doc::{MAX_ATTACHMENT, MAX_ATTACHMENTS, MAX_BYTES, MAX_LABEL, MAX_LINKS, MAX_NODES};
use crate::settings_xml::{element_from_xml, element_to_xml, xml_roundtrips};
use crate::{
    Container, Diagram, DocError, DocMeta, Extension, Link, NodeRef, Stream, StreamDoc, StreamNode,
};
use dms_strfmt::fastinfoset::{Attribute, Element, NamespaceDeclaration, Node, QualifiedName};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use toml::Value;

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Native {
    format: String,
    version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    imported_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    save_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_properties: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_properties_tree: Option<Value>,
    terminal_order: Vec<String>,
    node: Vec<NativeNode>,
    link: Vec<NativeLink>,
    attachment: Vec<NativeAttachment>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct NativeDiagram {
    node: Vec<NativeNode>,
    link: Vec<NativeLink>,
    terminal_order: Vec<String>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct NativeNode {
    uid: String,
    class: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    extension: Option<NativeExtension>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<(i64, i64)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_builder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    properties_attachment: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    containers: Vec<NativeContainer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    settings: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    settings_tree: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    binning_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    binning_output_tree: Option<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    result_items: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subdiagram: Option<NativeDiagram>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct NativeExtension {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    object: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    element: Option<String>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct NativeContainer {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    member: String,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct NativeLink {
    from: String,
    to: String,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct NativeAttachment {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base64: Option<String>,
}

fn format_error(e: impl std::fmt::Display) -> DocError {
    DocError::Format(e.to_string())
}
fn reference(r: &NodeRef) -> String {
    match r {
        NodeRef::Uid(s) => s.clone(),
        NodeRef::Unresolved(s) => format!("?{s}"),
    }
}
fn parse_reference(s: String) -> NodeRef {
    match s.strip_prefix('?') {
        Some(id) => NodeRef::Unresolved(id.into()),
        None => NodeRef::Uid(s),
    }
}
fn encode_links(d: &Diagram) -> Vec<NativeLink> {
    d.links
        .iter()
        .map(|l| NativeLink {
            from: reference(&l.from),
            to: reference(&l.to),
        })
        .collect()
}
fn decode_links(links: Vec<NativeLink>) -> Vec<Link> {
    links
        .into_iter()
        .map(|l| Link {
            from: parse_reference(l.from),
            to: parse_reference(l.to),
        })
        .collect()
}
fn setting(e: Option<&Element>) -> Result<(Option<String>, Option<Value>), DocError> {
    match e {
        None => Ok((None, None)),
        Some(e) if xml_roundtrips(e) => Ok((Some(element_to_xml(e)), None)),
        Some(e) => Ok((None, Some(tree_value(e)?))),
    }
}
fn read_setting(xml: Option<String>, tree: Option<Value>) -> Result<Option<Element>, DocError> {
    match (xml, tree) {
        (None, None) => Ok(None),
        (Some(xml), None) => element_from_xml(&xml).map(Some).map_err(format_error),
        (None, Some(tree)) => read_tree(&tree).map(Some),
        _ => Err(format_error("both XML text and structured tree supplied")),
    }
}
fn encode_nodes(
    d: &Diagram,
    extras: Option<&toml::map::Map<String, Value>>,
) -> Result<Vec<NativeNode>, DocError> {
    d.nodes
        .iter()
        .map(|n| {
            let (settings, settings_tree) = setting(n.settings.as_ref())?;
            let (binning_output, binning_output_tree) = setting(n.binning_output.as_ref())?;
            Ok(NativeNode {
                uid: n.uid.clone(),
                class: n.class.clone(),
                extension: n.extension.as_ref().map(|e| match e {
                    Extension::Object {
                        extension_id,
                        object_id,
                    } => NativeExtension {
                        id: extension_id.clone(),
                        object: Some(object_id.clone()),
                        element: None,
                    },
                    Extension::Element {
                        extension_id,
                        element_id,
                    } => NativeExtension {
                        id: extension_id.clone(),
                        element: Some(element_id.clone()),
                        object: None,
                    },
                }),
                name: n.name.clone(),
                label: n.label.clone(),
                position: n.position,
                model_builder: n.model_builder.clone(),
                properties_attachment: n.properties_attachment.clone(),
                containers: n
                    .containers
                    .iter()
                    .map(|c| NativeContainer {
                        name: c.name.clone(),
                        member: c.member.clone(),
                    })
                    .collect(),
                settings,
                settings_tree,
                binning_output,
                binning_output_tree,
                result_items: n
                    .result_items
                    .iter()
                    .map(|e| {
                        if xml_roundtrips(e) {
                            Ok(Value::String(element_to_xml(e)))
                        } else {
                            tree_value(e)
                        }
                    })
                    .collect::<Result<_, _>>()?,
                subdiagram: n
                    .subdiagram
                    .as_ref()
                    .map(|d| {
                        Ok::<_, DocError>(NativeDiagram {
                            node: encode_nodes(d, extras)?,
                            link: encode_links(d),
                            terminal_order: d.terminal_order.iter().map(reference).collect(),
                        })
                    })
                    .transpose()?,
                extra: extras
                    .and_then(|m| m.get(&n.uid))
                    .and_then(Value::as_table)
                    .map(|t| t.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default(),
            })
        })
        .collect()
}
fn decode_nodes(
    nodes: Vec<NativeNode>,
    extras: &mut toml::map::Map<String, Value>,
    depth: usize,
    counts: &mut (usize, usize),
) -> Result<Vec<StreamNode>, DocError> {
    if depth > 64 {
        return Err(DocError::TooLarge("supernode nesting exceeds 64".into()));
    }
    counts.0 = counts.0.saturating_add(nodes.len());
    if counts.0 > MAX_NODES {
        return Err(DocError::TooLarge("node count".into()));
    }
    nodes
        .into_iter()
        .map(|n| {
            if !n.extra.is_empty() {
                extras.insert(n.uid.clone(), Value::Table(n.extra.into_iter().collect()));
            }
            let subdiagram = n
                .subdiagram
                .map(|d| {
                    counts.1 = counts.1.saturating_add(d.link.len());
                    if counts.1 > MAX_LINKS || d.terminal_order.len() > MAX_NODES {
                        return Err(DocError::TooLarge(
                            "subdiagram links or terminal order".into(),
                        ));
                    }
                    Ok::<_, DocError>(Diagram {
                        nodes: decode_nodes(d.node, extras, depth.saturating_add(1), counts)?,
                        links: decode_links(d.link),
                        terminal_order: d.terminal_order.into_iter().map(parse_reference).collect(),
                    })
                })
                .transpose()?;
            let extension = n
                .extension
                .map(|e| match (e.object, e.element) {
                    (Some(object_id), None) => Ok(Extension::Object {
                        extension_id: e.id,
                        object_id,
                    }),
                    (None, Some(element_id)) => Ok(Extension::Element {
                        extension_id: e.id,
                        element_id,
                    }),
                    _ => Err(format_error(
                        "extension must have exactly one of object or element",
                    )),
                })
                .transpose()?;
            Ok(StreamNode {
                uid: n.uid,
                class: n.class,
                extension,
                name: n.name,
                label: n.label,
                position: n.position,
                model_builder: n.model_builder,
                properties_attachment: n.properties_attachment,
                containers: n
                    .containers
                    .into_iter()
                    .map(|c| Container {
                        name: c.name,
                        member: c.member,
                    })
                    .collect(),
                settings: read_setting(n.settings, n.settings_tree)?,
                binning_output: read_setting(n.binning_output, n.binning_output_tree)?,
                result_items: n
                    .result_items
                    .into_iter()
                    .map(|v| match v {
                        Value::String(xml) => element_from_xml(&xml).map_err(format_error),
                        tree => read_tree(&tree),
                    })
                    .collect::<Result<_, _>>()?,
                subdiagram,
            })
        })
        .collect()
}

pub fn to_toml(doc: &StreamDoc) -> Result<String, DocError> {
    bounded_stack(|| encode_document(doc))
}
// TOML's default 80-level parser limit cannot represent 64 nested supernodes or the
// 512-level structured fallback. A lexical bound precedes its unbounded parser, and
// all recursive library work runs on a fixed-size stack, including destruction.
thread_local! {
    static FORMAT_WORKER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
pub(crate) fn bounded_stack<T: Send>(
    f: impl FnOnce() -> Result<T, DocError> + Send,
) -> Result<T, DocError> {
    if FORMAT_WORKER.with(|flag| flag.get()) {
        return f();
    }
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("dms-native".into())
            .stack_size(64 << 20)
            .spawn_scoped(scope, move || {
                FORMAT_WORKER.with(|flag| flag.set(true));
                f()
            })
            .map_err(|e| DocError::Read(format!("cannot start format worker: {e}")))?
            .join()
            .map_err(|_| format_error("format worker failed"))?
    })
}
fn encode_document(doc: &StreamDoc) -> Result<String, DocError> {
    validate(doc)?;
    let mut extra = doc.meta.extra.clone();
    let node_extras = extra.remove("node");
    let (stream_properties, stream_properties_tree) = setting(doc.stream.properties.as_ref())?;
    let mut attachment = Vec::new();
    for (name, bytes) in &doc.attachments {
        let (text, base64) = match std::str::from_utf8(bytes) {
            Ok(text) if !text.contains('\0') => (Some(text.to_string()), None),
            _ => (None, Some(crate::base64::encode(bytes))),
        };
        attachment.push(NativeAttachment {
            name: name.clone(),
            text,
            base64,
        });
    }
    let native = Native {
        format: "dms-stream".into(),
        version: 1,
        imported_from: doc.meta.imported_from.clone(),
        file_version: doc.stream.file_version.clone(),
        save_version: doc.stream.save_version.clone(),
        stream_properties,
        stream_properties_tree,
        terminal_order: doc
            .stream
            .diagram
            .terminal_order
            .iter()
            .map(reference)
            .collect(),
        node: encode_nodes(
            &doc.stream.diagram,
            node_extras.as_ref().and_then(Value::as_table),
        )?,
        link: encode_links(&doc.stream.diagram),
        attachment,
        extra,
    };
    let value = Value::try_from(native).map_err(format_error)?;
    let mut text = String::new();
    write_table(&value, "", &mut text)?;
    if text.len() > MAX_BYTES {
        return Err(DocError::TooLarge("serialized file exceeds 1 GiB".into()));
    }
    Ok(text)
}

pub fn from_toml(text: &str) -> Result<StreamDoc, DocError> {
    check_toml_depth(text)?;
    bounded_stack(|| decode_document(text))
}
fn decode_document(text: &str) -> Result<StreamDoc, DocError> {
    if text.len() > MAX_BYTES {
        return Err(DocError::TooLarge("input exceeds 1 GiB".into()));
    }
    let table: toml::Table = toml::from_str(text).map_err(format_error)?;
    if let Some(version) = table
        .get("version")
        .and_then(Value::as_integer)
        .and_then(|v| u32::try_from(v).ok())
        && version > 1
    {
        return Err(DocError::Version(version));
    }
    let native: Native = table.try_into().map_err(format_error)?;
    if native.version > 1 {
        return Err(DocError::Version(native.version));
    }
    if native.version != 1 || native.format != "dms-stream" {
        return Err(format_error("expected format dms-stream, version 1"));
    }
    if native.link.len() > MAX_LINKS
        || native.terminal_order.len() > MAX_NODES
        || native.attachment.len() > MAX_ATTACHMENTS
    {
        return Err(DocError::TooLarge(
            "links, terminal order or attachments".into(),
        ));
    }
    let mut extra = native.extra;
    let mut node_extras = toml::map::Map::new();
    let mut counts = (0, native.link.len());
    let nodes = decode_nodes(native.node, &mut node_extras, 0, &mut counts)?;
    if !node_extras.is_empty() {
        extra.insert("node".into(), Value::Table(node_extras));
    }
    let mut attachments = BTreeMap::new();
    let mut total = 0usize;
    for a in native.attachment {
        let bytes: Arc<[u8]> = match (a.text, a.base64) {
            (Some(text), None) if text.len() <= MAX_ATTACHMENT => text.into_bytes().into(),
            (None, Some(base64)) => crate::base64::decode(&base64, MAX_ATTACHMENT)
                .map_err(format_error)?
                .into(),
            (Some(_), None) => return Err(DocError::TooLarge("attachment exceeds 256 MiB".into())),
            _ => {
                return Err(format_error(
                    "attachment requires exactly one of text or base64",
                ));
            }
        };
        total = total.saturating_add(bytes.len());
        if total > MAX_BYTES {
            return Err(DocError::TooLarge("total attachments exceed 1 GiB".into()));
        }
        if attachments.insert(a.name, bytes).is_some() {
            return Err(format_error("duplicate attachment name"));
        }
    }
    let doc = StreamDoc {
        stream: Stream {
            file_version: native.file_version,
            save_version: native.save_version,
            properties: read_setting(native.stream_properties, native.stream_properties_tree)?,
            diagram: Diagram {
                nodes,
                links: decode_links(native.link),
                terminal_order: native
                    .terminal_order
                    .into_iter()
                    .map(parse_reference)
                    .collect(),
            },
        },
        attachments,
        meta: DocMeta {
            imported_from: native.imported_from,
            extra,
        },
    };
    validate(&doc)?;
    Ok(doc)
}

/// Checks size before allocation, and caps the read too if the file grows after metadata().
pub fn load(path: &Path) -> Result<StreamDoc, DocError> {
    let file = File::open(path).map_err(|e| DocError::Read(format!("{}: {e}", path.display())))?;
    if file
        .metadata()
        .map_err(|e| DocError::Read(e.to_string()))?
        .len()
        > MAX_BYTES as u64
    {
        return Err(DocError::TooLarge("file exceeds 1 GiB".into()));
    }
    let mut text = String::new();
    file.take(MAX_BYTES as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| DocError::Read(format!("{}: {e}", path.display())))?;
    from_toml(&text)
}

static TEMP_ID: AtomicU64 = AtomicU64::new(0);
pub fn save_atomic(doc: &StreamDoc, path: &Path) -> Result<(), DocError> {
    let text = to_toml(doc)?;
    let name = path
        .file_stem()
        .ok_or_else(|| format_error("save path needs a file name"))?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    for _ in 0..100 {
        let mut temp_name = std::ffi::OsString::from(".");
        temp_name.push(name);
        temp_name.push(format!(
            ".dms.tmp-{}-{}",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let temp = parent.join(temp_name);
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(DocError::Read(format!(
                    "cannot create {}: {e}",
                    temp.display()
                )));
            }
        };
        let result = (|| {
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp, path)
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&temp);
            return Err(DocError::Read(format!(
                "cannot save {}: {e}",
                path.display()
            )));
        }
        return Ok(());
    }
    Err(DocError::Read(
        "could not reserve a temporary file name after 100 attempts".into(),
    ))
}

pub(crate) fn validate_element(e: &Element) -> Result<(), DocError> {
    let mut pending = vec![(e, 1usize)];
    let mut count = 0usize;
    while let Some((e, depth)) = pending.pop() {
        count = count
            .saturating_add(1)
            .saturating_add(e.children.len())
            .saturating_add(e.attributes.len());
        if depth > 512 || count > 2_000_000 {
            return Err(DocError::TooLarge("settings depth or item count".into()));
        }
        pending.extend(e.child_elements().map(|c| (c, depth.saturating_add(1))));
    }
    Ok(())
}
pub(crate) fn validate(doc: &StreamDoc) -> Result<(), DocError> {
    let mut extra_values: Vec<_> = doc.meta.extra.values().map(|v| (v, 0usize)).collect();
    let mut extra_count = 0usize;
    while let Some((v, depth)) = extra_values.pop() {
        extra_count = extra_count.saturating_add(1);
        if depth > 1024 || extra_count > 2_000_000 {
            return Err(DocError::TooLarge(
                "unknown fields depth or item count".into(),
            ));
        }
        match v {
            Value::Array(a) => extra_values.extend(a.iter().map(|v| (v, depth.saturating_add(1)))),
            Value::Table(t) => {
                extra_values.extend(t.values().map(|v| (v, depth.saturating_add(1))))
            }
            _ => {}
        }
    }
    if let Some(e) = &doc.stream.properties {
        validate_element(e)?;
    }
    let mut pending = vec![(&doc.stream.diagram, 0usize)];
    let mut uids = HashSet::new();
    let mut links = 0usize;
    while let Some((d, depth)) = pending.pop() {
        if depth > 64 {
            return Err(DocError::TooLarge("supernode depth exceeds 64".into()));
        }
        links = links.saturating_add(d.links.len());
        if links > MAX_LINKS || d.terminal_order.len() > MAX_NODES {
            return Err(DocError::TooLarge("links or terminal order".into()));
        }
        for n in &d.nodes {
            if !uids.insert(&n.uid) {
                return Err(format_error(format!("duplicate uid {}", n.uid)));
            }
            if uids.len() > MAX_NODES {
                return Err(DocError::TooLarge("nodes".into()));
            }
            if n.label
                .as_ref()
                .is_some_and(|s| s.chars().take(MAX_LABEL + 1).count() > MAX_LABEL)
            {
                return Err(DocError::TooLarge("label exceeds 1000 characters".into()));
            }
            if n.containers.len() > MAX_ATTACHMENTS || n.result_items.len() > MAX_NODES {
                return Err(DocError::TooLarge("containers or result items".into()));
            }
            for e in n
                .settings
                .iter()
                .chain(n.binning_output.iter())
                .chain(&n.result_items)
            {
                validate_element(e)?;
            }
            if let Some(d) = &n.subdiagram {
                pending.push((d, depth.saturating_add(1)));
            }
        }
    }
    if doc.attachments.len() > MAX_ATTACHMENTS {
        return Err(DocError::TooLarge("attachment count".into()));
    }
    let mut total = 0usize;
    for bytes in doc.attachments.values() {
        total = total.saturating_add(bytes.len());
        if bytes.len() > MAX_ATTACHMENT || total > MAX_BYTES {
            return Err(DocError::TooLarge("attachment bytes".into()));
        }
    }
    Ok(())
}

// Structured fallback retains every infoset field, including namespace declaration order,
// empty optional strings, invalid XML characters, and non-XML leaf kinds.
fn table(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Table(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}
fn string(s: &str) -> Value {
    Value::String(s.to_string())
}
fn optional(t: &mut Value, key: &str, value: Option<&str>) {
    if let (Some(t), Some(s)) = (t.as_table_mut(), value) {
        t.insert(key.into(), string(s));
    }
}
fn name_value(n: &QualifiedName) -> Value {
    let mut t = table([("local", string(&n.local))]);
    optional(&mut t, "prefix", n.prefix.as_deref());
    optional(&mut t, "namespace", n.namespace.as_deref());
    t
}
fn tree_value(e: &Element) -> Result<Value, DocError> {
    validate_element(e)?;
    fn encode(e: &Element) -> Value {
        let declarations = e
            .namespace_declarations
            .iter()
            .map(|n| {
                let mut t = table([]);
                optional(&mut t, "prefix", n.prefix.as_deref());
                optional(&mut t, "namespace", n.namespace.as_deref());
                t
            })
            .collect();
        table([
            ("name", name_value(&e.name)),
            ("namespace_declarations", Value::Array(declarations)),
            (
                "attributes",
                Value::Array(
                    e.attributes
                        .iter()
                        .map(|a| {
                            table([("name", name_value(&a.name)), ("value", string(&a.value))])
                        })
                        .collect(),
                ),
            ),
            (
                "children",
                Value::Array(
                    e.children
                        .iter()
                        .map(|n| match n {
                            Node::Element(e) => {
                                table([("kind", string("element")), ("element", encode(e))])
                            }
                            Node::Text(s) => table([("kind", string("text")), ("text", string(s))]),
                            Node::Comment(s) => {
                                table([("kind", string("comment")), ("text", string(s))])
                            }
                            Node::ProcessingInstruction { target, content } => table([
                                ("kind", string("pi")),
                                ("target", string(target)),
                                ("content", string(content)),
                            ]),
                            Node::UnexpandedEntityReference {
                                name,
                                system_identifier,
                                public_identifier,
                            } => {
                                let mut t =
                                    table([("kind", string("entity")), ("name", string(name))]);
                                optional(&mut t, "system_identifier", system_identifier.as_deref());
                                optional(&mut t, "public_identifier", public_identifier.as_deref());
                                t
                            }
                            Node::DocumentTypeDeclaration {
                                system_identifier,
                                public_identifier,
                                processing_instructions,
                            } => {
                                let mut t = table([
                                    ("kind", string("doctype")),
                                    (
                                        "processing_instructions",
                                        Value::Array(
                                            processing_instructions
                                                .iter()
                                                .map(|(a, b)| {
                                                    Value::Array(vec![string(a), string(b)])
                                                })
                                                .collect(),
                                        ),
                                    ),
                                ]);
                                optional(&mut t, "system_identifier", system_identifier.as_deref());
                                optional(&mut t, "public_identifier", public_identifier.as_deref());
                                t
                            }
                        })
                        .collect(),
                ),
            ),
        ])
    }
    Ok(encode(e))
}
fn required<'a>(v: &'a Value, key: &str) -> Result<&'a Value, DocError> {
    v.get(key)
        .ok_or_else(|| format_error(format!("missing tree field {key}")))
}
fn text_field<'a>(v: &'a Value, key: &str) -> Result<&'a str, DocError> {
    required(v, key)?
        .as_str()
        .ok_or_else(|| format_error(format!("tree {key} must be text")))
}
fn opt_field(v: &Value, key: &str) -> Result<Option<Arc<str>>, DocError> {
    v.get(key)
        .map(|v| {
            v.as_str()
                .map(Arc::from)
                .ok_or_else(|| format_error(format!("tree {key} must be text")))
        })
        .transpose()
}
fn list<'a>(v: &'a Value, key: &str) -> Result<&'a [Value], DocError> {
    match v.get(key) {
        None => Ok(&[]),
        Some(Value::Array(a)) => Ok(a),
        _ => Err(format_error(format!("tree {key} must be an array"))),
    }
}
fn read_name(v: &Value) -> Result<QualifiedName, DocError> {
    Ok(QualifiedName {
        local: Arc::from(text_field(v, "local")?),
        prefix: opt_field(v, "prefix")?,
        namespace: opt_field(v, "namespace")?,
    })
}
fn read_tree(v: &Value) -> Result<Element, DocError> {
    fn read(v: &Value, depth: usize, count: &mut usize) -> Result<Element, DocError> {
        if depth > 512 {
            return Err(DocError::TooLarge("settings depth".into()));
        }
        *count = count
            .saturating_add(1)
            .saturating_add(list(v, "children")?.len())
            .saturating_add(list(v, "attributes")?.len());
        if *count > 2_000_000 {
            return Err(DocError::TooLarge("settings items".into()));
        }
        let name = read_name(required(v, "name")?)?;
        let namespace_declarations = list(v, "namespace_declarations")?
            .iter()
            .map(|n| {
                Ok(NamespaceDeclaration {
                    prefix: opt_field(n, "prefix")?,
                    namespace: opt_field(n, "namespace")?,
                })
            })
            .collect::<Result<_, DocError>>()?;
        let attributes = list(v, "attributes")?
            .iter()
            .map(|a| {
                Ok(Attribute {
                    name: read_name(required(a, "name")?)?,
                    value: Arc::from(text_field(a, "value")?),
                })
            })
            .collect::<Result<_, DocError>>()?;
        let children = list(v, "children")?
            .iter()
            .map(|n| {
                Ok(match text_field(n, "kind")? {
                    "element" => Node::Element(read(
                        required(n, "element")?,
                        depth.saturating_add(1),
                        count,
                    )?),
                    "text" => Node::Text(Arc::from(text_field(n, "text")?)),
                    "comment" => Node::Comment(Arc::from(text_field(n, "text")?)),
                    "pi" => Node::ProcessingInstruction {
                        target: Arc::from(text_field(n, "target")?),
                        content: Arc::from(text_field(n, "content")?),
                    },
                    "entity" => Node::UnexpandedEntityReference {
                        name: Arc::from(text_field(n, "name")?),
                        system_identifier: opt_field(n, "system_identifier")?,
                        public_identifier: opt_field(n, "public_identifier")?,
                    },
                    "doctype" => Node::DocumentTypeDeclaration {
                        system_identifier: opt_field(n, "system_identifier")?,
                        public_identifier: opt_field(n, "public_identifier")?,
                        processing_instructions: list(n, "processing_instructions")?
                            .iter()
                            .map(|pair| {
                                let pair = pair
                                    .as_array()
                                    .filter(|p| p.len() == 2)
                                    .ok_or_else(|| format_error("doctype PI needs two strings"))?;
                                let a = pair
                                    .first()
                                    .and_then(Value::as_str)
                                    .ok_or_else(|| format_error("doctype PI target"))?;
                                let b = pair
                                    .get(1)
                                    .and_then(Value::as_str)
                                    .ok_or_else(|| format_error("doctype PI content"))?;
                                Ok((Arc::from(a), Arc::from(b)))
                            })
                            .collect::<Result<_, DocError>>()?,
                    },
                    other => {
                        return Err(format_error(format!(
                            "unknown structured child kind {other}"
                        )));
                    }
                })
            })
            .collect::<Result<_, DocError>>()?;
        Ok(Element {
            name,
            namespace_declarations,
            attributes,
            children,
        })
    }
    read(v, 1, &mut 0)
}

// Write section arrays explicitly, with inline extension/container/tree tables. XML strings
// use multiline literals whenever TOML can represent them exactly; other strings use the
// dependency's escaping (including attachments containing control characters).
fn write_value(v: &Value, xml: bool, out: &mut String) -> Result<(), DocError> {
    match v {
        Value::String(s)
            if xml
                && !s.contains("'''")
                && !s
                    .chars()
                    .any(|c| c == '\r' || (c.is_control() && c != '\n' && c != '\t')) =>
        {
            out.push_str("'''\n");
            out.push_str(s);
            out.push_str("'''");
        }
        Value::Array(values) => {
            out.push('[');
            for (i, v) in values.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_value(v, xml, out)?;
            }
            out.push(']');
        }
        Value::Table(values) => {
            out.push_str("{ ");
            for (i, (k, v)) in values.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_key(k, out)?;
                out.push_str(" = ");
                write_value(v, false, out)?;
            }
            out.push_str(" }");
        }
        _ => {
            v.serialize(toml::ser::ValueSerializer::new(out))
                .map_err(format_error)?;
        }
    }
    if out.len() > MAX_BYTES {
        return Err(DocError::TooLarge("serialized file exceeds 1 GiB".into()));
    }
    Ok(())
}
fn write_key(key: &str, out: &mut String) -> Result<(), DocError> {
    if !key.is_empty()
        && key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        out.push_str(key);
    } else {
        key.serialize(toml::ser::ValueSerializer::new(out))
            .map_err(format_error)?;
    }
    Ok(())
}
fn write_table(v: &Value, path: &str, out: &mut String) -> Result<(), DocError> {
    let values = v
        .as_table()
        .ok_or_else(|| format_error("expected a native table"))?;
    let node_table = path.split('.').next_back() == Some("node");
    let diagram_table = path.is_empty() || path.ends_with(".subdiagram");
    for (key, v) in values {
        if (diagram_table && ["node", "link"].contains(&key.as_str()))
            || (path.is_empty() && key == "attachment")
            || (node_table && key == "subdiagram")
        {
            continue;
        }
        write_key(key, out)?;
        out.push_str(" = ");
        write_value(
            v,
            [
                "settings",
                "binning_output",
                "stream_properties",
                "result_items",
                "text",
            ]
            .contains(&key.as_str()),
            out,
        )?;
        out.push('\n');
    }
    if let Some(d) = values.get("subdiagram").filter(|_| node_table) {
        let nested = format!("{path}.subdiagram");
        out.push_str(&format!("\n[{nested}]\n"));
        write_table(d, &nested, out)?;
    }
    for key in ["node", "link", "attachment"] {
        if let Some(Value::Array(items)) = values
            .get(key)
            .filter(|_| diagram_table && (key != "attachment" || path.is_empty()))
        {
            let nested = if path.is_empty() {
                key.to_string()
            } else {
                format!("{path}.{key}")
            };
            if items.is_empty() {
                continue;
            }
            for item in items {
                out.push_str(&format!("\n[[{nested}]]\n"));
                write_table(item, &nested, out)?;
            }
        }
    }
    Ok(())
}
fn check_toml_depth(text: &str) -> Result<(), DocError> {
    if text.len() > MAX_BYTES {
        return Err(DocError::TooLarge("input exceeds 1 GiB".into()));
    }
    let bytes = text.as_bytes();
    let mut at = 0usize;
    let mut brackets: Vec<(u8, usize)> = Vec::new();
    let mut table_depth = 0usize;
    let mut value_depth = 0usize;
    let mut dots = 0usize;
    let mut reading_key = true;
    let mut header = false;
    let mut quote = None;
    let mut triple = false;
    while let Some(&b) = bytes.get(at) {
        if let Some(q) = quote {
            if b == b'\\' && q == b'"' {
                at = at.saturating_add(2);
                continue;
            }
            if b == q {
                if !triple {
                    quote = None;
                } else {
                    let run = bytes
                        .get(at..)
                        .unwrap_or_default()
                        .iter()
                        .take_while(|&&b| b == q)
                        .count();
                    if run >= 3 {
                        quote = None;
                        at = at.saturating_add(run.saturating_sub(1));
                    }
                }
            }
        } else {
            match b {
                b'#' => {
                    while bytes.get(at).is_some_and(|&b| b != b'\n') {
                        at = at.saturating_add(1);
                    }
                    continue;
                }
                b'"' | b'\'' => {
                    quote = Some(b);
                    triple = bytes
                        .get(at..at.saturating_add(3))
                        .is_some_and(|s| s.iter().all(|&c| c == b));
                    if triple {
                        at = at.saturating_add(2);
                    }
                }
                b'[' | b'{' => {
                    if brackets.is_empty() && reading_key && b == b'[' {
                        header = true;
                    }
                    if header {
                        brackets.push((b, 0));
                    } else {
                        brackets.push((b, value_depth));
                        value_depth = value_depth.saturating_add(1);
                        reading_key = b == b'{';
                        dots = 0;
                    }
                }
                b']' | b'}' => {
                    brackets.pop();
                    if header {
                        if brackets.is_empty() {
                            table_depth = dots.saturating_add(1);
                            value_depth = table_depth;
                            header = false;
                            reading_key = false;
                            dots = 0;
                        }
                    } else {
                        value_depth = brackets
                            .last()
                            .map_or(table_depth, |(_, d)| *d)
                            .saturating_add(1);
                        reading_key = false;
                        dots = 0;
                    }
                }
                b'=' => {
                    let base = brackets.last().map_or(table_depth, |(_, d)| *d);
                    value_depth = base.saturating_add(dots).saturating_add(1);
                    reading_key = false;
                    dots = 0;
                }
                b',' => {
                    reading_key = brackets.last().is_some_and(|(b, _)| *b == b'{');
                    value_depth = brackets
                        .last()
                        .map_or(table_depth, |(_, d)| *d)
                        .saturating_add(1);
                    dots = 0;
                }
                b'.' if reading_key || header => dots = dots.saturating_add(1),
                b'\n' if !header => {
                    reading_key =
                        brackets.is_empty() || brackets.last().is_some_and(|(b, _)| *b == b'{');
                    value_depth = brackets
                        .last()
                        .map_or(table_depth, |(_, d)| *d)
                        .saturating_add(1);
                    dots = 0;
                }
                _ => {}
            }
            let base = if header {
                0
            } else {
                brackets.last().map_or(table_depth, |(_, d)| *d)
            };
            if brackets.len() > 2048
                || dots > 1024
                || value_depth > 2048
                || base.saturating_add(dots) > 2048
            {
                return Err(DocError::TooLarge("TOML nesting exceeds 2048".into()));
            }
        }
        at = at.saturating_add(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn doc() -> StreamDoc {
        let mut d = StreamDoc::new();
        let mut n = crate::edit::tests::node("n1");
        n.settings =
            Some(element_from_xml("<properties a=\"&amp;\">text<q/></properties>").unwrap());
        n.binning_output = n.settings.clone();
        n.result_items = vec![n.settings.clone().unwrap()];
        n.extension = Some(Extension::Element {
            extension_id: "ext".into(),
            element_id: "obj".into(),
        });
        n.containers.push(Container {
            name: None,
            member: "data/model".into(),
        });
        d.stream.diagram.nodes.push(n);
        d.stream
            .diagram
            .terminal_order
            .push(NodeRef::Unresolved("7".into()));
        d.attachments
            .insert("data/text".into(), Arc::from("hello\nworld".as_bytes()));
        d.attachments
            .insert("data/binary".into(), Arc::from([0u8, 255, 128]));
        d
    }
    #[test]
    fn deterministic_format_and_unknown_keys() {
        let d = doc();
        let one = to_toml(&d).unwrap();
        assert!(one.contains("settings = '''\n<properties"));
        let two = to_toml(&from_toml(&one).unwrap()).unwrap();
        assert_eq!(one, two);
        let text = one.replacen("class =", "future_node = { x = [1, 2] }\nclass =", 1);
        let text = format!("future_top = [1, 2]\nsubdiagram = 'unknown top key'\n{text}");
        let loaded = from_toml(&text).unwrap();
        let written = to_toml(&loaded).unwrap();
        let again = from_toml(&written).unwrap();
        assert_eq!(loaded.meta, again.meta);
        assert!(written.contains("future_node"));
        assert!(written.contains("future_top"));
        assert!(written.contains("unknown top key"));
        assert_eq!(to_toml(&again).unwrap(), written);
        assert!(matches!(
            from_toml("format='dms-stream'\nversion=2"),
            Err(DocError::Version(2))
        ));
        assert!(from_toml("format='other'\nversion=1").is_err());
        assert!(
            from_toml(
                "format='dms-stream'\nversion=1\n[[attachment]]\nname='x'\ntext='a'\nbase64='YQ=='"
            )
            .is_err()
        );
    }
    #[test]
    fn every_infoset_variant_uses_lossless_fallback() {
        let mut d = doc();
        let n = d.stream.diagram.nodes.first_mut().unwrap();
        let mut e = n.settings.clone().unwrap();
        e.children.extend([
            Node::Text(Arc::from("\0\r")),
            Node::Text(Arc::from("adjacent")),
            Node::Comment(Arc::from("invalid--comment")),
            Node::ProcessingInstruction {
                target: Arc::from("x"),
                content: Arc::from("a?>b"),
            },
            Node::UnexpandedEntityReference {
                name: Arc::from("missing"),
                system_identifier: Some(Arc::from("")),
                public_identifier: None,
            },
            Node::DocumentTypeDeclaration {
                system_identifier: None,
                public_identifier: Some(Arc::from("id")),
                processing_instructions: vec![(Arc::from("pi"), Arc::from("content"))],
            },
        ]);
        e.name.prefix = Some(Arc::from("unbound"));
        n.settings = Some(e.clone());
        n.binning_output = Some(e.clone());
        n.result_items = vec![e.clone()];
        d.stream.properties = Some(e.clone());
        let text = to_toml(&d).unwrap();
        assert!(text.contains("settings_tree = {"));
        let loaded = from_toml(&text).unwrap();
        let n = loaded.node("n1").unwrap();
        assert_eq!(n.settings, Some(e.clone()));
        assert_eq!(n.binning_output, Some(e.clone()));
        assert_eq!(n.result_items, vec![e.clone()]);
        assert_eq!(loaded.stream.properties, Some(e));
        assert_eq!(text, to_toml(&loaded).unwrap());
    }
    #[test]
    fn depth_boundaries_and_malformed_input_never_crash() {
        let mut d = StreamDoc::new();
        let mut n = crate::edit::tests::node("n64");
        for i in (0..64).rev() {
            let mut parent = crate::edit::tests::node(&format!("n{i}"));
            parent.subdiagram = Some(Diagram {
                nodes: vec![n],
                ..Default::default()
            });
            n = parent;
        }
        d.stream.diagram.nodes.push(n);
        let text = to_toml(&d).unwrap();
        assert_eq!(to_toml(&from_toml(&text).unwrap()).unwrap(), text);
        let deep = format!("{}x{}", "<p>".repeat(512), "</p>".repeat(512));
        let mut e = element_from_xml(&deep).unwrap();
        // Force fallback at depth 512, including an XML-invalid leaf deep in the tree.
        let mut inner = &mut e;
        for _ in 0..511 {
            inner = match inner.children.first_mut().unwrap() {
                Node::Element(e) => e,
                _ => panic!("element"),
            };
        }
        inner.children.push(Node::Text(Arc::from("\0")));
        let mut simple = StreamDoc::new();
        simple.stream.properties = Some(e);
        let encoded = to_toml(&simple).unwrap();
        let decoded = from_toml(&encoded).unwrap();
        assert_eq!(encoded, to_toml(&decoded).unwrap());
        assert!(from_toml(&format!("x={}0{}", "[".repeat(2049), "]".repeat(2049))).is_err());
        // Mutations of synthetic input, not team data. Syntax errors are ordinary Results.
        let seed_text = to_toml(&doc()).unwrap();
        for at in 0..seed_text.len() {
            if !seed_text.is_char_boundary(at) {
                continue;
            }
            let mut mutated = seed_text.clone();
            mutated.insert(at, '\0');
            let _ = from_toml(&mutated);
            let mut deleted = seed_text.clone();
            deleted.remove(at);
            let _ = from_toml(&deleted);
            let _ = from_toml(&seed_text[..at]);
        }
    }
    #[test]
    fn lexical_limits_distinguish_values_from_key_paths() {
        let many_floats = format!(
            "format='dms-stream'\nversion=1\nfuture=[{}]",
            vec!["0.1"; 3000].join(",")
        );
        assert!(from_toml(&many_floats).is_ok());
        let dotted = format!("{}=0", vec!["a"; 2050].join("."));
        assert!(matches!(from_toml(&dotted), Err(DocError::TooLarge(_))));
        let quoted = format!(
            "x='''value''''\ny={}0{}",
            "[".repeat(2049),
            "]".repeat(2049)
        );
        assert!(matches!(from_toml(&quoted), Err(DocError::TooLarge(_))));
        let key = vec!["k"; 601].join(".");
        let nested_keys = format!("{key} = {{ {key} = {{ {key} = {{ {key} = 0 }} }} }}");
        assert!(matches!(
            from_toml(&nested_keys),
            Err(DocError::TooLarge(_))
        ));
        let mut d = doc();
        for text in [
            "ending in '",
            "ending in ''",
            "contains ''' delimiter",
            "\nleading newline",
            "control \u{1} and \r",
        ] {
            d.attachments
                .insert("data/quotes".into(), Arc::from(text.as_bytes()));
            let serialized = to_toml(&d).unwrap();
            assert_eq!(from_toml(&serialized).unwrap().attachments, d.attachments);
        }
        assert!(matches!(
            from_toml("format='dms-stream'\nversion=2\nnode='changed schema'"),
            Err(DocError::Version(2))
        ));
        let mut m = DocMeta::default();
        m.extra.insert("nan".into(), Value::Float(f64::NAN));
        assert_eq!(m, m.clone());
    }
    #[test]
    fn atomic_overwrite_and_failed_rename_cleanup() {
        let dir = std::env::temp_dir().join(format!(
            "dms-wp1-save-{}-{}",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stream.dms");
        fs::write(&path, b"original").unwrap();
        save_atomic(&doc(), &path).unwrap();
        let first = fs::read(&path).unwrap();
        assert!(load(&path).is_ok());
        let mut bad = doc();
        bad.stream
            .diagram
            .nodes
            .push(crate::edit::tests::node("n1"));
        assert!(save_atomic(&bad, &path).is_err());
        assert_eq!(fs::read(&path).unwrap(), first);
        let blocked = dir.join("blocked.dms");
        fs::create_dir(&blocked).unwrap();
        fs::write(blocked.join("original"), b"safe").unwrap();
        assert!(save_atomic(&doc(), &blocked).is_err());
        assert_eq!(fs::read(blocked.join("original")).unwrap(), b"safe");
        assert!(fs::read_dir(&dir).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp-")
        }));
        let large = dir.join("large.dms");
        File::create(&large)
            .unwrap()
            .set_len(MAX_BYTES as u64 + 1)
            .unwrap();
        assert!(matches!(load(&large), Err(DocError::TooLarge(_))));
        fs::remove_dir_all(dir).unwrap();
    }
}
