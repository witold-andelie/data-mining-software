//! Session: the open document and the entry point that runs commands by id.
//!
//! Phase A spike (docs/gui/DESIGN.md §6): a read-only document imported from `.str`. Editing,
//! undo/redo, the native format and background jobs arrive in phase B with the interfaces the
//! design describes; the calling convention (`execute(id, args)` returning an [`Outcome`]) is
//! the one they will keep.

use crate::command::{self, Requires};
use dms_stream::Stream;
use dms_strfmt::{ArchiveLimits, read_archive};
use std::fmt;
use std::path::{Path, PathBuf};

/// Largest `.str` file the session reads (the archive reader has its own member limits).
const MAX_STREAM_FILE_BYTES: u64 = 1 << 30;

/// An open stream.
#[derive(Debug, Clone)]
pub struct Document {
    pub path: PathBuf,
    pub stream: Stream,
}

/// Arguments a command takes. Interfaces collect them (a file dialog, a form); programs pass
/// them directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    Path(PathBuf),
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

#[derive(Debug, Default)]
pub struct Session {
    document: Option<Document>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// A session over a stream that is already decoded (tests, synthetic streams).
    pub fn with_document(document: Document) -> Self {
        Session {
            document: Some(document),
        }
    }

    pub fn document(&self) -> Option<&Document> {
        self.document.as_ref()
    }

    /// Whether a command can run now, and if not, why.
    pub fn availability(&self, id: &str) -> Result<(), String> {
        let spec = command::by_id(id).ok_or_else(|| format!("unknown command {id:?}"))?;
        match spec.requires {
            Requires::Nothing => Ok(()),
            Requires::Document if self.document.is_some() => Ok(()),
            Requires::Document => Err("No stream is open".to_string()),
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
                let stream = open_stream(path).map_err(|reason| CommandError::Open {
                    path: path.clone(),
                    reason,
                })?;
                self.document = Some(Document {
                    path: path.clone(),
                    stream,
                });
                Ok(Outcome::Done)
            }
            "file.close" => {
                self.document = None;
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
pub fn open_stream(path: &Path) -> Result<Stream, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_STREAM_FILE_BYTES {
        return Err(format!(
            "file is {size} bytes; the limit is {MAX_STREAM_FILE_BYTES}"
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let archive = read_archive(&bytes, &ArchiveLimits::default()).map_err(|e| e.to_string())?;
    Stream::from_document(&archive.document).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
