//! Session: the open document and the entry point that runs commands by id.
//!
//! Runs use immutable document snapshots; results and progress are applied by `poll_jobs`.

use crate::command::{self, Requires};
use crate::job::{JobEvent, JobId, JobManager, Progress, Waker};
use crate::run::{RunReport, RunRequest, RunTargets, run_stream};
use dms_stream::Stream;
use dms_strfmt::{ArchiveLimits, read_archive};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Largest `.str` file the session reads (the archive reader has its own member limits).
const MAX_STREAM_FILE_BYTES: u64 = 1 << 30;

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
    /// The application should close (after asking about unsaved work, from phase B).
    Quit,
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
    document: Option<Document>,
    jobs: JobManager<RunReport>,
    results: Option<RunResults>,
    run_state: RunState,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// A session over a stream that is already decoded (tests, synthetic streams).
    pub fn with_document(document: Document) -> Self {
        Session {
            document: Some(document),
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

    fn cancel_run(&mut self) {
        if let RunState::Running { job, .. } = self.run_state {
            self.jobs.cancel(job);
            self.run_state = RunState::Idle;
        }
    }

    fn start_run(&mut self, targets: RunTargets) {
        let Some(document) = &self.document else {
            return;
        };
        let request = RunRequest {
            stream: Arc::new(document.stream.clone()),
            attachments: Arc::clone(&document.attachments),
            data_dirs: vec![
                document
                    .path
                    .parent()
                    .filter(|dir| !dir.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf(),
            ],
            export_dir: export_dir_for(&document.path),
            targets,
        };
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
            "file.open" => {
                let Some(Arg::Path(path)) = args.first() else {
                    return Ok(Outcome::NeedsArgument("path"));
                };
                self.cancel_run();
                let document = open_document(path).map_err(|reason| CommandError::Open {
                    path: path.clone(),
                    reason,
                })?;
                self.document = Some(document);
                self.results = None;
                Ok(Outcome::Done)
            }
            "file.close" => {
                self.cancel_run();
                self.document = None;
                self.results = None;
                Ok(Outcome::Done)
            }
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
            "app.quit" => Ok(Outcome::Quit),
            other => Err(CommandError::Disabled {
                id: spec.id,
                reason: format!("{other} has no implementation yet"),
            }),
        }
    }
}

/// Reads and decodes a `.str` file.
pub fn open_document(path: &Path) -> Result<Document, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_STREAM_FILE_BYTES {
        return Err(format!(
            "file is {size} bytes; the limit is {MAX_STREAM_FILE_BYTES}"
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let archive = read_archive(&bytes, &ArchiveLimits::default()).map_err(|e| e.to_string())?;
    let stream = Stream::from_document(&archive.document).map_err(|e| e.to_string())?;
    Ok(Document {
        path: path.to_path_buf(),
        stream,
        attachments: Arc::new(
            archive
                .attachments
                .into_iter()
                .map(|attachment| (attachment.name, attachment.bytes.into()))
                .collect(),
        ),
    })
}

/// Centralized default; the interface can later provide an explicit export destination.
pub fn export_dir_for(path: &Path) -> PathBuf {
    std::env::temp_dir().join("dms-exports").join(
        path.file_stem()
            .unwrap_or_else(|| std::ffi::OsStr::new("stream")),
    )
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
        let document = session.document().unwrap();
        assert_eq!(document.path, path);
        assert_eq!(
            document.attachments.get("data/model.dat").unwrap().as_ref(),
            b"synthetic attachment"
        );
    }

    #[test]
    fn export_directory_uses_the_document_stem() {
        assert_eq!(
            export_dir_for(Path::new("/some/folder/example.str")),
            std::env::temp_dir().join("dms-exports/example")
        );
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
