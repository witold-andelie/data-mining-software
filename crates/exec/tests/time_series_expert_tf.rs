//! Offline transfer selection evidence; never invokes Modeler.
#![cfg(feature = "corpus")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "corpus assertions")]
mod support;
use dms_algo::expert_tf::{self, Predictor};

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
