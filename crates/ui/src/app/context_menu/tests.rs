//! The context menus as the control channel and the shell drive them: what a click selects, which items show and how
//! they run, and the glyph menu's items (M4.19: Exports, More Tags…).

use std::sync::Arc;

use craft::behavior::CanvasView;
use craft::behavior::kurbo::Point as KPoint;
use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use egui::{Pos2, Rect, vec2};
use fontcraft_engine::Session;
use fontcraft_engine::model::{Font, Layer};
use serde_json::{Value, json};

use super::glyph_menu::{self, GlyphAction, MAX_MENU_TAGS, Mark};
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::control;

fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
    control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
}

/// Glyph A with a closed 200-unit square (shape 0, node 2 at (300, 300)), shown at one point per unit, nothing
/// selected.
fn edit_app() -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    app.session.execute("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.view.canvas = CanvasView::new(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0)), KPoint::new(400.0, 400.0), 1.0);
    app.drain();
    app
}

fn layer(app: &FontCraftApp) -> Layer {
    let font = app.session.font().unwrap();
    font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap().clone()
}

fn commands(menu: &Value, section: usize) -> Vec<String> {
    menu["sections"][section]["items"].as_array().unwrap().iter().map(|i| i["command"].as_str().unwrap().to_string()).collect()
}

#[test]
fn a_click_on_a_node_lists_the_node_commands_and_runs_them_like_the_command() {
    let mut app = edit_app();
    assert_eq!(call(&mut app, "ui.contextMenu.list", json!({})), json!({"open": false}));
    let menu = call(&mut app, "ui.contextMenu.list", json!({"x": 300, "y": 300}));
    assert_eq!(serde_json::to_value(&app.tools.selection).unwrap(), json!({"points": [[0, 2]]}), "the clicked node is selected first");
    let contexts: Vec<&str> = menu["sections"].as_array().unwrap().iter().map(|s| s["context"].as_str().unwrap()).collect();
    assert_eq!(contexts, ["canvas.node", "canvas"], "{menu}");
    let node = commands(&menu, 0);
    for id in ["path.toggleNodeType", "path.openCorner", "path.makeNodeFirst", "path.lockPath", "guide.add"] {
        assert!(node.contains(&id.to_string()), "{id} is in the node section: {node:?}");
    }
    assert!(!node.contains(&"path.convertSegmentToCurve".to_string()), "segment commands stay out: {node:?}");
    assert!(!node.contains(&"path.unlockPath".to_string()), "an unlocked path offers Lock Path only");
    let position = |id: &str| node.iter().position(|c| c == id).unwrap();
    assert!(position("path.toggleNodeType") < position("path.openCorner"), "rows' order: {node:?}");
    let glyph = commands(&menu, 1);
    assert!(glyph.contains(&"glyph.toggleLocked".to_string()) && glyph.contains(&super::SHOW_IN_FONT_VIEW.to_string()), "{glyph:?}");
    let connect = menu["sections"][0]["items"].as_array().unwrap().iter().find(|i| i["command"] == "path.connectNodes").unwrap();
    assert_eq!(connect["enabled"], false, "one node of a closed path cannot be connected: {connect}");

    let refused = control::dispatch(&mut app, "ui.contextMenu.invoke", &json!({"command": "path.connectNodes"}));
    assert!(refused.is_err());
    assert!(app.context_menus.canvas.is_some(), "a refused item leaves the menu open");
    call(&mut app, "ui.contextMenu.invoke", json!({"command": "path.openCorner"}));
    assert!(app.context_menus.canvas.is_none(), "running an item closes the menu");

    let mut direct = edit_app();
    direct.session.execute("path.openCorner", &json!({"glyph": "A", "points": [[0, 2]]})).unwrap();
    assert_eq!(layer(&app), layer(&direct), "the item gives the same font as the command");
    assert_ne!(layer(&app), layer(&edit_app()), "the corner was opened");
}

#[test]
fn a_click_on_empty_canvas_keeps_the_selection_and_the_window_path_runs_items() {
    let mut app = edit_app();
    app.tools.selection = fontcraft_tools::Selection::from_items([fontcraft_tools::Item::Point(0, 0)]);
    let menu = call(&mut app, "ui.contextMenu.list", json!({"x": 700, "y": 700}));
    assert_eq!(serde_json::to_value(&app.tools.selection).unwrap(), json!({"points": [[0, 0]]}), "the selection stays");
    assert_eq!(menu["sections"][0]["context"], "canvas.node", "the menu follows the selection: {menu}");

    // The window posts a secondary click as `contextMenu.open` and a pick as `contextMenu.invoke`.
    let mut app = edit_app();
    let screen = super::apply::camera(&app).to_screen(KPoint::new(300.0, 300.0));
    app.sent.push(Msg::Shell(CommandRef::with_params("contextMenu.open", json!({"x": screen.x, "y": screen.y}))));
    app.drain();
    assert!(app.context_menus.canvas.as_ref().is_some_and(|m| m.item("path.openCorner").is_some()), "{:?}", app.context_menus.canvas);
    app.sent.push(Msg::Shell(CommandRef::with_params("contextMenu.invoke", json!({"command": "path.openCorner"}))));
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
    assert_ne!(layer(&app), layer(&edit_app()), "the corner was opened from the window path");
}

fn font(app: &FontCraftApp) -> Arc<Font> {
    app.session.doc().unwrap().font.clone()
}

/// Glyphs A, B and C; B does not export.
fn glyphs_app() -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    for name in ["A", "B", "C"] {
        app.session.execute("glyph.newGlyph", &json!({ "name": name })).ok();
    }
    app.session.execute("glyph.setExport", &json!({"glyphs": ["B"], "export": false})).unwrap();
    app.session.execute("glyph.addTag", &json!({"glyphs": ["A"], "tag": "round"})).unwrap();
    app.drain();
    app
}

#[test]
fn the_glyph_menu_marks_exports_and_tags_and_its_items_run_their_commands() {
    let mut app = glyphs_app();
    let names: Vec<String> = ["A", "B", "C"].iter().map(|s| s.to_string()).collect();
    app.sent.push(Msg::Shell(CommandRef::with_params("fontView.openGlyphMenu", json!({"glyphs": names, "x": 10, "y": 20}))));
    app.drain();
    let menu = app.context_menus.glyph.clone().expect("the glyph menu is open");
    assert_eq!(menu.exports, Mark::Some, "B does not export");
    assert_eq!(menu.tags, vec![("round".to_string(), Mark::Some)]);
    assert!(!menu.more_tags);

    // Exports with mixed glyphs makes them all export, as the command does.
    for msg in glyph_menu::steps(&menu, &GlyphAction::Export) {
        app.run_now(msg, false).unwrap();
    }
    let mut direct = glyphs_app();
    direct.session.execute("glyph.setExport", &json!({"glyphs": ["A", "B", "C"], "export": true})).unwrap();
    assert!(font(&app).same_as(&font(&direct)));
    assert_eq!(glyph_menu::open(&app, &names, [0.0, 0.0]).exports, Mark::All);

    // Copy Glyph Names puts the command's text on the clipboard.
    for msg in glyph_menu::steps(&menu, &GlyphAction::CopyNames("spaceSeparated")) {
        app.run_now(msg, false).unwrap();
    }
    let text = direct.session.execute("glyph.copyGlyphNames", &json!({"glyphs": names, "format": "spaceSeparated"})).unwrap()["text"].clone();
    assert_eq!(app.clipboard.out.as_deref(), text.as_str());

    // Past MAX_MENU_TAGS the submenu ends with More Tags…, which shows the sidebar.
    let mut many = glyphs_app();
    for i in 0..=MAX_MENU_TAGS {
        // A glyph holds at most 256 tags, so they are spread over the three.
        let glyph = ["A", "B", "C"][i % 3];
        many.session.execute("glyph.addTag", &json!({"glyphs": [glyph], "tag": format!("t{i:04}")})).unwrap();
    }
    let menu = glyph_menu::open(&many, &names, [0.0, 0.0]);
    assert_eq!(menu.tags.len(), MAX_MENU_TAGS);
    assert!(menu.more_tags);
    for msg in glyph_menu::steps(&menu, &GlyphAction::MoreTags) {
        many.run_now(msg, false).unwrap();
    }
    assert_ne!(many.view.workspace.layout.panel_state(&crate::shell::dock::PanelId::Sidebar), craft::behavior::dock::PanelState::Hidden);
}

/// A glyph menu opened with Alt held colours the layer each glyph shows in the current master; without Alt the glyph.
#[test]
fn the_glyph_menu_opened_with_alt_colours_the_layer() {
    use crate::panels::context_menu::Pick;
    use crate::shell::context_menu::glyph_action;
    use fontcraft_engine::model::ColorLabel;

    let mut app = glyphs_app();
    app.sent.push(Msg::Shell(CommandRef::with_params("fontView.openGlyphMenu", json!({"glyphs": ["A"], "alt": true}))));
    app.drain();
    let menu = app.context_menus.glyph.clone().expect("the glyph menu is open");
    assert!(menu.alt);
    let action = glyph_action(&menu, Pick::Color(Some(3))).expect("a swatch acts");
    assert_eq!(action, GlyphAction::LayerColor(Some(3)));
    for msg in glyph_menu::steps(&menu, &action) {
        app.run_now(msg, false).unwrap();
    }
    let font = font(&app);
    let glyph = font.glyph("A").unwrap();
    assert_eq!(glyph.layer(&font.masters[0].id).unwrap().info.color, Some(ColorLabel::Index(3)));
    assert_eq!(glyph.info.color, None, "the glyph keeps no colour");

    app.sent.push(Msg::Shell(CommandRef::with_params("fontView.openGlyphMenu", json!({"glyphs": ["A"]}))));
    app.drain();
    let plain = app.context_menus.glyph.clone().expect("the glyph menu is open");
    assert_eq!(glyph_action(&plain, Pick::Color(Some(3))), Some(GlyphAction::Color(Some(3))));
}

/// While the TrueType Instructor is the active tool the menu ends with its view items, checked as their state is, and
/// each runs its command; other tools' menus leave them out.
#[test]
fn the_instructor_section_lists_its_view_items_checked_as_their_state_is() {
    use super::TT_INSTRUCTOR;
    let mut app = edit_app();
    let menu = call(&mut app, "ui.contextMenu.list", json!({"x": 700, "y": 700}));
    assert!(!menu["sections"].as_array().unwrap().iter().any(|s| s["context"] == TT_INSTRUCTOR), "{menu}");

    app.sent.push(Msg::View(ViewMsg::SelectTool(crate::canvas::hinting::INSTRUCTOR)));
    app.drain();
    let list = |app: &mut FontCraftApp| call(app, "ui.contextMenu.list", json!({"x": 700, "y": 700}));
    let menu = list(&mut app);
    let last = menu["sections"].as_array().unwrap().last().cloned().unwrap();
    assert_eq!(last["context"], TT_INSTRUCTOR, "{menu}");
    let ids: Vec<&str> = last["items"].as_array().unwrap().iter().map(|i| i["command"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        [
            "hint.tt.showPixelPreview",
            "contextMenu.hintingPreviewScale1x",
            "contextMenu.hintingPreviewScale2x",
            "contextMenu.hintingPreviewScale4x",
            "hint.tt.showPreviewAddress",
            "hint.tt.showPointIndexes"
        ]
    );
    let checked = |menu: &Value, id: &str| {
        menu["sections"].as_array().unwrap().iter().flat_map(|s| s["items"].as_array().unwrap()).find(|i| i["command"] == id).unwrap()["checked"]
            .clone()
    };
    assert_eq!(checked(&menu, "hint.tt.showPixelPreview"), json!(app.view.hinting.instructor.pixel_preview));
    assert_eq!(checked(&menu, "hint.tt.showPointIndexes"), json!(false));
    assert_eq!(checked(&menu, "contextMenu.hintingPreviewScale1x"), json!(true));
    assert_eq!(checked(&menu, "hint.tt.showPreviewAddress"), Value::Null, "not a toggle");

    call(&mut app, "ui.contextMenu.invoke", json!({"command": "hint.tt.showPointIndexes"}));
    assert!(app.view.hinting.instructor.point_indexes);
    list(&mut app);
    call(&mut app, "ui.contextMenu.invoke", json!({"command": "contextMenu.hintingPreviewScale4x"}));
    assert_eq!(app.models.hinting_waterfall.scale(), 4);
    let menu = list(&mut app);
    assert_eq!(checked(&menu, "hint.tt.showPointIndexes"), json!(true));
    assert_eq!(checked(&menu, "contextMenu.hintingPreviewScale4x"), json!(true));
    assert_eq!(checked(&menu, "contextMenu.hintingPreviewScale1x"), json!(false));
}
