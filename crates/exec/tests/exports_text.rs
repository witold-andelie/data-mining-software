//! Byte-for-byte comparison of Flat File, JSON and XML exports with the Modeler oracle
//! (`oracle/exports/<case>/exports/<file>`).

#![cfg(feature = "corpus")]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use dms_strfmt::fastinfoset::Element;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn text_exports_match_the_oracle_byte_for_byte() {
    let root = support::repo_root();
    let oracle = root.join("oracle").join("exports");
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&oracle)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            path.join("manifest.json").is_file()
                && (name.starts_with("flat_") || name.starts_with("json_") || name == "xml_default")
        })
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no flat/json/xml export oracle cases");

    let mut failures = Vec::new();
    for case_dir in &cases {
        let name = case_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("case")
            .to_string();
        match check_case(&root, case_dir) {
            Ok(line) => println!("{name}: {line}"),
            Err(message) => {
                let line = message.lines().next().unwrap_or(message.as_str());
                println!("{name}: {line}");
                failures.push(format!("{name}: {message}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "export mismatches:\n{}",
        failures.join("\n")
    );
}

fn check_case(root: &Path, case_dir: &Path) -> Result<String, String> {
    let name = case_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("case");
    let manifest_text = std::fs::read_to_string(case_dir.join("manifest.json"))
        .map_err(|error| error.to_string())?;
    let manifest: JsonValue =
        serde_json::from_str(&manifest_text).map_err(|error| error.to_string())?;
    let node_id = manifest["runs"][0]["node"]["id"]
        .as_str()
        .ok_or_else(|| format!("{name}: manifest has no run node id"))?
        .to_string();
    let oracle_file = export_file(case_dir)?;
    let expected = std::fs::read(&oracle_file).map_err(|error| error.to_string())?;

    let stream_path = root
        .join("corpus")
        .join("exports")
        .join(format!("{name}.str"));
    let bytes = std::fs::read(&stream_path).map_err(|error| error.to_string())?;
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default())
        .map_err(|error| error.to_string())?;
    let stream = Stream::from_document(&archive.document).map_err(|error| error.to_string())?;
    let settings_dump = stream
        .diagram
        .nodes
        .iter()
        .find(|node| node.uid == node_id)
        .map(|node| settings_text(node.settings.as_ref()))
        .unwrap_or_default();

    let data_dirs = [root.join("corpus").join("exports")]
        .into_iter()
        .filter(|dir| dir.is_dir())
        .collect();
    let attachments: HashMap<String, Arc<[u8]>> = archive
        .attachments
        .into_iter()
        .map(|attachment| (attachment.name, attachment.bytes.into()))
        .collect();
    let export_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    if export_dir.exists() {
        std::fs::remove_dir_all(&export_dir).map_err(|error| error.to_string())?;
    }
    std::fs::create_dir_all(&export_dir).map_err(|error| error.to_string())?;
    let context = ExecContext {
        files: FileResolver { data_dirs },
        stream_properties: stream.properties.clone().map(Arc::new),
        attachments: Arc::new(attachments),
        decimal_places: Some(15),
        export_dir: Some(export_dir.clone()),
    };

    let plan = Plan::new(&stream);
    let results = plan.evaluate(&[node_id.as_str()], &context);
    match results.get(&node_id) {
        Some(Ok(NodeOutput::Data(_))) => {}
        Some(Ok(other)) => {
            return Err(format!(
                "expected data output, got {other:?}\n{settings_dump}"
            ));
        }
        Some(Err(error)) => return Err(format!("{error}\n{settings_dump}")),
        None => return Err(format!("node {node_id} was not evaluated\n{settings_dump}")),
    }

    let produced = export_dir.join(
        oracle_file
            .file_name()
            .ok_or_else(|| "oracle export has no file name".to_string())?,
    );
    let actual = std::fs::read(&produced)
        .map_err(|error| format!("missing {}: {error}\n{settings_dump}", produced.display()))?;
    let line = first_difference(&actual, &expected);
    if line == "identical" {
        Ok(line)
    } else {
        Err(format!("{line}\n{settings_dump}"))
    }
}

fn export_file(case_dir: &Path) -> Result<PathBuf, String> {
    let dir = case_dir.join("exports");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|error| error.to_string())?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    match files.as_slice() {
        [file] => Ok(file.clone()),
        _ => Err(format!(
            "{} exports has {} files",
            case_dir.display(),
            files.len()
        )),
    }
}

fn settings_text(settings: Option<&Element>) -> String {
    let Some(settings) = settings else {
        return "settings: none".to_string();
    };
    let mut lines = Vec::new();
    for child in settings.child_elements() {
        let text = child.text().replace('\n', "\\n");
        lines.push(format!("  {}={text}", child.name.local));
    }
    lines.join("\n")
}

/// `identical`, or the first line (1-based, split on `\n`) whose bytes differ.
fn first_difference(actual: &[u8], expected: &[u8]) -> String {
    let actual_lines = split_lines(actual);
    let expected_lines = split_lines(expected);
    for (index, pair) in actual_lines.iter().zip(expected_lines.iter()).enumerate() {
        if pair.0 != pair.1 {
            return format!(
                "line {}: actual {} expected {}",
                index + 1,
                show_window(pair.0, pair.1),
                show_window(pair.1, pair.0)
            );
        }
    }
    if actual_lines.len() != expected_lines.len() {
        return format!(
            "line {}: actual has {} lines, expected {}",
            actual_lines.len().min(expected_lines.len()) + 1,
            actual_lines.len(),
            expected_lines.len()
        );
    }
    if actual == expected {
        "identical".to_string()
    } else {
        format!(
            "bytes differ after the shared lines ({} vs {} bytes)",
            actual.len(),
            expected.len()
        )
    }
}

fn split_lines(bytes: &[u8]) -> Vec<&[u8]> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            lines.push(&bytes[start..=index]);
            start = index.saturating_add(1);
        }
    }
    if start < bytes.len() {
        lines.push(&bytes[start..]);
    }
    lines
}

/// A window around the first byte that differs, so a one-line JSON file still shows the cell.
fn show_window(line: &[u8], other: &[u8]) -> String {
    let at = line
        .iter()
        .zip(other.iter())
        .position(|(left, right)| left != right)
        .unwrap_or_else(|| line.len().min(other.len()));
    let start = at.saturating_sub(40);
    let end = at.saturating_add(80).min(line.len());
    let snippet = line.get(start..end).unwrap_or(line);
    format!("at {at} {:?}", String::from_utf8_lossy(snippet))
}
