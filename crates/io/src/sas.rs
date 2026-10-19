//! SAS data files (`sas7bdat` and SAS Transport `XPORT v5`) as [`Table`]s, matching Modeler's
//! SAS File source node (`SASInputNode`, `oracle/sas/*`).
//!
//! Clean-room references:
//! - `XPORT v5`: SAS Technical Support document TS-140 (*The Record Layout of a Data Set in SAS
//!   Transport (XPORT) Format*).
//! - `sas7bdat`: M. Shotwell, *SAS7BDAT Database Binary Format* (public format description).
//!
//! Verified rules (`oracle/sas/sas9_names`, `oracle/sas/sas9_labels`, `oracle/sas/sas_transport`):
//! - Numeric variables without a date format become [`Storage::Real`] (`Measurement::Continuous`);
//!   SAS system missing (`.` / IEEE NaN in `sas7bdat`, `[0x2e, 0, ..]` in `XPORT`) and special
//!   missing values (`.A`–`.Z`, `._`) become [`Value::Null`] (`sas9_names`: row 2 `amount`, row 3
//!   `joined`, row 4 `score`; `sas_transport`: same cells).
//! - Numeric variables with a SAS date format (`DATE`, `YYMMDD`, `DDMMYY`, `MMDDYY`, `JULIAN`,
//!   `EURDFDD`, `IS8601DA`, `B8601DA`, `E8601DA`) become [`Storage::Date`] (`Measurement::Continuous`).
//!   SAS date `0` is `1960-01-01` (Unix day `-3653`), so `21915` is `2020-01-01` and `21974` is
//!   `2020-02-29` (`sas9_names` column `joined`, `sas_transport` column `JOINED`).
//! - Character variables become [`Storage::String`] (`Measurement::Categorical`) with trailing
//!   ASCII blanks (`' '`) stripped (`sas9_names` column `name`, stored width 12, `"Alice       "`
//!   -> `"Alice"`).
//! - [`NameMode::NamesAndLabels`] keeps the SAS variable name as the field name and stores a
//!   non-empty variable label on [`Field::label`] (`sas9_names`, `sas_transport`).
//! - [`NameMode::LabelsAsNames`] uses a non-empty variable label as the field name (`Customer name`,
//!   `Join date`, `Amount (EUR)`) and names unlabeled variables `field<1-based column position>`
//!   (`field2`, `field5`, `field6` in `sas9_labels`).
//! - In `XPORT v5`, [`SasOptions::member_name`] selects the matching member (case-insensitively
//!   after trimming trailing blanks); when `None` or empty, the first member in the transport file
//!   is read (`sas_transport`: `member_name = "SASTEST"`).
//!
//! Explicit `GAP:` errors are returned for unsupported variants:
//! - `sas7bdat`: 32-bit alignment (`align_1 != 0x33`), `align_2 != 0x33`, big-endian byte order,
//!   non-UTF-8/ASCII character encodings, RLE (`SASYZCRL`) or RDC (`SASYZCR2`) row compression,
//!   compressed/truncated subheaders, deleted/compacted page types, and numeric widths other than
//!   8 bytes.
//! - `XPORT`: v8 transport files (`HEADER RECORD*******LIBV8` / `MEMBV8`), `NAMESTR` sizes other
//!   than 140 or 136 bytes, and numeric variable widths other than 8 bytes.
//! - Time, datetime, and other unverified SAS numeric formats.

use dms_types::{DataModel, Field, Measurement, ModelError, Storage, Table, Value};
use std::fmt;

/// Days from Unix epoch (`1970-01-01`) to SAS epoch (`1960-01-01`).
pub(crate) const SAS_EPOCH_UNIX_DAYS: i64 = -3653;

/// Maximum page size accepted in a `sas7bdat` file (16 MiB).
const MAX_PAGE_SIZE: usize = 16 * 1024 * 1024;
/// Maximum number of pages accepted in a `sas7bdat` file.
const MAX_PAGE_COUNT: usize = 1_000_000;
/// Maximum number of columns accepted in a SAS dataset.
pub(crate) const MAX_COLUMNS: usize = 65_536;
/// Maximum number of rows accepted in a SAS dataset.
pub(crate) const MAX_ROWS: usize = 10_000_000;

pub(crate) const SAS7BDAT_MAGIC: [u8; 32] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc2, 0xea, 0x81, 0x60,
    0xb3, 0x14, 0x11, 0xcf, 0xbd, 0x92, 0x08, 0x00, 0x09, 0xc7, 0x31, 0x8c, 0x18, 0x1f, 0x10, 0x11,
];

pub(crate) const SIG_ROW_SIZE: [u8; 8] = [0xf7, 0xf7, 0xf7, 0xf7, 0x00, 0x00, 0x00, 0x00];
pub(crate) const SIG_COL_SIZE: [u8; 8] = [0xf6, 0xf6, 0xf6, 0xf6, 0x00, 0x00, 0x00, 0x00];
pub(crate) const SIG_COUNTS: [u8; 8] = [0x00, 0xfc, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
pub(crate) const SIG_COL_TEXT: [u8; 8] = [0xfd, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
pub(crate) const SIG_COL_NAME: [u8; 8] = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
pub(crate) const SIG_COL_ATTR: [u8; 8] = [0xfc, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
pub(crate) const SIG_COL_LIST: [u8; 8] = [0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
pub(crate) const SIG_COL_FMT: [u8; 8] = [0xfe, 0xfb, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];

pub(crate) const PAGE_META: u16 = 0x0000;
pub(crate) const PAGE_DATA: u16 = 0x0100;
pub(crate) const PAGE_MIX: u16 = 0x0200;
pub(crate) const PAGE_AMD: u16 = 0x0400;

const XPT_LIB_HEADER: &[u8; 48] = b"HEADER RECORD*******LIBRARY HEADER RECORD!!!!!!!";
const XPT_MEM_HEADER: &[u8; 48] = b"HEADER RECORD*******MEMBER  HEADER RECORD!!!!!!!";
const XPT_DSC_HEADER: &[u8; 48] = b"HEADER RECORD*******DSCRPTR HEADER RECORD!!!!!!!";
const XPT_NAM_HEADER: &[u8; 48] = b"HEADER RECORD*******NAMESTR HEADER RECORD!!!!!!!";
const XPT_OBS_HEADER: &[u8; 48] = b"HEADER RECORD*******OBS     HEADER RECORD!!!!!!!";

/// SAS file container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SasFormat {
    /// SAS 7–9 native dataset (`.sas7bdat`).
    #[default]
    Sas7bdat,
    /// SAS Transport (`XPORT v5`, `.xpt`).
    Transport,
}

/// How SAS variable names and labels map to Modeler field names and labels (`import_names`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NameMode {
    /// Variable name becomes the field name; non-empty variable label becomes [`Field::label`].
    #[default]
    NamesAndLabels,
    /// Non-empty variable label becomes the field name; unlabeled variables become
    /// `field<1-based column position>` (`sas9_labels`).
    LabelsAsNames,
}

/// Options for reading a SAS file into a [`Table`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SasOptions {
    pub format: SasFormat,
    /// Member name for [`SasFormat::Transport`]; `None` or empty selects the first member.
    pub member_name: Option<String>,
    pub names: NameMode,
}

/// Error returned when a SAS file cannot be decoded or uses an unsupported variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SasError {
    /// Malformed or truncated SAS file.
    Syntax(String),
    /// A valid SAS feature or variant that is not implemented (`GAP: ...`).
    Unsupported(String),
    /// The requested transport member was not found.
    MemberNotFound(String),
    /// The decoded fields or rows violated [`DataModel`] invariants.
    Model(ModelError),
}

impl fmt::Display for SasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SasError::Syntax(msg) | SasError::Unsupported(msg) => f.write_str(msg),
            SasError::MemberNotFound(name) => {
                write!(f, "SAS transport member {name:?} was not found")
            }
            SasError::Model(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for SasError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColType {
    Numeric,
    Character,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumKind {
    Real,
    Date,
}

#[derive(Debug, Clone)]
struct ColMeta {
    name: String,
    label: Option<String>,
    format: Option<String>,
    offset: usize,
    width: usize,
    col_type: ColType,
    num_kind: NumKind,
}

/// Reads a SAS dataset (`sas7bdat` or `XPORT v5`) according to `options`.
pub fn read_table(bytes: &[u8], options: &SasOptions) -> Result<Table, SasError> {
    match options.format {
        SasFormat::Sas7bdat => read_sas7bdat(bytes, options.names),
        SasFormat::Transport => read_xpt(bytes, options.member_name.as_deref(), options.names),
    }
}

/// Reads a `sas7bdat` file into a [`Table`].
pub fn read_sas7bdat(bytes: &[u8], names: NameMode) -> Result<Table, SasError> {
    if bytes.len() < 288 {
        return Err(SasError::Syntax(
            "sas7bdat file is shorter than the 288-byte minimum header".into(),
        ));
    }
    if bytes.get(0..32) != Some(SAS7BDAT_MAGIC.as_slice()) {
        return Err(SasError::Syntax(
            "not a sas7bdat file (magic header mismatch)".into(),
        ));
    }

    let align_1 = read_u8(bytes, 32)?;
    let u64_layout = match align_1 {
        0x33 => true,
        0x22 => false,
        0x32 => {
            return Err(SasError::Unsupported(
                "GAP: 32-bit sas7bdat layout (align_1 = 0x32) is not supported".into(),
            ));
        }
        _ => {
            return Err(SasError::Syntax(format!(
                "invalid sas7bdat align_1 flag 0x{align_1:02x}"
            )));
        }
    };

    let align_2 = read_u8(bytes, 35)?;
    let expected_align_2 = if u64_layout { 0x33 } else { 0x32 };
    if align_2 != expected_align_2 {
        return Err(SasError::Unsupported(format!(
            "GAP: sas7bdat align_2 flag 0x{align_2:02x} is not supported (expected 0x{expected_align_2:02x})"
        )));
    }

    let endian = read_u8(bytes, 37)?;
    if endian == 0x00 {
        return Err(SasError::Unsupported(
            "GAP: big-endian sas7bdat files are not supported".into(),
        ));
    }
    if endian != 0x01 {
        return Err(SasError::Syntax(format!(
            "invalid sas7bdat endianness flag 0x{endian:02x}"
        )));
    }

    let platform = read_u8(bytes, 39)?;
    if platform != b'1' && platform != b'2' {
        return Err(SasError::Unsupported(format!(
            "GAP: sas7bdat platform flag 0x{platform:02x} is not supported"
        )));
    }

    let encoding = read_u8(bytes, if u64_layout { 70 } else { 71 })?;
    // 20 (0x14) = UTF-8, 0 = unspecified/default ASCII, 28 = US-ASCII.
    if !matches!(encoding, 0 | 20 | 28) {
        return Err(SasError::Unsupported(format!(
            "GAP: sas7bdat character encoding code {encoding} is not supported (only UTF-8 / ASCII)"
        )));
    }

    if bytes.get(84..92) != Some(b"SAS FILE".as_slice()) {
        return Err(SasError::Syntax(
            "sas7bdat header is missing 'SAS FILE' marker at offset 84".into(),
        ));
    }

    let (header_length, page_size, page_count) = if u64_layout {
        let hl = usize::try_from(read_u32_le(bytes, 200)?)
            .map_err(|_| SasError::Syntax("sas7bdat header_length overflow".into()))?;
        let ps = usize::try_from(read_u32_le(bytes, 204)?)
            .map_err(|_| SasError::Syntax("sas7bdat page_size overflow".into()))?;
        let pc = usize::try_from(read_u64_le(bytes, 208)?)
            .map_err(|_| SasError::Syntax("sas7bdat page_count overflow".into()))?;
        (hl, ps, pc)
    } else {
        let hl = usize::try_from(read_u32_le(bytes, 196)?)
            .map_err(|_| SasError::Syntax("sas7bdat header_length overflow".into()))?;
        let ps = usize::try_from(read_u32_le(bytes, 200)?)
            .map_err(|_| SasError::Syntax("sas7bdat page_size overflow".into()))?;
        let pc = usize::try_from(read_u32_le(bytes, 204)?)
            .map_err(|_| SasError::Syntax("sas7bdat page_count overflow".into()))?;
        (hl, ps, pc)
    };

    if header_length < 288 || header_length > bytes.len() {
        return Err(SasError::Syntax(format!(
            "sas7bdat header_length {header_length} is out of bounds for {}-byte file",
            bytes.len()
        )));
    }
    if !(64..=MAX_PAGE_SIZE).contains(&page_size) {
        return Err(SasError::Syntax(format!(
            "sas7bdat page_size {page_size} is outside 64..={MAX_PAGE_SIZE}"
        )));
    }
    if page_count > MAX_PAGE_COUNT {
        return Err(SasError::Syntax(format!(
            "sas7bdat page_count {page_count} exceeds maximum {MAX_PAGE_COUNT}"
        )));
    }
    let total_pages_bytes = page_count
        .checked_mul(page_size)
        .and_then(|n| header_length.checked_add(n))
        .ok_or_else(|| SasError::Syntax("sas7bdat page geometry overflows file size".into()))?;
    if total_pages_bytes > bytes.len() {
        return Err(SasError::Syntax(format!(
            "sas7bdat file truncated: header ({header_length}) + {page_count} pages of {page_size} bytes exceeds file length {}",
            bytes.len()
        )));
    }

    let page_bit_off = if u64_layout { 32usize } else { 16usize };
    let page_hdr_size = if u64_layout { 40usize } else { 24usize };
    let ptr_size = if u64_layout { 24usize } else { 12usize };
    let sig_size = if u64_layout { 8usize } else { 4usize };

    let mut row_length: Option<usize> = None;
    let mut total_row_count: Option<usize> = None;
    let mut mix_page_row_count: usize = 0;
    let mut col_count: Option<usize> = None;
    let mut col_texts: Vec<Vec<u8>> = Vec::new();
    let mut col_names: Vec<(usize, usize, usize)> = Vec::new();
    let mut col_attrs: Vec<(usize, usize, ColType)> = Vec::new();
    let mut col_fmt_lbls: Vec<(usize, usize, usize, usize, usize, usize)> = Vec::new();

    // Pass 1: collect metadata subheaders across all pages.
    for page_idx in 0..page_count {
        let page_offset = header_length
            .checked_add(page_idx.saturating_mul(page_size))
            .ok_or_else(|| SasError::Syntax("page offset overflow".into()))?;
        let page = bytes
            .get(page_offset..page_offset.saturating_add(page_size))
            .ok_or_else(|| SasError::Syntax("page slice out of bounds".into()))?;

        let page_type = read_u16_le(page, page_bit_off)?;
        let subheaders_count = usize::from(read_u16_le(page, page_bit_off + 4)?);

        match page_type {
            PAGE_META | PAGE_MIX | PAGE_AMD => {}
            PAGE_DATA => continue,
            other => {
                return Err(SasError::Unsupported(format!(
                    "GAP: sas7bdat page type 0x{other:04x} is not supported"
                )));
            }
        }

        let ptrs_end = page_hdr_size
            .checked_add(
                subheaders_count
                    .checked_mul(ptr_size)
                    .ok_or_else(|| SasError::Syntax("subheader pointer count overflow".into()))?,
            )
            .ok_or_else(|| SasError::Syntax("subheader table end overflow".into()))?;
        if ptrs_end > page.len() {
            return Err(SasError::Syntax(format!(
                "sas7bdat page {page_idx}: {subheaders_count} subheader pointers exceed page size"
            )));
        }

        for sh_idx in 0..subheaders_count {
            let ptr_off = page_hdr_size + sh_idx * ptr_size;
            let (sh_offset, sh_length, sh_compression) = if u64_layout {
                let off = usize::try_from(read_u64_le(page, ptr_off)?)
                    .map_err(|_| SasError::Syntax("subheader offset overflow".into()))?;
                let len = usize::try_from(read_u64_le(page, ptr_off + 8)?)
                    .map_err(|_| SasError::Syntax("subheader length overflow".into()))?;
                let comp = read_u8(page, ptr_off + 16)?;
                (off, len, comp)
            } else {
                let off = usize::try_from(read_u32_le(page, ptr_off)?)
                    .map_err(|_| SasError::Syntax("subheader offset overflow".into()))?;
                let len = usize::try_from(read_u32_le(page, ptr_off + 4)?)
                    .map_err(|_| SasError::Syntax("subheader length overflow".into()))?;
                let comp = read_u8(page, ptr_off + 8)?;
                (off, len, comp)
            };

            // Truncated/empty subheader pointer.
            if sh_length == 0 || sh_compression == 1 {
                continue;
            }
            if sh_compression == 4 {
                return Err(SasError::Unsupported(
                    "GAP: compressed sas7bdat subheader/row (compression = 4) is not supported"
                        .into(),
                ));
            }
            if sh_compression != 0 {
                return Err(SasError::Unsupported(format!(
                    "GAP: sas7bdat subheader compression flag {sh_compression} is not supported"
                )));
            }

            let sh_end = sh_offset
                .checked_add(sh_length)
                .ok_or_else(|| SasError::Syntax("subheader end offset overflow".into()))?;
            let sh = page.get(sh_offset..sh_end).ok_or_else(|| {
                SasError::Syntax(format!(
                    "sas7bdat page {page_idx} subheader {sh_idx} ({sh_offset}..{sh_end}) out of bounds"
                ))
            })?;
            let Some(sig) = sh.get(0..sig_size) else {
                return Err(SasError::Syntax(
                    "sas7bdat subheader shorter than signature".into(),
                ));
            };

            if sig == &SIG_ROW_SIZE[..sig_size] {
                let (rl, rc, mprc) = if u64_layout {
                    if sh.len() < 128 {
                        return Err(SasError::Syntax(
                            "sas7bdat RowSize subheader shorter than 128 bytes".into(),
                        ));
                    }
                    let rl = usize::try_from(read_u64_le(sh, 40)?)
                        .map_err(|_| SasError::Syntax("row_length overflow".into()))?;
                    let rc = usize::try_from(read_u64_le(sh, 48)?)
                        .map_err(|_| SasError::Syntax("row_count overflow".into()))?;
                    let mprc = usize::try_from(read_u64_le(sh, 120)?)
                        .map_err(|_| SasError::Syntax("mix_page_row_count overflow".into()))?;
                    (rl, rc, mprc)
                } else {
                    if sh.len() < 64 {
                        return Err(SasError::Syntax(
                            "sas7bdat RowSize subheader shorter than 64 bytes".into(),
                        ));
                    }
                    let rl = usize::try_from(read_u32_le(sh, 20)?)
                        .map_err(|_| SasError::Syntax("row_length overflow".into()))?;
                    let rc = usize::try_from(read_u32_le(sh, 24)?)
                        .map_err(|_| SasError::Syntax("row_count overflow".into()))?;
                    let mprc = usize::try_from(read_u32_le(sh, 60)?)
                        .map_err(|_| SasError::Syntax("mix_page_row_count overflow".into()))?;
                    (rl, rc, mprc)
                };
                if rc > MAX_ROWS {
                    return Err(SasError::Syntax(format!(
                        "sas7bdat row_count {rc} exceeds maximum {MAX_ROWS}"
                    )));
                }
                row_length = Some(rl);
                total_row_count = Some(rc);
                mix_page_row_count = mprc;
            } else if sig == &SIG_COL_SIZE[..sig_size] {
                let cc = if u64_layout {
                    if sh.len() < 16 {
                        return Err(SasError::Syntax(
                            "sas7bdat ColumnSize subheader shorter than 16 bytes".into(),
                        ));
                    }
                    usize::try_from(read_u64_le(sh, 8)?)
                        .map_err(|_| SasError::Syntax("col_count overflow".into()))?
                } else {
                    if sh.len() < 8 {
                        return Err(SasError::Syntax(
                            "sas7bdat ColumnSize subheader shorter than 8 bytes".into(),
                        ));
                    }
                    usize::try_from(read_u32_le(sh, 4)?)
                        .map_err(|_| SasError::Syntax("col_count overflow".into()))?
                };
                if cc > MAX_COLUMNS {
                    return Err(SasError::Syntax(format!(
                        "sas7bdat col_count {cc} exceeds maximum {MAX_COLUMNS}"
                    )));
                }
                col_count = Some(cc);
            } else if sig == &SIG_COL_TEXT[..sig_size] {
                let payload = sh.get(sig_size..).ok_or_else(|| {
                    SasError::Syntax("sas7bdat ColumnText subheader missing payload".into())
                })?;
                if contains_subslice(payload, b"SASYZCRL") {
                    return Err(SasError::Unsupported(
                        "GAP: RLE-compressed sas7bdat (SASYZCRL) is not supported".into(),
                    ));
                }
                if contains_subslice(payload, b"SASYZCR2") {
                    return Err(SasError::Unsupported(
                        "GAP: RDC-compressed sas7bdat (SASYZCR2) is not supported".into(),
                    ));
                }
                col_texts.push(payload.to_vec());
            } else if sig == &SIG_COL_NAME[..sig_size] {
                let hdr_off = if u64_layout { 16usize } else { 12usize };
                let trl_len = if u64_layout { 12usize } else { 8usize };
                if sh.len() < hdr_off + trl_len {
                    return Err(SasError::Syntax(
                        "sas7bdat ColumnName subheader too short".into(),
                    ));
                }
                let entries_end = sh.len() - trl_len;
                let mut off = hdr_off;
                while off.saturating_add(8) <= entries_end {
                    let text_idx = usize::from(read_u16_le(sh, off)?);
                    let text_off = usize::from(read_u16_le(sh, off + 2)?);
                    let text_len = usize::from(read_u16_le(sh, off + 4)?);
                    col_names.push((text_idx, text_off, text_len));
                    off += 8;
                }
            } else if sig == &SIG_COL_ATTR[..sig_size] {
                let hdr_off = if u64_layout { 16usize } else { 12usize };
                let trl_len = if u64_layout { 12usize } else { 8usize };
                let entry_size = if u64_layout { 16usize } else { 12usize };
                if sh.len() < hdr_off + trl_len {
                    return Err(SasError::Syntax(
                        "sas7bdat ColumnAttributes subheader too short".into(),
                    ));
                }
                let entries_end = sh.len() - trl_len;
                let mut off = hdr_off;
                while off.saturating_add(entry_size) <= entries_end {
                    let (col_off, col_w, type_code) = if u64_layout {
                        let co = usize::try_from(read_u64_le(sh, off)?)
                            .map_err(|_| SasError::Syntax("column offset overflow".into()))?;
                        let cw = usize::try_from(read_u32_le(sh, off + 8)?)
                            .map_err(|_| SasError::Syntax("column width overflow".into()))?;
                        let tc = read_u8(sh, off + 14)?;
                        (co, cw, tc)
                    } else {
                        let co = usize::try_from(read_u32_le(sh, off)?)
                            .map_err(|_| SasError::Syntax("column offset overflow".into()))?;
                        let cw = usize::try_from(read_u32_le(sh, off + 4)?)
                            .map_err(|_| SasError::Syntax("column width overflow".into()))?;
                        let tc = read_u8(sh, off + 10)?;
                        (co, cw, tc)
                    };
                    let col_type = match type_code {
                        1 => ColType::Numeric,
                        2 => ColType::Character,
                        other => {
                            return Err(SasError::Syntax(format!(
                                "invalid sas7bdat column type code {other}"
                            )));
                        }
                    };
                    col_attrs.push((col_off, col_w, col_type));
                    off += entry_size;
                }
            } else if sig == &SIG_COL_FMT[..sig_size] {
                let base_off = if u64_layout { 46usize } else { 34usize };
                if sh.len() < base_off + 12 {
                    return Err(SasError::Syntax(
                        "sas7bdat ColumnFormatAndLabel subheader too short".into(),
                    ));
                }
                let fmt_idx = usize::from(read_u16_le(sh, base_off)?);
                let fmt_off = usize::from(read_u16_le(sh, base_off + 2)?);
                let fmt_len = usize::from(read_u16_le(sh, base_off + 4)?);
                let lbl_idx = usize::from(read_u16_le(sh, base_off + 6)?);
                let lbl_off = usize::from(read_u16_le(sh, base_off + 8)?);
                let lbl_len = usize::from(read_u16_le(sh, base_off + 10)?);
                col_fmt_lbls.push((fmt_idx, fmt_off, fmt_len, lbl_idx, lbl_off, lbl_len));
            } else if sig == &SIG_COUNTS[..sig_size] || sig == &SIG_COL_LIST[..sig_size] {
                // Bookkeeping subheaders not needed to decode uncompressed rows.
            } else {
                return Err(SasError::Unsupported(format!(
                    "GAP: unknown sas7bdat subheader signature {:02x?}",
                    sig
                )));
            }
        }
    }

    let row_length =
        row_length.ok_or_else(|| SasError::Syntax("sas7bdat missing RowSize subheader".into()))?;
    let total_row_count = total_row_count
        .ok_or_else(|| SasError::Syntax("sas7bdat missing total_row_count".into()))?;
    let num_cols = col_count.unwrap_or(col_names.len());
    if col_names.len() != num_cols || col_attrs.len() != num_cols {
        return Err(SasError::Syntax(format!(
            "sas7bdat column metadata count mismatch: size={num_cols}, names={}, attrs={}",
            col_names.len(),
            col_attrs.len()
        )));
    }

    let mut columns = Vec::with_capacity(num_cols);
    for idx in 0..num_cols {
        let &(name_idx, name_off, name_len) = col_names
            .get(idx)
            .ok_or_else(|| SasError::Syntax("missing column name entry".into()))?;
        let name = extract_text(&col_texts, name_idx, name_off, name_len)?;
        let &(offset, width, col_type) = col_attrs
            .get(idx)
            .ok_or_else(|| SasError::Syntax("missing column attribute entry".into()))?;
        if col_type == ColType::Numeric && width != 8 {
            return Err(SasError::Unsupported(format!(
                "GAP: sas7bdat numeric column {name:?} has width {width} (only 8-byte IEEE 754 doubles are supported)"
            )));
        }
        let end = offset
            .checked_add(width)
            .ok_or_else(|| SasError::Syntax("column offset + width overflow".into()))?;
        if end > row_length {
            return Err(SasError::Syntax(format!(
                "sas7bdat column {name:?} ({offset}..{end}) exceeds row_length {row_length}"
            )));
        }
        let (format, label) = if let Some(&(fi, fo, fl, li, lo, ll)) = col_fmt_lbls.get(idx) {
            let fmt = if fl > 0 {
                let s = extract_text(&col_texts, fi, fo, fl)?;
                let trimmed = s.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            } else {
                None
            };
            let lbl = if ll > 0 {
                let s = extract_text(&col_texts, li, lo, ll)?;
                (!s.is_empty()).then_some(s)
            } else {
                None
            };
            (fmt, lbl)
        } else {
            (None, None)
        };
        let num_kind = classify_numeric_format(&name, col_type, format.as_deref())?;
        columns.push(ColMeta {
            name,
            label,
            format,
            offset,
            width,
            col_type,
            num_kind,
        });
    }

    // Pass 2: read uncompressed rows from MIX and DATA pages.
    let mut rows = Vec::with_capacity(total_row_count);
    for page_idx in 0..page_count {
        if rows.len() >= total_row_count {
            break;
        }
        let page_offset = header_length + page_idx * page_size;
        let page = bytes
            .get(page_offset..page_offset + page_size)
            .ok_or_else(|| SasError::Syntax("page slice out of bounds".into()))?;
        let page_type = read_u16_le(page, page_bit_off)?;
        let block_count = usize::from(read_u16_le(page, page_bit_off + 2)?);
        let subheaders_count = usize::from(read_u16_le(page, page_bit_off + 4)?);

        let (data_start, max_rows_on_page) = match page_type {
            PAGE_MIX => {
                let ptrs_end =
                    page_hdr_size.saturating_add(subheaders_count.saturating_mul(ptr_size));
                let aligned = (ptrs_end.saturating_add(7)) & !7;
                (aligned, mix_page_row_count)
            }
            PAGE_DATA => (page_hdr_size, block_count),
            _ => continue,
        };

        let remaining_rows = total_row_count - rows.len();
        let to_read = remaining_rows.min(max_rows_on_page);
        for row_idx in 0..to_read {
            let row_off = data_start
                .checked_add(
                    row_idx
                        .checked_mul(row_length)
                        .ok_or_else(|| SasError::Syntax("row offset overflow".into()))?,
                )
                .ok_or_else(|| SasError::Syntax("row offset overflow".into()))?;
            let row_end = row_off
                .checked_add(row_length)
                .ok_or_else(|| SasError::Syntax("row end overflow".into()))?;
            let row_bytes = page.get(row_off..row_end).ok_or_else(|| {
                SasError::Syntax(format!(
                    "sas7bdat row {} on page {page_idx} ({row_off}..{row_end}) exceeds page size",
                    rows.len()
                ))
            })?;
            let mut record = Vec::with_capacity(columns.len());
            for col in &columns {
                let cell = row_bytes
                    .get(col.offset..col.offset + col.width)
                    .ok_or_else(|| SasError::Syntax("column slice out of bounds".into()))?;
                let value = match col.col_type {
                    ColType::Character => {
                        let s = std::str::from_utf8(cell).map_err(|e| {
                            SasError::Syntax(format!(
                                "column {:?} contains invalid UTF-8: {e}",
                                col.name
                            ))
                        })?;
                        Value::string(s.trim_end_matches(' '))
                    }
                    ColType::Numeric => {
                        let bits = read_u64_le(cell, 0)?;
                        let f = f64::from_bits(bits);
                        if f.is_nan() {
                            Value::Null
                        } else {
                            numeric_value(f, col.num_kind)?
                        }
                    }
                };
                record.push(value);
            }
            rows.push(record);
        }
    }

    if rows.len() != total_row_count {
        return Err(SasError::Syntax(format!(
            "sas7bdat expected {total_row_count} rows, found {}",
            rows.len()
        )));
    }

    build_table(&columns, rows, names)
}

/// Reads a member from a SAS Transport (`XPORT v5`) file into a [`Table`].
pub fn read_xpt(
    bytes: &[u8],
    member_name: Option<&str>,
    names: NameMode,
) -> Result<Table, SasError> {
    if bytes.starts_with(b"HEADER RECORD*******LIBV8")
        || bytes.starts_with(b"HEADER RECORD*******MEMBV8")
    {
        return Err(SasError::Unsupported(
            "GAP: SAS XPORT v8 format is not supported (only XPORT v5)".into(),
        ));
    }
    if bytes.len() < 240 || bytes.get(0..48) != Some(XPT_LIB_HEADER.as_slice()) {
        return Err(SasError::Syntax(
            "not a SAS XPORT v5 file (missing LIBRARY HEADER RECORD)".into(),
        ));
    }

    let wanted = member_name.map(str::trim).filter(|s| !s.is_empty());
    let mut pos = 240usize; // Skip 3 x 80-byte library header records.

    while pos.saturating_add(80) <= bytes.len() {
        let rec = bytes
            .get(pos..pos + 80)
            .ok_or_else(|| SasError::Syntax("truncated XPORT record".into()))?;
        if rec.iter().all(|&b| b == b' ') {
            pos += 80;
            continue;
        }
        if rec.get(0..48) != Some(XPT_MEM_HEADER.as_slice()) {
            return Err(SasError::Syntax(format!(
                "expected MEMBER HEADER RECORD at byte {pos}"
            )));
        }
        let namestr_size = parse_ascii_usize(xpt_field(rec, 74..78)?)?;
        if namestr_size != 140 && namestr_size != 136 {
            return Err(SasError::Unsupported(format!(
                "GAP: XPORT NAMESTR size {namestr_size} is not supported (expected 140)"
            )));
        }
        pos += 80;

        let dsc_hdr = bytes.get(pos..pos + 80).ok_or_else(|| {
            SasError::Syntax("truncated XPORT file before DSCRPTR HEADER RECORD".into())
        })?;
        if dsc_hdr.get(0..48) != Some(XPT_DSC_HEADER.as_slice()) {
            return Err(SasError::Syntax(format!(
                "expected DSCRPTR HEADER RECORD at byte {pos}"
            )));
        }
        pos += 80;

        let dsc_data = bytes
            .get(pos..pos + 160)
            .ok_or_else(|| SasError::Syntax("truncated XPORT member descriptor records".into()))?;
        let current_member = trim_ascii_field(xpt_field(dsc_data, 8..16)?)?;
        pos += 160;

        let nam_hdr = bytes.get(pos..pos + 80).ok_or_else(|| {
            SasError::Syntax("truncated XPORT file before NAMESTR HEADER RECORD".into())
        })?;
        if nam_hdr.get(0..48) != Some(XPT_NAM_HEADER.as_slice()) {
            return Err(SasError::Syntax(format!(
                "expected NAMESTR HEADER RECORD at byte {pos}"
            )));
        }
        let nvars = parse_ascii_usize(xpt_field(nam_hdr, 54..58)?)?;
        if nvars > MAX_COLUMNS {
            return Err(SasError::Syntax(format!(
                "XPORT variable count {nvars} exceeds maximum {MAX_COLUMNS}"
            )));
        }
        pos += 80;

        let namestr_raw_bytes = nvars
            .checked_mul(namestr_size)
            .ok_or_else(|| SasError::Syntax("XPORT NAMESTR byte size overflow".into()))?;
        let namestr_padded_bytes = namestr_raw_bytes
            .checked_add(79)
            .map(|n| (n / 80) * 80)
            .ok_or_else(|| SasError::Syntax("XPORT NAMESTR padded size overflow".into()))?;
        let namestr_block = bytes
            .get(pos..pos.saturating_add(namestr_padded_bytes))
            .ok_or_else(|| SasError::Syntax("truncated XPORT NAMESTR table".into()))?;
        pos += namestr_padded_bytes;

        let obs_hdr = bytes.get(pos..pos + 80).ok_or_else(|| {
            SasError::Syntax("truncated XPORT file before OBS HEADER RECORD".into())
        })?;
        if obs_hdr.get(0..48) != Some(XPT_OBS_HEADER.as_slice()) {
            return Err(SasError::Syntax(format!(
                "expected OBS HEADER RECORD at byte {pos}"
            )));
        }
        pos += 80;

        // Scan forward in 80-byte increments until the next MEMBER HEADER RECORD or EOF.
        let obs_start = pos;
        while pos.saturating_add(80) <= bytes.len() {
            if bytes.get(pos..pos + 48) == Some(XPT_MEM_HEADER.as_slice()) {
                break;
            }
            pos += 80;
        }
        let obs_bytes = bytes
            .get(obs_start..pos)
            .ok_or_else(|| SasError::Syntax("invalid XPORT OBS range".into()))?;

        let matches = match wanted {
            None => true,
            Some(target) => current_member.eq_ignore_ascii_case(target),
        };
        if !matches {
            continue;
        }

        let mut columns = Vec::with_capacity(nvars);
        let mut row_length = 0usize;
        for i in 0..nvars {
            let entry_off = i * namestr_size;
            let entry = namestr_block
                .get(entry_off..entry_off + namestr_size)
                .ok_or_else(|| SasError::Syntax("NAMESTR entry out of bounds".into()))?;
            let ntype = read_i16_be(entry, 0)?;
            let nlng = usize::try_from(read_i16_be(entry, 4)?).map_err(|_| {
                SasError::Syntax("negative variable length in XPORT NAMESTR".into())
            })?;
            let name = trim_ascii_field(xpt_field(entry, 8..16)?)?;
            let label_str = trim_ascii_field(xpt_field(entry, 16..56)?)?;
            let fmt_str = trim_ascii_field(xpt_field(entry, 56..64)?)?;
            let npos = usize::try_from(read_i32_be(entry, 84)?).map_err(|_| {
                SasError::Syntax("negative variable offset in XPORT NAMESTR".into())
            })?;

            let col_type = match ntype {
                1 => ColType::Numeric,
                2 => ColType::Character,
                other => {
                    return Err(SasError::Syntax(format!(
                        "invalid XPORT variable type {other} on {name:?}"
                    )));
                }
            };
            if col_type == ColType::Numeric && nlng != 8 {
                return Err(SasError::Unsupported(format!(
                    "GAP: XPORT numeric variable {name:?} has length {nlng} (only 8-byte IBM-370 floats are supported)"
                )));
            }
            if nlng == 0 {
                return Err(SasError::Syntax(format!(
                    "XPORT variable {name:?} has zero length"
                )));
            }
            let end = npos.checked_add(nlng).ok_or_else(|| {
                SasError::Syntax("XPORT variable offset + length overflow".into())
            })?;
            if end > row_length {
                row_length = end;
            }
            let label = (!label_str.is_empty()).then_some(label_str);
            let format = (!fmt_str.is_empty()).then_some(fmt_str);
            let num_kind = classify_numeric_format(&name, col_type, format.as_deref())?;
            columns.push(ColMeta {
                name,
                label,
                format,
                offset: npos,
                width: nlng,
                col_type,
                num_kind,
            });
        }

        let mut rows = Vec::new();
        if row_length > 0 {
            let mut row_off = 0usize;
            while row_off.saturating_add(row_length) <= obs_bytes.len() {
                let tail = &obs_bytes[row_off..];
                // The final 80-byte card image in an OBS block is padded with ASCII spaces.
                if tail.len() < 80
                    && tail.iter().all(|&b| b == b' ')
                    && columns.iter().any(|c| c.col_type == ColType::Numeric)
                {
                    break;
                }
                let row_slice = &obs_bytes[row_off..row_off + row_length];
                let mut record = Vec::with_capacity(columns.len());
                for col in &columns {
                    let cell = row_slice
                        .get(col.offset..col.offset + col.width)
                        .ok_or_else(|| SasError::Syntax("XPORT cell out of bounds".into()))?;
                    let value = match col.col_type {
                        ColType::Character => {
                            let s = std::str::from_utf8(cell).map_err(|e| {
                                SasError::Syntax(format!(
                                    "XPORT column {:?} contains invalid UTF-8: {e}",
                                    col.name
                                ))
                            })?;
                            Value::string(s.trim_end_matches(' '))
                        }
                        ColType::Numeric => {
                            let mut buf = [0u8; 8];
                            buf.copy_from_slice(cell);
                            match ibm370_to_f64(buf) {
                                None => Value::Null,
                                Some(f) => numeric_value(f, col.num_kind)?,
                            }
                        }
                    };
                    record.push(value);
                }
                rows.push(record);
                row_off += row_length;
            }
        }

        return build_table(&columns, rows, names);
    }

    Err(SasError::MemberNotFound(
        wanted.unwrap_or("<first>").to_string(),
    ))
}

/// Converts an 8-byte IBM System/370 hexadecimal floating-point number to IEEE 754 `f64`,
/// returning `None` for SAS system/special missing values (`.`, `._`, `.A`–`.Z`).
fn ibm370_to_f64(bytes: [u8; 8]) -> Option<f64> {
    if bytes == [0u8; 8] {
        return Some(0.0);
    }
    // In XPORT v5, missing values have a non-zero marker in byte 0 ('.' = 0x2e, '_' = 0x5f,
    // 'A'..='Z' = 0x41..=0x5a) and all zeros in bytes 1..8. A valid non-zero IBM-370 float
    // never has a zero mantissa.
    let mantissa = u64::from_be_bytes([
        0, bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]);
    if mantissa == 0 {
        return None;
    }
    let negative = (bytes[0] & 0x80) != 0;
    let exp16 = i32::from(bytes[0] & 0x7f) - 64;
    let value = (mantissa as f64) * 2f64.powi(4 * exp16 - 56);
    Some(if negative { -value } else { value })
}

fn classify_numeric_format(
    col_name: &str,
    col_type: ColType,
    format: Option<&str>,
) -> Result<NumKind, SasError> {
    if col_type != ColType::Numeric {
        return Ok(NumKind::Real);
    }
    let Some(raw) = format.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(NumKind::Real);
    };
    // Strip trailing width/decimals if present (e.g. "DATE9." -> "DATE").
    let base: String = raw
        .trim_end_matches('.')
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .to_ascii_uppercase();
    match base.as_str() {
        "" | "BEST" | "F" | "COMMA" | "DOLLAR" | "PERCENT" | "Z" | "E" | "EURO" | "EUR" => {
            Ok(NumKind::Real)
        }
        "DATE" | "DAY" | "DDMMYY" | "MMDDYY" | "YYMMDD" | "JULIAN" | "EURDFDD" | "EURDFDE"
        | "EURDFMY" | "IS8601DA" | "B8601DA" | "E8601DA" | "MINGUO" | "NLDATE" => Ok(NumKind::Date),
        "TIME" | "TOD" | "HHMM" | "MMSS" | "DATETIME" | "DTATE" | "DTMONYY" | "DTWKDATX"
        | "DTYEAR" | "IS8601DT" | "IS8601TM" | "B8601DT" | "E8601DT" | "B8601TM" | "E8601TM"
        | "NLDATM" | "NLTIMAP" => Err(SasError::Unsupported(format!(
            "GAP: SAS time/datetime format {raw:?} on column {col_name:?} is not supported"
        ))),
        _ => Err(SasError::Unsupported(format!(
            "GAP: SAS format {raw:?} on numeric column {col_name:?} is not supported"
        ))),
    }
}

fn numeric_value(value: f64, kind: NumKind) -> Result<Value, SasError> {
    match kind {
        NumKind::Real => Ok(Value::real(value)),
        NumKind::Date => {
            if !value.is_finite() || value < (i32::MIN as f64) || value > (i32::MAX as f64) {
                return Err(SasError::Syntax(format!(
                    "SAS date value {value} is out of range"
                )));
            }
            let sas_days = value.floor() as i64;
            let unix_days = sas_days
                .checked_add(SAS_EPOCH_UNIX_DAYS)
                .ok_or_else(|| SasError::Syntax("SAS date day count overflow".into()))?;
            Ok(Value::Date(unix_days))
        }
    }
}

fn build_table(
    columns: &[ColMeta],
    rows: Vec<Vec<Value>>,
    names: NameMode,
) -> Result<Table, SasError> {
    let mut fields = Vec::with_capacity(columns.len());
    for (idx, col) in columns.iter().enumerate() {
        let storage = match (col.col_type, col.num_kind) {
            (ColType::Character, _) => Storage::String,
            (ColType::Numeric, NumKind::Date) => Storage::Date,
            (ColType::Numeric, NumKind::Real) => Storage::Real,
        };
        let field_name = match names {
            NameMode::NamesAndLabels => col.name.clone(),
            NameMode::LabelsAsNames => match col.label.as_deref().filter(|l| !l.is_empty()) {
                Some(lbl) => lbl.to_string(),
                None => format!("field{}", idx + 1),
            },
        };
        let mut field = Field::new(field_name, storage);
        field.measurement = match storage {
            Storage::String => Measurement::Categorical,
            _ => Measurement::Continuous,
        };
        field.label = col.label.clone().filter(|l| !l.is_empty());
        let _ = &col.format;
        fields.push(field);
    }
    let model = DataModel::new(fields).map_err(SasError::Model)?;
    Table::new(model, rows).map_err(SasError::Model)
}

fn extract_text(
    texts: &[Vec<u8>],
    idx: usize,
    offset: usize,
    length: usize,
) -> Result<String, SasError> {
    let blob = texts.get(idx).ok_or_else(|| {
        SasError::Syntax(format!(
            "sas7bdat ColumnText index {idx} out of bounds (have {})",
            texts.len()
        ))
    })?;
    let end = offset
        .checked_add(length)
        .ok_or_else(|| SasError::Syntax("ColumnText slice end overflow".into()))?;
    let slice = blob.get(offset..end).ok_or_else(|| {
        SasError::Syntax(format!(
            "sas7bdat ColumnText slice {offset}..{end} out of bounds for {}-byte blob",
            blob.len()
        ))
    })?;
    std::str::from_utf8(slice)
        .map(str::to_string)
        .map_err(|e| SasError::Syntax(format!("sas7bdat metadata text is not valid UTF-8: {e}")))
}

fn trim_ascii_field(bytes: &[u8]) -> Result<String, SasError> {
    let text = std::str::from_utf8(bytes).map_err(|e| {
        SasError::Syntax(format!("XPORT header field is not valid ASCII/UTF-8: {e}"))
    })?;
    Ok(text.trim_end_matches([' ', '\0']).to_string())
}

/// A fixed-position field of an XPORT record; a short record is malformed, not defaulted.
fn xpt_field(record: &[u8], range: std::ops::Range<usize>) -> Result<&[u8], SasError> {
    record
        .get(range.clone())
        .ok_or_else(|| SasError::Syntax(format!("XPORT record too short for bytes {range:?}")))
}

fn parse_ascii_usize(bytes: &[u8]) -> Result<usize, SasError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| SasError::Syntax(format!("XPORT numeric field is not ASCII: {e}")))?
        .trim();
    if text.is_empty() {
        return Ok(0);
    }
    text.parse::<usize>()
        .map_err(|e| SasError::Syntax(format!("invalid XPORT integer {text:?}: {e}")))
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

fn read_u8(bytes: &[u8], offset: usize) -> Result<u8, SasError> {
    bytes
        .get(offset)
        .copied()
        .ok_or_else(|| SasError::Syntax(format!("unexpected EOF at byte {offset}")))
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Result<u16, SasError> {
    let s = bytes
        .get(offset..offset.saturating_add(2))
        .ok_or_else(|| SasError::Syntax(format!("unexpected EOF at byte {offset}")))?;
    let mut b = [0u8; 2];
    b.copy_from_slice(s);
    Ok(u16::from_le_bytes(b))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, SasError> {
    let s = bytes
        .get(offset..offset.saturating_add(4))
        .ok_or_else(|| SasError::Syntax(format!("unexpected EOF at byte {offset}")))?;
    let mut b = [0u8; 4];
    b.copy_from_slice(s);
    Ok(u32::from_le_bytes(b))
}

fn read_u64_le(bytes: &[u8], offset: usize) -> Result<u64, SasError> {
    let s = bytes
        .get(offset..offset.saturating_add(8))
        .ok_or_else(|| SasError::Syntax(format!("unexpected EOF at byte {offset}")))?;
    let mut b = [0u8; 8];
    b.copy_from_slice(s);
    Ok(u64::from_le_bytes(b))
}

fn read_i16_be(bytes: &[u8], offset: usize) -> Result<i16, SasError> {
    let s = bytes
        .get(offset..offset.saturating_add(2))
        .ok_or_else(|| SasError::Syntax(format!("unexpected EOF at byte {offset}")))?;
    let mut b = [0u8; 2];
    b.copy_from_slice(s);
    Ok(i16::from_be_bytes(b))
}

fn read_i32_be(bytes: &[u8], offset: usize) -> Result<i32, SasError> {
    let s = bytes
        .get(offset..offset.saturating_add(4))
        .ok_or_else(|| SasError::Syntax(format!("unexpected EOF at byte {offset}")))?;
    let mut b = [0u8; 4];
    b.copy_from_slice(s);
    Ok(i32::from_be_bytes(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ibm370_float_decodes_known_values_and_missings() {
        assert_eq!(ibm370_to_f64([0; 8]), Some(0.0));
        assert_eq!(
            ibm370_to_f64([0x2e, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
            None
        );
        assert_eq!(
            ibm370_to_f64([0x41, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
            Some(1.0)
        );
        assert_eq!(
            ibm370_to_f64([0x41, 0xc8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
            Some(12.5)
        );
        assert_eq!(
            ibm370_to_f64([0xc1, 0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
            Some(-1.5)
        );
        assert_eq!(
            ibm370_to_f64([0x44, 0x55, 0x9b, 0x00, 0x00, 0x00, 0x00, 0x00]),
            Some(21915.0)
        );
    }

    #[test]
    fn rejects_malformed_and_unsupported_inputs_without_panicking() {
        assert!(matches!(
            read_sas7bdat(&[], NameMode::NamesAndLabels),
            Err(SasError::Syntax(_))
        ));
        assert!(matches!(
            read_sas7bdat(&[0u8; 512], NameMode::NamesAndLabels),
            Err(SasError::Syntax(_))
        ));

        // 32-bit sas7bdat header is rejected as an explicit GAP.
        let mut hdr = vec![0u8; 300];
        hdr[0..32].copy_from_slice(&SAS7BDAT_MAGIC);
        hdr[32] = 0x32;
        assert!(matches!(
            read_sas7bdat(&hdr, NameMode::NamesAndLabels),
            Err(SasError::Unsupported(msg)) if msg.contains("32-bit")
        ));

        // Big-endian sas7bdat header is rejected as an explicit GAP.
        hdr[32] = 0x33;
        hdr[35] = 0x33;
        hdr[37] = 0x00;
        assert!(matches!(
            read_sas7bdat(&hdr, NameMode::NamesAndLabels),
            Err(SasError::Unsupported(msg)) if msg.contains("big-endian")
        ));

        // XPORT v8 is rejected as an explicit GAP.
        let v8 =
            b"HEADER RECORD*******LIBV8   HEADER RECORD!!!!!!!000000000000000000000000000000  ";
        assert!(matches!(
            read_xpt(v8, None, NameMode::NamesAndLabels),
            Err(SasError::Unsupported(msg)) if msg.contains("v8")
        ));

        // Truncated XPORT v5 is rejected cleanly.
        assert!(matches!(
            read_xpt(b"not an xpt", None, NameMode::NamesAndLabels),
            Err(SasError::Syntax(_))
        ));
    }

    #[test]
    fn reads_corpus_sas7bdat_and_xpt() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/sas");
        let sas7bdat_path = root.join("sastest.sas7bdat");
        if !sas7bdat_path.is_file() {
            return;
        }
        let sas_bytes = std::fs::read(&sas7bdat_path).unwrap();

        // NamesAndLabels mode (sas9_names)
        let table = read_sas7bdat(&sas_bytes, NameMode::NamesAndLabels).unwrap();
        let names: Vec<&str> = table
            .model()
            .fields()
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["name", "grp", "joined", "amount", "id", "score"]
        );
        assert_eq!(
            table.model().field("name").unwrap().label.as_deref(),
            Some("Customer name")
        );
        assert_eq!(table.model().field("grp").unwrap().label, None);
        assert_eq!(
            table.model().field("joined").unwrap().storage,
            Storage::Date
        );
        assert_eq!(table.row_count(), 5);
        assert_eq!(
            table.rows()[0],
            vec![
                Value::string("Alice"),
                Value::string("A"),
                Value::Date(18262), // 2020-01-01
                Value::Real(12.5),
                Value::Real(1.0),
                Value::Real(0.25),
            ]
        );
        assert_eq!(table.rows()[1][3], Value::Null);
        assert_eq!(table.rows()[2][2], Value::Null);
        assert_eq!(table.rows()[3][5], Value::Null);

        // LabelsAsNames mode (sas9_labels)
        let labeled = read_sas7bdat(&sas_bytes, NameMode::LabelsAsNames).unwrap();
        let label_names: Vec<&str> = labeled
            .model()
            .fields()
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(
            label_names,
            vec![
                "Customer name",
                "field2",
                "Join date",
                "Amount (EUR)",
                "field5",
                "field6",
            ]
        );

        // XPORT v5 (sas_transport)
        let xpt_bytes = std::fs::read(root.join("sastest.xpt")).unwrap();
        let xpt = read_xpt(&xpt_bytes, Some("SASTEST"), NameMode::NamesAndLabels).unwrap();
        let xpt_names: Vec<&str> = xpt
            .model()
            .fields()
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(
            xpt_names,
            vec!["NAME", "GRP", "JOINED", "AMOUNT", "ID", "SCORE"]
        );
        assert_eq!(xpt.rows(), table.rows());
        assert!(matches!(
            read_xpt(&xpt_bytes, Some("NO_SUCH_MEMBER"), NameMode::NamesAndLabels),
            Err(SasError::MemberNotFound(_))
        ));
    }
}
