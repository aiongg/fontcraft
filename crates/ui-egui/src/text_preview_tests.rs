//! The Text Preview window: each of its commands over the control channel changes what a
//! headless frame draws; characters the font lacks are drawn in the interface font; the text
//! limit; and the speed of laying out a book-length text (`text.longTextIsSlow`).

use std::sync::Arc;

use egui::{Order, Pos2, Shape};
use fontcraft_model::Layer;
use serde_json::{Value, json};

use crate::shell::text_preview::{self, Drawn, MAX_CHARS};
use crate::{FontcraftApp, control};

/// A headless window: one egui context kept across frames.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        let ctx = egui::Context::default();
        // A new window fades in, which would scale the colours a test compares.
        ctx.global_style_mut(|s| s.animation_time = 0.0);
        Self { ctx }
    }

    /// One frame with these input events.
    fn events(&self, app: &mut FontcraftApp, events: Vec<egui::Event>, modifiers: egui::Modifiers) {
        let mut events = events;
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
    }

    fn cmd_f(&self, app: &mut FontcraftApp) {
        let key = egui::Event::Key { key: egui::Key::F, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
        self.events(app, vec![key], egui::Modifiers::COMMAND);
    }

    /// One frame; the shapes it drew, nested lists flattened.
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

    /// Two frames (a new window sizes itself in the first); what the preview drew.
    fn drawn(&self, app: &mut FontcraftApp) -> Drawn {
        self.frame(app);
        self.frame(app);
        app.state.panels.text_preview.cache.drawn.clone()
    }
}

/// Runs a command through the control channel, as an agent does.
fn exec(app: &mut FontcraftApp, command: &str, params: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": "engine.execute", "params": {"command": command, "params": params}}));
    assert_eq!(r["ok"], true, "{command} {r}");
    r["result"].clone()
}

fn refused(app: &mut FontcraftApp, command: &str, params: Value) -> String {
    let r = control::handle(app, &json!({"id": 1, "method": "engine.execute", "params": {"command": command, "params": params}}));
    assert_eq!(r["ok"], false, "{command} {r}");
    r.to_string()
}

/// A and V with outlines, A V kerned −100; a second master "Bold" where every glyph is 300
/// units wider; the preview open.
fn preview_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 50, "y": 0, "width": 500, "height": 700})).unwrap();
    app.run("path.addRectangle", &json!({"glyph": "V", "x": 100, "y": 0, "width": 400, "height": 700})).unwrap();
    app.run("kerning.setPair", &json!({"left": "A", "right": "V", "value": -100})).unwrap();
    app.session
        .edit("setup", |font| {
            let mut m = font.default_master()?.clone();
            m.id = "m02".into();
            m.name = "Bold".into();
            Arc::make_mut(&mut font.masters).push(m);
            let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
            for name in names {
                let mut g = font.glyph_mut(&name)?;
                let width = g.layers.first().map_or(500.0, |l| l.width);
                let mut layer = g.layers.first().cloned().unwrap_or_else(|| Layer::new("m02", width));
                layer.id = "m02".into();
                layer.master = "m02".into();
                layer.width = width + 300.0;
                g.layers.push(layer);
            }
            Ok(())
        })
        .unwrap();
    exec(&mut app, "window.textPreview", json!({}));
    app
}

fn names(d: &Drawn) -> Vec<&str> {
    d.glyphs.iter().map(|(n, _)| &**n).collect()
}

fn xs(d: &Drawn) -> Vec<f32> {
    d.glyphs.iter().map(|(_, p)| p.x).collect()
}

fn width(app: &FontcraftApp, glyph: &str, master: &str) -> f32 {
    app.session.font().unwrap().glyph(glyph).unwrap().layer(master).unwrap().width as f32
}

fn first_master(app: &FontcraftApp) -> String {
    app.session.font().unwrap().masters[0].id.clone()
}

fn upm(app: &FontcraftApp) -> f32 {
    f32::from(app.session.font().unwrap().units_per_em)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn window_text_preview_opens_draws_its_text_and_closes() {
    let mut app = preview_app();
    let w = Window::new();
    let title = |shapes: &[Shape]| shapes.iter().any(|s| matches!(s, Shape::Text(t) if t.galley.text() == "Text Preview"));
    assert!(title(&w.frame(&mut app)) || title(&w.frame(&mut app)), "the window is drawn");
    assert!(w.drawn(&mut app).glyphs.is_empty(), "an empty text draws nothing");
    exec(&mut app, "textPreview.setText", json!({"text": "AVA"}));
    assert_eq!(names(&w.drawn(&mut app)), ["A", "V", "A"]);
    let r = exec(&mut app, "window.textPreview", json!({"open": false}));
    assert_eq!(r["open"], false);
    w.frame(&mut app);
    assert!(!title(&w.frame(&mut app)), "the window is closed");
}

#[test]
fn set_text_size_and_style_change_what_the_preview_draws() {
    let mut app = preview_app();
    let w = Window::new();
    let r = exec(&mut app, "textPreview.setText", json!({"text": "AV\r\nA"}));
    assert_eq!(r["length"], 4, "\\r\\n is one line break");
    let d = w.drawn(&mut app);
    assert_eq!(names(&d), ["A", "V", "A"]);
    assert_eq!(d.lines, 2);
    let regular = first_master(&app);
    let scale = 36.0 / upm(&app);
    // The document's kerning applies.
    let x = xs(&d);
    assert!(near(x[1] - x[0], (width(&app, "A", &regular) - 100.0) * scale), "{x:?}");
    assert!(d.glyphs[2].1.y > d.glyphs[0].1.y, "the second line is lower");

    let r = exec(&mut app, "textPreview.setFontSize", json!({"size": 72}));
    assert_eq!(r["size"], 72.0);
    let d = w.drawn(&mut app);
    assert_eq!(d.em, 72.0);
    let x = xs(&d);
    assert!(near(x[1] - x[0], (width(&app, "A", &regular) - 100.0) * 2.0 * scale), "{x:?}");

    let r = exec(&mut app, "textPreview.setStyle", json!({"instance": "Bold"}));
    assert_eq!((r["style"].as_str(), r["master"].as_str()), (Some("Bold"), Some("m02")));
    let d = w.drawn(&mut app);
    assert_eq!(d.master.as_deref(), Some("m02"));
    let x = xs(&d);
    // Bold's own widths, and Bold has no kerning.
    assert!(near(x[1] - x[0], width(&app, "A", "m02") * 2.0 * scale), "{x:?}");
    // By index, and back to following the Edit View master.
    exec(&mut app, "textPreview.setStyle", json!({"instance": 0}));
    assert_eq!(w.drawn(&mut app).master, Some(regular.clone()));
    exec(&mut app, "textPreview.setStyle", json!({"instance": "Bold"}));
    let r = exec(&mut app, "textPreview.setStyle", json!({}));
    assert_eq!(r["style"], Value::Null);
    assert_eq!(w.drawn(&mut app).master, Some(regular));
    refused(&mut app, "textPreview.setStyle", json!({"instance": "Black"}));
    refused(&mut app, "textPreview.setStyle", json!({"instance": 9}));
    refused(&mut app, "textPreview.setFontSize", json!({"size": 0}));
}

#[test]
fn text_preview_toggle_pinned_keeps_the_window_in_front() {
    let mut app = preview_app();
    let w = Window::new();
    assert_eq!(w.drawn(&mut app).order, Some(Order::Middle));
    assert_eq!(exec(&mut app, "textPreview.togglePinned", json!({}))["pinned"], true);
    assert_eq!(w.drawn(&mut app).order, Some(Order::Foreground));
    assert_eq!(exec(&mut app, "textPreview.togglePinned", json!({"on": false}))["pinned"], false);
    assert_eq!(w.drawn(&mut app).order, Some(Order::Middle));
}

#[test]
fn set_layout_orientation_and_writing_direction_reorder_the_glyphs() {
    let mut app = preview_app();
    let w = Window::new();
    exec(&mut app, "textPreview.setText", json!({"text": "AVA"}));
    let ltr = w.drawn(&mut app);
    assert!(xs(&ltr).windows(2).all(|p| p[0] < p[1]), "left to right");

    exec(&mut app, "textPreview.setLayoutOrientation", json!({"orientation": "vertical"}));
    let v = w.drawn(&mut app);
    assert_eq!(names(&v), ["A", "V", "A"]);
    assert!(v.glyphs.windows(2).all(|p| p[0].1.y < p[1].1.y), "top to bottom: {:?}", v.glyphs);
    exec(&mut app, "textPreview.setLayoutOrientation", json!({"orientation": "horizontal"}));

    let r = exec(&mut app, "textPreview.setWritingDirection", json!({"scope": "text", "direction": "rtl"}));
    assert_eq!(r["rtl"], true);
    let rtl = w.drawn(&mut app);
    assert!(xs(&rtl).windows(2).all(|p| p[0] > p[1]), "right to left: {:?}", xs(&rtl));
    exec(&mut app, "textPreview.setWritingDirection", json!({"scope": "text", "direction": "default"}));

    // A right-to-left selection reverses only its glyphs.
    let r = exec(&mut app, "textPreview.setWritingDirection", json!({"scope": "selection", "direction": "rtl", "range": [0, 2]}));
    assert_eq!(r["runs"], json!([{"start": 0, "end": 2, "direction": "rtl"}]));
    let x = xs(&w.drawn(&mut app));
    assert!(x[1] < x[0] && x[0] < x[2], "V A | A: {x:?}");
    // The run moves with its characters when text is typed before it.
    let r = exec(&mut app, "textPreview.setText", json!({"text": "VAVA"}));
    assert_eq!(r["runs"], json!([{"start": 1, "end": 3, "direction": "rtl"}]));
    refused(&mut app, "textPreview.setWritingDirection", json!({"scope": "selection", "direction": "rtl"}));
    refused(&mut app, "textPreview.setWritingDirection", json!({"scope": "selection", "direction": "rtl", "range": [3, 9]}));
    refused(&mut app, "textPreview.setLayoutOrientation", json!({"orientation": "diagonal"}));

    // The default direction follows the first letter.
    let r = exec(&mut app, "textPreview.setText", json!({"text": "\u{5d0}\u{5d1}"}));
    assert_eq!(r["rtl"], true);
    let d = w.drawn(&mut app);
    assert_eq!(d.fallback.iter().map(|(c, _)| *c).collect::<String>(), "\u{5d0}\u{5d1}");
    assert!(d.fallback[1].1.x < d.fallback[0].1.x, "{:?}", d.fallback);
}

#[test]
fn text_preview_find_searches_highlights_and_replaces() {
    let mut app = preview_app();
    let w = Window::new();
    exec(&mut app, "textPreview.setText", json!({"text": "Hat hat that"}));
    let r = exec(&mut app, "textPreview.find", json!({"query": "hat"}));
    assert_eq!((r["find"]["matches"].as_u64(), r["find"]["current"].as_u64()), (Some(3), Some(0)));
    assert_eq!(r["find"]["ranges"], json!([[0, 3], [4, 7], [9, 12]]));
    let d = w.drawn(&mut app);
    assert_eq!(d.matches, 3);
    let first = d.current.expect("the current match is highlighted");
    exec(&mut app, "textPreview.find", json!({"options": {"index": 2}}));
    assert!(w.drawn(&mut app).current.unwrap().min.x > first.min.x, "the third match is current");
    let r = exec(&mut app, "textPreview.find", json!({"options": {"match": "fullWord", "ignoreCase": false}}));
    assert_eq!(r["find"]["matches"], 1);
    assert_eq!(w.drawn(&mut app).matches, 1);
    let r = exec(&mut app, "textPreview.find", json!({"replace": "cap", "options": {"ignoreCase": true}}));
    assert_eq!((r["find"]["replaced"].as_u64(), r["length"].as_u64()), (Some(2), Some(12)));
    assert_eq!(&*app.state.panels.text_preview.text, "cap cap that");
    let r = exec(&mut app, "textPreview.find", json!({"query": "a", "replace": "", "options": {"match": "contains", "index": 0}}));
    assert_eq!(r["find"]["replaced"], 1);
    assert_eq!(&*app.state.panels.text_preview.text, "cp cap that");
    refused(&mut app, "textPreview.find", json!({"options": {"index": 99}}));
    assert_eq!(exec(&mut app, "textPreview.find", json!({"open": false}))["find"]["open"], false);
    assert_eq!(w.drawn(&mut app).matches, 0, "no highlights without the find bar");
}

#[test]
fn text_preview_fallback_font_draws_a_missing_character_in_the_interface_font() {
    let mut app = preview_app();
    let w = Window::new();
    exec(&mut app, "textPreview.setText", json!({"text": "A\u{e9}A"}));
    w.frame(&mut app);
    let shapes = w.frame(&mut app);
    let d = app.state.panels.text_preview.cache.drawn.clone();
    assert_eq!(names(&d), ["A", "A"]);
    assert_eq!(d.fallback.len(), 1);
    let (c, at) = d.fallback[0];
    assert_eq!(c, '\u{e9}');
    assert!(d.glyphs[0].1.x < at.x && at.x < d.glyphs[1].1.x, "between the two A: {d:?}");
    let colour = app.tokens.text_preview_fallback;
    assert!(shapes.iter().any(|s| matches!(s, Shape::Text(t) if t.galley.text() == "\u{e9}" && t.fallback_color == colour)));
}

#[test]
fn text_preview_holds_a_long_text_draws_only_the_lines_in_view_and_refuses_more_than_its_limit() {
    let mut app = preview_app();
    let w = Window::new();
    // Longer than the text field edits, too.
    let long: String = "AV A\n".repeat(20_001);
    let r = exec(&mut app, "textPreview.setText", json!({"text": long}));
    assert_eq!(r["length"], 100_005);
    let d = w.drawn(&mut app);
    assert_eq!(d.lines, 20_002);
    assert!(!d.glyphs.is_empty() && d.glyphs.len() < 1_000, "{} glyphs drawn", d.glyphs.len());
    let over: String = "A".repeat(MAX_CHARS + 1);
    refused(&mut app, "textPreview.setText", json!({"text": over}));
    assert_eq!(app.state.panels.text_preview.length, 100_005, "a refused text changes nothing");
}

/// `docs/plans/M5.md` § 5: Text Preview lays out 1,000,000 characters in under 500 ms (on Inter
/// Roman when the corpus is fetched). Run it in release:
/// `cargo test --release -p fontcraft-ui-egui text_preview_lays_out -- --ignored --nocapture`.
#[test]
#[ignore = "speed measure"]
fn text_preview_lays_out_a_million_characters_within_the_target() {
    let mut app = FontcraftApp::default();
    let inter = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/inter/src/Inter-Roman.glyphspackage");
    if inter.is_dir() {
        exec(&mut app, "file.open", json!({"path": inter.to_string_lossy()}));
    }
    let paragraph = "The quick brown fox jumps over the lazy dog; Waltz, nymph, for quick jigs vex Bud. AVATAR Typography 0123456789. ";
    let mut text = String::new();
    // The text is ASCII: bytes are characters.
    while text.len() + 8 * paragraph.len() + 1 < MAX_CHARS {
        for _ in 0..8 {
            text.push_str(paragraph);
        }
        text.push('\n');
    }
    text.extend(std::iter::repeat_n('x', MAX_CHARS - text.len()));
    exec(&mut app, "textPreview.setText", json!({"text": text}));
    let font = app.session.font().unwrap().clone();
    let index = app.session.kern_index().unwrap();
    let master = font.masters[0].id.clone();
    let state = &app.state.panels.text_preview;
    assert_eq!(state.length, MAX_CHARS);
    let mut best = std::time::Duration::MAX;
    for _ in 0..3 {
        let start = std::time::Instant::now();
        let (_, layout) = text_preview::lay_out(&font, &master, state, 20_000.0, Some(&index)).unwrap();
        let took = start.elapsed();
        best = best.min(took);
        assert_eq!(layout.placed.len(), MAX_CHARS);
        println!("{} glyphs in the font, {} lines: {took:?}", font.glyphs.len(), layout.lines.len());
    }
    if !cfg!(debug_assertions) {
        assert!(best.as_millis() < 500, "1,000,000 characters took {best:?}");
    }
}

#[test]
fn cmd_f_opens_the_find_bar_only_while_the_text_preview_has_the_keyboard() {
    let mut app = preview_app();
    app.run("view.newTab", &json!({"glyphs": ["A", "V"]})).unwrap();
    exec(&mut app, "window.textPreview", json!({}));
    let w = Window::new();
    w.frame(&mut app);
    w.frame(&mut app);
    w.cmd_f(&mut app);
    assert!(app.state.panels.text_preview.find.open, "the window opened last and has the keyboard");
    assert!(!app.state.panels.find_glyphs.open);

    exec(&mut app, "textPreview.find", json!({"open": false}));
    let outside = Pos2::new(5.0, 790.0);
    let button =
        |pressed| egui::Event::PointerButton { pos: outside, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    w.events(&mut app, vec![egui::Event::PointerMoved(outside), button(true)], egui::Modifiers::NONE);
    w.events(&mut app, vec![button(false)], egui::Modifiers::NONE);
    assert!(!app.state.panels.text_preview.keyboard);
    w.cmd_f(&mut app);
    assert!(!app.state.panels.text_preview.find.open);
    assert!(app.state.panels.find_glyphs.open, "Cmd+F is the Edit View's Find again");
}
