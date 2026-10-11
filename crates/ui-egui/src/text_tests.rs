//! Tab text: the shell commands over a tab's text, the cached layout and the display strings.

use std::sync::Arc;

use fontcraft_model::plist::{Number, Value as PlistValue};
use serde_json::{Value, json};

use crate::FontcraftApp;
use crate::canvas::view;

fn frames(app: &mut FontcraftApp, frames: usize) {
    let ctx = egui::Context::default();
    let input = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    for _ in 0..frames {
        let mut output = ctx.run_ui(input(), |ui| app.ui(ui));
        output.textures_delta.clear();
    }
}

fn get(app: &mut FontcraftApp) -> Value {
    app.run("editView.getText", &json!({})).unwrap()
}

#[test]
fn text_commands_edit_the_tab_text() {
    let mut app = FontcraftApp::default();
    let opened = app.run("view.newTab", &json!({"glyphs": ["A"]})).unwrap();
    let tab = opened["tab"].clone();
    // A tab opened on one glyph is a text of one glyph, in kerning mode, edited glyph after the cursor.
    let t = get(&mut app);
    assert_eq!((&t["items"], &t["cursor"], &t["current"]), (&json!(["A"]), &json!(0), &json!("A")));
    assert_eq!((&t["textMode"], &t["direction"], &t["kerning"], &t["tab"]), (&json!(false), &json!("ltr"), &json!("kerning"), &tab));

    // editView.toggleTextMode
    let t = app.run("editView.toggleTextMode", &json!({"tab": tab})).unwrap();
    assert_eq!((&t["textMode"], &t["cursor"]), (&json!(true), &json!(0)));
    assert_eq!(get(&mut app)["textMode"], true);

    // editView.setText: `/name` escapes; the cursor ends after the text.
    app.run("editView.setText", &json!({"text": "HOHO /a"})).unwrap();
    let t = get(&mut app);
    assert_eq!(t["items"], json!(["H", "O", "H", "O", "space", "a"]));
    assert_eq!((&t["cursor"], &t["current"], &t["text"]), (&json!(6), &json!("a"), &json!("HOHO /a")));
    assert_eq!(app.state.editing(), Some("a"), "the edited glyph is the current glyph");

    // editView.setCursor: the glyph after the cursor is current; the selection runs after it.
    app.run("editView.setCursor", &json!({"index": 1, "selectionLength": 2})).unwrap();
    let t = get(&mut app);
    assert_eq!((&t["cursor"], &t["selection"], &t["current"]), (&json!(1), &json!([1, 3]), &json!("O")));

    // editView.insertText: characters replace the selection, then glyphs at an index.
    app.run("editView.insertText", &json!({"chars": "nn"})).unwrap();
    let t = get(&mut app);
    assert_eq!((&t["items"], &t["cursor"], &t["current"]), (&json!(["H", "n", "n", "O", "space", "a"]), &json!(3), &json!("O")));
    app.run("editView.insertText", &json!({"glyphs": ["A"], "at": 0})).unwrap();
    let t = get(&mut app);
    assert_eq!((&t["items"][0], &t["cursor"], &t["current"]), (&json!("A"), &json!(1), &json!("H")));
    app.run("editView.insertText", &json!({"text": "/V", "replace": [1, 2]})).unwrap();
    assert_eq!(get(&mut app)["items"], json!(["A", "V", "n", "n", "O", "space", "a"]));

    // edit.addPlaceholder: it shows the current glyph.
    app.run("edit.addPlaceholder", &json!({"at": 0})).unwrap();
    let t = get(&mut app);
    assert_eq!((&t["items"][0], &t["cursor"], &t["current"]), (&json!({"placeholder": true}), &json!(1), &json!("A")));
    assert!(t["text"].as_str().unwrap().starts_with("/Placeholder/A"), "{}", t["text"]);

    // editView.nextGlyph and editView.previousGlyph replace the current glyph in glyph order,
    // and stop at the ends.
    app.run("editView.nextGlyph", &json!({})).unwrap();
    let t = get(&mut app);
    assert_eq!((&t["items"][1], &t["current"], &t["cursor"]), (&json!("B"), &json!("B"), &json!(1)));
    app.run("editView.previousGlyph", &json!({})).unwrap();
    app.run("editView.previousGlyph", &json!({})).unwrap();
    assert_eq!(get(&mut app)["items"][1], "space");
    app.run("editView.previousGlyph", &json!({})).unwrap();
    app.run("editView.previousGlyph", &json!({})).unwrap();
    let t = get(&mut app);
    assert_eq!((&t["items"][1], &t["current"]), (&json!(".notdef"), &json!(".notdef")), "the first glyph stays");

    // editView.nextVisibleGlyph and editView.previousVisibleGlyph step through Font View's list.
    // A glyph Font View does not list steps to the first one listed.
    app.run("fontView.setSearch", &json!({"query": "a", "fields": ["name"]})).unwrap();
    crate::shell::font_view::refresh(&mut app);
    let listed = crate::shell::font_view::visible_names(&app);
    assert!(listed.len() >= 2 && !listed.contains(&".notdef".to_string()), "{listed:?}");
    app.run("editView.nextVisibleGlyph", &json!({})).unwrap();
    assert_eq!(get(&mut app)["current"], json!(listed[0]));
    app.run("editView.nextVisibleGlyph", &json!({})).unwrap();
    assert_eq!(get(&mut app)["current"], json!(listed[1]));
    app.run("editView.previousVisibleGlyph", &json!({})).unwrap();
    assert_eq!(get(&mut app)["items"][1], json!(listed[0]));

    // editView.setWritingDirection
    let t = app.run("editView.setWritingDirection", &json!({"direction": "rtl"})).unwrap();
    assert_eq!(t["direction"], "rtl");
    app.run("editView.setWritingDirection", &json!({"direction": "ttb"})).unwrap();
    assert_eq!(get(&mut app)["direction"], "ttb");
    assert_eq!(app.run("view.getDisplayOptions", &json!({})).unwrap()["direction"], "ttb");

    // kerning.setMode: without a mode, the next one.
    app.run("kerning.setMode", &json!({})).unwrap();
    assert_eq!(get(&mut app)["kerning"], "locked");
    app.run("kerning.setMode", &json!({"tab": tab})).unwrap();
    assert_eq!(get(&mut app)["kerning"], "none");
    app.run("kerning.setMode", &json!({"mode": "kerning"})).unwrap();
    assert_eq!(get(&mut app)["kerning"], "kerning");

    // A failed command changes nothing.
    let before = get(&mut app);
    for (id, params) in [
        ("editView.insertText", json!({"chars": "x", "at": 999})),
        ("editView.insertText", json!({"chars": "x", "glyphs": ["A"]})),
        ("editView.insertText", json!({"glyphs": ["nope"]})),
        ("editView.insertText", json!({"chars": "A".repeat(10_001)})),
        ("editView.setText", json!({"text": "/".repeat(10_001)})),
        ("editView.setCursor", json!({"index": 2, "selectionLength": 100})),
        ("editView.setWritingDirection", json!({"direction": "up"})),
        ("kerning.setMode", json!({"mode": "loose"})),
        ("editView.toggleTextMode", json!({"mode": "maybe"})),
        ("editView.getText", json!({"tab": 999})),
    ] {
        assert!(app.run(id, &params).is_err(), "{id} {params}");
        assert_eq!(get(&mut app), before, "{id} {params}");
    }
    assert!(!app.session.doc().unwrap().modified(), "a text change does not mark the document modified");

    // editView.toggleTextMode back to editing; line breaks and text over several lines draw.
    app.run("editView.toggleTextMode", &json!({"mode": "edit"})).unwrap();
    assert_eq!(get(&mut app)["textMode"], false);
    app.run("editView.setWritingDirection", &json!({"direction": "ltr"})).unwrap();
    app.run("editView.setText", &json!({"chars": "AB\nCD"})).unwrap();
    assert_eq!(get(&mut app)["items"], json!(["A", "B", {"lineBreak": true}, "C", "D"]));
    frames(&mut app, 2);
}

#[test]
fn the_tab_layout_is_cached_and_follows_kerning_and_line_height() {
    let mut app = FontcraftApp::default();
    app.run("view.newTab", &json!({"glyphs": ["A", "V"]})).unwrap();
    app.run("kerning.setPair", &json!({"left": "A", "right": "V", "value": -100})).unwrap();
    app.run("editView.setCursor", &json!({"index": 1})).unwrap();
    let font = app.session.font().unwrap().clone();
    let width_a = font.glyph("A").unwrap().layers.first().unwrap().width;
    let x = |app: &FontcraftApp| view::edited_origin(&app.session, &app.layouts, app.state.tab().unwrap());
    assert_eq!(x(&app), [width_a - 100.0, 0.0], "kerning mode applies the pair");
    let tab = app.state.tab().unwrap();
    let (first, again) = (view::text_layout(&app.session, &app.layouts, tab).unwrap(), view::text_layout(&app.session, &app.layouts, tab).unwrap());
    assert!(Arc::ptr_eq(&first, &again), "an unchanged tab reuses its layout");
    app.run("kerning.setMode", &json!({"mode": "none"})).unwrap();
    assert_eq!(x(&app), [width_a, 0.0]);

    // The master's `EditView Line Height` spaces the lines.
    app.session
        .edit("line height", |font| {
            let masters = Arc::make_mut(&mut font.masters);
            masters.first_mut().unwrap().params.set("EditView Line Height", PlistValue::Number(Number::int(1500)));
            Ok(())
        })
        .unwrap();
    app.run("editView.setText", &json!({"chars": "A\nV"})).unwrap();
    app.run("editView.setCursor", &json!({"index": 2})).unwrap();
    assert_eq!(x(&app), [0.0, -1500.0]);
    let layout = view::text_layout(&app.session, &app.layouts, app.state.tab().unwrap()).unwrap();
    assert_eq!(layout.line_height, 1500.0);
}

#[test]
fn tabs_open_from_display_strings_and_saving_writes_the_open_tabs() {
    let dir = std::env::temp_dir().join(format!("fontcraft-display-strings-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("strings.glyphs").to_string_lossy().into_owned();
    let mut app = FontcraftApp::default();
    let long = "A".repeat(10_050);
    app.session
        .edit("strings", |font| {
            font.display_strings = Arc::new(vec!["HO/a".into(), long.clone()]);
            Ok(())
        })
        .unwrap();
    // Through the session: the app would write its (no) open tabs over the strings.
    app.session.execute("file.saveAs", &json!({"path": path})).unwrap();

    // Opening reads one tab per string; the over-long one is cut to the item limit.
    let mut app = FontcraftApp::default();
    app.run("file.open", &json!({"path": path})).unwrap();
    assert_eq!(app.state.tabs.len(), 2);
    assert_eq!(app.state.active_tab, None, "Font View stays showing");
    assert!(app.state.status.contains("10000"), "{}", app.state.status);
    let first = &app.state.tabs[0];
    assert_eq!(first.shown_glyphs().collect::<Vec<_>>(), ["H", "O", "a"]);
    assert_eq!(app.state.tabs[1].text.len(), fontcraft_textlayout::MAX_ITEMS);
    let id = first.id;

    // Saving writes the open tabs, without an undo step.
    app.run("editView.setText", &json!({"tab": id, "text": "/V/A"})).unwrap();
    app.run("view.closeTab", &json!({"index": 3})).unwrap();
    assert!(!app.session.doc().unwrap().modified());
    app.run("file.save", &json!({})).unwrap();
    assert!(app.session.doc().unwrap().history.undo.is_empty());
    let mut reopened = FontcraftApp::default();
    reopened.run("file.open", &json!({"path": path})).unwrap();
    assert_eq!(&*reopened.session.font().unwrap().display_strings, &["/V/A".to_string()]);
    assert_eq!(reopened.state.tabs.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn display_string_conversion_cuts_long_strings_and_skips_bad_ones() {
    let app = FontcraftApp::default();
    let font = app.session.font().unwrap();
    let bad = format!("/{}", "n".repeat(300));
    let (texts, warnings) = crate::shell::text::tabs_from_display_strings(font, &["AB".into(), "A".repeat(20_000), bad.clone()], 10);
    assert_eq!(texts.iter().map(|t| t.len()).collect::<Vec<_>>(), [2, fontcraft_textlayout::MAX_ITEMS]);
    assert_eq!(warnings.len(), 2, "{warnings:?}");

    // A bad name past the item limit does not matter; one in the items a tab would show does,
    // in a string of any length.
    let late = format!("{}{bad}", "A".repeat(10_050));
    let early = format!("{bad}{}", "A".repeat(3_000_000));
    let (texts, warnings) = crate::shell::text::tabs_from_display_strings(font, &[late, early], 10);
    assert_eq!(texts.iter().map(|t| t.len()).collect::<Vec<_>>(), [fontcraft_textlayout::MAX_ITEMS]);
    assert!(warnings[0].contains("shows the first"), "{warnings:?}");
    assert!(warnings[1].contains("cannot be read"), "{warnings:?}");

    // Strings past the tab limit are not read.
    let (texts, warnings) = crate::shell::text::tabs_from_display_strings(font, &vec!["A".to_string(); 5], 3);
    assert_eq!(texts.len(), 3);
    assert_eq!(warnings, ["the document has 5 display strings; only the first 3 open as tabs"]);
}

#[test]
fn each_document_keeps_and_saves_its_own_tabs() {
    let dir = std::env::temp_dir().join(format!("fontcraft-document-tabs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = |name: &str, strings: &[&str]| {
        let path = dir.join(name).to_string_lossy().into_owned();
        let mut app = FontcraftApp::default();
        let strings: Vec<String> = strings.iter().map(|s| s.to_string()).collect();
        app.session
            .edit("strings", |font| {
                font.display_strings = Arc::new(strings);
                Ok(())
            })
            .unwrap();
        app.session.execute("file.saveAs", &json!({"path": path})).unwrap();
        path
    };
    let (a, b) = (file("a.glyphs", &["AB", "C"]), file("b.glyphs", &["HO"]));
    let shown = |app: &FontcraftApp| app.state.tabs.iter().map(|t| t.title()).collect::<Vec<_>>();

    let mut app = FontcraftApp::default();
    app.run("file.open", &json!({"path": a})).unwrap();
    app.run("view.showTab", &json!({"index": 2})).unwrap();
    app.run("file.open", &json!({"path": b})).unwrap();
    assert_eq!(shown(&app), ["H O"], "only the active document's tabs show");
    app.run("glyph.removeGlyph", &json!({"glyphs": ["A"], "confirm": true})).unwrap();
    app.run("editView.setText", &json!({"tab": app.state.tabs[0].id, "text": "HOH"})).unwrap();

    // Back to the first file: its tabs come back as they were, the showing one included, and
    // removing A from the other document left them alone.
    app.run("document.activate", &json!({"path": a})).unwrap();
    assert_eq!(shown(&app), ["A B", "C"]);
    assert_eq!(app.state.active_tab, Some(0));
    app.run("file.save", &json!({})).unwrap();
    app.run("document.activate", &json!({"path": b})).unwrap();
    app.run("file.save", &json!({"force": true})).unwrap();

    // Closing a document closes its tabs; the first file shows again with its own.
    let closed = app.session.active().unwrap().id;
    app.run("file.close", &json!({"discard": true})).unwrap();
    assert_eq!(shown(&app), ["A B", "C"]);
    assert!(!app.state.other_documents.contains_key(&closed));

    let strings = |path: &str| {
        let mut app = FontcraftApp::default();
        app.run("file.open", &json!({"path": path})).unwrap();
        app.session.font().unwrap().display_strings.to_vec()
    };
    assert_eq!(strings(&a), ["AB", "C"]);
    assert_eq!(strings(&b), ["HOH"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn closed_tabs_lose_removed_glyphs() {
    let mut app = FontcraftApp::default();
    app.run("view.newTab", &json!({"glyphs": ["A", "B"]})).unwrap();
    app.run("view.newTab", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.closeTab", &json!({})).unwrap();
    app.run("view.closeTab", &json!({})).unwrap();
    app.run("glyph.removeGlyph", &json!({"glyphs": ["A"], "confirm": true})).unwrap();
    app.run("view.reopenLastClosedTab", &json!({})).unwrap();
    assert_eq!(app.state.tab().unwrap().shown_glyphs().collect::<Vec<_>>(), ["B"]);
    assert!(app.run("view.reopenLastClosedTab", &json!({})).is_err(), "the tab that held only A is gone");
}
