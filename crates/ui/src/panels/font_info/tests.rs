//! Behaviour tests of the Font Info panel, its parameter editors and its dialogs.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;

use craft::host::Host;
use craft::ui::contract::Tracked;
use craft::ui::dialog::{DialogResult, DialogStack, StackDialog};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use egui::{Key, Modifiers};
use fontcraft_engine::Session;
use serde_json::json;

use super::{Props, param_value, show};
use crate::app::models::font_info::{Editor, FieldWarning, FontInfoModel, RowModel, RowSort};
use crate::app::{APP, FontCraftApp, Msg};
use crate::dialogs::os2_bits::{BitSet, Os2BitsDialog, toggle};
use crate::shell::dock::PanelId;
use crate::shell::font_info::{FontInfoTab, Target};

/// The room the panel is shown in: a window, so every row of the Font tab fits.
const ROOM: egui::Vec2 = egui::vec2(1120.0, 720.0);

/// A fixture showing the panel over `model`, and the messages it posted.
fn panel(model: FontInfoModel) -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
    let model = Tracked::new(model);
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.room(ROOM).settle();
    (fixture, posted)
}

fn font_model(params: Vec<RowModel>) -> FontInfoModel {
    let mut model = FontInfoModel { open: true, ..FontInfoModel::default() };
    model.font.family_name = "Sample".into();
    model.font.units_per_em = 1000;
    model.font.params = params;
    model.masters = vec![("m01".into(), "Regular".into()), ("m02".into(), "Bold".into())];
    model
}

fn param(key: &str, editor: Editor) -> RowModel {
    RowModel { sort: RowSort::Param, key: key.into(), label: key.into(), enabled: true, removable: true, editor, ..RowModel::default() }
}

fn set_param(name: &str, value: serde_json::Value) -> Msg {
    Msg::run("font.customParameters.set", json!({ "scope": "font", "name": name, "index": 0, "value": value }))
}

/// A click on a tab posts `font.info.selectTab` with its name.
#[test]
fn a_tab_click_selects_the_tab() {
    let (mut fixture, posted) = panel(font_model(Vec::new()));
    let notes = fixture.by_label("Notes").expect("the Notes tab");
    fixture.click(&notes).settle();
    assert_eq!(*posted.borrow(), vec![Msg::Shell(CommandRef::with_params("font.info.selectTab", json!({ "tab": "notes" })))]);
    assert!(fixture.by_label("Close Notes").is_none(), "a tab of Font Info has no close button");
}

/// The family name field sets the field, the stepper steps the version, and the warnings `font.info.get` gives show
/// under their own fields.
#[test]
fn font_tab_fields_post_their_commands() {
    let mut model = font_model(Vec::new());
    model.font.family_name = "Sämple".into();
    model.font.warnings = vec![
        FieldWarning { field: "familyName".into(), message: "the family name should hold only ASCII letters".into() },
        FieldWarning { field: "unitsPerEm".into(), message: "some applications have trouble with more than 3000 units per em".into() },
    ];
    let (mut fixture, posted) = panel(model);
    let texts = fixture.texts();
    let at = |start: &str| texts.iter().find(|t| t.text.starts_with(start)).map(|t| t.rect.top());
    let family = at("The family name should hold").expect("the family name warning shows, sentence-cased");
    let upm = at("Some applications have trouble").expect("the units per em warning shows");
    assert!(family < upm, "each warning shows under its own field");
    let family = fixture.by_label("Family Name").expect("the Family Name field");
    fixture.click(&family).key(Key::A, Modifiers::COMMAND).type_text("Other").key(Key::Enter, Modifiers::NONE).settle();
    let raise = fixture.by_label("Raise the minor version by one").expect("the stepper");
    fixture.click(&raise).settle();
    assert_eq!(
        *posted.borrow(),
        vec![
            Msg::run("font.info.set", json!({ "scope": "font", "values": { "familyName": "Other" } })),
            Msg::run("font.info.stepVersion", json!({ "delta": 1 })),
        ]
    );
}

/// The menu of a parameter's values sets Post Table Type to 3, as a number.
#[test]
fn the_choice_menu_sets_post_table_type() {
    let row = param("Post Table Type", Editor::Choice { choices: vec!["2".into(), "3".into()], value: "2".into() });
    let (mut fixture, posted) = panel(font_model(vec![row]));
    let menu = fixture.by_label("Post Table Type").expect("the menu");
    fixture.click(&menu).settle().key(Key::ArrowDown, Modifiers::NONE).key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*posted.borrow(), vec![set_param("Post Table Type", json!(3))]);
}

/// The menu of masters sets Variable Font Origin to the picked master's id.
#[test]
fn the_master_menu_sets_variable_font_origin() {
    let row = param("Variable Font Origin", Editor::Master("m01".into()));
    let (mut fixture, posted) = panel(font_model(vec![row]));
    let menu = fixture.by_label("Variable Font Origin").expect("the menu");
    fixture.click(&menu).settle().key(Key::ArrowDown, Modifiers::NONE).key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*posted.borrow(), vec![set_param("Variable Font Origin", json!("m02"))]);
}

/// A value the command would refuse posts nothing and shows the command's message under the row.
#[test]
fn a_refused_value_shows_its_message() {
    let name = "Elidable STAT Axis Value Name";
    let message = param_value::refusal(name, &json!("much too long")).expect("the check refuses the text");
    let (mut fixture, posted) = panel(font_model(vec![param(name, Editor::Text(String::new()))]));
    let field = fixture.by_label(name).expect("the field");
    fixture.click(&field).type_text("much too long").key(Key::Enter, Modifiers::NONE).settle();
    assert!(posted.borrow().is_empty(), "{:?}", posted.borrow());
    let first: String = message.chars().take(12).collect();
    assert!(fixture.texts().iter().any(|t| t.text.contains(&first)), "`{message}` shows");
}

/// The check boxes of a multiple-choice parameter post the whole list.
#[test]
fn choice_boxes_post_the_checked_list() {
    let choices = ["TTF", "EOT", "WOFF", "WOFF2"].map(String::from).to_vec();
    let row = param("Webfont Formats", Editor::Choices { choices, on: vec!["WOFF".into()] });
    let (mut fixture, posted) = panel(font_model(vec![row]));
    let woff2 = fixture.by_label("WOFF2").expect("the WOFF2 box");
    fixture.click(&woff2).settle();
    assert_eq!(*posted.borrow(), vec![set_param("Webfont Formats", json!(["WOFF", "WOFF2"]))]);
}

/// Other Settings: a check box posts `font.settings.set` with its key.
#[test]
fn other_settings_boxes_post_settings_set() {
    let mut model = font_model(Vec::new());
    model.tab = FontInfoTab::Other;
    let (mut fixture, posted) = panel(model);
    let naming = fixture.by_label("Use Custom Naming").expect("the box");
    fixture.click(&naming).settle();
    assert_eq!(*posted.borrow(), vec![Msg::run("font.settings.set", json!({ "useCustomNaming": true }))]);
}

/// A click on a row's label selects the row and puts the keyboard nowhere, so the edit commands go to the selected
/// rows; a click into a field takes them back. Cmd-click toggles, Shift-click adds the rows from the last selected one.
#[test]
fn a_rows_label_selects_it_and_the_rows_take_the_edit_commands() {
    use super::rows::takes_edits;
    use crate::app::models::font_info::RowMark;
    use crate::shell::font_info::{RowKey, select_rows};
    let text = |key: &str| param(key, Editor::Text(String::new()));
    let mut rows = vec![text("vendorID"), text("note"), text("trademark")];
    rows[0].mark = RowMark::Lead;
    let (mut fixture, posted) = panel(font_model(rows));
    let key = |key: &str| RowKey { param: true, key: key.into(), index: 0 };
    let click = |fixture: &mut Fixture, label: &str, held: Modifiers| {
        let at = fixture.texts().iter().find(|t| t.text.starts_with(label)).expect(label).rect.center();
        fixture.hold(held).click_at(at).hold(Modifiers::NONE).settle();
    };
    click(&mut fixture, "note", Modifiers::NONE);
    assert!(takes_edits(fixture.ctx()), "a selected row shows and nothing has the keyboard");
    click(&mut fixture, "note", Modifiers::COMMAND);
    click(&mut fixture, "trademark", Modifiers::SHIFT);
    let wanted = [
        select_rows(fontcraft_engine::fontinfo::Scope::Font, &[key("note")], "set"),
        select_rows(fontcraft_engine::fontinfo::Scope::Font, &[key("note")], "toggle"),
        select_rows(fontcraft_engine::fontinfo::Scope::Font, &[key("vendorID"), key("note"), key("trademark")], "add"),
    ];
    let wanted: Vec<Msg> = wanted.into_iter().map(Msg::Shell).collect();
    assert_eq!(*posted.borrow(), wanted);
    let field = fixture.by_label("trademark").expect("the field");
    fixture.click(&field).settle();
    assert!(!takes_edits(fixture.ctx()), "the focused field keeps its own Copy");
}

/// A localizable field shows a plus per language, and the minus of each while Alt is held.
#[test]
fn alt_reveals_the_minus_of_a_language() {
    let mut model = font_model(Vec::new());
    let languages = vec![("dflt".to_string(), "Sample".to_string()), ("DEU".into(), "Muster".into())];
    model.font.fields = vec![RowModel {
        key: "familyNames".into(),
        label: "Family Names".into(),
        enabled: true,
        removable: true,
        editor: Editor::Languages(languages),
        ..RowModel::default()
    }];
    let (mut fixture, posted) = panel(model);
    assert!(fixture.by_label("Remove this language").is_none());
    let plus = fixture.by_label("Add a language").expect("a plus");
    fixture.click(&plus).settle();
    fixture.hold(Modifiers::ALT).settle();
    let minus = fixture.by_label("Remove this language").expect("the minus while Alt is held");
    assert!(fixture.by_label("Add a language").is_none());
    fixture.click(&minus).hold(Modifiers::NONE).settle();
    let got = posted.borrow().clone();
    let adds = |m: &Msg| m.command_id() == Some("font.info.setLocalizedValue");
    assert!(got.iter().any(adds), "{got:?}");
    let removal = json!({ "scope": "font", "key": "familyNames", "language": "dflt" });
    assert!(got.contains(&Msg::run("font.info.removeLocalizedValue", removal)), "{got:?}");
}

/// Notes: the typed note is committed when the field loses the keyboard.
#[test]
fn the_note_commits_when_the_field_loses_the_keyboard() {
    let mut model = font_model(Vec::new());
    model.tab = FontInfoTab::Notes;
    let (mut fixture, posted) = panel(model);
    let note = fixture.by_label("Font Note").expect("the note field");
    fixture.click(&note).type_text("Draft").key(Key::Tab, Modifiers::NONE).settle();
    assert_eq!(*posted.borrow(), vec![Msg::run("font.note.set", json!({ "text": "Draft" }))]);
}

/// The bits dialog: a search narrows the boxes, a box and OK answer the checked bits; an fsType usage level clears the
/// other levels; Cancel answers nothing.
#[test]
fn the_bits_dialog_answers_ok_with_the_checked_bits() {
    let mut levels = BTreeSet::from([2, 8]);
    toggle(BitSet::FsType, &mut levels, 3, true);
    assert_eq!(levels, BTreeSet::from([3, 8]));

    let answered = |press: &'static str| {
        let dialog = Rc::new(RefCell::new(Os2BitsDialog::new(Target::font(), "fsType".into(), 0, BitSet::FsType, &[8])));
        let result = Rc::new(RefCell::new(None));
        let (held, out) = (Rc::clone(&dialog), Rc::clone(&result));
        let mut fixture = Fixture::new(move |ui| {
            if let Some(r) = held.borrow_mut().show(ui, &DialogStack::default()) {
                *out.borrow_mut() = Some(r);
            }
        });
        fixture.settle();
        let search = fixture.by_label("Search").expect("the search field");
        fixture.click(&search).type_text("edit").settle();
        let editable = fixture.by_label("Editable embedding").expect("the box");
        fixture.click(&editable).settle();
        let button = fixture.by_label(press).expect("the button");
        fixture.click(&button).settle();
        result.borrow_mut().take()
    };
    match answered("OK") {
        Some(DialogResult::Ok((_, name, index, values))) => assert_eq!((name.as_str(), index, values), ("fsType", 0, vec![3, 8])),
        other => panic!("OK answered {other:?}"),
    }
    assert!(matches!(answered("Cancel"), Some(DialogResult::Cancel)));
}

/// `font.info` shows the panel on the tab asked for, and the view model follows it.
#[test]
fn font_info_shows_the_panel_on_a_tab() {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::Shell(CommandRef::with_params("font.info", json!({ "tab": "other" }))));
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
    assert!(app.models.font_info.open);
    assert_eq!(app.models.font_info.tab, FontInfoTab::Other);
    assert_ne!(app.view.workspace.layout().panel_state(&PanelId::FontInfo), craft::behavior::dock::PanelState::Hidden);
    assert!(app.models.font_info.other.numbers.iter().any(|(k, _)| k == "gridSpacing"));
}
