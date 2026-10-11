//! The Kerning window's questions and Show Group Members: each dialog gives the font its command
//! gives with the same answer, Cancel leaves the very font the dialog opened on, and the group
//! members drawn are those of the sides facing the edited pair.

use std::sync::Arc;

use egui::{Pos2, Shape};
use fontcraft_model::Font;
use fontcraft_spacing::kerning::GroupSide;
use serde_json::{Value, json};

use crate::canvas::{group_members, view};
use crate::{FontcraftApp, control};

fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
    assert_eq!(r["ok"], true, "{method}: {r}");
    r["result"].clone()
}

fn fails(app: &mut FontcraftApp, method: &str, params: Value) -> String {
    let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
    assert_eq!(r["ok"], false, "{method} should fail: {r}");
    r["error"].as_str().unwrap_or_default().to_string()
}

fn inspect(app: &mut FontcraftApp) -> Value {
    call(app, "ui.dialog.inspect", json!({}))
}

fn open(app: &mut FontcraftApp, dialog: &str, params: Value) -> Value {
    call(app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": dialog, "params": params}}))
}

fn font(app: &FontcraftApp) -> Arc<Font> {
    app.session.doc().unwrap().font.clone()
}

/// T (600 wide, right group T), Tbar (700 wide, right group T), o (600 wide), c (500 wide) and
/// e (550 wide), the last three in the left group o; c and o also share the right group o. The
/// pair T–@o is −60.
fn kerned_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("glyph.newGlyph", &json!({"name": "Tbar"})).unwrap();
    for (glyph, width) in [("T", 600), ("Tbar", 700), ("o", 600), ("c", 500), ("e", 550)] {
        app.run("path.addRectangle", &json!({"glyph": glyph, "x": 50, "y": 0, "width": width - 100, "height": 500})).unwrap();
        app.run("spacing.setWidth", &json!({"glyphs": [glyph], "value": width})).unwrap();
    }
    app.run("kerning.setRightGroup", &json!({"glyphs": ["T", "Tbar"], "group": "T"})).unwrap();
    app.run("kerning.setLeftGroup", &json!({"glyphs": ["o", "c", "e"], "group": "o"})).unwrap();
    app.run("kerning.setRightGroup", &json!({"glyphs": ["o", "c"], "group": "o"})).unwrap();
    app.run("kerning.setPair", &json!({"left": "T", "right": "@o", "value": -60})).unwrap();
    app
}

#[test]
fn rename_kerning_group_dialog_equals_kerning_rename_group_for_both_scopes() {
    let asked = json!({"side": "right", "from": "o", "to": "round", "pair": {"left": "T", "right": "@o"}});
    let mut app = kerned_app();
    let before = font(&app);
    let opened = open(&mut app, "dialog.renameKerningGroup", asked.clone());
    assert_eq!(opened["dialog"], "dialog.renameKerningGroup", "{opened}");
    assert_eq!(opened["buttons"], json!(["ok", "thisPair", "cancel"]));
    assert_eq!(opened["captions"], json!({"ok": "All Pairs", "thisPair": "This Pair", "cancel": "Cancel"}));
    assert_eq!(opened["fields"][0]["value"], "round", "the typed name is the first value: {opened}");
    assert!(Arc::ptr_eq(&font(&app), &before), "asking changes nothing");

    // All Pairs: the group is renamed on every glyph and pair.
    call(&mut app, "ui.click", json!({"id": "ok"}));
    assert_eq!(inspect(&mut app)["open"], false);
    let mut direct = kerned_app();
    let mut params = asked.clone();
    params["scope"] = json!("font");
    direct.run("kerning.renameGroup", &params).unwrap();
    assert_eq!(font(&app).glyph("c").unwrap().kern_groups.left.as_deref(), Some("round"));
    assert!(font(&app).same_as(&font(&direct)), "All Pairs gives the font scope font gives");

    // This Pair: only the pair's side points at the new name.
    let mut app = kerned_app();
    open(&mut app, "dialog.renameKerningGroup", asked.clone());
    call(&mut app, "ui.click", json!({"id": "thisPair"}));
    let mut direct = kerned_app();
    params["scope"] = json!("pair");
    direct.run("kerning.renameGroup", &params).unwrap();
    assert_eq!(font(&app).glyph("c").unwrap().kern_groups.left.as_deref(), Some("o"), "the glyphs keep their group");
    assert!(font(&app).same_as(&font(&direct)), "This Pair gives the font scope pair gives");

    // The name can be changed in the dialog; Cancel renames nothing.
    let mut app = kerned_app();
    let before = font(&app);
    open(&mut app, "dialog.renameKerningGroup", asked.clone());
    call(&mut app, "ui.dialog.set", json!({"field": "to", "value": "bowl"}));
    assert_eq!(inspect(&mut app)["params"]["to"], "bowl");
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert!(Arc::ptr_eq(&font(&app), &before), "Cancel renames nothing");

    // A name taken on that side refuses All Pairs and keeps the question open for This Pair.
    let mut app = kerned_app();
    app.run("kerning.setLeftGroup", &json!({"glyphs": ["T"], "group": "taken"})).unwrap();
    let before = font(&app);
    open(&mut app, "dialog.renameKerningGroup", json!({"side": "right", "from": "o", "to": "taken", "pair": {"left": "T", "right": "@o"}}));
    fails(&mut app, "ui.dialog.confirm", json!({}));
    assert!(Arc::ptr_eq(&font(&app), &before));
    assert_eq!(inspect(&mut app)["dialog"], "dialog.renameKerningGroup", "the question stays open");
    call(&mut app, "ui.dialog.cancel", json!({}));

    // Without a side and a group the question cannot be asked.
    let error = fails(
        &mut app,
        "engine.execute",
        json!({"command": "dialog.open", "params": {"dialog": "dialog.renameKerningGroup", "params": {"from": "o"}}}),
    );
    assert!(error.contains("side"), "{error}");
}

#[test]
fn paste_kerning_conflict_dialog_follows_an_asking_paste_and_equals_its_answers() {
    let pasted = json!({"pairs": [{"left": "T", "right": "@o", "value": -80}, {"left": "Tbar", "right": "e", "value": -20}]});
    let mut ask = pasted.clone();
    ask["onConflict"] = json!("ask");

    // An agent's paste only reports the conflict.
    let mut app = kerned_app();
    let before = font(&app);
    let r = call(&mut app, "engine.execute", json!({"command": "kerning.pastePairs", "params": ask}));
    assert_eq!((r["asked"].clone(), r["conflictCount"].clone()), (json!(true), json!(1)), "{r}");
    assert_eq!(inspect(&mut app)["open"], false, "engine.execute never asks");
    assert!(Arc::ptr_eq(&font(&app), &before));

    // The person's paste asks; Overwrite.
    let result = app.run_from_shell("kerning.pastePairs", &ask).unwrap();
    assert_eq!(result["asked"], true);
    let asked = inspect(&mut app);
    assert_eq!(asked["dialog"], "dialog.pasteKerningConflict", "{asked}");
    assert_eq!(asked["fields"][0]["value"], json!(["T  @o  -80 (now -60)"]), "the conflicting pair is listed: {asked}");
    assert_eq!(asked["fields"][0]["label"], "1 pair exists with another value");
    assert_eq!(asked["buttons"], json!(["ok", "keepExisting", "cancel"]));
    assert_eq!(asked["captions"], json!({"ok": "Overwrite", "keepExisting": "Keep Existing", "cancel": "Cancel"}));
    assert!(Arc::ptr_eq(&font(&app), &before), "asking pastes nothing");
    call(&mut app, "ui.click", json!({"id": "ok"}));
    let mut direct = kerned_app();
    let mut params = pasted.clone();
    params["onConflict"] = json!("overwrite");
    direct.run("kerning.pastePairs", &params).unwrap();
    assert_eq!(app.run("kerning.getPair", &json!({"left": "T", "right": "o"})).unwrap()["value"].as_f64(), Some(-80.0));
    assert!(font(&app).same_as(&font(&direct)), "Overwrite gives the font onConflict overwrite gives");
    assert_eq!(inspect(&mut app)["open"], false, "the answered paste asks nothing more");

    // Keep Existing.
    let mut app = kerned_app();
    app.run_from_shell("kerning.pastePairs", &ask).unwrap();
    call(&mut app, "ui.click", json!({"id": "keepExisting"}));
    let mut direct = kerned_app();
    params["onConflict"] = json!("keepExisting");
    direct.run("kerning.pastePairs", &params).unwrap();
    assert_eq!(app.run("kerning.getPair", &json!({"left": "T", "right": "o"})).unwrap()["value"].as_f64(), Some(-60.0));
    assert!(font(&app).same_as(&font(&direct)), "Keep Existing gives the font onConflict keepExisting gives");

    // Cancel pastes nothing at all.
    let mut app = kerned_app();
    let before = font(&app);
    app.run_from_shell("kerning.pastePairs", &ask).unwrap();
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert!(Arc::ptr_eq(&font(&app), &before), "Cancel pastes nothing");

    // Opened directly on pairs that do not conflict, the question still changes nothing until OK.
    let mut app = kerned_app();
    let before = font(&app);
    let opened = open(&mut app, "dialog.pasteKerningConflict", json!({"pairs": [{"left": "e", "right": "T", "value": 10}]}));
    assert_eq!(opened["fields"][0]["value"], json!([]), "{opened}");
    assert!(Arc::ptr_eq(&font(&app), &before), "listing the conflicts pastes nothing");
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(app.run("kerning.getPair", &json!({"left": "e", "right": "T"})).unwrap()["value"].as_f64(), Some(10.0));
}

/// A tab in text mode showing T o with the cursor before o, so o is edited and T precedes it.
fn group_members_app(direction: &str) -> FontcraftApp {
    let mut app = kerned_app();
    app.run("view.newTab", &json!({"glyphs": ["T", "o"]})).unwrap();
    app.run("editView.toggleTextMode", &json!({"mode": "text"})).unwrap();
    app.run("editView.setWritingDirection", &json!({"direction": direction})).unwrap();
    app.run("editView.setCursor", &json!({"index": 1})).unwrap();
    app
}

/// The members shown, as (glyph, item, side, x).
fn members(app: &mut FontcraftApp) -> Vec<(String, usize, GroupSide, f64)> {
    let tab = app.state.tab().unwrap().clone();
    app.layouts.prepare(&mut app.session, &tab);
    let layout = view::text_layout(&app.session, &app.layouts, &tab).unwrap();
    let font = font(app);
    group_members::shown(&font, &tab, &layout).into_iter().map(|m| (m.glyph, m.item, m.side, m.x)).collect()
}

fn item_x(app: &mut FontcraftApp, item: usize) -> (f64, f64) {
    let tab = app.state.tab().unwrap().clone();
    app.layouts.prepare(&mut app.session, &tab);
    let layout = view::text_layout(&app.session, &app.layouts, &tab).unwrap();
    let p = layout.placed.get(item).unwrap();
    (p.x, p.advance)
}

/// The lines of one frame drawn in the group members colour.
fn member_lines(app: &mut FontcraftApp) -> usize {
    let ctx = egui::Context::default();
    let input = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut count = 0;
    for _ in 0..2 {
        let mut output = ctx.run_ui(input(), |ui| app.ui(ui));
        output.textures_delta.clear();
        let mut out: Vec<Shape> = output.shapes.into_iter().map(|c| c.shape).collect();
        count = 0;
        while let Some(shape) = out.pop() {
            match shape {
                Shape::Vec(list) => out.extend(list),
                Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(app.tokens.group_members) => count += 1,
                _ => {}
            }
        }
    }
    count
}

#[test]
fn view_show_group_members_draws_the_facing_groups_of_the_edited_pair() {
    let mut app = group_members_app("ltr");
    assert_eq!(member_lines(&mut app), 0, "off at first");
    assert_eq!(call(&mut app, "engine.execute", json!({"command": "view.showGroupMembers", "params": {}}))["on"], true, "omitted: toggle");
    assert_eq!(app.run("view.showGroupMembers", &json!({"on": true})).unwrap()["on"], true);
    assert_eq!(app.run("view.getDisplayOptions", &json!({})).unwrap()["groupMembers"], true, "a display option like the others");

    // kerning.groupMembersDisplayScope: T shows its right group, o its left group.
    let shown = members(&mut app);
    let (t_x, t_advance) = item_x(&mut app, 0);
    let (o_x, _) = item_x(&mut app, 1);
    assert_eq!(
        shown,
        vec![
            ("Tbar".to_string(), 0, GroupSide::Right, t_x + t_advance - 700.0),
            ("c".to_string(), 1, GroupSide::Left, o_x),
            ("e".to_string(), 1, GroupSide::Left, o_x),
        ],
        "each member's facing side lines up with the glyph it stands in for"
    );
    assert_eq!(member_lines(&mut app), 3, "one closed outline per member");
    app.run("view.showGroupMembers", &json!({"on": false})).unwrap();
    assert_eq!(member_lines(&mut app), 0);

    // Right to left, o follows T on the left and faces it with its right side; T faces it with
    // its left side, and has no left group.
    let mut app = group_members_app("rtl");
    let (o_x, o_advance) = item_x(&mut app, 1);
    assert_eq!(members(&mut app), vec![("c".to_string(), 1, GroupSide::Right, o_x + o_advance - 500.0)]);

    // The first glyph of a line has nothing before it.
    let mut app = group_members_app("ltr");
    app.run("editView.setCursor", &json!({"index": 0})).unwrap();
    assert!(members(&mut app).is_empty(), "T is edited and has no left group: {:?}", members(&mut app));
}
