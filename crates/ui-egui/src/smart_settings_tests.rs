//! The Smart Glyph Settings and Smart Component settings windows and the Palette's Smart
//! Component section (M6.12), driven in headless frames as a person would use them: each control
//! gives the font its command gives, and a refused request changes nothing.

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use fontcraft_model::{Component, Glyph, Layer, Path, Shape};
use serde_json::{Value, json};

use crate::FontcraftApp;
use crate::dialogs::smart_component_settings::{Targets, rows, value_command, value_field_id};
use crate::dialogs::smart_glyph_settings::{SettingsTab, add_button_id, field_id, new_property_name, pole_command};
use crate::panels::{SECTIONS, palette_smart};

/// A headless window: one egui context kept across frames, so focus and fields carry over.
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

    /// Focuses field `id` (which selects its text), types `text` and presses Return.
    fn commit(&self, app: &mut FontcraftApp, id: egui::Id, text: &str) {
        self.ctx.memory_mut(|m| m.request_focus(id));
        self.frame(app, Vec::new());
        assert!(self.ctx.memory(|m| m.has_focus(id)), "the field takes the focus");
        let key = |key, pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
        self.frame(app, vec![Event::Text(text.into())]);
        self.frame(app, vec![key(Key::Enter, true), key(Key::Enter, false)]);
        self.frame(app, Vec::new());
    }

    /// Clicks the middle of control `id` as the last frame drew it.
    fn click(&self, app: &mut FontcraftApp, id: egui::Id) {
        let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
        let event = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.frame(app, vec![Event::PointerMoved(at)]);
        self.frame(app, vec![event(true)]);
        self.frame(app, vec![event(false)]);
        self.frame(app, Vec::new());
    }
}

/// `_part.bar`: a master layer with a 200 × 100 rectangle and a `Wide` layer with a 600 × 100
/// one, a Width property from 200 to 600 with the master layer at its minimum and `Wide` at its
/// maximum. `bars` holds two components of it; the first is selected and edited at actual size.
fn smart_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.session
        .edit("setup", |font| {
            let m = font.masters[0].id.clone();
            let mut part = Glyph::new("_part.bar");
            for (id, name, width) in [(m.as_str(), None, 200.0), ("wide", Some("Wide"), 600.0)] {
                let mut l = Layer::new(m.clone(), 600.0);
                l.id = id.into();
                l.info.name = name.map(String::from);
                l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, width, 100.0)));
                part.layers.push(l);
            }
            font.glyphs.push(part);
            let mut s = Glyph::new("bars");
            let mut l = Layer::new(m, 1600.0);
            l.shapes.push(Shape::Component(Component::new("_part.bar", 0.0, 0.0)));
            l.shapes.push(Shape::Component(Component::new("_part.bar", 800.0, 0.0)));
            s.layers.push(l);
            font.glyphs.push(s);
            Ok(())
        })
        .unwrap();
    app.run("smartGlyph.addProperty", &json!({"glyph": "_part.bar", "name": "Width", "min": 200, "max": 600})).unwrap();
    let master = app.session.font().unwrap().masters[0].id.clone();
    app.run("smartGlyph.setLayerPole", &json!({"glyph": "_part.bar", "layer": master, "property": "Width", "pole": "min"})).unwrap();
    app.run("smartGlyph.setLayerPole", &json!({"glyph": "_part.bar", "layer": "Wide", "property": "Width", "pole": "max"})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["bars"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [800, 50]})).unwrap();
    app.run("selection.set", &json!({"shapes": [0]})).unwrap();
    // The built-in Palette sections fold, so the contextual section sits in view.
    for (section, _) in SECTIONS {
        app.run("palette.toggleSection", &json!({"section": section, "expanded": false})).unwrap();
    }
    app
}

fn layer(app: &FontcraftApp, glyph: &str) -> Layer {
    let font = app.session.font().unwrap();
    font.glyph(glyph).unwrap().layer(&font.masters[0].id).unwrap().clone()
}

fn glyph(app: &FontcraftApp, name: &str) -> Glyph {
    app.session.font().unwrap().glyph(name).unwrap().as_ref().clone()
}

fn undo_steps(app: &FontcraftApp) -> usize {
    app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len)
}

#[test]
fn smart_component_settings_show_the_selected_component_and_its_field_sets_the_value_like_the_command() {
    let mut app = smart_app();
    let r = app.run("smartComponent.showSettings", &json!({})).unwrap();
    assert_eq!(r["components"], json!([0]), "the selected smart component");
    let width = &r["properties"][0];
    assert_eq!((width["name"].clone(), width["min"].clone(), width["max"].clone()), (json!("Width"), json!(200.0), json!(600.0)));
    assert_eq!(width["toggle"], json!(false));
    assert_eq!(width["value"], json!(200.0), "without a stored value, the end the master layer draws");
    assert!(app.state.panels.smart_component.open);

    let w = Window::new();
    w.settle(&mut app);
    let steps = undo_steps(&app);
    w.commit(&mut app, value_field_id("window", "Width"), "900");
    assert_eq!(undo_steps(&app), steps + 1, "{}", app.state.status);

    let mut headless = smart_app();
    headless.run("smartComponent.setValue", &json!({"glyph": "bars", "component": 0, "property": "Width", "value": 900})).unwrap();
    assert_eq!(layer(&app, "bars"), layer(&headless, "bars"), "a value past the range extrapolates, as the command allows");

    app.run("smartComponent.showSettings", &json!({"open": false})).unwrap();
    assert!(!app.state.panels.smart_component.open);
}

#[test]
fn the_palette_section_follows_the_selection_and_sets_the_value_like_the_command() {
    let mut app = smart_app();
    assert!(palette_smart::shown(&app), "a smart component is selected");
    let w = Window::new();
    w.settle(&mut app);
    w.commit(&mut app, value_field_id(palette_smart::SALT, "Width"), "450");
    let mut headless = smart_app();
    headless.run("smartComponent.setValue", &json!({"glyph": "bars", "component": 0, "property": "Width", "value": 450})).unwrap();
    assert_eq!(layer(&app, "bars"), layer(&headless, "bars"), "{}", app.state.status);

    app.run("selection.set", &json!({})).unwrap();
    assert!(!palette_smart::shown(&app), "nothing selected, no section");
}

#[test]
fn several_components_take_one_value_in_one_undo_step() {
    let mut app = smart_app();
    app.run("smartComponent.setValue", &json!({"glyph": "bars", "component": 1, "property": "Width", "value": 300})).unwrap();
    let r = app.run("smartComponent.showSettings", &json!({"glyph": "bars", "components": [1, 0]})).unwrap();
    assert_eq!(r["components"], json!([0, 1]));
    assert_eq!(r["properties"][0]["value"], Value::Null, "the components differ");

    let font = app.session.font().unwrap();
    let t = Targets { glyph: "bars".into(), layer: font.masters[0].id.clone(), side: Default::default(), components: vec![0, 1] };
    let row = rows(font, &t).into_iter().next().unwrap();
    let (id, params) = value_command(&t, &row, 500.0);
    assert_eq!(id, "command.batch");
    let steps = undo_steps(&app);
    app.run(id, &params).unwrap();
    assert_eq!(undo_steps(&app), steps + 1, "one undo step for both");
    let font = app.session.font().unwrap();
    let t = Targets { glyph: "bars".into(), layer: font.masters[0].id.clone(), side: Default::default(), components: vec![0, 1] };
    assert_eq!(rows(font, &t)[0].value, Some(500.0));
}

#[test]
fn smart_component_settings_refuse_what_is_not_a_smart_component() {
    let mut app = smart_app();
    app.run("path.addPath", &json!({"glyph": "bars", "closed": true, "nodes": [{"x": 0, "y": 0}, {"x": 10, "y": 0}, {"x": 10, "y": 10}]})).unwrap();
    app.run("smartComponent.showSettings", &json!({})).unwrap();
    let before = app.state.panels.smart_component.clone();
    assert!(app.run("smartComponent.showSettings", &json!({"glyph": "bars", "components": [2]})).is_err(), "a path");
    assert!(app.run("smartComponent.showSettings", &json!({"glyph": "bars", "components": [9]})).is_err(), "no such shape");
    assert!(app.run("smartComponent.showSettings", &json!({"glyph": "bars"})).is_err(), "`components` is required with `glyph`");
    assert!(app.run("smartComponent.showSettings", &json!({"glyph": "nope", "components": [0]})).is_err());
    app.run("selection.set", &json!({"shapes": [2]})).unwrap();
    assert!(app.run("smartComponent.showSettings", &json!({})).is_err(), "only a path is selected");
    assert_eq!(app.state.panels.smart_component, before, "a refused request changes nothing");
}

#[test]
fn smart_glyph_settings_fields_and_plus_button_run_the_property_commands() {
    let mut app = smart_app();
    let r = app.run("smartGlyph.showSettings", &json!({"glyph": "_part.bar"})).unwrap();
    assert_eq!(r["tab"], "properties");
    assert_eq!(r["properties"], json!([{"name": "Width", "min": 200.0, "max": 600.0, "toggle": false}]));
    assert_eq!(r["layers"].as_array().unwrap().len(), 2);

    let w = Window::new();
    w.settle(&mut app);
    w.commit(&mut app, field_id("min", 0), "150");
    w.commit(&mut app, field_id("name", 0), "Length");
    w.settle(&mut app);
    w.click(&mut app, add_button_id());

    let mut headless = smart_app();
    headless.run("smartGlyph.setProperty", &json!({"glyph": "_part.bar", "property": "Width", "min": 150})).unwrap();
    headless.run("smartGlyph.setProperty", &json!({"glyph": "_part.bar", "property": "Width", "name": "Length"})).unwrap();
    headless.run("smartGlyph.addProperty", &json!({"glyph": "_part.bar", "name": "Property"})).unwrap();
    assert_eq!(glyph(&app, "_part.bar"), glyph(&headless, "_part.bar"), "{}", app.state.status);
    assert_eq!(layer(&app, "bars"), layer(&headless, "bars"), "the rename reaches the components");

    // A minimum at or above the maximum is refused and changes nothing.
    let before = glyph(&app, "_part.bar");
    app.state.status.clear();
    w.commit(&mut app, field_id("min", 0), "700");
    assert_eq!(glyph(&app, "_part.bar"), before);
    assert!(!app.state.status.is_empty(), "the refusal is shown");
}

#[test]
fn smart_glyph_settings_layers_tab_ticks_set_the_layer_poles() {
    let mut app = smart_app();
    let r = app.run("smartGlyph.showSettings", &json!({"glyph": "_part.bar", "tab": "layers", "layer": "wide"})).unwrap();
    assert_eq!((r["tab"].clone(), r["layer"].clone()), (json!("layers"), json!("wide")));
    assert_eq!(app.state.panels.smart_glyph.tab, SettingsTab::Layers);
    // The Layers tab draws.
    let w = Window::new();
    w.settle(&mut app);

    // `Wide` draws the maximum: ticking its Maximum box again clears it, ticking Minimum moves it.
    let (id, params) = pole_command("_part.bar", "wide", "Width", Some(true), true);
    assert_eq!((id, params["pole"].clone()), ("smartGlyph.setLayerPole", json!("none")));
    let (id, params) = pole_command("_part.bar", "wide", "Width", Some(true), false);
    app.run(id, &params).unwrap();
    let mut headless = smart_app();
    headless.run("smartGlyph.setLayerPole", &json!({"glyph": "_part.bar", "layer": "wide", "property": "Width", "pole": "min"})).unwrap();
    assert_eq!(glyph(&app, "_part.bar"), glyph(&headless, "_part.bar"));

    assert!(app.run("smartGlyph.showSettings", &json!({"glyph": "_part.bar", "layer": "nope"})).is_err());
    assert!(app.run("smartGlyph.showSettings", &json!({"glyph": "_part.bar", "tab": "axes"})).is_err());
    assert_eq!(app.state.panels.smart_glyph.layer.as_deref(), Some("wide"), "a refused request changes nothing");
    app.run("smartGlyph.showSettings", &json!({"open": false})).unwrap();
    assert!(!app.state.panels.smart_glyph.open);
}

#[test]
fn a_zero_to_one_property_is_a_toggle_and_new_names_are_free() {
    let mut app = smart_app();
    app.run("smartGlyph.addProperty", &json!({"glyph": "_part.bar", "name": "Property", "min": 0, "max": 1})).unwrap();
    let r = app.run("smartComponent.showSettings", &json!({})).unwrap();
    let toggle = r["properties"].as_array().unwrap().iter().find(|p| p["name"] == "Property").unwrap();
    assert_eq!(toggle["toggle"], json!(true));
    assert_eq!(new_property_name(&glyph(&app, "_part.bar")), "Property 2");
}
