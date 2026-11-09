//! The application icon, drawn with the same vector glyphs as the canvas: a small stream
//! (data → tree model → table) on a rounded square. `crates/ui/tests/icon.rs` renders it to
//! `apps/dms/assets/icon.png`, from which `cargo xtask bundle-mac` makes the `.icns`.

use crate::graph::NodeKind;
use crate::theme;
use dms_engine::catalog::Palette;
use egui::epaint::CubicBezierShape;
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Stroke, pos2, vec2};

/// Paints the icon into `rect` (square). Proportions follow the macOS icon grid: the body is
/// 824/1024 of the canvas with a 185/1024 corner radius.
pub fn paint_app_icon(painter: &Painter, rect: Rect) {
    let unit = rect.width() / 1024.0;
    let at = |x: f32, y: f32| pos2(rect.min.x + x * unit, rect.min.y + y * unit);
    let body = Rect::from_min_max(at(100.0, 100.0), at(924.0, 924.0));
    let radius = CornerRadius::same((185.0 * unit).clamp(0.0, 255.0) as u8);
    painter.rect_filled(body, radius, Color32::from_rgb(0x0B, 0x4F, 0x5C));
    // A lighter upper band for depth.
    painter.rect_filled(
        Rect::from_min_max(body.min, pos2(body.max.x, body.center().y)),
        CornerRadius {
            nw: radius.nw,
            ne: radius.ne,
            sw: 0,
            se: 0,
        },
        Color32::from_rgb(0x12, 0x63, 0x70),
    );

    let nodes: [(Pos2, NodeKind); 3] = [
        (at(300.0, 640.0), NodeKind::Palette(Palette::Source)),
        (at(512.0, 380.0), NodeKind::Palette(Palette::Modeling)),
        (at(724.0, 640.0), NodeKind::Palette(Palette::Output)),
    ];
    let link = Stroke::new(22.0 * unit, Color32::from_rgb(0xE8, 0xF3, 0xF4));
    for pair in nodes.windows(2) {
        if let [(a, _), (b, _)] = pair {
            let pull = vec2((b.x - a.x) * 0.5, 0.0);
            painter.add(CubicBezierShape::from_points_stroke(
                [*a, *a + pull, *b - pull, *b],
                false,
                Color32::TRANSPARENT,
                link,
            ));
        }
    }
    let tile = vec2(190.0, 190.0) * unit;
    for (center, kind) in nodes {
        let square = Rect::from_center_size(center, tile);
        painter.rect_filled(
            square.translate(vec2(0.0, 10.0 * unit)),
            CornerRadius::same((44.0 * unit).clamp(0.0, 255.0) as u8),
            Color32::from_black_alpha(60),
        );
        painter.rect_filled(
            square,
            CornerRadius::same((44.0 * unit).clamp(0.0, 255.0) as u8),
            theme::tile_color(kind, false),
        );
        theme::glyph(painter, kind, center, tile.x * 0.52, Color32::WHITE);
    }
}
