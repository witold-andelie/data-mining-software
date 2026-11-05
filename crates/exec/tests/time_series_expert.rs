//! Saved-nugget scoring regressions from the Expert probe, not selection tests.
#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "oracle assertions")]
mod support;

use dms_algo::{arima, time_series::FittedSpec};
use dms_strfmt::{ArchiveLimits, read_length_prefixed_zip};

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
