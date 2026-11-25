//! Behaviour tests of the Kerning window, its questions and the kerning audit and Match Kerning dialogs.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::host::Host;
use craft::ui::contract::Tracked;
use craft::ui::dialog::{DialogResult, DialogStack, StackDialog};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use craft_ext::ui::list::{EditCommand, deliver};
use egui::{Key, Modifiers};
use fontcraft_engine::Session;
use fontcraft_engine::model::Layer;
use serde_json::{Map, Value, json};

use super::{Props, show};
use crate::app::clipboard::{self, PasteKind};
use crate::app::models::kerning_window::{KernRows, KerningWindowModel};
use crate::app::{APP, FontCraftApp, Msg};
use crate::dialogs::kerning_audit::AuditDialog;
use crate::dialogs::kerning_questions::{PasteConflict, RenameGroup};
use crate::shell::kerning_window::{Audit, Column, Finding, Question};
use crate::testing::TestApp;

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

fn rows(listed: &[(&str, &str, f64)]) -> KernRows {
    KernRows::of(listed.iter().enumerate().map(|(i, (l, r, v))| (l.to_string(), r.to_string(), *v, i)))
}

/// T and o 600 wide with a 50-unit sidebearing on each side; T in the right group T, o and c in the left group o;
/// the pair T–@o at `value`; A–V −100.
fn kerned_session(value: f64) -> Session {
    let mut s = Session::with_document();
    let mut run = |id: &str, p: Value| {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    };
    for glyph in ["T", "o", "c"] {
        run("path.addRectangle", json!({"glyph": glyph, "x": 50, "y": 0, "width": 500, "height": 500}));
        run("spacing.setWidth", json!({"glyphs": [glyph], "value": 600}));
    }
    run("kerning.setRightGroup", json!({"glyphs": ["T"], "group": "T"}));
    run("kerning.setLeftGroup", json!({"glyphs": ["o", "c"], "group": "o"}));
    run("kerning.setPair", json!({"left": "T", "right": "@o", "value": value}));
    run("kerning.setPair", json!({"left": "A", "right": "V", "value": -100}));
    s
}

fn app_over(session: Session) -> FontCraftApp {
    FontCraftApp::with_edit_tab(session, Arc::new(Host::builder(APP).build()))
}

fn ran(app: &mut FontCraftApp, msg: Msg) {
    app.sent.push(msg);
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
}

/// The value of the pair stored in the first master, left to right.
fn stored(app: &mut FontCraftApp, left: &str, right: &str) -> Option<f64> {
    let listed = app.session.query("kerning.listPairs", &json!({})).expect("the pairs");
    let pairs = listed["pairs"].as_array().cloned().unwrap_or_default();
    pairs.iter().find(|p| p["left"] == left && p["right"] == right).and_then(|p| p["value"].as_f64())
}

fn listed(app: &FontCraftApp) -> Vec<(String, String, f64)> {
    app.models.kerning_window.rows.iter().map(|r| (r.left.to_string(), r.right.to_string(), r.value)).collect()
}

/// Typing a term, a header press, a row press and Delete post the window's commands.
#[test]
fn the_window_posts_search_sort_selection_and_delete() {
    let model = Tracked::new(KerningWindowModel {
        has_doc: true,
        master: Some("m01".into()),
        direction: "ltr".into(),
        rows: rows(&[("@T", "@o", -80.0), ("A", "V", -100.0), ("L", "T", -60.0)]),
        total: 3,
        selected: vec![1].into(),
        current: Some(1),
        ..Default::default()
    });
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.room(egui::vec2(720.0, 560.0)).settle();
    let search = fixture.by_label("Search").expect("the search field");
    fixture.click(&search).type_text("A").settle();
    assert!(posted.borrow().contains(&shell("kerning.filterPairs", json!({"term": "A"}))), "{:?}", posted.borrow());

    let value = fixture.by_label("Value").expect("the Value header");
    fixture.click(&value).settle();
    assert!(
        posted.borrow().iter().any(|m| matches!(m, Msg::Shell(c) if c.id == "kerning.sortPairs" && c.params["column"] == "value")),
        "{:?}",
        posted.borrow()
    );

    let l = fixture.by_label("L").expect("the L row");
    fixture.click(&l).settle();
    assert!(posted.borrow().contains(&shell("window.kerning", json!({"select": [{"left": "L", "right": "T"}]}))), "{:?}", posted.borrow());

    posted.borrow_mut().clear();
    fixture.key(Key::Delete, Modifiers::NONE).settle();
    let deleted = posted.borrow().iter().find_map(|m| match m {
        Msg::Run(c) if c.id == "kerning.deletePairs" => Some(c.params.clone()),
        _ => None,
    });
    let deleted = deleted.expect("Delete posts kerning.deletePairs");
    assert_eq!(deleted["master"], "m01");
    assert_eq!(deleted["pairs"].as_array().map(Vec::len), Some(1));
}

/// `window.kerning` shows the pairs of the first master; the search, the sort and the steps change what is
/// listed and selected, and `ui.inspect` reports it.
#[test]
fn window_kerning_lists_filters_sorts_and_steps() {
    let mut t = TestApp::with(kerned_session(-60.0));
    t.ok("engine.execute", json!({"command": "window.kerning", "params": {}}));
    assert_eq!(listed(&t.app), vec![("T".into(), "@o".into(), -60.0), ("A".into(), "V".into(), -100.0)]);

    t.ok("engine.execute", json!({"command": "kerning.sortPairs", "params": {"column": "value"}}));
    assert_eq!(listed(&t.app).first().map(|r| r.2), Some(-100.0));
    t.ok("engine.execute", json!({"command": "kerning.filterPairs", "params": {"kind": "group"}}));
    assert_eq!(listed(&t.app), vec![("T".into(), "@o".into(), -60.0)]);
    t.ok("engine.execute", json!({"command": "kerning.resetFilter", "params": {}}));
    assert_eq!(listed(&t.app).len(), 2);

    t.ok("engine.execute", json!({"command": "kerning.selectNextPair", "params": {}}));
    t.ok("engine.execute", json!({"command": "kerning.selectNextPair", "params": {}}));
    let window = t.ok("ui.inspect", json!({}))["kerningWindow"].clone();
    assert_eq!(window["open"], true);
    assert_eq!(window["count"], 2);
    assert_eq!(window["sort"], json!({"column": "value", "descending": false}));
    assert_eq!(window["selected"], json!([{"left": "T", "right": "@o"}]));

    let refused = t.error("engine.execute", json!({"command": "kerning.filterPairs", "params": {"match": "near"}}));
    assert!(refused.contains("match"), "{refused}");
}

/// Show All Glyphs puts every glyph pair of the current row into the edit view's text, in place of the shown pair.
#[test]
fn show_all_glyphs_puts_every_pair_of_the_current_row_into_the_text() {
    let mut app = app_over(kerned_session(-60.0));
    ran(&mut app, shell("window.kerning", json!({"select": [{"left": "T", "right": "@o"}]})));
    ran(&mut app, shell("kerning.showAllGlyphs", json!({})));
    let font = app.session.font().unwrap().clone();
    let space = font.glyphs.iter().find(|g| g.unicodes.contains(&0x20)).map(|g| g.name.clone());
    let mut expected: Vec<String> = Vec::new();
    for (j, member) in fontcraft_spacing::kerning::members(&font, fontcraft_spacing::kerning::GroupSide::Left, "o").into_iter().enumerate() {
        if j > 0 {
            expected.extend(space.clone());
        }
        expected.extend(["T".to_string(), member.to_string()]);
    }
    let text = &app.view.text;
    let shown: Vec<String> = (0..text.text.len()).filter_map(|i| text.item_glyph(i).map(str::to_string)).collect();
    assert!(shown.starts_with(&expected), "{shown:?} starts with {expected:?}");

    app.sent.push(shell("kerning.showAllGlyphs", json!({"left": "T"})));
    app.drain();
    assert!(app.status.contains("both"), "one side alone is refused: {}", app.status);
}

/// The list follows the writing direction of the tab until the window names one; what the window posts goes to the
/// direction listed, and a new tab for a pair takes it.
#[test]
fn an_arabic_pair_is_listed_edited_and_deleted_in_its_direction() {
    let mut session = kerned_session(-60.0);
    session.execute("kerning.setPair", &json!({"left": "o", "right": "T", "value": -30, "direction": "rtl"})).expect("a right-to-left pair");
    let mut app = app_over(session);
    ran(&mut app, shell("window.kerning", json!({})));
    assert_eq!(listed(&app).len(), 2);
    ran(&mut app, shell("editView.setWritingDirection", json!({"direction": "rtl"})));
    assert_eq!(listed(&app), vec![("o".into(), "T".into(), -30.0)], "the tab's direction");
    ran(&mut app, shell("window.kerning", json!({"direction": "vertical"})));
    assert!(listed(&app).is_empty());
    ran(&mut app, shell("window.kerning", json!({"direction": null})));

    ran(&mut app, shell("window.kerning", json!({"select": [{"left": "o", "right": "T"}]})));
    let model = app.models.kerning_window.clone();
    assert_eq!((model.direction.as_str(), model.selected.to_vec()), ("rtl", vec![0]));
    let mut p = super::scope(&model);
    for (k, v) in [("left", json!("o")), ("right", json!("T")), ("value", json!(-44))] {
        p.insert(k.into(), v);
    }
    ran(&mut app, Msg::run("kerning.setPair", Value::Object(p)));
    assert_eq!(listed(&app), vec![("o".into(), "T".into(), -44.0)]);
    let mut sent = Vec::new();
    super::delete(&app.models.kerning_window, &[0], false, &mut sent);
    for msg in sent {
        ran(&mut app, msg);
    }
    assert!(listed(&app).is_empty());
    assert_eq!(stored(&mut app, "T", "@o"), Some(-60.0), "the left-to-right pairs stay");
}

/// Adds a second master m02 whose layers are 600 wide.
fn add_master(session: &mut Session) {
    session
        .edit("setup", |font| {
            let mut m = font.default_master()?.clone();
            m.id = "m02".into();
            m.name = "Bold".into();
            Arc::make_mut(&mut font.masters).push(m);
            let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
            for name in names {
                font.glyph_mut(&name)?.layers.push(Layer::new("m02", 600.0));
            }
            Ok(())
        })
        .expect("a second master");
}

/// Copy, switch master, Paste: without a clash the pairs arrive in the master shown; with one the question opens.
#[test]
fn copied_pairs_paste_into_the_master_shown() {
    let mut session = kerned_session(-60.0);
    add_master(&mut session);
    let mut app = app_over(session);
    ran(&mut app, shell("window.kerning", json!({})));
    ran(&mut app, shell("window.kerning", json!({"select": "all"})));
    let model = app.models.kerning_window.clone();
    assert_eq!(*model.selected, vec![0, 1]);
    let mut p = super::scope(&model);
    p.insert("pairs".into(), super::pairs_json(&model, &model.selected));
    ran(&mut app, Msg::run("kerning.copyPairs", Value::Object(p)));
    let text = app.clipboard.out.take().expect("the pairs' text");
    ran(&mut app, shell("view.selectMaster", json!({"master": "m02"})));
    assert!(listed(&app).is_empty());

    ran(&mut app, shell("dialog.pasteKerningConflict", json!({})));
    assert_eq!(app.clipboard.request.take(), Some(PasteKind::KerningPairs), "Paste asks for the clipboard's text");
    clipboard::paste(&mut app, &text, PasteKind::KerningPairs);
    app.drain();
    assert_eq!(listed(&app), vec![("T".into(), "@o".into(), -60.0), ("A".into(), "V".into(), -100.0)]);
    assert!(app.models.kerning_window.selected.is_empty(), "Select All ended with the first master's list");

    let params = json!({"left": "T", "right": "@o", "value": -10, "master": "m02"});
    app.run_now(Msg::run("kerning.setPair", params), false).expect("the pair is set");
    clipboard::paste(&mut app, &text, PasteKind::KerningPairs);
    app.drain();
    match app.view.kerning.question.take() {
        Some(Question::Paste { params, count, .. }) => assert_eq!((params["master"].clone(), count), (json!("m02"), 1)),
        other => panic!("a clash asks, got {other:?}"),
    }
    assert_eq!(listed(&app).first().map(|r| r.2), Some(-10.0), "asking pastes nothing");
}

/// A pair that names a glyph the font lacks can be selected and deleted; the status line says why it is not shown.
#[test]
fn an_obsolete_row_is_selected_and_deleted() {
    let mut app = app_over(kerned_session(-60.0));
    app.run_now(Msg::run("kerning.pastePairs", json!({"pairs": [{"left": "gone", "right": "A", "value": -20}]})), false).expect("pasted");
    app.status.clear();
    ran(&mut app, shell("window.kerning", json!({})));
    app.sent.push(shell("window.kerning", json!({"select": [{"left": "gone", "right": "A"}]})));
    app.drain();
    assert!(app.status.contains("gone"), "{}", app.status);
    let selected = app.models.kerning_window.selected.clone();
    assert_eq!(selected.len(), 1);
    app.status.clear();
    let mut sent = Vec::new();
    super::delete(&app.models.kerning_window, &selected, false, &mut sent);
    for msg in sent {
        ran(&mut app, msg);
    }
    assert_eq!(stored(&mut app, "gone", "A"), None);
    app.sent.push(shell("kerning.selectNextPair", json!({})));
    app.drain();
    assert_eq!(app.models.kerning_window.current, Some(0), "the steps go on");
}

/// The minus button deletes from the master shown, with Option held from every master; Copy, Paste and Select All
/// handed to the focused list post its commands.
#[test]
fn the_minus_button_and_the_edit_commands_act_on_the_list() {
    let model = Tracked::new(KerningWindowModel {
        has_doc: true,
        master: Some("m01".into()),
        direction: "rtl".into(),
        rows: rows(&[("A", "V", -100.0), ("L", "T", -60.0)]),
        total: 2,
        selected: vec![1].into(),
        current: Some(1),
        ..Default::default()
    });
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.room(egui::vec2(720.0, 560.0)).settle();
    let minus = fixture.by_label("Delete the selected pairs").expect("the minus button");
    fixture.hold(Modifiers::ALT).click(&minus).hold(Modifiers::NONE).settle();
    let pair = json!([{"left": "L", "right": "T"}]);
    let expected = json!({"master": "m01", "direction": "rtl", "pairs": pair, "allMasters": true});
    assert!(posted.borrow().contains(&Msg::run("kerning.deletePairs", expected)), "{:?}", posted.borrow());

    let l = fixture.by_label("L").expect("the L row");
    fixture.click(&l).settle();
    posted.borrow_mut().clear();
    let ctx = fixture.ctx().clone();
    for edit in [EditCommand::Copy, EditCommand::Paste, EditCommand::SelectAll] {
        assert!(deliver(&ctx, edit), "{edit:?}");
        fixture.settle();
    }
    let copy = json!({"master": "m01", "direction": "rtl", "pairs": pair});
    assert_eq!(
        *posted.borrow(),
        vec![Msg::run("kerning.copyPairs", copy), shell("dialog.pasteKerningConflict", json!({})), shell("window.kerning", json!({"select": "all"}))]
    );
}

/// A window over two group rows with the Value or Right cell of the first open for editing, and what it posts.
fn editing_fixture(column: Column) -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
    let model = Tracked::new(KerningWindowModel {
        has_doc: true,
        master: Some("m01".into()),
        direction: "ltr".into(),
        rows: rows(&[("@T", "@o", -80.0), ("A", "V", -100.0)]),
        total: 2,
        selected: vec![0].into(),
        current: Some(0),
        editing: Some((0, column)),
        ..Default::default()
    });
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.room(egui::vec2(720.0, 560.0)).settle();
    posted.borrow_mut().clear();
    (fixture, posted)
}

/// A value typed into the row sets the pair, Shift+Up steps it, Escape sends nothing but the close; a right group's
/// new name asks the rename question for the right side.
#[test]
fn cells_are_edited_in_place() {
    let close = shell("window.kerning", json!({"edit": null}));
    let (mut fixture, posted) = editing_fixture(Column::Value);
    fixture.key(Key::A, Modifiers::COMMAND).type_text("-72").key(Key::Enter, Modifiers::NONE).settle();
    let set = |v: f64| Msg::run("kerning.setPair", json!({"master": "m01", "direction": "ltr", "left": "@T", "right": "@o", "value": v}));
    assert!(posted.borrow().contains(&set(-72.0)) && posted.borrow().contains(&close), "{:?}", posted.borrow());

    let (mut fixture, posted) = editing_fixture(Column::Value);
    fixture.key(Key::ArrowUp, Modifiers::SHIFT).settle();
    assert!(posted.borrow().contains(&set(-70.0)), "{:?}", posted.borrow());

    // A draft is not stepped: Return stores it, once.
    let (mut fixture, posted) = editing_fixture(Column::Value);
    fixture.key(Key::A, Modifiers::COMMAND).type_text("-72").key(Key::ArrowUp, Modifiers::NONE).key(Key::Enter, Modifiers::NONE).settle();
    let runs: Vec<Msg> = posted.borrow().iter().filter(|m| matches!(m, Msg::Run(_))).cloned().collect();
    assert_eq!(runs, [set(-72.0)]);

    // A press elsewhere closes the field, typed into or not.
    let (mut fixture, posted) = editing_fixture(Column::Value);
    assert!(!posted.borrow().contains(&close));
    let search = fixture.by_label("Search").expect("the search field");
    fixture.click(&search).settle();
    assert!(posted.borrow().contains(&close), "{:?}", posted.borrow());
    assert!(!posted.borrow().iter().any(|m| matches!(m, Msg::Run(_))), "{:?}", posted.borrow());

    let (mut fixture, posted) = editing_fixture(Column::Value);
    fixture.key(Key::A, Modifiers::COMMAND).type_text("5").key(Key::Escape, Modifiers::NONE).settle();
    assert!(!posted.borrow().iter().any(|m| matches!(m, Msg::Run(_))), "{:?}", posted.borrow());
    assert!(posted.borrow().contains(&close), "{:?}", posted.borrow());

    let (mut fixture, posted) = editing_fixture(Column::Right);
    fixture.key(Key::A, Modifiers::COMMAND).type_text("round").key(Key::Enter, Modifiers::NONE).settle();
    let asked = posted.borrow().iter().find_map(|m| match m {
        Msg::Shell(c) if c.id == "dialog.renameKerningGroup" => Some(c.params.clone()),
        _ => None,
    });
    let asked = asked.expect("the rename question");
    assert_eq!((asked["side"].clone(), asked["from"].clone(), asked["to"].clone()), (json!("right"), json!("o"), json!("round")));
}

/// With several rows selected a typed text sets or changes them all in one command.
#[test]
fn a_text_typed_for_several_rows_sets_or_changes_them_all() {
    for (text, change) in
        [("40", ("value", 40.0)), ("=-40", ("value", -40.0)), ("+10", ("add", 10.0)), ("-10", ("add", -10.0)), ("*0.9", ("factor", 0.9))]
    {
        assert_eq!(super::change_of(text), Some(change), "{text}");
    }
    assert!(super::change_of("wide").is_none() && super::change_of("*inf").is_none());

    let model = Tracked::new(KerningWindowModel {
        has_doc: true,
        master: Some("m01".into()),
        direction: "ltr".into(),
        rows: rows(&[("@T", "@o", -80.0), ("A", "V", -100.0)]),
        total: 2,
        selected: vec![0, 1].into(),
        current: Some(1),
        ..Default::default()
    });
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.room(egui::vec2(720.0, 560.0)).settle();
    let field = fixture.by_label("Values of the Selected Pairs").expect("the values field");
    fixture.click(&field).type_text("*0.9").key(Key::Enter, Modifiers::NONE).settle();
    let pairs = json!([{"left": "@T", "right": "@o"}, {"left": "A", "right": "V"}]);
    let expected = json!({"master": "m01", "direction": "ltr", "pairs": pairs, "factor": 0.9});
    assert!(posted.borrow().contains(&Msg::run("kerning.setPairs", expected)), "{:?}", posted.borrow());
}

/// Make Exception stores the glyph pair Edit View shows when the row covers it, else the first members'.
#[test]
fn make_exception_takes_the_pair_edit_view_shows() {
    let mut app = app_over(kerned_session(-60.0));
    ran(&mut app, shell("window.kerning", json!({"select": [{"left": "T", "right": "@o"}]})));
    ran(&mut app, shell("editView.setText", json!({"glyphs": ["T", "o"]})));
    ran(&mut app, shell("editView.setCursor", json!({"index": 1})));
    ran(&mut app, shell("kerning.makeException", json!({})));
    assert_eq!((stored(&mut app, "T", "o"), stored(&mut app, "T", "c")), (Some(-60.0), None));
    ran(&mut app, shell("editView.setText", json!({"glyphs": ["A", "V"]})));
    ran(&mut app, shell("kerning.makeException", json!({})));
    assert_eq!(stored(&mut app, "T", "c"), Some(-60.0));
    app.sent.push(shell("kerning.makeException", json!({"left": "A", "right": "V"})));
    app.drain();
    assert!(app.status.contains("two glyphs"), "{}", app.status);
}

/// An edit of one value patches the list and shares its names; an edit elsewhere keeps the list as it is; a new pair
/// lists again; Select All is sent as the list's query.
#[test]
fn the_list_is_read_again_only_when_its_pairs_move() {
    let mut app = app_over(kerned_session(-60.0));
    ran(&mut app, shell("window.kerning", json!({})));
    let first = app.models.kerning_window.rows.clone();
    ran(&mut app, Msg::run("spacing.setWidth", json!({"glyphs": ["T"], "value": 700})));
    assert!(app.models.kerning_window.rows == first, "an outline edit keeps the list");
    ran(&mut app, Msg::run("kerning.setPair", json!({"left": "A", "right": "V", "value": -90})));
    let patched = app.models.kerning_window.rows.clone();
    assert!(patched != first && patched.shares_names(&first));
    assert_eq!(listed(&app).last().map(|r| r.2), Some(-90.0));
    ran(&mut app, shell("kerning.sortPairs", json!({"column": "value"})));
    ran(&mut app, Msg::run("kerning.setPair", json!({"left": "A", "right": "V", "value": 10})));
    assert_eq!(listed(&app).last().map(|r| r.2), Some(10.0), "a list sorted by value is sorted again");
    ran(&mut app, Msg::run("kerning.setPair", json!({"left": "L", "right": "T", "value": -5})));
    assert_eq!(listed(&app).len(), 3);

    ran(&mut app, shell("kerning.filterPairs", json!({"kind": "notGroup", "side": "right"})));
    ran(&mut app, shell("window.kerning", json!({"select": "all"})));
    let model = app.models.kerning_window.clone();
    let mut sent = Vec::new();
    super::delete(&model, &model.selected, false, &mut sent);
    let expected = json!({"term": "", "match": "contains", "kind": "notGroup", "side": "right"});
    assert!(matches!(sent.first(), Some(Msg::Run(c)) if c.params["query"] == expected && c.params.get("pairs").is_none()), "{sent:?}");
    for msg in sent {
        ran(&mut app, msg);
    }
    ran(&mut app, shell("kerning.resetFilter", json!({})));
    assert_eq!(listed(&app), vec![("T".into(), "@o".into(), -60.0)]);
}

/// With Font View in front a chosen row opens a tab for the pair when none is open, else brings the last tab forward;
/// the text around the pair stays.
#[test]
fn a_row_chosen_from_font_view_shows_in_an_edit_view_tab() {
    let mut app = FontCraftApp::new(kerned_session(-60.0), Arc::new(Host::builder(APP).build()));
    app.project();
    let names = |app: &FontCraftApp| -> Vec<String> {
        let text = &app.view.text;
        (0..text.text.len()).filter_map(|i| text.item_glyph(i).map(str::to_string)).collect()
    };
    ran(&mut app, shell("window.kerning", json!({"select": [{"left": "A", "right": "V"}]})));
    assert!(!app.font_view_shows());
    assert_eq!(names(&app), ["A", "V"]);
    ran(&mut app, shell("editView.setText", json!({"glyphs": ["T", "A", "V", "o"]})));
    ran(&mut app, shell("editView.setCursor", json!({"index": 2})));
    crate::shell::tabs::show(&mut app, None);
    ran(&mut app, shell("kerning.selectPreviousPair", json!({})));
    assert!(!app.font_view_shows());
    assert_eq!(app.view.tabs.tabs.len(), 1, "the same tab");
    assert_eq!(names(&app), ["T", "T", "c", "o"], "the text around the pair stays");
}

/// The rename question answers All Pairs with scope font and This Pair with scope pair, with the name typed.
#[test]
fn the_rename_question_answers_with_the_scope_and_the_new_name() {
    let answered = |press: &'static str| {
        let mut params = Map::new();
        for (k, v) in [("side", json!("right")), ("from", json!("o")), ("to", json!("o")), ("pair", json!({"left": "T", "right": "@o"}))] {
            params.insert(k.into(), v);
        }
        let dialog = Rc::new(RefCell::new(RenameGroup::new(params)));
        let result = Rc::new(RefCell::new(None));
        let (held, out) = (Rc::clone(&dialog), Rc::clone(&result));
        let mut fixture = Fixture::new(move |ui| {
            if let Some(r) = held.borrow_mut().show(ui, &DialogStack::default()) {
                *out.borrow_mut() = Some(r);
            }
        });
        fixture.settle();
        let field = fixture.by_label("New name for the group @o").expect("the name field");
        fixture.click(&field).key(Key::A, Modifiers::COMMAND).type_text("round").settle();
        let button = fixture.by_label(press).expect("the button");
        fixture.click(&button).settle();
        result.borrow_mut().take()
    };
    match answered("This Pair") {
        Some(DialogResult::Ok(p)) => assert_eq!((p["to"].clone(), p["scope"].clone()), (json!("round"), json!("pair"))),
        other => panic!("This Pair answered {other:?}"),
    }
    match answered("All Pairs") {
        Some(DialogResult::Ok(p)) => assert_eq!((p["to"].clone(), p["scope"].clone()), (json!("round"), json!("font"))),
        other => panic!("All Pairs answered {other:?}"),
    }
}

/// A paste that clashes pastes nothing and asks; Keep Existing pastes the pairs the master lacks.
#[test]
fn a_clashing_paste_asks_and_keep_existing_adds_only_new_pairs() {
    let mut app = app_over(kerned_session(-60.0));
    let before = app.session.doc().map(|d| d.font.clone()).expect("a document");
    let pairs = json!([{"left": "T", "right": "@o", "value": -30}, {"left": "L", "right": "T", "value": -40}]);
    ran(&mut app, shell("dialog.pasteKerningConflict", json!({"pairs": pairs})));
    assert!(Arc::ptr_eq(&before, &app.session.doc().map(|d| d.font.clone()).expect("a document")), "asking pasted something");
    let Some(Question::Paste { params, conflicts, count }) = app.view.kerning.question.clone() else { panic!("no question") };
    assert_eq!((count, conflicts.first().map(|c| (c.value, c.existing))), (1, Some((-30.0, -60.0))));

    let dialog = Rc::new(RefCell::new(PasteConflict::new(params, conflicts, count)));
    let result = Rc::new(RefCell::new(None));
    let (held, out) = (Rc::clone(&dialog), Rc::clone(&result));
    let mut fixture = Fixture::new(move |ui| {
        if let Some(r) = held.borrow_mut().show(ui, &DialogStack::default()) {
            *out.borrow_mut() = Some(r);
        }
    });
    fixture.settle();
    let keep = fixture.by_label("Keep Existing").expect("the Keep Existing button");
    fixture.click(&keep).settle();
    let Some(DialogResult::Ok(answer)) = result.borrow_mut().take() else { panic!("Keep Existing gave no answer") };
    assert_eq!(answer["onConflict"], "keepExisting");
    app.run_now(Msg::run("kerning.pastePairs", answer), false).expect("the paste runs");
    assert_eq!(stored(&mut app, "T", "@o"), Some(-60.0));
    assert_eq!(stored(&mut app, "L", "T"), Some(-40.0));
}

/// The audit lists the glyph pairs class kerning makes collide; with fix it adds their exceptions.
#[test]
fn the_audit_lists_colliding_pairs_and_fix_adds_exceptions() {
    let mut app = app_over(kerned_session(-200.0));
    ran(&mut app, shell("dialog.auditKerningExceptions", json!({})));
    let audit = app.view.kerning.audit.clone().expect("the audit report");
    let pairs: Vec<(&str, &str)> = audit.findings.iter().map(|f| (f.left.as_str(), f.right.as_str())).collect();
    assert!(pairs.contains(&("T", "o")) && pairs.contains(&("T", "c")), "{pairs:?}");
    assert!(!audit.fixed);

    ran(&mut app, shell("dialog.auditKerningExceptions", json!({"fix": true})));
    let audit = app.view.kerning.audit.clone().expect("the audit report");
    assert!(audit.fixed && audit.run == 2);
    assert!(audit.findings.iter().all(|f| f.fix.is_some()), "{:?}", audit.findings);
    assert!(stored(&mut app, "T", "o").is_some(), "no exception for T o");
}

/// Add Exceptions asks the shell command again with fix; Audit asks it without.
#[test]
fn the_audit_dialog_buttons_ask_for_a_run() {
    let report = Audit {
        direction: "ltr".into(),
        findings: vec![Finding { master: "Regular".into(), left: "T".into(), right: "o".into(), distance: -100.0, ..Default::default() }],
        found: 1,
        run: 1,
        ..Default::default()
    };
    let asked = |press: &'static str| {
        let dialog = Rc::new(RefCell::new(AuditDialog::new(report.clone())));
        let held = Rc::clone(&dialog);
        let mut fixture = Fixture::new(move |ui| {
            let _ = held.borrow_mut().show(ui, &DialogStack::default());
        });
        fixture.room(egui::vec2(900.0, 640.0)).settle();
        let button = fixture.by_label(press).expect("the button");
        fixture.click(&button).settle();
        dialog.borrow_mut().request.take()
    };
    assert_eq!(asked("Add Exceptions").map(|p| p["fix"].clone()), Some(json!(true)));
    assert_eq!(asked("Audit").map(|p| p["fix"].clone()), Some(json!(false)));
}

/// Match Kerning asks for the main master first; run, it gives both masters the same pairs and sums up.
#[test]
fn match_kerning_asks_then_reports_each_master() {
    let mut session = kerned_session(-60.0);
    add_master(&mut session);
    let mut app = app_over(session);
    ran(&mut app, shell("dialog.matchKerning", json!({})));
    let asked = app.view.kerning.matching.clone().expect("the question");
    assert!(asked.summary.is_none());
    assert_eq!(asked.choices.len(), 2);

    ran(&mut app, shell("dialog.matchKerning", json!({"run": true})));
    let summary = app.view.kerning.matching.clone().and_then(|m| m.summary).expect("the summary");
    assert_eq!(summary.masters.len(), 2, "{summary:?}");
    assert!(summary.masters.iter().all(|m| m.after == 2), "{summary:?}");
    assert!(summary.filled >= 2, "{summary:?}");
}
