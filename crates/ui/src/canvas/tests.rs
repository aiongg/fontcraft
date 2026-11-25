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
    FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()))
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
    // Labels need the fonts, which a context has after its first pass.
    let ctx = egui::Context::default();
    ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
    let painter = Painter::new(ctx, LayerId::background(), RECT);
    let mut view = app.view.canvas;
    view.rect = RECT;
    let top = app.models.canvas.glyph.as_ref().map_or(0.0, |g| g.ascender);
    let words = CanvasWords::default();
    let overlay_words = super::overlays::OverlayWords::default();
    let hint_tokens = crate::canvas::hinting::tokens::HintTokens::standard();
    let cx = PaintCx {
        painter: &painter,
        rect: RECT,
        camera: Camera::new(view, top),
        model: &app.models.canvas,
        tokens: &app.models.canvas_tokens,
        display: super::text::display(&app.view),
        edit_background: app.view.edit_background,
        words: &words,
        tools: &app.tools.view,
        text: &app.models.text,
        overlays: super::overlays::Painted { data: &app.overlays.data, words: &overlay_words },
        hinting: crate::canvas::hinting::HintPaint { model: &app.models.hint_canvas, view: &app.view.hinting, tokens: &hint_tokens },
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

/// The names of the labels among `shapes`.
fn labels(shapes: &[Shape]) -> Vec<String> {
    shapes.iter().filter_map(|s| if let Shape::Text(t) = s { Some(t.galley.text().to_string()) } else { None }).collect()
}

/// Selected components draw a wider outline in the selection colour, selected anchors a ring and their name; Escape, which
/// enters text mode, draws none of it.
#[test]
fn the_selection_marks_components_and_anchors() {
    let mut app = components_app();
    run(&mut app, "glyph.addComponent", json!({"glyph": "A", "component": "base"}));
    run(&mut app, "component.move", json!({"glyph": "A", "components": [3], "dx": 1000, "dy": 0}));
    let t = app.models.canvas_tokens.clone();
    let selection = super::tools::tokens::ToolTokens::standard().selection;
    let wide = |app: &FontCraftApp| {
        painted(app, paint::components::shapes)
            .iter()
            .filter(|s| matches!(s, Shape::Path(p) if p.stroke.color == ColorMode::Solid(selection) && p.stroke.width == t.component_selected_width))
            .count()
    };
    let fills = |app: &FontCraftApp| painted(app, paint::components::shapes).iter().filter(|s| matches!(s, Shape::Mesh(_))).count();
    assert_eq!((wide(&app), fills(&app)), (0, 3), "each component is filled lightly");
    assert!(labels(&painted(&app, paint::anchors::shapes)).is_empty(), "an unselected anchor shows no name");

    shell(&mut app, "selection.set", json!({"shapes": [1, 3], "anchors": ["top"]}));
    assert_eq!(wide(&app), 2);
    assert_eq!(count(&painted(&app, paint::components::shapes), t.component_manual), 1, "the third keeps its state's colour");
    let anchors = painted(&app, paint::anchors::shapes);
    assert_eq!(labels(&anchors), ["top"]);
    assert_eq!(count(&anchors, selection), 1, "one ring");

    crate::control::dispatch(&mut app, "ui.key", &json!({"key": "Escape"})).unwrap();
    app.drain();
    assert_eq!((wide(&app), fills(&app)), (0, 0), "text mode marks nothing");
    assert!(labels(&painted(&app, paint::anchors::shapes)).is_empty());
}

/// A click inside a placeholder selects its component, so the Info box names the base glyph.
#[test]
fn a_click_on_a_placeholder_selects_its_component() {
    let mut app = components_app();
    run(&mut app, "glyph.addComponent", json!({"glyph": "A", "component": "empty"}));
    run(&mut app, "component.move", json!({"glyph": "A", "components": [3], "dx": 0, "dy": -600}));
    app.view.canvas = CanvasView::new(RECT, Point::new(400.0, 400.0), 1.0);
    let box_corner = app.models.canvas.glyph.as_ref().unwrap().components.last().unwrap().placeholder.unwrap()[0];
    let at = [box_corner[0] + 30.0, box_corner[1] + 200.0];
    let events = json!([{"kind": "down", "x": at[0], "y": at[1]}, {"kind": "up", "x": at[0], "y": at[1]}]);
    crate::control::dispatch(&mut app, "ui.pointer", &json!({ "events": events })).unwrap();
    app.drain();
    assert!(app.tools.selection.shapes.contains(&3), "{:?}", app.tools.selection);
    assert_eq!(app.models.palette.info_box.component.as_ref().map(|c| c.base.as_str()), Some("empty"));
    let selection = super::tools::tokens::ToolTokens::standard().selection;
    assert_eq!(count(&painted(&app, paint::components::shapes), selection), 3, "its box and cross are in the selection colour");
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

/// A per-script ascender replaces the master's line for glyphs of that script: the canvas draws one ascender line at
/// it, its zone moves with it, and the tools snap to it.
#[test]
fn a_greek_ascender_replaces_the_masters_line_in_a_greek_glyph() {
    let mut app = app();
    app.session
        .edit("setup", |font| {
            let m = Arc::make_mut(&mut font.masters).first_mut().unwrap();
            m.params.set("ascender", fontcraft_engine::model::plist::Value::String("greek: 705".into()));
            font.glyph_mut("A")?.info.script = Some("greek".into());
            Ok(())
        })
        .unwrap();
    edit(&mut app, "A");
    let glyph = app.models.canvas.glyph.clone().unwrap();
    let ascenders: Vec<f64> = glyph.lines.iter().filter(|(name, _)| name == "ascender").map(|(_, y)| *y).collect();
    assert_eq!(ascenders, vec![705.0], "one ascender line, at the Greek value: {:?}", glyph.lines);
    assert!(glyph.zones.iter().any(|&(bottom, top)| bottom == 705.0 && top > 705.0), "the zone moves: {:?}", glyph.zones);
    assert!(!glyph.zones.iter().any(|&(bottom, _)| bottom == 800.0), "no zone at the master's ascender: {:?}", glyph.zones);
    let font = app.session.font().unwrap();
    let scene = super::tools::scene(font, app.session.glyph_data().ok().as_deref(), &app.view, None).unwrap();
    assert!(scene.metric_lines.contains(&705.0) && !scene.metric_lines.contains(&800.0), "{:?}", scene.metric_lines);
    let view = fontcraft_tools::View::default();
    let s = fontcraft_tools::snap(&scene, &view, fontcraft_geom::Point::new(50.0, 703.0), &fontcraft_tools::Moving::default(), 4.0);
    assert_eq!(s.point.y, 705.0, "a drag near 705 snaps to the Greek ascender");

    // A Latin glyph keeps the master's line.
    edit(&mut app, "B");
    let lines = &app.models.canvas.glyph.as_ref().unwrap().lines;
    assert!(lines.iter().any(|(name, y)| name == "ascender" && *y == 800.0), "{lines:?}");
}

/// The canvas is filled with its stored colour, and Fill Preview fills with the stored foreground stroke colour
/// (`view.fillColorFromSettings`).
#[test]
fn the_canvas_and_the_fill_take_the_appearance_colours() {
    let mut app = app();
    // High enough to lie inside RECT at the unfitted view: the fill is clipped to the canvas, the stroke is not.
    run(&mut app, "path.addRectangle", json!({"glyph": "A", "x": 100, "y": 500, "width": 200, "height": 200}));
    run(&mut app, "settings.set", json!({"id": "appearance.color.canvas.light", "value": "#ffeedd"}));
    run(&mut app, "settings.set", json!({"id": "appearance.color.foregroundStroke.light", "value": "#123456"}));
    edit(&mut app, "A");
    app.sent.push(Msg::View(ViewMsg::SelectTool("hand")));
    app.drain();
    let canvas = Color32::from_rgb(0xff, 0xee, 0xdd);
    let ink = Color32::from_rgb(0x12, 0x34, 0x56);
    assert_eq!(count(&painted(&app, paint::background::canvas_shapes), canvas), 1);
    assert!(app.view.display.fill_preview && app.tools.view.fills, "Fill Preview fills under the Hand tool");
    let outline = painted(&app, paint::outline::shapes);
    let filled = outline.iter().any(|s| matches!(s, Shape::Mesh(m) if m.vertices.first().is_some_and(|v| v.color == ink)));
    assert!(filled, "the fill takes the foreground colour");
    assert!(count(&outline, ink) >= 1, "so does the stroke");
}

/// The active master's Master Background Color is painted over the canvas colour, its Dark form on a dark canvas and
/// the light form again while Always use Light Mode keeps Edit View light; a master without one sets nothing.
#[test]
fn the_active_master_colours_the_canvas_for_the_appearance_in_use() {
    let mut app = app();
    run(&mut app, "font.master.add", json!({"name": "Bold"}));
    let bold = app.session.font().unwrap().masters[1].id.clone();
    let add = |name: &str, value: &str| json!({"scope": "master", "targets": bold, "name": name, "value": value});
    run(&mut app, "font.customParameters.add", add("Master Background Color", "#336699"));
    run(&mut app, "font.customParameters.add", add("Master Background Color Dark", "20, 30, 40, 255"));
    edit(&mut app, "A");
    let light = Color32::from_rgb(0x33, 0x66, 0x99);
    let dark = Color32::from_rgb(20, 30, 40);
    assert_eq!(app.models.canvas_tokens.master_canvas, None, "the first master sets no background");

    shell(&mut app, "view.selectMaster", json!({"master": 1}));
    assert_eq!(app.models.canvas_tokens.master_canvas, Some(light));
    let shapes = painted(&app, paint::background::canvas_shapes);
    assert_eq!(shapes.len(), 2, "the canvas colour, then the master's");
    assert_eq!(count(&shapes, light), 1);

    app.set_system_dark(true);
    app.drain();
    assert_eq!(app.models.canvas_tokens.master_canvas, Some(dark));
    run(&mut app, "settings.set", json!({"id": "appearance.alwaysLightModeEditView", "value": true}));
    assert_eq!(app.models.canvas_tokens.master_canvas, Some(light), "a canvas kept light takes the light value");
}

/// A drag's badge is painted beside the pointer: a filled box and its text (drag badges).
#[test]
fn a_drag_badge_is_painted_beside_the_pointer() {
    let mut app = app();
    edit(&mut app, "A");
    app.tools.view.overlays = vec![fontcraft_tools::Overlay::Badge { at: fontcraft_geom::Point::new(0.0, 0.0), text: "x:30 y:-20".into() }];
    let shapes = painted(&app, super::badges::shapes);
    let t = super::tools::tokens::ToolTokens::standard();
    let boxes: Vec<Rect> =
        shapes.iter().filter_map(|s| if let Shape::Rect(r) = s { (r.fill == t.badge_fill).then_some(r.rect) } else { None }).collect();
    let texts: Vec<(String, Pos2)> =
        shapes.iter().filter_map(|s| if let Shape::Text(text) = s { Some((text.galley.text().to_string(), text.pos)) } else { None }).collect();
    assert_eq!(boxes.len(), 1, "{shapes:?}");
    assert_eq!(texts.len(), 1, "{shapes:?}");
    assert_eq!(texts[0].0, "x:30 y:-20");
    assert!(boxes[0].contains(texts[0].1), "the text sits on its box");
    assert!(boxes[0].width() > 0.0 && boxes[0].height() > 0.0);
    // Off the pointer: the box starts right of and below the pointer's screen position.
    let mut view = app.view.canvas;
    view.rect = RECT;
    let top = app.models.canvas.glyph.as_ref().map_or(0.0, |g| g.ascender);
    let pointer = Camera::new(view, top).to_screen(fontcraft_geom::Point::new(0.0, 0.0));
    assert!(boxes[0].min.x > pointer.x && boxes[0].min.y > pointer.y, "{:?} {pointer:?}", boxes[0]);
}

/// Glyph A on three masters along Weight, its rectangle moving and widening; Medium shown with the Higher-Order
/// Interpolation tool.
fn interpolation_app() -> FontCraftApp {
    let mut app = app();
    run(&mut app, "font.axes.add", json!({"name": "Weight", "tag": "wght"}));
    for _ in 0..2 {
        run(&mut app, "font.master.add", json!({}));
    }
    for (master, weight, x, width) in [(0, 300, 100, 100), (1, 400, 200, 200), (2, 700, 300, 300)] {
        run(&mut app, "font.master.setAxisLocation", json!({"masters": [master], "location": {"wght": weight}}));
        let layer = app.session.font().unwrap().masters[master].id.clone();
        run(&mut app, "path.addRectangle", json!({"glyph": "A", "layer": layer, "x": x, "y": 0, "width": width, "height": 500}));
    }
    edit(&mut app, "A");
    shell(&mut app, "view.selectMaster", json!({"master": 1}));
    app.sent.push(Msg::View(ViewMsg::SelectTool(fontcraft_tools::hoi::ID)));
    app.drain();
    let ascender = app.models.canvas.glyph.as_ref().unwrap().ascender;
    app.view.canvas = CanvasView::new(RECT, Point::new(350.0, ascender - 250.0), 0.8);
    app
}

fn pointer(app: &mut FontCraftApp, kind: fontcraft_tools::PointerKind, x: f64, y: f64) {
    let ev = fontcraft_tools::PointerEvent::new(kind, x, y, fontcraft_tools::Modifiers::NONE);
    app.sent.push(Msg::Tool(crate::canvas::tools::ToolInput::Pointer(ev)));
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
}

#[test]
fn the_interpolation_tool_marks_the_layer_its_neighbours_and_the_paths_between() {
    use super::hoi::tokens::HoiTokens;
    let mut app = interpolation_app();
    let t = HoiTokens::standard();
    let diamonds = |app: &FontCraftApp| {
        let shapes = painted(app, super::hoi::shapes);
        [count(&shapes, t.below), count(&shapes, t.here), count(&shapes, t.above)]
    };
    assert_eq!(diamonds(&app), [4, 4, 4]);
    let shapes = painted(&app, super::hoi::shapes);
    assert_eq!(count(&shapes, t.linear_knob), 8, "a knob on each node's path up and down");
    assert!(count(&shapes, t.line) > 8, "the paths are dotted");
    assert_eq!(count(&shapes, t.curve), 0);

    shell(&mut app, "view.selectMaster", json!({"master": 2}));
    assert_eq!(diamonds(&app), [4, 4, 0], "the heaviest master has no layer above");
    shell(&mut app, "view.selectMaster", json!({"master": 0}));
    assert_eq!(diamonds(&app), [0, 4, 4]);

    app.sent.push(Msg::View(ViewMsg::SelectTool("select")));
    app.drain();
    assert!(painted(&app, super::hoi::shapes).is_empty(), "only with the tool");
}

#[test]
fn a_dragged_knob_bends_the_path_and_the_context_menu_sets_the_type() {
    use fontcraft_tools::PointerKind::{Down, Drag, Up};
    use fontcraft_tools::hoi::Kind;

    use super::hoi::tokens::HoiTokens;
    use crate::app::context_menu::{INTERPOLATION, Space, invoke_choice, open_canvas};
    let mut app = interpolation_app();
    let t = HoiTokens::standard();
    let spans = |app: &FontCraftApp| app.models.canvas.hoi.clone().unwrap().spans;
    let at = spans(&app).iter().position(|s| s.toward == fontcraft_tools::hoi::Toward::Down && s.node == 0).unwrap();
    let knob = spans(&app)[at].knobs[0];
    pointer(&mut app, Down, knob.x, knob.y);
    pointer(&mut app, Drag, knob.x + 20.0, knob.y + 60.0);
    pointer(&mut app, Up, knob.x + 20.0, knob.y + 60.0);
    let bent = &spans(&app)[at];
    assert_eq!((bent.kind, bent.knobs[0].y), (Kind::IntermediatePoint, knob.y + 60.0));
    assert!(count(&painted(&app, super::hoi::shapes), t.curve) >= 1, "the curve is drawn");
    let lower = app.session.font().unwrap().masters[0].id.clone();
    let hoi = app.models.palette.info_box.hoi.as_ref().unwrap();
    assert_eq!((hoi.layer.as_str(), hoi.kind.as_str()), (lower.as_str(), "intermediatePoint"), "the Info box shows the picked knob's curve");

    open_canvas(&mut app, bent.knobs[0], Space::Font).unwrap();
    invoke_choice(&mut app, INTERPOLATION, Some("rotation"), |app, msg| app.run_now(msg, false)).unwrap();
    assert_eq!(spans(&app)[at].kind, Kind::Rotation);
    run(&mut app, "edit.undo", json!({}));
    run(&mut app, "edit.undo", json!({}));
    assert_eq!(spans(&app)[at].kind, Kind::Linear, "the drag was one undo step");

    shell(&mut app, "hoi.selectAxis", json!({"axis": "wght"}));
    app.sent.push(Msg::Shell(CommandRef::with_params("hoi.selectAxis", json!({"axis": "none"}))));
    app.drain();
    assert!(app.status.contains("no axis"), "{}", app.status);
}
