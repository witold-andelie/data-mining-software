//! Auto Numeric comparison measures and ranking (`auto_train/an_*`, Modeler 19).
//!
//! `an_corr` keeps all four components (Regression, Linear-AS, Linear, CHAID), so its saved
//! nugget holds Modeler's own component models. Scoring the training records with them checks
//! our correlation and relative error against the `trainPartitionStats` Modeler saved, and the
//! ranking rule against the order of the other `an_*` cases, independently of our component
//! learners. Ranking starts from the Build node's enabled order (stable sort); `numvar`
//! ascends for a continuous target (unlike Auto Classifier's descending).

#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]

mod support;

use dms_exec::nodes::auto_build::{Comparison, saved_comparisons};
use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_stream::{Stream, StreamNode};
use dms_strfmt::ArchiveLimits;
use std::collections::HashMap;
use std::sync::Arc;

fn load(case: &str) -> (Stream, ExecContext) {
    let root = support::repo_root();
    let bytes = std::fs::read(root.join(format!("corpus/auto_train/{case}.str"))).unwrap();
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default()).unwrap();
    let stream = Stream::from_document(&archive.document).unwrap();
    let context = ExecContext {
        files: FileResolver {
            data_dirs: vec![root.join("corpus/auto_train")],
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

/// The nugget scoring the training records (240 rows) and its input.
fn training_nugget<'a>(
    stream: &'a Stream,
    context: &ExecContext,
) -> (&'a StreamNode, Arc<dms_types::Table>) {
    let plan = Plan::new(stream);
    stream
        .diagram
        .nodes
        .iter()
        .filter(|n| n.type_identifier() == "ApplyNumericPredictorNode")
        .find_map(|n| {
            let upstream = plan.inputs_of(&n.uid)[0];
            let results = plan.evaluate(&[upstream], context);
            match results.get(upstream) {
                Some(Ok(NodeOutput::Data(table))) if table.row_count() == 240 => {
                    Some((n, Arc::clone(table)))
                }
                _ => None,
            }
        })
        .unwrap()
}

/// Enabled component builders in saved order, from the Build node.
fn enabled_order(stream: &Stream) -> Vec<String> {
    let build = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "BuildNumericClassifierNode")
        .unwrap();
    let s = build.settings.as_ref().unwrap();
    s.child("ensembleAlgorithms")
        .map(|e| {
            e.child_elements()
                .filter(|m| {
                    m.name.local.as_ref() == "persistentObjectListMember"
                        && m.attribute("algorithmEnabled") == Some("true")
                })
                .filter_map(|m| m.attribute("algorithmName"))
                .map(str::to_string)
                .collect()
        })
        .unwrap()
}

/// The ensemble name for CHAID saves as `BuildChaidRuleNode`; the other three save as-is.
fn saved_id(enabled: &str) -> String {
    if enabled == "Chaid!spss.tree-extension" {
        "BuildChaidRuleNode".to_string()
    } else {
        enabled.to_string()
    }
}

/// Saved `trainPartitionStats` and `nrModelInputs` by builder, in Modeler's rank order.
fn saved_stats(nugget: &StreamNode) -> Vec<(String, f64, f64, usize)> {
    nugget
        .result_items
        .iter()
        .filter(|item| item.attribute("useModel") == Some("true"))
        .map(|item| {
            let builder = item
                .child("modelOutput")
                .and_then(|o| o.child("modelBuildInfo"))
                .and_then(|b| b.attribute("modelBuilderId"))
                .unwrap()
                .to_string();
            let stats = item.child("trainPartitionStats").unwrap();
            (
                builder,
                stats.attribute("correlation").unwrap().parse().unwrap(),
                stats.attribute("relativeError").unwrap().parse().unwrap(),
                item.attribute("nrModelInputs").unwrap().parse().unwrap(),
            )
        })
        .collect()
}

fn builders(case: &str) -> Vec<String> {
    let (stream, context) = load(case);
    let (nugget, input) = training_nugget(&stream, &context);
    saved_comparisons(nugget, &input, &context, 30, 10.0, 5.0)
        .unwrap()
        .into_iter()
        .map(|(builder, _)| builder)
        .collect()
}

#[test]
fn numeric_measures_and_ranking_match_modelers_saved_components() {
    let (stream, context) = load("an_corr");
    let (nugget, input) = training_nugget(&stream, &context);
    let measured = saved_comparisons(nugget, &input, &context, 30, 10.0, 5.0).unwrap();
    let stats = saved_stats(nugget);
    assert_eq!(measured.len(), 4);
    assert_eq!(stats.len(), 4);
    let by_builder: HashMap<String, Comparison> = measured.iter().cloned().collect();
    for (builder, correlation, relative_error, nr_inputs) in &stats {
        let m = by_builder.get(builder).unwrap();
        println!(
            "{builder}: correlation {:.7} (Modeler {correlation}), relativeError {:.7} (Modeler {relative_error}), nrInputs {nr_inputs}",
            m.correlation.unwrap(),
            m.relative_error.unwrap(),
        );
        // Modeler saves seven decimals.
        assert!(
            (m.correlation.unwrap() - correlation).abs() < 5e-8,
            "{builder} correlation"
        );
        assert!(
            (m.relative_error.unwrap() - relative_error).abs() < 5e-8,
            "{builder} relative error"
        );
    }
    // `an_corr` lists the components by descending correlation.
    let correlations: Vec<f64> = stats.iter().map(|(_, c, _, _)| *c).collect();
    assert!(
        correlations.windows(2).all(|w| w[0] > w[1]),
        "correlation order {correlations:?}"
    );
    // Enabled order with measures/fields for stable ranking.
    let enabled = enabled_order(&stream);
    assert_eq!(
        enabled,
        vec![
            "BuildRegressionNode",
            "spss.linearEngine!linearEngine",
            "LinearRegression!spss.alm-extension",
            "Chaid!spss.tree-extension",
        ]
    );
    let field_of: HashMap<String, usize> =
        stats.iter().map(|(b, _, _, n)| (b.clone(), *n)).collect();
    // Stable sorts from the enabled order (as saved ids).
    let mut by_corr: Vec<String> = enabled.iter().map(|b| saved_id(b)).collect();
    by_corr.sort_by(|a, b| {
        by_builder[b]
            .correlation
            .unwrap()
            .total_cmp(&by_builder[a].correlation.unwrap())
    });
    assert_eq!(by_corr[..2], builders("an_corr_n2"));
    assert_eq!(by_corr[..2], builders("an_training_n2"));
    let mut by_fields: Vec<String> = enabled.iter().map(|b| saved_id(b)).collect();
    by_fields.sort_by_key(|b| field_of[b]);
    // Regression (3, first) and Linear (3, second) over CHAID (3, later) and Linear-AS (4).
    assert_eq!(by_fields[..2], builders("an_fields_n2"));
    // Limits: correlation at least the cutoff, relative error at most it.
    let kept_corr: Vec<String> = by_corr
        .iter()
        .filter(|b| by_builder[*b].correlation.unwrap() >= 0.97)
        .cloned()
        .collect();
    assert_eq!(kept_corr, builders("an_corr_limit"));
    let kept_relerr: Vec<String> = by_corr
        .iter()
        .filter(|b| by_builder[*b].relative_error.unwrap() <= 0.1)
        .cloned()
        .collect();
    assert_eq!(kept_relerr, builders("an_relerr_limit"));
}
