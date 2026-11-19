//! The Cox build node against the model Modeler saved in `ibm-demos/telco_coxreg`.
//!
//! The stream's build node (stepwise, likelihood-ratio removal) reads `telco.sav`. Modeler
//! cannot run the node in the oracle install, but the stream embeds the model it trained. The
//! trainer must reproduce its twelve coefficients and its baseline cumulative hazard.

#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{CoxModel, Model, Pmml};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::sync::Arc;

fn saved_cox(attachments: &std::collections::HashMap<String, Arc<[u8]>>) -> CoxModel {
    let mut members: Vec<_> = attachments.keys().filter(|k| k.ends_with(".dat")).collect();
    members.sort();
    for member in members {
        let bytes = &attachments[member];
        let Some(start) = bytes.iter().position(|b| *b == b'<') else {
            continue;
        };
        let Ok(text) = std::str::from_utf8(&bytes[start..]) else {
            continue;
        };
        if !text.contains("COXREG") {
            continue;
        }
        if let Model::Cox(model) = Pmml::parse(text).unwrap().model {
            return model;
        }
    }
    panic!("no saved Cox model in the stream");
}

#[test]
fn cox_build_node_reproduces_the_model_modeler_saved() {
    let root = support::repo_root();
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(root.join("corpus/ibm-demos/telco_coxreg.str")).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let attachments: std::collections::HashMap<String, Arc<[u8]>> = archive
        .attachments
        .into_iter()
        .map(|a| (a.name, Arc::from(a.bytes)))
        .collect();
    let saved = saved_cox(&attachments);
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "spss.coxreg_id!buildcoxreg")
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
    let results = Plan::new(&stream).evaluate(&[builder.uid.as_str()], &context);
    let Some(Ok(NodeOutput::Model(trained))) = results.get(&builder.uid) else {
        panic!("the build node failed: {:?}", results.get(&builder.uid));
    };
    let Model::Cox(ours) = &trained.model else {
        panic!("not a Cox model");
    };
    assert_eq!(ours.time_field, saved.time_field);
    assert_eq!(ours.event_field, saved.event_field);
    assert_eq!(ours.event_value, saved.event_value);
    assert_eq!(ours.terms.len(), saved.terms.len(), "{:?}", ours.terms);
    for (a, b) in ours.terms.iter().zip(&saved.terms) {
        assert_eq!((&a.field, &a.level), (&b.field, &b.level));
        assert!(
            (a.beta - b.beta).abs() <= 1e-9 * b.beta.abs(),
            "{}: {} vs {}",
            a.field,
            a.beta,
            b.beta
        );
    }
    assert_eq!(ours.baseline.len(), saved.baseline.len());
    for ((t, h), (saved_t, saved_h)) in ours.baseline.iter().zip(&saved.baseline) {
        assert_eq!(t, saved_t);
        assert!(
            (h - saved_h).abs() <= 1e-9 * saved_h.abs(),
            "t={t}: {h} vs {saved_h}"
        );
    }
}
