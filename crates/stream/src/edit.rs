//! Reversible structural edits of the top-level canvas.
use crate::doc::{MAX_ATTACHMENT, MAX_ATTACHMENTS, MAX_BYTES, MAX_LABEL, MAX_LINKS, MAX_NODES};
use crate::{Link, NodeRef, StreamDoc, StreamNode};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};

// Keep the handoff API: AddNode owns a StreamNode rather than a Box.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Edit {
    AddNode {
        node: StreamNode,
        index: usize,
    },
    RemoveNode {
        uid: String,
    },
    Connect {
        link: Link,
        index: usize,
    },
    Disconnect {
        link: Link,
    },
    Move {
        uids: Vec<String>,
        dx: i64,
        dy: i64,
    },
    Rename {
        uid: String,
        label: Option<String>,
    },
    SetSettings {
        uid: String,
        settings: Option<dms_strfmt::fastinfoset::Element>,
    },
    SetTerminalOrder {
        order: Vec<NodeRef>,
    },
    PutAttachment {
        name: String,
        bytes: Option<Arc<[u8]>>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    UnknownNode(String),
    DuplicateUid(String),
    StillLinked(String),
    NotTopLevel(String),
    WouldCycle { from: String, to: String },
    SelfLink(String),
    Duplicate,
    OutOfRange(String),
    Limit(&'static str),
}
impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownNode(s) => write!(f, "unknown node {s}"),
            Self::DuplicateUid(s) => write!(f, "duplicate node uid {s}"),
            Self::StillLinked(s) => write!(f, "node {s} still has links"),
            Self::NotTopLevel(s) => write!(f, "node {s} is inside a read-only supernode"),
            Self::WouldCycle { from, to } => write!(f, "link {from} -> {to} would create a cycle"),
            Self::SelfLink(s) => write!(f, "cannot connect node {s} to itself"),
            Self::Duplicate => f.write_str("duplicate edit target or link"),
            Self::OutOfRange(s) => write!(f, "out of range: {s}"),
            Self::Limit(s) => write!(f, "edit limit exceeded: {s}"),
        }
    }
}
impl std::error::Error for EditError {}
impl StreamDoc {
    fn top_index(&self, uid: &str) -> Result<usize, EditError> {
        self.stream
            .diagram
            .nodes
            .iter()
            .position(|n| n.uid == uid)
            .ok_or_else(|| {
                if self.stream.all_nodes().iter().any(|n| n.uid == uid) {
                    EditError::NotTopLevel(uid.into())
                } else {
                    EditError::UnknownNode(uid.into())
                }
            })
    }
    fn endpoint(&self, reference: &NodeRef) -> Result<String, EditError> {
        match reference {
            NodeRef::Uid(uid) => {
                self.top_index(uid)?;
                Ok(uid.clone())
            }
            NodeRef::Unresolved(id) => Err(EditError::UnknownNode(format!("?{id}"))),
        }
    }
    // Disconnect must also validate the inverse: imported graphs can contain malformed
    // cycles or duplicate links, which an ordinary Connect is not allowed to recreate.
    fn check_link(&self, link: &Link, ignored: Option<usize>) -> Result<(), EditError> {
        let from = self.endpoint(&link.from)?;
        let to = self.endpoint(&link.to)?;
        if from == to {
            return Err(EditError::SelfLink(from));
        }
        let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();
        for (index, existing) in self.stream.diagram.links.iter().enumerate() {
            if Some(index) == ignored {
                continue;
            }
            if existing == link {
                return Err(EditError::Duplicate);
            }
            if let (NodeRef::Uid(a), NodeRef::Uid(b)) = (&existing.from, &existing.to) {
                edges.entry(a).or_default().push(b);
            }
        }
        let mut pending = vec![to.as_str()];
        let mut visited = HashSet::new();
        while let Some(uid) = pending.pop() {
            if uid == from {
                return Err(EditError::WouldCycle { from, to });
            }
            if visited.insert(uid)
                && let Some(next) = edges.get(uid)
            {
                pending.extend(next);
            }
        }
        Ok(())
    }
    /// Validate before changing any field. A missing position remains missing when moved;
    /// positioning such a node belongs to the canvas layout, not a relative delta.
    pub fn apply(&mut self, edit: Edit) -> Result<Edit, EditError> {
        match edit {
            Edit::AddNode { node, index } => {
                if index > self.stream.diagram.nodes.len() {
                    return Err(EditError::OutOfRange("node index".into()));
                }
                let mut existing: HashSet<&str> = self
                    .stream
                    .all_nodes()
                    .into_iter()
                    .map(|n| n.uid.as_str())
                    .collect();
                let mut count = existing.len();
                let mut links = self.stream.diagram.links.len();
                // Count existing nested links as well.
                for n in self.stream.all_nodes() {
                    if let Some(d) = &n.subdiagram {
                        links = links.saturating_add(d.links.len());
                    }
                }
                let mut pending = vec![(&node, 0usize)];
                while let Some((n, depth)) = pending.pop() {
                    if depth > 64 {
                        return Err(EditError::Limit("supernode depth"));
                    }
                    count = count.saturating_add(1);
                    if count > MAX_NODES {
                        return Err(EditError::Limit("nodes"));
                    }
                    if !existing.insert(&n.uid) {
                        return Err(EditError::DuplicateUid(n.uid.clone()));
                    }
                    if n.containers.len() > MAX_ATTACHMENTS || n.result_items.len() > MAX_NODES {
                        return Err(EditError::Limit("containers or result items"));
                    }
                    check_label(n.label.as_deref())?;
                    for e in n
                        .settings
                        .iter()
                        .chain(n.binning_output.iter())
                        .chain(&n.result_items)
                    {
                        crate::native::validate_element(e)
                            .map_err(|_| EditError::Limit("settings tree"))?;
                    }
                    if let Some(d) = &n.subdiagram {
                        links = links.saturating_add(d.links.len());
                        if links > MAX_LINKS {
                            return Err(EditError::Limit("links"));
                        }
                        if d.terminal_order.len() > MAX_NODES {
                            return Err(EditError::Limit("terminal order"));
                        }
                        pending.extend(d.nodes.iter().map(|n| (n, depth.saturating_add(1))));
                    }
                }
                if self.stream.diagram.links.iter().any(|link| {
                    link.from == NodeRef::Uid(node.uid.clone())
                        || link.to == NodeRef::Uid(node.uid.clone())
                }) {
                    return Err(EditError::StillLinked(node.uid));
                }
                let inverse = Edit::RemoveNode {
                    uid: node.uid.clone(),
                };
                self.stream.diagram.nodes.insert(index, node);
                Ok(inverse)
            }
            Edit::RemoveNode { uid } => {
                let index = self.top_index(&uid)?;
                if self.stream.diagram.links.iter().any(|l| {
                    l.from == NodeRef::Uid(uid.clone()) || l.to == NodeRef::Uid(uid.clone())
                }) {
                    return Err(EditError::StillLinked(uid));
                }
                let node = self.stream.diagram.nodes.remove(index);
                Ok(Edit::AddNode { node, index })
            }
            Edit::Connect { link, index } => {
                self.check_link(&link, None)?;
                if index > self.stream.diagram.links.len() {
                    return Err(EditError::OutOfRange("link index".into()));
                }
                let total = self
                    .stream
                    .all_nodes()
                    .iter()
                    .filter_map(|n| n.subdiagram.as_ref())
                    .fold(self.stream.diagram.links.len(), |n, d| {
                        n.saturating_add(d.links.len())
                    });
                if total >= MAX_LINKS {
                    return Err(EditError::Limit("links"));
                }
                self.stream.diagram.links.insert(index, link.clone());
                Ok(Edit::Disconnect { link })
            }
            Edit::Disconnect { link } => {
                self.endpoint(&link.from)?;
                self.endpoint(&link.to)?;
                let index = self
                    .stream
                    .diagram
                    .links
                    .iter()
                    .position(|l| l == &link)
                    .ok_or_else(|| EditError::OutOfRange("link not found".into()))?;
                self.check_link(&link, Some(index))?;
                self.stream.diagram.links.remove(index);
                Ok(Edit::Connect { link, index })
            }
            Edit::Move { uids, dx, dy } => {
                if uids.len() > MAX_NODES {
                    return Err(EditError::Limit("move targets"));
                }
                let inverse_dx = dx
                    .checked_neg()
                    .ok_or_else(|| EditError::OutOfRange("delta x".into()))?;
                let inverse_dy = dy
                    .checked_neg()
                    .ok_or_else(|| EditError::OutOfRange("delta y".into()))?;
                let indices: HashMap<&str, usize> = self
                    .stream
                    .diagram
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(i, n)| (n.uid.as_str(), i))
                    .collect();
                let mut seen = HashSet::new();
                let mut positions = Vec::new();
                for uid in &uids {
                    if !seen.insert(uid) {
                        return Err(EditError::Duplicate);
                    }
                    let index = match indices.get(uid.as_str()) {
                        Some(&index) => index,
                        None => self.top_index(uid)?,
                    };
                    if let Some((x, y)) = self
                        .stream
                        .diagram
                        .nodes
                        .get(index)
                        .and_then(|n| n.position)
                    {
                        check_coordinate(x)?;
                        check_coordinate(y)?;
                        let x = x
                            .checked_add(dx)
                            .ok_or_else(|| EditError::OutOfRange("x overflow".into()))?;
                        let y = y
                            .checked_add(dy)
                            .ok_or_else(|| EditError::OutOfRange("y overflow".into()))?;
                        check_coordinate(x)?;
                        check_coordinate(y)?;
                        positions.push((index, (x, y)));
                    }
                }
                for (index, position) in positions {
                    if let Some(n) = self.stream.diagram.nodes.get_mut(index) {
                        n.position = Some(position);
                    }
                }
                Ok(Edit::Move {
                    uids,
                    dx: inverse_dx,
                    dy: inverse_dy,
                })
            }
            Edit::Rename { uid, label } => {
                let index = self.top_index(&uid)?;
                check_label(label.as_deref())?;
                let node = self
                    .stream
                    .diagram
                    .nodes
                    .get_mut(index)
                    .ok_or_else(|| EditError::UnknownNode(uid.clone()))?;
                Ok(Edit::Rename {
                    uid,
                    label: std::mem::replace(&mut node.label, label),
                })
            }
            Edit::SetSettings { uid, settings } => {
                let index = self.top_index(&uid)?;
                // Bound both sides: an inverse must remain a valid, exact restoration.
                for root in settings.iter().chain(
                    self.stream
                        .diagram
                        .nodes
                        .get(index)
                        .and_then(|n| n.settings.as_ref()),
                ) {
                    let mut pending = vec![(root, 1usize)];
                    let mut count = 0usize;
                    while let Some((element, depth)) = pending.pop() {
                        count = count
                            .saturating_add(element.children.len())
                            .saturating_add(element.attributes.len())
                            .saturating_add(element.namespace_declarations.len());
                        if depth > 512 || count > 100_000 {
                            return Err(EditError::Limit("settings depth or item count"));
                        }
                        pending.extend(
                            element
                                .child_elements()
                                .map(|e| (e, depth.saturating_add(1))),
                        );
                    }
                }
                let node = self
                    .stream
                    .diagram
                    .nodes
                    .get_mut(index)
                    .ok_or_else(|| EditError::UnknownNode(uid.clone()))?;
                Ok(Edit::SetSettings {
                    uid,
                    settings: std::mem::replace(&mut node.settings, settings),
                })
            }
            Edit::SetTerminalOrder { order } => {
                if order.len() > MAX_NODES {
                    return Err(EditError::Limit("terminal order"));
                }
                // Preserve unresolved/imported orders, including dangling entries, for exact undo.
                // A reference resolving inside a supernode is still an attempted inner edit.
                let top: HashSet<&str> = self
                    .stream
                    .diagram
                    .nodes
                    .iter()
                    .map(|n| n.uid.as_str())
                    .collect();
                let all: HashSet<&str> = self
                    .stream
                    .all_nodes()
                    .into_iter()
                    .map(|n| n.uid.as_str())
                    .collect();
                for reference in order.iter().chain(&self.stream.diagram.terminal_order) {
                    if let NodeRef::Uid(uid) = reference
                        && !top.contains(uid.as_str())
                        && all.contains(uid.as_str())
                    {
                        return Err(EditError::NotTopLevel(uid.clone()));
                    }
                }
                Ok(Edit::SetTerminalOrder {
                    order: std::mem::replace(&mut self.stream.diagram.terminal_order, order),
                })
            }
            Edit::PutAttachment { name, bytes } => {
                if let Some(bytes) = &bytes {
                    if bytes.len() > MAX_ATTACHMENT {
                        return Err(EditError::Limit("attachment size"));
                    }
                    if !self.attachments.contains_key(&name)
                        && self.attachments.len() >= MAX_ATTACHMENTS
                    {
                        return Err(EditError::Limit("attachment count"));
                    }
                    let total = self
                        .attachments
                        .iter()
                        .filter(|(key, _)| *key != &name)
                        .fold(bytes.len(), |total, (_, b)| total.saturating_add(b.len()));
                    if total > MAX_BYTES {
                        return Err(EditError::Limit("total attachment size"));
                    }
                }
                let previous = match bytes {
                    Some(b) => self.attachments.insert(name.clone(), b),
                    None => self.attachments.remove(&name),
                };
                Ok(Edit::PutAttachment {
                    name,
                    bytes: previous,
                })
            }
        }
    }
    pub fn apply_all(&mut self, edits: Vec<Edit>) -> Result<Vec<Edit>, EditError> {
        if edits.len() > MAX_LINKS {
            return Err(EditError::Limit("edit batch"));
        }
        // Work on a candidate so even an invalid imported graph cannot make rollback fail.
        let mut candidate = crate::native::bounded_stack(|| Ok(self.clone()))
            .map_err(|_| EditError::Limit("document clone"))?;
        let mut inverses = Vec::new();
        for edit in edits {
            inverses.push(candidate.apply(edit)?);
        }
        inverses.reverse();
        *self = candidate;
        Ok(inverses)
    }
}
fn check_coordinate(n: i64) -> Result<(), EditError> {
    if !(-1_000_000..=1_000_000).contains(&n) {
        return Err(EditError::OutOfRange(
            "coordinate exceeds +/-1000000".into(),
        ));
    }
    Ok(())
}
fn check_label(label: Option<&str>) -> Result<(), EditError> {
    if label.is_some_and(|l| l.chars().take(MAX_LABEL + 1).count() > MAX_LABEL) {
        return Err(EditError::Limit("label characters"));
    }
    Ok(())
}
/// Randomness is injectable. After one random starting point, collision handling enumerates
/// the finite namespace, so even a constant RNG does not get stuck on the same uid.
pub fn new_uid(existing: &dyn Fn(&str) -> bool, rng: &mut dyn FnMut() -> u64) -> String {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut digits = [0u8; 11];
    for digit in &mut digits {
        *digit = (rng() % 36) as u8;
    }
    let mut collisions = 0usize;
    loop {
        let mut uid = String::from("id");
        for digit in &digits {
            if let Some(&c) = alphabet.get(usize::from(*digit)) {
                uid.push(char::from(c));
            }
        }
        // A valid document has at most MAX_NODES uids, so MAX_NODES + 1 distinct
        // candidates always include a free one. This also bounds a broken predicate.
        if !existing(&uid) || collisions >= MAX_NODES {
            return uid;
        }
        collisions = collisions.saturating_add(1);
        for digit in digits.iter_mut().rev() {
            *digit = digit.saturating_add(1);
            if *digit < 36 {
                break;
            }
            *digit = 0;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn node(uid: &str) -> StreamNode {
        StreamNode {
            uid: uid.into(),
            class: "SyntheticNode".into(),
            extension: None,
            model_builder: None,
            name: None,
            label: None,
            position: Some((0, 0)),
            properties_attachment: None,
            containers: Vec::new(),
            settings: None,
            binning_output: None,
            result_items: Vec::new(),
            subdiagram: None,
        }
    }
    fn link(a: &str, b: &str) -> Link {
        Link {
            from: NodeRef::Uid(a.into()),
            to: NodeRef::Uid(b.into()),
        }
    }
    fn snapshot(doc: &StreamDoc) -> String {
        format!("{doc:?}")
    }
    fn random(seed: &mut u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    }
    #[test]
    fn random_edit_sequences_restore_every_field_and_list_position() {
        // Deterministic property test: 128 generated histories of 80 mixed edits.
        let mut seed = 0xd05c_1234_9876_abcd;
        for round in 0..128 {
            let mut doc = StreamDoc::new();
            for i in 0..8 {
                let mut n = node(&format!("n{i}"));
                if i == 0 {
                    n.position = None;
                }
                doc.stream.diagram.nodes.push(n);
            }
            doc.meta.imported_from = Some("synthetic.str".into());
            doc.meta
                .extra
                .insert("future".into(), toml::Value::String("retained".into()));
            let before = snapshot(&doc);
            let mut inverses = Vec::new();
            for step in 0..80 {
                let r = random(&mut seed);
                let uid = format!("n{}", r % 8);
                let edit = match r % 9 {
                    0 => Edit::AddNode {
                        node: node(&format!("added{round}_{step}")),
                        index: (r as usize) % (doc.stream.diagram.nodes.len() + 1),
                    },
                    1 => Edit::RemoveNode { uid },
                    2 => Edit::Connect {
                        link: link(&uid, &format!("n{}", (r >> 8) % 8)),
                        index: (r as usize) % (doc.stream.diagram.links.len() + 1),
                    },
                    3 => Edit::Disconnect {
                        link: doc
                            .stream
                            .diagram
                            .links
                            .first()
                            .cloned()
                            .unwrap_or_else(|| link("n1", "n2")),
                    },
                    4 => Edit::Move {
                        uids: vec![uid],
                        dx: (r % 17) as i64 - 8,
                        dy: (r % 31) as i64 - 15,
                    },
                    5 => Edit::Rename {
                        uid,
                        label: Some(format!("Label {r}")),
                    },
                    6 => Edit::SetTerminalOrder {
                        order: vec![NodeRef::Uid(uid)],
                    },
                    7 => Edit::SetSettings {
                        uid,
                        settings: Some(
                            crate::settings_xml::element_from_xml(&format!(
                                "<properties><value>{r}</value></properties>"
                            ))
                            .unwrap(),
                        ),
                    },
                    _ => Edit::PutAttachment {
                        name: format!("data/{}", r % 3),
                        bytes: (r & 16 != 0).then(|| Arc::from(r.to_le_bytes())),
                    },
                };
                let state = snapshot(&doc);
                match doc.apply(edit.clone()) {
                    Ok(inverse) => {
                        // Check the single-step property too, then replay the original edit.
                        doc.apply(inverse).unwrap();
                        assert_eq!(snapshot(&doc), state);
                        inverses.push(doc.apply(edit).unwrap());
                    }
                    Err(_) => assert_eq!(snapshot(&doc), state),
                }
            }
            for inverse in inverses.into_iter().rev() {
                doc.apply(inverse).unwrap();
            }
            assert_eq!(snapshot(&doc), before, "history {round}");
        }
    }
    #[test]
    fn validations_and_atomic_batch() {
        let mut doc = StreamDoc::new();
        for uid in ["a", "b", "c"] {
            doc.apply(Edit::AddNode {
                node: node(uid),
                index: 0,
            })
            .unwrap();
        }
        doc.apply(Edit::Connect {
            link: link("a", "b"),
            index: 0,
        })
        .unwrap();
        doc.apply(Edit::Connect {
            link: link("b", "c"),
            index: 0,
        })
        .unwrap();
        let before = snapshot(&doc);
        let invalid = [
            Edit::Connect {
                link: link("c", "a"),
                index: 0,
            },
            Edit::Connect {
                link: link("a", "a"),
                index: 0,
            },
            Edit::Connect {
                link: link("a", "b"),
                index: 0,
            },
            Edit::Connect {
                link: link("x", "a"),
                index: 0,
            },
            Edit::RemoveNode { uid: "a".into() },
            Edit::AddNode {
                node: node("a"),
                index: 0,
            },
            Edit::Move {
                uids: vec!["a".into(), "b".into()],
                dx: 1_000_001,
                dy: 0,
            },
            Edit::Move {
                uids: vec!["a".into()],
                dx: i64::MIN,
                dy: 0,
            },
            Edit::Move {
                uids: vec!["a".into(), "a".into()],
                dx: 1,
                dy: 1,
            },
            Edit::Rename {
                uid: "a".into(),
                label: Some("界".repeat(1001)),
            },
        ];
        for edit in invalid {
            assert!(doc.apply(edit).is_err());
            assert_eq!(snapshot(&doc), before);
        }
        assert!(
            doc.apply_all(vec![
                Edit::Rename {
                    uid: "a".into(),
                    label: Some("ok".into())
                },
                Edit::RemoveNode { uid: "a".into() }
            ])
            .is_err()
        );
        assert_eq!(snapshot(&doc), before);
        let undo = doc
            .apply_all(vec![
                Edit::Disconnect {
                    link: link("a", "b"),
                },
                Edit::RemoveNode { uid: "a".into() },
            ])
            .unwrap();
        doc.apply_all(undo).unwrap();
        assert_eq!(snapshot(&doc), before);
        let undo = doc
            .apply(Edit::Move {
                uids: vec!["a".into()],
                dx: 1_000_000,
                dy: -1_000_000,
            })
            .unwrap();
        doc.apply(undo).unwrap();
        assert_eq!(snapshot(&doc), before);
        let mut outer = node("super");
        outer.subdiagram = Some(crate::Diagram {
            nodes: vec![node("inner")],
            ..Default::default()
        });
        doc.apply(Edit::AddNode {
            node: outer,
            index: 0,
        })
        .unwrap();
        assert!(doc.node("inner").is_none());
        for edit in [
            Edit::RemoveNode {
                uid: "inner".into(),
            },
            Edit::Rename {
                uid: "inner".into(),
                label: None,
            },
            Edit::Move {
                uids: vec!["inner".into()],
                dx: 1,
                dy: 1,
            },
            Edit::Connect {
                link: link("a", "inner"),
                index: 0,
            },
        ] {
            assert!(matches!(doc.apply(edit), Err(EditError::NotTopLevel(_))));
        }
    }
    #[test]
    fn malformed_imported_graph_edits_are_rejected_without_losing_undo() {
        let mut d = StreamDoc::new();
        d.stream.diagram.nodes = vec![node("a"), node("b")];
        for links in [
            vec![link("a", "b"), link("a", "b")],
            vec![link("a", "b"), link("b", "a")],
        ] {
            d.stream.diagram.links = links;
            let before = snapshot(&d);
            assert!(
                d.apply(Edit::Disconnect {
                    link: link("a", "b")
                })
                .is_err()
            );
            assert_eq!(snapshot(&d), before);
        }
        d.stream.diagram.links = vec![link("a", "future")];
        let before = snapshot(&d);
        assert!(
            d.apply(Edit::AddNode {
                node: node("future"),
                index: 0
            })
            .is_err()
        );
        assert_eq!(snapshot(&d), before);
        d.stream.diagram.links.clear();
        d.stream.diagram.nodes[0].position = Some((i64::MAX, i64::MIN));
        let before = snapshot(&d);
        let inverse = d.apply(Edit::RemoveNode { uid: "a".into() }).unwrap();
        d.apply(inverse).unwrap();
        assert_eq!(snapshot(&d), before);
    }
    #[test]
    fn full_node_limit_can_move_and_undo_as_one_batch() {
        let mut d = StreamDoc::new();
        d.stream.diagram.nodes = (0..MAX_NODES).map(|i| node(&format!("n{i}"))).collect();
        let uids: Vec<String> = d
            .stream
            .diagram
            .nodes
            .iter()
            .map(|n| n.uid.clone())
            .collect();
        let inverse = d
            .apply(Edit::Move {
                uids: uids.clone(),
                dx: 1_000_000,
                dy: -1_000_000,
            })
            .unwrap();
        assert!(
            d.stream
                .diagram
                .nodes
                .iter()
                .all(|n| n.position == Some((1_000_000, -1_000_000)))
        );
        d.apply(inverse).unwrap();
        assert!(
            d.stream
                .diagram
                .nodes
                .iter()
                .all(|n| n.position == Some((0, 0)))
        );
        let inverse = d
            .apply(Edit::SetTerminalOrder {
                order: uids.into_iter().map(NodeRef::Uid).collect(),
            })
            .unwrap();
        d.apply(inverse).unwrap();
        assert!(d.stream.diagram.terminal_order.is_empty());
        assert!(matches!(
            d.apply(Edit::AddNode {
                node: node("one_too_many"),
                index: 0
            }),
            Err(EditError::Limit("nodes"))
        ));
    }
    #[test]
    fn uid_collision_with_constant_random_source_terminates() {
        let first = new_uid(&|_| false, &mut || 0);
        let second = new_uid(&|s| s == first, &mut || 0);
        assert_ne!(first, second);
        assert_eq!(first.len(), 13);
        assert!(
            second
                .strip_prefix("id")
                .unwrap()
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        );
    }
}

#[cfg(test)]
mod settings_edit_tests {
    use super::*;
    use crate::settings_xml::element_from_xml;
    fn doc() -> StreamDoc {
        let mut doc = StreamDoc::new();
        let node = super::tests::node("n");
        doc.apply(Edit::AddNode { node, index: 0 }).unwrap();
        doc
    }
    #[test]
    fn settings_restore_absence_and_unknown_infoset_exactly() {
        let mut doc = doc();
        let settings=Some(element_from_xml("<properties><future q='keep'><!--keep--><?pi text?></future><value>old</value></properties>").unwrap());
        let inverse = doc
            .apply(Edit::SetSettings {
                uid: "n".into(),
                settings: settings.clone(),
            })
            .unwrap();
        assert_eq!(doc.node("n").unwrap().settings, settings);
        doc.apply(inverse).unwrap();
        assert!(doc.node("n").unwrap().settings.is_none());
        doc.apply(Edit::SetSettings {
            uid: "n".into(),
            settings: settings.clone(),
        })
        .unwrap();
        let undo = doc
            .apply(Edit::SetSettings {
                uid: "n".into(),
                settings: None,
            })
            .unwrap();
        doc.apply(undo).unwrap();
        assert_eq!(doc.node("n").unwrap().settings, settings);
    }
    #[test]
    fn oversized_unknown_and_inner_settings_edits_are_atomic() {
        let mut doc = doc();
        let mut root = element_from_xml("<properties/>").unwrap();
        root.children = vec![dms_strfmt::fastinfoset::Node::Comment("x".into()); 100_001];
        assert!(matches!(
            doc.apply(Edit::SetSettings {
                uid: "n".into(),
                settings: Some(root)
            }),
            Err(EditError::Limit(_))
        ));
        assert!(doc.node("n").unwrap().settings.is_none());
        assert!(matches!(
            doc.apply(Edit::SetSettings {
                uid: "unknown".into(),
                settings: None
            }),
            Err(EditError::UnknownNode(_))
        ));
        let mut parent = super::tests::node("parent");
        parent.subdiagram = Some(crate::Diagram {
            nodes: vec![super::tests::node("inner")],
            ..Default::default()
        });
        doc.apply(Edit::AddNode {
            node: parent,
            index: 1,
        })
        .unwrap();
        assert!(matches!(
            doc.apply(Edit::SetSettings {
                uid: "inner".into(),
                settings: None
            }),
            Err(EditError::NotTopLevel(_))
        ));
    }
}
