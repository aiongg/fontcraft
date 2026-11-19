//! Colour drawing: gradients, the work budget, the picture the canvas projects, and the gradient knobs.

use std::sync::Arc;

use craft::host::Host;
use fontcraft_engine::Session;
use fontcraft_engine::color::paint::{ColorLine, ColorRef, ColorStop, Paint};
use fontcraft_geom::{Affine, BezPath, Point};
use fontcraft_model::{Extend, Gradient, GradientKind, Layer, Path, Shape};
use serde_json::{Value, json};

use super::content::{Drawable, Leaf, leaf};
use super::knobs::{End, invocation, knobs};
use super::paint::{rasterise, texture_scale};
use super::raster::{Pixels, eval, paint_drawables_within};
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};

fn line(a: [f64; 4], b: [f64; 4]) -> ColorLine {
    ColorLine {
        stops: vec![ColorStop { offset: 0.0, color: ColorRef::Rgba(a) }, ColorStop { offset: 1.0, color: ColorRef::Rgba(b) }],
        extend: Extend::Pad,
    }
}

fn square(x: f64, size: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((x, 0.0));
    p.line_to((x + size, 0.0));
    p.line_to((x + size, size));
    p.line_to((x, size));
    p.close_path();
    p
}

#[test]
fn gradients_follow_their_geometry_and_extend_mode() {
    let red = [1.0, 0.0, 0.0, 1.0];
    let blue = [0.0, 0.0, 1.0, 1.0];
    // Left to right over 0..100, isolines vertical.
    let p = Paint::Linear { p0: Point::new(0.0, 0.0), p1: Point::new(100.0, 0.0), p2: Point::new(0.0, -100.0), line: line(red, blue) };
    let l = leaf(&p, &[], [0.0; 4]).unwrap();
    assert_eq!(eval(&l, Point::new(-50.0, 30.0)), Some([1.0, 0.0, 0.0, 1.0]), "padded before the start");
    let mid = eval(&l, Point::new(50.0, 999.0)).unwrap();
    assert!((mid[0] - 0.5).abs() < 1e-6 && (mid[2] - 0.5).abs() < 1e-6, "{mid:?}");
    let mut repeat = line(red, blue);
    repeat.extend = Extend::Repeat;
    let r = Paint::Radial { c0: Point::new(0.0, 0.0), r0: 0.0, c1: Point::new(0.0, 0.0), r1: 10.0, line: repeat };
    let l = leaf(&r, &[], [0.0; 4]).unwrap();
    let again = eval(&l, Point::new(12.5, 0.0)).unwrap();
    assert!((again[2] - 0.25).abs() < 1e-5, "repeats past the end circle: {again:?}");
    let sweep = Paint::Sweep { center: Point::new(0.0, 0.0), start: 0.0, end: 360.0, line: line(red, blue) };
    let l = leaf(&sweep, &[], [0.0; 4]).unwrap();
    let quarter = eval(&l, Point::new(0.0, 10.0)).unwrap();
    assert!((quarter[2] - 0.25).abs() < 1e-5, "counter-clockwise from the x axis: {quarter:?}");
}

#[test]
fn painting_past_the_work_budget_fills_the_outlines_plainly() {
    let red = Leaf::Solid([1.0, 0.0, 0.0, 1.0]);
    let list: Vec<Drawable> = (0..1_000).map(|_| Drawable { clips: vec![square(0.0, 64.0)], leaf: red.clone() }).collect();
    let green = [0.0, 1.0, 0.0, 1.0];
    let mut data = vec![0u8; 64 * 64 * 4];
    let all = paint_drawables_within(&mut Pixels { data: &mut data, width: 64, height: 64 }, &list, Affine::IDENTITY, 100 * 64 * 64 * 2, green);
    assert!(!all, "the budget runs out");
    assert!(data.chunks(4).all(|p| p == [0, 255, 0, 255]), "the whole outline is filled in the plain colour");
}

#[test]
fn the_texture_scale_steps_by_half_octaves_and_stays_within_its_limits() {
    let b = fontcraft_geom::Rect::new(0.0, 0.0, 500.0, 500.0);
    let (s, w, _) = texture_scale(0.9, b).unwrap();
    assert!((s - 1.0).abs() < 1e-9 && w == 500, "0.9 rounds up to the step at 1: {s}");
    let (s, w, h) = texture_scale(1000.0, b).unwrap();
    assert!(f64::from(w) * f64::from(h) <= 4_194_304.0 + 2.0 * 2049.0 && s < 1000.0, "{w} × {h}");
    assert!(texture_scale(f64::NAN, b).is_none());
}

fn app() -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    app
}

fn run(app: &mut FontCraftApp, id: &str, params: Value) -> Value {
    app.run_now(Msg::run(id, params), false).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A full-colour layer of glyph A holding a 200-unit square filled red, shown in the Layers panel, draws the square
/// in red; the master layer draws nothing in colour.
#[test]
fn a_shown_full_colour_layer_draws_its_fills() {
    let mut app = app();
    let master = app.session.font().unwrap().masters[0].id.clone();
    run(&mut app, "layers.addBackupLayer", json!({"glyphs": ["A"], "master": master}));
    let layer = app.session.font().unwrap().glyph("A").unwrap().layers.iter().find(|l| !l.is_master_layer()).unwrap().id.clone();
    run(&mut app, "layer.convertToColor", json!({"glyph": "A", "layer": layer}));
    run(&mut app, "path.addRectangle", json!({"glyph": "A", "layer": layer, "x": 100, "y": 100, "width": 200, "height": 200}));
    run(&mut app, "attributes.setFillColor", json!({"glyph": "A", "layer": layer, "shapes": [0], "color": "#FF0000FF"}));
    app.project();
    assert!(app.models.canvas.color.is_none(), "the master layer is shown");

    app.view.layers.layers.insert("A".into(), layer);
    app.view_changed();
    app.project();
    let picture = app.models.canvas.color.clone().expect("the colour layer is drawn");
    let b = fontcraft_geom::Rect::new(100.0, 100.0, 300.0, 300.0);
    let pixels = rasterise(&picture, b, 0.1, 20, 20);
    let centre = (10 * 20 + 10) * 4;
    assert_eq!(pixels[centre..centre + 4], [255, 0, 0, 255], "the square's middle is red");
}

/// A master with a Master Color fills the glyph's master layer in that colour (a layered font).
#[test]
fn a_master_color_fills_the_master_layer() {
    let mut app = app();
    let master = app.session.font().unwrap().masters[0].id.clone();
    run(&mut app, "path.addRectangle", json!({"glyph": "A", "x": 0, "y": 0, "width": 100, "height": 100}));
    run(&mut app, "font.customParameters.add", json!({"scope": "master", "targets": master, "name": "Master Color", "value": "0,0,255,255"}));
    app.project();
    let picture = app.models.canvas.color.clone().expect("a layered font draws in colour");
    assert!(picture.fills.iter().any(|d| d.leaf == Leaf::Solid([0.0, 0.0, 1.0, 1.0])), "{:?}", picture.fills.len());
}

#[test]
fn knobs_follow_the_gradient_and_drag_its_ends() {
    let mut layer = Layer::new("m01", 600.0);
    layer.info.attrs.full_color = true;
    let mut path = Path::rect(100.0, 100.0, 300.0, 200.0);
    path.info.attrs.gradient = Some(Gradient {
        kind: GradientKind::Linear { start: Point::new(0.0, 0.5), end: Point::new(1.0, 0.5) },
        stops: Vec::new(),
        extend: Extend::Pad,
    });
    layer.shapes.push(Shape::Path(path));
    let list = knobs(&layer, &[0], false);
    assert_eq!(list.len(), 2);
    assert_eq!((list[0].end, list[0].at), (End::Start, Point::new(100.0, 150.0)));
    assert_eq!(list[1].at, Point::new(300.0, 150.0));
    assert!(knobs(&layer, &[0], true).is_empty(), "no knobs while the background is edited");
    let params = invocation("A", "m01", &list[1], Point::new(250.0, 200.0));
    assert_eq!(params, json!({"glyph": "A", "layer": "m01", "shapes": [0], "start": [100.0, 150.0], "end": [250.0, 200.0]}));
    layer.info.attrs.full_color = false;
    assert!(knobs(&layer, &[0], false).is_empty(), "a layer that is not full colour has none");
}
