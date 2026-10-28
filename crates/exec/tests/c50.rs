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
        "c50_generality",
        "c50_noise20",
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
        // P alone changes this tree; R5 is required to keep its exact contract.
        ("models_next2", "ens_voting"),
        ("c50_weighted", "c50_x4_unit"),
        ("c50_weighted", "c50_x8t_unit"),
        ("c50_weighted", "c50_w2_unit_loose"),
    ]) {
        Fixture::load(label, case).compare();
    }
}

/// Probe `c50_globalprune_streams`: global and local pruning, empty branches and the
/// many-values rule on synthetic exception blocks; every unweighted case, exactly.
#[test]
fn c50_globalprune_trees_match_every_split_count_and_distribution() {
    let root = support::repo_root();
    let mut cases: Vec<String> = std::fs::read_dir(root.join("corpus/c50_globalprune"))
        .unwrap()
        .filter_map(|e| {
            let name = e.unwrap().file_name().into_string().unwrap();
            name.strip_suffix(".str").map(str::to_owned)
        })
        .filter(|c| !c.contains("w2"))
        .collect();
    cases.sort();
    assert!(cases.len() >= 100, "{} cases", cases.len());
    for case in &cases {
        Fixture::load("c50_globalprune", case).compare();
    }
    println!("GLOBALPRUNE {} trees exact", cases.len());
}

fn recorded_preset_cases() -> Vec<String> {
    let mut cases: Vec<_> = std::fs::read_dir(support::repo_root().join("corpus/c50_presets"))
        .unwrap()
        .filter_map(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_str()
                .unwrap()
                .strip_suffix(".str")
                .map(str::to_owned)
        })
        .collect();
    cases.sort();
    assert_eq!(cases.len(), 49, "update the preset evidence inventory");
    cases
}

const PRESET_BASELINE_GAPS: &[&str] = &["c50_wn100_expert"];

/// Scores alone miss g02_zero: the two trees agree on the scoring grid but test
/// a/b in opposite orders. Keep the complete PMML contract for all 15 witnesses.
#[test]
fn c50_discriminate_trees_match_every_split_count_and_distribution() {
    for case in [
        "g01_nominal",
        "g02_zero",
        "g03_constant",
        "g04_negative",
        "g05_six_numeric",
        "g06_six_flag",
        "g07_six_no_c",
        "g08_penalty_m2",
        "g09_penalty_m4",
        "r01_relabel",
        "r02_regrow",
        "r03_move_value",
        "r04_selection",
        "r05_reprune",
        "r06_unpruned",
    ] {
        Fixture::load("c50_discriminate", case).compare();
    }
}

/// Frozen phase-3 predictions distinguish the largest branch from the largest
/// internal branch. In auto44 the largest branch is a leaf, while the smaller
/// west branch supplies the oracle's promoted age test. Keep the complete-tree
/// and scoring contracts: equal training-error totals alone miss wn23's GAP.
#[test]
fn c50_prune_internal_branch_controls_match_exactly() {
    for case in ["auto44", "auto_noglobal", "wn11", "auto8"] {
        Fixture::load("c50_prune", case).compare();
    }
}

#[test]
fn c50_simple_presets_and_expert_controls_match_exactly() {
    let cases = recorded_preset_cases();
    let mut count = 0;
    for case in cases
        .iter()
        .filter(|c| !c.contains("winnow") && !PRESET_BASELINE_GAPS.contains(&c.as_str()))
    {
        let fixture = Fixture::load("c50_presets", case);
        fixture.compare();
        count += 1;
        for tag in ["grid", "noisy", "drug"] {
            if case.starts_with(&format!("c50_{tag}_")) {
                let baseline = Fixture::load("c50_presets", &format!("c50_{tag}_simple"));
                assert_eq!(fixture.oracle.model, baseline.oracle.model, "{case}");
            }
        }
    }
    assert_eq!(count, 29);
    println!("PRESETS {count} Simple trees and Expert controls exact");
    // Explicitly train Expert defaults on the three Simple baseline datasets too.
    for tag in ["grid", "noisy", "drug"] {
        let mut fixture = Fixture::load("c50_presets", &format!("c50_{tag}_simple"));
        fixture.set_builder_setting("expertMode", "true");
        fixture.compare();
    }
}

/// P+R5 reproduces wn; wn100 still retains two subtrees absent from Modeler.
#[test]
#[ignore = "GAP: wn100 extra pruning beyond P+R5"]
fn c50_remaining_preset_expert_controls_match_exactly() {
    let mut mismatches = Vec::new();
    for case in PRESET_BASELINE_GAPS {
        if std::panic::catch_unwind(|| Fixture::load("c50_presets", case).compare()).is_err() {
            mismatches.push(*case);
        }
    }
    assert!(mismatches.is_empty(), "baseline GAPs: {mismatches:?}");
}

fn recorded_winnowing_cases() -> Vec<(&'static str, String)> {
    let mut cases: Vec<_> = recorded_preset_cases()
        .into_iter()
        .filter(|c| c.contains("winnow"))
        .map(|c| ("c50_presets", c))
        .collect();
    cases.push(("models_ml", "c50_winnow".into()));
    assert_eq!(cases.len(), 20);
    cases
}

#[test]
fn c50_winnowing_remains_rejected_for_every_recorded_case() {
    for (label, case) in recorded_winnowing_cases() {
        let fixture = Fixture::load(label, &case);
        let results =
            dms_exec::Plan::new(&fixture.stream).evaluate(&[&fixture.uid], &fixture.context);
        let error = results.get(&fixture.uid).unwrap().as_ref().unwrap_err();
        assert!(
            error.to_string().contains("winnowing (GAP c50_presets"),
            "{label}/{case}: {error}"
        );
    }
}

#[test]
#[ignore = "GAP: winnowing elimination criterion and unwinnowed baseline trees; all 20 must match before enabling"]
fn c50_all_winnowing_trees_match_exactly() {
    let mut mismatches = Vec::new();
    for (label, case) in recorded_winnowing_cases() {
        if std::panic::catch_unwind(|| Fixture::load(label, &case).compare()).is_err() {
            mismatches.push(format!("{label}/{case}"));
        }
    }
    assert!(mismatches.is_empty(), "winnowing GAPs: {mismatches:?}");
}

#[test]
fn c50_unrecorded_simple_presets_remain_explicit_errors() {
    for (key, value) in [
        ("expectedNoise", "-1"),
        ("expectedNoise", "7"),
        ("expectedNoise", "51"),
        ("expectedNoise", "NaN"),
        ("expectedNoise", "999999999999999999999"),
        ("modellingBias", "unknown"),
        ("modelFormat", "ruleSet"),
        ("useBoosting", "true"),
    ] {
        let mut fixture = Fixture::load("c50_presets", "c50_drug_noise20");
        fixture.set_builder_setting(key, value);
        let results =
            dms_exec::Plan::new(&fixture.stream).evaluate(&[&fixture.uid], &fixture.context);
        let error = results.get(&fixture.uid).unwrap().as_ref().unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("expectedNoise") || message.contains("Simple"),
            "{key}={value}: {error}"
        );
    }
    let mut fixture = Fixture::load("c50_presets", "c50_drug_generality");
    fixture.set_builder_setting("expectedNoise", "30");
    let results = dms_exec::Plan::new(&fixture.stream).evaluate(&[&fixture.uid], &fixture.context);
    let error = results.get(&fixture.uid).unwrap().as_ref().unwrap_err();
    assert!(error.to_string().contains("GAP c50_presets"), "{error}");
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

fn recorded_weighted_cases() -> Vec<(&'static str, String)> {
    let mut cases: Vec<_> = std::fs::read_dir(support::repo_root().join("corpus/c50_weighted"))
        .unwrap()
        .filter_map(|entry| {
            let name = entry.unwrap().file_name().into_string().unwrap();
            name.strip_suffix(".str")
                .filter(|name| !name.contains("_unit"))
                .map(|name| ("c50_weighted", name.to_owned()))
        })
        .collect();
    cases.sort();
    assert_eq!(cases.len(), 22, "update the weighted evidence inventory");
    cases.push(("models_ml", "c50_weight".into()));
    cases
}

fn weighted_settings(case: &str) -> dms_algo::c50::C50Settings {
    dms_algo::c50::C50Settings {
        pruning_severity: if case.ends_with("_loose") {
            1.0
        } else if case.ends_with("_sev50") {
            50.0
        } else {
            75.0
        },
        global_pruning: !case.ends_with("_loose") && !case.ends_with("_noglobal"),
        ..dms_algo::c50::C50Settings::default()
    }
}

/// Recorded evidence, not a zeroing heuristic or a successful training contract.
#[test]
fn c50_weighted_zero_distribution_inventory() {
    fn collect(node: &dms_pmml::Node, leaves: &mut usize, zeros: &mut Vec<String>) {
        if node.children.is_empty() && node.record_count.is_some_and(|n| n > 0.0) {
            *leaves += 1;
            if node.distribution.iter().all(|d| d.record_count == 0.0) {
                assert!(!node.distribution.is_empty());
                assert!(
                    node.distribution
                        .iter()
                        .all(|d| { d.probability == Some(0.0) && d.confidence == Some(0.0) })
                );
                zeros.push(node.id.clone().unwrap());
            }
        }
        for child in &node.children {
            collect(child, leaves, zeros);
        }
    }

    let (mut leaves, mut zero_count) = (0, 0);
    for (label, case) in recorded_weighted_cases() {
        let fixture = Fixture::load(label, &case);
        let dms_pmml::Model::Tree(tree) = &fixture.oracle.model else {
            panic!("expected tree: {label}/{case}");
        };
        let mut zeros = Vec::new();
        collect(&tree.root, &mut leaves, &mut zeros);
        let expected: &[&str] = match case.as_str() {
            "c50_code1_loose" => &["5", "10", "23"],
            "c50_cw_loose" | "c50_cw_noglobal" | "c50_cw_sev50" | "c50_weight" => &["23", "24"],
            "c50_rc1" | "c50_rc1_loose" => &["7"],
            "c50_rc1_x6_10" | "c50_rc1_x6_10_loose" => &["2"],
            "c50_red_loose" => &["5", "7", "8"],
            _ => &[],
        };
        assert_eq!(zeros, expected, "{label}/{case}");
        zero_count += zeros.len();
    }
    assert_eq!((leaves, zero_count), (240, 18));
}

/// Exercise every weighted fixture, including ones with no zero distributions.
/// The three unweighted controls are covered by the enabled builders test above.
#[test]
#[ignore = "GAP: zero distributions, aggregation, ordinal domains and global pruning; useWeight stays rejected"]
fn c50_all_weighted_trees_match_exactly() {
    let mut mismatches = Vec::new();
    let mut exact = 0;
    for (label, case) in recorded_weighted_cases() {
        let fixture = Fixture::load(label, &case);
        let ours = fixture.train_weighted(weighted_settings(&case));
        let (dms_pmml::Model::Tree(a), dms_pmml::Model::Tree(b)) =
            (&ours.model, &fixture.oracle.model)
        else {
            panic!("expected trees: {label}/{case}");
        };
        if a.root == b.root {
            exact += 1;
        } else {
            mismatches.push(format!("{label}/{case}"));
        }
    }
    println!("WEIGHTED exact={exact}, mismatches={mismatches:?}");
    assert!(mismatches.is_empty(), "weighted training is not exact");
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
    /// Modify only the in-memory stream; never write shared corpus/oracle files.
    fn set_builder_setting(&mut self, key: &str, value: &str) {
        use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
        let settings = self
            .stream
            .diagram
            .nodes
            .iter_mut()
            .find(|n| n.uid == self.uid)
            .unwrap()
            .settings
            .as_mut()
            .unwrap();
        let value_node = Node::Text(value.into());
        if let Some(element) = settings.children.iter_mut().find_map(|child| match child {
            Node::Element(e) if &*e.name.local == key => Some(e),
            _ => None,
        }) {
            element.children = vec![value_node];
        } else {
            settings.children.push(Node::Element(Element {
                name: QualifiedName {
                    prefix: None,
                    namespace: None,
                    local: key.into(),
                },
                namespace_declarations: Vec::new(),
                attributes: Vec::new(),
                children: vec![value_node],
            }));
        }
    }

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
