//! The Layers section's gestures in headless frames: dragging a row within its master, dropping a
//! backup on its master's row, the refused drop under another master, and a master-row click
//! with glyphs selected in the text. Each must give what its command gives.

use std::sync::Arc;

use egui::{CursorIcon, Event, Modifiers, PointerButton, Pos2, Rect, vec2};
use fontcraft_model::Layer;
use serde_json::{Value, json};

use crate::FontcraftApp;
use crate::panels::layers::{drop_zone, name_part};

/// A headless window kept across frames, tall enough for the whole Palette.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    /// Runs a frame and returns the cursor it asked for.
    fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) -> CursorIcon {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 1400.0))), events, ..Default::default() };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
        output.platform_output.cursor_icon
    }

    fn settle(&self, app: &mut FontcraftApp) {
        for _ in 0..3 {
            self.frame(app, Vec::new());
        }
    }

    fn rect(&self, id: egui::Id) -> Rect {
        self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect
    }

    /// Drags from `from` to `to` and drops there; returns the cursor shown just before the drop.
    fn drag(&self, app: &mut FontcraftApp, from: Pos2, to: Pos2) -> CursorIcon {
        let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.frame(app, vec![Event::PointerMoved(from)]);
        self.frame(app, vec![button(from, true)]);
        self.frame(app, vec![Event::PointerMoved(from + vec2(0.0, 12.0))]);
        self.frame(app, vec![Event::PointerMoved(to)]);
        let cursor = self.frame(app, vec![Event::PointerMoved(to)]);
        self.frame(app, vec![button(to, false)]);
        self.settle(app);
        cursor
    }

    /// Drags the row of `layer` by its name onto the row of `onto`, in its upper or lower half.
    fn drag_row(&self, app: &mut FontcraftApp, layer: &str, onto: &str, upper: bool) -> CursorIcon {
        let from = self.rect(name_part(layer)).center();
        let zone = self.rect(drop_zone(onto));
        let to = zone.center() + vec2(0.0, if upper { -zone.height() / 4.0 } else { zone.height() / 4.0 });
        self.drag(app, from, to)
    }

    fn click(&self, app: &mut FontcraftApp, id: egui::Id) {
        let at = self.rect(id).center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.frame(app, vec![Event::PointerMoved(at)]);
        self.frame(app, vec![button(true)]);
        self.frame(app, vec![button(false)]);
        self.settle(app);
    }
}

/// Two masters, `m01` and `m02` (every glyph has a layer in both). Glyph A has the backups `one`,
/// `two` and `three` under `m01` and `other` under `m02`; returns their ids in that order.
fn two_masters() -> (FontcraftApp, [String; 4]) {
    let mut app = FontcraftApp::default();
    app.session
        .edit("setup", |font| {
            let mut m = font.default_master()?.clone();
            m.id = "m02".into();
            m.name = "Bold".into();
            Arc::make_mut(&mut font.masters).push(m);
            for name in ["A", "B", "C"] {
                font.glyph_mut(name)?.layers.push(Layer::new("m02", 600.0));
            }
            Ok(())
        })
        .unwrap();
    let mut add = |master: &str, name: &str| {
        app.run("layers.addBackupLayer", &json!({"glyphs": ["A"], "master": master, "name": name})).unwrap()["layer"].as_str().unwrap().to_string()
    };
    let ids = [add("m01", "one"), add("m01", "two"), add("m01", "three"), add("m02", "other")];
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    (app, ids)
}

/// The layer ids of A in the order the section lists them.
fn rows(app: &mut FontcraftApp) -> Vec<String> {
    let list = app.session.execute("layers.list", &json!({"glyph": "A"})).unwrap();
    list["layers"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect()
}

fn last(app: &FontcraftApp) -> (String, Value) {
    app.session.journal().last().cloned().expect("a command ran")
}

#[test]
fn dragging_a_row_within_its_master_runs_reorder() {
    let (mut app, [one, two, three, other]) = two_masters();
    let w = Window::new();
    w.settle(&mut app);

    // Onto the upper half of `one`: first among m01's layers.
    let cursor = w.drag_row(&mut app, &three, &one, true);
    assert_eq!(cursor, CursorIcon::Grabbing);
    assert_eq!(last(&app), ("layers.reorder".to_string(), json!({"glyph": "A", "layer": three, "toIndex": 0})));
    assert_eq!(rows(&mut app), ["m01", three.as_str(), one.as_str(), two.as_str(), "m02", other.as_str()]);

    // Onto the lower half of `two`: after it.
    w.drag_row(&mut app, &three, &two, false);
    assert_eq!(last(&app).1["toIndex"], 2);
    assert_eq!(rows(&mut app), ["m01", one.as_str(), two.as_str(), three.as_str(), "m02", other.as_str()]);

    // Dropped where it already is: nothing runs.
    let before = app.session.journal().len();
    w.drag_row(&mut app, &two, &three, true);
    assert_eq!(app.session.journal().len(), before);
}

#[test]
fn a_drop_under_another_master_is_refused() {
    let (mut app, [one, _, _, other]) = two_masters();
    let w = Window::new();
    w.settle(&mut app);
    let (before, journal) = (rows(&mut app), app.session.journal().len());

    assert_eq!(w.drag_row(&mut app, &one, &other, true), CursorIcon::NoDrop, "onto another master's backup");
    assert_eq!(w.drag_row(&mut app, &one, "m02", false), CursorIcon::NoDrop, "onto another master's row");
    assert_eq!(rows(&mut app), before);
    assert_eq!(app.session.journal().len(), journal, "nothing ran");
}

#[test]
fn dropping_a_backup_on_its_master_row_uses_it_as_master() {
    let (mut app, [_, two, _, _]) = two_masters();
    let w = Window::new();
    w.settle(&mut app);
    // Give the backup a shape the master layer lacks.
    app.run("path.addRectangle", &json!({"glyph": "A", "layer": two, "x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    w.settle(&mut app);
    let shapes =
        |app: &FontcraftApp, id: &str| app.session.font().unwrap().glyph("A").unwrap().layers.iter().find(|l| l.id == id).map(|l| l.shapes.clone());
    let restored = shapes(&app, &two);

    w.drag_row(&mut app, &two, "m01", true);
    assert_eq!(last(&app), ("layers.useAsMaster".to_string(), json!({"glyphs": ["A"], "layer": two})));
    assert_eq!(shapes(&app, "m01"), restored);
    assert_eq!(shapes(&app, &two), None, "the backup is gone");
}

#[test]
fn a_master_row_click_switches_the_glyphs_selected_in_the_text() {
    let (mut app, _) = two_masters();
    app.run("editView.toggleTextMode", &json!({"mode": "text"})).unwrap();
    app.run("editView.setText", &json!({"text": "ABC"})).unwrap();
    app.run("editView.setCursor", &json!({"index": 0, "selectionLength": 2})).unwrap();
    let w = Window::new();
    w.settle(&mut app);

    w.click(&mut app, name_part("m02"));
    let tab = app.state.tab().unwrap();
    let switched: Vec<(&str, &str)> = tab.layers.iter().map(|(g, l)| (g.as_str(), l.as_str())).collect();
    assert_eq!(switched, [("A", "m02"), ("B", "m02")], "C, outside the selection, keeps the tab's master");

    // Without a text selection the click switches the edited glyph only.
    app.run("editView.setCursor", &json!({"index": 2})).unwrap();
    w.settle(&mut app);
    w.click(&mut app, name_part("m02"));
    let tab = app.state.tab().unwrap();
    assert_eq!(tab.layers.get("C").map(String::as_str), Some("m02"));
    assert_eq!(tab.layers.len(), 3);
}
