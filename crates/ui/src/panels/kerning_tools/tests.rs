//! Behaviour tests of the Kerning Pairs Preview and the Kerning Groups Editor.

use std::cell::RefCell;
use std::rc::Rc;

use craft::ui::contract::Tracked;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use fontcraft_engine::Session;
use serde_json::{Value, json};

use super::{groups_editor, pairs_preview};
use crate::app::Msg;
use crate::app::models::kerning_tools::{Combination, GroupEntry, GroupsEditorModel, PairsPreviewModel, PreviewRow};
use crate::shell::kerning_tools::{EditorSide, GroupsView};
use crate::testing::TestApp;

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

/// Every glyph used 600 wide (V 640) with a rectangle; T and V in the right group T, e and o in the left group o;
/// @T @o −80 with the exception T o −30, A V −100, L T −60.
fn kerned() -> Session {
    let mut s = Session::with_document();
    let mut run = |id: &str, p: Value| {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    };
    for glyph in ["A", "L", "T", "V", "e", "o"] {
        run("path.addRectangle", json!({"glyph": glyph, "x": 50, "y": 0, "width": 500, "height": 500}));
        run("spacing.setWidth", json!({"glyphs": [glyph], "value": if glyph == "V" { 640 } else { 600 }}));
    }
    run("kerning.setRightGroup", json!({"glyphs": ["T", "V"], "group": "T"}));
    run("kerning.setLeftGroup", json!({"glyphs": ["e", "o"], "group": "o"}));
    for (left, right, value) in [("@T", "@o", -80), ("A", "V", -100), ("L", "T", -60), ("T", "o", -30)] {
        run("kerning.setPair", json!({"left": left, "right": right, "value": value}));
    }
    s
}

fn execute(t: &mut TestApp, command: &str, params: Value) -> Value {
    t.ok("engine.execute", json!({"command": command, "params": params}))
}

fn refused(t: &mut TestApp, command: &str, params: Value) -> String {
    t.error("engine.execute", json!({"command": command, "params": params}))
}

fn preview(t: &mut TestApp) -> Value {
    t.ok("ui.inspect", json!({}))["kerningPairsPreview"].clone()
}

fn editor(t: &mut TestApp) -> Value {
    t.ok("ui.inspect", json!({}))["kerningGroupsEditor"].clone()
}

fn sides(shown: &Value) -> Vec<(String, String)> {
    shown["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| (r["left"].as_str().unwrap_or("").into(), r["right"].as_str().unwrap_or("").into()))
        .collect()
}

fn pair(l: &str, r: &str) -> (String, String) {
    (l.into(), r.into())
}

/// The preview sorts by first glyph, second glyph and value, keeps one glyph's pairs, jumps to a typed character,
/// refuses bad options without changing anything, and marks a value changed since it opened.
#[test]
fn the_pairs_preview_sorts_filters_jumps_and_marks_changes() {
    let mut t = TestApp::with(kerned());
    execute(&mut t, "window.kerningPairsPreview", json!({}));
    let first = preview(&mut t);
    assert_eq!((first["open"].clone(), first["count"].clone(), first["sort"].clone()), (json!(true), json!(4), json!("first")));
    // Groups sort by the glyph they are drawn by: @T by T, @o by e.
    assert_eq!(sides(&first), vec![pair("A", "V"), pair("L", "T"), pair("@T", "@o"), pair("T", "o")]);
    execute(&mut t, "window.kerningPairsPreview", json!({"sort": "second"}));
    assert_eq!(sides(&preview(&mut t)), vec![pair("L", "T"), pair("A", "V"), pair("@T", "@o"), pair("T", "o")]);
    execute(&mut t, "window.kerningPairsPreview", json!({"sort": "value"}));
    assert_eq!(sides(&preview(&mut t)), vec![pair("A", "V"), pair("@T", "@o"), pair("L", "T"), pair("T", "o")]);
    // Only the pairs of one glyph: T itself, and the right group T it belongs to.
    execute(&mut t, "window.kerningPairsPreview", json!({"glyph": "T", "sort": "first"}));
    assert_eq!(sides(&preview(&mut t)), vec![pair("L", "T"), pair("@T", "@o"), pair("T", "o")]);
    // Typing a character scrolls to the first pair whose first glyph it encodes.
    execute(&mut t, "window.kerningPairsPreview", json!({"glyph": null, "jump": "L"}));
    assert_eq!(preview(&mut t)["scrollTo"], json!(1));

    assert!(refused(&mut t, "window.kerningPairsPreview", json!({"sort": "name"})).contains("sort"));
    assert!(refused(&mut t, "window.kerningPairsPreview", json!({"size": 50})).contains("size"));
    assert!(refused(&mut t, "window.kerningPairsPreview", json!({"glyph": "nothing"})).contains("glyph"));
    assert_eq!(preview(&mut t)["sort"], "first");

    execute(&mut t, "kerning.setPair", json!({"left": "A", "right": "V", "value": -90}));
    let rows = preview(&mut t)["rows"].clone();
    assert_eq!(rows[0], json!({"left": "A", "right": "V", "value": -90.0, "changed": true, "was": -100.0}));
    assert_eq!(rows[1]["changed"], false);

    execute(&mut t, "window.kerningPairsPreview", json!({"open": false}));
    assert_eq!(preview(&mut t)["open"], false);
}

/// A selected pair with a group side lists every glyph combination with the pair that applies to it.
#[test]
fn the_pairs_preview_lists_the_combinations_of_a_group_pair() {
    let mut t = TestApp::with(kerned());
    execute(&mut t, "window.kerningPairsPreview", json!({"select": {"left": "@T", "right": "@o"}}));
    let shown = preview(&mut t);
    let combos: Vec<(String, String, Value, Value)> = shown["combinations"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| (c["first"].as_str().unwrap_or("").into(), c["second"].as_str().unwrap_or("").into(), c["value"].clone(), c["pair"].clone()))
        .collect();
    let group = json!({"left": "@T", "right": "@o"});
    assert_eq!(
        combos,
        vec![
            ("T".into(), "e".into(), json!(-80.0), group.clone()),
            ("T".into(), "o".into(), json!(-30.0), json!({"left": "T", "right": "o"})),
            ("V".into(), "e".into(), json!(-80.0), group.clone()),
            ("V".into(), "o".into(), json!(-80.0), group),
        ]
    );
    execute(&mut t, "window.kerningPairsPreview", json!({"select": {"left": "T", "right": "o"}}));
    let picked = preview(&mut t);
    assert_eq!(picked["selected"], json!({"left": "T", "right": "o"}));
    assert_eq!(picked["combinations"], json!([]), "a glyph pair lists no combinations");
}

/// The editor lists one side's groups, stacks the members on the group's side, starts new groups, refuses bad names
/// and proposes groups for review.
#[test]
fn the_groups_editor_stacks_members_on_the_group_side() {
    let mut t = TestApp::with(kerned());
    execute(&mut t, "dialog.kerningGroupsEditor", json!({"side": "right", "group": "T", "view": "stacked"}));
    let right = editor(&mut t);
    assert_eq!((right["open"].clone(), right["members"].clone(), right["view"].clone()), (json!(true), json!(["T", "V"]), json!("stacked")));
    assert_eq!(right["groups"], json!([{"name": "T", "count": 2}]));
    // A right group lines its members up on their advance widths.
    assert_eq!(right["stacked"], json!({"aligned": "advance", "glyphs": [{"name": "T", "offset": -600.0}, {"name": "V", "offset": -640.0}]}));
    // Changing the side drops the current group; a left group lines its members up on their origins.
    execute(&mut t, "dialog.kerningGroupsEditor", json!({"side": "left"}));
    let left = editor(&mut t);
    assert_eq!((left["group"].clone(), left["members"].clone()), (Value::Null, json!([])));
    execute(&mut t, "dialog.kerningGroupsEditor", json!({"group": "o"}));
    assert_eq!(editor(&mut t)["stacked"], json!({"aligned": "origin", "glyphs": [{"name": "e", "offset": 0.0}, {"name": "o", "offset": 0.0}]}));
    // A new name starts an empty group; a bad name and an apply without a proposal are refused.
    execute(&mut t, "dialog.kerningGroupsEditor", json!({"group": "round"}));
    assert_eq!(editor(&mut t)["members"], json!([]));
    assert!(refused(&mut t, "dialog.kerningGroupsEditor", json!({"group": "two words"})).contains("group"));
    assert!(refused(&mut t, "dialog.kerningGroupsEditor", json!({"apply": true})).contains("no proposal"));
    assert_eq!(editor(&mut t)["group"], "round");

    execute(&mut t, "dialog.kerningGroupsEditor", json!({"propose": {"maxDifference": 20, "side": "left"}}));
    assert!(editor(&mut t)["proposal"].is_object(), "{}", editor(&mut t));
    execute(&mut t, "dialog.kerningGroupsEditor", json!({"propose": null}));
    assert_eq!(editor(&mut t)["proposal"], Value::Null);
}

fn row(left: &str, right: &str, value: f64) -> PreviewRow {
    PreviewRow { left: left.into(), right: right.into(), value, first: left.into(), second: right.into(), ..Default::default() }
}

fn fixture_of<M: 'static>(model: M, show: fn(&mut egui::Ui, &M, &mut Vec<Msg>)) -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, &model, &mut sent);
        seen.borrow_mut().extend(sent);
    });
    fixture.room(egui::vec2(720.0, 1200.0)).settle();
    (fixture, posted)
}

fn show_preview(ui: &mut egui::Ui, model: &Tracked<PairsPreviewModel>, sent: &mut Vec<Msg>) {
    pairs_preview::show(ui, pairs_preview::Props { model, sent });
}

fn show_editor(ui: &mut egui::Ui, model: &Tracked<GroupsEditorModel>, sent: &mut Vec<Msg>) {
    groups_editor::show(ui, groups_editor::Props { model, sent });
}

/// Clicking a pair selects it, the step buttons change the selected pair's value, and clicking a combination selects
/// the pair that applies to it.
#[test]
fn the_pairs_preview_posts_selection_value_steps_and_combination_picks() {
    let model = Tracked::new(PairsPreviewModel {
        has_font: true,
        master: Some("m01".into()),
        rows: vec![row("A", "V", -100.0), row("L", "T", -60.0)],
        selected: Some(0),
        selected_pair: Some(pair("A", "V")),
        ..Default::default()
    });
    let (mut fixture, posted) = fixture_of(model, show_preview);
    let l = fixture.by_label("L T").expect("the L T tile");
    fixture.click(&l).settle();
    assert!(posted.borrow().contains(&shell("window.kerningPairsPreview", json!({"select": {"left": "L", "right": "T"}}))), "{:?}", posted.borrow());
    let plus = fixture.by_label("+10").expect("the +10 button");
    fixture.click(&plus).settle();
    assert!(
        posted.borrow().contains(&Msg::run("kerning.setPair", json!({"left": "A", "right": "V", "value": -90.0, "master": "m01"}))),
        "{:?}",
        posted.borrow()
    );

    let combination = |first: &str, second: &str, value: f64, pair: (&str, &str)| Combination {
        first: first.into(),
        second: second.into(),
        value,
        pair: Some((pair.0.into(), pair.1.into())),
        step: None,
    };
    let model = Tracked::new(PairsPreviewModel {
        has_font: true,
        rows: vec![row("@T", "@o", -80.0)],
        selected: Some(0),
        selected_pair: Some(pair("@T", "@o")),
        combinations: vec![combination("T", "e", -80.0, ("@T", "@o")), combination("T", "o", -30.0, ("T", "o"))],
        ..Default::default()
    });
    let (mut fixture, posted) = fixture_of(model, show_preview);
    let to = fixture.by_label("T o").expect("the T o combination");
    fixture.click(&to).settle();
    assert!(posted.borrow().contains(&shell("window.kerningPairsPreview", json!({"select": {"left": "T", "right": "o"}}))), "{:?}", posted.borrow());
}

fn groups_model(view: GroupsView) -> Tracked<GroupsEditorModel> {
    Tracked::new(GroupsEditorModel {
        has_font: true,
        side: EditorSide::Left,
        view,
        group: Some("o".into()),
        groups: vec![GroupEntry { name: "n".into(), count: 1 }, GroupEntry { name: "o".into(), count: 2 }],
        members: vec!["e".into(), "o".into()],
        glyphs: vec!["A".into(), "e".into(), "n".into(), "o".into()],
        ascender: 800.0,
        descender: -200.0,
        ..Default::default()
    })
}

/// Picking a group, Clear Group, Remove from Group and a glyph dragged onto the members post the editor's commands.
#[test]
fn the_groups_editor_posts_group_picks_member_changes_and_drops() {
    let (mut fixture, posted) = fixture_of(groups_model(GroupsView::List), show_editor);
    let n = fixture.by_label("n (1)").expect("the n group");
    fixture.click(&n).settle();
    assert!(posted.borrow().contains(&shell("dialog.kerningGroupsEditor", json!({"group": "n"}))), "{:?}", posted.borrow());
    let clear = fixture.by_label("Clear Group").expect("Clear Group");
    fixture.click(&clear).settle();
    assert!(posted.borrow().contains(&Msg::run("kerning.setLeftGroup", json!({"glyphs": ["e", "o"], "group": null}))), "{:?}", posted.borrow());
    posted.borrow_mut().clear();
    let e = fixture.by_label("e").expect("the e member");
    fixture.click(&e).settle();
    let remove = fixture.by_label("Remove from Group").expect("Remove from Group");
    fixture.click(&remove).settle();
    assert!(posted.borrow().contains(&Msg::run("kerning.setLeftGroup", json!({"glyphs": ["e"], "group": null}))), "{:?}", posted.borrow());

    let (mut fixture, posted) = fixture_of(groups_model(GroupsView::Cells), show_editor);
    let a = fixture.by_label("A").expect("the A glyph");
    let member = fixture.by_label("e").expect("the e member");
    fixture.drag(&a, member.center() - a.center()).settle();
    assert!(posted.borrow().contains(&Msg::run("kerning.setLeftGroup", json!({"glyphs": ["A"], "group": "o"}))), "{:?}", posted.borrow());
}
