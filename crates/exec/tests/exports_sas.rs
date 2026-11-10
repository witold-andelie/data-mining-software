//! SAS export against the Modeler 19 oracle (`oracle/sas_export/*/exports/*`).
//!
//! For `.sas7bdat` exports (`SAS7` and `SAS8`), only the header timestamp bytes `166..=175` and
//! `290..=303` are masked; all other bytes must be identical.
//! For `.sd2` (`Windows`) and `.ssd` (`UNIX`) exports, every byte must be identical.

#![cfg(feature = "corpus")]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_io::sas::{NameMode, SasFormat, SasOptions, read_table};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use dms_strfmt::fastinfoset::Element;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn sas_exports_match_the_oracle() {
    let root = support::repo_root();
    let oracle = root.join("oracle").join("sas_export");
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&oracle)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.join("manifest.json").is_file())
        .collect();
    cases.sort();
    assert_eq!(cases.len(), 5, "expected 5 SAS export oracle cases");

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
        "SAS export mismatches:\n{}",
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
    let export_node_id = manifest["runs"][0]["node"]["id"]
        .as_str()
        .ok_or_else(|| format!("{name}: manifest has no run node id"))?
        .to_string();

    let oracle_file = export_file(case_dir)?;
    let expected_bytes = std::fs::read(&oracle_file).map_err(|error| error.to_string())?;

    let stream_path = root
        .join("corpus")
        .join("sas_export")
        .join(format!("{name}.str"));
    let bytes = std::fs::read(&stream_path).map_err(|error| error.to_string())?;
    let archive = dms_strfmt::container::read_archive(&bytes, &ArchiveLimits::default())
        .map_err(|error| error.to_string())?;
    let stream = Stream::from_document(&archive.document).map_err(|error| error.to_string())?;
    let settings_dump = stream
        .diagram
        .nodes
        .iter()
        .find(|node| node.uid == export_node_id)
        .map(|node| settings_text(node.settings.as_ref()))
        .unwrap_or_default();

    let data_dirs = [root.join("corpus").join("sas_export")]
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
        cancel: Default::default(),
        files: FileResolver { data_dirs },
        stream_properties: stream.properties.clone().map(Arc::new),
        attachments: Arc::new(attachments),
        decimal_places: Some(15),
        export_dir: Some(export_dir.clone()),
    };

    let plan = Plan::new(&stream);
    let results = plan.evaluate(&[export_node_id.as_str()], &context);
    match results.get(&export_node_id) {
        Some(Ok(NodeOutput::Data(table))) => {
            if table.rows().len() != 4 || table.model().fields().len() != 5 {
                return Err(format!(
                    "unexpected pass-through table shape: {}x{}\n{settings_dump}",
                    table.rows().len(),
                    table.model().fields().len()
                ));
            }
        }
        Some(Ok(other)) => {
            return Err(format!(
                "expected data output, got {other:?}\n{settings_dump}"
            ));
        }
        Some(Err(error)) => return Err(format!("{error}\n{settings_dump}")),
        None => {
            return Err(format!(
                "node {export_node_id} was not evaluated\n{settings_dump}"
            ));
        }
    }

    let produced = export_dir.join(
        oracle_file
            .file_name()
            .ok_or_else(|| "oracle export has no file name".to_string())?,
    );
    let actual_bytes = std::fs::read(&produced)
        .map_err(|error| format!("missing {}: {error}\n{settings_dump}", produced.display()))?;

    let is_sas7bdat = oracle_file
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("sas7bdat"));

    if is_sas7bdat {
        let opts = SasOptions {
            format: SasFormat::Sas7bdat,
            member_name: None,
            names: NameMode::NamesAndLabels,
        };
        let actual_table = read_table(&actual_bytes, &opts)
            .map_err(|e| format!("reading produced sas7bdat: {e}\n{settings_dump}"))?;
        let expected_table = read_table(&expected_bytes, &opts)
            .map_err(|e| format!("reading oracle sas7bdat: {e}\n{settings_dump}"))?;
        if actual_table.model().fields() != expected_table.model().fields() {
            return Err(format!(
                "parsed fields mismatch: {:?} vs {:?}\n{settings_dump}",
                actual_table.model().fields(),
                expected_table.model().fields()
            ));
        }
        if actual_table.rows() != expected_table.rows() {
            return Err(format!(
                "parsed rows mismatch: {:?} vs {:?}\n{settings_dump}",
                actual_table.rows(),
                expected_table.rows()
            ));
        }
    }

    if let Some(diff) = first_raw_difference(&actual_bytes, &expected_bytes, is_sas7bdat) {
        return Err(format!("{diff}\n{settings_dump}"));
    }

    Ok(format!("identical ({} bytes)", actual_bytes.len()))
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

fn first_raw_difference(
    actual: &[u8],
    expected: &[u8],
    mask_sas7_timestamps: bool,
) -> Option<String> {
    let mut actual_masked = actual.to_vec();
    let mut expected_masked = expected.to_vec();
    if mask_sas7_timestamps {
        mask_sas7bdat_stamps(&mut actual_masked);
        mask_sas7bdat_stamps(&mut expected_masked);
    }
    let shared = actual_masked.len().min(expected_masked.len());
    for offset in 0..shared {
        if actual_masked.get(offset) != expected_masked.get(offset) {
            return Some(format!(
                "byte {offset:#06x} ({offset}): actual {:02x} vs expected {:02x}",
                actual_masked.get(offset).copied().unwrap_or(0),
                expected_masked.get(offset).copied().unwrap_or(0)
            ));
        }
    }
    if actual_masked.len() != expected_masked.len() {
        Some(format!(
            "length {} vs {} after shared prefix ({shared} bytes)",
            actual_masked.len(),
            expected_masked.len()
        ))
    } else {
        None
    }
}

fn mask_sas7bdat_stamps(bytes: &mut [u8]) {
    // Bytes 164..172 and 172..180 are the creation and modification f64 timestamps;
    // bytes 166..=168 (creation) and 174..=176 (modification) plus 290..=303 vary across runs.
    if let Some(s1) = bytes.get_mut(166..=176) {
        s1.fill(0);
    }
    if let Some(s2) = bytes.get_mut(290..=303) {
        s2.fill(0);
    }
}
