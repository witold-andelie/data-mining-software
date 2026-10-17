//! Excel workbook (`.xlsx`) writer, Office Open XML written with the `zip` crate.
//!
//! The package matches the parts IBM SPSS Modeler 19 wrote for
//! `oracle/exports/excel_field_names` and `oracle/exports/excel_no_field_names`
//! (those two workbooks share every part except `xl/worksheets/sheet1.xml`).
//! Zip timestamps are not part of the match. Cell rules, each one visible in those
//! workbooks:
//!
//! - One worksheet, `xl/worksheets/sheet1.xml`. The sheet tab is [`WriteOptions::sheet_name`]
//!   (`Sheet1` in both probes), stored in `xl/workbook.xml` and `docProps/app.xml`.
//! - The first cell is [`WriteOptions::start_column`] / [`WriteOptions::start_row`]
//!   (`A1` in both probes). Further fields move right and records move down.
//! - When [`WriteOptions::field_names`] is set, row 1 at that origin is the field names as
//!   inline strings (`excel_field_names`). Otherwise the first record is the first row
//!   (`excel_no_field_names`).
//! - An integer is a numeric cell with stylesheet index 23, built-in format `0`
//!   (`<v>10</v>`, `<v>-10</v>`). A real is stylesheet index 0, format General, and the
//!   value text is Java's `Double.toString` (`-3.5`, `0.0`, `10.0`, `0.123456789012345`
//!   on column `rv`).
//! - A date is an Excel serial day with one fractional zero and stylesheet index 2,
//!   built-in format `d-mmm-yy`. 1970-01-01 is serial 25569, so `2020-01-15` is `43845.0`
//!   and `1999-12-31` is `36525.0` (column `dt`). Dates before 1900-03-01 subtract one
//!   for Excel's false 1900-02-29; the probes have no such date, so that adjustment is inferred.
//! - A missing date keeps the date style and an empty `<v>` (`id` 7 and `id` 17). A missing
//!   number, a missing string, or an empty string is `<c t="n"><v></v></c>` with no style
//!   (`id` 6 field `s`, `id` 8 fields `grp` and `flg`, `id` 11 fields `iv` and `rv`).
//! - Other strings are inline strings, not a shared-string table. Leading or trailing
//!   whitespace sets `xml:space="preserve"` (`id` 16, `  spaced  `). `&`, `<` and `>`
//!   are escaped.
//! - A tab in a string is deleted (`id` 9, `tab` + tab + `here` becomes `tabhere`). The
//!   other C0 controls except line feed and carriage return are deleted too, because XML 1.0
//!   forbids them; the probes do not contain those characters.
//! - Flag values stay the strings `T` and `F`. There is no boolean cell in either probe.
//! - Time, timestamp and list values are rejected. The probes do not contain them.
//!
//! `xl/styles.xml`, `xl/theme/theme1.xml`, the relationships, `[Content_Types].xml` and
//! `docProps/core.xml` are the fixed SPSS export-facility parts from those oracles (creator
//! `IBM SPSS Export Facility`, core timestamps `2011-08-01`). The stylesheet carries formats
//! `0.0` through 16 decimal places, currency, percent and time even though these two exports
//! only apply General, `0` and `d-mmm-yy`.

use dms_types::{Storage, Table, Value};
use std::fmt;
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Excel's worksheet limits (ECMA-376).
const MAX_COLUMN: u32 = 16_384;
const MAX_ROW: u32 = 1_048_576;

/// `cellXfs` indexes in the embedded `xl/styles.xml`.
const STYLE_GENERAL: u32 = 0;
const STYLE_DATE: u32 = 2;
const STYLE_INTEGER: u32 = 23;

/// Unix day number of 1900-03-01. Excel's serial epoch (1899-12-30, serial of 1970-01-01 = 25569)
/// is one too high before this date because it treats 1900 as a leap year.
const EXCEL_MARCH_1900: i64 = -25_508;
const EXCEL_UNIX_EPOCH: i64 = 25_569;

/// Where the table is written inside the one worksheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOptions {
    /// Worksheet tab name (`worksheetName` in the stream).
    pub sheet_name: String,
    /// 1-based column of the first cell (`A` is 1).
    pub start_column: u32,
    /// 1-based row of the first cell.
    pub start_row: u32,
    /// Write field names in the first row.
    pub field_names: bool,
}

/// A workbook could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XlsxError {
    /// The sheet name or the origin cell is not one Excel accepts.
    Sheet(String),
    /// The table does not fit on the worksheet from that origin.
    Grid(String),
    /// A cell value has no observed Excel form.
    Value(String),
    /// The zip package could not be assembled.
    Zip(String),
}

impl fmt::Display for XlsxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XlsxError::Sheet(message)
            | XlsxError::Grid(message)
            | XlsxError::Value(message)
            | XlsxError::Zip(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for XlsxError {}

/// Writes `table` as an `.xlsx` package.
pub fn write_table(table: &Table, options: &WriteOptions) -> Result<Vec<u8>, XlsxError> {
    validate_sheet_name(&options.sheet_name)?;
    if options.start_column == 0 || options.start_column > MAX_COLUMN {
        return Err(XlsxError::Sheet(format!(
            "start column {} is outside 1..={MAX_COLUMN}",
            options.start_column
        )));
    }
    if options.start_row == 0 || options.start_row > MAX_ROW {
        return Err(XlsxError::Sheet(format!(
            "start row {} is outside 1..={MAX_ROW}",
            options.start_row
        )));
    }
    let sheet = render_sheet(table, options)?;
    let app = app_xml(&options.sheet_name);
    let workbook = workbook_xml(&options.sheet_name);
    let parts: [(&str, &str); 9] = [
        ("_rels/.rels", PACKAGE_RELS),
        ("docProps/app.xml", app.as_str()),
        ("docProps/core.xml", CORE_PROPS),
        ("xl/theme/theme1.xml", THEME_XML),
        ("xl/styles.xml", STYLES_XML),
        ("[Content_Types].xml", CONTENT_TYPES),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
        ("xl/workbook.xml", workbook.as_str()),
        ("xl/worksheets/sheet1.xml", sheet.as_str()),
    ];
    zip_parts(&parts)
}

fn validate_sheet_name(name: &str) -> Result<(), XlsxError> {
    let count = name.chars().count();
    if count == 0 || count > 31 {
        return Err(XlsxError::Sheet(format!(
            "worksheet name {name:?} must be 1 to 31 characters"
        )));
    }
    if name
        .chars()
        .any(|c| c < ' ' || matches!(c, '\\' | '/' | '?' | '*' | '[' | ']' | ':'))
    {
        return Err(XlsxError::Sheet(format!(
            "worksheet name {name:?} contains a character Excel rejects"
        )));
    }
    Ok(())
}

fn render_sheet(table: &Table, options: &WriteOptions) -> Result<String, XlsxError> {
    let fields = table.model().fields();
    let columns = u32::try_from(fields.len()).map_err(|_| {
        XlsxError::Grid(format!("{} fields do not fit on a worksheet", fields.len()))
    })?;
    let header = options.field_names && columns > 0;
    let record_rows = u32::try_from(table.rows().len()).map_err(|_| {
        XlsxError::Grid(format!(
            "{} records do not fit on a worksheet",
            table.rows().len()
        ))
    })?;
    let body_rows = record_rows.saturating_add(u32::from(header));

    let mut out = String::new();
    out.push_str(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">",
    );
    if columns == 0 || body_rows == 0 {
        out.push_str(
            "<sheetViews><sheetView tabSelected=\"1\" workbookViewId=\"0\"></sheetView></sheetViews>\
<sheetFormatPr defaultRowHeight=\"15\"/><sheetData></sheetData>\
<pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/>\
</worksheet>",
        );
        return Ok(out);
    }

    let last_column = options
        .start_column
        .checked_add(columns - 1)
        .filter(|column| *column <= MAX_COLUMN)
        .ok_or_else(|| {
            XlsxError::Grid(format!(
                "{columns} columns starting at {} do not fit",
                options.start_column
            ))
        })?;
    let last_row = options
        .start_row
        .checked_add(body_rows - 1)
        .filter(|row| *row <= MAX_ROW)
        .ok_or_else(|| {
            XlsxError::Grid(format!(
                "{body_rows} rows starting at {} do not fit",
                options.start_row
            ))
        })?;
    let letters = column_letters_range(options.start_column, last_column)?;
    let origin = column_letters(options.start_column)?;
    let end = column_letters(last_column)?;
    out.push_str("<dimension ref=\"");
    out.push_str(&origin);
    out.push_str(&options.start_row.to_string());
    if last_column != options.start_column || last_row != options.start_row {
        out.push(':');
        out.push_str(&end);
        out.push_str(&last_row.to_string());
    }
    out.push_str(
        "\"/><sheetViews><sheetView tabSelected=\"1\" workbookViewId=\"0\"></sheetView></sheetViews>\
<sheetFormatPr defaultRowHeight=\"15\"/><sheetData>",
    );

    if header {
        let cells = fields
            .iter()
            .map(|field| render_header(&field.name))
            .collect::<Vec<_>>();
        out.push_str(&render_row(
            options.start_row,
            options.start_column,
            last_column,
            &letters,
            &cells,
        ));
        out.push('\n');
    }
    let first_record = options.start_row.saturating_add(u32::from(header));
    for (offset, row) in table.rows().iter().enumerate() {
        let Some(row_index) = u32::try_from(offset)
            .ok()
            .and_then(|offset| first_record.checked_add(offset))
        else {
            return Err(XlsxError::Grid(
                "record index does not fit on a worksheet".into(),
            ));
        };
        let mut cells = Vec::with_capacity(fields.len());
        for (field, value) in fields.iter().zip(row.iter()) {
            cells.push(render_value(value, field.storage)?);
        }
        out.push_str(&render_row(
            row_index,
            options.start_column,
            last_column,
            &letters,
            &cells,
        ));
        out.push('\n');
    }
    out.push_str(
        "</sheetData><pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" \
header=\"0.3\" footer=\"0.3\"/></worksheet>",
    );
    Ok(out)
}

fn render_row(
    row: u32,
    first_column: u32,
    last_column: u32,
    letters: &[String],
    cells: &[Rendered],
) -> String {
    let mut out = String::new();
    out.push_str("<row r=\"");
    out.push_str(&row.to_string());
    out.push_str("\" spans=\"");
    out.push_str(&first_column.to_string());
    out.push(':');
    out.push_str(&last_column.to_string());
    out.push_str("\" s=\"0\" outlineLevel=\"0\">");
    for (offset, cell) in cells.iter().enumerate() {
        let Some(letters) = letters.get(offset) else {
            continue;
        };
        out.push_str("<c r=\"");
        out.push_str(letters);
        out.push_str(&row.to_string());
        out.push('"');
        match cell {
            Rendered::Number { style, value } if value.is_empty() && *style == STYLE_GENERAL => {
                out.push_str(" t=\"n\"><v></v></c>");
            }
            Rendered::Number { style, value } => {
                out.push_str(" s=\"");
                out.push_str(&style.to_string());
                out.push_str("\"><v>");
                out.push_str(value);
                out.push_str("</v></c>");
            }
            Rendered::Text { text, preserve } => {
                out.push_str(" t=\"inlineStr\"><is><t");
                if *preserve {
                    out.push_str(" xml:space=\"preserve\"");
                }
                out.push('>');
                out.push_str(text);
                out.push_str("</t></is></c>");
            }
        }
    }
    out.push_str("</row>");
    out
}

fn column_letters_range(first: u32, last: u32) -> Result<Vec<String>, XlsxError> {
    let mut letters = Vec::new();
    let mut column = first;
    while column <= last {
        letters.push(column_letters(column)?);
        column = column.saturating_add(1);
        if column == 0 {
            break;
        }
    }
    Ok(letters)
}

/// 1-based column index to `A` .. `XFD`.
fn column_letters(index: u32) -> Result<String, XlsxError> {
    if index == 0 || index > MAX_COLUMN {
        return Err(XlsxError::Grid(format!(
            "column {index} is outside the sheet"
        )));
    }
    let mut n = index;
    let mut bytes = [0u8; 3];
    let mut len = 0usize;
    while n > 0 {
        n -= 1;
        let rem = u8::try_from(n % 26).unwrap_or(0);
        n /= 26;
        if len >= bytes.len() {
            return Err(XlsxError::Grid(format!(
                "column {index} is outside the sheet"
            )));
        }
        bytes[len] = b'A'.saturating_add(rem);
        len += 1;
    }
    let mut out = String::with_capacity(len);
    for slot in bytes[..len].iter().rev() {
        out.push(char::from(*slot));
    }
    Ok(out)
}

enum Rendered {
    Number { style: u32, value: String },
    Text { text: String, preserve: bool },
}

fn render_header(name: &str) -> Rendered {
    let (text, preserve, _) = excel_text(name);
    Rendered::Text { text, preserve }
}

fn render_value(value: &Value, storage: Storage) -> Result<Rendered, XlsxError> {
    match value {
        Value::Null => {
            if storage == Storage::Date {
                Ok(Rendered::Number {
                    style: STYLE_DATE,
                    value: String::new(),
                })
            } else {
                Ok(Rendered::Number {
                    style: STYLE_GENERAL,
                    value: String::new(),
                })
            }
        }
        Value::Integer(number) => Ok(Rendered::Number {
            style: STYLE_INTEGER,
            value: number.to_string(),
        }),
        Value::Real(number) => Ok(Rendered::Number {
            style: STYLE_GENERAL,
            value: format_excel_real(*number)?,
        }),
        Value::Date(days) => {
            let serial = excel_serial(*days)
                .ok_or_else(|| XlsxError::Value(format!("date day {days} has no Excel serial")))?;
            Ok(Rendered::Number {
                style: STYLE_DATE,
                value: format!("{serial}.0"),
            })
        }
        Value::String(text) => {
            let (text, preserve, empty) = excel_text(text);
            if empty {
                Ok(Rendered::Number {
                    style: STYLE_GENERAL,
                    value: String::new(),
                })
            } else {
                Ok(Rendered::Text { text, preserve })
            }
        }
        Value::Time(_) => Err(XlsxError::Value(
            "time values have no observed Excel export form".into(),
        )),
        Value::Timestamp(_) => Err(XlsxError::Value(
            "timestamp values have no observed Excel export form".into(),
        )),
        Value::List(_) => Err(XlsxError::Value(
            "list values have no observed Excel export form".into(),
        )),
    }
}

/// Excel serial day. `Some` 43845 is 2020-01-15 in both probe workbooks.
fn excel_serial(unix_days: i64) -> Option<i64> {
    let serial = unix_days.checked_add(EXCEL_UNIX_EPOCH)?;
    if unix_days < EXCEL_MARCH_1900 {
        serial.checked_sub(1)
    } else {
        Some(serial)
    }
}

/// Java `Double.toString`, which is what column `rv` stores (`0.0`, `-3.5`, `1.0E7` outside
/// the probes' magnitude).
fn format_excel_real(value: f64) -> Result<String, XlsxError> {
    if !value.is_finite() {
        return Err(XlsxError::Value(format!("real {value} is not finite")));
    }
    let negative = value.is_sign_negative();
    let body = java_double_body(&format!("{}", value.abs()))
        .ok_or_else(|| XlsxError::Value(format!("real {value} has no decimal form")))?;
    if negative {
        Ok(format!("-{body}"))
    } else {
        Ok(body)
    }
}

/// `text` is Rust's shortest non-negative `f64` rendering.
fn java_double_body(text: &str) -> Option<String> {
    let (coeff, exp) = if let Some((coeff, exp)) = text.split_once(['e', 'E']) {
        (coeff, exp.parse::<i32>().ok()?)
    } else {
        (text, 0)
    };
    let mut digits = String::new();
    let mut point = None;
    for c in coeff.chars() {
        match c {
            '.' if point.is_none() => point = Some(digits.len()),
            '0'..='9' => digits.push(c),
            _ => return None,
        }
    }
    if digits.is_empty() {
        return None;
    }
    let point = point.unwrap_or(digits.len());
    let Some(first) = digits.find(|c: char| c != '0') else {
        return Some("0.0".to_string());
    };
    let k = (point as i32)
        .checked_sub(first as i32)?
        .checked_sub(1)?
        .checked_add(exp)?;
    let sig = &digits[first..];
    // Index in `sig` where Rust's own fractional digits begin. `point` is past the
    // last digit when Rust printed an integer (`10000000` for 1e7).
    let frac_in_sig = point.saturating_sub(first);
    if (-3..7).contains(&k) {
        Some(format_plain(sig, k))
    } else {
        Some(format_scientific(sig, frac_in_sig, k))
    }
}

fn format_plain(sig: &str, k: i32) -> String {
    let mut out = String::new();
    if k >= 0 {
        let whole = (k as usize).saturating_add(1);
        if sig.len() <= whole {
            out.push_str(sig);
            out.extend(std::iter::repeat_n('0', whole - sig.len()));
            out.push_str(".0");
        } else {
            out.push_str(&sig[..whole]);
            out.push('.');
            out.push_str(&sig[whole..]);
        }
    } else {
        out.push_str("0.");
        let zeros = usize::try_from((-k - 1).max(0)).unwrap_or(0);
        out.extend(std::iter::repeat_n('0', zeros));
        out.push_str(sig);
    }
    out
}

/// Scientific form. Zeros that Rust printed only because it spelled an integer
/// (`10000000` → `1.0E7`) are dropped. Zeros Rust printed after a decimal point stay,
/// because those are part of the shortest spelling (`1.10E20`).
fn format_scientific(sig: &str, frac_in_sig: usize, k: i32) -> String {
    let trimmed = if frac_in_sig >= sig.len() {
        let trimmed = sig.trim_end_matches('0');
        if trimmed.is_empty() { "0" } else { trimmed }
    } else {
        sig
    };
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return format!("0.0E{k}");
    };
    let rest: String = chars.collect();
    if rest.is_empty() {
        format!("{first}.0E{k}")
    } else {
        format!("{first}.{rest}E{k}")
    }
}

/// `(escaped text, preserve whitespace, empty after control stripping)`.
fn excel_text(raw: &str) -> (String, bool, bool) {
    let mut kept = String::new();
    for c in raw.chars() {
        if keep_cell_char(c) {
            kept.push(c);
        }
    }
    let preserve = kept.chars().next().is_some_and(is_xml_space)
        || kept.chars().next_back().is_some_and(is_xml_space);
    let empty = kept.is_empty();
    let mut escaped = String::with_capacity(kept.len());
    for c in kept.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(c),
        }
    }
    (escaped, preserve, empty)
}

/// Tab is removed (probe `id` 9). Line feed and carriage return stay. Other C0 controls
/// cannot appear in XML 1.0.
fn keep_cell_char(c: char) -> bool {
    match c as u32 {
        0x09 => false,
        0x0A | 0x0D => true,
        code if code < 0x20 => false,
        0xFFFE | 0xFFFF => false,
        _ => true,
    }
}

fn is_xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

fn xml_attr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn app_xml(sheet: &str) -> String {
    let sheet = xml_attr(sheet);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\" xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes\"><Application>Microsoft Excel</Application><DocSecurity>0</DocSecurity><ScaleCrop>false</ScaleCrop><HeadingPairs><vt:vector size=\"2\" baseType=\"variant\"><vt:variant><vt:lpstr>Worksheets</vt:lpstr></vt:variant><vt:variant><vt:i4>1</vt:i4></vt:variant></vt:vector></HeadingPairs><TitlesOfParts><vt:vector size=\"1\" baseType=\"lpstr\"><vt:lpstr>{sheet}</vt:lpstr></vt:vector></TitlesOfParts><Company>IBM</Company><LinksUpToDate>false</LinksUpToDate><SharedDoc>false</SharedDoc><HyperlinksChanged>false</HyperlinksChanged><AppVersion>12.0000</AppVersion></Properties>"
    )
}

fn workbook_xml(sheet: &str) -> String {
    let sheet = xml_attr(sheet);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<workbook xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><fileVersion appName=\"xl\" lastEdited=\"4\" lowestEdited=\"4\" rupBuild=\"4506\"/><workbookPr showObjects=\"all\" updateLinks=\"userSet\" defaultThemeVersion=\"124226\"/><bookViews><workbookView visibility=\"visible\" xWindow=\"480\" yWindow=\"75\" windowWidth=\"18075\" windowHeight=\"12525\" tabRatio=\"600\" firstSheet=\"0\" activeTab=\"0\"/></bookViews><sheets><sheet name=\"{sheet}\" sheetId=\"1\" state=\"visible\" r:id=\"rId1\"/></sheets><calcPr calcId=\"125725\" calcMode=\"auto\" refMode=\"A1\" iterateCount=\"100\"/></workbook>"
    )
}

fn zip_parts(parts: &[(&str, &str)]) -> Result<Vec<u8>, XlsxError> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, contents) in parts {
        writer
            .start_file(*name, options)
            .map_err(|error| XlsxError::Zip(error.to_string()))?;
        writer
            .write_all(contents.as_bytes())
            .map_err(|error| XlsxError::Zip(error.to_string()))?;
    }
    writer
        .finish()
        .map(|cursor| cursor.into_inner())
        .map_err(|error| XlsxError::Zip(error.to_string()))
}

// Parts that do not depend on the table, copied from the excel_* oracles.
const PACKAGE_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override ContentType="application/vnd.openxmlformats-officedocument.theme+xml" PartName="/xl/theme/theme1.xml"/><Override ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml" PartName="/xl/styles.xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml" PartName="/xl/workbook.xml"/><Override ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml" PartName="/docProps/app.xml"/><Override ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml" PartName="/xl/worksheets/sheet1.xml"/><Override ContentType="application/vnd.openxmlformats-package.core-properties+xml" PartName="/docProps/core.xml"/></Types>"#;
const CORE_PROPS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:creator>IBM SPSS Export Facility</dc:creator><cp:lastModifiedBy>IBM SPSS Export Facility</cp:lastModifiedBy><dcterms:created xsi:type="dcterms:W3CDTF">2011-08-01T14:22:18Z</dcterms:created><dcterms:modified xsi:type="dcterms:W3CDTF">2011-08-01T14:23:37Z</dcterms:modified></cp:coreProperties>"#;
const THEME_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office Theme"><a:themeElements><a:clrScheme name="Office"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="1F497D"/></a:dk2><a:lt2><a:srgbClr val="EEECE1"/></a:lt2><a:accent1><a:srgbClr val="4F81BD"/></a:accent1><a:accent2><a:srgbClr val="C0504D"/></a:accent2><a:accent3><a:srgbClr val="9BBB59"/></a:accent3><a:accent4><a:srgbClr val="8064A2"/></a:accent4><a:accent5><a:srgbClr val="4BACC6"/></a:accent5><a:accent6><a:srgbClr val="F79646"/></a:accent6><a:hlink><a:srgbClr val="0000FF"/></a:hlink><a:folHlink><a:srgbClr val="800080"/></a:folHlink></a:clrScheme><a:fontScheme name="Office"><a:majorFont><a:latin typeface="Cambria"/><a:ea typeface=""/><a:cs typeface=""/><a:font script="Jpan" typeface="ＭＳ Ｐゴシック"/><a:font script="Hang" typeface="맑은 고딕"/><a:font script="Hans" typeface="宋体"/><a:font script="Hant" typeface="新細明體"/><a:font script="Arab" typeface="Times New Roman"/><a:font script="Hebr" typeface="Times New Roman"/><a:font script="Thai" typeface="Tahoma"/><a:font script="Ethi" typeface="Nyala"/><a:font script="Beng" typeface="Vrinda"/><a:font script="Gujr" typeface="Shruti"/><a:font script="Khmr" typeface="MoolBoran"/><a:font script="Knda" typeface="Tunga"/><a:font script="Guru" typeface="Raavi"/><a:font script="Cans" typeface="Euphemia"/><a:font script="Cher" typeface="Plantagenet Cherokee"/><a:font script="Yiii" typeface="Microsoft Yi Baiti"/><a:font script="Tibt" typeface="Microsoft Himalaya"/><a:font script="Thaa" typeface="MV Boli"/><a:font script="Deva" typeface="Mangal"/><a:font script="Telu" typeface="Gautami"/><a:font script="Taml" typeface="Latha"/><a:font script="Syrc" typeface="Estrangelo Edessa"/><a:font script="Orya" typeface="Kalinga"/><a:font script="Mlym" typeface="Kartika"/><a:font script="Laoo" typeface="DokChampa"/><a:font script="Sinh" typeface="Iskoola Pota"/><a:font script="Mong" typeface="Mongolian Baiti"/><a:font script="Viet" typeface="Times New Roman"/><a:font script="Uigh" typeface="Microsoft Uighur"/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/><a:font script="Jpan" typeface="ＭＳ Ｐゴシック"/><a:font script="Hang" typeface="맑은 고딕"/><a:font script="Hans" typeface="宋体"/><a:font script="Hant" typeface="新細明體"/><a:font script="Arab" typeface="Arial"/><a:font script="Hebr" typeface="Arial"/><a:font script="Thai" typeface="Tahoma"/><a:font script="Ethi" typeface="Nyala"/><a:font script="Beng" typeface="Vrinda"/><a:font script="Gujr" typeface="Shruti"/><a:font script="Khmr" typeface="DaunPenh"/><a:font script="Knda" typeface="Tunga"/><a:font script="Guru" typeface="Raavi"/><a:font script="Cans" typeface="Euphemia"/><a:font script="Cher" typeface="Plantagenet Cherokee"/><a:font script="Yiii" typeface="Microsoft Yi Baiti"/><a:font script="Tibt" typeface="Microsoft Himalaya"/><a:font script="Thaa" typeface="MV Boli"/><a:font script="Deva" typeface="Mangal"/><a:font script="Telu" typeface="Gautami"/><a:font script="Taml" typeface="Latha"/><a:font script="Syrc" typeface="Estrangelo Edessa"/><a:font script="Orya" typeface="Kalinga"/><a:font script="Mlym" typeface="Kartika"/><a:font script="Laoo" typeface="DokChampa"/><a:font script="Sinh" typeface="Iskoola Pota"/><a:font script="Mong" typeface="Mongolian Baiti"/><a:font script="Viet" typeface="Arial"/><a:font script="Uigh" typeface="Microsoft Uighur"/></a:minorFont></a:fontScheme><a:fmtScheme name="Office"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:gradFill rotWithShape="1"><a:gsLst><a:gs pos="0"><a:schemeClr val="phClr"><a:tint val="50000"/><a:satMod val="300000"/></a:schemeClr></a:gs><a:gs pos="35000"><a:schemeClr val="phClr"><a:tint val="37000"/><a:satMod val="300000"/></a:schemeClr></a:gs><a:gs pos="100000"><a:schemeClr val="phClr"><a:tint val="15000"/><a:satMod val="350000"/></a:schemeClr></a:gs></a:gsLst><a:lin ang="16200000" scaled="1"/></a:gradFill><a:gradFill rotWithShape="1"><a:gsLst><a:gs pos="0"><a:schemeClr val="phClr"><a:shade val="51000"/><a:satMod val="130000"/></a:schemeClr></a:gs><a:gs pos="80000"><a:schemeClr val="phClr"><a:shade val="93000"/><a:satMod val="130000"/></a:schemeClr></a:gs><a:gs pos="100000"><a:schemeClr val="phClr"><a:shade val="94000"/><a:satMod val="135000"/></a:schemeClr></a:gs></a:gsLst><a:lin ang="16200000" scaled="0"/></a:gradFill></a:fillStyleLst><a:lnStyleLst><a:ln w="9525" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"><a:shade val="95000"/><a:satMod val="105000"/></a:schemeClr></a:solidFill><a:prstDash val="solid"/></a:ln><a:ln w="25400" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln><a:ln w="38100" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst><a:outerShdw blurRad="40000" dist="20000" dir="5400000" rotWithShape="0"><a:srgbClr val="000000"><a:alpha val="38000"/></a:srgbClr></a:outerShdw></a:effectLst></a:effectStyle><a:effectStyle><a:effectLst><a:outerShdw blurRad="40000" dist="23000" dir="5400000" rotWithShape="0"><a:srgbClr val="000000"><a:alpha val="35000"/></a:srgbClr></a:outerShdw></a:effectLst></a:effectStyle><a:effectStyle><a:effectLst><a:outerShdw blurRad="40000" dist="23000" dir="5400000" rotWithShape="0"><a:srgbClr val="000000"><a:alpha val="35000"/></a:srgbClr></a:outerShdw></a:effectLst><a:scene3d><a:camera prst="orthographicFront"><a:rot lat="0" lon="0" rev="0"/></a:camera><a:lightRig rig="threePt" dir="t"><a:rot lat="0" lon="0" rev="1200000"/></a:lightRig></a:scene3d><a:sp3d><a:bevelT w="63500" h="25400"/></a:sp3d></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:gradFill rotWithShape="1"><a:gsLst><a:gs pos="0"><a:schemeClr val="phClr"><a:tint val="40000"/><a:satMod val="350000"/></a:schemeClr></a:gs><a:gs pos="40000"><a:schemeClr val="phClr"><a:tint val="45000"/><a:shade val="99000"/><a:satMod val="350000"/></a:schemeClr></a:gs><a:gs pos="100000"><a:schemeClr val="phClr"><a:shade val="20000"/><a:satMod val="255000"/></a:schemeClr></a:gs></a:gsLst><a:path path="circle"><a:fillToRect l="50000" t="-80000" r="50000" b="180000"/></a:path></a:gradFill><a:gradFill rotWithShape="1"><a:gsLst><a:gs pos="0"><a:schemeClr val="phClr"><a:tint val="80000"/><a:satMod val="300000"/></a:schemeClr></a:gs><a:gs pos="100000"><a:schemeClr val="phClr"><a:shade val="30000"/><a:satMod val="200000"/></a:schemeClr></a:gs></a:gsLst><a:path path="circle"><a:fillToRect l="50000" t="50000" r="50000" b="50000"/></a:path></a:gradFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>"#;
const STYLES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <numFmts count="17">
	    <numFmt numFmtId="164" formatCode="&quot;$&quot;#,##0.00"/>
        <numFmt numFmtId="165" formatCode="0.0"/>
        <numFmt numFmtId="166" formatCode="0.00"/>
        <numFmt numFmtId="167" formatCode="0.000"/>
        <numFmt numFmtId="168" formatCode="0.0000"/>
        <numFmt numFmtId="169" formatCode="0.00000"/>
        <numFmt numFmtId="170" formatCode="0.000000"/>
        <numFmt numFmtId="171" formatCode="0.0000000"/>
        <numFmt numFmtId="172" formatCode="0.00000000"/>
        <numFmt numFmtId="173" formatCode="0.000000000"/>
        <numFmt numFmtId="174" formatCode="0.0000000000"/>
        <numFmt numFmtId="175" formatCode="0.00000000000"/>
        <numFmt numFmtId="176" formatCode="0.000000000000"/>
        <numFmt numFmtId="177" formatCode="0.0000000000000"/>
        <numFmt numFmtId="178" formatCode="0.00000000000000"/>
        <numFmt numFmtId="179" formatCode="0.000000000000000"/>
        <numFmt numFmtId="180" formatCode="0.0000000000000000"/>
    </numFmts>
  <fonts count="1">
    <font><sz val="11"/><color theme="1"/><name val="Calibri"/><family val="2"/><scheme val="minor"/></font>
  </fonts>
  <fills count="2">
    <fill><patternFill patternType="none"/></fill>
    <fill><patternFill patternType="gray125"/></fill>
  </fills><borders count="1"><border>
  <left/>
  <right/>
  <top/>
  <bottom/>
  <diagonal/>
  </border>
  </borders>
  <cellStyleXfs count="1">
    <xf numFmtId="0" fontId="0" fillId="0" borderId="0"/>    
  </cellStyleXfs>
  <cellXfs count="24">
        <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" /> 
        <xf numFmtId="10" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1" />
        <xf numFmtId="15" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1" /> 
        <xf numFmtId="19" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1" /> 
        <xf numFmtId="22" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1" />                 
        <xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1" /> 
		<xf numFmtId="2" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1" /> 
        <xf numFmtId="165" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
        <xf numFmtId="166" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
        <xf numFmtId="167" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
        <xf numFmtId="168" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
        <xf numFmtId="169" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
        <xf numFmtId="170" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
        <xf numFmtId="171" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
        <xf numFmtId="172" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
        <xf numFmtId="173" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
        <xf numFmtId="174" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
        <xf numFmtId="175" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
        <xf numFmtId="176" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
        <xf numFmtId="177" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
        <xf numFmtId="178" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
        <xf numFmtId="179" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
        <xf numFmtId="180" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
        <xf numFmtId="1" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
  </cellXfs>
  <cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
  <dxfs count="0"/>
  <tableStyles count="0" defaultTableStyle="TableStyleMedium9" defaultPivotStyle="PivotStyleLight16"/>
</styleSheet>
"#;
const WORKBOOK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="styles.xml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Id="rId3"/><Relationship Target="theme/theme1.xml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Id="rId2"/><Relationship Target="worksheets/sheet1.xml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Id="rId1"/></Relationships>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use dms_types::calendar;
    use dms_types::{DataModel, Field, Table, Value};
    use std::io::Read;

    #[test]
    fn serials_match_the_probe_dates_and_the_excel_epoch() {
        let jan15 = calendar::days_from_civil(2020, 1, 15).unwrap();
        assert_eq!(excel_serial(jan15), Some(43_845));
        let dec31 = calendar::days_from_civil(1999, 12, 31).unwrap();
        assert_eq!(excel_serial(dec31), Some(36_525));
        assert_eq!(excel_serial(0), Some(25_569));
        // Inferred: Excel's false leap day shifts serials before 1900-03-01.
        assert_eq!(
            excel_serial(calendar::days_from_civil(1900, 1, 1).unwrap()),
            Some(1)
        );
        assert_eq!(
            excel_serial(calendar::days_from_civil(1900, 2, 28).unwrap()),
            Some(59)
        );
        assert_eq!(
            excel_serial(calendar::days_from_civil(1900, 3, 1).unwrap()),
            Some(61)
        );
    }

    #[test]
    fn reals_use_java_double_text() {
        assert_eq!(format_excel_real(0.0).unwrap(), "0.0");
        assert_eq!(format_excel_real(-0.0).unwrap(), "-0.0");
        assert_eq!(format_excel_real(-3.5).unwrap(), "-3.5");
        assert_eq!(format_excel_real(10.0).unwrap(), "10.0");
        assert_eq!(format_excel_real(1.25).unwrap(), "1.25");
        assert_eq!(format_excel_real(-2.25).unwrap(), "-2.25");
        assert_eq!(format_excel_real(1010.0).unwrap(), "1010.0");
        assert_eq!(
            format_excel_real(0.123_456_789_012_345).unwrap(),
            "0.123456789012345"
        );
        assert_eq!(format_excel_real(1e-7).unwrap(), "1.0E-7");
        assert_eq!(format_excel_real(1e7).unwrap(), "1.0E7");
        assert_eq!(format_excel_real(1.23e7).unwrap(), "1.23E7");
        assert_eq!(format_excel_real(0.001).unwrap(), "0.001");
        assert_eq!(format_excel_real(0.0001).unwrap(), "1.0E-4");
        assert_eq!(format_excel_real(9_999_999.0).unwrap(), "9999999.0");
    }

    #[test]
    fn tab_is_removed_and_edge_spaces_are_preserved() {
        let (text, preserve, empty) = excel_text("tab\there");
        assert_eq!(text, "tabhere");
        assert!(!preserve);
        assert!(!empty);
        let (text, preserve, empty) = excel_text("  spaced  ");
        assert_eq!(text, "  spaced  ");
        assert!(preserve);
        assert!(!empty);
        let (text, _, empty) = excel_text("");
        assert!(text.is_empty());
        assert!(empty);
        let (text, _, empty) = excel_text("a&b<c>d");
        assert_eq!(text, "a&amp;b&lt;c&gt;d");
        assert!(!empty);
    }

    #[test]
    fn sheet_contains_header_integer_stripped_tab_and_missing_date() {
        let model = DataModel::new(vec![
            Field::new("iv", Storage::Integer),
            Field::new("s", Storage::String),
            Field::new("dt", Storage::Date),
        ])
        .unwrap();
        let table = Table::new(
            model,
            vec![vec![
                Value::Integer(10),
                Value::string("tab\there"),
                Value::Null,
            ]],
        )
        .unwrap();
        let bytes = write_table(
            &table,
            &WriteOptions {
                sheet_name: "Sheet1".to_string(),
                start_column: 1,
                start_row: 1,
                field_names: true,
            },
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert!(zip.by_name("xl/theme/theme1.xml").is_ok());
        assert!(zip.by_name("xl/styles.xml").is_ok());
        let mut sheet = String::new();
        zip.by_name("xl/worksheets/sheet1.xml")
            .unwrap()
            .read_to_string(&mut sheet)
            .unwrap();
        assert!(sheet.contains("<t>iv</t>"), "{sheet}");
        assert!(sheet.contains("s=\"23\"><v>10</v>"), "{sheet}");
        assert!(sheet.contains("<t>tabhere</t>"), "{sheet}");
        assert!(sheet.contains("s=\"2\"><v></v>"), "{sheet}");
        assert!(sheet.contains("dimension ref=\"A1:C2\""), "{sheet}");
    }
}
