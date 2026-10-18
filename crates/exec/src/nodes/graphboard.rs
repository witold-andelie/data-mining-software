//! Graphboard (`GraphBoardNode`, catalog id `graphboard`): the data behind one graph, as
//! Modeler's VIZML export holds it (`embeddedSource` in `oracle/graphboard/*`,
//! `xtask/oracle/probes/graphboard_streams.py`). The saved `graphType` is the template.
//! `graphAxisSummary` stays `MEAN` on every case and does not override that template
//! (`pie_categories` still sums; `histogram_amount` still counts).
//!
//! Drawing settings (fonts, labels, colours, gridlines, panel layout, image size, output file
//! type) are ignored. `graphBoardInputFields` is the field list a successful run stored; column
//! order comes from the slots below, which match it (`bubble_score_amount`: weight, score,
//! amount; `scatter_color_panel`: score, amount, channel, region, segment).
//!
//! Rules, each shown by the case named with it:
//! - Histogram (`histogram_amount`): non-missing `graphXField` values. The bin width is the
//!   Heckbert ceiling of `max - min` (1, 2 or 5 times a power of ten; the span 19.87 ceilings
//!   to 20) divided by 12. Edges accumulate from the minimum (`edge += width`); a value goes
//!   to the greatest bin whose left edge is `<=` the value, and a value past the last edge
//!   stays in the last bin. The centre is the midpoint of the two edges. Empty bins are left
//!   out. System-missing values are their own first row (an empty bin cell and `count`).
//!   Cells are Java doubles (`6.0`, `-6.916666666666666`). Header `{x}_bin`, `count`.
//! - Pie (`pie_categories`) and bar (`bar_region_amount`): one row per `graphCategoriesField`
//!   value, first appearance, the sum of non-missing `graphValuesField`. Header
//!   `{categories}`, `{values}_sum`. A blank category is the cell `""` (`&quot;&quot;` in the
//!   VIZML). A group whose values are all missing keeps the row and leaves the sum empty.
//! - Pie and bar of counts (`piecounts_region`, `barcounts_region`): one row per
//!   `graphCategoriesField` value, first appearance, the number of records (blank categories
//!   included). Header `{categories}`, `count`. A bar of counts with `graphColorField`
//!   (`bar_counts_color`) groups by (categories, color) pairs in first appearance; header
//!   `{categories}`, `{color}`, `count`.
//! - Line (`line_score_amount`) and area (`area_score_amount`): one row per `graphXField`
//!   value, first appearance, the mean of non-missing `graphYField`. Header `{x}`, `{y}_mean`.
//!   A blank numeric x is an empty cell (the blank score mean is `-0.625`).
//! - Heatmap (`heatmap_rows`): one row per (`graphColumnsField`, `graphRowsField`), first
//!   appearance, the mean of non-missing `graphColorField`. Header columns, rows,
//!   `{color}_mean`. Store with a blank region has no present amount, so the mean cell is
//!   empty and the row stays.
//! - Scatter (`scatter_score_amount`, `scatter_color_panel`), dot plot (`dotplot_amount`),
//!   bubble (`bubble_score_amount`) and parallel (`parallel_fields`): one row per record, in
//!   input order. Columns: scatter is x, y, then `graphDownField`, `graphAcrossField` and
//!   `graphColorField` when set (the panel case stores channel in `graphDownField` and region
//!   in `graphAcrossField`); bubble is size, x, y; dot plot is x; parallel is `graphMultiFields`
//!   in saved order. A null or missing number is an empty cell; a blank string is `""`.
//!   Integers print as Java doubles.
//! - Boxplot (`boxplot_region_amount`): Tukey hinges of non-missing `graphYField` by
//!   `graphXField`. Categories are sorted by the printed category (`""`, East, North, South).
//!   Per category, five rows: median, first hinge, third hinge, upper whisker, lower whisker
//!   (duplicates kept). The half for a hinge is `ceil(n/2)` and includes the median when `n`
//!   is odd. Fences are 1.5 times the interquartile range, inclusive. Whiskers are the least
//!   and greatest inside points. Outliers follow every category block, in that category
//!   order. Header is the scale field, then the category field.
//!
//! GAP: the bin count 12 is what `histogram_amount` shows; no other Graphboard histogram was
//! captured. A constant x (one bin at that value) and several outliers in one category (emitted
//! in ascending value) are not in the cases. A category with no present y is omitted. Maps,
//! 3-D, animation, overlays, transparency, templates, an axis summary other than `MEAN`, and
//! inputs larger than `graphLargeDatasetReadThreshold` (default 2000; Modeler then samples or
//! bins) are rejected. So are graph types and field slots the cases do not use (pie or bar on
//! x/y, colour on a summed bar or any pie, heatmap on x/y, shape, panel, data labels, summary
//! field).
//!
//! `nodes::graph_data::BinSettings` is the Histogram node's binning and is not used here.

use super::graph_data::{MAX_BINS, java_double};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::{Document, DocumentTable, NodeOutput};
use crate::settings;
use dms_stream::StreamNode;
use dms_types::{Field, Storage, Table, Value};
use std::sync::Arc;

/// Bins over the Heckbert ceiling of the data span. `histogram_amount` has twelve.
const HISTOGRAM_BINS: usize = 12;

const ALWAYS_EMPTY: &[&str] = &[
    "graphAnimationField",
    "graphZField",
    "graphShapeField",
    "graphTransparencyField",
    "graphPanelField",
    "graphDatalabelField",
    "graphSummaryField",
    "graphLongitudeField",
    "graphLatitudeField",
    "graphMapColorField",
    "graphDataKeyField",
    "graphStartLongitudeField",
    "graphEndLongitudeField",
    "graphStartLatitudeField",
    "graphEndLatitudeField",
    "graphMapFile",
    "graphMapLayer",
    "graphMapAttribute",
    "graphTemplateName",
    "graphOverlayFunction",
    "graphReferenceMapFile",
    "graphReferenceMapLayer",
    "graphReferenceMapAttribute",
    "graphStyleSheet",
];

enum Kind {
    /// `{x}_bin` and `count`.
    Histogram { x: String },
    /// Key columns, then the summary column.
    Aggregate {
        keys: Vec<String>,
        summary: Summary,
        label: &'static str,
    },
    /// One input row per output row, these columns.
    Raw {
        columns: Vec<String>,
        label: &'static str,
    },
    /// Scale field, then the category field.
    Box { category: String, scale: String },
}

/// What an aggregate graph shows per key.
enum Summary {
    /// `{value}_mean` of the non-missing values.
    Mean(String),
    /// `{value}_sum` of the non-missing values.
    Sum(String),
    /// `count` of the records.
    Count,
}

struct Graphboard {
    kind: Kind,
    threshold: usize,
}

struct Slots {
    x: Option<String>,
    y: Option<String>,
    color: Option<String>,
    size: Option<String>,
    categories: Option<String>,
    values: Option<String>,
    rows: Option<String>,
    columns: Option<String>,
    down: Option<String>,
    across: Option<String>,
    multi: Vec<String>,
}

fn gap(message: &str) -> OperatorError {
    OperatorError::Settings(format!("GAP: {message}"))
}

fn named(s: Option<&dms_strfmt::fastinfoset::Element>, name: &str) -> Option<String> {
    settings::text(s, name)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn require(slot: &mut Option<String>, element: &str) -> Result<String, OperatorError> {
    slot.take()
        .ok_or_else(|| OperatorError::Settings(format!("Graphboard: no field ({element})")))
}

fn reject_text(
    s: Option<&dms_strfmt::fastinfoset::Element>,
    name: &str,
    allowed: &[&str],
) -> Result<(), OperatorError> {
    match named(s, name).as_deref() {
        None => Ok(()),
        Some(value) if allowed.contains(&value) => Ok(()),
        Some(value) => Err(gap(&format!(
            "Graphboard {name}={value:?} is not supported"
        ))),
    }
}

fn reject_flag(
    s: Option<&dms_strfmt::fastinfoset::Element>,
    name: &str,
) -> Result<(), OperatorError> {
    if settings::flag(s, name)?.unwrap_or(false) {
        return Err(gap(&format!("Graphboard {name} is not supported")));
    }
    Ok(())
}

/// Saved defaults that do not change the table, and the record threshold.
fn reject_common(s: Option<&dms_strfmt::fastinfoset::Element>) -> Result<usize, OperatorError> {
    for name in ALWAYS_EMPTY {
        if named(s, name).is_some() {
            return Err(gap(&format!("Graphboard setting {name} is not supported")));
        }
    }
    let mapped = settings::element(s, "graphMapDataFileMap")
        .is_some_and(|element| element.child_elements().next().is_some());
    if mapped {
        return Err(gap(
            "Graphboard setting graphMapDataFileMap is not supported",
        ));
    }
    reject_text(s, "graphOverlayType", &["None"])?;
    reject_text(s, "graphAxisSummary", &["MEAN"])?;
    reject_text(s, "graphLargeDatasetOption", &["Sample"])?;
    reject_text(s, "graphTransparencySummary", &["COUNT"])?;
    reject_flag(s, "graph3D")?;
    reject_flag(s, "graphIncludeOverlayFunction")?;
    reject_flag(s, "graphUseTransparency")?;
    match settings::integer(s, "graphLargeDatasetReadThreshold")? {
        None => Ok(2000),
        Some(n) => usize::try_from(n).map_err(|_| {
            OperatorError::Settings(
                "graphLargeDatasetReadThreshold: expected a non-negative integer".into(),
            )
        }),
    }
}

fn read_slots(s: Option<&dms_strfmt::fastinfoset::Element>) -> Slots {
    let multi = settings::children(s, "graphMultiFields", "field")
        .into_iter()
        .map(|field| field.text().trim().to_string())
        .filter(|field| !field.is_empty())
        .collect();
    Slots {
        x: named(s, "graphXField"),
        y: named(s, "graphYField"),
        color: named(s, "graphColorField"),
        size: named(s, "graphSizeField"),
        categories: named(s, "graphCategoriesField"),
        values: named(s, "graphValuesField"),
        rows: named(s, "graphRowsField"),
        columns: named(s, "graphColumnsField"),
        down: named(s, "graphDownField"),
        across: named(s, "graphAcrossField"),
        multi,
    }
}

fn reject_unused(slots: &Slots) -> Result<(), OperatorError> {
    let extras = [
        ("graphXField", slots.x.is_some()),
        ("graphYField", slots.y.is_some()),
        ("graphColorField", slots.color.is_some()),
        ("graphSizeField", slots.size.is_some()),
        ("graphCategoriesField", slots.categories.is_some()),
        ("graphValuesField", slots.values.is_some()),
        ("graphRowsField", slots.rows.is_some()),
        ("graphColumnsField", slots.columns.is_some()),
        ("graphDownField", slots.down.is_some()),
        ("graphAcrossField", slots.across.is_some()),
        ("graphMultiFields", !slots.multi.is_empty()),
    ];
    for (name, set) in extras {
        if set {
            return Err(gap(&format!(
                "Graphboard setting {name} is not supported for this graph type"
            )));
        }
    }
    Ok(())
}

fn build_kind(graph_type: &str, slots: &mut Slots) -> Result<Kind, OperatorError> {
    let kind = match graph_type.to_ascii_uppercase().as_str() {
        "HISTOGRAM" => Kind::Histogram {
            x: require(&mut slots.x, "graphXField")?,
        },
        "PIE" | "BAR" => {
            let label = if graph_type.eq_ignore_ascii_case("PIE") {
                "Pie"
            } else {
                "Bar"
            };
            if (slots.categories.is_none() || slots.values.is_none())
                && (slots.x.is_some() || slots.y.is_some())
            {
                return Err(gap(&format!(
                    "Graphboard {label} on graphXField/graphYField is not supported (pie_categories and bar_region_amount use graphCategoriesField and graphValuesField)"
                )));
            }
            Kind::Aggregate {
                keys: vec![require(&mut slots.categories, "graphCategoriesField")?],
                summary: Summary::Sum(require(&mut slots.values, "graphValuesField")?),
                label,
            }
        }
        "PIE_OF_COUNTS" => Kind::Aggregate {
            keys: vec![require(&mut slots.categories, "graphCategoriesField")?],
            summary: Summary::Count,
            label: "PieCounts",
        },
        "BAR_OF_COUNTS" => {
            let mut keys = vec![require(&mut slots.categories, "graphCategoriesField")?];
            keys.extend(slots.color.take());
            Kind::Aggregate {
                keys,
                summary: Summary::Count,
                label: "BarCounts",
            }
        }
        "LINE" | "AREA" => {
            let label = if graph_type.eq_ignore_ascii_case("LINE") {
                "Line"
            } else {
                "Area"
            };
            Kind::Aggregate {
                keys: vec![require(&mut slots.x, "graphXField")?],
                summary: Summary::Mean(require(&mut slots.y, "graphYField")?),
                label,
            }
        }
        "HEATMAP" => {
            if (slots.rows.is_none() || slots.columns.is_none() || slots.color.is_none())
                && (slots.x.is_some() || slots.y.is_some())
            {
                return Err(gap(
                    "Graphboard heatmap on graphXField/graphYField is not supported (heatmap_rows uses graphRowsField, graphColumnsField and graphColorField)",
                ));
            }
            Kind::Aggregate {
                keys: vec![
                    require(&mut slots.columns, "graphColumnsField")?,
                    require(&mut slots.rows, "graphRowsField")?,
                ],
                summary: Summary::Mean(require(&mut slots.color, "graphColorField")?),
                label: "Heatmap",
            }
        }
        "SCATTERPLOT" => {
            let mut columns = vec![
                require(&mut slots.x, "graphXField")?,
                require(&mut slots.y, "graphYField")?,
            ];
            columns.extend(slots.down.take());
            columns.extend(slots.across.take());
            columns.extend(slots.color.take());
            Kind::Raw {
                columns,
                label: "Scatterplot",
            }
        }
        "BUBBLE" => Kind::Raw {
            columns: vec![
                require(&mut slots.size, "graphSizeField")?,
                require(&mut slots.x, "graphXField")?,
                require(&mut slots.y, "graphYField")?,
            ],
            label: "Bubble",
        },
        "DOTPLOT" => Kind::Raw {
            columns: vec![require(&mut slots.x, "graphXField")?],
            label: "Dotplot",
        },
        "PARALLEL" => {
            let columns = std::mem::take(&mut slots.multi);
            if columns.is_empty() {
                return Err(OperatorError::Settings(
                    "Graphboard: no fields (graphMultiFields)".into(),
                ));
            }
            Kind::Raw {
                columns,
                label: "Parallel",
            }
        }
        "BOXPLOT" => Kind::Box {
            category: require(&mut slots.x, "graphXField")?,
            scale: require(&mut slots.y, "graphYField")?,
        },
        other => {
            return Err(gap(&format!(
                "Graphboard graph type {other:?} is not supported"
            )));
        }
    };
    reject_unused(slots)?;
    Ok(kind)
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let threshold = reject_common(s)?;
    let graph_type = named(s, "graphType")
        .ok_or_else(|| OperatorError::Settings("Graphboard: no graph type (graphType)".into()))?;
    let mut slots = read_slots(s);
    let kind = build_kind(&graph_type, &mut slots)?;
    Ok(Box::new(Graphboard { kind, threshold }))
}

fn label_of(kind: &Kind) -> &'static str {
    match kind {
        Kind::Histogram { .. } => "Histogram",
        Kind::Aggregate { label, .. } | Kind::Raw { label, .. } => label,
        Kind::Box { .. } => "Boxplot",
    }
}

fn position(input: &Table, name: &str) -> Result<usize, OperatorError> {
    input
        .model()
        .position(name)
        .ok_or_else(|| OperatorError::Data(format!("no field {name}")))
}

/// A string cell is `""` when null, missing or empty (`pie_categories`, `scatter_color_panel`).
/// A number is a Java double, or empty when null or missing (`dotplot_amount`, `line_score_amount`).
fn render(field: &Field, value: &Value) -> String {
    let textual = field.storage == Storage::String || matches!(value, Value::String(_));
    if textual {
        if value.is_null() || field.is_missing(value) || value.as_str().is_some_and(str::is_empty) {
            return "\"\"".to_string();
        }
        return value.to_string();
    }
    if value.is_null() || field.is_missing(value) {
        return String::new();
    }
    if let Some(number) = value.as_f64().filter(|number| number.is_finite()) {
        return java_double(number);
    }
    value.to_string()
}

fn cell_at(input: &Table, row: &[Value], at: usize) -> String {
    let Some(field) = input.model().fields().get(at) else {
        return String::new();
    };
    render(field, row.get(at).unwrap_or(&Value::Null))
}

fn number_at(input: &Table, row: &[Value], at: usize) -> Option<f64> {
    let field = input.model().fields().get(at)?;
    let value = row.get(at)?;
    if value.is_null() || field.is_missing(value) {
        return None;
    }
    value.as_f64().filter(|number| number.is_finite())
}

fn document(label: &str, table: DocumentTable) -> NodeOutput {
    NodeOutput::Document(Arc::new(Document {
        title: format!("Graphboard {label}"),
        tables: vec![table],
    }))
}

fn raw_table(input: &Table, columns: &[String]) -> Result<DocumentTable, OperatorError> {
    let positions = columns
        .iter()
        .map(|name| position(input, name))
        .collect::<Result<Vec<_>, _>>()?;
    let mut rows = Vec::with_capacity(input.row_count());
    for row in input.rows() {
        let mut cells = Vec::with_capacity(positions.len());
        for &at in &positions {
            cells.push(cell_at(input, row, at));
        }
        rows.push(cells);
    }
    Ok(DocumentTable {
        title: None,
        header: columns.to_vec(),
        rows,
    })
}

struct Accum {
    keys: Vec<String>,
    sum: f64,
    count: u64,
}

fn aggregate_table(
    input: &Table,
    keys: &[String],
    summary: &Summary,
) -> Result<DocumentTable, OperatorError> {
    let key_at = keys
        .iter()
        .map(|name| position(input, name))
        .collect::<Result<Vec<_>, _>>()?;
    let value_at = match summary {
        Summary::Mean(value) | Summary::Sum(value) => Some(position(input, value)?),
        Summary::Count => None,
    };
    let mut groups: Vec<Accum> = Vec::new();
    for row in input.rows() {
        let key_cells: Vec<String> = key_at.iter().map(|&at| cell_at(input, row, at)).collect();
        // A count takes every record; a mean or sum its non-missing values.
        let number = match value_at {
            Some(at) => number_at(input, row, at),
            None => Some(1.0),
        };
        if let Some(group) = groups.iter_mut().find(|group| group.keys == key_cells) {
            if let Some(number) = number {
                group.sum += number;
                group.count = group.count.saturating_add(1);
            }
        } else {
            let (sum, count) = match number {
                Some(number) => (number, 1),
                None => (0.0, 0),
            };
            groups.push(Accum {
                keys: key_cells,
                sum,
                count,
            });
        }
    }
    let mut header = keys.to_vec();
    header.push(match summary {
        Summary::Mean(value) => format!("{value}_mean"),
        Summary::Sum(value) => format!("{value}_sum"),
        Summary::Count => "count".to_string(),
    });
    let rows = groups
        .into_iter()
        .map(|group| {
            let mut cells = group.keys;
            let stat = if group.count == 0 {
                String::new()
            } else {
                let total = group.count as f64;
                java_double(match summary {
                    Summary::Mean(_) => group.sum / total,
                    Summary::Sum(_) | Summary::Count => group.sum,
                })
            };
            cells.push(stat);
            cells
        })
        .collect();
    Ok(DocumentTable {
        title: None,
        header,
        rows,
    })
}

/// Heckbert's "nice" ceiling: 1, 2 or 5 times a power of ten (`histogram_amount`: 19.87 → 20).
fn nice_ceil(span: f64) -> Option<f64> {
    if !(span.is_finite() && span > 0.0) {
        return None;
    }
    let exp = span.log10().floor();
    if !exp.is_finite() {
        return None;
    }
    let pow = 10f64.powf(exp);
    if !(pow.is_finite() && pow > 0.0) {
        return None;
    }
    let frac = span / pow;
    if !frac.is_finite() {
        return None;
    }
    let nice = if frac <= 1.0 {
        1.0
    } else if frac <= 2.0 {
        2.0
    } else if frac <= 5.0 {
        5.0
    } else {
        10.0
    };
    let result = nice * pow;
    (result.is_finite() && result > 0.0).then_some(result)
}

enum Scale {
    /// Every present value is equal (`histogram_amount` does not show this).
    One(f64),
    Many {
        edges: Vec<f64>,
        width: f64,
    },
}

fn histogram_scale(low: f64, high: f64) -> Result<Scale, OperatorError> {
    if high == low {
        return Ok(Scale::One(low));
    }
    let span = high - low;
    let nice = nice_ceil(span).ok_or_else(|| {
        OperatorError::Data(format!("Graphboard histogram cannot bin a range of {span}"))
    })?;
    let bins = HISTOGRAM_BINS.clamp(1, MAX_BINS);
    let width = nice / (bins as f64);
    if !(width.is_finite() && width > 0.0) {
        return Err(OperatorError::Data(
            "Graphboard histogram bin width is not usable".into(),
        ));
    }
    let mut edges = Vec::with_capacity(bins);
    let mut edge = low;
    for _ in 0..bins {
        if !edge.is_finite() {
            return Err(OperatorError::Data(
                "Graphboard histogram bin edge is not finite".into(),
            ));
        }
        edges.push(edge);
        edge += width;
    }
    Ok(Scale::Many { edges, width })
}

fn bin_index(value: f64, edges: &[f64]) -> usize {
    let mut chosen = 0;
    for (index, left) in edges.iter().enumerate() {
        if *left <= value {
            chosen = index;
        } else {
            break;
        }
    }
    chosen.min(edges.len().saturating_sub(1))
}

fn bin_centre(edges: &[f64], index: usize, width: f64) -> Option<f64> {
    let left = *edges.get(index)?;
    let right = edges
        .get(index.saturating_add(1))
        .copied()
        .unwrap_or(left + width);
    Some((left + right) / 2.0)
}

fn classify(scale: &Scale, value: f64) -> (usize, f64) {
    match scale {
        Scale::One(centre) => (0, *centre),
        Scale::Many { edges, width } => {
            let index = bin_index(value, edges);
            let centre = bin_centre(edges, index, *width).unwrap_or(value);
            (index, centre)
        }
    }
}

fn bump(rows: &mut Vec<(Option<f64>, u64)>, slot: &mut Option<usize>, centre: Option<f64>) {
    if slot.is_none() {
        *slot = Some(rows.len());
        rows.push((centre, 0));
    }
    let Some(at) = *slot else {
        return;
    };
    if let Some(row) = rows.get_mut(at) {
        row.1 = row.1.saturating_add(1);
    }
}

fn histogram_table(input: &Table, x: &str) -> Result<DocumentTable, OperatorError> {
    let at = position(input, x)?;
    let mut low = None;
    let mut high = None;
    for row in input.rows() {
        if let Some(number) = number_at(input, row, at) {
            low = Some(low.map_or(number, |seen: f64| seen.min(number)));
            high = Some(high.map_or(number, |seen: f64| seen.max(number)));
        }
    }
    let scale = match (low, high) {
        (Some(low), Some(high)) => Some(histogram_scale(low, high)?),
        _ => None,
    };
    let slot_count = match &scale {
        Some(Scale::Many { edges, .. }) => edges.len(),
        Some(Scale::One(_)) => 1,
        None => 0,
    };
    let mut slots = vec![None; slot_count];
    let mut missing = None;
    let mut seen: Vec<(Option<f64>, u64)> = Vec::new();
    for row in input.rows() {
        match number_at(input, row, at) {
            None => bump(&mut seen, &mut missing, None),
            Some(number) => {
                let Some(scale) = &scale else {
                    continue;
                };
                let (index, centre) = classify(scale, number);
                if let Some(slot) = slots.get_mut(index) {
                    bump(&mut seen, slot, Some(centre));
                }
            }
        }
    }
    let rows = seen
        .into_iter()
        .map(|(centre, count)| {
            vec![
                centre.map(java_double).unwrap_or_default(),
                java_double(count as f64),
            ]
        })
        .collect();
    Ok(DocumentTable {
        title: None,
        header: vec![format!("{x}_bin"), "count".to_string()],
        rows,
    })
}

fn median(sorted: &[f64]) -> Option<f64> {
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    if n % 2 == 1 {
        sorted.get(n / 2).copied()
    } else {
        let left = sorted.get(n / 2 - 1)?;
        let right = sorted.get(n / 2)?;
        Some((*left + *right) / 2.0)
    }
}

struct BoxStats {
    median: f64,
    q1: f64,
    q3: f64,
    upper: f64,
    lower: f64,
    outliers: Vec<f64>,
}

/// Five-number box of `boxplot_region_amount`. `values` is sorted here.
fn box_stats(values: &mut [f64]) -> Option<BoxStats> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|left, right| left.total_cmp(right));
    let mid = median(values)?;
    let n = values.len();
    let half = n.div_ceil(2);
    let lower = values.get(..half)?;
    let upper = values.get(n.saturating_sub(half)..)?;
    let q1 = median(lower)?;
    let q3 = median(upper)?;
    let iqr = q3 - q1;
    let fence_low = q1 - 1.5 * iqr;
    let fence_high = q3 + 1.5 * iqr;
    let mut inside_low = None;
    let mut inside_high = None;
    let mut outliers = Vec::new();
    for &value in values.iter() {
        if value >= fence_low && value <= fence_high {
            inside_low = Some(inside_low.map_or(value, |seen: f64| seen.min(value)));
            inside_high = Some(inside_high.map_or(value, |seen: f64| seen.max(value)));
        } else {
            outliers.push(value);
        }
    }
    let (lower_whisker, upper_whisker) = match (inside_low, inside_high) {
        (Some(low), Some(high)) => (low, high),
        // No point sits inside the fences. Not shown by the case.
        _ => (q1, q3),
    };
    Some(BoxStats {
        median: mid,
        q1,
        q3,
        upper: upper_whisker,
        lower: lower_whisker,
        outliers,
    })
}

struct BoxGroup {
    category: String,
    values: Vec<f64>,
}

fn box_table(input: &Table, category: &str, scale: &str) -> Result<DocumentTable, OperatorError> {
    let category_at = position(input, category)?;
    let scale_at = position(input, scale)?;
    let mut groups: Vec<BoxGroup> = Vec::new();
    for row in input.rows() {
        let Some(number) = number_at(input, row, scale_at) else {
            continue;
        };
        let key = cell_at(input, row, category_at);
        if let Some(group) = groups.iter_mut().find(|group| group.category == key) {
            group.values.push(number);
        } else {
            groups.push(BoxGroup {
                category: key,
                values: vec![number],
            });
        }
    }
    groups.sort_by(|left, right| left.category.cmp(&right.category));
    let mut rows = Vec::new();
    let mut pending = Vec::new();
    for group in &mut groups {
        let Some(stats) = box_stats(&mut group.values) else {
            continue;
        };
        for stat in [stats.median, stats.q1, stats.q3, stats.upper, stats.lower] {
            rows.push(vec![java_double(stat), group.category.clone()]);
        }
        pending.push((group.category.clone(), stats.outliers));
    }
    for (category, outliers) in pending {
        for value in outliers {
            rows.push(vec![java_double(value), category.clone()]);
        }
    }
    Ok(DocumentTable {
        title: None,
        header: vec![scale.to_string(), category.to_string()],
        rows,
    })
}

impl Operator for Graphboard {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Graphboard is a graph node and produces a Document, not a Table".into(),
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
            .ok_or_else(|| OperatorError::Inputs("Graphboard needs one input".into()))?;
        if input.row_count() > self.threshold {
            return Err(gap(&format!(
                "Graphboard of more than {} records is not supported (graphLargeDatasetOption samples or bins them)",
                self.threshold
            )));
        }
        let table = match &self.kind {
            Kind::Histogram { x } => histogram_table(input, x)?,
            Kind::Aggregate { keys, summary, .. } => aggregate_table(input, keys, summary)?,
            Kind::Raw { columns, .. } => raw_table(input, columns)?,
            Kind::Box { category, scale } => box_table(input, category, scale)?,
        };
        Ok(document(label_of(&self.kind), table))
    }
}

#[cfg(test)]
mod tests {
    use super::java_double;
    use super::{box_stats, nice_ceil};
    use crate::context::ExecContext;
    use crate::output::NodeOutput;
    use dms_stream::StreamNode;
    use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
    use dms_types::{DataModel, Field, Storage, Table, Value};
    use std::sync::Arc;

    fn qname(local: &str) -> QualifiedName {
        QualifiedName {
            prefix: None,
            namespace: None,
            local: Arc::from(local),
        }
    }

    fn prop(name: &str, text: &str) -> Node {
        Node::Element(Element {
            name: qname(name),
            namespace_declarations: Vec::new(),
            attributes: Vec::new(),
            children: vec![Node::Text(Arc::from(text))],
        })
    }

    fn properties(children: Vec<Node>) -> StreamNode {
        StreamNode {
            uid: "g".into(),
            class: "GraphBoardNode".into(),
            extension: None,
            model_builder: None,
            name: None,
            label: None,
            position: None,
            properties_attachment: None,
            containers: Vec::new(),
            settings: Some(Element {
                name: qname("properties"),
                namespace_declarations: Vec::new(),
                attributes: Vec::new(),
                children,
            }),
            binning_output: None,
            result_items: Vec::new(),
            subdiagram: None,
        }
    }

    fn run(
        settings_nodes: Vec<Node>,
        input: Table,
    ) -> Result<Arc<crate::Document>, crate::OperatorError> {
        let operator = super::build(&properties(settings_nodes))?;
        match operator.produce(&[Arc::new(input)], None, &ExecContext::default())? {
            NodeOutput::Document(document) => Ok(document),
            NodeOutput::Data(_) | NodeOutput::Model(_) => {
                panic!("graphboard produced a non-document")
            }
        }
    }

    fn settings_error(
        result: Result<Box<dyn crate::operator::Operator>, crate::OperatorError>,
    ) -> String {
        match result {
            Err(crate::OperatorError::Settings(message)) => message,
            Err(other) => panic!("expected a settings error, got {other:?}"),
            Ok(_) => panic!("expected a settings error, built an operator"),
        }
    }

    fn strings(names: &[(&str, Storage)], rows: Vec<Vec<Value>>) -> Table {
        let model = DataModel::new(
            names
                .iter()
                .map(|(name, storage)| Field::new(*name, *storage))
                .collect(),
        )
        .unwrap();
        Table::new(model, rows).unwrap()
    }

    #[test]
    fn other_graph_types_and_settings_are_rejected() {
        let bar = settings_error(super::build(&properties(vec![
            prop("graphType", "Bar"),
            prop("graphXField", "region"),
            prop("graphYField", "amount"),
        ])));
        assert!(bar.contains("GAP:"), "{bar}");
        assert!(bar.contains("graphXField"), "{bar}");

        let pie_axes = settings_error(super::build(&properties(vec![
            prop("graphType", "Pie"),
            prop("graphXField", "region"),
            prop("graphYField", "amount"),
        ])));
        assert!(pie_axes.contains("GAP:"), "{pie_axes}");

        let solid = settings_error(super::build(&properties(vec![
            prop("graphType", "HISTOGRAM"),
            prop("graphXField", "amount"),
            prop("graph3D", "true"),
        ])));
        assert!(solid.contains("GAP:"), "{solid}");

        let mean = super::build(&properties(vec![
            prop("graphType", "LINE"),
            prop("graphXField", "score"),
            prop("graphYField", "amount"),
            prop("graphAxisSummary", "MEAN"),
            prop("graphOverlayType", "None"),
            prop("graphLargeDatasetOption", "Sample"),
        ]));
        assert!(mean.is_ok());
    }

    #[test]
    fn histogram_amount_centres_match_the_oracle_text() {
        let span = 12.123456789012344 - (-7.75);
        assert_eq!(nice_ceil(span), Some(20.0));
        let width = 20.0 / 12.0;
        let mut edge = -7.75;
        let mut centres = Vec::new();
        for _ in 0..12 {
            let next = edge + width;
            centres.push(java_double((edge + next) / 2.0));
            edge = next;
        }
        for expected in [
            "-6.916666666666666",
            "-5.25",
            "-3.5833333333333326",
            "-1.9166666666666656",
            "-0.249999999999999",
            "1.4166666666666676",
            "3.0833333333333344",
            "4.750000000000002",
            "6.416666666666668",
            "8.083333333333336",
            "11.416666666666668",
        ] {
            assert!(
                centres.iter().any(|centre| centre == expected),
                "{expected} not in {centres:?}"
            );
        }
    }

    #[test]
    fn blank_category_box_matches_boxplot_region_amount() {
        let mut values = vec![-0.5, 4.25];
        let stats = box_stats(&mut values).unwrap();
        assert_eq!(java_double(stats.median), "1.875");
        assert_eq!(java_double(stats.q1), "-0.5");
        assert_eq!(java_double(stats.q3), "4.25");
        assert_eq!(java_double(stats.upper), "4.25");
        assert_eq!(java_double(stats.lower), "-0.5");
        assert!(stats.outliers.is_empty());
    }

    #[test]
    fn line_mean_skips_missing_y_and_keeps_first_appearance() {
        let input = strings(
            &[("score", Storage::Real), ("amount", Storage::Real)],
            vec![
                vec![Value::Null, Value::real(4.0)],
                vec![Value::real(1.0), Value::real(2.0)],
                vec![Value::real(1.0), Value::Null],
                vec![Value::real(0.5), Value::real(1.0)],
            ],
        );
        let document = run(
            vec![
                prop("graphType", "LINE"),
                prop("graphXField", "score"),
                prop("graphYField", "amount"),
            ],
            input,
        )
        .unwrap();
        let table = &document.tables[0];
        assert_eq!(table.header, vec!["score", "amount_mean"]);
        assert_eq!(
            table.rows,
            vec![
                vec!["".to_string(), "4.0".to_string()],
                vec!["1.0".to_string(), "2.0".to_string()],
                vec!["0.5".to_string(), "1.0".to_string()],
            ]
        );
    }

    #[test]
    fn pie_quotes_a_blank_category_and_sums_in_first_appearance_order() {
        let input = strings(
            &[("region", Storage::String), ("amount", Storage::Real)],
            vec![
                vec![Value::string(""), Value::real(1.5)],
                vec![Value::string("South"), Value::real(2.0)],
                vec![Value::string(""), Value::real(2.25)],
                vec![Value::string("South"), Value::Null],
            ],
        );
        let document = run(
            vec![
                prop("graphType", "PIE"),
                prop("graphCategoriesField", "region"),
                prop("graphValuesField", "amount"),
                prop("graphAxisSummary", "MEAN"),
            ],
            input,
        )
        .unwrap();
        let table = &document.tables[0];
        assert_eq!(table.header, vec!["region", "amount_sum"]);
        assert_eq!(
            table.rows,
            vec![
                vec!["\"\"".to_string(), "3.75".to_string()],
                vec!["South".to_string(), "2.0".to_string()],
            ]
        );
    }
}
