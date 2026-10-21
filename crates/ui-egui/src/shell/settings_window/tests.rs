//! The Settings window: opening on a section, refused sections, controls that write their
//! settings, refused values that change nothing, and a frame of every section.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect};
use serde_json::{Value, json};

use super::SECTIONS;
use crate::FontcraftApp;
use fontcraft_engine::extension_index::SOURCES_SETTING;

/// A headless window: one egui context kept across frames, so focus and pointer state carry over.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    fn frame_with(&self, app: &mut FontcraftApp, events: Vec<Event>) {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
    }

    fn frame(&self, app: &mut FontcraftApp) {
        self.frame_with(app, Vec::new());
    }

    fn click(&self, app: &mut FontcraftApp, at: Pos2) {
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.frame_with(app, vec![Event::PointerMoved(at), button(true)]);
        self.frame_with(app, vec![button(false)]);
        self.frame(app);
    }

    /// Clicks the control drawn for `key` in the last frame.
    fn click_control(&self, app: &mut FontcraftApp, key: &str) {
        let rect = *app.state.panels.settings_window.laid_out.get(key).unwrap_or_else(|| panic!("{key} was not drawn"));
        self.click(app, rect.center());
    }

    /// Clicks a text field, types `text` and presses Return.
    fn type_into(&self, app: &mut FontcraftApp, key: &str, text: &str) {
        self.click_control(app, key);
        self.frame_with(app, vec![Event::Text(text.into())]);
        self.frame_with(app, vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
        self.frame(app);
    }
}

fn stored(app: &FontcraftApp, id: &str) -> Value {
    app.session.settings().get(id).unwrap()
}

#[test]
fn app_settings_opens_on_a_section_and_refuses_sections_not_built() {
    let mut app = FontcraftApp::default();
    let r = app.run("app.settings", &json!({})).unwrap();
    assert_eq!((r["open"].as_bool(), r["section"].as_str()), (Some(true), Some("userSettings")));
    let ids: Vec<&str> = r["sections"].as_array().unwrap().iter().filter_map(|s| s["id"].as_str()).collect();
    assert_eq!(ids, ["updates", "appearance", "userSettings", "sampleStrings", "sharing", "addons", "shortcuts"]);
    assert_eq!(r["sections"][0]["available"], false);
    assert!(r["sections"][0]["reason"].is_string());

    assert_eq!(app.run("app.settings", &json!({"section": "addons"})).unwrap()["section"], "addons");
    // A section that is not built, or does not exist, is refused and the window stays as it was.
    let e = app.run("app.settings", &json!({"section": "updates"})).unwrap_err();
    assert!(e.contains("not available"), "{e}");
    assert!(app.run("app.settings", &json!({"section": "colours"})).is_err());
    assert!(app.run("app.settings", &json!({"section": 3})).is_err());
    let st = &app.state.panels.settings_window;
    assert_eq!((st.open, st.section.as_str()), (true, "addons"));

    // Closed and opened again with no section: the section last shown.
    assert_eq!(app.run("app.settings", &json!({"open": false})).unwrap()["open"], false);
    assert_eq!(app.run("app.settings", &json!({})).unwrap()["section"], "addons");
}

#[test]
fn user_settings_controls_write_their_settings_and_a_refused_value_changes_nothing() {
    let mut app = FontcraftApp::default();
    app.run("app.settings", &json!({"section": "userSettings"})).unwrap();
    let w = Window::new();
    w.frame(&mut app);
    w.frame(&mut app);

    for id in ["user.keepGlyphNamesFromImportedFiles", "user.useVersions", "user.sendCrashReports"] {
        assert_eq!(stored(&app, id), false, "{id} is off by default");
        w.click_control(&mut app, id);
        assert_eq!(stored(&app, id), true, "{id}: the checkbox writes the setting");
    }
    w.click_control(&mut app, "user.useVersions");
    assert_eq!(stored(&app, "user.useVersions"), false);

    w.type_into(&mut app, "user.crashReportName", "Ada");
    assert_eq!(stored(&app, "user.crashReportName"), "Ada");

    // Text over the limit is refused by settings.set: the setting keeps its value and the field
    // shows it again.
    let long = "x".repeat(fontcraft_engine::settings::MAX_TEXT_BYTES + 1);
    w.type_into(&mut app, "user.crashReportEmail", &long);
    assert_eq!(stored(&app, "user.crashReportEmail"), "");
    assert!(app.state.panels.settings_window.drafts.is_empty());

    // The same refusals headless: a wrong kind changes nothing.
    let revision = app.session.settings().revision();
    assert!(app.run("settings.set", &json!({"id": "user.sendCrashReports", "value": "yes"})).is_err());
    assert!(app.run("settings.set", &json!({"id": "user.crashReportEmail", "value": 5})).is_err());
    assert_eq!(app.session.settings().revision(), revision);
    assert_eq!(stored(&app, "user.sendCrashReports"), true);

    // Typing without leaving the field and then closing the window writes the text.
    w.click_control(&mut app, "user.crashReportEmail");
    w.frame_with(&mut app, vec![Event::Text("ada@example.com".into())]);
    app.run("app.settings", &json!({"open": false})).unwrap();
    assert_eq!(stored(&app, "user.crashReportEmail"), "ada@example.com");
}

#[test]
fn every_section_draws_and_the_switcher_skips_sections_not_built() {
    let mut app = FontcraftApp::default();
    let dir = std::env::temp_dir().join(format!("fontcraft-settings-window-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let index = dir.join("index.json");
    std::fs::write(&index, br#"{"format": "fontcraft.extensions", "version": 1, "entries": []}"#).unwrap();
    app.run("settings.set", &json!({"id": SOURCES_SETTING, "value": index.display().to_string()})).unwrap();

    app.run("app.settings", &json!({})).unwrap();
    let w = Window::new();
    for section in SECTIONS.iter().filter(|s| s.body.is_some()) {
        app.run("app.settings", &json!({"section": section.id})).unwrap();
        w.frame(&mut app);
        w.frame(&mut app);
        let laid_out = &app.state.panels.settings_window.laid_out;
        for s in SECTIONS {
            assert!(laid_out.contains_key(&format!("section:{}", s.id)), "{}: the switcher lists {}", section.id, s.id);
        }
    }
    // The Extensions section reads the index list as the Extension Manager does.
    let indexes = &app.state.panels.extensions.manager.indexes;
    assert_eq!(indexes.len(), 1, "{indexes:?}");
    assert_eq!(indexes[0]["source"], index.display().to_string());

    // User Settings draws every control of the handbook's list.
    app.run("app.settings", &json!({"section": "userSettings"})).unwrap();
    w.frame(&mut app);
    for id in [
        "user.keepGlyphNamesFromImportedFiles",
        "user.disableAutomaticAlignmentForImportedFiles",
        "user.disableLocalization",
        "user.useVersions",
        "user.sendCrashReports",
        "user.crashReportName",
        "user.crashReportEmail",
    ] {
        assert!(app.state.panels.settings_window.laid_out.contains_key(id), "{id} is drawn");
    }

    // A greyed section does not open; an available one does.
    w.click_control(&mut app, "section:updates");
    assert_eq!(app.state.panels.settings_window.section, "userSettings");
    w.click_control(&mut app, "section:addons");
    assert_eq!(app.state.panels.settings_window.section, "addons");
    let _ = std::fs::remove_dir_all(&dir);
}
