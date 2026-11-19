//! The KNN build node against the model Modeler saved in `ibm-demos/car_sales_knn`.
//!
//! The model identifies neighbours (no target): three of them, Euclidean distance, continuous
//! fields rescaled, 154 stored cases. The neighbour table itself is checked in
//! `dms-algo`'s `knn_oracle`. The nugget saves no neighbour details, so it adds no fields.

#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{KnnModel, Model, Pmml};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::sync::Arc;

#[test]
fn knn_build_node_reproduces_the_saved_model_and_the_nugget_adds_no_fields() {
    let root = support::repo_root();
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(root.join("corpus/ibm-demos/car_sales_knn.str")).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let attachments: std::collections::HashMap<String, Arc<[u8]>> = archive
        .attachments
        .into_iter()
        .map(|a| (a.name, Arc::from(a.bytes)))
        .collect();
    let saved: KnnModel = attachments
        .iter()
        .filter(|(name, _)| name.ends_with(".dat"))
        .find_map(|(_, bytes)| {
            let start = bytes.iter().position(|b| *b == b'<')?;
            let text = std::str::from_utf8(&bytes[start..]).ok()?;
            match Pmml::parse(text).ok()?.model {
                Model::Knn(model) => Some(model),
                _ => None,
            }
        })
        .unwrap();
    assert_eq!(
        (saved.k, saved.training_cases, saved.target.clone()),
        (3, 154, None)
    );
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "spss.KNN_id!KNNbuilder")
        .unwrap();
    let apply = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "spss.KNN_id!KNNapplier")
        .unwrap();
    let context = ExecContext {
        files: FileResolver {
            data_dirs: vec![
                root.join("corpus/ibm-demos"),
                root.join("corpus/ibm-demo-data"),
            ],
        },
        attachments: Arc::new(attachments),
        stream_properties: stream.properties.clone().map(Arc::new),
        ..ExecContext::default()
    };
    let results =
        Plan::new(&stream).evaluate(&[builder.uid.as_str(), apply.uid.as_str()], &context);
    let Some(Ok(NodeOutput::Model(trained))) = results.get(&builder.uid) else {
        panic!("the build node failed: {:?}", results.get(&builder.uid));
    };
    let Model::Knn(ours) = &trained.model else {
        panic!("not a KNN model");
    };
    assert_eq!(
        (
            ours.k,
            ours.training_cases,
            &ours.target,
            &ours.metric,
            ours.rescale
        ),
        (
            saved.k,
            saved.training_cases,
            &saved.target,
            &saved.metric,
            saved.rescale
        )
    );
    let Some(Ok(NodeOutput::Data(table))) = results.get(&apply.uid) else {
        panic!("the nugget failed: {:?}", results.get(&apply.uid));
    };
    assert!(
        table
            .model()
            .fields()
            .iter()
            .all(|f| !f.name.starts_with('$'))
    );
}
