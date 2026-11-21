//! Behaviour tests of the Kerning window, its questions and the kerning audit and Match Kerning dialogs.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::host::Host;
use craft::ui::contract::Tracked;
use craft::ui::dialog::{DialogResult, DialogStack, StackDialog};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use egui::{Key, Modifiers};
use fontcraft_engine::Session;
use fontcraft_engine::model::Layer;
use serde_json::{Map, Value, json};

use super::{Props, show};
use crate::app::models::kerning_window::{KernRow, KerningWindowModel};
use crate::app::{APP, FontCraftApp, Msg};
use crate::dialogs::kerning_audit::AuditDialog;
use crate::dialogs::kerning_questions::{PasteConflict, RenameGroup};
use crate::shell::kerning_window::{Audit, Finding, Question};
use crate::testing::TestApp;

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

fn row(left: &str, right: &str, value: f64) -> KernRow {
    KernRow { left: left.into(), right: right.into(), value }
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
    FontCraftApp::new(session, Arc::new(Host::builder(APP).build()))
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
    app.models.kerning_window.rows.iter().map(|r| (r.left.clone(), r.right.clone(), r.value)).collect()
}

/// Typing a term, a header press, a row press and Delete post the window's commands.
#[test]
fn the_window_posts_search_sort_selection_and_delete() {
    let model = Tracked::new(KerningWindowModel {
        has_doc: true,
        master: Some("m01".into()),
        direction: "ltr".into(),
        rows: vec![row("@T", "@o", -80.0), row("A", "V", -100.0), row("L", "T", -60.0)],
        total: 3,
        selected: vec![1],
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
