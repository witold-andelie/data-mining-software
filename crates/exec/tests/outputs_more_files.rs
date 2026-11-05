//! Outputs_more file exports: every file Modeler wrote under `oracle/outputs_more/<case>/exports/`
//! byte for byte, with numbers compared the way the screen tests compare them (same decimals,
//! relative 1e-12; `support::numbers_close` is private so the rule is repeated here). Integer
//! tables (Matrix counts) and the Analysis text/HTML match byte for byte; Means, Statistics and
//! Data Audit floating point values may differ by an ulp or two, exactly like the screen exports
//! (`cargo test -p dms-exec --features corpus --test outputs_statistics` prints the same
//! `within 1e-12` lines). `.cou` (FastInfoset with a save timestamp) and Data Audit HTML with
//! graph images (`.jpg`) stay explicit `GAP:` errors and are skipped here.
//!
//! Files use CRLF; HTML has no trailing newline, delimited and text files do.

#![cfg(feature = "corpus")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

mod support;

fn is_gap_case(name: &str) -> bool {
    name.contains("matrix_file_output")
        || name.contains("means_file_output")
        || name.contains("analysis_file_output")
        || name.contains("statistics_file_output")
        || name.contains("dataaudit_file_output")
        || name.contains("dataaudit_file_html")
}

fn numbers_close(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    // Split off a trailing "/Label" (Statistics correlations) or "%" (Analysis percents).
    let (a_num, a_suffix) = match a.find('/') {
        Some(at) => (&a[..at], &a[at..]),
        None => match a.strip_suffix('%') {
            Some(num) => (num, "%"),
            None => (a, ""),
        },
    };
    let (b_num, b_suffix) = match b.find('/') {
        Some(at) => (&b[..at], &b[at..]),
        None => match b.strip_suffix('%') {
            Some(num) => (num, "%"),
            None => (b, ""),
        },
    };
    if a_suffix != b_suffix {
        return false;
    }
    let (Ok(fa), Ok(fb)) = (a_num.parse::<f64>(), b_num.parse::<f64>()) else {
        return false;
    };
    if !fa.is_finite() || !fb.is_finite() {
        return false;
    }
    let decimals = |s: &str| s.split_once('.').map_or(0, |(_, frac)| frac.len());
    if decimals(a_num) != decimals(b_num) {
        return false;
    }
    (fa - fb).abs() <= 1e-12 * fa.abs().max(fb.abs()).max(1.0)
}

/// Split `text` into non-numeric structure and the numbers in order. A number starts with an
/// optional `-` followed by digits with an optional `.digits`, where the char before is not
/// alphanumeric (so `n1`, `before1`, `cat1` keep their digits) and the char after is not
/// alphanumeric (so `/Perfect` and `%` stay in the structure).
fn mask_numbers(text: &str) -> (String, Vec<String>) {
    let bytes = text.as_bytes();
    let mut masked = String::with_capacity(text.len());
    let mut numbers = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        let prev_alnum = i > 0 && (bytes[i - 1] as char).is_ascii_alphanumeric();
        let is_start = (c == '-' && i + 1 < bytes.len() && (bytes[i + 1] as char).is_ascii_digit())
            || c.is_ascii_digit();
        if is_start && !prev_alnum {
            let mut j = i + usize::from(c == '-');
            while j < bytes.len() && (bytes[j] as char).is_ascii_digit() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'.' {
                let mut k = j + 1;
                while k < bytes.len() && (bytes[k] as char).is_ascii_digit() {
                    k += 1;
                }
                if k > j + 1 {
                    j = k;
                }
            }
            let next_alnum = j < bytes.len() && (bytes[j] as char).is_ascii_alphanumeric();
            if !next_alnum {
                numbers.push(text[i..j].to_string());
                masked.push('N');
                i = j;
                continue;
            }
        }
        masked.push(c);
        // Keep char-boundary stepping for non-ASCII (the ¹ footnote marker).
        i += c.len_utf8().max(1);
        // `c` came from a single byte; for multi-byte chars recompute from the string.
        if !text.is_char_boundary(i) {
            // Advance to the next boundary without losing bytes (only the ¹ marker occurs).
            while i < text.len() && !text.is_char_boundary(i) {
                masked.pop();
                i += 1;
            }
            if let Some(ch) = text[i.saturating_sub(1)..].chars().next() {
                let _ = ch;
            }
            // Rebuild the tail safely: fall back to pushing the raw slice.
            masked.push_str(&text[i.saturating_sub(1)..i]);
        }
    }
    (masked, numbers)
}

#[test]
fn outputs_more_files_match_the_oracle() {
    let mut failures = Vec::new();
    let mut checked = 0;
    // Group exports by case so each stream runs once.
    let mut by_case: std::collections::BTreeMap<String, Vec<std::path::PathBuf>> =
        std::collections::BTreeMap::new();
    let root = support::repo_root().join("oracle").join("outputs_more");
    let Ok(cases) = std::fs::read_dir(&root) else {
        panic!("no oracle/outputs_more cases");
    };
    for entry in cases {
        let dir = entry.unwrap().path();
        if !dir.join("manifest.json").is_file() {
            continue;
        }
        let exports = dir.join("exports");
        let Ok(_) = std::fs::read_dir(&exports) else {
            continue;
        };
        let mut all: Vec<std::path::PathBuf> = Vec::new();
        collect_files(&exports, &mut all);
        if all.is_empty() {
            continue;
        }
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        by_case.insert(name, all);
    }
    for (case_name, expected_files) in &by_case {
        // Find the stream for this case from the manifest.
        let manifest_text =
            std::fs::read_to_string(root.join(case_name).join("manifest.json")).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest_text).unwrap();
        let stream_recorded = manifest["stream"].as_str().unwrap().to_string();
        let stream_path = support::local_path(&stream_recorded);
        // The terminal node id is the file-mode output node (the only output in these streams).
        let outputs = manifest["outputs"].as_array().cloned().unwrap_or_default();
        if outputs.is_empty() {
            continue;
        }
        let node_id = outputs[0]["id"].as_str().unwrap().to_string();
        let label = "outputs_more".to_string();
        let out = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("outputs_more_files")
            .join(case_name);
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let result =
            support::evaluate_stream_output_to(&label, &stream_path, &node_id, Some(out.clone()));
        // `.cou` and Data Audit HTML stay GAPs and write nothing.
        if is_gap_case(case_name) {
            match result {
                Err(error) => {
                    println!("{case_name}: GAP as expected ({error})");
                    continue;
                }
                Ok(_) => {
                    failures.push(format!("{case_name}: expected a GAP, but the node ran"));
                    continue;
                }
            }
        }
        if let Err(error) = &result {
            failures.push(format!("{case_name}: not evaluated ({error})"));
            continue;
        }
        for expected_path in expected_files {
            let rel = expected_path
                .strip_prefix(root.join(case_name).join("exports"))
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let name = format!("{case_name}/{rel}");
            // Skip image binaries and their folder (no byte-stable oracle).
            if rel.ends_with(".jpg") || rel.contains("_files") {
                continue;
            }
            let expected = std::fs::read(expected_path).unwrap();
            let actual = std::fs::read(out.join(&rel)).unwrap_or_default();
            checked += 1;
            if actual == expected {
                println!("{name}: identical ({} bytes)", expected.len());
                continue;
            }
            let expected_text = String::from_utf8_lossy(&expected);
            let actual_text = String::from_utf8_lossy(&actual);
            // Binary files (none expected here besides .cou, already skipped) must match exactly.
            if expected_text.contains('\u{FFFD}') || actual_text.contains('\u{FFFD}') {
                failures.push(format!(
                    "{name}: differs (ours {} bytes, Modeler {} bytes)",
                    actual.len(),
                    expected.len()
                ));
                continue;
            }
            let (masked_expected, nums_expected) = mask_numbers(&expected_text);
            let (masked_actual, nums_actual) = mask_numbers(&actual_text);
            if masked_expected != masked_actual {
                failures.push(format!(
                    "{name}: structure differs (ours {} bytes, Modeler {} bytes)",
                    actual.len(),
                    expected.len()
                ));
                continue;
            }
            if nums_expected.len() != nums_actual.len() {
                failures.push(format!("{name}: different number counts"));
                continue;
            }
            let mut ok = true;
            for (a, e) in nums_actual.iter().zip(&nums_expected) {
                if !numbers_close(a, e) {
                    ok = false;
                    break;
                }
            }
            if ok {
                println!(
                    "{name}: within 1e-12 ({} bytes, {} numbers)",
                    expected.len(),
                    nums_expected.len()
                );
            } else {
                failures.push(format!("{name}: numbers differ beyond 1e-12"));
            }
        }
    }
    assert!(checked > 0, "no outputs_more file exports compared");
    assert!(
        failures.is_empty(),
        "outputs_more file mismatches:\n{}",
        failures.join("\n")
    );
}

fn collect_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_files(&path, out);
        } else {
            out.push(path);
        }
    }
}
