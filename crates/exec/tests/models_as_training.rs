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

fn train_linear_as(
    label: &str,
    case: &str,
) -> (Pmml, Vec<dms_algo::linear_as::ParameterInfo>, Pmml, String) {
    use dms_algo::{Dataset, linear_as};
    let root = support::repo_root();
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(root.join(format!("corpus/{label}/{case}.str"))).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "spss.linearEngine!linearEngine")
        .unwrap();
    let input = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "TypeNode")
        .unwrap();
    let context = ExecContext {
        files: FileResolver {
            data_dirs: vec![root.join(format!("corpus/{label}"))],
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
    let trained = match results.get(&builder.uid) {
        Some(Ok(NodeOutput::Model(m))) => (**m).clone(),
        other => panic!("{label}/{case}: builder failed: {other:?}"),
    };
    let Some(Ok(NodeOutput::Data(table))) = results.get(&input.uid) else {
        panic!("training data failed");
    };
    let settings = builder.settings.as_ref();
    let target = table
        .model()
        .position(&dms_exec::settings::text(settings, "_targetField").unwrap())
        .unwrap();
    let inputs: Vec<_> = dms_exec::settings::children(settings, "_inputFieldList", "field")
        .iter()
        .map(|f| table.model().position(&f.text()).unwrap())
        .collect();
    let with_metadata = linear_as::train(&Dataset {
        table,
        target,
        inputs: &inputs,
        frequency: None,
    })
    .unwrap();
    // The builder and the direct fit (which keeps labels and df) carry the same model.
    assert_eq!(trained, with_metadata.pmml);
    let oracle_path = std::fs::read_dir(root.join(format!("oracle/{label}/{case}/models")))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "xml"))
        .unwrap();
    let oracle_xml = std::fs::read_to_string(oracle_path).unwrap();
    let oracle = Pmml::parse(&oracle_xml).unwrap();
    (trained, with_metadata.parameters, oracle, oracle_xml)
}

#[test]
fn trained_linear_as_models_match_oracle_parameters_and_scoring() {
    assert!(dms_exec::registry::is_implemented("linear_as"));
    for (label, case) in [
        ("linearas", "lin_default"),
        ("linearas", "lin_factor"),
        ("models_as", "linearas_default"),
    ] {
        let (ours, ours_info, oracle, oracle_xml) = train_linear_as(label, case);
        let (Model::GeneralRegression(a), Model::GeneralRegression(b)) =
            (&ours.model, &oracle.model)
        else {
            panic!("expected generalLinear");
        };
        assert_eq!(ours.version, oracle.version);
        assert_eq!(ours.data_dictionary, oracle.data_dictionary);
        assert_eq!(a.header.algorithm, b.header.algorithm);
        assert_eq!(a.header.function, b.header.function);
        assert_eq!(a.model_type, b.model_type);
        let old = case == "linearas_default";
        let count = if old { 6 } else { b.parameters.len() };
        if old {
            // Exact-linear residuals make the oracle's extra x4 square floating noise.
            assert_eq!(b.parameters.len(), 7);
            assert_eq!(b.pp_matrix.last().unwrap().value, "2");
            assert_eq!(b.pp_matrix.last().unwrap().predictor, "x4");
            assert!(b.param_matrix.last().unwrap().beta.abs() < 2e-16);
        } else {
            assert_eq!(a.parameters, b.parameters);
            assert_eq!(a.header.mining_schema, b.header.mining_schema);
            assert_eq!(a.factors, b.factors);
            assert_eq!(a.covariates, b.covariates);
            assert_eq!(a.pp_matrix, b.pp_matrix);
        }
        let oracle_doc = roxmltree::Document::parse(&oracle_xml).unwrap();
        let attrs = |doc: &roxmltree::Document<'_>, tag: &str, attr: &str| -> Vec<String> {
            doc.descendants()
                .filter(|n| n.has_tag_name(tag))
                .map(|n| n.attribute(attr).unwrap().to_string())
                .collect()
        };
        let labels: Vec<_> = ours_info.iter().map(|p| p.label.clone()).collect();
        let dfs: Vec<_> = ours_info.iter().map(|p| p.df.to_string()).collect();
        assert_eq!(
            labels[..count],
            attrs(&oracle_doc, "Parameter", "label")[..count]
        );
        assert_eq!(dfs[..count], attrs(&oracle_doc, "PCell", "df")[..count]);
        let mut max_relative = 0.0_f64;
        for (a, b) in a.param_matrix.iter().zip(&b.param_matrix).take(count) {
            assert_eq!(a.parameter, b.parameter);
            // The old exact-linear oracle also has a near-zero intercept; only
            // its five main-effect betas have meaningful relative precision.
            if old && a.parameter == "P0000001" {
                assert!(a.beta.abs() < 1e-12 && b.beta.abs() < 1e-12);
                continue;
            }
            let tolerance = 1e-9 * b.beta.abs();
            assert!(
                (a.beta - b.beta).abs() <= tolerance,
                "{case} {}: ours {}, oracle {}",
                a.parameter,
                a.beta,
                b.beta
            );
            if b.beta != 0.0 {
                max_relative = max_relative.max(((a.beta - b.beta) / b.beta).abs());
            }
        }
        let names: Vec<_> = ours
            .data_dictionary
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        let scorer = ours.bind(&names).unwrap();
        let mut scored = 0;
        let data_dir = support::repo_root().join(format!("oracle/{label}/{case}/data"));
        for path in std::fs::read_dir(data_dir)
            .unwrap()
            .map(|p| p.unwrap().path())
        {
            if path.extension().is_none_or(|e| e != "csv") {
                continue;
            }
            let text = std::fs::read_to_string(path).unwrap();
            let mut lines = text.lines();
            let header = support::parse_csv_line(lines.next().unwrap());
            let Some(predicted) = header.iter().position(|n| n == "$L-y") else {
                continue;
            };
            for line in lines.filter(|s| !s.is_empty()) {
                let cells = support::parse_csv_line(line);
                let row: Vec<_> = names
                    .iter()
                    .map(|name| {
                        let p = header.iter().position(|n| n == name).unwrap();
                        cells[p]
                            .parse::<f64>()
                            .map(dms_pmml::Value::Number)
                            .unwrap_or_else(|_| dms_pmml::Value::Text(cells[p].clone()))
                    })
                    .collect();
                let prediction = scorer.score(&row).unwrap();
                let dms_pmml::Value::Number(actual) = prediction.value else {
                    panic!("missing prediction");
                };
                let expected: f64 = cells[predicted].parse().unwrap();
                assert!(
                    (actual - expected).abs() <= 1e-9 * expected.abs().max(1.0),
                    "{case} row {scored}: {actual} vs {expected}"
                );
                scored += 1;
            }
        }
        assert!(scored > 0, "{case}: no oracle scored records");
        println!(
            "TRAINED {label}/{case}: {count} parameters, max relative beta error {max_relative:e}, {scored} scored records"
        );
    }
}

#[test]
fn unverified_linear_as_settings_return_named_gaps() {
    use dms_strfmt::fastinfoset::Node as XmlNode;
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(support::repo_root().join("corpus/linearas/lin_factor.str")).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let builder = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "spss.linearEngine!linearEngine")
        .unwrap();
    for (key, value) in [
        ("_varSelectionMethod", "None"),
        ("_criteriaForForwardStepwise", "AICC"),
        ("_detectTwoWayInteraction", "false"),
        ("_intercept", "false"),
        ("_useCustomMaxEffects", "true"),
        ("_useCustomMaxSteps", "true"),
        ("_maxEffects", "999999999999999999999"),
        ("_maxSteps", "NaN"),
        ("_Pin", "0.1"),
        ("_Pout", "0.2"),
        ("_useWeight", "true"),
        ("_useFrequency", "true"),
        ("_regressionWeightField", "x1"),
        ("_frequencyWeightField", "x1"),
        ("autoModelFieldSelection", "true"),
        ("useEvents", "true"),
        ("weightField", "x1"),
        ("useFrequency", "true"),
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
        property.children = vec![XmlNode::Text(value.into())];
        let error = match dms_exec::nodes::build::build(&node) {
            Err(e) => format!("{e:?}"),
            Ok(_) => panic!("accepted {key}={value}"),
        };
        assert!(error.contains("GAP:") && error.contains(key), "{error}");
    }
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
