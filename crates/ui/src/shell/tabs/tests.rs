use std::sync::Arc;

use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use serde_json::{Value, json};

use crate::app::{APP, FontCraftApp, Msg};

/// The app with one Edit View tab showing.
fn app() -> FontCraftApp {
    FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()))
}

/// Runs a shell command as `drain` does and returns the status line: empty when it worked.
fn shell(app: &mut FontCraftApp, id: &str, params: Value) -> String {
    app.status.clear();
    app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
    app.drain();
    app.status.clone()
}

/// The titles of the Edit View tabs after the one the test started with.
fn opened(app: &FontCraftApp) -> Vec<String> {
    app.models.tabs.tabs.iter().skip(1).map(|t| t.glyphs.join(" ")).collect()
}

#[test]
fn tabs_open_switch_close_and_reopen() {
    let mut app = app();
    assert_eq!((app.view.tabs.tabs.len(), app.view.tabs.position()), (1, 2), "the test starts on one Edit View tab");
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

/// A window opens on Font View with no Edit View tab, as the reference opens a font, and so does a new document.
#[test]
fn a_window_and_a_new_document_open_on_font_view() {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    assert!(app.font_view_shows() && app.view.tabs.tabs.is_empty(), "a window opens on Font View");
    assert_eq!(app.view.tabs.positions(), 1);
    assert!(!app.targets_chosen(), "no glyph is chosen");
    assert_eq!(shell(&mut app, "view.editSelectedGlyphs", json!({"glyphs": ["A"]})), "");
    assert_eq!((app.view.tabs.tabs.len(), app.view.tabs.position(), app.view.glyph.as_deref()), (1, 2, Some("A")));
    app.run_now(Msg::run("file.new", json!({})), false).unwrap();
    assert!(app.font_view_shows() && app.view.tabs.tabs.is_empty(), "a new document opens on Font View");
    // Cmd+Alt+2 with no Edit View tab names no tab.
    assert!(shell(&mut app, "view.showTab", json!({"index": 2})).contains("no tab at position 2"));
    assert!(shell(&mut app, "view.showNextTab", json!({})).is_empty());
    assert!(app.font_view_shows(), "with Font View alone, the next tab is Font View");
}

/// Each document keeps its own tabs: a new document opens on Font View, switching documents brings back each one's
/// tabs with the tab that showed, and closing a document drops its tabs.
#[test]
fn each_document_keeps_its_own_tabs() {
    let mut app = app();
    assert_eq!(shell(&mut app, "view.newTab", json!({"text": "AB"})), "");
    assert_eq!((opened(&app), app.view.tabs.position(), app.view.glyph.as_deref()), (vec!["A B".to_string()], 3, Some("A")));

    app.run_now(Msg::run("file.new", json!({})), false).unwrap();
    assert!(app.font_view_shows() && app.view.tabs.tabs.is_empty(), "a new document opens on Font View");
    assert_eq!(shell(&mut app, "view.newTab", json!({"text": "C"})), "");
    assert_eq!(shell(&mut app, "fontView.show", json!({})), "");
    assert_eq!(app.models.tabs.tabs.iter().map(|t| t.glyphs.join(" ")).collect::<Vec<_>>(), vec!["C".to_string()]);

    app.run_now(Msg::run("document.activate", json!({"index": 0})), false).unwrap();
    assert_eq!(
        (opened(&app), app.view.tabs.position(), app.view.glyph.as_deref()),
        (vec!["A B".to_string()], 3, Some("A")),
        "the first document's tabs come back with the tab that showed"
    );
    app.run_now(Msg::run("document.activate", json!({"index": 1})), false).unwrap();
    assert!(app.view.tabs.font_view_shows(), "the second document shows Font View again");
    assert_eq!(app.models.tabs.tabs.iter().map(|t| t.glyphs.join(" ")).collect::<Vec<_>>(), vec!["C".to_string()]);

    app.run_now(Msg::run("file.close", json!({})), false).unwrap();
    assert_eq!(opened(&app), vec!["A B".to_string()], "closing the second document shows the first one's tabs");
    assert!(app.view.tabs.others.is_empty(), "the closed document's tabs are dropped");
}

/// File › Open opens a tab per display string behind Font View, a string over the item limit cut to it with a warning
/// in the status line; saving writes the open tabs back, without an undo step or the modified mark.
#[test]
fn display_strings_open_as_tabs_and_saving_writes_the_open_tabs() {
    let dir = std::env::temp_dir().join(format!("fontcraft-ui-display-strings-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("strings.glyphs").to_string_lossy().into_owned();
    let mut session = Session::with_document();
    let long = "A".repeat(10_050);
    session
        .edit("strings", |font| {
            font.display_strings = Arc::new(vec!["HO/a".into(), long.clone()]);
            Ok(())
        })
        .unwrap();
    session.execute("file.saveAs", &json!({"path": path})).unwrap();

    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.run_now(Msg::run("file.open", json!({"path": path})), false).unwrap();
    assert!(app.font_view_shows(), "Font View stays showing");
    assert_eq!(app.view.tabs.tabs.len(), 2);
    assert!(app.status.contains("10000"), "{}", app.status);
    assert_eq!(app.models.tabs.tabs[0].glyphs, ["H", "O", "a"]);
    assert_eq!(app.view.tabs.tabs[1].state.text.text.len(), fontcraft_textlayout::MAX_ITEMS);

    assert_eq!(shell(&mut app, "view.closeTab", json!({"index": 3})), "");
    assert_eq!(shell(&mut app, "view.newTab", json!({"glyphs": ["V", "A"]})), "");
    assert!(!app.session.doc().unwrap().modified());
    app.run_now(Msg::run("file.save", json!({})), false).unwrap();
    assert!(app.session.doc().unwrap().history.undo.is_empty(), "storing the tabs makes no undo step");
    assert_eq!(&*app.session.font().unwrap().display_strings, &["HO/a".to_string(), "VA".into()]);

    let mut reopened = FontCraftApp::new(Session::new(), Arc::new(Host::builder(APP).build()));
    reopened.run_now(Msg::run("file.open", json!({"path": path})), false).unwrap();
    assert_eq!(reopened.models.tabs.tabs.iter().map(|t| t.glyphs.join(" ")).collect::<Vec<_>>(), ["H O a", "V A"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A renamed glyph keeps its place in every tab; a removed glyph leaves the tabs: a tab left without glyphs closes and
/// is not kept for Reopen Last Closed Tab, and a closed tab left without glyphs is forgotten.
#[test]
fn renamed_and_removed_glyphs_follow_into_the_tabs() {
    let mut app = app();
    for name in ["x.one", "x.two", "x.three"] {
        app.run_now(Msg::run("glyph.newGlyph", json!({"name": name})), false).unwrap();
    }
    shell(&mut app, "view.newTab", json!({"glyphs": ["x.one", "x.two"]}));
    shell(&mut app, "view.newTab", json!({"glyphs": ["x.three"]}));
    shell(&mut app, "view.newTab", json!({"glyphs": ["x.two"]}));
    shell(&mut app, "view.closeTab", json!({}));
    shell(&mut app, "view.showTab", json!({"index": 3}));
    assert_eq!((opened(&app), app.view.tabs.closed.len()), (vec!["x.one x.two".to_string(), "x.three".into()], 1));

    app.run_now(Msg::run("glyph.rename", json!({"glyph": "x.one", "newName": "x.first", "updateInfo": false})), false).unwrap();
    assert_eq!(opened(&app), vec!["x.first x.two".to_string(), "x.three".into()]);
    assert_eq!(app.view.glyph.as_deref(), Some("x.first"), "the edited glyph follows its new name");

    app.run_now(Msg::run("glyph.removeGlyph", json!({"glyphs": ["x.two"]})), false).unwrap();
    assert_eq!(opened(&app), vec!["x.first".to_string(), "x.three".into()]);
    assert!(app.view.tabs.closed.is_empty(), "the closed tab that held only x.two is forgotten");

    app.run_now(Msg::run("glyph.removeGlyph", json!({"glyphs": ["x.three"]})), false).unwrap();
    assert_eq!((opened(&app), app.view.tabs.position()), (vec!["x.first".to_string()], 3), "the showing tab stays");
    app.run_now(Msg::run("glyph.removeGlyph", json!({"glyphs": ["x.first"]})), false).unwrap();
    assert!(app.font_view_shows(), "the showing tab left without glyphs closes and Font View shows");
    assert!(opened(&app).is_empty() && app.view.tabs.closed.is_empty());
}

/// The display strings of a file are read into tab texts at most up to the room left, cut to the item limit; a
/// string that cannot be read opens no tab.
#[test]
fn display_string_conversion_cuts_long_strings_and_skips_bad_ones() {
    let session = Session::with_document();
    let font = session.font().unwrap();
    let bad = format!("/{}", "n".repeat(300));
    let (texts, warnings) = super::upkeep::tabs_from_display_strings(font, &["AB".into(), "A".repeat(20_000), bad], 10);
    assert_eq!(texts.iter().map(|t| t.text.len()).collect::<Vec<_>>(), [2, fontcraft_textlayout::MAX_ITEMS]);
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    let (texts, warnings) = super::upkeep::tabs_from_display_strings(font, &vec!["A".to_string(); 5], 3);
    assert_eq!(texts.len(), 3);
    assert_eq!(warnings, ["the document has 5 display strings; only the first 3 open as tabs"]);
}
