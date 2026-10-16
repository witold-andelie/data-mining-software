//! Developer tasks: `cargo xtask <command>`.

mod compare;
mod corpus;
mod layers;
mod oracle;
mod parity;

use std::path::PathBuf;
use std::process::{Command, ExitCode};

type Result<T> = std::result::Result<T, String>;

const USAGE: &str = "usage: cargo xtask <command>

commands:
  ci                         fmt, clippy -D warnings, tests, layers, parity --check
  layers                     check crate dependency layering
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
                             each terminal node with the Modeler oracle export";

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
        Some("layers") => layers::check(),
        Some("parity") => parity::run(rest.iter().any(|a| a == "--check")),
        Some("corpus") => corpus::run(rest),
        Some("oracle") => oracle::run(rest),
        Some("probe") => oracle::probe(rest),
        Some("compare") => compare::run(rest),
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
    println!("$ cargo {}", args.join(" "));
    let status = Command::new(cargo_program())
        .args(args)
        .current_dir(workspace_root()?)
        .status()
        .map_err(|e| format!("cannot run cargo {}: {e}", args.join(" ")))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo {} failed ({status})", args.join(" ")))
    }
}

fn workspace_root() -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| "xtask manifest has no parent directory".to_string())
}
