//! Glyph Info: the search the table shows, row selection, sorting and Add to Font equal the headless commands.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::host::Host;
use craft::ui::contract::Tracked;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use egui::Modifiers;
use fontcraft_engine::Session;
use serde_json::{Value, json};

use super::{Props, show};
use crate::app::{APP, FontCraftApp, Msg};
use crate::shell::glyph_info::{Column, search_names};

fn command(params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params("window.glyphInfo", params))
}

fn glyph_names(session: &Session) -> Vec<String> {
    session.font().unwrap().glyphs.iter().map(|g| g.name.clone()).collect()
}

#[test]
fn glyph_info_shows_the_search_selects_rows_and_adds_them_as_the_command_does() {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(command(json!({"query": "zhe", "sortBy": "unicode"})));
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
    let expected = search_names(&mut app.session, "zhe", Some(Column::Unicode)).unwrap();
    assert!(expected.len() >= 3, "{expected:?}");
    let rows: Vec<String> = app.models.glyph_info.rows.iter().map(|r| r.name.clone()).collect();
    assert_eq!(rows, expected);

    // The panel over the app: what it posts is drained after each pass.
    let app = Rc::new(RefCell::new(app));
    let shared = Rc::clone(&app);
    let mut fixture = Fixture::new(move |ui| {
        let mut app = shared.borrow_mut();
        let model = Tracked::new((*app.models.glyph_info).clone());
        let mut sent = Vec::new();
        show(ui, Props { model: &model, sent: &mut sent });
        app.sent.extend(sent);
        app.drain();
    });
    fixture.settle();
    let row = |fixture: &Fixture, i: usize| fixture.by_label(&expected[i]).unwrap_or_else(|| panic!("row {}", expected[i]));
    let selected = |app: &Rc<RefCell<FontCraftApp>>| app.borrow().view.glyph_info.selected.clone();

    let first = row(&fixture, 0);
    fixture.click(&first).settle();
    assert_eq!(selected(&app), [expected[0].clone()]);
    let third = row(&fixture, 2);
    fixture.hold(Modifiers::SHIFT).click(&third).hold(Modifiers::NONE).settle();
    assert_eq!(selected(&app), expected[..3].to_vec());
    let second = row(&fixture, 1);
    fixture.hold(Modifiers::COMMAND).click(&second).hold(Modifiers::NONE).settle();
    let picked = selected(&app);
    assert_eq!(picked.len(), 2, "{picked:?}");
    assert!(picked.contains(&expected[0]) && picked.contains(&expected[2]), "{picked:?}");

    // Add to Font gives the font the headless command gives.
    let mut headless = Session::with_document();
    headless.execute("glyphInfo.addToFont", &json!({"names": picked})).unwrap();
    let add = fixture.by_label("Add to Font").expect("the Add to Font button");
    fixture.click(&add).settle();
    assert_eq!(glyph_names(&app.borrow().session), glyph_names(&headless));

    // A header press sorts by its column.
    let name = fixture.by_label("Name").expect("the Name header");
    fixture.click(&name).settle();
    assert_eq!(app.borrow().view.glyph_info.sort_by, Some(Column::Name));
    drop(fixture);

    // A new query searches again and keeps only the selected rows still shown.
    let mut app = Rc::try_unwrap(app).ok().expect("the fixture is gone").into_inner();
    app.sent.push(command(json!({"query": "zhe-cy"})));
    app.drain();
    let again = search_names(&mut app.session, "zhe-cy", Some(Column::Name)).unwrap();
    let rows: Vec<String> = app.models.glyph_info.rows.iter().map(|r| r.name.clone()).collect();
    assert_eq!(rows, again);
    assert!(app.view.glyph_info.selected.iter().all(|n| again.contains(n)));

    // Refused parameters change nothing.
    let before = app.view.glyph_info.clone();
    for params in [json!({"sortBy": "size"}), json!({"select": ["not-a-result"]}), json!({"query": "x".repeat(65)})] {
        app.status.clear();
        app.sent.push(command(params));
        app.drain();
        assert!(app.status.starts_with("window.glyphInfo"), "{}", app.status);
    }
    assert_eq!(app.view.glyph_info, before);
}
