//! Saved transfer models through the public stream/node path. IBM artifacts stay
//! in the local, read-only corpus/oracle directories.
#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]
mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::sync::Arc;

#[test]
fn broadband_saved_transfer_targets_match_every_oracle_cell() {
    let root = support::repo_root();
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(root.join("corpus/ibm-demos/broadband_create_models.str")).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let context = ExecContext {
        files: FileResolver {
            data_dirs: vec![
                root.join("corpus/ibm-demos"),
                root.join("corpus/ibm-demo-data"),
            ],
        },
        attachments: Arc::new(
            archive
                .attachments
                .into_iter()
                .map(|a| (a.name, Arc::from(a.bytes)))
                .collect(),
        ),
        stream_properties: stream.properties.clone().map(Arc::new),
        ..ExecContext::default()
    };
    let uid = "id7JGRNXSM29U";
    let result = Plan::new(&stream).evaluate(&[uid], &context);
    let Some(Ok(NodeOutput::Data(table))) = result.get(uid) else {
        panic!("missing transfer output: {:?}", result.get(uid));
    };
    let text = std::fs::read_to_string(
        root.join("oracle/ibm-demos/broadband_create_models/data/02_applyts_id7JGRNXSM29U.csv"),
    )
    .unwrap();
    let mut lines = text.lines();
    let header = support::parse_csv_line(lines.next().unwrap());
    assert_eq!(
        table
            .model()
            .fields()
            .iter()
            .map(|f| &f.name)
            .collect::<Vec<_>>(),
        header.iter().collect::<Vec<_>>()
    );
    let rows: Vec<_> = lines.map(support::parse_csv_line).collect();
    assert_eq!(table.row_count(), rows.len());
    for target in ["Market_1", "Market_2", "Market_4", "Market_5"] {
        for prefix in ["$TS-", "$TSLCI-", "$TSUCI-"] {
            let name = format!("{prefix}{target}");
            let at = table.model().position(&name).unwrap();
            for (row, (actual, expected)) in table.rows().iter().zip(&rows).enumerate() {
                let value = actual[at].as_f64();
                if expected[at].is_empty() {
                    assert_eq!(value, None, "{name} row {}", row + 1);
                } else {
                    let expected: f64 = expected[at].parse().unwrap();
                    let actual = value.unwrap();
                    assert!(
                        (actual - expected).abs() <= 1e-9 * expected.abs().max(1.0),
                        "{name} row {}: {actual} != {expected}",
                        row + 1
                    );
                }
            }
        }
    }
}
