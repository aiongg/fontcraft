//! Floating panels in OS windows of their own: under a form dialog of the main window, and driven through the control
//! channel.

use egui::{CursorIcon, Event, PointerButton, Pos2, ViewportCommand, ViewportId, WindowLevel};
use serde_json::json;

use super::NativeApp;
use crate::shell::font_info::FontInfoTab;

/// Font Info, floated into an OS window of its own.
fn float_font_info(t: &mut NativeApp) -> ViewportId {
    t.ok("engine.execute", json!({"command": "font.info", "params": {}}));
    let windows = t.windows();
    assert_eq!(windows.len(), 1, "Font Info opens in an OS window of its own");
    windows[0]
}

/// The centre of the control named `name` in the OS window `id`, in the window's points.
fn control(t: &NativeApp, id: ViewportId, name: &str) -> Pos2 {
    t.window(id, |window| {
        let tree = window.tree.as_ref().expect("the window ran a pass");
        let key = tree.keys().into_iter().find(|key| tree.name_of(*key) == Some(name));
        key.and_then(|key| tree.bounds_of(key)).unwrap_or_else(|| panic!("no control {name} in the window")).center()
    })
}

/// A click at `at` in the OS window `id`.
fn click(t: &mut NativeApp, id: ViewportId, at: Pos2) {
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    t.window_frame(id, vec![Event::PointerMoved(at)]);
    t.window_frame(id, vec![button(true)]);
    t.window_frame(id, vec![button(false)]);
    t.frames(2);
}

fn main_focused(commands: &[(ViewportId, ViewportCommand)]) -> bool {
    commands.iter().any(|(id, command)| *id == ViewportId::ROOT && matches!(command, ViewportCommand::Focus))
}

/// A floated panel is a palette window, above the main one while the app is active, and takes clicks.
#[test]
fn a_floated_panel_is_a_palette_window_that_takes_clicks() {
    let mut t = NativeApp::new();
    let font_info = float_font_info(&mut t);
    assert_eq!(t.window(font_info, |w| w.level), Some(WindowLevel::AlwaysOnTop));
    let notes = control(&t, font_info, "Notes");
    click(&mut t, font_info, notes);
    assert_eq!(t.app.view.font_info.tab, FontInfoTab::Notes, "the click reaches the floating window");
}

/// While Add Glyphs is open in the main window, the floating Font Info drops to the normal level, the main window
/// comes to the front, and Font Info takes no clicks and shows the blocked cursor; after the dialog it is a palette
/// window again.
#[test]
fn a_form_dialog_lowers_and_blocks_a_floated_panel() {
    let mut t = NativeApp::new();
    let font_info = float_font_info(&mut t);
    let notes = control(&t, font_info, "Notes");
    t.take_commands();

    t.ok("ui.menu.invoke", json!({"command": "glyph.addGlyphs"}));
    assert_eq!(t.ok("ui.dialog.inspect", json!({}))["open"], true);
    assert_eq!(t.window(font_info, |w| w.level), Some(WindowLevel::Normal), "the palette goes down under the dialog");
    assert!(main_focused(&t.take_commands()), "the main window, with the dialog, comes to the front");
    t.frames(3);
    assert!(!main_focused(&t.take_commands()), "it is raised once, not every frame");

    click(&mut t, font_info, notes);
    assert_eq!(t.app.view.font_info.tab, FontInfoTab::Font, "the floating window takes no click while the dialog is open");
    assert_eq!(t.window(font_info, |w| w.cursor), CursorIcon::NotAllowed);
    assert!(main_focused(&t.take_commands()), "a press in the blocked window raises the dialog's window");

    t.ok("ui.dialog.cancel", json!({}));
    t.frames(2);
    assert_eq!(t.window(font_info, |w| w.level), Some(WindowLevel::AlwaysOnTop), "the palette level comes back");
    click(&mut t, font_info, notes);
    assert_eq!(t.app.view.font_info.tab, FontInfoTab::Notes, "the floating window takes clicks again");
}

/// The centre of the control named `name` in the OS window `id`, in main-window points: where `ui.click` finds it.
fn on_desktop(t: &NativeApp, id: ViewportId, name: &str) -> Pos2 {
    control(t, id, name) + t.place(id).min.to_vec2()
}

/// A `ui.click` at a point over a floated panel lands in its OS window: it switches Font Info's tab.
#[test]
fn a_control_click_over_a_floated_panel_reaches_its_window() {
    let mut t = NativeApp::new();
    let font_info = float_font_info(&mut t);
    let notes = on_desktop(&t, font_info, "Notes");
    let reply = t.ok("ui.click", json!({"x": notes.x, "y": notes.y}));
    assert_ne!(reply["window"], "main", "the click lands in Font Info's window: {reply}");
    t.frames(3);
    assert_eq!(t.app.view.font_info.tab, FontInfoTab::Notes, "the click reaches the floating window");
    let inspected = t.ok("ui.inspect", json!({}));
    let windows = inspected["windows"].as_array().expect("ui.inspect lists the windows");
    assert_eq!(windows.len(), 2, "the main window and Font Info: {windows:?}");
    assert_eq!(windows[0]["window"], "main");
}

/// After a `ui.click` on a field of a floated panel, its window has the keyboard, so `ui.text` and `ui.key` reach the
/// field: the typed note is committed when Tab moves the keyboard on.
#[test]
fn control_text_types_into_a_field_of_a_floated_panel() {
    let mut t = NativeApp::new();
    let font_info = float_font_info(&mut t);
    let notes = on_desktop(&t, font_info, "Notes");
    t.ok("ui.click", json!({"x": notes.x, "y": notes.y}));
    t.frames(3);
    let field = on_desktop(&t, font_info, "Font Note");
    t.ok("ui.click", json!({"x": field.x, "y": field.y}));
    t.frames(3);
    assert_eq!(t.focused(), font_info, "the click gives Font Info's window the keyboard");
    assert_eq!(t.ok("ui.text", json!({"text": "Draft"}))["field"], true, "a field of the floating window has the keyboard");
    t.frames(2);
    assert_eq!(t.ok("ui.key", json!({"key": "Tab"}))["field"], true);
    t.frames(3);
    assert_eq!(t.app.session.font().unwrap().note, "Draft", "the text was typed into the floating window's field");
}

/// The element list covers a floated panel's OS window, and an element there is clicked and set by its id.
#[test]
fn the_elements_of_a_floated_panel_are_listed_clicked_and_set() {
    let mut t = NativeApp::new();
    let font_info = float_font_info(&mut t);
    let element = |t: &mut NativeApp, prefix: &str, role: Option<&str>| {
        let list = t.ok("ui.elements", json!({"prefix": prefix, "role": role}));
        let all = list["elements"].as_array().cloned().unwrap_or_default();
        let floated = all.iter().find(|e| e["window"] != "main" && e["visible"] == true).cloned();
        floated.unwrap_or_else(|| panic!("no {prefix} in the floating window: {list}"))
    };
    let notes = element(&mut t, "Notes", None);
    let left = t.place(font_info).min.x;
    assert!(notes["rect"][0].as_f64().is_some_and(|x| x >= f64::from(left)), "rectangles are in main-window points: {notes}");
    let reply = t.ok("ui.click", json!({"element": notes["id"]}));
    assert_eq!(reply["window"], notes["window"], "the click goes to the element's window");
    t.frames(3);
    assert_eq!(t.app.view.font_info.tab, FontInfoTab::Notes);

    let note = element(&mut t, "Font Note", Some("multilineTextInput"));
    t.ok("ui.set", json!({"element": note["id"], "value": "First line\nSecond line"}));
    t.frames(12);
    assert_eq!(t.app.session.font().unwrap().note, "First line\nSecond line");
}
