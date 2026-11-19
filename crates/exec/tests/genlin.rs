//! Generalized Linear build nodes against the models Modeler saved in the demo streams
//! (`ulcer_genlin`, `car-insurance_genlin`).
//!
//! Modeler cannot run the node in the oracle install, but the streams embed the models it
//! trained earlier from the same settings. Our builders must reproduce every coefficient of one
//! of them, and the stream's nuggets must score with them.

#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{GeneralRegressionModel, Model, Pmml};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::collections::HashMap;
use std::sync::Arc;

struct Loaded {
    stream: Stream,
    context: ExecContext,
    saved: Vec<Pmml>,
}

fn load(stem: &str) -> Loaded {
    let root = support::repo_root();
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(root.join(format!("corpus/ibm-demos/{stem}.str"))).unwrap(),
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
            if !text.contains("GZLM") {
                return None;
            }
            Some(Pmml::parse(text).unwrap())
        })
        .collect();
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
    Loaded {
        stream,
        context,
        saved,
    }
}

fn general(pmml: &Pmml) -> &GeneralRegressionModel {
    match &pmml.model {
        Model::GeneralRegression(model) => model,
        other => panic!("not a general regression model: {other:?}"),
    }
}

/// Whether two models have the same link, parameters and coefficients.
fn same(a: &GeneralRegressionModel, b: &GeneralRegressionModel) -> bool {
    a.link_function == b.link_function
        && a.param_matrix.len() == b.param_matrix.len()
        && a.param_matrix.iter().zip(&b.param_matrix).all(|(x, y)| {
            x.parameter == y.parameter && (x.beta - y.beta).abs() <= 1e-6 * y.beta.abs().max(1e-3)
        })
}

fn builders_reproduce(stem: &str, expected_builders: usize, expected_models: usize) {
    let loaded = load(stem);
    assert_eq!(loaded.saved.len(), expected_models);
    let builders: Vec<_> = loaded
        .stream
        .diagram
        .nodes
        .iter()
        .filter(|n| n.type_identifier() == "spss.generalizedlinear_id!buildgeneralizedlinear")
        .collect();
    assert_eq!(builders.len(), expected_builders);
    let uids: Vec<&str> = builders.iter().map(|n| n.uid.as_str()).collect();
    let results = Plan::new(&loaded.stream).evaluate(&uids, &loaded.context);
    let mut reproduced = 0;
    for builder in builders {
        let Some(Ok(NodeOutput::Model(trained))) = results.get(&builder.uid) else {
            // the ulcer stream's no-intercept and empty-offset builders have no saved model
            // and may be GAPs; they must not panic
            continue;
        };
        let ours = general(trained);
        if loaded.saved.iter().any(|m| same(ours, general(m))) {
            reproduced += 1;
        }
    }
    assert_eq!(
        reproduced, expected_models,
        "every saved model must be reproduced by a build node"
    );
}

#[test]
fn genlin_car_insurance_build_node_reproduces_the_saved_model() {
    builders_reproduce("car-insurance_genlin", 1, 1);
}

#[test]
fn genlin_ulcer_build_nodes_reproduce_the_saved_models() {
    builders_reproduce("ulcer_genlin", 4, 2);
}

#[test]
fn genlin_nuggets_score_with_the_saved_models() {
    for (stem, nugget_columns) in [
        ("car-insurance_genlin", vec!["$G-claimamt"]),
        ("ulcer_genlin", vec!["$G-result2", "$GP-result2"]),
    ] {
        let loaded = load(stem);
        let apply: Vec<_> = loaded
            .stream
            .diagram
            .nodes
            .iter()
            .filter(|n| n.type_identifier() == "spss.generalizedlinear_id!applygeneralizedlinear")
            .collect();
        assert!(!apply.is_empty());
        let uids: Vec<&str> = apply.iter().map(|n| n.uid.as_str()).collect();
        let results = Plan::new(&loaded.stream).evaluate(&uids, &loaded.context);
        let mut seen = Vec::new();
        for node in apply {
            let Some(Ok(NodeOutput::Data(table))) = results.get(&node.uid) else {
                panic!("{stem}: the nugget failed: {:?}", results.get(&node.uid));
            };
            for field in table.model().fields() {
                seen.push(field.name.clone());
            }
        }
        for column in nugget_columns {
            assert!(
                seen.iter().any(|n| n == column),
                "{stem}: no {column} in {seen:?}"
            );
        }
    }
}
