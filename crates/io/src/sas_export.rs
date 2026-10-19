//! SAS dataset export writer (`.sas7bdat` for `SAS7`/`SAS8`, `.sd2` for `Windows`, `.ssd` for
//! `UNIX`), matching the files written by IBM SPSS Modeler 19's `SASExportNode`
//! (`oracle/sas_export/*`).
//!
//! Sources and clean-room basis:
//! - IBM SPSS Modeler 19 *Scripting and Automation Guide*, `sasexportnode` properties (`full_filename`,
//!   `format`, `export_names`, `generate_import`).
//! - Differential observation of the 5 Modeler 19 oracle files under `oracle/sas_export/`:
//!   - `sasx_sas8/exports/sasx_sas8.sas7bdat` (`format = "SAS8"`, `export_names = "NamesAndLabels"`)
//!   - `sasx_sas7/exports/sasx_sas7.sas7bdat` (`format = "SAS7"`, `export_names = "NamesAndLabels"`)
//!   - `sasx_names_as_labels/exports/sasx_names_as_labels.sas7bdat` (`format = "SAS8"`, `export_names = "NamesAsLabels"`)
//!   - `sasx_windows/exports/sasx_windows.sd2` (`format = "Windows"`, `export_names = "NamesAndLabels"`)
//!   - `sasx_unix/exports/sasx_unix.ssd` (`format = "UNIX"`, `export_names = "NamesAndLabels"`)

use crate::sas::{SAS_EPOCH_UNIX_DAYS, SAS7BDAT_MAGIC};
use dms_types::{Field, Storage, Table, Value};
use std::time::{SystemTime, UNIX_EPOCH};

/// SAS export file format (`format` scripting property on `sasexportnode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SasExportFormat {
    /// SAS v7 Windows/UNIX `.sas7bdat` (`format = "SAS7"`).
    Sas7,
    /// SAS v8 Windows/UNIX `.sas7bdat` (`format = "SAS8"`, default).
    #[default]
    Sas8,
    /// SAS v6 for Windows/OS2 `.sd2` (`format = "Windows"`).
    WindowsSd2,
    /// SAS v6 for UNIX (SunOS SPARC) `.ssd` (`format = "UNIX"`).
    UnixSsd,
}

/// Variable name / label export mode (`export_names` scripting property on `sasexportnode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SasExportNameMode {
    /// Export field names (sanitized/truncated) as SAS variable names and field labels as variable labels.
    #[default]
    NamesAndLabels,
    /// Export sequential names `VAR00001`, `VAR00002`, ... and store original field names as variable labels.
    NamesAsLabels,
}

/// Options for writing a [`Table`] to a SAS export file.
#[derive(Debug, Clone, Default)]
pub struct SasExportOptions {
    pub format: SasExportFormat,
    pub names: SasExportNameMode,
    /// Dataset stem (normally derived from the output file stem, e.g. `"sasx_sas8"` -> `"SASX_SAS8"`).
    pub dataset_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SasExportError {
    Settings(String),
    Data(String),
}

impl std::fmt::Display for SasExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SasExportError::Settings(msg) | SasExportError::Data(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for SasExportError {}

const SAS7_HEADER_LEN: usize = 1024;
const SAS7_PAGE_LEN: usize = 4096;
const SD2_HEADER_LEN: usize = 256;
const SSD_HEADER_LEN: usize = 8192;
const V6_PAGE_LEN: usize = 8192;

const MISSING_LE: [u8; 8] = [0x00, 0x00, 0x00, 0x00, 0x00, 0xd1, 0xff, 0xff];
const MISSING_BE: [u8; 8] = [0xff, 0xff, 0xfe, 0x00, 0x00, 0x00, 0x00, 0x00];

#[derive(Debug, Clone)]
struct PreparedCol {
    /// Logical column index in the input [`Table`].
    source_index: usize,
    /// Variable name in the SAS file (`<= 8` bytes for v6/v7/v8 in Modeler's Stat/Transfer engine).
    name: String,
    /// Variable label (`""` when empty).
    label: String,
    /// Format name (`"MMDDYY"` for `Storage::Date`, `""` otherwise).
    format: &'static str,
    /// True for `Integer`, `Real`, `Date`; false for `String`.
    is_numeric: bool,
    /// True for `Storage::Date`.
    is_date: bool,
    /// Byte offset within a data row.
    row_offset: usize,
    /// Byte width within a data row (`8` for numeric, `>= 1` for string).
    width: usize,
}

/// Serialize `table` into a SAS file according to `options`.
pub fn write_table(table: &Table, options: &SasExportOptions) -> Result<Vec<u8>, SasExportError> {
    for field in table.model().fields() {
        match field.storage {
            Storage::Integer | Storage::Real | Storage::Date | Storage::String => {}
            Storage::Time | Storage::Timestamp | Storage::List | Storage::Unknown => {
                return Err(SasExportError::Data(format!(
                    "GAP: SAS export does not support {:?} storage on field {:?}",
                    field.storage, field.name
                )));
            }
        }
    }

    // The oracles hold ASCII text only; the encoding of other characters is unverified.
    let non_ascii = table.rows().iter().flatten().any(|value| match value {
        Value::String(text) => !text.is_ascii(),
        _ => false,
    }) || table.model().fields().iter().any(|field| {
        !field.name.is_ascii()
            || field
                .label
                .as_deref()
                .is_some_and(|label| !label.is_ascii())
    });
    if non_ascii {
        return Err(SasExportError::Data(
            "GAP: SAS export of non-ASCII names, labels or text is not covered by the oracles"
                .into(),
        ));
    }

    match options.format {
        SasExportFormat::Sas7 | SasExportFormat::Sas8 => write_sas7bdat(table, options),
        SasExportFormat::WindowsSd2 => write_v6(table, options, false),
        SasExportFormat::UnixSsd => write_v6(table, options, true),
    }
}

fn prepare_columns(
    table: &Table,
    names_mode: SasExportNameMode,
    v6_short_names: bool,
    reorder_numeric_first: bool,
    even_string_width: bool,
) -> Result<(Vec<PreparedCol>, usize), SasExportError> {
    let fields = table.model().fields();
    let mut cols = Vec::with_capacity(fields.len());
    for (idx, field) in fields.iter().enumerate() {
        let (name, label) = resolve_name_and_label(idx, field, names_mode, v6_short_names);
        let is_numeric = !matches!(field.storage, Storage::String);
        let is_date = matches!(field.storage, Storage::Date);
        let format = if is_date { "MMDDYY" } else { "" };
        let width = if is_numeric {
            8
        } else {
            let max_len = max_string_byte_len(table, idx);
            let w = max_len.max(1);
            if even_string_width { (w + 1) & !1 } else { w }
        };
        cols.push(PreparedCol {
            source_index: idx,
            name,
            label,
            format,
            is_numeric,
            is_date,
            row_offset: 0,
            width,
        });
    }

    let mut row_offset = 0usize;
    if reorder_numeric_first {
        for col in cols.iter_mut().filter(|c| c.is_numeric) {
            col.row_offset = row_offset;
            row_offset = row_offset
                .checked_add(col.width)
                .ok_or_else(|| SasExportError::Data("row width overflow".into()))?;
        }
        for col in cols.iter_mut().filter(|c| !c.is_numeric) {
            col.row_offset = row_offset;
            row_offset = row_offset
                .checked_add(col.width)
                .ok_or_else(|| SasExportError::Data("row width overflow".into()))?;
        }
    } else {
        for col in &mut cols {
            col.row_offset = row_offset;
            row_offset = row_offset
                .checked_add(col.width)
                .ok_or_else(|| SasExportError::Data("row width overflow".into()))?;
        }
    }

    Ok((cols, row_offset))
}

fn resolve_name_and_label(
    index: usize,
    field: &Field,
    names_mode: SasExportNameMode,
    v6_short_names: bool,
) -> (String, String) {
    let field_label = field.label.as_deref().unwrap_or("");
    match names_mode {
        SasExportNameMode::NamesAndLabels => {
            let max_len = if v6_short_names { 8 } else { 32 };
            let mut name = String::new();
            for ch in field.name.chars() {
                if name.len() >= max_len {
                    break;
                }
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    if v6_short_names {
                        name.push(ch.to_ascii_uppercase());
                    } else {
                        name.push(ch);
                    }
                } else {
                    name.push('_');
                }
            }
            if name.is_empty() {
                name = format!("VAR{:05}", index.saturating_add(1));
            }
            (name, field_label.to_string())
        }
        SasExportNameMode::NamesAsLabels => {
            let name = format!("VAR{index:04}");
            let label = if field_label.is_empty() {
                field.name.clone()
            } else {
                field_label.to_string()
            };
            (name, label)
        }
    }
}

fn max_string_byte_len(table: &Table, col_idx: usize) -> usize {
    let mut max_len = 0usize;
    for row in table.rows() {
        if let Some(Value::String(s)) = row.get(col_idx) {
            max_len = max_len.max(s.len());
        }
    }
    max_len
}

fn write_sas7bdat(table: &Table, options: &SasExportOptions) -> Result<Vec<u8>, SasExportError> {
    let (cols, row_length) = prepare_columns(table, options.names, false, true, false)?;
    let num_cols = cols.len();
    let row_count = table.rows().len();

    let mut out = vec![0u8; SAS7_HEADER_LEN + SAS7_PAGE_LEN];
    write_sas7bdat_header(&mut out[..SAS7_HEADER_LEN], &options.dataset_name)?;
    write_sas7bdat_page(
        &mut out[SAS7_HEADER_LEN..],
        table,
        &cols,
        row_length,
        num_cols,
        row_count,
    )?;
    Ok(out)
}

fn write_sas7bdat_header(header: &mut [u8], dataset_name: &str) -> Result<(), SasExportError> {
    put_slice(header, 0, &SAS7BDAT_MAGIC)?;
    put_slice(
        header,
        0x20,
        &[
            0x22, 0x22, 0x00, 0x32, 0x22, 0x01, 0x02, 0x32, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x7e, 0x00, 0x00, 0x00, 0x03, 0x01, 0x18, 0x1f, 0x10, 0x11, 0x22, 0x22, 0x00, 0x32,
            0x22, 0x01, 0x02, 0x32, 0x04, 0x32, 0x01, 0x22, 0x22, 0x00, 0x7e, 0x00, 0x00, 0x10,
            0x03, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    )?;
    put_slice(header, 0x54, b"SAS FILE")?;

    let mut ds_buf = [b' '; 64];
    let stem_upper: String = dataset_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(32)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let stem_bytes = stem_upper.as_bytes();
    if let Some(dst) = ds_buf.get_mut(..stem_bytes.len()) {
        dst.copy_from_slice(stem_bytes);
    }
    put_slice(header, 0x5c, &ds_buf)?;
    put_slice(header, 0x9c, b"DATA    ")?;

    // Timestamp bytes 164..180 (0xa4..0xb4) and 288..304 (0x120..0x130).
    // Only bytes 166..=175 and 290..=303 vary with the export timestamp in Modeler's oracle files.
    let now_ticks = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let tick_word = (((now_ticks as u16) & 0x03ff) << 6) | 0x0040;
    let [t0, t1] = tick_word.to_le_bytes();
    let t2 = (0x8bu8).wrapping_add((now_ticks & 0x0f) as u8);

    put_slice(header, 0xa4, &[0x00, 0x00, t0, t1, 0x98, 0x69, 0xdf, 0x41])?;
    put_slice(header, 0xac, &[0x00, 0x00, t0, t1, 0x98, 0x69, 0xdf, 0x41])?;
    put_slice(
        header,
        0xb4,
        &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x20, 0xbc, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x20,
            0xbc, 0x40,
        ],
    )?;
    put_u32_le(header, 0xc4, SAS7_HEADER_LEN as u32)?;
    put_u32_le(header, 0xc8, SAS7_PAGE_LEN as u32)?;
    put_u32_le(header, 0xcc, 1)?;
    put_slice(header, 0xd8, b"8.0000M0")?;
    put_slice(header, 0xe0, b"WIN\0")?;
    put_slice(header, 0x100, b"WIN\0")?;
    put_slice(
        header,
        0x120,
        &[
            0x00, 0x00, t0, t1, 0x4c, 0x4e, t2, 0x91, 0x4c, 0x4e, t2, 0x91, 0x4c, 0x4e, t2, 0x91,
        ],
    )?;
    put_slice(header, 0x140, &[0x6a, 0x49, 0x57, 0x2d])?;
    Ok(())
}

fn write_sas7bdat_page(
    page: &mut [u8],
    table: &Table,
    cols: &[PreparedCol],
    row_length: usize,
    num_cols: usize,
    row_count: usize,
) -> Result<(), SasExportError> {
    // 1. Build ColumnText payload (after its 4-byte signature).
    let mut text_strings = Vec::new();
    let mut name_refs = Vec::with_capacity(num_cols);
    let mut label_refs = Vec::with_capacity(num_cols);
    let mut format_refs = Vec::with_capacity(num_cols);

    for col in cols {
        let name_off = (28usize.saturating_add(text_strings.len())) as u16;
        let name_len = col.name.len() as u16;
        append_padded4(&mut text_strings, col.name.as_bytes());
        name_refs.push((name_off, name_len));

        let (lbl_off, lbl_len) = if !col.label.is_empty() {
            let off = (28usize.saturating_add(text_strings.len())) as u16;
            let len = col.label.len() as u16;
            append_padded4(&mut text_strings, col.label.as_bytes());
            (off, len)
        } else {
            (0u16, 0u16)
        };
        label_refs.push((lbl_off, lbl_len));

        let (fmt_off, fmt_len) = if !col.format.is_empty() {
            let off = (28usize.saturating_add(text_strings.len())) as u16;
            let len = col.format.len() as u16;
            append_padded4(&mut text_strings, col.format.as_bytes());
            (off, len)
        } else {
            (0u16, 0u16)
        };
        format_refs.push((fmt_off, fmt_len));
    }

    let strings_end = 28usize.saturating_add(text_strings.len());
    let col_text_len = 4usize.saturating_add(strings_end).saturating_add(28);

    // Subheader offsets packed from end of page (0x1000) downwards:
    let row_size_len = 0x01e0usize;
    let row_size_off = SAS7_PAGE_LEN.saturating_sub(row_size_len); // 0x0e20

    let col_size_len = 0x000cusize;
    let col_size_off = row_size_off.saturating_sub(col_size_len); // 0x0e14

    let counts_len = 0x0130usize;
    let counts_off = col_size_off.saturating_sub(counts_len); // 0x0ce4

    let col_text_off = counts_off.saturating_sub(col_text_len);

    let col_name_len = 20usize.saturating_add(8usize.saturating_mul(num_cols));
    let col_name_off = col_text_off.saturating_sub(col_name_len);

    let col_attr_len = 20usize.saturating_add(12usize.saturating_mul(num_cols));
    let col_attr_off = col_name_off.saturating_sub(col_attr_len);

    let fmt_lbl_len = 52usize;
    let fmt_lbl_total = fmt_lbl_len.saturating_mul(num_cols);
    let col0_fmt_off = col_attr_off.saturating_sub(fmt_lbl_total);

    let subheaders_count = 7usize.saturating_add(num_cols);
    let ptr_table_end = 24usize.saturating_add(12usize.saturating_mul(subheaders_count));
    let data_start = (ptr_table_end.saturating_add(7)) & !7;
    let data_end = data_start.saturating_add(row_length.saturating_mul(row_count));

    if data_end > col0_fmt_off {
        return Err(SasExportError::Data(
            "GAP: SAS export of more than one sas7bdat page (the oracles fit on one page)".into(),
        ));
    }

    // Write page header (0..24)
    let free_space = (col0_fmt_off.saturating_sub(data_end)) as u32;
    put_slice(page, 0, &[0x6b, 0x49, 0x57, 0x2d])?;
    put_u32_le(page, 12, free_space)?;
    put_u16_le(page, 16, 0x0200)?;
    put_u16_le(
        page,
        18,
        (subheaders_count.saturating_add(row_count)) as u16,
    )?;
    put_u16_le(page, 20, subheaders_count as u16)?;

    // Write subheader pointers (starting at 24)
    let mut ptr_pos = 24usize;
    write_sas7_ptr(page, &mut ptr_pos, row_size_off, row_size_len, 0, 0)?;
    write_sas7_ptr(page, &mut ptr_pos, col_size_off, col_size_len, 0, 0)?;
    write_sas7_ptr(page, &mut ptr_pos, counts_off, counts_len, 0, 0)?;
    write_sas7_ptr(page, &mut ptr_pos, col_text_off, col_text_len, 0, 1)?;
    write_sas7_ptr(page, &mut ptr_pos, col_name_off, col_name_len, 0, 1)?;
    write_sas7_ptr(page, &mut ptr_pos, col_attr_off, col_attr_len, 0, 1)?;
    for i in 0..num_cols {
        let off = col0_fmt_off.saturating_add(i.saturating_mul(fmt_lbl_len));
        write_sas7_ptr(page, &mut ptr_pos, off, fmt_lbl_len, 0, 0)?;
    }
    write_sas7_ptr(page, &mut ptr_pos, col0_fmt_off, 0, 1, 0)?;

    // Write RowSize subheader (row_size_off..0x1000)
    let rs = page
        .get_mut(row_size_off..row_size_off + row_size_len)
        .ok_or_else(|| SasExportError::Data("RowSize slice out of bounds".into()))?;
    put_slice(rs, 0, &[0xf7, 0xf7, 0xf7, 0xf7])?;
    put_u32_le(rs, 4, 205)?;
    put_u32_le(rs, 8, 13)?;
    put_slice(rs, 16, &[0x11, 0x30, 0x02, 0x00])?;
    put_u32_le(rs, 20, row_length as u32)?;
    put_u32_le(rs, 24, row_count as u32)?;
    put_u32_le(rs, 36, num_cols as u32)?;
    put_u32_le(rs, 52, SAS7_PAGE_LEN as u32)?;
    put_u32_le(rs, 60, row_count as u32)?;
    put_slice(rs, 64, &[0xff; 8])?;
    put_slice(rs, 220, &[0x6b, 0x49, 0x57, 0x2d])?;
    put_u32_le(rs, 264, 1)?;
    put_u32_le(rs, 268, 2)?;
    put_u32_le(rs, 272, 1)?;
    put_u32_le(rs, 276, (6usize.saturating_add(num_cols)) as u32)?;
    put_u32_le(rs, 280, 1)?;
    put_u32_le(rs, 284, (8usize.saturating_add(num_cols)) as u32)?;
    put_u32_le(rs, 288, 1)?;
    put_u32_le(
        rs,
        292,
        (7usize.saturating_add(num_cols).saturating_add(row_count)) as u32,
    )?;
    put_u32_le(rs, 296, 1)?;
    put_u32_le(rs, 300, 7)?;
    put_u32_le(rs, 348, 4)?;
    put_u16_le(rs, 352, 12)?;
    put_u16_le(rs, 354, 16)?;
    put_u16_le(rs, 358, 20)?;
    put_u16_le(rs, 370, 12)?;
    put_u16_le(rs, 372, 8)?;
    put_u16_le(rs, 376, 28)?;
    put_u16_le(rs, 378, 4)?;
    put_slice(rs, 416, &[0x04, 0x00, 0x01, 0x00, 0x01, 0x00])?;
    put_u16_le(rs, 438, 109)?;
    put_slice(rs, 466, &[0x00, 0x01])?;

    // Write ColumnSize subheader
    let cs = page
        .get_mut(col_size_off..col_size_off + col_size_len)
        .ok_or_else(|| SasExportError::Data("ColumnSize slice out of bounds".into()))?;
    put_slice(cs, 0, &[0xf6, 0xf6, 0xf6, 0xf6])?;
    put_u32_le(cs, 4, num_cols as u32)?;

    // Write SubheaderCounts subheader
    let sc = page
        .get_mut(counts_off..counts_off + counts_len)
        .ok_or_else(|| SasExportError::Data("SubheaderCounts slice out of bounds".into()))?;
    put_slice(sc, 0, &[0x00, 0xfc, 0xff, 0xff])?;
    put_u32_le(sc, 4, 48)?;
    put_u32_le(sc, 8, 4)?;
    put_u32_le(sc, 12, 7)?;
    put_u32_le(sc, 56, 0x070c)?;
    put_slice(sc, 64, &[0xfc, 0xff, 0xff, 0xff])?;
    put_u32_le(sc, 68, 1)?;
    put_u32_le(sc, 72, 6)?;
    put_u32_le(sc, 76, 1)?;
    put_u32_le(sc, 80, 6)?;
    put_slice(sc, 84, &[0xfd, 0xff, 0xff, 0xff])?;
    put_u32_le(sc, 88, 1)?;
    put_u32_le(sc, 92, 4)?;
    put_u32_le(sc, 96, 1)?;
    put_u32_le(sc, 100, 4)?;
    put_slice(sc, 104, &[0xff, 0xff, 0xff, 0xff])?;
    put_u32_le(sc, 108, 1)?;
    put_u32_le(sc, 112, 5)?;
    put_u32_le(sc, 116, 1)?;
    put_u32_le(sc, 120, 5)?;
    put_slice(sc, 124, &[0xfe, 0xff, 0xff, 0xff])?;
    put_slice(sc, 144, &[0xfb, 0xff, 0xff, 0xff])?;
    put_slice(sc, 164, &[0xfa, 0xff, 0xff, 0xff])?;
    put_slice(sc, 184, &[0xf9, 0xff, 0xff, 0xff])?;

    // Write ColumnText subheader
    let ct = page
        .get_mut(col_text_off..col_text_off + col_text_len)
        .ok_or_else(|| SasExportError::Data("ColumnText slice out of bounds".into()))?;
    put_slice(ct, 0, &[0xfd, 0xff, 0xff, 0xff])?;
    let ct_payload = &mut ct[4..];
    put_u16_le(ct_payload, 0, (strings_end.saturating_add(20)) as u16)?;
    put_u16_le(ct_payload, 2, 20)?;
    put_u16_le(ct_payload, 4, strings_end as u16)?;
    put_slice(ct_payload, 12, b"Written by SAS\0\0")?;
    put_slice(ct_payload, 28, &text_strings)?;
    put_u32_le(ct_payload, strings_end, 1)?;
    put_u32_le(ct_payload, strings_end.saturating_add(4), 20)?;

    // Write ColumnName subheader
    let cn = page
        .get_mut(col_name_off..col_name_off + col_name_len)
        .ok_or_else(|| SasExportError::Data("ColumnName slice out of bounds".into()))?;
    put_slice(cn, 0, &[0xff, 0xff, 0xff, 0xff])?;
    put_u16_le(
        cn,
        4,
        (8usize.saturating_add(8usize.saturating_mul(num_cols))) as u16,
    )?;
    for (i, &(name_off, name_len)) in name_refs.iter().enumerate() {
        let base = 12usize.saturating_add(i.saturating_mul(8));
        put_u16_le(cn, base, 0)?;
        put_u16_le(cn, base + 2, name_off)?;
        put_u16_le(cn, base + 4, name_len)?;
        put_u16_le(cn, base + 6, 0)?;
    }

    // Write ColumnAttributes subheader
    let ca = page
        .get_mut(col_attr_off..col_attr_off + col_attr_len)
        .ok_or_else(|| SasExportError::Data("ColumnAttributes slice out of bounds".into()))?;
    put_slice(ca, 0, &[0xfc, 0xff, 0xff, 0xff])?;
    put_u16_le(
        ca,
        4,
        (8usize.saturating_add(12usize.saturating_mul(num_cols))) as u16,
    )?;
    for (i, col) in cols.iter().enumerate() {
        let base = 12usize.saturating_add(i.saturating_mul(12));
        put_u32_le(ca, base, col.row_offset as u32)?;
        put_u32_le(ca, base + 4, col.width as u32)?;
        put_u16_le(ca, base + 8, 0x0400)?;
        let col_type = if col.is_numeric { 1u8 } else { 2u8 };
        put_slice(ca, base + 10, &[col_type, 0])?;
    }

    // Write ColumnFormatAndLabel subheaders
    for i in 0..num_cols {
        let off = col0_fmt_off.saturating_add(i.saturating_mul(fmt_lbl_len));
        let fl = page.get_mut(off..off + fmt_lbl_len).ok_or_else(|| {
            SasExportError::Data("ColumnFormatAndLabel slice out of bounds".into())
        })?;
        put_slice(fl, 0, &[0xfe, 0xfb, 0xff, 0xff])?;
        if cols.get(i).is_some_and(|c| c.is_date) {
            put_u16_le(fl, 12, 8)?;
        }
        let (fmt_off, fmt_len) = format_refs.get(i).copied().unwrap_or((0, 0));
        let (lbl_off, lbl_len) = label_refs.get(i).copied().unwrap_or((0, 0));
        put_u16_le(fl, 34, 0)?;
        put_u16_le(fl, 36, fmt_off)?;
        put_u16_le(fl, 38, fmt_len)?;
        put_u16_le(fl, 40, 0)?;
        put_u16_le(fl, 42, lbl_off)?;
        put_u16_le(fl, 44, lbl_len)?;
    }

    // Write data rows
    write_rows(page, data_start, row_length, table, cols, false)?;
    Ok(())
}

fn write_sas7_ptr(
    page: &mut [u8],
    ptr_pos: &mut usize,
    offset: usize,
    length: usize,
    compression: u8,
    subheader_type: u8,
) -> Result<(), SasExportError> {
    put_u32_le(page, *ptr_pos, offset as u32)?;
    put_u32_le(page, *ptr_pos + 4, length as u32)?;
    put_slice(page, *ptr_pos + 8, &[compression, subheader_type, 0, 0])?;
    *ptr_pos = ptr_pos.saturating_add(12);
    Ok(())
}

fn append_padded4(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(bytes);
    let padded = (bytes.len().saturating_add(3)) & !3;
    buf.resize(
        buf.len().saturating_add(padded.saturating_sub(bytes.len())),
        0,
    );
}

fn write_v6(
    table: &Table,
    options: &SasExportOptions,
    big_endian: bool,
) -> Result<Vec<u8>, SasExportError> {
    let (cols, row_length) = prepare_columns(table, options.names, true, false, big_endian)?;
    let num_cols = cols.len();
    let row_count = table.rows().len();
    let header_len = if big_endian {
        SSD_HEADER_LEN
    } else {
        SD2_HEADER_LEN
    };

    let mut out = vec![0u8; header_len + V6_PAGE_LEN];
    if big_endian {
        write_ssd_header(&mut out[..header_len])?;
    } else {
        write_sd2_header(&mut out[..header_len])?;
    }
    write_v6_page(
        &mut out[header_len..],
        table,
        &cols,
        row_length,
        num_cols,
        row_count,
        big_endian,
    )?;
    Ok(out)
}

fn write_sd2_header(header: &mut [u8]) -> Result<(), SasExportError> {
    put_slice(header, 0x00, b"SAS     6.08.04 ")?;
    put_slice(header, 0x10, b"WIN      3.10   ")?;
    put_slice(header, 0x20, &[b' '; 32])?;
    put_slice(header, 0x40, b"STATTRAN")?;
    put_slice(
        header,
        0x48,
        &[
            0x00, 0x00, 0xc0, 0x4c, 0x0f, 0x3b, 0xd1, 0x41, 0x95, 0x63, 0x91, 0x14, 0x54, 0x32,
            0xc6, 0x1b, 0x23, 0x0e, 0x38, 0xdb,
        ],
    )?;
    put_u32_le(header, 0x5c, V6_PAGE_LEN as u32)?;
    put_u32_le(header, 0x74, 1)?;
    put_slice(
        header,
        0x78,
        &[
            0x00, 0x00, 0xc0, 0x4c, 0x0f, 0x3b, 0xd1, 0x41, 0x77, 0x6d, 0x25, 0x1f, 0x77, 0xad,
            0xa9, 0x5c, 0xb7, 0x21, 0xea, 0x68,
        ],
    )?;
    Ok(())
}

fn write_ssd_header(header: &mut [u8]) -> Result<(), SasExportError> {
    put_slice(header, 0x00, b"SAS     6.11.00 ")?;
    put_slice(header, 0x10, b"STATTRANDATA    ")?;
    put_slice(header, 0x20, b"SunOS   4.1     ")?;
    put_slice(header, 0x30, b"SUN     SUN 4   ")?;
    put_slice(header, 0x40, b"SPARC")?;
    put_slice(
        header,
        0x68,
        &[
            0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00,
            0x00, 0x01, 0x32, 0xa1, 0x86, 0xb2, 0x32, 0xa1, 0x86, 0xb3, 0x00, 0x00, 0x20, 0x00,
            0x7e, 0xef, 0xe9, 0xd7, 0x7e, 0xef, 0xe9, 0xd7, 0x7e, 0xef, 0xe9, 0xd7,
        ],
    )?;
    Ok(())
}

fn write_v6_page(
    page: &mut [u8],
    table: &Table,
    cols: &[PreparedCol],
    row_length: usize,
    num_cols: usize,
    row_count: usize,
    big_endian: bool,
) -> Result<(), SasExportError> {
    let total_blocks = (3usize.saturating_add(num_cols).saturating_add(row_count)) as u16;
    put_u16_end(page, 8, total_blocks, big_endian)?;
    put_u16_end(page, 10, 0x0200, big_endian)?;

    let yzrh_off = 0x1f60usize;
    let yzrh_len = 0x00a0usize;
    let xoeh_off = 0x1ee8usize;
    let xoeh_len = 0x0078usize;
    let name_len = 0x0070usize;

    let mut ptr_pos = 16usize;
    write_v6_ptr(page, &mut ptr_pos, yzrh_off, yzrh_len, 0, big_endian)?;
    write_v6_ptr(page, &mut ptr_pos, xoeh_off, xoeh_len, 0, big_endian)?;
    for i in 0..num_cols {
        let sh_off = xoeh_off.saturating_sub((i.saturating_add(1)).saturating_mul(name_len));
        write_v6_ptr(page, &mut ptr_pos, sh_off, name_len, 0, big_endian)?;
    }
    let last_name_off = xoeh_off.saturating_sub(num_cols.saturating_mul(name_len));
    write_v6_ptr(page, &mut ptr_pos, last_name_off, 0, 1, big_endian)?;

    let data_start = (ptr_pos.saturating_add(7)) & !7;
    let data_end = data_start.saturating_add(row_length.saturating_mul(row_count));
    if data_end > last_name_off {
        return Err(SasExportError::Data(
            "GAP: SAS export of more than one SAS v6 page (the oracles fit on one page)".into(),
        ));
    }

    // Write NAME subheaders
    for (i, col) in cols.iter().enumerate() {
        let sh_off = xoeh_off.saturating_sub((i.saturating_add(1)).saturating_mul(name_len));
        let nm = page
            .get_mut(sh_off..sh_off + name_len)
            .ok_or_else(|| SasExportError::Data("NAME subheader out of bounds".into()))?;
        put_slice(nm, 0, b"NAME")?;
        put_space_padded(nm, 4, 8, col.name.as_bytes())?;
        let col_type = if col.is_numeric { 1u8 } else { 2u8 };
        put_slice(nm, 12, &[col_type, 0, 0, 0])?;
        put_space_padded(nm, 20, 40, col.label.as_bytes())?;
        put_u32_end(nm, 64, col.row_offset as u32, big_endian)?;
        put_u16_end(nm, 68, col.width as u16, big_endian)?;
        put_u16_end(nm, 70, (i.saturating_add(1)) as u16, big_endian)?;
        put_space_padded(nm, 72, 8, col.format.as_bytes())?;
        let fmt_width = if col.is_date { 10u16 } else { 0u16 };
        put_u16_end(nm, 80, fmt_width, big_endian)?;
        put_space_padded(nm, 84, 8, b"")?;
    }

    // Write XOEH subheader
    let xoeh = page
        .get_mut(xoeh_off..xoeh_off + xoeh_len)
        .ok_or_else(|| SasExportError::Data("XOEH subheader out of bounds".into()))?;
    put_slice(xoeh, 0, b"XOEH")?;
    put_u32_end(xoeh, 4, 1, big_endian)?;
    put_u32_end(xoeh, 8, num_cols as u32, big_endian)?;
    put_space_padded(xoeh, 32, 48, b"")?;
    put_space_padded(xoeh, 80, 16, b"DATASTEP")?;

    // Write YZRH subheader
    let yzrh = page
        .get_mut(yzrh_off..yzrh_off + yzrh_len)
        .ok_or_else(|| SasExportError::Data("YZRH subheader out of bounds".into()))?;
    put_slice(yzrh, 0, b"YZRH")?;
    put_u32_end(yzrh, 4, 101, big_endian)?;
    put_u32_end(yzrh, 8, num_cols as u32, big_endian)?;
    put_u32_end(yzrh, 16, 228, big_endian)?;
    put_u32_end(yzrh, 20, row_length as u32, big_endian)?;
    put_u32_end(yzrh, 24, row_count as u32, big_endian)?;
    put_u32_end(yzrh, 32, V6_PAGE_LEN as u32, big_endian)?;
    put_u32_end(yzrh, 36, row_count as u32, big_endian)?;
    put_u32_end(yzrh, 40, 1, big_endian)?;
    put_u16_end(yzrh, 44, 2, big_endian)?;
    put_u32_end(yzrh, 48, 1, big_endian)?;
    put_u16_end(
        yzrh,
        52,
        (2usize.saturating_add(num_cols)) as u16,
        big_endian,
    )?;
    put_u32_end(yzrh, 56, 1, big_endian)?;
    put_u16_end(
        yzrh,
        60,
        (4usize.saturating_add(num_cols)) as u16,
        big_endian,
    )?;
    put_u32_end(yzrh, 64, 1, big_endian)?;
    put_u16_end(yzrh, 68, total_blocks, big_endian)?;
    if big_endian {
        put_slice(yzrh, 80, &[0xcd, 0x51, 0x5c, 0x5f])?;
    } else {
        put_slice(yzrh, 80, &[0x0f, 0x3b, 0x11, 0x0d])?;
    }
    put_slice(yzrh, 112, b"NONE    EL")?;
    put_slice(yzrh, 130, &[0x00, 0x01])?;

    write_rows(page, data_start, row_length, table, cols, big_endian)?;
    Ok(())
}

fn write_v6_ptr(
    page: &mut [u8],
    ptr_pos: &mut usize,
    offset: usize,
    length: usize,
    flag0: u8,
    big_endian: bool,
) -> Result<(), SasExportError> {
    put_u32_end(page, *ptr_pos, offset as u32, big_endian)?;
    put_u32_end(page, *ptr_pos + 4, length as u32, big_endian)?;
    put_slice(page, *ptr_pos + 8, &[flag0, 0, 0, 0])?;
    *ptr_pos = ptr_pos.saturating_add(12);
    Ok(())
}

fn write_rows(
    page: &mut [u8],
    data_start: usize,
    row_length: usize,
    table: &Table,
    cols: &[PreparedCol],
    big_endian: bool,
) -> Result<(), SasExportError> {
    for (row_idx, row) in table.rows().iter().enumerate() {
        let row_base = data_start.saturating_add(row_idx.saturating_mul(row_length));
        for col in cols {
            let cell_off = row_base.saturating_add(col.row_offset);
            let val = row.get(col.source_index).unwrap_or(&Value::Null);
            if col.is_numeric {
                let bytes = match val {
                    Value::Null => {
                        if big_endian {
                            MISSING_BE
                        } else {
                            MISSING_LE
                        }
                    }
                    Value::Integer(n) => {
                        let f = *n as f64;
                        if big_endian {
                            f.to_be_bytes()
                        } else {
                            f.to_le_bytes()
                        }
                    }
                    Value::Real(r) => {
                        if r.is_nan() {
                            if big_endian { MISSING_BE } else { MISSING_LE }
                        } else if big_endian {
                            r.to_be_bytes()
                        } else {
                            r.to_le_bytes()
                        }
                    }
                    Value::Date(unix_days) => {
                        let sas_days = (*unix_days - SAS_EPOCH_UNIX_DAYS) as f64;
                        if big_endian {
                            sas_days.to_be_bytes()
                        } else {
                            sas_days.to_le_bytes()
                        }
                    }
                    other => {
                        return Err(SasExportError::Data(format!(
                            "unexpected value {other:?} in numeric column {:?}",
                            col.name
                        )));
                    }
                };
                put_slice(page, cell_off, &bytes)?;
            } else {
                let text_bytes = match val {
                    Value::Null => &b""[..],
                    Value::String(s) => s.as_bytes(),
                    other => {
                        return Err(SasExportError::Data(format!(
                            "unexpected value {other:?} in string column {:?}",
                            col.name
                        )));
                    }
                };
                put_space_padded(page, cell_off, col.width, text_bytes)?;
            }
        }
    }
    Ok(())
}

fn put_slice(buf: &mut [u8], offset: usize, src: &[u8]) -> Result<(), SasExportError> {
    let end = offset
        .checked_add(src.len())
        .ok_or_else(|| SasExportError::Data("slice offset overflow".into()))?;
    let dst = buf
        .get_mut(offset..end)
        .ok_or_else(|| SasExportError::Data(format!("offset {offset}..{end} out of bounds")))?;
    dst.copy_from_slice(src);
    Ok(())
}

fn put_space_padded(
    buf: &mut [u8],
    offset: usize,
    width: usize,
    src: &[u8],
) -> Result<(), SasExportError> {
    let end = offset
        .checked_add(width)
        .ok_or_else(|| SasExportError::Data("slice offset overflow".into()))?;
    let dst = buf
        .get_mut(offset..end)
        .ok_or_else(|| SasExportError::Data(format!("offset {offset}..{end} out of bounds")))?;
    dst.fill(b' ');
    let copy_len = src.len().min(width);
    if let (Some(d), Some(s)) = (dst.get_mut(..copy_len), src.get(..copy_len)) {
        d.copy_from_slice(s);
    }
    Ok(())
}

fn put_u16_le(buf: &mut [u8], offset: usize, val: u16) -> Result<(), SasExportError> {
    put_slice(buf, offset, &val.to_le_bytes())
}

fn put_u32_le(buf: &mut [u8], offset: usize, val: u32) -> Result<(), SasExportError> {
    put_slice(buf, offset, &val.to_le_bytes())
}

fn put_u16_end(
    buf: &mut [u8],
    offset: usize,
    val: u16,
    big_endian: bool,
) -> Result<(), SasExportError> {
    let bytes = if big_endian {
        val.to_be_bytes()
    } else {
        val.to_le_bytes()
    };
    put_slice(buf, offset, &bytes)
}

fn put_u32_end(
    buf: &mut [u8],
    offset: usize,
    val: u32,
    big_endian: bool,
) -> Result<(), SasExportError> {
    let bytes = if big_endian {
        val.to_be_bytes()
    } else {
        val.to_le_bytes()
    };
    put_slice(buf, offset, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sas::{NameMode, SasFormat, SasOptions, read_table};
    use dms_types::{DataModel, Measurement, Role};

    fn sample_table() -> Table {
        let mut id = Field::new("id", Storage::Integer);
        id.measurement = Measurement::Continuous;
        id.role = Role::Input;

        let mut name = Field::new("name", Storage::String);
        name.measurement = Measurement::Nominal;
        name.role = Role::Input;

        let mut amount = Field::new("amount", Storage::Real);
        amount.measurement = Measurement::Continuous;
        amount.role = Role::Input;

        let mut joined = Field::new("joined", Storage::Date);
        joined.measurement = Measurement::Continuous;
        joined.role = Role::Input;

        let mut long = Field::new("long_field_name_x", Storage::Real);
        long.measurement = Measurement::Continuous;
        long.role = Role::Input;

        let model = DataModel::new(vec![id, name, amount, joined, long]).unwrap();
        Table::new(
            model,
            vec![
                vec![
                    Value::Integer(1),
                    Value::string("Alice"),
                    Value::Real(12.5),
                    Value::Date(18262),
                    Value::Real(0.25),
                ],
                vec![
                    Value::Integer(2),
                    Value::string("Bob"),
                    Value::Null,
                    Value::Date(18701),
                    Value::Real(-1.5),
                ],
                vec![
                    Value::Integer(3),
                    Value::string(""),
                    Value::Real(-7.25),
                    Value::Null,
                    Value::Real(3.0),
                ],
                vec![
                    Value::Integer(4),
                    Value::string("Dan"),
                    Value::Real(1000.0),
                    Value::Date(18261),
                    Value::Null,
                ],
            ],
        )
        .unwrap()
    }

    #[test]
    fn roundtrip_sas7bdat_names_and_labels() {
        let table = sample_table();
        let bytes = write_table(
            &table,
            &SasExportOptions {
                format: SasExportFormat::Sas8,
                names: SasExportNameMode::NamesAndLabels,
                dataset_name: "sasx_sas8".into(),
            },
        )
        .unwrap();
        assert_eq!(bytes.len(), 5120);

        let read_back = read_table(
            &bytes,
            &SasOptions {
                format: SasFormat::Sas7bdat,
                member_name: None,
                names: NameMode::NamesAndLabels,
            },
        )
        .unwrap();
        let fields = read_back.model().fields();
        assert_eq!(fields.len(), 5);
        assert_eq!(fields[0].name, "id");
        assert_eq!(fields[1].name, "name");
        assert_eq!(fields[2].name, "amount");
        assert_eq!(fields[3].name, "joined");
        assert_eq!(fields[3].storage, Storage::Date);
        assert_eq!(fields[4].name, "long_field_name_x");
        assert_eq!(read_back.rows().len(), 4);
        assert_eq!(read_back.rows()[0][3], Value::Date(18262));
        assert_eq!(read_back.rows()[1][2], Value::Null);
        assert_eq!(read_back.rows()[2][3], Value::Null);
        assert_eq!(read_back.rows()[3][4], Value::Null);
    }

    #[test]
    fn roundtrip_sas7bdat_names_as_labels() {
        let table = sample_table();
        let bytes = write_table(
            &table,
            &SasExportOptions {
                format: SasExportFormat::Sas8,
                names: SasExportNameMode::NamesAsLabels,
                dataset_name: "sasx_names_as_labels".into(),
            },
        )
        .unwrap();
        assert_eq!(bytes.len(), 5120);

        let read_back = read_table(
            &bytes,
            &SasOptions {
                format: SasFormat::Sas7bdat,
                member_name: None,
                names: NameMode::NamesAndLabels,
            },
        )
        .unwrap();
        let fields = read_back.model().fields();
        assert_eq!(fields[0].name, "VAR0000");
        assert_eq!(fields[0].label.as_deref(), Some("id"));
        assert_eq!(fields[4].name, "VAR0004");
        assert_eq!(fields[4].label.as_deref(), Some("long_field_name_x"));
    }

    #[test]
    fn writes_sd2_and_ssd_sizes() {
        let table = sample_table();
        let sd2 = write_table(
            &table,
            &SasExportOptions {
                format: SasExportFormat::WindowsSd2,
                names: SasExportNameMode::NamesAndLabels,
                dataset_name: "sasx_windows".into(),
            },
        )
        .unwrap();
        assert_eq!(sd2.len(), 256 + 8192);

        let ssd = write_table(
            &table,
            &SasExportOptions {
                format: SasExportFormat::UnixSsd,
                names: SasExportNameMode::NamesAndLabels,
                dataset_name: "sasx_unix".into(),
            },
        )
        .unwrap();
        assert_eq!(ssd.len(), 8192 + 8192);
    }

    #[test]
    fn rejects_unsupported_storage_with_gap() {
        let model = DataModel::new(vec![Field::new("t", Storage::Time)]).unwrap();
        let table = Table::new(model, vec![vec![Value::Time(0)]]).unwrap();
        let err = write_table(&table, &SasExportOptions::default()).unwrap_err();
        assert!(err.to_string().contains("GAP:"), "unexpected error: {err}");
    }
}
