//! Font View's sidebar, search field and filter dialogs, and the global guides' filters: clicks
//! and keys in headless frames, and each dialog filled over the control channel against the
//! command it runs.

use std::sync::Arc;

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, Shape};
use fontcraft_model::Font;
use serde_json::{Value, json};

use crate::shell::font_view::{search, sidebar};
use crate::theme::Tokens;
use crate::{FontcraftApp, control};

/// A headless window: one egui context kept across frames, so pointer, focus and double-click
/// state carry over.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    /// One frame with these events and modifiers; the shapes it drew, nested lists flattened.
    fn frame_with(&self, app: &mut FontcraftApp, events: Vec<Event>, modifiers: Modifiers) -> Vec<Shape> {
        let mut events = events;
        events.insert(0, Event::ModifiersChanged(modifiers));
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
        let mut out: Vec<Shape> = output.shapes.into_iter().map(|c| c.shape).collect();
        let mut flat = Vec::new();
        while let Some(shape) = out.pop() {
            match shape {
                Shape::Vec(list) => out.extend(list),
                other => flat.push(other),
            }
        }
        flat
    }

    fn frame(&self, app: &mut FontcraftApp) -> Vec<Shape> {
        self.frame_with(app, Vec::new(), Modifiers::NONE)
    }

    fn click(&self, app: &mut FontcraftApp, at: Pos2, mods: Modifiers) {
        self.frame_with(app, vec![Event::PointerMoved(at), button(at, true, mods)], mods);
        self.frame_with(app, vec![button(at, false, mods)], mods);
        self.frame(app);
    }

    /// A double-click after a pause, so earlier clicks do not join it.
    fn double_click(&self, app: &mut FontcraftApp, at: Pos2) {
        for _ in 0..40 {
            self.frame(app);
        }
        self.frame_with(app, vec![Event::PointerMoved(at), button(at, true, Modifiers::NONE)], Modifiers::NONE);
        for pressed in [false, true, false] {
            self.frame_with(app, vec![button(at, pressed, Modifiers::NONE)], Modifiers::NONE);
        }
        self.frame(app);
    }
}

impl Window {
    /// Presses at `from` after a pause and moves to `to` in steps, holding the button.
    fn press_and_move(&self, app: &mut FontcraftApp, from: Pos2, to: Pos2) {
        for _ in 0..40 {
            self.frame(app);
        }
        self.frame_with(app, vec![Event::PointerMoved(from), button(from, true, Modifiers::NONE)], Modifiers::NONE);
        for step in 1..=8 {
            let at = from + (to - from) * (step as f32 / 8.0);
            self.frame_with(app, vec![Event::PointerMoved(at)], Modifiers::NONE);
        }
    }

    fn release(&self, app: &mut FontcraftApp, at: Pos2) {
        self.frame_with(app, vec![button(at, false, Modifiers::NONE)], Modifiers::NONE);
        self.frame(app);
        self.frame(app);
    }

    fn drag(&self, app: &mut FontcraftApp, from: Pos2, to: Pos2) {
        self.press_and_move(app, from, to);
        self.release(app, to);
    }

    fn key(&self, app: &mut FontcraftApp, key: Key) {
        let event = Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        self.frame_with(app, vec![event], Modifiers::NONE);
        self.frame(app);
    }

    fn type_text(&self, app: &mut FontcraftApp, text: &str) {
        self.frame_with(app, vec![Event::Text(text.into())], Modifiers::NONE);
        self.frame(app);
    }
}

fn button(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers }
}

/// The sidebar row of the entry `path` (`categories/Letter`) in the last frame.
fn row(app: &FontcraftApp, path: &str) -> sidebar::Row {
    app.font_view.sidebar.laid_out.iter().find(|r| r.path.join("/") == path).cloned().unwrap_or_else(|| panic!("{path} was not laid out"))
}

fn state(app: &mut FontcraftApp) -> Value {
    app.run("fontView.getState", &json!({})).unwrap()
}

fn visible(app: &mut FontcraftApp) -> Vec<String> {
    let mut names: Vec<String> = state(app)["visible"].as_array().unwrap().iter().map(|n| n.as_str().unwrap().to_string()).collect();
    names.sort();
    names
}

fn entries(app: &mut FontcraftApp) -> Vec<String> {
    let mut list: Vec<String> = state(app)["sidebarEntries"].as_array().unwrap().iter().map(|n| n.as_str().unwrap().to_string()).collect();
    list.sort();
    list
}

fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
    assert_eq!(r["ok"], true, "{method}: {r}");
    r["result"].clone()
}

fn open(app: &mut FontcraftApp, params: Value) -> Value {
    call(app, "engine.execute", json!({"command": "dialog.open", "params": params}))
}

fn set(app: &mut FontcraftApp, field: &str, value: Value) -> Value {
    call(app, "ui.dialog.set", json!({"field": field, "value": value}))
}

fn field(dialog: &Value, key: &str) -> Value {
    dialog["fields"].as_array().unwrap().iter().find(|f| f["id"] == key).unwrap_or_else(|| panic!("no field {key}: {dialog}")).clone()
}

fn setting(app: &FontcraftApp, id: &str) -> Value {
    app.session.settings().get(id).unwrap()
}

fn font(app: &FontcraftApp) -> Arc<Font> {
    app.session.doc().unwrap().font.clone()
}

fn add_smart(app: &mut FontcraftApp, name: &str, rule: Value) {
    app.run("fontView.addSmartFilter", &json!({"name": name, "rules": {"all": [rule]}})).unwrap();
}

#[test]
fn sidebar_clicks_with_cmd_and_shift_combine_entries_as_documented() {
    let mut app = FontcraftApp::default();
    add_smart(&mut app, "Vowels", json!({"property": "glyphName", "op": "matches", "value": "[AEIOUaeiou]"}));
    add_smart(&mut app, "Early", json!({"property": "glyphName", "op": "matches", "value": "[A-Ea-e]"}));
    add_smart(&mut app, "Upper", json!({"property": "case", "op": "is", "value": "upper"}));
    let w = Window::new();
    w.frame(&mut app);
    assert_eq!(visible(&mut app).len(), 54, "All at first");

    // A click selects one entry; the chevron opens a category's subcategories.
    let at = row(&app, "categories/Letter").chevron.unwrap().center();
    w.click(&mut app, at, Modifiers::NONE);
    assert!(entries(&mut app).is_empty(), "the chevron selects nothing");
    let at = row(&app, "categories/Letter/Uppercase").rect.center();
    w.click(&mut app, at, Modifiers::NONE);
    assert_eq!(visible(&mut app).len(), 26);
    // Cmd-click: categories add up.
    let at = row(&app, "categories/Letter/Lowercase").rect.center();
    w.click(&mut app, at, Modifiers::COMMAND);
    assert_eq!(visible(&mut app).len(), 52);
    // Sections narrow each other.
    let at = row(&app, "filters/Vowels").rect.center();
    w.click(&mut app, at, Modifiers::COMMAND);
    assert_eq!(visible(&mut app), ["A", "E", "I", "O", "U", "a", "e", "i", "o", "u"]);
    // Filters narrow each other.
    let at = row(&app, "filters/Early").rect.center();
    w.click(&mut app, at, Modifiers::COMMAND);
    assert_eq!(visible(&mut app), ["A", "E", "a", "e"]);

    // Shift-click: the filters from the last one selected to the one clicked.
    let at = row(&app, "filters/Vowels").rect.center();
    w.click(&mut app, at, Modifiers::NONE);
    assert_eq!(entries(&mut app), ["filters/Vowels"]);
    let at = row(&app, "filters/Upper").rect.center();
    w.click(&mut app, at, Modifiers::SHIFT);
    assert_eq!(entries(&mut app), ["filters/Early", "filters/Upper", "filters/Vowels"]);
    assert_eq!(visible(&mut app), ["A", "E"]);
    // Cmd-click on a selected entry removes it.
    let at = row(&app, "filters/Early").rect.center();
    w.click(&mut app, at, Modifiers::COMMAND);
    assert_eq!(visible(&mut app), ["A", "E", "I", "O", "U"]);

    // All shows every glyph again.
    let at = row(&app, "all").rect.center();
    w.click(&mut app, at, Modifiers::NONE);
    let s = state(&mut app);
    assert_eq!((s["sidebarEntries"].clone(), s["counts"]["visible"].as_u64()), (json!([]), Some(54)));

    // Over the control channel, an unknown entry is refused and changes nothing.
    app.run("fontView.selectSidebarEntries", &json!({"entries": ["filters/Vowels"]})).unwrap();
    assert!(app.run("fontView.selectSidebarEntries", &json!({"entries": ["categories/Nonsense"]})).is_err());
    assert!(app.run("fontView.selectSidebarEntries", &json!({"entries": ["filters/Upper"], "mode": "sideways"})).is_err());
    assert_eq!(entries(&mut app), ["filters/Vowels"]);

    // A removed filter drops out of the selection.
    app.run("fontView.removeFilter", &json!({"filter": "filters/Vowels"})).unwrap();
    w.frame(&mut app);
    assert_eq!(state(&mut app)["sidebarEntries"], json!([]));
}

#[test]
fn the_search_field_narrows_the_grid_and_hidden_glyphs_are_deselected() {
    let mut app = FontcraftApp::default();
    let w = Window::new();
    w.frame(&mut app);
    // Cmd+F gives the field the keyboard; typing searches as it goes.
    let cmd_f = Event::Key { key: Key::F, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND };
    w.frame_with(&mut app, vec![cmd_f], Modifiers::COMMAND);
    w.frame(&mut app);
    w.frame(&mut app);
    assert_eq!(w.ctx.memory(|m| m.focused()), Some(search::field_id()));
    w.frame_with(&mut app, vec![Event::Text("b".into())], Modifiers::NONE);
    w.frame(&mut app);
    assert_eq!(state(&mut app)["search"], json!({"query": "b"}));
    assert_eq!(visible(&mut app), ["B", "b"]);

    app.run("fontView.setSearch", &json!({"query": "b", "matchCase": true})).unwrap();
    assert_eq!(visible(&mut app), ["b"]);
    // Options left out keep their value; a regular expression matches a whole name.
    app.run("fontView.setSearch", &json!({"query": "[a-c]", "regex": true})).unwrap();
    assert_eq!(visible(&mut app), ["a", "b", "c"]);
    let error = app.run("fontView.setSearch", &json!({"query": "[a-"})).unwrap_err();
    assert!(error.contains("fontView.setSearch"), "{error}");
    assert_eq!(state(&mut app)["search"]["query"], "[a-c]", "a refused term changes nothing");
    assert!(app.run("fontView.setSearch", &json!({"query": "a", "fields": ["shape"]})).is_err());

    // Selected glyphs that the new view hides are deselected.
    app.run("fontView.setSelection", &json!({"glyphs": ["a", "B"]})).unwrap();
    let r = app.run("fontView.setSearch", &json!({"query": "a", "matchCase": false, "regex": false, "fields": ["name"]})).unwrap();
    assert_eq!(r["count"], 3, "A, a and space: {r}");
    assert_eq!(app.state.selected_glyphs, ["a"]);

    // An empty term with the default options lists every glyph.
    app.run("fontView.setSearch", &json!({"query": "", "fields": []})).unwrap();
    let s = state(&mut app);
    assert_eq!((s["search"].clone(), s["counts"]["visible"].as_u64()), (Value::Null, Some(54)));

    // fontView.focusSearch, the command behind Cmd+F: the field takes the keyboard back from
    // wherever it went; in Edit View the command does nothing.
    w.ctx.memory_mut(|m| m.surrender_focus(search::field_id()));
    w.frame(&mut app);
    assert_ne!(w.ctx.memory(|m| m.focused()), Some(search::field_id()));
    assert_eq!(app.run("fontView.focusSearch", &json!({})).unwrap(), json!({"focused": true}));
    w.frame(&mut app);
    w.frame(&mut app);
    assert_eq!(w.ctx.memory(|m| m.focused()), Some(search::field_id()));
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    assert_eq!(app.run("fontView.focusSearch", &json!({})).unwrap(), json!({"focused": false}));
}

#[test]
fn the_smart_filter_editor_gives_the_settings_its_commands_give() {
    let mut app = FontcraftApp::default();
    let opened = open(&mut app, json!({"command": "fontView.addSmartFilter"}));
    assert_eq!(opened["dialog"], "dialog.smartFilterEditor");
    assert_eq!(field(&opened, "name")["value"], "New Filter");
    let starter = json!({"all": [{"property": "glyphName", "op": "contains", "value": ""}]});
    assert_eq!(field(&opened, "rules")["value"], starter, "a new filter starts with one rule");
    set(&mut app, "name", json!("Vowels"));
    // A count typed as text becomes a number, as the rows send it.
    set(
        &mut app,
        "rules",
        json!({"all": [
            {"property": "glyphName", "op": "matches", "value": "[AEIOU]"},
            {"property": "countOfPaths", "op": "==", "value": "0"},
        ]}),
    );
    call(&mut app, "ui.dialog.confirm", json!({}));

    let mut direct = FontcraftApp::default();
    let rules = json!({"all": [
        {"property": "glyphName", "op": "matches", "value": "[AEIOU]"},
        {"property": "countOfPaths", "op": "==", "value": 0},
    ]});
    direct.run("fontView.addSmartFilter", &json!({"name": "Vowels", "rules": rules})).unwrap();
    assert_eq!(setting(&app, "fontView.filters"), setting(&direct, "fontView.filters"));

    // A double-click on the filter's icon opens its editor, which reads the stored name and rules.
    let w = Window::new();
    w.frame(&mut app);
    let at = row(&app, "filters/Vowels").icon.unwrap().center();
    w.double_click(&mut app, at);
    let shown = call(&mut app, "ui.dialog.inspect", json!({}));
    assert_eq!((shown["open"].as_bool(), shown["command"].as_str()), (Some(true), Some("fontView.editFilter")));
    assert_eq!(field(&shown, "name")["value"], "Vowels");
    assert_eq!(field(&shown, "rules")["value"], rules);
    set(&mut app, "rules", json!({"any": [{"property": "glyphName", "op": "matches", "value": "[AEIOU]"}]}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    let rules = json!({"any": [{"property": "glyphName", "op": "matches", "value": "[AEIOU]"}]});
    direct.run("fontView.editFilter", &json!({"filter": "filters/Vowels", "name": "Vowels", "rules": rules})).unwrap();
    assert_eq!(setting(&app, "fontView.filters"), setting(&direct, "fontView.filters"));

    // Cancel changes nothing.
    let (before, revision) = (setting(&app, "fontView.filters"), app.session.settings().revision());
    let opened = open(&mut app, json!({"dialog": "dialog.smartFilterEditor", "params": {"filter": "filters/Vowels"}}));
    assert_eq!(field(&opened, "rules")["value"], rules);
    set(&mut app, "name", json!("Consonants"));
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert_eq!((setting(&app, "fontView.filters"), app.session.settings().revision()), (before, revision));
    let r = control::handle(
        &mut app,
        &json!({"id": 1, "method": "engine.execute", "params": {"command": "dialog.open", "params": {"dialog": "dialog.smartFilterEditor", "params": {"filter": "filters/Gone"}}}}),
    );
    assert_eq!(r["ok"], false, "{r}");
}

#[test]
fn the_rule_rows_build_a_nested_group_as_the_command_takes_it() {
    use crate::dialogs::rule_tree::control_id;
    let mut app = FontcraftApp::default();
    open(&mut app, json!({"command": "fontView.addSmartFilter"}));
    set(&mut app, "name", json!("Nested"));
    let w = Window::new();
    w.frame(&mut app);
    w.frame(&mut app);
    let press = |app: &mut FontcraftApp, path: &[usize], part: &str| {
        let id = control_id("rules", path, part);
        // The dialog centres itself again when a row makes it wider.
        for _ in 0..3 {
            w.frame(app);
        }
        let at = w.ctx.read_response(id).unwrap_or_else(|| panic!("{path:?} {part} was not drawn")).rect.center();
        w.click(app, at, Modifiers::NONE);
    };
    // … on the top group adds a nested Any group with one rule; + on that group adds a second.
    press(&mut app, &[], "addGroup");
    press(&mut app, &[1], "add");
    // Typing into the nested group's first rule sets its value.
    press(&mut app, &[1, 0], "value");
    w.frame_with(&mut app, vec![Event::Text("a.sc".into())], Modifiers::NONE);
    w.frame(&mut app);
    // − on the top group's first rule removes it.
    press(&mut app, &[0], "remove");
    let rule = |value: &str| json!({"property": "glyphName", "op": "contains", "value": value});
    let expected = json!({"all": [{"any": [rule("a.sc"), rule("")]}]});
    let shown = call(&mut app, "ui.dialog.inspect", json!({}));
    assert_eq!(field(&shown, "rules")["value"], expected);
    call(&mut app, "ui.dialog.confirm", json!({}));
    let mut direct = FontcraftApp::default();
    direct.run("fontView.addSmartFilter", &json!({"name": "Nested", "rules": expected})).unwrap();
    assert_eq!(setting(&app, "fontView.filters"), setting(&direct, "fontView.filters"));
}

#[test]
fn alt_on_a_plus_button_adds_a_nested_group_as_the_command_takes_it() {
    use crate::dialogs::rule_tree::control_id;
    let mut app = FontcraftApp::default();
    open(&mut app, json!({"command": "fontView.addSmartFilter"}));
    set(&mut app, "name", json!("Grouped"));
    let w = Window::new();
    let alt_press = |app: &mut FontcraftApp, path: &[usize]| {
        // With Alt held each + shows …, which can widen the rows; the dialog takes a few frames
        // to size and centre itself again.
        for _ in 0..8 {
            w.frame_with(app, Vec::new(), Modifiers::ALT);
        }
        let at = w.ctx.read_response(control_id("rules", path, "add")).unwrap_or_else(|| panic!("{path:?} + was not drawn")).rect.center();
        w.click(app, at, Modifiers::ALT);
    };
    // Alt on the top group's + adds a group at its end; on the first rule's +, a group after it.
    alt_press(&mut app, &[]);
    alt_press(&mut app, &[0]);
    let starter = json!({"property": "glyphName", "op": "contains", "value": ""});
    let expected = json!({"all": [starter, {"any": [starter]}, {"any": [starter]}]});
    let shown = call(&mut app, "ui.dialog.inspect", json!({}));
    assert_eq!(field(&shown, "rules")["value"], expected);
    call(&mut app, "ui.dialog.confirm", json!({}));
    let mut direct = FontcraftApp::default();
    direct.run("fontView.addSmartFilter", &json!({"name": "Grouped", "rules": expected})).unwrap();
    assert_eq!(setting(&app, "fontView.filters"), setting(&direct, "fontView.filters"));
}

/// The names of the top-level filters, in order.
fn top_filters(app: &FontcraftApp) -> Vec<String> {
    setting(app, "fontView.filters").as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn renaming_a_filter_in_place_gives_the_settings_rename_filter_gives() {
    let mut app = FontcraftApp::default();
    let mut direct = FontcraftApp::default();
    let rule = json!({"property": "glyphName", "op": "matches", "value": "[AEIOU]"});
    add_smart(&mut app, "Vowels", rule.clone());
    add_smart(&mut direct, "Vowels", rule);
    let w = Window::new();
    w.frame(&mut app);

    // A double-click on the name opens a field holding it, all selected; Return renames.
    let r = row(&app, "filters/Vowels");
    let at = Pos2::new(r.icon.unwrap().right() + 12.0, r.rect.center().y);
    w.double_click(&mut app, at);
    w.frame(&mut app);
    assert_eq!(w.ctx.memory(|m| m.focused()), Some(sidebar::rename_id()));
    w.type_text(&mut app, "Open");
    w.key(&mut app, Key::Enter);
    assert!(app.font_view.sidebar.renaming.is_none());
    direct.run("fontView.renameFilter", &json!({"filter": "filters/Vowels", "name": "Open"})).unwrap();
    assert_eq!(setting(&app, "fontView.filters"), setting(&direct, "fontView.filters"));
    assert_eq!(entries(&mut app), ["filters/Open"], "the renamed filter stays selected");

    // Return with the filter selected opens the field again; Escape changes nothing.
    let revision = app.session.settings().revision();
    w.key(&mut app, Key::Enter);
    assert_eq!(app.font_view.sidebar.renaming.as_ref().map(|r| r.path.join("/")), Some("filters/Open".to_string()));
    w.frame(&mut app);
    w.type_text(&mut app, "Shut");
    w.key(&mut app, Key::Escape);
    assert!(app.font_view.sidebar.renaming.is_none());
    assert_eq!(app.session.settings().revision(), revision, "Escape renames nothing");

    // Return again, a new name, Return.
    w.key(&mut app, Key::Enter);
    w.frame(&mut app);
    w.type_text(&mut app, "Shut");
    w.key(&mut app, Key::Enter);
    direct.run("fontView.renameFilter", &json!({"filter": "filters/Open", "name": "Shut"})).unwrap();
    assert_eq!(setting(&app, "fontView.filters"), setting(&direct, "fontView.filters"));

    // After a press outside the sidebar, Return leaves the filter's name alone.
    w.click(&mut app, Pos2::new(1000.0, 600.0), Modifiers::NONE);
    assert!(!sidebar::has_keyboard(&app));
    w.key(&mut app, Key::Enter);
    assert!(app.font_view.sidebar.renaming.is_none());
}

#[test]
fn dragging_a_filter_gives_the_settings_move_filter_gives() {
    let mut app = FontcraftApp::default();
    let mut direct = FontcraftApp::default();
    for a in [&mut app, &mut direct] {
        for name in ["A", "B", "C"] {
            add_smart(a, name, json!({"property": "glyphName", "op": "is", "value": name}));
        }
        a.run("fontView.addFilterFolder", &json!({"name": "Work"})).unwrap();
    }
    let base = top_filters(&app).iter().position(|n| n == "A").unwrap();
    let w = Window::new();
    w.frame(&mut app);

    // Onto a folder's middle: into the folder, which is framed while the filter is over it.
    let (a, work) = (row(&app, "filters/A"), row(&app, "filters/Work"));
    w.press_and_move(&mut app, a.rect.center(), work.rect.center());
    assert_eq!(app.font_view.sidebar.drop, Some(sidebar::DropTarget { parent: vec!["Work".into()], index: 0, mark: sidebar::Mark::Into(work.rect) }));
    w.release(&mut app, work.rect.center());
    direct.run("fontView.moveFilter", &json!({"filter": "filters/A", "parent": "filters/Work", "index": 0})).unwrap();
    assert_eq!(setting(&app, "fontView.filters"), setting(&direct, "fontView.filters"));

    // Onto the top of a row: before it.
    let (b, c) = (row(&app, "filters/B"), row(&app, "filters/C"));
    w.drag(&mut app, c.rect.center(), Pos2::new(b.rect.center().x, b.rect.top() + 2.0));
    direct.run("fontView.moveFilter", &json!({"filter": "filters/C", "index": base})).unwrap();
    assert_eq!(setting(&app, "fontView.filters"), setting(&direct, "fontView.filters"));

    // Onto the bottom of a row further down: after it, counted without the dragged filter.
    let (b, c) = (row(&app, "filters/B"), row(&app, "filters/C"));
    w.drag(&mut app, c.rect.center(), Pos2::new(b.rect.center().x, b.rect.bottom() - 2.0));
    direct.run("fontView.moveFilter", &json!({"filter": "filters/C", "index": base + 1})).unwrap();
    assert_eq!(setting(&app, "fontView.filters"), setting(&direct, "fontView.filters"));
    assert_eq!(top_filters(&app).get(base..), Some(&["B".to_string(), "C".into(), "Work".into()][..]));

    // A folder dragged onto an entry inside itself shows no mark, and its drop runs nothing.
    let chevron = row(&app, "filters/Work").chevron.unwrap().center();
    w.click(&mut app, chevron, Modifiers::NONE);
    let (work, inner) = (row(&app, "filters/Work"), row(&app, "filters/Work/A"));
    let revision = app.session.settings().revision();
    w.press_and_move(&mut app, work.rect.center(), inner.rect.center());
    assert!(app.font_view.sidebar.dragging.is_some());
    assert_eq!(app.font_view.sidebar.drop, None);
    w.release(&mut app, inner.rect.center());
    assert_eq!(app.session.settings().revision(), revision);
}

#[test]
fn a_groups_file_entry_shows_in_the_categories_section() {
    let mut app = FontcraftApp::default();
    let w = Window::new();
    w.frame(&mut app);
    let problems = app.session.set_user_groups(&[("Groups.plist", "{categories = ({name = Odd; list = (A, zz);});}")]);
    assert!(problems.is_empty(), "{problems:?}");
    w.frame(&mut app);
    let at = row(&app, "categories/Odd").rect.center();
    w.click(&mut app, at, Modifiers::NONE);
    assert_eq!(visible(&mut app), ["A"]);

    // Files set again with other entries replace the section's entries and drop the selection of
    // the one that went away.
    let problems = app.session.set_user_groups(&[("Groups.plist", "{categories = ({name = Evn; list = (B, zz);});}")]);
    assert!(problems.is_empty(), "{problems:?}");
    w.frame(&mut app);
    assert!(app.font_view.sidebar.laid_out.iter().all(|r| r.path.join("/") != "categories/Odd"));
    row(&app, "categories/Evn");
    assert!(entries(&mut app).is_empty(), "the removed entry is deselected");
}

#[test]
fn return_with_the_save_question_open_saves_instead_of_renaming() {
    let dir = std::env::temp_dir().join(format!("fontcraft-sidebar-return-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Kept.fontcraft");
    let mut app = FontcraftApp::default();
    add_smart(&mut app, "Vowels", json!({"property": "glyphName", "op": "matches", "value": "[AEIOU]"}));
    app.run("file.saveAs", &json!({"path": path.to_string_lossy()})).unwrap();
    let w = Window::new();
    w.frame(&mut app);
    // The sidebar takes the keyboard with one filter selected.
    let at = row(&app, "filters/Vowels").rect.center();
    w.click(&mut app, at, Modifiers::NONE);
    assert_eq!(entries(&mut app), ["filters/Vowels"]);
    assert!(sidebar::has_keyboard(&app));

    app.run("glyph.newGlyph", &json!({"name": "x.alt"})).unwrap();
    app.run_menu_command("file.close", json!({})).unwrap();
    assert!(app.close_prompt.is_open());
    w.key(&mut app, Key::Enter);
    assert!(app.font_view.sidebar.renaming.is_none());
    assert!(!app.close_prompt.is_open());
    assert!(app.session.documents().is_empty(), "Return is Save: the document is written, then closed");
    assert!(std::fs::read_to_string(&path).unwrap().contains("x.alt"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn return_on_a_selected_filter_inside_a_closed_folder_renames_nothing() {
    let mut app = FontcraftApp::default();
    add_smart(&mut app, "A", json!({"property": "glyphName", "op": "is", "value": "A"}));
    app.run("fontView.addFilterFolder", &json!({"name": "Work"})).unwrap();
    app.run("fontView.moveFilter", &json!({"filter": "filters/A", "parent": "filters/Work", "index": 0})).unwrap();
    app.run("fontView.selectSidebarEntries", &json!({"entries": ["filters/Work/A"]})).unwrap();
    let w = Window::new();
    w.frame(&mut app);
    // A press on a section heading gives the sidebar the keyboard and keeps the selection.
    let at = row(&app, "languages").rect.center();
    w.click(&mut app, at, Modifiers::NONE);
    assert!(sidebar::has_keyboard(&app));
    assert_eq!(entries(&mut app), ["filters/Work/A"]);

    w.key(&mut app, Key::Enter);
    assert!(app.font_view.sidebar.renaming.is_none(), "the filter's row is not drawn");

    // Opening the folder takes no keyboard; Return then opens the drawn row's name.
    let chevron = row(&app, "filters/Work").chevron.unwrap().center();
    w.click(&mut app, chevron, Modifiers::NONE);
    row(&app, "filters/Work/A");
    assert!(app.font_view.sidebar.renaming.is_none());
    assert_ne!(w.ctx.memory(|m| m.focused()), Some(sidebar::rename_id()));
    w.key(&mut app, Key::Enter);
    assert_eq!(app.font_view.sidebar.renaming.as_ref().map(|r| r.path.join("/")), Some("filters/Work/A".to_string()));
}

#[test]
fn a_drop_on_a_row_scrolled_out_of_view_does_nothing() {
    let mut app = FontcraftApp::default();
    for i in 0..60 {
        add_smart(&mut app, &format!("F{i}"), json!({"property": "glyphName", "op": "is", "value": "A"}));
    }
    let w = Window::new();
    w.frame(&mut app);
    let clip = app.font_view.sidebar.rows_clip.unwrap();
    // A filter row laid out below the part of the list shown, under the actions menu.
    let hidden = app
        .font_view
        .sidebar
        .laid_out
        .iter()
        .find(|r| {
            r.path.len() == 2
                && r.path.first().is_some_and(|p| p == "filters")
                && r.rect.center().y > clip.bottom() + 1.0
                && r.rect.center().y < 795.0
        })
        .cloned()
        .expect("a filter row hidden below the list");
    let source = row(&app, "filters/F0");
    assert!(clip.contains(source.rect.center()));
    let revision = app.session.settings().revision();

    w.press_and_move(&mut app, source.rect.center(), hidden.rect.center());
    assert!(app.font_view.sidebar.dragging.is_some());
    assert_eq!(app.font_view.sidebar.drop, None);
    w.release(&mut app, hidden.rect.center());
    assert_eq!(app.session.settings().revision(), revision, "nothing moved");
}

#[test]
fn a_right_button_drag_moves_no_filter() {
    let mut app = FontcraftApp::default();
    add_smart(&mut app, "A", json!({"property": "glyphName", "op": "is", "value": "A"}));
    app.run("fontView.addFilterFolder", &json!({"name": "Work"})).unwrap();
    let w = Window::new();
    w.frame(&mut app);
    let (a, work) = (row(&app, "filters/A"), row(&app, "filters/Work"));
    let revision = app.session.settings().revision();
    let right = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    for _ in 0..40 {
        w.frame(&mut app);
    }
    let (from, to) = (a.rect.center(), work.rect.center());
    w.frame_with(&mut app, vec![Event::PointerMoved(from), right(from, true)], Modifiers::NONE);
    for step in 1..=8 {
        w.frame_with(&mut app, vec![Event::PointerMoved(from + (to - from) * (step as f32 / 8.0))], Modifiers::NONE);
    }
    assert!(app.font_view.sidebar.dragging.is_none());
    assert_eq!(app.font_view.sidebar.drop, None);
    w.frame_with(&mut app, vec![right(to, false)], Modifiers::NONE);
    w.frame(&mut app);
    assert_eq!(app.session.settings().revision(), revision, "nothing moved");
}

#[test]
fn the_list_filter_editor_gives_the_settings_its_commands_give() {
    let mut app = FontcraftApp::default();
    // A new list filter opens with the glyphs selected in Font View.
    app.run("fontView.setSelection", &json!({"glyphs": ["A", "B"]})).unwrap();
    let opened = open(&mut app, json!({"command": "fontView.addListFilter"}));
    assert_eq!(opened["dialog"], "dialog.listFilterEditor");
    assert_eq!(field(&opened, "glyphs")["value"], "A B");
    set(&mut app, "name", json!("Pair"));
    call(&mut app, "ui.dialog.confirm", json!({}));
    let mut direct = FontcraftApp::default();
    direct.run("fontView.addListFilter", &json!({"name": "Pair", "glyphs": ["A", "B"]})).unwrap();
    assert_eq!(setting(&app, "fontView.filters"), setting(&direct, "fontView.filters"));

    // The editor of a stored filter reads it by its path.
    let opened = open(&mut app, json!({"dialog": "dialog.listFilterEditor", "params": {"filter": "filters/Pair"}}));
    assert_eq!((field(&opened, "name")["value"].clone(), field(&opened, "glyphs")["value"].clone()), (json!("Pair"), json!("A B")));
    set(&mut app, "glyphs", json!("C\nA  Q.ss01"));
    call(&mut app, "ui.dialog.confirm", json!({}));
    direct.run("fontView.editFilter", &json!({"filter": "filters/Pair", "glyphs": ["C", "A", "Q.ss01"]})).unwrap();
    assert_eq!(setting(&app, "fontView.filters"), setting(&direct, "fontView.filters"));

    let (before, revision) = (setting(&app, "fontView.filters"), app.session.settings().revision());
    open(&mut app, json!({"dialog": "dialog.listFilterEditor", "params": {"filter": "filters/Pair"}}));
    set(&mut app, "glyphs", json!("Z"));
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert_eq!((setting(&app, "fontView.filters"), app.session.settings().revision()), (before, revision));
    let r = control::handle(
        &mut app,
        &json!({"id": 1, "method": "engine.execute", "params": {"command": "dialog.open", "params": {"dialog": "dialog.listFilterEditor", "params": {"filter": "filters/Gone"}}}}),
    );
    assert_eq!(r["ok"], false, "{r}");
}

#[test]
fn the_languages_list_gives_the_settings_its_command_gives() {
    let mut app = FontcraftApp::default();
    let opened = open(&mut app, json!({"dialog": "dialog.languagesList"}));
    let scripts = field(&opened, "scripts");
    assert_eq!(scripts["value"], json!(["cyrillic", "greek", "latin"]), "the default list");
    assert!(scripts["choices"].as_array().unwrap().contains(&json!("arabic")), "{scripts}");
    let shown = set(&mut app, "scripts", json!(["arabic", "latin"]));
    let params = shown["params"].clone();
    call(&mut app, "ui.dialog.confirm", json!({}));
    let mut direct = FontcraftApp::default();
    direct.run("fontView.setVisibleScripts", &params).unwrap();
    let stored = setting(&app, "fontView.visibleScripts");
    assert_eq!(stored, setting(&direct, "fontView.visibleScripts"));
    assert_eq!(stored, json!(["latin", "arabic"]));

    let revision = app.session.settings().revision();
    open(&mut app, json!({"dialog": "dialog.languagesList"}));
    set(&mut app, "scripts", json!(["greek"]));
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert_eq!((setting(&app, "fontView.visibleScripts"), app.session.settings().revision()), (stored, revision));
}

#[test]
fn missing_glyphs_shows_the_badge_and_generate_gives_the_font_its_command_gives() {
    let mut app = FontcraftApp::default();
    app.run("fontView.addListFilter", &json!({"name": "Wanted", "glyphs": ["A", "Q.ss01", "Z.alt"]})).unwrap();
    let w = Window::new();
    let shapes = w.frame(&mut app);
    let r = row(&app, "filters/Wanted").rect;
    let texts: Vec<String> = shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Text(t) if r.expand(4.0).contains_rect(s.visual_bounding_rect()) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect();
    assert!(texts.iter().any(|t| t == "1/3"), "present / expected: {texts:?}");

    let before = font(&app);
    let opened = open(&mut app, json!({"dialog": "dialog.missingGlyphs", "params": {"entry": "filters/Wanted"}}));
    assert_eq!(field(&opened, "glyphs")["value"], json!(["Q.ss01", "Z.alt"]), "every missing glyph starts selected");
    // A click selects one row, Cmd+A every row, Cmd+click takes one out.
    let selected = |app: &mut FontcraftApp| field(&call(app, "ui.dialog.inspect", json!({})), "glyphs")["value"].clone();
    for _ in 0..3 {
        w.frame(&mut app);
    }
    let row_at = |at: usize| w.ctx.read_response(crate::dialogs::form::row_id("glyphs", at)).unwrap().rect.center();
    w.click(&mut app, row_at(1), Modifiers::NONE);
    assert_eq!(selected(&mut app), json!(["Z.alt"]));
    let cmd_a = |pressed| Event::Key { key: Key::A, physical_key: None, pressed, repeat: false, modifiers: Modifiers::COMMAND };
    w.frame_with(&mut app, vec![cmd_a(true), cmd_a(false)], Modifiers::COMMAND);
    w.frame(&mut app);
    assert_eq!(selected(&mut app), json!(["Q.ss01", "Z.alt"]));
    w.click(&mut app, row_at(0), Modifiers::COMMAND);
    assert_eq!(selected(&mut app), json!(["Z.alt"]));
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert!(Arc::ptr_eq(&before, &font(&app)), "Cancel puts back the very font");

    open(&mut app, json!({"dialog": "dialog.missingGlyphs", "params": {"entry": "filters/Wanted"}}));
    set(&mut app, "glyphs", json!(["Z.alt"]));
    let undo = app.session.doc().unwrap().history.undo.len();
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    let mut direct = FontcraftApp::default();
    direct.run("fontView.addListFilter", &json!({"name": "Wanted", "glyphs": ["A", "Q.ss01", "Z.alt"]})).unwrap();
    direct.run("fontView.generateMissingGlyphs", &json!({"entry": "filters/Wanted", "glyphs": ["Z.alt"]})).unwrap();
    assert!(font(&app).same_as(&font(&direct)));
    assert!(font(&app).glyph("Q.ss01").is_none());
}

#[test]
fn a_global_guide_shows_only_in_the_glyphs_its_filter_takes_in() {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [400, 250]})).unwrap();
    app.run("guide.addGlobal", &json!({"x": 150, "y": 0, "angle": 90})).unwrap();
    let t = Tokens::dark();
    let w = Window::new();
    let drawn = |shapes: &[Shape]| shapes.iter().any(|s| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == t.guide_global));
    let filter = |app: &mut FontcraftApp, text: &str| {
        app.run("guide.setProperties", &json!({"glyph": "A", "guide": {"scope": "global", "index": 0}, "filter": text})).unwrap();
        w.frame(app)
    };
    assert!(!drawn(&filter(&mut app, "name == 'B'")), "left out of A");
    assert!(drawn(&filter(&mut app, "name == 'A' OR name == 'B'")));
    assert!(drawn(&filter(&mut app, "name ==")), "a filter that does not read leaves the guide shown");
    assert!(drawn(&filter(&mut app, "")), "no filter: every glyph");
}
