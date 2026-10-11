//! The Preview Area and the Preview Panel: opening them, Invert, Flip, Blur, centring and the
//! offset, the kerning mode and the single line, the double-click that edits a glyph, the blank
//! panel without an Edit View tab, and the `Preview Ascender` and `Preview Descender` edges.

use std::sync::Arc;

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, Shape, vec2};
use fontcraft_model::plist::{Number, Value as PlistValue};
use serde_json::json;

use crate::FontcraftApp;
use crate::canvas::view;
use crate::panels::widgets;
use crate::shell::preview::{self, Picture, PreviewOptions, Target};

/// A headless window: one egui context kept across frames.
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

    fn settle(&self, app: &mut FontcraftApp) -> Vec<Shape> {
        self.frame(app, Vec::new());
        self.frame(app, Vec::new())
    }

    fn button(&self, app: &mut FontcraftApp, at: Pos2, pressed: bool) {
        self.frame(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
    }

    /// The rect of control `id` as the last frame drew it.
    fn rect(&self, id: egui::Id) -> Rect {
        self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect
    }

    fn click(&self, app: &mut FontcraftApp, id: egui::Id) {
        let at = self.rect(id).center();
        self.frame(app, vec![Event::PointerMoved(at)]);
        self.button(app, at, true);
        self.button(app, at, false);
    }

    /// Whether the last shapes hold a mesh with the preview texture of `target`.
    fn draws_preview(&self, shapes: &[Shape], target: Target) -> bool {
        let Some(texture) = preview::texture_id(&self.ctx, target) else { return false };
        shapes.iter().any(|s| matches!(s, Shape::Mesh(m) if m.texture_id == texture))
    }
}

/// A (outline 50 to 550) and V (100 to 500), each 600 wide, with A V kerned −100 and V A +40; a
/// tab holding "AVA" with the cursor at 1, so V is edited.
fn preview_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 50, "y": 0, "width": 500, "height": 700})).unwrap();
    app.run("path.addRectangle", &json!({"glyph": "V", "x": 100, "y": 0, "width": 400, "height": 700})).unwrap();
    for direction in ["ltr", "rtl"] {
        app.run("kerning.setPair", &json!({"left": "A", "right": "V", "value": -100, "direction": direction})).unwrap();
        app.run("kerning.setPair", &json!({"left": "V", "right": "A", "value": 40, "direction": direction})).unwrap();
    }
    app.run("view.newTab", &json!({"glyphs": ["A"]})).unwrap();
    app.run("editView.setText", &json!({"text": "AVA"})).unwrap();
    app.run("editView.setCursor", &json!({"index": 1})).unwrap();
    app
}

/// The strip used for the pure checks: 1000 × 120 points.
const STRIP: Rect = Rect { min: Pos2 { x: 100.0, y: 50.0 }, max: Pos2 { x: 1100.0, y: 170.0 } };

fn layout(app: &mut FontcraftApp) -> fontcraft_textlayout::Layout {
    let tab = app.state.tab().unwrap().clone();
    preview::line_layout(&mut app.session, &tab).unwrap()
}

fn placement(app: &mut FontcraftApp, options: &PreviewOptions) -> preview::Placement {
    let layout = layout(app);
    let tab = app.state.tab().unwrap().clone();
    let font = app.session.font().unwrap();
    let range = preview::vertical_range(font, view::tab_master(font, &tab).unwrap());
    preview::place(&layout, tab.edited_index(), STRIP, range, options, 16.0).unwrap()
}

/// The strip's picture at one pixel per point.
fn picture(app: &mut FontcraftApp, options: &PreviewOptions) -> egui::ColorImage {
    let layout = layout(app);
    let placed = placement(app, options);
    let tab = app.state.tab().unwrap().clone();
    let doc = app.session.active().map(|d| (d.id, d.revision)).unwrap();
    let font = app.session.font().unwrap();
    let p = Picture {
        font,
        tab: &tab,
        layout: &layout,
        placement: &placed,
        ppp: 1.0,
        doc,
        options,
        ink: egui::Color32::BLACK,
        paper: egui::Color32::WHITE,
    };
    preview::picture(&p, &mut app.fill_cache.outlines).unwrap()
}

fn area(app: &FontcraftApp) -> PreviewOptions {
    app.state.panels.preview.area
}

fn pixel(image: &egui::ColorImage, x: usize, y: usize) -> egui::Color32 {
    image.pixels[y * image.size[0] + x]
}

#[test]
fn preview_area_opens_under_the_canvas_with_invert_flip_and_blur() {
    let mut app = preview_app();
    let w = Window::new();
    w.settle(&mut app);
    let full = app.state.canvas_size;

    // view.togglePreviewArea splits the tab: the area takes its height from the canvas.
    let r = app.run("view.togglePreviewArea", &json!({})).unwrap();
    assert_eq!(r["on"], true);
    assert_eq!(r["height"], 140.0);
    let shapes = w.settle(&mut app);
    assert!(w.draws_preview(&shapes, Target::Area), "the area draws the line");
    assert!((full[1] - app.state.canvas_size[1] - 140.0).abs() < 1.0, "{full:?} → {:?}", app.state.canvas_size);
    let r = app.run("view.togglePreviewArea", &json!({"on": true, "height": 5})).unwrap();
    assert_eq!(r["height"], f64::from(app.tokens.preview_area_min_height), "the height is held to the smallest");
    app.run("view.togglePreviewArea", &json!({"height": 200})).unwrap();
    w.settle(&mut app);
    assert!((full[1] - app.state.canvas_size[1]).abs() < 1.0, "toggled off, the canvas has the whole height");

    // Dark on light; preview.invert swaps text and ground.
    let plain = picture(&mut app, &PreviewOptions::default());
    assert_eq!(pixel(&plain, 0, 0), egui::Color32::WHITE);
    assert!(plain.pixels.contains(&egui::Color32::BLACK));
    let r = app.run("preview.invert", &json!({})).unwrap();
    assert_eq!(r, json!({"target": "area", "on": true}));
    let options = app.state.panels.preview.area;
    assert!(!app.state.panels.preview.panel.invert, "the panel keeps its own setting");
    let inverted = picture(&mut app, &options);
    assert_eq!(pixel(&inverted, 0, 0), egui::Color32::BLACK);
    app.run("preview.invert", &json!({"on": false})).unwrap();

    // preview.flip turns the strip half a circle.
    app.run("preview.flip", &json!({"target": "area"})).unwrap();
    let flipped = {
        let o = area(&app);
        picture(&mut app, &o)
    };
    let turned: Vec<_> = plain.pixels.iter().rev().copied().collect();
    assert_eq!(flipped.pixels, turned);
    app.run("preview.flip", &json!({"on": false})).unwrap();

    // preview.setBlur softens edges; 0 is sharp; the amount is held to 0..10.
    assert_eq!(app.run("preview.setBlur", &json!({"amount": 25})).unwrap()["amount"], 10.0);
    app.run("preview.setBlur", &json!({"amount": 4})).unwrap();
    let blurred = {
        let o = area(&app);
        picture(&mut app, &o)
    };
    assert_ne!(blurred.pixels, plain.pixels);
    let grey = |img: &egui::ColorImage| img.pixels.iter().filter(|c| c.r() > 20 && c.r() < 235).count();
    assert!(grey(&blurred) > grey(&plain) * 2, "{} vs {}", grey(&blurred), grey(&plain));
    app.run("preview.setBlur", &json!({"amount": 0})).unwrap();
    assert_eq!(
        {
            let o = area(&app);
            picture(&mut app, &o)
        }
        .pixels,
        plain.pixels
    );
    assert!(app.run("preview.setBlur", &json!({})).is_err());
    assert!(app.run("preview.invert", &json!({"target": "window"})).is_err());

    // The buttons under the strip run the commands.
    app.run("view.togglePreviewArea", &json!({"on": true})).unwrap();
    w.settle(&mut app);
    w.click(&mut app, widgets::id(("preview.invert", "area")));
    assert!(app.state.panels.preview.area.invert);
    w.click(&mut app, widgets::id(("preview.flip", "area")));
    assert!(app.state.panels.preview.area.flip);
}

#[test]
fn centring_offset_kerning_mode_and_single_line_place_the_text() {
    let mut app = preview_app();
    // Kerned: A at 0, V at 500, A at 1140; the edited V's middle (800) sits in the middle.
    let l = layout(&mut app);
    let xs: Vec<f64> = l.placed.iter().map(|p| p.x).collect();
    assert_eq!(xs, [0.0, 500.0, 1140.0]);
    let centred = placement(&mut app, &PreviewOptions::default());
    assert!((centred.to_screen(800.0, 0.0).x - STRIP.center().x).abs() < 0.01);
    assert_eq!(centred.scale, 120.0 / 1200.0, "the strip reaches from the descender to 1000");

    // The previews follow the tab's kerning mode (preview.kerningModes).
    app.run("kerning.setMode", &json!({"mode": "none"})).unwrap();
    let xs: Vec<f64> = layout(&mut app).placed.iter().map(|p| p.x).collect();
    assert_eq!(xs, [0.0, 600.0, 1200.0]);
    app.run("kerning.setMode", &json!({"mode": "kerning"})).unwrap();

    // preview.alwaysCenterActiveGlyph off: a text that fits keeps flush left, or right for
    // right-to-left text.
    let r = app.run("preview.alwaysCenterActiveGlyph", &json!({})).unwrap();
    assert_eq!(r, json!({"target": "area", "on": false}));
    let options = app.state.panels.preview.area;
    let flush = placement(&mut app, &options);
    assert!((flush.to_screen(0.0, 0.0).x - (STRIP.min.x + 16.0)).abs() < 0.01);
    app.run("editView.setWritingDirection", &json!({"direction": "rtl"})).unwrap();
    let right = placement(&mut app, &options);
    assert!((right.to_screen(0.0, 0.0).x - (STRIP.max.x - 16.0)).abs() < 0.01);
    app.run("editView.setWritingDirection", &json!({"direction": "ltr"})).unwrap();

    // preview.setOffset shifts the line by screen points, to `x` or by `dx`.
    assert_eq!(app.run("preview.setOffset", &json!({"x": 25})).unwrap()["x"], 25.0);
    assert_eq!(app.run("preview.setOffset", &json!({"dx": 5})).unwrap()["x"], 30.0);
    let shifted = {
        let o = area(&app);
        placement(&mut app, &o)
    };
    assert!((shifted.x0 - flush.x0 - 30.0).abs() < 1e-9);
    assert!(app.run("preview.setOffset", &json!({"x": 1, "dx": 1})).is_err());

    // preview.singleLine: a tab of two lines previews on one, the line break taking no room.
    app.run("editView.setText", &json!({"chars": "A\nV"})).unwrap();
    let l = layout(&mut app);
    assert_eq!(l.lines.len(), 1);
    let xs: Vec<(f64, f64)> = l.placed.iter().map(|p| (p.x, p.y)).collect();
    assert_eq!(xs, [(0.0, 0.0), (600.0, 0.0), (600.0, 0.0)]);
}

#[test]
fn double_click_in_the_preview_edits_the_glyph_and_dragging_scrolls() {
    let mut app = preview_app();
    let w = Window::new();
    app.run("view.togglePreviewArea", &json!({"on": true})).unwrap();
    w.settle(&mut app);
    let strip = w.rect(widgets::id(("preview.strip", "area")));
    let options = app.state.panels.preview.area;
    let l = layout(&mut app);
    let tab = app.state.tab().unwrap().clone();
    let font = app.session.font().unwrap();
    let range = preview::vertical_range(font, view::tab_master(font, &tab).unwrap());
    let placed = preview::place(&l, tab.edited_index(), strip, range, &options, f64::from(app.tokens.preview_margin)).unwrap();
    let at = placed.to_screen(1140.0 + 300.0, 300.0);
    assert_eq!(preview::hit(&l, &tab, &placed, at), Some(2));

    // preview.doubleClickEdits: the third glyph becomes the edited one.
    for _ in 0..40 {
        w.frame(&mut app, Vec::new());
    }
    w.frame(&mut app, vec![Event::PointerMoved(at)]);
    for pressed in [true, false, true, false] {
        w.button(&mut app, at, pressed);
    }
    w.frame(&mut app, Vec::new());
    assert_eq!(app.state.tab().unwrap().text.cursor(), 2, "{}", app.state.status);
    assert_eq!(app.state.editing(), Some("A"));

    // Dragging the text sideways sets the offset.
    for _ in 0..40 {
        w.frame(&mut app, Vec::new());
    }
    w.button(&mut app, at, true);
    for step in 1..=4 {
        w.frame(&mut app, vec![Event::PointerMoved(at + vec2(10.0 * step as f32, 0.0))]);
    }
    w.button(&mut app, at + vec2(40.0, 0.0), false);
    let offset = app.state.panels.preview.area.offset;
    assert!(offset > 10.0 && offset <= 40.0, "{offset}");
}

#[test]
fn preview_panel_is_blank_without_an_edit_view_tab() {
    let mut app = preview_app();
    let w = Window::new();
    assert_eq!(app.run("window.previewPanel", &json!({})).unwrap(), json!({"on": true}));
    app.run("view.showTab", &json!({"index": 1})).unwrap();
    let shapes = w.settle(&mut app);
    assert!(app.state.tab().is_none());
    assert!(w.ctx.read_response(widgets::id(("preview.invert", "panel"))).is_some(), "the panel is open");
    assert!(!w.draws_preview(&shapes, Target::Panel), "no glyphs while Font View shows");

    // Showing the tab fills it in at once.
    app.run("view.showTab", &json!({"index": 2})).unwrap();
    let shapes = w.frame(&mut app, Vec::new());
    assert!(w.draws_preview(&shapes, Target::Panel));
    assert!(!w.draws_preview(&shapes, Target::Area), "the area stays closed");
    app.run("preview.flip", &json!({"target": "panel"})).unwrap();
    assert!(app.state.panels.preview.panel.flip && !app.state.panels.preview.area.flip);
    assert_eq!(app.run("window.previewPanel", &json!({})).unwrap(), json!({"on": false}));
    w.settle(&mut app);
    assert!(w.ctx.read_response(widgets::id(("preview.invert", "panel"))).is_none());
}

#[test]
fn preview_ascender_and_descender_set_the_strip_edges() {
    let mut app = preview_app();
    let range = |app: &FontcraftApp| {
        let font = app.session.font().unwrap();
        preview::vertical_range(font, font.masters.first().unwrap())
    };
    assert_eq!(range(&app), (-200.0, 1000.0), "1000 and the master's descender");
    let set = |app: &mut FontcraftApp, name: &'static str, value: i64| {
        app.session
            .edit("param", |font| {
                let masters = Arc::make_mut(&mut font.masters);
                masters.first_mut().unwrap().params.set(name, PlistValue::Number(Number::int(value)));
                Ok(())
            })
            .unwrap();
    };
    set(&mut app, "winDescent", 270);
    set(&mut app, "winAscent", 250);
    assert_eq!(range(&app), (-270.0, 1000.0), "winDescent, not winAscent");
    set(&mut app, "Preview Descender", -300);
    set(&mut app, "Preview Ascender", 1200);
    assert_eq!(range(&app), (-300.0, 1200.0));
    set(&mut app, "Preview Descender", 320);
    assert_eq!(range(&app).0, -320.0, "either sign is a distance below the baseline");
    set(&mut app, "Preview Ascender", -500);
    assert_eq!(range(&app), (-200.0, 800.0), "edges that enclose nothing fall back to descender and ascender");
}
