//! The Palette, the Info box and the bottom bar, driven like a user would in headless frames:
//! focus a field, type, press Return, Tab, Escape or the arrows, click a button; then check the
//! command that ran and what it did.

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use serde_json::{Value, json};

use crate::FontcraftApp;
use crate::panels::widgets;

/// A headless window: one egui context kept across frames, so focus and field state carry over.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
    }

    /// A few empty frames: new areas size themselves and focus requests land.
    fn settle(&self, app: &mut FontcraftApp) {
        for _ in 0..3 {
            self.frame(app, Vec::new());
        }
    }

    fn focus(&self, app: &mut FontcraftApp, id: egui::Id) {
        self.ctx.memory_mut(|m| m.request_focus(id));
        self.frame(app, Vec::new());
        assert!(self.focused(id), "the field takes the focus");
    }

    fn focused(&self, id: egui::Id) -> bool {
        self.ctx.memory(|m| m.has_focus(id))
    }

    fn type_text(&self, app: &mut FontcraftApp, text: &str) {
        self.frame(app, vec![Event::Text(text.into())]);
    }

    fn key(&self, app: &mut FontcraftApp, key: Key, modifiers: Modifiers) {
        self.frame(app, vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
    }

    /// Clicks the middle of the control `id` as it was drawn in the last frame.
    fn click(&self, app: &mut FontcraftApp, id: egui::Id) {
        let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.frame(app, vec![Event::PointerMoved(at)]);
        self.frame(app, vec![button(true)]);
        self.frame(app, vec![button(false)]);
    }
}

/// Glyph A with a square (shape 0, from (100, 100) to (300, 300)) and an open curve (shape 1:
/// (400, 100), handles (400, 200) and (450, 300), to (500, 300)), open in Edit View.
fn edit_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    let curve = json!([
        {"x": 400, "y": 100}, {"x": 400, "y": 200, "type": "offcurve"}, {"x": 450, "y": 300, "type": "offcurve"}, {"x": 500, "y": 300, "type": "curve"}
    ]);
    app.run("path.addPath", &json!({"glyph": "A", "closed": false, "nodes": curve})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app
}

fn select_square(app: &mut FontcraftApp) {
    app.run("selection.set", &json!({"points": [[0, 0], [0, 1], [0, 2], [0, 3]]})).unwrap();
}

/// The bounds of the square: [min x, min y, max x, max y].
fn square(app: &FontcraftApp) -> [f64; 4] {
    let font = app.session.font().unwrap();
    let layer = font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap().clone();
    let fontcraft_model::Shape::Path(path) = &layer.shapes[0] else { panic!("shape 0 is not a path") };
    let xs = path.nodes.iter().map(|n| n.x);
    let ys = path.nodes.iter().map(|n| n.y);
    [xs.clone().fold(f64::MAX, f64::min), ys.clone().fold(f64::MAX, f64::min), xs.fold(f64::MIN, f64::max), ys.fold(f64::MIN, f64::max)]
}

/// The last command the document recorded.
fn last(app: &FontcraftApp) -> (String, Value) {
    app.session.journal().last().cloned().expect("a command ran")
}

fn journal_len(app: &FontcraftApp) -> usize {
    app.session.journal().len()
}

#[test]
fn info_box_x_commits_with_return_through_set_position() {
    let mut app = edit_app();
    select_square(&mut app);
    let w = Window::new();
    w.settle(&mut app);
    let x = widgets::id("infoBox.x");
    w.focus(&mut app, x);
    w.type_text(&mut app, "150");
    assert_eq!(square(&app)[0], 100.0, "typing alone changes nothing");
    w.key(&mut app, Key::Enter, Modifiers::NONE);
    let (id, params) = last(&app);
    assert_eq!(id, "selection.setPosition");
    assert_eq!(params["x"].as_f64(), Some(150.0));
    assert_eq!(params["referencePoint"], "bottomLeft");
    assert_eq!(params["points"], json!([[0, 0], [0, 1], [0, 2], [0, 3]]));
    assert_eq!(square(&app), [150.0, 100.0, 350.0, 300.0]);
    assert!(!w.focused(x), "Return leaves the field");
    let steps = journal_len(&app);
    w.settle(&mut app);
    assert_eq!(journal_len(&app), steps, "the edit commits once");

    app.run("view.showInfo", &json!({"on": false})).unwrap();
    w.settle(&mut app);
    assert!(w.ctx.read_response(x).is_none(), "Show Info off hides the Info box");
}

#[test]
fn tab_commits_and_moves_on_escape_cancels() {
    let mut app = edit_app();
    select_square(&mut app);
    let w = Window::new();
    w.settle(&mut app);
    let (x, y) = (widgets::id("infoBox.x"), widgets::id("infoBox.y"));
    w.focus(&mut app, x);
    w.type_text(&mut app, "120");
    w.key(&mut app, Key::Tab, Modifiers::NONE);
    assert_eq!(last(&app).0, "selection.setPosition");
    assert_eq!(square(&app)[0], 120.0);
    w.frame(&mut app, Vec::new());
    assert!(w.focused(y), "Tab moves to the next field");

    let steps = journal_len(&app);
    w.type_text(&mut app, "999");
    w.key(&mut app, Key::Escape, Modifiers::NONE);
    w.settle(&mut app);
    assert_eq!(journal_len(&app), steps, "Escape commits nothing");
    assert_eq!(square(&app)[1], 100.0);
    assert!(w.ctx.memory(|m| m.focused()).is_none(), "Escape leaves no field focused");
    assert_eq!(app.state.tab().unwrap().selection.points.len(), 4, "the canvas did not take the Escape");
}

#[test]
fn arrows_step_a_field_by_one_by_ten_with_shift_and_by_a_hundred_with_cmd_shift() {
    let mut app = edit_app();
    select_square(&mut app);
    let w = Window::new();
    w.settle(&mut app);
    let y = widgets::id("infoBox.y");
    w.focus(&mut app, y);
    w.key(&mut app, Key::ArrowUp, Modifiers::SHIFT);
    assert_eq!(last(&app).1["y"].as_f64(), Some(110.0));
    assert_eq!(square(&app)[1], 110.0);
    w.key(&mut app, Key::ArrowDown, Modifiers::NONE);
    assert_eq!(square(&app)[1], 109.0);
    w.key(&mut app, Key::ArrowUp, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(square(&app)[1], 209.0);
    assert!(w.focused(y), "the field stays focused");
}

#[test]
fn width_scales_about_the_reference_point_chosen_on_the_grid() {
    let mut app = edit_app();
    select_square(&mut app);
    let w = Window::new();
    w.settle(&mut app);
    w.click(&mut app, widgets::id(("infoBox.reference", "right")));
    assert_eq!(app.state.panels.reference_point, "right");
    w.focus(&mut app, widgets::id("infoBox.width"));
    w.type_text(&mut app, "400");
    w.key(&mut app, Key::Enter, Modifiers::NONE);
    let (id, params) = last(&app);
    assert_eq!(id, "transform.scale");
    assert_eq!((params["x"].as_f64(), params["y"].as_f64()), (Some(200.0), Some(100.0)), "the lock is open");
    assert_eq!(params["origin"], json!({"mode": "box", "boxPoint": "right"}));
    assert_eq!(square(&app), [-100.0, 100.0, 300.0, 300.0], "the right edge stays");
}

#[test]
fn palette_buttons_and_fields_run_their_commands() {
    let mut app = edit_app();
    let w = Window::new();
    app.run("palette.toggleSection", &json!({"section": "dimensions"})).unwrap();
    assert_eq!(app.state.panels.collapsed, ["dimensions"]);
    app.run("selection.set", &json!({"points": [[1, 1]]})).unwrap();
    w.settle(&mut app);

    w.click(&mut app, widgets::id(("fitCurve.step", 8u8)));
    let (id, params) = last(&app);
    assert_eq!(id, "fitCurve.applyStep");
    assert_eq!((params["step"].as_u64(), params["min"].as_f64(), params["max"].as_f64()), (Some(8), Some(48.0), Some(74.0)));
    assert_eq!(params["points"], json!([[1, 1]]));

    w.focus(&mut app, widgets::id("fitCurve.min"));
    w.type_text(&mut app, "50");
    w.key(&mut app, Key::Enter, Modifiers::NONE);
    assert_eq!(app.state.panels.fit_curve_range, Some([50.0, 74.0]));

    let layers = app.session.font().unwrap().glyph("A").unwrap().layers.len();
    w.click(&mut app, widgets::id("layers.add"));
    assert_eq!(last(&app).0, "layers.addBackupLayer");
    assert_eq!(app.session.font().unwrap().glyph("A").unwrap().layers.len(), layers + 1);

    select_square(&mut app);
    app.run("palette.toggleSection", &json!({"section": "layers", "expanded": false})).unwrap();
    w.settle(&mut app);
    w.click(&mut app, widgets::id("transform.scale"));
    let (id, params) = last(&app);
    assert_eq!(id, "transform.scale");
    assert_eq!((params["x"].as_f64(), params["y"].as_f64()), (Some(110.0), Some(110.0)));
    assert_eq!(params["origin"], json!({"mode": "box", "boxPoint": "center"}));
    assert_eq!(square(&app), [90.0, 90.0, 310.0, 310.0]);
}

#[test]
fn dimensions_and_zoom_fields_run_their_commands() {
    let mut app = edit_app();
    app.run("dimensions.setValue", &json!({"key": "nV", "value": 80})).unwrap();
    let w = Window::new();
    w.settle(&mut app);
    w.focus(&mut app, widgets::id(("dimensions", "nV")));
    w.type_text(&mut app, "90");
    w.key(&mut app, Key::Enter, Modifiers::NONE);
    let (id, params) = last(&app);
    assert_eq!(id, "dimensions.setValue");
    assert_eq!((params["key"].as_str(), params["value"].as_f64()), (Some("nV"), Some(90.0)));

    w.focus(&mut app, widgets::id("bottomBar.zoom"));
    w.type_text(&mut app, "2000");
    w.key(&mut app, Key::Enter, Modifiers::NONE);
    assert_eq!(app.state.tab().unwrap().view.scale, 2.0);
}

#[test]
fn panel_commands_check_their_parameters() {
    let mut app = FontcraftApp::default();
    assert!(app.run("palette.toggleSection", &json!({"section": "nope"})).is_err());
    assert!(app.run("infoBox.setReferencePoint", &json!({"point": "middle"})).is_err());
    assert!(app.run("transform.setOrigin", &json!({"mode": "reference"})).is_err(), "a reference origin needs a point");
    assert!(app.run("transform.setSettings", &json!({"slant": 90})).is_err());
    assert!(app.run("transform.setSettings", &json!({"scaleX": 0})).is_err());
    assert_eq!(app.run("fitCurve.setRange", &json!({"min": -5, "max": 500})).unwrap(), json!({"min": 1.0, "max": 100.0}));
    app.run("transform.setOrigin", &json!({"mode": "metrics", "metric": "xHeight"})).unwrap();
    app.run("transform.setSettings", &json!({"rotate": 45})).unwrap();
    let settings = app.run("transform.getSettings", &json!({})).unwrap();
    assert_eq!(settings["origin"]["mode"], "metrics");
    assert_eq!(settings["rotateAngle"].as_f64(), Some(45.0));
    assert_eq!(app.state.transform.origin_param(), json!({"mode": "metrics", "metric": "xHeight"}));
}

#[test]
fn annotation_palette_replaces_the_info_box_and_chooses_the_sub_tool() {
    let mut app = edit_app();
    select_square(&mut app);
    let w = Window::new();
    w.settle(&mut app);
    let button = |name: &str| widgets::id(("infoBox.annotation", name));
    assert!(w.ctx.read_response(button("select")).is_none(), "no palette with the Select tool");
    assert!(w.ctx.read_response(widgets::id("infoBox.x")).is_some());

    app.run("tool.select", &json!({"tool": "annotation"})).unwrap();
    w.settle(&mut app);
    assert!(w.ctx.read_response(widgets::id("infoBox.x")).is_none(), "the palette replaces the selection fields");
    let steps = journal_len(&app);
    for (name, id) in [("circle", "annotationCircle"), ("text", "annotationText"), ("minus", "annotationMinus"), ("select", "annotationSelect")] {
        w.click(&mut app, button(name));
        assert_eq!(app.tools.toolbox.base_id(), id);
    }
    assert_eq!(journal_len(&app), steps, "a click on the palette adds no annotation to the canvas below");

    app.run("view.showInfo", &json!({"on": false})).unwrap();
    w.settle(&mut app);
    assert!(w.ctx.read_response(button("select")).is_none(), "Show Info off hides the palette");
    app.run("view.showInfo", &json!({"on": true})).unwrap();
    app.run("tool.select", &json!({"tool": "draw"})).unwrap();
    w.settle(&mut app);
    assert!(w.ctx.read_response(button("select")).is_none(), "leaving the Annotation tools hides the palette");
}
