//! Node pictograms: our own vector drawings, one per frequently used node type (by use in the
//! team's streams and the corpus), and the palette glyph of `theme` for the rest. Drawn in a unit
//! square (x and y from -1 to 1, y down) scaled to the tile, so they stay sharp at any zoom and on
//! Retina screens. No IBM artwork is used or imitated (AGENTS.md, clean room).

use crate::graph::NodeKind;
use crate::theme;
use egui::{Color32, Painter, Pos2, Shape, Stroke, pos2, vec2};

/// Draws the pictogram for `type_id` (a catalog id), or the palette glyph when there is none.
pub fn draw(
    painter: &Painter,
    type_id: &str,
    kind: NodeKind,
    center: Pos2,
    size: f32,
    color: Color32,
) {
    let pen = Pen {
        painter,
        center,
        r: size / 2.0,
        color,
        width: (size / 11.0).max(1.0),
    };
    if !draw_specific(&pen, type_id) {
        theme::glyph(painter, kind, center, size, color);
    }
}

/// Whether `type_id` has a pictogram of its own (otherwise the palette glyph is used).
pub fn has_pictogram(type_id: &str) -> bool {
    PICTOGRAMS.contains(&type_id)
}

/// Node types with their own pictogram.
pub const PICTOGRAMS: &[&str] = &[
    // Sources
    "var_file",
    "fixed_file",
    "statistics_file",
    "excel_import",
    "database_import",
    "user_input",
    "xml_import",
    "json_import",
    // Record operations
    "select",
    "sample",
    "balance",
    "aggregate",
    "sort",
    "merge",
    "append",
    "distinct",
    // Field operations
    "type",
    "filter",
    "derive",
    "filler",
    "reclassify",
    "binning",
    "partition",
    "auto_data_prep",
    "time_intervals",
    "restructure",
    // Graphs
    "plot",
    "histogram",
    "distribution",
    "graphboard",
    "evaluation",
    "time_plot",
    // Modeling
    "c50",
    "cart",
    "quest",
    "chaid",
    "random_trees",
    "decision_list",
    "logistic",
    "regression",
    "linear",
    "linear_as",
    "gle",
    "genlin",
    "neural_net",
    "kmeans",
    "twostep",
    "kohonen",
    "auto_cluster",
    "time_series",
    "apriori",
    "carma",
    "sequence",
    "feature_selection",
    "auto_classifier",
    "auto_numeric",
    "anomaly",
    "pca_factor",
    // Output
    "table",
    "analysis",
    "data_audit",
    "statistics",
    "matrix",
    "means",
    // Export
    "flat_file_export",
    "excel_export",
    "statistics_export",
    "database_export",
];

struct Pen<'a> {
    painter: &'a Painter,
    center: Pos2,
    r: f32,
    color: Color32,
    width: f32,
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.center.x + x * self.r, self.center.y + y * self.r)
    }
    fn stroke(&self) -> Stroke {
        Stroke::new(self.width, self.color)
    }
    fn thin(&self) -> Stroke {
        Stroke::new((self.width * 0.6).max(0.8), self.color)
    }
    fn line(&self, points: &[(f32, f32)]) {
        let points: Vec<Pos2> = points.iter().map(|&(x, y)| self.at(x, y)).collect();
        self.painter.add(Shape::line(points, self.stroke()));
    }
    fn thin_line(&self, points: &[(f32, f32)]) {
        let points: Vec<Pos2> = points.iter().map(|&(x, y)| self.at(x, y)).collect();
        self.painter.add(Shape::line(points, self.thin()));
    }
    fn closed(&self, points: &[(f32, f32)]) {
        let points: Vec<Pos2> = points.iter().map(|&(x, y)| self.at(x, y)).collect();
        self.painter.add(Shape::closed_line(points, self.stroke()));
    }
    fn fill(&self, points: &[(f32, f32)]) {
        let points: Vec<Pos2> = points.iter().map(|&(x, y)| self.at(x, y)).collect();
        self.painter
            .add(Shape::convex_polygon(points, self.color, Stroke::NONE));
    }
    fn dot(&self, x: f32, y: f32, radius: f32) {
        self.painter
            .circle_filled(self.at(x, y), radius * self.r, self.color);
    }
    fn ring(&self, x: f32, y: f32, radius: f32) {
        self.painter
            .circle_stroke(self.at(x, y), radius * self.r, self.thin());
    }
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.closed(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
    }
    fn bar(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.fill(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
    }
    /// A polyline through `f(t)` for t in 0..=1.
    fn curve(&self, f: impl Fn(f32) -> (f32, f32)) {
        let points: Vec<(f32, f32)> = (0..=24).map(|i| f(i as f32 / 24.0)).collect();
        self.line(&points);
    }
    /// A sheet of paper with a folded corner, the frame of file nodes.
    fn page(&self) {
        self.closed(&[
            (-0.6, -0.85),
            (0.3, -0.85),
            (0.65, -0.5),
            (0.65, 0.85),
            (-0.6, 0.85),
        ]);
        self.thin_line(&[(0.3, -0.85), (0.3, -0.5), (0.65, -0.5)]);
    }
    /// An arrow from `from` to `to` with a filled head.
    fn arrow(&self, from: (f32, f32), to: (f32, f32)) {
        self.line(&[from, to]);
        let direction = vec2(to.0 - from.0, to.1 - from.1).normalized();
        let side = direction.rot90() * 0.22;
        let back = direction * 0.32;
        self.fill(&[
            to,
            (to.0 - back.x + side.x, to.1 - back.y + side.y),
            (to.0 - back.x - side.x, to.1 - back.y - side.y),
        ]);
    }
    fn tree(&self, children: usize) {
        let xs: Vec<f32> = match children {
            3 => vec![-0.65, 0.0, 0.65],
            _ => vec![-0.55, 0.55],
        };
        for &x in &xs {
            self.line(&[(0.0, -0.55), (x, 0.55)]);
        }
        self.dot(0.0, -0.55, 0.2);
        for &x in &xs {
            self.dot(x, 0.55, 0.17);
        }
    }
}

fn draw_specific(pen: &Pen<'_>, type_id: &str) -> bool {
    match type_id {
        // --- Sources: a page, or a container, with what it holds.
        "var_file" => {
            pen.page();
            for y in [-0.15, 0.2, 0.55] {
                pen.thin_line(&[(-0.35, y), (-0.05, y)]);
                pen.thin_line(&[(0.1, y), (0.4, y)]);
            }
        }
        "fixed_file" => {
            pen.page();
            for y in [-0.15, 0.2, 0.55] {
                pen.thin_line(&[(-0.35, y), (0.4, y)]);
            }
            pen.thin_line(&[(0.05, -0.3), (0.05, 0.7)]);
        }
        "statistics_file" => {
            pen.page();
            pen.bar(-0.35, 0.25, -0.15, 0.6);
            pen.bar(-0.05, -0.1, 0.15, 0.6);
            pen.bar(0.25, 0.1, 0.45, 0.6);
        }
        "excel_import" | "excel_export" => {
            pen.page();
            pen.rect(-0.35, -0.2, 0.4, 0.6);
            pen.thin_line(&[(-0.35, 0.2), (0.4, 0.2)]);
            pen.thin_line(&[(0.02, -0.2), (0.02, 0.6)]);
            if type_id == "excel_export" {
                pen.arrow((0.2, 0.0), (0.95, 0.0));
            }
        }
        "database_import" | "database_export" => {
            pen.painter.add(Shape::ellipse_stroke(
                pen.at(0.0, -0.55),
                vec2(pen.r * 0.75, pen.r * 0.28),
                pen.stroke(),
            ));
            pen.line(&[(-0.75, -0.55), (-0.75, 0.55)]);
            pen.line(&[(0.75, -0.55), (0.75, 0.55)]);
            pen.curve(|t| {
                let a = std::f32::consts::PI * t;
                (-0.75 * a.cos(), 0.55 + 0.28 * a.sin())
            });
            pen.curve(|t| {
                let a = std::f32::consts::PI * t;
                (-0.75 * a.cos(), 0.0 + 0.28 * a.sin())
            });
            if type_id == "database_export" {
                pen.arrow((0.3, 0.9), (0.95, 0.9));
            }
        }
        "user_input" => {
            pen.rect(-0.8, -0.6, 0.8, 0.6);
            pen.thin_line(&[(-0.55, -0.2), (0.3, -0.2)]);
            pen.thin_line(&[(-0.55, 0.2), (-0.05, 0.2)]);
            pen.line(&[(0.1, 0.05), (0.1, 0.35)]);
        }
        "xml_import" => {
            pen.line(&[(-0.35, -0.5), (-0.8, 0.0), (-0.35, 0.5)]);
            pen.line(&[(0.35, -0.5), (0.8, 0.0), (0.35, 0.5)]);
            pen.line(&[(0.15, -0.65), (-0.15, 0.65)]);
        }
        "json_import" => {
            pen.curve(|t| {
                let y = -0.75 + 1.5 * t;
                let x = -0.35 - 0.2 * (1.0 - (y / 0.75).abs()).powi(4) - 0.15 * (y.abs() / 0.75);
                (x, y)
            });
            pen.curve(|t| {
                let y = -0.75 + 1.5 * t;
                let x = 0.35 + 0.2 * (1.0 - (y / 0.75).abs()).powi(4) + 0.15 * (y.abs() / 0.75);
                (x, y)
            });
            pen.dot(0.0, 0.0, 0.1);
        }
        // --- Record operations: rows.
        "select" => {
            pen.closed(&[
                (-0.8, -0.7),
                (0.8, -0.7),
                (0.15, 0.05),
                (0.15, 0.75),
                (-0.15, 0.75),
                (-0.15, 0.05),
            ]);
        }
        "sample" => {
            for (i, (x, y)) in [
                (-0.55, -0.55),
                (0.0, -0.55),
                (0.55, -0.55),
                (-0.55, 0.0),
                (0.0, 0.0),
                (0.55, 0.0),
                (-0.55, 0.55),
                (0.0, 0.55),
                (0.55, 0.55),
            ]
            .into_iter()
            .enumerate()
            {
                if i % 3 == 1 || i == 6 {
                    pen.dot(x, y, 0.17);
                } else {
                    pen.ring(x, y, 0.15);
                }
            }
        }
        "balance" => {
            pen.line(&[(0.0, -0.7), (0.0, 0.7)]);
            pen.line(&[(-0.45, 0.7), (0.45, 0.7)]);
            pen.line(&[(-0.75, -0.45), (0.75, -0.45)]);
            pen.line(&[(-0.75, -0.45), (-0.95, 0.1), (-0.55, 0.1), (-0.75, -0.45)]);
            pen.line(&[(0.75, -0.45), (0.55, 0.1), (0.95, 0.1), (0.75, -0.45)]);
        }
        "aggregate" => {
            pen.line(&[
                (0.55, -0.65),
                (-0.5, -0.65),
                (0.1, 0.0),
                (-0.5, 0.65),
                (0.55, 0.65),
            ]);
        }
        "sort" => {
            pen.line(&[(-0.75, -0.55), (0.15, -0.55)]);
            pen.line(&[(-0.75, 0.0), (-0.15, 0.0)]);
            pen.line(&[(-0.75, 0.55), (-0.45, 0.55)]);
            pen.arrow((0.6, -0.7), (0.6, 0.75));
        }
        "merge" => {
            pen.line(&[(-0.8, -0.6), (-0.2, 0.0)]);
            pen.line(&[(-0.8, 0.6), (-0.2, 0.0)]);
            pen.arrow((-0.2, 0.0), (0.85, 0.0));
        }
        "append" => {
            pen.rect(-0.75, -0.75, 0.35, -0.15);
            pen.rect(-0.75, 0.15, 0.35, 0.75);
            pen.line(&[(0.65, -0.2), (0.65, 0.2)]);
            pen.line(&[(0.45, 0.0), (0.85, 0.0)]);
        }
        "distinct" => {
            pen.rect(-0.8, -0.55, 0.0, 0.25);
            pen.rect(-0.35, -0.2, 0.45, 0.6);
            pen.line(&[(0.35, -0.75), (0.55, -0.55), (0.9, -0.95)]);
        }
        // --- Field operations: columns.
        "type" => {
            pen.closed(&[
                (-0.8, -0.35),
                (0.25, -0.35),
                (0.8, 0.0),
                (0.25, 0.35),
                (-0.8, 0.35),
            ]);
            pen.ring(0.25, 0.0, 0.12);
        }
        "filter" => {
            for x in [-0.55, 0.0, 0.55] {
                pen.line(&[(x, -0.7), (x, 0.7)]);
            }
            pen.line(&[(-0.3, -0.25), (0.3, 0.25)]);
            pen.line(&[(0.3, -0.25), (-0.3, 0.25)]);
        }
        "derive" => {
            pen.line(&[(-0.55, -0.7), (-0.55, 0.7)]);
            pen.line(&[(-0.15, -0.7), (-0.15, 0.7)]);
            pen.line(&[(0.5, -0.3), (0.5, 0.3)]);
            pen.line(&[(0.2, 0.0), (0.8, 0.0)]);
        }
        "filler" => {
            pen.curve(|t| {
                let a = std::f32::consts::PI * (0.15 + 1.7 * t);
                (0.45 * a.sin(), 0.25 - 0.45 * a.cos())
            });
            pen.line(&[(-0.42, 0.05), (0.0, -0.8), (0.42, 0.05)]);
        }
        "reclassify" => {
            pen.arrow((-0.75, -0.3), (0.75, -0.3));
            pen.arrow((0.75, 0.3), (-0.75, 0.3));
        }
        "binning" => {
            pen.line(&[
                (-0.8, 0.7),
                (-0.8, 0.3),
                (-0.3, 0.3),
                (-0.3, -0.1),
                (0.2, -0.1),
                (0.2, -0.5),
                (0.75, -0.5),
            ]);
        }
        "partition" => {
            pen.rect(-0.75, -0.65, 0.75, 0.65);
            pen.bar(-0.75, -0.65, 0.05, 0.65);
            pen.line(&[(0.05, -0.65), (0.05, 0.65)]);
        }
        "auto_data_prep" => {
            pen.line(&[(-0.75, 0.75), (0.3, -0.3)]);
            for (dx, dy) in [(0.0, -0.3), (0.0, 0.3), (-0.3, 0.0), (0.3, 0.0)] {
                pen.thin_line(&[(0.55, -0.55), (0.55 + dx, -0.55 + dy)]);
            }
        }
        "time_plot" => {
            pen.line(&[(-0.75, -0.8), (-0.75, 0.75), (0.8, 0.75)]);
            pen.curve(|t| (-0.6 + 1.35 * t, 0.15 - 0.35 * (9.0 * t).sin() - 0.3 * t));
        }
        "time_intervals" => {
            pen.ring(0.0, 0.0, 0.75);
            pen.line(&[(0.0, -0.5), (0.0, 0.0), (0.35, 0.25)]);
        }
        "restructure" => {
            pen.rect(-0.8, -0.3, -0.1, 0.3);
            pen.rect(0.25, -0.8, 0.55, 0.8);
            pen.arrow((-0.1, 0.0), (0.2, 0.0));
        }
        // --- Graphs: axes and marks.
        "plot" => {
            pen.line(&[(-0.75, -0.8), (-0.75, 0.75), (0.8, 0.75)]);
            for (x, y) in [
                (-0.4, 0.35),
                (-0.15, 0.05),
                (0.15, 0.2),
                (0.3, -0.25),
                (0.6, -0.5),
            ] {
                pen.dot(x, y, 0.11);
            }
        }
        "histogram" => {
            pen.bar(-0.8, 0.2, -0.4, 0.75);
            pen.bar(-0.4, -0.6, 0.0, 0.75);
            pen.bar(0.0, -0.25, 0.4, 0.75);
            pen.bar(0.4, 0.4, 0.8, 0.75);
        }
        "distribution" => {
            pen.line(&[(-0.8, -0.8), (-0.8, 0.8)]);
            pen.bar(-0.8, -0.65, 0.6, -0.3);
            pen.bar(-0.8, -0.15, 0.1, 0.2);
            pen.bar(-0.8, 0.35, 0.8, 0.7);
        }
        "graphboard" => {
            pen.line(&[(-0.75, -0.8), (-0.75, 0.75), (0.8, 0.75)]);
            pen.line(&[(-0.6, 0.4), (-0.2, -0.1), (0.15, 0.15), (0.65, -0.55)]);
        }
        "evaluation" => {
            pen.line(&[(-0.75, -0.8), (-0.75, 0.75), (0.8, 0.75)]);
            pen.thin_line(&[(-0.75, 0.75), (0.75, -0.75)]);
            pen.curve(|t| (-0.75 + 1.5 * t, 0.75 - 1.5 * (1.0 - (1.0 - t).powi(3))));
        }
        // --- Modeling.
        "c50" | "cart" | "quest" | "decision_list" => {
            if type_id == "decision_list" {
                for y in [-0.6, 0.0, 0.6] {
                    pen.dot(-0.65, y, 0.12);
                    pen.line(&[(-0.35, y), (0.75, y)]);
                }
            } else {
                pen.tree(2);
            }
        }
        "chaid" => pen.tree(3),
        "random_trees" => {
            // A small forest: two tree silhouettes with trunks.
            for cx in [-0.45, 0.45] {
                pen.fill(&[(cx, -0.75), (cx + 0.38, 0.25), (cx - 0.38, 0.25)]);
                pen.line(&[(cx, 0.25), (cx, 0.75)]);
            }
        }
        "logistic" => {
            pen.curve(|t| {
                let x = -0.85 + 1.7 * t;
                (x, 0.65 - 1.3 / (1.0 + (-6.0 * x).exp()))
            });
            pen.dot(-0.55, 0.65, 0.09);
            pen.dot(0.55, -0.65, 0.09);
        }
        "regression" | "linear" | "linear_as" => {
            for (x, y) in [
                (-0.6, 0.45),
                (-0.25, 0.3),
                (0.05, -0.05),
                (0.35, -0.1),
                (0.6, -0.5),
            ] {
                pen.dot(x, y, 0.1);
            }
            pen.line(&[(-0.85, 0.65), (0.85, -0.65)]);
        }
        "gle" | "genlin" => {
            for (x, y) in [
                (-0.6, 0.6),
                (-0.25, 0.45),
                (0.1, 0.2),
                (0.4, -0.25),
                (0.6, -0.7),
            ] {
                pen.dot(x, y, 0.1);
            }
            pen.curve(|t| (-0.85 + 1.7 * t, 0.75 - 1.5 * t.powf(2.2)));
        }
        "neural_net" => {
            let layers: [&[f32]; 3] = [&[-0.55, 0.0, 0.55], &[-0.3, 0.3], &[0.0]];
            let xs = [-0.7, 0.0, 0.7];
            for i in 0..2 {
                if let (Some(a), Some(b), Some(&xa), Some(&xb)) =
                    (layers.get(i), layers.get(i + 1), xs.get(i), xs.get(i + 1))
                {
                    for &ya in a.iter() {
                        for &yb in b.iter() {
                            pen.thin_line(&[(xa, ya), (xb, yb)]);
                        }
                    }
                }
            }
            for (layer, &x) in layers.iter().zip(&xs) {
                for &y in layer.iter() {
                    pen.dot(x, y, 0.15);
                }
            }
        }
        "kmeans" | "auto_cluster" => {
            for (cx, cy) in [(-0.45, -0.4), (0.45, -0.35), (0.0, 0.45)] {
                pen.ring(cx, cy, 0.38);
                pen.dot(cx - 0.1, cy, 0.07);
                pen.dot(cx + 0.12, cy - 0.08, 0.07);
                pen.dot(cx + 0.02, cy + 0.14, 0.07);
            }
        }
        "twostep" => {
            for (cx, cy) in [(-0.4, -0.1), (0.4, 0.1)] {
                pen.ring(cx, cy, 0.45);
                pen.dot(cx, cy, 0.12);
            }
        }
        "kohonen" => {
            let grid = [-0.55, 0.0, 0.55];
            for &y in &grid {
                pen.thin_line(&[(-0.55, y), (0.55, y)]);
            }
            for &x in &grid {
                pen.thin_line(&[(x, -0.55), (x, 0.55)]);
            }
            for &x in &grid {
                for &y in &grid {
                    pen.dot(x, y, 0.12);
                }
            }
        }
        "time_series" => {
            pen.curve(|t| {
                let x = -0.85 + 1.1 * t;
                (x, 0.35 * (7.0 * t).sin() + 0.2 - 0.3 * t)
            });
            for i in 0..4 {
                let a = 0.25 + 0.17 * i as f32;
                pen.dot(a, -0.15 - 0.12 * i as f32, 0.06);
            }
        }
        "apriori" | "carma" | "sequence" => {
            pen.ring(-0.55, 0.35, 0.25);
            pen.ring(0.0, -0.45, 0.25);
            pen.ring(0.55, 0.35, 0.25);
            if type_id == "sequence" {
                pen.arrow((-0.4, 0.15), (-0.18, -0.2));
                pen.arrow((0.18, -0.2), (0.4, 0.15));
            } else {
                pen.line(&[(-0.4, 0.15), (-0.15, -0.25)]);
                pen.line(&[(0.15, -0.25), (0.4, 0.15)]);
                pen.line(&[(-0.3, 0.35), (0.3, 0.35)]);
            }
        }
        "feature_selection" => {
            for x in [-0.6, -0.3, 0.0, 0.3, 0.6] {
                pen.thin_line(&[(x, -0.8), (x, -0.45)]);
            }
            pen.closed(&[(-0.75, -0.35), (0.75, -0.35), (0.15, 0.25), (-0.15, 0.25)]);
            pen.line(&[(0.0, 0.25), (0.0, 0.8)]);
        }
        "auto_classifier" | "auto_numeric" => {
            pen.rect(-0.8, -0.2, -0.2, 0.4);
            pen.rect(-0.3, -0.5, 0.3, 0.1);
            pen.rect(0.2, -0.8, 0.8, -0.2);
            pen.line(&[(-0.35, 0.6), (-0.1, 0.85), (0.5, 0.25)]);
        }
        "anomaly" => {
            for (x, y) in [
                (-0.55, 0.3),
                (-0.3, 0.5),
                (-0.4, 0.05),
                (-0.1, 0.25),
                (0.0, 0.55),
            ] {
                pen.dot(x, y, 0.1);
            }
            pen.dot(0.55, -0.5, 0.1);
            pen.ring(0.55, -0.5, 0.3);
        }
        "pca_factor" => {
            pen.arrow((-0.6, 0.6), (0.7, -0.3));
            pen.arrow((-0.6, 0.6), (-0.15, -0.75));
        }
        // --- Output.
        "table" => {
            pen.rect(-0.8, -0.65, 0.8, 0.65);
            pen.line(&[(-0.8, -0.2), (0.8, -0.2)]);
            pen.thin_line(&[(-0.8, 0.22), (0.8, 0.22)]);
            pen.thin_line(&[(-0.25, -0.65), (-0.25, 0.65)]);
            pen.thin_line(&[(0.27, -0.65), (0.27, 0.65)]);
        }
        "analysis" => {
            pen.ring(-0.15, -0.15, 0.5);
            pen.line(&[(0.22, 0.22), (0.75, 0.75)]);
        }
        "data_audit" => {
            pen.rect(-0.6, -0.65, 0.6, 0.85);
            pen.bar(-0.25, -0.8, 0.25, -0.55);
            pen.line(&[(-0.35, 0.1), (-0.1, 0.35), (0.35, -0.15)]);
        }
        "statistics" => {
            // A box plot: whiskers, box, median.
            pen.line(&[(-0.85, 0.0), (-0.4, 0.0)]);
            pen.line(&[(0.45, 0.0), (0.85, 0.0)]);
            pen.line(&[(-0.85, -0.25), (-0.85, 0.25)]);
            pen.line(&[(0.85, -0.25), (0.85, 0.25)]);
            pen.rect(-0.4, -0.42, 0.45, 0.42);
            pen.line(&[(0.05, -0.42), (0.05, 0.42)]);
        }
        "matrix" => {
            pen.rect(-0.75, -0.75, 0.75, 0.75);
            pen.thin_line(&[(-0.25, -0.75), (-0.25, 0.75)]);
            pen.thin_line(&[(0.25, -0.75), (0.25, 0.75)]);
            pen.thin_line(&[(-0.75, -0.25), (0.75, -0.25)]);
            pen.thin_line(&[(-0.75, 0.25), (0.75, 0.25)]);
            pen.bar(-0.75, -0.75, -0.25, -0.25);
            pen.bar(-0.25, -0.25, 0.25, 0.25);
            pen.bar(0.25, 0.25, 0.75, 0.75);
        }
        "means" => {
            for (x, top) in [(-0.5, -0.1), (0.0, -0.55), (0.5, 0.15)] {
                pen.bar(x - 0.17, top, x + 0.17, 0.75);
                pen.thin_line(&[(x, top - 0.25), (x, top)]);
                pen.thin_line(&[(x - 0.1, top - 0.25), (x + 0.1, top - 0.25)]);
            }
        }
        // --- Export: a page with an arrow leaving it.
        "flat_file_export" | "statistics_export" => {
            pen.closed(&[
                (-0.85, -0.85),
                (0.0, -0.85),
                (0.3, -0.55),
                (0.3, 0.85),
                (-0.85, 0.85),
            ]);
            for y in [-0.2, 0.15, 0.5] {
                pen.thin_line(&[(-0.6, y), (0.05, y)]);
            }
            pen.arrow((0.0, 0.1), (0.95, 0.1));
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use dms_engine::catalog::NODES;

    #[test]
    fn pictograms_name_catalog_nodes_once() {
        let mut seen = std::collections::HashSet::new();
        for id in PICTOGRAMS {
            assert!(seen.insert(id), "{id} listed twice");
            assert!(
                NODES.iter().any(|spec| spec.id == *id),
                "{id} is not a catalog id"
            );
        }
    }

    #[test]
    fn every_listed_pictogram_draws_and_others_fall_back() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter();
            for id in PICTOGRAMS {
                let pen = Pen {
                    painter,
                    center: pos2(50.0, 50.0),
                    r: 20.0,
                    color: Color32::WHITE,
                    width: 2.0,
                };
                assert!(draw_specific(&pen, id), "{id} has no drawing");
            }
            let pen = Pen {
                painter,
                center: pos2(50.0, 50.0),
                r: 20.0,
                color: Color32::WHITE,
                width: 2.0,
            };
            assert!(!draw_specific(&pen, "cognos_import"));
        });
        // Nothing renders here; drop the font atlas upload explicitly.
        output.textures_delta.clear();
    }
}
