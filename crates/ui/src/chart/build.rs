//! Interprets only documented executor table layouts. Unknown layouts return None.
use super::{Chart, ChartKind, Label, MAX_CATEGORIES, MAX_POINTS, MAX_SERIES, Series};
use dms_exec::{Document, DocumentTable, settings};
use dms_strfmt::fastinfoset::{Element, Node};
use std::collections::BTreeMap;

type Settings<'a> = Option<&'a Element>;

const MAX_LABEL_BYTES: usize = 16 * 1024;

/// Bounds both the settings readers' text allocation and the UI cache's recursive Clone/Eq.
/// Settings outside these display limits remain available in the original data view.
pub(crate) fn settings_are_bounded(s: Settings<'_>) -> bool {
    let Some(s) = s else {
        return true;
    };
    let mut pending = vec![(s, 0_usize)];
    let mut items = 0_usize;
    let mut bytes = 0_usize;
    while let Some((element, depth)) = pending.pop() {
        items = items
            .saturating_add(element.children.len())
            .saturating_add(element.attributes.len())
            .saturating_add(element.namespace_declarations.len());
        if depth > 64 || items > 16_384 {
            return false;
        }
        let mut text_bytes = 0_usize;
        for child in &element.children {
            match child {
                Node::Element(child) => pending.push((child, depth.saturating_add(1))),
                Node::Text(text) => {
                    text_bytes = text_bytes.saturating_add(text.len());
                    bytes = bytes.saturating_add(text.len());
                    if text_bytes > MAX_LABEL_BYTES || bytes > 1024 * 1024 {
                        return false;
                    }
                }
                // These information items have no graph-setting interpretation here.
                _ => return false,
            }
        }
    }
    true
}

fn field(s: Settings<'_>, key: &str) -> Option<String> {
    settings::text(s, key).filter(|s| !s.trim().is_empty())
}

fn flag(s: Settings<'_>, key: &str, default: bool) -> Option<bool> {
    Some(settings::flag(s, key).ok()?.unwrap_or(default))
}

fn number(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

fn present(text: &str) -> bool {
    !text.trim().is_empty() && !matches!(text.trim(), "$null$" | "NaN" | "Infinity" | "-Infinity")
}

fn cell(row: &[String], at: usize) -> Option<f64> {
    number(row.get(at)?)
}

fn column(table: &DocumentTable, name: &str) -> Option<usize> {
    table.header.iter().position(|h| h == name)
}

fn field_column(
    table: &DocumentTable,
    s: Settings<'_>,
    key: &str,
    fallback: usize,
) -> Option<usize> {
    match field(s, key) {
        Some(name) => column(table, &name),
        None => table.header.get(fallback).map(|_| fallback),
    }
}

// Preserve a missing requested field as an invalid layout, rather than dropping the overlay.
fn optional_column(table: &DocumentTable, s: Settings<'_>, key: &str) -> Option<Option<usize>> {
    match field(s, key) {
        Some(name) => column(table, &name).map(Some),
        None => Some(None),
    }
}

/// Two passes bound allocation independently of input length. Includes both endpoints.
fn sample(points: impl Iterator<Item = [f64; 2]> + Clone, name: Label) -> Series {
    let total = points.clone().count();
    let limit = total.min(MAX_POINTS);
    let mut out = Vec::with_capacity(limit);
    for (index, point) in points.enumerate() {
        let target = if total <= limit {
            out.len()
        } else {
            let numerator = (out.len() as u128).checked_mul(total.saturating_sub(1) as u128);
            numerator
                .and_then(|n| n.checked_div(limit.saturating_sub(1) as u128))
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(usize::MAX)
        };
        if out.len() < limit && index == target {
            out.push(point);
        }
    }
    Series {
        name,
        points: out,
        total_points: total,
        baseline: false,
    }
}

fn series(table: &DocumentTable, x: usize, y: usize, name: Label) -> Series {
    sample(
        table
            .rows
            .iter()
            .filter_map(|r| Some([cell(r, x)?, cell(r, y)?])),
        name,
    )
}

fn chart(
    kind: ChartKind,
    _document: &Document,
    s: Settings<'_>,
    x: Label,
    y: Label,
    series: Vec<Series>,
) -> Option<Chart> {
    if series.is_empty() || series.len() > MAX_SERIES || series.iter().all(|s| s.points.is_empty())
    {
        return None;
    }
    let axis = |key, auto, label| -> Option<Label> {
        if flag(s, auto, true)? {
            Some(label)
        } else {
            Some(Label::Text(field(s, key).unwrap_or_default()))
        }
    };
    Some(Chart {
        kind,
        title: field(s, "graphTitle").unwrap_or_default(),
        node_type: String::new(),
        title_field: x.clone(),
        stacked: false,
        x_label: axis("graphXLabel", "graphAutomaticXLabel", x)?,
        y_label: axis("graphYLabel", "graphAutomaticYLabel", y)?,
        series,
        categories: Vec::new(),
    })
}

/// `node_type` is a catalog id. Unsupported nodes/settings and unrecognised tables stay data.
pub fn chart_for(node_type: &str, s: Settings<'_>, document: &Document) -> Option<Chart> {
    if !matches!(
        node_type,
        "histogram"
            | "distribution"
            | "plot"
            | "multiplot"
            | "time_plot"
            | "evaluation"
            | "collection"
    ) {
        return None;
    }
    if document.tables.len() != 1
        || document.title.len() > MAX_LABEL_BYTES
        || !settings_are_bounded(s)
    {
        return None;
    }
    let table = document.tables.first()?;
    if table.rows.is_empty() || table.header.len() < 2 || table.header.len() > MAX_SERIES + 4 {
        return None;
    }
    // A malformed table must not turn into a plausible, but different, graph.
    if table
        .header
        .iter()
        .any(|h| h.trim().is_empty() || h.len() > MAX_LABEL_BYTES)
        || table
            .rows
            .iter()
            .any(|r| r.len() != table.header.len() || r.iter().any(|c| c.len() > MAX_LABEL_BYTES))
    {
        return None;
    }
    let mut result = match node_type {
        "histogram" => histogram(s, document, table),
        "distribution" => distribution(s, document, table),
        "collection" => collection(s, document, table),
        "plot" | "multiplot" | "time_plot" => lines(node_type, s, document, table),
        "evaluation" => evaluation(s, document, table),
        _ => None,
    }?;
    result.node_type = node_type.to_string();
    result.stacked = matches!(node_type, "histogram" | "distribution")
        && field(s, "graphColorField").is_some()
        && flag(s, "graphNormalizeData", false)?;
    if node_type == "evaluation" {
        result.title_field = result.y_label.clone();
    } else if node_type == "time_plot" {
        result.title_field = if result.series.len() == 1 {
            result.series.first()?.name.clone()
        } else {
            Label::Value
        };
    }
    Some(result)
}

fn numeric_width(series: &[Series], s: Settings<'_>) -> Option<f64> {
    if flag(s, "graphBinByWidth", false)? {
        let width = settings::real(s, "graphBinWidth").ok()?.unwrap_or(1.0);
        return (width > 0.0).then_some(width);
    }
    let mut xs: Vec<f64> = series
        .iter()
        .flat_map(|s| s.points.iter().map(|&[x, _]| x))
        .collect();
    xs.sort_by(f64::total_cmp);
    let width = xs
        .windows(2)
        .filter_map(|pair| {
            let delta = pair.get(1)? - pair.first()?;
            (delta.is_finite() && delta > 0.0).then_some(delta)
        })
        .min_by(f64::total_cmp)
        .unwrap_or(1.0);
    Some(width)
}

fn histogram(s: Settings<'_>, doc: &Document, table: &DocumentTable) -> Option<Chart> {
    if flag(s, "histogramSeparateBands", false)? {
        return None;
    }
    let x = field_column(table, s, "graphXField", 0)?;
    let y = column(table, "__count__").or_else(|| column(table, "__normalized__"))?;
    let overlay = field(s, "graphColorField");
    let expected = if overlay.is_some() { 3 } else { 2 };
    if table.header.len() != expected {
        return None;
    }
    let mut series_out = Vec::new();
    if let Some(overlay) = overlay {
        let at = column(table, &overlay)?;
        let mut groups = BTreeMap::new();
        for row in &table.rows {
            let value = row.get(at)?;
            if present(value) && cell(row, x).is_some() && cell(row, y).is_some() {
                if groups.len() >= MAX_SERIES && !groups.contains_key(value) {
                    return None;
                }
                let index = groups.len();
                groups.entry(value.clone()).or_insert(index);
            }
        }
        let mut ordered: Vec<_> = groups.into_iter().collect();
        ordered.sort_by_key(|(_, index)| *index);
        for (name, _) in ordered {
            series_out.push(sample(
                table
                    .rows
                    .iter()
                    .filter_map(|r| (r.get(at)? == &name).then_some([cell(r, x)?, cell(r, y)?])),
                Label::Text(name.clone()),
            ));
        }
    } else {
        series_out.push(series(table, x, y, Label::Count));
    }
    let width = numeric_width(&series_out, s)?;
    let normalized = table.header.get(y)? == "__normalized__";
    chart(
        ChartKind::Bars { width },
        doc,
        s,
        Label::Text(table.header.get(x)?.clone()),
        if normalized {
            Label::Percent
        } else {
            Label::Count
        },
        series_out,
    )
}

/// First 499 categories remain distinct; overflow, including category 500, becomes Other.
/// The map never grows beyond 500 keys, even if every row has a different category.
#[derive(Default)]
struct Categories {
    names: Vec<Label>,
    indices: BTreeMap<String, usize>,
}

impl Categories {
    fn index(&mut self, name: &str) -> usize {
        if let Some(&index) = self.indices.get(name) {
            return index;
        }
        if self.names.len() < MAX_CATEGORIES {
            let index = self.names.len();
            self.names.push(Label::Text(name.to_string()));
            self.indices.insert(name.to_string(), index);
            return index;
        }
        if let Some(last) = self.names.last_mut() {
            *last = Label::Other;
        }
        MAX_CATEGORIES.saturating_sub(1)
    }
}

fn categorical(
    s: Settings<'_>,
    doc: &Document,
    table: &DocumentTable,
    x: usize,
    y: usize,
    overlay: Option<usize>,
    y_label: Label,
) -> Option<Chart> {
    let mut categories = Categories::default();
    let mut groups: BTreeMap<String, usize> = BTreeMap::new();
    // Percentages within categories cannot be added when categories are merged. Retain the
    // documented tooltip's original counts so Other can be normalised over its own records.
    let normalized = y_label == Label::Percent;
    let tooltip = column(table, "__tooltip__");
    let mut values: Vec<BTreeMap<usize, (f64, Option<f64>)>> = Vec::new();
    for row in &table.rows {
        let category = row.get(x)?;
        let Some(value) = cell(row, y) else {
            continue;
        };
        if !present(category) {
            continue;
        }
        let name = match overlay {
            Some(at) => row.get(at)?.as_str(),
            None => "",
        };
        if overlay.is_some() && !present(name) {
            continue;
        }
        let group = match groups.get(name) {
            Some(&index) => index,
            None => {
                if groups.len() >= MAX_SERIES {
                    return None;
                }
                let index = groups.len();
                groups.insert(name.to_string(), index);
                values.push(BTreeMap::new());
                index
            }
        };
        let index = categories.index(category);
        let raw_count = tooltip
            .and_then(|at| row.get(at))
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("Count = ").and_then(number))
            })
            .filter(|n| *n >= 0.0);
        let sum = values
            .get_mut(group)?
            .entry(index)
            .or_insert((0.0, Some(0.0)));
        sum.0 += value;
        sum.1 = sum
            .1
            .zip(raw_count)
            .map(|(a, b)| a + b)
            .filter(|n| n.is_finite());
        if !sum.0.is_finite() {
            return None;
        }
    }
    if normalized && categories.names.last() == Some(&Label::Other) {
        let last = MAX_CATEGORIES.saturating_sub(1);
        let mut total = 0.0;
        for cells in &values {
            if let Some((_, count)) = cells.get(&last) {
                total += (*count)?;
            }
        }
        if !total.is_finite() || total <= 0.0 {
            return None;
        }
        for cells in &mut values {
            if let Some((value, count)) = cells.get_mut(&last) {
                *value = (*count)? / total * 100.0;
            }
        }
    }
    let mut ordered: Vec<_> = groups.into_iter().collect();
    ordered.sort_by_key(|(_, index)| *index);
    let series_out = ordered
        .into_iter()
        .filter_map(|(name, at)| {
            let points = values
                .get(at)?
                .iter()
                .map(|(&x, &(y, _))| [x as f64, y])
                .collect::<Vec<_>>();
            Some(Series {
                name: if overlay.is_some() {
                    Label::Text(name)
                } else {
                    y_label.clone()
                },
                total_points: points.len(),
                points,
                baseline: false,
            })
        })
        .collect();
    let mut chart = chart(
        ChartKind::Bars { width: 0.8 },
        doc,
        s,
        Label::Text(table.header.get(x)?.clone()),
        y_label,
        series_out,
    )?;
    chart.categories = categories.names;
    Some(chart)
}

fn distribution(s: Settings<'_>, doc: &Document, table: &DocumentTable) -> Option<Chart> {
    let x = field_column(table, s, "graphXField", 0)?;
    let y = column(table, "__count__")?;
    let overlay = optional_column(table, s, "graphColorField")?;
    let expected =
        2 + usize::from(overlay.is_some()) + usize::from(column(table, "__tooltip__").is_some());
    if table.header.len() != expected {
        return None;
    }
    categorical(
        s,
        doc,
        table,
        x,
        y,
        overlay,
        if flag(s, "graphNormalizeData", false)? {
            Label::Percent
        } else {
            Label::Count
        },
    )
}

fn collection(s: Settings<'_>, doc: &Document, table: &DocumentTable) -> Option<Chart> {
    if table.header.len() != 2 {
        return None;
    }
    let x = field_column(table, s, "graphXField", 0)?;
    let y = field_column(table, s, "graphYField", 1)?;
    let data = series(table, x, y, Label::Text(table.header.get(y)?.clone()));
    // Collection's executor currently supplies numeric bin centres. Keep their true spacing;
    // very many bins use the same categorical Other policy as Distribution.
    if data.total_points > MAX_CATEGORIES {
        // A mean or deviation cannot be merged without the bin populations. MIN/MAX also
        // need a different reducer; leave all non-additive summaries in their original table.
        if !matches!(
            field(s, "graphOperationType").as_deref(),
            None | Some("SUM")
        ) {
            return None;
        }
        return categorical(
            s,
            doc,
            table,
            x,
            y,
            None,
            Label::Text(table.header.get(y)?.clone()),
        );
    }
    let width = numeric_width(std::slice::from_ref(&data), s)?;
    chart(
        ChartKind::Bars { width },
        doc,
        s,
        Label::Text(table.header.get(x)?.clone()),
        Label::Text(table.header.get(y)?.clone()),
        vec![data],
    )
}

fn lines(node_type: &str, s: Settings<'_>, doc: &Document, table: &DocumentTable) -> Option<Chart> {
    // One chart cannot faithfully represent independent panels or non-colour aesthetics.
    if field(s, "graphPanelField").is_some()
        || field(s, "graphZField").is_some()
        || field(s, "graphAnimationField").is_some()
    {
        return None;
    }
    let x = if node_type == "time_plot" {
        column(table, "@INDEX")?
    } else {
        field_column(table, s, "graphXField", 0)?
    };
    let mut ys = Vec::new();
    let kind = if node_type == "plot" {
        for key in [
            "graphSizeField",
            "graphShapeField",
            "graphTransparencyField",
        ] {
            if field(s, key).is_some() {
                return None;
            }
        }
        ys.push(field_column(table, s, "graphYField", 1)?);
        if flag(s, "scatterShowLine", false)? {
            ChartKind::Lines
        } else {
            ChartKind::Scatter
        }
    } else {
        let key = if node_type == "time_plot" {
            "timePlotSeriesFields"
        } else {
            "graphYMultiFields"
        };
        if let Some(parent) = settings::element(s, key) {
            for child in parent
                .child_elements()
                .filter(|c| &*c.name.local == "field")
            {
                if ys.len() >= MAX_SERIES {
                    return None;
                }
                ys.push(column(table, child.text().trim())?);
            }
        } else {
            ys.extend((0..table.header.len()).filter(|&at| at != x));
        }
        if node_type == "multiplot" && flag(s, "graphUseOverlayExpression", false)? {
            return None;
        }
        if node_type == "time_plot"
            && (flag(s, "timePlotUseCustomXField", false)?
                || flag(s, "timePlotPanel", false)?
                || field(s, "graphColorField").is_some())
        {
            return None;
        }
        ChartKind::Lines
    };
    let overlay = if node_type == "plot" {
        optional_column(table, s, "graphColorField")?
    } else {
        None
    };
    if ys.is_empty()
        || ys.len() > MAX_SERIES
        || table.header.len() != 1 + ys.len() + usize::from(overlay.is_some())
    {
        return None;
    }
    let mut out = Vec::new();
    for y in &ys {
        if let Some(at) = overlay {
            let mut names = BTreeMap::new();
            for row in &table.rows {
                let name = row.get(at)?;
                if present(name) && cell(row, x).is_some() && cell(row, *y).is_some() {
                    if names.len() >= MAX_SERIES && !names.contains_key(name) {
                        return None;
                    }
                    let index = names.len();
                    names.entry(name.clone()).or_insert(index);
                }
            }
            let mut names: Vec<_> = names.into_iter().collect();
            names.sort_by_key(|(_, index)| *index);
            for (name, _) in names {
                out.push(sample(
                    table.rows.iter().filter_map(|r| {
                        (r.get(at)? == &name).then_some([cell(r, x)?, cell(r, *y)?])
                    }),
                    Label::Text(name.clone()),
                ));
            }
        } else {
            out.push(series(
                table,
                x,
                *y,
                Label::Text(table.header.get(*y)?.clone()),
            ));
        }
    }
    if node_type == "plot" && kind == ChartKind::Lines {
        match field(s, "scatterXMode").as_deref().unwrap_or("Sort") {
            "Sort" => {
                for s in &mut out {
                    s.points.sort_by(|a, b| {
                        a.first()
                            .partial_cmp(&b.first())
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
            }
            "AsRead" => {}
            _ => return None,
        }
    }
    let normalized =
        node_type != "plot" && flag(s, "graphNormalizeData", node_type == "time_plot")?;
    let y_label = if ys.len() == 1 {
        Label::Text(table.header.get(*ys.first()?)?.clone())
    } else if normalized {
        Label::Normalized
    } else {
        Label::Value
    };
    chart(
        kind,
        doc,
        s,
        if node_type == "time_plot" {
            Label::Record
        } else {
            Label::Text(table.header.get(x)?.clone())
        },
        y_label,
        out,
    )
}

fn evaluation(s: Settings<'_>, doc: &Document, table: &DocumentTable) -> Option<Chart> {
    if table.header.first()? != "nTileColumnName" || table.header.len() > MAX_SERIES {
        return None;
    }
    let tiles: f64 = match field(s, "plotNTiles").as_deref().unwrap_or("percentiles") {
        "percentiles" => 100.0,
        "quartiles" => 4.0,
        "quintiles" => 5.0,
        "deciles" => 10.0,
        "vingtiles" => 20.0,
        "1000-tiles" => 1000.0,
        _ => return None,
    };
    let cumulative = flag(s, "cumulativePlot", true)?;
    let kind = field(s, "chartType").unwrap_or_else(|| "gains".to_string());
    let y_label = match kind.as_str() {
        "gains" => Label::Gains,
        "lift" => Label::Lift,
        "response" => Label::Response,
        "profit" if cumulative => Label::Profit,
        "roi" if cumulative => Label::Roi,
        _ => return None,
    };
    let mut out: Vec<_> = table
        .header
        .iter()
        .enumerate()
        .skip(1)
        .map(|(y, name)| series(table, 0, y, Label::Text(name.clone())))
        .collect();
    if out.iter().all(|series| series.points.is_empty()) {
        return None;
    }
    // Response and monetary baselines require the population rate/total. It is available only
    // in a complete cumulative curve; non-cumulative response cannot be recovered from rates.
    let endpoint = if matches!(kind.as_str(), "response" | "profit" | "roi") {
        if !cumulative {
            return None;
        }
        let last = out.first()?.points.iter().find(|&&[x, _]| x == tiles)?;
        let &[_, value] = last;
        for series in &out {
            let &[_, other] = series.points.iter().find(|&&[x, _]| x == tiles)?;
            if (value - other).abs() > 1e-9 * value.abs().max(1.0) {
                return None;
            }
        }
        value
    } else {
        0.0
    };
    let points = match kind.as_str() {
        "gains" if cumulative => vec![[0.0, 0.0], [tiles, 100.0]],
        "gains" => vec![[1.0, 100.0 / tiles], [tiles, 100.0 / tiles]],
        "lift" => vec![[1.0, 1.0], [tiles, 1.0]],
        "profit" => vec![[0.0, 0.0], [tiles, endpoint]],
        _ => vec![[1.0, endpoint], [tiles, endpoint]],
    };
    out.push(Series {
        name: Label::Baseline,
        total_points: points.len(),
        points,
        baseline: true,
    });
    chart(ChartKind::Lines, doc, s, Label::NTile, y_label, out)
}
