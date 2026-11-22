//! The value dialogs of Font Info rows in the window's dialog host: what OK and Cancel post, and the font they give
//! next to the one `font.customParameters.set` gives with the same value.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::host::Host;
use craft::ui::dialog::DialogHost;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use egui::{Key, Modifiers};
use fontcraft_engine::Session;
use fontcraft_engine::fontinfo::Scope;
use serde_json::{Value, json};

use super::param_value::Edit;
use super::rows::{self, RowAction};
use crate::app::models::font_info::{Editor, RowDialog, RowModel, RowSort};
use crate::app::{APP, FontCraftApp, Msg};
use crate::dialogs::os2_bits::{numbers, stored_value};
use crate::dialogs::tt::Ask;
use crate::dialogs::tt::zones::Kind;
use crate::shell::font_info::{InfoDialog, Target, open_params};

const ROOM: egui::Vec2 = egui::vec2(1120.0, 900.0);
const SET: &str = "font.customParameters.set";

/// A document whose font holds the parameter `name` with `value`.
fn app_with(name: &str, value: Value) -> FontCraftApp {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.session.execute("font.customParameters.add", &json!({"scope": "font", "name": name, "value": value})).expect("the parameter is added");
    app
}

/// The app with its window's dialog host in a Fixture, after the row asked for `dialog`.
fn window(app: &Rc<RefCell<FontCraftApp>>, dialog: &InfoDialog) -> Fixture {
    {
        let mut app = app.borrow_mut();
        app.sent.push(Msg::Shell(CommandRef::with_params("font.info.openDialog", open_params(dialog))));
        app.drain();
        assert!(app.status.is_empty(), "{}", app.status);
    }
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(app);
    let mut fixture = Fixture::new(move |ui| {
        let (mut app, mut host) = (shown.borrow_mut(), host.borrow_mut());
        crate::dialogs::run(&mut host, &mut app);
        host.run(ui);
    });
    fixture.room(ROOM).settle();
    fixture
}

fn press(fixture: &mut Fixture, label: &str) {
    let target = fixture.by_label(label).unwrap_or_else(|| panic!("no control {label}"));
    fixture.click(&target).settle();
}

/// The `font.customParameters.set` messages the window posted.
fn sets(app: &Rc<RefCell<FontCraftApp>>) -> Vec<Msg> {
    app.borrow().sent.iter().filter(|m| m.command_id() == Some(SET)).cloned().collect()
}

/// After the posted messages ran, the font's `name` value equals the one the command gives `direct` with `value`.
fn same_as_the_command(app: &Rc<RefCell<FontCraftApp>>, mut direct: FontCraftApp, name: &str, value: Value) {
    let mut app = app.borrow_mut();
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
    direct.session.execute(SET, &json!({"scope": "font", "name": name, "index": 0, "value": value})).expect("the command takes the value");
    let read = |app: &FontCraftApp| stored_value(app, &Target::font(), name, 0).map(|v| numbers(&v));
    assert_eq!(read(&*app), read(&direct));
}

/// The bits dialog: a search, a box and OK post one `set` with the checked bits; Cancel posts nothing.
#[test]
fn the_bits_dialog_posts_one_set_and_cancel_posts_nothing() {
    let dialog = InfoDialog::Bits { target: Target::font(), name: "fsType".into(), index: 0 };
    for (button, posted) in [("OK", Some(json!([3, 8]))), ("Cancel", None)] {
        let app = Rc::new(RefCell::new(app_with("fsType", json!([8]))));
        let mut fixture = window(&app, &dialog);
        let search = fixture.by_label("Search").expect("the search field");
        fixture.click(&search).type_text("edit").settle();
        press(&mut fixture, "Editable embedding");
        press(&mut fixture, button);
        let expected: Vec<Msg> = posted.iter().map(|v| Msg::run(SET, json!({"scope": "font", "name": "fsType", "index": 0, "value": v}))).collect();
        assert_eq!(sets(&app), expected, "{button}");
        if let Some(value) = posted {
            same_as_the_command(&app, app_with("fsType", json!([8])), "fsType", value);
        }
    }
}

/// The PANOSE dialog: a pick in the Weight menu and OK post one `set` with the ten digits.
#[test]
fn the_panose_dialog_posts_the_ten_digits() {
    let stored = json!([2, 0, 5, 0, 0, 0, 0, 0, 0, 0]);
    let app = Rc::new(RefCell::new(app_with("panose", stored.clone())));
    let mut fixture = window(&app, &InfoDialog::Panose { target: Target::font(), index: 0 });
    let weight = fixture.by_label("Weight").expect("the Weight menu");
    fixture.click(&weight).settle().key(Key::ArrowDown, Modifiers::NONE).key(Key::Enter, Modifiers::NONE).settle();
    press(&mut fixture, "OK");
    let digits = json!([2, 0, 6, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(sets(&app), vec![Msg::run(SET, json!({"scope": "font", "name": "panose", "index": 0, "value": digits}))]);
    same_as_the_command(&app, app_with("panose", stored), "panose", digits);
}

/// A master's TTFStems and TTFZones rows open their editors on that master.
#[test]
fn the_ttf_rows_open_their_editors() {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    let first = app.session.font().ok().and_then(|f| f.masters.first()).map(|m| m.id.clone()).expect("a master");
    let target = Target { scope: Scope::Master, indices: vec![0] };
    for (key, dialog) in [("TTFStems", RowDialog::TtfStems), ("TTFZones", RowDialog::TtfZones)] {
        let row = RowModel {
            sort: RowSort::Param,
            key: key.into(),
            label: key.into(),
            enabled: true,
            removable: true,
            editor: Editor::Dialog { summary: String::new(), dialog },
            ..RowModel::default()
        };
        app.sent.extend(rows::commands(&target, &row, &RowAction::Edit(Edit::Dialog(dialog))));
    }
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
    assert_eq!(app.tt.asks, vec![Ask::Definitions(Kind::Stems, first.clone()), Ask::Definitions(Kind::Zones, first)]);
}
