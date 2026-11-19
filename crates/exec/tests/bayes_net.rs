//! Bayes Net build nodes against the models Modeler saved in `ibm-demos/bayes_bankloan`.
//!
//! The stream's three build nodes (one TAN, two Markov blanket) read the records of the Select
//! node. Modeler's Bayes Net node cannot run in the oracle install, but the stream embeds six
//! models Modeler trained earlier from the same settings. Our builders must reproduce each
//! build node's structure and every probability of one of them.

#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{BayesNetModel, Model, Pmml};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::sync::Arc;

fn saved_models(attachments: &std::collections::HashMap<String, Arc<[u8]>>) -> Vec<Pmml> {
    let mut members: Vec<_> = attachments.keys().filter(|k| k.ends_with(".dat")).collect();
    members.sort();
    members
        .into_iter()
        .map(|member| {
            let bytes = &attachments[member];
            // length-prefixed PMML text
            let text = std::str::from_utf8(&bytes[4..]).unwrap();
            Pmml::parse(text).unwrap()
        })
        .collect()
}

fn network(pmml: &Pmml) -> &BayesNetModel {
    match &pmml.model {
        Model::BayesNet(model) => model,
        other => panic!("not a Bayes Net model: {other:?}"),
    }
}

/// Whether two models have the same parents per node and the same probabilities.
fn same(a: &BayesNetModel, b: &BayesNetModel) -> bool {
    if a.nodes.len() != b.nodes.len() {
        return false;
    }
    a.nodes.iter().all(|x| {
        let Some(y) = b.nodes.iter().find(|y| y.name == x.name) else {
            return false;
        };
        let mut px = x.parents.clone();
        let mut py = y.parents.clone();
        px.sort();
        py.sort();
        if px != py || x.rows.len() != y.rows.len() {
            return false;
        }
        // rows are keyed by parent value, whatever the order of the parents
        x.rows.iter().all(|row| {
            let key: Vec<(&String, &String)> = {
                let mut k: Vec<_> = x.parents.iter().zip(&row.parent_values).collect();
                k.sort();
                k
            };
            y.rows.iter().any(|other| {
                let mut k: Vec<_> = y.parents.iter().zip(&other.parent_values).collect();
                k.sort();
                k == key
                    // the categories may be listed in another order (Modeler lists a flag's
                    // true value first)
                    && x.categories.iter().zip(&row.probabilities).all(|(category, p)| {
                        y.categories
                            .iter()
                            .position(|c| c == category)
                            .and_then(|k| other.probabilities.get(k))
                            .is_some_and(|q| (p - q).abs() < 1e-9)
                    })
            })
        })
    })
}

#[test]
fn bayes_net_builders_reproduce_the_models_modeler_saved() {
    let root = support::repo_root();
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(root.join("corpus/ibm-demos/bayes_bankloan.str")).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let attachments: std::collections::HashMap<String, Arc<[u8]>> = archive
        .attachments
        .into_iter()
        .map(|a| (a.name, Arc::from(a.bytes)))
        .collect();
    let saved = saved_models(&attachments);
    assert_eq!(saved.len(), 6);
    let builders: Vec<_> = stream
        .diagram
        .nodes
        .iter()
        .filter(|n| n.type_identifier() == "spss.bayesian_id!buildBN")
        .collect();
    assert_eq!(builders.len(), 3);
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
    let uids: Vec<&str> = builders.iter().map(|n| n.uid.as_str()).collect();
    let results = Plan::new(&stream).evaluate(&uids, &context);
    for builder in builders {
        let Some(Ok(NodeOutput::Model(trained))) = results.get(&builder.uid) else {
            panic!(
                "{}: the build node failed: {:?}",
                builder.uid,
                results.get(&builder.uid)
            );
        };
        let ours = network(trained);
        assert!(
            saved.iter().any(|m| same(ours, network(m))),
            "{}: no saved Modeler model has our structure and probabilities",
            builder.uid
        );
    }
}
