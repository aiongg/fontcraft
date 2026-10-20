//! Check marks on the menus' on/off switches, and the menu cache that carries them.

use serde_json::{Value, json};

use crate::shell::UI_COMMANDS;
use crate::shell::menus::{SWITCHES, menu_tree_for};
use crate::{FontcraftApp, control};

fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    control::handle(app, &json!({"id": 1, "method": method, "params": params}))
}

/// Runs `frames` frames of the app at 1280 × 800.
fn frames(app: &mut FontcraftApp, frames: usize) {
    let ctx = egui::Context::default();
    let input = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    for _ in 0..frames {
        let mut output = ctx.run_ui(input(), |ui| app.ui(ui));
        output.textures_delta.clear();
    }
}

/// The check mark `ui.menu.list` reports for `id`.
fn listed(app: &mut FontcraftApp, id: &str) -> Value {
    let tree = call(app, "ui.menu.list", json!({}));
    let menus = tree["result"].as_array().unwrap();
    let item = menus.iter().flat_map(|m| m["items"].as_array().unwrap()).find(|i| i["command"] == id);
    item.unwrap_or_else(|| panic!("{id} is in no menu"))["checked"].clone()
}

/// The check mark the window's cached menu carries for `id`.
fn drawn(app: &mut FontcraftApp, id: &str) -> Option<bool> {
    let tree = app.menu_cache.tree(&app.session, &app.state);
    tree.iter().flat_map(|m| m.items.iter()).find(|i| i.command == id).and_then(|i| i.checked)
}

#[test]
fn every_switch_in_the_menus_carries_a_check_mark() {
    let app = FontcraftApp::default();
    let tree = menu_tree_for(&app.session, &app.state);
    let items: Vec<_> = tree.iter().flat_map(|m| m.items.iter()).collect();
    for id in SWITCHES {
        let item = items.iter().find(|i| i.command == *id).unwrap_or_else(|| panic!("{id} is in no menu"));
        assert!(item.checked.is_some(), "{id} shows no check mark");
    }
    for item in items.iter().filter(|i| !SWITCHES.contains(&i.command)) {
        assert_eq!(item.checked, None, "{} is not a switch", item.command);
    }
    // A shell command whose menu item toggles must be listed as a switch.
    for (id, _, menu, _, params) in UI_COMMANDS.iter() {
        if !menu.is_empty() && params.contains("(omitted: toggle)") {
            assert!(SWITCHES.contains(id), "{id} toggles but is not in SWITCHES");
        }
    }
}

#[test]
fn toggling_a_switch_flips_its_check_mark() {
    let mut app = FontcraftApp::default();
    for id in [
        "view.showTunniLines",
        "view.lockOutline",
        "view.showMetrics",
        "view.showIndexes",
        "view.showStemWidths",
        "edit.keepLayerSelectionInSync",
        "path.togglePowerNudge",
        "window.palette",
    ] {
        let before = listed(&mut app, id);
        let on = before.as_bool().unwrap_or_else(|| panic!("{id}: {before}"));
        assert_eq!(drawn(&mut app, id), Some(on), "{id}: the window's menu agrees with ui.menu.list");
        app.run(id, &json!({})).unwrap();
        assert_eq!(listed(&mut app, id), json!(!on), "{id} flips in ui.menu.list");
        assert_eq!(drawn(&mut app, id), Some(!on), "{id} flips in the cached menu");
        app.run(id, &json!({})).unwrap();
        assert_eq!(listed(&mut app, id), json!(on), "{id} flips back");
    }
}

#[test]
fn the_menu_cache_rebuilds_only_when_something_changed() {
    let mut app = FontcraftApp::default();
    frames(&mut app, 2);
    let built = app.menu_cache.builds();
    assert!(built >= 1, "the menu bar was drawn");
    frames(&mut app, 3);
    assert_eq!(app.menu_cache.builds(), built, "frames with no change reuse the tree");
    app.run("view.showGuides", &json!({})).unwrap();
    frames(&mut app, 2);
    assert_eq!(app.menu_cache.builds(), built + 1, "a switch rebuilds the tree once");
    let guides = app.state.display.guides;
    assert_eq!(drawn(&mut app, "view.showGuides"), Some(guides));
}
