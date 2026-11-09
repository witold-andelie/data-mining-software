//! Our own node colours and glyphs. Every palette has a distinct shape as well as a colour, so
//! the canvas does not rely on colour alone; glyphs are vector paths (sharp on Retina).

use crate::graph::NodeKind;
use dms_engine::catalog::Palette;
use egui::{Color32, Painter, Pos2, Shape, Stroke, pos2, vec2};

pub fn tile_color(kind: NodeKind, dark: bool) -> Color32 {
    let (light, dark_variant) = match kind {
        NodeKind::Palette(Palette::Source) => (rgb(0x0E8A7E), rgb(0x2BB3A5)),
        NodeKind::Palette(Palette::RecordOps) => (rgb(0x2F6FBA), rgb(0x5B95DA)),
        NodeKind::Palette(Palette::FieldOps) => (rgb(0x7A5BC7), rgb(0x9C83E0)),
        NodeKind::Palette(Palette::Graphs) => (rgb(0xB7790B), rgb(0xD9A23A)),
        NodeKind::Palette(Palette::Modeling) => (rgb(0xC2465A), rgb(0xE06F82)),
        NodeKind::Palette(Palette::Output) => (rgb(0x4F8A3A), rgb(0x76B05E)),
        NodeKind::Palette(Palette::Export) => (rgb(0x8A6233), rgb(0xB78A57)),
        NodeKind::Palette(Palette::Structure) | NodeKind::Supernode => {
            (rgb(0x55606E), rgb(0x8A96A5))
        }
        NodeKind::Structural => (rgb(0x8A929C), rgb(0x6B7380)),
        NodeKind::Unknown => (rgb(0x9AA0A6), rgb(0x5F656B)),
    };
    if dark { dark_variant } else { light }
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Draws the glyph of a node kind centred at `center`, `size` points across.
pub fn glyph(painter: &Painter, kind: NodeKind, center: Pos2, size: f32, color: Color32) {
    let r = size / 2.0;
    let stroke = Stroke::new((size / 9.0).max(1.0), color);
    let at = |x: f32, y: f32| pos2(center.x + x * r, center.y + y * r);
    let polygon = |points: Vec<Pos2>| Shape::convex_polygon(points, color, Stroke::NONE);
    match kind {
        NodeKind::Palette(Palette::Source) => {
            // A cylinder: data at rest.
            painter.add(Shape::ellipse_stroke(
                at(0.0, -0.55),
                vec2(r * 0.8, r * 0.3),
                stroke,
            ));
            painter.line_segment([at(-0.8, -0.55), at(-0.8, 0.55)], stroke);
            painter.line_segment([at(0.8, -0.55), at(0.8, 0.55)], stroke);
            painter.add(Shape::ellipse_stroke(
                at(0.0, 0.55),
                vec2(r * 0.8, r * 0.3),
                stroke,
            ));
        }
        NodeKind::Palette(Palette::RecordOps) => {
            // Rows.
            for y in [-0.6, 0.0, 0.6] {
                painter.line_segment([at(-0.8, y), at(0.8, y)], stroke);
            }
        }
        NodeKind::Palette(Palette::FieldOps) => {
            // Columns.
            for x in [-0.6, 0.0, 0.6] {
                painter.line_segment([at(x, -0.8), at(x, 0.8)], stroke);
            }
        }
        NodeKind::Palette(Palette::Graphs) => {
            for (x, h) in [(-0.6, 0.6), (0.0, 1.4), (0.6, 1.0)] {
                painter.line_segment([at(x, 0.8), at(x, 0.8 - h)], stroke);
            }
        }
        NodeKind::Palette(Palette::Modeling) => {
            // A small tree.
            painter.line_segment([at(0.0, -0.7), at(-0.6, 0.6)], stroke);
            painter.line_segment([at(0.0, -0.7), at(0.6, 0.6)], stroke);
            painter.circle_filled(at(0.0, -0.7), r * 0.22, color);
            painter.circle_filled(at(-0.6, 0.6), r * 0.22, color);
            painter.circle_filled(at(0.6, 0.6), r * 0.22, color);
        }
        NodeKind::Palette(Palette::Output) => {
            // A table.
            painter.rect_stroke(
                egui::Rect::from_center_size(center, vec2(size * 0.8, size * 0.8)),
                1.0,
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.line_segment([at(-0.8, -0.2), at(0.8, -0.2)], stroke);
            painter.line_segment([at(-0.1, -0.8), at(-0.1, 0.8)], stroke);
        }
        NodeKind::Palette(Palette::Export) => {
            // An arrow leaving a box.
            painter.line_segment([at(-0.7, 0.0), at(0.5, 0.0)], stroke);
            painter.add(polygon(vec![at(0.8, 0.0), at(0.3, -0.45), at(0.3, 0.45)]));
            painter.line_segment([at(-0.7, -0.7), at(-0.7, 0.7)], stroke);
        }
        NodeKind::Palette(Palette::Structure) | NodeKind::Supernode => {
            // Nested squares.
            for s in [0.9, 0.45] {
                painter.rect_stroke(
                    egui::Rect::from_center_size(center, vec2(size * s, size * s)),
                    1.0,
                    stroke,
                    egui::StrokeKind::Middle,
                );
            }
        }
        NodeKind::Structural => {
            painter.circle_stroke(center, r * 0.6, stroke);
        }
        NodeKind::Unknown => {
            painter.add(polygon(vec![
                at(0.0, -0.8),
                at(0.8, 0.0),
                at(0.0, 0.8),
                at(-0.8, 0.0),
            ]));
        }
    }
}
