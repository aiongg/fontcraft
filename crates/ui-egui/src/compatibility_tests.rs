//! The compatibility indicators and the Master Compatibility view in headless frames: the red
//! bar and Font View corner on an incompatible glyph, the view's layout, lines and segment
//! colours, a dropped line end giving `glyph.swapShapes`, and Keep Layer Selection in Sync.

use std::sync::Arc;

use egui::epaint::{ColorMode, PathShape, RectShape};
use egui::{Color32, Pos2, Rect, Shape};
use fontcraft_geom::Point;
use fontcraft_model::plist::Value as Plist;
use fontcraft_model::{Axis, Font, Instance, Layer, Path, Shape as ModelShape};
use fontcraft_tools::Selection;
use serde_json::json;

use crate::FontcraftApp;
use crate::canvas::compatibility::{LinkKind, SegmentState, carried_selection, drop_swap, incompatible_in_tab, links, placement, segment_states};
use crate::tools_glue;

fn rect(x0: f64, w: f64) -> ModelShape {
    ModelShape::Path(Path::rect(x0, 0.0, x0 + w, 500.0))
}

fn layer(master: &str, shapes: Vec<ModelShape>) -> Layer {
    let mut l = Layer::new(master, 600.0);
    l.shapes = shapes;
    l
}

/// The default document with a weight axis, masters `thin` (100, the first master) and `black`
/// (900) and an instance at 400. `A` has two boxes, in opposite order on the two masters (a
/// shape-shifter, yet compatible); `B` has one box on Thin and two on Black (incompatible).
fn compat_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.session
        .edit("setup", |font| {
            font.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
            let mut thin = font.default_master()?.clone();
            let old = thin.id.clone();
            thin.id = "thin".into();
            thin.location = vec![100.0];
            let mut black = thin.clone();
            black.id = "black".into();
            black.name = "Black".into();
            black.location = vec![900.0];
            font.masters = Arc::new(vec![thin, black]);
            font.instances = Arc::new(vec![Instance { name: "Regular".into(), location: vec![400.0], ..Instance::default() }]);
            for i in 0..font.glyphs.len() {
                if let Some(mut g) = font.glyphs.get_mut_at(i) {
                    let shapes = g.layers.iter().find(|l| l.master == old).map(|l| l.shapes.clone()).unwrap_or_default();
                    g.layers = vec![layer("thin", shapes.clone()), layer("black", shapes)];
                }
            }
            font.glyph_mut("A")?.layers =
                vec![layer("thin", vec![rect(0.0, 100.0), rect(300.0, 100.0)]), layer("black", vec![rect(300.0, 200.0), rect(0.0, 200.0)])];
            font.glyph_mut("B")?.layers = vec![layer("thin", vec![rect(0.0, 100.0)]), layer("black", vec![rect(0.0, 200.0), rect(300.0, 100.0)])];
            Ok(())
        })
        .unwrap();
    app
}

fn font(app: &FontcraftApp) -> Arc<Font> {
    app.session.doc().unwrap().font.clone()
}

/// Two frames; the shapes of the last, nested lists flattened.
fn frame(ctx: &egui::Context, app: &mut FontcraftApp) -> Vec<Shape> {
    let mut flat = Vec::new();
    for _ in 0..2 {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
        let mut out: Vec<Shape> = output.shapes.into_iter().map(|c| c.shape).collect();
        flat.clear();
        while let Some(shape) = out.pop() {
            match shape {
                Shape::Vec(list) => out.extend(list),
                other => flat.push(other),
            }
        }
    }
    flat
}

/// Lines stroked in `colour`.
fn strokes(shapes: &[Shape], colour: Color32) -> usize {
    shapes.iter().filter(|s| matches!(s, Shape::Path(PathShape { stroke, .. }) if stroke.color == ColorMode::Solid(colour))).count()
}

#[test]
fn view_show_master_compatibility_draws_the_layers_and_a_dropped_line_end_equals_glyph_swap_shapes() {
    let mut app = compat_app();
    app.run("view.newTab", &json!({"glyphs": ["A"]})).unwrap();
    let ctx = egui::Context::default();
    let ok = app.tokens.compat_ok;
    assert_eq!(strokes(&frame(&ctx, &mut app), ok), 0, "off at first");
    assert_eq!(app.run("view.showMasterCompatibility", &json!({})).unwrap()["on"], true, "omitted: toggle");
    assert_eq!(strokes(&frame(&ctx, &mut app), ok), 16, "two boxes of four green segments on each of two layers");

    let font = font(&app);
    let glyph = font.glyph("A").unwrap();
    let (thin, black) = (glyph.layer("thin").unwrap(), glyph.layer("black").unwrap());
    let placed = placement(&font, glyph, thin, None);
    assert_eq!(placed.iter().map(|p| p.layer.as_str()).collect::<Vec<_>>(), ["thin", "black"]);
    assert_eq!(placed[0].offset, (0.0, 0.0), "the edited layer stays in place");
    let (dx, dy) = placed[1].offset;
    assert!(dx > thin.width && dy > 0.0, "Black is drawn after Thin's advance and higher: {dx}, {dy}");
    let from_black = placement(&font, glyph, black, None);
    assert_eq!(from_black[1].offset, (0.0, 0.0));
    assert!(from_black[0].offset.0 < 0.0, "editing Black puts Thin to its left");

    let layers = [thin, black];
    let all = links(&font, &placed, &layers, 0, &Selection::default());
    let start = |s: usize| all.iter().position(|l| l.kind == LinkKind::Start(s)).unwrap();
    let (s0, s1) = (start(0), start(1));
    assert_eq!(all[s0].ends[1].at, Point::new(300.0 + dx, dy), "shape 0 on Black is the box at 300: the lines cross");
    assert!(segment_states(black, &[thin]).iter().all(|(_, _, s)| *s == SegmentState::Compatible));

    let onto = all[s1].ends[1].at;
    let params = drop_swap("A", &layers, &all, (s0, 1), onto, 1.0, 0).unwrap();
    assert_eq!(params, json!({"glyph": "A", "layer": "black", "a": 0, "b": 1}));
    assert!(drop_swap("A", &layers, &all, (s0, 0), all[s1].ends[0].at, 1.0, 0).is_none(), "the edited layer's ends stay");
    assert!(drop_swap("A", &layers, &all, (s0, 1), Point::new(onto.x + 50.0, onto.y), 1.0, 0).is_none(), "dropped away from every end");

    app.run("glyph.swapShapes", &params).unwrap();
    let font = self::font(&app);
    let glyph = font.glyph("A").unwrap();
    let layers = [glyph.layer("thin").unwrap(), glyph.layer("black").unwrap()];
    let all = links(&font, &placed, &layers, 0, &Selection::default());
    assert_eq!(all[start(0)].ends[1].at, Point::new(dx, dy), "after the swap shape 0 is the box at 0 on both masters");
}

#[test]
fn an_incompatible_glyph_shows_red_segments_the_bar_and_the_font_view_corner() {
    let mut app = compat_app();
    let font = font(&app);
    let b = font.glyph("B").unwrap();
    let (thin, black) = (b.layer("thin").unwrap(), b.layer("black").unwrap());
    let states = segment_states(black, &[thin]);
    assert_eq!(states.len(), 8);
    assert!(states.iter().filter(|(i, _, _)| *i == 0).all(|(_, _, s)| *s == SegmentState::Compatible));
    assert!(states.iter().filter(|(i, _, _)| *i == 1).all(|(_, _, s)| *s == SegmentState::Incompatible), "Thin has no second path");
    let all = links(&font, &placement(&font, b, thin, None), &[thin, black], 0, &Selection::default());
    let second = all.iter().find(|l| l.kind == LinkKind::Start(1)).unwrap();
    assert!(second.ends[0].missing, "Thin lacks shape 1: its end is Thin's origin");

    // Font View: a red corner in B's cell only.
    let ctx = egui::Context::default();
    let shapes = frame(&ctx, &mut app);
    let corners = |app: &FontcraftApp, shapes: &[Shape], glyph: &str| {
        let cell = app.font_view.laid_out.iter().find(|(n, _)| n == glyph).map(|(_, r)| *r).unwrap();
        let fill = app.tokens.compat_bad;
        shapes.iter().filter(|s| matches!(s, Shape::Path(PathShape { points, fill: f, .. }) if *f == fill && points.len() == 3 && points.iter().all(|p| cell.contains(*p)))).count()
    };
    assert_eq!(corners(&app, &shapes, "B"), 1);
    assert_eq!(corners(&app, &shapes, "A"), 0);

    // Edit View: the red bar over B, not over A.
    app.run("view.newTab", &json!({"glyphs": ["A", "B"]})).unwrap();
    assert_eq!(incompatible_in_tab(&mut app).into_iter().collect::<Vec<_>>(), ["B"]);
    let bad = app.tokens.compat_bad;
    let bars = |shapes: &[Shape]| shapes.iter().filter(|s| matches!(s, Shape::Rect(RectShape { fill, .. }) if *fill == bad)).count();
    assert_eq!(bars(&frame(&ctx, &mut app)), 1);

    // Enforce Compatibility Check unchecked hides both indicators.
    app.session
        .edit("indicators off", |font| {
            Arc::make_mut(&mut font.info).params.set("Enforce Compatibility Check", Plist::Bool(false));
            Ok(())
        })
        .unwrap();
    assert!(incompatible_in_tab(&mut app).is_empty());
    assert_eq!(bars(&frame(&ctx, &mut app)), 0);
}

#[test]
fn edit_keep_layer_selection_in_sync_carries_the_selection_to_another_master() {
    let mut app = compat_app();
    app.run("view.newTab", &json!({"glyphs": ["A"]})).unwrap();
    let select_on_thin = |app: &mut FontcraftApp| {
        app.run("view.selectMaster", &json!({"master": 0})).unwrap();
        tools_glue::sync(app);
        app.run("selection.set", &json!({"points": [[1, 2]]})).unwrap();
        tools_glue::sync(app);
        assert_eq!(app.state.tab().unwrap().selection.points.len(), 1);
        app.run("view.selectMaster", &json!({"master": 1})).unwrap();
        tools_glue::sync(app);
        app.state.tab().unwrap().selection.points.iter().copied().collect::<Vec<_>>()
    };
    assert!(select_on_thin(&mut app).is_empty(), "off: another master starts with no selection");
    assert_eq!(app.run("edit.keepLayerSelectionInSync", &json!({})).unwrap()["on"], true, "omitted: toggle");
    assert_eq!(select_on_thin(&mut app), [(1, 2)], "on: node 2 of shape 1 stays selected on Black");

    // B cannot interpolate: nothing is carried.
    app.run("view.newTab", &json!({"glyphs": ["B"]})).unwrap();
    app.run("view.selectMaster", &json!({"master": 0})).unwrap();
    tools_glue::sync(&mut app);
    app.run("selection.set", &json!({"points": [[0, 0]]})).unwrap();
    tools_glue::sync(&mut app);
    app.run("view.selectMaster", &json!({"master": 1})).unwrap();
    tools_glue::sync(&mut app);
    assert!(app.state.tab().unwrap().selection.is_empty());

    // Items the new layer lacks are dropped.
    let mut selection = Selection::default();
    selection.points.extend([(0, 1), (0, 9), (4, 0)]);
    selection.anchors.insert("top".into());
    let carried = carried_selection(&selection, &layer("black", vec![rect(0.0, 10.0)]));
    assert_eq!(carried.points.into_iter().collect::<Vec<_>>(), [(0, 1)]);
    assert!(carried.anchors.is_empty());
}
