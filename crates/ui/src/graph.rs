//! What the canvas draws: a stream diagram resolved against the node catalog, in canvas
//! coordinates. Built once per document change, not per frame.

use crate::i18n::tr;
use dms_engine::catalog::{self, Palette, Resolution, Scope};
use dms_engine::nodes;
use dms_stream::{Diagram, NodeRef};
use egui::{Pos2, Rect, Vec2, pos2, vec2};
use std::collections::HashMap;

/// Saved positions are trusted only within this range (canvas units).
const MAX_COORDINATE: i64 = 1_000_000;
/// Spacing of the fallback grid for nodes saved without a position.
const GRID_STEP: f32 = 110.0;
/// Upper bound on nodes drawn from one diagram; the rest are counted, not drawn.
pub const MAX_NODES: usize = 20_000;

/// The size of a node tile in canvas units; `pos` is the tile centre.
pub const TILE: Vec2 = vec2(52.0, 52.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Palette(Palette),
    Supernode,
    /// Supernode connectors and similar structure.
    Structural,
    /// The catalog does not know this node type; it is kept and shown, not run.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    Ready,
    /// In the catalog but not implemented; the scope says why.
    NotImplemented(Scope),
    Unknown,
}

impl NodeStatus {
    pub fn explanation(self) -> &'static str {
        match self {
            NodeStatus::Ready => tr("Supported"),
            NodeStatus::NotImplemented(Scope::Mvp) => tr("Not implemented yet"),
            NodeStatus::NotImplemented(Scope::Later) => tr("Planned for a later release"),
            NodeStatus::NotImplemented(Scope::LicenseBlocked) => {
                tr("Cannot be verified: blocked by the reference licence")
            }
            NodeStatus::NotImplemented(Scope::OutOfScope) => tr("Not planned"),
            NodeStatus::Unknown => tr("Unknown node type: kept as saved, not run"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphNode {
    pub uid: String,
    pub title: String,
    /// Catalog id, or the stream's own type identifier when the catalog does not know it.
    pub type_id: String,
    pub kind: NodeKind,
    pub status: NodeStatus,
    pub pos: Pos2,
    /// A model nugget (a trained model), drawn differently from the node that builds it.
    pub nugget: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphLink {
    pub from: usize,
    pub to: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StreamGraph {
    pub nodes: Vec<GraphNode>,
    pub links: Vec<GraphLink>,
    /// Links whose ends are not nodes of this diagram (kept in the stream, not drawn).
    pub dangling_links: usize,
    /// Nodes beyond [`MAX_NODES`] (kept in the stream, not drawn).
    pub hidden_nodes: usize,
}

impl StreamGraph {
    /// The top-level diagram of a stream; supernodes appear as single nodes.
    pub fn from_diagram(diagram: &Diagram) -> Self {
        let mut graph = StreamGraph::default();
        let mut index = HashMap::new();
        let mut unplaced = 0_usize;
        for node in &diagram.nodes {
            if graph.nodes.len() >= MAX_NODES {
                graph.hidden_nodes = graph.hidden_nodes.saturating_add(1);
                continue;
            }
            let (kind, status, type_id, catalog_label) = match catalog::resolve(node) {
                Resolution::Node(spec) => (
                    NodeKind::Palette(spec.palette),
                    if nodes::is_implemented(spec.id) {
                        NodeStatus::Ready
                    } else {
                        NodeStatus::NotImplemented(spec.scope)
                    },
                    spec.id.to_string(),
                    Some(spec.label),
                ),
                Resolution::Structural => (
                    NodeKind::Structural,
                    NodeStatus::Ready,
                    node.type_identifier(),
                    None,
                ),
                Resolution::Unmapped => (
                    NodeKind::Unknown,
                    NodeStatus::Unknown,
                    node.type_identifier(),
                    None,
                ),
            };
            let kind = if node.subdiagram.is_some() {
                NodeKind::Supernode
            } else {
                kind
            };
            let title = node
                .label
                .clone()
                .or_else(|| node.name.clone())
                .or_else(|| catalog_label.map(str::to_string))
                .unwrap_or_else(|| type_id.clone());
            let pos = match node.position {
                Some((x, y)) => pos2(coordinate(x), coordinate(y)),
                None => {
                    let column = (unplaced % 8) as f32;
                    let row = (unplaced / 8) as f32;
                    unplaced = unplaced.saturating_add(1);
                    pos2(column * GRID_STEP, -GRID_STEP * (row + 1.0))
                }
            };
            let nugget = node.model_builder.is_some()
                || dms_stream::simple_class_name(&node.class).starts_with("Apply");
            index.insert(node.uid.clone(), graph.nodes.len());
            graph.nodes.push(GraphNode {
                uid: node.uid.clone(),
                title,
                type_id,
                kind,
                status,
                pos,
                nugget,
            });
        }
        for link in &diagram.links {
            let end = |r: &NodeRef| match r {
                NodeRef::Uid(uid) => index.get(uid).copied(),
                NodeRef::Unresolved(_) => None,
            };
            match (end(&link.from), end(&link.to)) {
                (Some(from), Some(to)) => graph.links.push(GraphLink { from, to }),
                _ => graph.dangling_links = graph.dangling_links.saturating_add(1),
            }
        }
        graph
    }

    /// The canvas area the nodes cover, tiles and labels included.
    pub fn bounds(&self) -> Option<Rect> {
        let mut nodes = self.nodes.iter();
        let first = nodes.next()?;
        let mut rect = Rect::from_center_size(first.pos, TILE);
        for node in nodes {
            rect = rect.union(Rect::from_center_size(node.pos, TILE));
        }
        // Room for the label under the bottom row and wide labels at the sides.
        Some(rect.expand2(vec2(40.0, 0.0)).with_max_y(rect.max.y + 28.0))
    }

    /// How many nodes cannot run in this version.
    pub fn unsupported(&self) -> usize {
        self.nodes
            .iter()
            .filter(|node| node.status != NodeStatus::Ready)
            .count()
    }
}

fn coordinate(value: i64) -> f32 {
    value.clamp(-MAX_COORDINATE, MAX_COORDINATE) as f32
}

/// Imported model_builder values usually identify a type, rather than a uid. Link only when
/// there is one matching build node, so a stream with two trainers does not claim false provenance.
pub fn model_source(diagram: &Diagram, uid: &str) -> Option<(String, Option<String>)> {
    let model = diagram.nodes.iter().find(|n| n.uid == uid)?;
    let builder = model.model_builder.as_deref()?;
    let is_builder = |n: &&dms_stream::StreamNode| {
        n.uid != uid
            && n.model_builder.is_none()
            && !dms_stream::simple_class_name(&n.class).starts_with("Apply")
    };
    let direct = diagram
        .nodes
        .iter()
        .filter(is_builder)
        .find(|n| n.uid == builder);
    let mut matches = diagram
        .nodes
        .iter()
        .filter(is_builder)
        .filter(|n| n.type_identifier() == builder);
    let first = matches.next();
    let source = direct.or_else(|| first.filter(|_| matches.next().is_none()));
    Some(match source {
        Some(node) => (
            node.label
                .clone()
                .or_else(|| node.name.clone())
                .or_else(|| {
                    catalog::by_modeler_identifier(&node.type_identifier())
                        .map(|s| s.label.to_string())
                })
                .unwrap_or_else(|| node.type_identifier()),
            Some(node.uid.clone()),
        ),
        None => (builder.to_string(), None),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic;

    #[test]
    fn resolves_nodes_links_and_status() {
        let stream = synthetic::stream(6);
        let graph = StreamGraph::from_diagram(&stream.diagram);
        assert_eq!(graph.nodes.len(), 6);
        assert_eq!(graph.links.len(), 5);
        assert_eq!(graph.dangling_links, 0);
        assert_eq!(graph.nodes[0].kind, NodeKind::Palette(Palette::Source));
        assert_eq!(graph.nodes[0].status, NodeStatus::Ready);
        assert!(graph.bounds().is_some());
    }

    #[test]
    fn keeps_unknown_nodes_and_counts_dangling_links() {
        let mut stream = synthetic::stream(2);
        stream.diagram.nodes[1].class = "com.example.MysteryNode".into();
        stream.diagram.nodes[1].position = None;
        stream.diagram.links.push(dms_stream::Link {
            from: NodeRef::Unresolved("7".into()),
            to: NodeRef::Uid(stream.diagram.nodes[0].uid.clone()),
        });
        let graph = StreamGraph::from_diagram(&stream.diagram);
        assert_eq!(graph.nodes[1].kind, NodeKind::Unknown);
        assert_eq!(graph.nodes[1].status, NodeStatus::Unknown);
        assert_eq!(graph.nodes[1].type_id, "MysteryNode");
        assert_eq!(graph.dangling_links, 1);
        assert_eq!(graph.unsupported(), 1);
    }

    #[test]
    fn hostile_coordinates_are_clamped() {
        let mut stream = synthetic::stream(1);
        stream.diagram.nodes[0].position = Some((i64::MAX, i64::MIN));
        let graph = StreamGraph::from_diagram(&stream.diagram);
        assert_eq!(graph.nodes[0].pos, pos2(1_000_000.0, -1_000_000.0));
    }

    #[test]
    fn empty_diagram_has_no_bounds() {
        assert_eq!(
            StreamGraph::from_diagram(&Diagram::default()).bounds(),
            None
        );
    }
}
