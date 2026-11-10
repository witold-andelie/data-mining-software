//! Editable document lifecycle and structural command transactions.
use crate::{
    autosave, clipboard, nodes,
    session::{Arg, CommandError, Document, Outcome, Session},
    undo::{Transaction, UndoStack},
};
use dms_stream::{DocMeta, Edit, Extension, Link, NodeRef, StreamDoc, StreamNode, native, new_uid};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug)]
pub struct DocumentState {
    pub doc: StreamDoc,
    pub native_path: Option<PathBuf>,
    pub imported_from: Option<PathBuf>,
    pub selection: Vec<String>,
    pub(crate) revision: u64,
    saved_revision: Option<u64>,
    next_revision: u64,
    undo: UndoStack,
    pub(crate) id: String,
    pub(crate) recovery_path: Option<PathBuf>,
    pub(crate) autosave_path: Option<PathBuf>,
}

// No external RNG dependency: time/process salt plus a monotonic counter seed the injectable
// WP1 uid generator. Collision resolution is performed against all document uids.
static NONCE: AtomicU64 = AtomicU64::new(1);
pub(crate) fn random_word() -> u64 {
    let tick = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let time = u64::try_from(tick & u128::from(u64::MAX)).unwrap_or_default();
    let mut value = time
        ^ NONCE
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9e3779b97f4a7c15)
        ^ u64::from(std::process::id());
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

impl DocumentState {
    pub(crate) fn new(
        doc: StreamDoc,
        native_path: Option<PathBuf>,
        imported_from: Option<PathBuf>,
        saved: bool,
    ) -> Self {
        Self {
            doc,
            native_path,
            imported_from,
            selection: Vec::new(),
            revision: 0,
            saved_revision: saved.then_some(0),
            next_revision: 0,
            undo: UndoStack::default(),
            id: new_uid(&|_| false, &mut random_word),
            recovery_path: None,
            autosave_path: None,
        }
    }
    pub fn is_dirty(&self) -> bool {
        self.saved_revision != Some(self.revision)
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.undo_label()
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.undo.redo_label()
    }

    fn commit(
        &mut self,
        label: &'static str,
        edits: Vec<Edit>,
        selection: Vec<String>,
        meta: Option<DocMeta>,
    ) -> Result<(), String> {
        if edits.is_empty() {
            return Ok(());
        }
        let revision = self
            .next_revision
            .checked_add(1)
            .ok_or("Revision counter exhausted")?;
        let inverses = self.doc.apply_all(edits).map_err(|e| e.to_string())?;
        let old_meta = match meta {
            Some(meta) => std::mem::replace(&mut self.doc.meta, meta),
            None => self.doc.meta.clone(),
        };
        self.undo.push(Transaction {
            label,
            edits: inverses,
            revision: self.revision,
            selection: std::mem::replace(&mut self.selection, selection),
            meta: old_meta,
        });
        self.next_revision = revision;
        self.revision = revision;
        Ok(())
    }
    fn travel(&mut self, redo: bool) -> Result<(), String> {
        let source = if redo {
            &mut self.undo.redo
        } else {
            &mut self.undo.undo
        };
        let Some(target) = source.back() else {
            return Err("No transaction to restore".into());
        };
        let inverse = self
            .doc
            .apply_all(target.edits.clone())
            .map_err(|e| e.to_string())?;
        if let Some(target) = source.pop_back() {
            let inverse = Transaction {
                label: target.label,
                edits: inverse,
                revision: std::mem::replace(&mut self.revision, target.revision),
                selection: std::mem::replace(&mut self.selection, target.selection),
                meta: std::mem::replace(&mut self.doc.meta, target.meta),
            };
            if redo {
                self.undo.undo.push_back(inverse);
            } else {
                self.undo.redo.push_back(inverse);
            }
        }
        Ok(())
    }
    fn select(&mut self, uids: &[String]) -> Result<(), String> {
        if uids.len() > 100_000 {
            return Err("Selection exceeds 100000 nodes".into());
        }
        let top: HashSet<_> = self
            .doc
            .stream
            .diagram
            .nodes
            .iter()
            .map(|n| n.uid.as_str())
            .collect();
        let mut seen = HashSet::new();
        for uid in uids {
            if !top.contains(uid.as_str()) {
                return Err(format!("Node {uid} is not on the top-level canvas"));
            }
            if !seen.insert(uid) {
                return Err("Duplicate selection uid".into());
            }
        }
        self.selection = uids.to_vec();
        Ok(())
    }
    fn delete(&mut self, label: &'static str) -> Result<(), String> {
        let selected: HashSet<_> = self.selection.iter().map(String::as_str).collect();
        if selected.is_empty() {
            return Ok(());
        }
        let mut edits = Vec::new();
        for link in &self.doc.stream.diagram.links {
            if [&link.from, &link.to]
                .iter()
                .any(|r| matches!(r, NodeRef::Uid(uid) if selected.contains(uid.as_str())))
            {
                edits.push(Edit::Disconnect { link: link.clone() });
            }
        }
        let order = self
            .doc
            .stream
            .diagram
            .terminal_order
            .iter()
            .filter(|r| !matches!(r, NodeRef::Uid(uid) if selected.contains(uid.as_str())))
            .cloned()
            .collect();
        edits.push(Edit::SetTerminalOrder { order });
        edits.extend(
            self.selection
                .iter()
                .map(|uid| Edit::RemoveNode { uid: uid.clone() }),
        );
        self.commit(label, edits, Vec::new(), None)
    }
}

pub(crate) fn read_state(path: &Path) -> Result<DocumentState, String> {
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("dms"))
    {
        let doc = native::load(path).map_err(|e| e.to_string())?;
        let imported = doc.meta.imported_from.as_ref().map(PathBuf::from);
        return Ok(DocumentState::new(
            doc,
            Some(path.to_path_buf()),
            imported,
            true,
        ));
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    const LIMIT: u64 = 1 << 30;
    if file.metadata().map_err(|e| e.to_string())?.len() > LIMIT {
        return Err("Stream exceeds 1 GiB".into());
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let doc = StreamDoc::import_str(&bytes, &path.to_string_lossy()).map_err(|e| e.to_string())?;
    Ok(DocumentState::new(
        doc,
        None,
        Some(path.to_path_buf()),
        false,
    ))
}

pub(crate) fn document_view(state: &DocumentState) -> Document {
    Document {
        path: state
            .native_path
            .as_ref()
            .or(state.imported_from.as_ref())
            .cloned()
            .unwrap_or_else(|| PathBuf::from("Untitled.dms")),
        stream: state.doc.stream.clone(),
        attachments: Arc::new(
            state
                .doc
                .attachments
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<HashMap<_, _>>(),
        ),
    }
}

fn failure(id: &'static str, reason: String) -> CommandError {
    CommandError::Failed { id, reason }
}
fn bad(id: &'static str, expected: &'static str) -> CommandError {
    CommandError::BadArgument { id, expected }
}

impl Session {
    pub fn document_state(&self) -> Option<&DocumentState> {
        self.document_state.as_ref()
    }
    pub fn is_dirty(&self) -> bool {
        self.document_state
            .as_ref()
            .is_some_and(DocumentState::is_dirty)
    }
    pub fn revision(&self) -> u64 {
        self.document_state
            .as_ref()
            .map_or(0, DocumentState::revision)
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.document_state
            .as_ref()
            .and_then(DocumentState::undo_label)
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.document_state
            .as_ref()
            .and_then(DocumentState::redo_label)
    }
    pub fn results_stale(&self) -> bool {
        self.results
            .as_ref()
            .is_some_and(|r| r.revision != self.revision())
    }
    /// Dynamic source label for interfaces; the static registry retains the base label.
    pub fn command_label(&self, id: &str) -> Option<String> {
        let spec = crate::command::by_id(id)?;
        let label = match id {
            "edit.undo" => self.undo_label(),
            "edit.redo" => self.redo_label(),
            _ => None,
        };
        Some(label.map_or_else(
            || spec.label.to_string(),
            |label| format!("{} {label}", spec.label),
        ))
    }
    pub(crate) fn sync_document(&mut self) {
        self.document = self.document_state.as_ref().map(document_view);
    }
    fn replace_document(&mut self, state: Option<DocumentState>) -> Result<(), String> {
        self.remove_autosave()?;
        self.cancel_run();
        self.document_state = state;
        self.results = None;
        self.result_inputs.clear();
        self.autosave.last = Some(Instant::now());
        self.sync_document();
        Ok(())
    }
    pub(crate) fn execute_document_command(
        &mut self,
        id: &'static str,
        args: &[Arg],
    ) -> Result<Outcome, CommandError> {
        let force = args.iter().any(|a| matches!(a, Arg::Force));
        if matches!(
            id,
            "file.new" | "file.open" | "file.close" | "app.quit" | "file.recover"
        ) && self.is_dirty()
            && !force
        {
            return Ok(Outcome::ConfirmDiscard);
        }
        match id {
            "file.new" => {
                self.replace_document(Some(DocumentState::new(
                    StreamDoc::new(),
                    None,
                    None,
                    false,
                )))
                .map_err(|e| failure(id, e))?;
            }
            "file.open" => {
                let Some(path) = args
                    .iter()
                    .find_map(|a| if let Arg::Path(p) = a { Some(p) } else { None })
                else {
                    return Ok(Outcome::NeedsArgument("path"));
                };
                let state = read_state(path).map_err(|reason| CommandError::Open {
                    path: path.clone(),
                    reason,
                })?;
                self.replace_document(Some(state))
                    .map_err(|e| failure(id, e))?;
            }
            "file.recover" => {
                let Some(path) = args
                    .iter()
                    .find_map(|a| if let Arg::Path(p) = a { Some(p) } else { None })
                else {
                    return Ok(Outcome::NeedsArgument("path"));
                };
                let (doc, original) = autosave::load_recovery(path).map_err(|e| failure(id, e))?;
                let mut state = DocumentState::new(doc, None, original, false);
                // Only application-owned files in the configured directory are removed on
                // save/close; an arbitrary recovery path is read-only input.
                if self.recoverable().iter().any(|r| &r.path == path) {
                    state.recovery_path = Some(path.clone());
                }
                self.replace_document(Some(state))
                    .map_err(|e| failure(id, e))?;
            }
            "file.close" | "app.quit" => {
                self.replace_document(None).map_err(|e| failure(id, e))?;
                if id == "app.quit" {
                    return Ok(Outcome::Quit);
                }
            }
            "file.save" | "file.save_as" => {
                let Some(state) = self.document_state.as_mut() else {
                    return Err(bad(id, "an open document"));
                };
                let path = if id == "file.save" {
                    let Some(path) = state.native_path.clone() else {
                        return Ok(Outcome::NeedsArgument("save_path"));
                    };
                    path
                } else {
                    let Some(path) = args
                        .iter()
                        .find_map(|a| if let Arg::Path(p) = a { Some(p) } else { None })
                    else {
                        return Ok(Outcome::NeedsArgument("save_path"));
                    };
                    let mut path = path.clone();
                    if path.extension().is_none_or(|ext| ext != "dms") {
                        let mut name = path.as_os_str().to_os_string();
                        name.push(".dms");
                        path = PathBuf::from(name);
                    }
                    path
                };
                native::save_atomic(&state.doc, &path).map_err(|e| failure(id, e.to_string()))?;
                state.native_path = Some(path);
                state.saved_revision = Some(state.revision);
                self.sync_document();
                self.remove_autosave().map_err(|e| failure(id, e))?;
            }
            _ => return self.execute_edit_command(id, args),
        }
        Ok(Outcome::Done)
    }

    fn execute_edit_command(
        &mut self,
        id: &'static str,
        args: &[Arg],
    ) -> Result<Outcome, CommandError> {
        if id == "node.set_param" {
            return self.set_param_command(args);
        }
        let Some(state) = self.document_state.as_mut() else {
            return Err(bad(id, "an open document"));
        };
        match id {
            "edit.undo" | "edit.redo" => state.travel(id == "edit.redo"),
            "edit.select" => {
                let Some(uids) = args.iter().find_map(|a| {
                    if let Arg::Nodes(uids) = a {
                        Some(uids)
                    } else {
                        None
                    }
                }) else {
                    return Ok(Outcome::NeedsArgument("nodes"));
                };
                state.select(uids).map_err(|e| failure(id, e))?;
                return Ok(Outcome::Done);
            }
            "edit.select_all" => {
                state.selection = state
                    .doc
                    .stream
                    .diagram
                    .nodes
                    .iter()
                    .map(|n| n.uid.clone())
                    .collect();
                return Ok(Outcome::Done);
            }
            "edit.copy" | "edit.cut" => {
                let text =
                    clipboard::encode(&state.doc, &state.selection).map_err(|e| failure(id, e))?;
                if id == "edit.cut" {
                    state.delete("Cut").map_err(|e| failure(id, e))?;
                    self.sync_document();
                }
                return Ok(Outcome::Clipboard(text));
            }
            "edit.paste" | "edit.duplicate" => {
                let text = if id == "edit.duplicate" {
                    clipboard::encode(&state.doc, &state.selection).map_err(|e| failure(id, e))?
                } else {
                    let Some(text) = args
                        .iter()
                        .find_map(|a| if let Arg::Text(s) = a { Some(s) } else { None })
                    else {
                        return Ok(Outcome::NeedsArgument("text"));
                    };
                    if text.len() > clipboard::MAX_BYTES {
                        return Err(bad(id, "clipboard text up to 16 MiB"));
                    }
                    text.clone()
                };
                let position = if id == "edit.paste" {
                    args.iter().find_map(|a| {
                        if let Arg::Position(x, y) = a {
                            Some((*x, *y))
                        } else {
                            None
                        }
                    })
                } else {
                    None
                };
                let paste = clipboard::paste(&state.doc, &text, position, &mut random_word)
                    .map_err(|e| failure(id, e))?;
                state.commit(
                    if id == "edit.paste" {
                        "Paste"
                    } else {
                        "Duplicate"
                    },
                    paste.edits,
                    paste.selection,
                    Some(paste.meta),
                )
            }
            "edit.delete" => state.delete("Delete"),
            "node.add" => {
                let Some(kind) = args
                    .iter()
                    .find_map(|a| if let Arg::Kind(k) = a { Some(k) } else { None })
                else {
                    return Ok(Outcome::NeedsArgument("kind"));
                };
                let Some(position) = args.iter().find_map(|a| {
                    if let Arg::Position(x, y) = a {
                        Some((*x, *y))
                    } else {
                        None
                    }
                }) else {
                    return Ok(Outcome::NeedsArgument("position"));
                };
                let spec = crate::catalog::NODES
                    .iter()
                    .find(|s| s.id == kind && nodes::is_implemented(kind))
                    .ok_or_else(|| bad(id, "an implemented catalog kind"))?;
                if !(-1_000_000..=1_000_000).contains(&position.0)
                    || !(-1_000_000..=1_000_000).contains(&position.1)
                {
                    return Err(bad(id, "position within +/-1000000"));
                }
                let identifier = spec
                    .modeler
                    .first()
                    .ok_or_else(|| bad(id, "a mapped catalog kind"))?;
                let (class, extension) = match identifier.split_once('!') {
                    Some((object, extension)) => (
                        String::new(),
                        Some(Extension::Object {
                            object_id: object.into(),
                            extension_id: extension.into(),
                        }),
                    ),
                    None => (identifier.to_string(), None),
                };
                let used: HashSet<_> = state
                    .doc
                    .stream
                    .all_nodes()
                    .iter()
                    .map(|n| n.uid.as_str())
                    .collect();
                let uid = new_uid(&|s| used.contains(s), &mut random_word);
                let node = StreamNode {
                    uid: uid.clone(),
                    class,
                    extension,
                    model_builder: None,
                    name: None,
                    label: None,
                    position: Some(position),
                    properties_attachment: None,
                    containers: Vec::new(),
                    settings: None,
                    binning_output: None,
                    result_items: Vec::new(),
                    subdiagram: None,
                };
                state.commit(
                    "Add Node",
                    vec![Edit::AddNode {
                        node,
                        index: state.doc.stream.diagram.nodes.len(),
                    }],
                    vec![uid],
                    None,
                )
            }
            "node.move" => {
                let Some(uids) = args.iter().find_map(|a| {
                    if let Arg::Nodes(uids) = a {
                        Some(uids)
                    } else {
                        None
                    }
                }) else {
                    return Ok(Outcome::NeedsArgument("nodes"));
                };
                let Some((dx, dy)) = args.iter().find_map(|a| {
                    if let Arg::Delta(x, y) = a {
                        Some((*x, *y))
                    } else {
                        None
                    }
                }) else {
                    return Ok(Outcome::NeedsArgument("delta"));
                };
                if uids.len() > 100_000 {
                    return Err(bad(id, "at most 100000 move targets"));
                }
                state.commit(
                    "Move",
                    vec![Edit::Move {
                        uids: uids.clone(),
                        dx,
                        dy,
                    }],
                    state.selection.clone(),
                    None,
                )
            }
            "node.connect" | "node.disconnect" => {
                let Some((a, b)) = args.iter().find_map(|a| {
                    if let Arg::Link(a, b) = a {
                        Some((a, b))
                    } else {
                        None
                    }
                }) else {
                    return Ok(Outcome::NeedsArgument("link"));
                };
                let link = Link {
                    from: NodeRef::Uid(a.clone()),
                    to: NodeRef::Uid(b.clone()),
                };
                let (label, edit) = if id == "node.connect" {
                    (
                        "Connect",
                        Edit::Connect {
                            link,
                            index: state.doc.stream.diagram.links.len(),
                        },
                    )
                } else {
                    ("Disconnect", Edit::Disconnect { link })
                };
                state.commit(label, vec![edit], state.selection.clone(), None)
            }
            "node.rename" => {
                let Some(uid) = args
                    .iter()
                    .find_map(|a| if let Arg::Node(s) = a { Some(s) } else { None })
                else {
                    return Ok(Outcome::NeedsArgument("node"));
                };
                let Some(text) = args
                    .iter()
                    .find_map(|a| if let Arg::Text(s) = a { Some(s) } else { None })
                else {
                    return Ok(Outcome::NeedsArgument("text"));
                };
                if text.chars().take(1001).count() > 1000 {
                    return Err(bad(id, "a label up to 1000 characters"));
                }
                state.commit(
                    "Rename",
                    vec![Edit::Rename {
                        uid: uid.clone(),
                        label: (!text.is_empty()).then(|| text.clone()),
                    }],
                    state.selection.clone(),
                    None,
                )
            }
            _ => return Err(bad(id, "an editing command")),
        }
        .map_err(|e| failure(id, e))?;
        self.sync_document();
        Ok(Outcome::Done)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::run::tests::{node, stream};
    use std::{fs, time::Duration};

    pub(crate) struct TempDir(pub PathBuf);
    impl TempDir {
        pub(crate) fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "dms-wp2-{}-{}",
                std::process::id(),
                NONCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) fn session() -> Session {
        let mut a = node("a", "SelectNode");
        a.position = Some((10, 20));
        let mut b = node("b", "TableNode");
        b.position = Some((80, 90));
        let mut doc = StreamDoc::new();
        doc.stream = stream(vec![a, b], &[("a", "b")], &["b"]);
        let mut session = Session::new();
        session
            .replace_document(Some(DocumentState::new(
                doc,
                None,
                Some("example.str".into()),
                true,
            )))
            .unwrap();
        session.execute("edit.select_all", &[]).unwrap();
        session
    }
    pub(crate) fn text(session: &Session) -> String {
        native::to_toml(&session.document_state().unwrap().doc).unwrap()
    }
    fn copied(session: &mut Session) -> String {
        match session.execute("edit.copy", &[]).unwrap() {
            Outcome::Clipboard(text) => text,
            other => panic!("{other:?}"),
        }
    }
    fn rename(session: &mut Session, label: &str) {
        session
            .execute(
                "node.rename",
                &[Arg::Text(label.into()), Arg::Node("a".into())],
            )
            .unwrap();
    }

    #[test]
    fn every_structural_command_is_one_exactly_reversible_transaction() {
        let clipboard = copied(&mut session());
        let cases = vec![
            (
                "node.add",
                vec![Arg::Kind("select".into()), Arg::Position(100, 200)],
            ),
            (
                "node.move",
                vec![Arg::Nodes(vec!["a".into(), "b".into()]), Arg::Delta(-5, 25)],
            ),
            (
                "node.rename",
                vec![Arg::Node("a".into()), Arg::Text("New label".into())],
            ),
            ("node.disconnect", vec![Arg::Link("a".into(), "b".into())]),
            ("node.connect", vec![Arg::Link("a".into(), "b".into())]),
            ("edit.delete", vec![]),
            ("edit.cut", vec![]),
            ("edit.duplicate", vec![]),
            (
                "edit.paste",
                vec![Arg::Text(clipboard), Arg::Position(-100, -200)],
            ),
        ];
        for (id, args) in cases {
            let mut session = session();
            if id == "node.connect" {
                session
                    .execute("node.disconnect", &[Arg::Link("a".into(), "b".into())])
                    .unwrap();
            }
            let before = text(&session);
            let selection = session.document_state().unwrap().selection.clone();
            let revision = session.revision();
            session.execute(id, &args).unwrap();
            let after = text(&session);
            assert_ne!(after, before, "{id}");
            assert!(session.revision() > revision);
            assert!(session.undo_label().is_some());
            assert_eq!(
                session.document().unwrap().stream.diagram.nodes.len(),
                session
                    .document_state()
                    .unwrap()
                    .doc
                    .stream
                    .diagram
                    .nodes
                    .len()
            );
            session.execute("edit.undo", &[]).unwrap();
            assert_eq!(text(&session), before, "undo {id}");
            assert_eq!(session.revision(), revision);
            assert_eq!(session.document_state().unwrap().selection, selection);
            session.execute("edit.redo", &[]).unwrap();
            assert_eq!(text(&session), after, "redo {id}");
            session.execute("edit.undo", &[]).unwrap();
            assert_eq!(text(&session), before, "second undo {id}");
        }
    }

    #[test]
    fn selection_copy_and_empty_label_do_not_change_unrelated_state() {
        let mut session = session();
        let before = text(&session);
        let revision = session.revision();
        session
            .execute("edit.select", &[Arg::Nodes(vec!["a".into()])])
            .unwrap();
        let clipboard = copied(&mut session);
        assert_eq!(
            clipboard::decode(&clipboard)
                .unwrap()
                .stream
                .diagram
                .nodes
                .len(),
            1
        );
        assert_eq!(text(&session), before);
        assert_eq!(session.revision(), revision);
        assert_eq!(session.undo_label(), None);
        rename(&mut session, "custom");
        rename(&mut session, "");
        assert_eq!(
            session
                .document_state()
                .unwrap()
                .doc
                .node("a")
                .unwrap()
                .label,
            None
        );
        session
            .execute("edit.select", &[Arg::Nodes(vec![])])
            .unwrap();
        for id in ["edit.cut", "edit.copy", "edit.delete", "edit.duplicate"] {
            assert!(!session.is_enabled(id));
        }
        session.execute("edit.select_all", &[]).unwrap();
        assert_eq!(session.document_state().unwrap().selection, ["a", "b"]);
    }

    #[test]
    fn save_open_and_undo_track_saved_content_across_branches() {
        let dir = TempDir::new();
        let mut session = session();
        session
            .execute("file.save_as", &[Arg::Path(dir.0.join("flow.str"))])
            .unwrap();
        let path = dir.0.join("flow.str.dms");
        assert_eq!(
            session.document_state().unwrap().native_path.as_ref(),
            Some(&path)
        );
        assert!(!session.is_dirty());
        rename(&mut session, "saved");
        assert!(session.is_dirty());
        let saved_revision = session.revision();
        session.execute("file.save", &[]).unwrap();
        let saved_text = text(&session);
        assert!(!session.is_dirty());
        rename(&mut session, "later");
        session.execute("edit.undo", &[]).unwrap();
        assert_eq!(session.revision(), saved_revision);
        assert!(!session.is_dirty());
        session.execute("edit.undo", &[]).unwrap();
        assert!(session.is_dirty());
        rename(&mut session, "branch");
        assert_ne!(session.revision(), saved_revision);
        assert!(session.is_dirty());
        assert_eq!(session.redo_label(), None);
        assert_eq!(
            session.command_label("edit.undo").as_deref(),
            Some("Undo Rename")
        );
        session
            .execute("file.open", &[Arg::Path(path.clone()), Arg::Force])
            .unwrap();
        assert_eq!(text(&session), saved_text);
        assert!(!session.is_dirty());
        assert_eq!(session.undo_label(), None);
        assert_eq!(open_document_for_test(&path), saved_text);
    }
    fn open_document_for_test(path: &Path) -> String {
        let view = crate::session::open_document(path).unwrap();
        native::to_toml(&StreamDoc {
            stream: view.stream,
            attachments: view
                .attachments
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            meta: Default::default(),
        })
        .unwrap()
    }

    #[test]
    fn dirty_lifecycle_requires_discard_and_force_clears_the_document() {
        let dir = TempDir::new();
        let valid = dir.0.join("valid.dms");
        native::save_atomic(&StreamDoc::new(), &valid).unwrap();
        for id in [
            "file.new",
            "file.open",
            "file.close",
            "app.quit",
            "file.recover",
        ] {
            let mut session = session();
            rename(&mut session, "dirty");
            let before = text(&session);
            assert_eq!(
                session.execute(id, &[Arg::Path(valid.clone())]).unwrap(),
                Outcome::ConfirmDiscard,
                "{id}"
            );
            assert_eq!(text(&session), before);
            let result = session
                .execute(id, &[Arg::Force, Arg::Path(valid.clone())])
                .unwrap();
            assert_eq!(
                result,
                if id == "app.quit" {
                    Outcome::Quit
                } else {
                    Outcome::Done
                }
            );
            if matches!(id, "file.close" | "app.quit") {
                assert!(session.document_state().is_none());
            } else {
                assert!(
                    session
                        .document_state()
                        .unwrap()
                        .doc
                        .stream
                        .diagram
                        .nodes
                        .is_empty()
                );
            }
        }
        let mut session = Session::new();
        assert_eq!(session.execute("file.new", &[]).unwrap(), Outcome::Done);
        assert!(session.is_dirty());
        assert_eq!(
            session.execute("file.save", &[]).unwrap(),
            Outcome::NeedsArgument("save_path")
        );
    }

    #[test]
    fn failed_open_and_failed_save_preserve_content_history_and_paths() {
        let dir = TempDir::new();
        let mut session = session();
        rename(&mut session, "important");
        let before = text(&session);
        let revision = session.revision();
        assert!(
            session
                .execute(
                    "file.open",
                    &[Arg::Force, Arg::Path(dir.0.join("missing.dms"))]
                )
                .is_err()
        );
        assert!(
            session
                .execute("file.save_as", &[Arg::Path(dir.0.join("missing/f.dms"))])
                .is_err()
        );
        assert_eq!(text(&session), before);
        assert_eq!(session.revision(), revision);
        assert!(session.is_dirty());
        assert!(session.document_state().unwrap().native_path.is_none());
        assert_eq!(session.undo_label(), Some("Rename"));
    }

    #[test]
    fn clipboard_remaps_uids_links_and_positions_as_one_transaction() {
        let mut session = session();
        let original = text(&session);
        let clipboard = copied(&mut session);
        assert!(clipboard.contains("format = \"dms-clipboard\""));
        session
            .execute(
                "edit.paste",
                &[Arg::Position(500, 600), Arg::Text(clipboard)],
            )
            .unwrap();
        let state = session.document_state().unwrap();
        let ids = &state.selection;
        assert_eq!(ids.len(), 2);
        assert!(
            ids.iter()
                .all(|uid| uid.len() == 13 && uid.starts_with("id") && uid != "a" && uid != "b")
        );
        assert_ne!(ids[0], ids[1]);
        assert_eq!(state.doc.node(&ids[0]).unwrap().position, Some((500, 600)));
        assert_eq!(state.doc.node(&ids[1]).unwrap().position, Some((570, 670)));
        assert_eq!(
            state.doc.stream.diagram.links[1],
            Link {
                from: NodeRef::Uid(ids[0].clone()),
                to: NodeRef::Uid(ids[1].clone())
            }
        );
        session.execute("edit.undo", &[]).unwrap();
        assert_eq!(text(&session), original);
        session.execute("edit.duplicate", &[]).unwrap();
        let state = session.document_state().unwrap();
        assert_eq!(
            state.doc.node(&state.selection[0]).unwrap().position,
            Some((40, 50))
        );
    }

    #[test]
    fn malformed_clipboards_and_numeric_overflow_leave_everything_unchanged() {
        let mut session = session();
        rename(&mut session, "keep history");
        let before = text(&session);
        let revision = session.revision();
        let selection = session.document_state().unwrap().selection.clone();
        let valid = copied(&mut session);
        let bad = vec![
            "x".repeat(clipboard::MAX_BYTES + 1),
            format!(
                "format='dms-clipboard'\nversion=1\nx={}0{}",
                "[".repeat(3000),
                "]".repeat(3000)
            ),
            String::from_utf8_lossy(&[0xff, 0xfe]).into_owned(),
            valid.replace("dms-clipboard", "dms-stream"),
            valid.replace("version = 1", "version = 2"),
            "format='dms-clipboard'\nversion=1\n[[node]]\nuid=5".into(),
            "format='dms-stream'\nversion=1\nx='''\nformat='dms-clipboard'\n'''".into(),
            "format='dms-clipboard'\nversion=1\nx='\u{fffd}'".into(),
        ];
        for bad in bad {
            assert!(session.execute("edit.paste", &[Arg::Text(bad)]).is_err());
            assert_eq!(text(&session), before);
            assert_eq!(session.revision(), revision);
            assert_eq!(session.undo_label(), Some("Rename"));
            assert_eq!(session.document_state().unwrap().selection, selection);
        }
        for args in [
            vec![Arg::Nodes(vec!["a".into()]), Arg::Delta(i64::MAX, 0)],
            vec![Arg::Nodes(vec!["a".into()]), Arg::Delta(i64::MIN, 0)],
            vec![
                Arg::Nodes(vec!["a".into(), "missing".into()]),
                Arg::Delta(10, 10),
            ],
        ] {
            assert!(session.execute("node.move", &args).is_err());
            assert_eq!(text(&session), before);
        }
        assert!(
            session
                .execute(
                    "edit.paste",
                    &[Arg::Text(valid), Arg::Position(i64::MIN, 0)]
                )
                .is_err()
        );
        assert_eq!(text(&session), before);
    }

    #[test]
    fn inner_nodes_are_read_only_and_invalid_graph_edits_are_atomic() {
        let mut session = session();
        let state = session.document_state.as_mut().unwrap();
        state.doc.stream.diagram.nodes[0].subdiagram = Some(dms_stream::Diagram {
            nodes: vec![node("inner", "SelectNode")],
            ..Default::default()
        });
        session.sync_document();
        let before = text(&session);
        for (id, args) in [
            ("edit.select", vec![Arg::Nodes(vec!["inner".into()])]),
            (
                "node.rename",
                vec![Arg::Node("inner".into()), Arg::Text("x".into())],
            ),
            (
                "node.move",
                vec![Arg::Nodes(vec!["inner".into()]), Arg::Delta(1, 1)],
            ),
            ("node.connect", vec![Arg::Link("inner".into(), "a".into())]),
            ("node.connect", vec![Arg::Link("b".into(), "a".into())]),
            ("node.connect", vec![Arg::Link("a".into(), "a".into())]),
            ("node.connect", vec![Arg::Link("a".into(), "b".into())]),
            (
                "node.add",
                vec![Arg::Kind("not_implemented".into()), Arg::Position(0, 0)],
            ),
        ] {
            assert!(session.execute(id, &args).is_err(), "{id}");
            assert_eq!(text(&session), before);
        }
    }

    #[test]
    fn added_extension_nodes_use_the_first_catalog_identifier() {
        let mut session = session();
        let spec = crate::catalog::NODES
            .iter()
            .find(|s| {
                nodes::is_implemented(s.id) && s.modeler.first().is_some_and(|s| s.contains('!'))
            })
            .unwrap();
        session
            .execute(
                "node.add",
                &[Arg::Kind(spec.id.into()), Arg::Position(0, 0)],
            )
            .unwrap();
        let state = session.document_state().unwrap();
        let node = state.doc.node(&state.selection[0]).unwrap();
        assert!(node.class.is_empty());
        assert!(node.settings.is_none());
        assert_eq!(node.type_identifier(), spec.modeler[0]);
    }

    #[test]
    fn undo_history_is_bounded_and_new_edits_clear_redo() {
        let mut session = session();
        for i in 0..505 {
            rename(&mut session, &i.to_string());
        }
        assert_eq!(session.document_state().unwrap().undo.undo.len(), 500);
        for _ in 0..500 {
            session.execute("edit.undo", &[]).unwrap();
        }
        assert!(!session.is_enabled("edit.undo"));
        assert_eq!(session.revision(), 5);
        assert!(
            session
                .command_label("edit.redo")
                .unwrap()
                .starts_with("Redo Rename")
        );
        rename(&mut session, "branch");
        assert!(!session.is_enabled("edit.redo"));
        assert_eq!(session.revision(), 506);
    }

    #[test]
    fn editing_while_running_marks_results_with_the_snapshot_revision() {
        let mut session = session();
        let revision = session.revision();
        session.execute("run.stream", &[]).unwrap();
        rename(&mut session, "after run started");
        let deadline = Instant::now() + Duration::from_secs(10);
        while session.results().is_none() {
            session.poll_jobs();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(session.results().unwrap().revision, revision);
        assert!(session.results_stale());
        session.execute("edit.undo", &[]).unwrap();
        assert!(!session.results_stale());
    }

    #[test]
    fn editing_commands_request_missing_arguments_without_dialogs() {
        let mut session = session();
        for (id, arg) in [
            ("node.add", "kind"),
            ("node.move", "nodes"),
            ("node.connect", "link"),
            ("node.disconnect", "link"),
            ("node.rename", "node"),
            ("edit.select", "nodes"),
            ("edit.paste", "text"),
            ("file.save_as", "save_path"),
        ] {
            assert_eq!(
                session.execute(id, &[]).unwrap(),
                Outcome::NeedsArgument(arg),
                "{id}"
            );
        }
    }
}

// A bounded registry of static command labels keeps undo.rs's existing static-label contract.
fn parameter_label(spec: &dms_exec::schema::ParamSpec) -> &'static str {
    static LABELS: std::sync::OnceLock<HashMap<&'static str, String>> = std::sync::OnceLock::new();
    LABELS
        .get_or_init(|| {
            crate::catalog::NODES
                .iter()
                .filter_map(|n| dms_exec::schema::schema(n.id))
                .flatten()
                .map(|p| (p.label, format!("Change {}", p.label)))
                .collect()
        })
        .get(spec.label)
        .map(String::as_str)
        .unwrap_or(spec.label)
}

impl Session {
    pub fn node_schema(
        &self,
        uid: &str,
    ) -> Option<(&'static str, &'static [dms_exec::schema::ParamSpec])> {
        let node = self.document_state.as_ref()?.doc.node(uid)?;
        let identifier = node.type_identifier();
        // Applied models have different readers and must not receive builder parameters.
        if identifier.starts_with("Apply") || node.model_builder.is_some() {
            return None;
        }
        let catalog = crate::catalog::by_modeler_identifier(&identifier)?;
        Some((catalog.id, dms_exec::schema::schema(catalog.id)?))
    }

    pub fn node_settings(&self, uid: &str) -> Option<&dms_strfmt::fastinfoset::Element> {
        self.document_state
            .as_ref()?
            .doc
            .node(uid)?
            .settings
            .as_ref()
    }

    fn parameter_spec(
        &self,
        uid: &str,
        key: &str,
    ) -> Result<&'static dms_exec::schema::ParamSpec, String> {
        self.node_schema(uid)
            .ok_or_else(|| format!("Node {uid} has no editable schema"))?
            .1
            .iter()
            .find(|p| p.key == key)
            .ok_or_else(|| format!("Unknown parameter {key:?} for node {uid}"))
    }

    fn parameter_attachment(&self, uid: &str) -> Result<(&str, &[u8]), String> {
        let state = self.document_state.as_ref().ok_or("No stream is open")?;
        let node = state
            .doc
            .node(uid)
            .ok_or("Node is not on the top-level canvas")?;
        let name = node
            .properties_attachment
            .as_deref()
            .ok_or("The node has no parameters attachment")?;
        let bytes = state
            .doc
            .attachments
            .get(name)
            .ok_or_else(|| format!("Missing parameters attachment {name}"))?;
        Ok((name, bytes))
    }

    pub fn param_value(
        &self,
        uid: &str,
        key: &str,
    ) -> Result<dms_exec::schema::ParamValue, String> {
        let spec = self.parameter_spec(uid, key)?;
        match spec.source {
            dms_exec::schema::ParamSource::Properties => {
                dms_exec::schema::read(self.node_settings(uid), spec)
            }
            dms_exec::schema::ParamSource::Attachment => {
                let (_, bytes) = self.parameter_attachment(uid)?;
                dms_exec::schema::read_attachment(Some(bytes), spec)
            }
        }
    }

    pub fn param_read_only(&self, uid: &str, key: &str) -> Option<String> {
        let check = || -> Result<(), String> {
            let spec = self.parameter_spec(uid, key)?;
            if spec.source == dms_exec::schema::ParamSource::Attachment {
                let (name, _) = self.parameter_attachment(uid)?;
                let state = self.document_state.as_ref().ok_or("No stream is open")?;
                if state.doc.stream.all_nodes().iter().any(|node| {
                    node.uid != uid
                        && (node.properties_attachment.as_deref() == Some(name)
                            || node.containers.iter().any(|c| c.member == name))
                }) {
                    return Err(format!(
                        "Parameters attachment {name} is shared by another node"
                    ));
                }
            }
            self.param_value(uid, key)?;
            Ok(())
        };
        check().err()
    }

    pub fn input_fields(&self, uid: &str) -> Option<Vec<crate::session::FieldInfo>> {
        let state = self.document_state.as_ref()?;
        state.doc.node(uid)?;
        let report = &self.results.as_ref()?.report;
        let inputs = self.result_inputs.get(uid)?;
        if inputs.is_empty() {
            return None;
        }
        let mut fields = Vec::new();
        let mut seen = HashSet::new();
        for input in inputs {
            let output = report.results.get(input)?.as_ref().ok()?;
            let Some(table) = output.data() else {
                continue;
            };
            for field in table.model().fields() {
                if fields.len() >= 100_000 {
                    return None;
                }
                if seen.insert(field.name.clone()) {
                    let measure = dms_exec::schema::measurement_name(field.measurement).to_string();
                    fields.push(crate::session::FieldInfo {
                        name: field.name.clone(),
                        storage: format!("{:?}", field.storage).to_ascii_lowercase(),
                        measure,
                    });
                }
            }
        }
        (!fields.is_empty()).then_some(fields)
    }

    fn set_param_command(&mut self, args: &[Arg]) -> Result<Outcome, CommandError> {
        use dms_exec::schema::{self, ParamSource};
        const ID: &str = "node.set_param";
        let Some(uid) = args.iter().find_map(|a| {
            if let Arg::Node(uid) = a {
                Some(uid.as_str())
            } else {
                None
            }
        }) else {
            return Ok(Outcome::NeedsArgument("node"));
        };
        let Some((key, value)) = args.iter().find_map(|a| {
            if let Arg::Param(key, value) = a {
                Some((key.as_str(), value))
            } else {
                None
            }
        }) else {
            return Ok(Outcome::NeedsArgument("param"));
        };
        let spec = self.parameter_spec(uid, key).map_err(|e| failure(ID, e))?;
        if let Some(reason) = self.param_read_only(uid, key) {
            return Err(failure(ID, reason));
        }
        schema::validate(spec, value).map_err(|e| failure(ID, e))?;
        let mut edits = Vec::new();
        match spec.source {
            ParamSource::Properties => {
                let old = self.node_settings(uid);
                let new = schema::write(old, spec, value).map_err(|e| failure(ID, e))?;
                if old != Some(&new) && !(old.is_none() && new.children.is_empty()) {
                    edits.push(Edit::SetSettings {
                        uid: uid.into(),
                        settings: Some(new),
                    });
                }
            }
            ParamSource::Attachment => {
                let (name, old) = self.parameter_attachment(uid).map_err(|e| failure(ID, e))?;
                let new = schema::write_attachment(old, spec, value).map_err(|e| failure(ID, e))?;
                if new != old {
                    edits.push(Edit::PutAttachment {
                        name: name.into(),
                        bytes: Some(new.into()),
                    });
                }
            }
        }
        let state = self
            .document_state
            .as_mut()
            .ok_or_else(|| bad(ID, "an open document"))?;
        state
            .commit(parameter_label(spec), edits, state.selection.clone(), None)
            .map_err(|e| failure(ID, e))?;
        self.sync_document();
        Ok(Outcome::Done)
    }
}

#[cfg(test)]
mod settings_tests {
    use super::tests::TempDir;
    use super::*;
    use crate::run::tests::{node, stream};
    use dms_exec::schema::ParamValue;
    use dms_stream::settings_xml::element_from_xml;
    use std::time::{Duration, Instant};

    fn xml(text: &str) -> dms_strfmt::fastinfoset::Element {
        element_from_xml(text).unwrap()
    }
    fn select_session() -> Session {
        let mut n = node("s", "SelectNode");
        n.settings = Some(xml(
            "<properties><selectCondition>x &gt; 1</selectCondition><future keep='yes'/></properties>",
        ));
        Session::with_document(Document::from_stream(
            PathBuf::from("a.dms"),
            stream(vec![n], &[], &[]),
        ))
    }
    fn set(
        session: &mut Session,
        uid: &str,
        key: &str,
        value: ParamValue,
    ) -> Result<Outcome, CommandError> {
        session.execute(
            "node.set_param",
            &[Arg::Node(uid.into()), Arg::Param(key.into(), value)],
        )
    }
    #[test]
    fn property_edit_is_one_named_transaction_and_undo_restores_the_save_revision() {
        let mut s = select_session();
        assert_eq!(s.node_schema("s").unwrap().0, "select");
        assert!(s.input_fields("s").is_none());
        assert!(s.param_read_only("s", "condition").is_none());
        let original = s.node_settings("s").cloned();
        set(&mut s, "s", "condition", ParamValue::Text("x > 2".into())).unwrap();
        assert_eq!(
            s.param_value("s", "condition").unwrap(),
            ParamValue::Text("x > 2".into())
        );
        assert_eq!(
            s.command_label("edit.undo").as_deref(),
            Some("Undo Change Record condition")
        );
        assert!(s.is_dirty());
        assert_eq!(s.revision(), 1);
        s.execute("edit.undo", &[]).unwrap();
        assert!(!s.is_dirty());
        assert_eq!(s.node_settings("s"), original.as_ref());
        assert!(s.undo_label().is_none());
        s.execute("edit.redo", &[]).unwrap();
        assert_eq!(s.revision(), 1);
        let revision = s.revision();
        set(&mut s, "s", "condition", ParamValue::Text("x > 2".into())).unwrap();
        assert_eq!(s.revision(), revision);
        assert!(set(&mut s, "s", "mode", ParamValue::Text("invalid".into())).is_err());
        assert!(set(&mut s, "s", "unknown", ParamValue::Bool(true)).is_err());
        assert_eq!(s.revision(), revision);
    }
    fn attachment_session() -> Session {
        let mut n = node("tree", "CFBuildModelNode");
        n.extension = Some(Extension::Object {
            object_id: "Chaid".into(),
            extension_id: "spss.tree-extension".into(),
        });
        n.properties_attachment = Some("params.xml".into());
        let mut doc = Document::from_stream(PathBuf::from("a.dms"), stream(vec![n], &[], &[]));
        doc.attachments=Arc::new([("params.xml".into(),Arc::from(b"<?xml version='1.0'?><Parameters>\r\n<Parameter name='tree.MaxTreeDepth' value='5'/><!--unknown--><Parameter name='future' value='&amp;'/></Parameters>".as_slice()))].into_iter().collect());
        Session::with_document(doc)
    }
    #[test]
    fn attachment_edit_is_one_undo_step_and_restores_exact_bytes_in_both_views() {
        let mut s = attachment_session();
        let original = s
            .document()
            .unwrap()
            .attachments
            .get("params.xml")
            .unwrap()
            .clone();
        let settings = s.node_settings("tree").cloned();
        set(&mut s, "tree", "max_depth", ParamValue::Int(8)).unwrap();
        assert_eq!(
            s.param_value("tree", "max_depth").unwrap(),
            ParamValue::Int(8)
        );
        assert_ne!(
            s.document().unwrap().attachments.get("params.xml").unwrap(),
            &original
        );
        assert_eq!(s.undo_label(), Some("Change Depth limit"));
        s.execute("edit.undo", &[]).unwrap();
        assert!(s.undo_label().is_none());
        assert!(!s.is_dirty());
        assert_eq!(
            s.document().unwrap().attachments.get("params.xml"),
            Some(&original)
        );
        assert_eq!(
            s.document_state()
                .unwrap()
                .doc
                .attachments
                .get("params.xml"),
            Some(&original)
        );
        assert_eq!(s.node_settings("tree"), settings.as_ref());
        s.execute("edit.redo", &[]).unwrap();
        assert_eq!(
            s.param_value("tree", "max_depth").unwrap(),
            ParamValue::Int(8)
        );
        let tmp = TempDir::new();
        let path = tmp.0.join("saved.dms");
        s.execute("file.save_as", &[Arg::Path(path.clone())])
            .unwrap();
        let reopened = Session::with_document(crate::session::open_document(&path).unwrap());
        assert_eq!(
            reopened.param_value("tree", "max_depth").unwrap(),
            ParamValue::Int(8)
        );
    }
    #[test]
    fn missing_malformed_shared_and_nested_attachments_are_read_only_and_atomic() {
        for mode in 0..6 {
            let mut s = attachment_session();
            let state = s.document_state.as_mut().unwrap();
            match mode {
                0 => state.doc.stream.diagram.nodes[0].properties_attachment = None,
                1 => {
                    state.doc.attachments.remove("params.xml");
                }
                2 => {
                    state
                        .doc
                        .attachments
                        .insert("params.xml".into(), Arc::from(b"broken".as_slice()));
                }
                3 => {
                    let mut other = node("other", "SelectNode");
                    other.properties_attachment = Some("params.xml".into());
                    state.doc.stream.diagram.nodes.push(other);
                }
                4 => {
                    state
                        .doc
                        .attachments
                        .insert("params.xml".into(), Arc::from(b"<Parameters/>".as_slice()));
                }
                _ => {
                    let mut inner = node("inner", "SelectNode");
                    inner.properties_attachment = Some("params.xml".into());
                    let mut parent = node("parent", "SuperNode");
                    parent.subdiagram = Some(dms_stream::Diagram {
                        nodes: vec![inner],
                        ..Default::default()
                    });
                    state.doc.stream.diagram.nodes.push(parent);
                }
            }
            s.sync_document();
            let original = native::to_toml(&s.document_state().unwrap().doc).unwrap();
            assert!(s.param_read_only("tree", "max_depth").is_some());
            assert!(set(&mut s, "tree", "max_depth", ParamValue::Int(9)).is_err());
            assert_eq!(
                native::to_toml(&s.document_state().unwrap().doc).unwrap(),
                original
            );
            assert_eq!(s.revision(), 0);
            assert!(s.undo_label().is_none());
        }
    }
    #[test]
    fn fields_come_from_the_last_run_input_instead_of_the_nodes_own_output() {
        let tmp = TempDir::new();
        let path = tmp.0.join("input.csv");
        std::fs::write(&path, "x\n1\n2\n").unwrap();
        let mut reader = node("reader", "DelimitedFileImportNode");
        reader.settings = Some(
            dms_exec::schema::write(
                None,
                dms_exec::schema::schema("var_file")
                    .unwrap()
                    .iter()
                    .find(|s| s.key == "full_filename")
                    .unwrap(),
                &ParamValue::Text(path.to_string_lossy().into_owned()),
            )
            .unwrap(),
        );
        let mut derive = node("derive", "DeriveNode");
        derive.settings = Some(xml(
            "<properties><deriveSingleFieldName>added</deriveSingleFieldName><deriveFormulaExpression>x * 2</deriveFormulaExpression></properties>",
        ));
        let mut s = Session::with_document(Document::from_stream(
            tmp.0.join("stream.dms"),
            stream(vec![reader, derive], &[("reader", "derive")], &["derive"]),
        ));
        assert!(s.input_fields("derive").is_none());
        s.execute("run.stream", &[]).unwrap();
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            if !s.poll_jobs().is_empty() {
                break;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(s.results().unwrap().report.root_causes.is_empty());
        let fields = s.input_fields("derive").unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "x");
        assert_eq!(fields[0].storage, "integer");
        assert_eq!(fields[0].measure, "range");
        assert!(s.input_fields("reader").is_none());
        set(
            &mut s,
            "derive",
            "formula_expr",
            ParamValue::Text("x * 3".into()),
        )
        .unwrap();
        assert!(s.results_stale());
        assert_eq!(s.input_fields("derive"), Some(fields.clone()));
        s.execute(
            "node.disconnect",
            &[Arg::Link("reader".into(), "derive".into())],
        )
        .unwrap();
        assert_eq!(s.input_fields("derive"), Some(fields));
    }
}
