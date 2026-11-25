use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::host::Host;
use craft::ui::contract::Tracked;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use fontcraft_engine::Session;
use fontcraft_engine::model::Layer;
use serde_json::{Value, json};

use super::{DropAction, Props, drop_action, drop_command, row_command, row_entries, show};
use crate::app::models::layers::{LayerRow, LayersModel};
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};

/// A font whose glyph A has a second master `m02`, three backups under `m01` and one under `m02`; the ids of the
/// backups.
fn two_masters() -> (FontCraftApp, [String; 4]) {
    let mut session = Session::with_document();
    session
        .edit("setup", |font| {
            let mut m = font.default_master()?.clone();
            m.id = "m02".into();
            m.name = "Bold".into();
            Arc::make_mut(&mut font.masters).push(m);
            for name in ["A", "B"] {
                font.glyph_mut(name)?.layers.push(Layer::new("m02", 600.0));
            }
            Ok(())
        })
        .unwrap();
    let mut add = |master: &str, name: &str| {
        session.execute("layers.addBackupLayer", &json!({"glyphs": ["A"], "master": master, "name": name})).unwrap()["layer"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let ids = [add("m01", "one"), add("m01", "two"), add("m01", "three"), add("m02", "other")];
    let mut app = FontCraftApp::with_edit_tab(session, Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    (app, ids)
}

fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
    app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
    app.drain();
}

fn row<'m>(model: &'m LayersModel, id: &str) -> &'m LayerRow {
    model.row(id).unwrap()
}

/// Drops within a master reorder by the index `layers.reorder` takes; a drop under another master is refused; a
/// backup dropped on its master's row becomes the master layer; a drop where the row already is does nothing. The
/// reorder then reaches the engine and the panel's rows.
#[test]
fn drops_reorder_within_a_master_and_refuse_others() {
    let (mut app, [one, two, three, other]) = two_masters();
    let model = app.models.layers.clone();
    assert_eq!(model.shown, ["m01", one.as_str(), two.as_str(), three.as_str(), "m02", other.as_str()]);
    let r = |id: &str| Some(row(&model, id));

    // Between m01 and `one`: first among m01's layers.
    assert_eq!(drop_action(&model, &three, r("m01"), r(&one), r(&one)), DropAction::Reorder(0));
    // Between `two` and `three` for `one`: after `two`.
    assert_eq!(drop_action(&model, &one, r(&two), r(&three), r(&two)), DropAction::Reorder(1));
    // Onto another master's rows.
    assert_eq!(drop_action(&model, &one, r("m02"), r(&other), r(&other)), DropAction::Refused);
    assert_eq!(drop_action(&model, &one, r(&three), r("m02"), r("m02")), DropAction::Reorder(2), "below the last row of its own master");
    assert_eq!(drop_action(&model, &one, r(&three), r("m02"), None), DropAction::Reorder(2));
    assert_eq!(drop_action(&model, &one, r("m02"), r(&other), None), DropAction::Refused);
    // Where it already is.
    assert_eq!(drop_action(&model, &two, r(&one), r(&two), r(&two)), DropAction::Nothing);
    // A backup over its master's row.
    assert_eq!(drop_action(&model, &two, None, r("m01"), r("m01")), DropAction::UseAsMaster);
    // A master row does not move.
    assert_eq!(drop_action(&model, "m02", r(&one), r(&two), None), DropAction::Refused);

    app.sent.extend(drop_command(&model, &three, DropAction::Reorder(0)));
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
    assert_eq!(app.models.layers.shown, ["m01", three.as_str(), one.as_str(), two.as_str(), "m02", other.as_str()]);
}

/// A click picks the active layer, Shift and Cmd extend the row selection from it, a master switch draws the glyph
/// in that master, the filters hide rows, and a command that fails changes nothing.
#[test]
fn layer_selection_masters_and_filters() {
    let (mut app, [one, two, _, other]) = two_masters();
    assert_eq!(app.models.layers.active_layer.as_deref(), Some("m01"));

    shell(&mut app, "layers.select", json!({"layer": two, "extend": "range"}));
    assert_eq!(app.models.layers.selection, ["m01", one.as_str(), two.as_str()]);
    assert_eq!(app.models.layers.active_layer.as_deref(), Some("m01"), "extending keeps the active layer");
    shell(&mut app, "layers.select", json!({"layer": one, "extend": "toggle"}));
    assert_eq!(app.models.layers.selection, ["m01", two.as_str()]);

    shell(&mut app, "layers.select", json!({"layer": other}));
    assert_eq!(app.models.layers.active_layer.as_deref(), Some(other.as_str()));
    assert_eq!(app.models.layers.active_master.as_deref(), Some("m02"));

    shell(&mut app, "view.selectMaster", json!({"master": 1}));
    assert_eq!(app.models.masters.current.as_deref(), Some("m02"));
    assert_eq!(app.models.layers.active_layer.as_deref(), Some("m02"));

    shell(&mut app, "layers.onlyShowLayersFromCurrentMaster", json!({}));
    assert_eq!(app.models.layers.shown, ["m02", other.as_str()]);
    shell(&mut app, "layers.hideBackupLayers", json!({"on": true}));
    assert_eq!(app.models.layers.shown, ["m02"]);

    let before = app.view.layers.clone();
    shell(&mut app, "view.selectMaster", json!({"master": 2}));
    assert!(app.status.starts_with("view.selectMaster"), "{}", app.status);
    shell(&mut app, "layers.select", json!({"layer": "nope"}));
    assert!(app.status.starts_with("layers.select"), "{}", app.status);
    assert_eq!(app.view.layers, before, "a command that fails changes nothing");
}

/// The panel posts what its rows and buttons ask for: a click on a row selects that layer, New Backup Layer backs up
/// the active layer, and Use as Master waits for a selected backup.
#[test]
fn rows_and_buttons_post_their_commands() {
    let (app, [one, ..]) = two_masters();
    let model = Tracked::new((*app.models.layers).clone());
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.settle();
    let row = fixture.by_label("one").expect("the row of the backup `one`");
    fixture.click(&row).settle();
    let backup = fixture.by_label("New Backup Layer").expect("the New Backup Layer button");
    fixture.click(&backup).settle();
    let use_as_master = fixture.by_label("Use as Master").expect("the Use as Master button");
    fixture.click(&use_as_master).settle();
    let posted = posted.borrow();
    let [select, Msg::Run(add)] = posted.as_slice() else { panic!("{posted:?}") };
    assert_eq!(*select, Msg::Shell(CommandRef::with_params("layers.select", json!({"layer": one}))));
    let mut params = add.params.clone();
    let name = params.as_object_mut().unwrap().remove("name").expect("the local date and time");
    assert!(name.as_str().is_some_and(|n| n.len() == 16), "{name}");
    assert_eq!((add.id.as_str(), params), ("layers.addBackupLayer", json!({"glyphs": ["A"], "master": "m01", "from": "m01"})));
}

/// Clicks the button `label` of the panel as `app` shows it and runs what it posts.
fn press(app: &mut FontCraftApp, label: &str) {
    let model = Tracked::new((*app.models.layers).clone());
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.settle();
    let button = fixture.by_label(label).unwrap_or_else(|| panic!("no button {label}"));
    fixture.click(&button).settle();
    app.sent.extend(posted.borrow_mut().drain(..));
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
}

fn names(app: &FontCraftApp) -> Vec<String> {
    app.models.layers.all.iter().map(|r| r.name.clone()).collect()
}

/// With Font View in front the section lists the layers all selected glyphs have, by name where their ids differ, and
/// each command changes all of them in one undo step; nothing selected lists nothing.
#[test]
fn the_font_view_selection_shares_rows_and_changes_together() {
    let (mut app, _) = two_masters();
    shell(&mut app, "fontView.show", json!({}));
    assert!(app.models.layers.glyphs.is_empty(), "nothing is selected");
    app.session.execute("layers.addBackupLayer", &json!({"glyphs": ["B"], "master": "m01", "name": "two"})).unwrap();
    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["A", "B"]}));
    let master = app.session.font().unwrap().masters[0].name.clone();
    assert_eq!(names(&app), [master.as_str(), "two", "Bold"], "B has no `one`, `three` or `other`");
    assert_eq!(app.models.layers.active_layer.as_deref(), Some("m01"));

    press(&mut app, "New Backup Layer");
    let layers = |app: &FontCraftApp, glyph: &str| app.session.font().unwrap().glyph(glyph).unwrap().layers.len();
    assert_eq!((layers(&app, "A"), layers(&app, "B")), (7, 4));
    assert_eq!(names(&app).len(), 4, "the new backup is shared");
    app.session.execute("edit.undo", &json!({})).unwrap();
    app.drain();
    assert_eq!((layers(&app, "A"), layers(&app, "B")), (6, 3), "one undo step");

    // `two` has an id of its own in each glyph.
    let two = app.models.layers.all[1].id.clone();
    shell(&mut app, "layers.select", json!({"layer": two}));
    assert_eq!(app.models.layers.selection, [two.clone()]);
    assert!(app.view.layers.layers.is_empty(), "a click in Font View picks the row only");
    press(&mut app, "Delete Layer");
    assert_eq!((layers(&app, "A"), layers(&app, "B")), (5, 2));
    app.session.execute("edit.undo", &json!({})).unwrap();
    app.drain();
    assert_eq!((layers(&app, "A"), layers(&app, "B")), (6, 3), "one undo step");
}

/// The glyphs of a text selection share the section as Font View's do; a click shows each of them in the row's layer.
#[test]
fn a_click_shows_the_text_selection_in_the_layer() {
    let (mut app, _) = two_masters();
    app.session.execute("layers.addBackupLayer", &json!({"glyphs": ["B"], "master": "m01", "name": "two"})).unwrap();
    shell(&mut app, "editView.setText", json!({"glyphs": ["A", "B"]}));
    shell(&mut app, "editView.toggleTextMode", json!({"mode": "text"}));
    shell(&mut app, "editView.setCursor", json!({"index": 2, "anchor": 0}));
    assert_eq!(app.models.layers.glyphs, ["A", "B"]);
    let row = app.models.layers.all[1].clone();
    assert_eq!(row.name, "two");
    shell(&mut app, "layers.select", json!({"layer": row.id}));
    assert!(app.status.is_empty(), "{}", app.status);
    let shown = |glyph: &str| app.view.layers.layers.get(glyph).map(String::as_str);
    assert_eq!((shown("A"), shown("B")), (Some(row.layer_in(0)), Some(row.layer_in(1))));
    assert_ne!(row.layer_in(0), row.layer_in(1));
    assert_eq!(app.models.layers.active_layer.as_deref(), Some(row.id.as_str()));
}

/// The row menu of one of three selected rows acts on all three; the active layer deleted, the glyph shows its master
/// layer, also when that is not the current master's.
#[test]
fn the_row_menu_acts_on_the_selected_rows_and_the_master_follows_a_deleted_layer() {
    let (mut app, [one, two, three, other]) = two_masters();
    shell(&mut app, "layers.select", json!({"layer": one}));
    shell(&mut app, "layers.select", json!({"layer": three, "extend": "range"}));
    let model = app.models.layers.clone();
    assert_eq!(model.selection, [one.clone(), two.clone(), three.clone()]);
    let words = super::LayersWords::default();
    let refused = |id: &str| row_entries(&model, row(&model, &two), &words).iter().any(|e| e.value().is_some_and(|v| v == id) && !e.is_enabled());
    assert!(refused("layers.useAsMaster") && !refused("layers.delete"), "three backups of one master");
    app.sent.extend(row_command(&model, row(&model, &two), "layers.delete"));
    app.drain();
    assert_eq!(app.models.layers.shown, ["m01", "m02", other.as_str()]);
    assert_eq!(app.models.layers.active_layer.as_deref(), Some("m01"));

    shell(&mut app, "layers.select", json!({"layer": other}));
    let model = app.models.layers.clone();
    app.sent.extend(row_command(&model, row(&model, &other), "layers.delete"));
    app.drain();
    assert_eq!(app.models.layers.active_layer.as_deref(), Some("m02"), "the deleted layer's master, not the current one");
}
