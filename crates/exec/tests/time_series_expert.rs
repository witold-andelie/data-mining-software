//! Saved-nugget scoring regressions and ES-only Expert selection tests.
#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]
mod support;

use dms_algo::{
    arima,
    smooth::{self, Kind, SmoothSpec},
    time_series::{self, FittedSpec},
};
use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_strfmt::{ArchiveLimits, read_length_prefixed_zip};
use std::collections::HashSet;
use std::sync::Arc;

#[test]
fn expert_white_noise_and_second_difference_scores() {
    let root = support::repo_root().join("corpus/ts_expert");
    for (case, data, difference) in [("pr_wn_ar", "wn", 0), ("pr_i2_ar", "i2", 2)] {
        let archive = dms_strfmt::container::read_archive(
            &std::fs::read(root.join(format!("{case}.str"))).unwrap(),
            &ArchiveLimits::default(),
        )
        .unwrap();
        let stream = dms_stream::Stream::from_document(&archive.document).unwrap();
        let nugget = stream
            .diagram
            .nodes
            .iter()
            .find(|n| n.type_identifier() == "spss.ts!ts_applier")
            .unwrap();
        let member = &nugget
            .containers
            .iter()
            .find(|c| c.name.as_deref() == Some("PMML"))
            .unwrap()
            .member;
        let bytes = &archive
            .attachments
            .iter()
            .find(|a| &a.name == member)
            .unwrap()
            .bytes;
        let entries = read_length_prefixed_zip(bytes, &ArchiveLimits::default()).unwrap();
        let xml = std::str::from_utf8(&entries.iter().find(|e| e.name == "0.xml").unwrap().bytes)
            .unwrap();
        let FittedSpec::Arima(spec) = dms_exec::nodes::time_series::parse_saved_model(xml).unwrap()
        else {
            panic!("expected ARIMA: {case}");
        };
        assert_eq!(spec.d, difference);
        assert!(spec.ar.is_empty() && spec.ma.is_empty());
        assert_ne!(spec.constant, 0.0);
        let input = std::fs::read_to_string(root.join(format!("{data}_train.csv"))).unwrap();
        let y: Vec<f64> = input
            .lines()
            .skip(1)
            .map(|line| support::parse_csv_line(line)[2].parse().unwrap())
            .collect();
        let scored = arima::score(&spec, &y, 6, 95.0).unwrap();
        let expected = std::fs::read_to_string(root.join(format!("{case}_scores_2.csv"))).unwrap();
        let mut lines = expected.lines();
        let columns = support::parse_csv_line(lines.next().unwrap());
        let indices: Vec<_> = ["$TS-y", "$TSLCI-y", "$TSUCI-y"]
            .iter()
            .map(|field| columns.iter().position(|c| c == field).unwrap())
            .collect();
        let forecasts: Vec<_> = scored
            .fitted
            .into_iter()
            .chain(scored.forecasts.into_iter().map(Some))
            .collect();
        let rows: Vec<_> = lines.map(support::parse_csv_line).collect();
        assert_eq!(forecasts.len(), rows.len());
        let mut max_error = 0.0_f64;
        for (row, forecast) in rows.iter().zip(forecasts) {
            if let Some(f) = forecast {
                for (index, value) in indices.iter().zip([f.point, f.lower, f.upper]) {
                    let oracle: f64 = row[*index].parse().unwrap();
                    max_error = max_error.max((value - oracle).abs());
                    assert!((value - oracle).abs() <= 1e-9 * oracle.abs().max(1.0));
                }
            } else {
                assert!(indices.iter().all(|index| row[*index] == "$null$"));
            }
        }
        println!(
            "{case}: {} rows, max absolute error {max_error}",
            rows.len()
        );
    }
}

#[test]
fn expert_finite_history_conditioning_and_short_seasonal_scores() {
    for (label, case, data, tol) in [
        ("ts_expert", "pr_air_ar", "air", 1e-9),
        ("ts_expert", "pr_seastr_ar", "seastr", 1e-9),
        ("ts_expert2", "p2_seastr_ar", "seastr", 1e-9),
        ("ts_expert2", "p2_alt18_es", "alt18", 1e-9),
    ] {
        let root = support::repo_root().join("corpus").join(label);
        let archive = dms_strfmt::container::read_archive(
            &std::fs::read(root.join(format!("{case}.str"))).unwrap(),
            &ArchiveLimits::default(),
        )
        .unwrap();
        let stream = dms_stream::Stream::from_document(&archive.document).unwrap();
        let nugget = stream
            .diagram
            .nodes
            .iter()
            .find(|n| n.type_identifier() == "spss.ts!ts_applier")
            .unwrap();
        let member = &nugget
            .containers
            .iter()
            .find(|c| c.name.as_deref() == Some("PMML"))
            .unwrap()
            .member;
        let bytes = &archive
            .attachments
            .iter()
            .find(|a| &a.name == member)
            .unwrap()
            .bytes;
        let entries = read_length_prefixed_zip(bytes, &ArchiveLimits::default()).unwrap();
        let xml = std::str::from_utf8(&entries.iter().find(|e| e.name == "0.xml").unwrap().bytes)
            .unwrap();
        let spec = dms_exec::nodes::time_series::parse_saved_model(xml).unwrap();
        let input = std::fs::read_to_string(root.join(format!("{data}_train.csv"))).unwrap();
        let y: Vec<f64> = input
            .lines()
            .skip(1)
            .map(|line| support::parse_csv_line(line)[2].parse().unwrap())
            .collect();
        let forecasts: Vec<Option<[f64; 3]>> = match &spec {
            FittedSpec::Arima(arima_spec) => {
                let s = arima::score(arima_spec, &y, 6, 95.0).unwrap();
                s.fitted
                    .into_iter()
                    .chain(s.forecasts.into_iter().map(Some))
                    .map(|f| f.map(|v| [v.point, v.lower, v.upper]))
                    .collect()
            }
            FittedSpec::Smooth(smooth_spec) => {
                let s = smooth::score(smooth_spec, &y, 6, 95.0).unwrap();
                s.fitted
                    .into_iter()
                    .chain(s.forecasts)
                    .map(|v| Some([v.point, v.lower, v.upper]))
                    .collect()
            }
            FittedSpec::Transfer(_) => panic!("unexpected transfer model"),
        };
        let expected = std::fs::read_to_string(root.join(format!("{case}_scores_2.csv"))).unwrap();
        let mut lines = expected.lines();
        let columns = support::parse_csv_line(lines.next().unwrap());
        let indices: Vec<_> = ["$TS-y", "$TSLCI-y", "$TSUCI-y"]
            .iter()
            .map(|field| columns.iter().position(|c| c == field).unwrap())
            .collect();
        let rows: Vec<_> = lines.map(support::parse_csv_line).collect();
        assert_eq!(forecasts.len(), rows.len());
        for (row, forecast) in rows.iter().zip(forecasts) {
            if let Some(f) = forecast {
                for (index, value) in indices.iter().zip(f) {
                    let oracle: f64 = row[*index].parse().unwrap();
                    assert!(
                        (value - oracle).abs() <= tol * oracle.abs().max(1.0),
                        "{label}/{case}: got {value}, expected {oracle}"
                    );
                }
            } else {
                assert!(indices.iter().all(|index| row[*index] == "$null$"));
            }
        }
    }
}

/// The case's stream archive and decoded stream.
fn open_case(
    label: &str,
    case: &str,
) -> (dms_strfmt::container::StreamArchive, dms_stream::Stream) {
    let dir = support::repo_root().join("corpus").join(label);
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(dir.join(format!("{case}.str"))).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    let stream = dms_stream::Stream::from_document(&archive.document).unwrap();
    (archive, stream)
}

/// The specification Modeler selected, from the nugget's saved PMML.
fn saved_spec(
    archive: &dms_strfmt::container::StreamArchive,
    stream: &dms_stream::Stream,
) -> FittedSpec {
    let nugget = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "spss.ts!ts_applier")
        .unwrap();
    let member = &nugget
        .containers
        .iter()
        .find(|c| c.name.as_deref() == Some("PMML"))
        .unwrap()
        .member;
    let bytes = &archive
        .attachments
        .iter()
        .find(|a| &a.name == member)
        .unwrap()
        .bytes;
    let entries = read_length_prefixed_zip(bytes, &ArchiveLimits::default()).unwrap();
    let xml =
        std::str::from_utf8(&entries.iter().find(|e| e.name == "0.xml").unwrap().bytes).unwrap();
    dms_exec::nodes::time_series::parse_saved_model(xml).unwrap()
}

/// The target series reaching the builder (its Type node's output, field `target`).
fn input_series(label: &str, stream: &dms_stream::Stream, target: &str) -> Vec<f64> {
    let dir = support::repo_root().join("corpus").join(label);
    let type_uid = stream
        .diagram
        .nodes
        .iter()
        .find(|n| n.type_identifier() == "TypeNode")
        .unwrap()
        .uid
        .clone();
    let context = ExecContext {
        files: FileResolver {
            data_dirs: vec![dir],
        },
        attachments: Arc::new(Default::default()),
        stream_properties: stream.properties.clone().map(Arc::new),
        ..ExecContext::default()
    };
    let results = Plan::new(stream).evaluate(&[&type_uid], &context);
    let Some(Ok(NodeOutput::Data(table))) = results.get(&type_uid) else {
        panic!("missing input: {:?}", results.get(&type_uid));
    };
    let at = table.model().position(target).unwrap();
    table
        .rows()
        .iter()
        .map(|r| r[at].as_f64().unwrap())
        .collect()
}

/// Every recorded Expert ARIMA selection: the label, case and saved specification, with the
/// series reaching its builder.
fn expert_arima_selections() -> Vec<(String, String, arima::ArimaSpec, Vec<f64>)> {
    let mut out = Vec::new();
    let mut add = |label: &str, case: &str| {
        let (archive, stream) = open_case(label, case);
        if let FittedSpec::Arima(spec) = saved_spec(&archive, &stream) {
            let y = input_series(label, &stream, "y");
            out.push((label.to_string(), case.to_string(), spec, y));
        }
    };
    for label in [
        "ts_expert",
        "ts_expert2",
        "ts_expert3",
        "ts_expert4",
        "ts_expert5",
        "ts_expert6",
    ] {
        let dir = support::repo_root().join("corpus").join(label);
        let mut cases: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| {
                let name = entry.unwrap().file_name().into_string().unwrap();
                name.strip_suffix(".str").map(str::to_string)
            })
            .collect();
        cases.sort();
        for case in cases {
            add(label, &case);
        }
    }
    add("ts_more", "ts_expert_arima_seasonal");
    add("ts_more", "ts_expert_arima_nonseasonal");
    out
}

/// Differencing (Guide appendix Step 3) reproduces every recorded `(d, D)`.
#[test]
fn expert_arima_differencing_matches_every_recorded_arima_selection() {
    let selections = expert_arima_selections();
    let mut seasonal = 0;
    for (label, case, spec, y) in &selections {
        let z: Vec<f64> = match spec.transform {
            arima::Transform::Log => y.iter().map(|v| v.ln()).collect(),
            arima::Transform::Sqrt => y.iter().map(|v| v.sqrt()).collect(),
            arima::Transform::None => y.clone(),
        };
        assert_eq!(
            arima::expert_differencing(&z, spec.period),
            (spec.d, spec.seasonal_d),
            "{label}/{case}: (d, D)"
        );
        seasonal += usize::from(spec.seasonal_d == 1);
    }
    eprintln!(
        "{} ARIMA selections, {seasonal} seasonally differenced",
        selections.len()
    );
    assert!(selections.len() >= 90 && seasonal >= 10);
}

fn kind_of(spec: &FittedSpec) -> Kind {
    match spec {
        FittedSpec::Smooth(SmoothSpec::Simple { .. }) => Kind::Simple,
        FittedSpec::Smooth(SmoothSpec::Holt { .. }) => Kind::Holt,
        FittedSpec::Smooth(SmoothSpec::General(g)) => g.kind,
        other => panic!("expected smooth spec, got {other:?}"),
    }
}

#[test]
fn expert_exponential_smoothing_selects_all_22_oracle_models_without_saved_pmml() {
    let cases = [
        ("ts_more", "ts_expert_exsmooth_nonseasonal", Kind::Simple),
        ("ts_more", "ts_expert_exsmooth_seasonal", Kind::WintersAdd),
        ("ts_expert", "pr_wn_es", Kind::Simple),
        ("ts_expert", "pr_rw_es", Kind::Simple),
        ("ts_expert", "pr_trend_es", Kind::Holt),
        ("ts_expert", "pr_i2_es", Kind::Brown),
        ("ts_expert", "pr_growthadd_es", Kind::Holt),
        ("ts_expert", "pr_growthlog_es", Kind::Holt),
        ("ts_expert", "pr_seas_es", Kind::Seasonal),
        ("ts_expert", "pr_seas_ns_es", Kind::Damped),
        ("ts_expert", "pr_seastr_es", Kind::WintersAdd),
        ("ts_expert", "pr_seasmul_es", Kind::WintersMul),
        ("ts_expert2", "p2_line_es", Kind::Holt),
        ("ts_expert2", "p2_i2_es", Kind::Brown),
        ("ts_expert2", "p2_rwflat_es", Kind::Simple),
        ("ts_expert2", "p2_rwunit_es", Kind::Simple),
        ("ts_expert2", "p2_ma2_es", Kind::Simple),
        ("ts_expert2", "p2_seastr_es", Kind::WintersAdd),
        ("ts_expert2", "p2_growth_es", Kind::Holt),
        ("ts_expert2", "p2_offset_es", Kind::Holt),
        ("ts_expert2", "p2_alt18_es", Kind::Seasonal),
        ("ts_expert2", "p2_alt60_es", Kind::Seasonal),
    ];
    for (label, case, expected_kind) in cases {
        let dir = support::repo_root().join("corpus").join(label);
        let (archive, mut stream) = open_case(label, case);
        let saved = saved_spec(&archive, &stream);
        assert_eq!(kind_of(&saved), expected_kind, "{label}/{case} saved kind");

        let builder = stream
            .diagram
            .nodes
            .iter_mut()
            .find(|n| n.type_identifier() == "spss.ts!ts_builder")
            .unwrap();
        // Strip saved containers so selection must run from observations and settings alone.
        builder.containers.clear();
        let builder_uid = builder.uid.clone();
        let input_uid = stream
            .diagram
            .nodes
            .iter()
            .find(|n| n.type_identifier() == "TypeNode")
            .unwrap()
            .uid
            .clone();
        let context = ExecContext {
            files: FileResolver {
                data_dirs: vec![dir],
            },
            attachments: Arc::new(Default::default()),
            stream_properties: stream.properties.clone().map(Arc::new),
            ..ExecContext::default()
        };
        let results = Plan::new(&stream).evaluate(&[&builder_uid, &input_uid], &context);
        let Some(Ok(NodeOutput::Model(pmml))) = results.get(&builder_uid) else {
            panic!("{label}/{case}: {:?}", results.get(&builder_uid));
        };
        let Some(Ok(NodeOutput::Data(table))) = results.get(&input_uid) else {
            panic!("{label}/{case}: missing input");
        };
        let dms_pmml::Model::Mining(m) = &pmml.model else {
            panic!("{label}/{case}: expected mining model");
        };
        let dms_pmml::Model::TimeSeries { xml: built_xml, .. } = m.segments[0].model.as_ref()
        else {
            panic!("{label}/{case}: expected TimeSeries segment");
        };
        let ours = dms_exec::nodes::time_series::parse_saved_model(built_xml).unwrap();
        assert_eq!(
            kind_of(&ours),
            expected_kind,
            "{label}/{case}: selected {:?}, expected {expected_kind:?}",
            kind_of(&ours)
        );
        let at = table.model().position("y").unwrap();
        let y: Vec<f64> = table
            .rows()
            .iter()
            .map(|r| r[at].as_f64().unwrap())
            .collect();
        let our_sse = time_series::objective(&ours, &y).unwrap();
        let saved_sse = time_series::objective(&saved, &y).unwrap();
        assert!(
            our_sse <= saved_sse + 1e-6 * saved_sse.abs().max(1.0),
            "{label}/{case}: our_sse={our_sse} > saved_sse={saved_sse}"
        );
    }
}

/// The constant: kept for a model without AR or MA terms, otherwise exactly when its t
/// statistic (ARMA part held fixed) exceeds 1.96, for every recorded ARIMA selection.
#[test]
fn expert_arima_constant_matches_every_recorded_arima_selection() {
    let selections = expert_arima_selections();
    let mut dropped = 0;
    let (mut max_dropped, mut min_kept_arma) = (0.0_f64, f64::INFINITY);
    for (label, case, spec, y) in &selections {
        let kept = spec.constant != 0.0;
        let t = arima::expert_constant_t(spec, y).unwrap();
        assert_eq!(
            arima::expert_keeps_constant(spec, y).unwrap(),
            kept,
            "{label}/{case}: constant kept={kept}, t={t}"
        );
        dropped += usize::from(!kept);
        let has_arma = [&spec.ar, &spec.ma, &spec.seasonal_ar, &spec.seasonal_ma]
            .iter()
            .any(|c| c.iter().any(|v| *v != 0.0));
        if kept && has_arma {
            min_kept_arma = min_kept_arma.min(t.abs());
        } else if !kept {
            max_dropped = max_dropped.max(t.abs());
        }
    }
    eprintln!(
        "{} ARIMA selections, {dropped} dropped constants; max |t| dropped {max_dropped:.3}; min |t| kept with ARMA terms {min_kept_arma:.3}",
        selections.len()
    );
    assert!(selections.len() >= 90 && dropped >= 10);
}

/// The transform (Guide appendix Step 2: OLS high-order AR on y, ln y and sqrt y, 4% gates) for
/// every recorded ARIMA selection.
#[test]
fn expert_arima_transform_matches_every_recorded_arima_selection() {
    let selections = expert_arima_selections();
    let mut logged = 0;
    for (label, case, spec, y) in &selections {
        assert_eq!(
            arima::expert_transform(y, spec.period),
            spec.transform,
            "{label}/{case}: transform"
        );
        logged += usize::from(spec.transform == arima::Transform::Log);
    }
    eprintln!("{} ARIMA selections, {logged} logged", selections.len());
    assert!(selections.len() >= 90 && logged >= 15);
}

// -----------------------------------------------------------------------------------------
// Steps 4-6 of the ARIMA Expert Modeler: full-selection parity with Modeler and with the
// Python reference (`target/ts-expert/expert.py`).

/// The structural part of a selected specification: AR/MA lag sets, seasonal orders, d/D,
/// transform and whether the constant is kept. Coefficient magnitudes/sigma2/df are NOT part
/// of the selection, so they are deliberately excluded.
#[derive(Debug, PartialEq, Eq)]
struct Signature {
    transform: arima::Transform,
    d: usize,
    dd: usize,
    ar: Vec<usize>,
    ma: Vec<usize>,
    p: usize,
    q: usize,
    constant: bool,
}

fn signature(spec: &arima::ArimaSpec) -> Signature {
    let lags = |coefs: &[f64]| -> Vec<usize> {
        coefs
            .iter()
            .enumerate()
            .filter(|(_, v)| **v != 0.0)
            .map(|(i, _)| i + 1)
            .collect()
    };
    Signature {
        transform: spec.transform,
        d: spec.d,
        dd: spec.seasonal_d,
        ar: lags(&spec.ar),
        ma: lags(&spec.ma),
        p: spec.seasonal_ar.len(),
        q: spec.seasonal_ma.len(),
        constant: spec.constant != 0.0,
    }
}

fn transform_name(t: arima::Transform) -> &'static str {
    match t {
        arima::Transform::None => "none",
        arima::Transform::Log => "log",
        arima::Transform::Sqrt => "sqrt",
    }
}

fn order_name(m: arima::OrderMethod) -> &'static str {
    match m {
        arima::OrderMethod::Acf => "acf",
        arima::OrderMethod::Pacf => "pacf",
        arima::OrderMethod::Eacf => "eacf",
    }
}

/// Frozen list of (label, case) pairs that `expert_arima` reproduces exactly. An improvement
/// makes a NOT-listed case start matching, which fails this test until the list is updated.
fn reproduced_cases() -> HashSet<(String, String)> {
    // Frozen after the first full run: 82 of the 93 recorded selections reproduce Modeler
    // exactly. Every case NOT listed must still mismatch (an improvement fails until added).
    // See REPORT-HY4.md for the per-record table and the count.
    let pairs: &[(&str, &str)] = &[
        ("ts_expert", "pr_alt18_all"),
        ("ts_expert", "pr_alt30_all"),
        ("ts_expert", "pr_alt96_all"),
        ("ts_expert", "pr_ar1_all"),
        ("ts_expert", "pr_ar1_ar"),
        ("ts_expert", "pr_growthadd_ar"),
        ("ts_expert", "pr_growthlog_ar"),
        ("ts_expert", "pr_i2_ar"),
        ("ts_expert", "pr_ma2_ar"),
        ("ts_expert", "pr_rw_ar"),
        ("ts_expert", "pr_seas30_ar"),
        ("ts_expert", "pr_seasmul_ar"),
        ("ts_expert", "pr_trend_ar"),
        ("ts_expert", "pr_wide_ar"),
        ("ts_expert", "pr_wn_ar"),
        ("ts_expert2", "p2_alt18_all"),
        ("ts_expert2", "p2_alt18_ar"),
        ("ts_expert2", "p2_alt60_all"),
        ("ts_expert2", "p2_alt60_ar"),
        ("ts_expert2", "p2_growth_ar"),
        ("ts_expert2", "p2_i2_ar"),
        ("ts_expert2", "p2_line_ar"),
        ("ts_expert2", "p2_offset_ar"),
        ("ts_expert2", "p2_rwflat_ar"),
        ("ts_expert2", "p2_rwunit_ar"),
        ("ts_expert2", "p2_seastr_ar"),
        ("ts_expert3", "p3_ar188_ar"),
        ("ts_expert3", "p3_ar2_all"),
        ("ts_expert3", "p3_ar2_ar"),
        ("ts_expert3", "p3_ima11nc_ar"),
        ("ts_expert3", "p3_ma3_all"),
        ("ts_expert3", "p3_ma3_ar"),
        ("ts_expert3", "p3_off1000_ar"),
        ("ts_expert3", "p3_off200_ar"),
        ("ts_expert3", "p3_seas36_ar"),
        ("ts_expert3", "p3_seas48_ar"),
        ("ts_expert3", "p3_sqrthet_ar"),
        ("ts_expert4", "p4_adfcrit_ar"),
        ("ts_expert4", "p4_adflag_ar"),
        ("ts_expert4", "p4_airzero_ar"),
        ("ts_expert4", "p4_arstart85_ar"),
        ("ts_expert4", "p4_arstart92_ar"),
        ("ts_expert4", "p4_difforder_ar"),
        ("ts_expert4", "p4_gate35_ar"),
        ("ts_expert4", "p4_gate36_ar"),
        ("ts_expert4", "p4_imazero_ar"),
        ("ts_expert4", "p4_ma5_ar"),
        ("ts_expert4", "p4_off500_ar"),
        ("ts_expert4", "p4_off800_ar"),
        ("ts_expert4", "p4_scale500_ar"),
        ("ts_expert4", "p4_scaleall1_ar"),
        ("ts_expert4", "p4_scaleall2_ar"),
        ("ts_expert4", "p4_seasonmid_ar"),
        ("ts_expert4", "p4_seasonweak_ar"),
        ("ts_expert4", "p4_weakma1_ar"),
        ("ts_expert4", "p4_weakma2_ar"),
        ("ts_expert5", "p5_gate600_ar"),
        ("ts_expert5", "p5_gate620_ar"),
        ("ts_expert5", "p5_gate640_ar"),
        ("ts_expert5", "p5_gate660_ar"),
        ("ts_expert5", "p5_gate680_ar"),
        ("ts_expert5", "p5_h090_ar"),
        ("ts_expert5", "p5_h130_ar"),
        ("ts_expert5", "p5_h155_ar"),
        ("ts_expert5", "p5_h170_ar"),
        ("ts_expert5", "p5_h185_ar"),
        ("ts_expert5", "p5_h220_ar"),
        ("ts_expert5", "p5_h300_ar"),
        ("ts_expert6", "p6_flatcent_ar"),
        ("ts_expert6", "p6_flatfive_ar"),
        ("ts_expert6", "p6_growthx10_ar"),
        ("ts_expert6", "p6_ma12n144_ar"),
        ("ts_expert6", "p6_ma12n240_ar"),
        ("ts_expert6", "p6_ma16n240_ar"),
        ("ts_expert6", "p6_ma8n144_ar"),
        ("ts_expert6", "p6_ma9n144_ar"),
        ("ts_expert6", "p6_off200x10_ar"),
        ("ts_expert6", "p6_rev85_ar"),
        ("ts_expert6", "p6_revadflag_ar"),
        ("ts_expert6", "p6_weak1_ar"),
        ("ts_expert6", "p6_weak4_ar"),
        ("ts_more", "ts_expert_arima_nonseasonal"),
    ];
    pairs
        .iter()
        .map(|(l, c)| (l.to_string(), c.to_string()))
        .collect()
}

/// Test 1: run `arima::expert_arima` on every recorded selection; match exactly for the frozen
/// list, and assert every other case still mismatches.
#[test]
fn expert_arima_full_selection_matches_recorded_cases() {
    let frozen = reproduced_cases();
    let mut unexpected = Vec::new();
    let mut reproduced = 0usize;
    let mut total = 0usize;
    for (label, case, spec, y) in expert_arima_selections() {
        total += 1;
        let got = arima::expert_arima(&y, spec.period).unwrap();
        let matches = signature(&got) == signature(&spec);
        if frozen.contains(&(label.clone(), case.clone())) {
            assert!(
                matches,
                "{label}/{case}: frozen reproduction FAILED\n got {:?}\n exp {:?}",
                signature(&got),
                signature(&spec)
            );
            reproduced += 1;
        } else if matches {
            unexpected.push(format!("{label}/{case}: {:?}", signature(&got)));
        }
    }
    assert!(
        unexpected.is_empty(),
        "{} newly reproduced cases (add to reproduced_cases):\n{}",
        unexpected.len(),
        unexpected.join("\n")
    );
    assert_eq!(
        reproduced,
        frozen.len(),
        "frozen cases missing from the selections"
    );
    eprintln!("{reproduced} reproduced / {total} recorded Expert ARIMA selections");
}

/// Load the 67 Python-reference records: name, effective seasonal setting, and the target
/// series (from the same corpus CSV columns the reference used).
fn reference_inputs() -> Vec<(String, usize, Vec<f64>)> {
    let root = support::repo_root();
    let reference: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("target/ts-expert/expert-reference.json")).unwrap(),
    )
    .unwrap();
    let audited: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("target/ts-expert/round5/audited120.json")).unwrap(),
    )
    .unwrap();
    let mut out = Vec::new();
    for (name, record) in reference.as_object().unwrap() {
        let season = record["season"].as_u64().unwrap() as usize;
        let entry = &audited[name];
        let data_path = entry["data_path"].as_str().unwrap().replace('\\', "/");
        let column = entry["column"].as_str().unwrap();
        let path = root.join(&data_path);
        let text = std::fs::read_to_string(&path).unwrap();
        let mut lines = text.lines();
        let header = support::parse_csv_line(lines.next().unwrap());
        let at = header.iter().position(|h| h == column).unwrap();
        let y: Vec<f64> = lines
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| {
                support::parse_csv_line(line)
                    .get(at)
                    .and_then(|v| v.parse::<f64>().ok())
            })
            .collect();
        assert!(!y.is_empty(), "{name}: empty series from {path:?}");
        out.push((name.clone(), season, y));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The two documented optimizer-difference exceptions allowed by the brief. In each, the Rust
/// bounded quasi-Newton converges tightly to the MA invertibility/admissibility boundary while
/// scipy's L-BFGS-B terminates early at an interior coefficient. That flips the MA boundary
/// rule's verdict (`min |root| < 1.001`) and hence the selection. Causes, with the compared
/// intermediate t-values, are in REPORT-HY4.md.
fn documented_exceptions() -> HashSet<&'static str> {
    ["mn_sales", "pr_seasmul_ar"].into_iter().collect()
}

/// Test 2: `#[ignore]` agreement with the Python reference on all 67 records, including the
/// initial orders, with at most the two documented exceptions. Writes a diagnostic dump of the
/// Rust selections and the intermediate trace for offline comparison.
/// Development only: it needs the Python lab files in `target/ts-expert/`, which are not in git.
#[test]
#[ignore]
fn expert_arima_agrees_with_python_reference() {
    let root = support::repo_root();
    let reference: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("target/ts-expert/expert-reference.json")).unwrap(),
    )
    .unwrap();
    let inputs = reference_inputs();
    let exceptions = documented_exceptions();
    let mut undoc_mismatches = Vec::new();
    let mut unexpected_agreement = Vec::new();
    let mut seen_exceptions = 0usize;
    let mut dump = serde_json::Map::new();
    let mut trace_lines = Vec::new();
    let mut agree = 0usize;
    for (name, season, y) in &inputs {
        dms_algo::expert::begin_trace();
        let sel = arima::expert_selection(y, *season).unwrap();
        for line in dms_algo::expert::end_trace() {
            trace_lines.push(format!("{name}\t{line}"));
        }
        let sig = signature(&sel.spec);
        let py = &reference[name]["python"];
        let arr = |key: &str| -> Vec<usize> {
            py[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as usize)
                .collect()
        };
        let want = Signature {
            transform: match py["transform"].as_str().unwrap() {
                "log" => arima::Transform::Log,
                "sqrt" => arima::Transform::Sqrt,
                _ => arima::Transform::None,
            },
            d: py["d"].as_u64().unwrap() as usize,
            dd: py["D"].as_u64().unwrap() as usize,
            ar: arr("ar"),
            ma: arr("ma"),
            p: py["P"].as_u64().unwrap() as usize,
            q: py["Q"].as_u64().unwrap() as usize,
            constant: py["constant"].as_bool().unwrap(),
        };
        let initial = py["initial"].as_array().unwrap();
        let want_initial = (
            initial[0].as_str().unwrap(),
            initial[1].as_u64().unwrap() as usize,
            initial[2].as_u64().unwrap() as usize,
        );
        let got_initial = (order_name(sel.initial.method), sel.initial.p, sel.initial.q);
        let ok = sig == want && got_initial == want_initial;
        let is_exception = exceptions.contains(name.as_str());
        if ok {
            agree += 1;
            if is_exception {
                unexpected_agreement.push(format!(
                    "{name}: documented exception now agrees; remove it from documented_exceptions"
                ));
            }
        } else if is_exception {
            seen_exceptions += 1;
        } else {
            undoc_mismatches.push(format!(
                "{name}: spec_eq={} initial_eq={}\n  got  {:?} init {:?}\n  want {:?} init {:?}",
                sig == want,
                got_initial == want_initial,
                sig,
                got_initial,
                want,
                want_initial
            ));
        }
        dump.insert(
            name.clone(),
            serde_json::json!({
                "ar": sig.ar, "ma": sig.ma, "P": sig.p, "Q": sig.q, "c": sig.constant,
                "d": sig.d, "D": sig.dd, "tr": transform_name(sig.transform),
                "initial": [got_initial.0, got_initial.1, got_initial.2],
            }),
        );
    }
    std::fs::write(
        root.join("target/ts-expert/rust-port.json"),
        serde_json::to_string_pretty(&dump).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("target/ts-expert/rust-trace.txt"),
        trace_lines.join("\n"),
    )
    .unwrap();
    eprintln!(
        "{agree}/{} agree with the Python reference; {} documented exceptions",
        inputs.len(),
        exceptions.len()
    );
    assert!(
        undoc_mismatches.is_empty(),
        "{} undocumented mismatches:\n{}",
        undoc_mismatches.len(),
        undoc_mismatches.join("\n")
    );
    assert!(
        unexpected_agreement.is_empty(),
        "{}",
        unexpected_agreement.join("\n")
    );
    assert_eq!(
        seen_exceptions,
        exceptions.len(),
        "documented exceptions no longer mismatch"
    );
    assert!(exceptions.len() <= 2, "too many documented exceptions");
}

/// The Time Series builder, with its saved containers stripped, selects ARIMA-only Expert
/// models through `arima::expert_arima` and fits the selected structure. Every reproduced
/// selection whose builder is ARIMA-only must come out of the node with Modeler's structure,
/// unless the fitter rejects the structure as a GAP.
#[test]
fn expert_arima_builder_selects_without_saved_pmml() {
    let frozen = reproduced_cases();
    let mut cases: Vec<_> = frozen.iter().cloned().collect();
    cases.sort();
    let (mut built, mut fit_gaps, mut other_family) = (Vec::new(), Vec::new(), Vec::new());
    for (label, case) in cases {
        let dir = support::repo_root().join("corpus").join(&label);
        let (archive, mut stream) = open_case(&label, &case);
        let FittedSpec::Arima(saved) = saved_spec(&archive, &stream) else {
            panic!("{label}/{case}: saved model is not ARIMA");
        };
        let builder = stream
            .diagram
            .nodes
            .iter_mut()
            .find(|n| n.type_identifier() == "spss.ts!ts_builder")
            .unwrap();
        builder.containers.clear();
        let builder_uid = builder.uid.clone();
        let context = ExecContext {
            files: FileResolver {
                data_dirs: vec![dir],
            },
            attachments: Arc::new(Default::default()),
            stream_properties: stream.properties.clone().map(Arc::new),
            ..ExecContext::default()
        };
        let results = Plan::new(&stream).evaluate(&[&builder_uid], &context);
        match results.get(&builder_uid) {
            Some(Ok(NodeOutput::Model(pmml))) => {
                let dms_pmml::Model::Mining(m) = &pmml.model else {
                    panic!("{label}/{case}: expected mining model");
                };
                let dms_pmml::Model::TimeSeries { xml, .. } = m.segments[0].model.as_ref() else {
                    panic!("{label}/{case}: expected TimeSeries segment");
                };
                let FittedSpec::Arima(ours) =
                    dms_exec::nodes::time_series::parse_saved_model(xml).unwrap()
                else {
                    panic!("{label}/{case}: built model is not ARIMA");
                };
                assert_eq!(
                    signature(&ours),
                    signature(&saved),
                    "{label}/{case}: built structure differs from Modeler's"
                );
                built.push(format!("{label}/{case}"));
            }
            Some(Err(e)) if e.to_string().contains("no recorded specification") => {
                other_family.push(format!("{label}/{case}"));
            }
            Some(Err(e)) if e.to_string().contains("GAP") => {
                fit_gaps.push(format!("{label}/{case}: {e}"));
            }
            other => panic!("{label}/{case}: {other:?}"),
        }
    }
    eprintln!(
        "built {} / fit GAP {} / AllModels {}\n{}",
        built.len(),
        fit_gaps.len(),
        other_family.len(),
        fit_gaps.join("\n")
    );
    // 8 reproduced selections come from AllModels builders (automatic AllModels selection is
    // GAP). 8 select structures the fitter rejects: d = 2, or more than four AR or MA lags.
    assert_eq!(
        (built.len(), fit_gaps.len(), other_family.len()),
        (66, 8, 8)
    );
}
