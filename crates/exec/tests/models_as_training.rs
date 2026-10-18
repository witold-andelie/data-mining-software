//! Evaluate the builders directly: a saved nugget cannot mask failed training.
#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{Expr, Model, Node, Pmml};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use std::sync::Arc;

fn close(a: f64, b: f64, context: &str) {
    assert!(
        a.is_finite() && b.is_finite() && (a - b).abs() <= 1e-10 * b.abs().max(1.0),
        "{context}: ours {a}, Modeler {b}"
    );
}

#[test]
fn unverified_tree_as_settings_return_gap() {
    use dms_strfmt::fastinfoset::Node as XmlNode;
    let root = support::repo_root();
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(root.join("corpus/models_as/treeas_default.str")).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.class.ends_with("ExtensionBuildModelNode"))
        .unwrap();
    for (key, value) in [
        ("_max_depth", "-1"),
        ("_max_depth", "99999999999999999999"),
        ("_max_depth", "NaN"),
        ("_num_bins", "10"),
        ("_split_alpha", "0.1"),
        ("_merge_alpha", "NaN"),
        ("_method", "exhaustive"),
        ("_chi_square", "LR"),
        ("_split_merged_categories", "true"),
        ("_use_costs", "true"),
        ("_bonferroni_adjustment", "false"),
        ("_record_threshold", "1"),
        ("_effect_size_threshold_cont", "0.1"),
        ("_min_child_records_pc", "2"),
        ("_minimum_record_use", "use_absolute"),
        ("autoModelFieldSelection", "false"),
        ("useWeight", "true"),
        ("useFrequency", "true"),
        ("_display_rule_id", "invalid"),
    ] {
        let mut node = builder.clone();
        let property = node
            .settings
            .as_mut()
            .unwrap()
            .children
            .iter_mut()
            .find_map(|n| match n {
                XmlNode::Element(e) if &*e.name.local == key => Some(e),
                _ => None,
            })
            .unwrap();
        property.children = vec![XmlNode::Text(Arc::from(value))];
        assert!(
            matches!(dms_exec::nodes::build::build(&node), Err(dms_exec::OperatorError::Settings(s)) if s.starts_with("GAP:")),
            "{key}={value}"
        );
    }
}

#[test]
fn linear_as_selection_gap_is_measured_and_rejected() {
    use dms_algo::{
        Dataset,
        linear::{self, LinearSettings, Selection},
    };
    use dms_types::Role;
    let root = support::repo_root();
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(root.join("corpus/models_as/linearas_default.str")).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.class.ends_with("ExtensionBuildModelNode"))
        .unwrap();
    let input = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "TypeNode")
        .unwrap();
    let context = ExecContext {
        files: FileResolver {
            data_dirs: vec![root.join("corpus/models_as")],
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
    let results =
        Plan::new(&stream).evaluate(&[builder.uid.as_str(), input.uid.as_str()], &context);
    assert!(
        matches!(results.get(&builder.uid), Some(Err(dms_exec::NodeError { kind: dms_exec::ErrorKind::Settings(s), .. })) if s.starts_with("GAP:") && s.contains("7 parameters"))
    );
    assert!(!dms_exec::registry::is_implemented("linear_as"));
    let Some(Ok(NodeOutput::Data(table))) = results.get(&input.uid) else {
        panic!("training data failed");
    };
    let fields = table.model().fields();
    let target = fields.iter().position(|f| f.role == Role::Target).unwrap();
    let inputs: Vec<_> = fields
        .iter()
        .enumerate()
        .filter(|(_, f)| f.role == Role::Input)
        .map(|(i, _)| i)
        .collect();
    let fitted = linear::train(
        &Dataset {
            table,
            target,
            inputs: &inputs,
            frequency: None,
        },
        &LinearSettings {
            selection: Selection::AdjustedRSquare,
            ..LinearSettings::default()
        },
    )
    .unwrap();
    let oracle = Pmml::parse(
        &std::fs::read_to_string(
            root.join("oracle/models_as/linearas_default/models/01_models_as.xml"),
        )
        .unwrap(),
    )
    .unwrap();
    let (Model::GeneralRegression(a), Model::GeneralRegression(b)) = (&fitted.model, &oracle.model)
    else {
        panic!("expected generalLinear");
    };
    assert_eq!(a.param_matrix.len(), 6);
    assert_eq!(b.param_matrix.len(), 7);
    assert_eq!(a.pp_matrix, &b.pp_matrix[..5]);
    let extra = b.pp_matrix.last().unwrap();
    assert_eq!((&*extra.predictor, &*extra.value), ("x4", "2"));
    let mut max_error = 0.0_f64;
    for (a, b) in a.param_matrix.iter().zip(&b.param_matrix) {
        close(a.beta, b.beta, &a.parameter);
        max_error = max_error.max((a.beta - b.beta).abs());
    }
    println!(
        "GAP linearas_default: reused Linear 6 parameters, Modeler 7; max common coefficient error {max_error:e}; oracle x4^2 beta {:e}",
        b.param_matrix.last().unwrap().beta
    );
}

fn tree(a: &Node, b: &Node, case: &str) -> usize {
    let context = format!("{case} node {:?}", b.id);
    assert_eq!(a.id, b.id, "{context}");
    assert_eq!(a.predicate, b.predicate, "{context}");
    assert_eq!(a.default_child, b.default_child, "{context}");
    close(a.record_count.unwrap(), b.record_count.unwrap(), &context);
    match (
        a.score.as_ref().unwrap().parse::<f64>(),
        b.score.as_ref().unwrap().parse::<f64>(),
    ) {
        (Ok(a), Ok(b)) => close(a, b, &context),
        _ => assert_eq!(a.score, b.score, "{context}"),
    }
    assert_eq!(a.distribution.len(), b.distribution.len(), "{context}");
    for (a, b) in a.distribution.iter().zip(&b.distribution) {
        assert_eq!(a.value, b.value, "{context}");
        close(a.record_count, b.record_count, &context);
        close(a.probability.unwrap(), b.probability.unwrap(), &context);
        close(a.confidence.unwrap(), b.confidence.unwrap(), &context);
    }
    assert_eq!(a.children.len(), b.children.len(), "{context}: children");
    1 + a
        .children
        .iter()
        .zip(&b.children)
        .map(|(a, b)| tree(a, b, case))
        .sum::<usize>()
}

#[test]
fn trained_tree_as_models_equal_modeler_pmml() {
    let root = support::repo_root();
    for case in ["treeas_default", "treeas_depth", "treeas_numeric"] {
        let archive = dms_strfmt::container::read_archive(
            &std::fs::read(root.join(format!("corpus/models_as/{case}.str"))).unwrap(),
            &ArchiveLimits::default(),
        )
        .unwrap();
        let stream = Stream::from_document(&archive.document).unwrap();
        let builder = stream
            .diagram
            .nodes
            .iter()
            .find(|n| n.class.ends_with("ExtensionBuildModelNode"))
            .unwrap();
        let context = ExecContext {
            files: FileResolver {
                data_dirs: vec![root.join("corpus/models_as")],
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
            Some(Ok(NodeOutput::Model(m))) => m,
            other => panic!("{case}: builder failed: {other:?}"),
        };
        let oracle = Pmml::parse(
            &std::fs::read_to_string(
                root.join(format!("oracle/models_as/{case}/models/01_models_as.xml")),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            trained.transformations.len(),
            oracle.transformations.len(),
            "{case}"
        );
        for (a, b) in trained.transformations.iter().zip(&oracle.transformations) {
            assert_eq!(a.name, b.name, "{case}");
            assert_eq!(a.optype, b.optype, "{case}");
            assert_eq!(a.data_type, b.data_type, "{case}");
            let (
                Expr::Discretize {
                    field: af,
                    bins: ab,
                    ..
                },
                Expr::Discretize {
                    field: bf,
                    bins: bb,
                    ..
                },
            ) = (&a.expr, &b.expr)
            else {
                panic!("expected discretization")
            };
            assert_eq!(af, bf, "{case}");
            assert_eq!(ab.len(), bb.len(), "{case}");
            for (a, b) in ab.iter().zip(bb) {
                assert_eq!(
                    (a.left, a.right, a.left_closed, a.right_closed),
                    (b.left, b.right, b.left_closed, b.right_closed),
                    "{case} {af}"
                );
                let number = |v: &dms_pmml::Value| match v {
                    dms_pmml::Value::Number(n) => *n,
                    dms_pmml::Value::Text(s) => s.parse::<f64>().unwrap(),
                    _ => panic!("missing bin value"),
                };
                close(number(&a.value), number(&b.value), &format!("{case} {af}"));
            }
        }
        match (&trained.model, &oracle.model) {
            (Model::Tree(a), Model::Tree(b)) => {
                assert_eq!(a.header.algorithm, b.header.algorithm);
                assert_eq!(a.header.function, b.header.function);
                assert_eq!(a.missing_value_strategy, b.missing_value_strategy);
                assert_eq!(a.no_true_child_strategy, b.no_true_child_strategy);
                let count = tree(&a.root, &b.root, case);
                println!(
                    "TRAINED {case}: {count} nodes and {} discretizations equal Modeler's",
                    trained.transformations.len()
                );
            }
            _ => panic!("{case}: wrong model type"),
        }
    }
}
