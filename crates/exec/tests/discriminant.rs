//! The Discriminant build node against the model Modeler saved in
//! `ibm-demos/telco_custcat_discriminant`.
//!
//! The stream's build node (stepwise, Wilks' lambda, F-to-enter 3.84, F-to-remove 2.71) keeps
//! education, years with the employer and people in the household. The nugget's `model` container
//! holds the canonical functions, the group centroids and the priors, which the build node must
//! reproduce; its scores must follow from them.

#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{DiscriminantModel, Model};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::collections::HashMap;
use std::sync::Arc;

#[test]
fn discriminant_build_node_reproduces_the_saved_model_and_the_nugget_scores() {
    let root = support::repo_root();
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(root.join("corpus/ibm-demos/telco_custcat_discriminant.str")).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let attachments: HashMap<String, Arc<[u8]>> = archive
        .attachments
        .into_iter()
        .map(|a| (a.name, Arc::from(a.bytes)))
        .collect();
    let saved = DiscriminantModel::from_spssml(
        std::str::from_utf8(&attachments["data/0002.dat"][4..]).unwrap(),
    )
    .unwrap();
    assert_eq!(saved.predictors, ["ed", "employ", "reside"]);
    assert_eq!(saved.groups, ["1", "2", "3", "4"]);
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "spss.discriminant_id!builddiscriminant")
        .unwrap();
    let apply = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "spss.discriminant_id!applydiscriminant")
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
    let Model::Discriminant(ours) = &trained.model else {
        panic!("not a discriminant model");
    };
    assert_eq!(ours.predictors, saved.predictors);
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * (1.0 + b.abs());
    for (a, b) in ours
        .coefficients
        .iter()
        .flatten()
        .zip(saved.coefficients.iter().flatten())
    {
        assert!(close(*a, *b), "coefficient {a} vs {b}");
    }
    for (a, b) in ours.constants.iter().zip(&saved.constants) {
        assert!(close(*a, *b), "constant {a} vs {b}");
    }
    for (a, b) in ours
        .centroids
        .iter()
        .flatten()
        .zip(saved.centroids.iter().flatten())
    {
        assert!(close(*a, *b), "centroid {a} vs {b}");
    }
    for (a, b) in ours.priors.iter().zip(&saved.priors) {
        assert!(close(*a, *b), "prior {a} vs {b}");
    }
    let Some(Ok(NodeOutput::Data(table))) = results.get(&apply.uid) else {
        panic!("the nugget failed: {:?}", results.get(&apply.uid));
    };
    let model = table.model();
    let predicted = model.position("$D-custcat").unwrap();
    let probability = model.position("$DP-custcat").unwrap();
    let truth = model.position("custcat").unwrap();
    let rows = table.rows();
    assert_eq!(rows.len(), 1000);
    // Modeler's classification table: 125 + 15 + 112 + 143 of 1000 are right (39.5%)
    let correct = rows
        .iter()
        .filter(|r| r[predicted].to_string() == r[truth].to_string())
        .count();
    assert_eq!(correct, 395);
    assert!(rows.iter().all(|r| {
        r[probability]
            .as_f64()
            .is_some_and(|p| (0.0..=1.0).contains(&p))
    }));
}
