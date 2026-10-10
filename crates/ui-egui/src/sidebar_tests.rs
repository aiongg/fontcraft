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

    // A double-click on the filter opens its editor, which reads the stored name and rules.
    let w = Window::new();
    w.frame(&mut app);
    let at = row(&app, "filters/Vowels").rect.center();
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
