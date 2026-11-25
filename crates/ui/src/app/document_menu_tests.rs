#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use craft::ui::testing::Fixture;
use serde_json::{Value, json};

use super::*;
use crate::testing::TestApp;

/// The rows of the menu `menu` in the submenu `path`, as `ui.menu.list` reports them.
fn rows(t: &mut TestApp, menu: &str, path: &[&str]) -> Vec<Value> {
    let menus = t.ok("ui.menu.list", json!({}));
    let items = menus.as_array().unwrap().iter().find(|m| m["title"] == menu).expect("the menu")["items"].clone();
    items.as_array().unwrap().iter().filter(|i| i["path"] == json!(path)).cloned().collect()
}

/// The document rows of the Window menu: label and check mark.
fn documents(t: &mut TestApp) -> Vec<(String, bool)> {
    let window = rows(t, "Window", &[]);
    let listed = window.iter().filter(|r| r["command"] == "document.activate" || r["command"] == "window.project.select");
    listed.map(|r| (r["label"].as_str().unwrap().to_string(), r["checked"] == true)).collect()
}

/// Three documents switch by menu row and by key; each shows its own tabs and its own glyph selection again, the
/// changed one says Edited, and a closed one leaves the menu.
#[test]
fn the_window_menu_lists_and_shows_the_open_documents() {
    let mut t = TestApp::new();
    for _ in 0..2 {
        t.ok("engine.execute", json!({"command": "file.new", "params": {}}));
    }
    t.ok("engine.execute", json!({"command": "document.activate", "params": {"index": 0}}));
    t.frame();
    let listed = documents(&mut t);
    assert_eq!(listed.iter().map(|(_, on)| *on).collect::<Vec<_>>(), [true, false, false], "{listed:?}");

    // The first document: an Edit View tab, and A selected in Font View.
    t.ok("engine.execute", json!({"command": "fontView.show", "params": {}}));
    t.app.view.font_view.selected = vec!["A".into()];
    t.ok("engine.execute", json!({"command": "glyph.newGlyph", "params": {"name": "documentMenuGlyph"}}));
    t.app.view.font_view.selected = vec!["A".into()];
    t.frame();
    assert!(documents(&mut t)[0].0.ends_with("— Edited"));

    t.ok("ui.menu.invoke", json!({"command": "document.activate", "params": {"index": 1}}));
    t.frame();
    assert!(t.app.view.font_view.selected.is_empty() && t.app.view.tabs.tabs.is_empty(), "the second document has no tabs and no selection");
    assert!(documents(&mut t)[1].1);
    t.ok("ui.key", json!({"key": "`", "cmd": true}));
    t.frame();
    assert!(documents(&mut t)[2].1, "the key shows the next document");
    t.ok("engine.execute", json!({"command": "document.showNext", "params": {}}));
    t.frame();
    assert_eq!(
        (t.app.view.font_view.selected.as_slice(), t.app.view.tabs.tabs.len()),
        (["A".to_string()].as_slice(), 1),
        "after the last comes the first"
    );
    t.ok("engine.execute", json!({"command": "document.showPrevious", "params": {}}));
    t.frame();
    assert!(documents(&mut t)[2].1);

    t.ok("engine.execute", json!({"command": "file.close", "params": {}}));
    t.frame();
    assert_eq!(documents(&mut t).len(), 2, "a closed document leaves the menu");
    t.ok("engine.execute", json!({"command": "file.newProject", "params": {}}));
    t.frame();
    let listed = documents(&mut t);
    assert_eq!(listed.len(), 3, "projects follow the fonts: {listed:?}");
}

#[test]
fn names_that_repeat_show_their_folder() {
    let path = |text: &'static str| Some(Path::new(text));
    let names = [
        ("A.glyphs".to_string(), path("/x/one/A.glyphs")),
        ("A.glyphs".to_string(), path("/x/two/A.glyphs")),
        ("A.glyphs".to_string(), path("/y/two/A.glyphs")),
        ("B.glyphs".to_string(), path("/x/one/B.glyphs")),
    ];
    assert_eq!(told_apart(&names), ["A.glyphs — one", "A.glyphs — /x/two", "A.glyphs — /y/two", "B.glyphs"]);

    let many: Vec<OpenDocument> = (0..MAX_LISTED + 2)
        .map(|i| OpenDocument { command: CommandRef::with_params("document.activate", json!({"index": i})), label: format!("{i}"), active: i == 0 })
        .collect();
    let rows = document_rows(&many);
    assert_eq!(rows.iter().filter(|r| r.path.len() == 1 && r.label != "---").count(), MAX_LISTED);
    assert_eq!(rows.iter().filter(|r| r.path == MORE_MENU).count(), 2);
}

/// File › Open Recent lists the stored files, greys out one that is gone when the window is focused again, and Clear
/// Menu empties it.
#[test]
fn open_recent_is_a_submenu_of_the_recent_files() {
    let dir = std::env::temp_dir().join(format!("fontcraft-ui-recent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (kept, gone) = (dir.join("kept.txt"), dir.join("gone.txt"));
    std::fs::write(&kept, "x").unwrap();
    std::fs::write(&gone, "x").unwrap();
    let mut t = TestApp::new();
    let recent = rows(&mut t, "File", &["Open Recent"]);
    assert_eq!(recent.iter().map(|r| (r["label"].as_str().unwrap(), r["enabled"] == true)).collect::<Vec<_>>(), [("No Recent Files", false)]);
    t.app.session.settings_mut().remember_document(&kept).unwrap();
    t.app.session.settings_mut().remember_document(&gone).unwrap();
    t.frame();
    let shown = |t: &mut TestApp| -> Vec<(String, bool)> {
        rows(t, "File", &["Open Recent"]).iter().map(|r| (r["label"].as_str().unwrap().to_string(), r["enabled"] == true)).collect()
    };
    assert_eq!(shown(&mut t), [("gone.txt".into(), true), ("kept.txt".into(), true), ("Clear Menu".into(), true)]);
    std::fs::remove_file(&gone).unwrap();
    t.frame_with(vec![egui::Event::WindowFocused(true)]);
    t.frames(2);
    assert_eq!(shown(&mut t)[0], ("gone.txt".into(), false));
    let error = t.error("ui.menu.invoke", json!({"command": "file.openRecent", "params": {"path": gone.to_string_lossy()}}));
    assert!(error.contains("no longer exists"), "{error}");
    t.ok("ui.menu.invoke", json!({"command": "file.openRecent", "params": {"clear": true}}));
    t.frame();
    assert_eq!(shown(&mut t), [("No Recent Files".into(), false)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_pop_up_runs_the_picked_documents_command() {
    let documents: Vec<OpenDocument> = ["One", "Two"]
        .iter()
        .enumerate()
        .map(|(i, name)| OpenDocument {
            command: CommandRef::with_params("document.activate", json!({"index": i})),
            label: (*name).to_string(),
            active: i == 0,
        })
        .collect();
    let picked = Rc::new(RefCell::new(None));
    let seen = Rc::clone(&picked);
    let listed = documents.clone();
    let mut fixture = Fixture::new(move |ui| {
        let opener = button(ui);
        if let Some(command) = show_menu(ui, &opener, &listed) {
            *seen.borrow_mut() = Some(command);
        }
    });
    fixture.settle();
    let opener = fixture.by_label("Documents").expect("the pop-up's button");
    fixture.click(&opener).settle();
    let two = fixture.by_label("Two").expect("the second document");
    fixture.click(&two).settle();
    assert_eq!(*picked.borrow(), Some(documents[1].command.clone()));
}
