//! Headless UI tests: frames are run in a bare egui context, with no window.

use fontcraft_engine::command_specs;
use serde_json::{Value, json};

use crate::control;
use crate::menus::{MENU_ORDER, UI_COMMANDS, menu_tree};
use crate::{FontcraftApp, theme::Tokens};

/// Runs `frames` frames of the app at 1280 × 800 and returns how many shapes the last one drew.
fn frames(app: &mut FontcraftApp, frames: usize) -> usize {
    let ctx = egui::Context::default();
    let input = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut shapes = 0;
    for _ in 0..frames {
        let mut output = ctx.run_ui(input(), |ui| app.ui(ui));
        // No renderer takes the font atlas here; say so, or epaint flags the dropped texture update.
        output.textures_delta.clear();
        shapes = output.shapes.len();
    }
    shapes
}

fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    control::handle(app, &json!({"id": 1, "method": method, "params": params}))
}

#[test]
fn font_view_and_edit_view_draw_without_a_window() {
    let mut app = FontcraftApp::default();
    let grid = frames(&mut app, 2);
    assert!(grid > 50, "Font View drew only {grid} shapes");
    app.run("path.addPath", &json!({"glyph": "A", "nodes": [{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 310, "y": 700}]})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyph": "A"})).unwrap();
    let edit = frames(&mut app, 2);
    assert!(edit > 5, "Edit View drew only {edit} shapes");
    // With no document the app still draws.
    app.run("file.close", &json!({"discard": true})).unwrap();
    assert!(app.state.editing.is_none(), "closing the document leaves Edit View");
    frames(&mut app, 1);
}

#[test]
fn every_menu_item_is_a_command_and_the_bar_is_in_order() {
    let app = FontcraftApp::default();
    let tree = menu_tree(&app.session);
    let titles: Vec<&str> = tree.iter().map(|m| m.title).collect();
    let mut expected: Vec<&str> = MENU_ORDER.iter().copied().filter(|t| titles.contains(t)).collect();
    expected.extend(titles.iter().filter(|t| !MENU_ORDER.contains(t)));
    assert_eq!(titles, expected);
    let ids: Vec<&str> = command_specs().iter().map(|c| c.id).chain(UI_COMMANDS.iter().map(|c| c.0)).collect();
    for item in tree.iter().flat_map(|m| &m.items) {
        assert!(ids.contains(&item.command), "{} is in a menu but is not a command", item.command);
    }
    let undo = tree.iter().flat_map(|m| &m.items).find(|i| i.command == "edit.undo").unwrap();
    assert!(!undo.enabled && undo.disabled_reason.as_deref() == Some("nothing to undo"));
}

#[test]
fn ui_and_engine_command_ids_do_not_clash() {
    let mut seen = std::collections::HashSet::new();
    for id in command_specs().iter().map(|c| c.id).chain(UI_COMMANDS.iter().map(|c| c.0)) {
        assert!(seen.insert(id), "{id} is registered twice");
    }
    let shortcuts: Vec<&str> =
        command_specs().iter().filter_map(|c| c.shortcut).chain(UI_COMMANDS.iter().map(|c| c.3).filter(|s| !s.is_empty())).collect();
    let unique: std::collections::HashSet<&&str> = shortcuts.iter().collect();
    assert_eq!(shortcuts.len(), unique.len(), "two commands share a default shortcut");
}

#[test]
fn the_control_protocol_drives_the_app() {
    let mut app = FontcraftApp::default();
    let r = call(&mut app, "engine.execute", json!({"command": "glyph.newGlyph", "params": {"name": "agent"}}));
    assert_eq!((r["ok"].as_bool(), r["result"]["name"].as_str()), (Some(true), Some("agent")));
    assert_eq!(call(&mut app, "engine.execute", json!({"command": "fontView.setSelection", "params": {"glyphs": ["agent"]}}))["ok"], true);
    assert_eq!(call(&mut app, "ui.inspect", json!({}))["result"]["selected_glyphs"], json!(["agent"]));
    assert_eq!(call(&mut app, "document.inspect", json!({}))["result"]["documents"][0]["undo"], json!(["New Glyph"]));
    let commands = call(&mut app, "engine.commands", json!({}));
    let list = commands["result"]["commands"].as_array().unwrap();
    assert_eq!(list.len(), command_specs().len() + UI_COMMANDS.len());
    assert!(call(&mut app, "ui.menu.list", json!({}))["result"].as_array().unwrap().len() >= 4);
    assert_eq!(call(&mut app, "ui.menu.invoke", json!({"command": "edit.undo"}))["ok"], true);
    assert_eq!(call(&mut app, "ui.menu.invoke", json!({"command": "font.info.get"}))["ok"], false);
    // Removing the selected glyph clears the selection; errors come back as replies.
    assert_eq!(call(&mut app, "ui.inspect", json!({}))["result"]["selected_glyphs"], json!([]));
    for bad in [
        json!({"method": "nope"}),
        json!({"id": 2}),
        json!({"method": 5}),
        json!({"method": "engine.execute"}),
        json!({"method": "engine.execute", "params": {"command": "no.such"}}),
        json!([]),
        json!("x"),
    ] {
        let reply = control::handle(&mut app, &bad);
        assert_eq!(reply["ok"], false, "{bad}");
        assert!(reply["error"].as_str().is_some_and(|e| !e.is_empty()));
    }
}

#[test]
fn ui_commands_reject_junk_without_panicking() {
    let mut app = FontcraftApp::default();
    let junk = [
        Value::Null,
        json!({}),
        json!([]),
        json!("x"),
        json!({"glyphs": 5}),
        json!({"glyphs": [5]}),
        json!({"glyphs": ["nope"]}),
        json!({"glyph": []}),
        json!({"show": "yes"}),
        json!({"glyph": "nope"}),
    ];
    for (id, ..) in UI_COMMANDS {
        for params in &junk {
            let _ = app.run(id, params);
            assert!(crate::state::ZOOM_RANGE.contains(&app.state.zoom));
        }
    }
    for _ in 0..200 {
        app.run("view.zoomIn", &json!({})).unwrap();
    }
    assert_eq!(app.state.zoom, *crate::state::ZOOM_RANGE.end());
    frames(&mut app, 1);
}

#[test]
fn a_large_font_draws_only_the_rows_in_view() {
    let mut app = FontcraftApp::default();
    let small = frames(&mut app, 2);
    // Twenty thousand more glyphs: the frame must not grow with the font.
    app.session
        .edit("many", |font| {
            for i in 0..20_000 {
                font.add_glyph(&format!("g{i:05}"), vec![], 500.0)?;
            }
            Ok(())
        })
        .unwrap();
    let start = std::time::Instant::now();
    let large = frames(&mut app, 2);
    assert!(large < small * 4, "Font View drew {large} shapes for a large font, {small} for a small one");
    assert!(start.elapsed().as_secs() < 5);
    // Selecting every glyph is quick too.
    let all: Vec<String> = app.session.font().unwrap().glyphs.iter().map(|g| g.name.clone()).collect();
    let start = std::time::Instant::now();
    app.run("fontView.setSelection", &json!({"glyphs": all})).unwrap();
    assert!(start.elapsed().as_secs() < 2);
    assert_eq!(app.state.selected_glyphs.len(), 20_054);
}

#[test]
fn themes_differ_only_where_they_should() {
    let (dark, light) = (Tokens::dark(), Tokens::light());
    assert!(dark.dark && !light.dark);
    assert_ne!(dark.panel, light.panel);
    assert_eq!(dark.accent, light.accent);
}

#[test]
fn ui_render_draws_glyph_pixels_and_screenshots_need_a_window() {
    let mut app = FontcraftApp::default();
    app.run("path.addPath", &json!({"glyph": "A", "nodes": [{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 310, "y": 700}]})).unwrap();
    let render = |app: &mut FontcraftApp, params: Value| call(app, "ui.render", params);
    let first = render(&mut app, json!({"glyphs": ["A"], "size": 100}));
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!((first["result"]["width"].as_u64(), first["result"]["height"].as_u64()), (Some(76), Some(116)));
    assert_eq!(render(&mut app, json!({"glyphs": ["A"], "size": 100}))["result"]["png"], first["result"]["png"], "the same bytes twice");
    // Text maps characters to glyphs; with nothing named, the edited glyph is drawn.
    assert_eq!(render(&mut app, json!({"text": "AB", "scale": 0.1}))["result"]["glyphs"], json!(["A", "B"]));
    app.run("view.editSelectedGlyphs", &json!({"glyph": "A"})).unwrap();
    assert_eq!(render(&mut app, json!({}))["result"]["glyphs"], json!(["A"]));
    for bad in [
        json!({"glyphs": ["nope"]}),
        json!({"text": "\u{1F600}"}),
        json!({"size": -1}),
        json!({"scale": 1e9}),
        json!({"glyphs": 5}),
        json!({"path": 5}),
    ] {
        assert_eq!(render(&mut app, bad.clone())["ok"], false, "{bad}");
    }
    let shot = call(&mut app, "ui.screenshot", json!({}));
    assert!(shot["error"].as_str().is_some_and(|e| e.starts_with("ui.screenshot:")), "{shot}");
    match control::handle_request(&mut app, &json!({"id": 7, "method": "ui.screenshot", "params": {"path": "x.png"}})) {
        control::Handled::Screenshot(request) => {
            assert_eq!(request.path.as_deref(), Some("x.png"));
            let reply = control::ScreenshotRequest { path: None, ..request }.finish(2, 1, vec![255; 8], 1.0);
            assert_eq!((reply["id"].as_i64(), reply["result"]["width"].as_u64()), (Some(7), Some(2)));
        }
        other => panic!("ui.screenshot was answered without a window: {other:?}"),
    }
}
