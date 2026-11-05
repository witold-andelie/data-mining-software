//! `cargo xtask regress [--label <label>]... [--jobs <n>] [--out <file>]`: run every corpus
//! stream that has a Modeler oracle export and classify it.
//!
//! Classes:
//! - `match`: every terminal input matches the oracle export;
//! - `statistical`: every terminal input was produced and compared, at least one differs, and the
//!   stream trains a builder held to statistical parity (`dms_exec::registry::is_statistical`,
//!   `docs/statistical-parity.md`);
//! - `known-gap`: a mismatch (or missing terminal output) listed in `xtask/known-gaps.tsv` with
//!   its reason;
//! - `regression`: any other mismatch, a missing terminal output on an unlisted stream, or a
//!   stream that cannot be run.
//!
//! The command exits non-zero on a regression, and on a `known-gaps.tsv` entry whose stream now
//! matches (a stale entry). `--label` limits the run to some corpus labels (and the stale check
//! to their entries); `--jobs` bounds concurrent `dms-cli` runs (default 2); `--out` writes every
//! stream's class as TSV.

use crate::compare::{self, StreamComparison};
use crate::{Result, oracle, workspace_root};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Concurrent `dms-cli` runs when `--jobs` is not given.
const DEFAULT_JOBS: usize = 2;
/// Upper bound on threads: an input is never trusted to size an allocation.
const MAX_JOBS: usize = 64;
/// Mismatching terminal-input lines printed per stream (`--out` keeps all of them).
const MAX_PROBLEM_LINES: usize = 3;
/// Streams between progress lines on stderr.
const PROGRESS_EVERY: usize = 100;

pub fn run(args: &[String]) -> Result<()> {
    let options = Options::parse(args)?;
    let root = workspace_root()?;
    let gaps = KnownGaps::read(&root.join("xtask").join("known-gaps.tsv"))?;
    let streams = oracle::streams(&options.labels)?;
    if streams.is_empty() {
        return Err(if options.labels.is_empty() {
            "no oracle streams found (run `cargo xtask oracle <label> <path>` first)".into()
        } else {
            format!(
                "no oracle streams for label(s) {}",
                options.labels.join(", ")
            )
        });
    }
    let results = compare_all(&streams, &gaps, options.jobs, &root)?;
    report(&results, &gaps, &options, &root)
}

fn usage() -> String {
    "usage: regress [--label <label>]... [--jobs <n>] [--out <file>]".into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    /// Corpus labels to compare (empty: every label).
    labels: Vec<String>,
    jobs: usize,
    /// Per-stream TSV, relative to the workspace root when not absolute.
    out: Option<PathBuf>,
}

impl Options {
    fn parse(args: &[String]) -> Result<Options> {
        let mut options = Options {
            labels: Vec::new(),
            jobs: DEFAULT_JOBS,
            out: None,
        };
        let mut rest = args.iter();
        while let Some(arg) = rest.next() {
            match arg.as_str() {
                "--label" => {
                    let label = rest.next().ok_or("--label needs a value")?;
                    if label.is_empty()
                        || label.contains('/')
                        || label.contains('\\')
                        || label.chars().any(char::is_whitespace)
                    {
                        return Err(format!("invalid label {label:?}"));
                    }
                    options.labels.push(label.clone());
                }
                "--jobs" => {
                    let value = rest.next().ok_or("--jobs needs a value")?;
                    let jobs = value
                        .parse::<usize>()
                        .map_err(|e| format!("invalid --jobs {value:?}: {e}"))?;
                    if !(1..=MAX_JOBS).contains(&jobs) {
                        return Err(format!("--jobs must be between 1 and {MAX_JOBS}"));
                    }
                    options.jobs = jobs;
                }
                "--out" => {
                    let value = rest.next().ok_or("--out needs a file name")?;
                    if value.is_empty() {
                        return Err("--out needs a file name".into());
                    }
                    options.out = Some(PathBuf::from(value));
                }
                other => return Err(format!("unknown argument {other:?}; {}", usage())),
            }
        }
        Ok(options)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Match,
    Statistical,
    KnownGap,
    Regression,
}

impl Class {
    fn label(self) -> &'static str {
        match self {
            Class::Match => "match",
            Class::Statistical => "statistical",
            Class::KnownGap => "known-gap",
            Class::Regression => "regression",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamResult {
    label: String,
    stem: String,
    class: Class,
    /// `8/9 terminal inputs match Modeler`, the known-gap reason, or the run error.
    detail: String,
    /// One line per terminal input that did not match.
    problems: Vec<String>,
}

impl StreamResult {
    /// This result as `label<TAB>stem<TAB>class<TAB>detail<TAB>problems`, fields escaped.
    fn tsv(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}",
            tsv_field(&self.label),
            tsv_field(&self.stem),
            self.class.label(),
            tsv_field(&self.detail),
            tsv_field(&self.problems.join(" | "))
        )
    }
}

/// The class of one stream. A run failure is a regression even when the stream is listed;
/// otherwise a known-gap entry, which names the cause, takes precedence over a statistical
/// builder: `team/decision_trees_heart` trains a statistical C&R Tree, but its records already
/// differ upstream at the unseeded Balance node. A statistical builder only explains value
/// differences in produced terminal inputs (`all_produced`), not missing outputs or read errors.
fn classify(comparison: &Result<StreamComparison>, statistical: bool, listed: bool) -> Class {
    match comparison {
        Err(_) => Class::Regression,
        Ok(comparison) if comparison.is_match() => Class::Match,
        Ok(_) if listed => Class::KnownGap,
        Ok(comparison) if statistical && comparison.all_produced() => Class::Statistical,
        Ok(_) => Class::Regression,
    }
}

fn compare_all(
    streams: &[oracle::OracleStream],
    gaps: &KnownGaps,
    jobs: usize,
    root: &Path,
) -> Result<Vec<StreamResult>> {
    let total = streams.len();
    let workers = jobs.min(total);
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let slots: Mutex<Vec<Option<StreamResult>>> = Mutex::new((0..total).map(|_| None).collect());
    eprintln!("xtask regress: comparing {total} streams with {workers} job(s)");
    std::thread::scope(|scope| -> Result<()> {
        let (next, done, slots) = (&next, &done, &slots);
        for worker in 0..workers {
            std::thread::Builder::new()
                .name(format!("regress-{worker}"))
                .spawn_scoped(scope, move || {
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(stream) = streams.get(index) else {
                            break;
                        };
                        let result = compare_one(stream, gaps, root);
                        slots
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())[index] = Some(result);
                        let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
                        if finished.is_multiple_of(PROGRESS_EVERY) {
                            eprintln!("xtask regress: {finished}/{total} streams compared");
                        }
                    }
                })
                .map_err(|e| format!("cannot start a worker thread: {e}"))?;
        }
        Ok(())
    })?;
    let slots = slots
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut results = Vec::with_capacity(total);
    for (index, slot) in slots.into_iter().enumerate() {
        match slot {
            Some(result) => results.push(result),
            None => {
                return Err(format!(
                    "stream {} was not compared",
                    streams
                        .get(index)
                        .map_or("?", |stream| stream.path.as_str())
                ));
            }
        }
    }
    Ok(results)
}

fn compare_one(stream: &oracle::OracleStream, gaps: &KnownGaps, root: &Path) -> StreamResult {
    let corpus = root.join("corpus").join(&stream.label).join(&stream.path);
    let statistical = statistical_builder(&corpus);
    let comparison = compare::compare_stream(&stream.label, &stream.path, false);
    let listed = gaps.reason(&stream.label, &stream.stem);
    let class = classify(
        &comparison,
        statistical.as_ref().is_ok_and(|found| found.is_some()),
        listed.is_some(),
    );
    let mut problems = Vec::new();
    let detail = match &comparison {
        Err(error) => error.clone(),
        Ok(comparison) => {
            problems = comparison.problems();
            if let Err(error) = &statistical {
                problems.push(format!("cannot look for statistical builders: {error}"));
            }
            let matched = format!(
                "{}/{} terminal inputs match Modeler",
                comparison.matched, comparison.total
            );
            match class {
                Class::Match | Class::Regression => matched,
                Class::Statistical => match &statistical {
                    Ok(Some(id)) => format!("{matched} ({id} is held to statistical parity)"),
                    _ => matched,
                },
                Class::KnownGap => match listed {
                    Some(reason) => format!("{matched}: {reason}"),
                    None => matched,
                },
            }
        }
    };
    StreamResult {
        label: stream.label.clone(),
        stem: stream.stem.clone(),
        class,
        detail: one_line(&detail),
        problems,
    }
}

/// A console or TSV line: a child process error can span several lines.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The catalog id of the first build node held to statistical parity, if the stream trains one.
/// A nugget only scores the model saved in the stream, so it cannot differ because of a
/// generator or an optimizer.
fn statistical_builder(stream: &Path) -> Result<Option<String>> {
    let bytes = fs::read(stream).map_err(|e| format!("cannot read {}: {e}", stream.display()))?;
    let archive = dms_strfmt::read_archive(&bytes, &dms_strfmt::ArchiveLimits::default())
        .map_err(|e| format!("{}: {e}", stream.display()))?;
    let stream = dms_stream::Stream::from_document(&archive.document)
        .map_err(|e| format!("{}: {e}", stream.display()))?;
    for node in stream.all_nodes() {
        if node.model_builder.is_some() {
            continue;
        }
        if let dms_engine::catalog::Resolution::Node(spec) = dms_engine::catalog::resolve(node)
            && dms_exec::registry::is_statistical(spec.id)
        {
            return Ok(Some(spec.id.to_string()));
        }
    }
    Ok(None)
}

/// `xtask/known-gaps.tsv`: one `label<TAB>stem<TAB>reason` line per mismatch we know about and
/// accept for now. Missing file: no known gaps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct KnownGaps {
    entries: BTreeMap<(String, String), String>,
}

impl KnownGaps {
    fn read(path: &Path) -> Result<KnownGaps> {
        match fs::read_to_string(path) {
            Ok(text) => {
                KnownGaps::parse(&text).map_err(|error| format!("{}: {error}", path.display()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(KnownGaps::default()),
            Err(error) => Err(format!("cannot read {}: {error}", path.display())),
        }
    }

    fn parse(text: &str) -> Result<KnownGaps> {
        let mut entries = BTreeMap::new();
        for (index, line) in text.lines().enumerate() {
            let number = index + 1;
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            let [label, stem, reason] = fields.as_slice() else {
                return Err(format!(
                    "line {number}: expected label<TAB>stem<TAB>reason, found {} field(s)",
                    fields.len()
                ));
            };
            for (what, value) in [("label", label), ("stem", stem)] {
                if value.is_empty()
                    || value.contains('/')
                    || value.contains('\\')
                    || value.chars().any(char::is_whitespace)
                {
                    return Err(format!(
                        "line {number}: {what} {value:?} must be non-empty and contain no \
                         whitespace or path separator"
                    ));
                }
            }
            let reason = reason.trim();
            if reason.is_empty() {
                return Err(format!("line {number}: the reason is empty"));
            }
            let key = (label.to_string(), stem.to_string());
            if entries.insert(key, reason.to_string()).is_some() {
                return Err(format!("line {number}: duplicate entry for {label}/{stem}"));
            }
        }
        Ok(KnownGaps { entries })
    }

    fn reason(&self, label: &str, stem: &str) -> Option<&str> {
        self.entries
            .get(&(label.to_string(), stem.to_string()))
            .map(String::as_str)
    }
}

fn report(
    results: &[StreamResult],
    gaps: &KnownGaps,
    options: &Options,
    root: &Path,
) -> Result<()> {
    for result in results.iter().filter(|result| result.class != Class::Match) {
        println!(
            "{:<11} {}/{}: {}",
            result.class.label(),
            result.label,
            result.stem,
            result.detail
        );
        for problem in result.problems.iter().take(MAX_PROBLEM_LINES) {
            println!("  {problem}");
        }
        if result.problems.len() > MAX_PROBLEM_LINES {
            println!(
                "  ... and {} more mismatch line(s)",
                result.problems.len() - MAX_PROBLEM_LINES
            );
        }
    }
    if let Some(out) = &options.out {
        let out = if out.is_absolute() {
            out.clone()
        } else {
            root.join(out)
        };
        write_tsv(&out, results)?;
    }
    let stale = stale_entries(results, gaps, &options.labels);
    for line in &stale {
        println!("stale known GAP: {line}");
    }
    let count = |class: Class| {
        results
            .iter()
            .filter(|result| result.class == class)
            .count()
    };
    let (matched, statistical, gap, regressions) = (
        count(Class::Match),
        count(Class::Statistical),
        count(Class::KnownGap),
        count(Class::Regression),
    );
    println!(
        "xtask regress: {} streams: {matched} match, {statistical} statistical, \
         {gap} known GAPs, {regressions} regressions",
        results.len()
    );
    let mut failures = Vec::new();
    if regressions > 0 {
        failures.push(format!("{regressions} regressions"));
    }
    if !stale.is_empty() {
        failures.push(format!("{} stale known GAP entries", stale.len()));
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

/// Known-gap entries that no longer describe their stream: it now matches, fails to run, or
/// was not compared (an entry for a removed oracle or a label outside `--label`).
fn stale_entries(results: &[StreamResult], gaps: &KnownGaps, labels: &[String]) -> Vec<String> {
    let mut stale = Vec::new();
    for ((label, stem), reason) in &gaps.entries {
        if !labels.is_empty() && !labels.contains(label) {
            continue;
        }
        let result = results
            .iter()
            .find(|result| &result.label == label && &result.stem == stem);
        match result {
            Some(result) if result.class == Class::KnownGap => {}
            Some(result) => stale.push(format!(
                "{label}/{stem} is listed as a known GAP but is now {} ({reason})",
                result.class.label()
            )),
            None => stale.push(format!(
                "{label}/{stem} is listed as a known GAP but has no oracle stream to compare"
            )),
        }
    }
    stale
}

fn write_tsv(path: &Path, results: &[StreamResult]) -> Result<()> {
    let mut text = String::from("label\tstem\tclass\tdetail\tproblems\n");
    for result in results {
        text.push_str(&result.tsv());
        text.push('\n');
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// A TSV field cannot hold tabs or line ends; oracle CSV cells may contain both.
fn tsv_field(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c == '\t' || c == '\r' || c == '\n' {
                ' '
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comparison_with_statuses(statuses: &[&str]) -> StreamComparison {
        let matched = statuses.iter().filter(|&&status| status == "match").count();
        let nodes = statuses
            .iter()
            .enumerate()
            .map(|(index, status)| {
                serde_json::json!({
                    "uid": format!("n{index}"),
                    "type": "table",
                    "result": { "status": status }
                })
            })
            .collect();
        StreamComparison {
            matched,
            total: statuses.len(),
            nodes,
        }
    }

    fn comparison(matched: usize, total: usize) -> StreamComparison {
        let mut statuses = vec!["match"; matched];
        statuses.resize(total, "mismatch");
        comparison_with_statuses(&statuses)
    }

    #[test]
    fn classifies_by_precedence() {
        let matched: Result<StreamComparison> = Ok(comparison(2, 2));
        let mismatch: Result<StreamComparison> = Ok(comparison(1, 2));
        let failed: Result<StreamComparison> = Err("dms-cli: boom".into());
        assert_eq!(classify(&matched, true, true), Class::Match);
        assert_eq!(classify(&mismatch, true, true), Class::KnownGap);
        assert_eq!(classify(&mismatch, true, false), Class::Statistical);
        assert_eq!(classify(&mismatch, false, true), Class::KnownGap);
        assert_eq!(classify(&mismatch, false, false), Class::Regression);
        assert_eq!(classify(&failed, true, true), Class::Regression);
        // A manifest with no export matches, like `cargo xtask compare` reports 0/0.
        let empty: Result<StreamComparison> = Ok(comparison(0, 0));
        assert_eq!(classify(&empty, false, false), Class::Match);
    }

    #[test]
    fn statistical_requires_all_terminals_produced() {
        let missing_only: Result<StreamComparison> =
            Ok(comparison_with_statuses(&["not produced"]));
        let mixed_missing: Result<StreamComparison> = Ok(comparison_with_statuses(&[
            "match",
            "mismatch",
            "not produced",
        ]));
        let read_error: Result<StreamComparison> =
            Ok(comparison_with_statuses(&["match", "error"]));
        let all_compared: Result<StreamComparison> =
            Ok(comparison_with_statuses(&["match", "mismatch"]));

        for unproduced in [&missing_only, &mixed_missing, &read_error] {
            assert!(!unproduced.as_ref().expect("ok").all_produced());
            assert_eq!(classify(unproduced, true, false), Class::Regression);
            assert_eq!(classify(unproduced, true, true), Class::KnownGap);
            assert_eq!(classify(unproduced, false, false), Class::Regression);
            assert_eq!(classify(unproduced, false, true), Class::KnownGap);
        }
        assert!(all_compared.as_ref().expect("ok").all_produced());
        assert_eq!(classify(&all_compared, true, false), Class::Statistical);
        assert_eq!(classify(&all_compared, true, true), Class::KnownGap);
    }

    #[test]
    fn parses_options() {
        let parse =
            |args: &[&str]| Options::parse(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>());
        let defaults = parse(&[]).expect("defaults");
        assert_eq!(defaults.jobs, DEFAULT_JOBS);
        assert!(defaults.labels.is_empty());
        assert_eq!(defaults.out, None);
        let options = parse(&[
            "--label",
            "records",
            "--label",
            "random",
            "--jobs",
            "8",
            "--out",
            "target/r.tsv",
        ])
        .expect("options");
        assert_eq!(options.labels, ["records", "random"]);
        assert_eq!(options.jobs, 8);
        assert_eq!(options.out, Some(PathBuf::from("target/r.tsv")));
        assert!(parse(&["--jobs", "0"]).is_err());
        assert!(parse(&["--jobs", "65"]).is_err());
        assert!(parse(&["--jobs", "many"]).is_err());
        assert!(parse(&["--label"]).is_err());
        assert!(parse(&["--label", "a/b"]).is_err());
        assert!(parse(&["extra"]).is_err());
    }

    #[test]
    fn parses_known_gaps() {
        let gaps = KnownGaps::parse(
            "# label\tstem\treason\n\nrecords\tsort_multi_keys\ttie order\nrandom\tpart_default\tunseeded\n",
        )
        .expect("valid file");
        assert_eq!(gaps.entries.len(), 2);
        assert_eq!(gaps.reason("records", "sort_multi_keys"), Some("tie order"));
        assert_eq!(gaps.reason("records", "missing"), None);
        assert_eq!(gaps.reason("other", "sort_multi_keys"), None);
    }

    #[test]
    fn rejects_malformed_known_gaps() {
        for text in [
            "records\tonly_two_fields\n",
            "a\tb\treason\textra\n",
            "\tb\treason\n",
            "a\t\treason\n",
            "a b\tc\treason\n",
            "a\tb c\treason\n",
            "a/b\tc\treason\n",
            "a\tb\\c\treason\n",
            "a\tb\t \n",
        ] {
            assert!(
                KnownGaps::parse(text).is_err(),
                "{text:?} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_duplicate_known_gaps() {
        let error = KnownGaps::parse("a\tb\tone\na\tb\ttwo\n").expect_err("duplicate");
        assert!(error.contains("duplicate"), "{error}");
    }

    #[test]
    fn writes_tsv_with_escaped_fields() {
        let result = StreamResult {
            label: "records".into(),
            stem: "sort".into(),
            class: Class::KnownGap,
            detail: "1/2 terminal inputs\tmatch".into(),
            problems: vec!["a\nb".into()],
        };
        assert_eq!(
            result.tsv(),
            "records\tsort\tknown-gap\t1/2 terminal inputs match\ta b"
        );
    }

    #[test]
    fn reports_stale_known_gaps() {
        let gaps = KnownGaps::parse(
            "records\ta\tstill open\nrecords\tb\tfixed\nrecords\tc\tgone\nrandom\td\tother label\n",
        )
        .expect("known gaps");
        let results = vec![
            StreamResult {
                label: "records".into(),
                stem: "a".into(),
                class: Class::KnownGap,
                detail: String::new(),
                problems: Vec::new(),
            },
            StreamResult {
                label: "records".into(),
                stem: "b".into(),
                class: Class::Match,
                detail: String::new(),
                problems: Vec::new(),
            },
        ];
        let stale = stale_entries(&results, &gaps, &[]);
        assert_eq!(stale.len(), 3, "{stale:?}");
        assert!(
            stale
                .iter()
                .any(|line| line.contains("records/b") && line.contains("now match")),
            "{stale:?}"
        );
        assert!(
            stale.iter().any(|line| line.contains("records/c")),
            "{stale:?}"
        );
        // `--label records` checks only the entries of the labels it ran.
        let stale = stale_entries(&results, &gaps, &["records".to_string()]);
        assert_eq!(stale.len(), 2, "{stale:?}");
        let stale = stale_entries(&results, &gaps, &["random".to_string()]);
        assert_eq!(stale.len(), 1, "{stale:?}");
        assert!(stale[0].contains("random/d"), "{stale:?}");
    }

    /// Integration test on the shared corpus and oracle trees; skipped without them (macOS CI,
    /// fresh clones). Runs one real stream through `dms-cli`.
    #[test]
    fn compares_real_oracle_streams() {
        let Ok(root) = workspace_root() else {
            return;
        };
        if !root.join("oracle").is_dir() || !root.join("corpus").is_dir() {
            return;
        }
        let streams = oracle::streams(&[]).expect("enumerate oracle streams");
        assert!(!streams.is_empty(), "no oracle streams found");
        for stream in &streams {
            assert!(!stream.stem.is_empty());
            assert!(
                stream.dir.join("manifest.json").is_file(),
                "{}/{}",
                stream.label,
                stream.stem
            );
        }
        let Some(stream) = streams
            .iter()
            .find(|stream| stream.label == "records" && stream.stem == "sort_multi_keys")
        else {
            return;
        };
        let comparison =
            compare::compare_stream(&stream.label, &stream.path, false).expect("compare stream");
        assert!(
            comparison.is_match(),
            "records/sort_multi_keys: {:?}",
            comparison.problems()
        );

        let sequence = root
            .join("corpus")
            .join("train_next")
            .join("sequence_support10.str");
        if sequence.is_file() {
            assert_eq!(
                statistical_builder(&sequence).expect("parse sequence stream"),
                Some("sequence".to_string())
            );
        }
        let c50 = root
            .join("corpus")
            .join("c50_presets")
            .join("c50_wn_expert.str");
        if c50.is_file() {
            assert_eq!(statistical_builder(&c50).expect("parse C5.0 stream"), None);
        }
    }
}
