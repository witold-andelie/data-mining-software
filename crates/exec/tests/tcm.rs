//! The Temporal Causal Model build node against the model Modeler saved in `ibm-demos/tcm_kpi`.
//!
//! The stream's nugget holds a `TCMXML` document per series (5 levers, 25 KPIs). The build node
//! must select the same predictors in the same order and reproduce every intercept and lag
//! coefficient, and the nugget must add one fitted-value field per KPI.

#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{Model, TcmModel};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::collections::HashMap;
use std::sync::Arc;

fn saved_model(attachments: &HashMap<String, Arc<[u8]>>) -> TcmModel {
    let setup = dms_strfmt::read_members(&attachments["data/0004.dat"]).unwrap();
    let json: serde_json::Value = serde_json::from_slice(
        &setup
            .iter()
            .find(|m| m.name == "TSDPOutput.json")
            .unwrap()
            .body,
    )
    .unwrap();
    let series: Vec<String> = json["settings"]["metricFieldList"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let members = dms_strfmt::read_members(&attachments["data/0003.dat"]).unwrap();
    let documents: Vec<String> = members
        .iter()
        .filter(|m| m.body.starts_with(b"<?xml") && m.body.windows(6).any(|w| w == b"TCMXML"))
        .map(|m| String::from_utf8(m.body.clone()).unwrap())
        .collect();
    let documents: Vec<&str> = documents.iter().map(String::as_str).collect();
    TcmModel::from_documents(series, &documents).unwrap()
}

#[test]
fn tcm_build_node_reproduces_the_model_modeler_saved() {
    let root = support::repo_root();
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(root.join("corpus/ibm-demos/tcm_kpi.str")).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let attachments: HashMap<String, Arc<[u8]>> = archive
        .attachments
        .into_iter()
        .map(|a| (a.name, Arc::from(a.bytes)))
        .collect();
    let saved = saved_model(&attachments);
    assert_eq!(saved.targets.len(), 30);
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "tcm!pasw.timeseries-extension")
        .unwrap();
    let apply = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "tcmmodel!pasw.timeseries-extension")
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
    let Model::Tcm(ours) = &trained.model else {
        panic!("not a TCM model");
    };
    assert_eq!(ours.series, saved.series);
    assert_eq!(ours.lag, saved.lag);
    assert_eq!(ours.targets.len(), saved.targets.len());
    for (a, b) in ours.targets.iter().zip(&saved.targets) {
        let name = &saved.series[b.series];
        assert_eq!((a.series, a.ar_only), (b.series, b.ar_only), "{name}");
        assert!(
            (a.intercept - b.intercept).abs() <= 1e-8 * b.intercept.abs().max(1.0),
            "{name} intercept"
        );
        assert_eq!(a.predictors.len(), b.predictors.len(), "{name}");
        for (p, q) in a.predictors.iter().zip(&b.predictors) {
            assert_eq!(p.series, q.series, "{name}: predictor order");
            for (x, y) in p.coefficients.iter().zip(&q.coefficients) {
                assert!(
                    (x - y).abs() <= 1e-8 * y.abs().max(1e-6),
                    "{name}: {x} vs {y}"
                );
            }
        }
    }
    let Some(Ok(NodeOutput::Data(table))) = results.get(&apply.uid) else {
        panic!("the nugget failed: {:?}", results.get(&apply.uid));
    };
    let fitted: Vec<&str> = table
        .model()
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .filter(|n| n.starts_with("$TCM-"))
        .collect();
    assert_eq!(fitted.len(), 25, "{fitted:?}");
}
