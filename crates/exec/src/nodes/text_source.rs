//! Settings shared by the text sources, Var. File (`var_file`) and Fixed File (`fixed_file`):
//! `fileEncoding` (`clientDefault`, `serverDefault`, or a WHATWG label), `invalidCharacterMode`
//! (`discard`, `replace` with `invalidCharacterReplacement`), `trimMode` (`none`, `left`,
//! `right`, `both`), `scanLineCount` (clamped `>= 1`), `autoRecognizeDates`, `realDecimalSymbol`
//! (`default` uses the stream's, `period`, `comma`) and the `textImportTable/rows/row` storage
//! overrides with their `dateFormat` and `timeFormat`. At run time the stream's
//! `importDatetimeAsString`, `dateFormat` and `date2DigitBaseline` apply.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::settings;
use dms_io::text::{
    DateFormat, DecimalSymbol, InvalidCharacters, StorageOverride, TextEncoding, TextOptions,
    TimeFormat, Trim,
};
use dms_strfmt::fastinfoset::Element;
use dms_types::Storage;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy)]
enum DecimalChoice {
    StreamDefault,
    Fixed(DecimalSymbol),
}

/// The node's text settings, completed by the stream's at run time.
pub(crate) struct TextSettings {
    options: TextOptions,
    decimal: DecimalChoice,
}

impl TextSettings {
    /// Reads the settings of node `node` (for messages); `trim` is used without `trimMode`.
    pub(crate) fn read(s: Option<&Element>, node: &str, trim: Trim) -> Result<Self, OperatorError> {
        let bad = |what: String| OperatorError::Settings(format!("{node}: {what}"));
        let scan_raw = settings::integer(s, "scanLineCount")?.unwrap_or(50);
        if scan_raw < 0 {
            return Err(bad(format!(
                "scanLineCount must be non-negative, got {scan_raw}"
            )));
        }
        let decimal = match settings::text(s, "realDecimalSymbol")
            .as_deref()
            .map(str::trim)
        {
            None | Some("") | Some("default") => DecimalChoice::StreamDefault,
            Some("period") => DecimalChoice::Fixed(DecimalSymbol::Period),
            Some("comma") => DecimalChoice::Fixed(DecimalSymbol::Comma),
            Some(other) => return Err(bad(format!("unknown realDecimalSymbol {other:?}"))),
        };
        let trim = match settings::text(s, "trimMode").as_deref().map(str::trim) {
            None | Some("") => trim,
            Some("none") => Trim::None,
            Some("left") => Trim::Left,
            Some("right") => Trim::Right,
            Some("both") => Trim::Both,
            Some(other) => return Err(bad(format!("unknown trimMode {other:?}"))),
        };
        let invalid = match settings::text(s, "invalidCharacterMode")
            .as_deref()
            .map(str::trim)
        {
            None | Some("") | Some("discard") => InvalidCharacters::Discard,
            Some("replace") => InvalidCharacters::Replace(
                settings::text(s, "invalidCharacterReplacement").unwrap_or_default(),
            ),
            Some(other) => return Err(bad(format!("unknown invalidCharacterMode {other:?}"))),
        };
        let encoding = match settings::text(s, "fileEncoding").as_deref().map(str::trim) {
            None | Some("") | Some("clientDefault") | Some("serverDefault") => TextEncoding::Auto,
            Some(label) if TextEncoding::is_valid_label(label) => {
                TextEncoding::Label(label.to_owned())
            }
            Some(label) => return Err(bad(format!("unknown fileEncoding {label:?}"))),
        };
        Ok(TextSettings {
            options: TextOptions {
                encoding,
                invalid,
                skip_lines: 0,
                trim,
                scan_lines: usize::try_from(scan_raw).unwrap_or(50).max(1),
                decimal: DecimalSymbol::Period,
                recognize_dates: settings::flag(s, "autoRecognizeDates")?.unwrap_or(true),
                date_format: DateFormat::YyyyMmDdDash,
                time_format: None,
                two_digit_year_baseline: 1930,
                overrides: storage_overrides(s, node)?,
            },
            decimal,
        })
    }

    /// Lines (or records) skipped before the data.
    pub(crate) fn skip_lines(&mut self, lines: usize) {
        self.options.skip_lines = lines;
    }

    /// The options with the stream's settings applied.
    pub(crate) fn resolve(&self, context: &ExecContext) -> Result<TextOptions, OperatorError> {
        let sp = context.stream_properties.as_deref();
        let mut options = self.options.clone();
        options.decimal = match self.decimal {
            DecimalChoice::Fixed(symbol) => symbol,
            DecimalChoice::StreamDefault => {
                match settings::text(sp, "realDecimalSymbol")
                    .as_deref()
                    .map(str::trim)
                {
                    Some("comma") => DecimalSymbol::Comma,
                    _ => DecimalSymbol::Period,
                }
            }
        };
        if settings::flag(sp, "importDatetimeAsString")?.unwrap_or(false) {
            options.recognize_dates = false;
        }
        if let Some(id) = settings::text(sp, "dateFormat") {
            // Reading dates in a format we do not know would give wrong dates; fail instead.
            options.date_format = DateFormat::from_modeler_id(id.trim()).ok_or_else(|| {
                OperatorError::Settings(format!(
                    "the stream date format {id:?} is not supported yet"
                ))
            })?;
        }
        if let Some(base) = settings::integer(sp, "date2DigitBaseline")?
            && let Ok(base) = i32::try_from(base)
        {
            options.two_digit_year_baseline = base;
        }
        Ok(options)
    }
}

/// The `textImportTable` rows that override a field's storage.
fn storage_overrides(
    s: Option<&Element>,
    node: &str,
) -> Result<Vec<StorageOverride>, OperatorError> {
    let bad = |what: String| OperatorError::Settings(format!("{node}: {what}"));
    let mut overrides = Vec::new();
    for row in settings::children(s, "textImportTable/rows", "row") {
        let name = row.child("name").map(|e| e.text()).unwrap_or_default();
        if name.is_empty() || !settings::flag(Some(row), "override")?.unwrap_or(true) {
            continue;
        }
        let storage = match settings::text(Some(row), "storage")
            .as_deref()
            .map(str::trim)
        {
            None | Some("") | Some("unknown") => continue,
            Some("string") => Storage::String,
            Some("integer") => Storage::Integer,
            Some("real") => Storage::Real,
            Some("date") => Storage::Date,
            Some("time") => Storage::Time,
            Some("timestamp") => Storage::Timestamp,
            Some("list") => {
                return Err(bad(format!(
                    "list storage on field {name:?} is not supported yet"
                )));
            }
            Some(other) => return Err(bad(format!("unknown storage {other:?} on field {name:?}"))),
        };
        let format = |key: &str| {
            settings::text(Some(row), key)
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
        };
        let date_format = format("dateFormat")
            .map(|id| {
                DateFormat::from_modeler_id(&id)
                    .ok_or_else(|| bad(format!("unsupported dateFormat {id:?} on field {name:?}")))
            })
            .transpose()?;
        let time_format = format("timeFormat")
            .map(|id| {
                TimeFormat::from_modeler_id(&id)
                    .ok_or_else(|| bad(format!("unsupported timeFormat {id:?} on field {name:?}")))
            })
            .transpose()?;
        overrides.push(StorageOverride {
            field: name,
            storage,
            date_format,
            time_format,
        });
    }
    Ok(overrides)
}

/// The path and bytes of the node's data file.
pub(crate) fn read_file(
    context: &ExecContext,
    name: &str,
) -> Result<(PathBuf, Vec<u8>), OperatorError> {
    let resolved = context
        .files
        .resolve(name)
        .ok_or_else(|| OperatorError::Io(format!("cannot find data file {name:?}")))?;
    let path = resolved.path().to_path_buf();
    let bytes =
        std::fs::read(&path).map_err(|e| OperatorError::Io(format!("{}: {e}", path.display())))?;
    Ok((path, bytes))
}
