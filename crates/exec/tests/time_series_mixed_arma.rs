//! Mixed ARMA estimates against the ones Modeler saved (`ts_est` probes).
//!
//! Modeler's Marquardt iteration stops early, and for ARMA(1,1) models with both terms we do
//! not reproduce its stopping point (the estimates agree to 2-4 digits). What decides which
//! estimate is better is the exact Gaussian likelihood the Guide's objective stands for: at
//! Modeler's saved estimates it equals ours (the error variance to 1e-16), so it can be
//! evaluated at both. Our estimates must never have the higher objective.

#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;

use dms_algo::time_series::{self, FittedSpec};
use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_strfmt::{ArchiveLimits, read_length_prefixed_zip};
use std::sync::Arc;

const PROBES: &[&str] = &[
    "est_ari_a",
    "est_ari_a_nc",
    "est_ari_b",
    "est_ari_b_nc",
    "est_ari_c",
    "est_ari_c_nc",
    "est_arma_a",
    "est_arma_a_nc",
    "est_arma_b",
    "est_arma_b_nc",
    "est_arma_c",
    "est_arma_c_nc",
];

fn saved_spec(
    archive: &dms_strfmt::container::StreamArchive,
    stream: &dms_stream::Stream,
) -> FittedSpec {
    let nugget = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "spss.ts!ts_applier")
        .unwrap();
    let member = &nugget
        .containers
        .iter()
        .find(|c| c.name.as_deref() == Some("PMML"))
        .unwrap()
        .member;
    let bytes = &archive
        .attachments
        .iter()
        .find(|a| &a.name == member)
        .unwrap()
        .bytes;
    let entries = read_length_prefixed_zip(bytes, &ArchiveLimits::default()).unwrap();
    let xml =
        std::str::from_utf8(&entries.iter().find(|e| e.name == "0.xml").unwrap().bytes).unwrap();
    dms_exec::nodes::time_series::parse_saved_model(xml).unwrap()
}

#[test]
fn mixed_arma_estimates_have_the_lower_likelihood_objective_than_modelers() {
    let dir = support::repo_root().join("corpus/ts_est");
    for case in PROBES {
        let archive = dms_strfmt::container::read_archive(
            &std::fs::read(dir.join(format!("{case}.str"))).unwrap(),
            &ArchiveLimits::default(),
        )
        .unwrap();
        let mut stream = dms_stream::Stream::from_document(&archive.document).unwrap();
        let modeler = saved_spec(&archive, &stream);
        let builder = stream
            .diagram
            .nodes
            .iter_mut()
            .find(|n| n.type_identifier() == "spss.ts!ts_builder")
            .unwrap();
        builder.containers.clear();
        let builder_uid = builder.uid.clone();
        let type_uid = stream
            .diagram
            .nodes
            .iter()
            .find(|n| n.type_identifier() == "TypeNode")
            .unwrap()
            .uid
            .clone();
        let context = ExecContext {
            files: FileResolver {
                data_dirs: vec![dir.clone()],
            },
            attachments: Arc::new(Default::default()),
            stream_properties: stream.properties.clone().map(Arc::new),
            ..ExecContext::default()
        };
        let results =
            Plan::new(&stream).evaluate(&[builder_uid.as_str(), type_uid.as_str()], &context);
        let Some(Ok(NodeOutput::Data(table))) = results.get(&type_uid) else {
            panic!("{case}: no input series");
        };
        let at = table.model().position("y").unwrap();
        let y: Vec<f64> = table
            .rows()
            .iter()
            .map(|row| row[at].as_f64().unwrap())
            .collect();
        let Some(Ok(NodeOutput::Model(pmml))) = results.get(&builder_uid) else {
            panic!(
                "{case}: the builder failed: {:?}",
                results.get(&builder_uid)
            );
        };
        let dms_pmml::Model::Mining(mining) = &pmml.model else {
            panic!("{case}: expected a mining model");
        };
        let dms_pmml::Model::TimeSeries { xml, .. } = mining.segments[0].model.as_ref() else {
            panic!("{case}: expected a time series segment");
        };
        let ours = dms_exec::nodes::time_series::parse_saved_model(xml).unwrap();
        let ours_objective = time_series::objective(&ours, &y).unwrap();
        let modeler_objective = time_series::objective(&modeler, &y).unwrap();
        assert!(
            ours_objective <= modeler_objective + 1e-9,
            "{case}: our objective {ours_objective} is above Modeler's {modeler_objective}"
        );
    }
}
