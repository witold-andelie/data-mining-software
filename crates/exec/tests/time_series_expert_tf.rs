//! Offline transfer selection evidence; never invokes Modeler.
#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "corpus assertions")]
mod support;
use dms_algo::expert_tf::{self, Predictor};
use dms_algo::time_series::FittedSpec;
use dms_strfmt::{ArchiveLimits, read_length_prefixed_zip};
use std::collections::BTreeSet;

/// Lags of nonzero coefficients, `offset`-based.
fn lags(v: &[f64], offset: usize) -> Vec<usize> {
    v.iter()
        .enumerate()
        .filter(|(_, v)| **v != 0.0)
        .map(|(i, _)| i + offset)
        .collect()
}

/// The selected structure: transform, (d, D), noise lags, constant and, per kept predictor,
/// transform, differencing, delay and numerator/denominator lags. Field names are left out:
/// Modeler's PMML names predictors by index.
fn structure(spec: &FittedSpec) -> String {
    let ar = match spec {
        FittedSpec::Arima(s) => s,
        FittedSpec::Transfer(s) => &s.noise,
        FittedSpec::Smooth(_) => return "smoothing".into(),
    };
    let mut text = format!(
        "{:?}; d={},D={}; AR{:?} MA{:?} SAR{:?} SMA{:?}; s={}; c={}",
        ar.transform,
        ar.d,
        ar.seasonal_d,
        lags(&ar.ar, 1),
        lags(&ar.ma, 1),
        lags(&ar.seasonal_ar, 1),
        lags(&ar.seasonal_ma, 1),
        ar.period,
        ar.constant != 0.0
    );
    match spec {
        FittedSpec::Transfer(s) if !s.effects.is_empty() => {
            for e in &s.effects {
                text.push_str(&format!(
                    "; x: {:?} d={},D={}, delay={}, num{:?}, den{:?}",
                    e.transform,
                    e.difference,
                    e.seasonal_difference,
                    e.delay,
                    lags(&e.numerator, 0),
                    lags(&e.denominator, 1)
                ));
            }
        }
        _ => text.push_str("; no predictors"),
    }
    text
}

/// Modeler's selection, from the nugget's saved PMML (its PMML export of a model with
/// predictors is an empty file).
fn modeler_structure(case: &str) -> String {
    let path = support::repo_root().join(format!("corpus/ts_expert_tf/tfx_{case}.str"));
    let archive = dms_strfmt::container::read_archive(
        &std::fs::read(path).unwrap(),
        &ArchiveLimits::default(),
    )
    .unwrap();
    for attachment in &archive.attachments {
        let Ok(entries) = read_length_prefixed_zip(&attachment.bytes, &ArchiveLimits::default())
        else {
            continue;
        };
        let names: BTreeSet<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        if names.contains("0.xml") && names.contains("TSDPOutput.json") {
            let entry = entries.iter().find(|e| e.name == "0.xml").unwrap();
            let xml = std::str::from_utf8(&entry.bytes).unwrap();
            return structure(&dms_exec::nodes::time_series::parse_saved_model(xml).unwrap());
        }
    }
    panic!("{case}: no saved Time Series model");
}

/// The probe's case recipes (JSON embedded in the probe script; nothing is executed).
fn recipes() -> Vec<serde_json::Value> {
    let source = std::fs::read_to_string(
        support::repo_root().join("xtask/oracle/probes/ts_expert_tf_streams.py"),
    )
    .unwrap();
    let data = source
        .split("CASES = json.loads(r'''")
        .nth(1)
        .unwrap()
        .split("''')")
        .next()
        .unwrap();
    serde_json::from_str(data).unwrap()
}

/// Every prospective case Modeler ran (`corpus/ts_expert_tf`, 23 of 26: the TS node has no
/// `forced_inputs` property). The selector runs on the recorded input CSV. The cases listed
/// here reproduce Modeler's saved structure; every other case must still differ, so an
/// improvement fails until it is listed.
#[test]
fn probe_selections_match_modeler() {
    let reproduced: BTreeSet<&str> = [
        "constant",
        "delay1",
        "delay12",
        "delay3",
        "delay6",
        "event_pulse",
        "irrelevant_candidate",
        "log",
        "log_nonpositive_input",
        "numerator1",
        "numerator_gap",
        "seasonal",
        "short11",
        "short19",
        "short20",
        "short_season",
        "static",
        "two_candidates",
    ]
    .into_iter()
    .collect();
    let root = support::repo_root().join("corpus/ts_expert_tf");
    let flag = |c: &serde_json::Value, k: &str| c[k].as_bool().unwrap_or(false);
    let (mut ran, mut matched, mut unexpected) = (0, 0, Vec::new());
    for c in recipes() {
        let name = c["name"].as_str().unwrap();
        if !root.join(format!("tfx_{name}.str")).exists() {
            continue;
        }
        ran += 1;
        let text = std::fs::read_to_string(root.join(format!("{name}_train.csv"))).unwrap();
        let rows: Vec<Vec<String>> = text.lines().skip(1).map(support::parse_csv_line).collect();
        let column = |i: usize| -> Vec<f64> {
            rows.iter()
                .map(|r| r[i].trim().parse::<f64>().unwrap_or(f64::NAN))
                .collect()
        };
        let mut predictors = vec![Predictor {
            field: "x".into(),
            values: column(3),
            forced: flag(&c, "forced"),
            event: flag(&c, "event"),
        }];
        if flag(&c, "irrelevant") {
            predictors.push(Predictor {
                field: "z".into(),
                values: column(4),
                forced: false,
                event: false,
            });
        }
        let season = c["season"].as_u64().unwrap() as usize;
        let ours = expert_tf::select(&column(2), &predictors, season, flag(&c, "all")).unwrap();
        let got = structure(&ours.fit.spec);
        let want = modeler_structure(name);
        if reproduced.contains(name) {
            assert_eq!(got, want, "{name}: frozen reproduction lost");
            matched += 1;
        } else if got == want {
            unexpected.push(name.to_string());
        }
    }
    assert!(
        unexpected.is_empty(),
        "newly reproduced (add to the list): {unexpected:?}"
    );
    assert_eq!((ran, matched), (23, reproduced.len()));
}

#[test]
fn broadband_selection_without_saved_parameters() {
    let text = std::fs::read_to_string(
        support::repo_root()
            .join("oracle/ibm-demos/broadband_create_models/data/03_type_id3JCTGSEU4GM.csv"),
    )
    .unwrap();
    let mut lines = text.lines();
    let names = support::parse_csv_line(lines.next().unwrap());
    let rows: Vec<_> = lines.map(support::parse_csv_line).collect();
    let column = |i: usize| {
        rows.iter()
            .map(|r| r[i].parse::<f64>().unwrap())
            .collect::<Vec<_>>()
    };
    for (i, name) in names.iter().take(6).enumerate() {
        let inputs: Vec<_> = names
            .iter()
            .take(5)
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(j, name)| Predictor {
                field: name.clone(),
                values: column(j),
                forced: false,
                event: false,
            })
            .collect();
        let result = expert_tf::select(&column(i), &inputs, 12, true).unwrap();
        println!(
            "TFX {name}: {:?}; fallback={:?}",
            result.fit.spec, result.fallback_reason
        );
        assert!(result.fit.objective.is_finite());
    }
}
