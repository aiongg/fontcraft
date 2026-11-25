//! The Master Compatibility view's paint layer (`view.showMasterCompatibility`, rows `editView.masterCompatibility`,
//! `masterCompatibility.*`). It paints `app::models::compatibility`, projected in the edited layer's font units.
//!
//! | Item | Drawn as |
//! |---|---|
//! | each shape of each layer | a wide stroke in the tint of its index; a component also has a checkered fill |
//! | each path segment | green (matches), yellow (turns more than the warning angle) or red (no match) |
//! | start nodes, component centres | a line across the layers of a group |
//! | anchors | a dashed line across the layers of a group |
//! | the selected nodes | a line to the same nodes of the group's other layers |
//! | the ends of shape lines on the layers beside the edited one | dots; a ring around the one under the pointer |
//! | a component base or anchor name the group disagrees on | written in red at its point |
//! | the edited layer's shapes | their number, counted from 1, beside the start node or centre |
//!
//! A line end dragged onto the end of another shape's line on the same layer swaps the two shapes there
//! (`glyph.swapShapes`); the press goes to the view before the tool ([`pointer`]).
//!
//! It also paints the incompatibility bars (`compatibility.editViewRedBar`, projected by `app::models::incompatibility`
//! whether or not the view is on) and, under the pointer, a bar's reason. A click on a bar edits its glyph and turns the
//! view on.

use craft::ui::runtime::commands::CommandRef;
use egui::{Align2, FontId, Pos2, Rect, Shape, Stroke, Vec2, vec2};
use fontcraft_geom::Point;
use fontcraft_tools::{Key, KeyEvent, PointerEvent, PointerKind};
use serde_json::json;

use super::camera::Camera;
use super::layers::PaintCx;
use super::paint::checker::checker;
use super::paint::{flatten, screen};
use super::tokens::CompatTokens;
use super::tools::tokens::ToolTokens;
use crate::app::models::compatibility::{CompatibilityModel, EndDrag, LinkEnd, LinkKind, SegmentState};
use crate::app::models::incompatibility::Bar;
use crate::app::{FontCraftApp, Msg};

/// A press this module took.
#[derive(Clone, Debug, PartialEq)]
pub enum Press {
    /// On an incompatibility bar.
    Bar(Bar),
    /// On the end of a shape's line.
    End(EndDrag),
}

/// A drop swaps with the shape whose line end lies within this many end radii.
const DROP_RADII: f32 = 3.0;

/// The draggable line end nearest to `at` (screen points) within `reach`, on layer `only` when given.
fn end_at<'a>(model: &'a CompatibilityModel, camera: &Camera, at: Pos2, reach: f32, only: Option<usize>) -> Option<&'a LinkEnd> {
    let ends = model.links.iter().flat_map(|link| link.ends.iter().filter(move |end| model.draggable(link, end)));
    ends.filter(|end| only.is_none_or(|placed| end.placed == placed))
        .map(|end| (camera.to_screen(Point::new(end.at[0], end.at[1])).distance(at), end))
        .filter(|(d, _)| *d <= reach)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, end)| end)
}

/// How near a press must be to a line end.
fn end_reach(t: &CompatTokens) -> f32 {
    t.end_radius.max(ToolTokens::standard().hit_tolerance)
}

/// After the projections: the dragged line end and the difference under the pointer.
pub fn refresh(app: &mut FontCraftApp) {
    if app.models.compatibility.layers.is_empty() {
        app.compat_pointer = None;
        if matches!(app.compat_press, Some(Press::End(_))) {
            app.compat_press = None;
        }
    }
    let reach = f64::from(ToolTokens::standard().hit_tolerance) / super::tools::apply::camera(app).scale();
    let drag = match &app.compat_press {
        Some(Press::End(drag)) => Some(drag),
        _ => None,
    };
    crate::app::models::compatibility::project_pointer(&mut app.models.compatibility, app.compat_pointer, reach, drag);
}

/// Escape ends a line end's drag without a swap.
pub fn key(app: &mut FontCraftApp, ev: &KeyEvent) -> Option<Vec<String>> {
    let ends = ev.pressed && ev.key == Key::Escape && app.compat_press.is_some();
    if ends {
        app.compat_press = None;
    }
    ends.then(Vec::new)
}

/// Where a bar is drawn: above the ascender, the glyph's advance wide. `None` when it does not lie on a screen.
pub fn bar_rect(camera: &Camera, bar: &Bar, t: &CompatTokens) -> Option<Rect> {
    let a = camera.to_screen(Point::new(bar.x0, bar.y));
    let b = camera.to_screen(Point::new(bar.x1, bar.y));
    let mut rect = Rect::from_min_max(Pos2::new(a.x.min(b.x), a.y - t.bar_gap - t.bar_height), Pos2::new(a.x.max(b.x), a.y - t.bar_gap));
    if rect.width() < t.bar_min_width {
        rect = Rect::from_center_size(rect.center(), vec2(t.bar_min_width, rect.height()));
    }
    rect.is_finite().then_some(rect)
}

/// The bar at `at` (screen points), a little room around it included.
fn bar_at<'a>(bars: &'a [Bar], camera: &Camera, t: &CompatTokens, at: Pos2) -> Option<&'a Bar> {
    let reach = t.bar_gap / 2.0;
    bars.iter().find(|bar| bar_rect(camera, bar, t).is_some_and(|rect| rect.expand(reach).contains(at)))
}

/// Paints the bars.
pub fn paint_bars(cx: &PaintCx<'_>) {
    let t = &cx.tokens.compat;
    for bar in &cx.compatibility.bars {
        if let Some(rect) = bar_rect(&cx.camera, bar, t).filter(|r| r.intersects(cx.rect)) {
            cx.painter.rect_filled(rect, 0.0, t.bad);
        }
    }
}

/// Paints the reason of the bar under the resting pointer, as the drag badges are drawn.
pub fn paint_reason(cx: &PaintCx<'_>) {
    if cx.compatibility.bars.is_empty() {
        return;
    }
    let (at, pressed) = cx.painter.ctx().input(|i| (i.pointer.hover_pos(), i.pointer.any_down()));
    let Some(at) = at.filter(|at| !pressed && cx.rect.contains(*at)) else { return };
    let Some(bar) = bar_at(&cx.compatibility.bars, &cx.camera, &cx.tokens.compat, at) else { return };
    let t = ToolTokens::standard();
    let galley = cx.painter.layout(bar.reason.clone(), FontId::proportional(t.badge_text_size), t.badge_text, cx.tokens.compat.reason_width);
    let pad = vec2(t.badge_padding, t.badge_padding);
    let size = galley.size() + pad * 2.0;
    // Below the pointer, moved to stay on the canvas.
    let right = (cx.rect.right() - size.x).max(cx.rect.left());
    let bottom = (cx.rect.bottom() - size.y).max(cx.rect.top());
    let corner = Pos2::new((at.x + t.badge_offset).min(right), (at.y + t.badge_offset).min(bottom));
    cx.painter.rect_filled(Rect::from_min_size(corner, size), t.badge_padding, t.badge_fill);
    cx.painter.galley(corner + pad, galley, t.badge_text);
}

/// What a pointer event does here before the tools see it: a press on a bar or a line end and what follows until the
/// release. Returns the errors; `None` when the event is the tools'.
pub fn pointer(app: &mut FontCraftApp, ev: &PointerEvent) -> Option<Vec<String>> {
    if !app.models.compatibility.layers.is_empty() && ev.pos.x.is_finite() && ev.pos.y.is_finite() {
        app.compat_pointer = Some(ev.pos);
    }
    if app.compat_press.is_some() {
        return match ev.kind {
            PointerKind::Up if matches!(app.compat_press, Some(Press::End(_))) => Some(drop_end(app, ev)),
            PointerKind::Up => app.compat_press.take().map(|press| release(app, press, ev)),
            PointerKind::Down => {
                app.compat_press = None;
                pressed(app, ev)
            }
            PointerKind::Drag | PointerKind::Move | PointerKind::DoubleClick => {
                drag_end(app, ev);
                Some(Vec::new())
            }
        };
    }
    if ev.kind == PointerKind::Down { pressed(app, ev) } else { None }
}

fn pressed(app: &mut FontCraftApp, ev: &PointerEvent) -> Option<Vec<String>> {
    if app.tools.toolbox.busy() || app.session.interaction().is_some() {
        return None;
    }
    let camera = super::tools::apply::camera(app);
    let at = camera.to_screen(ev.pos);
    let (model, t) = (&app.models.compatibility, &app.models.canvas_tokens.compat);
    let press = match end_at(model, &camera, at, end_reach(t), None) {
        Some(&LinkEnd { placed, shape: Some(shape), .. }) => {
            let layer = model.layers.get(placed)?.layer.clone();
            let here = [ev.pos.x, ev.pos.y];
            Press::End(EndDrag { layer, placed, shape, from: here, at: here, moved: false, onto: None })
        }
        _ => Press::Bar(bar_at(&model.bars, &camera, t, at)?.clone()),
    };
    app.compat_press = Some(press);
    Some(Vec::new())
}

/// Moves a dragged line end to the pointer and finds the end it would swap with.
fn drag_end(app: &mut FontCraftApp, ev: &PointerEvent) {
    let camera = super::tools::apply::camera(app);
    let Some(Press::End(drag)) = app.compat_press.as_mut() else { return };
    if !(ev.pos.x.is_finite() && ev.pos.y.is_finite()) {
        return;
    }
    let at = camera.to_screen(ev.pos);
    let from = camera.to_screen(Point::new(drag.from[0], drag.from[1]));
    drag.at = [ev.pos.x, ev.pos.y];
    drag.moved |= at.distance(from) > ToolTokens::standard().drag_threshold;
    let reach = app.models.canvas_tokens.compat.end_radius * DROP_RADII;
    let found = end_at(&app.models.compatibility, &camera, at, reach, Some(drag.placed)).and_then(|end| end.shape);
    drag.onto = found.filter(|shape| *shape != drag.shape);
}

/// A dragged line end released: on another shape's end the two shapes of that layer swap, one undo step.
fn drop_end(app: &mut FontCraftApp, ev: &PointerEvent) -> Vec<String> {
    drag_end(app, ev);
    let Some(Press::End(drag)) = app.compat_press.take() else { return Vec::new() };
    let Some(onto) = drag.onto.filter(|_| drag.moved) else { return Vec::new() };
    let glyph = app.session.font().ok().and_then(|font| super::tools::edited_glyph(font, &app.view)).map(str::to_string);
    let Some(glyph) = glyph else { return Vec::new() };
    let swap = json!({"glyph": glyph, "layer": drag.layer, "a": drag.shape, "b": onto});
    app.apply_msg(Msg::run("glyph.swapShapes", swap)).err().into_iter().collect()
}

fn release(app: &mut FontCraftApp, press: Press, ev: &PointerEvent) -> Vec<String> {
    let Press::Bar(bar) = press else { return Vec::new() };
    let camera = super::tools::apply::camera(app);
    let on_it = bar_rect(&camera, &bar, &app.models.canvas_tokens.compat).is_some_and(|r| r.expand(2.0).contains(camera.to_screen(ev.pos)));
    if !on_it {
        return Vec::new();
    }
    let mut commands = Vec::new();
    if let Some(item) = bar.item.filter(|i| Some(*i) != app.view.text.edited_index()) {
        commands.push(CommandRef::with_params("editView.setCursor", json!({"index": item})));
    }
    commands.push(CommandRef::with_params("view.showMasterCompatibility", json!({"on": true})));
    commands.into_iter().filter_map(|c| app.apply_msg(Msg::Shell(c)).err()).collect()
}

pub fn paint(cx: &PaintCx<'_>) {
    cx.painter.extend(shapes(cx));
}

/// The view's shapes, none while it is off; the labels are painted at once.
pub fn shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let model = &**cx.compatibility;
    if !cx.display.master_compatibility || model.layers.is_empty() {
        return Vec::new();
    }
    let t = &cx.tokens.compat;
    let affine = cx.camera.affine();
    let font = FontId::proportional(t.label_size);
    let mut out = Vec::new();
    for layer in &model.layers {
        for shape in &layer.shapes {
            let stroke = Stroke::new(t.tint_width, t.tint(shape.index, false));
            let contours = flatten(&shape.outline, affine);
            if shape.component
                && let Some(mesh) = checker(&contours, cx.rect, t.checker, t.tint(shape.index, false))
            {
                out.push(Shape::mesh(mesh));
            }
            out.extend(contours.into_iter().map(|contour| Shape::line(contour, stroke)));
            if let Some(at) = shape.label.and_then(|p| screen(cx, p)) {
                let gap = Vec2::new(t.label_gap, -t.label_gap);
                let number = shape.index.saturating_add(1).to_string();
                cx.painter.text(at + gap, Align2::LEFT_BOTTOM, number, font.clone(), t.tint(shape.index, true));
            }
        }
        for segment in &layer.segments {
            let colour = match segment.state {
                SegmentState::Compatible => t.ok,
                SegmentState::Turns => t.warn,
                SegmentState::Incompatible => t.bad,
            };
            out.extend(flatten(&segment.path, affine).into_iter().map(|line| Shape::line(line, Stroke::new(t.segment_width, colour))));
        }
    }
    let line = Stroke::new(t.line_width, t.line);
    for link in model.links.iter().chain(&model.probe) {
        let points: Vec<_> = link.ends.iter().filter_map(|end| screen(cx, end.at)).collect();
        if link.kind == LinkKind::Node {
            out.extend(points.iter().map(|p| Shape::circle_filled(*p, t.end_radius, t.probe)));
        }
        if points.len() >= 2 {
            match link.kind {
                LinkKind::Anchor => out.extend(Shape::dashed_line(&points, line, t.dash, t.dash_gap)),
                LinkKind::Start | LinkKind::Centre => out.push(Shape::line(points, line)),
                LinkKind::Node => out.push(Shape::line(points, Stroke::new(t.line_width, t.probe))),
            }
        }
        let ends = link.ends.iter().filter(|end| model.draggable(link, end)).filter_map(|end| screen(cx, end.at));
        out.extend(ends.map(|p| Shape::circle_filled(p, t.end_radius, t.line)));
        for (at, name) in &link.mismatches {
            if let Some(at) = screen(cx, *at) {
                cx.painter.text(at - Vec2::new(0.0, t.label_gap), Align2::CENTER_BOTTOM, name, font.clone(), t.bad);
            }
        }
    }
    match &model.drag {
        Some(drag) => dragged(cx, model, drag, &mut out),
        None => {
            let pointer = cx.painter.ctx().input(|i| i.pointer.hover_pos()).filter(|at| cx.rect.contains(*at));
            let end = pointer.and_then(|at| end_at(model, &cx.camera, at, end_reach(t), None));
            ring(cx, end, &mut out);
        }
    }
    out
}

/// A ring around a line end.
fn ring(cx: &PaintCx<'_>, end: Option<&LinkEnd>, out: &mut Vec<Shape>) {
    let t = &cx.tokens.compat;
    if let Some(at) = end.and_then(|end| screen(cx, end.at)) {
        out.push(Shape::circle_stroke(at, t.end_ring_radius, Stroke::new(t.segment_width, t.probe)));
    }
}

/// The dragged end: its line's neighbours joined to the pointer, and the end a release would swap with.
fn dragged(cx: &PaintCx<'_>, model: &CompatibilityModel, drag: &EndDrag, out: &mut Vec<Shape>) {
    let t = &cx.tokens.compat;
    let Some(at) = screen(cx, drag.at) else { return };
    let of = |shape: usize| {
        let has = move |end: &LinkEnd| end.placed == drag.placed && end.shape == Some(shape);
        model.links.iter().find_map(|link| link.ends.iter().position(has).map(|i| (link, i)))
    };
    if let Some((link, i)) = of(drag.shape) {
        for k in [i.checked_sub(1), i.checked_add(1)].into_iter().flatten() {
            if let Some(beside) = link.ends.get(k).and_then(|end| screen(cx, end.at)) {
                out.extend(Shape::dashed_line(&[beside, at], Stroke::new(t.segment_width, t.probe), t.dash, t.dash_gap));
            }
        }
    }
    out.push(Shape::circle_filled(at, t.end_radius, t.probe));
    let onto = drag.onto.and_then(of).and_then(|(link, i)| link.ends.get(i));
    ring(cx, onto, out);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use egui::epaint::ColorMode;
    use egui::{LayerId, Painter, Pos2, Rect, Shape};
    use fontcraft_engine::Session;
    use fontcraft_tools::{Modifiers, PointerEvent, PointerKind};
    use serde_json::json;

    use super::super::camera::Camera;
    use super::super::layers::PaintCx;
    use super::super::paint::CanvasWords;
    use crate::app::models::compatibility::SegmentState;
    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
    use crate::canvas::tools::ToolInput;

    const RECT: Rect = Rect { min: Pos2::ZERO, max: Pos2 { x: 1280.0, y: 800.0 } };

    fn strokes(shapes: &[Shape], color: egui::Color32) -> usize {
        shapes.iter().filter(|s| matches!(s, Shape::Path(p) if p.stroke.color == ColorMode::Solid(color))).count()
    }

    /// Two masters of `n`; the second master's square has one node more, so its segments do not match from there.
    #[test]
    fn show_master_compatibility_places_every_master_and_colours_the_segments() {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        let run = |app: &mut FontCraftApp, id: &str, params: serde_json::Value| app.session.execute(id, &params).unwrap();
        run(&mut app, "font.master.add", json!({}));
        run(&mut app, "glyph.newGlyph", json!({"name": "n.test"}));
        let font = app.session.font().unwrap();
        let (m0, m1) = (font.masters[0].id.clone(), font.masters[1].id.clone());
        run(&mut app, "path.addRectangle", json!({"glyph": "n.test", "layer": m0, "x": 100, "y": 0, "width": 200, "height": 500}));
        run(&mut app, "path.addRectangle", json!({"glyph": "n.test", "layer": m1, "x": 100, "y": 0, "width": 300, "height": 500}));
        run(&mut app, "anchor.add", json!({"glyph": "n.test", "layer": m0, "name": "top", "x": 200, "y": 600}));
        app.sent.push(Msg::View(ViewMsg::SelectGlyph("n.test".into())));
        app.drain();
        assert!(app.models.compatibility.layers.is_empty(), "the view starts off");

        app.sent.push(Msg::Shell(CommandRef::with_params("view.showMasterCompatibility", json!({}))));
        app.drain();
        let model = app.models.compatibility.clone();
        assert_eq!(model.layers.len(), 2, "both masters are placed");
        assert!(model.layers[0].current && !model.layers[1].current);
        let states: Vec<SegmentState> = model.layers.iter().flat_map(|l| l.segments.iter().map(|s| s.state)).collect();
        assert_eq!(states.len(), 8, "four segments per square");
        assert!(states.iter().all(|s| *s == SegmentState::Compatible), "two rectangles match: {states:?}");
        assert_eq!(model.links.len(), 2, "one start-node line and one anchor line");
        assert_eq!(model.links[1].mismatches.len(), 1, "the anchor only one master has is named");

        // The labels need the fonts, which a context has after its first pass.
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        let painter = Painter::new(ctx, LayerId::background(), RECT);
        let mut view = app.view.canvas;
        view.rect = RECT;
        let words = CanvasWords::default();
        let overlay_words = crate::canvas::overlays::OverlayWords::default();
        let cx = PaintCx {
            painter: &painter,
            rect: RECT,
            camera: Camera::new(view, app.models.canvas.glyph.as_ref().map_or(0.0, |g| g.ascender)),
            model: &app.models.canvas,
            tokens: &app.models.canvas_tokens,
            display: app.view.display,
            edit_background: false,
            words: &words,
            tools: &app.tools.view,
            text: &app.models.text,
            overlays: crate::canvas::overlays::Painted { data: &app.overlays.data, words: &overlay_words },
            hinting: crate::canvas::hinting::HintPaint { model: &app.models.hint_canvas, view: &app.view.hinting, tokens: &app.models.hint_tokens },
            compatibility: &app.models.compatibility,
        };
        let shapes = super::shapes(&cx);
        assert_eq!(strokes(&shapes, cx.tokens.compat.ok), 8, "every segment green");

        app.sent.push(Msg::Shell(CommandRef::with_params("view.showMasterCompatibility", json!({"on": false}))));
        app.drain();
        assert!(app.models.compatibility.layers.is_empty(), "off again");
    }

    fn pointer(app: &mut FontCraftApp, kind: PointerKind, at: [f64; 2]) {
        let pos = fontcraft_geom::Point::new(at[0], at[1]);
        app.sent.push(Msg::Tool(ToolInput::Pointer(PointerEvent { kind, pos, mods: Modifiers::NONE })));
        app.drain();
        assert!(app.status.is_empty(), "{}", app.status);
    }

    /// The left edges of a layer's paths, in order.
    fn lefts(app: &FontCraftApp, layer: &str) -> Vec<f64> {
        let glyph = app.session.font().unwrap().glyph("n.test").unwrap();
        let shapes = &glyph.layers.iter().find(|l| l.id == layer).unwrap().shapes;
        let left = |s: &fontcraft_engine::model::Shape| match s {
            fontcraft_engine::model::Shape::Path(p) => p.nodes.iter().map(|n| n.x).fold(f64::INFINITY, f64::min),
            _ => f64::NAN,
        };
        shapes.iter().map(left).collect()
    }

    /// Two squares a master, the second master's in the other order: dragging one line end onto the other swaps them
    /// there, in one undo step. A press elsewhere reaches the tool, and the probe follows the selection.
    #[test]
    fn dragging_a_line_end_onto_another_swaps_the_shapes_of_that_layer() {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        let run = |app: &mut FontCraftApp, id: &str, params: serde_json::Value| app.session.execute(id, &params).unwrap();
        run(&mut app, "font.master.add", json!({}));
        run(&mut app, "glyph.newGlyph", json!({"name": "n.test"}));
        let font = app.session.font().unwrap();
        let (m0, m1) = (font.masters[0].id.clone(), font.masters[1].id.clone());
        for (layer, xs) in [(&m0, [0, 400]), (&m1, [400, 0])] {
            for x in xs {
                run(&mut app, "path.addRectangle", json!({"glyph": "n.test", "layer": layer, "x": x, "y": 0, "width": 100, "height": 100}));
            }
        }
        app.sent.push(Msg::View(ViewMsg::SelectGlyph("n.test".into())));
        app.sent.push(Msg::Shell(CommandRef::with_params("view.showMasterCompatibility", json!({"on": true}))));
        app.view.canvas.rect = RECT;
        app.view.canvas.zoom = 0.5;
        app.drain();
        let model = app.models.compatibility.clone();
        assert_eq!(model.links.len(), 2, "a line per shape");
        // The lines cross: the first shape's end lies right of the second's on the other master only.
        let x = |link: usize, end: usize| model.links[link].ends[end].at[0];
        assert!(x(0, 0) < x(1, 0) && x(0, 1) > x(1, 1));

        let (from, to) = (model.links[0].ends[1].at, model.links[1].ends[1].at);
        pointer(&mut app, PointerKind::Down, from);
        pointer(&mut app, PointerKind::Drag, [(from[0] + to[0]) / 2.0, from[1]]);
        assert!(app.models.compatibility.drag.as_ref().is_some_and(|d| d.onto.is_none()), "between the ends nothing would swap");
        pointer(&mut app, PointerKind::Drag, to);
        assert_eq!(app.models.compatibility.drag.as_ref().and_then(|d| d.onto), Some(1));
        pointer(&mut app, PointerKind::Up, to);
        assert_eq!((lefts(&app, &m0), lefts(&app, &m1)), (vec![0.0, 400.0], vec![0.0, 400.0]), "the other master only");
        let x = |app: &FontCraftApp, link: usize, end: usize| app.models.compatibility.links[link].ends[end].at[0];
        assert!(x(&app, 0, 1) < x(&app, 1, 1), "the crossing is gone");
        app.sent.push(Msg::run("edit.undo", json!({})));
        app.drain();
        assert_eq!(lefts(&app, &m1), [400.0, 0.0], "one undo step");

        // A press on the edited layer's own end, and a drop on nothing, swap nothing.
        let own = model.links[0].ends[0].at;
        pointer(&mut app, PointerKind::Down, own);
        assert!(app.compat_press.is_none(), "the edited layer's ends are the tool's");
        pointer(&mut app, PointerKind::Up, own);
        pointer(&mut app, PointerKind::Down, from);
        pointer(&mut app, PointerKind::Up, [from[0], from[1] + 300.0]);
        assert_eq!(lefts(&app, &m1), [400.0, 0.0]);

        app.tools.selection = fontcraft_tools::Selection::new();
        app.tools.selection.points.insert((0, 0));
        app.drain();
        let probe = app.models.compatibility.probe.clone();
        assert_eq!((probe.len(), probe[0].ends.len()), (1, 2), "the selected node's line");
        app.tools.selection = fontcraft_tools::Selection::new();
        app.drain();
        assert!(app.models.compatibility.probe.is_empty());
    }
}
