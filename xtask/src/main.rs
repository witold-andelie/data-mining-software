//! Developer tasks: `cargo xtask <command>`.

mod bundle;
mod bundle_windows;
mod compare;
mod corpus;
mod layers;
mod notices;
mod oracle;
mod parity;
mod perf;
mod regress;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

type Result<T> = std::result::Result<T, String>;

const USAGE: &str = "usage: cargo xtask <command>

commands:
  ci                         fmt, clippy -D warnings, tests, layers, parity --check
  bundle-mac                 build target/dist/Data Mining.app and DataMining-<version>.dmg
                             (macOS; ad-hoc signed, not notarized)
  bundle-windows             build target/dist/windows/Data-Mining-<version>-windows-x64.zip
                             (Windows; static C runtime, not code-signed)
  layers                    check crate dependency layering
  perf                       release CPU frames (200/1000 nodes); 200-node p95 <= 4 ms
  parity [--check]           regenerate docs/parity-checklist.md (--check: fail if stale)
  corpus record <label> <dir> [ext,...]
                             write manifest entries for files under <dir> (default ext: str)
  corpus sync <label> <dir>  copy manifest files from <dir> into corpus/<label>/ and verify
  corpus verify              verify corpus/ against xtask/corpus.tsv
  oracle <label> <path> [--timeout <secs>]
                             run a pinned corpus stream through IBM SPSS Modeler (Windows,
                             DMS_MODELER_HOME) and keep CSV/PMML results in oracle/
  probe <name>               run xtask/oracle/probes/<name>.py in Modeler; output goes to
                             oracle/probes/<name>/ (differential format observation)
  test-corpus                verify corpus/ and run the corpus-gated tests
  compare <label> <path> [--strict]
                             run a corpus stream with dms-cli and compare the data reaching
                             each terminal node with the Modeler oracle export
  regress [--label <label>]... [--jobs <n>] [--out <file>] [--via-dms]
                             compare every stream that has an oracle export: match,
                             statistical (a statistical-parity builder trains), known GAP
                             (xtask/known-gaps.tsv) or regression; fails on a regression or
                             a stale known-gap entry (--out: per-stream TSV; --via-dms: convert then run)";

/// Crates with real-file and oracle tests behind their `corpus` feature.
const CORPUS_CRATES: &[&str] = &[
    "dms-strfmt",
    "dms-stream",
    "dms-sav",
    "dms-io",
    "dms-clem",
    "dms-pmml",
    "dms-algo",
    "dms-exec",
];

fn test_corpus() -> Result<()> {
    corpus::run(&["verify".to_string()])?;
    let features: Vec<String> = CORPUS_CRATES
        .iter()
        .map(|krate| format!("{krate}/corpus"))
        .collect();
    let features = features.join(",");
    let mut args = vec!["test"];
    for krate in CORPUS_CRATES {
        args.push("-p");
        args.push(krate);
    }
    args.push("--features");
    args.push(&features);
    cargo(&args)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    let outcome = match args.first().map(String::as_str) {
        Some("ci") => ci(),
        Some("bundle-mac") => bundle::run(rest),
        Some("bundle-windows") => bundle_windows::run(rest),
        Some("layers") => layers::check(),
        Some("perf") => perf::run(rest),
        Some("parity") => parity::run(rest.iter().any(|a| a == "--check")),
        Some("corpus") => corpus::run(rest),
        Some("oracle") => oracle::run(rest),
        Some("probe") => oracle::probe(rest),
        Some("compare") => compare::run(rest),
        Some("regress") => regress::run(rest),
        Some("test-corpus") => test_corpus(),
        _ => Err(USAGE.to_string()),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

fn ci() -> Result<()> {
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(&["test", "--workspace"])?;
    layers::check()?;
    parity::run(true)?;
    println!("xtask ci: all gates passed");
    Ok(())
}

fn cargo_program() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

fn cargo(args: &[&str]) -> Result<()> {
    run_cargo(args, true, &[])
}

/// Like `cargo`, without echoing the command: `regress` runs hundreds of them.
fn cargo_quiet(args: &[&str]) -> Result<()> {
    run_cargo(args, false, &[])
}

/// A release build for binaries that are handed to other people: the paths of this machine (the
/// Cargo home with the dependency sources, and the checkout) are replaced in the binaries, which
/// otherwise carry them in panic locations and debug output, with the user name inside.
fn cargo_release(args: &[&str], extra_flags: &[&str]) -> Result<()> {
    let flags = release_rustflags(
        extra_flags,
        std::env::var("CARGO_ENCODED_RUSTFLAGS").ok().as_deref(),
        std::env::var("RUSTFLAGS").ok().as_deref(),
        &cargo_home()?,
        &workspace_root()?,
    );
    run_cargo(args, true, &[("CARGO_ENCODED_RUSTFLAGS", flags)])
}

fn cargo_home() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        return Ok(PathBuf::from(home));
    }
    let user = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    user.map(|user| PathBuf::from(user).join(".cargo"))
        .ok_or_else(|| "cannot find the Cargo home (set CARGO_HOME)".to_string())
}

/// The value of `CARGO_ENCODED_RUSTFLAGS` (flags joined by 0x1f, which allows spaces in paths):
/// the flags the environment already sets, then the path remapping. Cargo prefers this variable
/// over `RUSTFLAGS`, so both are carried over.
fn release_rustflags(
    extra: &[&str],
    encoded: Option<&str>,
    plain: Option<&str>,
    cargo_home: &Path,
    root: &Path,
) -> String {
    let mut flags: Vec<String> = match (encoded, plain) {
        (Some(encoded), _) => encoded.split('\x1f').map(str::to_string).collect(),
        (None, Some(plain)) => plain.split_whitespace().map(str::to_string).collect(),
        (None, None) => Vec::new(),
    };
    flags.retain(|flag| !flag.is_empty());
    flags.extend(extra.iter().map(|flag| (*flag).to_string()));
    flags.push(format!(
        "--remap-path-prefix={}=/cargo",
        cargo_home.display()
    ));
    flags.push(format!("--remap-path-prefix={}=/build", root.display()));
    flags.join("\x1f")
}

fn run_cargo(args: &[&str], echo: bool, env: &[(&str, String)]) -> Result<()> {
    let mut command = Command::new(cargo_program());
    command.args(args).current_dir(workspace_root()?);
    command.envs(env.iter().map(|(name, value)| (*name, value)));
    if echo {
        println!("$ cargo {}", args.join(" "));
        let status = command
            .status()
            .map_err(|e| format!("cannot run cargo {}: {e}", args.join(" ")))?;
        return if status.success() {
            Ok(())
        } else {
            Err(format!("cargo {} failed ({status})", args.join(" ")))
        };
    }
    let output = command
        .output()
        .map_err(|e| format!("cannot run cargo {}: {e}", args.join(" ")))?;
    if output.status.success() {
        return Ok(());
    }
    // Keep the child's own report: `regress` prints one line per stream, not a wall of text.
    let mut message = format!("cargo {} failed ({})", args.join(" "), output.status);
    for (name, bytes) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
        let text = String::from_utf8_lossy(bytes);
        let text = text.trim();
        if !text.is_empty() {
            message.push_str(&format!("; {name}: {text}"));
        }
    }
    Err(message)
}

fn workspace_root() -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| "xtask manifest has no parent directory".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_rustflags_remap_the_machine_paths_and_keep_the_flags_already_set() {
        let home = Path::new("/Users/some user/.cargo");
        let root = Path::new("/Users/some user/dev/dms");
        let remap = "--remap-path-prefix=/Users/some user/.cargo=/cargo\x1f\
                     --remap-path-prefix=/Users/some user/dev/dms=/build";
        assert_eq!(release_rustflags(&[], None, None, home, root), remap);
        // the plain variable is split on white space, the encoded one on 0x1f and wins
        assert_eq!(
            release_rustflags(&[], None, Some("-C target-cpu=native"), home, root),
            format!("-C\x1ftarget-cpu=native\x1f{remap}")
        );
        assert_eq!(
            release_rustflags(&[], Some("-Cdebuginfo=0"), Some("-C ignored"), home, root),
            format!("-Cdebuginfo=0\x1f{remap}")
        );
        // flags the task itself needs come after the environment's and before the remapping
        assert_eq!(
            release_rustflags(
                &["-C", "target-feature=+crt-static"],
                None,
                None,
                home,
                root
            ),
            format!("-C\x1ftarget-feature=+crt-static\x1f{remap}")
        );
    }
}
