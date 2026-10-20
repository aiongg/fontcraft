//! Edit View display aids: each switch draws its aid and only then, Lock Outline refuses pointer
//! edits of the outline but not commands, and a drag shows the outline as it was.

use egui::{Color32, Pos2, Shape};
use serde_json::{Value, json};

use crate::{FontcraftApp, control, tools_glue};

/// One frame of a headless window; the shapes it drew, nested lists flattened.
fn frame(ctx: &egui::Context, app: &mut FontcraftApp) -> Vec<Shape> {
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut output = ctx.run_ui(input, |ui| app.ui(ui));
    output.textures_delta.clear();
    let mut out: Vec<Shape> = output.shapes.into_iter().map(|c| c.shape).collect();
    let mut flat = Vec::new();
    while let Some(shape) = out.pop() {
        match shape {
            Shape::Vec(list) => out.extend(list),
            other => flat.push(other),
        }
    }
    flat
}

/// The shapes of a settled frame.
fn settled(ctx: &egui::Context, app: &mut FontcraftApp) -> Vec<Shape> {
    frame(ctx, app);
    frame(ctx, app)
}

/// The texts drawn in `color`, sorted.
fn texts(shapes: &[Shape], color: Color32) -> Vec<String> {
    let mut out: Vec<String> = shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Text(t) if t.fallback_color == color => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect();
    out.sort();
    out
}

/// The font point `(x, y)` of the edited glyph on screen.
fn screen(app: &FontcraftApp, x: f64, y: f64) -> Pos2 {
    let p = tools_glue::current_view(app).unwrap().to_screen(fontcraft_geom::Point::new(x, y));
    Pos2::new(p.x as f32, p.y as f32)
}

/// Glyph A: one open cubic from (0, 0) to (100, 100) whose handle lines meet at (100, 0), with
/// handles 75 and 60 units long (tensions 75% and 60%), and an anchor. Edited in a tab at 0.5
/// points per unit.
fn aids_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    let nodes = json!([
        {"x": 0, "y": 0}, {"x": 75, "y": 0, "type": "offcurve"}, {"x": 100, "y": 40, "type": "offcurve"}, {"x": 100, "y": 100, "type": "curve"}
    ]);
    app.run("path.addPath", &json!({"glyph": "A", "closed": false, "nodes": nodes})).unwrap();
    app.run("anchor.add", &json!({"glyph": "A", "name": "top", "x": 50, "y": 300})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 500, "center": [100, 150]})).unwrap();
    app
}

#[test]
fn view_show_tunni_lines_draws_the_dotted_line_and_the_meeting_point() {
    let mut app = aids_app();
    let ctx = egui::Context::default();
    let color = app.tokens.aids.tunni;
    let dots = |shapes: &[Shape]| -> Vec<Pos2> {
        shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Circle(c) if c.fill == color => Some(c.center),
                _ => None,
            })
            .collect()
    };
    let dashes = |shapes: &[Shape]| shapes.iter().filter(|s| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == color)).count();
    let shapes = settled(&ctx, &mut app);
    assert_eq!((dots(&shapes).len(), dashes(&shapes)), (0, 0), "off by default");
    assert_eq!(app.run("view.showTunniLines", &json!({})).unwrap(), json!({"on": true}));
    let shapes = settled(&ctx, &mut app);
    assert!(dashes(&shapes) > 1, "a dotted line between the handle ends");
    let tunni = screen(&app, 100.0, 0.0);
    let found = dots(&shapes);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found.iter().all(|p| p.distance(tunni) < 0.5), "the dot sits where the handle lines meet: {found:?} vs {tunni:?}");
    assert_eq!(app.run("view.showTunniLines", &json!({"on": false})).unwrap(), json!({"on": false}));
    assert_eq!(dots(&settled(&ctx, &mut app)).len(), 0);
    assert!(app.run("view.showTunniLines", &json!({"on": "yes"})).is_err());
    assert!(!app.state.panels.edit_aids.tunni_lines, "a failed toggle changes nothing");
}

#[test]
fn view_show_curve_tension_prints_each_handles_percentage() {
    let mut app = aids_app();
    let ctx = egui::Context::default();
    let color = app.tokens.aids.tension;
    assert!(texts(&settled(&ctx, &mut app), color).is_empty());
    app.run("view.showCurveTension", &json!({"on": true})).unwrap();
    assert_eq!(texts(&settled(&ctx, &mut app), color), ["60%", "75%"]);
    // Parallel handles have no meeting point and so no tension.
    app.run(
        "path.addPath",
        &json!({"glyph": "A", "closed": false, "nodes": [
            {"x": 300, "y": 0}, {"x": 300, "y": 50, "type": "offcurve"}, {"x": 400, "y": 50, "type": "offcurve"}, {"x": 400, "y": 0, "type": "curve"}
        ]}),
    )
    .unwrap();
    assert_eq!(texts(&settled(&ctx, &mut app), color), ["60%", "75%"]);
}

#[test]
fn view_show_curve_length_prints_the_arc_length_of_curves_only() {
    let mut app = aids_app();
    let ctx = egui::Context::default();
    let color = app.tokens.aids.curve_length;
    // A cubic drawn straight, 300 units long, then a line.
    app.run("path.addPath", &json!({"glyph": "A", "closed": false, "nodes": [
        {"x": 0, "y": 200}, {"x": 100, "y": 200, "type": "offcurve"}, {"x": 200, "y": 200, "type": "offcurve"}, {"x": 300, "y": 200, "type": "curve"},
        {"x": 300, "y": 250, "type": "line"}
    ]}))
    .unwrap();
    assert!(texts(&settled(&ctx, &mut app), color).is_empty());
    app.run("view.showCurveLength", &json!({})).unwrap();
    let shown = texts(&settled(&ctx, &mut app), color);
    assert_eq!(shown.len(), 2, "the two curves, not the line: {shown:?}");
    assert!(shown.contains(&"300".to_string()), "{shown:?}");
}

#[test]
fn view_show_indexes_labels_the_kinds_asked_for() {
    let mut app = aids_app();
    let ctx = egui::Context::default();
    let t = app.tokens.clone();
    assert_eq!(
        app.run("view.showIndexes", &json!({"points": true})).unwrap(),
        json!({"points": true, "segments": false, "contours": false, "components": false, "anchors": false})
    );
    let shapes = settled(&ctx, &mut app);
    assert_eq!(texts(&shapes, t.measure_on_curve), ["0", "1", "2", "3"]);
    assert!(texts(&shapes, t.anchor).is_empty());
    app.run("view.showIndexes", &json!({"anchors": true, "segments": true})).unwrap();
    let shapes = settled(&ctx, &mut app);
    assert_eq!(texts(&shapes, t.measure_on_curve).len(), 4, "points stay on");
    assert_eq!(texts(&shapes, t.anchor), ["0"]);
    assert_eq!(texts(&shapes, t.measure_ruler), ["0"], "one segment");
    // Without parameters: everything off while something is on, then everything on.
    let off = app.run("view.showIndexes", &json!({})).unwrap();
    assert_eq!(off["points"], false);
    assert!(texts(&settled(&ctx, &mut app), t.measure_on_curve).is_empty());
    let on = app.run("view.showIndexes", &json!(null)).unwrap();
    assert_eq!(on, json!({"points": true, "segments": true, "contours": true, "components": true, "anchors": true}));
    assert_eq!(texts(&settled(&ctx, &mut app), t.guide_local), ["0"], "one path");
    assert!(app.run("view.showIndexes", &json!({"points": 1})).is_err());
}

#[test]
fn view_show_details_across_glyphs_draws_the_other_glyphs_nodes() {
    let mut app = FontcraftApp::default();
    for glyph in ["A", "B"] {
        app.run("path.addRectangle", &json!({"glyph": glyph, "x": 100, "y": 0, "width": 200, "height": 300})).unwrap();
    }
    app.run("view.newTab", &json!({"glyphs": ["A", "B"]})).unwrap();
    assert_eq!(app.state.tab().unwrap().edited_glyph(), Some("A"));
    app.run("view.setZoom", &json!({"size": 300, "center": [600, 150]})).unwrap();
    app.run("view.showIndexes", &json!({"points": true})).unwrap();
    let ctx = egui::Context::default();
    let corner = app.tokens.node_corner;
    let corners = |shapes: &[Shape]| shapes.iter().filter(|s| matches!(s, Shape::Rect(r) if r.fill == corner)).count();
    let shapes = settled(&ctx, &mut app);
    let (one, labels) = (corners(&shapes), texts(&shapes, app.tokens.measure_on_curve).len());
    assert!(one > 0);
    assert_eq!(labels, 4);
    assert_eq!(app.run("view.showDetailsAcrossGlyphs", &json!({})).unwrap(), json!({"on": true}));
    let shapes = settled(&ctx, &mut app);
    assert_eq!(corners(&shapes), 2 * one, "B's nodes too");
    assert_eq!(texts(&shapes, app.tokens.measure_on_curve).len(), 8, "and B's indexes");
}

fn pointer(app: &mut FontcraftApp, events: Value) {
    let r = control::handle(app, &json!({"id": 1, "method": "ui.pointer", "params": {"events": events}}));
    assert_eq!(r["ok"], true, "{r}");
}

fn layer(app: &FontcraftApp) -> fontcraft_model::Layer {
    let font = app.session.font().unwrap();
    font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap().clone()
}

fn drag(app: &mut FontcraftApp, from: (f64, f64), to: (f64, f64)) {
    pointer(
        app,
        json!([
            {"kind": "down", "x": from.0, "y": from.1},
            {"kind": "drag", "x": (from.0 + to.0) / 2.0, "y": (from.1 + to.1) / 2.0},
            {"kind": "drag", "x": to.0, "y": to.1},
            {"kind": "up", "x": to.0, "y": to.1}
        ]),
    );
}

#[test]
fn view_lock_outline_refuses_drags_but_not_commands() {
    let mut app = aids_app();
    let ctx = egui::Context::default();
    settled(&ctx, &mut app);
    let locked = app.tokens.aids.lock_badge;
    assert!(texts(&settled(&ctx, &mut app), locked).is_empty());
    assert_eq!(app.run("view.lockOutline", &json!({})).unwrap(), json!({"on": true}));
    assert_eq!(texts(&settled(&ctx, &mut app), locked), ["Outline locked"], "the badge");
    let before = layer(&app);
    drag(&mut app, (100.0, 100.0), (150.0, 130.0));
    assert_eq!(layer(&app).shapes, before.shapes, "the node stays");
    assert_eq!(app.session.interaction(), None);
    assert_eq!(app.state.status, crate::shell::edit_aids::LOCKED);
    // Anchors stay editable.
    drag(&mut app, (50.0, 300.0), (70.0, 320.0));
    let top = layer(&app).anchors.into_iter().find(|a| a.name == "top").unwrap();
    assert_eq!((top.x, top.y), (70.0, 320.0));
    // Commands are not affected.
    app.run("selection.move", &json!({"glyph": "A", "points": [[0, 3]], "dx": 10, "dy": 0})).unwrap();
    assert_ne!(layer(&app).shapes, before.shapes);
    // Unlocked, the drag edits again.
    app.run("view.lockOutline", &json!({"on": false})).unwrap();
    let moved = layer(&app);
    drag(&mut app, (0.0, 0.0), (0.0, -40.0));
    assert_ne!(layer(&app).shapes, moved.shapes);
}

#[test]
fn drag_ghost_outline_shows_the_changed_shapes_as_they_were() {
    let mut app = aids_app();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 300, "y": 0, "width": 100, "height": 100})).unwrap();
    let ctx = egui::Context::default();
    let grey = app.tokens.background_outline;
    let ghosts =
        |shapes: &[Shape]| shapes.iter().filter(|s| matches!(s, Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(grey))).count();
    assert_eq!(ghosts(&settled(&ctx, &mut app)), 0);
    // Press on the curve's end node and drag without letting go.
    pointer(&mut app, json!([{"kind": "down", "x": 100, "y": 100}, {"kind": "drag", "x": 130, "y": 140}]));
    assert!(app.session.interaction().is_some(), "the drag runs");
    let shapes = settled(&ctx, &mut app);
    assert_eq!(ghosts(&shapes), 1, "the curve as it was; the rectangle did not change");
    pointer(&mut app, json!([{"kind": "up", "x": 130, "y": 140}]));
    assert_eq!(ghosts(&settled(&ctx, &mut app)), 0, "gone once the drag ends");
}

#[test]
fn view_show_metrics_beam_prints_the_sidebearings_where_the_line_meets_the_outline() {
    let mut app = FontcraftApp::default();
    // A right triangle (100, 0), (500, 0), (100, 600) in a 600-unit advance.
    let nodes = json!([{"x": 100, "y": 0}, {"x": 500, "y": 0}, {"x": 100, "y": 600}]);
    app.run("path.addPath", &json!({"glyph": "A", "nodes": nodes})).unwrap();
    app.run("spacing.setWidth", &json!({"glyphs": ["A"], "value": 600})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 300, "center": [300, 300]})).unwrap();
    let ctx = egui::Context::default();
    let color = app.tokens.aids.beam_label;
    assert!(texts(&settled(&ctx, &mut app), color).is_empty());
    let r = app.run("view.showMetricsBeam", &json!({"y": 300})).unwrap();
    assert_eq!(r, json!({"on": true, "y": 300.0}), "a height alone shows the beam");
    // At 300 the triangle reaches from 100 to 300: left 100, right 600 − 300 = 300.
    assert_eq!(texts(&settled(&ctx, &mut app), color), ["100", "300"]);
    app.run("view.showMetricsBeam", &json!({"y": 450})).unwrap();
    assert_eq!(texts(&settled(&ctx, &mut app), color), ["100", "400"]);
    for bad in [json!({"y": "high"}), json!({"y": 1e9}), json!({"on": 1})] {
        assert!(app.run("view.showMetricsBeam", &bad).is_err(), "{bad}");
    }
    assert_eq!(app.run("view.showMetricsBeam", &json!({})).unwrap(), json!({"on": false, "y": 450.0}));
    assert!(texts(&settled(&ctx, &mut app), color).is_empty());
}

#[test]
fn dragging_the_metrics_beam_beside_the_glyph_equals_view_show_metrics_beam() {
    let mut app = FontcraftApp::default();
    app.run("spacing.setWidth", &json!({"glyphs": ["A"], "value": 600})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 300, "center": [300, 300]})).unwrap();
    app.run("view.showMetricsBeam", &json!({"y": 300})).unwrap();
    let ctx = egui::Context::default();
    settled(&ctx, &mut app);
    let events = |ctx: &egui::Context, app: &mut FontcraftApp, events: Vec<egui::Event>| {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let _ = ctx.run_ui(input, |ui| app.ui(ui));
    };
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    // Left of the glyph's origin, where the beam can be grabbed.
    let (from, to) = (screen(&app, -100.0, 300.0), screen(&app, -100.0, 400.0));
    events(&ctx, &mut app, vec![egui::Event::PointerMoved(from)]);
    events(&ctx, &mut app, vec![button(from, true)]);
    events(&ctx, &mut app, vec![egui::Event::PointerMoved(from.lerp(to, 0.5))]);
    events(&ctx, &mut app, vec![egui::Event::PointerMoved(to)]);
    events(&ctx, &mut app, vec![button(to, false)]);
    settled(&ctx, &mut app);
    let beam = app.run("view.showMetricsBeam", &json!({"on": true})).unwrap();
    assert_eq!(beam, json!({"on": true, "y": 400.0}), "the beam follows the pointer in whole units");
    assert!(app.state.tab().unwrap().selection.is_empty(), "the drag selects nothing");
}

#[test]
fn the_aids_draw_in_their_own_theme_tokens() {
    let mut app = aids_app();
    let ctx = egui::Context::default();
    let (tunni, tension, length, badge) =
        (Color32::from_rgb(1, 2, 3), Color32::from_rgb(4, 5, 6), Color32::from_rgb(7, 8, 9), Color32::from_rgb(10, 11, 12));
    app.tokens.aids.tunni = tunni;
    app.tokens.aids.tension = tension;
    app.tokens.aids.curve_length = length;
    app.tokens.aids.lock_badge = badge;
    for id in ["view.showTunniLines", "view.showCurveTension", "view.showCurveLength", "view.lockOutline"] {
        app.run(id, &json!({"on": true})).unwrap();
    }
    let shapes = settled(&ctx, &mut app);
    assert!(shapes.iter().any(|s| matches!(s, Shape::Circle(c) if c.fill == tunni)), "the Tunni point");
    assert_eq!(texts(&shapes, tension), ["60%", "75%"]);
    assert_eq!(texts(&shapes, length).len(), 1, "the curve's length");
    assert_eq!(texts(&shapes, badge), ["Outline locked"]);
    assert!(texts(&shapes, app.tokens.measure_off_curve).is_empty(), "nothing borrows the measurement colours");
}
