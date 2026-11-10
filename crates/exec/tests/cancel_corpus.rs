//! Discover the ten slowest pinned streams, then time cooperative cancellation on each.
//! All vendor data remain read-only. Exports are redirected into this checkout's target/.
#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "corpus assertions fail loudly"
)]

use dms_exec::{CancelToken, ErrorKind, ExecContext, FileResolver, Plan};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn load(label: &str, relative: &str) -> (Stream, ExecContext) {
    let root = root();
    let path = root.join("corpus").join(label).join(relative);
    let bytes = std::fs::read(&path).unwrap();
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let mut dirs = vec![
        root.join("corpus").join(label),
        root.join("corpus/ibm-demo-data"),
    ];
    if let Some(home) = std::env::var_os("DMS_MODELER_HOME") {
        dirs.push(Path::new(&home).join("Demos"));
    }
    // Some saved paths reference another corpus label. Search the pinned labels after the
    // stream's own data, just as relocation does for streams from another machine.
    let mut labels: Vec<_> = std::fs::read_dir(root.join("corpus"))
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name() != "database" && entry.path().is_dir())
        .map(|entry| entry.path())
        .collect();
    labels.sort();
    dirs.extend(labels);
    let context = ExecContext {
        files: FileResolver {
            data_dirs: dirs.into_iter().filter(|dir| dir.is_dir()).collect(),
        },
        attachments: Arc::new(
            archive
                .attachments
                .into_iter()
                .map(|a| (a.name, a.bytes.into()))
                .collect(),
        ),
        stream_properties: stream.properties.clone().map(Arc::new),
        export_dir: Some(
            root.join("target/cancel-corpus")
                .join(label)
                .join(relative)
                .join("exports"),
        ),
        ..Default::default()
    };
    (stream, context)
}

#[test]
fn ten_slowest_streams_stop_within_the_latency_budget() {
    let manifest = std::fs::read_to_string(root().join("xtask/corpus.tsv")).unwrap();
    let mut timings = Vec::new();
    let mut measured = 0usize;
    for line in manifest.lines().filter(|line| !line.starts_with('#')) {
        let mut cells = line.split('\t');
        let label = cells.next().unwrap();
        let relative = cells.next().unwrap();
        if label == "database" || !relative.ends_with(".str") {
            continue;
        }
        let (stream, context) = load(label, relative);
        let plan = Plan::new(&stream);
        let targets = plan.terminals_in_run_order();
        let started = Instant::now();
        let results = plan.evaluate(&targets, &context);
        let elapsed = started.elapsed();
        measured += 1;
        if measured.is_multiple_of(100) {
            println!("timed {measured} streams (database excluded)");
        }
        // Keep just ten timings, not the data or models of the whole corpus.
        timings.push((elapsed, label.to_string(), relative.to_string()));
        timings.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        timings.truncate(10);
        drop(results);
    }
    assert_eq!(timings.len(), 10);
    println!(
        "slowest 10 ({}):",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    for (elapsed, label, path) in &timings {
        println!("  {label}/{path}: {:.3} ms", elapsed.as_secs_f64() * 1000.0);
    }
    let limit = if cfg!(debug_assertions) {
        Duration::from_secs(1)
    } else {
        Duration::from_millis(250)
    };
    let mut maximum = Duration::ZERO;
    let mut cancelled_runs = 0usize;
    for (elapsed, label, path) in &timings {
        let (stream, context) = load(label, path);
        let plan = Plan::new(&stream);
        let targets = plan.terminals_in_run_order();
        for delay in [
            Duration::from_millis(50),
            Duration::from_millis(200),
            *elapsed / 2,
        ] {
            let context = ExecContext {
                cancel: CancelToken::new(),
                ..context.clone()
            };
            let token = context.cancel.clone();
            let (finished, done) = mpsc::channel();
            let (results, returned, cancelled) = std::thread::scope(|scope| {
                let timer = scope.spawn(move || {
                    if done.recv_timeout(delay).is_ok() {
                        return None;
                    }
                    let when = Instant::now();
                    token.cancel();
                    Some(when)
                });
                let results = plan.evaluate(&targets, &context);
                let returned = Instant::now();
                let _ = finished.send(());
                (results, returned, timer.join().unwrap())
            });
            if let Some(cancelled) = cancelled {
                let latency = returned.saturating_duration_since(cancelled);
                maximum = maximum.max(latency);
                cancelled_runs += 1;
                println!(
                    "  {label}/{path} cancel at {:.3} ms -> {:.3} ms",
                    delay.as_secs_f64() * 1000.0,
                    latency.as_secs_f64() * 1000.0
                );
                assert!(
                    latency <= limit,
                    "{label}/{path}: cancellation took {latency:?}, limit {limit:?}"
                );
                assert!(
                    results.values().any(
                        |result| matches!(result, Err(error) if error.kind == ErrorKind::Cancelled)
                    ),
                    "{label}/{path}: token was cancelled but no node observed it"
                );
            } else {
                println!(
                    "  {label}/{path}: finished before {:.3} ms cancellation",
                    delay.as_secs_f64() * 1000.0
                );
            }
        }
    }
    println!(
        "maximum cancellation latency: {:.3} ms; {cancelled_runs}/30 requests reached running streams; limit {} ms",
        maximum.as_secs_f64() * 1000.0,
        limit.as_millis()
    );
    assert!(cancelled_runs > 0, "no request reached a running stream");
}
