//! Colour glyphs in Edit View and the previews (M11.14), in headless frames and in the pictures
//! they are drawn from: palette layers of other glyphs, a full-colour layer with a gradient and a
//! stroke, an iColor image at its size, an svg layer, the masters of a layered font in their
//! Master Color, the gradient knobs and their drag, the Preview Area, and the frame-time measure.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Color32, Pos2, Shape, TextureId};
use fontcraft_engine::imagery::{Rgba8, to_png};
use fontcraft_geom::{Affine, Point};
use fontcraft_model::Gradient;
use serde_json::json;

use crate::canvas::color::{self, Mode, Sources, SvgCache};
use crate::canvas::{gradient_knobs, view};
use crate::shell::preview::{self, Picture};
use crate::{FontcraftApp, control, tools_glue};

struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    fn frame(&self, app: &mut FontcraftApp) -> Vec<Shape> {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
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

    fn settle(&self, app: &mut FontcraftApp) -> Vec<Shape> {
        for _ in 0..3 {
            self.frame(app);
        }
        self.frame(app)
    }
}

fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-color-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn master(app: &FontcraftApp) -> String {
    app.session.font().unwrap().masters[0].id.clone()
}

/// The non-master layer of `glyph`.
fn colour_layer(app: &FontcraftApp, glyph: &str) -> String {
    app.session.font().unwrap().glyph(glyph).unwrap().layers.iter().find(|l| !l.is_master_layer()).unwrap().id.clone()
}

/// Glyphs `names`, each a square (100,100)–(300,300) with a backup layer converted by
/// `convert`, shown as one text with the cursor on the first, at actual size.
fn colour_app(names: &[&str], convert: &str) -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("colorPalette.addColor", &json!({"color": "#FF0000"})).unwrap();
    for name in names {
        if app.session.font().unwrap().glyph(name).is_none() {
            app.run("glyph.newGlyph", &json!({"name": name})).unwrap();
        }
        app.run("path.addRectangle", &json!({"glyph": name, "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    }
    let m = master(&app);
    app.run("layers.addBackupLayer", &json!({"glyphs": names, "master": m})).unwrap();
    for name in names {
        let layer = colour_layer(&app, name);
        app.run(convert, &json!({"glyph": name, "layer": layer})).unwrap();
    }
    app.run("view.newTab", &json!({"glyphs": [names[0]]})).unwrap();
    app.run("editView.setText", &json!({"glyphs": names})).unwrap();
    app.run("editView.setCursor", &json!({"index": 0})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [300, 200]})).unwrap();
    app
}

fn select_colour_layer(app: &mut FontcraftApp, glyph: &str) -> String {
    let layer = colour_layer(app, glyph);
    app.run("layers.select", &json!({"layer": layer})).unwrap();
    layer
}

/// The colour drawing of `glyph` as the tab's mode draws it, 400 × 400 pixels, one per unit,
/// y up from the bottom.
fn picture(app: &FontcraftApp, glyph: &str, exact: Option<&str>) -> egui::ColorImage {
    let font = app.session.font().unwrap();
    let tab = app.state.tab().unwrap();
    let mode = color::mode(font, tab, false).expect("the tab draws in colour");
    let layer = exact.map(|id| font.glyph(glyph).unwrap().layers.iter().find(|l| l.id == id).unwrap());
    let assets = app.session.assets();
    let mut svgs = SvgCache::default();
    let mut src = Sources { assets: assets.as_ref(), svgs: &mut svgs, revision: 0, text: Color32::BLACK };
    let to_pixels = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 400.0]);
    color::glyph_image(font, glyph, &mode, layer, &mut src, 400, 400, to_pixels).expect("vector content")
}

fn at(image: &egui::ColorImage, x: usize, y: usize) -> Color32 {
    image.pixels[(399 - y) * 400 + x]
}

/// Meshes with a texture other than the fill's: colour glyphs and images.
fn textured(app: &FontcraftApp, shapes: &[Shape]) -> usize {
    let fill = app.fill_cache.texture_id();
    shapes
        .iter()
        .filter(|s| matches!(s, Shape::Mesh(m) if m.texture_id != TextureId::default() && Some(m.texture_id) != fill && m.vertices.len() == 4))
        .count()
}

#[test]
fn color_preview_edit_view_other_glyphs_draw_in_colour_while_a_colour_layer_is_selected() {
    let mut app = colour_app(&["A", "B"], "layer.convertToColorPalette");
    if app.session.font().unwrap().glyph("C").is_none() {
        app.run("glyph.newGlyph", &json!({"name": "C"})).unwrap();
    }
    app.run("path.addRectangle", &json!({"glyph": "C", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.run("editView.setText", &json!({"glyphs": ["A", "B", "C"]})).unwrap();
    app.run("editView.setCursor", &json!({"index": 0})).unwrap();
    let w = Window::new();
    let plain = w.settle(&mut app);
    assert_eq!(textured(&app, &plain), 0, "a master layer selected: ordinary drawing");
    assert!(!color::replaces_fill(app.session.font().unwrap(), app.state.tab().unwrap(), "B"));

    select_colour_layer(&mut app, "A");
    let shapes = w.settle(&mut app);
    assert_eq!(app.color_ui.sprites(), 2, "A and B have colour layers; C has none");
    assert_eq!(textured(&app, &shapes), 2);
    let (font, tab) = (app.session.font().unwrap(), app.state.tab().unwrap());
    assert!(color::replaces_fill(font, tab, "B") && !color::replaces_fill(font, tab, "C"));
    let b = picture(&app, "B", None);
    assert_eq!(at(&b, 200, 200), Color32::from_rgb(255, 0, 0), "palette row 0");
    assert_eq!(at(&b, 50, 50).a(), 0, "nothing outside the outline");

    // The text colour row draws in the canvas's text colour.
    let layer = colour_layer(&app, "B");
    app.run("layer.setColorPaletteIndex", &json!({"glyph": "B", "layer": layer, "index": "*"})).unwrap();
    assert_eq!(at(&picture(&app, "B", None), 200, 200), Color32::BLACK);
}

#[test]
fn color_full_color_layer_draws_its_gradient_and_its_stroke_above_the_fill() {
    let mut app = colour_app(&["A"], "layer.convertToColor");
    let layer = select_colour_layer(&mut app, "A");
    app.run(
        "attributes.addGradient",
        &json!({"glyph": "A", "layer": layer, "stops": [{"offset": 0, "color": "#0000FFFF"}, {"offset": 1, "color": "#0000FF00"}]}),
    )
    .unwrap();
    app.run("attributes.setStrokeWidth", &json!({"glyph": "A", "layer": layer, "width": 20})).unwrap();
    let image = picture(&app, "A", None);
    let left = at(&image, 120, 200);
    let right = at(&image, 280, 200);
    assert!(left.b() > 150 && left.a() > 150, "opaque blue near the start: {left:?}");
    assert!(right.a() < 80, "nearly transparent near the end: {right:?}");
    assert_eq!(at(&image, 100, 200), Color32::BLACK, "the centred black stroke lies on the edge, above the fill");
    // A fill made fully transparent paints nothing.
    app.run("attributes.removeGradient", &json!({"glyph": "A", "layer": layer})).unwrap();
    app.run("attributes.removeFill", &json!({"glyph": "A", "layer": layer})).unwrap();
    assert_eq!(at(&picture(&app, "A", None), 200, 200).a(), 0);
}

#[test]
fn color_sbix_size_scales_the_icolor_image_on_the_canvas() {
    let dir = folder("icolor");
    let file = dir.join("E.png");
    std::fs::write(&file, to_png(&Rgba8 { width: 64, height: 32, pixels: [255, 0, 0, 255].repeat(64 * 32) }).unwrap()).unwrap();
    let mut app = FontcraftApp::default();
    app.run("glyph.newGlyph", &json!({"name": "E.icolor"})).unwrap();
    let m = master(&app);
    app.run("layers.addBackupLayer", &json!({"glyphs": ["E.icolor"], "master": m})).unwrap();
    let layer = colour_layer(&app, "E.icolor");
    app.run("layer.convertToIColor", &json!({"glyph": "E.icolor", "layer": layer, "size": 125})).unwrap();
    app.run("layer.setIColorImage", &json!({"glyph": "E.icolor", "layer": layer, "path": file.to_string_lossy()})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["E.icolor"]})).unwrap();
    app.run("layers.select", &json!({"layer": layer})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [300, 200]})).unwrap();
    let upm = f64::from(app.session.font().unwrap().units_per_em);
    let w = Window::new();
    for size in [125, 250] {
        if size != 125 {
            app.run("layer.setIColorSize", &json!({"glyph": "E.icolor", "layer": layer, "size": size})).unwrap();
        }
        let k = upm / f64::from(size);
        let shapes = w.settle(&mut app);
        let meshes: Vec<&egui::Mesh> = shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Mesh(m) if m.texture_id != TextureId::default() && m.vertices.len() == 4 => Some(&**m),
                _ => None,
            })
            .collect();
        assert_eq!(meshes.len(), 1, "the iColor image, once (not again as a placed image)");
        let mesh = meshes[0];
        assert_eq!(mesh.vertices[0].color.a(), 255, "the image is the glyph: drawn opaque");
        let corner = tools_glue::current_view(&app).unwrap().to_screen(Point::new(64.0 * k, 32.0 * k));
        assert!((mesh.vertices[2].pos - Pos2::new(corner.x as f32, corner.y as f32)).length() < 0.01, "{size} pixels per em: {k} units per pixel");
    }
}

#[test]
fn color_svg_layer_draws_its_file_and_no_missing_image_marker() {
    let dir = folder("svg");
    let file = dir.join("F.svg");
    std::fs::write(
        &file,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect width="100" height="100" fill="#00FF00"/></svg>"##,
    )
    .unwrap();
    let mut app = colour_app(&["F"], "layer.convertToSvg");
    let layer = colour_layer(&app, "F");
    app.run("layer.setSvgImage", &json!({"glyph": "F", "layer": layer, "path": file.to_string_lossy()})).unwrap();
    select_colour_layer(&mut app, "F");
    let shapes = Window::new().settle(&mut app);
    let marker = app.tokens.images.marker;
    assert!(!shapes.iter().any(|s| matches!(s, Shape::Rect(r) if r.stroke.color == marker)), "an svg file is not a missing image");
    assert_eq!(app.color_ui.sprites(), 1);
    let image = picture(&app, "F", Some(&layer));
    assert_eq!(at(&image, 50, 50), Color32::from_rgb(0, 255, 0), "the file's bottom left at the layer origin, a unit per unit");
    assert_eq!(at(&image, 150, 150).a(), 0);
}

#[test]
fn color_svg_layer_draws_its_filters() {
    // A 40-unit bar moved 50 units right by its filter.
    let dir = folder("svg-filter");
    let file = dir.join("F.svg");
    std::fs::write(
        &file,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="100" height="100"><feOffset dx="50" dy="0"/></filter><rect width="40" height="100" fill="#00FF00" filter="url(#f)"/></svg>"##,
    )
    .unwrap();
    let mut app = colour_app(&["F"], "layer.convertToSvg");
    let layer = colour_layer(&app, "F");
    app.run("layer.setSvgImage", &json!({"glyph": "F", "layer": layer, "path": file.to_string_lossy()})).unwrap();
    select_colour_layer(&mut app, "F");
    let shapes = Window::new().settle(&mut app);
    assert_eq!(app.color_ui.sprites(), 1);
    assert_eq!(textured(&app, &shapes), 1, "the filtered file is one texture");
    let image = picture(&app, "F", Some(&layer));
    assert_eq!(at(&image, 70, 50), Color32::from_rgb(0, 255, 0), "moved 50 units right");
    assert_eq!(at(&image, 20, 50).a(), 0, "and not left where it was");
}

/// An app whose glyphs `names` each have an svg layer drawing `svg`, the first glyph's selected.
fn svg_app(tag: &str, names: &[&str], svg: &str) -> FontcraftApp {
    let file = folder(tag).join("F.svg");
    std::fs::write(&file, svg).unwrap();
    let mut app = colour_app(names, "layer.convertToSvg");
    for name in names {
        let layer = colour_layer(&app, name);
        app.run("layer.setSvgImage", &json!({"glyph": name, "layer": layer, "path": file.to_string_lossy()})).unwrap();
    }
    select_colour_layer(&mut app, names[0]);
    app
}

const OFFSET_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="100" height="100"><feOffset dx="50" dy="0"/></filter><rect width="40" height="100" fill="#00FF00" filter="url(#f)"/></svg>"##;

#[test]
fn color_svg_filtered_glyphs_along_a_long_line_stay_within_the_texture_limit() {
    // At 10 pixels a unit each file is a texture of a million pixels: 80 of them would hold
    // 320 MB, past the limit. Only the files on the canvas are drawn, and a still frame draws
    // nothing again.
    let names: Vec<String> = (0..80).map(|i| format!("f{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut app = svg_app("svg-filter-line", &refs, OFFSET_SVG);
    app.run("view.setZoom", &json!({"size": 10_000, "center": [50, 50]})).unwrap();
    let w = Window::new();
    w.settle(&mut app);
    let (made, drawn) = app.color_ui.made();
    assert!((1..=3).contains(&drawn), "the files on the canvas only: {drawn}");
    let bytes = app.color_ui.sprite_bytes();
    assert!(bytes <= 3 * 4 * 1_100_000, "{bytes} bytes");
    let shapes = w.frame(&mut app);
    assert_eq!(app.color_ui.made(), (made, drawn), "a still frame makes nothing");
    assert!(textured(&app, &shapes) >= 1);
    // A glyph further along the line comes into view: its file is drawn, and the sprites stay
    // within the limit.
    let m = master(&app);
    let advance = app.session.font().unwrap().glyph("f0").unwrap().layer(&m).unwrap().width;
    app.run("view.scrollTo", &json!({"x": advance * 10.0 + 50.0, "y": 50})).unwrap();
    w.settle(&mut app);
    let (_, later) = app.color_ui.made();
    assert!(later > drawn && later <= drawn + 3, "{drawn} then {later}");
    assert!(app.color_ui.sprite_bytes() <= crate::canvas::color::MAX_SPRITE_BYTES);
    let still = app.color_ui.made();
    w.frame(&mut app);
    assert_eq!(app.color_ui.made(), still);
}

#[test]
fn color_svg_file_refused_at_a_zoom_is_not_drawn_again_while_scrolling() {
    // Masks nested ten deep take more layers than a drawing may: the file is refused and drawn
    // from its outlines, and the canvas says why.
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><defs><mask id="m"><rect width="100" height="100" fill="#FFFFFF"/></mask><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="100" height="100"><feOffset dx="10" dy="0"/></filter></defs>{}<rect width="50" height="100" fill="#00FF00" filter="url(#f)"/>{}</svg>"##,
        r#"<g mask="url(#m)">"#.repeat(10),
        "</g>".repeat(10)
    );
    let mut app = svg_app("svg-filter-refused", &["F"], &svg);
    // 6,400 pixels across: larger than the canvas, so a long scroll draws the outlines again.
    app.run("view.setZoom", &json!({"size": 64_000, "center": [50, 50]})).unwrap();
    let w = Window::new();
    let shapes = w.settle(&mut app);
    let (made, drawn) = app.color_ui.made();
    assert_eq!(drawn, 1, "the file is tried once");
    let said = shapes.iter().any(|s| matches!(s, Shape::Text(t) if t.galley.text().contains("without its filters")));
    assert!(said, "the canvas says why the file is drawn without its filters");
    for _ in 0..3 {
        app.run("view.scrollTo", &json!({"dx": 600, "dy": 0})).unwrap();
        w.frame(&mut app);
    }
    let (remade, redrawn) = app.color_ui.made();
    assert!(remade > made, "the outlines are drawn again for the new part of the canvas");
    assert_eq!(redrawn, 1, "the refused file is not tried again");
}

#[test]
fn color_layered_masters_draw_in_their_master_color_first_master_on_top() {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.session
        .edit("setup", |font| {
            let mut m = font.default_master()?.clone();
            m.id = "m02".into();
            m.name = "Shadow".into();
            Arc::make_mut(&mut font.masters).push(m);
            font.glyph_mut("A")?.layers.push(fontcraft_model::Layer::new("m02", 600.0));
            Ok(())
        })
        .unwrap();
    app.run("path.addRectangle", &json!({"glyph": "A", "layer": "m02", "x": 200, "y": 200, "width": 200, "height": 200})).unwrap();
    let m1 = master(&app);
    for (id, name, value) in
        [(m1.as_str(), "Master Color", "255,0,0,255"), ("m02", "Master Color", "0,0,255,128"), (m1.as_str(), "Master Color Dark", "0,255,0,255")]
    {
        app.run("font.customParameters.add", &json!({"scope": "master", "targets": id, "name": name, "value": value})).unwrap();
    }
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    let (font, tab) = (app.session.font().unwrap(), app.state.tab().unwrap());
    assert_eq!(
        color::mode(font, tab, false),
        Some(Mode::Layered { masters: vec![(m1.clone(), [1.0, 0.0, 0.0, 1.0])] }),
        "only the selected master is shown"
    );
    app.run("layers.toggleVisibility", &json!({"glyphs": ["A"], "layer": "m02", "visible": true})).unwrap();
    let (font, tab) = (app.session.font().unwrap(), app.state.tab().unwrap());
    let Some(Mode::Layered { masters }) = color::mode(font, tab, false) else { panic!("a layered font") };
    assert_eq!(masters.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["m02", m1.as_str()], "bottom first: the first master on top");
    assert!((masters[0].1[3] - 128.0 / 255.0).abs() < 1e-6, "the Master Color's opacity");
    let Some(Mode::Layered { masters: dark }) = color::mode(font, tab, true) else { panic!("a layered font") };
    assert_eq!(dark[1].1, [0.0, 1.0, 0.0, 1.0], "Master Color Dark in the dark appearance");
    assert_eq!(dark[0].1[2], 1.0, "a master without Master Color Dark keeps its Master Color");
    let image = picture(&app, "A", None);
    assert_eq!(at(&image, 250, 250), Color32::from_rgb(255, 0, 0), "the first master covers the second");
    assert!(at(&image, 350, 350).b() > 0 && at(&image, 350, 350).a() < 255, "the second master, half transparent");
    let shapes = Window::new().settle(&mut app);
    assert_eq!(textured(&app, &shapes), 1, "the edited glyph is filled in colour");
}

#[test]
fn attributes_set_gradient_position_dragging_a_knob_equals_the_command() {
    let mut app = colour_app(&["A"], "layer.convertToColor");
    let layer = select_colour_layer(&mut app, "A");
    app.run("attributes.addGradient", &json!({"glyph": "A", "layer": layer})).unwrap();
    app.run("selection.set", &json!({"shapes": [0]})).unwrap();
    let knobs = gradient_knobs::current(&app);
    let points: Vec<(f64, f64)> = knobs.iter().map(|k| (k.at.x, k.at.y)).collect();
    assert_eq!(points, [(100.0, 200.0), (300.0, 200.0)], "a linear gradient from the left middle to the right middle");
    let shapes = Window::new().settle(&mut app);
    let t = app.tokens.color.clone();
    let drawn = shapes
        .iter()
        .filter(|s| matches!(s, Shape::Circle(c) if c.fill == t.knob && c.radius == t.knob_radius && c.stroke.color == t.knob_outline))
        .count();
    assert_eq!(drawn, 2, "two knobs drawn");

    let undo = app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len);
    let events = json!([
        {"kind": "down", "x": 100, "y": 200},
        {"kind": "drag", "x": 125, "y": 225},
        {"kind": "drag", "x": 150, "y": 250},
        {"kind": "up", "x": 150, "y": 250},
    ]);
    let r = control::handle(&mut app, &json!({"id": 1, "method": "ui.pointer", "params": {"events": events}}));
    assert_eq!(r["result"]["errors"], json!([]), "{r}");
    assert_eq!(app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len), undo + 1, "one undo step");
    let (id, params) = app.session.journal().last().cloned().unwrap();
    assert_eq!(id, "attributes.setGradientPosition");
    assert_eq!((&params["start"], &params["end"]), (&json!([150.0, 250.0]), &json!([300.0, 200.0])), "the start moves, the end stays");

    let gradient = |app: &FontcraftApp| -> Option<Gradient> {
        let font = app.session.font().unwrap();
        let l = font.glyph("A").unwrap().layers.iter().find(|l| l.id == layer).unwrap().clone();
        match &l.shapes[0] {
            fontcraft_model::Shape::Path(p) => p.info.attrs.gradient.clone(),
            _ => None,
        }
    };
    let mut by_command = colour_app(&["A"], "layer.convertToColor");
    let other = colour_layer(&by_command, "A");
    by_command.run("attributes.addGradient", &json!({"glyph": "A", "layer": other})).unwrap();
    by_command
        .run("attributes.setGradientPosition", &json!({"glyph": "A", "layer": other, "shapes": [0], "start": [150, 250], "end": [300, 200]}))
        .unwrap();
    assert_eq!(gradient(&app), {
        let font = by_command.session.font().unwrap();
        match &font.glyph("A").unwrap().layers.iter().find(|l| l.id == other).unwrap().shapes[0] {
            fontcraft_model::Shape::Path(p) => p.info.attrs.gradient.clone(),
            _ => None,
        }
    });
}

#[test]
fn color_preview_preview_area_draws_a_glyph_in_colour_only_while_its_colour_layer_is_shown() {
    let mut app = colour_app(&["A", "B"], "layer.convertToColorPalette");
    select_colour_layer(&mut app, "A");
    let tab = app.state.tab().unwrap().clone();
    let layout = preview::line_layout(&mut app.session, &tab).unwrap();
    let options = app.state.panels.preview.area;
    let rect = egui::Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1000.0, 200.0));
    let font = app.session.font().unwrap();
    let range = preview::vertical_range(font, view::tab_master(font, &tab).unwrap());
    let placement = preview::place(&layout, tab.edited_index(), rect, range, &options, 16.0).unwrap();
    let doc = app.session.active().map(|d| (d.id, d.revision)).unwrap();
    let p = Picture {
        font,
        tab: &tab,
        layout: &layout,
        placement: &placement,
        ppp: 1.0,
        doc,
        options: &options,
        ink: Color32::BLACK,
        paper: Color32::WHITE,
    };
    let assets = app.session.assets();
    let mut svgs = SvgCache::default();
    let mut src = Sources { assets: assets.as_ref(), svgs: &mut svgs, revision: 0, text: Color32::BLACK };
    let image = preview::picture_with(&p, &mut app.fill_cache.outlines, Some(&mut src)).unwrap();
    let pixel = |x: f64, y: f64| {
        let q = placement.to_screen(x, y);
        image.pixels[q.y as usize * image.size[0] + q.x as usize]
    };
    let b_x = layout.placed[1].x;
    assert_eq!(pixel(200.0, 200.0), Color32::from_rgb(255, 0, 0), "A shows its palette layer: red");
    assert_eq!(pixel(b_x + 200.0, 200.0), Color32::BLACK, "B shows its master layer: black");
}

#[test]
fn color_export_colr_glyph_component_draws_the_referenced_glyph_in_colour() {
    let mut app = colour_app(&["B", "A", "E", "F"], "layer.convertToColor");
    let a = colour_layer(&app, "A");
    app.run("attributes.setFillColor", &json!({"glyph": "A", "layer": a, "color": "#FF0000"})).unwrap();
    // B's full-colour layer holds only a component of A moved right by 50; E's and F's each a
    // component of the other, which would be a cycle.
    let component = |glyph: &str, dx: f64| {
        let mut c = fontcraft_model::Component::new(glyph, 0.0, 0.0);
        c.transform = fontcraft_model::Transform::Matrix([1.0, 0.0, 0.0, 1.0, dx, 0.0]);
        fontcraft_model::Shape::Component(c)
    };
    let edits: Vec<(&str, String, fontcraft_model::Shape)> =
        [("B", "A", 50.0), ("E", "F", 0.0), ("F", "E", 0.0)].into_iter().map(|(g, t, dx)| (g, colour_layer(&app, g), component(t, dx))).collect();
    app.session
        .edit("components", |font| {
            for (g, layer, shape) in edits {
                let mut glyph = font.glyph_mut(g).unwrap();
                glyph.layers.iter_mut().find(|l| l.id == layer).unwrap().shapes = vec![shape];
            }
            Ok(())
        })
        .unwrap();
    app.run("editView.setText", &json!({"glyphs": ["B", "A"]})).unwrap();
    app.run("editView.setCursor", &json!({"index": 0})).unwrap();
    select_colour_layer(&mut app, "B");
    let w = Window::new();
    let shapes = w.settle(&mut app);
    assert_eq!(app.color_ui.sprites(), 2);
    assert_eq!(textured(&app, &shapes), 2);

    // A's square (100–300) is drawn red where B's component puts it, not as a grey outline.
    let b = picture(&app, "B", None);
    assert_eq!(at(&b, 320, 200), Color32::from_rgb(255, 0, 0), "A's red, moved by the component");
    assert_eq!(at(&b, 200, 200), Color32::from_rgb(255, 0, 0));
    assert_eq!(at(&b, 120, 200).a(), 0, "nothing where the square was before the move");

    // The cycle falls back to the component's outline in grey, as the compile does, with a warning.
    let e = picture(&app, "E", None);
    let grey = at(&e, 200, 200);
    assert!(grey.a() == 255 && grey.r() == grey.g() && grey.g() == grey.b() && (120..=136).contains(&grey.r()), "{grey:?}");
    let font = app.session.font().unwrap();
    let m = master(&app);
    let mut warnings = Vec::new();
    let graph = fontcraft_engine::color::paint::MasterPaints::new(font, &m).paint(font.glyph("E").unwrap(), &mut warnings).unwrap();
    assert!(graph.is_some() && warnings.iter().any(|w| w.contains("lead back")), "{warnings:?}");
}

/// § 5 of the M11 plan: an Edit View frame with 50 full-colour glyphs under 16 ms. Run with
/// `cargo test --profile measure -p fontcraft-ui-egui color_full_color_frame_speed -- --ignored --nocapture`.
#[test]
#[ignore = "speed measure"]
fn color_full_color_frame_speed() {
    let names: Vec<String> = (0..50).map(|i| format!("g{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut app = colour_app(&refs, "layer.convertToColor");
    for name in &refs {
        let layer = colour_layer(&app, name);
        app.run("attributes.addGradient", &json!({"glyph": name, "layer": layer})).unwrap();
        app.run("attributes.setStrokeWidth", &json!({"glyph": name, "layer": layer, "width": 12})).unwrap();
    }
    select_colour_layer(&mut app, "g0");
    app.run("view.setZoom", &json!({"size": 60, "center": [6000, 300]})).unwrap();
    let w = Window::new();
    let started = std::time::Instant::now();
    w.frame(&mut app);
    let cold = started.elapsed();
    for _ in 0..3 {
        w.frame(&mut app);
    }
    let runs = 20;
    let started = std::time::Instant::now();
    for _ in 0..runs {
        w.frame(&mut app);
    }
    let steady = started.elapsed() / runs;
    let started = std::time::Instant::now();
    for _ in 0..runs {
        app.run("view.scrollTo", &json!({"dx": 3, "dy": 0})).unwrap();
        w.frame(&mut app);
    }
    let scrolling = started.elapsed() / runs;
    let layer = colour_layer(&app, "g7");
    app.run("attributes.setStrokeWidth", &json!({"glyph": "g7", "layer": layer, "width": 30})).unwrap();
    let started = std::time::Instant::now();
    w.frame(&mut app);
    let after_edit = started.elapsed();
    assert_eq!(app.color_ui.sprites(), 50);
    println!("50 full-colour glyphs: first frame {cold:?}, steady {steady:?}, scrolling {scrolling:?}, after editing one glyph {after_edit:?}");
    if !cfg!(debug_assertions) {
        assert!(steady.as_secs_f64() < 0.016 && scrolling.as_secs_f64() < 0.016, "steady {steady:?}, scrolling {scrolling:?}");
    }
}
