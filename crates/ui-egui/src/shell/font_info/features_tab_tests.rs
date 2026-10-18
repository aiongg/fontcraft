//! The Features tab in headless frames: each control queues the command it stands for, and the
//! font it gives is the font the command gives; the picker's Cancel changes nothing; typed code
//! and notes are committed when their box loses the keyboard; the queries fill the popovers.

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use serde_json::{Value, json};

use super::super::font_tab::tests::{Tab, app, font, same_as_command};
use super::{
    Act, EntryKey, Kind, NOTES_ROWS, PopoverContent, automatic_id, compile_id, divider_id, editor_id, entry_id, error_id, fix_id, label_minus_id,
    label_text_id, new_label_text_id, notes_id, perform, plus_id, remove_id, rename_id, show, show_folder_id, sidebar_error_id, snippet_item_id,
    snippets_id, spec_id, update_all_id, update_id,
};
use crate::FontcraftApp;
use crate::dialogs::feature_picker::{cancel_id, search_id};
use crate::shell::font_info::code_editor;

fn key(kind: Kind, name: &str, index: usize) -> EntryKey {
    EntryKey { kind, name: name.into(), index }
}

fn feature(app: &mut FontcraftApp, tag: &str, code: &str) {
    app.run("font.features.addFeature", &json!({"tag": tag, "code": code})).unwrap();
}

fn state(app: &FontcraftApp) -> &super::FeaturesState {
    &app.state.panels.font_info.features
}

fn key_event(key: Key) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
}

fn center(tab: &Tab, id: egui::Id) -> Pos2 {
    tab.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} is drawn")).rect.center()
}

/// Gives the code editor the keyboard with the cursor at character `at`.
fn focus_editor(tab: &Tab, app: &mut FontcraftApp, at: usize) {
    tab.frame(app, Vec::new());
    code_editor::set_cursor(&tab.ctx, editor_id(), at);
    tab.ctx.memory_mut(|m| m.request_focus(editor_id()));
    tab.frame(app, Vec::new());
    assert!(tab.ctx.memory(|m| m.has_focus(editor_id())), "the editor takes the keyboard");
}

/// Takes the keyboard from `id`; the commands the next frames queued.
fn blur(tab: &Tab, app: &mut FontcraftApp, id: egui::Id) -> Vec<(&'static str, Value)> {
    tab.ctx.memory_mut(|m| m.surrender_focus(id));
    let mut pending = tab.frame(app, Vec::new());
    pending.extend(tab.frame(app, Vec::new()));
    pending
}

#[test]
fn features_sidebar_groups_and_buttons_give_the_font_their_commands_give() {
    let mut app = app();
    feature(&mut app, "calt", "sub a by b;");
    let tab = Tab::new(show);
    tab.frame(&mut app, Vec::new());
    // Prefix, Classes, Features, always, in that order.
    let top = |id| tab.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} is drawn")).rect.top();
    assert!(top(plus_id(Kind::Prefix)) < top(plus_id(Kind::Class)));
    assert!(top(plus_id(Kind::Class)) < top(plus_id(Kind::Feature)));
    assert_eq!(state(&app).selected, [key(Kind::Feature, "calt", 0)], "the first entry is selected");
    assert!(tab.ctx.read_response(update_id(Kind::Feature, 0)).is_none(), "a manual entry has no update button");

    let pending = tab.click(&mut app, plus_id(Kind::Prefix));
    let f = same_as_command(&mut app, pending, "font.features.addPrefix", json!({}));
    assert_eq!(f.features.prefixes.len(), 1);
    let pending = tab.click(&mut app, plus_id(Kind::Class));
    let f = same_as_command(&mut app, pending, "font.features.addClass", json!({"name": "NewClass"}));
    assert_eq!(f.features.classes[0].name, "NewClass");

    let pending = tab.click(&mut app, compile_id());
    assert_eq!(pending, vec![("font.features.compile", json!({"master": "m01"}))], "Compile names the master shown");

    // Update adds liga for f_i, as an automatic entry with its update button.
    app.run("glyph.newGlyph", &json!({"name": "f_i"})).unwrap();
    let pending = tab.click(&mut app, update_all_id());
    let f = same_as_command(&mut app, pending, "font.features.update", json!({}));
    let liga = f.features.features.iter().position(|e| e.name == "liga").expect("Update adds liga");
    assert!(f.features.features[liga].automatic);
    let pending = tab.click(&mut app, update_id(Kind::Feature, liga));
    assert_eq!(pending, vec![("font.features.updateEntry", json!({"kind": "feature", "name": "liga", "index": liga}))]);

    // The minus button deletes the selected entries.
    tab.click(&mut app, entry_id(Kind::Feature, liga));
    assert_eq!(state(&app).selected, [key(Kind::Feature, "liga", liga)]);
    let pending = tab.click(&mut app, remove_id());
    let f = same_as_command(&mut app, pending, "font.features.remove", json!({"entries": [{"kind": "feature", "name": "liga", "index": liga}]}));
    assert!(f.features.features.iter().all(|e| e.name != "liga"));
}

#[test]
fn features_automatic_checkbox_turns_generation_off_and_automatic_code_is_read_only() {
    let mut app = app();
    app.run("glyph.newGlyph", &json!({"name": "f_i"})).unwrap();
    app.run("font.features.update", &json!({})).unwrap();
    let liga = font(&app).features.features.iter().position(|e| e.name == "liga").unwrap();
    let tab = Tab::new(show);
    tab.frame(&mut app, Vec::new());
    tab.click(&mut app, entry_id(Kind::Feature, liga));

    // Typing into automatic code changes nothing.
    let code = font(&app).features.features[liga].code.clone();
    focus_editor(&tab, &mut app, 0);
    assert!(tab.frame(&mut app, vec![Event::Text("x".into())]).is_empty());
    assert!(blur(&tab, &mut app, editor_id()).is_empty(), "nothing to commit");
    assert_eq!(font(&app).features.features[liga].code, code);

    let pending = tab.click(&mut app, automatic_id());
    let f = same_as_command(
        &mut app,
        pending,
        "font.features.setAutomatic",
        json!({"kind": "feature", "name": "liga", "index": liga, "automatic": false}),
    );
    assert!(!f.features.features[liga].automatic);
}

#[test]
fn typed_code_is_committed_with_set_code_when_the_editor_loses_the_keyboard() {
    let mut app = app();
    feature(&mut app, "calt", "sub a by b;");
    let tab = Tab::new(show);
    focus_editor(&tab, &mut app, 11);
    assert!(tab.frame(&mut app, vec![Event::Text(" # done".into())]).is_empty(), "typing queues nothing");
    assert_eq!(font(&app).features.features[0].code, "sub a by b;", "the code waits for the editor to lose the keyboard");
    let pending = blur(&tab, &mut app, editor_id());
    let f = same_as_command(
        &mut app,
        pending,
        "font.features.setCode",
        json!({"kind": "feature", "name": "calt", "index": 0, "code": "sub a by b; # done"}),
    );
    assert_eq!(f.features.features[0].code, "sub a by b; # done");
}

#[test]
fn features_autocomplete_return_inserts_the_highlighted_suggestion() {
    let mut app = app();
    app.run("glyph.newGlyph", &json!({"name": "f_i"})).unwrap();
    feature(&mut app, "calt", "sub a by b;");
    let tab = Tab::new(show);
    focus_editor(&tab, &mut app, 11);
    tab.frame(&mut app, vec![Event::Text(" sub f_".into())]);
    let list = state(&app).completions.clone().expect("typing a glyph name opens the suggestions");
    assert!(list.items.iter().any(|s| s.label == "f_i"), "{:?}", list.items);
    // Down moves the highlight as far as the list goes, Up back to the first.
    tab.frame(&mut app, vec![key_event(Key::ArrowDown), key_event(Key::ArrowUp)]);
    assert_eq!(state(&app).completions.as_ref().map(|c| c.highlighted), Some(0));
    let first = list.items[0].clone();
    tab.frame(&mut app, vec![key_event(Key::Enter)]);
    let expected = format!("sub a by b; sub {}", first.label);
    assert_eq!(state(&app).draft.as_ref().map(|d| d.text.as_str()), Some(expected.as_str()), "Return inserts, it starts no line");
    assert!(state(&app).completions.is_none());
    let pending = blur(&tab, &mut app, editor_id());
    same_as_command(&mut app, pending, "font.features.setCode", json!({"kind": "feature", "name": "calt", "index": 0, "code": expected}));
}

#[test]
fn feature_picker_adds_the_highlighted_feature_and_cancel_changes_nothing() {
    let mut app = app();
    feature(&mut app, "calt", "sub a by b;");
    let tab = Tab::new(show);
    tab.click(&mut app, plus_id(Kind::Feature));
    assert!(state(&app).picker.is_some(), "the plus beside Features opens the picker");
    tab.frame(&mut app, Vec::new());
    tab.ctx.memory_mut(|m| m.request_focus(search_id()));
    tab.frame(&mut app, vec![Event::Text("smcp".into())]);
    tab.frame(&mut app, Vec::new());
    let picker = state(&app).picker.clone().unwrap();
    let highlighted = picker.highlighted.and_then(|i| picker.choices.get(i)).expect("the search finds a feature");
    assert_eq!(highlighted.tag, "smcp", "{:?}", picker.choices);
    let automatic = highlighted.can_generate;
    let mut pending = tab.frame(&mut app, vec![key_event(Key::Enter)]);
    pending.extend(tab.frame(&mut app, Vec::new()));
    assert!(state(&app).picker.is_none(), "Return adds and closes");
    let f = same_as_command(&mut app, pending, "font.features.addFeature", json!({"tag": "smcp", "automatic": automatic}));
    assert!(f.features.features.iter().any(|e| e.name == "smcp"));

    let before = font(&app);
    tab.click(&mut app, plus_id(Kind::Feature));
    tab.frame(&mut app, Vec::new());
    tab.frame(&mut app, Vec::new());
    let picker = state(&app).picker.clone().unwrap();
    assert!(picker.choices.iter().find(|c| c.tag == "smcp").is_some_and(|c| c.in_font), "a tag the font has is marked");
    assert!(tab.click(&mut app, cancel_id()).is_empty());
    assert!(state(&app).picker.is_none());
    assert_eq!(font(&app), before, "Cancel changes nothing");
}

#[test]
fn double_click_renames_and_drag_reorders_entries() {
    let mut app = app();
    feature(&mut app, "calt", "sub a by b;");
    feature(&mut app, "liga", "sub f i by f_i;");
    let tab = Tab::new(show);
    tab.frame(&mut app, Vec::new());
    let at = center(&tab, entry_id(Kind::Feature, 0));
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    tab.frame(&mut app, vec![Event::PointerMoved(at)]);
    for pressed in [true, false, true, false] {
        tab.frame(&mut app, vec![button(pressed)]);
    }
    assert!(state(&app).renaming.is_some(), "a double-click starts a rename");
    let pending = tab.enter(&mut app, rename_id(), "rlig");
    same_as_command(&mut app, pending, "font.features.rename", json!({"kind": "feature", "name": "calt", "index": 0, "newName": "rlig"}));
    tab.frame(&mut app, Vec::new());
    assert!(state(&app).renaming.is_none());
    assert_eq!(state(&app).selected, [key(Kind::Feature, "rlig", 0)], "the selection follows the rename");

    // Drag the first feature onto the second.
    let from = center(&tab, entry_id(Kind::Feature, 0));
    let to = center(&tab, entry_id(Kind::Feature, 1));
    let press = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    tab.frame(&mut app, vec![Event::PointerMoved(from)]);
    tab.frame(&mut app, vec![press(from, true)]);
    let mut pending = Vec::new();
    for step in 1..=4 {
        let y = from.y + (to.y - from.y) * step as f32 / 4.0;
        pending.extend(tab.frame(&mut app, vec![Event::PointerMoved(egui::pos2(from.x, y))]));
    }
    pending.extend(tab.frame(&mut app, vec![press(to, false)]));
    pending.extend(tab.frame(&mut app, Vec::new()));
    let f = same_as_command(&mut app, pending, "font.features.reorder", json!({"kind": "feature", "order": [1, 0]}));
    let names: Vec<&str> = f.features.features.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["liga", "rlig"]);
}

#[test]
fn features_stylistic_set_names_rows_give_the_font_their_commands_give() {
    let mut app = app();
    feature(&mut app, "ss01", "sub a by a.ss01;");
    let tab = Tab::new(show);
    tab.frame(&mut app, Vec::new());
    assert!(tab.ctx.read_response(new_label_text_id()).is_some(), "a set without a name shows an empty Default row");
    let pending = tab.enter(&mut app, new_label_text_id(), "Swash Forms");
    let f =
        same_as_command(&mut app, pending, "font.features.setLabel", json!({"tag": "ss01", "index": 0, "language": "dflt", "name": "Swash Forms"}));
    assert_eq!(f.features.features[0].labels[0].value, "Swash Forms");
    tab.frame(&mut app, Vec::new());
    let pending = tab.enter(&mut app, label_text_id(0), "Swashes");
    same_as_command(&mut app, pending, "font.features.setLabel", json!({"tag": "ss01", "index": 0, "language": "dflt", "name": "Swashes"}));
    let pending = tab.click(&mut app, label_minus_id(0));
    let f = same_as_command(&mut app, pending, "font.features.removeLabel", json!({"tag": "ss01", "index": 0, "language": "dflt"}));
    assert!(f.features.features[0].labels.is_empty());
    // Other features have no name rows.
    feature(&mut app, "calt", "sub a by b;");
    tab.click(&mut app, entry_id(Kind::Feature, 1));
    assert!(tab.ctx.read_response(new_label_text_id()).is_none());
}

#[test]
fn features_notes_divider_resizes_the_notes_and_typed_notes_are_committed() {
    let mut app = app();
    feature(&mut app, "calt", "sub a by b;");
    let tab = Tab::new(show);
    tab.frame(&mut app, Vec::new());
    tab.ctx.memory_mut(|m| m.request_focus(notes_id()));
    tab.frame(&mut app, Vec::new());
    assert!(tab.frame(&mut app, vec![Event::Text("Check with the client".into())]).is_empty());
    let pending = blur(&tab, &mut app, notes_id());
    let f = same_as_command(
        &mut app,
        pending,
        "font.features.setNotes",
        json!({"kind": "feature", "name": "calt", "index": 0, "notes": "Check with the client"}),
    );
    assert_eq!(f.features.features[0].notes, "Check with the client");

    let from = center(&tab, divider_id());
    let press = |pressed| Event::PointerButton { pos: from, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    tab.frame(&mut app, vec![Event::PointerMoved(from)]);
    tab.frame(&mut app, vec![press(true)]);
    for step in 1..=3 {
        tab.frame(&mut app, vec![Event::PointerMoved(egui::pos2(from.x, from.y - 20.0 * step as f32))]);
    }
    tab.frame(&mut app, vec![press(false)]);
    let height = state(&app).notes_height.expect("the drag sets the height");
    assert!(height > app.tokens.list_row_height * NOTES_ROWS, "dragging up makes the notes taller: {height}");
}

#[test]
fn features_inline_errors_mark_the_entry_and_the_fix_gives_the_font_apply_fix_gives() {
    let mut app = app();
    app.run("glyph.newGlyph", &json!({"name": "f_i"})).unwrap();
    feature(&mut app, "calt", "sub a by b;");
    feature(&mut app, "liga", "sub f i by F_I;");
    let r = app.run("font.features.compile", &json!({})).unwrap();
    let error = r["diagnostics"].as_array().unwrap().iter().find(|d| d["severity"] == "error").unwrap()["id"].as_u64().unwrap();
    let tab = Tab::new(show);
    tab.frame(&mut app, Vec::new());
    assert!(tab.ctx.read_response(sidebar_error_id(Kind::Feature, 1)).is_some(), "liga has the error icon");
    assert!(tab.ctx.read_response(sidebar_error_id(Kind::Feature, 0)).is_none(), "calt has none");

    // The sidebar icon jumps to the error.
    tab.click(&mut app, sidebar_error_id(Kind::Feature, 1));
    assert_eq!(state(&app).selected, [key(Kind::Feature, "liga", 1)]);
    tab.frame(&mut app, Vec::new());
    assert!(tab.ctx.memory(|m| m.has_focus(editor_id())), "the editor takes the keyboard at the error");

    // The icon under the code expands the message; Fix applies the suggestion.
    tab.click(&mut app, error_id(error));
    assert!(matches!(state(&app).popover.as_ref().map(|p| &p.content), Some(PopoverContent::Error(id, Ok(_))) if *id == error));
    // The fix needs the compile to be current, so it is run once, not undone and run again.
    let pending = tab.click(&mut app, fix_id(0));
    assert_eq!(pending, vec![("font.features.applyFix", json!({"error": error, "fix": 0}))]);
    for (id, params) in &pending {
        app.run(id, params).unwrap();
    }
    assert_eq!(font(&app).features.features[1].code, "sub f i by f_i;");
    tab.frame(&mut app, Vec::new());
    assert!(state(&app).popover.is_none());
    assert!(tab.ctx.read_response(error_id(error)).is_none(), "the font changed: the markers go");
}

#[test]
fn features_name_and_token_popovers_show_what_the_queries_give() {
    let mut app = app();
    app.run("font.features.addClass", &json!({"name": "Upper", "code": "A B"})).unwrap();
    feature(&mut app, "calt", "sub @Upper by a;");
    let tab = Tab::new(show);
    tab.frame(&mut app, Vec::new());
    let at = Pos2::new(100.0, 100.0);
    perform(&mut app, &tab.ctx, Some("m01"), Act::Preview { name: "@Upper".into(), at });
    let content = state(&app).popover.as_ref().map(|p| p.content.clone());
    let Some(PopoverContent::Name(Ok(r))) = content else { panic!("a class preview: {content:?}") };
    assert_eq!(r["glyphs"], json!(["A", "B"]));
    tab.frame(&mut app, Vec::new());
    assert!(state(&app).popover.is_some(), "the popover stays until a click elsewhere or Escape");

    perform(&mut app, &tab.ctx, Some("m01"), Act::Token { token: "$[name == \"b\" OR name == \"a\"]".into(), at });
    let content = state(&app).popover.as_ref().map(|p| p.content.clone());
    let Some(PopoverContent::Token(Ok(r))) = content else { panic!("a token preview: {content:?}") };
    assert_eq!(r["glyphs"], json!(["a", "b"]));
    // Escape closes it.
    tab.frame(&mut app, vec![key_event(Key::Escape)]);
    assert!(state(&app).popover.is_none());
}

#[test]
fn features_snippets_menu_inserts_over_the_selection_and_spec_opens_the_specification() {
    let mut app = app();
    feature(&mut app, "calt", "sub a by b;");
    let tab = Tab::new(show);
    tab.click(&mut app, snippets_id());
    let listing = state(&app).snippets.as_ref().and_then(|m| m.listing.clone()).expect("the menu lists the snippets");
    let title = listing.built_in.first().cloned().expect("built-in snippets");
    app.state.panels.font_info.features.selection = Some((key(Kind::Feature, "calt", 0), 0..3));
    tab.frame(&mut app, Vec::new());
    let pending = tab.click(&mut app, snippet_item_id(None, &title));
    same_as_command(
        &mut app,
        pending,
        "font.features.insertSnippet",
        json!({"kind": "feature", "name": "calt", "index": 0, "snippet": title, "position": 0, "end": 3}),
    );
    assert!(state(&app).snippets.is_none(), "choosing a snippet closes the menu");

    // A headless session has no snippet folder: Show Snippet Folder says so and changes nothing.
    let before = font(&app);
    app.state.status.clear();
    tab.click(&mut app, snippets_id());
    tab.frame(&mut app, Vec::new());
    assert!(tab.click(&mut app, show_folder_id()).is_empty());
    assert!(!app.state.status.is_empty());
    assert_eq!(font(&app), before);

    tab.click(&mut app, spec_id());
    assert!(app.state.status.contains("calt"), "{}", app.state.status);
}
