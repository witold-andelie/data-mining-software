//! Anomaly Detection training: the trained peer groups equal Modeler's exported PMML
//! (`oracle/models_next/anomaly_default`) and the cutoff equals the nugget's saved
//! `cutoffValue`. `xtask compare` alone cannot show this, since a failing builder leaves the
//! nugget its saved model.
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

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12 * b.abs().max(1.0)
}

#[test]
fn trained_peer_groups_equal_modeler_model() {
    let root = support::repo_root();
    let case = "anomaly_default";
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
        .find(|n| n.type_identifier() == "BuildAnomalyNode")
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
    let (Model::Clustering(ours), Model::Clustering(theirs)) = (&trained.model, &oracle.model)
    else {
        panic!("expected clustering models");
    };
    let fields = |m: &dms_pmml::ClusteringModel| {
        m.fields
            .iter()
            .map(|f| (f.field.clone(), f.weight))
            .collect::<Vec<_>>()
    };
    for ((a, wa), (b, wb)) in fields(ours).iter().zip(&fields(theirs)) {
        assert_eq!(a, b);
        assert!(close(*wa, *wb), "{a}: weight {wa} vs {wb}");
    }
    assert_eq!(ours.fields.len(), theirs.fields.len());
    assert_eq!(ours.clusters.len(), theirs.clusters.len(), "peer groups");
    for (i, (a, b)) in ours.clusters.iter().zip(&theirs.clusters).enumerate() {
        assert_eq!(a.size, b.size, "cluster {i} size");
        for (x, y) in a.center.iter().zip(&b.center) {
            assert!(close(*x, *y), "cluster {i} mean {x} vs {y}");
        }
        for (x, y) in a.variances.iter().zip(&b.variances) {
            assert!(close(*x, *y), "cluster {i} variance {x} vs {y}");
        }
        assert_eq!(a.category_counts, b.category_counts, "cluster {i} counts");
        // Modeler's own sum differs in the 15th digit (4.263016123503995 here, 4.26301612350401
        // saved).
        let (x, y) = (a.mean_distance.unwrap(), b.mean_distance.unwrap());
        assert!(close(x, y), "cluster {i} mean distance {x} vs {y}");
    }
    let replacements = |m: &dms_pmml::ClusteringModel| {
        m.header
            .mining_schema
            .iter()
            .map(|f| (f.name.clone(), f.missing_replacement.clone()))
            .collect::<Vec<_>>()
    };
    let (ours_r, theirs_r) = (replacements(ours), replacements(theirs));
    for (a, b) in ours_r.iter().zip(&theirs_r) {
        assert_eq!(a.0, b.0);
        let parse = |t: &Option<String>| t.as_deref().map(|t| t.parse::<f64>().unwrap());
        assert_eq!(parse(&a.1), parse(&b.1), "{} replacement", a.0);
    }
    assert_eq!(ours.anomaly_cutoff, Some(1.37201), "cutoff");
    println!(
        "ANOMALY {case}: {} peer groups and cutoff equal Modeler's",
        ours.clusters.len()
    );
}
