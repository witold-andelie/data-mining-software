//! Read-only comparison of all 15 Feature Selection nuggets with Modeler's kept fields.
#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "oracle test assertions"
)]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::sync::Arc;

#[test]
fn all_fifteen_nuggets_preserve_oracle_fields_rows_and_metadata() {
    let root = support::repo_root();
    let cases = [
        "fs_continuous_target",
        "fs_cramers_v",
        "fs_importance_levels",
        "fs_importance_value_50",
        "fs_lambda",
        "fs_likelihood",
        "fs_manual_fields",
        "fs_pearson",
        "fs_screen_categories",
        "fs_screen_cv",
        "fs_screen_dominance",
        "fs_screen_missing",
        "fs_screen_sd",
        "fs_tie_column_order",
        "fs_training_partition",
    ];
    for case in cases {
        let archive = dms_strfmt::container::read_archive(
            &std::fs::read(root.join(format!("corpus/models_stats/{case}.str"))).unwrap(),
            &ArchiveLimits::default(),
        )
        .unwrap();
        let stream = Stream::from_document(&archive.document).unwrap();
        let nugget = stream
            .diagram
            .nodes
            .iter()
            .find(|n| n.type_identifier() == "ApplyFeatureSelectionNode")
            .unwrap();
        let builder = stream
            .diagram
            .nodes
            .iter()
            .find(|n| n.type_identifier() == "BuildFeatureSelectionNode")
            .unwrap();
        let source = stream
            .diagram
            .nodes
            .iter()
            .find(|n| n.type_identifier() == "TypeNode")
            .unwrap();
        let context = ExecContext {
            cancel: Default::default(),
            files: FileResolver {
                data_dirs: vec![root.join("corpus/models_stats")],
            },
            attachments: Arc::new(
                archive
                    .attachments
                    .into_iter()
                    .map(|a| (a.name, Arc::from(a.bytes)))
                    .collect(),
            ),
            stream_properties: stream.properties.clone().map(Arc::new),
            decimal_places: Some(15),
            export_dir: None,
        };
        let results = Plan::new(&stream).evaluate(
            &[
                nugget.uid.as_str(),
                builder.uid.as_str(),
                source.uid.as_str(),
            ],
            &context,
        );
        assert!(
            matches!(results.get(&builder.uid), Some(Ok(NodeOutput::Model(_)))),
            "{case}: builder failed: {:?}",
            results.get(&builder.uid)
        );
        if case != "fs_manual_fields" {
            let trained = results
                .get(&builder.uid)
                .unwrap()
                .as_ref()
                .unwrap()
                .model()
                .unwrap();
            let dms_pmml::Model::Mining(model) = &trained.model else {
                panic!("{case}: no selection envelope");
            };
            let mut learned: Vec<_> = model
                .header
                .mining_schema
                .iter()
                .filter(|f| f.usage == dms_pmml::Usage::Active)
                .map(|f| f.name.clone())
                .collect();
            let mut saved: Vec<_> = dms_exec::settings::children(
                nugget.settings.as_ref(),
                "rankedFieldSelection",
                "value",
            )
            .into_iter()
            .map(|e| e.text())
            .collect();
            learned.sort();
            saved.sort();
            assert_eq!(
                learned, saved,
                "{case}: automatic selection must be learned from records"
            );
        }
        let output = results
            .get(&nugget.uid)
            .unwrap()
            .as_ref()
            .unwrap()
            .data()
            .unwrap();
        let input = results
            .get(&source.uid)
            .unwrap()
            .as_ref()
            .unwrap()
            .data()
            .unwrap();
        // Numeric prefixes depend on terminal iteration order; identify exports by node type.
        let csv = std::fs::read_dir(root.join(format!("oracle/models_stats/{case}/data")))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains("_applyfeatureselection_")
            })
            .unwrap();
        let text = std::fs::read_to_string(csv).unwrap();
        let mut lines = text.lines();
        let expected = support::parse_csv_line(lines.next().unwrap());
        let actual: Vec<_> = output
            .model()
            .fields()
            .iter()
            .map(|f| f.name.clone())
            .collect();
        assert_eq!(actual, expected, "{case}");
        assert_eq!(output.row_count(), lines.count(), "{case}");
        assert_eq!(output.row_count(), input.row_count(), "{case}");
        for (p, f) in output.model().fields().iter().enumerate() {
            assert_eq!(Some(f), input.model().field(&f.name), "{case}: metadata");
            let original = input.model().position(&f.name).unwrap();
            for (out, row) in output.rows().iter().zip(input.rows()) {
                assert_eq!(out.get(p), row.get(original), "{case}: record data");
            }
        }
        // Reopen a disconnected nugget: saved choices are sufficient, including manual fields.
        let saved = dms_exec::registry::factory("feature_selection").unwrap()(nugget).unwrap();
        let disconnected = saved.run(&[Arc::clone(input)], &context).unwrap();
        let table_csv = std::fs::read_dir(root.join(format!("oracle/models_stats/{case}/outputs")))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|s| s == "csv"))
            .unwrap();
        let table_text = std::fs::read_to_string(table_csv).unwrap();
        assert_eq!(
            disconnected
                .model()
                .fields()
                .iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>(),
            support::parse_csv_line(table_text.lines().next().unwrap()),
            "{case}: disconnected nugget/initial Table"
        );
    }
}
