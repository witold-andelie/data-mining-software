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

/// Seasonal differencing: D = 1 exactly when one seasonal difference lowers the variance of the
/// undifferenced (transformed) series, for every recorded seasonal ARIMA selection.
#[test]
fn expert_arima_seasonal_difference_matches_all_16_seasonal_oracle_models() {
    let cases = [
        ("ts_expert", "pr_seas_ar"),
        ("ts_expert", "pr_seastr_ar"),
        ("ts_expert", "pr_seasmul_ar"),
        ("ts_expert", "pr_air_ar"),
        ("ts_expert", "pr_alt96_all"),
        ("ts_expert2", "p2_seastr_ar"),
        ("ts_expert2", "p2_alt60_ar"),
        ("ts_expert2", "p2_alt60_all"),
        ("ts_expert3", "p3_seas36_ar"),
        ("ts_expert3", "p3_seas48_ar"),
        ("ts_more", "ts_expert_arima_seasonal"),
        ("ts_expert4", "p4_seasonweak_ar"),
        ("ts_expert4", "p4_seasonmid_ar"),
        ("ts_expert4", "p4_difforder_ar"),
        ("ts_expert4", "p4_airzero_ar"),
        ("ts_expert4", "p4_gate36_ar"),
    ];
    let mut seen_zero = 0;
    for (label, case) in cases {
        let (archive, stream) = open_case(label, case);
        let FittedSpec::Arima(spec) = saved_spec(&archive, &stream) else {
            panic!("{label}/{case}: Modeler did not select ARIMA");
        };
        assert_eq!(spec.period, 12, "{label}/{case}: period");
        let y = input_series(label, &stream, "y");
        let z: Vec<f64> = match spec.transform {
            arima::Transform::Log => y.iter().map(|v| v.ln()).collect(),
            arima::Transform::Sqrt => y.iter().map(|v| v.sqrt()).collect(),
            arima::Transform::None => y,
        };
        assert_eq!(
            arima::expert_seasonal_difference(&z, 12),
            spec.seasonal_d,
            "{label}/{case}: seasonal D"
        );
        seen_zero += usize::from(spec.seasonal_d == 0);
    }
    assert_eq!(seen_zero, 4, "both outcomes are covered");
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
    let mut checked = 0;
    let mut dropped = 0;
    let (mut max_dropped, mut min_kept_arma) = (0.0_f64, f64::INFINITY);
    for label in [
        "ts_expert",
        "ts_expert2",
        "ts_expert3",
        "ts_expert4",
        "ts_expert5",
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
            let (archive, stream) = open_case(label, &case);
            let FittedSpec::Arima(spec) = saved_spec(&archive, &stream) else {
                continue;
            };
            let y = input_series(label, &stream, "y");
            let kept = spec.constant != 0.0;
            let t = arima::expert_constant_t(&spec, &y).unwrap();
            assert_eq!(
                arima::expert_keeps_constant(&spec, &y).unwrap(),
                kept,
                "{label}/{case}: constant kept={kept}, t={t}"
            );
            checked += 1;
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
    }
    eprintln!(
        "{checked} ARIMA selections, {dropped} dropped constants; max |t| dropped {max_dropped:.3}; min |t| kept with ARMA terms {min_kept_arma:.3}"
    );
    assert!(checked >= 60, "{checked} ARIMA selections checked");
    assert!(dropped >= 10, "{dropped} dropped constants checked");
}
