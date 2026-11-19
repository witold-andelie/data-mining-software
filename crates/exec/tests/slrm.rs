//! The SLRM nodes against the model Modeler saved in `ibm-demos/pm_selflearn`.
//!
//! The build node continues the training of a model Modeler keeps in a state we do not read, so
//! it is a documented gap; the nugget scores the 27 records of the stream with the saved model.

#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{Model, Pmml};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::collections::HashMap;
use std::sync::Arc;

#[test]
fn slrm_nugget_scores_with_the_saved_model_and_the_build_node_is_a_gap() {
    let root = support::repo_root();
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(root.join("corpus/ibm-demos/pm_selflearn.str")).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let attachments: HashMap<String, Arc<[u8]>> = archive
        .attachments
        .into_iter()
        .map(|a| (a.name, Arc::from(a.bytes)))
        .collect();
    // the saved model: 4-byte length prefix and PMML text
    let text = std::str::from_utf8(&attachments["data/0001.dat"][4..]).unwrap();
    let saved = Pmml::parse(text).unwrap();
    let Model::Slrm(saved) = &saved.model else {
        panic!("not an SLRM model");
    };
    assert_eq!(saved.target, "campaign");
    let names: Vec<&str> = saved.products.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Mortgage", "Car loan", "Savings", "Pension"]);
    assert_eq!(saved.products[0].target_counts, [1371, 467]);
    assert_eq!(saved.products[1].target_counts, [11552, 2399]);

    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "spss.selflearning!selflearning")
        .unwrap();
    let apply = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "spss.selflearning!applyselflearning")
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
    let Some(Err(error)) = results.get(&builder.uid) else {
        panic!(
            "the build node should be a gap: {:?}",
            results.get(&builder.uid)
        );
    };
    assert!(
        error.to_string().contains("continuing the training"),
        "{error}"
    );
    let Some(Ok(NodeOutput::Data(table))) = results.get(&apply.uid) else {
        panic!("the nugget failed: {:?}", results.get(&apply.uid));
    };
    let model = table.model();
    let first = model.position("$S-campaign-1").unwrap();
    let score = model.position("$SC-campaign-1").unwrap();
    assert_eq!(table.rows().len(), 27);
    for row in table.rows() {
        assert!(names.contains(&row[first].to_string().as_str()));
        let s = row[score].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&s));
    }
}
