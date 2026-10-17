//! Statistics export against the Modeler oracle (`oracle/exports/stat_*/exports/*.sav`).
//!
//! Header creation date and time (bytes 92..109) are the only bytes allowed to differ.

#![cfg(feature = "corpus")]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_io::sav_export::{Limits, SavFile, SystemValue as Value, read};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use dms_strfmt::fastinfoset::Element;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn sav_exports_match_the_oracle() {
    let root = support::repo_root();
    let oracle = root.join("oracle").join("exports");
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&oracle)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            path.join("manifest.json").is_file() && name.starts_with("stat_")
        })
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no statistics export oracle cases");

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
        "sav export mismatches:\n{}",
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
    let expected_bytes = std::fs::read(&oracle_file).map_err(|error| error.to_string())?;

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
    let actual_bytes = std::fs::read(&produced)
        .map_err(|error| format!("missing {}: {error}\n{settings_dump}", produced.display()))?;

    let limits = Limits::default();
    let actual = read(&actual_bytes, &limits)
        .map_err(|error| format!("reading produced sav: {error}\n{settings_dump}"))?;
    let expected = read(&expected_bytes, &limits)
        .map_err(|error| format!("reading oracle sav: {error}\n{settings_dump}"))?;
    if let Some(diff) = first_model_difference(&actual, &expected) {
        return Err(format!("{diff}\n{settings_dump}"));
    }
    if let Some(diff) = first_raw_difference(&actual_bytes, &expected_bytes) {
        return Err(format!("{diff}\n{settings_dump}"));
    }
    Ok("identical".to_string())
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

fn first_model_difference(actual: &SavFile, expected: &SavFile) -> Option<String> {
    let header = &actual.header;
    let want = &expected.header;
    if header.product != want.product {
        return Some(format!(
            "product: {:?} vs {:?}",
            header.product, want.product
        ));
    }
    if header.layout_code != want.layout_code {
        return Some(format!(
            "layout: {} vs {}",
            header.layout_code, want.layout_code
        ));
    }
    if header.endianness != want.endianness {
        return Some(format!(
            "endian: {:?} vs {:?}",
            header.endianness, want.endianness
        ));
    }
    if header.compression != want.compression {
        return Some(format!(
            "compression: {:?} vs {:?}",
            header.compression, want.compression
        ));
    }
    if header.nominal_case_size != want.nominal_case_size {
        return Some(format!(
            "nominal_case_size: {} vs {}",
            header.nominal_case_size, want.nominal_case_size
        ));
    }
    if header.weight_variable != want.weight_variable {
        return Some(format!(
            "weight: {:?} vs {:?}",
            header.weight_variable, want.weight_variable
        ));
    }
    if header.case_count != want.case_count {
        return Some(format!(
            "case_count: {:?} vs {:?}",
            header.case_count, want.case_count
        ));
    }
    if header.bias != want.bias {
        return Some(format!("bias: {:?} vs {:?}", header.bias, want.bias));
    }
    if header.file_label != want.file_label {
        return Some(format!(
            "file_label: {:?} vs {:?}",
            header.file_label, want.file_label
        ));
    }
    if actual.variables.len() != expected.variables.len() {
        return Some(format!(
            "variable count: {} vs {}",
            actual.variables.len(),
            expected.variables.len()
        ));
    }
    for (index, (got, want)) in actual.variables.iter().zip(&expected.variables).enumerate() {
        if got.name != want.name {
            return Some(format!(
                "var {index} name: {:?} vs {:?}",
                got.name, want.name
            ));
        }
        if got.short_name != want.short_name {
            return Some(format!(
                "var {index} short_name: {:?} vs {:?}",
                got.short_name, want.short_name
            ));
        }
        if got.label != want.label {
            return Some(format!(
                "var {index} label: {:?} vs {:?}",
                got.label, want.label
            ));
        }
        if got.width != want.width {
            return Some(format!(
                "var {index} width: {} vs {}",
                got.width, want.width
            ));
        }
        if got.print_format != want.print_format {
            return Some(format!(
                "var {index} print: {:?} vs {:?}",
                got.print_format, want.print_format
            ));
        }
        if got.write_format != want.write_format {
            return Some(format!(
                "var {index} write: {:?} vs {:?}",
                got.write_format, want.write_format
            ));
        }
        if got.measure != want.measure {
            return Some(format!(
                "var {index} measure: {:?} vs {:?}",
                got.measure, want.measure
            ));
        }
        if got.alignment != want.alignment {
            return Some(format!(
                "var {index} alignment: {:?} vs {:?}",
                got.alignment, want.alignment
            ));
        }
        if got.display_width != want.display_width {
            return Some(format!(
                "var {index} display_width: {:?} vs {:?}",
                got.display_width, want.display_width
            ));
        }
        if got.missing != want.missing {
            return Some(format!(
                "var {index} missing: {:?} vs {:?}",
                got.missing, want.missing
            ));
        }
    }
    if actual.value_labels != expected.value_labels {
        return Some(format!(
            "value labels: {:?} vs {:?}",
            actual.value_labels, expected.value_labels
        ));
    }
    if actual.documents != expected.documents {
        return Some(format!(
            "documents: {:?} vs {:?}",
            actual.documents, expected.documents
        ));
    }
    if actual.encoding != expected.encoding {
        return Some(format!(
            "encoding: {:?} vs {:?}",
            actual.encoding, expected.encoding
        ));
    }
    if actual.encoding_used != expected.encoding_used {
        return Some(format!(
            "encoding_used: {:?} vs {:?}",
            actual.encoding_used, expected.encoding_used
        ));
    }
    if actual.unknown_extensions != expected.unknown_extensions {
        return Some(format!(
            "extensions: {} record(s) vs {}; first difference {}",
            actual.unknown_extensions.len(),
            expected.unknown_extensions.len(),
            extension_diff(actual, expected)
        ));
    }
    if actual.rows.len() != expected.rows.len() {
        return Some(format!(
            "rows: {} vs {}",
            actual.rows.len(),
            expected.rows.len()
        ));
    }
    for (row_index, (got_row, want_row)) in actual.rows.iter().zip(&expected.rows).enumerate() {
        if got_row.len() != want_row.len() {
            return Some(format!(
                "row {row_index} width: {} vs {}",
                got_row.len(),
                want_row.len()
            ));
        }
        for (col, (got, want)) in got_row.iter().zip(want_row).enumerate() {
            if !same_value(got, want) {
                return Some(format!(
                    "row {row_index} col {col}: {} vs {}",
                    show_value(got),
                    show_value(want)
                ));
            }
        }
    }
    None
}

fn extension_diff(actual: &SavFile, expected: &SavFile) -> String {
    for (index, (got, want)) in actual
        .unknown_extensions
        .iter()
        .zip(&expected.unknown_extensions)
        .enumerate()
    {
        if got != want {
            let at = got
                .data
                .iter()
                .zip(&want.data)
                .position(|(a, b)| a != b)
                .unwrap_or(got.data.len().min(want.data.len()));
            let got_text = String::from_utf8_lossy(&got.data);
            let want_text = String::from_utf8_lossy(&want.data);
            return format!(
                "at {index}: subtype {} vs {} ({} vs {} bytes), first data byte {at}, actual {got_text:?} expected {want_text:?}",
                got.subtype,
                want.subtype,
                got.data.len(),
                want.data.len()
            );
        }
    }
    "length".to_string()
}

fn same_value(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::SystemMissing, Value::SystemMissing) => true,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => a.to_bits() == b.to_bits(),
        _ => false,
    }
}

fn show_value(value: &Value) -> String {
    match value {
        Value::SystemMissing => "sysmis".to_string(),
        Value::String(text) => format!("{text:?}"),
        Value::Number(number) => format!("{number:?} ({:#x})", number.to_bits()),
    }
}

/// First byte that differs outside the creation date (9 bytes at 92) and time (8 bytes at 101).
fn first_raw_difference(actual: &[u8], expected: &[u8]) -> Option<String> {
    let mut actual_masked = actual.to_vec();
    let mut expected_masked = expected.to_vec();
    mask_stamp(&mut actual_masked);
    mask_stamp(&mut expected_masked);
    let shared = actual_masked.len().min(expected_masked.len());
    for offset in 0..shared {
        if actual_masked.get(offset) != expected_masked.get(offset) {
            return Some(format!(
                "byte {offset:#x} in {}: {:02x} vs {:02x}",
                record_at(expected, offset),
                actual_masked.get(offset).copied().unwrap_or(0),
                expected_masked.get(offset).copied().unwrap_or(0)
            ));
        }
    }
    if actual_masked.len() != expected_masked.len() {
        Some(format!(
            "length {} vs {} after the shared prefix ({shared} bytes, {})",
            actual_masked.len(),
            expected_masked.len(),
            record_at(expected, shared)
        ))
    } else {
        None
    }
}

fn mask_stamp(bytes: &mut [u8]) {
    if let Some(stamp) = bytes.get_mut(92..109) {
        for byte in stamp {
            *byte = 0;
        }
    }
}

fn record_at(file: &[u8], offset: usize) -> String {
    if offset < 176 {
        return format!("header byte {offset}");
    }
    let mut pos = 176usize;
    while pos.saturating_add(4) <= file.len() {
        let Some(rec_bytes) = file.get(pos..pos + 4) else {
            break;
        };
        let Ok(rec_bytes) = <[u8; 4]>::try_from(rec_bytes) else {
            break;
        };
        let rec = i32::from_le_bytes(rec_bytes);
        if rec == 999 {
            let end = pos.saturating_add(8);
            if offset < end {
                return format!("dictionary termination at {pos:#x}");
            }
            return format!("data at {end:#x}");
        }
        let Some(end) = record_end(file, pos, rec) else {
            return format!("record type {rec} at {pos:#x}");
        };
        if offset < end {
            return match rec {
                2 => format!("variable record at {pos:#x}"),
                3 => format!("value labels at {pos:#x}"),
                7 => format!(
                    "extension subtype {} at {pos:#x}",
                    extension_subtype(file, pos)
                ),
                other => format!("record type {other} at {pos:#x}"),
            };
        }
        pos = end;
    }
    format!("offset {offset:#x}")
}

fn extension_subtype(file: &[u8], pos: usize) -> i32 {
    file.get(pos + 4..pos + 8)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
        .map(i32::from_le_bytes)
        .unwrap_or(0)
}

fn record_end(file: &[u8], pos: usize, rec: i32) -> Option<usize> {
    match rec {
        2 => variable_end(file, pos),
        3 => value_label_end(file, pos),
        7 => {
            let size = i32_at(file, pos + 8)?;
            let count = i32_at(file, pos + 12)?;
            let nbytes = usize::try_from(size)
                .ok()?
                .checked_mul(usize::try_from(count).ok()?)?;
            pos.checked_add(16)?.checked_add(nbytes)
        }
        _ => None,
    }
}

fn variable_end(file: &[u8], pos: usize) -> Option<usize> {
    let has_label = i32_at(file, pos + 8)?;
    let missing = i32_at(file, pos + 12)?;
    let mut cursor = pos.checked_add(32)?;
    if has_label == 1 {
        let len = usize::try_from(i32_at(file, cursor)?).ok()?;
        let padded = len.div_ceil(4).saturating_mul(4);
        cursor = cursor.checked_add(4)?.checked_add(padded)?;
    }
    let n_missing = match missing {
        0 => 0,
        1..=3 => usize::try_from(missing).ok()?,
        -2 => 2,
        -3 => 3,
        _ => return None,
    };
    cursor.checked_add(n_missing.saturating_mul(8))
}

fn value_label_end(file: &[u8], pos: usize) -> Option<usize> {
    let count = usize::try_from(i32_at(file, pos + 4)?).ok()?;
    let mut cursor = pos.checked_add(8)?;
    for _ in 0..count {
        let len = usize::from(*file.get(cursor.checked_add(8)?)?);
        let padded = (len.saturating_add(1))
            .div_ceil(8)
            .saturating_mul(8)
            .saturating_sub(1);
        cursor = cursor.checked_add(8)?.checked_add(1)?.checked_add(padded)?;
    }
    let nvars = usize::try_from(i32_at(file, cursor + 4)?).ok()?;
    cursor.checked_add(8)?.checked_add(nvars.saturating_mul(4))
}

fn i32_at(file: &[u8], pos: usize) -> Option<i32> {
    let bytes = file.get(pos..pos.checked_add(4)?)?;
    Some(i32::from_le_bytes(bytes.try_into().ok()?))
}
