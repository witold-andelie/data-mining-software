//! Session: the open document and the entry point that runs commands by id.
//!
//! Runs use immutable document snapshots; results and progress are applied by `poll_jobs`.

pub use crate::autosave::Recovery;
use crate::command::{self, Requires};
pub use crate::edit_session::DocumentState;
use crate::edit_session::{document_view, read_state};
use crate::job::{JobEvent, JobId, JobManager, Progress, Waker};
use crate::run::{RunReport, RunRequest, RunTargets, run_stream};
use dms_stream::Stream;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Upper bound on extra data folders.
pub const MAX_DATA_FOLDERS: usize = 64;

/// An open stream.
#[derive(Debug, Clone)]
pub struct Document {
    pub path: PathBuf,
    pub stream: Stream,
    pub attachments: Arc<HashMap<String, Arc<[u8]>>>,
}

impl Document {
    pub fn from_stream(path: PathBuf, stream: Stream) -> Document {
        Document {
            path,
            stream,
            attachments: Arc::new(HashMap::new()),
        }
    }
}

/// Arguments a command takes. Interfaces collect them (a file dialog, a form); programs pass
/// them directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    Path(PathBuf),
    Node(String),
    Force,
    Text(String),
    Kind(String),
    Position(i64, i64),
    Delta(i64, i64),
    Nodes(Vec<String>),
    Link(String, String),
}

/// View changes the engine asks the interface to make; the engine does not own view state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewAction {
    ZoomIn,
    ZoomOut,
    ZoomFit,
    ToggleFullscreen,
    ShowAbout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The session changed (or nothing needed to change).
    Done,
    /// The interface applies a view change.
    View(ViewAction),
    /// The command needs this argument; an interface asks the person (for `path`: a file
    /// dialog) and calls again. Programs get this instead of a dialog.
    NeedsArgument(&'static str),
    /// The application can close; the discard guard has already passed.
    Quit,
    /// Retry the requested lifecycle command with `Arg::Force` after user confirmation.
    ConfirmDiscard,
    /// The interface places this text on its clipboard.
    Clipboard(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    Unknown(String),
    Disabled {
        id: &'static str,
        reason: String,
    },
    BadArgument {
        id: &'static str,
        expected: &'static str,
    },
    Failed {
        id: &'static str,
        reason: String,
    },
    Open {
        path: PathBuf,
        reason: String,
    },
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::Unknown(id) => write!(f, "unknown command {id:?}"),
            CommandError::Disabled { id, reason } => write!(f, "{id} is not available: {reason}"),
            CommandError::BadArgument { id, expected } => write!(f, "{id} expects {expected}"),
            CommandError::Failed { id, reason } => write!(f, "{id}: {reason}"),
            CommandError::Open { path, reason } => {
                write!(f, "cannot open {}: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for CommandError {}

#[derive(Debug)]
pub struct RunResults {
    pub job: JobId,
    pub report: RunReport,
    pub revision: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RunState {
    #[default]
    Idle,
    Running {
        job: JobId,
        progress: Progress,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunNotice {
    Finished {
        job: JobId,
        root_causes: usize,
        elapsed: std::time::Duration,
    },
    Failed {
        job: JobId,
        message: String,
    },
    Cancelled {
        job: JobId,
    },
}

#[derive(Debug, Default)]
pub struct Session {
    pub(crate) document: Option<Document>,
    pub(crate) document_state: Option<DocumentState>,
    pub(crate) autosave: crate::autosave::Autosave,
    run_revision: u64,
    /// Folders searched by file name for data a stream references, after the stream's own
    /// folder (streams often name files on the machine that saved them).
    data_folders: Vec<PathBuf>,
    jobs: JobManager<RunReport>,
    pub(crate) results: Option<RunResults>,
    run_state: RunState,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// A clean session over an already decoded stream (tests, synthetic streams).
    /// File imports should use `file.open` to track their unsaved native document.
    pub fn with_document(document: Document) -> Self {
        let native = document.path.extension().is_some_and(|s| s == "dms");
        let state = DocumentState::new(
            dms_stream::StreamDoc {
                stream: document.stream,
                attachments: document
                    .attachments
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                meta: Default::default(),
            },
            native.then(|| document.path.clone()),
            (!native).then_some(document.path),
            true,
        );
        Session {
            document: Some(document_view(&state)),
            document_state: Some(state),
            ..Self::default()
        }
    }

    pub fn document(&self) -> Option<&Document> {
        self.document.as_ref()
    }

    pub fn set_waker(&mut self, waker: Option<Waker>) {
        self.jobs.set_waker(waker);
    }

    pub fn run_state(&self) -> RunState {
        self.run_state.clone()
    }

    pub fn results(&self) -> Option<&RunResults> {
        self.results.as_ref()
    }

    /// Applies worker news once per frame. Old document jobs cannot replace current results.
    pub fn poll_jobs(&mut self) -> Vec<RunNotice> {
        let mut notices = Vec::new();
        for event in self.jobs.poll() {
            match event {
                JobEvent::Progress { job, progress } => {
                    if matches!(self.run_state, RunState::Running { job: current, .. } if current == job)
                    {
                        self.run_state = RunState::Running { job, progress };
                    }
                }
                JobEvent::Finished { job, output } => {
                    if matches!(self.run_state, RunState::Running { job: current, .. } if current == job)
                    {
                        notices.push(RunNotice::Finished {
                            job,
                            root_causes: output.root_causes.len(),
                            elapsed: output.elapsed,
                        });
                        self.results = Some(RunResults {
                            job,
                            report: output,
                            revision: self.run_revision,
                        });
                        self.run_state = RunState::Idle;
                    }
                }
                JobEvent::Panicked { job, message } => {
                    if matches!(self.run_state, RunState::Running { job: current, .. } if current == job)
                    {
                        self.run_state = RunState::Idle;
                        notices.push(RunNotice::Failed { job, message });
                    }
                }
                JobEvent::Cancelled { job } => {
                    if matches!(self.run_state, RunState::Running { job: current, .. } if current == job)
                    {
                        self.run_state = RunState::Idle;
                    }
                    notices.push(RunNotice::Cancelled { job });
                }
            }
        }
        notices
    }

    pub(crate) fn cancel_run(&mut self) {
        if let RunState::Running { job, .. } = self.run_state {
            self.jobs.cancel(job);
            self.run_state = RunState::Idle;
        }
    }

    /// The extra data folders, in search order.
    pub fn data_folders(&self) -> &[PathBuf] {
        &self.data_folders
    }

    /// Replaces the extra data folders (restored from the interface's saved state). Duplicates
    /// and more than [`MAX_DATA_FOLDERS`] folders are dropped.
    pub fn set_data_folders(&mut self, folders: Vec<PathBuf>) {
        self.data_folders.clear();
        for folder in folders {
            self.add_data_folder(folder);
        }
    }

    fn add_data_folder(&mut self, folder: PathBuf) {
        if self.data_folders.len() < MAX_DATA_FOLDERS && !self.data_folders.contains(&folder) {
            self.data_folders.push(folder);
        }
    }

    fn start_run(&mut self, targets: RunTargets) {
        let Some(state) = &self.document_state else {
            return;
        };
        let request = RunRequest {
            stream: Arc::new(state.doc.stream.clone()),
            attachments: Arc::new(
                state
                    .doc
                    .attachments
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
            data_dirs: std::iter::once(
                state
                    .native_path
                    .as_ref()
                    .or(state.imported_from.as_ref())
                    .and_then(|path| path.parent())
                    .filter(|dir| !dir.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf(),
            )
            .chain(self.data_folders.iter().cloned())
            .collect(),
            export_dir: export_dir_for(
                state
                    .native_path
                    .as_deref()
                    .or(state.imported_from.as_deref())
                    .unwrap_or_else(|| Path::new("Untitled.dms")),
            ),
            targets,
        };
        self.run_revision = state.revision;
        let job = self.jobs.spawn(move |ctx| run_stream(request, ctx));
        self.run_state = RunState::Running {
            job,
            progress: Progress {
                done: 0,
                total: 0,
                current: None,
            },
        };
    }

    /// Whether a command can run now, and if not, why.
    pub fn availability(&self, id: &str) -> Result<(), String> {
        let spec = command::by_id(id).ok_or_else(|| format!("unknown command {id:?}"))?;
        match id {
            "edit.undo" if self.undo_label().is_none() => return Err("Nothing to undo".into()),
            "edit.redo" if self.redo_label().is_none() => return Err("Nothing to redo".into()),
            "edit.cut" | "edit.copy" | "edit.duplicate" | "edit.delete"
                if self
                    .document_state
                    .as_ref()
                    .is_none_or(|s| s.selection.is_empty()) =>
            {
                return Err("No nodes are selected".into());
            }
            _ => {}
        }
        match spec.requires {
            Requires::Nothing => Ok(()),
            Requires::Document if self.document.is_some() => Ok(()),
            Requires::Document => Err("No stream is open".to_string()),
            Requires::DocumentIdle if self.document.is_none() => {
                Err("No stream is open".to_string())
            }
            Requires::DocumentIdle if matches!(self.run_state, RunState::Running { .. }) => {
                Err("A stream is already running".to_string())
            }
            Requires::DocumentIdle => Ok(()),
            Requires::Running if matches!(self.run_state, RunState::Running { .. }) => Ok(()),
            Requires::Running => Err("No stream is running".to_string()),
            Requires::Planned(reason) => Err(reason.to_string()),
        }
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.availability(id).is_ok()
    }

    /// Runs a command by id. Never shows a dialog and never panics.
    pub fn execute(&mut self, id: &str, args: &[Arg]) -> Result<Outcome, CommandError> {
        let spec = command::by_id(id).ok_or_else(|| CommandError::Unknown(id.to_string()))?;
        self.availability(id)
            .map_err(|reason| CommandError::Disabled {
                id: spec.id,
                reason,
            })?;
        match spec.id {
            "file.new" | "file.open" | "file.save" | "file.save_as" | "file.close"
            | "file.recover" | "app.quit" | "edit.undo" | "edit.redo" | "edit.cut"
            | "edit.copy" | "edit.paste" | "edit.duplicate" | "edit.delete" | "edit.select_all"
            | "edit.select" | "node.add" | "node.move" | "node.connect" | "node.disconnect"
            | "node.rename" => self.execute_document_command(spec.id, args),
            "run.stream" => {
                self.start_run(RunTargets::Stream);
                Ok(Outcome::Done)
            }
            "run.node" => {
                let Some(Arg::Node(uid)) = args.first() else {
                    return Ok(Outcome::NeedsArgument("node"));
                };
                self.start_run(RunTargets::Nodes(vec![uid.clone()]));
                Ok(Outcome::Done)
            }
            "file.add_data_folder" => {
                let Some(Arg::Path(folder)) = args.first() else {
                    return Ok(Outcome::NeedsArgument("folder"));
                };
                if !folder.is_dir() {
                    return Err(CommandError::BadArgument {
                        id: spec.id,
                        expected: "an existing folder",
                    });
                }
                self.add_data_folder(folder.clone());
                Ok(Outcome::Done)
            }
            "file.clear_data_folders" => {
                self.data_folders.clear();
                Ok(Outcome::Done)
            }
            "run.cancel" => {
                // Keep a completed-but-unpolled job active until its final event is applied.
                if let RunState::Running { job, .. } = self.run_state
                    && self.jobs.cancel(job)
                {
                    self.run_state = RunState::Idle;
                }
                Ok(Outcome::Done)
            }
            "view.zoom_in" => Ok(Outcome::View(ViewAction::ZoomIn)),
            "view.zoom_out" => Ok(Outcome::View(ViewAction::ZoomOut)),
            "view.zoom_fit" => Ok(Outcome::View(ViewAction::ZoomFit)),
            "view.fullscreen" => Ok(Outcome::View(ViewAction::ToggleFullscreen)),
            "app.about" => Ok(Outcome::View(ViewAction::ShowAbout)),
            other => Err(CommandError::Disabled {
                id: spec.id,
                reason: format!("{other} has no implementation yet"),
            }),
        }
    }
}

/// Reads an imported `.str` or native `.dms` into the v0.1 compatibility view.
pub fn open_document(path: &Path) -> Result<Document, String> {
    read_state(path).map(|state| document_view(&state))
}

/// Centralized default; the interface can later provide an explicit export destination.
/// Where a stream's export nodes write: `Documents/Data Mining/Exports/<stream name>/` in the
/// person's home folder (macOS `HOME`, Windows `USERPROFILE`), or the temporary folder when
/// there is no home folder.
pub fn export_dir_for(path: &Path) -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|home| !home.is_empty())
        .map(PathBuf::from);
    exports_root(home).join(
        path.file_stem()
            .unwrap_or_else(|| std::ffi::OsStr::new("stream")),
    )
}

fn exports_root(home: Option<PathBuf>) -> PathBuf {
    match home {
        Some(home) => home.join("Documents").join("Data Mining").join("Exports"),
        None => std::env::temp_dir().join("Data Mining Exports"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::run::tests::{mystery_stream, node, stream};
    use std::time::{Duration, Instant};

    fn with_stream() -> Session {
        Session::with_document(Document::from_stream(
            PathBuf::from("/tmp/example.str"),
            mystery_stream(),
        ))
    }

    fn wait_for_notice(session: &mut Session) -> Vec<RunNotice> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let notices = session.poll_jobs();
            if !notices.is_empty() {
                return notices;
            }
            assert!(Instant::now() < deadline, "session job did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn run_commands_require_a_document_an_idle_job_and_a_node_argument() {
        let empty = Session::new();
        assert!(!empty.is_enabled("run.stream"));
        assert!(!empty.is_enabled("run.node"));
        assert!(!empty.is_enabled("run.cancel"));
        let mut session = with_stream();
        assert_eq!(
            session.execute("run.node", &[]),
            Ok(Outcome::NeedsArgument("node"))
        );
        assert!(session.is_enabled("run.stream"));
        assert_eq!(session.execute("run.stream", &[]), Ok(Outcome::Done));
        assert!(!session.is_enabled("run.stream"));
        assert!(!session.is_enabled("run.node"));
        assert!(session.is_enabled("run.cancel"));
        assert!(matches!(session.run_state(), RunState::Running { .. }));
        assert!(matches!(
            wait_for_notice(&mut session).as_slice(),
            [RunNotice::Finished { root_causes: 1, .. }]
        ));
        assert_eq!(session.run_state(), RunState::Idle);
        assert!(session.is_enabled("run.stream"));
        assert!(!session.is_enabled("run.cancel"));
        assert_eq!(
            session.results().unwrap().report.root_causes[0].uid,
            "mystery"
        );
    }

    #[test]
    fn selected_run_replaces_results_and_close_clears_them() {
        let mut session = with_stream();
        session.execute("run.stream", &[]).unwrap();
        wait_for_notice(&mut session);
        let first = session.results().unwrap().job;
        session
            .execute("run.node", &[Arg::Node("mystery".into())])
            .unwrap();
        wait_for_notice(&mut session);
        let results = session.results().unwrap();
        assert_ne!(results.job, first);
        assert_eq!(results.report.targets, ["mystery"]);
        assert_eq!(results.report.results.len(), 1);
        session.execute("file.close", &[]).unwrap();
        assert!(session.results().is_none());
        assert!(session.document().is_none());
    }

    #[test]
    fn cancelling_a_run_and_closing_drop_late_results() {
        let mut session = with_stream();
        let (ready, arrived) = std::sync::mpsc::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let job = session.jobs.spawn(move |_| {
            ready.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(10)).unwrap();
            RunReport {
                targets: Vec::new(),
                results: HashMap::new(),
                root_causes: Vec::new(),
                elapsed: Duration::ZERO,
            }
        });
        session.run_state = RunState::Running {
            job,
            progress: Progress {
                done: 0,
                total: 0,
                current: None,
            },
        };
        arrived.recv_timeout(Duration::from_secs(10)).unwrap();
        session.execute("run.cancel", &[]).unwrap();
        assert_eq!(session.run_state(), RunState::Idle);
        assert_eq!(session.poll_jobs(), [RunNotice::Cancelled { job }]);
        session.execute("file.close", &[]).unwrap();
        release.send(()).unwrap();
        session.execute("file.open", &[]).unwrap();
        assert!(session.poll_jobs().is_empty());
        assert!(session.results().is_none());
    }

    #[test]
    fn closing_cancels_active_work_and_ignores_unpolled_finished_work() {
        for already_finished in [false, true] {
            let mut session = with_stream();
            let (ready, arrived) = std::sync::mpsc::channel();
            let (release, gate) = std::sync::mpsc::channel();
            let job = session.jobs.spawn(move |_| {
                ready.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(10)).unwrap();
                RunReport {
                    targets: Vec::new(),
                    results: HashMap::new(),
                    root_causes: Vec::new(),
                    elapsed: Duration::ZERO,
                }
            });
            session.run_state = RunState::Running {
                job,
                progress: Progress {
                    done: 0,
                    total: 0,
                    current: None,
                },
            };
            arrived.recv_timeout(Duration::from_secs(10)).unwrap();
            if already_finished {
                release.send(()).unwrap();
                let deadline = Instant::now() + Duration::from_secs(10);
                while !session.jobs.running().is_empty() {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            session.execute("file.close", &[]).unwrap();
            if !already_finished {
                release.send(()).unwrap();
            }
            session.poll_jobs();
            assert!(session.results().is_none());
            assert_eq!(session.run_state(), RunState::Idle);
        }
    }

    #[test]
    fn worker_panic_is_a_failed_notice_and_returns_to_idle() {
        let mut session = with_stream();
        let job = session.jobs.spawn(|_| panic!("session worker boom"));
        session.run_state = RunState::Running {
            job,
            progress: Progress {
                done: 0,
                total: 0,
                current: None,
            },
        };
        assert_eq!(
            wait_for_notice(&mut session),
            [RunNotice::Failed {
                job,
                message: "session worker boom".into()
            }]
        );
        assert_eq!(session.run_state(), RunState::Idle);
        assert!(session.results().is_none());
    }

    #[test]
    fn opening_a_document_preserves_attachments_and_clears_previous_results() {
        // A synthetic ZIP and hand-encoded Fast Infoset stream; no vendor files.
        const ARCHIVE: &[u8] = &[
            0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0x5c,
            0xbe, 0xf6, 0xd6, 0x29, 0x21, 0x00, 0x00, 0x00, 0x21, 0x00, 0x00, 0x00, 0x0a, 0x00,
            0x00, 0x00, 0x73, 0x74, 0x72, 0x65, 0x61, 0x6d, 0x2e, 0x78, 0x6d, 0x6c, 0xe0, 0x00,
            0x00, 0x01, 0x00, 0x3c, 0x0f, 0x63, 0x6c, 0x65, 0x6d, 0x65, 0x6e, 0x74, 0x69, 0x6e,
            0x65, 0x53, 0x74, 0x72, 0x65, 0x61, 0x6d, 0x3c, 0x05, 0x73, 0x74, 0x72, 0x65, 0x61,
            0x6d, 0xff, 0xf0, 0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x21, 0x5c, 0x5b, 0x07, 0xa2, 0x22, 0x14, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00,
            0x00, 0x0e, 0x00, 0x00, 0x00, 0x64, 0x61, 0x74, 0x61, 0x2f, 0x6d, 0x6f, 0x64, 0x65,
            0x6c, 0x2e, 0x64, 0x61, 0x74, 0x73, 0x79, 0x6e, 0x74, 0x68, 0x65, 0x74, 0x69, 0x63,
            0x20, 0x61, 0x74, 0x74, 0x61, 0x63, 0x68, 0x6d, 0x65, 0x6e, 0x74, 0x50, 0x4b, 0x01,
            0x02, 0x14, 0x03, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0x5c, 0xbe,
            0xf6, 0xd6, 0x29, 0x21, 0x00, 0x00, 0x00, 0x21, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x73, 0x74, 0x72, 0x65, 0x61, 0x6d, 0x2e, 0x78, 0x6d, 0x6c, 0x50, 0x4b, 0x01,
            0x02, 0x14, 0x03, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0x5c, 0x5b,
            0x07, 0xa2, 0x22, 0x14, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, 0x0e, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x01, 0x49, 0x00, 0x00,
            0x00, 0x64, 0x61, 0x74, 0x61, 0x2f, 0x6d, 0x6f, 0x64, 0x65, 0x6c, 0x2e, 0x64, 0x61,
            0x74, 0x50, 0x4b, 0x05, 0x06, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x02, 0x00, 0x74,
            0x00, 0x00, 0x00, 0x89, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let path =
            std::env::temp_dir().join(format!("dms-engine-attachments-{}.str", std::process::id()));
        std::fs::write(&path, ARCHIVE).unwrap();
        let mut session = with_stream();
        session.execute("run.stream", &[]).unwrap();
        wait_for_notice(&mut session);
        assert!(session.results().is_some());
        let opened = session.execute("file.open", &[Arg::Path(path.clone())]);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(opened, Ok(Outcome::Done));
        assert!(session.results().is_none());
        assert!(session.is_dirty());
        assert!(session.document_state().unwrap().native_path.is_none());
        assert_eq!(
            session.execute("file.save", &[]),
            Ok(Outcome::NeedsArgument("save_path"))
        );
        let document = session.document().unwrap();
        assert_eq!(document.path, path);
        assert_eq!(
            document.attachments.get("data/model.dat").unwrap().as_ref(),
            b"synthetic attachment"
        );
    }

    #[test]
    fn export_directory_uses_the_document_stem() {
        let dir = export_dir_for(Path::new("/some/folder/example.str"));
        assert_eq!(dir.file_name(), Some(std::ffi::OsStr::new("example")));
        assert!(dir.parent().is_some_and(|p| p.ends_with("Exports")));
        let document = Document::from_stream(
            PathBuf::from("test.str"),
            stream(vec![node("n", "Mystery")], &[], &[]),
        );
        assert!(document.attachments.is_empty());
    }

    #[test]
    fn every_registered_command_is_handled_or_reports_why_not() {
        let mut session = Session::new();
        for spec in command::COMMANDS {
            match session.execute(spec.id, &[]) {
                Ok(_) => {}
                Err(CommandError::Disabled { reason, .. }) => assert!(!reason.is_empty()),
                Err(other) => panic!("{}: {other}", spec.id),
            }
        }
    }

    #[test]
    fn exports_go_to_documents_in_the_home_folder() {
        let home = PathBuf::from("/Users/someone");
        assert_eq!(
            exports_root(Some(home.clone())),
            home.join("Documents").join("Data Mining").join("Exports")
        );
        assert!(exports_root(None).starts_with(std::env::temp_dir()));
        assert_eq!(
            export_dir_for(Path::new("/x/Stream č.1.str")).file_name(),
            Some(std::ffi::OsStr::new("Stream č.1"))
        );
    }

    #[test]
    fn data_folders_are_added_once_and_checked() {
        let mut session = Session::new();
        assert_eq!(
            session.execute("file.add_data_folder", &[]),
            Ok(Outcome::NeedsArgument("folder"))
        );
        let dir = std::env::temp_dir();
        for _ in 0..2 {
            assert_eq!(
                session.execute("file.add_data_folder", &[Arg::Path(dir.clone())]),
                Ok(Outcome::Done)
            );
        }
        assert_eq!(session.data_folders(), std::slice::from_ref(&dir));
        assert!(matches!(
            session.execute(
                "file.add_data_folder",
                &[Arg::Path(dir.join("dms-no-such-folder"))]
            ),
            Err(CommandError::BadArgument { .. })
        ));
        session.set_data_folders((0..100).map(|i| dir.join(i.to_string())).collect());
        assert_eq!(session.data_folders().len(), MAX_DATA_FOLDERS);
        assert_eq!(
            session.execute("file.clear_data_folders", &[]),
            Ok(Outcome::Done)
        );
        assert!(session.data_folders().is_empty());
    }

    #[test]
    fn open_without_a_path_asks_for_one_instead_of_a_dialog() {
        let mut session = Session::new();
        assert_eq!(
            session.execute("file.open", &[]),
            Ok(Outcome::NeedsArgument("path"))
        );
        assert!(session.document().is_none());
    }

    #[test]
    fn document_commands_need_a_document() {
        let mut session = Session::new();
        assert!(!session.is_enabled("file.close"));
        assert!(!session.is_enabled("view.zoom_in"));
        assert!(matches!(
            session.execute("view.zoom_in", &[]),
            Err(CommandError::Disabled { .. })
        ));
        assert!(session.is_enabled("file.open"));
        assert!(session.availability("file.save").is_err());
    }

    #[test]
    fn unknown_commands_and_bad_files_are_errors() {
        let mut session = Session::new();
        assert_eq!(
            session.execute("no.such", &[]),
            Err(CommandError::Unknown("no.such".into()))
        );
        let missing = std::env::temp_dir().join("dms-engine-no-such-file.str");
        let error = session
            .execute("file.open", &[Arg::Path(missing)])
            .unwrap_err();
        assert!(matches!(error, CommandError::Open { .. }), "{error}");

        let garbage =
            std::env::temp_dir().join(format!("dms-engine-garbage-{}.str", std::process::id()));
        std::fs::write(&garbage, b"not a stream").unwrap();
        let error = session
            .execute("file.open", &[Arg::Path(garbage.clone())])
            .unwrap_err();
        std::fs::remove_file(&garbage).unwrap();
        assert!(matches!(error, CommandError::Open { .. }), "{error}");
        assert!(session.document().is_none());
    }

    #[test]
    fn view_and_app_commands_return_outcomes() {
        let mut session = Session::new();
        assert_eq!(session.execute("app.quit", &[]), Ok(Outcome::Quit));
        assert_eq!(
            session.execute("app.about", &[]),
            Ok(Outcome::View(ViewAction::ShowAbout))
        );
        assert_eq!(
            session.execute("view.fullscreen", &[]),
            Ok(Outcome::View(ViewAction::ToggleFullscreen))
        );
    }
}
