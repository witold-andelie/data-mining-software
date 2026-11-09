//! Transient canvas gestures. Only completed gestures produce engine commands.
use crate::{
    canvas::{self, CanvasView, RunMark},
    graph::{StreamGraph, TILE},
};
use dms_engine::session::Arg;
use egui::{Color32, Key, Pos2, Rect, Role, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo, vec2};
use std::collections::{HashMap, HashSet};

pub enum Action {
    Command(&'static str, Vec<Arg>),
    Rename(String),
}

enum Gesture {
    Move { start: Pos2, targets: Vec<String> },
    Connect { from: usize },
    Box { start: Pos2, base: Vec<String> },
    Pan { last: Pos2 },
}

pub struct Rename {
    pub uid: String,
    pub text: String,
    first_frame: bool,
}

#[derive(Default)]
pub struct Editor {
    gesture: Option<Gesture>,
    pub selected_link: Option<(String, String)>,
    pub rename: Option<Rename>,
    pub focused: bool,
    /// Retain a rejected connection's explanation after release.
    pub rejection: Option<String>,
    context_link: Option<(String, String)>,
    /// "Connect From Here": the next node clicked becomes the target (Esc cancels).
    pub connect_from: Option<String>,
}

impl Editor {
    pub fn begin_rename(&mut self, uid: &str, graph: &StreamGraph) {
        if let Some(node) = graph.nodes.iter().find(|n| n.uid == uid) {
            self.rename = Some(Rename {
                uid: uid.into(),
                text: node.title.clone(),
                first_frame: true,
            });
        }
    }
}

/// How a connection is made, shown while the pointer is over a node.
pub fn connect_hint(mac: bool) -> &'static str {
    if mac {
        "To connect: drag from the circle on the right edge, Option-drag the node, or right-click > Connect From Here"
    } else {
        "To connect: drag from the circle on the right edge, Alt-drag the node, or right-click > Connect From Here"
    }
}

/// Screen radius within which a press starts a connection from a node's output port.
const PORT_HIT: f32 = 14.0;

/// Bounded, iterative reachability over the displayed top-level graph.
fn connection_error(graph: &StreamGraph, from: usize, to: usize) -> Option<&'static str> {
    if graph.hidden_nodes > 0 {
        return Some("Connections are unavailable while nodes are hidden.");
    }
    if from == to {
        return Some("A node cannot connect to itself.");
    }
    if graph.links.iter().any(|l| l.from == from && l.to == to) {
        return Some("This connection already exists.");
    }
    let mut outgoing: HashMap<usize, Vec<usize>> = HashMap::new();
    for link in &graph.links {
        outgoing.entry(link.from).or_default().push(link.to);
    }
    let mut seen = HashSet::new();
    let mut stack = vec![to];
    while let Some(current) = stack.pop() {
        if current == from {
            return Some("This connection would create a cycle.");
        }
        if seen.insert(current)
            && let Some(next) = outgoing.get(&current)
        {
            stack.extend(next.iter().copied().filter(|n| *n < graph.nodes.len()));
        }
    }
    None
}

fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let v = b - a;
    let length = v.length_sq();
    if length <= f32::EPSILON {
        return p.distance(a);
    }
    p.distance(a + v * ((p - a).dot(v) / length).clamp(0.0, 1.0))
}

/// Hit test the same curve that canvas::draw_link paints, with a fixed subdivision budget.
fn on_link(p: Pos2, a: Pos2, b: Pos2, zoom: f32) -> bool {
    let direction = (b - a).normalized();
    if !direction.is_finite() || direction == Vec2::ZERO {
        return false;
    }
    let start = a + direction * (TILE.x * zoom / 2.0);
    let end = b - direction * (TILE.x * zoom / 2.0 + 2.0);
    if (b.x - a.x).abs() <= (b.y - a.y).abs() * 0.5 {
        return distance_to_segment(p, start, end) < 6.0;
    }
    let pull =
        ((end.x - start.x).abs() * 0.4).max(10.0) * if end.x >= start.x { 1.0 } else { -1.0 };
    let c1 = start + vec2(pull, 0.0);
    let c2 = end - vec2(pull, 0.0);
    let mut last = start;
    for step in 1..=24 {
        let t = step as f32 / 24.0;
        let u = 1.0 - t;
        let next = (start.to_vec2() * u.powi(3)
            + c1.to_vec2() * (3.0 * u * u * t)
            + c2.to_vec2() * (3.0 * u * t * t)
            + end.to_vec2() * t.powi(3))
        .to_pos2();
        if distance_to_segment(p, last, next) < 6.0 {
            return true;
        }
        last = next;
    }
    false
}

/// Convert finite, bounded view coordinates to the engine's integer canvas coordinates.
pub fn coordinate(value: f32) -> i64 {
    if value.is_finite() {
        value.round().clamp(-1_000_000.0, 1_000_000.0) as i64
    } else {
        0
    }
}

pub fn show(
    ui: &mut Ui,
    graph: &StreamGraph,
    view: &mut CanvasView,
    marks: &HashMap<String, RunMark>,
    selection: &[String],
    editor: &mut Editor,
) -> Vec<Action> {
    view.sanitize();
    let rect = ui.available_rect_before_wrap();
    let background = ui.allocate_rect(rect, Sense::click_and_drag());
    background.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Stream canvas"));
    let origin = rect.min;
    let mut actions = Vec::new();
    let (pointer, pressed, released, shift, space, alt, escape) = ui.input(|i| {
        (
            i.pointer.interact_pos(),
            i.pointer.primary_pressed(),
            i.pointer.primary_released(),
            i.modifiers.shift,
            i.key_down(Key::Space),
            i.modifiers.alt,
            i.key_pressed(Key::Escape),
        )
    });
    if escape {
        editor.connect_from = None;
    }
    let inside = pointer.is_some_and(|p| rect.contains(p))
        && ui.ctx().layer_id_at(pointer.unwrap_or(rect.min)) == Some(ui.layer_id());
    if inside {
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta(), i.zoom_delta()));
        if zoom != 1.0 {
            view.zoom_about(origin, pointer.unwrap_or(rect.center()), zoom);
        } else {
            view.pan = [view.pan[0] + scroll.x, view.pan[1] + scroll.y];
        }
    }
    let selected: HashSet<&str> = selection.iter().map(String::as_str).collect();
    let offset = match (&editor.gesture, pointer) {
        (Some(Gesture::Move { start, .. }), Some(p)) => (p - *start) / view.zoom,
        _ => Vec2::ZERO,
    };
    let moved: HashSet<&str> = match &editor.gesture {
        Some(Gesture::Move { targets, .. }) => targets.iter().map(String::as_str).collect(),
        _ => HashSet::new(),
    };
    let center = |index: usize| {
        graph.nodes.get(index).map(|n| {
            view.to_screen(
                origin,
                n.pos
                    + if moved.contains(n.uid.as_str()) {
                        offset
                    } else {
                        Vec2::ZERO
                    },
            )
        })
    };
    let hovered = pointer.filter(|_| inside).and_then(|p| {
        graph.nodes.iter().enumerate().rev().find_map(|(i, _)| {
            center(i)
                .filter(|c| {
                    Rect::from_center_size(*c, TILE * view.zoom)
                        .expand2(vec2(0.0, 20.0 * view.zoom))
                        .contains(p)
                })
                .map(|_| i)
        })
    });
    let port = pointer.filter(|_| inside).and_then(|p| {
        graph.nodes.iter().enumerate().rev().find_map(|(i, _)| {
            center(i)
                .filter(|c| p.distance(*c + vec2(TILE.x * view.zoom / 2.0, 0.0)) <= PORT_HIT)
                .map(|_| i)
        })
    });
    let hit_link = pointer
        .filter(|_| inside && hovered.is_none() && port.is_none())
        .and_then(|p| {
            graph.links.iter().find_map(|link| {
                let (a, b) = (center(link.from)?, center(link.to)?);
                if on_link(p, a, b, view.zoom) {
                    Some((
                        graph.nodes.get(link.from)?.uid.clone(),
                        graph.nodes.get(link.to)?.uid.clone(),
                    ))
                } else {
                    None
                }
            })
        });
    let painter = ui.painter_at(rect);
    let dark = ui.visuals().dark_mode;
    painter.rect_filled(rect, 0.0, canvas::canvas_background(dark));
    canvas::draw_grid(&painter, rect, view, dark);
    for link in &graph.links {
        let (Some(a), Some(b), Some(from), Some(to)) = (
            center(link.from),
            center(link.to),
            graph.nodes.get(link.from),
            graph.nodes.get(link.to),
        ) else {
            continue;
        };
        if !rect.expand(40.0).intersects(Rect::from_two_pos(a, b)) {
            continue;
        }
        let active = editor
            .selected_link
            .as_ref()
            .is_some_and(|l| l.0 == from.uid && l.1 == to.uid);
        let color = if active {
            ui.visuals().selection.stroke.color
        } else {
            Color32::from_gray(if dark { 150 } else { 110 })
        };
        canvas::draw_link(
            &painter,
            a,
            b,
            view.zoom,
            Stroke::new(
                if active {
                    3.0
                } else {
                    (1.5 * view.zoom).clamp(0.75, 3.0)
                },
                color,
            ),
        );
    }
    for (i, node) in graph.nodes.iter().enumerate() {
        let Some(c) = center(i) else {
            continue;
        };
        let tile = Rect::from_center_size(c, TILE * view.zoom);
        if !rect.expand(40.0 * view.zoom.max(1.0)).intersects(tile) {
            continue;
        }
        let response = ui.interact(
            tile.with_max_y(tile.max.y + 22.0 * view.zoom),
            ui.scope_id().with((&node.uid, "node")),
            Sense::click_and_drag(),
        );
        response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &node.title));
        canvas::draw_node(
            &painter,
            node,
            tile,
            view.zoom,
            dark,
            hovered == Some(i),
            selected.contains(node.uid.as_str()),
            ui.visuals(),
        );
        if let Some(mark) = marks.get(&node.uid) {
            canvas::draw_mark(&painter, tile, view.zoom, *mark, dark);
        }
        let port_pos = tile.right_center();
        let port_response = ui.interact(
            Rect::from_center_size(port_pos, Vec2::splat(16.0)),
            ui.scope_id().with((&node.uid, "port")),
            Sense::drag(),
        );
        port_response.widget_info(|| {
            WidgetInfo::labeled(Role::Button, true, format!("Connect from {}", node.title))
        });
        // The output port: always shown (a ring), filled and larger under the pointer.
        let accent = ui.visuals().selection.stroke.color;
        let ring = (4.5 * view.zoom).clamp(3.0, 6.0);
        if port == Some(i) || editor.connect_from.as_deref() == Some(node.uid.as_str()) {
            painter.circle_filled(port_pos, ring + 2.0, accent);
        } else if hovered == Some(i) {
            painter.circle_filled(port_pos, ring + 1.0, accent);
        } else if view.zoom >= 0.3 {
            painter.circle(
                port_pos,
                ring,
                canvas::canvas_background(dark),
                Stroke::new(1.5, accent),
            );
        }
        if response.double_clicked() {
            actions.push(Action::Rename(node.uid.clone()));
        }
        response.context_menu(|ui| {
            if ui.button("Connect From Here").clicked() {
                editor.connect_from = Some(node.uid.clone());
                ui.close();
            }
            for (label, id) in [
                ("Run to Here", "run.node"),
                ("Rename", "node.rename"),
                ("Copy", "edit.copy"),
                ("Delete", "edit.delete"),
            ] {
                if ui.button(label).clicked() {
                    actions.push(Action::Command(
                        "edit.select",
                        vec![Arg::Nodes(vec![node.uid.clone()])],
                    ));
                    if id == "node.rename" {
                        actions.push(Action::Rename(node.uid.clone()));
                    } else {
                        actions.push(Action::Command(
                            id,
                            if id == "run.node" {
                                vec![Arg::Node(node.uid.clone())]
                            } else {
                                vec![]
                            },
                        ));
                    }
                    ui.close();
                }
            }
        });
    }
    // End the immutable view borrows before changing pan or the gesture.
    if pressed && inside && editor.rename.is_none() {
        editor.focused = true;
        editor.rejection = None;
        if let Some(from_uid) = editor.connect_from.take() {
            // Second click of "Connect From Here": a node completes it, anything else cancels.
            let from = graph.nodes.iter().position(|n| n.uid == from_uid);
            if let (Some(from), Some(to)) = (from, hovered) {
                if let Some(reason) = connection_error(graph, from, to) {
                    editor.rejection = Some(reason.into());
                } else if let (Some(a), Some(b)) = (graph.nodes.get(from), graph.nodes.get(to)) {
                    actions.push(Action::Command(
                        "node.connect",
                        vec![Arg::Link(a.uid.clone(), b.uid.clone())],
                    ));
                }
            }
        } else if space {
            editor.gesture = pointer.map(|last| Gesture::Pan { last });
        } else if let Some(from) = port.or(hovered.filter(|_| alt)) {
            editor.selected_link = None;
            editor.gesture = Some(Gesture::Connect { from });
        } else if let Some(node) = hovered.and_then(|i| graph.nodes.get(i)) {
            editor.selected_link = None;
            let mut targets = selection.to_vec();
            if shift {
                if selected.contains(node.uid.as_str()) {
                    targets.retain(|uid| uid != &node.uid);
                } else {
                    targets.push(node.uid.clone());
                }
            } else if !selected.contains(node.uid.as_str()) {
                targets = vec![node.uid.clone()];
            }
            actions.push(Action::Command(
                "edit.select",
                vec![Arg::Nodes(targets.clone())],
            ));
            editor.gesture = pointer.map(|start| Gesture::Move { start, targets });
        } else if let Some(link) = &hit_link {
            editor.selected_link = Some(link.clone());
            actions.push(Action::Command("edit.select", vec![Arg::Nodes(vec![])]));
        } else {
            editor.selected_link = None;
            editor.gesture = pointer.map(|start| Gesture::Box {
                start,
                base: if shift { selection.to_vec() } else { vec![] },
            });
            if !shift {
                actions.push(Action::Command("edit.select", vec![Arg::Nodes(vec![])]));
            }
        }
    } else if pressed && !inside && editor.rename.is_none() {
        editor.focused = false;
    }
    if let (Some(Gesture::Pan { last }), Some(p)) = (&mut editor.gesture, pointer) {
        let delta = p - *last;
        view.pan = [view.pan[0] + delta.x, view.pan[1] + delta.y];
        *last = p;
    }
    if let (Some(Gesture::Box { start, .. }), Some(p)) = (&editor.gesture, pointer) {
        painter.rect_stroke(
            Rect::from_two_pos(*start, p),
            0.0,
            ui.visuals().selection.stroke,
            StrokeKind::Inside,
        );
    }
    let mut invalid = None;
    if let (Some(Gesture::Connect { from }), Some(p)) = (&editor.gesture, pointer)
        && let Some(n) = graph.nodes.get(*from)
    {
        let a = view.to_screen(origin, n.pos) + vec2(TILE.x * view.zoom / 2.0, 0.0);
        invalid = hovered.and_then(|to| connection_error(graph, *from, to));
        painter.line_segment(
            [a, p],
            Stroke::new(
                2.0,
                if invalid.is_some() {
                    ui.visuals().error_fg_color
                } else {
                    ui.visuals().selection.stroke.color
                },
            ),
        );
    }
    // "Connect From Here" waiting for its target: a line from the source port to the pointer.
    if let (Some(from_uid), Some(p)) = (&editor.connect_from, pointer)
        && let Some(n) = graph.nodes.iter().find(|n| &n.uid == from_uid)
    {
        let a = view.to_screen(origin, n.pos) + vec2(TILE.x * view.zoom / 2.0, 0.0);
        painter.line_segment(
            [a, p],
            Stroke::new(2.0, ui.visuals().selection.stroke.color),
        );
    }
    // How to connect, while the pointer rests on a node or a click-to-connect is waiting.
    let hint = if editor.connect_from.is_some() {
        Some("Click the node to connect to (Esc cancels)")
    } else if editor.gesture.is_none() && (hovered.is_some() || port.is_some()) {
        Some(connect_hint(
            ui.ctx().os() == egui::os::OperatingSystem::Mac,
        ))
    } else {
        None
    };
    if let Some(hint) = hint {
        painter.text(
            rect.left_bottom() + vec2(10.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            hint,
            egui::FontId::proportional(12.0),
            ui.visuals().weak_text_color(),
        );
    }
    if let Some(reason) = invalid.or(editor.rejection.as_deref()) {
        if let Some(p) = pointer {
            painter.text(
                p + vec2(12.0, 12.0),
                egui::Align2::LEFT_TOP,
                reason,
                egui::FontId::proportional(13.0),
                ui.visuals().error_fg_color,
            );
        }
        // Also expose the explanation to assistive technology and headless tests.
        ui.label(egui::RichText::new(reason).color(ui.visuals().error_fg_color));
    }
    if released && let Some(gesture) = editor.gesture.take() {
        match (gesture, pointer) {
            (Gesture::Move { start, targets }, Some(p)) => {
                let d = (p - start) / view.zoom;
                let (x, y) = (coordinate(d.x), coordinate(d.y));
                if (x != 0 || y != 0) && !targets.is_empty() {
                    actions.push(Action::Command(
                        "node.move",
                        vec![Arg::Nodes(targets), Arg::Delta(x, y)],
                    ));
                }
            }
            (Gesture::Connect { from }, _) => {
                if let Some(to) = hovered {
                    if let Some(reason) = connection_error(graph, from, to) {
                        editor.rejection = Some(reason.into());
                    } else if let (Some(a), Some(b)) = (graph.nodes.get(from), graph.nodes.get(to))
                    {
                        actions.push(Action::Command(
                            "node.connect",
                            vec![Arg::Link(a.uid.clone(), b.uid.clone())],
                        ));
                    }
                }
            }
            (Gesture::Box { start, mut base }, Some(p)) => {
                let bounds = Rect::from_two_pos(start, p);
                let existing: HashSet<String> = base.iter().cloned().collect();
                base.extend(
                    graph
                        .nodes
                        .iter()
                        .filter(|n| {
                            bounds.contains_rect(Rect::from_center_size(
                                view.to_screen(origin, n.pos),
                                TILE * view.zoom,
                            )) && !existing.contains(&n.uid)
                        })
                        .map(|n| n.uid.clone()),
                );
                actions.push(Action::Command("edit.select", vec![Arg::Nodes(base)]));
            }
            _ => {}
        }
    }
    if inside && ui.input(|i| i.pointer.secondary_pressed()) {
        editor.context_link = hit_link.clone();
        if hit_link.is_some() {
            editor.selected_link = hit_link.clone();
        }
    }
    background.context_menu(|ui| {
        if let Some((from, to)) = &editor.context_link {
            if ui.button("Delete Connection").clicked() {
                actions.push(Action::Command(
                    "node.disconnect",
                    vec![Arg::Link(from.clone(), to.clone())],
                ));
                editor.selected_link = None;
                ui.close();
            }
        } else {
            for (label, id) in [("Paste", "edit.paste"), ("Select All", "edit.select_all")] {
                if ui.button(label).clicked() {
                    actions.push(Action::Command(id, vec![]));
                    ui.close();
                }
            }
        }
    });
    if let Some(rename) = &mut editor.rename {
        if let Some(node) = graph.nodes.iter().find(|n| n.uid == rename.uid) {
            let p = view.to_screen(origin, node.pos) + vec2(-90.0, TILE.y * view.zoom / 2.0 + 4.0);
            let response = ui.put(
                Rect::from_min_size(p, vec2(180.0, 26.0)),
                egui::TextEdit::singleline(&mut rename.text)
                    .id(ui.scope_id().with("rename"))
                    .hint_text("Node name")
                    .char_limit(1000),
            );
            if rename.first_frame {
                response.request_focus();
                rename.first_frame = false;
            }
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
                response.surrender_focus();
                editor.rename = None;
            } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
                actions.push(Action::Command(
                    "node.rename",
                    vec![
                        Arg::Node(rename.uid.clone()),
                        Arg::Text(rename.text.clone()),
                    ],
                ));
                response.surrender_focus();
                editor.rename = None;
            }
        } else {
            editor.rename = None;
        }
    } else if editor.focused
        && !ui.ctx().egui_wants_keyboard_input()
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape))
    {
        editor.gesture = None;
        editor.selected_link = None;
        editor.rejection = None;
    }
    view.sanitize();
    actions
}
