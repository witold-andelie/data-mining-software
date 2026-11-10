//! Bounded, plot-library-independent graph data and its egui_plot presentation.
//!
//! The table contracts and setting defaults come from the executor's graph node modules and
//! docs/formats/nodes/{histogram,distribution,plot}.md. No data is recomputed from raw records.

mod build;

pub use build::chart_for;
pub(crate) use build::settings_are_bounded;

use crate::i18n::{tr, tr_fmt};
use egui::{Color32, Ui};
use egui_plot::{Bar, BarChart, Legend, Line, LineStyle, Plot, Points};

pub const MAX_POINTS: usize = 100_000;
pub const MAX_CATEGORIES: usize = 500;
/// Excessively wide graphs fall back to their data table.
pub const MAX_SERIES: usize = 64;

/// Generated labels are translated at draw time; field names and saved titles stay verbatim.
/// Keeping these distinct also keeps chart_for independent of the current UI language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Label {
    Text(String),
    Count,
    Percent,
    Normalized,
    Value,
    Record,
    NTile,
    Gains,
    Lift,
    Response,
    Profit,
    Roi,
    Baseline,
    Other,
}

impl Label {
    pub fn display(&self) -> &str {
        match self {
            Self::Text(text) => text,
            Self::Count => tr("Count"),
            Self::Percent => tr("Percent"),
            Self::Normalized => tr("Normalized value"),
            Self::Value => tr("Value"),
            Self::Record => tr("Record"),
            Self::NTile => tr("N-tile"),
            Self::Gains => tr("Gains (%)"),
            Self::Lift => tr("Lift"),
            Self::Response => tr("Response (%)"),
            Self::Profit => tr("Profit"),
            Self::Roi => tr("ROI (%)"),
            Self::Baseline => tr("Baseline"),
            Self::Other => tr("Other"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChartKind {
    /// Width of a group; individual series are drawn side by side within it.
    Bars {
        width: f64,
    },
    Scatter,
    Lines,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub name: Label,
    pub points: Vec<[f64; 2]>,
    /// Valid points before uniform sampling (not including null / non-finite cells).
    pub total_points: usize,
    pub baseline: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    pub kind: ChartKind,
    pub title: String,
    pub x_label: Label,
    pub y_label: Label,
    pub series: Vec<Series>,
    /// For categorical bars, x is the index here; numeric axes leave this empty.
    pub categories: Vec<Label>,
}

impl Chart {
    /// Lookup only after validating the floating point coordinate, including before its cast.
    pub fn category_at(&self, x: f64) -> Option<&Label> {
        if !x.is_finite() || x < 0.0 || x >= self.categories.len() as f64 {
            return None;
        }
        let rounded = x.round();
        if (x - rounded).abs() > 1e-6 {
            return None;
        }
        self.categories.get(rounded as usize)
    }
}

/// Theme colours from our node palette, with separate variants for light and dark backgrounds.
fn series_color(index: usize, dark: bool) -> Color32 {
    const COLORS: [(u32, u32); 8] = [
        (0x477BB3, 0x79A8DC),
        (0xC2465A, 0xE06F82),
        (0x4F8A3A, 0x76B05E),
        (0x8E63AC, 0xB18ACD),
        (0x8A6233, 0xD0A46D),
        (0x267D83, 0x5FB8BD),
        (0xA65B27, 0xE39455),
        (0x55606E, 0xA6B2C1),
    ];
    let (light, dark_variant) = COLORS
        .get(index % COLORS.len())
        .copied()
        .unwrap_or_default();
    let hex = if dark { dark_variant } else { light };
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Draws a chart without a window or GPU requirement; hover rulers show the original values.
pub fn show_chart(ui: &mut Ui, chart: &Chart, id: impl egui::AsIdSalt) {
    if !chart.title.is_empty() {
        ui.heading(&chart.title);
    }
    let sampled = chart.series.iter().any(|s| s.points.len() < s.total_points);
    let height = (ui.available_height() - if sampled { 24.0 } else { 0.0 }).max(80.0);
    let mut plot = Plot::new(ui.scope_id().with(id))
        .height(height)
        .x_axis_label(chart.x_label.display())
        .y_axis_label(chart.y_label.display())
        .allow_scroll(false);
    if chart.series.len() > 1 {
        plot = plot.legend(Legend::default());
    }
    if !chart.categories.is_empty() {
        plot = plot.x_axis_formatter(|mark, _| {
            chart
                .category_at(mark.value)
                .map_or_else(String::new, |l| l.display().to_string())
        });
    }
    let dark = ui.visuals().dark_mode;
    plot.show(ui, |plot_ui| {
        for (index, series) in chart.series.iter().enumerate() {
            let name = series.name.display();
            let color = series_color(index, dark);
            match chart.kind {
                ChartKind::Bars { width } => {
                    let slot = width / chart.series.len().max(1) as f64;
                    let offset = (index as f64 + 0.5) * slot - width / 2.0;
                    let bars = series
                        .points
                        .iter()
                        .map(|&[x, y]| {
                            let category = chart
                                .category_at(x)
                                .map_or_else(|| x.to_string(), |l| l.display().to_string());
                            Bar::new(x + offset, y).width(slot * 0.9).name(category)
                        })
                        .collect();
                    let y_label = chart.y_label.display().to_string();
                    let series_name = name.to_string();
                    plot_ui.bar_chart(
                        BarChart::new(name, bars)
                            .id(index)
                            .color(color)
                            .element_formatter(Box::new(move |bar, _| {
                                format!("{series_name}\n{}\n{y_label}: {}", bar.name, bar.value)
                            })),
                    );
                }
                ChartKind::Scatter => {
                    plot_ui.points(
                        Points::new(name, series.points.clone())
                            .id(index)
                            .color(color)
                            .radius(2.5),
                    );
                }
                ChartKind::Lines => {
                    let line = Line::new(name, series.points.clone())
                        .id(index)
                        .color(color);
                    plot_ui.line(if series.baseline {
                        line.style(LineStyle::dashed_dense())
                    } else {
                        line
                    });
                }
            }
        }
    });
    for series in &chart.series {
        if series.points.len() < series.total_points {
            ui.weak(tr_fmt(
                "{series}: showing {shown} / {total} points",
                &[
                    ("series", series.name.display()),
                    ("shown", &series.points.len().to_string()),
                    ("total", &series.total_points.to_string()),
                ],
            ));
        }
    }
}
