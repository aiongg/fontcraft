//! The Glyph Info window and the Start Window: what they show and run against the headless query
//! and command, the restored chooser picks, and the recent documents list.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use egui::{Event, Modifiers, PointerButton, Pos2, Rect};
use fontcraft_engine::Session;
use fontcraft_model::Font;
use fontcraft_model::glyphdata::sets;
use serde_json::{Value, json};

use crate::FontcraftApp;
use crate::shell::start_window::{self, Reveal};
use fontcraft_engine::settings::MAX_RECENT_DOCUMENTS as MAX_RECENT;

/// A headless window: one egui context kept across frames, so pointer state carries over.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    fn frame_with(&self, app: &mut FontcraftApp, events: Vec<Event>, modifiers: Modifiers) {
        let mut events = events;
        events.insert(0, Event::ModifiersChanged(modifiers));
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
    }

    fn frame(&self, app: &mut FontcraftApp) {
        self.frame_with(app, Vec::new(), Modifiers::NONE);
    }

    fn click(&self, app: &mut FontcraftApp, at: Pos2, mods: Modifiers) {
        self.frame_with(app, vec![Event::PointerMoved(at), button(at, true, mods)], mods);
        self.frame_with(app, vec![button(at, false, mods)], mods);
        self.frame(app);
    }
}

fn button(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers }
}

fn glyph_names(font: &Font) -> Vec<String> {
    font.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect()
}

fn result_names(v: &Value) -> Vec<String> {
    v["results"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn glyph_info_shows_the_search_selects_rows_and_adds_them_as_the_command_does() {
    let mut app = FontcraftApp::default();
    let shown = app.run("window.glyphInfo", &json!({"query": "zhe", "sortBy": "unicode"})).unwrap();
    let headless = app.run("glyphData.search", &json!({"query": "zhe", "sortBy": "unicode"})).unwrap();
    let expected = result_names(&headless);
    assert!(expected.len() >= 3, "{expected:?}");
    let rows: Vec<String> = app.state.glyph_info.results.rows.iter().map(|r| r.name.clone()).collect();
    assert_eq!(rows, expected);
    assert_eq!(shown["count"], json!(expected.len()));
    let first = &app.state.glyph_info.results.rows[0];
    assert_eq!(first.unicode, headless["results"][0]["unicode"].as_str().unwrap_or_default());
    assert_eq!(first.category, headless["results"][0]["category"].as_str().unwrap_or_default());

    // The window lays out the rows in the search's order.
    let w = Window::new();
    w.frame(&mut app);
    w.frame(&mut app);
    let laid: Vec<(String, Rect)> = app.state.glyph_info.laid_out.clone();
    assert_eq!(laid.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(), expected[..laid.len()].to_vec());

    // Click, Shift+click and Cmd+click.
    w.click(&mut app, laid[0].1.center(), Modifiers::NONE);
    assert_eq!(app.state.glyph_info.selected, [laid[0].0.clone()]);
    w.click(&mut app, laid[2].1.center(), Modifiers::SHIFT);
    assert_eq!(app.state.glyph_info.selected, [laid[0].0.clone(), laid[1].0.clone(), laid[2].0.clone()]);
    w.click(&mut app, laid[1].1.center(), Modifiers::COMMAND);
    assert_eq!(app.state.glyph_info.selected, [laid[0].0.clone(), laid[2].0.clone()]);

    // Add to Font gives the font the headless command gives.
    let picked = app.state.glyph_info.selected.clone();
    let mut headless = Session::with_document();
    headless.execute("glyphInfo.addToFont", &json!({"names": picked})).unwrap();
    let add = app.state.glyph_info.add_button.unwrap();
    w.click(&mut app, add.center(), Modifiers::NONE);
    assert_eq!(glyph_names(app.session.font().unwrap()), glyph_names(headless.font().unwrap()));

    // A new query searches again and keeps only the selected rows still shown.
    app.run("window.glyphInfo", &json!({"query": "zhe-cy"})).unwrap();
    let again = result_names(&app.run("glyphData.search", &json!({"query": "zhe-cy", "sortBy": "unicode"})).unwrap());
    let rows: Vec<String> = app.state.glyph_info.results.rows.iter().map(|r| r.name.clone()).collect();
    assert_eq!(rows, again);
    assert!(app.state.glyph_info.selected.iter().all(|n| again.contains(n)));

    // An empty field lists the whole database, as the empty query does.
    let shown = app.run("window.glyphInfo", &json!({"query": "", "sortBy": "name"})).unwrap();
    let all = app.run("glyphData.search", &json!({"query": "", "sortBy": "name", "limit": 10000})).unwrap();
    assert_eq!(shown["count"], all["count"]);
    assert_eq!(all["count"], json!(10_000), "the database has more records than the window lists");
    assert_eq!(app.state.glyph_info.results.rows.first().map(|r| r.name.clone()), Some(result_names(&all)[0].clone()));

    // Refused parameters change nothing.
    let before = app.state.glyph_info.clone();
    assert!(app.run("window.glyphInfo", &json!({"sortBy": "size"})).is_err());
    assert!(app.run("window.glyphInfo", &json!({"select": ["not-a-result"]})).is_err());
    assert!(app.run("window.glyphInfo", &json!({"query": "x".repeat(65)})).is_err());
    assert_eq!(app.state.glyph_info, before);
}

/// The default glyph sets of `script`, by the built-in data.
fn defaults(script: &str) -> Vec<String> {
    let data = sets::builtin().unwrap();
    data.glyph_sets.iter().filter(|g| g.script == script && g.default).map(|g| g.id.clone()).collect()
}

#[test]
fn create_document_gives_the_document_the_headless_command_gives() {
    let mut app = FontcraftApp::new(Session::new());
    app.run("window.startWindow", &json!({})).unwrap();
    let w = Window::new();
    w.frame(&mut app);
    w.frame(&mut app);
    // No recent documents: the chooser shows, with Latin included the first time.
    let laid = app.state.start.laid_out.clone();
    assert!(laid.recent.is_empty());
    assert!(laid.scripts.iter().any(|(s, _)| s == "cyrillic"), "{:?}", laid.scripts);
    let latin = app.state.start.picks.iter().find(|p| p.script == "latin").unwrap();
    assert!(latin.included);
    assert_eq!(latin.glyph_sets, defaults("latin"));

    // Click Cyrillic, then Yes: it is included with its default sets.
    let cyrillic = laid.scripts.iter().find(|(s, _)| s == "cyrillic").unwrap().1;
    w.click(&mut app, cyrillic.center(), Modifiers::NONE);
    assert_eq!(app.state.start.script.as_deref(), Some("cyrillic"));
    let yes = app.state.start.laid_out.yes.unwrap();
    w.click(&mut app, yes.center(), Modifiers::NONE);
    let pick = app.state.start.picks.iter().find(|p| p.script == "cyrillic").unwrap();
    assert!(pick.included);
    assert_eq!(pick.glyph_sets, defaults("cyrillic"));

    let create = app.state.start.laid_out.create.unwrap();
    w.frame(&mut app);
    assert_eq!(app.state.start.laid_out.create, Some(create), "the window keeps its size");
    w.click(&mut app, create.center(), Modifiers::NONE);
    assert!(!app.state.start.open, "{:?}", app.state.start.error);
    let mut headless = Session::new();
    headless.execute("file.newFromGlyphSets", &json!({"scripts": [{"script": "latin"}, {"script": "cyrillic"}]})).unwrap();
    assert_eq!(glyph_names(app.session.font().unwrap()), glyph_names(headless.font().unwrap()));
}

#[test]
fn the_chooser_restores_the_last_picks() {
    let data = sets::builtin().unwrap();
    let greek: Vec<&str> = data.glyph_sets.iter().filter(|g| g.script == "greek").map(|g| g.id.as_str()).collect();
    let chosen = *greek.last().unwrap();
    let mut app = FontcraftApp::new(Session::new());
    app.run("file.newFromGlyphSets", &json!({"scripts": [{"script": "greek", "glyphSets": [chosen]}]})).unwrap();

    let check = |app: &FontcraftApp| {
        for p in &app.state.start.picks {
            assert_eq!(p.included, p.script == "greek", "{p:?}");
        }
        let pick = app.state.start.picks.iter().find(|p| p.script == "greek").unwrap();
        assert_eq!(pick.glyph_sets, [chosen]);
        assert_eq!(start_window::picks_param(&app.state.start.picks), json!([{"script": "greek", "glyphSets": [chosen]}]));
    };
    app.run("window.startWindow", &json!({})).unwrap();
    check(&app);

    // The next launch reads the same settings.
    let mut session = Session::new();
    *session.settings_mut() = app.session.settings().clone();
    let mut next = FontcraftApp::new(session);
    next.run("window.startWindow", &json!({})).unwrap();
    check(&next);

    // File › New from Glyph Sets… from the menu opens the chooser with them.
    let mut session = Session::new();
    *session.settings_mut() = app.session.settings().clone();
    let mut menu = FontcraftApp::new(session);
    assert!(start_window::opens_chooser(&mut menu, "file.newFromGlyphSets", &json!({})));
    assert!(!start_window::opens_chooser(&mut menu, "file.newFromGlyphSets", &json!({"scripts": [{"script": "latin"}]})));
    assert!(menu.state.start.open && menu.state.start.chooser);
    check(&menu);
}

static REVEALED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

fn fake_reveal(path: &Path) -> Result<(), String> {
    REVEALED.lock().unwrap().push(path.to_path_buf());
    Ok(())
}

#[test]
fn recent_documents_newest_first_capped_and_opened_from_the_window() {
    let dir = std::env::temp_dir().join(format!("fontcraft-recent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = |i: usize| dir.join(format!("f{i}.fontcraft")).to_string_lossy().into_owned();
    for i in 0..12 {
        std::fs::write(file(i), b"").unwrap();
    }
    let mut session = Session::new();
    for i in 0..12 {
        session.settings_mut().remember_document(Path::new(&file(i))).unwrap();
    }
    let list = start_window::recent_documents(&session);
    assert_eq!(list.len(), MAX_RECENT);
    assert_eq!(list, (2..12).rev().map(file).collect::<Vec<_>>());
    // Remembered again: first, once.
    session.settings_mut().remember_document(Path::new(&file(5))).unwrap();
    let list = start_window::recent_documents(&session);
    assert_eq!(list.len(), MAX_RECENT);
    assert_eq!(list[0], file(5));
    assert_eq!(list.iter().filter(|p| **p == file(5)).count(), 1);
    // A file that no longer exists is left out.
    std::fs::remove_file(file(11)).unwrap();
    assert!(!start_window::recent_documents(&session).contains(&file(11)));

    // Saving puts the document first; the window lists it first and a double-click opens it.
    let mut app = FontcraftApp::new(session);
    app.run("file.new", &json!({})).unwrap();
    let saved = dir.join("saved.fontcraft").to_string_lossy().into_owned();
    app.run("file.saveAs", &json!({"path": saved})).unwrap();
    assert_eq!(start_window::recent_documents(&app.session)[0], saved);
    app.run("file.close", &json!({})).unwrap();
    app.run("window.startWindow", &json!({})).unwrap();
    let w = Window::new();
    w.frame(&mut app);
    w.frame(&mut app);
    let laid = app.state.start.laid_out.recent.clone();
    assert_eq!(laid.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(), start_window::recent_documents(&app.session));
    let at = laid[0].1.center();
    w.click(&mut app, at, Modifiers::NONE);
    w.click(&mut app, at, Modifiers::NONE);
    assert_eq!(app.session.doc().unwrap().path.as_deref(), Some(Path::new(&saved)));
    assert!(!app.state.start.open);

    // By index; refused when out of range, not in the list, or not a font (the f files are empty).
    let r = app.run("file.openRecent", &json!({"index": 0})).unwrap();
    assert_eq!(r["path"], json!(saved));
    assert_eq!(start_window::recent_documents(&app.session)[1], file(5));
    assert!(app.run("file.openRecent", &json!({"index": 1})).is_err());
    assert!(app.run("file.openRecent", &json!({"index": 99})).is_err());
    assert!(app.run("file.openRecent", &json!({"path": "/nowhere/x.glyphs"})).is_err());

    // Show in Finder needs the platform's file manager and an existing file.
    assert!(app.run("file.showInFinder", &json!({"path": saved})).unwrap_err().contains("file manager"));
    app.state.start.reveal = Reveal(Some(fake_reveal));
    app.run("file.showInFinder", &json!({"path": saved})).unwrap();
    assert!(REVEALED.lock().unwrap().contains(&PathBuf::from(&saved)));
    assert!(app.run("file.showInFinder", &json!({"path": file(11)})).is_err());

    // Clearing empties the list; the window then shows the chooser.
    app.run("file.openRecent", &json!({"clear": true})).unwrap();
    assert!(start_window::recent_documents(&app.session).is_empty());
    let shown = app.run("window.startWindow", &json!({})).unwrap();
    assert_eq!(shown["chooser"], json!(true));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn show_window_on_startup_is_the_setting() {
    let mut app = FontcraftApp::new(Session::new());
    app.run("window.startWindow", &json!({"chooser": false})).unwrap();
    // With a recent document the window shows the list with the checkbox.
    let dir = std::env::temp_dir().join(format!("fontcraft-start-setting-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.fontcraft").to_string_lossy().into_owned();
    std::fs::write(&path, b"").unwrap();
    app.session.settings_mut().remember_document(Path::new(&path)).unwrap();
    let w = Window::new();
    w.frame(&mut app);
    w.frame(&mut app);
    assert!(app.state.start.laid_out.plus.is_some());
    assert_eq!(app.session.settings().get_bool(start_window::SHOW_AT_LAUNCH), Some(true));
    app.run("settings.set", &json!({"id": start_window::SHOW_AT_LAUNCH, "value": false})).unwrap();
    w.frame(&mut app);
    assert_eq!(app.session.settings().get_bool(start_window::SHOW_AT_LAUNCH), Some(false));
    // The plus button shows the chooser.
    let plus = app.state.start.laid_out.plus.unwrap();
    w.click(&mut app, plus.center(), Modifiers::NONE);
    assert!(app.state.start.chooser);
    assert!(app.state.start.laid_out.create.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}
