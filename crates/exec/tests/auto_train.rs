//! Auto Classifier comparison measures and ranking (`auto_train`, Modeler 19).
//!
//! `ac_lift` keeps all four components (C5.0, Logistic, Decision List, CHAID), so its saved
//! nugget holds Modeler's own component models. Scoring the training records with them checks
//! our measures against the `trainPartitionStats` Modeler saved, and the ranking rule against
//! the order of the other `ac_*` cases, independently of our component learners.

#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;

use dms_exec::nodes::auto_build::{Comparison, saved_comparisons};
use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_stream::{Stream, StreamNode};
use dms_strfmt::ArchiveLimits;
use std::sync::Arc;

fn load(case: &str) -> (Stream, ExecContext) {
    load_from("auto_train", case)
}

fn load_from(label: &str, case: &str) -> (Stream, ExecContext) {
    let root = support::repo_root();
    let bytes = std::fs::read(root.join(format!("corpus/{label}/{case}.str"))).unwrap();
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
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
        decimal_places: Some(15),
        ..ExecContext::default()
    };
    (stream, context)
}

/// A saved component's model name, accuracy and AUC (`trainPartitionStats`).
type SavedStats = (String, f64, f64);

/// The nugget scoring the training records and its saved stats, in Modeler's rank order.
fn saved(case: &str) -> (Vec<(String, Comparison)>, Vec<SavedStats>) {
    let (stream, context) = load(case);
    let plan = Plan::new(&stream);
    // The nugget that scores the training records (360 rows); the other scores the grid.
    let (nugget, input): (&StreamNode, _) = stream
        .diagram
        .nodes
        .iter()
        .filter(|n| n.type_identifier() == "ApplyAutoClassifierNode")
        .find_map(|n| {
            let upstream = plan.inputs_of(&n.uid)[0];
            let results = plan.evaluate(&[upstream], &context);
            match results.get(upstream) {
                Some(Ok(NodeOutput::Data(table))) if table.row_count() == 360 => {
                    Some((n, Arc::clone(table)))
                }
                _ => None,
            }
        })
        .unwrap();
    let measured = saved_comparisons(nugget, &input, &context, 30, 10.0, 5.0).unwrap();
    let stats = nugget
        .result_items
        .iter()
        .filter(|item| item.attribute("useModel") == Some("true"))
        .map(|item| {
            let stats = item.child("trainPartitionStats").unwrap();
            (
                item.attribute("modelName").unwrap().to_string(),
                stats.attribute("accuracy").unwrap().parse().unwrap(),
                stats.attribute("auc").unwrap().parse().unwrap(),
            )
        })
        .collect();
    (measured, stats)
}

fn builders(case: &str) -> Vec<String> {
    saved(case)
        .0
        .into_iter()
        .map(|(builder, _)| builder)
        .collect()
}

/// The two best of `ac_lift`'s components by `key`, descending, ties in saved order.
fn top2(all: &[(String, Comparison)], key: fn(&Comparison) -> f64) -> Vec<String> {
    let mut ranked: Vec<_> = all.iter().collect();
    ranked.sort_by(|a, b| key(&b.1).total_cmp(&key(&a.1)));
    ranked.iter().take(2).map(|(b, _)| b.clone()).collect()
}

#[test]
fn measures_and_ranking_match_modelers_saved_components() {
    let (all, stats) = saved("ac_lift");
    assert_eq!(all.len(), 4);
    for ((builder, measured), (name, accuracy, auc)) in all.iter().zip(&stats) {
        println!(
            "{builder} ({name}): accuracy {:.7} (Modeler {accuracy}), AUC {:.7} (Modeler {auc}), lift {:.6}, profit {}",
            measured.accuracy.unwrap(),
            measured.auc.unwrap(),
            measured.lift.unwrap(),
            measured.profit.unwrap()
        );
        // Modeler saves seven decimals.
        assert!(
            (measured.accuracy.unwrap() - accuracy).abs() < 5e-8,
            "{builder} accuracy"
        );
        assert!((measured.auc.unwrap() - auc).abs() < 5e-8, "{builder} AUC");
    }
    // ac_lift lists the components by descending lift.
    let lifts: Vec<f64> = all.iter().map(|(_, c)| c.lift.unwrap()).collect();
    assert!(
        lifts.windows(2).all(|w| w[0] > w[1]),
        "lift order {lifts:?}"
    );
    // The other rankings, on the same training records and the same component models.
    assert_eq!(
        top2(&all, |c| c.accuracy.unwrap()),
        builders("ac_accuracy_n2")
    );
    assert_eq!(top2(&all, |c| c.auc.unwrap()), builders("ac_auc_n2"));
    assert_eq!(top2(&all, |c| c.profit.unwrap()), builders("ac_profit_n2"));
    assert_eq!(top2(&all, |c| c.lift.unwrap()), builders("ac_lift_n2"));
}

/// Modeler's kept components of an `auto_lift` case, in rank order, by builder.
fn lift_ranking(case: &str) -> Vec<String> {
    let (stream, _) = load_from("auto_lift", case);
    let nugget = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "ApplyAutoClassifierNode")
        .unwrap();
    nugget
        .result_items
        .iter()
        .map(|item| {
            item.child("modelOutput")
                .and_then(|output| output.child("modelBuildInfo"))
                .and_then(|info| info.attribute("modelBuilderId"))
                .unwrap()
                .to_string()
        })
        .collect()
}

/// Lift at a fractional cut (`auto_lift`: q % of 360 records, 172.8 .. 288): the interpolated
/// lift ranks Modeler's own `ac_lift` components as Modeler does, except the near tie at q = 57
/// (`dms_algo::model_comparison`). Ties keep the enabled order.
#[test]
fn fractional_lift_cut_interpolates() {
    let (stream, context) = load("ac_lift");
    let plan = Plan::new(&stream);
    let (nugget, input) = stream
        .diagram
        .nodes
        .iter()
        .filter(|n| n.type_identifier() == "ApplyAutoClassifierNode")
        .find_map(|n| {
            let upstream = plan.inputs_of(&n.uid)[0];
            match plan.evaluate(&[upstream], &context).get(upstream) {
                Some(Ok(NodeOutput::Data(table))) if table.row_count() == 360 => {
                    Some((n, Arc::clone(table)))
                }
                _ => None,
            }
        })
        .unwrap();
    let enabled = [
        "BuildC5RuleNode",
        "BuildLogisticNode",
        "BuildDecisionListNode",
        "BuildChaidRuleNode",
    ];
    for q in [48u32, 52, 56, 57, 58, 61, 63, 78, 80] {
        let measured = saved_comparisons(nugget, &input, &context, q, 10.0, 5.0).unwrap();
        let lift = |builder: &str| {
            measured
                .iter()
                .find(|(b, _)| b == builder)
                .and_then(|(_, c)| c.lift)
                .unwrap()
        };
        let mut ours: Vec<&str> = enabled.to_vec();
        ours.sort_by(|a, b| lift(b).total_cmp(&lift(a)));
        let modeler = lift_ranking(&format!("ac_lift_q{q}"));
        println!("q{q}: ours {ours:?}, Modeler {modeler:?}");
        if q == 57 {
            // Near tie (1.622011 vs 1.622088): Modeler keeps C5.0 first, we put Logistic first.
            assert_eq!(&modeler[..2], ["BuildC5RuleNode", "BuildLogisticNode"]);
            assert_eq!(&ours[..2], ["BuildLogisticNode", "BuildC5RuleNode"]);
        } else {
            assert_eq!(ours, modeler, "q = {q}");
        }
    }
}
