//! Strokes and masks on the canvas, the Palette's Stroke section and the Transformations boolean
//! buttons, in headless frames.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{ColorImage, Event, Key, Modifiers, PointerButton, Pos2, Shape, TextureId};
use fontcraft_model::{Dimension, Shape as ModelShape};
use serde_json::{Value, json};

use crate::FontcraftApp;
use crate::panels::widgets;

fn input(events: Vec<Event>) -> egui::RawInput {
    egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() }
}

/// A headless window: one egui context kept across frames. It keeps the last full image of every
/// texture and the shapes of the last frame.
#[derive(Default)]
struct Window {
    ctx: egui::Context,
    images: HashMap<TextureId, Arc<ColorImage>>,
    shapes: Vec<Shape>,
}

impl Window {
    fn frame(&mut self, app: &mut FontcraftApp, events: Vec<Event>) {
        let mut output = self.ctx.run_ui(input(events), |ui| app.ui(ui));
        let set = std::mem::take(&mut output.textures_delta.set);
        output.textures_delta.clear();
        for (id, deltas) in set {
            for delta in deltas {
                let egui::ImageData::Color(image) = delta.image;
                if delta.pos.is_none() {
                    self.images.insert(id, image);
                }
            }
        }
        let mut out: Vec<Shape> = output.shapes.into_iter().map(|c| c.shape).collect();
        self.shapes.clear();
        while let Some(shape) = out.pop() {
            match shape {
                Shape::Vec(list) => out.extend(list),
                other => self.shapes.push(other),
            }
        }
    }

    fn settle(&mut self, app: &mut FontcraftApp) {
        for _ in 0..3 {
            self.frame(app, Vec::new());
        }
    }

    fn click(&mut self, app: &mut FontcraftApp, id: egui::Id) {
        let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.frame(app, vec![Event::PointerMoved(at)]);
        self.frame(app, vec![button(true)]);
        self.frame(app, vec![button(false)]);
    }

    /// Pixels of the canvas fill texture that are more than half covered (0 without a texture).
    fn filled_pixels(&self, app: &FontcraftApp) -> usize {
        let Some(id) = app.fill_cache.texture_id() else { return 0 };
        let image = self.images.get(&id).expect("the fill texture was uploaded");
        image.pixels.iter().filter(|p| p.a() > 127).count()
    }

    /// Whether the last frame drew a line in `color` with `width`.
    fn drew_line(&self, color: egui::Color32, width: f32) -> bool {
        self.shapes.iter().any(|s| match s {
            Shape::Path(p) => p.stroke.width == width && matches!(p.stroke.color, egui::epaint::ColorMode::Solid(c) if c == color),
            _ => false,
        })
    }
}

/// Glyph A with the open line (200, 300)–(600, 300) as shape 0, at actual size, `extra` run on it.
fn line_app(extra: Option<Value>) -> FontcraftApp {
    let mut app = FontcraftApp::default();
    let nodes = json!([{"x": 200, "y": 300}, {"x": 600, "y": 300}]);
    app.run("path.addPath", &json!({"glyph": "A", "closed": false, "nodes": nodes})).unwrap();
    if let Some(params) = extra {
        app.run("path.setStroke", &params).unwrap();
    }
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [400, 300]})).unwrap();
    app
}

fn last(app: &FontcraftApp) -> (String, Value) {
    app.session.journal().last().cloned().expect("a command ran")
}

fn shapes(app: &FontcraftApp) -> Vec<ModelShape> {
    let font = app.session.font().unwrap();
    font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap().shapes.clone()
}

fn collapse(app: &mut FontcraftApp, sections: &[&str]) {
    for section in sections {
        app.run("palette.toggleSection", &json!({"section": section, "expanded": false})).unwrap();
    }
}

#[test]
fn a_stroked_open_path_fills_as_its_outline() {
    let mut plain = line_app(None);
    let tokens = plain.tokens.clone();
    // The Hand tool fills the edited glyph.
    plain.run("tool.select", &json!({"tool": "hand"})).unwrap();
    let mut w = Window::default();
    w.settle(&mut plain);
    assert!(w.filled_pixels(&plain) < 100, "an open path without a stroke fills nothing");
    assert!(!w.drew_line(tokens.source_outline, tokens.source_outline_width), "no grey source line without strokes or masks");

    let mut stroked = line_app(Some(json!({"glyph": "A", "shapes": [0], "width": 100})));
    stroked.run("tool.select", &json!({"tool": "hand"})).unwrap();
    let mut w = Window::default();
    w.settle(&mut stroked);
    // A 400 × 100 band with butt caps, one pixel per unit.
    let filled = w.filled_pixels(&stroked);
    assert!((38_000..=42_000).contains(&filled), "the band fills {filled} pixels");
    assert!(w.drew_line(tokens.source_outline, tokens.source_outline_width), "the centre line is drawn light grey");
    assert!(w.drew_line(tokens.outline, tokens.outline_width), "the stroke outline is drawn black");
}

#[test]
fn the_stroke_section_runs_set_stroke_on_the_selected_paths() {
    let mut app = line_app(None);
    collapse(&mut app, &["dimensions", "fitCurve", "layers", "transformations"]);
    let mut w = Window::default();
    w.settle(&mut app);
    let width = widgets::id("stroke.width");
    assert!(w.ctx.read_response(width).is_none(), "no Stroke section without a selected path");

    app.run("selection.set", &json!({"points": [[0, 1]]})).unwrap();
    w.settle(&mut app);
    w.ctx.memory_mut(|m| m.request_focus(width));
    w.frame(&mut app, Vec::new());
    w.frame(&mut app, vec![Event::Text("40".into())]);
    w.frame(&mut app, vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
    let (id, params) = last(&app);
    assert_eq!(id, "path.setStroke");
    assert_eq!(params["shapes"], json!([0]), "a path with a selected node counts as selected");
    assert_eq!(params["width"].as_f64(), Some(40.0));
    let all = shapes(&app);
    let Some(ModelShape::Path(path)) = all.first() else { panic!("shape 0 is a path") };
    assert_eq!(path.info.attrs.stroke_width, Some(Dimension::Number(40.0)));

    w.settle(&mut app);
    w.click(&mut app, widgets::id(("stroke.cap", "capEnd", "round")));
    let (id, params) = last(&app);
    assert_eq!(id, "path.setStroke");
    assert_eq!(params["capEnd"], "round");
    assert!(params.get("width").is_none(), "a control sends only its own key");
}

#[test]
fn the_boolean_buttons_run_their_commands() {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 200, "y": 200, "width": 200, "height": 200})).unwrap();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 600, "y": 100, "width": 100, "height": 100})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    collapse(&mut app, &["dimensions", "fitCurve", "layers"]);
    let mut w = Window::default();
    w.settle(&mut app);

    app.run("selection.set", &json!({"points": [[1, 0]]})).unwrap();
    w.settle(&mut app);
    w.click(&mut app, widgets::id("transform.booleanSubtract"));
    let (id, params) = last(&app);
    assert_eq!(id, "transform.booleanSubtract");
    assert_eq!(params["shapes"], json!([1]), "the path with a selected node is the one subtracted");

    app.run("edit.deselectAll", &json!({})).unwrap();
    w.settle(&mut app);
    w.click(&mut app, widgets::id("transform.booleanUnion"));
    let (id, params) = last(&app);
    assert_eq!(id, "transform.booleanUnion");
    assert!(params.get("shapes").is_none(), "with nothing selected the union takes every path");
}
