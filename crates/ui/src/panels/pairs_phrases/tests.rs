//! Kerning review: the pair lists, stepping and selecting rows, the list mode, and auto-kerning a list for review.

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
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::shell::dock::PanelId;

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

/// T and V in the right group T, e and o in the left group o; @T @o −80, A V −100, L T −60; the panel open.
fn kern_app() -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::run("kerning.setRightGroup", json!({"glyphs": ["T", "V"], "group": "T"})));
    app.sent.push(Msg::run("kerning.setLeftGroup", json!({"glyphs": ["e", "o"], "group": "o"})));
    for (left, right, value) in [("@T", "@o", -80), ("A", "V", -100), ("L", "T", -60)] {
        app.sent.push(Msg::run("kerning.setPair", json!({"left": left, "right": right, "value": value})));
    }
    app.sent.push(Msg::View(ViewMsg::TogglePanel(PanelId::PairsAndPhrases)));
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
    app
}

fn run(app: &mut FontCraftApp, id: &str, params: Value) -> Result<(), String> {
    app.status.clear();
    app.sent.push(shell(id, params));
    app.drain();
    if app.status.is_empty() { Ok(()) } else { Err(app.status.clone()) }
}

#[test]
fn kerning_load_pair_list_and_step_pair_list_walk_the_rows() {
    let mut app = kern_app();
    run(&mut app, "kerning.loadPairList", json!({"text": "# work\nA V\n@T @o\nno\\nHO\n", "list": "pairs"})).unwrap();
    let state = app.view.kerning_review.pairs.clone();
    assert_eq!((state.source.as_deref(), state.current, state.rows.as_ref().map(Vec::len)), (Some("text"), None, Some(3)));
    run(&mut app, "kerning.setPairListMode", json!({"mode": "pairs"})).unwrap();
    let labels: Vec<(String, Option<f64>)> = app.models.pairs_phrases.rows.iter().map(|r| (r.label.clone(), r.value)).collect();
    assert_eq!(labels, [("A V".into(), Some(-100.0)), ("@T @o".into(), Some(-80.0)), ("no \\n HO".into(), None)]);

    // A pair row asks the edit view to show the pair; a phrase steps through its lines before the row changes.
    app.sent.push(shell("kerning.stepPairList", json!({"list": "pairs"})));
    app.drain();
    assert_eq!(app.sent, vec![shell("kerning.showPairInEditView", json!({"left": "A", "right": "V"}))]);
    app.sent.clear();
    let at = |app: &FontCraftApp| (app.view.kerning_review.pairs.current, app.view.kerning_review.pairs.line);
    let step = |app: &mut FontCraftApp, params: Value| {
        app.sent.push(shell("kerning.stepPairList", params));
        app.drain();
        std::mem::take(&mut app.sent)
    };
    step(&mut app, json!({"direction": "next", "list": "pairs"}));
    let shown = step(&mut app, json!({"list": "pairs"}));
    assert_eq!(at(&app), (Some(2), 0));
    assert_eq!(shown.first(), Some(&shell("editView.setText", json!({"glyphs": ["n", "o"]}))));
    step(&mut app, json!({"list": "pairs"}));
    step(&mut app, json!({"list": "pairs"}));
    assert_eq!(at(&app), (Some(2), 1), "at the last line of the last row, next stays");
    step(&mut app, json!({"direction": "previous", "list": "pairs"}));
    assert_eq!((at(&app), app.view.kerning_review.pairs.selected.clone()), ((Some(2), 0), vec![2]));
    // Selecting shows nothing.
    assert!(step(&mut app, json!({"select": [0, 1], "list": "pairs"})).is_empty());
    assert_eq!(app.view.kerning_review.pairs.selected, [0, 1]);

    // Refusals change nothing.
    let before = app.view.kerning_review.clone();
    assert!(run(&mut app, "kerning.stepPairList", json!({"index": 9, "list": "pairs"})).is_err());
    assert!(run(&mut app, "kerning.loadPairList", json!({"text": "# nothing\n", "list": "pairs"})).is_err());
    assert!(run(&mut app, "kerning.loadPairList", json!({"text": "A V", "reset": true})).is_err());
    assert!(run(&mut app, "kerning.setPairListMode", json!({"mode": "words"})).is_err());
    assert_eq!(app.view.kerning_review, before);
    // Reset brings back the built-in list.
    run(&mut app, "kerning.loadPairList", json!({"reset": true, "list": "pairs"})).unwrap();
    assert!(app.models.pairs_phrases.rows.len() >= 20 && app.models.pairs_phrases.source.is_none());
}

#[test]
fn kerning_auto_kern_pair_list_proposes_for_review_then_applies() {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    // n has 60 units each side; i 20: n i holds 80 of white against n n's 120.
    for (glyph, nodes, width) in [
        ("n", json!([{"x": 60, "y": 0}, {"x": 440, "y": 0}, {"x": 440, "y": 500}, {"x": 60, "y": 500}]), 500),
        ("i", json!([{"x": 20, "y": 0}, {"x": 80, "y": 0}, {"x": 80, "y": 500}, {"x": 20, "y": 500}]), 100),
    ] {
        app.sent.push(Msg::run("path.addPath", json!({"glyph": glyph, "nodes": nodes})));
        app.sent.push(Msg::run("spacing.setWidth", json!({"glyphs": [glyph], "value": width})));
    }
    app.drain();
    let value =
        |app: &mut FontCraftApp| app.session.query("kerning.getPair", &json!({"left": "n", "right": "i"})).unwrap()["value"].as_f64().unwrap_or(0.0);
    run(&mut app, "kerning.loadPairList", json!({"text": "n i\nnnoon\n", "list": "pairs"})).unwrap();
    run(&mut app, "kerning.autoKernPairList", json!({"list": "pairs"})).unwrap();
    let proposal = app.view.kerning_review.auto_kern.clone().expect("a proposal");
    assert_eq!(proposal.rows, [0], "the phrase row is left out");
    assert_eq!(proposal.reply["pairs"][0]["value"].as_f64(), Some(40.0));
    assert_eq!(value(&mut app), 0.0, "a proposal stores nothing");
    run(&mut app, "kerning.autoKernPairList", json!({"apply": true})).unwrap();
    assert_eq!(value(&mut app), 40.0);
    assert!(app.view.kerning_review.auto_kern.is_none());
    // A proposal made before the font changed is refused.
    run(&mut app, "kerning.autoKernPairList", json!({"list": "pairs"})).unwrap();
    app.sent.push(Msg::run("kerning.setPair", json!({"left": "n", "right": "n", "value": 5})));
    app.drain();
    assert!(run(&mut app, "kerning.autoKernPairList", json!({"apply": true})).is_err());
    run(&mut app, "kerning.autoKernPairList", json!({"discard": true})).unwrap();
    assert!(app.view.kerning_review.auto_kern.is_none());
}

/// A click on a row steps to it; Cmd+click adds a row to the selection; the mode segments post the mode.
#[test]
fn pairs_panel_clicks_step_select_and_set_the_mode() {
    let mut app = kern_app();
    run(&mut app, "kerning.loadPairList", json!({"text": "A V\nL T\n", "list": "pairs"})).unwrap();
    run(&mut app, "kerning.setPairListMode", json!({"mode": "pairs"})).unwrap();
    let app = Rc::new(RefCell::new(app));
    let shared = Rc::clone(&app);
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut app = shared.borrow_mut();
        let model = Tracked::new((*app.models.pairs_phrases).clone());
        let mut sent = Vec::new();
        show(ui, Props { model: &model, sent: &mut sent });
        seen.borrow_mut().extend(sent.iter().cloned());
        // Only the list's own commands run: the edit view's follow-ups are dropped.
        app.sent.extend(sent);
        app.drain();
        app.sent.clear();
    });
    fixture.settle();
    let first = fixture.by_label("A V").expect("the first row");
    fixture.click(&first).settle();
    let second = fixture.by_label("L T").expect("the second row");
    fixture.hold(Modifiers::COMMAND).click(&second).hold(Modifiers::NONE).settle();
    let phrases = fixture.by_label("Phrases").expect("the Phrases segment");
    fixture.click(&phrases).settle();
    let posted = posted.borrow();
    assert_eq!(
        *posted,
        vec![
            shell("kerning.stepPairList", json!({"index": 0, "list": "pairs"})),
            shell("kerning.stepPairList", json!({"select": [0, 1], "list": "pairs"})),
            shell("kerning.setPairListMode", json!({"mode": "phrases"})),
        ]
    );
    assert_eq!(app.borrow().view.kerning_review.pairs.selected, [0, 1]);
}
