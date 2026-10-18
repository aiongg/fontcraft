//! Placed images in Edit View (M11.13), in headless frames: a placed, a rotated, a cropped, a
//! locked and a missing image drawn; View › Show Image and empty glyphs; every gesture equal to
//! the command it runs; the context menu's image section; the image Info box; Reveal Image File;
//! a file dropped on the canvas. Image files are PNGs written by the test.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Shape, TextureId};
use fontcraft_engine::imagery::{Rgba8, to_png};
use fontcraft_geom::Point;
use fontcraft_model::Image;
use serde_json::{Value, json};

use crate::canvas::images;
use crate::panels::info_box_image;
use crate::shell::context_menu::{self, IMAGE};
use crate::shell::start_window::Reveal;
use crate::{FontcraftApp, control, tools_glue};

/// A headless window: one egui context kept across frames.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    fn frame_with(&self, app: &mut FontcraftApp, input: egui::RawInput) -> Vec<Shape> {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..input };
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

    fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) -> Vec<Shape> {
        self.frame_with(app, egui::RawInput { events, ..Default::default() })
    }

    fn settle(&self, app: &mut FontcraftApp) -> Vec<Shape> {
        for _ in 0..4 {
            self.frame(app, Vec::new());
        }
        self.frame(app, Vec::new())
    }

    fn enter(&self, app: &mut FontcraftApp, id: egui::Id, text: &str) {
        self.ctx.memory_mut(|m| m.request_focus(id));
        self.frame(app, Vec::new());
        assert!(self.ctx.memory(|m| m.has_focus(id)), "the field takes the focus");
        self.frame(app, vec![Event::Text(text.into())]);
        self.frame(app, vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
    }

    fn click(&self, app: &mut FontcraftApp, id: egui::Id) {
        let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.frame(app, vec![Event::PointerMoved(at)]);
        self.frame(app, vec![button(true)]);
        self.frame(app, vec![button(false)]);
    }
}

/// A folder of its own for one test.
fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-images-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A `width × height` PNG of opaque red at `dir/name`; its path.
fn png(dir: &Path, name: &str, width: u32, height: u32) -> String {
    let pixels = [255, 0, 0, 255].repeat((width * height) as usize);
    let bytes = to_png(&Rgba8 { width, height, pixels }).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

/// Glyph A with a square (300,300)–(400,400) and a 100 × 50 PNG placed at the origin (one pixel
/// to one unit), open in Edit View at actual size.
fn image_app(tag: &str) -> (FontcraftApp, String) {
    let dir = folder(tag);
    let file = png(&dir, "A.png", 100, 50);
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 300, "y": 300, "width": 100, "height": 100})).unwrap();
    app.run("glyph.addImage", &json!({"files": [file], "glyphs": ["A"]})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [200, 200]})).unwrap();
    (app, file)
}

fn image(app: &FontcraftApp, glyph: &str) -> Option<Image> {
    let font = app.session.font().unwrap();
    font.glyph(glyph).unwrap().layer(&font.masters[0].id).unwrap().info.image.clone()
}

fn meshes(shapes: &[Shape]) -> Vec<&egui::Mesh> {
    shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Mesh(m) if m.texture_id != TextureId::default() && m.vertices.len() == 4 => Some(&**m),
            _ => None,
        })
        .collect()
}

/// The screen point of font point (x, y) of the edited glyph.
fn screen(app: &FontcraftApp, x: f64, y: f64) -> Pos2 {
    let p = tools_glue::current_view(app).unwrap().to_screen(Point::new(x, y));
    Pos2::new(p.x as f32, p.y as f32)
}

fn near(a: Pos2, b: Pos2) -> bool {
    (a - b).length() < 0.01
}

fn pointer(app: &mut FontcraftApp, events: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": "ui.pointer", "params": {"events": events}}));
    assert_eq!(r["ok"], true, "{r}");
    r["result"].clone()
}

fn drag(app: &mut FontcraftApp, from: (f64, f64), to: (f64, f64), mods: Value) {
    let mid = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
    let r = pointer(
        app,
        json!([
            {"kind": "down", "x": from.0, "y": from.1, "mods": mods},
            {"kind": "drag", "x": mid.0, "y": mid.1, "mods": mods},
            {"kind": "drag", "x": to.0, "y": to.1, "mods": mods},
            {"kind": "up", "x": to.0, "y": to.1, "mods": mods},
        ]),
    );
    assert_eq!(r["errors"], json!([]), "{r}");
}

fn click(app: &mut FontcraftApp, at: (f64, f64)) {
    let r = pointer(app, json!([{"kind": "down", "x": at.0, "y": at.1}, {"kind": "up", "x": at.0, "y": at.1}]));
    assert_eq!(r["errors"], json!([]), "{r}");
}

fn undo_steps(app: &FontcraftApp) -> usize {
    app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len)
}

/// The image that `steps` give glyph A of a fresh app made by `image_app(tag)`: the same file,
/// so the images compare whole.
fn by_command(tag: &str, steps: &[(&str, Value)]) -> Option<Image> {
    let (mut app, _) = image_app(tag);
    for (id, params) in steps {
        app.run(id, params).unwrap();
    }
    image(&app, "A")
}

#[test]
fn a_placed_image_is_drawn_as_a_texture_at_its_place_and_opacity() {
    let (mut app, _) = image_app("placed");
    let w = Window::new();
    let shapes = w.settle(&mut app);
    let found = meshes(&shapes);
    assert_eq!(found.len(), 1, "one placed image is drawn");
    let mesh = found[0];
    let corners = [(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)];
    for (v, (x, y)) in mesh.vertices.iter().zip(corners) {
        assert!(near(v.pos, screen(&app, x, y)), "{:?} lies at ({x}, {y})", v.pos);
    }
    // The bottom-left corner of the image is the bottom-left of the texture.
    assert_eq!((mesh.vertices[0].uv, mesh.vertices[2].uv), (egui::pos2(0.0, 1.0), egui::pos2(1.0, 0.0)));
    assert!(mesh.vertices[0].color.a() < 255 && mesh.vertices[0].color.a() > 0, "drawn at 50 % when the file states no opacity");
}

#[test]
fn show_image_hides_images_except_on_glyphs_without_paths_or_components() {
    let (mut app, file) = image_app("show");
    let w = Window::new();
    assert_eq!(app.run("view.showImage", &json!({})).unwrap()["on"], false, "omitted: toggle");
    assert!(meshes(&w.settle(&mut app)).is_empty(), "A has a path: its image is hidden");
    app.run("glyph.newGlyph", &json!({"name": "A.empty"})).unwrap();
    app.run("glyph.addImage", &json!({"files": [file], "glyphs": ["A.empty"]})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A.empty"]})).unwrap();
    assert_eq!(meshes(&w.settle(&mut app)).len(), 1, "A.empty has no paths: its image shows anyway");
    assert_eq!(app.run("view.showImage", &json!({"on": true})).unwrap()["on"], true);
}

#[test]
fn a_rotated_cropped_image_draws_only_its_shown_part() {
    let (mut app, _) = image_app("rotated");
    app.run("image.setCrop", &json!({"glyph": "A", "rect": [0, 0, 50, 50]})).unwrap();
    app.run("image.setTransform", &json!({"glyph": "A", "rotation": 90})).unwrap();
    let w = Window::new();
    let shapes = w.settle(&mut app);
    let mesh = meshes(&shapes)[0];
    // A quarter turn about the origin: pixel (x, y) lands at (−y, x).
    let corners = [(0.0, 0.0), (0.0, 50.0), (-50.0, 50.0), (-50.0, 0.0)];
    for (v, (x, y)) in mesh.vertices.iter().zip(corners) {
        assert!(near(v.pos, screen(&app, x, y)), "{:?} lies at ({x}, {y})", v.pos);
    }
    assert!(mesh.vertices.iter().all(|v| v.uv.x <= 0.5), "the right half is cropped away");
}

#[test]
fn a_missing_file_draws_the_marker_and_a_locked_image_is_still_drawn() {
    let (mut app, file) = image_app("missing");
    app.run("image.lock", &json!({"glyph": "A"})).unwrap();
    let w = Window::new();
    assert_eq!(meshes(&w.settle(&mut app)).len(), 1, "a locked image is drawn");
    std::fs::remove_file(&file).unwrap();
    // The window getting the focus back is when files are read again.
    w.frame_with(&mut app, egui::RawInput { focused: false, ..Default::default() });
    let shapes = w.settle(&mut app);
    assert!(meshes(&shapes).is_empty());
    let marker = app.tokens.images.marker;
    assert!(shapes.iter().any(|s| matches!(s, Shape::Rect(r) if r.stroke.color == marker)), "the missing-image square");
    assert!(shapes.iter().any(|s| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == marker)), "its cross");
}

#[test]
fn dragging_the_image_equals_image_set_transform() {
    let (mut app, _) = image_app("move");
    Window::new().settle(&mut app);
    let before = undo_steps(&app);
    drag(&mut app, (50.0, 25.0), (80.0, 45.0), json!([]));
    assert!(images::is_selected(&app), "the drag selects the image");
    assert_eq!(undo_steps(&app), before + 1, "one undo step");
    let (id, params) = app.session.journal().last().cloned().unwrap();
    assert_eq!((id.as_str(), &params["x"], &params["y"]), ("image.setTransform", &json!(30.0), &json!(20.0)));
    assert_eq!(image(&app, "A"), by_command("move", &[("image.setTransform", json!({"glyph": "A", "x": 30, "y": 20}))]));
}

#[test]
fn a_click_on_a_path_goes_to_the_path_not_the_image() {
    let (mut app, _) = image_app("under");
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 20, "y": 10, "width": 40, "height": 30})).unwrap();
    Window::new().settle(&mut app);
    click(&mut app, (20.0, 10.0));
    assert!(!images::is_selected(&app));
    assert!(!app.state.tab().unwrap().selection.is_empty(), "the node under the click is selected");
}

#[test]
fn knobs_scale_the_image_and_ctrl_alt_crops_it() {
    let (mut app, _) = image_app("knobs");
    app.run("view.showBoundingBox", &json!({"on": true})).unwrap();
    let w = Window::new();
    w.settle(&mut app);
    click(&mut app, (50.0, 25.0));
    assert!(images::is_selected(&app));
    // The top-right knob, dragged twice as far from the bottom-left corner.
    drag(&mut app, (100.0, 50.0), (200.0, 100.0), json!([]));
    let scaled = by_command("knobs", &[("image.setTransform", json!({"glyph": "A", "x": 0, "y": 0, "width": 200, "height": 100}))]);
    assert_eq!(image(&app, "A"), scaled);
    let shapes = w.settle(&mut app);
    let knob = app.tokens.bounding_box;
    assert!(shapes.iter().filter(|s| matches!(s, Shape::Rect(r) if r.fill == knob)).count() >= 8, "eight knobs");
    // Ctrl+Alt on the same knob, now at (200, 100): the right edge to pixel 75.
    drag(&mut app, (200.0, 100.0), (150.0, 100.0), json!(["ctrl", "alt"]));
    assert_eq!(image(&app, "A").unwrap().crop, Some([0.0, 0.0, 75.0, 50.0]));
}

#[test]
fn the_rotate_tool_turns_the_selected_image_about_its_centre() {
    let (mut app, _) = image_app("rotate");
    Window::new().settle(&mut app);
    click(&mut app, (50.0, 25.0));
    app.run("tool.select", &json!({"tool": "rotate"})).unwrap();
    // A quarter turn about the centre (50, 25).
    drag(&mut app, (100.0, 25.0), (50.0, 75.0), json!([]));
    let turned = by_command("rotate", &[("image.setTransform", json!({"glyph": "A", "x": 75, "y": -25, "rotation": 90}))]);
    assert_eq!(image(&app, "A"), turned);
}

#[test]
fn a_locked_image_ignores_clicks_and_its_menu_offers_unlock() {
    let (mut app, _) = image_app("locked");
    app.run("image.lock", &json!({"glyph": "A"})).unwrap();
    Window::new().settle(&mut app);
    let before = image(&app, "A");
    drag(&mut app, (50.0, 25.0), (80.0, 45.0), json!([]));
    assert!(!images::is_selected(&app));
    assert_eq!(image(&app, "A"), before, "the drag passes through a locked image");

    context_menu::open(&mut app, Point::new(50.0, 25.0), tools_glue::Space::Font).unwrap();
    let menu = app.tools.context_menu.clone().unwrap();
    assert_eq!(menu.sections[0].context, IMAGE);
    let items: Vec<&str> = menu.sections[0].items.iter().map(|i| i.command).collect();
    assert_eq!(items, ["image.unlock"]);
    context_menu::invoke(&mut app, "image.unlock").unwrap();
    assert!(!image(&app, "A").unwrap().locked);

    context_menu::open(&mut app, Point::new(50.0, 25.0), tools_glue::Space::Font).unwrap();
    let menu = app.tools.context_menu.clone().unwrap();
    let items: Vec<&str> = menu.sections[0].items.iter().map(|i| i.command).collect();
    assert_eq!(items, ["image.setCropToLayerBounds", "image.resetCrop"]);
    assert!(images::is_selected(&app), "a right-click selects an unlocked image");
}

#[test]
fn the_info_box_equals_image_set_transform() {
    let (mut app, _) = image_app("infobox");
    let w = Window::new();
    w.settle(&mut app);
    click(&mut app, (50.0, 25.0));
    w.settle(&mut app);
    w.enter(&mut app, info_box_image::field("width"), "300");
    let (id, params) = app.session.journal().last().cloned().unwrap();
    assert_eq!((id.as_str(), &params["width"]), ("image.setTransform", &json!(300.0)));
    w.settle(&mut app);
    w.enter(&mut app, info_box_image::field("rotation"), "45");
    let expected = by_command(
        "infobox",
        &[("image.setTransform", json!({"glyph": "A", "width": 300})), ("image.setTransform", json!({"glyph": "A", "rotation": 45}))],
    );
    assert_eq!(image(&app, "A"), expected);
    w.settle(&mut app);
    w.click(&mut app, info_box_image::field("lock"));
    assert!(image(&app, "A").unwrap().locked);
    w.settle(&mut app);
    assert!(!images::is_selected(&app), "a locked image cannot stay selected");
}

static REVEALED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

fn fake_reveal(path: &Path) -> Result<(), String> {
    REVEALED.lock().unwrap().push(path.to_path_buf());
    Ok(())
}

#[test]
fn reveal_image_file_shows_the_file_the_image_names() {
    let (mut app, file) = image_app("reveal");
    let e = app.run("image.revealFile", &json!({})).unwrap_err();
    assert!(e.contains("file manager"), "{e}");
    app.state.start.reveal = Reveal(Some(fake_reveal));
    app.run("image.revealFile", &json!({})).unwrap();
    assert_eq!(REVEALED.lock().unwrap().last(), Some(&PathBuf::from(&file)));
    app.run("glyph.newGlyph", &json!({"name": "A.empty"})).unwrap();
    let e = app.run("image.revealFile", &json!({"glyph": "A.empty"})).unwrap_err();
    assert!(e.contains("no image"), "{e}");
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

#[test]
fn a_file_dropped_on_the_canvas_is_added_to_the_edited_glyph() {
    let dir = folder("drop");
    let file = png(&dir, "photo.png", 20, 10);
    let mut app = FontcraftApp::default();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    let w = Window::new();
    w.settle(&mut app);
    let dropped: egui::DroppedFileHandle = std::sync::Arc::new(Dropped(PathBuf::from(&file)));
    w.frame_with(&mut app, egui::RawInput { dropped_files: vec![dropped], ..Default::default() });
    w.settle(&mut app);
    let placed = image(&app, "A").expect("the image is placed on A");
    assert!(placed.path.is_some());
    assert_eq!(meshes(&w.settle(&mut app)).len(), 1, "and drawn");
}
