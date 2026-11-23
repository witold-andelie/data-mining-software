//! `cargo xtask bundle-windows`: the Windows x64 distribution archive.
//!
//! Builds `dms` and `dms-cli` in release mode with the C runtime linked in (so no Visual C++
//! redistributable is needed) and the paths of the build machine removed, and writes under
//! `target/dist/windows/`:
//!
//! - `Data-Mining-<version>-windows-x64/` with `dms.exe`, `dms-cli.exe`, `LICENSE`,
//!   `THIRD-PARTY-LICENSES.txt` (every Rust dependency with its licence) and `README.txt`;
//! - `Data-Mining-<version>-windows-x64.zip` of that folder and its `.sha256`.
//!
//! The programs are not code-signed. Before the archive is written the executables are checked:
//! they must not import the dynamic C runtime and must not contain the Cargo home or the
//! checkout path.

use crate::notices;
use crate::{Result, cargo_release, workspace_root};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const PLATFORM: &str = "x86_64-pc-windows-msvc";

const README: &str = "Data Mining\r\n\
===========\r\n\
\r\n\
A data-mining workbench that opens, runs and edits IBM SPSS Modeler 19 streams.\r\n\
Not affiliated with IBM. Source code and documentation:\r\n\
https://github.com/witold-andelie/data-mining-software\r\n\
\r\n\
Run it\r\n\
------\r\n\
Double-click dms.exe, or from a terminal:  dms.exe path\\to\\stream.str\r\n\
No stream at hand?  dms.exe --synthetic 24   opens a generated one.\r\n\
dms-cli.exe runs a stream without a window:  dms-cli.exe run stream.str --out results\r\n\
\r\n\
The first time\r\n\
--------------\r\n\
The programs are not code-signed, so Windows SmartScreen may say it protected your PC.\r\n\
Check the download against the .sha256 file first (PowerShell: Get-FileHash <file>), then\r\n\
choose \"More info\" and \"Run anyway\". Some antivirus programs also flag new unsigned programs.\r\n\
\r\n\
Licence\r\n\
-------\r\n\
PolyForm Noncommercial License 1.0.0 (file LICENSE): free for individuals and educational\r\n\
institutions, no commercial use. The Rust libraries inside the programs and their licences are\r\n\
listed in THIRD-PARTY-LICENSES.txt.\r\n";

pub fn run(_args: &[String]) -> Result<()> {
    if !cfg!(windows) {
        return Err("bundle-windows runs on Windows only".into());
    }
    let root = workspace_root()?;
    let license = root.join("LICENSE");
    if !license.is_file() {
        return Err(format!(
            "repository licence file {} is missing",
            license.display()
        ));
    }
    let version = env!("CARGO_PKG_VERSION");
    cargo_release(
        &["build", "--release", "-p", "dms", "-p", "dms-cli"],
        &["-C", "target-feature=+crt-static"],
    )?;

    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let name = format!("Data-Mining-{version}-windows-x64");
    let dist = target.join("dist").join("windows");
    let stage = dist.join(&name);
    let archive = dist.join(format!("{name}.zip"));
    let checksum = dist.join(format!("{name}.zip.sha256"));
    // only our own staging folder and archive are replaced
    for path in [&archive, &checksum] {
        if path.exists() {
            fs::remove_file(path).map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
        }
    }
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|e| format!("cannot clear {}: {e}", stage.display()))?;
    }
    fs::create_dir_all(&stage).map_err(|e| format!("cannot create {}: {e}", stage.display()))?;

    for program in ["dms.exe", "dms-cli.exe"] {
        let from = target.join("release").join(program);
        check_executable(&from, &root)?;
        copy(&from, &stage.join(program))?;
    }
    copy(&license, &stage.join("LICENSE"))?;
    write(
        &stage.join("THIRD-PARTY-LICENSES.txt"),
        &notices::rust_dependencies(&["dms", "dms-cli"], PLATFORM)?,
    )?;
    write(&stage.join("README.txt"), README)?;

    let tar = Path::new(&std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
        .join("System32")
        .join("tar.exe");
    let status = Command::new(&tar)
        .args(["-a", "-c", "-f"])
        .arg(&archive)
        .arg("-C")
        .arg(&dist)
        .arg(&name)
        .status()
        .map_err(|e| format!("cannot run {}: {e}", tar.display()))?;
    if !status.success() {
        return Err(format!("{} failed ({status})", tar.display()));
    }
    let bytes =
        fs::read(&archive).map_err(|e| format!("cannot read {}: {e}", archive.display()))?;
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    write(&checksum, &format!("{digest} *{name}.zip\n"))?;
    println!(
        "xtask bundle-windows: {} ({} bytes, sha256 {digest})",
        archive.display(),
        bytes.len()
    );
    Ok(())
}

/// Refuse an executable that needs the Visual C++ redistributable or carries a path of this
/// machine.
fn check_executable(path: &Path, root: &Path) -> Result<()> {
    let bytes = fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let contains = |needle: &str| {
        !needle.is_empty()
            && bytes
                .windows(needle.len())
                .any(|window| window == needle.as_bytes())
    };
    for dynamic in ["VCRUNTIME140", "MSVCP140", "api-ms-win-crt-"] {
        if contains(dynamic) {
            return Err(format!(
                "{} imports the dynamic C runtime ({dynamic}); the static runtime was not linked",
                path.display()
            ));
        }
    }
    let cargo_home = crate::cargo_home()?;
    for (what, prefix) in [
        ("the Cargo home", &cargo_home),
        ("the checkout", &root.to_path_buf()),
    ] {
        if contains(&prefix.to_string_lossy()) {
            return Err(format!(
                "{} contains the path of {what} ({})",
                path.display(),
                prefix.display()
            ));
        }
    }
    Ok(())
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("cannot copy {} to {}: {e}", from.display(), to.display()))
}

fn write(path: &Path, text: &str) -> Result<()> {
    fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}
