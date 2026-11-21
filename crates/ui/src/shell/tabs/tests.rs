use std::sync::Arc;

use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use serde_json::{Value, json};

use crate::app::{APP, FontCraftApp, Msg};

fn app() -> FontCraftApp {
    FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()))
}

/// Runs a shell command as `drain` does and returns the status line: empty when it worked.
fn shell(app: &mut FontCraftApp, id: &str, params: Value) -> String {
    app.status.clear();
    app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
    app.drain();
    app.status.clone()
}

/// The titles of the Edit View tabs after the one the window opened with.
fn opened(app: &FontCraftApp) -> Vec<String> {
    app.models.tabs.tabs.iter().skip(1).map(|t| t.glyphs.join(" ")).collect()
}

#[test]
fn tabs_open_switch_close_and_reopen() {
    let mut app = app();
    assert_eq!((app.view.tabs.tabs.len(), app.view.tabs.position()), (1, 2), "a window opens on one Edit View tab");
    assert_eq!(shell(&mut app, "fontView.show", json!({})), "");
    assert!(app.view.tabs.font_view_shows());
    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["A", "B"]}));
    assert_eq!(shell(&mut app, "view.newTab", json!({})), "");
    assert_eq!((app.view.tabs.position(), app.view.glyph.as_deref()), (3, Some("A")), "the new tab shows and edits its first glyph");
    assert_eq!(shell(&mut app, "view.newTab", json!({"text": "CA"})), "");
    assert_eq!(shell(&mut app, "view.editSelectedGlyphs", json!({"glyphs": ["A", "B"]})), "");
    assert_eq!((opened(&app), app.view.tabs.position()), (vec!["A B".to_string(), "C A".into()], 3), "an open tab is reused");
    shell(&mut app, "view.editSelectedGlyphs", json!({"glyphs": ["D"]}));
    assert_eq!(app.view.tabs.tabs.len(), 4);

    shell(&mut app, "view.showNextTab", json!({}));
    assert!(app.view.tabs.font_view_shows(), "after the last tab comes Font View");
    shell(&mut app, "view.showPreviousTab", json!({}));
    assert_eq!(app.view.tabs.position(), 5, "before Font View comes the last tab");
    shell(&mut app, "view.showTab", json!({"index": 4}));
    assert_eq!(app.view.glyph.as_deref(), Some("C"));
    assert_eq!(app.view.text.text.len(), 2, "the tab's text came back with it");

    shell(&mut app, "view.closeTab", json!({}));
    assert_eq!((opened(&app).len(), app.view.tabs.position(), app.view.glyph.as_deref()), (2, 3, Some("A")), "the tab to the left shows");
    shell(&mut app, "view.reopenLastClosedTab", json!({}));
    assert_eq!((opened(&app)[1].as_str(), app.view.tabs.position()), ("C A", 4), "back where it was");
    shell(&mut app, "view.closeOtherTabs", json!({"keep": 5}));
    assert_eq!((opened(&app), app.view.tabs.tabs.len(), app.view.tabs.position()), (Vec::<String>::new(), 1, 2));
    assert_eq!(app.models.tabs.tabs[0].glyphs, ["D"]);
    assert!(shell(&mut app, "view.closeTab", json!({"index": 1})).contains("Font View cannot be closed"));
    assert!(shell(&mut app, "view.showTab", json!({"index": 3})).contains("no tab at position 3"));
    assert_eq!(app.view.tabs.closed.len(), 3, "every closed tab can come back");
}

/// Each tab keeps its own camera, Edit Background and shown glyph.
#[test]
fn a_tab_keeps_its_own_view() {
    let mut app = app();
    shell(&mut app, "view.newTab", json!({"glyphs": ["E"]}));
    app.view.canvas.zoom = 3.0;
    app.view.edit_background = true;
    shell(&mut app, "view.showTab", json!({"index": 2}));
    assert_ne!(app.view.canvas.zoom, 3.0);
    assert!(!app.view.edit_background);
    assert_ne!(app.view.glyph.as_deref(), Some("E"));
    shell(&mut app, "view.showTab", json!({"index": 3}));
    assert_eq!((app.view.canvas.zoom, app.view.edit_background, app.view.glyph.as_deref()), (3.0, true, Some("E")));
    // The camera comes back with a reopened tab too.
    shell(&mut app, "view.closeTab", json!({}));
    shell(&mut app, "view.reopenLastClosedTab", json!({}));
    assert_eq!((app.view.canvas.zoom, app.view.glyph.as_deref()), (3.0, Some("E")));
}

/// Commands fired without targets act on Font View's selection only while Font View is the showing tab.
#[test]
fn font_view_targets_follow_the_showing_tab() {
    let mut app = app();
    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["A", "B"]}));
    assert!(!app.font_view_shows());
    shell(&mut app, "fontView.show", json!({}));
    assert!(app.font_view_shows() && app.targets_chosen());
    shell(&mut app, "view.editSelectedGlyphs", json!({}));
    assert!(!app.font_view_shows(), "opening the selection shows its tab");
    assert_eq!(app.view.glyph.as_deref(), Some("A"));
}

/// `ui.inspect` lists the tabs, the showing one and how many closed tabs can come back.
#[test]
fn inspect_reports_the_tabs() {
    let mut app = app();
    shell(&mut app, "view.newTab", json!({"glyphs": ["A", "B"]}));
    shell(&mut app, "view.newTab", json!({"glyphs": ["C"]}));
    shell(&mut app, "view.closeTab", json!({}));
    let state = crate::control::dispatch(&mut app, "ui.inspect", &json!({})).unwrap();
    assert_eq!(state["activeTab"], json!(1), "{state}");
    assert_eq!(state["closedTabs"], json!(1));
    assert_eq!(state["tabs"][1]["glyphs"], json!(["A", "B"]));
    assert_eq!(state["tabs"][1]["position"], json!(3));
    assert_eq!(state["tabs"][1]["edited"], json!("A"));
    shell(&mut app, "fontView.show", json!({}));
    let state = crate::control::dispatch(&mut app, "ui.inspect", &json!({})).unwrap();
    assert_eq!(state["activeTab"], Value::Null);
}

/// More tabs than the limit are refused, and the closed-tab stack keeps only the most recent ones.
#[test]
fn tab_counts_are_capped() {
    let mut app = app();
    for _ in 1..super::MAX_TABS {
        app.sent.push(Msg::Shell(CommandRef::with_params("view.newTab", json!({"glyphs": ["A"]}))));
    }
    app.drain();
    assert_eq!(app.view.tabs.tabs.len(), super::MAX_TABS);
    assert!(shell(&mut app, "view.newTab", json!({"glyphs": ["B"]})).contains("at most"));
    shell(&mut app, "view.closeOtherTabs", json!({"keep": 2}));
    assert_eq!(app.view.tabs.closed.len(), super::MAX_CLOSED_TABS);
}

/// Each document keeps its own tabs: a new document opens with one Edit View tab, switching documents brings back
/// each one's tabs with the tab that showed, and closing a document drops its tabs.
#[test]
fn each_document_keeps_its_own_tabs() {
    let mut app = app();
    assert_eq!(shell(&mut app, "view.newTab", json!({"text": "AB"})), "");
    assert_eq!((opened(&app), app.view.tabs.position(), app.view.glyph.as_deref()), (vec!["A B".to_string()], 3, Some("A")));

    app.run_now(Msg::run("file.new", json!({})), false).unwrap();
    assert_eq!((app.view.tabs.tabs.len(), app.view.tabs.position()), (1, 2), "a new document opens with one Edit View tab");
    assert_eq!(shell(&mut app, "view.newTab", json!({"text": "C"})), "");
    assert_eq!(shell(&mut app, "fontView.show", json!({})), "");
    assert_eq!(opened(&app), vec!["C".to_string()]);

    app.run_now(Msg::run("document.activate", json!({"index": 0})), false).unwrap();
    assert_eq!(
        (opened(&app), app.view.tabs.position(), app.view.glyph.as_deref()),
        (vec!["A B".to_string()], 3, Some("A")),
        "the first document's tabs come back with the tab that showed"
    );
    app.run_now(Msg::run("document.activate", json!({"index": 1})), false).unwrap();
    assert!(app.view.tabs.font_view_shows(), "the second document shows Font View again");
    assert_eq!(opened(&app), vec!["C".to_string()]);

    app.run_now(Msg::run("file.close", json!({})), false).unwrap();
    assert_eq!(opened(&app), vec!["A B".to_string()], "closing the second document shows the first one's tabs");
    assert!(app.view.tabs.others.is_empty(), "the closed document's tabs are dropped");
}
