//! PCA/Factor training: the trained factor-score coefficients equal Modeler's exported PMML
//! (`oracle/models_next/factor_*`). `xtask compare` alone cannot show this, since a failing
//! builder leaves the nugget its saved model.
#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "oracle test assertions"
)]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{Model, Pmml};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::sync::Arc;

fn betas(model: &Model) -> Vec<Vec<(String, f64)>> {
    let Model::Mining(mining) = model else {
        panic!("expected one model per factor");
    };
    mining
        .segments
        .iter()
        .map(|segment| match segment.model.as_ref() {
            Model::GeneralRegression(reg) => reg
                .param_matrix
                .iter()
                .map(|c| (c.parameter.clone(), c.beta))
                .collect(),
            other => panic!("unexpected segment {other:?}"),
        })
        .collect()
}

#[test]
fn trained_factor_scores_equal_modeler_coefficients() {
    let root = support::repo_root();
    for case in ["factor_varimax", "factor_nfactors"] {
        let archive = dms_strfmt::container::read_archive(
            &std::fs::read(root.join(format!("corpus/models_next/{case}.str"))).unwrap(),
            &ArchiveLimits::default(),
        )
        .unwrap();
        let stream = Stream::from_document(&archive.document).unwrap();
        let builder = stream
            .diagram
            .nodes
            .iter()
            .find(|n| n.type_identifier() == "BuildPCANode")
            .unwrap();
        let context = ExecContext {
            files: FileResolver {
                data_dirs: vec![root.join("corpus/models_next")],
            },
            attachments: Arc::new(
                archive
                    .attachments
                    .into_iter()
                    .map(|a| (a.name, Arc::from(a.bytes)))
                    .collect(),
            ),
            stream_properties: stream.properties.clone().map(Arc::new),
            ..ExecContext::default()
        };
        let results = Plan::new(&stream).evaluate(&[builder.uid.as_str()], &context);
        let trained = match results.get(&builder.uid) {
            Some(Ok(NodeOutput::Model(model))) => model.clone(),
            other => panic!("{case}: builder failed: {other:?}"),
        };
        let dir = root.join(format!("oracle/models_next/{case}/models"));
        let file = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "xml"))
            .unwrap();
        let oracle = Pmml::parse(&std::fs::read_to_string(file).unwrap()).unwrap();
        let (ours, theirs) = (betas(&trained.model), betas(&oracle.model));
        assert_eq!(ours.len(), theirs.len(), "{case}: factor count");
        for (factor, (a, b)) in ours.iter().zip(&theirs).enumerate() {
            assert_eq!(a.len(), b.len(), "{case}: factor {factor} parameters");
            for ((pa, va), (pb, vb)) in a.iter().zip(b) {
                assert_eq!(pa, pb, "{case}: factor {factor}");
                assert!(
                    (va - vb).abs() <= 1e-10 * vb.abs().max(1.0),
                    "{case}: factor {factor} {pa}: ours {va}, Modeler {vb}"
                );
            }
        }
        println!("FACTOR {case}: {} factors equal Modeler's", ours.len());
    }
}
