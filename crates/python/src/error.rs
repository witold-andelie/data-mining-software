//! Bridge errors. Every message says what to do next, because the usual causes (no Python, a
//! missing package, an error in the user's script) are fixed by the user, not by us.

use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeError {
    /// No usable Python was found. `tried` lists the commands attempted.
    NotFound {
        tried: Vec<String>,
    },
    /// The configured or found Python is older than [`crate::MIN_VERSION`].
    TooOld {
        python: String,
        version: String,
    },
    /// Packages the bridge needs are not importable in that Python.
    MissingPackages {
        python: String,
        packages: Vec<String>,
    },
    /// The user's script raised an exception. `line` is the line in the script, when known.
    Script {
        message: String,
        line: Option<u32>,
        traceback: String,
    },
    /// The script ran but its result cannot be used (no data frame, an unsupported column type).
    Output(String),
    /// Data could not be passed to Python (an unsupported field type, a value out of range).
    Input(String),
    /// The Python process failed outside the script (crashed, killed, unreadable result).
    Process(String),
    /// Temporary files could not be written or read.
    Io(String),
    Cancelled,
    Timeout(Duration),
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { tried } => write!(
                f,
                "Python was not found (tried {}). Install Python 3.9 or later, or choose the \
                 Python executable in Settings.",
                tried.join(", ")
            ),
            Self::TooOld { python, version } => write!(
                f,
                "{python} is Python {version}; Python 3.9 or later is required. Choose a newer \
                 Python in Settings."
            ),
            Self::MissingPackages { python, packages } => write!(
                f,
                "{python} is missing {}. Install with: {python} -m pip install {}",
                packages.join(", "),
                packages.join(" ")
            ),
            Self::Script {
                message,
                line: Some(line),
                ..
            } => write!(f, "Python script error at line {line}: {message}"),
            Self::Script { message, .. } => write!(f, "Python script error: {message}"),
            Self::Output(message) => write!(f, "Python script result: {message}"),
            Self::Input(message) => write!(f, "Data for Python: {message}"),
            Self::Process(message) => write!(f, "Python process: {message}"),
            Self::Io(message) => write!(f, "Python bridge files: {message}"),
            Self::Cancelled => f.write_str("cancelled"),
            Self::Timeout(limit) => write!(f, "Python did not finish within {} s", limit.as_secs()),
        }
    }
}

impl std::error::Error for BridgeError {}

impl From<std::io::Error> for BridgeError {
    fn from(error: std::io::Error) -> Self {
        BridgeError::Io(error.to_string())
    }
}
