//! Content comparison of Excel exports with the Modeler oracle
//! (`oracle/exports/excel_*/exports/*.xlsx`).
//!
//! Zip timestamps differ, so the workbooks are compared by sheet name, each cell's kind,
//! stored value and number format, and by the set of package parts.

#![cfg(feature = "corpus")]

mod support;

use dms_exec::{ExecContext, FileResolver, NodeOutput, Plan};
use dms_stream::Stream;
use dms_strfmt::ArchiveLimits;
use dms_strfmt::fastinfoset::Element;
use roxmltree::Document;
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn excel_exports_match_the_oracle_by_content() {
    let root = support::repo_root();
    let oracle = root.join("oracle").join("exports");
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&oracle)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            path.join("manifest.json").is_file() && name.starts_with("excel_")
        })
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no excel export oracle cases");

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
    let export_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("excel-export")
        .join(name);
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
    let expected_bytes = std::fs::read(&oracle_file).map_err(|error| error.to_string())?;
    let actual = workbook(&actual_bytes).map_err(|error| format!("{error}\n{settings_dump}"))?;
    let expected = workbook(&expected_bytes)?;
    match first_difference(&actual, &expected) {
        None => Ok("identical content".to_string()),
        Some(difference) => Err(format!("{difference}\n{settings_dump}")),
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

struct Workbook {
    /// Part names in zip order.
    parts: Vec<String>,
    sheets: Vec<Sheet>,
}

struct Sheet {
    name: String,
    cells: Vec<Cell>,
}

#[derive(Debug, PartialEq, Eq)]
struct Cell {
    coord: String,
    kind: String,
    value: String,
    format: String,
}

fn workbook(bytes: &[u8]) -> Result<Workbook, String> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let mut parts = Vec::new();
    let mut files = BTreeMap::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|error| error.to_string())?;
        let name = file
            .name()
            .map_err(|error| error.to_string())?
            .trim_start_matches('/')
            .to_string();
        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .map_err(|error| error.to_string())?;
        parts.push(name.clone());
        files.insert(name, data);
    }
    let sheets = read_sheets(&files)?;
    Ok(Workbook { parts, sheets })
}

fn read_sheets(files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<Sheet>, String> {
    let workbook = xml_part(files, "xl/workbook.xml")?;
    let rels = xml_part(files, "xl/_rels/workbook.xml.rels")?;
    let styles = xml_part(files, "xl/styles.xml")?;
    let formats = style_formats(&styles)?;
    let mut targets = HashMap::new();
    for node in rels
        .descendants()
        .filter(|node| node.tag_name().name() == "Relationship")
    {
        let Some(id) = attr(node, "Id") else {
            continue;
        };
        let Some(target) = attr(node, "Target") else {
            continue;
        };
        targets.insert(id.to_string(), target.to_string());
    }
    let shared = files
        .get("xl/sharedStrings.xml")
        .map(|bytes| shared_strings(bytes))
        .transpose()?
        .unwrap_or_default();
    let mut sheets = Vec::new();
    for node in workbook
        .descendants()
        .filter(|node| node.tag_name().name() == "sheet")
    {
        let name = attr(node, "name").unwrap_or("").to_string();
        let id = attr(node, "id").unwrap_or("");
        let target = targets
            .get(id)
            .ok_or_else(|| format!("sheet {name} has no relationship"))?;
        let path = if target.starts_with('/') {
            target.trim_start_matches('/').to_string()
        } else {
            format!("xl/{target}")
        };
        let sheet = xml_part(files, &path)?;
        let cells = sheet_cells(&sheet, &formats, &shared)?;
        sheets.push(Sheet { name, cells });
    }
    Ok(sheets)
}

fn xml_part<'a>(files: &'a BTreeMap<String, Vec<u8>>, name: &str) -> Result<Document<'a>, String> {
    let bytes = files
        .get(name)
        .ok_or_else(|| format!("missing part {name}"))?;
    let text = std::str::from_utf8(bytes).map_err(|error| format!("{name}: {error}"))?;
    Document::parse(text).map_err(|error| format!("{name}: {error}"))
}

fn style_formats(styles: &Document<'_>) -> Result<Vec<String>, String> {
    let mut custom = HashMap::new();
    for node in styles
        .descendants()
        .filter(|node| node.tag_name().name() == "numFmt")
    {
        let Some(id) = attr(node, "numFmtId").and_then(|text| text.parse::<u32>().ok()) else {
            continue;
        };
        custom.insert(id, attr(node, "formatCode").unwrap_or("").to_string());
    }
    let mut formats = Vec::new();
    let Some(cell_xfs) = styles
        .descendants()
        .find(|node| node.tag_name().name() == "cellXfs")
    else {
        return Ok(vec!["General".to_string()]);
    };
    for node in cell_xfs
        .children()
        .filter(|node| node.tag_name().name() == "xf")
    {
        let id = attr(node, "numFmtId")
            .and_then(|text| text.parse::<u32>().ok())
            .unwrap_or(0);
        let code = custom
            .get(&id)
            .cloned()
            .unwrap_or_else(|| builtin_format(id).to_string());
        formats.push(code);
    }
    if formats.is_empty() {
        formats.push("General".to_string());
    }
    Ok(formats)
}

fn builtin_format(id: u32) -> &'static str {
    match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "mm-dd-yy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yy h:mm",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mmss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => "General",
    }
}

fn shared_strings(bytes: &[u8]) -> Result<Vec<String>, String> {
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let doc = Document::parse(text).map_err(|error| error.to_string())?;
    let mut strings = Vec::new();
    for node in doc
        .descendants()
        .filter(|node| node.tag_name().name() == "si")
    {
        let mut value = String::new();
        for text_node in node
            .descendants()
            .filter(|node| node.tag_name().name() == "t")
        {
            value.push_str(text_node.text().unwrap_or(""));
        }
        strings.push(value);
    }
    Ok(strings)
}

fn sheet_cells(
    sheet: &Document<'_>,
    formats: &[String],
    shared: &[String],
) -> Result<Vec<Cell>, String> {
    let mut cells = Vec::new();
    for node in sheet
        .descendants()
        .filter(|node| node.tag_name().name() == "c")
    {
        let coord = attr(node, "r").unwrap_or("").to_string();
        let type_attr = attr(node, "t").unwrap_or("");
        let style = attr(node, "s")
            .and_then(|text| text.parse::<usize>().ok())
            .unwrap_or(0);
        let format = formats
            .get(style)
            .cloned()
            .unwrap_or_else(|| "General".to_string());
        let raw = cell_value(node, type_attr, shared)?;
        let kind = match type_attr {
            "b" => "boolean",
            "inlineStr" | "s" | "str" => "string",
            "e" => "error",
            _ if is_date_format(&format) => "date",
            _ => "number",
        };
        cells.push(Cell {
            coord,
            kind: kind.to_string(),
            value: raw,
            format,
        });
    }
    Ok(cells)
}

fn cell_value(
    node: roxmltree::Node<'_, '_>,
    type_attr: &str,
    shared: &[String],
) -> Result<String, String> {
    match type_attr {
        "inlineStr" => Ok(node
            .descendants()
            .find(|child| child.tag_name().name() == "t")
            .and_then(|child| child.text())
            .unwrap_or("")
            .to_string()),
        "s" => {
            let index = node
                .descendants()
                .find(|child| child.tag_name().name() == "v")
                .and_then(|child| child.text())
                .unwrap_or("")
                .parse::<usize>()
                .map_err(|_| "shared string index is not a number".to_string())?;
            shared
                .get(index)
                .cloned()
                .ok_or_else(|| format!("shared string {index} is missing"))
        }
        _ => Ok(node
            .children()
            .find(|child| child.tag_name().name() == "v")
            .and_then(|child| child.text())
            .unwrap_or("")
            .to_string()),
    }
}

fn is_date_format(code: &str) -> bool {
    match code {
        "mm-dd-yy" | "d-mmm-yy" | "d-mmm" | "mmm-yy" | "h:mm AM/PM" | "h:mm:ss AM/PM" | "h:mm"
        | "h:mm:ss" | "m/d/yy h:mm" | "mm:ss" | "[h]:mm:ss" | "mmss.0" => true,
        _ => format_has_date_token(code),
    }
}

/// True when `code`, outside quoted literals, contains a date or time token.
fn format_has_date_token(code: &str) -> bool {
    let mut quoted = false;
    for c in code.chars() {
        if c == '"' {
            quoted = !quoted;
            continue;
        }
        if !quoted && matches!(c, 'y' | 'd' | 'h' | 's' | 'Y' | 'D' | 'H' | 'S') {
            return true;
        }
    }
    false
}

fn attr<'a>(node: roxmltree::Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|attribute| attribute.name() == name)
        .map(|attribute| attribute.value())
}

fn first_difference(actual: &Workbook, expected: &Workbook) -> Option<String> {
    for name in &expected.parts {
        if !actual.parts.iter().any(|part| part == name) {
            return Some(format!("missing oracle part {name}"));
        }
    }
    if actual.sheets.len() != expected.sheets.len() {
        return Some(format!(
            "sheet count {} vs {}",
            actual.sheets.len(),
            expected.sheets.len()
        ));
    }
    for (actual_sheet, expected_sheet) in actual.sheets.iter().zip(expected.sheets.iter()) {
        if actual_sheet.name != expected_sheet.name {
            return Some(format!(
                "sheet {:?} vs {:?}",
                actual_sheet.name, expected_sheet.name
            ));
        }
        for (actual_cell, expected_cell) in
            actual_sheet.cells.iter().zip(expected_sheet.cells.iter())
        {
            if actual_cell != expected_cell {
                return Some(format!("cell {actual_cell:?} vs {expected_cell:?}"));
            }
        }
        if actual_sheet.cells.len() != expected_sheet.cells.len() {
            return Some(format!(
                "sheet {:?} cell count {} vs {}",
                actual_sheet.name,
                actual_sheet.cells.len(),
                expected_sheet.cells.len()
            ));
        }
    }
    None
}
