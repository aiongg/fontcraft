//! The Font tab in headless frames: each control queues the command it stands for, and the font
//! it gives is the font the command gives; the scale prompt's Cancel changes nothing. The
//! harness ([`Tab`]) is shared with the Other Settings and Notes tests.

use std::sync::Arc;

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use fontcraft_engine::fontinfo::Scope;
use fontcraft_engine::fontinfo::rows::RowKind;
use fontcraft_model::Font;
use serde_json::{Value, json};

use super::super::rows::{plus_id, row_id};
use super::super::{Picker, TabFn};
use crate::FontcraftApp;
use crate::panels::Pending;

/// One tab drawn alone in a context of its own, kept across frames so focus and field state
/// carry over.
pub(crate) struct Tab {
    pub ctx: egui::Context,
    draw: TabFn,
}

impl Tab {
    pub fn new(draw: TabFn) -> Self {
        Self { ctx: egui::Context::default(), draw }
    }

    /// One frame; the commands it queued.
    pub fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) -> Pending {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut pending = Vec::new();
        let mut output = self.ctx.run_ui(input, |ui| {
            // A pass egui discards runs again: keep only the last pass's commands.
            pending.clear();
            app.state.panels.font_info.shown_rows.clear();
            (self.draw)(app, ui, &mut pending);
        });
        output.textures_delta.clear();
        pending
    }

    /// Clicks the control `id`; the commands the release queued.
    pub fn click(&self, app: &mut FontcraftApp, id: egui::Id) -> Pending {
        self.frame(app, Vec::new());
        let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} is drawn")).rect.center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.frame(app, vec![Event::PointerMoved(at)]);
        self.frame(app, vec![button(true)]);
        let mut pending = self.frame(app, vec![button(false)]);
        pending.extend(self.frame(app, Vec::new()));
        pending
    }

    /// Focuses the field `id`, types `text` over its contents and presses Return; the commands
    /// that queued.
    pub fn enter(&self, app: &mut FontcraftApp, id: egui::Id, text: &str) -> Pending {
        self.frame(app, Vec::new());
        self.ctx.memory_mut(|m| m.request_focus(id));
        self.frame(app, Vec::new());
        assert!(self.ctx.memory(|m| m.has_focus(id)), "{id:?} takes the keyboard");
        let mut pending = self.frame(app, vec![Event::Text(text.into())]);
        pending.extend(
            self.frame(app, vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]),
        );
        pending.extend(self.frame(app, Vec::new()));
        pending
    }
}

/// A document with a fixed clock, so edits made at different moments stamp the same time.
pub(crate) fn app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.session.set_clock(Some(Arc::new(|| Some(1_760_184_000))));
    app
}

pub(crate) fn font(app: &FontcraftApp) -> Font {
    app.session.font().unwrap().clone()
}

/// Runs what a control queued, checks it is the one command `id` with `params`, undoes it, runs
/// the command directly, and checks both give the same font. The font after the control.
pub(crate) fn same_as_command(app: &mut FontcraftApp, pending: Pending, id: &str, params: Value) -> Font {
    assert_eq!(pending, vec![(id, params.clone())], "the control queues `{id}`");
    let before = font(app);
    for (cmd, p) in &pending {
        app.run(cmd, p).unwrap_or_else(|e| panic!("{cmd}: {e}"));
    }
    let through_control = font(app);
    assert_ne!(through_control, before, "`{id}` changed the font");
    app.run("edit.undo", &json!({})).unwrap();
    assert_eq!(font(app), before, "undo takes the change back");
    app.run(id, &params).unwrap();
    assert_eq!(font(app), through_control, "the control gives the font `{id}` gives");
    through_control
}

fn field(key: &str) -> egui::Id {
    row_id(Scope::Font, RowKind::Field, key, 0).with("value")
}

#[test]
fn font_tab_fields_give_the_font_their_commands_give() {
    let mut app = app();
    let tab = Tab::new(super::show);
    tab.frame(&mut app, Vec::new());
    let shown: Vec<String> = app.state.panels.font_info.shown_rows.iter().map(|r| r.key.clone()).collect();
    assert_eq!(shown, ["familyName", "unitsPerEm", "versionMajor", "versionMinor"], "a font without a date has no date row to copy");

    let pending = tab.enter(&mut app, field("familyName"), "Cafe Sans");
    let f = same_as_command(&mut app, pending, "font.info.set", json!({"scope": "font", "values": {"familyName": "Cafe Sans"}}));
    assert_eq!(f.family_name, "Cafe Sans");

    let pending = tab.enter(&mut app, field("unitsPerEm"), "2048");
    let f = same_as_command(&mut app, pending, "font.info.set", json!({"scope": "font", "values": {"unitsPerEm": 2048}}));
    assert_eq!(f.units_per_em, 2048);

    let pending = tab.enter(&mut app, super::version_major_id(), "2");
    let f = same_as_command(&mut app, pending, "font.info.set", json!({"scope": "font", "values": {"versionMajor": 2}}));
    assert_eq!(f.version_major, 2);

    let pending = tab.enter(&mut app, super::version_minor_id(), "5");
    let f = same_as_command(&mut app, pending, "font.info.set", json!({"scope": "font", "values": {"versionMinor": 5}}));
    assert_eq!(f.version_minor, 5);

    let pending = tab.click(&mut app, super::version_step_id(true));
    let f = same_as_command(&mut app, pending, "font.info.stepVersion", json!({"delta": 1}));
    assert_eq!(f.version_minor, 6);
    let pending = tab.click(&mut app, super::version_step_id(false));
    let f = same_as_command(&mut app, pending, "font.info.stepVersion", json!({"delta": -1}));
    assert_eq!(f.version_minor, 5);

    let pending = tab.enter(&mut app, super::date_field_id(), "2026-10-11 12:00:00 +0000");
    let f = same_as_command(&mut app, pending, "font.info.set", json!({"scope": "font", "values": {"date": "2026-10-11 12:00:00 +0000"}}));
    assert_eq!(f.info.date.as_deref(), Some("2026-10-11 12:00:00 +0000"));

    let pending = tab.click(&mut app, super::date_update_id());
    let f = same_as_command(&mut app, pending, "font.info.updateDate", json!({}));
    assert_ne!(f.info.date.as_deref(), Some("2026-10-11 12:00:00 +0000"), "the update button sets the clock's time");
}

#[test]
fn the_minor_version_stepper_stops_at_zero_and_changes_nothing() {
    let mut app = app();
    let tab = Tab::new(super::show);
    let before = font(&app);
    assert_eq!(before.version_minor, 0);
    let pending = tab.click(&mut app, super::version_step_id(false));
    assert_eq!(pending, vec![("font.info.stepVersion", json!({"delta": -1}))]);
    for (id, p) in pending {
        assert!(app.run(id, &p).is_err(), "a step below 0 is refused");
    }
    assert_eq!(font(&app), before);
}

#[test]
fn the_scale_prompt_scales_like_the_command_and_cancel_changes_nothing() {
    let mut app = app();
    let tab = Tab::new(super::show);
    let before = font(&app);
    let is_open = |tab: &Tab| tab.ctx.data(|d| d.get_temp::<super::ScalePrompt>(super::scale_prompt_key())).is_some();

    // Cancel: the prompt closes, nothing is queued and the font stays as it was.
    assert!(tab.click(&mut app, super::scale_button_id()).is_empty(), "the button only opens the prompt");
    assert!(is_open(&tab));
    assert!(tab.enter(&mut app, super::scale_field_id(), "2048").is_empty(), "typing a size queues nothing");
    assert!(tab.click(&mut app, super::scale_cancel_id()).is_empty(), "Cancel runs nothing");
    assert!(!is_open(&tab), "Cancel closes the prompt");
    assert_eq!(font(&app), before);

    // Scale: the prompt starts at the font's size and queues the scaling command.
    tab.click(&mut app, super::scale_button_id());
    let prompt = tab.ctx.data(|d| d.get_temp::<super::ScalePrompt>(super::scale_prompt_key()));
    assert_eq!(prompt, Some(super::ScalePrompt { units_per_em: 1000 }));
    tab.enter(&mut app, super::scale_field_id(), "2048");
    let pending = tab.click(&mut app, super::scale_ok_id());
    assert!(!is_open(&tab), "Scale closes the prompt");
    let f = same_as_command(&mut app, pending, "font.info.scaleToUnitsPerEm", json!({"unitsPerEm": 2048}));
    assert_eq!(f.units_per_em, 2048);
}

#[test]
fn the_family_name_warning_matches_the_commands_warning() {
    let mut app = app();
    for name in ["Cafe Sans", "Café & Co", "Sans-Serif", "Mono 2"] {
        let r = app.run("font.info.set", &json!({"values": {"familyName": name}})).unwrap();
        let warned = r.get("warnings").and_then(Value::as_array).is_some_and(|w| !w.is_empty());
        assert_eq!(super::family_name_needs_warning(name), warned, "`{name}`");
    }
}

#[test]
fn the_version_row_selects_both_parts_and_the_plus_opens_the_field_picker() {
    let mut app = app();
    let tab = Tab::new(super::show);
    tab.click(&mut app, super::version_label_id());
    let fi = &app.state.panels.font_info;
    let keys: Vec<&str> = fi.selected_rows.iter().filter_map(|i| fi.shown_rows.get(*i)).map(|r| r.key.as_str()).collect();
    assert_eq!(keys, ["versionMajor", "versionMinor"]);
    tab.click(&mut app, plus_id(Scope::Font, "general"));
    assert!(matches!(app.state.panels.font_info.picker, Some(Picker::Field(_))));
    // Added fields follow the built-in ones and act through the shared rows' commands.
    app.state.panels.font_info.picker = None;
    app.run("font.info.addField", &json!({"scope": "font", "key": "designers", "value": "Ann"})).unwrap();
    tab.frame(&mut app, Vec::new());
    let shown: Vec<String> = app.state.panels.font_info.shown_rows.iter().map(|r| r.key.clone()).collect();
    assert_eq!(shown, ["familyName", "unitsPerEm", "versionMajor", "versionMinor", "designers"]);
}

/// Drags the handle `from` onto the row `to`; the commands the frames queued.
fn drag(tab: &Tab, app: &mut FontcraftApp, from: egui::Id, to: egui::Id) -> Pending {
    tab.frame(app, Vec::new());
    let center = |id: egui::Id| tab.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} is drawn")).rect.center();
    let (from, to) = (center(from), center(to));
    let press = |pressed: bool, pos: Pos2| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let mut pending = Vec::new();
    for events in [
        vec![Event::PointerMoved(from)],
        vec![press(true, from)],
        vec![Event::PointerMoved(from + egui::vec2(0.0, 10.0))],
        vec![Event::PointerMoved(to)],
        vec![Event::PointerMoved(to)],
        vec![press(false, to)],
        Vec::new(),
    ] {
        pending.extend(tab.frame(app, events));
    }
    pending
}

fn axes(f: &Font) -> Vec<(String, String, bool)> {
    f.axes.iter().map(|a| (a.name.clone(), a.tag.clone(), a.hidden)).collect()
}

#[test]
fn axes_plus_predefined_menu_fields_hidden_drag_and_minus_give_the_font_the_axis_commands_give() {
    let mut app = app();
    let tab = Tab::new(super::show);
    let pending = tab.click(&mut app, super::axes_plus_id());
    let f = same_as_command(&mut app, pending, "font.axes.add", json!({}));
    assert_eq!(axes(&f), [("Weight".to_string(), "wght".to_string(), false)], "the first axis is Weight");
    let pending = tab.click(&mut app, super::axes_plus_id());
    same_as_command(&mut app, pending, "font.axes.add", json!({}));

    // A predefined name from the menu sets the tag too.
    assert!(tab.click(&mut app, super::axis_id(1, "menu")).is_empty(), "the button only opens the menu");
    let pending = tab.click(&mut app, super::predefined_item_id(1, "wdth"));
    let f = same_as_command(&mut app, pending, "font.axes.set", json!({"axis": 1, "name": "Width"}));
    assert_eq!(f.axes.get(1).map(|a| (a.name.as_str(), a.tag.as_str())), Some(("Width", "wdth")));

    let pending = tab.click(&mut app, super::axis_id(1, "hidden"));
    let f = same_as_command(&mut app, pending, "font.axes.set", json!({"axis": 1, "hidden": true}));
    assert!(f.axes.get(1).is_some_and(|a| a.hidden));

    // Dropping Width onto Weight's row moves it first.
    let pending = drag(&tab, &mut app, super::axis_id(1, "handle"), super::axis_id(0, "row"));
    let f = same_as_command(&mut app, pending, "font.axes.reorder", json!({"order": [1, 0]}));
    let tags: Vec<&str> = f.axes.iter().map(|a| a.tag.as_str()).collect();
    assert_eq!(tags, ["wdth", "wght"]);

    // A private axis: its name and tag fields; a lowercase private tag gets the hint.
    let pending = tab.enter(&mut app, super::axis_id(1, "name"), "Swash Length");
    same_as_command(&mut app, pending, "font.axes.set", json!({"axis": 1, "name": "Swash Length"}));
    let pending = tab.enter(&mut app, super::axis_id(1, "tag"), "swln");
    let f = same_as_command(&mut app, pending, "font.axes.set", json!({"axis": 1, "tag": "swln"}));
    assert_eq!(f.axes.get(1).map(|a| (a.name.as_str(), a.tag.as_str())), Some(("Swash Length", "swln")));
    tab.frame(&mut app, Vec::new());
    assert!(tab.ctx.read_response(super::axis_hint_id(1, 0)).is_some(), "a lowercase private tag gets the hint");
    assert!(tab.ctx.read_response(super::axis_hint_id(0, 0)).is_none(), "a registered tag gets none");

    let pending = tab.click(&mut app, super::axis_id(0, "minus"));
    let f = same_as_command(&mut app, pending, "font.axes.remove", json!({"axis": 0}));
    assert_eq!(axes(&f), [("Swash Length".to_string(), "swln".to_string(), false)]);
}

#[test]
fn the_axes_show_exactly_the_warnings_font_axes_list_gives() {
    let mut app = app();
    let tab = Tab::new(super::show);
    app.run("font.axes.add", &json!({"name": "Weight"})).unwrap();
    app.run("font.axes.add", &json!({"name": "Swash", "tag": "Ab"})).unwrap();
    app.run("font.axes.add", &json!({"name": "Grade", "tag": "GRAD"})).unwrap();
    let check = |app: &mut FontcraftApp| {
        tab.frame(app, Vec::new());
        let reply = app.run("font.axes.list", &json!({})).unwrap();
        let listed: Vec<Vec<String>> = reply["axes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect())
            .collect();
        assert_eq!(*super::axis_warnings(&tab.ctx, &mut app.session), listed, "the tab keeps the query's warnings");
        for (at, warnings) in listed.iter().enumerate() {
            for n in 0..warnings.len() {
                assert!(tab.ctx.read_response(super::axis_hint_id(at, n)).is_some(), "axis {at} shows warning {n}");
            }
            assert!(tab.ctx.read_response(super::axis_hint_id(at, warnings.len())).is_none(), "axis {at} shows no more");
        }
        listed
    };
    let listed = check(&mut app);
    assert!(listed[0].is_empty(), "a registered tag has none: {listed:?}");
    assert_eq!(listed[1].len(), 2, "a short private tag is padded and not four capitals: {listed:?}");
    assert!(listed[2].is_empty(), "four capitals make a good private tag: {listed:?}");
    // An edit is a new revision: the tab asks again.
    app.run("font.axes.set", &json!({"axis": 1, "tag": "SWSH"})).unwrap();
    let listed = check(&mut app);
    assert!(listed[1].is_empty(), "{listed:?}");
}

#[test]
fn two_axes_with_one_tag_are_hidden_reordered_and_warned_by_index() {
    // Files can hold two axes with one tag; the font is built as such a file opens.
    let mut app = app();
    app.session
        .edit("two wght axes", |font| {
            *Arc::make_mut(&mut font.axes) = vec![
                fontcraft_model::Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() },
                fontcraft_model::Axis { name: "Weight Too".into(), tag: "wght".into(), ..Default::default() },
            ];
            Ok(())
        })
        .unwrap();
    let tab = Tab::new(super::show);
    tab.frame(&mut app, Vec::new());
    assert!(tab.ctx.read_response(super::axis_hint_id(1, 0)).is_some(), "the shared tag is warned of");

    let pending = tab.click(&mut app, super::axis_id(1, "hidden"));
    let f = same_as_command(&mut app, pending, "font.axes.set", json!({"axis": 1, "hidden": true}));
    let hidden: Vec<bool> = f.axes.iter().map(|a| a.hidden).collect();
    assert_eq!(hidden, [false, true], "the second axis is hidden, not the first");

    let pending = drag(&tab, &mut app, super::axis_id(1, "handle"), super::axis_id(0, "row"));
    let f = same_as_command(&mut app, pending, "font.axes.reorder", json!({"order": [1, 0]}));
    let names: Vec<&str> = f.axes.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Weight Too", "Weight"]);
}
