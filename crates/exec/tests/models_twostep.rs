//! TwoStep end to end: each `corpus/twostep/*` probe stream and `models/twostep_default` trains
//! its build node, and the nugget scores the training records with that model. The clusters must
//! hold the same records as Modeler's; the names must match where our CF-tree order is known to
//! agree (the GAP in `dms_algo::twostep`).

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use dms_types::Table;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Cases whose cluster names equal Modeler's.
const SAME_NAMES: &[&str] = &[
    "ts_drug8",
    "ts_drug8_rev",
    "ts_drug8_cont",
    "ts_drug9",
    "ts_drug20",
    "ts_drug50",
    "ts_prefix3",
    "ts_prefix4",
    "ts_prefix5",
    "ts_prefix6",
    "ts_prefix7",
    "ts_prefix8",
];

fn scored(label: &str, stream_path: &Path, uid: &str) -> Arc<Table> {
    let root = support::repo_root();
    let bytes = std::fs::read(stream_path).unwrap();
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let context = ExecContext {
        files: FileResolver {
            data_dirs: vec![root.join("corpus").join(label)],
        },
        stream_properties: stream.properties.clone().map(Arc::new),
        attachments: Arc::new(
            archive
                .attachments
                .into_iter()
                .map(|a| (a.name, a.bytes.into()))
                .collect(),
        ),
        decimal_places: Some(15),
        export_dir: None,
    };
    let results = Plan::new(&stream).evaluate(&[uid], &context);
    match results.get(uid).unwrap() {
        Ok(NodeOutput::Data(table)) => Arc::clone(table),
        other => panic!("{}: {other:?}", stream_path.display()),
    }
}

#[test]
fn twostep_build_nodes_find_modelers_clusters() {
    let root = support::repo_root();
    let mut cases: Vec<(String, String)> = vec![("models".into(), "twostep_default".into())];
    let mut names: Vec<String> = std::fs::read_dir(root.join("corpus/twostep"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "str"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    cases.extend(names.into_iter().map(|n| ("twostep".to_string(), n)));

    let mut failures = Vec::new();
    for (label, case) in &cases {
        let data = root.join("oracle").join(label).join(case).join("data");
        let oracle = std::fs::read_dir(&data)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.to_string_lossy().contains("_applytwostep_"))
            .unwrap();
        let uid = oracle
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .rsplit('_')
            .next()
            .unwrap()
            .to_string();
        let text = std::fs::read_to_string(&oracle).unwrap();
        let mut lines = text.lines().filter(|line| !line.is_empty());
        let header = support::parse_csv_line(lines.next().unwrap());
        let column = header.iter().position(|c| c.starts_with("$T-")).unwrap();
        let theirs: Vec<String> = lines
            .map(|line| {
                support::parse_csv_line(line)[column]
                    .trim_matches('"')
                    .to_string()
            })
            .collect();

        let table = scored(
            label,
            &root.join("corpus").join(label).join(format!("{case}.str")),
            &uid,
        );
        let position = table
            .model()
            .position(&header[column])
            .unwrap_or_else(|| panic!("{case}: no {}", header[column]));
        let ours: Vec<String> = table
            .rows()
            .iter()
            .map(|row| row[position].to_string())
            .collect();

        // The same clusters: our names map one to one onto Modeler's.
        let mut mapping: HashMap<&str, &str> = HashMap::new();
        let same_partition = ours.len() == theirs.len()
            && ours
                .iter()
                .zip(&theirs)
                .all(|(a, b)| *mapping.entry(a).or_insert(b) == b.as_str())
            && {
                let mut targets: Vec<&&str> = mapping.values().collect();
                targets.sort();
                targets.dedup();
                targets.len() == mapping.len()
            };
        let same_names = ours == theirs;
        println!("{case}: same clusters {same_partition}, same names {same_names}");
        if !same_partition || (SAME_NAMES.contains(&case.as_str()) && !same_names) {
            failures.push(case.clone());
        }
    }
    assert!(failures.is_empty(), "cases that differ: {failures:?}");
}
