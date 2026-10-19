//! Placed images and colour glyphs in Font View cells (M11.16), in headless frames: an image
//! behind the outline, View › Show Image and empty glyphs, the missing-image marker, the decode
//! cap per frame, a palette glyph and a layered font in colour, files dropped on cells, and the
//! frame-time measure. Image files are PNGs written by the test.

use std::path::{Path, PathBuf};

use egui::{Color32, Event, Pos2, Rect, Shape, TextureId};
use fontcraft_engine::color::layers::ColorLayerKind;
use fontcraft_engine::imagery::{Rgba8, to_png};
use fontcraft_model::Image;
use serde_json::json;

use super::{MAX_DECODES_PER_FRAME, colour_image, layered_mode};
use crate::FontcraftApp;
use crate::canvas::color::{Mode, Sources, SvgCache};
use crate::shell::font_view::images::Frame;

/// A headless window: one egui context kept across frames.
struct Window {
    ctx: egui::Context,
    size: egui::Vec2,
}

impl Window {
    fn new() -> Self {
        Self::sized(1280.0, 800.0)
    }

    fn sized(width: f32, height: f32) -> Self {
        Self { ctx: egui::Context::default(), size: egui::vec2(width, height) }
    }

    /// One frame; the shapes it drew in paint order, nested lists flattened.
    fn frame_with(&self, app: &mut FontcraftApp, input: egui::RawInput) -> Vec<Shape> {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)), ..input };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
        fn flatten(out: &mut Vec<Shape>, shape: Shape) {
            match shape {
                Shape::Vec(list) => list.into_iter().for_each(|s| flatten(out, s)),
                other => out.push(other),
            }
        }
        let mut flat = Vec::new();
        for clipped in output.shapes {
            flatten(&mut flat, clipped.shape);
        }
        flat
    }

    fn frame(&self, app: &mut FontcraftApp) -> Vec<Shape> {
        self.frame_with(app, egui::RawInput::default())
    }

    fn settle(&self, app: &mut FontcraftApp) -> Vec<Shape> {
        for _ in 0..4 {
            self.frame(app);
        }
        self.frame(app)
    }
}

/// A file dropped on the window, as the desktop integration hands it over.
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

/// A folder of its own for one test.
fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-cell-images-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A `side × side` PNG of one colour at `dir/name`; its path.
fn png(dir: &Path, name: &str, side: u32, rgba: [u8; 4]) -> String {
    let pixels = rgba.repeat((side * side) as usize);
    let bytes = to_png(&Rgba8 { width: side, height: side, pixels }).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

fn master(app: &FontcraftApp) -> String {
    app.session.font().unwrap().masters[0].id.clone()
}

fn image(app: &FontcraftApp, glyph: &str) -> Option<Image> {
    let font = app.session.font().unwrap();
    font.glyph(glyph).unwrap().layer(&font.masters[0].id).unwrap().info.image.clone()
}

/// Where the last frame laid out the cell of `glyph`.
fn cell(app: &FontcraftApp, glyph: &str) -> Rect {
    app.font_view.laid_out.iter().find(|(n, _)| n == glyph).unwrap_or_else(|| panic!("{glyph} was not laid out")).1
}

/// What the textured quads drawn inside `rect` are, in paint order: `outline` for the glyph's
/// ordinary fill, `colour` for a colour glyph, `image` for a placed image.
fn quads(app: &FontcraftApp, shapes: &[Shape], rect: Rect) -> Vec<&'static str> {
    shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Mesh(m) if m.texture_id != TextureId::default() && m.vertices.len() == 4 && rect.expand(1.0).contains_rect(m.calc_bounds()) => {
                let c = m.vertices[0].color;
                Some(if c == app.tokens.glyph_fill {
                    "outline"
                } else if c == Color32::WHITE {
                    "colour"
                } else {
                    "image"
                })
            }
            _ => None,
        })
        .collect()
}

fn rectangle(app: &mut FontcraftApp, glyph: &str) {
    app.run("path.addRectangle", &json!({"glyph": glyph, "x": 100, "y": 0, "width": 300, "height": 500})).unwrap();
}

#[test]
fn font_view_cell_image_display_draws_the_placed_image_behind_the_outline() {
    let dir = folder("display");
    let file = png(&dir, "A.png", 200, [255, 0, 0, 255]);
    let mut app = FontcraftApp::default();
    rectangle(&mut app, "A");
    app.run("glyph.addImage", &json!({"files": [file], "glyphs": ["A"]})).unwrap();
    let w = Window::new();
    let shapes = w.settle(&mut app);
    assert_eq!(quads(&app, &shapes, cell(&app, "A")), ["image", "outline"], "the image under the outline");
    let a = cell(&app, "A");
    let mesh = shapes
        .iter()
        .find_map(|s| match s {
            Shape::Mesh(m)
                if m.texture_id != TextureId::default()
                    && a.expand(1.0).contains_rect(m.calc_bounds())
                    && m.vertices.first().is_some_and(|v| v.color != app.tokens.glyph_fill && v.color != Color32::WHITE) =>
            {
                Some(m)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(mesh.vertices[0].color, Color32::WHITE.gamma_multiply(0.5), "at the default opacity, as in Edit View");
    assert!(quads(&app, &shapes, cell(&app, "B")).is_empty(), "B has neither image nor outline");
    assert_eq!(app.font_view.art.thumbs(), 1);
}

#[test]
fn font_view_image_show_image_option_hides_images_except_on_empty_glyphs() {
    let dir = folder("show");
    let files = [png(&dir, "A.png", 100, [255, 0, 0, 255]), png(&dir, "B.png", 100, [0, 255, 0, 255])];
    let mut app = FontcraftApp::default();
    rectangle(&mut app, "A");
    app.run("glyph.addImage", &json!({"files": files})).unwrap();
    let w = Window::new();
    let shown = w.settle(&mut app);
    assert_eq!(quads(&app, &shown, cell(&app, "A")), ["image", "outline"]);
    assert_eq!(quads(&app, &shown, cell(&app, "B")), ["image"]);
    app.run("view.showImage", &json!({"on": false})).unwrap();
    let hidden = w.settle(&mut app);
    assert_eq!(quads(&app, &hidden, cell(&app, "A")), ["outline"], "A has a path: its image is hidden");
    assert_eq!(quads(&app, &hidden, cell(&app, "B")), ["image"], "B is empty: its image shows regardless");
}

#[test]
fn font_view_cell_image_display_draws_the_marker_for_a_missing_file_and_reads_it_once() {
    let dir = folder("missing");
    let file = png(&dir, "A.png", 100, [255, 0, 0, 255]);
    let mut app = FontcraftApp::default();
    app.run("glyph.addImage", &json!({"files": [file], "glyphs": ["A"]})).unwrap();
    std::fs::remove_file(&file).unwrap();
    let w = Window::new();
    let shapes = w.settle(&mut app);
    let marker = app.tokens.images.marker;
    let a = cell(&app, "A");
    assert!(
        shapes.iter().any(|s| matches!(s, Shape::Rect(r) if r.stroke.color == marker && a.contains_rect(r.rect))),
        "the missing-image marker inside A's cell"
    );
    assert!(quads(&app, &shapes, a).is_empty());
    assert_eq!(app.font_view.art.decoded(), 0, "a file that failed is not read again every frame");
}

#[test]
fn font_view_cell_image_display_decodes_a_few_files_per_frame() {
    let dir = folder("cap");
    let names: Vec<String> = ('A'..='L').map(String::from).collect();
    let files: Vec<String> = names.iter().map(|n| png(&dir, &format!("{n}.png"), 64, [0, 0, 255, 255])).collect();
    let mut app = FontcraftApp::default();
    app.run("glyph.addImage", &json!({"files": files})).unwrap();
    let w = Window::new();
    w.frame(&mut app);
    assert_eq!(app.font_view.art.decoded(), MAX_DECODES_PER_FRAME, "the first frame decodes its share");
    assert!(app.font_view.art.thumbs() < names.len(), "and leaves the rest for later frames");
    for _ in 0..names.len() {
        w.frame(&mut app);
    }
    assert_eq!(app.font_view.art.thumbs(), names.len(), "later frames decode the rest");
    let shapes = w.frame(&mut app);
    assert_eq!(app.font_view.art.decoded(), 0);
    for n in &names {
        assert_eq!(quads(&app, &shapes, cell(&app, n)), ["image"], "{n}");
    }
}

#[test]
fn color_preview_palette_drives_font_view_draws_palette_glyphs_in_colour() {
    let mut app = FontcraftApp::default();
    app.run("colorPalette.addColor", &json!({"color": "#FF0000"})).unwrap();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": -2000, "y": -2000, "width": 5000, "height": 5000})).unwrap();
    rectangle(&mut app, "B");
    let m = master(&app);
    app.run("layers.addBackupLayer", &json!({"glyphs": ["A"], "master": m})).unwrap();
    let layer = app.session.font().unwrap().glyph("A").unwrap().layers.iter().find(|l| !l.is_master_layer()).unwrap().id.clone();
    app.run("layer.convertToColorPalette", &json!({"glyph": "A", "layer": layer})).unwrap();
    let w = Window::new();
    let shapes = w.settle(&mut app);
    assert_eq!(quads(&app, &shapes, cell(&app, "A")), ["colour"], "A in colour, in place of its outline");
    assert_eq!(quads(&app, &shapes, cell(&app, "B")), ["outline"], "B has no colour layer");
    assert_eq!(app.font_view.art.sprites(), 1);

    let font = app.session.font().unwrap();
    let assets = app.session.assets();
    let mut svgs = SvgCache::default();
    let mut src = Sources { assets: assets.as_ref(), svgs: &mut svgs, revision: 0, text: Color32::BLACK };
    let mode = Mode::Layers { master: m.clone(), kind: ColorLayerKind::FullColor };
    let width = font.glyph("A").unwrap().layer(&m).unwrap().width;
    let picture = colour_image(font, "A", &mode, &mut src, Frame::of(font, &m), width, 64).expect("A draws in colour");
    assert_eq!(picture.pixels[32 * 64 + 32], Color32::from_rgb(255, 0, 0), "palette 0, entry 0");

    // An edit of another glyph keeps A's texture.
    rectangle(&mut app, "C");
    w.frame(&mut app);
    assert_eq!(app.font_view.art.coloured, 0, "A's texture is kept");
}

#[test]
fn color_preview_palette_drives_font_view_layered_masters_in_their_master_color() {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": -2000, "y": -2000, "width": 5000, "height": 5000})).unwrap();
    let m = master(&app);
    app.run("font.customParameters.add", &json!({"scope": "master", "targets": m, "name": "Master Color", "value": "0,0,255,255"})).unwrap();
    let w = Window::new();
    let shapes = w.settle(&mut app);
    assert_eq!(quads(&app, &shapes, cell(&app, "A")), ["colour"], "every glyph in its master's colour");
    let font = app.session.font().unwrap();
    let mode = layered_mode(font, &m, false).expect("the master has a Master Color");
    let assets = app.session.assets();
    let mut svgs = SvgCache::default();
    let mut src = Sources { assets: assets.as_ref(), svgs: &mut svgs, revision: 0, text: Color32::BLACK };
    let width = font.glyph("A").unwrap().layer(&m).unwrap().width;
    let picture = colour_image(font, "A", &mode, &mut src, Frame::of(font, &m), width, 64).unwrap();
    assert_eq!(picture.pixels[32 * 64 + 32], Color32::from_rgb(0, 0, 255));
}

#[test]
fn font_view_image_drop_on_cell_equals_add_image() {
    let dir = folder("drop");
    let file = png(&dir, "photo.png", 40, [255, 0, 0, 255]);
    let mut app = FontcraftApp::default();
    let w = Window::new();
    w.settle(&mut app);
    let at = cell(&app, "B").center();
    let dropped: egui::DroppedFileHandle = std::sync::Arc::new(Dropped(PathBuf::from(&file)));
    w.frame_with(&mut app, egui::RawInput { events: vec![Event::PointerMoved(at)], dropped_files: vec![dropped], ..Default::default() });
    let shapes = w.settle(&mut app);
    assert!(image(&app, "A").is_none());
    let placed = image(&app, "B").expect("the image is placed on B");
    let mut expected = FontcraftApp::default();
    let m = master(&expected);
    expected.run("glyph.addImage", &json!({"files": [file], "glyphs": ["B"], "layer": m})).unwrap();
    assert_eq!(Some(placed), image(&expected, "B"), "the same as Add Image on B's layer of Font View's master");
    assert_eq!(quads(&app, &shapes, cell(&app, "B")), ["image"], "and drawn");
}

#[test]
fn image_drag_and_drop_several_files_on_font_view_go_to_the_glyphs_they_name() {
    let dir = folder("drop-many");
    let files = [png(&dir, "A.png", 40, [255, 0, 0, 255]), png(&dir, "C.png", 40, [0, 255, 0, 255]), png(&dir, "nothing.png", 40, [0, 0, 255, 255])];
    let mut app = FontcraftApp::default();
    let w = Window::new();
    w.settle(&mut app);
    let at = cell(&app, "B").center();
    let dropped: Vec<egui::DroppedFileHandle> =
        files.iter().map(|f| std::sync::Arc::new(Dropped(PathBuf::from(f))) as egui::DroppedFileHandle).collect();
    w.frame_with(&mut app, egui::RawInput { events: vec![Event::PointerMoved(at)], dropped_files: dropped, ..Default::default() });
    w.settle(&mut app);
    assert!(image(&app, "A").is_some_and(|i| i.path.is_some_and(|p| p.ends_with("A.png"))));
    assert!(image(&app, "C").is_some_and(|i| i.path.is_some_and(|p| p.ends_with("C.png"))));
    assert!(image(&app, "B").is_none(), "matched by name, not by the cell under the pointer");
}

/// § 5 of the M11 plan: a Font View frame of 500 cells with images under 16 ms once the
/// textures are cached; half of the glyphs are palette glyphs drawn in colour. Run with
/// `cargo test --profile measure -p fontcraft-ui-egui font_view_image_and_colour_cells_frame_speed -- --ignored --nocapture`.
#[test]
#[ignore = "speed measure"]
fn font_view_image_and_colour_cells_frame_speed() {
    let dir = folder("speed");
    let mut app = FontcraftApp::default();
    app.run("colorPalette.addColor", &json!({"color": "#FF0000"})).unwrap();
    let names: Vec<String> = (0..1000).map(|i| format!("g{i:04}")).collect();
    app.session
        .edit("many", |font| {
            for n in &names {
                font.add_glyph(n, vec![], 600.0)?;
            }
            Ok(())
        })
        .unwrap();
    let files: Vec<String> = names.iter().map(|n| png(&dir, &format!("{n}.png"), 64, [0, 0, 255, 255])).collect();
    app.run("glyph.addImage", &json!({"files": files})).unwrap();
    let coloured: Vec<&str> = names.iter().step_by(2).map(String::as_str).collect();
    for n in &coloured {
        rectangle(&mut app, n);
    }
    let m = master(&app);
    app.run("layers.addBackupLayer", &json!({"glyphs": coloured, "master": m})).unwrap();
    for n in &coloured {
        let layer = app.session.font().unwrap().glyph(n).unwrap().layers.iter().find(|l| !l.is_master_layer()).unwrap().id.clone();
        app.run("layer.convertToColorPalette", &json!({"glyph": n, "layer": layer})).unwrap();
    }
    app.run("fontView.setCellSize", &json!({"size": 40})).unwrap();
    let w = Window::sized(1920.0, 1200.0);
    let started = std::time::Instant::now();
    w.frame(&mut app);
    let first = started.elapsed();
    let started = std::time::Instant::now();
    let mut warm = 1;
    while (app.font_view.art.decoded() > 0 || app.font_view.art.coloured > 0) && warm < 2_000 {
        w.frame(&mut app);
        warm += 1;
    }
    let warming = started.elapsed();
    let image_cells = app.font_view.laid_out.iter().filter(|(n, _)| n.starts_with('g')).count();
    assert!(image_cells >= 500, "{image_cells} cells with images in view");
    let runs = 20;
    let started = std::time::Instant::now();
    for _ in 0..runs {
        w.frame(&mut app);
    }
    let steady = started.elapsed() / runs;
    rectangle(&mut app, "g0001");
    let started = std::time::Instant::now();
    w.frame(&mut app);
    let after_edit = started.elapsed();
    println!(
        "{image_cells} image cells ({} thumbnails, {} colour textures): first frame {first:?}, {warm} frames to warm up in {warming:?}, steady {steady:?}, after editing one glyph {after_edit:?}",
        app.font_view.art.thumbs(),
        app.font_view.art.sprites()
    );
    if !cfg!(debug_assertions) {
        assert!(steady.as_secs_f64() < 0.016, "steady {steady:?}");
    }
}
