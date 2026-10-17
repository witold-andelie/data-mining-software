//! C5.0 rule-set training: Modeler PMML for `c50_rules_voting` and `c50_rules_firsthit`.
#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;
use dms_pmml::{Model, Pmml, Predicate, RuleSetModel, SimpleRule, Value};
use serde_json::Value as Json;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

const CASES: &[&str] = &["c50_rules_voting", "c50_rules_firsthit"];

#[test]
fn c50_rule_sets_match_modeler_and_score_every_oracle_record() {
    use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
    use dms_stream::Stream;
    use dms_strfmt::ArchiveLimits;

    let root = support::repo_root();
    let mut scored = 0;
    for case in CASES {
        let bytes = std::fs::read(root.join(format!("corpus/models_ml/{case}.str"))).unwrap();
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
                    root.join("corpus/models_ml"),
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
            other => panic!("{case}: {other:?}"),
        };
        let dir = root.join(format!("oracle/models_ml/{case}"));
        let manifest: Json =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap())
                .unwrap();
        let model = manifest["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["label"] == "models_ml")
            .unwrap();
        let file = Path::new(model["file"].as_str().unwrap())
            .file_name()
            .unwrap();
        let theirs =
            Pmml::parse(&std::fs::read_to_string(dir.join("models").join(file)).unwrap()).unwrap();
        println!("RULES models_ml/{case}");
        match (&ours.model, &theirs.model) {
            (Model::RuleSet(a), Model::RuleSet(b)) => compare_rule_sets(a, b),
            _ => panic!("{case}: expected rule sets"),
        }
        for entry in manifest["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["type"] == "applyc50")
        {
            let file = Path::new(entry["file"].as_str().unwrap())
                .file_name()
                .unwrap();
            let text = std::fs::read_to_string(dir.join("data").join(file)).unwrap();
            let mut lines = text.lines();
            let header = support::parse_csv_line(lines.next().unwrap());
            let names: Vec<&str> = header.iter().map(String::as_str).collect();
            let scorer = ours.bind(&names).unwrap();
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
                    "{case} row {i}"
                );
                let expected: f64 = cells[confidence_index].parse().unwrap();
                assert!(
                    (p.confidence.unwrap() - expected).abs() < 1e-12,
                    "{case} row {i}: {:?} vs {expected}",
                    p.confidence
                );
                rows += 1;
            }
            println!(
                "TRAINED {case} {}: {rows}/{rows} scores",
                entry["terminal"]["label"]
            );
            scored += rows;
        }
    }
    println!("scored {scored} oracle records");
}

fn compare_rule_sets(a: &RuleSetModel, b: &RuleSetModel) {
    assert_eq!(a.selection, b.selection, "selection");
    assert_eq!(a.default_score, b.default_score, "defaultScore");
    assert_eq!(
        a.default_confidence, b.default_confidence,
        "defaultConfidence"
    );
    assert_eq!(a.rules.len(), b.rules.len(), "rule count");
    for (i, (ours, theirs)) in a.rules.iter().zip(&b.rules).enumerate() {
        compare_rule(i, ours, theirs);
    }
}

fn compare_rule(i: usize, a: &SimpleRule, b: &SimpleRule) {
    assert_eq!(a.id, b.id, "rule {i} id");
    assert_eq!(a.score, b.score, "rule {i} score");
    compare_predicate(i, &a.predicate, &b.predicate);
    assert_eq!(a.record_count, b.record_count, "rule {i} recordCount");
    assert_eq!(a.correct, b.correct, "rule {i} nbCorrect");
    assert!(
        (a.confidence - b.confidence).abs() < 1e-12,
        "rule {i} confidence {} vs {}",
        a.confidence,
        b.confidence
    );
    assert!(
        (a.weight - b.weight).abs() < 1e-12,
        "rule {i} weight {} vs {}",
        a.weight,
        b.weight
    );
}

fn compare_predicate(i: usize, a: &Predicate, b: &Predicate) {
    match (a, b) {
        (
            Predicate::Simple {
                field: af,
                operator: ao,
                value: av,
            },
            Predicate::Simple {
                field: bf,
                operator: bo,
                value: bv,
            },
        ) => {
            assert_eq!(af, bf, "rule {i} field");
            assert_eq!(ao, bo, "rule {i} operator");
            if let (Some(av), Some(bv)) = (
                av.as_ref().and_then(|s| s.parse::<f64>().ok()),
                bv.as_ref().and_then(|s| s.parse::<f64>().ok()),
            ) {
                assert!((av - bv).abs() < 1e-12, "rule {i} cut {av} vs {bv}");
            } else {
                assert_eq!(av, bv, "rule {i} value");
            }
        }
        (
            Predicate::Compound {
                operator: ao,
                predicates: ap,
            },
            Predicate::Compound {
                operator: bo,
                predicates: bp,
            },
        ) => {
            assert_eq!(ao, bo, "rule {i} boolean");
            assert_eq!(ap.len(), bp.len(), "rule {i} condition count");
            for (a, b) in ap.iter().zip(bp) {
                compare_predicate(i, a, b);
            }
        }
        (a, b) => assert_eq!(a, b, "rule {i} predicate"),
    }
}
