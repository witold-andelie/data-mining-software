//! XML export (`xml_export!spss.xml_data`): writes the input table as a single XML document and
//! passes that table through unchanged.
//!
//! File settings are `Parameter` entries in the extension attachment (`node@properties`,
//! `data/0001.dat`), read when the node runs. `build` rejects an `executionMode` other than
//! `export_data`. A parameter this node does not implement is rejected with
//! [`OperatorError::Settings`] at run time, which is the first moment the attachment bytes are
//! available.
//!
//! Implemented parameters (the `xml_default` probe saves each of these):
//!
//! | parameter | accepted values |
//! |---|---|
//! | `xmlExport.full_filename` | path; the file written is [`ExecContext::export_path`] |
//! | `xmlExport.encoding` | `utf_8` (also `utf-8` / `utf8`). The declaration always says `UTF-8` |
//! | `xmlExport.mode` | `single` |
//! | `xmlExport.use_xml_schema` | `false` |
//! | `xmlExport.full_schema_filename` | empty |
//! | `xmlExport.records` | empty (the default `<records>` / `<record>` layout) |
//! | `xmlExport.map` | empty list |
//! | `xmlExport.recurse` | `false` |
//! | `xmlExport.scanner_type` | empty |
//! | `xmlExport.selected_root` | empty |
//! | `xmlExport.generate_import` | `true` or `false`; does not change the file |
//! | `xmlPreview.preview_xml` | ignored (a preview cache) |
//! | `xmlPreview.display_attributes` | `true` or `false`; the export elements carry no attributes |
//!
//! `pasw.annotation_*` parameters are labels and are ignored. Publish settings on the node
//! element do not change the file. `use_xml_schema` true, a non-empty `records` path, a non-empty
//! `map`, `recurse` true, a non-empty schema path, scanner or selected root, a mode other than
//! `single`, and any encoding other than UTF-8 are rejected.
//!
//! Rules from `oracle/exports/xml_default/exports/xml_default.xml`:
//!
//! - UTF-8, no byte-order mark, LF line endings, and no newline after the final `</records>`.
//! - The document starts `<?xml version="1.0" encoding="UTF-8"?><records><record>` and then a
//!   newline. Each field is one line `<name>value</name>`. Each record ends with `</record>` and
//!   a newline. The next record opens with `<record>` at the start of the line.
//! - A missing value and an empty string are both an empty element (`<iv></iv>`, `<s></s>`), not
//!   a self-closing tag. See the row with `id` 11 (missing `iv` and `rv`) and the row with `id` 6
//!   (empty `s`).
//! - Integers are written with no decimal part (`10`, `-10`). Reals always have 6 decimal places
//!   (`-3.500000`, `0.000000`, `0.123457` from `0.123456789012345`). The stream saves 15 decimal
//!   places and the export still has 6.
//! - Dates are `YYYY-MM-DD` (`2020-01-15`). A missing date is an empty element (the row with
//!   `id` 7).
//! - `&`, `<` and `>` in text are escaped. A tab is written as a raw tab (the row with `id` 9).
//! - A `"` inside a string is kept. The probe source cells `say ""hi""` and
//!   `q ""and"" semi;colon,comma` are `say hi` and `q and semi;colon,comma` in this oracle file,
//!   because Modeler's variable-file import dropped those quote characters before the export ran.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::format_real;
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Table, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

const XML_REAL_PLACES: usize = 6;

const KNOWN_PARAMETERS: &[&str] = &[
    "xmlExport.full_filename",
    "xmlExport.encoding",
    "xmlExport.mode",
    "xmlExport.use_xml_schema",
    "xmlExport.full_schema_filename",
    "xmlExport.records",
    "xmlExport.map",
    "xmlExport.recurse",
    "xmlExport.scanner_type",
    "xmlExport.selected_root",
    "xmlExport.generate_import",
    "xmlPreview.preview_xml",
    "xmlPreview.display_attributes",
];

struct XmlExport {
    attachment: Option<String>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    match settings::text(node.settings.as_ref(), "executionMode")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("export_data") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "XML export: executionMode {other:?} is not implemented"
            )));
        }
    }
    Ok(Box::new(XmlExport {
        attachment: node.properties_attachment.clone(),
    }))
}

impl Operator for XmlExport {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("XML export needs one input".into()))?;
        let params = load_parameters(self.attachment.as_deref(), context, "XML export")?;
        let path = xml_settings(&params)?;
        let text = render(input)?;
        write_bytes(&context.export_path(&path), text.as_bytes())?;
        Ok(input.as_ref().clone())
    }
}

fn xml_settings(params: &HashMap<String, Param>) -> Result<String, OperatorError> {
    reject_unknown(params, KNOWN_PARAMETERS, "XML export")?;
    let path = text(params, "xmlExport.full_filename")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            OperatorError::Settings("XML export: xmlExport.full_filename is missing".into())
        })?;
    match text(params, "xmlExport.encoding").as_deref().map(str::trim) {
        None | Some("") => {}
        Some(value)
            if matches!(
                value.to_ascii_lowercase().as_str(),
                "utf_8" | "utf-8" | "utf8"
            ) => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "XML export: encoding {other:?} is not implemented (only UTF-8)"
            )));
        }
    }
    match text(params, "xmlExport.mode").as_deref().map(str::trim) {
        None | Some("") | Some("single") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "XML export: mode {other:?} is not implemented"
            )));
        }
    }
    if flag(params, "xmlExport.use_xml_schema")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "XML export: use_xml_schema is not implemented".into(),
        ));
    }
    if text(params, "xmlExport.full_schema_filename").is_some_and(|value| !value.trim().is_empty())
    {
        return Err(OperatorError::Settings(
            "XML export: full_schema_filename is not implemented".into(),
        ));
    }
    if text(params, "xmlExport.records").is_some_and(|value| !value.trim().is_empty()) {
        return Err(OperatorError::Settings(
            "XML export: a records path is not implemented".into(),
        ));
    }
    match params.get("xmlExport.map") {
        None => {}
        Some(Param::List(values)) if values.is_empty() => {}
        Some(_) => {
            return Err(OperatorError::Settings(
                "XML export: map is not implemented".into(),
            ));
        }
    }
    if flag(params, "xmlExport.recurse")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "XML export: recurse is not implemented".into(),
        ));
    }
    if text(params, "xmlExport.scanner_type").is_some_and(|value| !value.trim().is_empty()) {
        return Err(OperatorError::Settings(
            "XML export: scanner_type is not implemented".into(),
        ));
    }
    if text(params, "xmlExport.selected_root").is_some_and(|value| !value.trim().is_empty()) {
        return Err(OperatorError::Settings(
            "XML export: selected_root is not implemented".into(),
        ));
    }
    let _ = flag(params, "xmlExport.generate_import")?;
    let _ = flag(params, "xmlPreview.display_attributes")?;
    Ok(path)
}

fn render(table: &Table) -> Result<String, OperatorError> {
    let fields = table.model().fields();
    for field in fields {
        if !xml_name_ok(&field.name) {
            return Err(OperatorError::Data(format!(
                "XML export: field name {:?} is not an XML name",
                field.name
            )));
        }
    }
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?><records>");
    for row in table.rows() {
        out.push_str("<record>\n");
        for (index, field) in fields.iter().enumerate() {
            let value = row.get(index).unwrap_or(&Value::Null);
            out.push('<');
            out.push_str(&field.name);
            out.push('>');
            out.push_str(&xml_text(value)?);
            out.push_str("</");
            out.push_str(&field.name);
            out.push_str(">\n");
        }
        out.push_str("</record>\n");
    }
    out.push_str("</records>");
    Ok(out)
}

fn xml_text(value: &Value) -> Result<String, OperatorError> {
    match value {
        Value::Null => Ok(String::new()),
        Value::Integer(number) => Ok(number.to_string()),
        Value::Real(number) => Ok(format_real(*number, XML_REAL_PLACES)),
        Value::String(text) => xml_escape(text),
        Value::Date(_) | Value::Time(_) | Value::Timestamp(_) | Value::List(_) => {
            xml_escape(&value.to_string())
        }
    }
}

fn xml_escape(text: &str) -> Result<String, OperatorError> {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if xml_char(c) => out.push(c),
            c => {
                return Err(OperatorError::Data(format!(
                    "XML export: character U+{:04X} cannot be written",
                    c as u32
                )));
            }
        }
    }
    Ok(out)
}

fn xml_char(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'
    )
}

fn xml_name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if xml_name_start(first) => chars.all(xml_name_char),
        _ => false,
    }
}

fn xml_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn xml_name_char(c: char) -> bool {
    xml_name_start(c) || c.is_ascii_digit() || c == '-' || c == '.'
}

enum Param {
    Value(String),
    List(Vec<String>),
}

fn load_parameters(
    attachment: Option<&str>,
    context: &ExecContext,
    label: &str,
) -> Result<HashMap<String, Param>, OperatorError> {
    let member = attachment.ok_or_else(|| {
        OperatorError::Settings(format!("{label}: the node has no parameters attachment"))
    })?;
    let bytes = context.attachments.get(member).ok_or_else(|| {
        OperatorError::Io(format!(
            "{label}: the stream file has no attachment {member}"
        ))
    })?;
    let text = std::str::from_utf8(bytes)
        .map_err(|_| OperatorError::Data(format!("{label}: {member} is not UTF-8 text")))?;
    parse_parameters(text)
        .map_err(|error| OperatorError::Data(format!("{label}: {member}: {error}")))
}

fn parse_parameters(text: &str) -> Result<HashMap<String, Param>, String> {
    let document = roxmltree::Document::parse(text).map_err(|error| error.to_string())?;
    let mut values = HashMap::new();
    for element in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "Parameter")
    {
        let Some(name) = element.attribute("name") else {
            continue;
        };
        let parameter = match element.attribute("value") {
            Some(value) => Param::Value(value.to_string()),
            None => Param::List(
                element
                    .descendants()
                    .filter(|node| node.is_element() && node.tag_name().name() == "Value")
                    .filter_map(|node| node.attribute("value"))
                    .map(str::to_string)
                    .collect(),
            ),
        };
        values.insert(name.to_string(), parameter);
    }
    Ok(values)
}

fn text(params: &HashMap<String, Param>, name: &str) -> Option<String> {
    match params.get(name) {
        Some(Param::Value(value)) => Some(value.clone()),
        _ => None,
    }
}

fn flag(params: &HashMap<String, Param>, name: &str) -> Result<Option<bool>, OperatorError> {
    match text(params, name).as_deref().map(str::trim) {
        None => Ok(None),
        Some("true") => Ok(Some(true)),
        Some("false") => Ok(Some(false)),
        Some(other) => Err(OperatorError::Settings(format!(
            "parameter {name}: {other:?} is not true or false"
        ))),
    }
}

fn reject_unknown(
    params: &HashMap<String, Param>,
    known: &[&str],
    label: &str,
) -> Result<(), OperatorError> {
    for name in params.keys() {
        if name.starts_with("pasw.annotation_") || known.contains(&name.as_str()) {
            continue;
        }
        return Err(OperatorError::Settings(format!(
            "{label}: unknown parameter {name}"
        )));
    }
    Ok(())
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), OperatorError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| OperatorError::Io(format!("{}: {error}", parent.display())))?;
    }
    std::fs::write(path, bytes)
        .map_err(|error| OperatorError::Io(format!("{}: {error}", path.display())))
}
