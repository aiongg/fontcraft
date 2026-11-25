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

use super::{DropAction, Props, drop_action, drop_command, row_command, show};
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

    app.sent.extend(drop_command("A", &three, DropAction::Reorder(0)));
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
    assert_eq!(
        *posted.borrow(),
        vec![
            Msg::Shell(CommandRef::with_params("layers.select", json!({"layer": one}))),
            Msg::run("layers.addBackupLayer", json!({"glyphs": ["A"], "master": "m01", "from": "m01"})),
        ]
    );
}

/// Choosing Alternate on one of several selected rows opens the editor on all of them, filled with the first one's
/// values; what is typed there goes to every layer, For All Masters adds one under the other masters, OK is one undo
/// step and Cancel changes nothing. Unticking takes the status off the selected rows that have it.
#[test]
fn the_special_layer_editors_act_on_the_selected_rows_and_on_all_masters() {
    let (mut app, [one, two, _, other]) = two_masters();
    app.session.execute("font.axes.add", &json!({"name": "Weight", "tag": "wght"})).unwrap();
    app.session.execute("font.master.setAxisLocation", &json!({"masters": [0], "location": [100]})).unwrap();
    app.session.execute("font.master.setAxisLocation", &json!({"masters": [1], "location": [900]})).unwrap();
    shell(&mut app, "layers.select", json!({"layer": one}));
    shell(&mut app, "layers.select", json!({"layer": other, "extend": "toggle"}));
    let model = (*app.models.layers).clone();
    assert_eq!(model.selection, [one.clone(), other.clone()]);
    assert_eq!(
        row_command(&model, "A", row(&model, &two), "layers.setAlternate"),
        super::edit_special("A", &[two.as_str()], super::Part::Alternate),
        "a row outside the selection stands alone"
    );
    let call = |app: &mut FontCraftApp, method: &str, params: Value| crate::control::dispatch(app, method, &params).unwrap();
    let label = |app: &FontCraftApp, id: &str| app.models.layers.row(id).and_then(|r| r.bracket.clone());
    let steps = |app: &FontCraftApp| app.session.doc().unwrap().history.undo.len();

    app.sent.push(row_command(&model, "A", row(&model, &one), "layers.setAlternate"));
    app.drain();
    let opened = call(&mut app, "ui.dialog.inspect", json!({}));
    assert_eq!((&opened["dialog"], &opened["params"]["layers"]), (&json!("popover.alternateLayer"), &json!([one, other])), "{opened}");
    let before = app.session.doc().unwrap().font.clone();
    call(&mut app, "ui.dialog.set", json!({"field": "wght.min", "value": "600"}));
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert_eq!(app.session.doc().unwrap().font.glyphs, before.glyphs, "Cancel");

    app.sent.push(row_command(&model, "A", row(&model, &one), "layers.setAlternate"));
    app.drain();
    let count = steps(&app);
    call(&mut app, "ui.dialog.set", json!({"field": "wght.min", "value": "600"}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    app.drain();
    assert_eq!(steps(&app), count + 1, "one undo step");
    assert_eq!((label(&app, &one).as_deref(), label(&app, &other).as_deref()), (Some("[600‹wg]"), Some("[600‹wg]")));

    // The editor opens on the layer's range. With another range the other master has no layer, so it gets one.
    let model = (*app.models.layers).clone();
    app.sent.push(super::edit_special("A", &[one.as_str()], super::Part::Alternate));
    app.drain();
    let opened = call(&mut app, "ui.dialog.inspect", json!({}));
    assert_eq!(opened["params"]["ranges"], json!({"wght": {"min": 600.0}}), "{opened}");
    call(&mut app, "ui.dialog.set", json!({"field": "wght.min", "value": "700"}));
    call(&mut app, "ui.dialog.set", json!({"field": "allMasters", "value": true}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    app.drain();
    assert_eq!(done["params"]["allMasters"], true);
    let of = |master: &str| app.models.layers.all.iter().filter(|r| r.master == master && r.bracket.as_deref() == Some("[700‹wg]")).count();
    assert_eq!((of("m01"), of("m02")), (1, 1), "a layer under the other master");

    app.sent.push(row_command(&model, "A", row(&model, &one), "layers.setAlternate"));
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
    assert_eq!((label(&app, &one), label(&app, &other)), (None, None), "unticked on both");

    // Intermediate leaves the master rows of the selection out.
    let mut model = (*app.models.layers).clone();
    model.selection = vec!["m01".into(), one.clone(), other.clone()];
    let msg = row_command(&model, "A", row(&model, &one), "layers.setIntermediate");
    assert_eq!(msg, super::edit_special("A", &[one.as_str(), other.as_str()], super::Part::Intermediate));
}
