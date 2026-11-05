//! Matrix (Output palette): builds a two-dimensional cross-tabulation or function summary matrix
//! across `rowField` and `columnField`, returning a [`Document`] with one table matching Modeler's
//! exported CSV (`docs/oracle.md`, `docs/formats/nodes/matrix.md`).
//!
//! Implemented settings (saved names, from the `outputs` probes and `team/cluster_analysis`):
//! - `matrixType`: `specified` (the selected row and column fields), `flags` or `numbers`
//!   (`oracle/outputs_next/matrix_*`):
//!   - `flags`: every flag field against every other, in field order; a cell counts the records
//!     where both are true, the diagonal is empty, and `showTotals` adds row and column sums.
//!   - `numbers`: every continuous numeric field against every other, in field order; a cell is
//!     the sum of the products of the two fields over the records where both are present
//!     (pairwise; `oracle/outputs_more/matrix_numerics_missing`), printed as an integer when
//!     both are integers and with 3 decimals otherwise; the diagonal is empty and there are no
//!     totals. Row, column, cell-contents and function settings do not change it.
//!   - Missing values in flag matrices are treated as false for the both-true counts
//!     (`oracle/outputs_more/matrix_flags_missing` matches); missing values in `specified`
//!     cross-tabulations follow `includeMissingValues`.
//! - `rowField`, `columnField`: categorical fields for the matrix rows and columns (required for
//!   `specified`).
//! - `cellContents`: `coincidences` (cross-tabulation) or `parameter` (a function of a field).
//! - `parameterField`: numeric field summarized when `cellContents = parameter` (required then).
//! - `parameterFunction`: `mean`, `sum`, `sdev`, `max`, or `min`.
//! - `coincidenceFunction/setElement`: subset of `{counts, expected, residual, row, column,
//!   total}` (default when absent: `counts`). Displayed in Modeler's fixed order.
//! - `rowColumnSort`: `unsorted` only.
//! - `showTotals`: whether to append a `Total` column and `Total` row block for cross-tabulations
//!   (default `false`).
//! - `includeMissingValues`: whether records with missing values (`$null$` or blanks) in the row
//!   or column field are included as a category (default `true`).
//! - Highlighting display settings (`useHighlightTop`, `highlightTop`, `useHighlightBottom`,
//!   `highlightBottom`) are validated; `outputMode` (`screen`, `file`) with `outputFileType`
//!   (`formatted` `.tab`, `delimited` `.csv`, `html`, `output_object` `.cou`) and `outputFilePath`
//!   write the file Modeler writes (`oracle/outputs_more/matrix_file_*`). `Formatted` and
//!   `Delimited` are the screen table with tab or comma separators and CRLF; `HTML` is the
//!   screen table as a `clementine.css` page with a chi-square footer (counts with totals only).
//!   The `.cou` output object (FastInfoset with a save timestamp) and paged HTML stay a `GAP:`.
//!
//! Files use CRLF; the HTML has no trailing newline. The `clementine.css` written beside an HTML
//! file is Modeler's matrix page style (extra `selected`/`unselected` classes).

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::nodes::field_util::position;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput, format_real};
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::{Field, Storage, Table, Value, ValueSpec};
use std::cmp::Ordering;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellMode {
    Counts,
    Function,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverlayFunction {
    Mean,
    Sum,
    StdDev,
    Max,
    Min,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CrossTabItem {
    Count,
    Expected,
    Residual,
    RowPercent,
    ColPercent,
    TotPercent,
}

/// Modeler's fixed display order for cross-tabulation sub-rows regardless of the order in which
/// `coincidenceFunction/setElement` is saved.
const CROSSTAB_ORDER: &[(CrossTabItem, &str)] = &[
    (CrossTabItem::Count, "Count"),
    (CrossTabItem::Expected, "Expected"),
    (CrossTabItem::Residual, "Residual"),
    (CrossTabItem::RowPercent, "Row %"),
    (CrossTabItem::ColPercent, "Column %"),
    (CrossTabItem::TotPercent, "Total %"),
];

/// Modeler's Matrix CSV export always formats floating-point cells to 3 decimal places.
const MATRIX_DECIMALS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatrixKind {
    Specified,
    Flags,
    Numbers,
}

#[derive(Debug, Clone)]
struct MatrixOp {
    kind: MatrixKind,
    row_field: String,
    col_field: String,
    cell_mode: CellMode,
    overlay_field: Option<String>,
    overlay_fn: OverlayFunction,
    crosstab: Vec<CrossTabItem>,
    show_totals: bool,
    include_missing: bool,
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
                    OperatorError::Settings("Matrix outputFilePath is required in file mode".into())
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
                        "Matrix outputFileType {other:?} is unknown"
                    )));
                }
            };
            Some((path, kind))
        }
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Matrix outputMode {other:?} is not supported"
            )));
        }
    };

    let _ = settings::flag(s, "useHighlightTop")?;
    let _ = settings::integer(s, "highlightTop")?;
    let _ = settings::flag(s, "useHighlightBottom")?;
    let _ = settings::integer(s, "highlightBottom")?;

    let kind = match settings::text(s, "matrixType").as_deref().map(str::trim) {
        None | Some("") | Some("specified") => MatrixKind::Specified,
        Some("flags") => MatrixKind::Flags,
        Some("numbers") => MatrixKind::Numbers,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Matrix matrixType {other:?} is unknown"
            )));
        }
    };
    let field_setting = |key: &str| {
        let value = settings::text(s, key)
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        match (value, kind) {
            (Some(value), _) => Ok(value),
            (None, MatrixKind::Specified) => {
                Err(OperatorError::Settings(format!("Matrix requires a {key}")))
            }
            (None, _) => Ok(String::new()),
        }
    };
    let row_field = field_setting("rowField")?;
    let col_field = field_setting("columnField")?;
    match settings::text(s, "rowColumnSort").as_deref().map(str::trim) {
        None | Some("") | Some("unsorted") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Matrix rowColumnSort {other:?} is not supported yet"
            )));
        }
    }

    let cell_mode = match settings::text(s, "cellContents").as_deref().map(str::trim) {
        None | Some("") | Some("coincidences") => CellMode::Counts,
        Some("parameter") => CellMode::Function,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Matrix cellContents {other:?} is unknown"
            )));
        }
    };

    let overlay_field = settings::text(s, "parameterField")
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    if cell_mode == CellMode::Function && overlay_field.is_none() {
        return Err(OperatorError::Settings(
            "Matrix with cellContents=parameter requires a parameterField".into(),
        ));
    }

    let overlay_fn = match settings::text(s, "parameterFunction")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("mean") => OverlayFunction::Mean,
        Some("sum") => OverlayFunction::Sum,
        Some("sdev") => OverlayFunction::StdDev,
        Some("max") => OverlayFunction::Max,
        Some("min") => OverlayFunction::Min,
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Matrix parameterFunction {other:?} is unknown"
            )));
        }
    };

    let crosstab = parse_crosstab(s)?;
    if cell_mode == CellMode::Counts && crosstab.is_empty() {
        return Err(OperatorError::Settings(
            "Matrix coincidenceFunction cannot be empty".into(),
        ));
    }

    let show_totals = settings::flag(s, "showTotals")?.unwrap_or(false);
    let include_missing = settings::flag(s, "includeMissingValues")?.unwrap_or(true);

    Ok(Box::new(MatrixOp {
        kind,
        row_field,
        col_field,
        cell_mode,
        overlay_field,
        overlay_fn,
        crosstab,
        show_totals,
        include_missing,
        file,
    }))
}

fn parse_crosstab(s: Option<&Element>) -> Result<Vec<CrossTabItem>, OperatorError> {
    if settings::element(s, "coincidenceFunction").is_none() {
        return Ok(vec![CrossTabItem::Count]);
    }
    let mut selected = Vec::new();
    for el in settings::children(s, "coincidenceFunction", "setElement") {
        let item = match el.text().trim() {
            "counts" => CrossTabItem::Count,
            "expected" => CrossTabItem::Expected,
            "residual" => CrossTabItem::Residual,
            "row" => CrossTabItem::RowPercent,
            "column" => CrossTabItem::ColPercent,
            "total" => CrossTabItem::TotPercent,
            other => {
                return Err(OperatorError::Settings(format!(
                    "Matrix coincidenceFunction item {other:?} is unknown"
                )));
            }
        };
        if !selected.contains(&item) {
            selected.push(item);
        }
    }
    let ordered: Vec<CrossTabItem> = CROSSTAB_ORDER
        .iter()
        .filter_map(|(k, _)| selected.contains(k).then_some(*k))
        .collect();
    Ok(ordered)
}

impl Operator for MatrixOp {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Matrix is a terminal output node and produces a Document, not a Table".into(),
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
            .ok_or_else(|| OperatorError::Inputs("Matrix needs one input".into()))?;
        let table = match self.kind {
            MatrixKind::Flags => Some(self.flags_table(input)?),
            MatrixKind::Numbers => Some(numbers_table(input)?),
            MatrixKind::Specified => None,
        };
        if let Some(table) = table {
            let doc = Document {
                title: String::new(),
                tables: vec![table],
            };
            self.write_file(&doc, context)?;
            return Ok(NodeOutput::Document(Arc::new(doc)));
        }

        let row_pos = position(input.model(), &self.row_field)?;
        let col_pos = position(input.model(), &self.col_field)?;
        let row_field = input
            .model()
            .fields()
            .get(row_pos)
            .ok_or_else(|| OperatorError::Data(format!("no field at {row_pos}")))?;
        let col_field = input
            .model()
            .fields()
            .get(col_pos)
            .ok_or_else(|| OperatorError::Data(format!("no field at {col_pos}")))?;

        let row_cats = matrix_categories(input, row_pos, row_field, self.include_missing);
        let col_cats = matrix_categories(input, col_pos, col_field, self.include_missing);

        let table = match self.cell_mode {
            CellMode::Counts => self.build_counts_table(
                input, row_pos, row_field, &row_cats, col_pos, col_field, &col_cats,
            ),
            CellMode::Function => {
                let overlay_name = self.overlay_field.as_deref().unwrap_or_default();
                let ov_pos = position(input.model(), overlay_name)?;
                let ov_field = input
                    .model()
                    .fields()
                    .get(ov_pos)
                    .ok_or_else(|| OperatorError::Data(format!("no field at {ov_pos}")))?;
                self.build_function_table(
                    input, row_pos, row_field, &row_cats, col_pos, col_field, &col_cats, ov_pos,
                    ov_field,
                )
            }
        };

        let doc = Document {
            title: String::new(),
            tables: vec![table],
        };
        self.write_file(&doc, context)?;
        Ok(NodeOutput::Document(Arc::new(doc)))
    }
}

impl MatrixOp {
    fn write_file(&self, doc: &Document, context: &ExecContext) -> Result<(), OperatorError> {
        let Some((path, kind)) = &self.file else {
            return Ok(());
        };
        // Screen checks (`evaluate_stream_output` with no export dir) must not overwrite the
        // corpus file the stream was saved with; the CLI always redirects (`export_dir` is
        // `Some`), and the file byte test does too.
        if context.export_dir.is_none() {
            if matches!(kind, FileKind::Output) {
                return Err(OperatorError::Settings(
                    "GAP: Matrix: Output (.cou) file is FastInfoset with a save timestamp and has no byte-stable oracle"
                        .into(),
                ));
            }
            if matches!(kind, FileKind::Html)
                && (self.kind != MatrixKind::Specified
                    || self.cell_mode != CellMode::Counts
                    || self.crosstab != [CrossTabItem::Count])
            {
                return Err(OperatorError::Settings(
                    "GAP: Matrix: HTML file for this matrix type has no oracle".into(),
                ));
            }
            return Ok(());
        }
        match kind {
            FileKind::Output => Err(OperatorError::Settings(
                "GAP: Matrix: Output (.cou) file is FastInfoset with a save timestamp and has no byte-stable oracle"
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
                if self.kind != MatrixKind::Specified
                    || self.cell_mode != CellMode::Counts
                    || self.crosstab != [CrossTabItem::Count]
                {
                    return Err(OperatorError::Settings(
                        "GAP: Matrix: HTML file for this matrix type has no oracle".into(),
                    ));
                }
                let table = &doc.tables[0];
                let html = matrix_html(
                    &self.row_field,
                    &self.col_field,
                    table,
                    self.include_missing,
                );
                let out = context.export_path(path);
                write_bytes(&out, html.as_bytes())?;
                let css_path = out.with_file_name("clementine.css");
                write_bytes(&css_path, MATRIX_CSS.as_bytes())?;
                Ok(())
            }
        }
    }
}

/// A missing value in a flag or numeric matrix has no oracle.
fn missing_gap(field: &Field) -> OperatorError {
    OperatorError::Settings(format!(
        "GAP: Matrix: missing values in {:?} have no oracle",
        field.name
    ))
}

/// The `numbers` matrix: sums of products of every pair of continuous numeric fields,
/// pairwise over the records where both fields are present (`oracle/outputs_more/matrix_numerics_missing`:
/// `n1` x `id` is `3700 - 386 = 3314`, the eight records where `n1` is missing excluded).
fn numbers_table(input: &Table) -> Result<DocumentTable, OperatorError> {
    let fields: Vec<(usize, &Field)> = input
        .model()
        .fields()
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            f.measurement == dms_types::Measurement::Continuous
                && matches!(f.storage, Storage::Integer | Storage::Real)
        })
        .collect();
    let names: Vec<String> = fields.iter().map(|(_, f)| f.name.clone()).collect();
    let mut header = vec![String::new()];
    header.extend(names.iter().cloned());
    let mut rows = Vec::with_capacity(fields.len());
    for (i, (col_i, field_i)) in fields.iter().enumerate() {
        let mut line = vec![names[i].clone()];
        for (j, (col_j, field_j)) in fields.iter().enumerate() {
            if i == j {
                line.push(String::new());
                continue;
            }
            let mut sum = 0.0f64;
            for row in input.rows() {
                let (Some(vi), Some(vj)) = (row.get(*col_i), row.get(*col_j)) else {
                    continue;
                };
                if field_i.is_missing(vi) || field_j.is_missing(vj) {
                    continue;
                }
                let (Some(x), Some(y)) = (vi.as_f64(), vj.as_f64()) else {
                    continue;
                };
                if !x.is_finite() || !y.is_finite() {
                    continue;
                }
                sum += x * y;
            }
            let integers =
                field_i.storage == Storage::Integer && field_j.storage == Storage::Integer;
            line.push(if integers {
                format!("{}", sum.round() as i64)
            } else {
                format_real(sum, MATRIX_DECIMALS)
            });
        }
        rows.push(line);
    }
    Ok(DocumentTable {
        title: None,
        header,
        rows,
    })
}

impl MatrixOp {
    /// The `flags` matrix: counts of records where two flag fields are both true.
    fn flags_table(&self, input: &Table) -> Result<DocumentTable, OperatorError> {
        let mut flags: Vec<(String, Vec<bool>)> = Vec::new();
        for (index, field) in input.model().fields().iter().enumerate() {
            let ValueSpec::Flag { true_value, .. } = &field.values else {
                continue;
            };
            if field.measurement != dms_types::Measurement::Flag {
                continue;
            }
            let mut column = Vec::with_capacity(input.row_count());
            for row in input.rows() {
                let value = &row[index];
                if value.is_null() || field.is_missing(value) {
                    return Err(missing_gap(field));
                }
                column.push(value.total_cmp(true_value) == Ordering::Equal);
            }
            flags.push((field.name.clone(), column));
        }
        let k = flags.len();
        let counts: Vec<Vec<usize>> = (0..k)
            .map(|i| {
                (0..k)
                    .map(|j| {
                        if i == j {
                            0
                        } else {
                            flags[i]
                                .1
                                .iter()
                                .zip(&flags[j].1)
                                .filter(|(a, b)| **a && **b)
                                .count()
                        }
                    })
                    .collect()
            })
            .collect();
        let mut header = vec![String::new()];
        header.extend(flags.iter().map(|(name, _)| name.clone()));
        if self.show_totals {
            header.push("Total".into());
        }
        let mut rows: Vec<Vec<String>> = (0..k)
            .map(|i| {
                let mut line = vec![flags[i].0.clone()];
                line.extend((0..k).map(|j| {
                    if i == j {
                        String::new()
                    } else {
                        counts[i][j].to_string()
                    }
                }));
                if self.show_totals {
                    line.push(counts[i].iter().sum::<usize>().to_string());
                }
                line
            })
            .collect();
        if self.show_totals {
            let mut total = vec!["Total".to_string()];
            total.extend((0..k).map(|j| (0..k).map(|i| counts[i][j]).sum::<usize>().to_string()));
            total.push(counts.iter().flatten().sum::<usize>().to_string());
            rows.push(total);
        }
        Ok(DocumentTable {
            title: None,
            header,
            rows,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn build_counts_table(
        &self,
        input: &Table,
        row_pos: usize,
        row_field: &Field,
        row_cats: &[Value],
        col_pos: usize,
        col_field: &Field,
        col_cats: &[Value],
    ) -> DocumentTable {
        let nr = row_cats.len();
        let nc = col_cats.len();
        let mut counts = vec![vec![0usize; nc]; nr];
        let mut row_totals = vec![0usize; nr];
        let mut col_totals = vec![0usize; nc];
        let mut grand_total = 0usize;

        for row in input.rows() {
            let (Some(rv), Some(cv)) = (row.get(row_pos), row.get(col_pos)) else {
                continue;
            };
            if !self.include_missing && (row_field.is_missing(rv) || col_field.is_missing(cv)) {
                continue;
            }
            let Some(ri) = row_cats
                .iter()
                .position(|c| c.total_cmp(rv) == Ordering::Equal)
            else {
                continue;
            };
            let Some(ci) = col_cats
                .iter()
                .position(|c| c.total_cmp(cv) == Ordering::Equal)
            else {
                continue;
            };
            counts[ri][ci] += 1;
            row_totals[ri] += 1;
            col_totals[ci] += 1;
            grand_total += 1;
        }

        let multi_subrow = self.crosstab.len() > 1;
        let mut header = Vec::new();
        header.push(self.row_field.clone());
        if multi_subrow {
            header.push(String::new());
        }
        for c in col_cats {
            header.push(category_label(c));
        }
        if self.show_totals {
            header.push("Total".to_string());
        }

        let mut rows = Vec::new();
        for (ri, rcat) in row_cats.iter().enumerate() {
            let r_label = category_label(rcat);
            for (sub_idx, &item) in self.crosstab.iter().enumerate() {
                let mut out_row = Vec::new();
                if multi_subrow {
                    out_row.push(if sub_idx == 0 {
                        r_label.clone()
                    } else {
                        String::new()
                    });
                    out_row.push(crosstab_label(item).to_string());
                } else {
                    out_row.push(r_label.clone());
                }
                for ci in 0..nc {
                    out_row.push(format_crosstab_cell(
                        item,
                        counts[ri][ci],
                        row_totals[ri],
                        col_totals[ci],
                        grand_total,
                    ));
                }
                if self.show_totals {
                    out_row.push(format_row_total_cell(item, row_totals[ri], grand_total));
                }
                rows.push(out_row);
            }
        }

        if self.show_totals {
            for (sub_idx, &item) in self.crosstab.iter().enumerate() {
                let mut out_row = Vec::new();
                if multi_subrow {
                    out_row.push(if sub_idx == 0 {
                        "Total".to_string()
                    } else {
                        String::new()
                    });
                    out_row.push(crosstab_label(item).to_string());
                } else {
                    out_row.push("Total".to_string());
                }
                for &col_tot in &col_totals {
                    out_row.push(format_col_total_cell(item, col_tot, grand_total));
                }
                out_row.push(format_grand_total_cell(item, grand_total));
                rows.push(out_row);
            }
        }

        DocumentTable {
            title: None,
            header,
            rows,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_function_table(
        &self,
        input: &Table,
        row_pos: usize,
        row_field: &Field,
        row_cats: &[Value],
        col_pos: usize,
        col_field: &Field,
        col_cats: &[Value],
        ov_pos: usize,
        ov_field: &Field,
    ) -> DocumentTable {
        let nr = row_cats.len();
        let nc = col_cats.len();
        let mut cells: Vec<Vec<Vec<f64>>> = vec![vec![Vec::new(); nc]; nr];

        for row in input.rows() {
            let (Some(rv), Some(cv), Some(ov)) =
                (row.get(row_pos), row.get(col_pos), row.get(ov_pos))
            else {
                continue;
            };
            if !self.include_missing && (row_field.is_missing(rv) || col_field.is_missing(cv)) {
                continue;
            }
            if ov_field.is_missing(ov) {
                continue;
            }
            let Some(x) = ov.as_f64().filter(|x| x.is_finite()) else {
                continue;
            };
            let Some(ri) = row_cats
                .iter()
                .position(|c| c.total_cmp(rv) == Ordering::Equal)
            else {
                continue;
            };
            let Some(ci) = col_cats
                .iter()
                .position(|c| c.total_cmp(cv) == Ordering::Equal)
            else {
                continue;
            };
            cells[ri][ci].push(x);
        }

        let mut header = Vec::with_capacity(1 + nc);
        header.push(self.row_field.clone());
        for c in col_cats {
            header.push(category_label(c));
        }

        let mut rows = Vec::with_capacity(nr);
        for (rcat, row_cells) in row_cats.iter().zip(&cells) {
            let mut out_row = Vec::with_capacity(1 + nc);
            out_row.push(category_label(rcat));
            for values in row_cells {
                out_row.push(format_function_cell(values, self.overlay_fn));
            }
            rows.push(out_row);
        }

        DocumentTable {
            title: None,
            header,
            rows,
        }
    }
}

/// Matrix categories for a row or column field:
/// - Declared categories (`ValueSpec::Categories` or `ValueSpec::Flag` in `false_value`, `true_value`
///   sorted order) when available, otherwise distinct observed values sorted by `Value::total_cmp`.
/// - When `include_missing` is `true` and `$null$` occurs in the data (and is not already in the
///   category list), `Value::Null` is placed first.
/// - When `include_missing` is `false`, any missing/blank category is filtered out.
fn matrix_categories(
    input: &Table,
    col: usize,
    field: &Field,
    include_missing: bool,
) -> Vec<Value> {
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
    } else if include_missing {
        let has_null = input
            .rows()
            .iter()
            .any(|r| r.get(col).is_some_and(Value::is_null));
        if has_null && !cats.iter().any(Value::is_null) {
            cats.insert(0, Value::Null);
        }
    }

    if !include_missing {
        cats.retain(|v| !field.is_missing(v));
    }
    cats
}

fn category_label(v: &Value) -> String {
    match v {
        Value::Null => "$null$".to_string(),
        other => other.to_string(),
    }
}

fn crosstab_label(item: CrossTabItem) -> &'static str {
    match item {
        CrossTabItem::Count => "Count",
        CrossTabItem::Expected => "Expected",
        CrossTabItem::Residual => "Residual",
        CrossTabItem::RowPercent => "Row %",
        CrossTabItem::ColPercent => "Column %",
        CrossTabItem::TotPercent => "Total %",
    }
}

fn format_crosstab_cell(
    item: CrossTabItem,
    count: usize,
    row_tot: usize,
    col_tot: usize,
    grand_tot: usize,
) -> String {
    match item {
        CrossTabItem::Count => count.to_string(),
        CrossTabItem::Expected => {
            if grand_tot == 0 {
                "0.000".to_string()
            } else {
                let exp = (row_tot as f64) * (col_tot as f64) / (grand_tot as f64);
                format_real(exp, MATRIX_DECIMALS)
            }
        }
        CrossTabItem::Residual => {
            if grand_tot == 0 {
                "0.000".to_string()
            } else {
                let exp = (row_tot as f64) * (col_tot as f64) / (grand_tot as f64);
                let res = (count as f64) - exp;
                format_real(res, MATRIX_DECIMALS)
            }
        }
        CrossTabItem::RowPercent => {
            if row_tot == 0 {
                "0.000".to_string()
            } else {
                let pct = (count as f64) * 100.0 / (row_tot as f64);
                format_real(pct, MATRIX_DECIMALS)
            }
        }
        CrossTabItem::ColPercent => {
            if col_tot == 0 {
                "0.000".to_string()
            } else {
                let pct = (count as f64) * 100.0 / (col_tot as f64);
                format_real(pct, MATRIX_DECIMALS)
            }
        }
        CrossTabItem::TotPercent => {
            if grand_tot == 0 {
                "0.000".to_string()
            } else {
                let pct = (count as f64) * 100.0 / (grand_tot as f64);
                format_real(pct, MATRIX_DECIMALS)
            }
        }
    }
}

fn format_row_total_cell(item: CrossTabItem, row_tot: usize, grand_tot: usize) -> String {
    match item {
        CrossTabItem::Count | CrossTabItem::Expected => row_tot.to_string(),
        CrossTabItem::Residual => "0".to_string(),
        CrossTabItem::RowPercent => {
            if row_tot == 0 {
                "0".to_string()
            } else {
                "100".to_string()
            }
        }
        CrossTabItem::ColPercent | CrossTabItem::TotPercent => {
            if grand_tot == 0 {
                "0.000".to_string()
            } else {
                let pct = (row_tot as f64) * 100.0 / (grand_tot as f64);
                format_real(pct, MATRIX_DECIMALS)
            }
        }
    }
}

fn format_col_total_cell(item: CrossTabItem, col_tot: usize, grand_tot: usize) -> String {
    match item {
        CrossTabItem::Count | CrossTabItem::Expected => col_tot.to_string(),
        CrossTabItem::Residual => "0".to_string(),
        CrossTabItem::ColPercent => {
            if col_tot == 0 {
                "0".to_string()
            } else {
                "100".to_string()
            }
        }
        CrossTabItem::RowPercent | CrossTabItem::TotPercent => {
            if grand_tot == 0 {
                "0.000".to_string()
            } else {
                let pct = (col_tot as f64) * 100.0 / (grand_tot as f64);
                format_real(pct, MATRIX_DECIMALS)
            }
        }
    }
}

fn format_grand_total_cell(item: CrossTabItem, grand_tot: usize) -> String {
    match item {
        CrossTabItem::Count | CrossTabItem::Expected => grand_tot.to_string(),
        CrossTabItem::Residual => "0".to_string(),
        CrossTabItem::RowPercent | CrossTabItem::ColPercent | CrossTabItem::TotPercent => {
            if grand_tot == 0 {
                "0".to_string()
            } else {
                "100".to_string()
            }
        }
    }
}

fn format_function_cell(values: &[f64], func: OverlayFunction) -> String {
    let n = values.len();
    if n == 0 {
        return "0".to_string();
    }
    let val = match func {
        OverlayFunction::Mean => values.iter().sum::<f64>() / (n as f64),
        OverlayFunction::Sum => values.iter().sum::<f64>(),
        OverlayFunction::StdDev => {
            if n < 2 {
                return "0".to_string();
            }
            let mean = values.iter().sum::<f64>() / (n as f64);
            let ss: f64 = values
                .iter()
                .map(|&v| {
                    let d = v - mean;
                    d * d
                })
                .sum();
            (ss / ((n - 1) as f64)).sqrt()
        }
        OverlayFunction::Max => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        OverlayFunction::Min => values.iter().copied().fold(f64::INFINITY, f64::min),
    };
    format_real(val, MATRIX_DECIMALS)
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

const MATRIX_CSS: &str = "<STYLE>\r\n\r\n   .bodyStyle  {\r\n\t\t   font-family: Verdana, arial, geneva, helvetica, sans-serif;\r\n\t\t   font-style : normal;\r\n\t\t   font-size: 100%;\r\n\t\t   background-color: #ffffff;\r\n\t\t   color : #000000;\r\n   }\r\n\t\t   \r\n\r\n   .rowHeaderStyle  {\r\n\t\t   font-family: Verdana, arial, geneva, helvetica, sans-serif;\r\n\t\t   font-style : normal;\r\n\t\t   font-size: 100%;\r\n\t\t   background-color: #808080;\r\n\t\t   color : #ffffff;\r\n   }\r\n\t\t   \r\n\r\n   .colHeaderStyle  {\r\n\t\t   font-family: Verdana, arial, geneva, helvetica, sans-serif;\r\n\t\t   font-style : normal;\r\n\t\t   font-size: 100%;\r\n\t\t   background-color: #808080;\r\n\t\t   color : #ffffff;\r\n   }\r\n\t\t   \r\n\r\n   .blkTextWhiteBk  {\r\n\t\t   font-family: Verdana, arial, geneva, helvetica, sans-serif;\r\n\t\t   font-weight : bold;\r\n\t\t   font-style : normal;\r\n\t\t   font-size: 100%;\r\n\t\t   background-color: #ffffff;\r\n\t\t   color : #000000;\r\n   }\r\n\t\t   \r\n\r\n   .top  {\r\n\t\t   font-family: Verdana, arial, geneva, helvetica, sans-serif;\r\n\t\t   font-weight : bold;\r\n\t\t   font-style : normal;\r\n\t\t   font-size: 100%;\r\n\t\t   background-color: #ffffff;\r\n\t\t   color : #ff0000;\r\n   }\r\n\t\t   \r\n\r\n   .bottom  {\r\n\t\t   font-family: Verdana, arial, geneva, helvetica, sans-serif;\r\n\t\t   font-weight : bold;\r\n\t\t   font-style : normal;\r\n\t\t   font-size: 100%;\r\n\t\t   background-color: #ffffff;\r\n\t\t   color : #00ff00;\r\n   }\r\n\t\t   \r\n\r\n   .selected  {\r\n\t\t   font-family: Verdana, arial, geneva, helvetica, sans-serif;\r\n\t\t   font-weight : bold;\r\n\t\t   font-style : normal;\r\n\t\t   font-size: 100%;\r\n\t\t   background-color: #ffffff;\r\n\t\t   color : #ff0000;\r\n   }\r\n\t\t   \r\n\r\n   .unselected  {\r\n\t\t   font-family: Verdana, arial, geneva, helvetica, sans-serif;\r\n\t\t   font-weight : bold;\r\n\t\t   font-style : normal;\r\n\t\t   font-size: 100%;\r\n\t\t   background-color: #ffffff;\r\n\t\t   color : #000000;\r\n   }\r\n\t\t   \r\n</STYLE>\r\n";

fn matrix_html(
    row_field: &str,
    col_field: &str,
    table: &DocumentTable,
    include_missing: bool,
) -> String {
    let mut lines = Vec::new();
    lines.push("<HTML><HEAD>".to_string());
    lines.push("<META HTTP-EQUIV=\"Cache-Control\" CONTENT=\"no-cache\">".to_string());
    lines.push("<META HTTP-EQUIV=\"Pragma\" CONTENT=\"no-cache\">".to_string());
    lines
        .push("<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html;charset=UTF-8\">".to_string());
    lines.push(
        "<LINK REL=\"stylesheet\" HREF=\"clementine.css\" type=\"text/css\"></HEAD>".to_string(),
    );
    lines.push(format!(
        "<CENTER><STRONG><FONT FACE=\"\" SIZE=5 COLOR=#000000>Matrix of {row_field} by {col_field}</FONT></STRONG></CENTER><BR><TABLE WIDTH=\"100%\" BORDER=\"2\" role=\"presentation\" cellspacing=\"0\" cellpadding=\"0\">"
    ));
    lines.push("<TR>".to_string());
    for cell in &table.header {
        lines.push(format!(
            "<TD CLASS=\"colHeaderStyle\" ALIGN=\"CENTER\" NOWRAP><NOBR><FONT CLASS=\"colHeaderStyle\">{cell}</FONT></NOBR></TD>"
        ));
    }
    lines.push("</TR>".to_string());
    for row in &table.rows {
        lines.push("<TR>".to_string());
        for (ci, cell) in row.iter().enumerate() {
            if ci == 0 {
                lines.push(format!(
                    "<TD CLASS=\"rowHeaderStyle\" ALIGN=\"CENTER\" NOWRAP><NOBR><FONT CLASS=\"rowHeaderStyle\">{cell}</FONT></NOBR></TD>"
                ));
            } else {
                lines.push(format!(
                    "<TD CLASS=\"unselected\" ALIGN=\"CENTER\" NOWRAP><NOBR><FONT CLASS=\"unselected\">{cell}</FONT></NOBR></TD>"
                ));
            }
        }
        lines.push("</TR>".to_string());
    }
    let (chi2, df, prob) = chi_square_footer(table);
    let missing = if include_missing {
        "including missing values"
    } else {
        "excluding missing values"
    };
    lines.push(format!(
        "</TABLE><P>Cells contain: cross-tabulation of fields ({missing})"
    ));
    lines.push(format!(
        "<BR>Chi-square = {chi2}, df = {df}, probability = {prob}</P>"
    ));
    lines.push("<P ALIGN=CENTER>&nbsp;&nbsp;&nbsp;</P></HTML>".to_string());
    lines.join("\r\n")
}

fn chi_square_footer(table: &DocumentTable) -> (String, String, String) {
    let ncols = table.header.len();
    let nrows = table.rows.len();
    let has_totals = ncols > 1
        && nrows > 0
        && table.header.last().is_some_and(|h| h == "Total")
        && table
            .rows
            .last()
            .is_some_and(|r| r.first().is_some_and(|c| c == "Total"));
    if !has_totals || ncols < 3 || nrows < 2 {
        return ("--".to_string(), "--".to_string(), "--".to_string());
    }
    let inner_cols = ncols - 2;
    let inner_rows = nrows - 1;
    let mut col_totals: Vec<f64> = vec![0.0; inner_cols];
    let mut row_totals: Vec<f64> = vec![0.0; inner_rows];
    let mut observed: Vec<Vec<f64>> = vec![vec![0.0; inner_cols]; inner_rows];
    for (ri, row) in table.rows.iter().take(inner_rows).enumerate() {
        for (ci, cell) in row.iter().skip(1).take(inner_cols).enumerate() {
            let v: f64 = cell.parse().unwrap_or(0.0);
            observed[ri][ci] = v;
            row_totals[ri] += v;
        }
    }
    if let Some(total_row) = table.rows.last() {
        for (ci, cell) in total_row.iter().skip(1).take(inner_cols).enumerate() {
            col_totals[ci] = cell.parse().unwrap_or(0.0);
        }
    }
    let grand: f64 = row_totals.iter().sum();
    if grand <= 0.0 {
        return ("0".to_string(), "0".to_string(), "1".to_string());
    }
    let mut chi2 = 0.0f64;
    for ri in 0..inner_rows {
        for ci in 0..inner_cols {
            let expected = row_totals[ri] * col_totals[ci] / grand;
            if expected > 0.0 {
                let diff = observed[ri][ci] - expected;
                chi2 += diff * diff / expected;
            }
        }
    }
    let df = inner_rows.saturating_sub(1) * inner_cols.saturating_sub(1);
    let prob = dms_algo::stats::chi_square_p(chi2, df as f64);
    (trimmed(chi2), df.to_string(), trimmed(prob))
}

fn trimmed(value: f64) -> String {
    if !value.is_finite() {
        return "--".to_string();
    }
    let text = format_real(value, 3);
    let Some((int_part, frac)) = text.split_once('.') else {
        return text;
    };
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() {
        int_part.to_string()
    } else {
        format!("{int_part}.{frac}")
    }
}
