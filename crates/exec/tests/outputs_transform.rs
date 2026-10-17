//! Transform output node against the Modeler oracle (`oracle/misc_next/transform_*`,
//! `oracle/misc/misc_tr_*`): the field-by-transformation table (Modeler exports its graph cells
//! empty).

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions and corpus setup fail loudly"
)]

mod support;

/// File output writes graph images (GAP): the node refuses it.
const REFUSED: &[&str] = &[
    "transform_html",
    "misc_tr_domains",
    "misc_tr_exp",
    "misc_tr_fractional_offset",
    "misc_tr_html",
    "misc_tr_html_pages",
    "misc_tr_inverse",
    "misc_tr_inverse_offset",
    "misc_tr_ln",
    "misc_tr_ln_offset",
    "misc_tr_log10",
    "misc_tr_log10_offset",
    "misc_tr_none",
    "misc_tr_output_object",
    "misc_tr_sqrt",
];

#[test]
fn transform_tables_match_the_oracle() {
    let cases = support::collect_oracle_cases(&["transform"]);
    assert!(cases.len() >= 4, "transform oracle cases: {}", cases.len());
    let mut failures = Vec::new();
    for case in cases {
        let name = case.stream_name.clone();
        let actual = support::evaluate_stream_output(&case.label, &case.stream_path, &case.node_id);
        let refused = REFUSED.contains(&name.as_str());
        let actual = match (actual, refused) {
            (Ok(actual), false) => actual,
            (Err(error), true) => {
                println!("{name}: refused as expected ({error})");
                continue;
            }
            (Ok(_), true) => {
                failures.push(format!("{name}: expected a refusal"));
                continue;
            }
            (Err(error), false) => {
                failures.push(format!("{name}: {error}"));
                continue;
            }
        };
        let text = std::fs::read_to_string(&case.output_file).unwrap();
        let result = support::compare_documents(&actual, &support::parse_csv_document(&text));
        println!("{name}: {}/{} cells", result.matched, result.total);
        if result.matched != result.total {
            failures.push(format!("{name}: {:?}", result.diffs));
        }
    }
    assert!(
        failures.is_empty(),
        "transform mismatches:\n{}",
        failures.join("\n")
    );
}
