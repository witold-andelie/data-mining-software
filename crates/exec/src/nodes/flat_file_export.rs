//! Flat File export (`TextFileExportNode`): writes the input table to a delimited text file and
//! passes that table through unchanged.
//!
//! Implemented settings (element names under the node's `properties`, the file form of the
//! `outputfilenode` scripting properties):
//!
//! | element | scripting property | accepted values |
//! |---|---|---|
//! | `exportFileName` | `full_filename` | path; the file written is [`ExecContext::export_path`] |
//! | `fileEncoding` | `encoding` | `UTF-8` (no byte-order mark) |
//! | `writeMode` | `write_mode` | `overwrite` (default), `append` |
//! | `includeFieldNames` | `inc_field_names` | `true` (default) or `false` |
//! | `fieldSeparator` | `delimit_mode` | `comma` (default), `tab`, `space`, `other` |
//! | `delimiterChars` | `other_delimiter` | the separator when `fieldSeparator` is `other` |
//! | `quoteSymbolicValues` | `quote_mode` | `doubleQuotes` (default), `singleQuotes`, empty/`none` |
//! | `userQuoteMark` | `other_quote` | the quote text when `quoteSymbolicValues` is `other` |
//! | `realDecimalSymbol` | `decimal_symbol` | `period` (default), `comma`, `default` (the stream's symbol) |
//! | `newlineAfterRecord` | `use_newline_after_records` | `true` (a CRLF after every record, including the last) |
//!
//! `executionMode` must be `export_data` when it is saved. Publish settings
//! (`autoGenerateImportNode`, `publishMetadata`, `outputFileName`, `publishParameters`,
//! `publishUseParameters`) do not change the file and are ignored.
//!
//! Rules, each one visible in a probe export under `oracle/exports/`:
//!
//! - Records end with CRLF, including the last record, and the file has no BOM (`flat_default`).
//! - The header row is the field names separated by the delimiter, not quoted unless a name
//!   contains the delimiter, the quote or a line break (`flat_default`, `flat_no_field_names`).
//! - String values are always wrapped in the quote character; numbers, dates, times and
//!   timestamps are not. A system-missing value is an empty field. An empty string is a quoted
//!   empty field (`flat_default` row 6 `""` against row 11's missing `iv`).
//! - A quote character inside a value is written as itself. Modeler does not double it
//!   (a value `a"b` is exported as `"a"b"`).
//! - The probe cells `say ""hi""` and `q ""and"" semi;colon,comma` in `export_main.csv` are
//!   `say hi` and `q and semi;colon,comma` in `flat_default` (and in the JSON and XML oracles).
//!   Modeler's variable-file `pairDiscard` import removed those quote characters. This node
//!   writes the string it receives and does not remove `"` from a value.
//! - Reals use [`format_real`] with the execution context's decimal places (the oracle runs with
//!   15, `flat_default`: `-3.500000000000000`). Integers have no decimal part (`10`, `-10`).
//! - Dates print as `YYYY-MM-DD` (`flat_default`).
//! - `fieldSeparator` `tab` is a tab character (`flat_tab`); `other` uses `delimiterChars`
//!   (`flat_semicolon`, the character `;`).
//! - A comma decimal symbol is written with `,`, and a comma field separator is then written as
//!   `;` so the two marks do not collide (`flat_decimal_comma`). The saved `fieldSeparator` stays
//!   `comma`.
//! - `quoteSymbolicValues` empty writes no quotes at all (`flat_quote_none`); `singleQuotes`
//!   wraps strings in `'` (`flat_quote_single`); `doubleQuotes` is the default and matches an
//!   explicit double-quote save (`flat_quote_double`, `flat_default`).
//! - `writeMode` `overwrite` replaces the file (`flat_overwrite`, identical to `flat_default`).
//!   `append` adds records to an existing non-empty file and does not repeat the header; a
//!   missing or empty file is written as an overwrite. No probe saves `append`.
//!
//! `newlineAfterRecord` `false` is rejected: the probes only save `true`, and the record
//! separator Modeler uses without newlines was not observed.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::format_real;
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::{Table, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

/// Separator and quoting for one export.
struct Layout {
    separator: String,
    /// Empty when symbolic values are not quoted.
    quote: String,
    decimal: char,
    header: bool,
    append: bool,
    decimals: usize,
}

struct FlatFileExport {
    path: String,
    encoding_utf8: bool,
    separator_kind: SeparatorKind,
    other_separator: String,
    quote_kind: QuoteKind,
    user_quote: String,
    decimal_kind: DecimalKind,
    header: bool,
    append: bool,
}

enum SeparatorKind {
    Comma,
    Tab,
    Space,
    Other,
}

enum QuoteKind {
    Double,
    Single,
    None,
    Other,
}

enum DecimalKind {
    Period,
    Comma,
    StreamDefault,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let path = settings::text(s, "exportFileName")
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            OperatorError::Settings("Flat File: exportFileName is missing".to_string())
        })?;

    let encoding = settings::text(s, "fileEncoding").unwrap_or_else(|| "UTF-8".to_string());
    let encoding_utf8 = matches!(
        encoding.trim().to_ascii_lowercase().as_str(),
        "utf-8" | "utf8" | "utf_8"
    );
    if !encoding_utf8 {
        return Err(OperatorError::Settings(format!(
            "Flat File: fileEncoding {encoding:?} is not implemented (only UTF-8)"
        )));
    }

    match settings::text(s, "executionMode").as_deref().map(str::trim) {
        None | Some("") | Some("export_data") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Flat File: executionMode {other:?} is not implemented"
            )));
        }
    }

    let newline = settings::flag(s, "newlineAfterRecord")?.unwrap_or(true);
    if !newline {
        return Err(OperatorError::Settings(
            "Flat File: newlineAfterRecord false is not implemented".into(),
        ));
    }

    let separator_kind = match settings::text(s, "fieldSeparator")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("comma") => SeparatorKind::Comma,
        Some("tab") => SeparatorKind::Tab,
        Some("space") => SeparatorKind::Space,
        Some("other") => SeparatorKind::Other,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Flat File: unknown fieldSeparator {other:?}"
            )));
        }
    };
    let other_separator = settings::text(s, "delimiterChars").unwrap_or_default();
    if matches!(separator_kind, SeparatorKind::Other) && other_separator.is_empty() {
        return Err(OperatorError::Settings(format!(
            "Flat File: fieldSeparator is other but delimiterChars is empty ({})",
            child_summary(s)
        )));
    }

    let quote_kind = match settings::text(s, "quoteSymbolicValues")
        .as_deref()
        .map(str::trim)
    {
        None | Some("doubleQuotes") => QuoteKind::Double,
        Some("singleQuotes") => QuoteKind::Single,
        Some("" | "none") => QuoteKind::None,
        Some("other") => QuoteKind::Other,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Flat File: unknown quoteSymbolicValues {other:?}"
            )));
        }
    };
    let user_quote = settings::text(s, "userQuoteMark").unwrap_or_default();
    if matches!(quote_kind, QuoteKind::Other) && user_quote.is_empty() {
        return Err(OperatorError::Settings(
            "Flat File: quoteSymbolicValues is other but userQuoteMark is empty".into(),
        ));
    }

    let decimal_kind = match settings::text(s, "realDecimalSymbol")
        .or_else(|| settings::text(s, "decimalSymbol"))
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("period") => DecimalKind::Period,
        Some("comma") => DecimalKind::Comma,
        Some("default") => DecimalKind::StreamDefault,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Flat File: unknown realDecimalSymbol {other:?}"
            )));
        }
    };

    let header = settings::flag(s, "includeFieldNames")?.unwrap_or(true);
    let append = match settings::text(s, "writeMode").as_deref().map(str::trim) {
        None | Some("") | Some("overwrite") | Some("Overwrite") => false,
        Some("append") | Some("Append") => true,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Flat File: unknown writeMode {other:?}"
            )));
        }
    };

    Ok(Box::new(FlatFileExport {
        path,
        encoding_utf8,
        separator_kind,
        other_separator,
        quote_kind,
        user_quote,
        decimal_kind,
        header,
        append,
    }))
}

fn child_summary(settings: Option<&Element>) -> String {
    let Some(settings) = settings else {
        return "no settings".to_string();
    };
    let mut parts = Vec::new();
    for child in settings.child_elements() {
        let text = child.text();
        let shown: String = text.chars().take(40).collect();
        parts.push(format!("{}={shown:?}", child.name.local));
    }
    parts.join(", ")
}

impl FlatFileExport {
    fn layout(&self, context: &ExecContext) -> Result<Layout, OperatorError> {
        let mut separator = match self.separator_kind {
            SeparatorKind::Comma => ",".to_string(),
            SeparatorKind::Tab => "\t".to_string(),
            SeparatorKind::Space => " ".to_string(),
            SeparatorKind::Other => self.other_separator.clone(),
        };
        let decimal = match self.decimal_kind {
            DecimalKind::Period => '.',
            DecimalKind::Comma => ',',
            DecimalKind::StreamDefault => {
                match settings::text(context.stream_properties.as_deref(), "realDecimalSymbol")
                    .as_deref()
                    .map(str::trim)
                {
                    Some("comma") => ',',
                    _ => '.',
                }
            }
        };
        // flat_decimal_comma: the saved separator is still "comma", and the file uses ';'.
        if decimal == ',' && separator == "," {
            separator = ";".to_string();
        }
        let quote = match self.quote_kind {
            QuoteKind::Double => "\"".to_string(),
            QuoteKind::Single => "'".to_string(),
            QuoteKind::None => String::new(),
            QuoteKind::Other => self.user_quote.clone(),
        };
        Ok(Layout {
            separator,
            quote,
            decimal,
            header: self.header,
            append: self.append,
            decimals: context.decimal_places(),
        })
    }
}

impl Operator for FlatFileExport {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Flat File export needs one input".into()))?;
        if !self.encoding_utf8 {
            return Err(OperatorError::Settings(
                "Flat File: fileEncoding is not UTF-8".into(),
            ));
        }
        let layout = self.layout(context)?;
        let path = context.export_path(&self.path);
        let bytes = render(input, &layout);
        let skip_header = layout.append && file_nonempty(&path)?;
        write_file(&path, &bytes, layout.append, skip_header && layout.header)?;
        Ok(input.as_ref().clone())
    }
}

/// True when `path` exists and contains at least one byte.
fn file_nonempty(path: &Path) -> Result<bool, OperatorError> {
    match std::fs::metadata(path) {
        Ok(meta) => Ok(meta.is_file() && meta.len() > 0),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(OperatorError::Io(format!("{}: {error}", path.display()))),
    }
}

fn write_file(
    path: &Path,
    bytes: &[u8],
    append: bool,
    skip_header: bool,
) -> Result<(), OperatorError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| OperatorError::Io(format!("{}: {error}", parent.display())))?;
    }
    let mut options = OpenOptions::new();
    if append {
        options.create(true).append(true);
    } else {
        options.create(true).write(true).truncate(true);
    }
    let mut file = options
        .open(path)
        .map_err(|error| OperatorError::Io(format!("{}: {error}", path.display())))?;
    let payload = if skip_header {
        // The header is the first CRLF-terminated record.
        bytes
            .windows(2)
            .position(|pair| pair == b"\r\n")
            .and_then(|at| bytes.get(at.saturating_add(2)..))
            .unwrap_or(bytes)
    } else {
        bytes
    };
    file.write_all(payload)
        .map_err(|error| OperatorError::Io(format!("{}: {error}", path.display())))?;
    Ok(())
}

fn render(table: &Table, layout: &Layout) -> Vec<u8> {
    let mut out = Vec::new();
    if layout.header {
        let names: Vec<String> = table
            .model()
            .fields()
            .iter()
            .map(|field| quote_if_needed(&field.name, layout))
            .collect();
        push_record(&mut out, &names, &layout.separator);
    }
    for row in table.rows() {
        let cells: Vec<String> = row.iter().map(|value| format_cell(value, layout)).collect();
        push_record(&mut out, &cells, &layout.separator);
    }
    out
}

fn push_record(out: &mut Vec<u8>, cells: &[String], separator: &str) {
    for (index, cell) in cells.iter().enumerate() {
        if index > 0 {
            out.extend_from_slice(separator.as_bytes());
        }
        out.extend_from_slice(cell.as_bytes());
    }
    out.extend_from_slice(b"\r\n");
}

fn format_cell(value: &Value, layout: &Layout) -> String {
    match value {
        Value::Null => String::new(),
        Value::Integer(value) => value.to_string(),
        Value::Real(value) => format_decimal(*value, layout.decimals, layout.decimal),
        Value::String(text) => wrap(text, &layout.quote),
        Value::Date(_) | Value::Time(_) | Value::Timestamp(_) => value.to_string(),
        Value::List(_) => wrap(&value.to_string(), &layout.quote),
    }
}

fn format_decimal(value: f64, decimals: usize, decimal: char) -> String {
    let text = format_real(value, decimals);
    if decimal == '.' {
        text
    } else {
        text.replace('.', &decimal.to_string())
    }
}

/// Header names stay bare unless they would be split or would look quoted.
fn quote_if_needed(name: &str, layout: &Layout) -> String {
    let needs = name.contains(['\n', '\r'])
        || (!layout.separator.is_empty() && name.contains(&layout.separator))
        || (!layout.quote.is_empty() && name.contains(&layout.quote));
    if needs {
        wrap(name, &layout.quote)
    } else {
        name.to_string()
    }
}

/// Wraps `text` in `mark` without doubling marks that occur inside it.
fn wrap(text: &str, mark: &str) -> String {
    if mark.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len().saturating_add(mark.len().saturating_mul(2)));
    out.push_str(mark);
    out.push_str(text);
    out.push_str(mark);
    out
}
