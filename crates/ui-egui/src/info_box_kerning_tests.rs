//! The Info box's kerning part and the preceding glyph's box, driven in headless frames: type
//! into the kern and group fields, step them with the arrows, empty them, click the locks; then
//! check the command that ran and the kerning it left.

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

    fn settle(&self, app: &mut FontcraftApp) {
        for _ in 0..3 {
            self.frame(app, Vec::new());
        }
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

    /// Focuses field `id`, types `text` over what it shows and presses Return.
    fn enter(&self, app: &mut FontcraftApp, id: egui::Id, text: &str) {
        self.focus(app, id);
        self.type_text(app, text);
        self.key(app, Key::Enter, Modifiers::NONE);
        self.settle(app);
    }

    fn click(&self, app: &mut FontcraftApp, id: egui::Id) {
        let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.frame(app, vec![Event::PointerMoved(at)]);
        self.frame(app, vec![button(true)]);
        self.frame(app, vec![button(false)]);
        self.settle(app);
    }
}

/// T with the right group `T`, o and ö (`odieresis`) with the left group `o`, `@T @o` kerned −40;
/// a tab in text mode holding "Töo" with the cursor before ö, and the Text tool.
fn kern_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("glyph.newGlyph", &json!({"name": "odieresis"})).unwrap();
    app.run("kerning.setRightGroup", &json!({"glyphs": ["T"], "group": "T"})).unwrap();
    app.run("kerning.setLeftGroup", &json!({"glyphs": ["o", "odieresis"], "group": "o"})).unwrap();
    app.run("kerning.setPair", &json!({"left": "@T", "right": "@o", "value": -40})).unwrap();
    app.run("view.newTab", &json!({"glyphs": ["T"]})).unwrap();
    app.run("editView.toggleTextMode", &json!({"mode": "text"})).unwrap();
    app.run("editView.setText", &json!({"glyphs": ["T", "odieresis", "o"]})).unwrap();
    app.run("editView.setCursor", &json!({"index": 1})).unwrap();
    app.run("tool.select", &json!({"tool": "text"})).unwrap();
    app
}

fn get_pair(app: &mut FontcraftApp, left: &str, right: &str, direction: &str) -> Value {
    app.run("kerning.getPair", &json!({"left": left, "right": right, "direction": direction})).unwrap()
}

/// The last command the document recorded.
fn last(app: &FontcraftApp) -> (String, Value) {
    app.session.journal().last().cloned().expect("a command ran")
}

fn journal_len(app: &FontcraftApp) -> usize {
    app.session.journal().len()
}

fn sides(params: &Value) -> (Value, Value, Option<f64>) {
    (params["left"].clone(), params["right"].clone(), params["value"].as_f64())
}

#[test]
fn info_box_kerning_field_sets_the_pair_the_locks_name_steps_and_clearing_removes_it() {
    let mut app = kern_app();
    let w = Window::new();
    w.settle(&mut app);
    let kern_left = widgets::id("infoBox.kernLeft");

    // Both locks closed: the typed value goes to the group pair.
    w.enter(&mut app, kern_left, "-55");
    let (id, params) = last(&app);
    assert_eq!(id, "kerning.setPair");
    assert_eq!(sides(&params), (json!("@T"), json!("@o"), Some(-55.0)));
    assert_eq!(get_pair(&mut app, "T", "o", "ltr")["value"], -55.0, "the group pair kerns T o too");

    // Up by 1, Shift+Up by 10, Cmd+Shift+Down by 100, each committed at once.
    w.focus(&mut app, kern_left);
    w.key(&mut app, Key::ArrowUp, Modifiers::NONE);
    assert_eq!(get_pair(&mut app, "T", "odieresis", "ltr")["value"], -54.0);
    w.key(&mut app, Key::ArrowUp, Modifiers::SHIFT);
    assert_eq!(get_pair(&mut app, "T", "odieresis", "ltr")["value"], -44.0);
    w.key(&mut app, Key::ArrowDown, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(get_pair(&mut app, "T", "odieresis", "ltr")["value"], -144.0);
    w.key(&mut app, Key::Enter, Modifiers::NONE);
    w.settle(&mut app);

    // Emptying the field removes the pair that applies; the field then shows the placeholder.
    let steps = journal_len(&app);
    w.focus(&mut app, kern_left);
    w.key(&mut app, Key::Backspace, Modifiers::NONE);
    w.key(&mut app, Key::Enter, Modifiers::NONE);
    w.settle(&mut app);
    assert_eq!(journal_len(&app), steps + 1, "one command for the emptied field");
    let (id, params) = last(&app);
    assert_eq!(id, "kerning.removePair");
    assert_eq!((params["left"].clone(), params["right"].clone()), (json!("@T"), json!("@o")));
    assert!(get_pair(&mut app, "T", "odieresis", "ltr")["step"].is_null(), "no pair applies");

    // Up in the empty field starts from zero.
    w.focus(&mut app, kern_left);
    w.key(&mut app, Key::ArrowUp, Modifiers::NONE);
    assert_eq!(sides(&last(&app).1), (json!("@T"), json!("@o"), Some(1.0)));
    w.key(&mut app, Key::Escape, Modifiers::NONE);

    // The right edge: ö has no right group, o's left group is closed.
    w.enter(&mut app, widgets::id("infoBox.kernRight"), "12");
    let (id, params) = last(&app);
    assert_eq!(id, "kerning.setPair");
    assert_eq!(sides(&params), (json!("odieresis"), json!("@o"), Some(12.0)));
}

#[test]
fn info_box_kerning_locks_open_and_close_exceptions() {
    let mut app = kern_app();
    let w = Window::new();
    w.settle(&mut app);

    // ö's lock towards T: opening it stores T's group against ö at the value that applied.
    w.click(&mut app, widgets::id("infoBox.lockLeft"));
    let (id, params) = last(&app);
    assert_eq!(id, "kerning.addException");
    assert_eq!((params["left"].clone(), params["right"].clone(), params["side"].clone()), (json!("T"), json!("odieresis"), json!("right")));
    let p = get_pair(&mut app, "T", "odieresis", "ltr");
    assert_eq!((p["locks"]["right"].clone(), p["value"].clone()), (json!("open"), json!(-40.0)));
    assert_eq!(p["pair"], json!({"left": "@T", "right": "odieresis"}));

    // Clicking it again closes it: the exception is gone.
    w.click(&mut app, widgets::id("infoBox.lockLeft"));
    assert_eq!(last(&app).0, "kerning.removeException");
    assert_eq!(get_pair(&mut app, "T", "odieresis", "ltr")["locks"]["right"], "closed");

    // The preceding glyph's lock is T's side of the pair.
    w.click(&mut app, widgets::id("infoBox.preceding.lock"));
    let (id, params) = last(&app);
    assert_eq!((id.as_str(), params["side"].clone()), ("kerning.addException", json!("left")));
    let p = get_pair(&mut app, "T", "odieresis", "ltr");
    assert_eq!((p["locks"]["left"].clone(), p["pair"].clone()), (json!("open"), json!({"left": "T", "right": "@o"})));

    // ö has no right group, so its right lock does nothing.
    let steps = journal_len(&app);
    w.click(&mut app, widgets::id("infoBox.lockRight"));
    assert_eq!(journal_len(&app), steps);
}

#[test]
fn info_box_kerning_refused_lock_shows_in_the_status_line() {
    let mut app = kern_app();
    app.run("kerning.setPair", &json!({"left": "T", "right": "@o", "value": -30})).unwrap();
    app.run("kerning.setPair", &json!({"left": "T", "right": "odieresis", "value": -20})).unwrap();
    let p = get_pair(&mut app, "T", "odieresis", "ltr");
    assert_eq!(p["locks"], json!({"left": "open", "right": "open"}));
    let w = Window::new();
    w.settle(&mut app);
    let steps = journal_len(&app);
    // Closing T's lock alone would leave `T @o` applying before `@T ö`.
    w.click(&mut app, widgets::id("infoBox.preceding.lock"));
    assert_eq!(journal_len(&app), steps, "the refused command changes nothing");
    assert!(app.state.status.contains("cannot close alone"), "{}", app.state.status);
    assert_eq!(get_pair(&mut app, "T", "odieresis", "ltr")["value"], -20.0);
}

#[test]
fn info_box_kerning_group_fields_and_the_preceding_glyph_box() {
    let mut app = kern_app();
    let w = Window::new();
    w.settle(&mut app);
    let groups = |app: &FontcraftApp, glyph: &str| app.session.font().unwrap().glyph(glyph).unwrap().kern_groups.clone();

    w.enter(&mut app, widgets::id("infoBox.groupLeft"), "oo");
    let (id, params) = last(&app);
    assert_eq!((id.as_str(), params["glyphs"].clone(), params["group"].clone()), ("kerning.setLeftGroup", json!(["odieresis"]), json!("oo")));
    w.enter(&mut app, widgets::id("infoBox.groupRight"), "od");
    assert_eq!(last(&app).0, "kerning.setRightGroup");
    assert_eq!(groups(&app, "odieresis").right.as_deref(), Some("od"));

    // The preceding box edits T's group that faces ö: its right group.
    w.enter(&mut app, widgets::id("infoBox.preceding.group"), "TT");
    let (id, params) = last(&app);
    assert_eq!((id.as_str(), params["glyphs"].clone()), ("kerning.setRightGroup", json!(["T"])));
    assert_eq!(groups(&app, "T").right.as_deref(), Some("TT"));

    // An emptied group field clears the group.
    w.focus(&mut app, widgets::id("infoBox.groupLeft"));
    w.key(&mut app, Key::Backspace, Modifiers::NONE);
    w.key(&mut app, Key::Enter, Modifiers::NONE);
    w.settle(&mut app);
    assert_eq!(groups(&app, "odieresis").left, None);

    // The preceding glyph's box shows only with the Text tool.
    app.run("tool.select", &json!({"tool": "select"})).unwrap();
    w.settle(&mut app);
    assert!(w.ctx.read_response(widgets::id("infoBox.preceding.lock")).is_none());
    assert!(w.ctx.read_response(widgets::id("infoBox.kernLeft")).is_some(), "the kern fields stay");

    app.run("view.showInfo", &json!({"on": false})).unwrap();
    w.settle(&mut app);
    assert!(w.ctx.read_response(widgets::id("infoBox.kernLeft")).is_none(), "Show Info off hides the box");
}

#[test]
fn info_box_kerning_follows_right_to_left_text() {
    let mut app = kern_app();
    app.run("editView.setWritingDirection", &json!({"direction": "rtl"})).unwrap();
    let w = Window::new();
    w.settle(&mut app);

    // Right to left, the left edge is the pair with the glyph after ö, o, drawn on its left.
    w.enter(&mut app, widgets::id("infoBox.kernLeft"), "15");
    let (id, params) = last(&app);
    assert_eq!(id, "kerning.setPair");
    assert_eq!(sides(&params), (json!("@o"), json!("o"), Some(15.0)));
    assert_eq!(params["direction"], "rtl");
    assert_eq!(get_pair(&mut app, "odieresis", "o", "rtl")["value"], 15.0);

    w.click(&mut app, widgets::id("infoBox.lockLeft"));
    let (id, params) = last(&app);
    assert_eq!(id, "kerning.addException");
    assert_eq!((params["left"].clone(), params["right"].clone(), params["side"].clone()), (json!("odieresis"), json!("o"), json!("left")));
    assert_eq!(get_pair(&mut app, "odieresis", "o", "rtl")["locks"]["left"], "open");
}
