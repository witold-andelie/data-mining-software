//! Var. File (Sources palette, `DelimitedFileImportNode`): reads a delimited text file into a
//! [`Table`] using [`dms_io::delimited`].
//!
//! Implemented: `inputFileName` (resolved via [`ExecContext::files`]), `skipHeaderCount`,
//! `useManualFieldCount`/`manualFieldCount`, `delimitOnComma`/`delimitOnTab`/`delimitOnSpace`/
//! `delimitOnNonPrintChars`, `useDelimiterChars`/`delimiterChars`, `fieldNamesIncluded`,
//! `singleQuoteMode`/`doubleQuoteMode` (`pairDiscard`, `discard`, `include`), the text settings
//! of `super::text_source` (no trimming by default), and embedded `filterTable`/`typeTable`
//! rules.
//!
//! `multiBlankDelimiters` joins adjacent blank delimiters, so it has no effect unless a space or
//! a tab is a delimiter: with comma delimiters only it reads the same table (`ibm-demos`
//! `drugplot`, `goodsplot`, `goodslearn`, `cplex_goodslearn` match Modeler). With blank
//! delimiters it is a `GAP:` (no oracle).
//!
//! Not implemented: `treatSquareBracketsAsLists`, `eolCommentChars`, `list` storage in
//! `textImportTable`, and non-`None` `check` modes in `typeTable`.

use super::text_source::{TextSettings, read_file};
use super::r#type::{SourceTables, apply_source_tables, parse_source_tables};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_io::delimited::{DelimitedOptions, QuoteMode, read_table};
use dms_io::text::Trim;
use dms_stream::StreamNode;
use dms_types::Table;
use std::sync::Arc;

struct VarFile {
    file_name: String,
    options: DelimitedOptions,
    text: TextSettings,
    tables: SourceTables,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let file_name = settings::text(s, "inputFileName")
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| OperatorError::Settings("Var. File: inputFileName is required".into()))?;

    if settings::flag(s, "multiBlankDelimiters")?.unwrap_or(false)
        && (settings::flag(s, "delimitOnSpace")?.unwrap_or(false)
            || settings::flag(s, "delimitOnTab")?.unwrap_or(false)
            || settings::text(s, "delimiterChars").is_some_and(|chars| chars.contains([' ', '\t'])))
    {
        return Err(OperatorError::Settings(
            "Var. File: GAP: multiBlankDelimiters with space or tab delimiters has no oracle"
                .into(),
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

    let use_other = settings::flag(s, "useDelimiterChars")?.unwrap_or(false);
    let other = if use_other {
        settings::text(s, "delimiterChars")
            .unwrap_or_default()
            .chars()
            .collect()
    } else {
        Vec::new()
    };

    let mut text = TextSettings::read(s, "Var. File", Trim::None)?;
    text.skip_lines(skip_lines);
    // `text` is replaced at run time by the text settings completed with the stream's.
    let options = DelimitedOptions {
        comma: settings::flag(s, "delimitOnComma")?.unwrap_or(true),
        tab: settings::flag(s, "delimitOnTab")?.unwrap_or(false),
        space: settings::flag(s, "delimitOnSpace")?.unwrap_or(false),
        non_printing: settings::flag(s, "delimitOnNonPrintChars")?.unwrap_or(false),
        other,
        field_names_included: settings::flag(s, "fieldNamesIncluded")?.unwrap_or(true),
        manual_field_count,
        single_quote: parse_quote_mode("singleQuoteMode", settings::text(s, "singleQuoteMode"))?,
        double_quote: parse_quote_mode("doubleQuoteMode", settings::text(s, "doubleQuoteMode"))?,
        text: Default::default(),
    };

    Ok(Box::new(VarFile {
        file_name,
        options,
        text,
        tables: parse_source_tables(s)?,
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
        let (path, bytes) = read_file(context, &self.file_name)?;
        let options = DelimitedOptions {
            text: self.text.resolve(context)?,
            ..self.options.clone()
        };
        let table = read_table(&bytes, &options)
            .map_err(|e| OperatorError::Data(format!("{}: {e}", path.display())))?;
        apply_source_tables(&self.tables, table)
    }
}
