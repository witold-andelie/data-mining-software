//! The GLMM build node against the model Modeler saved in `ibm-demos/testscores_glmm`.
//!
//! The stream's build node fits a normal target with seven fixed effects and two nested random
//! intercepts (school, classroom within school). The nugget's PMML holds the fixed-effect
//! coefficients, which the build node must reproduce; the nugget scores the fixed part.

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
fn glmm_build_node_reproduces_the_saved_fixed_effects_and_the_nugget_scores() {
    let root = support::repo_root();
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(root.join("corpus/ibm-demos/testscores_glmm.str")).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let attachments: HashMap<String, Arc<[u8]>> = archive
        .attachments
        .into_iter()
        .map(|a| (a.name, Arc::from(a.bytes)))
        .collect();
    let saved =
        Pmml::parse(&dms_strfmt::read_model_xml(&attachments["data/0002.dat"]).unwrap()).unwrap();
    let Model::GeneralRegression(saved) = &saved.model else {
        panic!("not a general regression model");
    };
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "GLMM!spss.glmm-extension")
        .unwrap();
    let apply = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "GLMMModel!spss.glmm-extension")
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
    let Model::GeneralRegression(ours) = &trained.model else {
        panic!("not a general regression model");
    };
    assert_eq!(ours.param_matrix.len(), saved.param_matrix.len());
    for (a, b) in ours.param_matrix.iter().zip(&saved.param_matrix) {
        assert!(
            (a.beta - b.beta).abs() <= 1e-6 * b.beta.abs().max(1e-3),
            "{}: {} vs {}",
            a.parameter,
            a.beta,
            b.beta
        );
    }
    assert_eq!(ours.pp_matrix.len(), saved.pp_matrix.len());
    let Some(Ok(NodeOutput::Data(table))) = results.get(&apply.uid) else {
        panic!("the nugget failed: {:?}", results.get(&apply.uid));
    };
    let model = table.model();
    let predicted = model.position("$L-posttest").unwrap();
    assert!(
        table
            .rows()
            .iter()
            .all(|row| row[predicted].as_f64().is_some())
    );
}
