//! Stream model (L1): the nodes, links and properties of a Modeler stream.
//!
//! [`Stream::from_document`] interprets the decoded `.str` XML (see `docs/formats/str.md`):
//!
//! ```text
//! clementineStream[@version] / stream[@saveVersion]
//!   properties/*                                  stream properties
//!   diagram/nodes/node[@id @uid @class]           nodes (supernodes nest another diagram)
//!     properties/*                                node settings, under Modeler's internal names
//!     container[@containerData]/containerName     attachments (PMML, StatXML, ...)
//!     modelBuildInfo[@modelBuilderId]             the builder of a model nugget
//!   diagram/links/link[@srcId @dstId]             links between the per-save node ids
//! ```
//!
//! The per-save `id` is only meaningful within one diagram of one file; nodes are identified by
//! `uid`, and links are resolved to uids. Settings are kept as the original XML elements: typed
//! interpretation of each node's settings belongs to the node implementations.

use dms_strfmt::fastinfoset::{Document, Element};
use std::collections::HashMap;
use std::fmt;

/// Supernodes nest diagrams; deeper nesting than this is rejected.
const MAX_SUPERNODE_DEPTH: usize = 64;

#[derive(Debug, Clone)]
pub struct Stream {
    /// `clementineStream@version`: the Modeler version that saved the file.
    pub file_version: Option<String>,
    /// `stream@saveVersion`
    pub save_version: Option<String>,
    /// Stream properties (`stream/properties`), as saved.
    pub properties: Option<Element>,
    pub diagram: Diagram,
}

#[derive(Debug, Clone, Default)]
pub struct Diagram {
    pub nodes: Vec<StreamNode>,
    pub links: Vec<Link>,
    /// `terminalNodeOrder`: terminal nodes in execution order.
    pub terminal_order: Vec<NodeRef>,
}

#[derive(Debug, Clone)]
pub struct StreamNode {
    /// Stable node identity (`node@uid`).
    pub uid: String,
    /// Java class that implements the node in Modeler (`node@class`).
    pub class: String,
    /// How an extension node names its type (`node@extensionID` with `extensionObjectID` or
    /// `extensionElementID`); the class of such nodes is a generic wrapper.
    pub extension: Option<Extension>,
    /// `modelBuildInfo@modelBuilderId`: the node type that built a model nugget.
    pub model_builder: Option<String>,
    pub name: Option<String>,
    pub label: Option<String>,
    pub position: Option<(i64, i64)>,
    /// `node@properties`: attachment holding an extension node's settings.
    pub properties_attachment: Option<String>,
    /// `container` children: attachments with their container names.
    pub containers: Vec<Container>,
    /// The node's settings element (`properties`), as saved.
    pub settings: Option<Element>,
    /// A supernode's inner diagram.
    pub subdiagram: Option<Diagram>,
}

impl StreamNode {
    /// The identifier that names this node type in the catalog: the extension identifier for
    /// extension nodes, otherwise the simple class name.
    pub fn type_identifier(&self) -> String {
        match &self.extension {
            Some(Extension::Object {
                extension_id,
                object_id,
            }) => format!("{object_id}!{extension_id}"),
            Some(Extension::Element {
                extension_id,
                element_id,
            }) => format!("{extension_id}!{element_id}"),
            None => simple_class_name(&self.class).to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Extension {
    /// `extensionID` + `extensionObjectID` (identified as `object!extension`).
    Object {
        extension_id: String,
        object_id: String,
    },
    /// `extensionID` + `extensionElementID` (identified as `extension!element`).
    Element {
        extension_id: String,
        element_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub name: Option<String>,
    /// ZIP member, for example `data/0002.dat`.
    pub member: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub from: NodeRef,
    pub to: NodeRef,
}

/// A reference to a node of the same diagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeRef {
    Uid(String),
    /// A per-save id that matches no node in the diagram (kept, not dropped).
    Unresolved(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamError {
    /// The document element is not `clementineStream`.
    NotAStream(String),
    MissingElement(&'static str),
    NodeWithoutUid {
        class: String,
    },
    DuplicateUid(String),
    SupernodesTooDeep,
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamError::NotAStream(root) => {
                write!(f, "document element is {root:?}, not clementineStream")
            }
            StreamError::MissingElement(path) => write!(f, "missing {path}"),
            StreamError::NodeWithoutUid { class } => write!(f, "node of class {class} has no uid"),
            StreamError::DuplicateUid(uid) => write!(f, "uid {uid} used by several nodes"),
            StreamError::SupernodesTooDeep => {
                write!(f, "supernodes nested deeper than {MAX_SUPERNODE_DEPTH}")
            }
        }
    }
}

impl std::error::Error for StreamError {}

impl Stream {
    pub fn from_document(document: &Document) -> Result<Stream, StreamError> {
        let root = document
            .root()
            .ok_or(StreamError::MissingElement("document element"))?;
        if &*root.name.local != "clementineStream" {
            return Err(StreamError::NotAStream(root.name.local.to_string()));
        }
        let stream = root
            .child("stream")
            .ok_or(StreamError::MissingElement("clementineStream/stream"))?;
        let diagram = match stream.child("diagram") {
            Some(diagram) => read_diagram(diagram, 0)?,
            None => Diagram::default(),
        };
        let stream = Stream {
            file_version: root.attribute("version").map(str::to_string),
            save_version: stream.attribute("saveVersion").map(str::to_string),
            properties: stream.child("properties").cloned(),
            diagram,
        };
        stream.check_unique_uids()?;
        Ok(stream)
    }

    /// Every node, including nodes inside supernodes (depth-first, document order).
    pub fn all_nodes(&self) -> Vec<&StreamNode> {
        let mut out = Vec::new();
        let mut pending: Vec<&StreamNode> = self.diagram.nodes.iter().rev().collect();
        while let Some(node) = pending.pop() {
            out.push(node);
            if let Some(subdiagram) = &node.subdiagram {
                pending.extend(subdiagram.nodes.iter().rev());
            }
        }
        out
    }

    fn check_unique_uids(&self) -> Result<(), StreamError> {
        let mut seen = std::collections::HashSet::new();
        for node in self.all_nodes() {
            if !seen.insert(node.uid.as_str()) {
                return Err(StreamError::DuplicateUid(node.uid.clone()));
            }
        }
        Ok(())
    }
}

fn read_diagram(diagram: &Element, depth: usize) -> Result<Diagram, StreamError> {
    if depth > MAX_SUPERNODE_DEPTH {
        return Err(StreamError::SupernodesTooDeep);
    }
    let mut nodes = Vec::new();
    let mut uid_by_id = HashMap::new();
    if let Some(list) = diagram.child("nodes") {
        for element in list.child_elements().filter(|e| &*e.name.local == "node") {
            let node = read_node(element, depth)?;
            if let Some(id) = element.attribute("id") {
                uid_by_id.insert(id.to_string(), node.uid.clone());
            }
            nodes.push(node);
        }
    }
    let resolve = |id: &str| match uid_by_id.get(id) {
        Some(uid) => NodeRef::Uid(uid.clone()),
        None => NodeRef::Unresolved(id.to_string()),
    };
    let links = diagram
        .child("links")
        .map(|list| {
            list.child_elements()
                .filter(|e| &*e.name.local == "link")
                .map(|link| Link {
                    from: resolve(link.attribute("srcId").unwrap_or_default()),
                    to: resolve(link.attribute("dstId").unwrap_or_default()),
                })
                .collect()
        })
        .unwrap_or_default();
    let terminal_order = diagram
        .child("terminalNodeOrder")
        .map(|list| {
            list.child_elements()
                .filter_map(|node| node.attribute("id").map(&resolve))
                .collect()
        })
        .unwrap_or_default();
    Ok(Diagram {
        nodes,
        links,
        terminal_order,
    })
}

fn read_node(element: &Element, depth: usize) -> Result<StreamNode, StreamError> {
    let class = element.attribute("class").unwrap_or_default().to_string();
    let uid = element
        .attribute("uid")
        .ok_or_else(|| StreamError::NodeWithoutUid {
            class: class.clone(),
        })?
        .to_string();
    let settings = element.child("properties");
    let text = |name: &str| {
        settings
            .and_then(|settings| settings.child(name))
            .map(Element::text)
            .filter(|text| !text.is_empty())
    };
    let position = settings
        .and_then(|settings| settings.child("position"))
        .and_then(|position| {
            let coordinate = |axis: &str| position.child(axis)?.text().trim().parse::<i64>().ok();
            Some((coordinate("x")?, coordinate("y")?))
        });
    let extension = match (
        element.attribute("extensionID"),
        element.attribute("extensionObjectID"),
        element.attribute("extensionElementID"),
    ) {
        (Some(extension_id), Some(object_id), _) => Some(Extension::Object {
            extension_id: extension_id.to_string(),
            object_id: object_id.to_string(),
        }),
        (Some(extension_id), None, Some(element_id)) => Some(Extension::Element {
            extension_id: extension_id.to_string(),
            element_id: element_id.to_string(),
        }),
        _ => None,
    };
    let containers = element
        .child_elements()
        .filter(|e| &*e.name.local == "container")
        .filter_map(|container| {
            Some(Container {
                member: container.attribute("containerData")?.to_string(),
                name: container
                    .child("containerName")
                    .map(Element::text)
                    .filter(|name| !name.is_empty()),
            })
        })
        .collect();
    let subdiagram = match element.child("diagram") {
        Some(diagram) => Some(read_diagram(diagram, depth + 1)?),
        None => None,
    };
    Ok(StreamNode {
        uid,
        extension,
        model_builder: element
            .child("modelBuildInfo")
            .and_then(|info| info.attribute("modelBuilderId"))
            .map(str::to_string),
        name: text("name"),
        label: text("label"),
        position,
        properties_attachment: element.attribute("properties").map(str::to_string),
        containers,
        settings: settings.cloned(),
        subdiagram,
        class,
    })
}

/// `com.spss.clementine.component.node.SelectNode` -> `SelectNode`.
pub fn simple_class_name(class: &str) -> &str {
    class.rsplit('.').next().unwrap_or(class)
}
