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

// Reader: Office Open XML (`.xlsx`) as tables, the way Modeler's Excel source node reads the
// probe workbooks `corpus/sources/source_book.xlsx`, `source_book_1904.xlsx`
// (`oracle/sources/excel_*`, `xtask/oracle/probes/source_streams.py`) and
// `corpus/excel_more/excel_more.xlsx` (`oracle/excel_more/*`, `excel_more_streams.py`).
//
// Evidence (which oracle shows which rule):
// - A sheet with a `<row>` element without cells is a `GAP:` error. Excel writes no such
//   element for an empty row, and Modeler reads those sheets inconsistently: the same block
//   at B3:I10 reads 7 typed records without them (`more_off_noempty`) but 5 all-string records
//   with two empty `<row>`s above it (`more_offset`, `sources/excel_sheet_index`); explicit
//   ranges there read nothing (`more_explicit_*`, `sources/excel_explicit_a1`); blank rows
//   move to the end with the last row repeated (`more_blank2_return`,
//   `sources/excel_blank_return`).
// - Sheets, order and names from `xl/workbook.xml` (`excel_sheet_*` by 0-based index and by
//   name). Day serials stay numbers (`excel_dates_1900`/`1904` read the same values), so
//   `date1904` does not change table values.
// - `FirstNonBlank` is the bounding box of non-blank cells, wherever it starts
//   (`more_off5` C5, `more_offa3` A3, `more_offc1` C1, `more_off_noempty` B3).
// - Shared and inline strings; numbers (`t="n"`), booleans (`t="b"` → 1/0), cached formula
//   values, error cells (`t="e"` → null). An absent or empty cell is `""` in a string column
//   and null otherwise (`excel_default` id 2).
// - Every comma of a text cell reads as a period (`more_commas`: `a,b` → `a.b`, `1,5` → `1.5`,
//   `3,000.5` → `3.000.5`; `excel_default` `comma,` → `comma.`). A field name with a comma
//   has no oracle and is a `GAP:` error.
// - Styles (`cellXfs` → `numFmtId`, built-in plus custom `numFmts`): Text `@` forces strings
//   (`excel_text_format`: `12` → `"12"`, `0012` kept); time `hh:mm:ss` and timestamp
//   `yyyy-mm-dd hh:mm:ss` read as times of day (`excel_dates_*`: `0.5` → `12:00:00`,
//   `serial+.5` → `12:00:00`); date `yyyy-mm-dd` stays numeric (`43890`).
// - Field names: the first row when `fieldNamesIncluded` (`excel_default`), else `C1…Cn` and
//   the first row is data (`excel_no_headers`). Empty/absent names generate `C<relative
//   column>` (`excel_header_names`: `C3`, `C7`); duplicates get ` (1)`, … (`dup (1)`); a
//   numeric name prints with 6 decimals (`17` → `17.000000`, `more_explicit_numeric`
//   `-10` → `-10.000000`).
// - `ExplicitRange` in A1 or R1C1 notation reads that rectangle, also past the used range
//   (`more_explicit_numeric` C4:E9, `more_explicit_a3_beyond` A3:C12).
// - `blankRows`: `stopReading` ends at the first blank row (`more_blank_noempty_stop`,
//   `more_blank2_noempty_stop`); `returnBlankRows` keeps blank rows in place, null in numeric
//   columns and `""` in string ones (`more_blank_noempty_return`, `more_blank2_noempty_return`,
//   `more_blank3_noempty_return`: a blank row right after the field names is kept).
// - `scanLineCount` (default 200) is the string-vs-numeric horizon over data rows
//   (`excel_scan_1` saved 200 vs `excel_scan_250`, 205 data rows: `late_text` numeric with
//   null beyond 200 vs string; `late_real` real in both since `2.5` promotes the column;
//   `late_numeric` all-null in the first 200 still numeric).
// - Only `.xlsx` (`v2007`) is read; `.xls`, other `excel_file_type` values and named ranges
//   stay `GAP:` errors (`excel_named_range` has no oracle: Modeler drops the connection).
//
// Limits (like the writer's): at most 16384 columns, 1048576 rows, 1000 sheets, 1000 zip
// members, 32 MiB per XML part, 8M grid cells, 1M shared strings, 10k styles, 512 XML depth
// (via `roxmltree`). Malformed packages fail closed with [`XlsxError`].
use std::collections::{BTreeMap, HashMap};

/// Worksheet limits (ECMA-376, same as the writer).
const MAX_SHEETS: usize = 1000;
/// Zip members in one package.
const MAX_ZIP_MEMBERS: usize = 1000;
/// Bytes of one decompressed XML part.
const MAX_XML_BYTES: u64 = 32 * 1024 * 1024;
/// Cells of one sheet grid or one read range.
const MAX_GRID_CELLS: u64 = 8_000_000;
/// Shared-string entries of one package.
const MAX_SHARED_STRINGS: usize = 1_000_000;
/// Style (`cellXfs`) entries of one package.
const MAX_STYLES: usize = 10_000;

/// How blank rows read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlankRows {
    /// Skip entirely blank rows (`blankRows` `stopReading`, `excel_blank_stop`).
    StopReading,
    /// Keep them as all-null rows (`blankRows` `returnBlankRows`, `excel_blank_return`).
    ReturnBlankRows,
}

/// Which worksheet to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SheetSelector {
    /// 0-based `worksheetIndex` (`excel_sheet_index` 1 → `Offset Data`).
    Index(usize),
    /// Exact `worksheetName` (`excel_sheet_name` `Offset Data`).
    Name(String),
}

/// Which rectangle to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeSelector {
    /// Bounding box of non-blank cells (`dataRange` `firstNonBlank`).
    FirstNonBlank,
    /// Inclusive `rangeStart`..`rangeEnd` in A1 or R1C1 (`dataRange` `explicitRange`,
    /// `excel_explicit_a1` `B3:I12`, `excel_explicit_r1c1` `R3C2:R12C9`).
    Explicit { start: (u32, u32), end: (u32, u32) },
}

/// What rectangle of which sheet becomes the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcelOptions {
    pub sheet: SheetSelector,
    /// `fieldNamesIncluded`: first row are headers (`excel_default`) vs data with `C1…`
    /// names (`excel_no_headers`).
    pub field_names: bool,
    pub range: RangeSelector,
    pub blank_rows: BlankRows,
    /// `scanLineCount` string-vs-numeric horizon (`excel_scan_1` 200 vs `excel_scan_250` 250).
    pub scan_lines: usize,
}

impl Default for ExcelOptions {
    fn default() -> Self {
        ExcelOptions {
            sheet: SheetSelector::Index(0),
            field_names: true,
            range: RangeSelector::FirstNonBlank,
            blank_rows: BlankRows::StopReading,
            scan_lines: 200,
        }
    }
}

/// 1-based `(column, row)`. Column `A` is 1, row 1 is the first row.
pub fn parse_a1(text: &str) -> Result<(u32, u32), XlsxError> {
    let invalid = || XlsxError::Grid(format!("{text:?} is not an A1 cell"));
    let text = text.trim();
    if text.is_empty() {
        return Err(invalid());
    }
    // Optional `$` absolutes (`$B$3`, `B$3`, `$B3`), as in defined names.
    let bytes = text.as_bytes();
    let mut index = 0usize;
    let mut column: u32 = 0;
    let mut letters = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'$' {
            index += 1;
            continue;
        }
        if byte.is_ascii_alphabetic() {
            if letters >= 3 {
                return Err(invalid());
            }
            let value = u32::from(byte.to_ascii_uppercase().saturating_sub(b'A')).saturating_add(1);
            column = column
                .checked_mul(26)
                .and_then(|next| next.checked_add(value))
                .filter(|next| *next <= MAX_COLUMN)
                .ok_or_else(invalid)?;
            letters += 1;
            index += 1;
        } else {
            break;
        }
    }
    if letters == 0 || column == 0 {
        return Err(invalid());
    }
    let Some(rest) = text.get(index..) else {
        return Err(invalid());
    };
    let rest = rest.strip_prefix('$').unwrap_or(rest);
    if rest.is_empty() || rest.len() > 7 || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let row: u32 = rest.parse().map_err(|_| invalid())?;
    if row == 0 || row > MAX_ROW {
        return Err(invalid());
    }
    Ok((column, row))
}

/// 1-based `(column, row)` in `R<row>C<col>` notation (`R3C2` is `B3`).
pub fn parse_r1c1(text: &str) -> Result<(u32, u32), XlsxError> {
    let invalid = || XlsxError::Grid(format!("{text:?} is not an R1C1 cell"));
    let text = text.trim();
    if text.is_empty() {
        return Err(invalid());
    }
    // Absolute only (`R3C2`); relative (`R[3]C[2]`) is rejected.
    if text.contains('[') || text.contains(']') || text.contains('$') {
        return Err(invalid());
    }
    let upper = text.to_ascii_uppercase();
    let Some(after_r) = upper.strip_prefix('R') else {
        return Err(invalid());
    };
    let Some((row_text, col_text)) = after_r.split_once('C') else {
        return Err(invalid());
    };
    if row_text.is_empty() || col_text.is_empty() {
        return Err(invalid());
    }
    if row_text.len() > 7 || col_text.len() > 5 {
        return Err(invalid());
    }
    if !row_text.bytes().all(|b| b.is_ascii_digit())
        || !col_text.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid());
    }
    let row: u32 = row_text.parse().map_err(|_| invalid())?;
    let col: u32 = col_text.parse().map_err(|_| invalid())?;
    if row == 0 || row > MAX_ROW || col == 0 || col > MAX_COLUMN {
        return Err(invalid());
    }
    Ok((col, row))
}

/// Either notation (`B3` or `R3C2`).
pub fn parse_range_bound(text: &str) -> Result<(u32, u32), XlsxError> {
    if let Ok(cell) = parse_a1(text) {
        return Ok(cell);
    }
    parse_r1c1(text)
}

/// Unix days of an Excel 1900-system serial day. `None` for serial 60 (the false 1900-02-29)
/// and out-of-range serials.
pub fn serial_1900_to_unix_days(serial: i64) -> Option<i64> {
    if serial == 60 {
        return None;
    }
    if !(0..=5_000_000).contains(&serial) {
        return None;
    }
    if serial < 60 {
        serial.checked_sub(25_569)?.checked_add(1)
    } else {
        serial.checked_sub(25_569)
    }
}

/// Unix days of an Excel 1904-system serial day (`date1904`, epoch 1904-01-01).
pub fn serial_1904_to_unix_days(serial: i64) -> Option<i64> {
    if !(0..=5_000_000).contains(&serial) {
        return None;
    }
    let base = dms_types::calendar::days_from_civil(1904, 1, 1)?;
    base.checked_add(serial)
}

/// Microseconds since midnight of an Excel serial fraction (`0.5` → `12:00:00`,
/// `excel_dates_1900/1904` clock/stamp columns). `None` for non-finite or out-of-range.
pub fn serial_fraction_to_micros(serial: f64) -> Option<i64> {
    if !serial.is_finite() {
        return None;
    }
    let fraction = serial.rem_euclid(1.0);
    let micros = (fraction * dms_types::calendar::MICROS_PER_DAY as f64).round();
    if micros < 0.0 || micros >= dms_types::calendar::MICROS_PER_DAY as f64 {
        return None;
    }
    let micros = micros as i64;
    if (0..dms_types::calendar::MICROS_PER_DAY).contains(&micros) {
        Some(micros)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumKind {
    General,
    Text,
    Date,
    Time,
    Timestamp,
}

fn classify_builtin(num_fmt: u32) -> NumKind {
    match num_fmt {
        49 => NumKind::Text,
        14 | 15 | 16 | 17 | 27 | 28 | 29 | 30 | 31 | 32 | 33 | 34 | 35 | 36 | 50 | 51 | 52 | 53
        | 54 | 55 | 56 | 57 | 58 => NumKind::Date,
        18 | 19 | 20 | 21 | 45 | 46 | 47 => NumKind::Time,
        22 => NumKind::Timestamp,
        _ => NumKind::General,
    }
}

/// Custom `formatCode` (`yyyy-mm-dd`, `hh:mm:ss`, `yyyy-mm-dd hh:mm:ss`, `@`).
fn classify_custom(code: &str) -> NumKind {
    if code.contains('@') {
        return NumKind::Text;
    }
    // Skip quoted literals (`"h"` etc.) so they do not fake date/time letters.
    let mut plain = String::with_capacity(code.len());
    let mut quoted = false;
    for c in code.chars() {
        match c {
            '"' => quoted = !quoted,
            _ if quoted => {}
            _ => plain.push(c),
        }
    }
    let lower = plain.to_ascii_lowercase();
    let has_date = lower.contains('y') || lower.contains('d');
    let has_time = lower.contains('h') || lower.contains('s');
    match (has_date, has_time) {
        (true, true) => NumKind::Timestamp,
        (true, false) => NumKind::Date,
        (false, true) => NumKind::Time,
        (false, false) => NumKind::General,
    }
}

#[derive(Debug, Clone)]
struct RawCell {
    style: u32,
    kind: RawKind,
}

#[derive(Debug, Clone)]
enum RawKind {
    Number(String),
    Shared(usize),
    Inline(String),
    Boolean(bool),
    Error,
    Empty,
}

fn child_text<'a>(parent: roxmltree::Node<'a, '_>, name: &str) -> Option<String> {
    let mut out = String::new();
    let mut found = false;
    for child in parent
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name)
    {
        found = true;
        for text in child.descendants().filter(|n| n.is_text()) {
            if let Some(text) = text.text() {
                out.push_str(text);
            }
        }
    }
    if found { Some(out) } else { None }
}

fn inline_text(cell: roxmltree::Node<'_, '_>) -> Result<String, XlsxError> {
    let mut out = String::new();
    let mut seen = false;
    for is in cell
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "is")
    {
        seen = true;
        for t in is
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "t")
        {
            for text in t.children().filter(|n| n.is_text()) {
                if let Some(text) = text.text() {
                    out.push_str(text);
                }
            }
        }
    }
    if seen {
        Ok(out)
    } else {
        Err(XlsxError::Value("inlineStr cell without <is>".into()))
    }
}

#[allow(
    clippy::type_complexity,
    reason = "grid positions are two small tuples"
)]
fn parse_cell(
    node: roxmltree::Node<'_, '_>,
    shared_len: usize,
) -> Result<Option<((u32, u32), RawCell)>, XlsxError> {
    let bad = |what: &str| XlsxError::Value(format!("sheet cell: {what}"));
    let address = node.attribute("r").ok_or_else(|| bad("cell without r"))?;
    let (col, row) = parse_a1(address).map_err(|_| bad("cell r is not a cell"))?;
    let style = match node.attribute("s") {
        None => 0,
        Some(text) => text
            .trim()
            .parse::<u32>()
            .map_err(|_| bad("cell s is not a style index"))?,
    };
    let kind_attr = node.attribute("t").unwrap_or("n");
    // A formula keeps its cached `<v>`; the `<f>` itself is not evaluated.
    let formula = node
        .children()
        .any(|n| n.is_element() && n.tag_name().name() == "f");
    let value_text = child_text(node, "v");
    let kind = match kind_attr {
        "s" => {
            let text = value_text.unwrap_or_default();
            let trimmed = text.trim();
            if trimmed.is_empty() && !formula {
                RawKind::Empty
            } else {
                let index: usize = trimmed
                    .parse()
                    .map_err(|_| bad("shared string index is not a number"))?;
                if index >= shared_len {
                    return Err(bad("shared string index is out of range"));
                }
                RawKind::Shared(index)
            }
        }
        "inlineStr" => {
            let text = inline_text(node)?;
            if text.is_empty() {
                RawKind::Empty
            } else {
                RawKind::Inline(text)
            }
        }
        "b" => {
            let text = value_text.unwrap_or_default();
            let trimmed = text.trim();
            if trimmed.is_empty() && !formula {
                RawKind::Empty
            } else {
                match trimmed {
                    "1" | "true" | "TRUE" => RawKind::Boolean(true),
                    "0" | "false" | "FALSE" => RawKind::Boolean(false),
                    _ => return Err(bad("boolean cell is not 0 or 1")),
                }
            }
        }
        "e" => RawKind::Error,
        "str" => match value_text {
            None => RawKind::Empty,
            Some(text) if text.is_empty() => RawKind::Empty,
            Some(text) => RawKind::Inline(text),
        },
        "n" => {
            // A formula without `t` reads its cached `<v>` as a number.
            match value_text {
                None => RawKind::Empty,
                Some(text) if text.trim().is_empty() => RawKind::Empty,
                Some(text) => RawKind::Number(text.trim().to_string()),
            }
        }
        _ => return Err(bad("cell t is not supported")),
    };
    Ok(Some(((col, row), RawCell { style, kind })))
}

fn read_part(
    zip: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>,
    name: &str,
) -> Result<String, XlsxError> {
    let file = zip
        .by_name(name)
        .map_err(|error| XlsxError::Zip(format!("{name}: {error}")))?;
    if file.size() > MAX_XML_BYTES {
        return Err(XlsxError::Zip(format!("{name} exceeds the size limit")));
    }
    let mut text = String::new();
    use std::io::Read as _;
    file.take(MAX_XML_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|error| XlsxError::Zip(format!("{name}: {error}")))?;
    if text.len() as u64 > MAX_XML_BYTES {
        return Err(XlsxError::Zip(format!("{name} exceeds the size limit")));
    }
    Ok(text)
}

fn parse_document<'a>(text: &'a str, name: &str) -> Result<roxmltree::Document<'a>, XlsxError> {
    roxmltree::Document::parse(text).map_err(|error| XlsxError::Zip(format!("{name}: {error}")))
}

fn parse_styles(text: &str) -> Result<Vec<NumKind>, XlsxError> {
    let bad = |what: &str| XlsxError::Value(format!("styles: {what}"));
    let document = parse_document(text, "xl/styles.xml")?;
    let root = document.root_element();
    if root.tag_name().name() != "styleSheet" {
        return Err(bad("root is not styleSheet"));
    }
    let mut customs: HashMap<u32, String> = HashMap::new();
    for num_fmts in root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "numFmts")
    {
        for num_fmt in num_fmts
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "numFmt")
        {
            let Some(id_text) = num_fmt.attribute("numFmtId") else {
                continue;
            };
            let Some(code) = num_fmt.attribute("formatCode") else {
                continue;
            };
            let Ok(id) = id_text.trim().parse::<u32>() else {
                continue;
            };
            if customs.len() >= MAX_STYLES {
                return Err(bad("too many number formats"));
            }
            customs.insert(id, code.to_string());
        }
    }
    let mut kinds = Vec::new();
    for cell_xfs in root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "cellXfs")
    {
        for xf in cell_xfs
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "xf")
        {
            if kinds.len() >= MAX_STYLES {
                return Err(bad("too many cell formats"));
            }
            let num_fmt = match xf.attribute("numFmtId") {
                None => 0,
                Some(text) => text
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| bad("xf numFmtId is not a number"))?,
            };
            let apply = xf.attribute("applyNumberFormat").unwrap_or("1");
            if apply == "0" || apply.eq_ignore_ascii_case("false") {
                kinds.push(NumKind::General);
                continue;
            }
            if num_fmt < 164 {
                kinds.push(classify_builtin(num_fmt));
            } else if let Some(code) = customs.get(&num_fmt) {
                kinds.push(classify_custom(code));
            } else {
                // Unknown custom id without a definition: treat as general, never guess dates.
                kinds.push(NumKind::General);
            }
        }
    }
    Ok(kinds)
}

fn parse_shared_strings(text: &str) -> Result<Vec<String>, XlsxError> {
    let bad = |what: &str| XlsxError::Value(format!("sharedStrings: {what}"));
    let document = parse_document(text, "xl/sharedStrings.xml")?;
    let root = document.root_element();
    if root.tag_name().name() != "sst" {
        return Err(bad("root is not sst"));
    }
    let mut out = Vec::new();
    for si in root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "si")
    {
        if out.len() >= MAX_SHARED_STRINGS {
            return Err(bad("too many shared strings"));
        }
        let mut text_out = String::new();
        for t in si
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "t")
        {
            for child in t.children().filter(|n| n.is_text()) {
                if let Some(part) = child.text() {
                    text_out.push_str(part);
                }
            }
        }
        out.push(text_out);
    }
    Ok(out)
}

struct SheetGrid {
    name: String,
    cells: BTreeMap<(u32, u32), RawCell>,
    /// The sheet has a `<row>` element without cells. Excel writes none for empty rows, and
    /// Modeler reads such sheets inconsistently (`excel_more/more_offset`: rows past the
    /// dimension's height dropped and every column a string; `more_blank2_return`: blank rows
    /// moved to the end and the last row repeated), so they are refused.
    empty_row_elements: bool,
}

fn parse_dimension(text: &str) -> Option<((u32, u32), (u32, u32))> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Some((first, second)) = text.split_once(':') {
        let start = parse_a1(first).ok()?;
        let end = parse_a1(second).ok()?;
        Some((start, end))
    } else {
        let cell = parse_a1(text).ok()?;
        Some((cell, cell))
    }
}

fn parse_sheet(name: String, text: &str, shared_len: usize) -> Result<SheetGrid, XlsxError> {
    let bad = |what: String| XlsxError::Value(format!("sheet {name}: {what}"));
    let document = parse_document(text, "sheet").map_err(|e| bad(e.to_string()))?;
    let root = document.root_element();
    if root.tag_name().name() != "worksheet" {
        return Err(bad("root is not worksheet".into()));
    }
    // `<dimension>` is parsed to fail closed on malformed references; the data rectangle
    // comes from the cells themselves (`FirstNonBlank` bounding box).
    for child in root.children().filter(|n| n.is_element()) {
        if child.tag_name().name() == "dimension"
            && let Some(reference) = child.attribute("ref")
        {
            let _ = parse_dimension(reference);
        }
    }
    let mut cells: BTreeMap<(u32, u32), RawCell> = BTreeMap::new();
    let mut empty_row_elements = false;
    for sheet_data in root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "sheetData")
    {
        for row in sheet_data
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "row")
        {
            if let Some(r_text) = row.attribute("r") {
                let parsed: u32 = r_text
                    .trim()
                    .parse()
                    .map_err(|_| bad("row r is not a number".into()))?;
                if parsed == 0 || parsed > MAX_ROW {
                    return Err(bad("row r is outside the sheet".into()));
                }
            }
            if !row
                .children()
                .any(|n| n.is_element() && n.tag_name().name() == "c")
            {
                empty_row_elements = true;
            }
            for cell in row
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "c")
            {
                let Some(((col, row), raw)) = parse_cell(cell, shared_len)? else {
                    continue;
                };
                if col == 0 || col > MAX_COLUMN || row == 0 || row > MAX_ROW {
                    return Err(bad("cell r is outside the sheet".into()));
                }
                if cells.len() as u64 >= MAX_GRID_CELLS {
                    return Err(bad("sheet exceeds the cell limit".into()));
                }
                cells.insert((col, row), raw);
            }
        }
    }
    Ok(SheetGrid {
        name,
        cells,
        empty_row_elements,
    })
}

#[derive(Debug, Clone, PartialEq)]
enum Typed {
    Integer(i64),
    Real(f64),
    Text(String),
    Time(i64),
}

fn parse_integer_text(text: &str) -> Option<i64> {
    let digits = text.strip_prefix(['-', '+']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.strip_prefix('+').unwrap_or(text).parse().ok()
}

fn parse_real_text(text: &str) -> Option<f64> {
    if text.is_empty()
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E'))
        || !text.bytes().any(|b| b.is_ascii_digit())
    {
        return None;
    }
    text.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn typed_of(
    kind: &RawKind,
    style_kind: NumKind,
    shared: &[String],
) -> Result<Option<Typed>, XlsxError> {
    let bad = |what: &str| XlsxError::Value(format!("cell value: {what}"));
    match kind {
        RawKind::Empty | RawKind::Error => Ok(None),
        RawKind::Boolean(flag) => {
            if style_kind == NumKind::Text {
                Ok(Some(Typed::Text(if *flag {
                    "1".into()
                } else {
                    "0".into()
                })))
            } else {
                Ok(Some(Typed::Integer(if *flag { 1 } else { 0 })))
            }
        }
        RawKind::Shared(index) => {
            let Some(text) = shared.get(*index) else {
                return Err(bad("shared string index is out of range"));
            };
            if text.is_empty() {
                Ok(None)
            } else {
                // A string with a time style stays a string (no oracle settles it).
                Ok(Some(Typed::Text(text.clone())))
            }
        }
        RawKind::Inline(text) => {
            if text.is_empty() {
                Ok(None)
            } else {
                // A string with a time style stays a string (no oracle settles it).
                Ok(Some(Typed::Text(text.clone())))
            }
        }
        RawKind::Number(raw) => {
            if style_kind == NumKind::Text {
                if raw.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(Typed::Text(raw.clone())))
                }
            } else if style_kind == NumKind::Time || style_kind == NumKind::Timestamp {
                let serial: f64 = raw
                    .parse()
                    .map_err(|_| bad("time serial is not a number"))?;
                let Some(micros) = serial_fraction_to_micros(serial) else {
                    return Err(bad("time serial is out of range"));
                };
                Ok(Some(Typed::Time(micros)))
            } else if let Some(int) = parse_integer_text(raw) {
                Ok(Some(Typed::Integer(int)))
            } else if let Some(real) = parse_real_text(raw) {
                Ok(Some(Typed::Real(real)))
            } else {
                Err(bad("numeric cell is not a number"))
            }
        }
    }
}

#[allow(
    clippy::type_complexity,
    reason = "grid positions are two small tuples"
)]
fn bounding_box(
    cells: &BTreeMap<(u32, u32), RawCell>,
    shared: &[String],
    styles: &[NumKind],
) -> Result<Option<((u32, u32), (u32, u32))>, XlsxError> {
    let mut min_col = u32::MAX;
    let mut min_row = u32::MAX;
    let mut max_col = 0u32;
    let mut max_row = 0u32;
    let mut any = false;
    for ((col, row), cell) in cells {
        let style_kind = styles
            .get(cell.style as usize)
            .copied()
            .unwrap_or(NumKind::General);
        let typed = typed_of(&cell.kind, style_kind, shared)?;
        // Empty inline strings and empty numerics are blank for the origin.
        if typed.is_none() {
            continue;
        }
        any = true;
        min_col = min_col.min(*col);
        min_row = min_row.min(*row);
        max_col = max_col.max(*col);
        max_row = max_row.max(*row);
    }
    if any {
        Ok(Some(((min_col, min_row), (max_col, max_row))))
    } else {
        Ok(None)
    }
}

/// Reads `bytes` (an `.xlsx` package) as a [`Table`] following `options`.
pub fn read_table(bytes: &[u8], options: &ExcelOptions) -> Result<Table, XlsxError> {
    use dms_types::{DataModel, Field, Measurement, Table, Value};

    if bytes.len() as u64 > MAX_XML_BYTES * 4 {
        return Err(XlsxError::Zip("workbook exceeds the size limit".into()));
    }
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec()))
        .map_err(|error| XlsxError::Zip(error.to_string()))?;
    if zip.len() > MAX_ZIP_MEMBERS {
        return Err(XlsxError::Zip("workbook has too many parts".into()));
    }
    // Collect member names first (bounded), so later borrows do not clash.
    let mut names: Vec<String> = Vec::new();
    for index in 0..zip.len() {
        let file = zip
            .by_index(index)
            .map_err(|error| XlsxError::Zip(error.to_string()))?;
        let name = file
            .name()
            .map_err(|error| XlsxError::Zip(error.to_string()))?
            .to_string();
        names.push(name);
    }
    let has = |suffix: &str| names.iter().any(|n| n == suffix);
    if !has("xl/workbook.xml") {
        return Err(XlsxError::Zip("xl/workbook.xml is missing".into()));
    }
    let workbook_text = read_part(&mut zip, "xl/workbook.xml")?;
    let workbook_document = parse_document(&workbook_text, "xl/workbook.xml")?;
    let workbook_root = workbook_document.root_element();
    let mut sheet_names: Vec<String> = Vec::new();
    let mut sheet_rids: Vec<Option<String>> = Vec::new();
    if let Some(sheets) = workbook_root
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "sheets")
    {
        for sheet in sheets
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "sheet")
        {
            if sheet_names.len() >= MAX_SHEETS {
                return Err(XlsxError::Value("too many sheets".into()));
            }
            let Some(name) = sheet.attribute("name") else {
                return Err(XlsxError::Value("sheet without a name".into()));
            };
            sheet_names.push(name.to_string());
            sheet_rids.push(sheet.attribute("r:id").map(str::to_string));
        }
    }
    if sheet_names.is_empty() {
        return Err(XlsxError::Value("workbook has no sheets".into()));
    }
    // Relationship targets for sheet parts.
    let mut rel_targets: HashMap<String, String> = HashMap::new();
    if has("xl/_rels/workbook.xml.rels") {
        let rels_text = read_part(&mut zip, "xl/_rels/workbook.xml.rels")?;
        let rels_document = parse_document(&rels_text, "xl/_rels/workbook.xml.rels")?;
        for node in rels_document
            .root_element()
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "Relationship")
        {
            if let (Some(id), Some(target)) = (node.attribute("Id"), node.attribute("Target")) {
                rel_targets.insert(id.to_string(), target.to_string());
            }
        }
    }
    let styles = if has("xl/styles.xml") {
        let styles_text = read_part(&mut zip, "xl/styles.xml")?;
        parse_styles(&styles_text)?
    } else {
        Vec::new()
    };
    let shared: Vec<String> = if has("xl/sharedStrings.xml") {
        let shared_text = read_part(&mut zip, "xl/sharedStrings.xml")?;
        parse_shared_strings(&shared_text)?
    } else {
        Vec::new()
    };
    // Load every sheet grid (bounded).
    let mut grids: Vec<SheetGrid> = Vec::new();
    for (position, name) in sheet_names.iter().enumerate() {
        let by_rid = sheet_rids
            .get(position)
            .and_then(|rid| rid.as_deref())
            .and_then(|rid| rel_targets.get(rid))
            .map(|target| {
                if target.starts_with("worksheets/") {
                    format!("xl/{target}")
                } else if target.starts_with('/') {
                    target.trim_start_matches('/').to_string()
                } else if target.starts_with("xl/") {
                    target.to_string()
                } else {
                    format!("xl/{target}")
                }
            });
        let mut candidates = Vec::new();
        if let Some(path) = by_rid {
            candidates.push(path);
        }
        candidates.push(format!("xl/worksheets/sheet{}.xml", position + 1));
        let mut loaded: Option<SheetGrid> = None;
        let mut missing = true;
        for candidate in candidates {
            if !has(&candidate) {
                continue;
            }
            missing = false;
            let sheet_text = read_part(&mut zip, &candidate)?;
            loaded = Some(parse_sheet(name.clone(), &sheet_text, shared.len())?);
            break;
        }
        if missing {
            return Err(XlsxError::Value(format!("sheet {name:?} part is missing")));
        }
        if let Some(grid) = loaded {
            grids.push(grid);
        }
    }
    // Select the worksheet.
    let grid: &SheetGrid = match &options.sheet {
        SheetSelector::Index(index) => grids.get(*index).ok_or_else(|| {
            XlsxError::Sheet(format!("worksheet index {index} is outside the workbook"))
        })?,
        SheetSelector::Name(wanted) => grids
            .iter()
            .find(|grid| grid.name == *wanted)
            .ok_or_else(|| XlsxError::Sheet(format!("worksheet {wanted:?} was not found")))?,
    };
    if grid.empty_row_elements {
        return Err(XlsxError::Value(format!(
            "GAP: worksheet {:?} has <row> elements without cells, which Modeler reads              inconsistently",
            grid.name
        )));
    }
    // Determine the rectangle.
    let ((first_col, first_row), (last_col, last_row)) = match &options.range {
        RangeSelector::FirstNonBlank => match bounding_box(&grid.cells, &shared, &styles)? {
            Some(corners) => corners,
            None => {
                let model =
                    DataModel::new(Vec::new()).map_err(|e| XlsxError::Value(e.to_string()))?;
                return Table::new(model, Vec::new()).map_err(|e| XlsxError::Value(e.to_string()));
            }
        },
        RangeSelector::Explicit { start, end } => {
            let (start_col, start_row) = *start;
            let (end_col, end_row) = *end;
            if start_col == 0
                || start_row == 0
                || end_col == 0
                || end_row == 0
                || start_col > MAX_COLUMN
                || end_col > MAX_COLUMN
                || start_row > MAX_ROW
                || end_row > MAX_ROW
            {
                return Err(XlsxError::Grid(
                    "explicit range is outside the sheet".into(),
                ));
            }
            if start_col > end_col || start_row > end_row {
                return Err(XlsxError::Grid(
                    "explicit range start is after its end".into(),
                ));
            }
            let width = (end_col as u64)
                .checked_sub(start_col as u64)
                .and_then(|w| w.checked_add(1))
                .ok_or_else(|| XlsxError::Grid("explicit range is too large".into()))?;
            let height = (end_row as u64)
                .checked_sub(start_row as u64)
                .and_then(|h| h.checked_add(1))
                .ok_or_else(|| XlsxError::Grid("explicit range is too large".into()))?;
            let cells = width
                .checked_mul(height)
                .ok_or_else(|| XlsxError::Grid("explicit range is too large".into()))?;
            if cells > MAX_GRID_CELLS {
                return Err(XlsxError::Grid("explicit range is too large".into()));
            }
            ((start_col, start_row), (end_col, end_row))
        }
    };
    let width = (last_col as usize)
        .checked_sub(first_col as usize)
        .and_then(|w| w.checked_add(1))
        .ok_or_else(|| XlsxError::Grid("range is too large".into()))?;
    let height = (last_row as usize)
        .checked_sub(first_row as usize)
        .and_then(|h| h.checked_add(1))
        .ok_or_else(|| XlsxError::Grid("range is too large".into()))?;
    if (width as u64)
        .checked_mul(height as u64)
        .is_some_and(|n| n > MAX_GRID_CELLS)
    {
        return Err(XlsxError::Grid("range is too large".into()));
    }
    if width == 0 || height == 0 {
        let model = DataModel::new(Vec::new()).map_err(|e| XlsxError::Value(e.to_string()))?;
        return Table::new(model, Vec::new()).map_err(|e| XlsxError::Value(e.to_string()));
    }
    // Materialize the rectangle: `None` is an absent/blank cell.
    let mut rectangle: Vec<Vec<Option<Typed>>> = Vec::with_capacity(height);
    for row in first_row..=last_row {
        let mut line = Vec::with_capacity(width);
        for col in first_col..=last_col {
            let typed = match grid.cells.get(&(col, row)) {
                None => None,
                Some(cell) => {
                    let style_kind = styles
                        .get(cell.style as usize)
                        .copied()
                        .unwrap_or(NumKind::General);
                    typed_of(&cell.kind, style_kind, &shared)?
                }
            };
            // An empty string counts as blank for the grid, like an absent cell.
            let typed = match typed {
                Some(Typed::Text(text)) if text.is_empty() => None,
                other => other,
            };
            line.push(typed);
        }
        rectangle.push(line);
    }
    // Header row vs data rows.
    let (header_line, data_lines) = if options.field_names {
        if rectangle.is_empty() {
            (Vec::new(), Vec::new())
        } else {
            let mut lines = rectangle;
            let header = lines.remove(0);
            (header, lines)
        }
    } else {
        (Vec::new(), rectangle)
    };
    // Blank rows: entirely `None`.
    let mut records: Vec<Vec<Option<Typed>>> = Vec::with_capacity(data_lines.len());
    for line in data_lines {
        let blank = line.iter().all(|cell| cell.is_none());
        if blank {
            match options.blank_rows {
                // `excel_more/more_blank2_noempty_stop`: reading ends at the first blank row.
                BlankRows::StopReading => break,
                BlankRows::ReturnBlankRows => records.push(line),
            }
        } else {
            records.push(line);
        }
    }
    // Field names.
    let mut names: Vec<String> = Vec::with_capacity(width);
    if options.field_names {
        for (offset, cell) in header_line.iter().enumerate() {
            let relative = offset.checked_add(1).unwrap_or(1);
            let name = match cell {
                None => format!("C{relative}"),
                Some(Typed::Text(text)) if text.trim().is_empty() => {
                    format!("C{relative}")
                }
                Some(Typed::Text(text)) if text.contains(',') => {
                    return Err(XlsxError::Value(format!(
                        "GAP: field name {text:?} has a comma; only data cells have an oracle"
                    )));
                }
                Some(Typed::Text(text)) => text.clone(),
                Some(Typed::Integer(number)) => format!("{number}.000000"),
                Some(Typed::Real(number)) => format!("{number:.6}"),
                Some(Typed::Time(_)) => format!("C{relative}"),
            };
            names.push(name);
        }
        // A header shorter than the range cannot happen here (rectangle is full width),
        // but keep the width stable.
        while names.len() < width {
            let relative = names.len().checked_add(1).unwrap_or(1);
            names.push(format!("C{relative}"));
        }
    } else {
        for relative in 1..=width {
            names.push(format!("C{relative}"));
        }
    }
    // Duplicate headers gain ` (1)`, ` (2)`, … (second `dup` → `dup (1)`).
    {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for name in names.iter_mut() {
            let base = name.clone();
            let entry = counts.get(&base).copied().unwrap_or(0);
            if entry > 0 {
                *name = format!("{base} ({entry})");
            }
            counts
                .entry(base)
                .and_modify(|n| *n = n.saturating_add(1))
                .or_insert(1);
        }
    }
    // Storage inference over the scan horizon (first `scan_lines` data rows).
    let horizon = options.scan_lines.max(1);
    let scanned = records
        .get(..records.len().min(horizon))
        .unwrap_or(&records);
    let mut storages: Vec<dms_types::Storage> = Vec::with_capacity(width);
    for col in 0..width {
        let mut has_string = false;
        let mut has_time = false;
        for row in scanned.iter() {
            match row.get(col).and_then(|c| c.as_ref()) {
                Some(Typed::Text(_)) => has_string = true,
                Some(Typed::Time(_)) => has_time = true,
                _ => {}
            }
            if has_string {
                break;
            }
        }
        if has_string {
            storages.push(dms_types::Storage::String);
            continue;
        }
        // No strings in the horizon: the column stays numeric or time; a late string
        // beyond a short horizon becomes null.
        if has_time {
            storages.push(dms_types::Storage::Time);
            continue;
        }
        // Any time beyond the horizon also makes a numeric column a time column only
        // when the horizon already saw time (here it did not, since `has_time` is false),
        // unless the scanned rows are all time-or-null; otherwise the late times become
        // null. Check the whole column for time.
        let mut any_time = false;
        for row in records.iter() {
            if matches!(row.get(col).and_then(|c| c.as_ref()), Some(Typed::Time(_))) {
                any_time = true;
                break;
            }
        }
        if any_time && scanned_all_time_or_null(scanned, col) {
            storages.push(dms_types::Storage::Time);
            continue;
        }
        // Numeric columns are real, whole numbers included (every numeric column of the
        // `excel_more` tables, e.g. `more_off5` id 1..6 reads `1.0`). A column with no number
        // reads as a string when strings come later, else as an all-null real.
        let any_number = records.iter().any(|row| {
            matches!(
                row.get(col).and_then(|c| c.as_ref()),
                Some(Typed::Integer(_) | Typed::Real(_))
            )
        });
        let any_late_string = records
            .iter()
            .any(|row| matches!(row.get(col).and_then(|c| c.as_ref()), Some(Typed::Text(_))));
        storages.push(if !any_number && any_late_string {
            dms_types::Storage::String
        } else {
            dms_types::Storage::Real
        });
    }
    // Rows: an absent/empty cell is `""` in a string column (`excel_default` id 2 text, blank
    // rows in `excel_more/more_blank_noempty_return`) and null elsewhere; late strings beyond
    // a numeric horizon are null (`excel_scan_1`).
    let mut fields = Vec::with_capacity(width);
    for (name, storage) in names.iter().zip(storages.iter()) {
        let mut field = Field::new(name.clone(), *storage);
        field.measurement = match storage {
            dms_types::Storage::String => Measurement::Categorical,
            _ => Measurement::Continuous,
        };
        fields.push(field);
    }
    let model = DataModel::new(fields).map_err(|e| XlsxError::Value(e.to_string()))?;
    let mut rows: Vec<Vec<Value>> = Vec::with_capacity(records.len());
    for record in records.iter() {
        let mut out = Vec::with_capacity(width);
        for (col, storage) in storages.iter().enumerate() {
            let cell = record.get(col).and_then(|c| c.as_ref());
            match (cell, storage) {
                (None, dms_types::Storage::String) => out.push(Value::string("")),
                (None, _) => out.push(Value::Null),
                // Modeler turns every comma of a text cell into a period
                // (`excel_more/more_commas`: `a,b` -> `a.b`, `1,5` -> `1.5`, `x,y,z` -> `x.y.z`).
                (Some(Typed::Text(text)), dms_types::Storage::String) => {
                    out.push(Value::string(text.replace(',', ".")));
                }
                (Some(Typed::Text(_)), _) => out.push(Value::Null),
                (Some(Typed::Time(micros)), dms_types::Storage::Time) => {
                    out.push(Value::Time(*micros));
                }
                (Some(Typed::Time(micros)), dms_types::Storage::String) => {
                    out.push(Value::string(Value::Time(*micros).to_string()));
                }
                (Some(Typed::Time(_)), _) => out.push(Value::Null),
                (Some(Typed::Integer(number)), dms_types::Storage::Real) => {
                    out.push(Value::real(*number as f64));
                }
                (Some(Typed::Integer(number)), dms_types::Storage::String) => {
                    out.push(Value::string(number.to_string()));
                }
                (Some(Typed::Integer(_)), _) => out.push(Value::Null),
                (Some(Typed::Real(number)), dms_types::Storage::Real) => {
                    out.push(Value::real(*number));
                }
                (Some(Typed::Real(number)), dms_types::Storage::String) => {
                    out.push(Value::string(number.to_string()));
                }
                (Some(Typed::Real(_)), _) => out.push(Value::Null),
            }
        }
        rows.push(out);
    }
    Table::new(model, rows).map_err(|e| XlsxError::Value(e.to_string()))
}

fn scanned_all_time_or_null(scanned: &[Vec<Option<Typed>>], col: usize) -> bool {
    for row in scanned {
        match row.get(col).and_then(|c| c.as_ref()) {
            None => {}
            Some(Typed::Time(_)) => {}
            _ => return false,
        }
    }
    true
}

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

    #[test]
    fn a1_and_r1c1_cells_parse_and_reject_bad_refs() {
        assert_eq!(parse_a1("A1"), Ok((1, 1)));
        assert_eq!(parse_a1("b3"), Ok((2, 3)));
        assert_eq!(parse_a1("$B$3"), Ok((2, 3)));
        assert_eq!(parse_a1("B$3"), Ok((2, 3)));
        assert_eq!(parse_a1("$B3"), Ok((2, 3)));
        assert_eq!(parse_a1("XFD1048576"), Ok((16_384, 1_048_576)));
        assert!(parse_a1("").is_err());
        assert!(parse_a1("A0").is_err());
        assert!(parse_a1("A1048577").is_err());
        assert!(parse_a1("XFE1").is_err());
        assert!(parse_a1("R3C2").is_err());
        assert_eq!(parse_r1c1("R3C2"), Ok((2, 3)));
        assert_eq!(parse_r1c1("r12c9"), Ok((9, 12)));
        assert_eq!(parse_r1c1("R1C16384"), Ok((16384, 1)));
        assert!(parse_r1c1("R0C1").is_err());
        assert!(parse_r1c1("R1C0").is_err());
        assert!(parse_r1c1("R[3]C[2]").is_err());
        assert!(parse_r1c1("B3").is_err());
        assert_eq!(parse_range_bound("B3"), Ok((2, 3)));
        assert_eq!(parse_range_bound("R3C2"), Ok((2, 3)));
        assert!(parse_range_bound("").is_err());
    }

    #[test]
    fn serial_dates_cover_the_1900_bug_and_1904() {
        use dms_types::calendar;
        // 1900 system: 1 = 1900-01-01, 59 = 1900-02-28, 60 is the false leap day.
        assert_eq!(
            serial_1900_to_unix_days(1),
            calendar::days_from_civil(1900, 1, 1)
        );
        assert_eq!(
            serial_1900_to_unix_days(59),
            calendar::days_from_civil(1900, 2, 28)
        );
        assert_eq!(serial_1900_to_unix_days(60), None);
        assert_eq!(
            serial_1900_to_unix_days(61),
            calendar::days_from_civil(1900, 3, 1)
        );
        assert_eq!(
            serial_1900_to_unix_days(43_890),
            calendar::days_from_civil(2020, 2, 29)
        );
        // 1904 system: 0 = 1904-01-01.
        assert_eq!(
            serial_1904_to_unix_days(0),
            calendar::days_from_civil(1904, 1, 1)
        );
        assert_eq!(
            serial_1904_to_unix_days(1),
            calendar::days_from_civil(1904, 1, 2)
        );
        // Fractions are times of day, independent of the epoch.
        assert_eq!(
            serial_fraction_to_micros(0.5),
            calendar::micros_from_time(12, 0, 0, 0)
        );
        assert_eq!(
            serial_fraction_to_micros(0.25),
            calendar::micros_from_time(6, 0, 0, 0)
        );
        assert_eq!(
            serial_fraction_to_micros(4_3890.5),
            calendar::micros_from_time(12, 0, 0, 0)
        );
        assert!(serial_fraction_to_micros(f64::NAN).is_none());
    }

    fn minimal_workbook(
        sheets: &[(&str, &str)],
        styles: &str,
        shared: Option<&str>,
        date1904: bool,
    ) -> Vec<u8> {
        use std::io::Write as _;
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        let mut write = |name: &str, data: &str| {
            writer.start_file(name, opts).unwrap();
            writer.write_all(data.as_bytes()).unwrap();
        };
        let mut content_types = String::from(
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#,
        );
        let main_ns = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
        let rel_ns = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
        let pkg_ns = "http://schemas.openxmlformats.org/package/2006/relationships";
        let mut workbook = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="{main_ns}" xmlns:r="{rel_ns}"><workbookPr date1904="{date}"/><sheets>"#,
            date = if date1904 { "1" } else { "0" }
        );
        let mut rels =
            format!(r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{pkg_ns}">"#);
        for (index, (name, sheet_xml)) in sheets.iter().enumerate() {
            let id = index + 1;
            content_types.push_str(&format!(
                r#"<Override PartName="/xl/worksheets/sheet{id}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
            ));
            workbook.push_str(&format!(
                r#"<sheet name="{name}" sheetId="{id}" r:id="rId{id}"/>"#
            ));
            rels.push_str(&format!(
                r#"<Relationship Id="rId{id}" Type="{rel_ns}/worksheet" Target="worksheets/sheet{id}.xml"/>"#
            ));
            write(&format!("xl/worksheets/sheet{id}.xml"), sheet_xml);
        }
        workbook.push_str("</sheets></workbook>");
        rels.push_str(&format!(
            r#"<Relationship Id="rIdStyles" Type="{rel_ns}/styles" Target="styles.xml"/></Relationships>"#
        ));
        content_types.push_str("</Types>");
        write("[Content_Types].xml", &content_types);
        write(
            "_rels/.rels",
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{pkg_ns}"><Relationship Id="rId1" Type="{rel_ns}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
            ),
        );
        write("xl/workbook.xml", &workbook);
        write("xl/_rels/workbook.xml.rels", &rels);
        write("xl/styles.xml", styles);
        if let Some(sst) = shared {
            write("xl/sharedStrings.xml", sst);
        }
        writer.finish().unwrap().into_inner()
    }

    fn probe_styles() -> String {
        let ns = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="{ns}"><numFmts count="3"><numFmt numFmtId="164" formatCode="yyyy-mm-dd"/><numFmt numFmtId="165" formatCode="hh:mm:ss"/><numFmt numFmtId="166" formatCode="yyyy-mm-dd hh:mm:ss"/></numFmts><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="1"><fill><patternFill patternType="none"/></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="5"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="49" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="165" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="166" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/></cellXfs></styleSheet>"#
        )
    }

    #[test]
    fn shared_inline_numbers_booleans_formulas_and_styles_read() {
        let sheet = r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>id</t></is></c><c r="B1" t="inlineStr"><is><t>when</t></is></c><c r="C1" t="inlineStr"><is><t>as_text</t></is></c><c r="D1" t="inlineStr"><is><t>flag</t></is></c><c r="E1" t="inlineStr"><is><t>calc</t></is></c></row><row r="2"><c r="A2" t="n"><v>3</v></c><c r="B2" s="3" t="n"><v>0.5</v></c><c r="C2" s="1" t="n"><v>12</v></c><c r="D2" t="b"><v>1</v></c><c r="E2"><f>1+2</f><v>3</v></c></row><row r="3"><c r="A3" t="s"><v>0</v></c><c r="B3" s="4" t="n"><v>1.5</v></c><c r="C3" s="1" t="inlineStr"><is><t>oops</t></is></c><c r="D3" t="b"><v>0</v></c><c r="E3" t="n"><v></v></c></row></sheetData></worksheet>"#;
        let shared = r#"<?xml version="1.0" encoding="UTF-8"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1" uniqueCount="1"><si><t>banana</t></si></sst>"#;
        let bytes = minimal_workbook(&[("S", sheet)], &probe_styles(), Some(shared), false);
        let table = read_table(&bytes, &ExcelOptions::default()).unwrap();
        let names: Vec<&str> = table
            .model()
            .fields()
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(names, ["id", "when", "as_text", "flag", "calc"]);
        let col = |name: &str| table.column(name).unwrap().cloned().collect::<Vec<_>>();
        assert_eq!(col("id"), [Value::string("3"), Value::string("banana")]);
        assert_eq!(
            col("when"),
            [
                Value::Time(calendar::micros_from_time(12, 0, 0, 0).unwrap()),
                Value::Time(calendar::micros_from_time(12, 0, 0, 0).unwrap())
            ]
        );
        assert_eq!(col("as_text"), [Value::string("12"), Value::string("oops")]);
        assert_eq!(col("flag"), [Value::real(1.0), Value::real(0.0)]);
        assert_eq!(col("calc"), [Value::real(3.0), Value::Null]);
        // Shared-string out of range, bad numbers and bad refs fail closed.
        assert!(read_table(b"not a zip", &ExcelOptions::default()).is_err());
        assert!(read_table(&bytes[..bytes.len() / 2], &ExcelOptions::default()).is_err());
    }

    /// A sheet of `<row>` elements, each a list of inline-string or numeric cells in columns A..
    fn rows_sheet(rows: &[&[&str]]) -> String {
        let mut xml = String::from(
            r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
        );
        for (index, row) in rows.iter().enumerate() {
            let number = index + 1;
            if row.is_empty() {
                continue; // Excel writes no element for an empty row
            }
            xml.push_str(&format!(r#"<row r="{number}">"#));
            for (column, text) in row.iter().enumerate() {
                let column = char::from(b'A' + u8::try_from(column).unwrap());
                if text.parse::<f64>().is_ok() {
                    xml.push_str(&format!(r#"<c r="{column}{number}"><v>{text}</v></c>"#));
                } else {
                    xml.push_str(&format!(
                        r#"<c r="{column}{number}" t="inlineStr"><is><t>{text}</t></is></c>"#
                    ));
                }
            }
            xml.push_str("</row>");
        }
        xml.push_str("</sheetData></worksheet>");
        xml
    }

    /// `excel_more` rules: numeric columns are real, commas in text read as periods, blank rows
    /// stop reading or stay in place (`""` in string columns), and a sheet with a cell-less
    /// `<row>` element is refused.
    #[test]
    fn blank_rows_commas_and_empty_row_elements_follow_the_oracles() {
        let sheet = rows_sheet(&[&["id", "s"], &["1", "a,b"], &[], &["2", "x,y,z"]]);
        let bytes = minimal_workbook(&[("S", &sheet)], &probe_styles(), None, false);
        let read = |blank_rows| {
            let options = ExcelOptions {
                blank_rows,
                ..Default::default()
            };
            read_table(&bytes, &options).unwrap()
        };
        let all = read(BlankRows::ReturnBlankRows);
        assert_eq!(all.model().fields()[0].storage, dms_types::Storage::Real);
        let rows: Vec<Vec<Value>> = all.rows().to_vec();
        assert_eq!(
            rows,
            [
                vec![Value::real(1.0), Value::string("a.b")],
                vec![Value::Null, Value::string("")],
                vec![Value::real(2.0), Value::string("x.y.z")],
            ]
        );
        assert_eq!(read(BlankRows::StopReading).row_count(), 1);

        let with_empty_row = sheet.replace(r#"<row r="4">"#, r#"<row r="3"/><row r="4">"#);
        let bytes = minimal_workbook(&[("S", &with_empty_row)], &probe_styles(), None, false);
        let error = read_table(&bytes, &ExcelOptions::default()).unwrap_err();
        assert!(error.to_string().starts_with("GAP:"), "{error}");
    }

    #[test]
    fn malformed_packages_fail_closed() {
        let good_sheet = r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>id</t></is></c></row><row r="2"><c r="A2" t="n"><v>1</v></c></row></sheetData></worksheet>"#;
        let bytes = minimal_workbook(&[("S", good_sheet)], &probe_styles(), None, false);
        assert!(read_table(&bytes, &ExcelOptions::default()).is_ok());
        // Truncated zip, non-XML sheet, missing workbook, bad shared index, bad style.
        assert!(read_table(&bytes[..20], &ExcelOptions::default()).is_err());
        let bad_sheet = good_sheet.replace("A1", "ZZZ9");
        let bad = minimal_workbook(&[("S", &bad_sheet)], &probe_styles(), None, false);
        assert!(read_table(&bad, &ExcelOptions::default()).is_err());
        let bad_shared = r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>id</t></is></c></row><row r="2"><c r="A2" t="s"><v>9</v></c></row></sheetData></worksheet>"#;
        let bad = minimal_workbook(&[("S", bad_shared)], &probe_styles(), None, false);
        assert!(read_table(&bad, &ExcelOptions::default()).is_err());
        // Bad range bounds fail closed.
        let out_of_range = ExcelOptions {
            range: RangeSelector::Explicit {
                start: (1, 1),
                end: (16_385, 1),
            },
            ..Default::default()
        };
        assert!(read_table(&bytes, &out_of_range).is_err());
        // Unknown sheet fails closed.
        let missing = ExcelOptions {
            sheet: SheetSelector::Name("Nope".into()),
            ..Default::default()
        };
        assert!(read_table(&bytes, &missing).is_err());
    }
}
