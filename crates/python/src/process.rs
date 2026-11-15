//! Running a Python command: output read on threads into bounded buffers, cancellation and an
//! optional time limit enforced by killing the process.

use crate::BridgeError;
use dms_types::CancelToken;
use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How often a running process is checked for exit or cancellation.
const POLL: Duration = Duration::from_millis(20);

/// A Python interpreter command: the program and the arguments that come before the script
/// (`py -3` on Windows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonCommand {
    pub program: OsString,
    pub prefix: Vec<OsString>,
}

impl PythonCommand {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            prefix: Vec::new(),
        }
    }

    /// How the command is shown in messages (`py -3`, `/usr/bin/python3`).
    pub fn display(&self) -> String {
        std::iter::once(&self.program)
            .chain(&self.prefix)
            .map(|part| part.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub struct Finished {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Output beyond the limit was dropped.
    pub truncated: bool,
}

/// Reads all of `source`, keeping at most `limit` bytes.
fn drain(mut source: impl Read + Send + 'static, limit: usize) -> JoinHandle<(Vec<u8>, bool)> {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut truncated = false;
        let mut chunk = [0_u8; 8192];
        loop {
            match source.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = limit.saturating_sub(kept.len());
                    if n > room {
                        truncated = true;
                    }
                    kept.extend_from_slice(chunk.get(..n.min(room)).unwrap_or_default());
                }
            }
        }
        (kept, truncated)
    })
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Runs `python [prefix] [args]` in `dir` without a console window, with an isolated
/// interpreter (`-I`: no user site directory or environment variables changing module
/// lookup) and unbuffered output. Stdout and stderr keep at most `limit` bytes each.
pub fn run(
    python: &PythonCommand,
    args: &[OsString],
    dir: &Path,
    cancel: &CancelToken,
    timeout: Option<Duration>,
    limit: usize,
) -> Result<Finished, BridgeError> {
    let mut command = Command::new(&python.program);
    command
        .args(&python.prefix)
        .arg("-I")
        .arg("-u")
        .args(args)
        .current_dir(dir)
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes up behind the app.
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn()?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        stop(&mut child);
        return Err(BridgeError::Process("output pipes unavailable".into()));
    };
    let stdout = drain(stdout, limit);
    let stderr = drain(stderr, limit);
    let started = Instant::now();
    let status = loop {
        if cancel.is_cancelled() {
            stop(&mut child);
            return Err(BridgeError::Cancelled);
        }
        if let Some(limit) = timeout
            && started.elapsed() > limit
        {
            stop(&mut child);
            return Err(BridgeError::Timeout(limit));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(POLL),
            Err(error) => {
                stop(&mut child);
                return Err(error.into());
            }
        }
    };
    let (stdout, out_cut) = stdout
        .join()
        .map_err(|_| BridgeError::Process("output reader failed".into()))?;
    let (stderr, err_cut) = stderr
        .join()
        .map_err(|_| BridgeError::Process("output reader failed".into()))?;
    Ok(Finished {
        status,
        stdout,
        stderr,
        truncated: out_cut || err_cut,
    })
}
