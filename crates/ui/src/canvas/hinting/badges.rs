//! The TrueType Instructor on the canvas: the nodes as gray discs, the instruction badges in both directions (the other
//! direction dimmed), the point numbers, and what the pointer and Delete do to badges.
//!
//! Rows: `hinting.tt.instructions.badgeColours`, `hinting.tt.tool.otherDirectionDimmed`,
//! `hinting.tt.tool.masterOutline`, `hint.tt.showPointIndexes`, `hint.tt.selectInstructions`,
//! `hint.tt.moveInstruction`, `hint.tt.deleteInstructions`.
//!
//! Badges sit beside the point an instruction moves (Snap, Delta, Interpolate, with a leader line to it; Interpolate
//! also has lines to its two references) or halfway along the arrow from origin to target (Stem, Shift). Badges on
//! one spot stack away from the point.
//!
//! The Instructor's pointer input reaches the app as tool input (`Msg::Tool`); [`pointer`] takes it first. A press on
//! a badge of the current direction belongs to the badge: a click runs `hint.tt.selectInstructions` (Shift toggles),
//! a drag dropped on a node or handle runs `hint.tt.moveInstruction` on it. Every other press goes on to the tool.
//! Delete or Backspace with instructions selected runs `hint.tt.deleteInstructions` ([`key`]).

use egui::{Align2, Color32, FontId, Pos2, Shape, Stroke, Vec2};
use fontcraft_engine::hinting::tt_instructions;
use fontcraft_geom::Point;
use fontcraft_tools::tt_instructor::TtDirection;
use fontcraft_tools::{Key, KeyEvent, PointerEvent, PointerKind};
use serde_json::json;

use super::tokens::HintTokens;
use super::{BadgeDrag, INSTRUCTOR, SelectedInstructions};
use crate::app::models::hint_canvas::{BadgeKind, TtCanvas, TtMark};
use crate::app::{FontCraftApp, Msg};
use crate::canvas::camera::Camera;
use crate::canvas::layers::PaintCx;
use crate::canvas::tools::tokens::ToolTokens;

/// One instruction as drawn, in screen points.
#[derive(Clone, Debug, PartialEq)]
pub struct Badge {
    /// The instruction's position in the layer's instruction list.
    pub index: usize,
    pub kind: BadgeKind,
    /// The instruction has the current direction: drawn in colour and selectable.
    pub current: bool,
    /// The badge's centre.
    pub at: Pos2,
    /// The point the instruction moves.
    pub target: Pos2,
    /// Its reference points.
    pub references: Vec<Pos2>,
}

/// The colour of a badge of `kind`.
pub fn colour(kind: BadgeKind, t: &HintTokens) -> Color32 {
    match kind {
        BadgeKind::Snap => t.tt_snap,
        BadgeKind::Stem => t.tt_stem,
        BadgeKind::TripleStem => t.tt_triple,
        BadgeKind::Shift => t.tt_shift,
        BadgeKind::Interpolate => t.tt_interpolate,
        BadgeKind::Delta => t.tt_delta,
    }
}

/// Where the badges of `marks` go through `camera`.
pub fn badges(marks: &[TtMark], camera: &Camera, t: &HintTokens, current: TtDirection) -> Vec<Badge> {
    let screen = |[x, y]: [f64; 2]| camera.to_screen(Point::new(x, y));
    let mut stacked: Vec<((i64, i64, bool), usize)> = Vec::new();
    let mut out = Vec::with_capacity(marks.len());
    for m in marks {
        let target = screen(m.target);
        let references: Vec<Pos2> = m.references.iter().map(|&p| screen(p)).collect();
        // Away from the point across the direction the instruction moves it.
        let side = if m.horizontal { Vec2::new(0.0, 1.0) } else { Vec2::new(-1.0, 0.0) };
        let anchor = match (m.kind, references.first()) {
            (BadgeKind::Stem | BadgeKind::TripleStem | BadgeKind::Shift, Some(origin)) => origin.lerp(target, 0.5),
            _ => target + side * t.tt_badge_offset,
        };
        if !anchor.x.is_finite() || !anchor.y.is_finite() || !target.x.is_finite() || !target.y.is_finite() {
            continue;
        }
        let spot = (anchor.x.round() as i64, anchor.y.round() as i64, m.horizontal);
        let slot = match stacked.iter_mut().find(|(s, _)| *s == spot) {
            Some((_, n)) => {
                *n += 1;
                *n - 1
            }
            None => {
                stacked.push((spot, 1));
                0
            }
        };
        let at = anchor + side * (t.tt_badge_radius * 2.0 + t.tt_selected_gap) * slot as f32;
        let current = m.horizontal == (current == TtDirection::Horizontal);
        out.push(Badge { index: m.index, kind: m.kind, current, at, target, references });
    }
    out
}

/// The Instructor's picture, while it is active on the foreground.
fn instructor<'a>(cx: &'a PaintCx<'_>) -> Option<&'a TtCanvas> {
    cx.hinting.model.tt.as_ref().filter(|tt| tt.instructor && !cx.edit_background)
}

/// The on-curve nodes as gray discs, the selected ones in the selection colour (`hinting.tt.tool.masterOutline`).
pub fn nodes(cx: &PaintCx<'_>) {
    cx.painter.extend(node_shapes(cx));
}

/// The shapes of [`nodes`].
pub fn node_shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let Some(tt) = instructor(cx) else { return Vec::new() };
    let t = cx.hinting.tokens;
    let selected = &cx.tools.selection.points;
    tt.nodes
        .iter()
        .filter_map(|n| {
            let at = cx.camera.to_screen(Point::new(n.at[0], n.at[1]));
            let fill = if selected.contains(&(n.shape, n.node)) { t.tt_selected } else { t.tt_node };
            (at.x.is_finite() && at.y.is_finite()).then(|| Shape::circle_filled(at, t.tt_node_radius, fill))
        })
        .collect()
}

/// The instruction badges, the other direction behind, and the badge under a drag.
pub fn badge_layer(cx: &PaintCx<'_>) {
    cx.painter.extend(badge_shapes(cx));
}

/// The shapes of [`badge_layer`].
pub fn badge_shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let Some(tt) = instructor(cx) else { return Vec::new() };
    let t = cx.hinting.tokens;
    let state = &cx.hinting.view.instructor;
    let selected = state.selected_in(&tt.glyph, &tt.layer);
    let marks = badges(&tt.instructions, &cx.camera, t, state.direction);
    let mut out = Vec::new();
    for b in marks.iter().filter(|b| !b.current) {
        paint(&mut out, t, b, false);
    }
    for b in marks.iter().filter(|b| b.current) {
        paint(&mut out, t, b, selected.contains(&b.index));
    }
    if let Some(drag) = cx.hinting.view.drag.filter(|d| d.moved)
        && let Some(b) = marks.iter().find(|b| b.index == drag.index)
    {
        // The badge follows the pointer, tied to the point it would act on after the drop.
        let to = drop_point(tt, &cx.camera, drag.at).map_or(drag.at, |(_, p)| p);
        let c = colour(b.kind, t);
        out.push(Shape::line_segment([b.target, to], Stroke::new(t.tt_line_width, c)));
        out.push(Shape::circle_filled(drag.at, t.tt_badge_radius, c));
        out.push(Shape::circle_stroke(drag.at, t.tt_badge_radius, Stroke::new(t.tt_rim_width, t.tt_selected)));
    }
    out
}

/// One badge: its lines, the badge, and the ring when it is selected.
fn paint(out: &mut Vec<Shape>, t: &HintTokens, b: &Badge, selected: bool) {
    let colour = if b.current { colour(b.kind, t) } else { t.tt_dimmed };
    let line = Stroke::new(t.tt_line_width, colour);
    match b.kind {
        BadgeKind::Stem | BadgeKind::TripleStem | BadgeKind::Shift => match b.references.first() {
            Some(&origin) => {
                out.push(Shape::line_segment([origin, b.target], line));
                arrowhead(out, t, origin, b.target, colour);
            }
            None => out.push(Shape::line_segment([b.target, b.at], line)),
        },
        BadgeKind::Interpolate => {
            for &r in &b.references {
                out.push(Shape::line_segment([r, b.target], Stroke::new(t.tt_line_width * 0.5, colour)));
            }
            out.push(Shape::line_segment([b.target, b.at], line));
        }
        BadgeKind::Snap | BadgeKind::Delta => out.push(Shape::line_segment([b.target, b.at], line)),
    }
    out.push(Shape::circle_filled(b.at, t.tt_badge_radius, colour));
    out.push(Shape::circle_stroke(b.at, t.tt_badge_radius, Stroke::new(t.tt_rim_width, t.tt_rim)));
    if selected {
        out.push(Shape::circle_stroke(b.at, t.tt_badge_radius + t.tt_selected_gap, Stroke::new(t.tt_line_width, t.tt_selected)));
    }
}

/// A filled arrowhead at `to`, pointing away from `from`.
fn arrowhead(out: &mut Vec<Shape>, t: &HintTokens, from: Pos2, to: Pos2, colour: Color32) {
    let size = t.tt_arrow_size;
    let d = to - from;
    let len = d.length();
    if !len.is_finite() || len <= size {
        return;
    }
    let dir = d / len;
    let normal = Vec2::new(-dir.y, dir.x);
    let base = to - dir * size;
    out.push(Shape::convex_polygon(vec![to, base + normal * size * 0.5, base - normal * size * 0.5], colour, Stroke::NONE));
}

/// Each on-curve node's point number beside it (`hint.tt.showPointIndexes`).
pub fn numbers(cx: &PaintCx<'_>) {
    let Some(tt) = cx.hinting.model.tt.as_ref().filter(|_| !cx.edit_background) else { return };
    let t = cx.hinting.tokens;
    for (at, number) in number_labels(cx, tt) {
        cx.painter.text(at, Align2::LEFT_BOTTOM, number.to_string(), FontId::proportional(t.tt_index_size), t.tt_index_text);
    }
}

/// Where each point number is drawn, inside the canvas.
pub fn number_labels(cx: &PaintCx<'_>, tt: &TtCanvas) -> Vec<(Pos2, u16)> {
    let t = cx.hinting.tokens;
    tt.numbers
        .iter()
        .map(|&([x, y], n)| {
            let p = cx.camera.to_screen(Point::new(x, y));
            (Pos2::new(p.x + t.tt_index_gap, p.y - t.tt_index_gap), n)
        })
        .filter(|(at, _)| cx.rect.contains(*at))
        .collect()
}

/// The node or handle nearest to the screen point `at` within the hit tolerance, as `[path, node]`, and where it is.
fn drop_point(tt: &TtCanvas, camera: &Camera, at: Pos2) -> Option<((usize, usize), Pos2)> {
    let tolerance = ToolTokens::standard().hit_tolerance;
    let mut best: Option<(f32, (usize, usize), Pos2)> = None;
    for &(node, [x, y]) in &tt.drop_points {
        let screen = camera.to_screen(Point::new(x, y));
        let d = screen.distance(at);
        if d.is_finite() && d <= tolerance && best.as_ref().is_none_or(|(b, _, _)| d < *b) {
            best = Some((d, node, screen));
        }
    }
    best.map(|(_, node, screen)| (node, screen))
}

/// The Instructor's picture while the Instructor is the chosen tool, on the foreground, with no gesture running.
fn active(app: &FontCraftApp) -> Option<&TtCanvas> {
    if app.view.tool != INSTRUCTOR || app.view.edit_background {
        return None;
    }
    app.models.hint_canvas.tt.as_ref().filter(|tt| tt.instructor)
}

/// Takes the Instructor's pointer input that belongs to a badge. `None` when the event goes on to the tool.
pub fn pointer(app: &mut FontCraftApp, ev: &PointerEvent) -> Option<Vec<String>> {
    let camera = crate::canvas::tools::apply::camera(app);
    let at = camera.to_screen(ev.pos);
    if let Some(mut drag) = app.view.hinting.drag {
        match ev.kind {
            PointerKind::Drag | PointerKind::Move => {
                if drag.from.distance(at) > ToolTokens::standard().drag_threshold {
                    drag.moved = true;
                }
                drag.at = at;
                app.view.hinting.drag = Some(drag);
                return Some(Vec::new());
            }
            PointerKind::Up => {
                app.view.hinting.drag = None;
                return Some(release(app, drag, &camera, at));
            }
            PointerKind::DoubleClick => return Some(Vec::new()),
            PointerKind::Down => app.view.hinting.drag = None,
        }
    }
    if ev.kind != PointerKind::Down || app.tools.toolbox.busy() {
        return None;
    }
    let index = badge_hit(app, &camera, at)?;
    app.view.hinting.drag = Some(BadgeDrag { index, from: at, at, shift: ev.mods.shift, moved: false });
    Some(Vec::new())
}

/// The instruction whose badge of the current direction lies under `at` (screen points), the topmost one, while the
/// Instructor is active.
fn badge_hit(app: &FontCraftApp, camera: &Camera, at: Pos2) -> Option<usize> {
    let tt = active(app)?;
    let t = HintTokens::standard();
    let direction = app.view.hinting.instructor.direction;
    badges(&tt.instructions, camera, &t, direction)
        .into_iter()
        .rev()
        .find(|b| b.current && b.at.distance(at) <= t.tt_badge_radius + ToolTokens::standard().hit_tolerance * 0.5)
        .map(|b| b.index)
}

/// The instruction whose badge a click at `at` (screen points) lands on, as a press on it would take it: the context
/// menu's hit test.
pub fn badge_at(app: &FontCraftApp, at: Pos2) -> Option<usize> {
    badge_hit(app, &crate::canvas::tools::apply::camera(app), at)
}

/// The Instructor is the active tool and its badges show.
pub fn instructor_shows(app: &FontCraftApp) -> bool {
    active(app).is_some()
}

/// The end of a press on a badge: a click selects, a drop on a node or handle moves the instruction there.
fn release(app: &mut FontCraftApp, drag: BadgeDrag, camera: &Camera, at: Pos2) -> Vec<String> {
    let Some(tt) = active(app) else { return Vec::new() };
    let msg = if drag.moved {
        let Some((node, _)) = drop_point(tt, camera, at) else { return Vec::new() };
        // A badge dropped on its own point stays: there is nothing to move.
        if tt.instructions.iter().find(|m| m.index == drag.index).is_some_and(|m| m.target_node == Some(node)) {
            return Vec::new();
        }
        Msg::run("hint.tt.moveInstruction", json!({"glyph": tt.glyph, "layer": tt.layer, "instruction": drag.index, "point": [node.0, node.1]}))
    } else {
        let mode = if drag.shift { "toggle" } else { "replace" };
        Msg::Shell(craft::ui::runtime::commands::CommandRef::with_params(
            "hint.tt.selectInstructions",
            json!({"instructions": [drag.index], "mode": mode}),
        ))
    };
    app.apply_msg(msg).err().into_iter().collect()
}

/// Delete or Backspace with no modifier, while the Instructor is active and instructions of the edited layer are
/// selected: `hint.tt.deleteInstructions` on them, then no instruction is selected, since the positions after them
/// change. `None` for any other key, so it goes on to the tool.
pub fn key(app: &mut FontCraftApp, ev: &KeyEvent) -> Option<Vec<String>> {
    let m = ev.mods;
    if !ev.pressed || !matches!(ev.key, Key::Delete | Key::Backspace) || m.shift || m.alt || m.cmd || m.ctrl || app.tools.toolbox.busy() {
        return None;
    }
    if app.view.tool != INSTRUCTOR || app.view.edit_background {
        return None;
    }
    let target = crate::canvas::tools::target(&app.session, &app.view)?;
    let count = {
        let font = app.session.font().ok()?;
        let layer = font.glyph(&target.glyph)?.layers.iter().find(|l| l.id == target.layer)?;
        tt_instructions(layer).map(|l| l.len()).unwrap_or(0)
    };
    let selected: Vec<usize> = app.view.hinting.instructor.selected_in(&target.glyph, &target.layer).into_iter().filter(|&i| i < count).collect();
    if selected.is_empty() {
        return None;
    }
    let params = json!({"glyph": target.glyph, "layer": target.layer, "instructions": selected});
    let errors = match app.apply_msg(Msg::run("hint.tt.deleteInstructions", params)) {
        Ok(()) => {
            app.view.hinting.instructor.selected = SelectedInstructions::default();
            app.view_changed();
            Vec::new()
        }
        Err(e) => vec![e],
    };
    Some(errors)
}
