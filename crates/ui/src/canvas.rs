//! The stream canvas: a pannable, zoomable view of a [`StreamGraph`], drawn with the egui
//! painter. Trackpad: two-finger scroll pans, pinch zooms around the pointer; mouse: drag the
//! background or scroll to pan, ⌘/Ctrl + scroll to zoom.
//!
//! The view (pan, zoom, selection) is plain serializable data; the graph is never mutated here
//! (editing goes through engine commands from phase B).

use crate::graph::{GraphNode, NodeKind, NodeStatus, StreamGraph, TILE};
use crate::theme;
use egui::epaint::CubicBezierShape;
use egui::{
    Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind,
    Ui, Vec2, pos2, vec2,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 4.0;
/// Zoom factor of one zoom-in or zoom-out command.
pub const ZOOM_STEP: f32 = 1.25;
/// Below this zoom labels are not drawn (unreadable, and the costliest part of a frame).
const LABEL_MIN_ZOOM: f32 = 0.35;
const LABEL_SIZE: f32 = 12.0;
/// Room around the stream when zooming to fit, in screen points.
const FIT_MARGIN: f32 = 32.0;

/// Pan, zoom and selection of one canvas. `pan` is the screen offset of the canvas origin from
/// the canvas rectangle's top-left corner, in points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CanvasView {
    pub pan: [f32; 2],
    pub zoom: f32,
    pub selected: Option<String>,
}

impl Default for CanvasView {
    fn default() -> Self {
        CanvasView {
            pan: [0.0, 0.0],
            zoom: 1.0,
            selected: None,
        }
    }
}

impl CanvasView {
    fn pan_vec(&self) -> Vec2 {
        vec2(self.pan[0], self.pan[1])
    }

    /// Restores invariants after deserializing untrusted state.
    pub fn sanitize(&mut self) {
        if !self.zoom.is_finite() {
            self.zoom = 1.0;
        }
        self.zoom = self.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        for v in &mut self.pan {
            if !v.is_finite() {
                *v = 0.0;
            }
            *v = v.clamp(-1.0e7, 1.0e7);
        }
    }

    pub fn to_screen(&self, origin: Pos2, p: Pos2) -> Pos2 {
        origin + self.pan_vec() + p.to_vec2() * self.zoom
    }

    pub fn to_canvas(&self, origin: Pos2, p: Pos2) -> Pos2 {
        ((p - origin - self.pan_vec()) / self.zoom).to_pos2()
    }

    /// Zooms by `factor`, keeping the canvas point under `anchor` (screen) in place.
    pub fn zoom_about(&mut self, origin: Pos2, anchor: Pos2, factor: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let before = self.to_canvas(origin, anchor);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.to_screen(origin, before);
        let shift = anchor - after;
        self.pan = [self.pan[0] + shift.x, self.pan[1] + shift.y];
    }

    /// Shows all of `bounds` (canvas units) centred in a viewport of `size` points.
    pub fn fit(&mut self, bounds: Rect, size: Vec2) {
        let room = (size - Vec2::splat(2.0 * FIT_MARGIN)).max(Vec2::splat(1.0));
        let extent = bounds.size().max(Vec2::splat(1.0));
        self.zoom = (room.x / extent.x)
            .min(room.y / extent.y)
            .clamp(MIN_ZOOM, 1.5);
        let offset = size / 2.0 - bounds.center().to_vec2() * self.zoom;
        self.pan = [offset.x, offset.y];
    }
}

/// How a node fared in the latest run, drawn as a badge on its tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMark {
    Running,
    Done,
    Failed,
    /// Not run because a node upstream failed.
    Blocked,
}

/// What happened on the canvas this frame.
#[derive(Debug, Default)]
pub struct CanvasEvents {
    pub hovered: Option<usize>,
    /// Number of nodes drawn (after culling).
    pub drawn_nodes: usize,
}

/// Draws the canvas into all of `ui`'s available space and handles pan, zoom and selection.
pub fn show(
    ui: &mut Ui,
    graph: &StreamGraph,
    view: &mut CanvasView,
    marks: &HashMap<String, RunMark>,
) -> (Response, CanvasEvents) {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let origin = rect.min;

    if response.hovered() {
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta(), i.zoom_delta()));
        if zoom != 1.0 {
            let anchor = response.hover_pos().unwrap_or(rect.center());
            view.zoom_about(origin, anchor, zoom);
        } else if scroll != Vec2::ZERO {
            view.pan = [view.pan[0] + scroll.x, view.pan[1] + scroll.y];
        }
    }
    if response.dragged() {
        let delta = response.drag_delta();
        view.pan = [view.pan[0] + delta.x, view.pan[1] + delta.y];
    }

    let painter = ui.painter_at(rect);
    let visuals = ui.visuals().clone();
    let dark = visuals.dark_mode;
    painter.rect_filled(rect, CornerRadius::ZERO, canvas_background(dark));
    draw_grid(&painter, rect, view, dark);

    let link_color = if dark {
        Color32::from_gray(150)
    } else {
        Color32::from_gray(110)
    };
    let link_stroke = Stroke::new((1.5 * view.zoom).clamp(0.75, 3.0), link_color);
    let visible = rect.expand(40.0 * view.zoom.max(1.0));
    for link in &graph.links {
        let (Some(from), Some(to)) = (graph.nodes.get(link.from), graph.nodes.get(link.to)) else {
            continue;
        };
        let a = view.to_screen(origin, from.pos);
        let b = view.to_screen(origin, to.pos);
        if !visible.intersects(Rect::from_two_pos(a, b)) {
            continue;
        }
        draw_link(&painter, a, b, view.zoom, link_stroke);
    }

    let pointer = response.hover_pos();
    let mut events = CanvasEvents::default();
    for (index, node) in graph.nodes.iter().enumerate() {
        let center = view.to_screen(origin, node.pos);
        let tile = Rect::from_center_size(center, TILE * view.zoom);
        if !visible.intersects(tile) {
            continue;
        }
        events.drawn_nodes += 1;
        let hovered = pointer.is_some_and(|p| tile.contains(p));
        if hovered {
            events.hovered = Some(index);
        }
        let selected = view.selected.as_deref() == Some(node.uid.as_str());
        draw_node(
            &painter, node, tile, view.zoom, dark, hovered, selected, &visuals,
        );
        if let Some(mark) = marks.get(&node.uid) {
            draw_mark(&painter, tile, view.zoom, *mark, dark);
        }
    }

    if response.clicked() {
        view.selected = events
            .hovered
            .and_then(|i| graph.nodes.get(i))
            .map(|node| node.uid.clone());
    }
    let response = match events.hovered.and_then(|i| graph.nodes.get(i)) {
        Some(node) => response.on_hover_text(format!(
            "{}\n{}\n{}",
            node.title,
            node.type_id,
            node.status.explanation()
        )),
        None => response,
    };
    (response, events)
}

/// A badge at the tile's top-right corner: a ring for running, a dot for done, an exclamation
/// disc for failed, a hollow square for blocked. Shapes differ, not only colours.
fn draw_mark(painter: &egui::Painter, tile: Rect, zoom: f32, mark: RunMark, dark: bool) {
    let r = (7.0 * zoom).clamp(4.0, 10.0);
    let center = tile.right_top() + vec2(-r * 0.3, r * 0.3);
    let outline = if dark {
        Color32::from_gray(30)
    } else {
        Color32::WHITE
    };
    let ring = |color: Color32, width: f32| {
        painter.rect_stroke(
            tile.expand(4.0 * zoom.max(0.5)),
            CornerRadius::same((12.0 * zoom).clamp(2.0, 22.0) as u8),
            Stroke::new(width, color),
            StrokeKind::Outside,
        );
    };
    match mark {
        RunMark::Running => {
            let blue = Color32::from_rgb(0x2F, 0x80, 0xED);
            ring(blue, 2.0);
            painter.circle_filled(center, r, outline);
            painter.circle_stroke(center, r * 0.7, Stroke::new(r * 0.4, blue));
        }
        RunMark::Done => {
            painter.circle_filled(center, r * 0.75, Color32::from_rgb(0x2E, 0xA0, 0x43));
            painter.circle_stroke(center, r * 0.75, Stroke::new(1.5, outline));
        }
        RunMark::Failed => {
            let red = Color32::from_rgb(0xD9, 0x30, 0x25);
            ring(red, 2.5);
            painter.circle_filled(center, r, red);
            painter.circle_stroke(center, r, Stroke::new(1.5, outline));
            painter.line_segment(
                [center + vec2(0.0, -r * 0.55), center + vec2(0.0, r * 0.15)],
                Stroke::new(r * 0.28, Color32::WHITE),
            );
            painter.circle_filled(center + vec2(0.0, r * 0.5), r * 0.15, Color32::WHITE);
        }
        RunMark::Blocked => {
            let amber = Color32::from_rgb(0xC9, 0x8A, 0x12);
            let square = Rect::from_center_size(center, vec2(r * 1.3, r * 1.3));
            painter.rect_filled(square, 2.0, outline);
            painter.rect_stroke(square, 2.0, Stroke::new(r * 0.3, amber), StrokeKind::Inside);
        }
    }
}

fn canvas_background(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(0x1E, 0x20, 0x24)
    } else {
        Color32::from_rgb(0xF7, 0xF7, 0xF5)
    }
}

fn draw_grid(painter: &egui::Painter, rect: Rect, view: &CanvasView, dark: bool) {
    let step = 40.0 * view.zoom;
    if step < 12.0 {
        return;
    }
    let color = if dark {
        Color32::from_gray(52)
    } else {
        Color32::from_gray(222)
    };
    let start = rect.min + view.pan_vec();
    let first_x = rect.min.x + (start.x - rect.min.x).rem_euclid(step);
    let first_y = rect.min.y + (start.y - rect.min.y).rem_euclid(step);
    let radius = (1.2 * view.zoom).clamp(0.6, 1.5);
    let mut y = first_y;
    while y < rect.max.y {
        let mut x = first_x;
        while x < rect.max.x {
            painter.circle_filled(pos2(x, y), radius, color);
            x += step;
        }
        y += step;
    }
}

fn draw_link(painter: &egui::Painter, a: Pos2, b: Pos2, zoom: f32, stroke: Stroke) {
    let half = TILE.x * zoom / 2.0;
    let direction = (b - a).normalized();
    if !direction.is_finite() || direction == Vec2::ZERO {
        return;
    }
    // Leave and enter the tiles at their edges, with horizontal tangents when the target is to
    // the right (the usual left-to-right flow), straight otherwise.
    let start = a + direction * half;
    let end = b - direction * (half + 2.0);
    let horizontal = (b.x - a.x).abs() > (b.y - a.y).abs() * 0.5;
    if horizontal {
        let pull = ((end.x - start.x).abs() * 0.4).max(10.0);
        let sign = if end.x >= start.x { 1.0 } else { -1.0 };
        let bezier = CubicBezierShape::from_points_stroke(
            [
                start,
                start + vec2(pull * sign, 0.0),
                end - vec2(pull * sign, 0.0),
                end,
            ],
            false,
            Color32::TRANSPARENT,
            stroke,
        );
        painter.add(bezier);
    } else {
        painter.line_segment([start, end], stroke);
    }
    let tip = end;
    let back = if horizontal {
        vec2(if end.x >= start.x { 1.0 } else { -1.0 }, 0.0)
    } else {
        direction
    };
    let size = (7.0 * zoom).clamp(3.0, 12.0);
    let side = back.rot90() * size * 0.5;
    painter.add(Shape::convex_polygon(
        vec![tip, tip - back * size + side, tip - back * size - side],
        stroke.color,
        Stroke::NONE,
    ));
}

#[allow(clippy::too_many_arguments)]
fn draw_node(
    painter: &egui::Painter,
    node: &GraphNode,
    tile: Rect,
    zoom: f32,
    dark: bool,
    hovered: bool,
    selected: bool,
    visuals: &egui::Visuals,
) {
    let fill = theme::tile_color(node.kind, dark);
    let ready = node.status == NodeStatus::Ready;
    let fill = if ready {
        fill
    } else {
        fill.gamma_multiply(0.45)
    };
    let radius = CornerRadius::same((10.0 * zoom).clamp(2.0, 20.0) as u8);
    painter.rect_filled(tile, radius, fill);
    if selected || hovered {
        let color = if selected {
            visuals.selection.stroke.color
        } else {
            visuals.widgets.hovered.fg_stroke.color
        };
        painter.rect_stroke(
            tile.expand(3.0 * zoom.max(0.5)),
            radius,
            Stroke::new(if selected { 2.5 } else { 1.5 }, color),
            StrokeKind::Outside,
        );
    }
    if node.kind == NodeKind::Unknown || !ready {
        // Not runnable: a dashed outline, readable without colour.
        let outline = if dark {
            Color32::from_gray(200)
        } else {
            Color32::from_gray(70)
        };
        let corners = [
            tile.left_top(),
            tile.right_top(),
            tile.right_bottom(),
            tile.left_bottom(),
            tile.left_top(),
        ];
        let dash = (6.0 * zoom).max(2.0);
        painter.extend(Shape::dashed_line(
            &corners,
            Stroke::new(1.2, outline),
            dash,
            dash * 0.7,
        ));
    }
    theme::glyph(
        painter,
        node.kind,
        tile.center(),
        tile.width() * 0.5,
        Color32::WHITE,
    );
    if zoom >= LABEL_MIN_ZOOM {
        let text_color = visuals.text_color();
        painter.text(
            pos2(tile.center().x, tile.max.y + 4.0 * zoom),
            Align2::CENTER_TOP,
            elide(&node.title, 22),
            FontId::proportional(LABEL_SIZE * zoom),
            text_color,
        );
    }
}

/// At most `max` characters, cut on a character boundary.
fn elide(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_the_anchor_in_place_and_is_clamped() {
        let origin = pos2(10.0, 20.0);
        let mut view = CanvasView::default();
        let anchor = pos2(300.0, 200.0);
        let before = view.to_canvas(origin, anchor);
        view.zoom_about(origin, anchor, 2.0);
        let after = view.to_canvas(origin, anchor);
        assert!((before - after).length() < 1e-3);
        view.zoom_about(origin, anchor, 1000.0);
        assert_eq!(view.zoom, MAX_ZOOM);
        view.zoom_about(origin, anchor, f32::NAN);
        assert_eq!(view.zoom, MAX_ZOOM);
        view.zoom_about(origin, anchor, 0.0);
        assert_eq!(view.zoom, MAX_ZOOM);
    }

    #[test]
    fn fit_centres_the_bounds() {
        let mut view = CanvasView::default();
        let bounds = Rect::from_min_max(pos2(100.0, 100.0), pos2(500.0, 300.0));
        view.fit(bounds, vec2(800.0, 600.0));
        let centre = view.to_screen(Pos2::ZERO, bounds.center());
        assert!((centre - pos2(400.0, 300.0)).length() < 1e-3);
        assert!(view.zoom > 0.0 && view.zoom <= 1.5);
    }

    #[test]
    fn view_state_round_trips_and_sanitizes() {
        let view = CanvasView {
            pan: [12.5, -3.0],
            zoom: 1.75,
            selected: Some("idX".into()),
        };
        let json = serde_json::to_string(&view).unwrap();
        assert_eq!(serde_json::from_str::<CanvasView>(&json).unwrap(), view);
        // Older or partial state still loads.
        let partial: CanvasView = serde_json::from_str(r#"{"zoom": 2.0}"#).unwrap();
        assert_eq!(partial.pan, [0.0, 0.0]);
        let mut hostile: CanvasView =
            serde_json::from_str(r#"{"zoom": 1e30, "pan": [1e30, -1e30]}"#).unwrap();
        hostile.sanitize();
        assert_eq!(hostile.zoom, MAX_ZOOM);
        assert_eq!(hostile.pan, [1.0e7, -1.0e7]);
    }

    #[test]
    fn elide_cuts_on_characters() {
        assert_eq!(elide("Výběr", 22), "Výběr");
        assert_eq!(elide("Žluťoučký kůň úpěl ďábelské ódy", 10), "Žluťoučký…");
    }
}
