//! Sample strings and Find Glyphs: the Select Sample Text window and its commands, Next and
//! Previous wrapping around, the strings kept in the user settings, and the Find Glyphs pop-up
//! inserting what `glyph.search` finds.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect};
use fontcraft_engine::settings::{Reserved, UserSettings};
use serde_json::{Value, json};

use crate::FontcraftApp;
use crate::dialogs::sample_strings::{self, Group};

/// A headless window: one egui context kept across frames, so pointer and focus carry over.
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
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: mods };
        self.frame_with(app, vec![Event::PointerMoved(at), button(true)], mods);
        self.frame_with(app, vec![button(false)], mods);
        self.frame(app);
    }

    fn key(&self, app: &mut FontcraftApp, key: Key) {
        let event = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
        self.frame_with(app, vec![event(true), event(false)], Modifiers::NONE);
        self.frame(app);
    }
}

fn text(app: &mut FontcraftApp) -> Value {
    app.run("editView.getText", &json!({})).unwrap()["items"].clone()
}

fn groups() -> Vec<Group> {
    vec![Group { name: "Round".into(), strings: vec!["HOH".into(), "/a\\nH".into()] }, Group { name: "Straight".into(), strings: vec!["OHO".into()] }]
}

/// An app with a tab on `H` and the sample strings of [`groups`] in the user settings.
fn app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.session.settings_mut().set_reserved(Reserved::SampleStrings, &groups()).unwrap();
    app.run("view.newTab", &json!({"glyphs": ["H"]})).unwrap();
    app
}

#[test]
fn select_sample_text_window_puts_the_chosen_string_into_the_tab() {
    let mut app = app();
    let w = Window::new();
    // edit.selectSampleText without a choice opens the window with the groups.
    let shown = app.run("edit.selectSampleText", &json!({})).unwrap();
    assert_eq!(shown["groups"], json!(groups()));
    assert_eq!((&shown["open"], &shown["pinned"]), (&json!(true), &json!(false)));
    w.frame(&mut app);
    w.frame(&mut app);
    let laid = app.state.panels.sample_strings.laid_out.clone();
    assert_eq!(laid.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0, 1, 2], "every string of every group, in order");

    // A click puts the string in place of the whole text, `\n` as a line break, and closes the
    // unpinned window.
    w.click(&mut app, laid[1].1.center(), Modifiers::NONE);
    assert_eq!(text(&mut app), json!(["a", {"lineBreak": true}, "H"]));
    assert!(!app.state.panels.sample_strings.open);

    // sampleStrings.togglePinned: the Pin button keeps the window open after a choice.
    app.run("edit.selectSampleText", &json!({})).unwrap();
    w.frame(&mut app);
    let pin = app.state.panels.sample_strings.pin_button.unwrap();
    w.click(&mut app, pin.center(), Modifiers::NONE);
    assert!(app.state.panels.sample_strings.pinned);
    let laid = app.state.panels.sample_strings.laid_out.clone();
    w.click(&mut app, laid[2].1.center(), Modifiers::NONE);
    assert_eq!(text(&mut app), json!(["O", "H", "O"]));
    assert!(app.state.panels.sample_strings.open, "pinned, the window stays open");
    assert_eq!(app.run("sampleStrings.togglePinned", &json!({"on": false})).unwrap(), json!({"pinned": false}));

    // Down in the window chooses the next string; it wraps to the first.
    w.key(&mut app, Key::ArrowDown);
    assert_eq!(text(&mut app), json!(["H", "O", "H"]));
    w.key(&mut app, Key::Escape);
    assert!(!app.state.panels.sample_strings.open);

    // The command chooses by index or by text, as the window does.
    let r = app.run("edit.selectSampleText", &json!({"index": 2})).unwrap();
    assert_eq!((&r["items"], &r["index"]), (&json!(["O", "H", "O"]), &json!(2)));
    app.run("edit.selectSampleText", &json!({"text": "H/Placeholder H"})).unwrap();
    assert_eq!(text(&mut app), json!(["H", {"placeholder": true}, "H"]));

    // Refused parameters change nothing.
    let before = text(&mut app);
    for bad in [json!({"index": 3}), json!({"index": 0, "text": "H"}), json!({"index": -1}), json!({"text": "H".repeat(4097)})] {
        assert!(app.run("edit.selectSampleText", &bad).is_err(), "{bad}");
    }
    assert_eq!(text(&mut app), before);
}

#[test]
fn next_and_previous_sample_string_wrap_around() {
    let mut app = app();
    let step = |app: &mut FontcraftApp, id: &str| app.run(id, &json!({})).unwrap()["index"].clone();
    // None chosen yet: Next starts at the first string, Previous at the last.
    assert_eq!(step(&mut app, "edit.selectPreviousSampleString"), json!(2));
    assert_eq!(text(&mut app), json!(["O", "H", "O"]));
    assert_eq!(step(&mut app, "edit.selectNextSampleString"), json!(0), "after the last comes the first");
    assert_eq!(text(&mut app), json!(["H", "O", "H"]));
    assert_eq!(step(&mut app, "edit.selectNextSampleString"), json!(1));
    assert_eq!(step(&mut app, "edit.selectPreviousSampleString"), json!(0));
    assert_eq!(step(&mut app, "edit.selectPreviousSampleString"), json!(2), "before the first comes the last");

    // No sample strings: refused, the text stays.
    app.session.settings_mut().set_reserved(Reserved::SampleStrings, &Vec::<Group>::new()).unwrap();
    let before = text(&mut app);
    assert!(app.run("edit.selectNextSampleString", &json!({})).is_err());
    assert_eq!(text(&mut app), before);
}

#[test]
fn sample_strings_persist_in_the_user_settings() {
    // Nothing stored: the built-in groups, every string readable as tab text.
    let mut fresh = FontcraftApp::default();
    assert_eq!(sample_strings::groups(&fresh.session), sample_strings::default_groups());
    fresh.run("view.newTab", &json!({"glyphs": ["H"]})).unwrap();
    let count: usize = sample_strings::default_groups().iter().map(|g| g.strings.len()).sum();
    for i in 0..count {
        fresh.run("edit.selectSampleText", &json!({"index": i})).unwrap();
    }

    // Stored strings come back from the settings file in a new session.
    let app = app();
    let (loaded, problems) = UserSettings::load(&app.session.settings().to_json());
    assert!(problems.is_empty(), "{problems:?}");
    let mut next = FontcraftApp::default();
    *next.session.settings_mut() = loaded;
    assert_eq!(sample_strings::groups(&next.session), groups());
    assert_eq!(next.run("settings.get", &json!({"id": "sampleStrings.groups"})).unwrap()["value"], json!(groups()));

    // An unreadable value falls back to the built-in groups.
    next.session.settings_mut().set_reserved(Reserved::SampleStrings, &json!({"not": "a list"})).unwrap();
    assert_eq!(sample_strings::groups(&next.session), sample_strings::default_groups());
}

#[test]
fn find_glyphs_inserts_what_glyph_search_finds() {
    let mut app = app();
    for name in ["Ha-cy", "Sha-cy", "sha-cy.loclBGR"] {
        app.run("glyph.newGlyph", &json!({"name": name})).unwrap();
    }
    app.run("editView.setText", &json!({"text": "HH"})).unwrap();
    app.run("editView.setCursor", &json!({"index": 1})).unwrap();
    let w = Window::new();

    // edit.find opens the pop-up; typing searches as glyph.search does and selects the first.
    app.run("edit.find", &json!({})).unwrap();
    w.frame(&mut app);
    w.frame_with(&mut app, vec![Event::Text("ha cy".into())], Modifiers::NONE);
    w.frame(&mut app);
    let headless = app.run("glyph.search", &json!({"query": "ha cy"})).unwrap();
    let expected: Vec<String> = headless["results"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(expected.len(), 3, "{expected:?}");
    let fg = &app.state.panels.find_glyphs;
    assert_eq!(fg.query, "ha cy");
    assert_eq!(fg.results.iter().map(|r| r.name.clone()).collect::<Vec<_>>(), expected);
    assert_eq!(fg.selected, [expected[0].clone()], "the first result is selected");

    // Shift+click selects a range; Return inserts the selection at the cursor and closes.
    let laid = app.state.panels.find_glyphs.laid_out.clone();
    w.click(&mut app, laid[2].1.center(), Modifiers::SHIFT);
    assert_eq!(app.state.panels.find_glyphs.selected.len(), 3);
    w.key(&mut app, Key::Enter);
    let mut items = vec![json!("H")];
    items.extend(expected.iter().map(|n| json!(n)));
    items.push(json!("H"));
    assert_eq!(text(&mut app), json!(items));
    assert!(!app.state.panels.find_glyphs.open);

    // Over the control channel the command does the same: Unicode mode, a typed character.
    app.run("editView.setText", &json!({"text": "H"})).unwrap();
    let r = app.run("edit.find", &json!({"query": "0425", "mode": "unicode", "insert": true})).unwrap();
    assert_eq!(r["inserted"], json!(["Ha-cy"]));
    let r = app.run("edit.find", &json!({"query": "Ш", "insert": true})).unwrap();
    assert_eq!(r["inserted"], json!(["Sha-cy"]));
    assert_eq!(text(&mut app), json!(["H", "Ha-cy", "Sha-cy"]));
    let r = app.run("edit.find", &json!({"query": "cy", "select": ["sha-cy.loclBGR", "Ha-cy"], "insert": true})).unwrap();
    assert_eq!(r["inserted"], json!(["Ha-cy", "sha-cy.loclBGR"]), "in list order");

    // Refused parameters change nothing.
    let before = text(&mut app);
    for bad in [
        json!({"query": "cy", "select": ["H"], "insert": true}),
        json!({"query": "zzz", "insert": true}),
        json!({"mode": "regex"}),
        json!({"query": "x".repeat(65)}),
    ] {
        assert!(app.run("edit.find", &bad).is_err(), "{bad}");
    }
    assert_eq!(text(&mut app), before);
}
