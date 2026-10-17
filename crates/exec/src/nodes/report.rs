//! Report (Output palette, `ReportNode`): text from a template with CLEM expressions
//! (`oracle/misc/misc_rep_*`). The document holds one row per line of the report; in file mode
//! the node also writes the file Modeler writes.
//!
//! Settings: `reportText` (the template), `reportHighlightInserted` (default true),
//! `reportFormatMode` (`Auto` default, `Custom`), `outputMode` (`screen`, `file`),
//! `reportOutputFileType` (`html` default, `text`), `outputFilePath`, `reportTitle`,
//! `outputLinesPerPage` (default 400). Rejected: the `output_object` file type.
//!
//! Template rules:
//! - Lines outside a block appear once, in order (`misc_rep_default`, `misc_rep_no_matches`).
//! - `#ALL` starts a block repeated for every record, `#WHERE <condition>` one repeated for the
//!   records where the condition is true (it may be in brackets: `#WHERE [rv >= 1]`,
//!   `misc_rep_where`; or not: `#WHERE grp = 'A'`, `misc_rep_scopes`). A line `#` ends the block.
//!   Blocks are written one after the other, each over all its records (`misc_rep_scopes`).
//! - `[expression]` in a block line is the CLEM value on the record; `@INDEX` is the record
//!   number in the input (`misc_rep_reverse`). Integers are plain, reals have the stream's
//!   decimal places, null is `$null$`; `<` and `>` become `&lt;` and `&gt;`, `&` stays
//!   (`misc_rep_all`: `<tag>&` gives `&lt;tag&gt;&`).
//! - With highlighting on, format `Auto` and file type `html` (also on screen), each inserted
//!   value is in `<strong>...</strong>` (`misc_rep_all`, `misc_rep_auto_html`); not for
//!   `Custom` (`misc_rep_custom_html`), text files (`misc_rep_text_file`) or highlighting off
//!   (`misc_rep_plain_html`).
//! - An input without records gives an empty report, fixed lines too (`misc_rep_empty_input`).
//!
//! Files: lines end with LF and the last has none. A text file and a `Custom` HTML file are the
//! report itself (`misc_rep_text_file`, `misc_rep_custom_html`). An `Auto` HTML file wraps it
//! in Modeler's page with the title; a report with more lines than `outputLinesPerPage` is
//! split into pages `<name>_<n>.html` with a frameset `<name>.html` and a page list
//! `<name>_contents.html` (`misc_rep_decorated`, `misc_rep_plain_html`); a page after the first
//! starts with the line end before its first line.
//!
//! GAPs: `[expression]` outside a block is rejected (no probe); dates and times print as
//! `YYYY-MM-DD` / `HH:MM:SS` (no probe); one paged probe has 24 lines at 20 per page in pages of
//! 21 and 3, so pages hold `outputLinesPerPage + 1` lines here; the title is not HTML-escaped.

use super::field_util::{bind, bind_condition, eval_all, truths};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput, format_real};
use crate::settings;
use dms_clem::{Datum, Expr, parse};
use dms_stream::StreamNode;
use dms_types::{Table, Value};
use std::path::Path;
use std::sync::Arc;

const SETTING: &str = "reportText";

const HTML_HEAD: &str = "<HTML><HEAD>\r\n<META HTTP-EQUIV=\"Cache-Control\" CONTENT=\"no-cache\">\r\n\
<META HTTP-EQUIV=\"Pragma\" CONTENT=\"no-cache\">\r\n\
<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html;charset=UTF-8\">\r\n</HEAD>\r\n";

enum Segment {
    Text(String),
    Insert(Expr),
}

enum Item {
    Line(String),
    Block {
        condition: Option<Expr>,
        lines: Vec<Vec<Segment>>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileType {
    Html,
    Text,
}

struct Report {
    items: Vec<Item>,
    highlight: bool,
    custom: bool,
    file: Option<(String, FileType)>,
    title: String,
    lines_per_page: usize,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let bad = |what: String| OperatorError::Settings(format!("Report: {what}"));
    let custom = match settings::text(s, "reportFormatMode")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("Auto") => false,
        Some("Custom") => true,
        Some(other) => return Err(bad(format!("unknown reportFormatMode {other:?}"))),
    };
    let file_type = match settings::text(s, "reportOutputFileType")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("html") => FileType::Html,
        Some("text") => FileType::Text,
        Some(other) => return Err(bad(format!("file type {other:?} is not supported yet"))),
    };
    let file = match settings::text(s, "outputMode").as_deref().map(str::trim) {
        None | Some("") | Some("screen") => None,
        Some("file") => {
            let path = settings::text(s, "outputFilePath")
                .filter(|p| !p.trim().is_empty())
                .ok_or_else(|| bad("outputFilePath is required in file mode".into()))?;
            Some((path, file_type))
        }
        Some(other) => return Err(bad(format!("unknown outputMode {other:?}"))),
    };
    let lines_per_page = settings::integer(s, "outputLinesPerPage")?
        .and_then(|n| usize::try_from(n).ok())
        .filter(|&n| n > 0)
        .unwrap_or(400);
    Ok(Box::new(Report {
        items: template(&settings::text(s, SETTING).unwrap_or_default())?,
        highlight: settings::flag(s, "reportHighlightInserted")?.unwrap_or(true)
            && !custom
            && file_type == FileType::Html,
        custom,
        file,
        title: settings::text(s, "reportTitle").unwrap_or_default(),
        lines_per_page,
    }))
}

fn template(text: &str) -> Result<Vec<Item>, OperatorError> {
    let bad = |what: String| OperatorError::Settings(format!("{SETTING}: {what}"));
    let mut items = Vec::new();
    let mut block: Option<(Option<Expr>, Vec<Vec<Segment>>)> = None;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let directive = line.trim();
        if directive == "#" {
            let (condition, lines) = block
                .take()
                .ok_or_else(|| bad("`#` without an open block".into()))?;
            items.push(Item::Block { condition, lines });
            continue;
        }
        let opens = if directive == "#ALL" {
            Some(None)
        } else if let Some(condition) = directive.strip_prefix("#WHERE") {
            let condition = condition.trim();
            let condition = condition
                .strip_prefix('[')
                .and_then(|c| c.strip_suffix(']'))
                .unwrap_or(condition);
            let expr = parse(condition).map_err(|e| bad(format!("#WHERE {condition}: {e}")))?;
            Some(Some(expr))
        } else {
            None
        };
        if let Some(condition) = opens {
            if block.is_some() {
                return Err(bad("nested blocks are not supported".into()));
            }
            block = Some((condition, Vec::new()));
            continue;
        }
        match &mut block {
            Some((_, lines)) => lines.push(segments(line)?),
            None if line.contains('[') => {
                return Err(bad(format!(
                    "an inserted value outside #ALL or #WHERE is not supported yet: {line:?}"
                )));
            }
            None => items.push(Item::Line(line.to_string())),
        }
    }
    if block.is_some() {
        return Err(bad("a block is not closed with `#`".into()));
    }
    Ok(items)
}

fn segments(line: &str) -> Result<Vec<Segment>, OperatorError> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        if open > 0 {
            out.push(Segment::Text(rest[..open].to_string()));
        }
        let after = &rest[open + 1..];
        let close = after.find(']').ok_or_else(|| {
            OperatorError::Settings(format!("{SETTING}: unclosed `[` in {line:?}"))
        })?;
        let source = &after[..close];
        let expr = parse(source)
            .map_err(|e| OperatorError::Settings(format!("{SETTING}: [{source}]: {e}")))?;
        out.push(Segment::Insert(expr));
        rest = &after[close + 1..];
    }
    if !rest.is_empty() {
        out.push(Segment::Text(rest.to_string()));
    }
    Ok(out)
}

impl Report {
    fn render(&self, input: &Table, decimals: usize) -> Result<Vec<String>, OperatorError> {
        let rows = input.rows();
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let model = input.model();
        let mut out = Vec::new();
        for item in &self.items {
            let (condition, lines) = match item {
                Item::Line(text) => {
                    out.push(text.clone());
                    continue;
                }
                Item::Block { condition, lines } => (condition, lines),
            };
            let selected: Vec<bool> = match condition {
                None => vec![true; rows.len()],
                Some(expr) => {
                    let compiled = bind_condition(SETTING, expr, model, None)?;
                    truths(SETTING, &compiled, rows)?
                        .into_iter()
                        .map(|t| t == Some(true))
                        .collect()
                }
            };
            // Every inserted value on every record, line by line.
            let mut texts: Vec<Vec<Vec<String>>> = Vec::with_capacity(lines.len());
            for line in lines {
                let mut line_texts = Vec::new();
                for segment in line {
                    if let Segment::Insert(expr) = segment {
                        let compiled = bind(SETTING, expr, model, None)?;
                        let values = eval_all(SETTING, &compiled, rows)?
                            .iter()
                            .map(|datum| self.inserted(datum, decimals))
                            .collect();
                        line_texts.push(values);
                    }
                }
                texts.push(line_texts);
            }
            for (record, _) in selected.iter().enumerate().filter(|(_, keep)| **keep) {
                for (line, values) in lines.iter().zip(&texts) {
                    let mut text = String::new();
                    let mut inserted = values.iter();
                    for segment in line {
                        match segment {
                            Segment::Text(t) => text.push_str(t),
                            Segment::Insert(_) => {
                                if let Some(value) = inserted.next().and_then(|v| v.get(record)) {
                                    text.push_str(value);
                                }
                            }
                        }
                    }
                    out.push(text);
                }
            }
        }
        Ok(out)
    }

    fn inserted(&self, datum: &Datum, decimals: usize) -> String {
        let text = match datum {
            Datum::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            Datum::Value(Value::Null) => "$null$".to_string(),
            Datum::Value(Value::Real(r)) => format_real(*r, decimals),
            Datum::Value(value) => value.to_string(),
        };
        let text = text.replace('<', "&lt;").replace('>', "&gt;");
        if self.highlight {
            format!("<strong>{text}</strong>")
        } else {
            text
        }
    }

    fn write(
        &self,
        path: &Path,
        file_type: FileType,
        lines: &[String],
    ) -> Result<(), OperatorError> {
        let write = |path: &Path, text: String| {
            std::fs::write(path, text)
                .map_err(|e| OperatorError::Io(format!("{}: {e}", path.display())))
        };
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| OperatorError::Io(format!("{}: {e}", parent.display())))?;
        }
        if file_type == FileType::Text || self.custom {
            return write(path, lines.join("\n"));
        }
        let page_size = self.lines_per_page + 1;
        if lines.len() <= page_size {
            return write(
                path,
                format!(
                    "{HTML_HEAD}<CENTER><STRONG><FONT FACE=\"\" SIZE=5 COLOR=#000000>{}</FONT>\
                     </STRONG></CENTER><BR><pre>{}</pre></HTML>",
                    self.title,
                    lines.join("\n")
                ),
            );
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("report")
            .to_string();
        let sibling = |suffix: &str| path.with_file_name(format!("{stem}{suffix}.html"));
        let name = |suffix: &str| format!("{stem}{suffix}.html");
        let pages: Vec<&[String]> = lines.chunks(page_size).collect();
        for (index, page) in pages.iter().enumerate() {
            write(
                &sibling(&format!("_{}", index + 1)),
                // A page after the first starts with the line end before its first line.
                format!(
                    "{HTML_HEAD}<pre>{}{}</pre></HTML>",
                    if index == 0 { "" } else { "\n" },
                    page.join("\n")
                ),
            )?;
        }
        let options: String = (1..=pages.len())
            .map(|n| format!("<option VALUE={}> {n}\r\n", name(&format!("_{n}"))))
            .collect();
        write(
            &sibling("_contents"),
            format!(
                "{HTML_HEAD}\r\n<TABLE WIDTH=\"100%\" BORDER=\"0\" role=\"presentation\" \
                 cellspacing=\"0\" cellpadding=\"0\">\r\n<TR>\r\n<TD WIDTH=25% ALIGN=LEFT \
                 BACKGROUND=NO>\r\n<form>\r\n<STRONG><FONT FACE=\"\" SIZE=2 COLOR=#000000>Page: \
                 </FONT></STRONG>\r\n<select name= RowSelectionCombo onchange=\"parent.\
                 selectRowsToShow(RowSelectionCombo.options[RowSelectionCombo.selectedIndex].\
                 value);\" size=1>\r\n{options}</select>\r\n</form></td>\r\n<TD WIDTH=50% \
                 ALIGN=CENTER BACKGROUND=NO><STRONG><FONT FACE=\"\" SIZE=5 COLOR=#000000>{}\
                 </FONT></STRONG></td>\r\n<TD WIDTH=25% ALIGN=LEFT BACKGROUND=NO></td>\r\n\
                 </TR></HTML>",
                self.title
            ),
        )?;
        write(
            path,
            format!(
                "\r\n<SCRIPT LANGUAGE=\"JavaScript\"><!--\r\n\tfunction \
                 selectRowsToShow(pageFileName)\r\n\t{{\r\n\tthis.frames[1].location= \
                 pageFileName\r\n \t}}\r\n\t\r\n //--></SCRIPT>\r\n{HTML_HEAD}\r\n<FRAMESET \
                 ROWS=\"10%,70%\">\r\n<FRAME SRC=\"{}\"NAME =\"top\">\r\n<FRAME SRC=\"{}\"NAME \
                 =\"bottom\" SCROLLING=\"auto\"></HTML>",
                name("_contents"),
                name("_1")
            ),
        )
    }
}

impl Operator for Report {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Report is an output node and produces a Document, not a Table".into(),
        ))
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        _model: Option<Arc<dms_pmml::Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Report needs one input".into()))?;
        let lines = self.render(input, context.decimal_places())?;
        if let Some((path, file_type)) = &self.file {
            self.write(&context.export_path(path), *file_type, &lines)?;
        }
        Ok(NodeOutput::Document(Arc::new(Document {
            title: "Report".into(),
            tables: vec![DocumentTable {
                title: None,
                header: Vec::new(),
                rows: lines.into_iter().map(|line| vec![line]).collect(),
            }],
        })))
    }
}
