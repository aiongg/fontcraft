//! The Axis Mappings dialog as an agent drives it (`axisMappings.*`, `ui.inspect`) and as the person does (its view).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::host::Host;
use craft::ui::testing::Fixture;
use egui::{Key, Modifiers};
use fontcraft_engine::Session;
use fontcraft_model::{Axis, Font, Instance, Master};
use serde_json::{Value, json};

use super::{AxisModel, EDIT, Graph, GraphDrag, new_point_internal, view};
use crate::app::{APP, FontCraftApp, Msg};
use crate::control;

fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
    control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
}

fn execute(app: &mut FontCraftApp, command: &str, params: Value) -> Value {
    call(app, "engine.execute", json!({"command": command, "params": params}));
    call(app, "ui.inspect", json!({}))["axisMappings"].clone()
}

fn refused(app: &mut FontCraftApp, command: &str, params: Value) -> String {
    control::dispatch(app, "engine.execute", &json!({"command": command, "params": params})).expect_err(&format!("{command} should fail"))
}

fn font_of(app: &FontCraftApp) -> Arc<Font> {
    app.session.doc().unwrap().font.clone()
}

fn undo_steps(app: &FontCraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// Weight 30 to 120 and width 100, two masters, a static instance Regular and a variable font setting VF; wght maps
/// 30 → 30 and 120 → 120, wdth has no entry.
fn mapped() -> FontCraftApp {
    let mut f = Font::new("Axis Mappings");
    f.axes = Arc::new(vec![
        Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() },
        Axis { name: "Width".into(), tag: "wdth".into(), ..Axis::default() },
    ]);
    let template = f.masters[0].clone();
    f.masters = Arc::new(vec![
        Master { name: "Light".into(), location: vec![30.0, 100.0], ..template.clone() },
        Master { id: "bold".into(), name: "Bold".into(), location: vec![120.0, 100.0], ..template },
    ]);
    f.instances = Arc::new(vec![
        Instance { name: "Regular".into(), location: vec![75.0, 100.0], ..Instance::default() },
        Instance { name: "VF".into(), kind: Some("variable".into()), ..Instance::default() },
    ]);
    let mut session = Session::new();
    session.add_document(f, None).unwrap();
    let mut app = FontCraftApp::with_edit_tab(session, Arc::new(Host::builder(APP).build()));
    call(
        &mut app,
        "engine.execute",
        json!({"command": "font.customParameters.add", "params": {"scope": "font", "name": "Axis Mappings", "value": {"wght": {"30": 30, "120": 120}}}}),
    );
    app
}

#[test]
fn the_axis_mappings_dialog_previews_its_edits_and_ok_keeps_one_undo_step() {
    let mut app = mapped();
    let steps = undo_steps(&app);
    let shown = execute(&mut app, "axisMappings.showDialog", json!({}));
    assert_eq!((shown["open"].clone(), shown["axis"].clone()), (json!(true), json!("wght")));
    assert_eq!(shown["axes"].as_array().unwrap().len(), 2);
    assert_eq!(shown["axes"][0]["points"], json!([{"internal": 30.0, "external": 30.0}, {"internal": 120.0, "external": 120.0}]));

    // While the dialog is open, other edits wait.
    assert!(!refused(&mut app, "glyph.newGlyph", json!({"name": "x"})).is_empty());

    let added = execute(&mut app, "axisMappings.dialogEdit", json!({"command": "axisMappings.addPoint", "params": {"internal": 75}}));
    assert_eq!(added["axes"][0]["points"][1], json!({"internal": 75.0, "external": 75.0}), "the preview shows the new point, on the line");
    assert_eq!(added["point"], 1, "the new point is selected");
    execute(&mut app, "axisMappings.dialogEdit", json!({"command": "axisMappings.nudgePoint", "params": {"dExternal": 10}}));
    let nudged = execute(&mut app, "axisMappings.dialogEdit", json!({"command": "axisMappings.nudgePoint", "params": {"dExternal": 5}}));
    assert_eq!(nudged["edits"], 2, "moves of one point merge into one step");

    // An edit that breaks the order changes nothing; neither does a refused request.
    let error = refused(&mut app, "axisMappings.dialogEdit", json!({"command": "axisMappings.setPoint", "params": {"external": 1_000_000}}));
    assert!(!error.is_empty());
    assert_eq!(execute(&mut app, "axisMappings.dialogEdit", json!({})), nudged, "the refused edit left the dialog as it was");
    assert!(!refused(&mut app, "axisMappings.dialogEdit", json!({"axis": "wdth", "command": "axisMappings.nope"})).is_empty());
    assert_eq!(execute(&mut app, "axisMappings.dialogEdit", json!({})), nudged, "a refused request keeps the axis it was on");
    assert_eq!(execute(&mut app, "axisMappings.dialogEdit", json!({"point": null}))["point"], json!(null), "null clears the selection");

    let done = execute(&mut app, "axisMappings.closeDialog", json!({"confirm": true}));
    assert_eq!(done, json!({"open": false}));
    assert_eq!(undo_steps(&app), steps + 1, "OK makes one undo step");
    assert!(app.session.interaction().is_none());

    let mut direct = mapped();
    for (command, params) in [
        ("axisMappings.addPoint", json!({"axis": "wght", "internal": 75})),
        ("axisMappings.nudgePoint", json!({"axis": "wght", "index": 1, "dExternal": 15})),
    ] {
        call(&mut direct, "engine.execute", json!({"command": command, "params": params}));
    }
    assert_eq!(font_of(&app).info.params, font_of(&direct).info.params, "the dialog gives the font its commands give");

    // Cancel gives back the very font the dialog opened on, also for a variable font setting.
    let original = font_of(&app);
    execute(&mut app, "axisMappings.showDialog", json!({"variableFontSetting": "VF", "axis": "wdth"}));
    execute(&mut app, "axisMappings.dialogEdit", json!({"command": "axisMappings.addPoint", "params": {"internal": 110, "external": 110}}));
    assert_eq!(execute(&mut app, "axisMappings.closeDialog", json!({})), json!({"open": false}));
    assert!(Arc::ptr_eq(&font_of(&app), &original));
    assert!(refused(&mut app, "axisMappings.showDialog", json!({"variableFontSetting": "Regular"})).contains("static"));
    assert!(app.session.interaction().is_none(), "a refused opening begins nothing");
}

#[test]
fn the_font_info_button_opens_the_dialog_and_the_window_closes_with_it() {
    let mut app = mapped();
    // The Font Info row's button posts this shell command; it must name a defined command.
    app.sent.push(Msg::Shell(craft::ui::runtime::commands::CommandRef::with_params("axisMappings.showDialog", json!({}))));
    app.drain();
    assert!(app.view.axis_mappings.open, "{}", app.status);
    let model = AxisModel::of(&app).unwrap();
    assert_eq!(model.axis, "wght");
    assert_eq!(model.points, vec![(30.0, 30.0), (120.0, 120.0)]);
    assert_eq!(model.setting, None);

    // An undo ends the interaction elsewhere: the dialog has nothing left to edit and closes.
    app.session.cancel().unwrap();
    let mut host = craft::ui::dialog::DialogHost::new();
    super::run_dialog(&mut host, &mut app);
    assert!(!app.view.axis_mappings.open);
}

#[test]
fn plus_adds_a_point_halfway_to_the_next() {
    let points = [(30.0, 30.0), (80.0, 70.0), (120.0, 120.0)];
    assert_eq!(new_point_internal(&points, Some(0)), 50.0);
    assert_eq!(new_point_internal(&points, Some(2)), 95.0, "after the last point, halfway back to the one before");
    assert_eq!(new_point_internal(&points, None), 50.0);
    assert_eq!(new_point_internal(&[], None), 0.0);
}

#[test]
fn the_dialog_view_posts_its_edits() {
    let model = AxisModel {
        setting: None,
        axes: vec![("wght".into(), "Weight".into()), ("wdth".into(), "Width".into())],
        axis: "wght".into(),
        points: vec![(30.0, 30.0), (120.0, 120.0)],
        point: Some(1),
    };
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut drag = GraphDrag::default();
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        view(ui, &model, &mut drag, &mut sent);
        seen.borrow_mut().extend(sent);
    });
    fixture.settle();
    let edit = |params: Value| Msg::Shell(craft::ui::runtime::commands::CommandRef::with_params(EDIT, params));

    let external = fixture.by_label("External Coordinate").expect("the External Coordinate field");
    fixture.click(&external).key(Key::A, Modifiers::COMMAND).type_text("90").key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(
        posted.borrow().last(),
        Some(&edit(json!({"command": "axisMappings.setPoint", "params": {"index": 1, "external": 90.0}}))),
        "the field moves the selected point"
    );

    let plus = fixture.by_label("+").expect("the + button");
    fixture.click(&plus).settle();
    assert_eq!(
        posted.borrow().last(),
        Some(&edit(json!({"command": "axisMappings.addPoint", "params": {"internal": 75.0}}))),
        "+ adds a point halfway back to the point before the selected last one"
    );
    let minus = fixture.by_label("−").expect("the − button");
    fixture.click(&minus).settle();
    assert_eq!(posted.borrow().last(), Some(&edit(json!({"command": "axisMappings.removePoint", "params": {}}))));
    assert!(fixture.by_label("Mapping graph: internal coordinates across, external coordinates up").is_some(), "the graph");
}

/// The arrows move the selected point while no field has focus: Shift for ten units, Up and Down the external value.
/// A focused coordinate field keeps them to step its own value.
#[test]
fn arrow_keys_move_the_selected_point_while_no_field_has_focus() {
    let model = AxisModel {
        setting: None,
        axes: vec![("wght".into(), "Weight".into())],
        axis: "wght".into(),
        points: vec![(30.0, 30.0), (120.0, 120.0)],
        point: Some(0),
    };
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut drag = GraphDrag::default();
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        view(ui, &model, &mut drag, &mut sent);
        seen.borrow_mut().extend(sent);
    });
    fixture.settle();
    let nudge = |d_internal: f64, d_external: f64| {
        Msg::Shell(craft::ui::runtime::commands::CommandRef::with_params(
            EDIT,
            json!({"command": "axisMappings.nudgePoint", "params": {"dInternal": d_internal, "dExternal": d_external}}),
        ))
    };
    fixture.key(Key::ArrowRight, Modifiers::SHIFT).settle();
    fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
    assert_eq!(*posted.borrow(), vec![nudge(10.0, 0.0), nudge(0.0, 1.0)]);

    let external = fixture.by_label("External Coordinate").expect("the External Coordinate field");
    fixture.click(&external).settle();
    posted.borrow_mut().clear();
    fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
    assert!(!posted.borrow().iter().any(|m| *m == nudge(0.0, 1.0)), "a focused field keeps the arrows: {:?}", posted.borrow());
}

/// Page Down, Page Up, Home and End select the graph's points and Delete removes one, with no pointer; a focused field
/// keeps its keys, and the points are named for assistive technology.
#[test]
fn the_graphs_points_are_selected_and_removed_with_the_keyboard() {
    let point = Rc::new(RefCell::new(None));
    let posted = Rc::new(RefCell::new(Vec::new()));
    let (seen, at) = (Rc::clone(&posted), Rc::clone(&point));
    let mut drag = GraphDrag::default();
    let mut fixture = Fixture::new(move |ui| {
        let points = vec![(30.0, 30.0), (60.0, 80.0), (120.0, 120.0)];
        let axes = vec![("wght".into(), "Weight".into())];
        let model = AxisModel { setting: None, axes, axis: "wght".into(), points, point: *at.borrow() };
        let mut sent = Vec::new();
        view(ui, &model, &mut drag, &mut sent);
        seen.borrow_mut().extend(sent);
    });
    fixture.settle();
    let edit = |params: Value| Msg::Shell(craft::ui::runtime::commands::CommandRef::with_params(EDIT, params));
    let remove = edit(json!({"command": "axisMappings.removePoint", "params": {}}));
    fixture.key(Key::Delete, Modifiers::NONE).settle();
    assert!(posted.borrow().is_empty(), "nothing is selected to remove: {:?}", posted.borrow());
    for (from, key, to) in [
        (None, Key::PageDown, Some(0)),
        (Some(0), Key::PageDown, Some(1)),
        (Some(1), Key::PageUp, Some(0)),
        (Some(0), Key::PageUp, None),
        (Some(0), Key::End, Some(2)),
        (Some(2), Key::Home, Some(0)),
    ] {
        *point.borrow_mut() = from;
        fixture.settle();
        posted.borrow_mut().clear();
        fixture.key(key, Modifiers::NONE).settle();
        let expected: Vec<Msg> = to.map(|at| edit(json!({"point": at}))).into_iter().collect();
        assert_eq!(*posted.borrow(), expected, "{key:?} from {from:?}");
    }
    *point.borrow_mut() = Some(1);
    fixture.settle();
    posted.borrow_mut().clear();
    fixture.key(Key::Delete, Modifiers::NONE).settle();
    assert_eq!(*posted.borrow(), vec![remove.clone()]);
    assert!(fixture.by_label("Point 2").is_some(), "the second point is named");

    let external = fixture.by_label("External Coordinate").expect("the External Coordinate field");
    fixture.click(&external).settle();
    posted.borrow_mut().clear();
    fixture.key(Key::Backspace, Modifiers::NONE).settle();
    assert!(!posted.borrow().contains(&remove), "a focused field keeps its keys: {:?}", posted.borrow());
}

/// In the graph a click on a point selects it, a click on the line adds a point there, and a drag moves a point's
/// external value, posted once at the release.
#[test]
fn the_graph_selects_adds_and_drags_points() {
    let points = vec![(30.0, 30.0), (120.0, 120.0)];
    let model =
        AxisModel { setting: None, axes: vec![("wght".into(), "Weight".into())], axis: "wght".into(), points: points.clone(), point: Some(1) };
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut drag = GraphDrag::default();
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        view(ui, &model, &mut drag, &mut sent);
        seen.borrow_mut().extend(sent);
    });
    fixture.room(egui::vec2(1200.0, 800.0)).settle();
    let graph = fixture.by_label("Mapping graph: internal coordinates across, external coordinates up").expect("the graph");
    let g = Graph::new(graph.rect, &points);
    let edit = |params: Value| Msg::Shell(craft::ui::runtime::commands::CommandRef::with_params(EDIT, params));

    fixture.click_at(g.at((30.0, 30.0))).settle();
    assert_eq!(*posted.borrow(), vec![edit(json!({"point": 0}))], "a click on a point selects it");

    posted.borrow_mut().clear();
    fixture.click_at(g.at((75.0, 75.0))).settle();
    assert_eq!(*posted.borrow(), vec![edit(json!({"command": "axisMappings.addPoint", "params": {"internal": 75.0}}))]);

    posted.borrow_mut().clear();
    let from = g.at((120.0, 120.0));
    fixture.drag_at(from, egui::vec2(0.0, 30.0)).settle();
    let external = g.external_at(from.y + 30.0).round();
    assert!(external < 120.0);
    assert_eq!(*posted.borrow(), vec![edit(json!({"command": "axisMappings.setPoint", "params": {"index": 1, "external": external}}))]);
}
