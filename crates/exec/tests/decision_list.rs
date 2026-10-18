//! Compare actual builder output with saved Modeler PMML, independently of nugget fallback.
#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{Model, Pmml, RuleSetModel};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::collections::HashMap;
use std::sync::Arc;

fn rules(pmml: &Pmml) -> &RuleSetModel {
    match &pmml.model {
        Model::RuleSet(m) => m,
        other => panic!("expected RuleSet: {other:?}"),
    }
}

fn train(label: &str, case: &str) -> Result<(Pmml, Pmml), String> {
    let root = support::repo_root();
    let bytes = std::fs::read(root.join(format!("corpus/{label}/{case}.str"))).unwrap();
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "BuildDecisionListNode")
        .unwrap();
    let attachments: HashMap<String, Arc<[u8]>> = archive
        .attachments
        .into_iter()
        .map(|a| (a.name, a.bytes.into()))
        .collect();
    let context = ExecContext {
        files: FileResolver {
            data_dirs: vec![
                root.join(format!("corpus/{label}")),
                root.join("corpus/ibm-demo-data"),
            ],
        },
        stream_properties: stream.properties.clone().map(Arc::new),
        attachments: Arc::new(attachments),
        decimal_places: Some(15),
        export_dir: None,
    };
    let result = Plan::new(&stream).evaluate(&[builder.uid.as_str()], &context);
    let ours = match result.get(builder.uid.as_str()).unwrap() {
        Ok(NodeOutput::Model(pmml)) => (**pmml).clone(),
        Err(error) => return Err(format!("{error:?}")),
        other => panic!("{label}/{case}: {other:?}"),
    };
    let path = std::fs::read_dir(root.join(format!("oracle/{label}/{case}/models")))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "xml"))
        .unwrap();
    let oracle = Pmml::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
    Ok((ours, oracle))
}

#[test]
fn trained_lists_match_modeler_and_refuse_unsettled_ties() {
    for (label, cases) in [
        (
            "train_next",
            &[
                "dl_default",
                "dl_max_rules2",
                "dl_max_rules4",
                "dl_search",
                "dl_min_group",
                "dl_confidence80",
                "dl_numeric",
            ][..],
        ),
        ("models_next2", &["dl_min_group"][..]),
    ] {
        for case in cases {
            let (ours, oracle) = train(label, case).unwrap();
            assert_eq!(rules(&ours), rules(&oracle), "{label}/{case}");
            println!(
                "{label}/{case}: {} rules equal (including remainder)",
                rules(&ours).rules.len()
            );
        }
    }
    // Pure segments: every candidate list ties on rate and coverage, in different rule
    // orders, and Modeler's pick (north, web, gold; web, gold) is not settled: the builder
    // refuses (GAP) so the nugget keeps its saved model.
    for case in ["dl_default", "dl_max_rules"] {
        let error = train("models_next2", case).unwrap_err();
        assert!(error.contains("GAP"), "models_next2/{case}: {error}");
        println!("models_next2/{case}: refused as a GAP");
    }
}

#[test]
fn unverified_search_and_weight_settings_are_rejected() {
    use dms_strfmt::fastinfoset::Node;
    let bytes =
        std::fs::read(support::repo_root().join("corpus/train_next/dl_default.str")).unwrap();
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "BuildDecisionListNode")
        .unwrap();
    let factory = dms_exec::registry::factory("decision_list").unwrap();
    assert!(dms_exec::registry::is_implemented("decision_list"));
    for (key, value) in [
        ("searchDirection", "down"),
        ("reuseFields", "false"),
        ("useFrequency", "true"),
        ("useWeight", "true"),
        ("useEvents", "true"),
        ("useInteractiveState", "true"),
    ] {
        let mut node = builder.clone();
        let element = node
            .settings
            .as_mut()
            .unwrap()
            .children
            .iter_mut()
            .find_map(|c| match c {
                Node::Element(e) if &*e.name.local == key => Some(e),
                _ => None,
            })
            .unwrap();
        element.children = vec![Node::Text(value.into())];
        assert!(factory(&node).is_err(), "{key}={value}");
    }
}
