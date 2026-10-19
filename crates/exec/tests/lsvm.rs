//! LSVM training at **statistical parity** (`docs/statistical-parity.md`, acceptance rule 3).
//!
//! Modeler solves the documented objective with an ADMM that stops before the optimum by an
//! undocumented rule, so its coefficients are not reproducible (`models_as/lsvm_numeric`:
//! intercept -0.0155 against the optimum's -0.0359). We solve the same objective to
//! convergence, so rule 3 accepts our result when
//!
//! - our objective is no worse than the value of Modeler's own coefficients on the same design
//!   (within a relative 1e-9),
//! - the predicted class of every scored oracle record is identical, and
//! - continuous predictions differ from Modeler's by at most 1 % of the target's standard
//!   deviation.
//!
//! Coefficients are reported (printed), never compared.

#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;

use dms_algo::Dataset;
use dms_algo::lsvm::{self, LsvmSettings, Penalty};
use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_pmml::{Model, Pmml, Value};
use dms_stream::{NodeRef, Stream, StreamNode};
use dms_strfmt::ArchiveLimits;
use dms_types::{Role, Table};
use std::sync::Arc;

/// One Linear SVM oracle: the corpus stream that trains it and the field Modeler's apply node
/// wrote.
struct Oracle {
    case: &'static str,
    /// `$L-respond` for a flag target, `$L-y` for a continuous one.
    predicted: &'static str,
}

const ORACLES: [Oracle; 3] = [
    Oracle {
        case: "lsvm_default",
        predicted: "$L-respond",
    },
    Oracle {
        case: "lsvm_l1",
        predicted: "$L-respond",
    },
    Oracle {
        case: "lsvm_numeric",
        predicted: "$L-y",
    },
];

/// The corpus stream of one oracle, read and decoded.
fn stream_of(case: &str) -> (Stream, std::collections::HashMap<String, Arc<[u8]>>) {
    let root = support::repo_root();
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(root.join(format!("corpus/models_as/{case}.str"))).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let attachments = archive
        .attachments
        .into_iter()
        .map(|a| (a.name, Arc::from(a.bytes)))
        .collect();
    (stream, attachments)
}

fn builder_of(stream: &Stream) -> &StreamNode {
    stream
        .diagram
        .nodes
        .iter()
        .find(|node| node.type_identifier() == "spss.linearSVM!linearSVM")
        .unwrap()
}

/// Trains through the builder and returns the model, the training records and the settings the
/// stream saved.
fn train(case: &str) -> (Pmml, Arc<Table>, LsvmSettings) {
    let root = support::repo_root();
    let (stream, attachments) = stream_of(case);
    let builder = builder_of(&stream);
    // The training records are what reaches the build node.
    let source = stream
        .diagram
        .links
        .iter()
        .find(|link| link.to == NodeRef::Uid(builder.uid.clone()))
        .and_then(|link| match &link.from {
            NodeRef::Uid(uid) => stream.diagram.nodes.iter().find(|node| &node.uid == uid),
            NodeRef::Unresolved(_) => None,
        })
        .unwrap();
    let context = ExecContext {
        files: FileResolver {
            data_dirs: vec![root.join("corpus/models_as")],
        },
        attachments: Arc::new(attachments),
        stream_properties: stream.properties.clone().map(Arc::new),
        ..ExecContext::default()
    };
    let results =
        Plan::new(&stream).evaluate(&[builder.uid.as_str(), source.uid.as_str()], &context);
    let trained = match results.get(&builder.uid) {
        Some(Ok(NodeOutput::Model(model))) => (**model).clone(),
        other => panic!("{case}: builder failed: {other:?}"),
    };
    let Some(Ok(NodeOutput::Data(table))) = results.get(&source.uid) else {
        panic!("{case}: the training records failed");
    };
    let penalty = match dms_exec::settings::text(builder.settings.as_ref(), "_penaltyFunction")
        .as_deref()
        .map(str::trim)
    {
        None | Some("" | "L2") => Penalty::L2,
        Some("L1") => Penalty::L1,
        Some(other) => panic!("_penaltyFunction={other:?}"),
    };
    (
        trained,
        Arc::clone(table),
        LsvmSettings {
            penalty,
            ..LsvmSettings::default()
        },
    )
}

/// The target and the Type-role inputs a model uses, as `nodes::build` resolves them.
fn roles(table: &Table) -> (usize, Vec<usize>) {
    let fields = table.model().fields();
    let target = (0..fields.len())
        .find(|&i| fields[i].role == Role::Target)
        .unwrap();
    let inputs = (0..fields.len())
        .filter(|&i| matches!(fields[i].role, Role::Input | Role::Both))
        .collect();
    (target, inputs)
}

/// Modeler's saved model for one oracle.
fn oracle_pmml(case: &str) -> Pmml {
    let path =
        support::repo_root().join(format!("oracle/models_as/{case}/models/01_models_as.xml"));
    Pmml::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Every scored record of an oracle: `(file, rows, header)` of the apply nodes' exports.
fn scored_records(case: &str) -> Vec<(String, Vec<Vec<String>>, Vec<String>)> {
    let dir = support::repo_root().join(format!("oracle/models_as/{case}/data"));
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "csv"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(&path).unwrap();
        let mut lines = text.lines().filter(|line| !line.is_empty());
        let header = support::parse_csv_line(lines.next().unwrap());
        if !header.iter().any(|name| name.starts_with("$L-")) {
            continue;
        }
        out.push((
            path.file_name().unwrap().to_string_lossy().into_owned(),
            lines.map(support::parse_csv_line).collect(),
            header,
        ));
    }
    out
}

#[test]
fn lsvm_training_reaches_the_optimum_and_keeps_modelers_classes() {
    for oracle in &ORACLES {
        let (ours, table, settings) = train(oracle.case);
        let (target, inputs) = roles(&table);
        let dataset = Dataset {
            table: &table,
            target,
            inputs: &inputs,
            frequency: None,
        };
        let modeler = oracle_pmml(oracle.case);

        // Rule 3: our objective is no worse than Modeler's coefficients on the same design.
        let mine = lsvm::evaluate_objective(&dataset, &settings, &ours).unwrap();
        let theirs = lsvm::evaluate_objective(&dataset, &settings, &modeler).unwrap();
        assert!(
            mine <= theirs + 1e-9 * theirs.abs(),
            "{}: our objective {mine} is worse than Modeler's {theirs}",
            oracle.case
        );
        // The builder reproduces the direct solve exactly.
        let solution = lsvm::solve(&dataset, &settings).unwrap();
        assert_eq!(solution.pmml, ours, "{}", oracle.case);
        assert_eq!(solution.objective, mine, "{}", oracle.case);

        // Coefficients are reported, not compared (Modeler's ADMM stops early).
        let mine_coefficients = lsvm::coefficients(&ours).unwrap();
        let their_coefficients = lsvm::coefficients(&modeler).unwrap();
        assert_eq!(
            mine_coefficients.len(),
            their_coefficients.len(),
            "{}",
            oracle.case
        );
        let mut worst = (String::new(), 0.0_f64);
        for ((label, a), (their_label, b)) in mine_coefficients.iter().zip(&their_coefficients) {
            assert_eq!(label, their_label, "{}", oracle.case);
            if (a - b).abs() > worst.1 {
                worst = (label.clone(), (a - b).abs());
            }
        }
        println!(
            "{}: our objective {mine:.9} in {} iterations, Modeler's {theirs:.9} on the same \
             design; {} coefficients, the largest gap is {} = {:.6}",
            oracle.case,
            solution.iterations,
            mine_coefficients.len(),
            worst.0,
            worst.1
        );
        for ((label, a), (_, b)) in mine_coefficients.iter().zip(&their_coefficients) {
            println!(
                "    {label:<18} ours {a:>12.9}  Modeler {b:>12.9}  gap {:>12.9}",
                a - b
            );
        }

        // Rule 3: identical predicted classes, and continuous predictions within 1 % of the
        // target's standard deviation.
        let names: Vec<&str> = ours
            .data_dictionary
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        let scorer = ours.bind(&names).unwrap();
        let target_field = table.model().fields().get(target).unwrap();
        let continuous = target_field.measurement == dms_types::Measurement::Continuous;
        // Rule 3 allows a continuous prediction to differ by 1 % of the target's standard
        // deviation; a flag target has none, so only the classes are compared.
        let values: Vec<f64> = if continuous {
            table
                .rows()
                .iter()
                .filter_map(|row| row.get(target))
                .filter(|value| !target_field.is_missing(value))
                .filter_map(|value| value.as_f64())
                .collect()
        } else {
            Vec::new()
        };
        let mean = values.iter().sum::<f64>() / values.len().max(1) as f64;
        let deviation = (values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>()
            / values.len().max(1) as f64)
            .sqrt();
        let tolerance = 0.01 * deviation;
        let mut records = 0_usize;
        let mut worst_prediction = 0.0_f64;
        for (file, rows, header) in scored_records(oracle.case) {
            let at = header.iter().position(|name| name == oracle.predicted);
            println!("    {file}: {} scored records", rows.len());
            for row in &rows {
                let scored: Vec<Value> = names
                    .iter()
                    .map(|name| {
                        let column = header.iter().position(|n| n == name).unwrap();
                        row[column]
                            .parse::<f64>()
                            .map(Value::Number)
                            .unwrap_or_else(|_| Value::Text(row[column].clone()))
                    })
                    .collect();
                let prediction = scorer.score(&scored).unwrap();
                match (&prediction.value, at) {
                    (Value::Text(ours), Some(at)) => {
                        assert_eq!(
                            ours, &row[at],
                            "{}: record {records} predicted {ours}, Modeler {}",
                            oracle.case, row[at]
                        );
                    }
                    (Value::Number(ours), Some(at)) => {
                        let expected: f64 = row[at].parse().unwrap();
                        worst_prediction = worst_prediction.max((ours - expected).abs());
                        assert!(
                            (ours - expected).abs() <= tolerance,
                            "{}: record {records} predicted {ours}, Modeler {expected} \
                             (tolerance {tolerance})",
                            oracle.case
                        );
                    }
                    _ => panic!("{}: no prediction for record {records}", oracle.case),
                }
                records += 1;
            }
        }
        assert!(records > 0, "{}: no scored oracle records", oracle.case);
        if continuous {
            println!(
                "{}: {records} scored records, the largest prediction deviation is \
                 {worst_prediction:.6} (tolerance {tolerance:.6})",
                oracle.case
            );
        } else {
            println!(
                "{}: {records} scored records with identical predicted classes",
                oracle.case
            );
        }
    }
}

#[test]
fn lsvm_pmml_has_the_shape_modeler_saves() {
    for oracle in &ORACLES {
        let (document, _, _) = train(oracle.case);
        let modeler_document = oracle_pmml(oracle.case);
        let (Model::GeneralRegression(ours), Model::GeneralRegression(modeler)) =
            (&document.model, &modeler_document.model)
        else {
            panic!("expected a GeneralRegressionModel");
        };
        assert_eq!(ours.model_type, modeler.model_type, "{}", oracle.case);
        assert_eq!(
            ours.header.function, modeler.header.function,
            "{}",
            oracle.case
        );
        assert_eq!(
            ours.header.algorithm, modeler.header.algorithm,
            "{}",
            oracle.case
        );
        assert_eq!(ours.parameters, modeler.parameters, "{}", oracle.case);
        assert_eq!(ours.factors, modeler.factors, "{}", oracle.case);
        assert_eq!(ours.covariates, modeler.covariates, "{}", oracle.case);
        assert_eq!(ours.pp_matrix, modeler.pp_matrix, "{}", oracle.case);
        assert_eq!(
            ours.target_reference_category, modeler.target_reference_category,
            "{}",
            oracle.case
        );
        let ours_targets: Vec<Option<&String>> = ours
            .param_matrix
            .iter()
            .map(|cell| cell.target_category.as_ref())
            .collect();
        let their_targets: Vec<Option<&String>> = modeler
            .param_matrix
            .iter()
            .map(|cell| cell.target_category.as_ref())
            .collect();
        assert_eq!(ours_targets, their_targets, "{}", oracle.case);
        assert_eq!(
            ours.header.mining_schema, modeler.header.mining_schema,
            "{}",
            oracle.case
        );
        assert_eq!(
            document.data_dictionary, modeler_document.data_dictionary,
            "{}",
            oracle.case
        );
    }
}

#[test]
fn unverified_lsvm_settings_return_gap() {
    use dms_strfmt::fastinfoset::Node as XmlNode;
    for case in ["lsvm_default", "lsvm_l1"] {
        let (stream, _) = stream_of(case);
        let builder = builder_of(&stream);
        for (key, value) in [
            ("_lambda", "0.5"),
            ("_epsilon", "0.05"),
            ("_intercept", "false"),
            ("_targetOrder", "DESCENDING"),
            ("_penaltyFunction", "L0"),
            ("autoModelFieldSelection", "false"),
            ("useWeight", "true"),
            ("useFrequency", "true"),
            ("_calculatePI", "false"),
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
            let error = match dms_exec::nodes::build::build(&node) {
                Err(e) => format!("{e:?}"),
                Ok(_) => panic!("accepted {key}={value}"),
            };
            assert!(error.contains("GAP:"), "{case} {key}={value}: {error}");
        }
    }
}

/// The settings the three oracle streams actually save, which `analytic_build::lsvm` accepts.
/// A new oracle that saves anything else must fail with a GAP, not train on guessed values.
#[test]
fn the_oracle_streams_save_the_documented_lsvm_defaults() {
    for (case, penalty) in [
        ("lsvm_default", "L2"),
        ("lsvm_l1", "L1"),
        ("lsvm_numeric", "L2"),
    ] {
        let (stream, _) = stream_of(case);
        let builder = builder_of(&stream);
        let saved = |key: &str| {
            dms_exec::settings::text(builder.settings.as_ref(), key)
                .unwrap_or_else(|| panic!("{case}: no {key}"))
        };
        assert_eq!(saved("_lambda"), "0.1", "{case}");
        assert_eq!(saved("_epsilon"), "0.1", "{case}");
        assert_eq!(saved("_intercept"), "true", "{case}");
        assert_eq!(saved("_penaltyFunction"), penalty, "{case}");
        assert_eq!(saved("_targetOrder"), "ASCENDING", "{case}");
        assert_eq!(saved("_excludeMissingValues"), "false", "{case}");
        assert_eq!(saved("_inputs"), "", "{case}");
        assert_eq!(saved("_target"), "", "{case}");
        assert_eq!(saved("autoModelFieldSelection"), "true", "{case}");
        assert_eq!(saved("expertMode"), "false", "{case}");
        assert_eq!(saved("useWeight"), "false", "{case}");
        assert_eq!(saved("useFrequency"), "false", "{case}");
        assert_eq!(saved("weightField"), "", "{case}");
        assert_eq!(saved("frequencyField"), "", "{case}");
        assert_eq!(saved("partitionField"), "", "{case}");
        assert_eq!(saved("eventFields"), "", "{case}");
        assert_eq!(saved("modelSplitFieldSelection"), "", "{case}");
        // The builder accepts exactly these.
        assert!(dms_exec::nodes::build::build(builder).is_ok(), "{case}");
    }
}

/// The nugget's saved Modeler PMML still scores exactly (exact tier): with the builder removed
/// from the stream, every apply node's records equal the oracle export to 1e-9.
#[test]
fn saved_lsvm_nuggets_score_exactly() {
    let root = support::repo_root();
    for oracle in &ORACLES {
        let (mut stream, attachments) = stream_of(oracle.case);
        let builder = builder_of(&stream).uid.clone();
        stream.diagram.nodes.retain(|node| node.uid != builder);
        stream.diagram.links.retain(|link| {
            link.from != NodeRef::Uid(builder.clone()) && link.to != NodeRef::Uid(builder.clone())
        });
        let appliers: Vec<String> = stream
            .diagram
            .nodes
            .iter()
            .filter(|node| node.type_identifier() == "spss.linearSVM!applyLinearSVM")
            .map(|node| node.uid.clone())
            .collect();
        assert!(!appliers.is_empty(), "{}", oracle.case);
        let context = ExecContext {
            files: FileResolver {
                data_dirs: vec![root.join("corpus/models_as")],
            },
            attachments: Arc::new(attachments),
            stream_properties: stream.properties.clone().map(Arc::new),
            ..ExecContext::default()
        };
        let ids: Vec<&str> = appliers.iter().map(String::as_str).collect();
        let results = Plan::new(&stream).evaluate(&ids, &context);
        let dir = root.join(format!("oracle/models_as/{}/data", oracle.case));
        for uid in &appliers {
            let Some(Ok(NodeOutput::Data(table))) = results.get(uid) else {
                panic!("{} {uid}: the saved nugget did not score", oracle.case);
            };
            let path = std::fs::read_dir(&dir)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| path.to_string_lossy().ends_with(&format!("_{uid}.csv")))
                .unwrap();
            let text = std::fs::read_to_string(path).unwrap();
            let mut lines = text.lines().filter(|line| !line.is_empty());
            let header = support::parse_csv_line(lines.next().unwrap());
            let rows: Vec<_> = lines.map(support::parse_csv_line).collect();
            assert_eq!(table.rows().len(), rows.len(), "{} {uid}", oracle.case);
            for (column, name) in header.iter().enumerate() {
                if !name.starts_with("$L") {
                    continue;
                }
                let at = table.model().position(name).unwrap();
                for (row, expected) in table.rows().iter().zip(&rows) {
                    let actual = &row[at];
                    match expected[column].parse::<f64>() {
                        Ok(b) => {
                            let a = actual.as_f64().unwrap();
                            assert!(
                                (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0),
                                "{} {uid} {name}: {a} vs {b}",
                                oracle.case
                            );
                        }
                        Err(_) => assert_eq!(
                            actual.to_string(),
                            expected[column].trim_matches('"'),
                            "{} {uid} {name}",
                            oracle.case
                        ),
                    }
                }
            }
        }
    }
}
