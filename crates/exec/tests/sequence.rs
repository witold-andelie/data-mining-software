//! Sequence (`BuildSequenceNode`): statistical parity (tie order free) with the eight oracle
//! cases in `oracle/train_next/sequence_*` and `oracle/models_next/sequence_*`.
//!
//! (a) Every mined rule and its `confidence`, `support` (antecedent share) and `occurrence`
//! matches at 15 significant digits, the precision the oracle PMML stores. Rule order may
//! differ only inside a tie group on (confidence, antecedent length, antecedent support): the
//! only key Modeler does not expose.
//!
//! (b) Every scored row of every oracle table matches: original fields, prediction confidences
//! and null positions. Prediction items may differ only inside an equal-confidence group, and
//! only when both items are consequents of rules with that confidence.
//!
//! The tie-group counts are printed so the freedom used stays visible.

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

mod support;

use dms_exec::NodeOutput;
use dms_pmml::{Model, Pmml, SequenceModel, SequenceRule};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use dms_types::{Storage, Table, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// `(label, case)` of every Sequence oracle.
const CASES: [(&str, &str); 8] = [
    ("train_next", "sequence_default"),
    ("train_next", "sequence_conf75"),
    ("train_next", "sequence_same_time"),
    ("train_next", "sequence_size3"),
    ("train_next", "sequence_support10"),
    ("train_next", "sequence_support20_size2"),
    ("models_next", "sequence_default"),
    ("models_next", "sequence_minsupp"),
];

/// The 15 significant digits the oracle PMML and CSV exports keep. Modeler rounds through
/// decimal text, so binary values that differ below that precision are the same number to it.
fn sig(value: f64) -> String {
    format!("{value:.14e}")
}

fn itemset_text(set: &[(String, String)]) -> String {
    set.iter()
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>()
        .join("&")
}

fn sequence_text(sets: &[Vec<(String, String)>]) -> String {
    sets.iter()
        .map(|set| itemset_text(set))
        .collect::<Vec<_>>()
        .join(" > ")
}

fn rule_text(rule: &SequenceRule) -> String {
    format!(
        "{} => {} (confidence {}, support {}, occurrence {})",
        sequence_text(&rule.antecedent),
        sequence_text(&rule.consequent),
        sig(rule.confidence),
        sig(rule.support),
        rule.occurrence
    )
}

/// The order key Modeler is held to: confidence descending, then antecedent length. Within one
/// key the order is free: it changes when an unused Type value is added (`seq_ties`), so it is no
/// function of the rules and their statistics (statistical-parity rule 4).
fn tie_key(rule: &SequenceRule) -> (String, usize) {
    (sig(rule.confidence), rule.antecedent.len())
}

fn tie_groups(rules: &[SequenceRule]) -> (usize, usize) {
    let mut groups = 0;
    let mut tied = 0;
    let mut start = 0;
    while start < rules.len() {
        let mut end = start + 1;
        while end < rules.len() && tie_key(&rules[end]) == tie_key(&rules[start]) {
            end += 1;
        }
        if end - start > 1 {
            groups += 1;
            tied += end - start;
        }
        start = end;
    }
    (groups, tied)
}

fn load(label: &str, case: &str) -> (Stream, HashMap<String, Arc<[u8]>>) {
    let root = support::repo_root();
    let bytes = std::fs::read(root.join(format!("corpus/{label}/{case}.str"))).unwrap();
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let attachments: HashMap<String, Arc<[u8]>> = archive
        .attachments
        .into_iter()
        .map(|attachment| (attachment.name, attachment.bytes.into()))
        .collect();
    (stream, attachments)
}

fn context(
    label: &str,
    stream: &Stream,
    attachments: &HashMap<String, Arc<[u8]>>,
) -> dms_exec::ExecContext {
    let root = support::repo_root();
    dms_exec::ExecContext {
        files: dms_exec::FileResolver {
            data_dirs: vec![root.join(format!("corpus/{label}"))],
        },
        stream_properties: stream.properties.clone().map(Arc::new),
        attachments: Arc::new(attachments.clone()),
        decimal_places: Some(15),
        export_dir: None,
    }
}

fn builder_uid(stream: &Stream) -> String {
    stream
        .diagram
        .nodes
        .iter()
        .find(|node| node.type_identifier() == "BuildSequenceNode")
        .map(|node| node.uid.clone())
        .unwrap_or_else(|| panic!("the stream has no BuildSequenceNode"))
}

fn sequence(pmml: &Pmml) -> SequenceModel {
    match &pmml.model {
        Model::Sequence(model) => model.clone(),
        other => panic!("not a Sequence model: {other:?}"),
    }
}

fn trained(label: &str, case: &str) -> SequenceModel {
    let (stream, attachments) = load(label, case);
    let builder = builder_uid(&stream);
    let context = context(label, &stream, &attachments);
    let results = dms_exec::Plan::new(&stream).evaluate(&[builder.as_str()], &context);
    match results.get(builder.as_str()).unwrap() {
        Ok(NodeOutput::Model(model)) => sequence(model),
        other => panic!("{label}/{case}: {other:?}"),
    }
}

fn oracle(label: &str, case: &str) -> SequenceModel {
    let dir = support::repo_root().join(format!("oracle/{label}/{case}/models"));
    let xml = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "xml"))
        .unwrap();
    let text = std::fs::read_to_string(xml).unwrap();
    sequence(&Pmml::parse(&text).unwrap())
}

#[test]
fn rules_and_statistics_match_modeler() {
    let mut total_rules = 0;
    let mut total_groups = 0;
    let mut total_tied = 0;
    let mut failures = Vec::new();
    for (label, case) in CASES {
        let ours = trained(label, case);
        let theirs = oracle(label, case);
        if ours.number_of_transactions != theirs.number_of_transactions {
            failures.push(format!(
                "{label}/{case}: {} transactions vs {}",
                ours.number_of_transactions, theirs.number_of_transactions
            ));
        }
        if ours.id_field != theirs.id_field
            || ours.time_field != theirs.time_field
            || ours.content_fields != theirs.content_fields
        {
            failures.push(format!(
                "{label}/{case}: fields (id {:?}, time {:?}, content {:?}) vs (id {:?}, time \
                 {:?}, content {:?})",
                ours.id_field,
                ours.time_field,
                ours.content_fields,
                theirs.id_field,
                theirs.time_field,
                theirs.content_fields
            ));
        }
        if ours.rules.len() != theirs.rules.len() {
            failures.push(format!(
                "{label}/{case}: {} rules vs {}",
                ours.rules.len(),
                theirs.rules.len()
            ));
        }
        let mut matched = vec![false; theirs.rules.len()];
        let mut order_differences = 0;
        for (index, rule) in ours.rules.iter().enumerate() {
            let found = theirs
                .rules
                .iter()
                .enumerate()
                .position(|(other, candidate)| {
                    !matched[other]
                        && candidate.antecedent == rule.antecedent
                        && candidate.consequent == rule.consequent
                        && sig(candidate.confidence) == sig(rule.confidence)
                        && sig(candidate.support) == sig(rule.support)
                        && candidate.occurrence == rule.occurrence
                });
            match found {
                Some(other) => {
                    matched[other] = true;
                    if other != index {
                        order_differences += 1;
                        // Compare with Modeler's rule at this position, not with the same rule.
                        if tie_key(&theirs.rules[index]) != tie_key(rule) {
                            failures.push(format!(
                                "{label}/{case}: rule {} moved across a key group: {:?} vs {:?}",
                                index + 1,
                                tie_key(rule),
                                tie_key(&theirs.rules[index])
                            ));
                        }
                    }
                }
                None => failures.push(format!(
                    "{label}/{case}: extra rule {}: {}",
                    index + 1,
                    rule_text(rule)
                )),
            }
        }
        for (index, rule) in theirs.rules.iter().enumerate() {
            if !matched[index] {
                failures.push(format!(
                    "{label}/{case}: missing rule {}: {}",
                    index + 1,
                    rule_text(rule)
                ));
            }
        }
        let (groups, tied) = tie_groups(&theirs.rules);
        total_rules += theirs.rules.len();
        total_groups += groups;
        total_tied += tied;
        println!(
            "{label}/{case}: {} rules, {order_differences} order differences, {groups} tie groups \
             ({tied} rules), {} transactions",
            theirs.rules.len(),
            theirs.number_of_transactions
        );
    }
    println!(
        "rules total: {total_rules} rules, {total_groups} tie groups ({total_tied} tied rules)"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn cell(table: &Table, row: usize, column: usize) -> String {
    table
        .rows()
        .get(row)
        .and_then(|row| row.get(column))
        .filter(|value| !value.is_null())
        .map(Value::to_string)
        .unwrap_or_default()
}

fn real_cell(table: &Table, row: usize, column: usize) -> String {
    cell(table, row, column)
        .parse::<f64>()
        .map(|value| format!("{value:.15}"))
        .unwrap_or_else(|_| cell(table, row, column))
}

fn confidence_key(text: &str) -> Option<String> {
    if text == "$null$" || text.is_empty() {
        None
    } else {
        text.parse::<f64>().ok().map(sig)
    }
}

/// `(consequent item, confidence)` of every rule with a one-item consequent.
fn candidates(model: &SequenceModel) -> Vec<(String, String)> {
    model
        .rules
        .iter()
        .filter_map(|rule| {
            let [set] = rule.consequent.as_slice() else {
                return None;
            };
            let [(_, item)] = set.as_slice() else {
                return None;
            };
            Some((item.clone(), sig(rule.confidence)))
        })
        .collect()
}

#[test]
fn scored_records_match_modeler() {
    let root = support::repo_root();
    let mut total_rows = 0;
    let mut total_groups = 0;
    let mut total_tie_rows = 0;
    let mut failures = Vec::new();
    for (label, case) in CASES {
        let (stream, attachments) = load(label, case);
        let builder = builder_uid(&stream);
        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(format!("oracle/{label}/{case}/manifest.json")))
                .unwrap(),
        )
        .unwrap();
        let outputs = manifest["outputs"].as_array().unwrap();
        let targets: Vec<&str> = outputs
            .iter()
            .map(|output| output["id"].as_str().unwrap())
            .collect();
        let context = context(label, &stream, &attachments);
        let results = dms_exec::Plan::new(&stream).evaluate(&targets, &context);
        let model = match results.get(builder.as_str()).unwrap() {
            Ok(NodeOutput::Model(pmml)) => sequence(pmml),
            other => {
                failures.push(format!("{label}/{case}: builder failed: {other:?}"));
                continue;
            }
        };
        let candidates = candidates(&model);
        for output in outputs {
            let uid = output["id"].as_str().unwrap();
            let name = output["label"].as_str().unwrap();
            let file = support::local_path(output["file"].as_str().unwrap());
            let expected_text = std::fs::read_to_string(&file).unwrap();
            let document = support::parse_csv_document(&expected_text);
            let expected = &document.tables[0];
            let table = match results.get(uid).unwrap() {
                Ok(NodeOutput::Data(table)) => table,
                other => {
                    failures.push(format!("{label}/{case} {name}: {other:?}"));
                    continue;
                }
            };
            let header: Vec<String> = table
                .model()
                .fields()
                .iter()
                .map(|field| field.name.clone())
                .collect();
            if header != expected.header {
                failures.push(format!(
                    "{label}/{case} {name}: header {header:?} vs {:?}",
                    expected.header
                ));
                continue;
            }
            let prediction: Vec<(usize, usize)> = expected
                .header
                .iter()
                .enumerate()
                .filter(|(_, field)| field.starts_with("$S-"))
                .filter_map(|(item, field)| {
                    let confidence = expected
                        .header
                        .iter()
                        .position(|name| *name == field.replacen("$S-", "$SC-", 1))?;
                    Some((item, confidence))
                })
                .collect();
            let mut groups = 0;
            let mut tie_rows = 0;
            for (row_index, expected_row) in expected.rows.iter().enumerate() {
                let Some(actual_row) = table.rows().get(row_index) else {
                    failures.push(format!(
                        "{label}/{case} {name}: missing row {}",
                        row_index + 1
                    ));
                    continue;
                };
                let _ = actual_row;
                for (column, field) in expected.header.iter().enumerate() {
                    if prediction.iter().any(|(item, _)| *item == column) {
                        continue;
                    }
                    let theirs = expected_row.get(column).cloned().unwrap_or_default();
                    let ours = cell(table, row_index, column);
                    let storage = table
                        .model()
                        .fields()
                        .get(column)
                        .map(|field| field.storage);
                    let equal = if theirs == "$null$" {
                        ours.is_empty()
                    } else if storage == Some(Storage::Real) {
                        real_cell(table, row_index, column) == theirs
                    } else {
                        ours == theirs
                    };
                    if !equal {
                        failures.push(format!(
                            "{label}/{case} {name}: row {} column {field}: {ours:?} vs {:?}",
                            row_index + 1,
                            theirs
                        ));
                    }
                }
                let mut start = 0;
                while start < prediction.len() {
                    let their_confidence = &expected_row[prediction[start].1];
                    let key = confidence_key(their_confidence);
                    let mut end = start + 1;
                    while end < prediction.len()
                        && confidence_key(&expected_row[prediction[end].1]) == key
                    {
                        end += 1;
                    }
                    if end - start > 1 && key.is_some() {
                        groups += 1;
                        tie_rows += 1;
                    }
                    let mut ours: Vec<String> = prediction[start..end]
                        .iter()
                        .map(|(item, _)| cell(table, row_index, *item))
                        .collect();
                    let mut theirs: Vec<String> = prediction[start..end]
                        .iter()
                        .map(|(item, _)| {
                            let value = expected_row.get(*item).cloned().unwrap_or_default();
                            if value == "$null$" {
                                String::new()
                            } else {
                                value
                            }
                        })
                        .collect();
                    let ours_empty = ours.iter().filter(|item| item.is_empty()).count();
                    let theirs_empty = theirs.iter().filter(|item| item.is_empty()).count();
                    ours.sort();
                    theirs.sort();
                    if ours != theirs {
                        let valid = |item: &str| {
                            key.as_ref().is_some_and(|confidence| {
                                candidates
                                    .iter()
                                    .any(|(candidate, at)| candidate == item && at == confidence)
                            })
                        };
                        let allowed = ours_empty == theirs_empty
                            && ours
                                .iter()
                                .filter(|item| !theirs.contains(item))
                                .all(|item| valid(item))
                            && theirs
                                .iter()
                                .filter(|item| !ours.contains(item))
                                .all(|item| valid(item));
                        if !allowed {
                            failures.push(format!(
                                "{label}/{case} {name}: row {} predictions {:?} vs {:?} \
                                 (confidence {:?})",
                                row_index + 1,
                                ours,
                                theirs,
                                their_confidence
                            ));
                        }
                    }
                    start = end;
                }
            }
            if table.row_count() != expected.rows.len() {
                failures.push(format!(
                    "{label}/{case} {name}: {} rows vs {}",
                    table.row_count(),
                    expected.rows.len()
                ));
            }
            total_rows += expected.rows.len();
            total_groups += groups;
            total_tie_rows += tie_rows;
            println!(
                "{label}/{case} {name}: {} rows, {groups} equal-confidence groups in {tie_rows} \
                 rows",
                expected.rows.len()
            );
        }
    }
    println!(
        "scores total: {total_rows} rows, {total_groups} equal-confidence groups in \
         {total_tie_rows} rows"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
