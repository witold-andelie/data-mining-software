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
                assert_eq!(av, bv, "node {:?} cut", b.id);
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
    for (label, case) in [
        "c50_default",
        "c50_expert",
        "c50_min_child",
        "c50_group_symbols",
        "c50_no_global_prune",
        "c50_prune90",
        "c50_costs",
    ]
    .iter()
    .map(|c| ("models_ml", *c))
    .chain([
        ("models", "c50_default"),
        ("c50_weighted", "c50_x4_unit"),
        ("c50_weighted", "c50_x8t_unit"),
        ("c50_weighted", "c50_w2_unit_loose"),
    ]) {
        Fixture::load(label, case).compare();
    }
}

/// Case weights through the algorithm (the node still rejects `useWeight`): probe
/// `c50_weighted_streams`, settings as each stream sets them.
#[test]
fn c50_weighted_trees_match_every_split_count_and_distribution() {
    use dms_algo::c50::C50Settings;
    let expert = |severity: f64, global: bool| C50Settings {
        pruning_severity: severity,
        global_pruning: global,
        ..C50Settings::default()
    };
    for (case, settings) in [
        ("c50_w2_default", C50Settings::default()),
        ("c50_w2_noglobal", expert(75.0, false)),
        ("c50_w2_sev50", expert(50.0, true)),
        ("c50_x4_w2", C50Settings::default()),
        ("c50_x4_w2_loose", expert(1.0, false)),
    ] {
        let fixture = Fixture::load("c50_weighted", case);
        let ours = fixture.train_weighted(settings);
        println!("TREE c50_weighted/{case}");
        match (&ours.model, &fixture.oracle.model) {
            (dms_pmml::Model::Tree(a), dms_pmml::Model::Tree(b)) => compare_tree(&a.root, &b.root),
            _ => panic!("expected trees"),
        }
    }
}

struct Fixture {
    stream: dms_stream::Stream,
    context: dms_exec::ExecContext,
    uid: String,
    label: String,
    case: String,
    dir: std::path::PathBuf,
    manifest: Json,
    oracle: Pmml,
}

impl Fixture {
    fn load(label: &str, case: &str) -> Self {
        use dms_exec::{ExecContext, FileResolver};
        use dms_stream::Stream;
        use dms_strfmt::ArchiveLimits;
        use std::sync::Arc;
        let root = support::repo_root();
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
            .clone();
        let context = ExecContext {
            files: FileResolver {
                data_dirs: vec![
                    root.join(format!("corpus/{label}")),
                    root.join("corpus/ibm-demo-data"),
                ],
            },
            attachments: Arc::new(
                archive
                    .attachments
                    .into_iter()
                    .map(|a| (a.name, Arc::<[u8]>::from(a.bytes)))
                    .collect(),
            ),
            stream_properties: stream.properties.clone().map(Arc::new),
            ..ExecContext::default()
        };
        let dir = root.join(format!("oracle/{label}/{case}"));
        let manifest: Json =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap())
                .unwrap();
        // Builder labels can be generated and absent from StreamNode::label. Resolve
        // the UID through the oracle run manifest, then match the export by label.
        let builder_label = manifest["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["node"]["id"] == uid)
            .unwrap()["node"]["label"]
            .as_str()
            .unwrap();
        let model = manifest["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["label"] == builder_label)
            .unwrap();
        let file = Path::new(model["file"].as_str().unwrap())
            .file_name()
            .unwrap();
        let oracle =
            Pmml::parse(&std::fs::read_to_string(dir.join("models").join(file)).unwrap()).unwrap();
        Self {
            stream,
            context,
            uid,
            label: label.into(),
            case: case.into(),
            dir,
            manifest,
            oracle,
        }
    }

    /// Train on the builder's input table with its `weight` field as case weights.
    fn train_weighted(&self, settings: dms_algo::c50::C50Settings) -> Pmml {
        use dms_exec::{NodeOutput, Plan};
        use dms_types::Role;
        let source = self.manifest["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["type"] == "type" && e["terminal"]["id"] == self.uid)
            .unwrap()["id"]
            .as_str()
            .unwrap();
        let results = Plan::new(&self.stream).evaluate(&[source], &self.context);
        let table = match results.get(source).unwrap() {
            Ok(NodeOutput::Data(t)) => t,
            other => panic!("weighted training input: {other:?}"),
        };
        let fields = table.model().fields();
        let target = fields.iter().position(|f| f.role == Role::Target).unwrap();
        let inputs: Vec<usize> = fields
            .iter()
            .enumerate()
            .filter(|(_, f)| f.role == Role::Input)
            .map(|(i, _)| i)
            .collect();
        dms_algo::c50::train_tree(
            &dms_algo::Dataset {
                table,
                target,
                inputs: &inputs,
                frequency: None,
            },
            &dms_algo::c50::C50Settings {
                weight_field: table.model().position("weight"),
                ..settings
            },
        )
        .unwrap()
    }

    fn compare(&self) {
        use dms_exec::{NodeOutput, Plan};
        let results = Plan::new(&self.stream).evaluate(&[&self.uid], &self.context);
        let ours = match results.get(&self.uid).unwrap() {
            Ok(NodeOutput::Model(m)) => m,
            other => panic!("{}/{}: {other:?}", self.label, self.case),
        };
        println!("TREE {}/{}", self.label, self.case);
        match (&ours.model, &self.oracle.model) {
            (dms_pmml::Model::Tree(a), dms_pmml::Model::Tree(b)) => compare_tree(&a.root, &b.root),
            _ => panic!("expected trees"),
        }
        let mut rows = 0;
        for entry in self.manifest["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["type"] == "applyc50")
        {
            let file = Path::new(entry["file"].as_str().unwrap())
                .file_name()
                .unwrap();
            let text = std::fs::read_to_string(self.dir.join("data").join(file)).unwrap();
            let mut lines = text.lines();
            let header = support::parse_csv_line(lines.next().unwrap());
            let names: Vec<&str> = header.iter().map(String::as_str).collect();
            let scorer = ours.bind(&names).unwrap();
            let expected = self.oracle.bind(&names).unwrap();
            for line in lines {
                let record: Vec<Value> = support::parse_csv_line(line)
                    .iter()
                    .map(|c| {
                        c.parse()
                            .map_or_else(|_| Value::Text(c.clone()), Value::Number)
                    })
                    .collect();
                assert_eq!(
                    scorer.score(&record).unwrap(),
                    expected.score(&record).unwrap(),
                    "{}/{} score {rows}",
                    self.label,
                    self.case
                );
                rows += 1;
            }
        }
        println!(
            "TRAINED {}/{}: {rows}/{rows} exact PMML scores",
            self.label, self.case
        );
    }
}

#[test]
#[ignore = "GAP: weighted pruning, zero distributions, internal count aggregation; useWeight stays rejected"]
fn c50_weight_builder_matches_every_split_count_and_distribution() {
    Fixture::load("models_ml", "c50_weight").compare();
}

#[test]
fn c50_weight_remains_rejected() {
    let fixture = Fixture::load("models_ml", "c50_weight");
    let results = dms_exec::Plan::new(&fixture.stream).evaluate(&[&fixture.uid], &fixture.context);
    let error = results.get(&fixture.uid).unwrap().as_ref().unwrap_err();
    assert!(error.to_string().contains("case weights"), "{error}");
    println!(
        "GAP models_ml/c50_weight: builder rejects case weights; exact training contract remains ignored"
    );
}

#[test]
fn c50_weight_export_precision_and_unresolved_nodes() {
    let fixture = Fixture::load("models_ml", "c50_weight");
    let ours = fixture.train_weighted(dms_algo::c50::C50Settings::default());
    let (dms_pmml::Model::Tree(ours), dms_pmml::Model::Tree(oracle)) =
        (&ours.model, &fixture.oracle.model)
    else {
        panic!("expected trees");
    };
    let mut exact_nodes = 0;
    let mut gaps = Vec::new();
    let mut internal_counts = Vec::new();
    compare_weighted_evidence(
        &ours.root,
        &oracle.root,
        &mut exact_nodes,
        &mut gaps,
        &mut internal_counts,
    );
    assert_eq!(gaps, ["23", "24", "34"]);
    assert_eq!(internal_counts, ["28"]);
    assert_eq!(exact_nodes, 25);
    println!(
        "WEIGHT evidence: {exact_nodes} nodes have exact exported counts/distributions; GAP nodes {gaps:?}, internal distributions {internal_counts:?}, and affected ancestors remain unmatched"
    );
}

/// This is an evidence test, NOT the full weighted training contract above.
/// Identify pruning, zero-distribution, and internal-count mismatches. Exact
/// comparisons cover leaves and the internal nodes whose aggregation is settled.
fn compare_weighted_evidence(
    a: &dms_pmml::Node,
    b: &dms_pmml::Node,
    exact: &mut usize,
    gaps: &mut Vec<String>,
    internal_counts: &mut Vec<String>,
) -> bool {
    assert_eq!(a.predicate, b.predicate, "node {:?}", b.id);
    assert_eq!(a.score, b.score, "node {:?}", b.id);
    match b.id.as_deref() {
        Some("23" | "24") => {
            assert!(a.record_count.is_some_and(|c| c > 0.0));
            assert!(b.record_count.is_some_and(|c| c > 0.0));
            assert!(b.distribution.iter().all(|d| d.record_count == 0.0
                && d.probability == Some(0.0)
                && d.confidence == Some(0.0)));
            assert!(a.distribution.iter().any(|d| d.record_count > 0.0));
            assert!(a.children.is_empty() && b.children.is_empty());
            gaps.push(b.id.clone().unwrap());
            return true;
        }
        Some("34") => {
            assert_eq!(a.children.len(), 2);
            assert!(b.children.is_empty());
            gaps.push("34".into());
            return true;
        }
        _ => {}
    }
    assert_eq!(a.children.len(), b.children.len(), "node {:?}", b.id);
    let mut affected = false;
    for (a, b) in a.children.iter().zip(&b.children) {
        affected |= compare_weighted_evidence(a, b, exact, gaps, internal_counts);
    }
    if !affected {
        assert_eq!(a.record_count, b.record_count, "node {:?}", b.id);
        if a.distribution != b.distribution {
            assert!(!b.children.is_empty(), "leaf {:?} must match", b.id);
            internal_counts.push(b.id.clone().unwrap());
        } else {
            *exact += 1;
        }
    }
    affected
}
