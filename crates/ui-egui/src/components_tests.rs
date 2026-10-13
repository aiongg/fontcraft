//! Components, anchors and corner components in Edit View (M6.10): what each display state draws,
//! and each pointer and key gesture giving what its command gives.

use egui::epaint::ColorMode;
use egui::{Color32, Event, Modifiers as EguiMods, PointerButton, Pos2, Shape};
use fontcraft_geom::Point;
use fontcraft_model::{Component, Font, Glyph, Layer, Path};
use serde_json::{Value, json};

use crate::canvas::anchors::{Marker, marker};
use crate::canvas::components::{Placeholder, State, state};
use crate::canvas::nodes::Metrics;
use crate::theme::Tokens;
use crate::{FontcraftApp, control, tools_glue};

struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) -> Vec<Shape> {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
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
}

/// How many shapes are stroked or filled in `color`.
fn painted(shapes: &[Shape], color: Color32) -> usize {
    shapes
        .iter()
        .filter(|s| match s {
            Shape::Path(p) => p.stroke.color == ColorMode::Solid(color) || p.fill == color,
            Shape::LineSegment { stroke, .. } => stroke.color == color,
            Shape::Circle(c) => c.fill == color || c.stroke.color == color,
            Shape::Rect(r) => r.fill == color || r.stroke.color == color,
            Shape::Text(t) => t.fallback_color == color,
            _ => false,
        })
        .count()
}

fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
    assert_eq!(r["ok"], true, "{r}");
    r["result"].clone()
}

fn pointer(app: &mut FontcraftApp, events: Value) {
    call(app, "ui.pointer", json!({ "events": events }));
}

fn layer(app: &FontcraftApp, glyph: &str) -> Layer {
    let font = app.session.font().unwrap();
    font.glyph(glyph).unwrap().layer(&font.masters[0].id).unwrap().clone()
}

fn selection(app: &FontcraftApp) -> Value {
    serde_json::to_value(&app.state.tab().unwrap().selection).unwrap()
}

/// `base`: a square (0,0)–(200,200); `empty`: nothing; `comp`: only a component of `base`
/// (aligned). `A`: a square (100,100)–(300,300) (shape 0), components of `base` at (400, 0)
/// (shape 1) and (700, 0) (shape 2), manual because of the path; anchors `top` on the x-height
/// and `_bottom` on the baseline. Edited at actual size.
fn comp_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    for name in ["base", "empty", "comp"] {
        app.run("glyph.newGlyph", &json!({"name": name})).unwrap();
    }
    app.run("path.addRectangle", &json!({"glyph": "base", "x": 0, "y": 0, "width": 200, "height": 200})).unwrap();
    app.run("glyph.addComponent", &json!({"glyph": "comp", "component": "base"})).unwrap();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    for x in [400, 700] {
        let r = app.run("glyph.addComponent", &json!({"glyph": "A", "component": "base"})).unwrap();
        let shape = r["shape"].as_u64().unwrap();
        app.run("component.move", &json!({"glyph": "A", "components": [shape], "dx": x, "dy": 0})).unwrap();
    }
    app.run("anchor.add", &json!({"glyph": "A", "name": "top", "x": 150, "y": 500})).unwrap();
    app.run("anchor.add", &json!({"glyph": "A", "name": "_bottom", "x": 150, "y": 0})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [450, 250]})).unwrap();
    app
}

fn screen(app: &FontcraftApp, x: f64, y: f64) -> Pos2 {
    let p = tools_glue::current_view(app).unwrap().to_screen(Point::new(x, y));
    Pos2::new(p.x as f32, p.y as f32)
}

#[test]
fn component_states_follow_alignment_and_placeholders() {
    let mut font = Font::new("Test");
    let m = font.masters[0].id.clone();
    for name in ["base", "empty", "loop1", "loop2", "number", "host"] {
        let mut g = Glyph::new(name);
        g.layers.push(Layer::new(m.clone(), 600.0));
        font.glyphs.push(g);
    }
    let mut add = |glyph: &str, shape: fontcraft_model::Shape| font.glyph_mut(glyph).unwrap().layer_mut(&m).unwrap().shapes.push(shape);
    add("base", fontcraft_model::Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
    add("loop1", fontcraft_model::Shape::Component(Component::new("loop2", 0.0, 0.0)));
    add("loop2", fontcraft_model::Shape::Component(Component::new("loop1", 0.0, 0.0)));
    for base in ["base", "empty", "missing", "loop1"] {
        add("host", fontcraft_model::Shape::Component(Component::new(base, 0.0, 0.0)));
    }
    let host = font.glyph("host").unwrap().layer(&m).unwrap().clone();
    let states: Vec<Option<State>> = (0..4).map(|i| state(&font, "host", &host, i)).collect();
    assert_eq!(
        states,
        vec![
            Some(State::Aligned),
            Some(State::Placeholder(Placeholder::Empty)),
            Some(State::Placeholder(Placeholder::Missing)),
            Some(State::Placeholder(Placeholder::Circular)),
        ]
    );
    // A component of the glyph it sits in is circular too.
    let loop1 = font.glyph("loop2").unwrap().layer(&m).unwrap().clone();
    assert_eq!(state(&font, "loop2", &loop1, 0), Some(State::Placeholder(Placeholder::Circular)));
    // Anchor markers by metric position.
    let metrics = Metrics { lines: vec![0.0, 500.0], zones: vec![(-16.0, 0.0)] };
    let at = |y: f64| marker(&fontcraft_model::Anchor::new("a", 0.0, y), &metrics);
    assert_eq!((at(500.0), at(-8.0), at(250.0)), (Marker::Diamond, Marker::Square, Marker::Circle));
}

#[test]
fn edit_view_draws_components_anchors_and_corners() {
    let mut app = comp_app();
    let t = Tokens::dark();
    let w = Window::new();
    let shapes = w.frame(&mut app, Vec::new());
    assert!(painted(&shapes, t.component_manual) >= 2, "the two manual components are grey");
    assert_eq!(painted(&shapes, t.component_aligned), 0);
    // Anchors: a filled diamond on the x-height, a hollow one on the baseline; no names unselected.
    let diamonds = shapes.iter().filter(|s| matches!(s, Shape::Path(p) if p.points.len() == 4 && p.fill == t.anchor)).count();
    let hollow = shapes.iter().filter(|s| matches!(s, Shape::Path(p) if p.points.len() == 4 && p.stroke.color == ColorMode::Solid(t.anchor))).count();
    assert_eq!((diamonds, hollow), (1, 1));
    let names = |shapes: &[Shape]| shapes.iter().filter(|s| matches!(s, Shape::Text(x) if x.galley.text() == "top")).count();
    assert_eq!(names(&shapes), 0, "only selected anchors show their name");
    app.run("selection.set", &json!({"anchors": ["top"], "shapes": [1]})).unwrap();
    let shapes = w.frame(&mut app, Vec::new());
    assert_eq!(names(&shapes), 1);
    assert!(painted(&shapes, t.component_manual) >= 1 && painted(&shapes, t.selection) >= 2, "the selected component and anchor");
    app.run("view.showAnchors", &json!({"on": false})).unwrap();
    let shapes = w.frame(&mut app, Vec::new());
    assert_eq!(painted(&shapes, t.anchor), 0, "View › Show Anchors off hides them");
    app.run("view.showAnchors", &json!({})).unwrap();
    // An aligned composite is green.
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["comp"]})).unwrap();
    let shapes = w.frame(&mut app, Vec::new());
    assert!(painted(&shapes, t.component_aligned) >= 1);
    // A component of an empty glyph is a placeholder that names the problem.
    app.run("glyph.addComponent", &json!({"glyph": "comp", "component": "empty"})).unwrap();
    let shapes = w.frame(&mut app, Vec::new());
    assert!(painted(&shapes, t.component_placeholder) >= 3, "box, cross and label");
    assert!(shapes.iter().any(|s| matches!(s, Shape::Text(x) if x.galley.text() == "empty: empty")));
    // A corner component gets its faint disc.
    app.run("glyph.newGlyph", &json!({"name": "_corner.serif"})).unwrap();
    app.run("path.addPath", &json!({"glyph": "_corner.serif", "closed": false, "nodes": [{"x": 0, "y": 0}, {"x": 20, "y": -20}, {"x": 40, "y": 0}]}))
        .unwrap();
    app.run("cornerComponent.add", &json!({"glyph": "A", "points": [[0, 1]], "corner": "_corner.serif"})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    let shapes = w.frame(&mut app, Vec::new());
    assert_eq!(painted(&shapes, t.corner_highlight), 1);
}

#[test]
fn a_base_glyph_edit_redraws_its_composites_while_dragging() {
    let mut app = comp_app();
    let t = Tokens::dark();
    app.run("view.fillPreview", &json!({"on": false})).unwrap();
    app.run("editView.setText", &json!({"glyphs": ["base", "comp"]})).unwrap();
    app.run("editView.setCursor", &json!({"index": 0})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [300, 100]})).unwrap();
    let w = Window::new();
    let others = |shapes: &[Shape]| -> Vec<Vec<Pos2>> {
        shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Path(p) if p.stroke.color == ColorMode::Solid(t.glyph_fill) => Some(p.points.clone()),
                _ => None,
            })
            .collect()
    };
    let before = others(&w.frame(&mut app, Vec::new()));
    assert!(!before.is_empty(), "the composite is drawn as an outline");
    // Drag the base glyph's top right node; the composite follows before the button comes up.
    pointer(&mut app, json!([{"kind": "down", "x": 200, "y": 200}, {"kind": "drag", "x": 260, "y": 200}]));
    assert_eq!(app.session.interaction(), Some("Move"));
    let during = others(&w.frame(&mut app, Vec::new()));
    assert_ne!(before, during, "the composite redraws during the drag");
    pointer(&mut app, json!([{"kind": "up", "x": 260, "y": 200}]));
}

#[test]
fn component_gestures_equal_their_commands() {
    let mut app = comp_app();
    let click = |app: &mut FontcraftApp, x: f64, y: f64, mods: Value| {
        pointer(app, json!([{"kind": "down", "x": x, "y": y, "mods": mods}, {"kind": "up", "x": x, "y": y, "mods": mods}]));
    };
    // A click inside a component selects it; Shift adds another.
    click(&mut app, 500.0, 100.0, json!([]));
    assert_eq!(selection(&app), json!({"shapes": [1]}));
    click(&mut app, 800.0, 100.0, json!(["shift"]));
    assert_eq!(selection(&app), json!({"shapes": [1, 2]}));
    // A marquee skips components; with Alt it takes them (and on-curve nodes only), as
    // selection.selectInRegion with nodesOnly and components does.
    let marquee = |app: &mut FontcraftApp, mods: Value| {
        pointer(
            app,
            json!([
                {"kind": "down", "x": 50, "y": -50, "mods": mods}, {"kind": "drag", "x": 950, "y": 400, "mods": mods},
                {"kind": "up", "x": 950, "y": 400, "mods": mods},
            ]),
        );
    };
    marquee(&mut app, json!([]));
    assert_eq!(selection(&app)["shapes"], Value::Null, "{}", selection(&app));
    marquee(&mut app, json!(["alt"]));
    let by_gesture = selection(&app);
    assert_eq!(by_gesture["shapes"], json!([1, 2]));
    let by_command = app.run("selection.selectInRegion", &json!({"rect": [50, -50, 900, 450], "nodesOnly": true, "components": true})).unwrap();
    assert_eq!(selection(&app), by_gesture, "{by_command}");
    // Select All: the paths first, then the components and anchors too.
    app.run("edit.deselectAll", &json!({})).unwrap();
    app.run("edit.selectAll", &json!({})).unwrap();
    assert_eq!(selection(&app), json!({"points": [[0, 0], [0, 1], [0, 2], [0, 3]]}));
    app.run("edit.selectAll", &json!({})).unwrap();
    assert_eq!(selection(&app)["shapes"], json!([1, 2]));
    assert_eq!(selection(&app)["anchors"], json!(["_bottom", "top"]));
    // Tab moves from a component to the next one, Shift+Tab back; selection.selectNext agrees.
    app.run("selection.set", &json!({"shapes": [1]})).unwrap();
    call(&mut app, "ui.key", json!({"key": "Tab"}));
    assert_eq!(selection(&app), json!({"shapes": [2]}));
    call(&mut app, "ui.key", json!({"key": "Tab", "shift": true}));
    assert_eq!(selection(&app), json!({"shapes": [1]}));
    app.run("selection.selectNext", &json!({})).unwrap();
    assert_eq!(selection(&app), json!({"shapes": [2]}));
    // Alt-drag duplicates the component: selection.move with duplicate.
    pointer(
        &mut app,
        json!([
            {"kind": "down", "x": 500, "y": 100, "mods": ["alt"]}, {"kind": "drag", "x": 500, "y": 400, "mods": ["alt"]},
            {"kind": "up", "x": 500, "y": 400, "mods": ["alt"]},
        ]),
    );
    let mut headless = comp_app();
    headless.run("selection.move", &json!({"glyph": "A", "shapes": [1], "dx": 0, "dy": 300, "duplicate": true})).unwrap();
    assert_eq!(layer(&app, "A"), layer(&headless, "A"), "{}", app.state.status);
    // Delete removes the selected components, as edit.delete does.
    app.run("selection.set", &json!({"shapes": [1, 2]})).unwrap();
    call(&mut app, "ui.key", json!({"key": "Delete"}));
    headless.run("edit.delete", &json!({"glyph": "A", "shapes": [1, 2]})).unwrap();
    assert_eq!(layer(&app, "A"), layer(&headless, "A"));
}

#[test]
fn double_click_opens_the_base_glyph_like_edit_base_glyph() {
    let mut app = comp_app();
    pointer(&mut app, json!([{"kind": "doubleclick", "x": 500, "y": 100}]));
    let mut headless = comp_app();
    headless.run("component.editBaseGlyph", &json!({"component": 1})).unwrap();
    let text = |app: &mut FontcraftApp| app.run("editView.getText", &json!({})).unwrap();
    let (by_gesture, by_command) = (text(&mut app), text(&mut headless));
    assert_eq!(by_gesture, by_command);
    assert_eq!(by_gesture["current"], "base", "{by_gesture}");
    assert_eq!(by_gesture["items"], json!(["base", "A"]));
    // The selected component, or a glyph by name; anything else is refused.
    let mut app = comp_app();
    app.run("selection.set", &json!({"shapes": [2]})).unwrap();
    assert_eq!(app.run("component.editBaseGlyph", &json!({})).unwrap(), json!({"glyph": "base"}));
    assert!(app.run("component.editBaseGlyph", &json!({"component": 0})).is_err(), "shape 0 is a path");
    assert!(app.run("component.editBaseGlyph", &json!({"glyph": "nosuch"})).is_err());
}

#[test]
fn anchors_move_like_nodes_and_component_nodes_are_snap_targets() {
    let mut app = comp_app();
    let t = Tokens::dark();
    let w = Window::new();
    w.frame(&mut app, Vec::new());
    // Drag an anchor: selection.move with the anchor; arrows nudge it.
    pointer(
        &mut app,
        json!([{"kind": "down", "x": 150, "y": 500}, {"kind": "drag", "x": 170, "y": 530, "mods": ["ctrl"]}, {"kind": "up", "x": 170, "y": 530, "mods": ["ctrl"]}]),
    );
    let mut headless = comp_app();
    headless.run("selection.move", &json!({"glyph": "A", "anchors": ["top"], "dx": 20, "dy": 30})).unwrap();
    assert_eq!(layer(&app, "A").anchors, layer(&headless, "A").anchors);
    call(&mut app, "ui.key", json!({"key": "ArrowRight"}));
    let top = layer(&app, "A").anchors.into_iter().find(|a| a.name == "top").unwrap();
    assert_eq!((top.x, top.y), (171.0, 530.0));
    // While a node is dragged, the nodes inside components show, and the node snaps onto one.
    let (from, near) = (screen(&app, 300.0, 300.0), screen(&app, 398.0, 201.0));
    w.frame(&mut app, vec![Event::PointerMoved(from)]);
    w.frame(&mut app, vec![Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: EguiMods::NONE }]);
    w.frame(&mut app, vec![Event::PointerMoved(Pos2::new(from.x + 30.0, from.y))]);
    let shapes = w.frame(&mut app, vec![Event::PointerMoved(near)]);
    assert!(painted(&shapes, t.component_node) >= 8, "the corners of both components show");
    w.frame(&mut app, vec![Event::PointerButton { pos: near, button: PointerButton::Primary, pressed: false, modifiers: EguiMods::NONE }]);
    let node = match &layer(&app, "A").shapes[0] {
        fontcraft_model::Shape::Path(p) => (p.nodes[2].x, p.nodes[2].y),
        other => panic!("shape 0 is not a path: {other:?}"),
    };
    assert_eq!(node, (400.0, 200.0), "the node snapped onto the component's corner");
}

/// § 5 of the M6 plan: an Edit View frame with 200 composites in text mode under 16 ms. Run with
/// `cargo test --profile measure -p fontcraft-ui-egui composite_text_frame_speed -- --ignored --nocapture`.
#[test]
#[ignore]
fn composite_text_frame_speed() {
    let mut app = comp_app();
    app.run("glyph.newGlyph", &json!({"name": "acutecomb"})).unwrap();
    app.run("path.addRectangle", &json!({"glyph": "acutecomb", "x": 50, "y": 500, "width": 100, "height": 150})).unwrap();
    let mut names = Vec::new();
    for i in 0..200 {
        let name = format!("comp{i}");
        app.run("glyph.newGlyph", &json!({"name": name})).unwrap();
        app.run("glyph.addComponent", &json!({"glyph": name, "component": "base"})).unwrap();
        app.run("glyph.addComponent", &json!({"glyph": name, "component": "acutecomb"})).unwrap();
        names.push(name);
    }
    app.run("editView.setText", &json!({"glyphs": names})).unwrap();
    app.run("editView.setCursor", &json!({"index": 0})).unwrap();
    app.run("editView.toggleTextMode", &json!({"mode": "text"})).unwrap();
    app.run("view.setZoom", &json!({"size": 60, "center": [6000, -2000]})).unwrap();
    let w = Window::new();
    for _ in 0..3 {
        w.frame(&mut app, Vec::new());
    }
    let runs = 20;
    let started = std::time::Instant::now();
    for i in 0..runs {
        app.run("editView.setCursor", &json!({"index": i})).unwrap();
        w.frame(&mut app, Vec::new());
    }
    let steady = started.elapsed() / runs;
    // After a base-glyph edit every composite is rebuilt.
    app.run("path.addRectangle", &json!({"glyph": "base", "x": 0, "y": 300, "width": 50, "height": 50})).unwrap();
    let started = std::time::Instant::now();
    w.frame(&mut app, Vec::new());
    let after_edit = started.elapsed();
    println!("text mode, 200 composites: steady frame {steady:?}, frame after a base-glyph edit {after_edit:?}");
}

/// `_part.bar`: a master layer with a 200 × 100 rectangle and a `Wide` layer with a 600 × 100
/// one, a Width property from 200 to 600 with those layers at its ends, and an anchor `Width` on
/// the right edge of each. `bars` holds one component of it, selected, edited at actual size.
fn smart_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.session
        .edit("setup", |font| {
            let m = font.masters[0].id.clone();
            let mut part = Glyph::new("_part.bar");
            for (id, name, width) in [(m.as_str(), None, 200.0), ("wide", Some("Wide"), 600.0)] {
                let mut l = Layer::new(m.clone(), 600.0);
                l.id = id.into();
                l.info.name = name.map(String::from);
                l.shapes.push(fontcraft_model::Shape::Path(Path::rect(0.0, 0.0, width, 100.0)));
                l.anchors.push(fontcraft_model::Anchor::new("Width", width, 50.0));
                part.layers.push(l);
            }
            font.glyphs.push(part);
            let mut s = Glyph::new("bars");
            let mut l = Layer::new(m, 800.0);
            l.shapes.push(fontcraft_model::Shape::Component(Component::new("_part.bar", 0.0, 0.0)));
            s.layers.push(l);
            font.glyphs.push(s);
            Ok(())
        })
        .unwrap();
    app.run("smartGlyph.addProperty", &json!({"glyph": "_part.bar", "name": "Width", "min": 200, "max": 600})).unwrap();
    let master = app.session.font().unwrap().masters[0].id.clone();
    app.run("smartGlyph.setLayerPole", &json!({"glyph": "_part.bar", "layer": master, "property": "Width", "pole": "min"})).unwrap();
    app.run("smartGlyph.setLayerPole", &json!({"glyph": "_part.bar", "layer": "Wide", "property": "Width", "pole": "max"})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["bars"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [300, 50]})).unwrap();
    app.run("selection.set", &json!({"shapes": [0]})).unwrap();
    app
}

#[test]
fn a_smart_handle_drag_sets_the_property_like_the_command() {
    let mut app = smart_app();
    let t = Tokens::dark();
    let w = Window::new();
    let shapes = w.frame(&mut app, Vec::new());
    let handle = screen(&app, 200.0, 50.0);
    let drawn = shapes.iter().any(|s| matches!(s, Shape::Circle(c) if c.fill == t.smart_handle && c.center.distance(handle) < 1.0));
    assert!(drawn, "the Width anchor is a grey handle on the selected component");
    let steps = app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len);
    pointer(
        &mut app,
        json!([{"kind": "down", "x": 200, "y": 50}, {"kind": "drag", "x": 350, "y": 60}, {"kind": "drag", "x": 500, "y": 70}, {"kind": "up", "x": 500, "y": 70}]),
    );
    assert_eq!(app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len), steps + 1, "one undo step");
    let mut headless = smart_app();
    headless.run("smartComponent.setValue", &json!({"glyph": "bars", "component": 0, "property": "Width", "value": 500})).unwrap();
    assert_eq!(layer(&app, "bars"), layer(&headless, "bars"), "{}", app.state.status);
    // Without the anchor on the poles a property has no handle.
    app.run("selection.set", &json!({})).unwrap();
    let shapes = w.frame(&mut app, Vec::new());
    assert_eq!(painted(&shapes, t.smart_handle), 0, "only selected components show handles");
}

#[test]
fn bounding_box_knobs_set_width_on_a_smart_component() {
    let mut app = smart_app();
    app.run("view.showBoundingBox", &json!({"on": true})).unwrap();
    let w = Window::new();
    w.frame(&mut app, Vec::new());
    let knob = |app: &FontcraftApp| {
        let view = tools_glue::current_view(app).unwrap();
        let font = app.session.font().unwrap();
        let tab = app.state.tab().unwrap();
        let scene = tools_glue::scene(font, tab).unwrap();
        let rect = fontcraft_tools::bbox::shown_box(&scene, &tab.selection).unwrap();
        let (_, at) = fontcraft_tools::bbox::knob_points(rect, 1.0 / view.scale())[4];
        (rect, at)
    };
    let (rect, at) = knob(&app);
    assert_eq!(rect.width(), 200.0);
    let to = Point::new(at.x + 200.0, at.y + 30.0);
    pointer(
        &mut app,
        json!([{"kind": "down", "x": at.x, "y": at.y}, {"kind": "drag", "x": at.x + 100.0, "y": at.y}, {"kind": "drag", "x": to.x, "y": to.y}, {"kind": "up", "x": to.x, "y": to.y}]),
    );
    let mut headless = smart_app();
    headless.run("smartComponent.setValue", &json!({"glyph": "bars", "component": 0, "property": "Width", "value": 400})).unwrap();
    assert_eq!(layer(&app, "bars"), layer(&headless, "bars"), "{}", app.state.status);
}
