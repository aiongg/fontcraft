//! The Font Info window: the commands that open it and switch tabs, the shared rows (selection,
//! the minus button, Alt on localizable rows), Copy, Paste and Cmd+Delete on the selected rows,
//! and the two pickers, whose Add gives the font the command gives.

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use fontcraft_engine::fontinfo::Scope;
use fontcraft_engine::fontinfo::rows::RowKind;
use serde_json::{Value, json};

use super::rows::{self, Target, plus_id, row_id};
use super::{Picker, Tab};
use crate::FontcraftApp;
use crate::dialogs::custom_parameter_picker::{self, ParameterPicker};
use crate::dialogs::field_picker::{self, FieldPicker};
use crate::panels::Pending;

fn input(events: Vec<Event>) -> egui::RawInput {
    egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() }
}

/// The whole app, one egui context kept across frames.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) {
        let mut output = self.ctx.run_ui(input(events), |ui| app.ui(ui));
        output.textures_delta.clear();
    }

    fn click(&self, app: &mut FontcraftApp, id: egui::Id) {
        let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} is drawn")).rect.center();
        self.frame(app, vec![Event::PointerMoved(at)]);
        self.frame(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        self.frame(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    }
}

/// Only the General and Custom Parameters sections of the font, drawn in a context of their own,
/// as a tab draws them.
struct Sections {
    ctx: egui::Context,
}

impl Sections {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) -> Pending {
        let mut pending = Vec::new();
        let mut output = self.ctx.run_ui(input(events), |ui| {
            let FontcraftApp { session, state, tokens, .. } = &mut *app;
            let Ok(font) = session.font() else { return };
            let fi = &mut state.panels.font_info;
            fi.shown_rows.clear();
            rows::general_section(ui, tokens, fi, font, &Target::font(), &mut pending);
            rows::parameters_section(ui, tokens, fi, font, &Target::font(), &mut pending);
        });
        output.textures_delta.clear();
        pending
    }

    fn click(&self, app: &mut FontcraftApp, id: egui::Id, modifiers: Modifiers) -> Pending {
        self.frame(app, Vec::new());
        let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} is drawn")).rect.center();
        self.frame(app, vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(at)]);
        self.frame(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers }]);
        let pending = self.frame(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers }]);
        self.frame(app, vec![Event::ModifiersChanged(Modifiers::NONE)]);
        pending
    }
}

/// A font with an added field (`designers`) and two custom parameters.
fn info_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("font.info.addField", &json!({"scope": "font", "key": "designers", "value": "Ann"})).unwrap();
    app.run("font.customParameters.add", &json!({"scope": "font", "name": "typoAscender", "value": 800})).unwrap();
    app.run("font.customParameters.add", &json!({"scope": "font", "name": "underlineThickness", "value": 50})).unwrap();
    app
}

fn param_names(app: &FontcraftApp) -> Vec<String> {
    app.session.font().unwrap().info.params.0.iter().map(|p| p.name.clone()).collect()
}

fn has_field(app: &FontcraftApp, key: &str) -> bool {
    app.session.font().unwrap().info.properties.0.iter().any(|p| p.key == key)
}

fn field(key: &str) -> egui::Id {
    row_id(Scope::Font, RowKind::Field, key, 0)
}

fn param(name: &str) -> egui::Id {
    row_id(Scope::Font, RowKind::Param, name, 0)
}

#[test]
fn font_info_opens_on_a_tab_and_the_tab_bar_switches_tabs() {
    let mut app = info_app();
    let w = Window::new();
    let r = app.run("font.info", &json!({"tab": "masters"})).unwrap();
    assert_eq!(r, json!({"open": true, "tab": "masters"}));
    assert!(app.run("font.info.selectTab", &json!({"tab": "kerning"})).is_err(), "an unknown tab is refused");
    assert!(app.run("font.info.selectTab", &json!({})).is_err(), "selectTab needs a tab");
    assert_eq!(app.state.panels.font_info.tab, Tab::Masters, "a refused command changes nothing");
    w.frame(&mut app, Vec::new());
    w.frame(&mut app, Vec::new());
    for tab in Tab::ALL {
        assert!(w.ctx.read_response(super::tab_id(tab)).is_some(), "the {} tab is in the bar", tab.label());
    }
    w.click(&mut app, super::tab_id(Tab::Features));
    assert_eq!(app.state.panels.font_info.tab, Tab::Features, "a click on a tab shows it");
    // Opening without a tab keeps the tab shown last.
    app.state.panels.font_info.open = false;
    app.run("font.info", &json!({})).unwrap();
    assert_eq!((app.state.panels.font_info.open, app.state.panels.font_info.tab), (true, Tab::Features));
    // With no document the window closes.
    app.run("file.close", &json!({"discard": true})).unwrap();
    w.frame(&mut app, Vec::new());
    assert!(!app.state.panels.font_info.open);
    assert!(app.run("font.info", &json!({})).is_err(), "Font Info needs a font");
}

#[test]
fn rows_select_with_click_cmd_click_and_shift_click_and_minus_removes_through_a_command() {
    let mut app = info_app();
    let s = Sections::new();
    s.frame(&mut app, Vec::new());
    let shown: Vec<String> = app.state.panels.font_info.shown_rows.iter().map(|r| r.key.clone()).collect();
    assert_eq!(shown, ["designers", "typoAscender", "underlineThickness"], "fields first, then parameters, in stored order");
    s.click(&mut app, field("designers"), Modifiers::NONE);
    assert_eq!(app.state.panels.font_info.selected_rows, [0]);
    s.click(&mut app, param("underlineThickness"), Modifiers::COMMAND);
    assert_eq!(app.state.panels.font_info.selected_rows, [0, 2], "Cmd-click adds a row");
    s.click(&mut app, param("underlineThickness"), Modifiers::COMMAND);
    assert_eq!(app.state.panels.font_info.selected_rows, [0], "Cmd-click on a selected row drops it");
    s.click(&mut app, field("designers"), Modifiers::NONE);
    s.click(&mut app, param("underlineThickness"), Modifiers::SHIFT);
    assert_eq!(app.state.panels.font_info.selected_rows, [0, 1, 2], "Shift-click selects the range");
    s.click(&mut app, param("typoAscender"), Modifiers::NONE);
    assert_eq!(app.state.panels.font_info.selected_rows, [1], "a plain click selects one row");
    // The minus button reports; the command it queues removes the row.
    let pending = s.click(&mut app, param("typoAscender").with("minus"), Modifiers::NONE);
    assert_eq!(pending, vec![("font.customParameters.remove", json!({"scope": "font", "name": "typoAscender", "index": 0}))]);
    for (id, params) in pending {
        app.run(id, &params).unwrap();
    }
    assert_eq!(param_names(&app), ["underlineThickness"]);
    // The plus buttons beside the headings open the pickers.
    s.click(&mut app, plus_id(Scope::Font, "general"), Modifiers::NONE);
    assert!(matches!(app.state.panels.font_info.picker, Some(Picker::Field(_))));
    s.click(&mut app, plus_id(Scope::Font, "parameters"), Modifiers::NONE);
    assert!(matches!(app.state.panels.font_info.picker, Some(Picker::Parameter(_))));
}

#[test]
fn alt_turns_a_localizable_rows_plus_into_its_minus() {
    let mut app = info_app();
    let s = Sections::new();
    s.frame(&mut app, Vec::new());
    let plus = field("designers").with(("plus", 0));
    let minus = field("designers").with(("minus", 0));
    assert!(s.ctx.read_response(plus).is_some() && s.ctx.read_response(minus).is_none(), "the plus shows without Alt");
    // The plus adds the first language the field lacks.
    let pending = s.click(&mut app, plus, Modifiers::NONE);
    assert_eq!(pending, vec![("font.info.setLocalizedValue", json!({"scope": "font", "key": "designers", "language": "AFK", "value": ""}))]);
    s.frame(&mut app, vec![Event::ModifiersChanged(Modifiers::ALT)]);
    s.frame(&mut app, Vec::new());
    assert!(s.ctx.read_response(minus).is_some() && s.ctx.read_response(plus).is_none(), "Alt shows the minus");
    let pending = s.click(&mut app, minus, Modifiers::ALT);
    assert_eq!(pending, vec![("font.info.removeLocalizedValue", json!({"scope": "font", "key": "designers", "language": "dflt"}))]);
}

#[test]
fn cmd_delete_removes_two_selected_rows() {
    let mut app = info_app();
    let s = Sections::new();
    s.click(&mut app, field("designers"), Modifiers::NONE);
    s.click(&mut app, param("underlineThickness"), Modifiers::COMMAND);
    assert_eq!(app.state.panels.font_info.selected_rows, [0, 2]);
    let (id, _) = crate::shell::keymap::command_for(Key::Delete, Modifiers::COMMAND).expect("Cmd+Delete is a shortcut");
    // While the window does not have the keyboard, the shortcut is not the window's.
    app.state.panels.font_info.open = true;
    assert!(super::menu_command(&mut app, id).is_none());
    app.state.panels.font_info.keys = true;
    let r = app.run_menu_command(id, json!({})).unwrap();
    assert_eq!(r, json!({"removed": 2}));
    assert!(!has_field(&app, "designers"));
    assert_eq!(param_names(&app), ["typoAscender"], "the unselected row stays");
    assert!(app.state.panels.font_info.selected_rows.is_empty());
    // Each removal is a command of its own: undo brings the parameter back, then the field.
    app.run("edit.undo", &json!({})).unwrap();
    assert_eq!(param_names(&app), ["typoAscender", "underlineThickness"]);
}

#[test]
fn copy_puts_the_selected_rows_on_the_clipboard_and_paste_adds_them_to_another_font() {
    let mut app = info_app();
    let s = Sections::new();
    s.click(&mut app, field("designers"), Modifiers::NONE);
    s.click(&mut app, param("typoAscender"), Modifiers::COMMAND);
    app.state.panels.font_info.open = true;
    app.state.panels.font_info.keys = true;
    let r = app.run_menu_command("edit.copy", json!({})).unwrap();
    assert_eq!(r, json!({"rows": 2}));
    let text = app.clipboard_out.take().expect("the rows are on the clipboard");
    // Paste into another document's Font tab, as the platform hands the text over.
    let mut other = FontcraftApp::default();
    let w = Window::new();
    other.run("font.info", &json!({"tab": "font"})).unwrap();
    w.frame(&mut other, Vec::new());
    other.state.panels.font_info.keys = true;
    w.frame(&mut other, vec![Event::Paste(text)]);
    assert!(has_field(&other, "designers"));
    assert_eq!(param_names(&other), ["typoAscender"]);
}

/// Opens `picker`, lets the window fill its list, clicks `item` and Add.
fn pick(app: &mut FontcraftApp, picker: Picker, item: egui::Id, add: egui::Id) {
    let w = Window::new();
    app.run("font.info", &json!({"tab": "font"})).unwrap();
    app.state.panels.font_info.picker = Some(picker);
    w.frame(app, Vec::new());
    w.frame(app, Vec::new());
    w.click(app, item);
    w.click(app, add);
    assert!(app.state.panels.font_info.picker.is_none(), "Add closes the picker");
}

#[test]
fn the_field_picker_offers_missing_fields_and_adds_what_add_field_adds() {
    let mut app = info_app();
    let mut listing = FieldPicker::new(Target::font());
    listing.fill(&app.run("font.info.listFields", &listing.query()).unwrap());
    let offered: Vec<String> = listing.choices.unwrap().into_iter().map(|c| c.key).collect();
    assert!(!offered.contains(&"designers".to_string()), "a field the font has is not offered");
    assert!(offered.contains(&"copyrights".to_string()));
    pick(&mut app, Picker::Field(FieldPicker::new(Target::font())), field_picker::item_id("copyrights"), field_picker::add_id());
    let mut direct = info_app();
    direct.run("font.info.addField", &json!({"scope": "font", "key": "copyrights"})).unwrap();
    let props = |a: &FontcraftApp| format!("{:?}", a.session.font().unwrap().info.properties);
    assert_eq!(props(&app), props(&direct), "the picker gives the font the command gives");
    // Cancel changes nothing.
    let before = props(&app);
    let w = Window::new();
    app.state.panels.font_info.picker = Some(Picker::Field(FieldPicker::new(Target::font())));
    w.frame(&mut app, Vec::new());
    w.frame(&mut app, Vec::new());
    w.click(&mut app, field_picker::item_id("licenses"));
    w.click(&mut app, field_picker::cancel_id());
    assert!(app.state.panels.font_info.picker.is_none());
    assert_eq!(props(&app), before);
}

#[test]
fn the_parameter_picker_searches_and_adds_what_add_parameter_adds() {
    let mut app = info_app();
    let mut picker = ParameterPicker::new(Target::font());
    picker.query = "underlineposition".into();
    picker.fill(&app.run("font.customParameters.describe", &picker.query_params()).unwrap());
    let names: Vec<&str> = picker.choices.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"underlinePosition"), "the search ignores case: {names:?}");
    let mut sorted = names.clone();
    sorted.sort_by_key(|n| n.to_lowercase());
    assert_eq!(names, sorted, "the list is in alphabetical order");
    pick(&mut app, Picker::Parameter(picker), custom_parameter_picker::item_id("underlinePosition"), custom_parameter_picker::add_id());
    let mut direct = info_app();
    direct.run("font.customParameters.add", &json!({"scope": "font", "name": "underlinePosition"})).unwrap();
    let params = |a: &FontcraftApp| format!("{:?}", a.session.font().unwrap().info.params);
    assert_eq!(params(&app), params(&direct), "the picker gives the font the command gives");
    assert_eq!(param_names(&app).last().map(String::as_str), Some("underlinePosition"));
}

#[test]
fn param_value_commands_carry_the_kind_and_the_row() {
    let target = Target { scope: Scope::Master, indices: vec![0, 2] };
    let row = fontcraft_engine::fontinfo::rows::Row {
        scope: Scope::Master,
        kind: RowKind::Param,
        key: "Filter".into(),
        value: fontcraft_model::plist::Value::String(fontcraft_model::plist::Str::new("a")),
        enabled: true,
    };
    let edit = rows::RowAction::Edit(fontcraft_model::plist::Value::Bool(true));
    let commands = rows::action_commands(&target, &row, 1, &edit);
    let expected: Value = json!({"scope": "master", "targets": [0, 2], "name": "Filter", "index": 1, "value": true});
    assert_eq!(commands, vec![("font.customParameters.set", expected)]);
    assert_eq!(rows::param_kind("typoAscender", &row.value), fontcraft_engine::fontinfo::ParamKind::Number);
}
