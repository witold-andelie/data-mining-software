//! C5.0 oracle contract: match exports by manifest node label, never file number.
#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;
use dms_pmml::{Pmml, Value};
use serde_json::Value as Json;
use std::path::Path;

pub const CASES: &[&str] = &[
    "c50_boost_rules",
    "c50_boost_tree",
    "c50_costs",
    "c50_default",
    "c50_expert",
    "c50_generality",
    "c50_group_symbols",
    "c50_min_child",
    "c50_noise20",
    "c50_no_global_prune",
    "c50_partition",
    "c50_prune90",
    "c50_rules_firsthit",
    "c50_rules_voting",
    "c50_weight",
    "c50_winnow",
];

#[test]
fn modeler_c50_pmml_scores_every_oracle_record() {
    let mut total = 0;
    for (label, case) in CASES
        .iter()
        .map(|c| ("models_ml", *c))
        .chain([("models", "c50_default")])
    {
        let dir = support::repo_root().join("oracle").join(label).join(case);
        let manifest: Json =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap())
                .unwrap();
        for entry in manifest["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["type"] == "applyc50")
        {
            let model_entry = manifest["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["label"] == entry["label"])
                .unwrap();
            let file = Path::new(model_entry["file"].as_str().unwrap())
                .file_name()
                .unwrap();
            let pmml =
                Pmml::parse(&std::fs::read_to_string(dir.join("models").join(file)).unwrap())
                    .unwrap();
            let file = Path::new(entry["file"].as_str().unwrap())
                .file_name()
                .unwrap();
            let text = std::fs::read_to_string(dir.join("data").join(file)).unwrap();
            let mut lines = text.lines();
            let header = support::parse_csv_line(lines.next().unwrap());
            let names: Vec<&str> = header.iter().map(String::as_str).collect();
            let scorer = pmml.bind(&names).unwrap();
            let value_index = header.iter().position(|s| s.starts_with("$C-")).unwrap();
            let confidence_index = header.iter().position(|s| s.starts_with("$CC-")).unwrap();
            let mut rows = 0;
            for (i, line) in lines.enumerate() {
                let cells = support::parse_csv_line(line);
                let record: Vec<Value> = cells
                    .iter()
                    .map(|c| {
                        if c.is_empty() {
                            Value::Missing
                        } else {
                            c.parse()
                                .map_or_else(|_| Value::Text(c.clone()), Value::Number)
                        }
                    })
                    .collect();
                let p = scorer.score(&record).unwrap();
                assert_eq!(
                    p.value,
                    Value::Text(cells[value_index].clone()),
                    "{label}/{case} row {i}"
                );
                let expected: f64 = cells[confidence_index].parse().unwrap();
                assert!(
                    (p.confidence.unwrap() - expected).abs() < 1e-12,
                    "{label}/{case} row {i}: {:?} vs {expected}",
                    p.confidence
                );
                rows += 1;
            }
            println!(
                "PMML {label}/{case} {}: {rows}/{rows} scores",
                entry["terminal"]["label"]
            );
            total += rows;
        }
    }
    assert_eq!(total, 8809);
}

fn compare_tree(a: &dms_pmml::Node, b: &dms_pmml::Node) {
    assert_eq!(a.id, b.id);
    assert_eq!(a.score, b.score, "node {:?}", b.id);
    assert_eq!(a.record_count, b.record_count, "node {:?}", b.id);
    assert_eq!(a.default_child, b.default_child, "node {:?}", b.id);
    match (&a.predicate, &b.predicate) {
        (
            dms_pmml::Predicate::Simple {
                field: af,
                operator: ao,
                value: av,
            },
            dms_pmml::Predicate::Simple {
                field: bf,
                operator: bo,
                value: bv,
            },
        ) => {
            assert_eq!(af, bf);
            assert_eq!(ao, bo);
            if let (Some(av), Some(bv)) = (
                av.as_ref().and_then(|s| s.parse::<f64>().ok()),
                bv.as_ref().and_then(|s| s.parse::<f64>().ok()),
            ) {
                assert!((av - bv).abs() < 1e-12, "node {:?} cut {av} vs {bv}", b.id);
            } else {
                assert_eq!(av, bv);
            }
        }
        (a, b) => assert_eq!(a, b),
    }
    assert_eq!(a.distribution, b.distribution, "node {:?}", b.id);
    assert_eq!(a.children.len(), b.children.len(), "node {:?}", b.id);
    for (a, b) in a.children.iter().zip(&b.children) {
        compare_tree(a, b);
    }
}

#[test]
fn c50_builders_match_every_split_count_and_distribution() {
    use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
    use dms_stream::Stream;
    use dms_strfmt::ArchiveLimits;
    use std::collections::HashMap;
    use std::sync::Arc;
    let root = support::repo_root();
    for (label, case) in [
        "c50_default",
        "c50_expert",
        "c50_min_child",
        "c50_group_symbols",
        "c50_no_global_prune",
        "c50_prune90",
    ]
    .iter()
    .map(|c| ("models_ml", *c))
    .chain([("models", "c50_default")])
    {
        let bytes = std::fs::read(root.join(format!("corpus/{label}/{case}.str"))).unwrap();
        let archive =
            dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
        let stream = Stream::from_document(&archive.document).unwrap();
        let uid = stream
            .diagram
            .nodes
            .iter()
            .find(|n| n.type_identifier() == "BuildC5RuleNode")
            .unwrap()
            .uid
            .as_str();
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
            attachments: Arc::new(attachments),
            stream_properties: stream.properties.clone().map(Arc::new),
            ..ExecContext::default()
        };
        let results = Plan::new(&stream).evaluate(&[uid], &context);
        let ours = match results.get(uid).unwrap() {
            Ok(NodeOutput::Model(m)) => m,
            other => panic!("{label}/{case}: {other:?}"),
        };
        let dir = root.join(format!("oracle/{label}/{case}"));
        let manifest: Json =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap())
                .unwrap();
        let model = &manifest["models"][0];
        let path = dir.join("models").join(
            Path::new(model["file"].as_str().unwrap())
                .file_name()
                .unwrap(),
        );
        let theirs = Pmml::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        println!("TREE {label}/{case}");
        match (&ours.model, &theirs.model) {
            (dms_pmml::Model::Tree(a), dms_pmml::Model::Tree(b)) => compare_tree(&a.root, &b.root),
            _ => panic!("expected trees"),
        }
    }
}
