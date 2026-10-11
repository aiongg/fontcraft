//! The Text tool's keys: typing, deleting, the cursor keys, Escape, Space, the clipboard as text,
//! and the spacing and kerning shortcuts in text mode.

use std::sync::Arc;

use egui::{Event, Key, Modifiers, OutputCommand};
use fontcraft_model::Layer;
use serde_json::{Value, json};

use crate::{FontcraftApp, control};

struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
        output
    }

    /// A key pressed and let go in one frame.
    fn key(&self, app: &mut FontcraftApp, key: Key, modifiers: Modifiers) {
        self.frame(app, vec![press(key, modifiers, true), press(key, modifiers, false)]);
    }
}

fn press(key: Key, modifiers: Modifiers, pressed: bool) -> Event {
    Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
}

fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    control::handle(app, &json!({"id": 1, "method": method, "params": params}))
}

fn get(app: &mut FontcraftApp) -> Value {
    app.run("editView.getText", &json!({})).unwrap()
}

fn items(app: &mut FontcraftApp) -> Value {
    get(app)["items"].clone()
}

fn cursor(app: &mut FontcraftApp) -> (Value, Value) {
    let t = get(app);
    (t["cursor"].clone(), t["selection"].clone())
}

/// A tab on `H` in text mode with the Text tool.
fn text_app(w: &Window) -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("view.newTab", &json!({"glyphs": ["H"]})).unwrap();
    app.run("tool.select", &json!({"tool": "text"})).unwrap();
    w.frame(&mut app, Vec::new());
    app
}

fn copied(output: &egui::FullOutput) -> Vec<String> {
    output
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn typing_deleting_and_the_cursor_keys_edit_the_tab_text() {
    let w = Window::new();
    let mut app = text_app(&w);
    assert_eq!((get(&mut app)["textMode"].clone(), app.tools.toolbox.active_id()), (json!(true), "text"));
    app.run("editView.setText", &json!({"chars": "HOH"})).unwrap();

    // Typing inserts characters as typed: a slash is the slash, not a `/name` escape. Letters
    // type instead of choosing tools.
    w.frame(&mut app, vec![press(Key::N, Modifiers::NONE, true), Event::Text("n/".into()), press(Key::N, Modifiers::NONE, false)]);
    let slash = if app.session.font().unwrap().glyph("slash").is_some() { json!("slash") } else { json!({"missing": "/"}) };
    assert_eq!(items(&mut app), json!(["H", "O", "H", "n", slash]));
    assert_eq!(app.tools.toolbox.active_id(), "text", "the N key typed and chose no tool");

    // Backspace and Delete at the edges.
    w.key(&mut app, Key::Backspace, Modifiers::NONE);
    assert_eq!((items(&mut app), cursor(&mut app).0), (json!(["H", "O", "H", "n"]), json!(4)));
    w.key(&mut app, Key::Delete, Modifiers::NONE);
    assert_eq!(items(&mut app), json!(["H", "O", "H", "n"]), "Delete at the end removes nothing");
    app.run("editView.setCursor", &json!({"index": 0})).unwrap();
    w.key(&mut app, Key::Backspace, Modifiers::NONE);
    assert_eq!(items(&mut app), json!(["H", "O", "H", "n"]), "Backspace at the start removes nothing");
    w.key(&mut app, Key::Delete, Modifiers::NONE);
    assert_eq!((items(&mut app), cursor(&mut app).0), (json!(["O", "H", "n"]), json!(0)));

    // Arrows move the cursor; Shift extends the selection; a plain arrow collapses it.
    w.key(&mut app, Key::ArrowRight, Modifiers::NONE);
    w.key(&mut app, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(cursor(&mut app), (json!(2), json!([2, 2])));
    w.key(&mut app, Key::ArrowLeft, Modifiers::SHIFT);
    assert_eq!(cursor(&mut app), (json!(1), json!([1, 2])));
    w.key(&mut app, Key::ArrowLeft, Modifiers::SHIFT);
    assert_eq!(cursor(&mut app), (json!(0), json!([0, 2])));
    w.key(&mut app, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(cursor(&mut app), (json!(2), json!([2, 2])), "Right collapses the selection to its end");
    w.key(&mut app, Key::ArrowLeft, Modifiers::NONE);
    w.key(&mut app, Key::ArrowLeft, Modifiers::NONE);
    w.key(&mut app, Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(cursor(&mut app).0, json!(0), "the cursor stops at the start");

    // Right to left: Left steps forward in the text.
    app.run("editView.setWritingDirection", &json!({"direction": "rtl"})).unwrap();
    w.key(&mut app, Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(cursor(&mut app).0, json!(1));
    app.run("editView.setWritingDirection", &json!({"direction": "ltr"})).unwrap();

    // Cmd+A selects the whole text; typing replaces the selection.
    w.key(&mut app, Key::A, Modifiers::COMMAND);
    assert_eq!(cursor(&mut app), (json!(3), json!([0, 3])));
    w.frame(&mut app, vec![Event::Text("H".into())]);
    assert_eq!(items(&mut app), json!(["H"]));

    // Return types a line break; Up and Down move between lines.
    w.frame(&mut app, vec![Event::Text("O".into())]);
    w.key(&mut app, Key::Enter, Modifiers::NONE);
    w.frame(&mut app, vec![Event::Text("nH".into())]);
    assert_eq!(items(&mut app), json!(["H", "O", {"lineBreak": true}, "n", "H"]));
    app.run("editView.setCursor", &json!({"index": 1})).unwrap();
    w.key(&mut app, Key::ArrowDown, Modifiers::NONE);
    let down = cursor(&mut app).0.as_u64().unwrap();
    assert!((3..=5).contains(&down), "Down lands on the second line, at {down}");
    w.key(&mut app, Key::ArrowUp, Modifiers::NONE);
    let up = cursor(&mut app).0.as_u64().unwrap();
    assert!(up <= 2, "Up lands on the first line, at {up}");
    w.key(&mut app, Key::ArrowUp, Modifiers::NONE);
    assert_eq!(cursor(&mut app).0, json!(0), "Up on the first line goes to the start");
    w.key(&mut app, Key::ArrowDown, Modifiers::NONE);
    w.key(&mut app, Key::ArrowDown, Modifiers::NONE);
    assert_eq!(cursor(&mut app).0, json!(5), "Down on the last line goes to the end");

    // Home and End put the previous and next glyph of the font in place of the current one.
    app.run("editView.setCursor", &json!({"index": 0})).unwrap();
    w.key(&mut app, Key::End, Modifiers::NONE);
    let t = get(&mut app);
    assert_ne!(t["current"], json!("H"));
    assert_eq!(t["items"][0], t["current"]);
    w.key(&mut app, Key::Home, Modifiers::NONE);
    assert_eq!((get(&mut app)["current"].clone(), items(&mut app)[0].clone()), (json!("H"), json!("H")));
}

#[test]
fn space_types_cmd_space_pans_and_escape_switches_modes() {
    let w = Window::new();
    let mut app = text_app(&w);
    app.run("editView.setText", &json!({"chars": "HO"})).unwrap();

    // Space types a space (the keyboard sends the key and its text).
    w.frame(&mut app, vec![press(Key::Space, Modifiers::NONE, true), Event::Text(" ".into()), press(Key::Space, Modifiers::NONE, false)]);
    assert_eq!(items(&mut app), json!(["H", "O", "space"]));
    assert_eq!(app.tools.toolbox.active_id(), "text");

    // Cmd+Space holds the Hand tool, also once Cmd is let go, and types nothing.
    w.frame(&mut app, vec![press(Key::Space, Modifiers::COMMAND, true)]);
    assert_eq!(app.tools.toolbox.active_id(), "hand");
    w.frame(&mut app, vec![press(Key::Space, Modifiers::NONE, true), Event::Text(" ".into())]);
    assert_eq!(app.tools.toolbox.active_id(), "hand");
    w.frame(&mut app, vec![press(Key::Space, Modifiers::NONE, false)]);
    assert_eq!(app.tools.toolbox.active_id(), "text");
    assert_eq!(items(&mut app), json!(["H", "O", "space"]), "panning typed nothing");

    // Escape leaves text mode for the Select tool, and comes back.
    w.key(&mut app, Key::Escape, Modifiers::NONE);
    assert_eq!((get(&mut app)["textMode"].clone(), app.tools.toolbox.active_id()), (json!(false), "select"));
    w.key(&mut app, Key::Escape, Modifiers::NONE);
    assert_eq!((get(&mut app)["textMode"].clone(), app.tools.toolbox.active_id()), (json!(true), "text"));

    // Another tool ends text mode; T chooses the Text tool again.
    app.run("tool.select", &json!({"tool": "draw"})).unwrap();
    assert_eq!(get(&mut app)["textMode"], json!(false));
    w.key(&mut app, Key::T, Modifiers::NONE);
    assert_eq!((get(&mut app)["textMode"].clone(), app.tools.toolbox.active_id()), (json!(true), "text"));

    // The control channel types through the same keys.
    call(&mut app, "ui.text", json!({"text": "Hn"}));
    assert_eq!(items(&mut app), json!(["H", "O", "space", "H", "n"]));
}

#[test]
fn copy_and_paste_carry_the_text() {
    let w = Window::new();
    let mut app = text_app(&w);
    app.run("glyph.newGlyph", &json!({"name": "H.alt"})).unwrap();
    app.run("editView.setText", &json!({"text": "HO/H.alt n"})).unwrap();
    assert_eq!(items(&mut app), json!(["H", "O", "H.alt", "n"]));

    app.run("editView.setCursor", &json!({"index": 0, "selectionLength": 3})).unwrap();
    let output = w.frame(&mut app, vec![Event::Copy]);
    let text = copied(&output);
    assert_eq!(text.len(), 1, "Copy puts the selected text on the clipboard");
    assert!(text[0].starts_with("HO/H.alt"), "{text:?}");

    app.run("editView.setCursor", &json!({"index": 4})).unwrap();
    w.frame(&mut app, vec![Event::Paste(text[0].clone())]);
    assert_eq!(items(&mut app), json!(["H", "O", "H.alt", "n", "H", "O", "H.alt"]), "Paste types the copied text back");

    app.run("editView.setCursor", &json!({"index": 0, "selectionLength": 1})).unwrap();
    let output = w.frame(&mut app, vec![Event::Cut]);
    assert_eq!(copied(&output), ["H"]);
    assert_eq!(items(&mut app), json!(["O", "H.alt", "n", "H", "O", "H.alt"]), "Cut removes the selection");

    // Text from another app: characters, a line break.
    app.run("editView.setCursor", &json!({"index": 0})).unwrap();
    w.frame(&mut app, vec![Event::Paste("n\r\nH".into())]);
    assert_eq!(items(&mut app).as_array().unwrap()[..3], [json!("n"), json!({"lineBreak": true}), json!("H")]);
}

/// Two masters; every glyph of `glyphs` gets a second-master layer as wide as its first.
fn two_masters(glyphs: &[&str]) -> (FontcraftApp, String) {
    let mut app = FontcraftApp::default();
    let mut first = String::new();
    app.session
        .edit("setup", |font| {
            let mut m = font.default_master()?.clone();
            first = m.id.clone();
            m.id = "m02".into();
            m.name = "Bold".into();
            Arc::make_mut(&mut font.masters).push(m);
            for name in glyphs {
                let mut g = font.glyph_mut(name)?;
                let width = g.layers.first().map_or(500.0, |l| l.width);
                g.layers.push(Layer::new("m02", width));
            }
            Ok(())
        })
        .unwrap();
    (app, first)
}

fn width(app: &FontcraftApp, glyph: &str, layer: &str) -> f64 {
    app.session.font().unwrap().glyph(glyph).unwrap().layers.iter().find(|l| l.id == layer).unwrap().width
}

fn key(app: &mut FontcraftApp, params: Value) -> Value {
    call(app, "ui.key", params)
}

#[test]
fn a_spacing_key_in_text_mode_spaces_the_glyph_after_the_cursor_in_the_tabs_master() {
    let (mut app, first) = two_masters(&["H", "O"]);
    app.run("view.newTab", &json!({"glyphs": ["H"]})).unwrap();
    app.run("tool.select", &json!({"tool": "text"})).unwrap();
    app.run("editView.setText", &json!({"chars": "HOH"})).unwrap();
    app.run("view.selectMaster", &json!({"master": 1})).unwrap();
    app.run("editView.setCursor", &json!({"index": 1})).unwrap();
    let (o, o_first, h) = (width(&app, "O", "m02"), width(&app, "O", &first), width(&app, "H", "m02"));

    // Cmd+Right: one more unit on the right of O, in the tab's master only.
    let r = key(&mut app, json!({"key": "ArrowRight", "cmd": true}));
    assert_eq!(r["result"]["handled"], json!(true), "{r}");
    assert_eq!((width(&app, "O", "m02"), width(&app, "O", &first), width(&app, "H", "m02")), (o + 1.0, o_first, h));
    assert_eq!(get(&mut app)["cursor"], json!(1), "the cursor stays");
    // Cmd+Shift+Right: ten units more.
    key(&mut app, json!({"key": "ArrowRight", "cmd": true, "shift": true}));
    assert_eq!(width(&app, "O", "m02"), o + 11.0);

    // The spacing keys are off in the locked kerning mode.
    app.run("kerning.setMode", &json!({"mode": "locked"})).unwrap();
    key(&mut app, json!({"key": "ArrowRight", "cmd": true}));
    assert_eq!(width(&app, "O", "m02"), o + 11.0);

    // Outside text mode the key does not space the glyph.
    app.run("kerning.setMode", &json!({"mode": "kerning"})).unwrap();
    app.run("editView.toggleTextMode", &json!({"mode": "edit"})).unwrap();
    key(&mut app, json!({"key": "ArrowRight", "cmd": true}));
    assert_eq!(width(&app, "O", "m02"), o + 11.0);
}

#[test]
fn a_kerning_key_kerns_the_pair_at_the_cursor_and_does_nothing_without_kerning() {
    let (mut app, _) = two_masters(&["A", "V"]);
    app.run("view.newTab", &json!({"glyphs": ["A"]})).unwrap();
    app.run("tool.select", &json!({"tool": "text"})).unwrap();
    app.run("editView.setText", &json!({"chars": "AVA"})).unwrap();
    app.run("view.selectMaster", &json!({"master": 1})).unwrap();
    app.run("editView.setCursor", &json!({"index": 1})).unwrap();
    let pair = |app: &mut FontcraftApp, left: &str, right: &str, direction: &str| {
        app.run("kerning.getPair", &json!({"left": left, "right": right, "master": "m02", "direction": direction})).unwrap()["value"]
            .as_f64()
            .unwrap_or(0.0)
    };

    // Ctrl+Alt+Left: the pair before and after the cursor; Cmd+Alt+Right: the pair after it.
    key(&mut app, json!({"key": "ArrowLeft", "ctrl": true, "alt": true}));
    assert_eq!(pair(&mut app, "A", "V", "ltr"), 1.0);
    key(&mut app, json!({"key": "ArrowRight", "cmd": true, "alt": true, "shift": true}));
    assert_eq!(pair(&mut app, "V", "A", "ltr"), 10.0);
    let first = app.session.font().unwrap().default_master().unwrap().id.clone();
    let in_first = app.run("kerning.getPair", &json!({"left": "A", "right": "V", "master": first})).unwrap();
    assert_eq!(in_first["value"].as_f64().unwrap_or(0.0), 0.0, "the tab's master only: {in_first}");

    // No-kerning mode: the keys do nothing.
    app.run("kerning.setMode", &json!({"mode": "none"})).unwrap();
    let before = app.session.font().unwrap().clone();
    key(&mut app, json!({"key": "ArrowLeft", "ctrl": true, "alt": true}));
    assert!(app.session.font().unwrap().same_as(&before));

    // Right to left: Ctrl picks the current glyph and the one after it, the pair on its left.
    app.run("kerning.setMode", &json!({"mode": "kerning"})).unwrap();
    app.run("editView.setWritingDirection", &json!({"direction": "rtl"})).unwrap();
    key(&mut app, json!({"key": "ArrowLeft", "ctrl": true, "alt": true}));
    assert_eq!(pair(&mut app, "V", "A", "rtl"), 1.0);
    assert_eq!(get(&mut app)["text"], json!("AVA"), "the text is unchanged");
}

#[test]
fn the_edit_menu_copies_cuts_and_selects_the_text_in_text_mode() {
    let w = Window::new();
    let mut app = text_app(&w);
    app.run("editView.setText", &json!({"chars": "HOn"})).unwrap();
    let before = app.session.font().unwrap().clone();

    // Edit › Copy and Cut over the control channel, as a click on the menu item.
    app.run("editView.setCursor", &json!({"index": 0, "selectionLength": 2})).unwrap();
    let r = call(&mut app, "ui.menu.invoke", json!({"command": "edit.copy"}));
    assert_eq!(r["ok"], json!(true), "{r}");
    assert_eq!(copied(&w.frame(&mut app, Vec::new())), ["HO"]);
    call(&mut app, "ui.menu.invoke", json!({"command": "edit.cut"}));
    assert_eq!(copied(&w.frame(&mut app, Vec::new())), ["HO"]);
    assert_eq!(items(&mut app), json!(["n"]), "Cut removes the selected text");

    // Edit › Select All from the menu bar selects the whole text, not the outline.
    app.run("editView.setText", &json!({"chars": "HOH"})).unwrap();
    app.run_menu_command("edit.selectAll", json!({})).unwrap();
    assert_eq!(cursor(&mut app), (json!(3), json!([0, 3])));
    assert!(app.session.font().unwrap().same_as(&before), "the font is unchanged");
}

#[test]
fn the_kerning_keys_at_the_end_of_the_text_take_the_last_glyph() {
    let mut app = FontcraftApp::default();
    app.run("view.newTab", &json!({"glyphs": ["A"]})).unwrap();
    app.run("tool.select", &json!({"tool": "text"})).unwrap();
    app.run("editView.setText", &json!({"chars": "AV"})).unwrap();
    assert_eq!(get(&mut app)["cursor"], json!(2));
    let r = key(&mut app, json!({"key": "ArrowLeft", "ctrl": true, "alt": true}));
    assert_eq!(r["result"]["errors"], json!([]), "{r}");
    assert_eq!(app.run("kerning.getPair", &json!({"left": "A", "right": "V"})).unwrap()["value"], json!(1.0));
}

#[test]
fn a_spacing_key_spaces_the_layer_picked_for_the_current_glyph() {
    let mut app = FontcraftApp::default();
    let master = app.session.font().unwrap().default_master().unwrap().id.clone();
    let backup = app.run("layers.addBackupLayer", &json!({"glyphs": ["O"], "master": master})).unwrap()["layer"].as_str().unwrap().to_string();
    app.run("view.newTab", &json!({"glyphs": ["O"]})).unwrap();
    app.run("layers.select", &json!({"layer": backup})).unwrap();
    app.run("tool.select", &json!({"tool": "text"})).unwrap();
    let (o, o_backup) = (width(&app, "O", &master), width(&app, "O", &backup));
    key(&mut app, json!({"key": "ArrowRight", "cmd": true}));
    assert_eq!((width(&app, "O", &master), width(&app, "O", &backup)), (o, o_backup + 1.0));
}

#[test]
fn outline_edits_from_the_menu_leave_the_outline_alone_in_text_mode() {
    let w = Window::new();
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "H", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.run("view.newTab", &json!({"glyphs": ["H", "O"]})).unwrap();
    app.run_menu_command("edit.selectAll", json!({})).unwrap();
    assert!(!app.state.tab().unwrap().selection.is_empty(), "the nodes of H are selected");
    app.run("tool.select", &json!({"tool": "text"})).unwrap();
    w.frame(&mut app, Vec::new());
    let before = app.session.font().unwrap().clone();

    app.run("editView.setCursor", &json!({"index": 0, "selectionLength": 1})).unwrap();
    app.run_menu_command("edit.delete", json!({})).unwrap();
    assert_eq!(items(&mut app), json!(["O"]), "Delete removes the selected text");
    assert!(app.run_menu_command("edit.deleteInAllMasters", json!({})).is_err());
    let r = call(&mut app, "ui.menu.invoke", json!({"command": "edit.pasteSpecial"}));
    assert_eq!(r["ok"], json!(false), "{r}");
    // The paste key types the clipboard's text; with Alt (Paste Special) it types nothing.
    w.frame(&mut app, vec![Event::Paste("xyz".into())]);
    assert_eq!(items(&mut app), json!(["x", "y", "z", "O"]));
    w.frame(&mut app, vec![Event::ModifiersChanged(Modifiers::ALT), Event::Paste("abc".into())]);
    assert_eq!(items(&mut app), json!(["x", "y", "z", "O"]));
    assert!(app.session.font().unwrap().same_as(&before), "the outline is unchanged");
}
