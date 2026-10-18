//! Report node against the Modeler oracle (`oracle/misc/misc_rep_*`): the report's lines and,
//! in file mode, every file Modeler wrote, byte for byte.

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

mod support;

use dms_exec::{Document, DocumentTable};

/// The report text of an oracle output: a text file, or the `<pre>` of an HTML page.
fn report_document(text: &str) -> Document {
    let text = match (text.find("<pre>"), text.rfind("</pre>")) {
        (Some(start), Some(end)) if start < end => &text[start + 5..end],
        _ => text,
    };
    let rows = if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n')
            .map(|line| vec![line.strip_suffix('\r').unwrap_or(line).to_string()])
            .collect()
    };
    Document {
        title: "Report".into(),
        tables: vec![DocumentTable {
            title: None,
            header: Vec::new(),
            rows,
        }],
    }
}

/// Cases our Report refuses: Modeler's output object file (`.cou`) is a FastInfoset document
/// with a save timestamp.
const REFUSED: &[&str] = &["misc_rep_output_object"];

#[test]
fn report_lines_match_the_oracle() {
    let mut failures = Vec::new();
    for case in support::collect_oracle_cases(&["report"]) {
        let file = case.output_file.file_name().unwrap().to_string_lossy();
        let name = format!("{}/{file}", case.stream_name);
        // Corpus junctions are read-only. File-mode reports still need to write while
        // checking their returned lines, so redirect them just as the byte comparison does.
        let out = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("report_lines")
            .join(&case.stream_name);
        std::fs::create_dir_all(&out).unwrap();
        let actual = support::evaluate_stream_output_to(
            &case.label,
            &case.stream_path,
            &case.node_id,
            Some(out),
        );
        let refused = REFUSED.contains(&case.stream_name.as_str());
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
        let result = support::compare_documents(&actual, &report_document(&text));
        println!("{name}: {}/{} lines", result.matched, result.total);
        if result.matched != result.total {
            failures.push(format!("{name}: {:?}", result.diffs));
        }
    }
    assert!(
        failures.is_empty(),
        "report mismatches:
{}",
        failures.join(
            "
"
        )
    );
}

#[test]
fn report_files_match_the_oracle_byte_for_byte() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for case in support::collect_oracle_cases(&["report"]) {
        let exports = case
            .output_file
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("exports");
        let Ok(entries) = std::fs::read_dir(&exports) else {
            continue;
        };
        if REFUSED.contains(&case.stream_name.as_str()) {
            continue;
        }
        let out = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("report_exports")
            .join(&case.stream_name);
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let result = support::evaluate_stream_output_to(
            &case.label,
            &case.stream_path,
            &case.node_id,
            Some(out.clone()),
        );
        for entry in entries {
            let expected_path = entry.unwrap().path();
            let file = expected_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let name = format!("{}/{file}", case.stream_name);
            if let Err(error) = &result {
                println!("{name}: not evaluated ({error})");
                failures.push(name);
                continue;
            }
            let expected = std::fs::read(&expected_path).unwrap();
            let actual = std::fs::read(out.join(&file)).unwrap_or_default();
            checked += 1;
            if actual == expected {
                println!("{name}: identical ({} bytes)", expected.len());
            } else {
                println!(
                    "{name}: differs (ours {} bytes, Modeler {} bytes)",
                    actual.len(),
                    expected.len()
                );
                failures.push(name);
            }
        }
    }
    assert!(checked > 0, "no report file exports in the oracle");
    assert!(failures.is_empty(), "report files differ: {failures:?}");
}
