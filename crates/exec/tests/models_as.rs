//! Saved Analytic Engine nuggets: all columns of every oracle terminal input.
#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use dms_types::Value;
use serde_json::Value as Json;
use std::sync::Arc;

#[test]
fn all_analytic_engine_terminal_inputs_equal_oracle() {
    let root = support::repo_root();
    let mut terminals = 0;
    for case in [
        "rt_default",
        "rt_fewer",
        "rt_shallow",
        "rt_native",
        "rt_conf",
        "rt_noconf",
        "rt_batch",
        "rt_numeric",
        "rt_numeric_batch",
        "treeas_default",
        "treeas_depth",
        "treeas_numeric",
        "linearas_default",
        "lsvm_default",
        "lsvm_numeric",
        "lsvm_l1",
    ] {
        let archive = dms_strfmt::container::read_archive(
            &std::fs::read(root.join(format!("corpus/models_as/{case}.str"))).unwrap(),
            &ArchiveLimits::default(),
        )
        .unwrap();
        let stream = Stream::from_document(&archive.document).unwrap();
        let context = ExecContext {
            files: FileResolver {
                data_dirs: vec![root.join("corpus/models_as")],
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
        let manifest: Json = serde_json::from_str(
            &std::fs::read_to_string(root.join(format!("oracle/models_as/{case}/manifest.json")))
                .unwrap(),
        )
        .unwrap();
        let entries = manifest["data"].as_array().unwrap();
        let targets: Vec<_> = entries
            .iter()
            .map(|entry| entry["id"].as_str().unwrap())
            .collect();
        let results = Plan::new(&stream).evaluate(&targets, &context);
        for entry in entries {
            let uid = entry["id"].as_str().unwrap();
            let Some(Ok(NodeOutput::Data(actual))) = results.get(uid) else {
                panic!("{case}/{uid}: {:?}", results.get(uid));
            };
            let text =
                std::fs::read_to_string(support::local_path(entry["file"].as_str().unwrap()))
                    .unwrap();
            let mut lines = text.trim_start_matches('\u{feff}').lines();
            let names = support::parse_csv_line(lines.next().unwrap());
            assert_eq!(
                actual
                    .model()
                    .fields()
                    .iter()
                    .map(|f| f.name.as_str())
                    .collect::<Vec<_>>(),
                names,
                "{case}/{uid}"
            );
            let rows: Vec<_> = lines.map(support::parse_csv_line).collect();
            assert_eq!(actual.rows().len(), rows.len(), "{case}/{uid}");
            // LSVM trains to convergence (`dms_algo::lsvm`), so its nugget scores our optimum
            // instead of Modeler's early-stopped coefficients. Rule 3 of
            // `docs/statistical-parity.md` then allows a continuous score to differ by 1 % of
            // the target's standard deviation; the predicted classes still match exactly.
            // `lsvm.rs` holds the full statistical-parity check.
            let band = case.strip_prefix("lsvm_").map(|_| {
                let target = names
                    .iter()
                    .find_map(|name| {
                        name.strip_prefix("$L-")
                            .filter(|rest| !rest.starts_with('C'))
                    })
                    .unwrap_or_default();
                let values: Vec<f64> = names
                    .iter()
                    .position(|name| *name == target)
                    .map(|at| {
                        actual
                            .rows()
                            .iter()
                            .filter_map(|row| row.get(at).and_then(|v| v.as_f64()))
                            .collect()
                    })
                    .unwrap_or_default();
                if values.is_empty() {
                    // A flag target: the score is a probability, so 1 % of its range.
                    return 0.01;
                }
                let mean = values.iter().sum::<f64>() / values.len() as f64;
                0.01 * (values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>()
                    / values.len() as f64)
                    .sqrt()
            });
            for (r, (actual, expected)) in actual.rows().iter().zip(&rows).enumerate() {
                assert_eq!(actual.len(), expected.len());
                for (c, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                    let matches = match actual {
                        Value::Null => expected.is_empty() || expected == "$null$",
                        Value::String(s) => s.as_ref() == expected,
                        Value::Integer(_) | Value::Real(_) => {
                            let a = actual.as_f64().unwrap();
                            let b = expected.parse::<f64>().unwrap();
                            let tolerance = match band {
                                Some(band) if names[c].starts_with("$L") => band,
                                _ => 1e-9,
                            };
                            a.is_finite()
                                && b.is_finite()
                                && (a - b).abs() <= tolerance * a.abs().max(b.abs()).max(1.0)
                        }
                        other => panic!("unexpected AS value {other:?}"),
                    };
                    assert!(
                        matches,
                        "{case}/{uid} row {r} column {c}: {actual:?} != {expected:?}"
                    );
                }
            }
            terminals += 1;
        }
    }
    assert_eq!(terminals, 48);
}
