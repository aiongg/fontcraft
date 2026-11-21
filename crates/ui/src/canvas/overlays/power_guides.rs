//! Power guides, guide thickness and markers, drawn over the guides while guides show (layer `powerGuides`).
//!
//! | Item | Drawn as |
//! |---|---|
//! | a guide with a thickness (`guide.setThickness`) | its second edge, a thinner line at that distance, and a faint dashed centre line |
//! | markers (`guide.addMarkers`) | short ticks across the guide, `guide_marker_tick` each side |
//! | a power guide (`guide.makePowerGuide`) | a rhombus at its knob, and a faint dashed line from each node it holds to the guide |
//!
//! The data is the guide's (`fontcraft_model::guide_data`). Colours are the guide's own where it has one, else
//! `guide_local` or `guide_global`. A global guide's links to the edited glyph show while its master layer's
//! foreground is edited; links to other glyphs never show here.

use egui::{Color32, Pos2, Shape, Stroke};
use fontcraft_engine::model::guide_data::{self, Link};
use fontcraft_engine::model::{Guide, Layer, Shape as ModelShape};
use fontcraft_geom::{Point, Vec2};

use super::Drawn;
use super::guides::{clip, parse_color};
use super::tokens::OverlayTokens;
use crate::canvas::layers::PaintCx;

/// Most link lines drawn for one guide.
const MAX_DRAWN_LINKS: usize = 2_000;

fn usable(p: Pos2) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// The guide's unit direction, in font units.
fn direction(g: &Guide) -> Vec2 {
    let a = if g.angle.is_finite() { g.angle.to_radians() } else { 0.0 };
    Vec2::new(a.cos(), a.sin())
}

/// The part inside the canvas of the line through `at` (font units) along `dir`.
fn line_through(cx: &PaintCx<'_>, at: Point, dir: Vec2) -> Option<[Pos2; 2]> {
    let a = cx.camera.to_screen(at);
    let b = cx.camera.to_screen(at + dir);
    let d = (b - a).normalized();
    if !(usable(a) && d.x.is_finite() && d.y.is_finite()) {
        return None;
    }
    let (p, q) = clip(a, d, cx.rect)?;
    Some([p, q])
}

pub fn power_guides(cx: &PaintCx<'_>) {
    power_guide_shapes(cx).paint(cx);
}

/// The thickness edges, markers and power guide marks of the edited layer's guides and its master's global guides.
pub fn power_guide_shapes(cx: &PaintCx<'_>) -> Drawn {
    let mut out = Drawn::default();
    let Some(glyph) = cx.glyph() else { return out };
    if !cx.display.guides {
        return out;
    }
    let t = OverlayTokens::standard();
    let layer = &glyph.layer;
    for g in &layer.info.guides {
        let color = g.color.as_deref().and_then(parse_color).unwrap_or(t.guide_local);
        guide(cx, &t, layer, g, color, |l| l.glyph.is_none(), &mut out);
    }
    let data = cx.overlays.data;
    let own = !cx.edit_background && layer.is_master_layer();
    for (i, g) in data.global_guides.iter().enumerate() {
        if data.out_of_scope.contains(&i) {
            continue;
        }
        let color = g.color.as_deref().and_then(parse_color).unwrap_or(t.guide_global);
        guide(cx, &t, layer, g, color, |l| own && l.glyph.as_deref() == Some(glyph.name.as_str()), &mut out);
    }
    out
}

fn guide(cx: &PaintCx<'_>, t: &OverlayTokens, layer: &Layer, g: &Guide, color: Color32, mine: impl Fn(&Link) -> bool, out: &mut Drawn) {
    let knob = Point::new(g.x, g.y);
    if !(knob.x.is_finite() && knob.y.is_finite()) {
        return;
    }
    let dir = direction(g);
    let normal = Vec2::new(-dir.y, dir.x);
    let faint = color.gamma_multiply(0.5);
    if let Some(thickness) = guide_data::thickness(g)
        && let Some(edge) = line_through(cx, knob + normal * thickness, dir)
    {
        out.shapes.push(Shape::line_segment(edge, Stroke::new(t.guide_width * 0.5, color)));
        if let Some(centre) = line_through(cx, knob + normal * (thickness / 2.0), dir) {
            out.shapes.extend(Shape::dashed_line(&centre, Stroke::new(t.guide_width * 0.5, faint), t.guide_link_dash, t.guide_link_gap));
        }
    }
    for d in guide_data::markers(g) {
        let at = cx.camera.to_screen(knob + dir * d);
        let tip = cx.camera.to_screen(knob + dir * d + normal);
        let across = (tip - at).normalized();
        if !(usable(at) && across.x.is_finite() && across.y.is_finite()) {
            continue;
        }
        let tick = across * t.guide_marker_tick;
        out.shapes.push(Shape::line_segment([at - tick, at + tick], Stroke::new(t.guide_width, color)));
    }
    let links = guide_data::links(g);
    if links.is_empty() {
        return;
    }
    let k = cx.camera.to_screen(knob);
    if usable(k) {
        let r = t.guide_knob_radius * 1.4;
        let rhombus = vec![Pos2::new(k.x, k.y - r), Pos2::new(k.x + r, k.y), Pos2::new(k.x, k.y + r), Pos2::new(k.x - r, k.y)];
        out.shapes.push(Shape::convex_polygon(rhombus, color, Stroke::NONE));
    }
    for link in links.iter().filter(|l| mine(l)).take(MAX_DRAWN_LINKS) {
        let Some(ModelShape::Path(path)) = layer.shapes.get(link.shape) else { continue };
        let Some(node) = path.nodes.get(link.node) else { continue };
        let p = node.point();
        let foot = knob + dir * (p - knob).dot(dir);
        let (a, b) = (cx.camera.to_screen(p), cx.camera.to_screen(foot));
        if usable(a) && usable(b) {
            out.shapes.extend(Shape::dashed_line(&[a, b], Stroke::new(t.guide_width, faint), t.guide_link_dash, t.guide_link_gap));
            out.shapes.push(Shape::circle_filled(a, t.guide_knob_radius * 0.6, faint));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use std::sync::Arc;

    use craft::behavior::CanvasView;
    use craft::behavior::kurbo::Point as KPoint;
    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use egui::{LayerId, Painter, Pos2, Rect, Shape};
    use fontcraft_engine::Session;
    use fontcraft_geom::Point;
    use serde_json::{Value, json};

    use super::super::tokens::OverlayTokens;
    use super::super::{OverlayWords, Painted};
    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
    use crate::canvas::camera::Camera;
    use crate::canvas::layers::PaintCx;
    use crate::canvas::paint::CanvasWords;

    const RECT: Rect = Rect { min: Pos2::ZERO, max: Pos2 { x: 1000.0, y: 800.0 } };

    fn run(app: &mut FontCraftApp, id: &str, params: Value) {
        let r = app.run_now(Msg::run(id, params), false);
        assert!(r.is_ok(), "{id}: {r:?}");
    }

    /// Glyph A with a 200-unit square from (100, 100) and a level guide along its top edge, shown at one point per
    /// unit.
    fn app() -> FontCraftApp {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
        app.drain();
        run(&mut app, "path.addRectangle", json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200}));
        run(&mut app, "guide.add", json!({"glyph": "A", "x": 0, "y": 300, "angle": 0}));
        app.view.canvas = CanvasView::new(RECT, KPoint::new(400.0, 400.0), 1.0);
        app.drain();
        app
    }

    fn camera(app: &FontCraftApp) -> Camera {
        let mut view = app.view.canvas;
        view.rect = RECT;
        Camera::new(view, app.models.canvas.glyph.as_ref().map_or(0.0, |g| g.ascender))
    }

    fn drawn(app: &FontCraftApp) -> Vec<Shape> {
        let painter = Painter::new(egui::Context::default(), LayerId::background(), RECT);
        let (words, overlay_words) = (CanvasWords::default(), OverlayWords::default());
        let cx = PaintCx {
            painter: &painter,
            rect: RECT,
            camera: camera(app),
            model: &app.models.canvas,
            tokens: &app.models.canvas_tokens,
            display: app.view.display,
            edit_background: app.view.edit_background,
            words: &words,
            tools: &app.tools.view,
            text: &app.models.text,
            overlays: Painted { data: &app.overlays.data, words: &overlay_words },
            hinting: crate::canvas::hinting::HintPaint { model: &app.models.hint_canvas, view: &app.view.hinting, tokens: &app.models.hint_tokens },
            compatibility: &app.models.compatibility,
        };
        super::power_guide_shapes(&cx).shapes
    }

    /// Solid lines in the local guide colour whose ends both lie on `on` (a test of a screen point).
    fn solid(shapes: &[Shape], on: impl Fn(Pos2) -> bool) -> usize {
        let color = OverlayTokens::standard().guide_local;
        shapes.iter().filter(|s| matches!(s, Shape::LineSegment { points, stroke } if stroke.color == color && points.iter().all(|p| on(*p)))).count()
    }

    /// A guide's thickness draws its second edge, its markers draw ticks across it, and a power guide draws its rhombus
    /// and a dot on each node it holds; Show Guides off hides them all.
    #[test]
    fn guide_thickness_markers_and_power_guides_are_drawn() {
        let mut app = app();
        assert!(drawn(&app).is_empty(), "a plain guide draws nothing here");
        let guide = json!({"scope": "local", "index": 0});
        let cam = camera(&app);
        let near = |a: f32, b: f32| (a - b).abs() < 0.01;

        run(&mut app, "guide.setThickness", json!({"glyph": "A", "guide": guide, "thickness": 40}));
        let edge = cam.to_screen(Point::new(0.0, 340.0)).y;
        assert_eq!(solid(&drawn(&app), |p| near(p.y, edge)), 1, "the second edge, 40 units above the guide");

        run(&mut app, "guide.addMarkers", json!({"glyph": "A", "guide": guide, "positions": [50, 150]}));
        let shapes = drawn(&app);
        for d in [50.0, 150.0] {
            let x = cam.to_screen(Point::new(d, 300.0)).x;
            assert_eq!(solid(&shapes, |p| near(p.x, x)), 1, "a tick across the guide at {d}");
        }

        run(&mut app, "guide.makePowerGuide", json!({"glyph": "A", "guides": [guide]}));
        let shapes = drawn(&app);
        assert_eq!(shapes.iter().filter(|s| matches!(s, Shape::Path(_))).count(), 1, "the rhombus at the knob");
        assert_eq!(shapes.iter().filter(|s| matches!(s, Shape::Circle(_))).count(), 2, "a dot on each node of the top edge");

        app.sent.push(Msg::Shell(CommandRef::with_params("view.showGuides", json!({"on": false}))));
        app.drain();
        assert!(drawn(&app).is_empty(), "Show Guides off hides them");
    }
}
