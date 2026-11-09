//! Atomic recovery snapshots in a caller-selected application data directory.
use crate::session::Session;
use dms_stream::{StreamDoc, native};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

const INTERVAL: Duration = Duration::from_secs(30);
const MAX_FILES: usize = 20;
const ORIGINAL: &str = "dms_autosave_original";

#[derive(Debug, Clone)]
pub struct Recovery {
    pub path: PathBuf,
    pub original: Option<PathBuf>,
    pub saved_at: SystemTime,
}

#[derive(Debug, Default)]
pub(crate) struct Autosave {
    pub dir: Option<PathBuf>,
    pub last: Option<Instant>,
}

fn owned_path(path: &Path) -> bool {
    path.extension().is_some_and(|s| s == "dms")
        && path.file_stem().and_then(|s| s.to_str()).is_some_and(|s| {
            s.len() == 13
                && s.starts_with("id")
                && s.get(2..).is_some_and(|s| {
                    s.bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
                })
        })
}

fn files(dir: &Path) -> Result<Vec<(PathBuf, SystemTime)>, String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(format!(
                "Cannot read autosave directory {}: {e}",
                dir.display()
            ));
        }
    };
    let mut paths = Vec::new();
    for (count, entry) in entries.enumerate() {
        if count >= 10_000 {
            return Err("Autosave directory exceeds 10000 entries".into());
        }
        let entry = entry.map_err(|e| format!("Cannot read autosave entry: {e}"))?;
        let path = entry.path();
        if !owned_path(&path) {
            continue;
        }
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.file_type().is_file() {
            paths.push((path, meta.modified().map_err(|e| e.to_string())?));
        }
    }
    paths.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(paths)
}

pub(crate) fn remove(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Cannot remove autosave {}: {e}", path.display())),
    }
}

pub(crate) fn load_recovery(path: &Path) -> Result<(StreamDoc, Option<PathBuf>), String> {
    let mut doc = native::load(path).map_err(|e| e.to_string())?;
    let original = doc
        .meta
        .extra
        .remove(ORIGINAL)
        .and_then(|v| v.as_str().map(PathBuf::from));
    Ok((doc, original))
}

impl Session {
    pub fn set_autosave_dir(&mut self, dir: Option<PathBuf>) {
        self.autosave.dir = dir;
        self.autosave.last = Some(Instant::now());
    }

    pub(crate) fn remove_autosave(&self) -> Result<(), String> {
        if let Some(state) = &self.document_state {
            if let Some(dir) = &self.autosave.dir {
                remove(&dir.join(format!("{}.dms", state.id)))?;
            }
            if let Some(path) = &state.recovery_path {
                remove(path)?;
            }
            if let Some(path) = &state.autosave_path {
                remove(path)?;
            }
        }
        Ok(())
    }

    /// Cheap on each frame until a dirty document's 30-second deadline is reached.
    pub fn autosave_if_due(&mut self, now: Instant) -> Result<bool, String> {
        if !self.is_dirty() {
            return Ok(false);
        }
        let Some(dir) = &self.autosave.dir else {
            return Ok(false);
        };
        if self
            .autosave
            .last
            .is_some_and(|last| now.checked_duration_since(last).unwrap_or_default() < INTERVAL)
        {
            return Ok(false);
        }
        let Some(state) = &self.document_state else {
            return Ok(false);
        };
        fs::create_dir_all(dir).map_err(|e| format!("Cannot create autosave directory: {e}"))?;
        let mut doc = state.doc.clone();
        doc.meta.extra.remove(ORIGINAL);
        if let Some(original) = state.native_path.as_ref().or(state.imported_from.as_ref()) {
            doc.meta.extra.insert(
                ORIGINAL.into(),
                original.to_string_lossy().into_owned().into(),
            );
        }
        let path = dir.join(format!("{}.dms", state.id));
        native::save_atomic(&doc, &path).map_err(|e| e.to_string())?;
        if let Some(previous) = &state.autosave_path
            && previous != &path
        {
            remove(previous)?;
        }
        if let Some(state) = &mut self.document_state {
            state.autosave_path = Some(path.clone());
        }
        // Retain the snapshot just written even on file systems with coarse timestamps.
        let mut retained = 1usize;
        for (old, _) in files(dir)? {
            if old == path {
                continue;
            }
            if retained >= MAX_FILES {
                remove(&old)?;
            }
            retained = retained.saturating_add(1);
        }
        self.autosave.last = Some(now);
        Ok(true)
    }

    /// Leftover valid snapshots, newest first. Active-document snapshots are not recovery offers.
    pub fn recoverable(&self) -> Vec<Recovery> {
        let Some(dir) = &self.autosave.dir else {
            return Vec::new();
        };
        files(dir)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(path, saved_at)| {
                if self.document_state.as_ref().is_some_and(|state| {
                    path == dir.join(format!("{}.dms", state.id))
                        || state.recovery_path.as_ref() == Some(&path)
                }) {
                    return None;
                }
                let (_, original) = load_recovery(&path).ok()?;
                Some(Recovery {
                    path,
                    original,
                    saved_at,
                })
            })
            .take(MAX_FILES)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        edit_session::tests::{TempDir, session, text},
        session::{Arg, Outcome},
    };

    fn dirty(session: &mut Session) {
        session
            .execute(
                "node.rename",
                &[Arg::Node("a".into()), Arg::Text("dirty".into())],
            )
            .unwrap();
    }
    fn due(session: &Session) -> Instant {
        session.autosave.last.unwrap() + INTERVAL
    }
    fn leftovers(dir: &Path) -> usize {
        files(dir).unwrap().len()
    }

    #[test]
    fn autosave_deadlines_disabled_clean_and_backwards_clock_are_cheap() {
        let dir = TempDir::new();
        let mut session = session();
        dirty(&mut session);
        assert!(!session.autosave_if_due(Instant::now()).unwrap());
        session.set_autosave_dir(Some(dir.0.clone()));
        let start = session.autosave.last.unwrap();
        assert!(
            !session
                .autosave_if_due(start - Duration::from_secs(1))
                .unwrap()
        );
        assert!(
            !session
                .autosave_if_due(start + Duration::from_secs(29))
                .unwrap()
        );
        assert_eq!(leftovers(&dir.0), 0);
        let now = start + INTERVAL;
        assert!(session.autosave_if_due(now).unwrap());
        assert!(!session.autosave_if_due(now).unwrap());
        assert_eq!(leftovers(&dir.0), 1);
        assert!(session.recoverable().is_empty());
        let file = files(&dir.0).unwrap().remove(0).0;
        let (snapshot, original) = load_recovery(&file).unwrap();
        assert_eq!(native::to_toml(&snapshot).unwrap(), text(&session));
        assert_eq!(original, Some("example.str".into()));
        session.execute("edit.undo", &[]).unwrap();
        assert!(!session.is_dirty());
        assert!(!session.autosave_if_due(now + INTERVAL).unwrap());
        session.set_autosave_dir(None);
        assert!(!session.autosave_if_due(now + INTERVAL).unwrap());
    }

    #[test]
    fn recovery_is_newest_first_and_opened_as_an_unsaved_document() {
        let dir = TempDir::new();
        let mut first = session();
        dirty(&mut first);
        first.set_autosave_dir(Some(dir.0.clone()));
        first.autosave_if_due(due(&first)).unwrap();
        let older = files(&dir.0).unwrap().remove(0).0;
        // A distinct mtime without depending on sleeps or file-system clock resolution.
        fs::File::options()
            .write(true)
            .open(&older)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(10)),
            )
            .unwrap();
        let mut second = session();
        dirty(&mut second);
        second.set_autosave_dir(Some(dir.0.clone()));
        second.autosave_if_due(due(&second)).unwrap();
        let newer = dir
            .0
            .join(format!("{}.dms", second.document_state().unwrap().id));
        drop(first);
        drop(second); // Simulate termination without a normal close command.
        let mut recovered = Session::new();
        recovered.set_autosave_dir(Some(dir.0.clone()));
        let candidates = recovered.recoverable();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].path, newer);
        assert_eq!(candidates[1].path, older);
        assert!(candidates[0].saved_at > candidates[1].saved_at);
        assert_eq!(candidates[0].original, Some("example.str".into()));
        recovered
            .execute("file.recover", &[Arg::Path(newer.clone())])
            .unwrap();
        assert!(recovered.is_dirty());
        assert!(recovered.document_state().unwrap().native_path.is_none());
        assert_eq!(
            recovered.execute("file.save", &[]).unwrap(),
            Outcome::NeedsArgument("save_path")
        );
        assert_eq!(
            recovered
                .document_state()
                .unwrap()
                .doc
                .node("a")
                .unwrap()
                .label
                .as_deref(),
            Some("dirty")
        );
        assert_eq!(recovered.recoverable().len(), 1);
        recovered
            .execute("file.save_as", &[Arg::Path(dir.0.join("recovered.dms"))])
            .unwrap();
        assert!(!newer.exists());
        assert!(older.exists());
        assert!(!recovered.is_dirty());
    }

    #[test]
    fn normal_save_close_new_and_quit_clean_only_their_own_snapshot() {
        for command in ["file.save_as", "file.close", "file.new", "app.quit"] {
            let dir = TempDir::new();
            let mut session = session();
            dirty(&mut session);
            session.set_autosave_dir(Some(dir.0.clone()));
            session.autosave_if_due(due(&session)).unwrap();
            let original_id = session.document_state().unwrap().id.clone();
            let args = if command == "file.save_as" {
                vec![Arg::Path(dir.0.join("normal"))]
            } else {
                vec![Arg::Force]
            };
            session.execute(command, &args).unwrap();
            assert_eq!(leftovers(&dir.0), 0, "{command}");
            if command == "file.new" {
                assert_ne!(session.document_state().unwrap().id, original_id);
            }
        }
    }

    #[test]
    fn retains_twenty_newest_owned_snapshots_and_ignores_other_files() {
        let dir = TempDir::new();
        let doc = StreamDoc::new();
        let mut oldest = PathBuf::new();
        for i in 0..22u64 {
            let path = dir.0.join(format!("id{i:011}.dms"));
            native::save_atomic(&doc, &path).unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(
                    fs::FileTimes::new()
                        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(i)),
                )
                .unwrap();
            if i == 0 {
                oldest = path;
            }
        }
        let unrelated = dir.0.join("important.dms");
        fs::write(&unrelated, "keep").unwrap();
        let malformed = dir.0.join("idZZZZZZZZZZZ.dms");
        fs::write(&malformed, "invalid").unwrap();
        let mut session = session();
        dirty(&mut session);
        session.set_autosave_dir(Some(dir.0.clone()));
        session.autosave_if_due(due(&session)).unwrap();
        assert_eq!(leftovers(&dir.0), 20);
        assert!(!oldest.exists());
        assert_eq!(fs::read_to_string(unrelated).unwrap(), "keep");
        let candidates = session.recoverable();
        assert!(candidates.len() <= 19);
        assert!(!candidates.iter().any(|r| r.path == malformed));
        assert!(
            candidates
                .windows(2)
                .all(|w| w[0].saved_at >= w[1].saved_at)
        );
    }

    #[test]
    fn autosave_failures_are_errors_and_leave_document_dirty_and_retryable() {
        let dir = TempDir::new();
        let blocked = dir.0.join("file");
        fs::write(&blocked, "not a directory").unwrap();
        let mut session = session();
        dirty(&mut session);
        session.set_autosave_dir(Some(blocked));
        let before = text(&session);
        let last = session.autosave.last;
        assert!(session.autosave_if_due(due(&session)).is_err());
        assert_eq!(session.autosave.last, last);
        assert!(session.is_dirty());
        assert_eq!(text(&session), before);
        session.set_autosave_dir(Some(dir.0.join("valid")));
        assert!(session.autosave_if_due(due(&session)).unwrap());
    }

    #[test]
    fn changing_or_disabling_the_directory_still_cleans_the_previous_snapshot() {
        let first = TempDir::new();
        let second = TempDir::new();
        let mut session = session();
        dirty(&mut session);
        session.set_autosave_dir(Some(first.0.clone()));
        session.autosave_if_due(due(&session)).unwrap();
        assert_eq!(leftovers(&first.0), 1);
        session.set_autosave_dir(Some(second.0.clone()));
        session.autosave_if_due(due(&session)).unwrap();
        assert_eq!(leftovers(&first.0), 0);
        assert_eq!(leftovers(&second.0), 1);
        session.set_autosave_dir(None);
        session
            .execute("file.save_as", &[Arg::Path(first.0.join("saved.dms"))])
            .unwrap();
        assert_eq!(leftovers(&second.0), 0);
    }

    #[test]
    fn arbitrary_recovery_inputs_are_not_deleted_on_close() {
        let dir = TempDir::new();
        let source = dir.0.join("user-document.dms");
        native::save_atomic(&StreamDoc::new(), &source).unwrap();
        let mut session = Session::new();
        session.set_autosave_dir(Some(dir.0.clone()));
        session
            .execute("file.recover", &[Arg::Path(source.clone())])
            .unwrap();
        session.execute("file.close", &[Arg::Force]).unwrap();
        assert!(source.exists());
    }
}
