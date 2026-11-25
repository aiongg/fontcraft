//! Colour drawing: the work budget, the texture scale, the pictures the canvas, its text and Font View project, iColor
//! and svg layers, and the gradient knobs.

use std::sync::Arc;

use craft::host::Host;
use fontcraft_engine::Session;
use fontcraft_engine::color::MemoryAssets;
use fontcraft_geom::{Affine, BezPath, Point};
use fontcraft_model::{Extend, Font, Glyph, Gradient, GradientKind, Image, Layer, Path, Shape};
use fontcraft_render::color::artwork::{Drawable, Item, Leaf};
use serde_json::{Value, json};

use super::knobs::{End, invocation, knobs};
use super::paint::{rasterise, texture_scale};
use super::{ColorFiles, Content, Mode, Sources, TEXT, content, text_colors};
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};

/// A 2 × 2 PNG, every pixel opaque red.
const RED_PNG: [u8; 74] = [
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 6, 0, 0, 0, 114, 182, 13, 36, 0, 0, 0, 17, 73, 68, 65,
    84, 120, 156, 99, 248, 207, 192, 240, 31, 132, 25, 96, 12, 0, 71, 202, 7, 249, 103, 89, 110, 183, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

fn square(x: f64, size: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((x, 0.0));
    p.line_to((x + size, 0.0));
    p.line_to((x + size, size));
    p.line_to((x, size));
    p.close_path();
    p
}

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;
    pixels[at..at + 4].try_into().unwrap()
}

#[test]
fn painting_past_the_work_budget_fills_the_outlines_plainly() {
    let red = Leaf::Solid([1.0, 0.0, 0.0, 1.0]);
    let items: Vec<Item> = (0..20_000).map(|_| Item::Fill(Drawable { clips: vec![square(0.0, 64.0)], leaf: red.clone() })).collect();
    let green = [0.0, 1.0, 0.0, 1.0];
    let (pixels, drawn) = rasterise(&items, Affine::IDENTITY, 64, 64, green);
    assert!(!drawn, "the budget runs out");
    assert_eq!(pixel(&pixels, 64, 32, 32), [0, 255, 0, 255], "the outline is filled in the plain colour");
    let (pixels, drawn) = rasterise(&items[..2], Affine::IDENTITY, 64, 64, green);
    assert!(drawn);
    assert_eq!(pixel(&pixels, 64, 32, 32), [255, 0, 0, 255], "within the budget the artwork itself is drawn");
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
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    app
}

fn run(app: &mut FontCraftApp, id: &str, params: Value) -> Value {
    app.run_now(Msg::run(id, params), false).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Adds a full-colour layer to `glyph` holding a 200-unit square at (100, 100) filled red; its id.
fn red_square_layer(app: &mut FontCraftApp, glyph: &str) -> String {
    let master = app.session.font().unwrap().masters[0].id.clone();
    run(app, "layers.addBackupLayer", json!({"glyphs": [glyph], "master": master}));
    let layer = app.session.font().unwrap().glyph(glyph).unwrap().layers.iter().find(|l| !l.is_master_layer()).unwrap().id.clone();
    run(app, "layer.convertToColor", json!({"glyph": glyph, "layer": layer}));
    run(app, "path.addRectangle", json!({"glyph": glyph, "layer": layer, "x": 100, "y": 100, "width": 200, "height": 200}));
    run(app, "attributes.setFillColor", json!({"glyph": glyph, "layer": layer, "shapes": [0], "color": "#FF0000FF"}));
    layer
}

fn items(content: &Content) -> &[Item] {
    match content {
        Content::Items(items) => items,
        Content::Bitmap { .. } => panic!("vector content expected"),
    }
}

/// A full-colour layer shown in the Layers panel draws its fills through the artwork, shadows included; the master
/// layer draws nothing in colour.
#[test]
fn a_shown_full_colour_layer_draws_its_artwork() {
    let mut app = app();
    let layer = red_square_layer(&mut app, "A");
    app.project();
    assert!(app.models.canvas.color.is_none(), "the master layer is shown");

    app.view.layers.layers.insert("A".into(), layer.clone());
    app.view_changed();
    app.project();
    let picture = app.models.canvas.color.clone().expect("the colour layer is drawn");
    let to_pixels = Affine::new([0.1, 0.0, 0.0, -0.1, -10.0, 30.0]);
    let (pixels, drawn) = rasterise(items(&picture.art.content), to_pixels, 20, 20, TEXT);
    assert!(drawn);
    assert_eq!(pixel(&pixels, 20, 10, 10), [255, 0, 0, 255], "the square's middle is red");

    let key = picture.art.key;
    run(&mut app, "attributes.addAttribute", json!({"glyph": "A", "layer": layer, "shapes": [0], "attribute": "shadow"}));
    app.project();
    let picture = app.models.canvas.color.clone().unwrap();
    assert_ne!(picture.art.key, key, "a new drawing gets a new texture key");
    assert!(items(&picture.art.content).iter().any(|i| matches!(i, Item::Shadow { .. })), "the shadow is part of the artwork");
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
    let blue = Leaf::Solid([0.0, 0.0, 1.0, 1.0]);
    assert!(items(&picture.art.content).iter().any(|i| matches!(i, Item::Fill(d) if d.leaf == blue)));
}

/// While the edited glyph shows a colour layer, the text's other glyphs with colour layers in that master are drawn
/// in colour at their places; a glyph without colour layers is not.
#[test]
fn the_texts_other_colour_glyphs_are_drawn_in_colour() {
    let mut app = app();
    let a = red_square_layer(&mut app, "A");
    red_square_layer(&mut app, "B");
    app.view.layers.layers.insert("A".into(), a);
    let mut files = ColorFiles::default();
    let placed =
        text_colors(&app.session, &app.view, &mut files, &[("B", None, [600.0, 0.0]), ("C", None, [1200.0, 0.0]), ("B", None, [1800.0, 0.0])]);
    assert_eq!(placed.iter().map(|p| p.at).collect::<Vec<_>>(), vec![[600.0, 0.0], [1800.0, 0.0]]);
    assert!(Arc::ptr_eq(&placed[0].art, &placed[1].art), "one drawing per glyph name");
    app.view.layers.layers.clear();
    assert!(text_colors(&app.session, &app.view, &mut files, &[("B", None, [600.0, 0.0])]).is_empty(), "the master layer is shown");
}

/// An iColor layer draws its image placed by its transform; an svg layer draws the file's outlines in their colours.
#[test]
fn icolor_and_svg_layers_draw_their_files() {
    let mut assets = MemoryAssets::new();
    assets.insert("red.png", RED_PNG.to_vec());
    let svg =
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect x="0" y="0" width="50" height="50" fill="#00ff00"/></svg>"##;
    assets.insert("green.svg", svg.to_vec());
    let mut files = ColorFiles::default();
    let mut src = Sources { assets: &assets, files: &mut files, text: TEXT };
    let font = Font::new("Test");
    let mut glyph = Glyph::new("x");
    let mut icolor = Layer::new("m01", 600.0);
    icolor.id = "icolor".into();
    icolor.info.attrs.sbix_size = Some(64);
    icolor.info.image =
        Some(Image { path: Some("red.png".into()), transform: fontcraft_model::Transform::translate(10.0, 20.0), ..Image::default() });
    let mut svg_layer = Layer::new("m01", 600.0);
    svg_layer.id = "svg".into();
    svg_layer.info.attrs.svg = true;
    svg_layer.info.image = Some(Image { path: Some("green.svg".into()), ..Image::default() });
    glyph.layers.extend([icolor.clone(), svg_layer.clone()]);
    let mode = Mode::Layers { master: "m01".into(), kind: fontcraft_engine::color::layers::ColorLayerKind::IColor(64) };

    match content(&font, &glyph, &mode, Some(&icolor), &mut src) {
        Some(Content::Bitmap { bitmap, place }) => {
            assert_eq!((bitmap.image.width, bitmap.image.height), (2, 2));
            assert_eq!(&bitmap.image.pixels[..4], &[255, 0, 0, 255]);
            assert_eq!(place * Point::new(0.0, 0.0), Point::new(10.0, 20.0), "placed by the image transform");
        }
        other => panic!("the image is drawn: {other:?}"),
    }
    let drawn = content(&font, &glyph, &mode, Some(&svg_layer), &mut src).expect("the svg layer is drawn");
    let green = Leaf::Solid([0.0, 1.0, 0.0, 1.0]);
    assert!(items(&drawn).iter().any(|i| matches!(i, Item::Fill(d) if d.leaf == green)), "{drawn:?}");
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
