//! The sidebar's dialogs, filled over the control channel and by the person in the window: each gives the settings
//! or the font its command gives.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::host::Host;
use craft::ui::dialog::DialogHost;
use craft::ui::testing::Fixture;
use egui::{Key, Modifiers};
use fontcraft_engine::Session;
use fontcraft_engine::model::Font;
use serde_json::{Value, json};

use crate::app::{APP, FontCraftApp};
use crate::control;
use crate::dialogs::form;

fn app() -> FontCraftApp {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.session.execute("glyph.newGlyph", &json!({"name": "A"})).ok();
    app
}

fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
    control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
}

fn open(app: &mut FontCraftApp, params: Value) -> Value {
    form::open_command(app, &params).unwrap_or_else(|e| panic!("dialog.open {params}: {e}"))
}

fn set(app: &mut FontCraftApp, key: &str, value: Value) -> Value {
    call(app, "ui.dialog.set", json!({"field": key, "value": value}))
}

fn field(shown: &Value, key: &str) -> Value {
    shown["fields"].as_array().and_then(|l| l.iter().find(|f| f["id"] == key)).cloned().unwrap_or_else(|| panic!("no field {key}: {shown}"))
}

fn setting(app: &FontCraftApp, id: &str) -> Value {
    app.session.settings().get(id).unwrap()
}

fn font(app: &FontCraftApp) -> Arc<Font> {
    app.session.doc().unwrap().font.clone()
}

/// The app with its window's dialog host in a Fixture.
fn window(app: &Rc<RefCell<FontCraftApp>>) -> Fixture {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(app);
    let mut fixture = Fixture::new(move |ui| {
        let (mut app, mut host) = (shown.borrow_mut(), host.borrow_mut());
        super::super::run(&mut host, &mut app);
        host.run(ui);
    });
    fixture.settle();
    fixture
}

fn press(fixture: &mut Fixture, label: &str, mods: Modifiers) {
    let target = fixture.by_label(label).unwrap_or_else(|| panic!("no control {label}"));
    fixture.hold(mods).click(&target).hold(Modifiers::NONE).settle();
}

#[test]
fn the_smart_filter_editor_gives_the_settings_its_commands_give() {
    let mut app = app();
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

    let mut direct = self::app();
    let rules = json!({"all": [
        {"property": "glyphName", "op": "matches", "value": "[AEIOU]"},
        {"property": "countOfPaths", "op": "==", "value": 0},
    ]});
    direct.session.execute("fontView.addSmartFilter", &json!({"name": "Vowels", "rules": rules})).unwrap();
    assert_eq!(setting(&app, "fontView.filters"), setting(&direct, "fontView.filters"));

    // The editor of a stored filter reads its name and rules.
    let shown = open(&mut app, json!({"dialog": "dialog.smartFilterEditor", "params": {"filter": "filters/Vowels"}}));
    assert_eq!(shown["command"], "fontView.editFilter");
    assert_eq!(field(&shown, "name")["value"], "Vowels");
    assert_eq!(field(&shown, "rules")["value"], rules);
    let rules = json!({"any": [{"property": "glyphName", "op": "matches", "value": "[AEIOU]"}]});
    set(&mut app, "rules", rules.clone());
    call(&mut app, "ui.dialog.confirm", json!({}));
    direct.session.execute("fontView.editFilter", &json!({"filter": "filters/Vowels", "name": "Vowels", "rules": rules})).unwrap();
    assert_eq!(setting(&app, "fontView.filters"), setting(&direct, "fontView.filters"));

    // Cancel changes nothing; a filter that is not there opens nothing.
    let (before, revision) = (setting(&app, "fontView.filters"), app.session.settings().revision());
    open(&mut app, json!({"dialog": "dialog.smartFilterEditor", "params": {"filter": "filters/Vowels"}}));
    set(&mut app, "name", json!("Consonants"));
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert_eq!((setting(&app, "fontView.filters"), app.session.settings().revision()), (before, revision));
    assert!(form::open_command(&mut app, &json!({"dialog": "dialog.smartFilterEditor", "params": {"filter": "filters/Gone"}})).is_err());
    assert!(app.dialogs.open.is_none());
}

/// The person builds a nested group with the rule rows' buttons and types a rule's value.
#[test]
fn the_rule_rows_build_a_nested_group_as_the_command_takes_it() {
    let app = Rc::new(RefCell::new(app()));
    open(&mut app.borrow_mut(), json!({"command": "fontView.addSmartFilter"}));
    set(&mut app.borrow_mut(), "name", json!("Nested"));
    let mut fixture = window(&app);
    // … on the top group adds a nested Any group with one rule; − on the first rule removes it; + on the nested rule
    // adds a second rule after it.
    press(&mut fixture, "Add a nested group", Modifiers::NONE);
    press(&mut fixture, "Remove this rule", Modifiers::NONE);
    press(&mut fixture, "Add a rule after this one", Modifiers::NONE);
    let value = fixture.by_label("Value").expect("the first rule's value");
    fixture.click(&value).type_text("a.sc").key(Key::Tab, Modifiers::NONE).settle();

    let rule = |value: &str| json!({"property": "glyphName", "op": "contains", "value": value});
    let expected = json!({"all": [{"any": [rule("a.sc"), rule("")]}]});
    assert_eq!(field(&form::inspect(&app.borrow()), "rules")["value"], expected);
    call(&mut app.borrow_mut(), "ui.dialog.confirm", json!({}));
    let mut direct = self::app();
    direct.session.execute("fontView.addSmartFilter", &json!({"name": "Nested", "rules": expected})).unwrap();
    assert_eq!(setting(&app.borrow(), "fontView.filters"), setting(&direct, "fontView.filters"));
}

/// With Alt held, `+` on a group adds a group at its end, and on a rule a group after it.
#[test]
fn alt_on_a_plus_button_adds_a_nested_group() {
    let app = Rc::new(RefCell::new(app()));
    open(&mut app.borrow_mut(), json!({"command": "fontView.addSmartFilter"}));
    let mut fixture = window(&app);
    press(&mut fixture, "Add a rule", Modifiers::ALT);
    press(&mut fixture, "Add a rule after this one", Modifiers::ALT);
    let starter = json!({"property": "glyphName", "op": "contains", "value": ""});
    let expected = json!({"all": [starter, {"any": [starter]}, {"any": [starter]}]});
    assert_eq!(field(&form::inspect(&app.borrow()), "rules")["value"], expected);
}

#[test]
fn the_list_filter_editor_gives_the_settings_its_commands_give() {
    let mut app = app();
    // A new list filter opens with the names it is given (the sidebar gives the selected glyphs).
    let opened = open(&mut app, json!({"command": "fontView.addListFilter", "params": {"glyphs": "A B"}}));
    assert_eq!(opened["dialog"], "dialog.listFilterEditor");
    assert_eq!(field(&opened, "name")["value"], "New List");
    assert_eq!(field(&opened, "glyphs")["value"], "A B");
    set(&mut app, "name", json!("Pair"));
    call(&mut app, "ui.dialog.confirm", json!({}));
    let mut direct = self::app();
    direct.session.execute("fontView.addListFilter", &json!({"name": "Pair", "glyphs": ["A", "B"]})).unwrap();
    assert_eq!(setting(&app, "fontView.filters"), setting(&direct, "fontView.filters"));

    // The editor of a stored filter reads it by its path.
    let opened = open(&mut app, json!({"dialog": "dialog.listFilterEditor", "params": {"filter": "filters/Pair"}}));
    assert_eq!((field(&opened, "name")["value"].clone(), field(&opened, "glyphs")["value"].clone()), (json!("Pair"), json!("A B")));
    set(&mut app, "glyphs", json!("C\nA  Q.ss01"));
    call(&mut app, "ui.dialog.confirm", json!({}));
    direct.session.execute("fontView.editFilter", &json!({"filter": "filters/Pair", "glyphs": ["C", "A", "Q.ss01"]})).unwrap();
    assert_eq!(setting(&app, "fontView.filters"), setting(&direct, "fontView.filters"));

    let (before, revision) = (setting(&app, "fontView.filters"), app.session.settings().revision());
    open(&mut app, json!({"dialog": "dialog.listFilterEditor", "params": {"filter": "filters/Pair"}}));
    set(&mut app, "glyphs", json!("Z"));
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert_eq!((setting(&app, "fontView.filters"), app.session.settings().revision()), (before, revision));
    assert!(form::open_command(&mut app, &json!({"dialog": "dialog.listFilterEditor", "params": {"filter": "filters/Gone"}})).is_err());
}

/// Every missing glyph starts selected; clicks, Shift-clicks and Cmd-clicks in the list pick rows; Cancel puts back the very font, and Generate is
/// one undo step that adds the selected glyphs as the command does.
#[test]
fn missing_glyphs_selects_rows_and_generate_gives_the_font_its_command_gives() {
    let app = Rc::new(RefCell::new(app()));
    let wanted = json!({"name": "Wanted", "glyphs": ["A", "Q.ss01", "Z.alt"]});
    app.borrow_mut().session.execute("fontView.addListFilter", &wanted).unwrap();
    let before = font(&app.borrow());
    let opened = open(&mut app.borrow_mut(), json!({"dialog": "dialog.missingGlyphs", "params": {"entry": "filters/Wanted"}}));
    assert_eq!(field(&opened, "glyphs")["value"], json!(["Q.ss01", "Z.alt"]), "every missing glyph starts selected");

    let mut fixture = window(&app);
    let selected = |app: &Rc<RefCell<FontCraftApp>>| field(&form::inspect(&app.borrow()), "glyphs")["value"].clone();
    press(&mut fixture, "Z.alt", Modifiers::NONE);
    assert_eq!(selected(&app), json!(["Z.alt"]), "a click selects one row");
    press(&mut fixture, "Q.ss01", Modifiers::SHIFT);
    assert_eq!(selected(&app), json!(["Q.ss01", "Z.alt"]), "Shift-click selects the rows from the last one clicked");
    press(&mut fixture, "Q.ss01", Modifiers::COMMAND);
    assert_eq!(selected(&app), json!(["Z.alt"]), "Cmd-click takes one out");
    call(&mut app.borrow_mut(), "ui.dialog.cancel", json!({}));
    assert!(Arc::ptr_eq(&before, &font(&app.borrow())), "Cancel puts back the very font");

    let mut app = app.borrow_mut();
    open(&mut app, json!({"dialog": "dialog.missingGlyphs", "params": {"entry": "filters/Wanted"}}));
    set(&mut app, "glyphs", json!(["Z.alt"]));
    let undo = app.session.doc().unwrap().history.undo.len();
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    let mut direct = self::app();
    direct.session.execute("fontView.addListFilter", &wanted).unwrap();
    direct.session.execute("fontView.generateMissingGlyphs", &json!({"entry": "filters/Wanted", "glyphs": ["Z.alt"]})).unwrap();
    assert!(font(&app).same_as(&font(&direct)));
    assert!(font(&app).glyph("Q.ss01").is_none());
}
