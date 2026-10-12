//! Font View's component and anchor batch forms (`fontView.batch.addComponent`,
//! `createComposite`, `decomposeComponents`, `setAnchors`, `resetAnchors`) and the editable
//! Components cell (`fontView.column.components`).

use egui::{Event, Key, Modifiers, Pos2, Rect};
use fontcraft_model::{Layer, Shape};
use serde_json::{Value, json};

use crate::FontcraftApp;
use crate::shell::font_view::InlineEdit;
use crate::shell::font_view::columns::Column;
use crate::tools_glue;

/// The default document with `acutecomb`, and `Aacute` and `eacute` built from it.
fn app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("glyph.addGlyphs", &json!({"text": "acutecomb Aacute eacute"})).unwrap();
    app
}

fn layers(app: &FontcraftApp, glyph: &str) -> Vec<Layer> {
    app.session.font().unwrap().glyph(glyph).unwrap().layers.clone()
}

fn components(app: &FontcraftApp, glyph: &str) -> Vec<String> {
    layers(app, glyph)[0].shapes.iter().filter_map(|s| if let Shape::Component(c) = s { Some(c.glyph.clone()) } else { None }).collect()
}

fn undo_steps(app: &FontcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

#[test]
fn font_view_batch_forms_equal_their_commands_on_the_selection() {
    assert_eq!(components(&app(), "Aacute"), ["A", "acutecomb"], "Add Glyphs builds the composite");
    let cases: [(&str, &[&str]); 4] = [
        ("glyph.createComposite", &["Aacute", "eacute"]),
        ("glyph.decomposeGlyph", &["Aacute", "eacute"]),
        ("glyph.setAnchors", &["a", "e", "acutecomb"]),
        ("glyph.resetAnchors", &["a", "e"]),
    ];
    for (id, selection) in cases {
        let setup = || {
            let mut app = app();
            // Something for Create Composite to restore.
            app.run("glyph.setComponents", &json!({"glyph": "Aacute", "components": ["A"]})).unwrap();
            app.run("glyph.setComponents", &json!({"glyph": "eacute", "components": []})).unwrap();
            if id == "glyph.decomposeGlyph" {
                app.run("glyph.createComposite", &json!({"glyphs": ["Aacute", "eacute"]})).unwrap();
            }
            app
        };
        let mut batch = setup();
        let before: Vec<Vec<Layer>> = selection.iter().map(|g| layers(&batch, g)).collect();
        batch.run("fontView.setSelection", &json!({"glyphs": selection})).unwrap();
        assert_eq!(tools_glue::with_targets(&batch, id, Value::Null), json!({"glyphs": selection, "layer": "m01"}), "{id}");
        let steps = undo_steps(&batch);
        batch.run_menu_command(id, Value::Null).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(undo_steps(&batch), steps + 1, "{id}: one undo step for the selection");

        let mut direct = setup();
        direct.run(id, &json!({"glyphs": selection, "layer": "m01"})).unwrap();
        let after: Vec<Vec<Layer>> = selection.iter().map(|g| layers(&batch, g)).collect();
        assert_ne!(after, before, "{id} changed the selected glyphs");
        for g in selection {
            assert_eq!(layers(&batch, g), layers(&direct, g), "{id} on {g}");
        }
    }
}

#[test]
fn add_component_in_font_view_adds_to_every_selected_glyph() {
    let mut batch = app();
    batch.run("fontView.setSelection", &json!({"glyphs": ["a", "o"]})).unwrap();
    let steps = undo_steps(&batch);
    batch.run_menu_command("glyph.addComponent", json!({"component": "acutecomb"})).unwrap();
    assert_eq!(undo_steps(&batch), steps + 1, "one undo step for the selection");
    let mut direct = app();
    for g in ["a", "o"] {
        direct.run("glyph.addComponent", &json!({"glyph": g, "layer": "m01", "component": "acutecomb"})).unwrap();
        assert_eq!(components(&batch, g), ["acutecomb"]);
        assert_eq!(layers(&batch, g), layers(&direct, g), "{g}");
    }
    // The all-masters form addresses no layer; a refusal on one glyph changes none.
    let r = batch.run_menu_command("glyph.addComponentAllMasters", json!({"component": "a"}));
    assert!(r.is_err(), "a cannot contain itself");
    assert_eq!(components(&batch, "o"), ["acutecomb"]);
    // A command given its own glyph keeps it.
    batch.run_menu_command("glyph.addComponent", json!({"glyph": "b", "component": "acutecomb"})).unwrap();
    assert_eq!(components(&batch, "b"), ["acutecomb"]);
    assert_eq!(components(&batch, "a"), ["acutecomb"]);
}

#[test]
fn the_components_cell_edits_the_shown_masters_components() {
    let mut app = app();
    app.run("fontView.setViewMode", &json!({"mode": "list"})).unwrap();
    app.run("fontView.setColumns", &json!({"visible": ["name", "components"]})).unwrap();
    assert_eq!(Column::Components.editing(), crate::shell::font_view::columns::Editing::Text);
    let ctx = egui::Context::default();
    let frame = |app: &mut FontcraftApp, events: Vec<Event>| {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let _ = ctx.run_ui(input, |ui| app.ui(ui));
    };
    frame(&mut app, Vec::new());
    app.font_view.edit = Some(InlineEdit { glyph: "b".into(), column: Column::Components, text: String::new(), fresh: true });
    frame(&mut app, Vec::new());
    frame(&mut app, vec![Event::Text("o, acutecomb acutecomb".into())]);
    let enter = |pressed| Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    frame(&mut app, vec![enter(true), enter(false)]);
    frame(&mut app, Vec::new());
    assert_eq!(components(&app, "b"), ["o", "acutecomb", "acutecomb"], "{}", app.state.status);
    // The command the cell runs names Font View's master.
    let row = app.font_view.cache.row_of("b").unwrap();
    let (id, params) = Column::Components.edit_command_in(Some("m01"), row, "o,acutecomb").unwrap();
    assert_eq!((id, params), ("glyph.setComponents", json!({"glyph": "b", "components": ["o", "acutecomb"], "layer": "m01"})));
}
