//! The canvas's drawing: what each display state paints, from the projected model through the paint layers.

use std::sync::Arc;

use craft::behavior::CanvasView;
use craft::behavior::kurbo::Point;
use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use egui::epaint::ColorMode;
use egui::{Color32, LayerId, Painter, Pos2, Rect, Shape};
use fontcraft_engine::Session;
use serde_json::{Value, json};

use super::camera::Camera;
use super::layers::PaintCx;
use super::paint::{self, CanvasWords};
use crate::app::models::canvas::{Placeholder, State};
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};

fn app() -> FontCraftApp {
    FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()))
}

fn run(app: &mut FontCraftApp, id: &str, params: Value) {
    app.sent.push(Msg::run(id, params));
    app.drain();
    assert!(app.status.is_empty(), "{id}: {}", app.status);
}

fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
    app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
    app.drain();
    assert!(app.status.is_empty(), "{id}: {}", app.status);
}

fn edit(app: &mut FontCraftApp, glyph: &str) {
    app.sent.push(Msg::View(ViewMsg::SelectGlyph(glyph.into())));
    app.drain();
}

/// The canvas rect the tests paint into.
const RECT: Rect = Rect { min: Pos2::ZERO, max: Pos2 { x: 1280.0, y: 800.0 } };

/// The shapes `layer` paints for `app`'s canvas, the view at the app's canvas view placed in [`RECT`].
fn painted(app: &FontCraftApp, layer: fn(&PaintCx<'_>) -> Vec<Shape>) -> Vec<Shape> {
    let painter = Painter::new(egui::Context::default(), LayerId::background(), RECT);
    let mut view = app.view.canvas;
    view.rect = RECT;
    let top = app.models.canvas.glyph.as_ref().map_or(0.0, |g| g.ascender);
    let words = CanvasWords::default();
    let cx = PaintCx {
        painter: &painter,
        rect: RECT,
        camera: Camera::new(view, top),
        model: &app.models.canvas,
        tokens: &app.models.canvas_tokens,
        display: app.view.display,
        edit_background: app.view.edit_background,
        words: &words,
        tools: &app.tools.view,
        compatibility: &app.models.compatibility,
    };
    layer(&cx)
}

/// How many shapes are drawn in `color`, as fill or stroke.
fn count(shapes: &[Shape], color: Color32) -> usize {
    shapes
        .iter()
        .filter(|s| match s {
            Shape::Path(p) => p.fill == color || p.stroke.color == ColorMode::Solid(color),
            Shape::Rect(r) => r.fill == color || r.stroke.color == color,
            Shape::Circle(c) => c.fill == color || c.stroke.color == color,
            Shape::LineSegment { stroke, .. } => stroke.color == color,
            _ => false,
        })
        .count()
}

fn components_app() -> FontCraftApp {
    let mut app = app();
    for name in ["base", "empty", "comp"] {
        run(&mut app, "glyph.newGlyph", json!({"name": name}));
    }
    run(&mut app, "path.addRectangle", json!({"glyph": "base", "x": 0, "y": 0, "width": 200, "height": 200}));
    run(&mut app, "glyph.addComponent", json!({"glyph": "comp", "component": "base"}));
    run(&mut app, "path.addRectangle", json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200}));
    for x in [400, 700] {
        let shape = app.session.font().unwrap().glyph("A").unwrap().layers[0].shapes.len();
        run(&mut app, "glyph.addComponent", json!({"glyph": "A", "component": "base"}));
        run(&mut app, "component.move", json!({"glyph": "A", "components": [shape], "dx": x, "dy": 0}));
    }
    run(&mut app, "anchor.add", json!({"glyph": "A", "name": "top", "x": 150, "y": 500}));
    run(&mut app, "anchor.add", json!({"glyph": "A", "name": "_bottom", "x": 150, "y": 0}));
    edit(&mut app, "A");
    shell(&mut app, "view.setZoom", json!({"size": 1000, "center": [450, 250]}));
    app
}

#[test]
fn components_and_anchors_draw_by_state() {
    let mut app = components_app();
    let t = app.models.canvas_tokens.clone();
    let states: Vec<State> = app.models.canvas.glyph.as_ref().unwrap().components.iter().map(|c| c.state).collect();
    assert_eq!(states, vec![State::Manual, State::Manual], "moved components are placed by hand");
    assert_eq!(count(&painted(&app, paint::components::shapes), t.component_manual), 2);

    // Anchors: a filled diamond on the x-height, a hollow one on the baseline.
    let anchors = painted(&app, paint::anchors::shapes);
    let diamonds = |hollow: bool| {
        anchors
            .iter()
            .filter(|s| matches!(s, Shape::Path(p) if p.points.len() == 4 && if hollow { p.stroke.color == ColorMode::Solid(t.anchor) } else { p.fill == t.anchor }))
            .count()
    };
    assert_eq!((diamonds(false), diamonds(true)), (1, 1));
    shell(&mut app, "view.showAnchors", json!({"on": false}));
    assert!(painted(&app, paint::anchors::shapes).is_empty(), "View › Show Anchors off hides them");

    // An aligned composite is green; a component of an empty glyph is a crossed box.
    edit(&mut app, "comp");
    assert!(count(&painted(&app, paint::components::shapes), t.component_aligned) >= 1);
    run(&mut app, "glyph.addComponent", json!({"glyph": "comp", "component": "empty"}));
    let marks = &app.models.canvas.glyph.as_ref().unwrap().components;
    assert_eq!(marks.last().map(|m| m.state), Some(State::Placeholder(Placeholder::Empty)));
    assert_eq!(count(&painted(&app, paint::components::shapes), t.component_placeholder), 3, "box and cross");
}

#[test]
fn handle_size_scales_nodes_and_extra_nodes_mark_crossings() {
    let mut app = app();
    run(&mut app, "path.addRectangle", json!({"glyph": "A", "x": 0, "y": 0, "width": 200, "height": 200}));
    run(&mut app, "path.addRectangle", json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200}));
    edit(&mut app, "A");
    let side = |app: &FontCraftApp| {
        painted(app, paint::nodes::shapes)
            .iter()
            .find_map(|s| match s {
                Shape::Rect(r) if r.fill == app.models.canvas_tokens.node_corner => Some(r.rect.width()),
                _ => None,
            })
            .unwrap()
    };
    let regular = side(&app);
    run(&mut app, "settings.set", json!({"id": "appearance.handleSize", "value": "large"}));
    assert!((side(&app) - regular * 1.5).abs() < 1e-3, "a larger handle size paints larger node marks");

    let extra = app.models.canvas_tokens.extra_node;
    assert_eq!(app.models.canvas.glyph.as_ref().unwrap().extra_nodes.len(), 2, "two squares cross twice");
    assert_eq!(count(&painted(&app, paint::nodes::shapes), extra), 0, "Extra Nodes start hidden");
    shell(&mut app, "view.showExtraNodes", json!({}));
    assert_eq!(count(&painted(&app, paint::nodes::shapes), extra), 2);

    // Two overlapping rectangles, one inside the other's span on both axes, cross four times.
    run(&mut app, "glyph.newGlyph", json!({"name": "plus"}));
    run(&mut app, "path.addRectangle", json!({"glyph": "plus", "x": 0, "y": 100, "width": 300, "height": 100}));
    run(&mut app, "path.addRectangle", json!({"glyph": "plus", "x": 100, "y": 0, "width": 100, "height": 300}));
    edit(&mut app, "plus");
    assert_eq!(count(&painted(&app, paint::nodes::shapes), extra), 4);
}

#[test]
fn editing_the_background_tints_the_canvas_and_shows_the_foreground_behind() {
    let mut app = app();
    run(&mut app, "path.addRectangle", json!({"glyph": "A", "x": 0, "y": 0, "width": 200, "height": 200}));
    edit(&mut app, "A");
    let t = app.models.canvas_tokens.clone();
    let background_outlines = |app: &FontCraftApp| count(&painted(app, paint::background::shapes), t.background_outline);
    assert_eq!(background_outlines(&app), 0, "an empty background draws nothing");

    shell(&mut app, "path.editBackground", json!({}));
    let glyph = app.models.canvas.glyph.as_ref().unwrap();
    assert!(glyph.layer.shapes.is_empty(), "the edited side is the empty background");
    assert_eq!(background_outlines(&app), 1, "the foreground is the outline behind");
    shell(&mut app, "view.showBackground", json!({"on": false}));
    assert_eq!(background_outlines(&app), 0, "Show Background hides it");
    shell(&mut app, "view.showBackground", json!({"on": true}));

    shell(&mut app, "path.editBackground", json!({"on": false}));
    assert_eq!(app.models.canvas.glyph.as_ref().unwrap().layer.shapes.len(), 1);
    app.sent.push(Msg::Shell(CommandRef::with_params("path.editBackground", json!({"on": 1}))));
    app.drain();
    assert!(app.status.contains("`on` must be true or false"), "{}", app.status);
}

#[test]
fn zoom_commands_place_font_points_on_screen() {
    let mut app = app();
    edit(&mut app, "A");
    shell(&mut app, "view.setZoom", json!({"size": 2000, "center": [250, 300]}));
    let top = app.models.canvas.glyph.as_ref().unwrap().ascender;
    let mut view: CanvasView = app.view.canvas;
    view.rect = RECT;
    let camera = Camera::new(view, top);
    assert_eq!(camera.scale(), 2.0);
    let centre = camera.to_screen(Point::new(250.0, 300.0));
    assert!((centre - RECT.center()).length() < 1e-3, "the centre font point is at the canvas centre: {centre:?}");
    shell(&mut app, "view.scrollTo", json!({"dx": 20, "dy": 10}));
    let mut view = app.view.canvas;
    view.rect = RECT;
    let moved = Camera::new(view, top).to_screen(Point::new(250.0, 300.0));
    assert!((moved - centre - egui::vec2(20.0, 10.0)).length() < 1e-3, "content moves right and down: {moved:?}");
    shell(&mut app, "view.zoomIn", json!({}));
    assert!((app.view.canvas.zoom - 2.0 * std::f64::consts::SQRT_2).abs() < 1e-9);
    app.sent.push(Msg::Shell(CommandRef::with_params("view.setZoom", json!({"size": 0}))));
    app.drain();
    assert!(app.status.contains("more than 0"), "{}", app.status);
}

#[test]
fn a_scoped_metric_hides_its_line_in_glyphs_outside_the_scope() {
    let mut app = app();
    edit(&mut app, "A");
    let lines = |app: &FontCraftApp| app.models.canvas.glyph.as_ref().unwrap().lines.clone();
    let plain = lines(&app).len();
    assert!(plain >= 5, "the default metric lines: {plain}");
    run(&mut app, "font.metrics.add", json!({"category": "x-height", "name": "Small x", "scope": "name == 'B'"}));
    let master = app.session.font().unwrap().masters[0].id.clone();
    run(&mut app, "font.master.setMetric", json!({"masters": [master], "metric": "Small x", "position": 420}));
    assert_eq!(lines(&app).len(), plain, "a metric scoped to B shows no line in A");
    run(&mut app, "font.metrics.set", json!({"metric": "Small x", "scope": "name == 'A'"}));
    assert!(lines(&app).iter().any(|(_, y)| *y == 420.0), "a metric scoped to A shows its line in A");
}
