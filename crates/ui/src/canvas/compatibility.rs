//! The Master Compatibility view's paint layer (`view.showMasterCompatibility`, rows `editView.masterCompatibility`,
//! `masterCompatibility.*`). It paints `app::models::compatibility`, projected in the edited layer's font units.
//!
//! | Item | Drawn as |
//! |---|---|
//! | each shape of each layer | a wide stroke in the tint of its index; dashed for a component |
//! | each path segment | green (matches), yellow (turns more than the warning angle) or red (no match) |
//! | start nodes, component centres | a line across the layers of a group |
//! | anchors | a dashed line across the layers of a group |
//! | a component base or anchor name the group disagrees on | written in red at its point |
//! | the edited layer's shapes | their index beside the start node or centre |
//!
//! It also paints the incompatibility bars (`compatibility.editViewRedBar`, projected by `app::models::incompatibility`
//! whether or not the view is on) and, under the pointer, a bar's reason. A click on a bar edits its glyph and turns the
//! view on ([`pointer`]).

use craft::ui::runtime::commands::CommandRef;
use egui::{Align2, FontId, Pos2, Rect, Shape, Stroke, Vec2, vec2};
use fontcraft_geom::Point;
use fontcraft_tools::{PointerEvent, PointerKind};
use serde_json::json;

use super::camera::Camera;
use super::layers::PaintCx;
use super::paint::{flatten, screen};
use super::tokens::CompatTokens;
use super::tools::tokens::ToolTokens;
use crate::app::models::compatibility::{LinkKind, SegmentState};
use crate::app::models::incompatibility::Bar;
use crate::app::{FontCraftApp, Msg};

/// A press this module took.
#[derive(Clone, Debug, PartialEq)]
pub enum Press {
    /// On an incompatibility bar.
    Bar(Bar),
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

/// The indicators' part of a pointer event, before the tools see it: a press on a bar and what follows until the
/// release. Returns the errors; `None` when the event is the tools'.
pub fn pointer(app: &mut FontCraftApp, ev: &PointerEvent) -> Option<Vec<String>> {
    if app.compat_press.is_some() {
        return match ev.kind {
            PointerKind::Up => app.compat_press.take().map(|press| release(app, press, ev)),
            PointerKind::Down => {
                app.compat_press = None;
                pressed(app, ev)
            }
            PointerKind::Drag | PointerKind::Move | PointerKind::DoubleClick => Some(Vec::new()),
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
    let bar = bar_at(&app.models.compatibility.bars, &camera, &app.models.canvas_tokens.compat, at)?.clone();
    app.compat_press = Some(Press::Bar(bar));
    Some(Vec::new())
}

fn release(app: &mut FontCraftApp, press: Press, ev: &PointerEvent) -> Vec<String> {
    let Press::Bar(bar) = press;
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
            for contour in flatten(&shape.outline, affine) {
                if shape.component {
                    out.extend(Shape::dashed_line(&contour, stroke, t.dash, t.dash_gap));
                } else {
                    out.push(Shape::line(contour, stroke));
                }
            }
            if let Some(at) = shape.label.and_then(|p| screen(cx, p)) {
                let gap = Vec2::new(t.label_gap, -t.label_gap);
                cx.painter.text(at + gap, Align2::LEFT_BOTTOM, shape.index.to_string(), font.clone(), t.tint(shape.index, true));
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
    for link in &model.links {
        let points: Vec<_> = link.points.iter().filter_map(|p| screen(cx, *p)).collect();
        if points.len() >= 2 {
            match link.kind {
                LinkKind::Anchor => out.extend(Shape::dashed_line(&points, line, t.dash, t.dash_gap)),
                LinkKind::Start | LinkKind::Centre => out.push(Shape::line(points, line)),
            }
        }
        for (at, name) in &link.mismatches {
            if let Some(at) = screen(cx, *at) {
                cx.painter.text(at - Vec2::new(0.0, t.label_gap), Align2::CENTER_BOTTOM, name, font.clone(), t.bad);
            }
        }
    }
    out
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
    use serde_json::json;

    use super::super::camera::Camera;
    use super::super::layers::PaintCx;
    use super::super::paint::CanvasWords;
    use crate::app::models::compatibility::SegmentState;
    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};

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
}
