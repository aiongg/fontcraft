//! The glyph Info box's metrics fields, driven in headless frames: numbers and keys in each
//! field, the arrow steps, the out-of-sync and unlinked colours, the update button, the vertical
//! layout and the glyph after the text cursor.

use egui::{Color32, Event, Key, Modifiers, PointerButton, Pos2, Shape};
use serde_json::{Value, json};

use crate::FontcraftApp;
use crate::panels::info_box_metrics::{self, KeyState};
use crate::panels::widgets;

/// A headless window: one egui context kept across frames, so focus and field state carry over.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    /// One frame; the shapes it drew, nested lists flattened.
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

    /// Frames until the box has found its place: the glyph's position, then the box's size.
    fn settle(&self, app: &mut FontcraftApp) -> Vec<Shape> {
        for _ in 0..4 {
            self.frame(app, Vec::new());
        }
        self.frame(app, Vec::new())
    }

    fn focus(&self, app: &mut FontcraftApp, id: egui::Id) {
        self.ctx.memory_mut(|m| m.request_focus(id));
        self.frame(app, Vec::new());
        assert!(self.ctx.memory(|m| m.has_focus(id)), "the field takes the focus");
    }

    fn type_text(&self, app: &mut FontcraftApp, text: &str) {
        self.frame(app, vec![Event::Text(text.into())]);
    }

    fn key(&self, app: &mut FontcraftApp, key: Key, modifiers: Modifiers) {
        self.frame(app, vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
    }

    /// Focuses `id`, types `text` and presses Return.
    fn enter(&self, app: &mut FontcraftApp, id: egui::Id, text: &str) {
        self.focus(app, id);
        self.type_text(app, text);
        self.key(app, Key::Enter, Modifiers::NONE);
    }

    fn click(&self, app: &mut FontcraftApp, id: egui::Id) {
        let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.frame(app, vec![Event::PointerMoved(at)]);
        self.frame(app, vec![button(true)]);
        self.frame(app, vec![button(false)]);
    }

    fn drawn(&self, id: egui::Id) -> bool {
        self.ctx.read_response(id).is_some()
    }
}

fn field(name: &str) -> egui::Id {
    widgets::id(("infoBox.metric", name))
}

/// n (outline 40 to 460, width 500) and o (outline 30 to 470, width 500), n open in Edit View.
fn spacing_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "n", "x": 40, "y": 0, "width": 420, "height": 500})).unwrap();
    app.run("path.addRectangle", &json!({"glyph": "o", "x": 30, "y": 0, "width": 440, "height": 500})).unwrap();
    app.run("spacing.setWidth", &json!({"glyphs": ["n", "o"], "value": 500})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["n"]})).unwrap();
    app
}

fn master(app: &FontcraftApp) -> String {
    app.session.font().unwrap().masters[0].id.clone()
}

fn metrics(app: &mut FontcraftApp, glyph: &str) -> Value {
    app.run("spacing.getMetrics", &json!({ "glyph": glyph })).unwrap()
}

fn last(app: &FontcraftApp) -> (String, Value) {
    app.session.journal().last().cloned().expect("a command ran")
}

fn state(app: &mut FontcraftApp, name: &str) -> KeyState {
    let tokens = app.tokens.clone();
    let m = info_box_metrics::read(app, &tokens).expect("a current glyph");
    m.fields.iter().find(|f| f.field == name).map(|f| f.state).expect("the field is shown")
}

/// The colours the text `text` was drawn in.
fn colors_of(shapes: &[Shape], text: &str) -> Vec<Color32> {
    let mut out = Vec::new();
    for shape in shapes {
        if let Shape::Text(t) = shape
            && t.galley.text() == text
        {
            out.extend(t.override_text_color);
            out.extend(t.galley.job.sections.iter().map(|s| s.format.color));
        }
    }
    out
}

#[test]
fn lsb_rsb_and_width_fields_take_numbers_and_keys() {
    let mut app = spacing_app();
    let w = Window::new();
    w.settle(&mut app);
    for name in ["left", "right", "width"] {
        assert!(w.drawn(field(name)), "the {name} field is drawn");
    }
    assert!(!w.drawn(field("top")), "no vertical fields in horizontal text");

    w.enter(&mut app, field("left"), "50");
    let (id, params) = last(&app);
    assert_eq!(id, "spacing.setLeftSidebearing");
    assert_eq!(params, json!({"glyphs": ["n"], "layer": master(&app), "value": 50.0}));
    assert_eq!(metrics(&mut app, "n")["left"], 50.0);

    w.enter(&mut app, field("right"), "o");
    let (id, params) = last(&app);
    assert_eq!((id.as_str(), &params["value"]), ("spacing.setRightSidebearing", &json!("o")));
    let m = metrics(&mut app, "n");
    assert_eq!(m["right"], 30.0, "the key is applied at once");
    assert!(m["keys"]["right"].is_object(), "the key is stored");

    w.enter(&mut app, field("width"), "=600");
    assert_eq!(last(&app).0, "spacing.setWidth");
    assert_eq!(metrics(&mut app, "n")["width"], 600.0);
    w.settle(&mut app);
    assert_eq!(state(&mut app, "width"), KeyState::InSync);
    assert_eq!(state(&mut app, "left"), KeyState::Number);

    // Emptying a field that holds a key keeps the number and drops the key.
    let steps = app.session.journal().len();
    w.focus(&mut app, field("width"));
    w.key(&mut app, Key::Backspace, Modifiers::NONE);
    w.key(&mut app, Key::Enter, Modifiers::NONE);
    assert!(app.session.journal().len() > steps);
    let m = metrics(&mut app, "n");
    assert_eq!((m["width"].as_f64(), &m["keys"]["width"]), (Some(600.0), &Value::Null));
}

#[test]
fn arrows_step_by_one_ten_and_hundred_and_leave_keys_alone() {
    let mut app = spacing_app();
    let w = Window::new();
    w.settle(&mut app);
    w.focus(&mut app, field("left"));
    w.key(&mut app, Key::ArrowUp, Modifiers::NONE);
    assert_eq!(last(&app).1["value"], 41.0);
    w.key(&mut app, Key::ArrowUp, Modifiers::SHIFT);
    assert_eq!(last(&app).1["value"], 51.0);
    w.key(&mut app, Key::ArrowDown, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(last(&app).1["value"], -49.0);
    assert_eq!(metrics(&mut app, "n")["left"], -49.0);

    app.run("spacing.setRightSidebearing", &json!({"glyphs": ["n"], "value": "o"})).unwrap();
    w.settle(&mut app);
    let steps = app.session.journal().len();
    w.focus(&mut app, field("right"));
    w.key(&mut app, Key::ArrowUp, Modifiers::SHIFT);
    w.settle(&mut app);
    assert_eq!(app.session.journal().len(), steps, "arrows leave a key unchanged");
}

#[test]
fn out_of_sync_key_is_red_and_its_update_button_syncs_it() {
    let mut app = spacing_app();
    app.run("spacing.setRightSidebearing", &json!({"glyphs": ["n"], "value": "o"})).unwrap();
    app.run("spacing.setRightSidebearing", &json!({"glyphs": ["o"], "value": 70})).unwrap();
    let w = Window::new();
    let shapes = w.settle(&mut app);
    assert_eq!(state(&mut app, "right"), KeyState::OutOfSync);
    let key = metrics(&mut app, "n")["keys"]["right"]["key"].as_str().unwrap().to_string();
    let red = app.tokens.info_box_key_out_of_sync;
    assert!(colors_of(&shapes, &key).contains(&red), "the key {key} is drawn red");
    let update = widgets::id(("infoBox.updateMetric", "right"));
    assert!(!w.drawn(widgets::id(("infoBox.updateMetric", "left"))), "only out-of-sync keys get the button");

    w.click(&mut app, update);
    let (id, params) = last(&app);
    assert_eq!(id, "spacing.updateMetricsKey");
    assert_eq!(params, json!({"glyph": "n", "layer": master(&app), "field": "right"}));
    assert_eq!(metrics(&mut app, "n")["right"], 70.0);
    let shapes = w.settle(&mut app);
    assert_eq!(state(&mut app, "right"), KeyState::InSync);
    assert!(!w.drawn(update), "the button goes once the key is in sync");
    assert!(!colors_of(&shapes, &key).contains(&red));
}

#[test]
fn key_naming_a_removed_glyph_is_turquoise() {
    let mut app = spacing_app();
    app.run("glyph.newGlyph", &json!({"name": "q.alt", "width": 500})).unwrap();
    app.run("path.addRectangle", &json!({"glyph": "q.alt", "x": 60, "y": 0, "width": 380, "height": 500})).unwrap();
    app.run("spacing.setLeftSidebearing", &json!({"glyphs": ["n"], "value": "q.alt"})).unwrap();
    app.run("glyph.removeGlyph", &json!({"glyphs": ["q.alt"], "metricsKeys": "keep"})).unwrap();
    let w = Window::new();
    let shapes = w.settle(&mut app);
    assert_eq!(state(&mut app, "left"), KeyState::Unlinked);
    let turquoise = app.tokens.info_box_key_unlinked;
    let key = metrics(&mut app, "n")["keys"]["left"]["key"].as_str().unwrap().to_string();
    assert!(colors_of(&shapes, &key).contains(&turquoise), "the key {key} is drawn turquoise");
    assert!(!w.drawn(widgets::id(("infoBox.updateMetric", "left"))), "an unlinked key cannot be updated");

    w.enter(&mut app, field("left"), "45");
    assert_eq!(metrics(&mut app, "n")["keys"]["left"], Value::Null, "a number repairs it");
}

#[test]
fn vertical_text_shows_top_and_bottom_fields() {
    let mut app = spacing_app();
    app.run("editView.setWritingDirection", &json!({"direction": "ttb"})).unwrap();
    let w = Window::new();
    w.settle(&mut app);
    assert!(w.drawn(field("top")) && w.drawn(field("bottom")));
    assert!(!w.drawn(field("left")) && !w.drawn(field("width")), "the horizontal fields give way");

    w.enter(&mut app, field("top"), "120");
    let (id, params) = last(&app);
    assert_eq!((id.as_str(), &params["value"]), ("spacing.setTopSidebearing", &json!(120.0)));
    assert_eq!(metrics(&mut app, "n")["top"], 120.0);
    w.enter(&mut app, field("bottom"), "o");
    assert_eq!(last(&app).0, "spacing.setBottomSidebearing");
    assert!(metrics(&mut app, "n")["keys"]["bottom"].is_object());

    app.run("editView.setWritingDirection", &json!({"direction": "rtl"})).unwrap();
    w.settle(&mut app);
    assert!(w.drawn(field("left")) && !w.drawn(field("top")), "right-to-left text shows LSB and RSB");
}

#[test]
fn the_box_describes_the_glyph_after_the_text_cursor() {
    let mut app = spacing_app();
    app.run("editView.toggleTextMode", &json!({"mode": "text"})).unwrap();
    app.run("editView.setText", &json!({"text": "no"})).unwrap();
    app.run("editView.setCursor", &json!({"index": 1})).unwrap();
    let w = Window::new();
    w.settle(&mut app);
    let tokens = app.tokens.clone();
    assert_eq!(info_box_metrics::read(&mut app, &tokens).unwrap().glyph, "o");
    w.enter(&mut app, field("width"), "520");
    let (id, params) = last(&app);
    assert_eq!((id.as_str(), &params["glyphs"]), ("spacing.setWidth", &json!(["o"])));
    assert_eq!(metrics(&mut app, "o")["width"], 520.0);
    assert_eq!(metrics(&mut app, "n")["width"], 500.0);

    app.run("view.showInfo", &json!({"on": false})).unwrap();
    w.settle(&mut app);
    assert!(!w.drawn(field("width")), "Show Info off hides the box");
}

/// Aacute built from A (outline 20–580, width 600) and a wide acute that reaches past both sides,
/// stored as alignment places it; open in Edit View.
fn aligned_app() -> FontcraftApp {
    use fontcraft_engine::model::{Anchor, Component, Glyph, Layer, Path, Shape as Drawn};
    let mut app = FontcraftApp::default();
    app.session
        .edit("fixture", |f| {
            let master = f.masters[0].id.clone();
            {
                let mut a = f.glyph_mut("A")?;
                let l = a.layers.iter_mut().find(|l| l.id == master).expect("A has the master's layer");
                l.shapes = vec![Drawn::Path(Path::rect(20.0, 0.0, 580.0, 700.0))];
                l.width = 600.0;
                l.anchors.push(Anchor::new("top", 300.0, 700.0));
            }
            let mut acute = Glyph::new("acutecomb");
            let mut l = Layer::new(&master, 0.0);
            l.shapes.push(Drawn::Path(Path::rect(-400.0, 720.0, 400.0, 800.0)));
            l.anchors.push(Anchor::new("_top", 0.0, 500.0));
            acute.layers.push(l);
            f.glyphs.push(acute);
            let mut aacute = Glyph::new("Aacute");
            let mut l = Layer::new(&master, 600.0);
            l.shapes.push(Drawn::Component(Component::new("A", 0.0, 0.0)));
            l.shapes.push(Drawn::Component(Component::new("acutecomb", 300.0, 200.0)));
            aacute.layers.push(l);
            f.glyphs.push(aacute);
            Ok(())
        })
        .unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["Aacute"]})).unwrap();
    app
}

fn shown(app: &mut FontcraftApp) -> Vec<String> {
    let tokens = app.tokens.clone();
    let m = info_box_metrics::read(app, &tokens).expect("a current glyph");
    m.fields.iter().map(info_box_metrics::MetricField::shown).collect()
}

#[test]
fn an_aligned_composite_shows_auto_values_and_takes_offsets() {
    let mut app = aligned_app();
    let w = Window::new();
    w.settle(&mut app);
    assert_eq!(state(&mut app, "left"), KeyState::Auto);
    // The derived sidebearings leave the mark out.
    assert_eq!(shown(&mut app), ["auto (20)", "auto (20)", "auto (600)"]);

    w.enter(&mut app, field("left"), "=+30");
    let (id, params) = last(&app);
    assert_eq!((id.as_str(), &params["value"]), ("spacing.setLeftSidebearing", &json!("=+30")));
    w.settle(&mut app);
    assert_eq!(shown(&mut app), ["=+30 (50)", "auto (20)", "auto (630)"]);

    // A number is sent as typed and refused: nothing changes.
    let steps = app.session.journal().len();
    w.enter(&mut app, field("right"), "40");
    w.settle(&mut app);
    assert_eq!(app.session.journal().len(), steps, "the setter refuses a number on an aligned glyph");
    assert_eq!(metrics(&mut app, "Aacute")["width"], 630.0);

    // Emptying the field that holds the offset removes it.
    w.focus(&mut app, field("left"));
    w.key(&mut app, Key::Backspace, Modifiers::NONE);
    w.key(&mut app, Key::Enter, Modifiers::NONE);
    assert_eq!(last(&app).1["value"], json!("=+0"));
    assert_eq!(metrics(&mut app, "Aacute")["width"], 600.0);
}
