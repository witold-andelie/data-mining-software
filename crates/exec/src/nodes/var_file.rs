//! Var. File (Sources palette, `DelimitedFileImportNode`): reads a delimited text file into a
//! [`Table`] using [`dms_io::delimited`].
//!
//! Implemented: `inputFileName` (resolved via [`ExecContext::files`]), `skipHeaderCount`,
//! `useManualFieldCount`/`manualFieldCount`, `delimitOnComma`/`delimitOnTab`/`delimitOnSpace`/
//! `delimitOnNonPrintChars`, `useDelimiterChars`/`delimiterChars`, `realDecimalSymbol` (`default`
//! uses the stream's `realDecimalSymbol`, `period`, `comma`), `fieldNamesIncluded`, `trimMode`
//! (`none`, `left`, `right`, `both`), `invalidCharacterMode` (`discard`, `replace` with
//! `invalidCharacterReplacement`), `scanLineCount` (clamped `>= 1`), `autoRecognizeDates`
//! (combined with the stream's `importDatetimeAsString`, `dateFormat` and `date2DigitBaseline`),
//! `singleQuoteMode`/`doubleQuoteMode` (`pairDiscard`, `discard`, `include`), `fileEncoding`
//! (`clientDefault`, `serverDefault`, or a WHATWG label), `textImportTable/rows/row` storage
//! overrides, and embedded `filterTable`/`typeTable` rules.
//!
//! Not implemented: `multiBlankDelimiters`, `treatSquareBracketsAsLists`, `eolCommentChars`,
//! `list` storage in `textImportTable`, and non-`None` `check` modes in `typeTable`.
//! GAP: a per-field `dateFormat` in `textImportTable` is validated but dates are read in the
//! stream's format.

use super::r#type::{SourceTables, apply_source_tables, parse_source_tables};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_io::delimited::{
    DateFormat, DecimalSymbol, DelimitedOptions, InvalidCharacters, QuoteMode, StorageOverride,
    TextEncoding, Trim, read_table,
};
use dms_stream::StreamNode;
use dms_types::{Storage, Table};
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
enum DecimalChoice {
    StreamDefault,
    Fixed(DecimalSymbol),
}

struct VarFile {
    file_name: String,
    options: DelimitedOptions,
    decimal_choice: DecimalChoice,
    tables: SourceTables,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let file_name = settings::text(s, "inputFileName")
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| OperatorError::Settings("Var. File: inputFileName is required".into()))?;

    if settings::flag(s, "multiBlankDelimiters")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "Var. File: multiBlankDelimiters is not supported yet".into(),
        ));
    }
    if settings::flag(s, "treatSquareBracketsAsLists")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "Var. File: treatSquareBracketsAsLists is not supported yet".into(),
        ));
    }
    if settings::text(s, "eolCommentChars").is_some_and(|chars| !chars.is_empty()) {
        return Err(OperatorError::Settings(
            "Var. File: eolCommentChars is not supported yet".into(),
        ));
    }

    let skip_raw = settings::integer(s, "skipHeaderCount")?.unwrap_or(0);
    let skip_lines = usize::try_from(skip_raw).map_err(|_| {
        OperatorError::Settings(format!(
            "Var. File: skipHeaderCount must be non-negative, got {skip_raw}"
        ))
    })?;

    let use_manual_count = settings::flag(s, "useManualFieldCount")?.unwrap_or(false);
    let manual_raw = settings::integer(s, "manualFieldCount")?.unwrap_or(1);
    if manual_raw < 0 {
        return Err(OperatorError::Settings(format!(
            "Var. File: manualFieldCount must be non-negative, got {manual_raw}"
        )));
    }
    let manual_field_count = if use_manual_count {
        let count = usize::try_from(manual_raw).unwrap_or(1).max(1);
        Some(count)
    } else {
        None
    };

    let scan_raw = settings::integer(s, "scanLineCount")?.unwrap_or(50);
    if scan_raw < 0 {
        return Err(OperatorError::Settings(format!(
            "Var. File: scanLineCount must be non-negative, got {scan_raw}"
        )));
    }
    let scan_lines = usize::try_from(scan_raw).unwrap_or(50).max(1);

    let comma = settings::flag(s, "delimitOnComma")?.unwrap_or(true);
    let tab = settings::flag(s, "delimitOnTab")?.unwrap_or(false);
    let space = settings::flag(s, "delimitOnSpace")?.unwrap_or(false);
    let non_printing = settings::flag(s, "delimitOnNonPrintChars")?.unwrap_or(false);
    let use_other = settings::flag(s, "useDelimiterChars")?.unwrap_or(false);
    let other = if use_other {
        settings::text(s, "delimiterChars")
            .unwrap_or_default()
            .chars()
            .collect()
    } else {
        Vec::new()
    };

    let decimal_choice = match settings::text(s, "realDecimalSymbol")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("default") => DecimalChoice::StreamDefault,
        Some("period") => DecimalChoice::Fixed(DecimalSymbol::Period),
        Some("comma") => DecimalChoice::Fixed(DecimalSymbol::Comma),
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Var. File: unknown realDecimalSymbol {other:?}"
            )));
        }
    };

    let field_names_included = settings::flag(s, "fieldNamesIncluded")?.unwrap_or(true);
    let trim = match settings::text(s, "trimMode").as_deref().map(str::trim) {
        None | Some("") | Some("none") => Trim::None,
        Some("left") => Trim::Left,
        Some("right") => Trim::Right,
        Some("both") => Trim::Both,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Var. File: unknown trimMode {other:?}"
            )));
        }
    };

    let invalid = match settings::text(s, "invalidCharacterMode")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("discard") => InvalidCharacters::Discard,
        Some("replace") => InvalidCharacters::Replace(
            settings::text(s, "invalidCharacterReplacement").unwrap_or_default(),
        ),
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Var. File: unknown invalidCharacterMode {other:?}"
            )));
        }
    };

    let recognize_dates = settings::flag(s, "autoRecognizeDates")?.unwrap_or(true);
    let single_quote = parse_quote_mode("singleQuoteMode", settings::text(s, "singleQuoteMode"))?;
    let double_quote = parse_quote_mode("doubleQuoteMode", settings::text(s, "doubleQuoteMode"))?;

    let encoding = match settings::text(s, "fileEncoding").as_deref().map(str::trim) {
        None | Some("") | Some("clientDefault") | Some("serverDefault") => TextEncoding::Auto,
        Some(label) => {
            if !TextEncoding::is_valid_label(label) {
                return Err(OperatorError::Settings(format!(
                    "Var. File: unknown fileEncoding {label:?}"
                )));
            }
            TextEncoding::Label(label.to_owned())
        }
    };

    let mut overrides = Vec::new();
    for row in settings::children(s, "textImportTable/rows", "row") {
        let name = row.child("name").map(|e| e.text()).unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let is_override = settings::flag(Some(row), "override")?.unwrap_or(true);
        if !is_override {
            continue;
        }
        let storage = match settings::text(Some(row), "storage")
            .as_deref()
            .map(str::trim)
        {
            None | Some("") | Some("unknown") => Storage::Unknown,
            Some("string") => Storage::String,
            Some("integer") => Storage::Integer,
            Some("real") => Storage::Real,
            Some("date") => Storage::Date,
            Some("time") => Storage::Time,
            Some("timestamp") => Storage::Timestamp,
            Some("list") => {
                return Err(OperatorError::Settings(format!(
                    "Var. File: list storage on field {name:?} is not supported yet"
                )));
            }
            Some(other) => {
                return Err(OperatorError::Settings(format!(
                    "Var. File: unknown storage {other:?} on field {name:?}"
                )));
            }
        };
        if let Some(df) = settings::text(Some(row), "dateFormat")
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            && DateFormat::from_modeler_id(df).is_none()
        {
            return Err(OperatorError::Settings(format!(
                "Var. File: unsupported dateFormat {df:?} on field {name:?}"
            )));
        }
        if storage != Storage::Unknown {
            overrides.push(StorageOverride {
                field: name,
                storage,
            });
        }
    }

    let tables = parse_source_tables(s)?;
    let options = DelimitedOptions {
        comma,
        tab,
        space,
        non_printing,
        other,
        field_names_included,
        skip_lines,
        manual_field_count,
        scan_lines,
        single_quote,
        double_quote,
        trim,
        decimal: DecimalSymbol::Period,
        encoding,
        invalid,
        recognize_dates,
        date_format: DateFormat::YyyyMmDdDash,
        two_digit_year_baseline: 1930,
        overrides,
    };

    Ok(Box::new(VarFile {
        file_name,
        options,
        decimal_choice,
        tables,
    }))
}

fn parse_quote_mode(setting: &str, value: Option<String>) -> Result<QuoteMode, OperatorError> {
    match value.as_deref().map(str::trim) {
        None | Some("") | Some("pairDiscard") => Ok(QuoteMode::PairAndDiscard),
        Some("discard") => Ok(QuoteMode::Discard),
        Some("include") => Ok(QuoteMode::Include),
        Some(other) => Err(OperatorError::Settings(format!(
            "Var. File: unknown {setting} {other:?}"
        ))),
    }
}

impl Operator for VarFile {
    fn arity(&self) -> Arity {
        Arity::Exactly(0)
    }

    fn run(&self, _inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let resolved = context.files.resolve(&self.file_name).ok_or_else(|| {
            OperatorError::Io(format!("cannot find data file {:?}", self.file_name))
        })?;
        let bytes = std::fs::read(resolved.path())
            .map_err(|e| OperatorError::Io(format!("{}: {e}", resolved.path().display())))?;

        let sp = context.stream_properties.as_deref();
        let mut options = self.options.clone();
        options.decimal = match self.decimal_choice {
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
                    "Var. File: the stream date format {id:?} is not supported yet"
                ))
            })?;
        }
        if let Some(base) = settings::integer(sp, "date2DigitBaseline")?
            && let Ok(base_i32) = i32::try_from(base)
        {
            options.two_digit_year_baseline = base_i32;
        }

        let table = read_table(&bytes, &options)
            .map_err(|e| OperatorError::Data(format!("{}: {e}", resolved.path().display())))?;
        apply_source_tables(&self.tables, table)
    }
}
