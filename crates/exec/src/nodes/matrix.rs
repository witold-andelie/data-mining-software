//! Matrix (Output palette): builds a two-dimensional cross-tabulation or function summary matrix
//! across `rowField` and `columnField`, returning a [`Document`] with one table matching Modeler's
//! exported CSV (`docs/oracle.md`, `docs/formats/nodes/matrix.md`).
//!
//! Implemented settings (saved names, from the `outputs` probes and `team/cluster_analysis`):
//! - `matrixType`: `specified` only (the selected row and column fields); the flag and numeric
//!   matrices are rejected.
//! - `rowField`, `columnField`: categorical fields for the matrix rows and columns (required).
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
//!   `highlightBottom`) are validated; `outputMode` values other than `screen` are rejected with
//!   [`OperatorError::Settings`].

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::nodes::field_util::position;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput, format_real};
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::{Field, Table, Value, ValueSpec};
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

#[derive(Debug, Clone)]
struct MatrixOp {
    row_field: String,
    col_field: String,
    cell_mode: CellMode,
    overlay_field: Option<String>,
    overlay_fn: OverlayFunction,
    crosstab: Vec<CrossTabItem>,
    show_totals: bool,
    include_missing: bool,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();

    match settings::text(s, "outputMode").as_deref().map(str::trim) {
        None | Some("") | Some("screen") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Matrix outputMode {other:?} is not supported"
            )));
        }
    }

    let _ = settings::flag(s, "useHighlightTop")?;
    let _ = settings::integer(s, "highlightTop")?;
    let _ = settings::flag(s, "useHighlightBottom")?;
    let _ = settings::integer(s, "highlightBottom")?;

    let row_field = settings::text(s, "rowField")
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| OperatorError::Settings("Matrix requires a rowField".into()))?;
    let col_field = settings::text(s, "columnField")
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| OperatorError::Settings("Matrix requires a columnField".into()))?;

    match settings::text(s, "matrixType").as_deref().map(str::trim) {
        None | Some("") | Some("specified") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "Matrix matrixType {other:?} is not supported yet"
            )));
        }
    }
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
        row_field,
        col_field,
        cell_mode,
        overlay_field,
        overlay_fn,
        crosstab,
        show_totals,
        include_missing,
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
        _context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Matrix needs one input".into()))?;

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

        Ok(NodeOutput::Document(Arc::new(Document {
            title: String::new(),
            tables: vec![table],
        })))
    }
}

impl MatrixOp {
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
