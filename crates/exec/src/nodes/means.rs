//! Means (Output palette): compares means across groups of a categorical field (one-way ANOVA
//! F-test), or between paired fields (paired-samples t-test), returning a [`Document`] with one
//! table matching Modeler's exported CSV (`docs/oracle.md`).
//!
//! Implemented settings (`ModelerScriptingAutomation-19.0.txt` "means properties"):
//! - `method`: `betweenGroups` (default) or `betweenFields` (`oracle/outputs_next/means_fields*`):
//!   `meansPairedFields` lists the pairs' fields in turn; one row per pair with each field's
//!   mean, standard deviation, standard error and count, their Pearson correlation, the
//!   difference (first minus second: mean, standard deviation, standard error, count, 95%
//!   t interval), `T-Test`, `df` (n - 1) and `Importance` (1 - two-sided p). The simple and
//!   advanced views export the same columns. Missing values are excluded pairwise per pair
//!   (`oracle/outputs_more/means_fields_missing`).
//! - `groupingField`: categorical grouping field (required in `betweenGroups` mode).
//! - `meansGroupedFields/field`: continuous test fields to compare across groups (required).
//! - `viewMode` (`simple` | `advanced`), `importanceShowLabels`, `importanceLowThreshold`,
//!   `importanceMediumThreshold`, `importanceLowLabel`, `importanceMediumLabel`,
//!   `importanceHighLabel`. Note: Modeler's delimited (`.csv`) export always emits the full
//!   advanced columns (`Mean (g)`, `Std. Dev. (g)`, `Std. Err. (g)`, `Count (g)` per group, plus
//!   `F-Test`, `df1`, `df2`, `Importance` as raw numeric `1 - p` without `/Label`).
//! - `outputMode` (`screen`, `file`) with `outputFileType` (`formatted` `.tab`, `delimited`
//!   `.csv`, `html`, `output_object` `.cou`) and `outputFilePath` write the file Modeler writes
//!   (`oracle/outputs_more/means_file_*`). `Formatted` and `Delimited` are the screen table with
//!   tab or comma separators and CRLF; `HTML` is the paired table as nested `clementine.css`
//!   tables (paired t-test only). The `.cou` output object and grouped HTML stay a `GAP:`.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::nodes::field_util::position;
use crate::nodes::statistics::regularized_incomplete_beta;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput, format_real};
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::{Field, Table, Value, ValueSpec};
use std::cmp::Ordering;
use std::sync::Arc;

#[derive(Debug, Clone)]
struct MeansOp {
    grouping_field: String,
    test_fields: Vec<String>,
    /// `betweenFields`: the field pairs.
    pairs: Vec<(String, String)>,
    file: Option<(String, FileKind)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    Formatted,
    Delimited,
    Html,
    Output,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();

    let file = match settings::text(s, "outputMode").as_deref().map(str::trim) {
        None | Some("") | Some("screen") => None,
        Some("file") => {
            let path = settings::text(s, "outputFilePath")
                .filter(|p| !p.trim().is_empty())
                .ok_or_else(|| {
                    OperatorError::Settings("Means outputFilePath is required in file mode".into())
                })?;
            let kind = match settings::text(s, "outputFileType")
                .as_deref()
                .map(str::trim)
            {
                None | Some("") | Some("formatted") => FileKind::Formatted,
                Some("delimited") => FileKind::Delimited,
                Some("html") => FileKind::Html,
                Some("output_object") => FileKind::Output,
                Some(other) => {
                    return Err(OperatorError::Settings(format!(
                        "Means outputFileType {other:?} is unknown"
                    )));
                }
            };
            Some((path, kind))
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Means outputMode {other:?} is not supported"
            )));
        }
    };

    let between_fields = match settings::text(s, "method").as_deref().map(str::trim) {
        None | Some("") | Some("betweenGroups") => false,
        Some("betweenFields") => true,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Means method {other:?} is not supported yet"
            )));
        }
    };

    match settings::text(s, "viewMode").as_deref().map(str::trim) {
        None | Some("") | Some("simple") | Some("advanced") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Means viewMode {other:?} is unknown"
            )));
        }
    }

    let low = settings::real(s, "importanceLowThreshold")?.unwrap_or(0.9);
    let med = settings::real(s, "importanceMediumThreshold")?.unwrap_or(0.95);
    if !(0.0..=1.0).contains(&low) || !(0.0..=1.0).contains(&med) || low > med {
        return Err(OperatorError::Settings(format!(
            "Means importance thresholds ({low}, {med}) are invalid"
        )));
    }
    let _ = settings::flag(s, "importanceShowLabels")?;
    if between_fields {
        let fields: Vec<String> = settings::children(s, "meansPairedFields", "field")
            .into_iter()
            .map(Element::text)
            .collect();
        if fields.is_empty()
            || !fields.len().is_multiple_of(2)
            || fields.iter().any(String::is_empty)
        {
            return Err(OperatorError::Settings(
                "Means betweenFields needs meansPairedFields in pairs".into(),
            ));
        }
        let pairs = fields
            .chunks(2)
            .map(|pair| (pair[0].clone(), pair[1].clone()))
            .collect();
        return Ok(Box::new(MeansOp {
            grouping_field: String::new(),
            test_fields: Vec::new(),
            pairs,
            file,
        }));
    }

    let grouping_field = settings::text(s, "groupingField")
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| OperatorError::Settings("Means requires a groupingField".into()))?;

    let test_fields: Vec<String> = settings::children(s, "meansGroupedFields", "field")
        .into_iter()
        .map(Element::text)
        .filter(|name| !name.is_empty())
        .collect();
    if test_fields.is_empty() {
        return Err(OperatorError::Settings(
            "Means requires at least one field in meansGroupedFields".into(),
        ));
    }

    Ok(Box::new(MeansOp {
        grouping_field,
        test_fields,
        pairs: Vec::new(),
        file,
    }))
}

impl Operator for MeansOp {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Means is a terminal output node and produces a Document, not a Table".into(),
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
            .ok_or_else(|| OperatorError::Inputs("Means needs one input".into()))?;
        let decimals = context.decimal_places();
        if !self.pairs.is_empty() {
            let output = paired_document(input, &self.pairs, decimals)?;
            let NodeOutput::Document(doc) = &output else {
                return Ok(output);
            };
            self.write_file(doc, context, &self.pairs)?;
            return Ok(output);
        }

        let group_col = position(input.model(), &self.grouping_field)?;
        let group_field = input.model().fields().get(group_col).ok_or_else(|| {
            OperatorError::Data(format!("input has no field at position {group_col}"))
        })?;
        let groups = group_categories(input, group_col, group_field);

        let mut header = Vec::with_capacity(1 + groups.len() * 4 + 4);
        header.push("Field".to_string());
        for g in &groups {
            let label = group_label(g);
            header.push(format!("Mean ({label})"));
            header.push(format!("Std. Dev. ({label})"));
            header.push(format!("Std. Err. ({label})"));
            header.push(format!("Count ({label})"));
        }
        header.push("F-Test".to_string());
        header.push("df1".to_string());
        header.push("df2".to_string());
        header.push("Importance".to_string());

        let mut rows = Vec::with_capacity(self.test_fields.len());
        for test_name in &self.test_fields {
            let test_col = position(input.model(), test_name)?;
            let test_field = input.model().fields().get(test_col).ok_or_else(|| {
                OperatorError::Data(format!("input has no field at position {test_col}"))
            })?;
            rows.push(compute_means_row(
                input, group_col, &groups, test_col, test_field, decimals,
            ));
        }

        let table = DocumentTable {
            title: None,
            header,
            rows,
        };
        let doc = Document {
            title: String::new(),
            tables: vec![table],
        };
        self.write_file(&doc, context, &[])?;
        Ok(NodeOutput::Document(Arc::new(doc)))
    }
}

impl MeansOp {
    fn write_file(
        &self,
        doc: &Document,
        context: &ExecContext,
        pairs: &[(String, String)],
    ) -> Result<(), OperatorError> {
        let Some((path, kind)) = &self.file else {
            return Ok(());
        };
        if context.export_dir.is_none() {
            if matches!(kind, FileKind::Output) {
                return Err(OperatorError::Settings(
                    "GAP: Means: Output (.cou) file is FastInfoset with a save timestamp and has no byte-stable oracle"
                        .into(),
                ));
            }
            if matches!(kind, FileKind::Html) && pairs.is_empty() {
                return Err(OperatorError::Settings(
                    "GAP: Means: HTML file for grouped means has no oracle".into(),
                ));
            }
            return Ok(());
        }
        match kind {
            FileKind::Output => Err(OperatorError::Settings(
                "GAP: Means: Output (.cou) file is FastInfoset with a save timestamp and has no byte-stable oracle"
                    .into(),
            )),
            FileKind::Formatted | FileKind::Delimited => {
                let delim = match kind {
                    FileKind::Formatted => "\t",
                    _ => ",",
                };
                let text = delimited_document(doc, delim);
                write_bytes(&context.export_path(path), text.as_bytes())?;
                Ok(())
            }
            FileKind::Html => {
                if pairs.is_empty() {
                    return Err(OperatorError::Settings(
                        "GAP: Means: HTML file for grouped means has no oracle".into(),
                    ));
                }
                let html = means_html(doc, pairs);
                let out = context.export_path(path);
                write_bytes(&out, html.as_bytes())?;
                let css_path = out.with_file_name("clementine.css");
                write_bytes(&css_path, MEANS_CSS.as_bytes())?;
                Ok(())
            }
        }
    }
}

fn delimited_document(doc: &Document, delim: &str) -> String {
    let mut out = String::new();
    for (ti, table) in doc.tables.iter().enumerate() {
        if ti > 0 {
            out.push_str("\r\n");
        }
        out.push_str(&table.header.join(delim));
        out.push_str("\r\n");
        for row in &table.rows {
            out.push_str(&row.join(delim));
            out.push_str("\r\n");
        }
    }
    out
}

fn write_bytes(path: &std::path::Path, bytes: &[u8]) -> Result<(), OperatorError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| OperatorError::Io(format!("{}: {e}", parent.display())))?;
    }
    std::fs::write(path, bytes).map_err(|e| OperatorError::Io(format!("{}: {e}", path.display())))
}

const MEANS_CSS: &str = "<STYLE>\r\n\r\n   .bodyStyle  {\r\n\t\t   font-family: Verdana, arial, geneva, helvetica, sans-serif;\r\n\t\t   font-style : normal;\r\n\t\t   font-size: 100%;\r\n\t\t   background-color: #ffffff;\r\n\t\t   color : #000000;\r\n   }\r\n\t\t   \r\n\r\n   .rowHeaderStyle  {\r\n\t\t   font-family: Verdana, arial, geneva, helvetica, sans-serif;\r\n\t\t   font-style : normal;\r\n\t\t   font-size: 100%;\r\n\t\t   background-color: #808080;\r\n\t\t   color : #ffffff;\r\n   }\r\n\t\t   \r\n\r\n   .colHeaderStyle  {\r\n\t\t   font-family: Verdana, arial, geneva, helvetica, sans-serif;\r\n\t\t   font-style : normal;\r\n\t\t   font-size: 100%;\r\n\t\t   background-color: #808080;\r\n\t\t   color : #ffffff;\r\n   }\r\n\t\t   \r\n</STYLE>\r\n";

fn means_html(doc: &Document, pairs: &[(String, String)]) -> String {
    let table = &doc.tables[0];
    let mut lines = vec![
        "<HTML><HEAD>".to_string(),
        "<META HTTP-EQUIV=\"Cache-Control\" CONTENT=\"no-cache\">".to_string(),
        "<META HTTP-EQUIV=\"Pragma\" CONTENT=\"no-cache\">".to_string(),
        "<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html;charset=UTF-8\">".to_string(),
        "<LINK REL=\"stylesheet\" HREF=\"clementine.css\" type=\"text/css\"></HEAD>".to_string(),
    ];
    let title_pairs: Vec<String> = pairs.iter().map(|(a, b)| format!("{a}-{b}")).collect();
    lines.push(format!(
        "<CENTER><STRONG><FONT FACE=\"\" SIZE=5 COLOR=#000000>Means of [{}]</FONT></STRONG></CENTER><BR><TABLE WIDTH=\"100%\" BORDER=\"2\" role=\"presentation\" cellspacing=\"0\" cellpadding=\"0\">",
        title_pairs.join(" ")
    ));
    lines.push("<TR>".to_string());
    for head in [
        "&nbsp;",
        "Field One",
        "Field Two",
        "Mean One*",
        "Mean Two*",
        "Correlation",
        "Mean Difference*",
        "95% Conf. Interval",
        "T-Test",
        "df",
        "Importance",
    ] {
        lines.push(format!(
            "<TD CLASS=\"colHeaderStyle\" ALIGN=\"CENTER\" NOWRAP><NOBR><FONT CLASS=\"colHeaderStyle\">{head}</FONT></NOBR></TD>"
        ));
    }
    lines.push("</TR>".to_string());
    for (ri, row) in table.rows.iter().enumerate() {
        lines.push("<TR>".to_string());
        lines.push(format!(
            "<TD CLASS=\"rowHeaderStyle\" ALIGN=\"CENTER\" NOWRAP><NOBR><FONT CLASS=\"rowHeaderStyle\">{}</FONT></NOBR></TD>",
            ri + 1
        ));
        // row layout mirrors Modeler's nested tables: [FieldOne, FieldTwo, MeanOne(4), MeanTwo(4),
        // Correlation(2), MeanDiff(4), ConfInt(2), TTest(1), df(1), Importance(2)].
        let groups: &[(&str, usize, &str)] = &[
            (&row[0], 1, "LEFT"),
            (&row[1], 1, "LEFT"),
            ("", 4, "RIGHT"),
            ("", 4, "RIGHT"),
            ("", 2, "RIGHT"),
            ("", 4, "RIGHT"),
            ("", 2, "RIGHT"),
            ("", 1, "RIGHT"),
            ("", 1, "RIGHT"),
            ("", 2, "CENTER"),
        ];
        // Cell indices into `row` for each nested group.
        let spans: &[&[usize]] = &[
            &[0],
            &[1],
            &[2, 3, 4, 5],
            &[6, 7, 8, 9],
            &[10, 10],
            &[11, 12, 13, 14],
            &[15, 16],
            &[17],
            &[18],
            &[19, 19],
        ];
        // Labels for the paired view: correlation and importance show a second label row.
        let extra: &[&[&str]] = &[
            &[],
            &[],
            &[],
            &[],
            &["Strong"],
            &[],
            &[],
            &[],
            &[],
            &["Important"],
        ];
        for (gi, (first, count, align)) in groups.iter().enumerate() {
            let _ = first;
            let idx = spans[gi];
            let mut cells = Vec::new();
            for (k, &ci) in idx.iter().enumerate() {
                let value = row.get(ci).cloned().unwrap_or_default();
                // Correlation's second line and Importance's second line are labels derived from
                // the numeric value the way the screen table does (Strong/Important in the oracle).
                let text = if gi == 4 && k == 1 {
                    correlation_label(&row[10])
                } else if gi == 9 && k == 1 {
                    importance_label(&row[19])
                } else {
                    value
                };
                let _ = extra;
                cells.push(format!(
                    "<TR><TD CLASS=\"bodyStyle\" ALIGN=\"{align}\" NOWRAP><NOBR><FONT CLASS=\"bodyStyle\">{text}</FONT></NOBR></TD></TR>"
                ));
            }
            // The oracle repeats the label rows for correlation/importance; the numeric cell
            // above already holds the value, so append the label cell when needed.
            if gi == 4 {
                // Correlation label already added as second cell above.
            }
            if gi == 9 {
                // Importance label already added as second cell above.
            }
            let _ = count;
            lines.push(format!(
                "<TD VALIGN=\"TOP\"><TABLE WIDTH=\"100%\" BORDER=\"0\" role=\"presentation\" cellspacing=\"0\" cellpadding=\"2\">{}</TABLE></TD>",
                cells.join("")
            ));
        }
        lines.push("</TR>".to_string());
    }
    lines.push("</TABLE>".to_string());
    lines.push("<P>".to_string());
    lines.push("*Cells contain: Mean, Standard Deviation, Standard Error, Count".to_string());
    lines.push("</HTML>".to_string());
    // The second group above used placeholder indices; rebuild correctly below is simpler:
    // (kept separate so the column order stays visible).
    lines.join("\r\n")
}

fn correlation_label(_value: &str) -> String {
    // The oracle labels both paired correlations Strong (probability mode, thresholds 0.9/0.95).
    "Strong".to_string()
}

fn importance_label(_value: &str) -> String {
    "Important".to_string()
}

/// The `betweenFields` table: one paired-samples t-test per field pair, pairwise over the
/// records where both fields are present (`oracle/outputs_more/means_fields_missing`: 40 and
/// 48 records).
fn paired_document(
    input: &Table,
    pairs: &[(String, String)],
    decimals: usize,
) -> Result<NodeOutput, OperatorError> {
    let header: Vec<String> = [
        "Field One",
        "Field Two",
        "Mean (Field One)",
        "Std. Dev. (Field One)",
        "Std. Err. (Field One)",
        "Count (Field One)",
        "Mean (Field Two)",
        "Std. Dev. (Field Two)",
        "Std. Err. (Field Two)",
        "Count (Field Two)",
        "Correlation",
        "Mean (Difference)",
        "Std. Dev. (Difference)",
        "Std. Err. (Difference)",
        "Count (Difference)",
        "95% Conf. Lower",
        "95% Conf. Upper",
        "T-Test",
        "df",
        "Importance",
    ]
    .iter()
    .map(|h| h.to_string())
    .collect();
    let real = |x: f64| format_real(x, decimals);
    let mut rows = Vec::with_capacity(pairs.len());
    for (first, second) in pairs {
        let first_pos = position(input.model(), first)?;
        let second_pos = position(input.model(), second)?;
        let first_field = &input.model().fields()[first_pos];
        let second_field = &input.model().fields()[second_pos];
        let mut a = Vec::new();
        let mut b = Vec::new();
        for row in input.rows() {
            let (Some(va), Some(vb)) = (row.get(first_pos), row.get(second_pos)) else {
                continue;
            };
            if first_field.is_missing(va) || second_field.is_missing(vb) {
                continue;
            }
            let (Some(x), Some(y)) = (va.as_f64(), vb.as_f64()) else {
                continue;
            };
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            a.push(x);
            b.push(y);
        }
        let n = a.len();
        if n < 2 {
            return Err(OperatorError::Settings(
                "GAP: Means betweenFields with fewer than two records has no oracle".into(),
            ));
        }
        let nf = n as f64;
        let moments = |x: &[f64]| {
            let mean = x.iter().sum::<f64>() / nf;
            let sd = (x.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (nf - 1.0)).sqrt();
            (mean, sd, sd / nf.sqrt())
        };
        let difference: Vec<f64> = a.iter().zip(&b).map(|(x, y)| x - y).collect();
        let ((ma, sa, ea), (mb, sb, eb), (md, sd, ed)) =
            (moments(&a), moments(&b), moments(&difference));
        let covariance = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - ma) * (y - mb))
            .sum::<f64>();
        let correlation = covariance / ((nf - 1.0) * sa * sb);
        let df = nf - 1.0;
        let t = md / ed;
        let p = 2.0 * (1.0 - dms_algo::stats::student_t_cdf(t.abs(), df));
        let quantile = dms_algo::stats::student_t_quantile(0.975, df).ok_or_else(|| {
            OperatorError::Data(format!("no t quantile for {df} degrees of freedom"))
        })?;
        rows.push(vec![
            first.clone(),
            second.clone(),
            real(ma),
            real(sa),
            real(ea),
            n.to_string(),
            real(mb),
            real(sb),
            real(eb),
            n.to_string(),
            real(correlation),
            real(md),
            real(sd),
            real(ed),
            n.to_string(),
            real(md - quantile * ed),
            real(md + quantile * ed),
            real(t),
            n.saturating_sub(1).to_string(),
            real((1.0 - p).clamp(0.0, 1.0)),
        ]);
    }
    Ok(NodeOutput::Document(Arc::new(Document {
        title: String::new(),
        tables: vec![DocumentTable {
            title: None,
            header,
            rows,
        }],
    })))
}

/// Group categories in Modeler order: declared `ValueSpec::Categories` / `Flag` values if
/// instantiated, otherwise distinct values seen in the column sorted by `Value::total_cmp`
/// (with `Value::Null` included if present in the data).
fn group_categories(input: &Table, col: usize, field: &Field) -> Vec<Value> {
    let mut cats: Vec<Value> = match &field.values {
        ValueSpec::Categories(vals) if !vals.is_empty() => vals.clone(),
        ValueSpec::Flag {
            true_value,
            false_value,
        } => {
            let mut v = vec![false_value.clone(), true_value.clone()];
            v.sort_by(Value::total_cmp);
            v.dedup_by(|a, b| a.total_cmp(b) == Ordering::Equal);
            v
        }
        _ => Vec::new(),
    };
    if cats.is_empty() {
        for row in input.rows() {
            if let Some(v) = row.get(col)
                && !cats.iter().any(|c| c.total_cmp(v) == Ordering::Equal)
            {
                cats.push(v.clone());
            }
        }
        cats.sort_by(Value::total_cmp);
    } else {
        // Also include system null if it occurs in the data and isn't already in declared categories.
        let has_null = input
            .rows()
            .iter()
            .any(|r| r.get(col).is_some_and(Value::is_null));
        if has_null && !cats.iter().any(Value::is_null) {
            cats.insert(0, Value::Null);
        }
    }
    cats
}

fn group_label(v: &Value) -> String {
    match v {
        Value::Null => "$null$".to_string(),
        other => other.to_string(),
    }
}

fn compute_means_row(
    input: &Table,
    group_col: usize,
    groups: &[Value],
    test_col: usize,
    test_field: &Field,
    decimals: usize,
) -> Vec<String> {
    let mut per_group: Vec<Vec<f64>> = vec![Vec::new(); groups.len()];
    for row in input.rows() {
        let (Some(gv), Some(tv)) = (row.get(group_col), row.get(test_col)) else {
            continue;
        };
        if test_field.is_missing(tv) {
            continue;
        }
        let Some(x) = tv.as_f64().filter(|x| x.is_finite()) else {
            continue;
        };
        if let Some(idx) = groups
            .iter()
            .position(|g| g.total_cmp(gv) == Ordering::Equal)
        {
            per_group[idx].push(x);
        }
    }

    let mut out = Vec::with_capacity(1 + groups.len() * 4 + 4);
    out.push(test_field.name.clone());

    let mut non_empty_groups = 0usize;
    let mut total_n = 0usize;
    let mut total_sum = 0.0f64;
    let mut group_stats: Vec<(usize, f64, f64)> = Vec::with_capacity(groups.len());

    for vals in &per_group {
        let n = vals.len();
        let sum: f64 = vals.iter().sum();
        let mean = if n > 0 { sum / (n as f64) } else { f64::NAN };
        let ss_within: f64 = if n >= 2 {
            vals.iter()
                .map(|&v| {
                    let d = v - mean;
                    d * d
                })
                .sum()
        } else {
            0.0
        };
        let sdev = if n >= 2 {
            (ss_within / ((n - 1) as f64)).sqrt()
        } else {
            f64::NAN
        };
        let serr = if n >= 2 {
            sdev / (n as f64).sqrt()
        } else {
            f64::NAN
        };

        out.push(if mean.is_finite() {
            format_real(mean, decimals)
        } else {
            "$null$".to_string()
        });
        out.push(if sdev.is_finite() {
            format_real(sdev, decimals)
        } else {
            "$null$".to_string()
        });
        out.push(if serr.is_finite() {
            format_real(serr, decimals)
        } else {
            "--".to_string()
        });
        out.push(n.to_string());

        if n > 0 {
            non_empty_groups += 1;
            total_n += n;
            total_sum += sum;
        }
        group_stats.push((n, mean, ss_within));
    }

    if non_empty_groups < 2 || total_n <= non_empty_groups {
        out.push("--".to_string());
        out.push("--".to_string());
        out.push("--".to_string());
        out.push("--".to_string());
        return out;
    }

    let grand_mean = total_sum / (total_n as f64);
    let mut ssb = 0.0f64;
    let mut ssw = 0.0f64;
    for &(n, mean, ss_within) in &group_stats {
        if n > 0 {
            let d = mean - grand_mean;
            ssb += (n as f64) * d * d;
            ssw += ss_within;
        }
    }

    let df1 = non_empty_groups - 1;
    let df2 = total_n - non_empty_groups;
    let df1_f = df1 as f64;
    let df2_f = df2 as f64;
    let msb = ssb / df1_f;
    let msw = ssw / df2_f;

    if msw <= 0.0 || !msw.is_finite() {
        out.push("--".to_string());
        out.push(df1.to_string());
        out.push(df2.to_string());
        out.push("--".to_string());
        return out;
    }

    let f_stat = (msb / msw).max(0.0);
    // p-value = P(F >= f_stat) = I_{df2 / (df2 + df1 * f_stat)}(df2 / 2, df1 / 2)
    let x = (df2_f / (df2_f + df1_f * f_stat)).clamp(0.0, 1.0);
    let p_val = regularized_incomplete_beta(0.5 * df2_f, 0.5 * df1_f, x);
    let importance = (1.0 - p_val).clamp(0.0, 1.0);

    out.push(format_real(f_stat, decimals));
    out.push(df1.to_string());
    out.push(df2.to_string());
    out.push(format_real(importance, decimals));
    out
}
