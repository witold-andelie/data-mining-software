//! SVM build nodes against the models Modeler saved in `ibm-demos/svm_cancer`.
//!
//! Modeler cannot run the node in the oracle install, but the stream embeds two models it
//! trained earlier (an RBF and a polynomial kernel) from the same settings. Our builders must
//! find the same support vectors and, for the polynomial kernel, the same coefficients.

#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{Model, Pmml, SvmModel};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::collections::HashMap;
use std::sync::Arc;

struct Loaded {
    stream: Stream,
    context: ExecContext,
    saved: Vec<Pmml>,
}

fn load() -> Loaded {
    let root = support::repo_root();
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(root.join("corpus/ibm-demos/svm_cancer.str")).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let attachments: HashMap<String, Arc<[u8]>> = archive
        .attachments
        .into_iter()
        .map(|a| (a.name, Arc::from(a.bytes)))
        .collect();
    let mut members: Vec<_> = attachments.keys().filter(|k| k.ends_with(".dat")).collect();
    members.sort();
    let saved = members
        .into_iter()
        .filter_map(|member| {
            let bytes = &attachments[member];
            let start = bytes.iter().position(|b| *b == b'<')?;
            let text = std::str::from_utf8(&bytes[start..]).ok()?;
            text.contains("SupportVectorMachineModel")
                .then(|| Pmml::parse(text).unwrap())
        })
        .collect();
    // The demo's `cell_samples.data` is not in the corpus; the oracle exported the records the
    // build nodes read (the same file with its text fields quoted), so rebuild it from them.
    let data_dir = root.join("target/test-data/svm_cancer");
    std::fs::create_dir_all(&data_dir).unwrap();
    let exported = std::fs::read_to_string(
        root.join("oracle/ibm-demos/svm_cancer/data/01_unknownprocess_idE9S9B6FWYN.csv"),
    )
    .unwrap();
    std::fs::write(
        data_dir.join("cell_samples.data"),
        exported.replace('"', ""),
    )
    .unwrap();
    let context = ExecContext {
        files: FileResolver {
            data_dirs: vec![
                data_dir,
                root.join("corpus/ibm-demos"),
                root.join("corpus/ibm-demo-data"),
            ],
        },
        attachments: Arc::new(attachments),
        stream_properties: stream.properties.clone().map(Arc::new),
        ..ExecContext::default()
    };
    Loaded {
        stream,
        context,
        saved,
    }
}

fn svm(pmml: &Pmml) -> &SvmModel {
    match &pmml.model {
        Model::Svm(model) => model,
        other => panic!("not an SVM model: {other:?}"),
    }
}

/// Rows of the support vectors as sorted text, to compare sets of vectors.
fn vector_keys(model: &SvmModel) -> Vec<String> {
    let mut keys: Vec<String> = model
        .vectors
        .iter()
        .map(|v| {
            v.iter()
                .map(|x| format!("{x:.5}"))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect();
    keys.sort();
    keys
}

#[test]
fn svm_build_nodes_find_the_support_vectors_modeler_saved() {
    let loaded = load();
    assert_eq!(loaded.saved.len(), 2);
    let builders: Vec<_> = loaded
        .stream
        .diagram
        .nodes
        .iter()
        .filter(|n| n.type_identifier() == "spss.svm_id!buildSVM")
        .collect();
    assert_eq!(builders.len(), 2);
    let uids: Vec<&str> = builders.iter().map(|n| n.uid.as_str()).collect();
    let results = Plan::new(&loaded.stream).evaluate(&uids, &loaded.context);
    for builder in builders {
        let Some(Ok(NodeOutput::Model(trained))) = results.get(&builder.uid) else {
            panic!(
                "{}: the build node failed: {:?}",
                builder.uid,
                results.get(&builder.uid)
            );
        };
        let ours = svm(trained);
        let saved = loaded
            .saved
            .iter()
            .map(svm)
            .find(|m| m.kernel == ours.kernel)
            .unwrap();
        assert_eq!(vector_keys(ours), vector_keys(saved), "support vectors");
        assert!(
            (ours.constant - saved.constant).abs() < 1e-2,
            "constant {} vs {}",
            ours.constant,
            saved.constant
        );
    }
}

#[test]
fn svm_nuggets_score_the_training_records_like_the_saved_models() {
    let loaded = load();
    let apply: Vec<_> = loaded
        .stream
        .diagram
        .nodes
        .iter()
        .filter(|n| n.type_identifier() == "spss.svm_id!applySVM")
        .collect();
    assert_eq!(apply.len(), 2);
    let uids: Vec<&str> = apply.iter().map(|n| n.uid.as_str()).collect();
    let results = Plan::new(&loaded.stream).evaluate(&uids, &loaded.context);
    for node in apply {
        let Some(Ok(NodeOutput::Data(table))) = results.get(&node.uid) else {
            panic!("the nugget failed: {:?}", results.get(&node.uid));
        };
        let model = table.model();
        let truth = model.position("Class").unwrap();
        let predicted = model.position("$S-Class").unwrap();
        assert!(model.position("$SP-Class").is_some());
        let wrong = table
            .rows()
            .iter()
            .filter(|row| row[truth].to_string() != row[predicted].to_string())
            .count();
        assert!(
            wrong <= 15,
            "{wrong} of {} records differ",
            table.rows().len()
        );
    }
}
