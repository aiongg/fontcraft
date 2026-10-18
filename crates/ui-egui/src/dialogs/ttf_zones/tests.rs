//! The TTFZones and TTFStems editors in headless frames: each control queues exactly the command
//! an agent would run for the same change, and running it changes the master's list as shown.

use std::sync::Arc;

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use fontcraft_engine::fontinfo::Scope;
use fontcraft_engine::fontinfo::rows::{Row, RowKind};
use fontcraft_engine::hinting::DeltaDir;
use fontcraft_engine::model::plist::{Str, Value as PValue, openstep};
use fontcraft_glyphquery::RuleTree;
use serde_json::json;

use super::*;
use crate::app::FontcraftApp;
use crate::dialogs::tt_deltas::{cell_id, done_id as table_done_id};

fn second_master(app: &mut FontcraftApp) -> String {
    let out = app.run("font.master.duplicate", &json!({"masters": ["m01"]})).unwrap();
    out["masters"][0]["id"].as_str().unwrap().to_string()
}

fn list(app: &FontcraftApp, kind: Kind, master: &str) -> List {
    List::read(app.session.font().unwrap(), kind, master).unwrap()
}

fn run(app: &mut FontcraftApp, queued: &[(&'static str, serde_json::Value)]) {
    for (id, params) in queued {
        app.run(id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

/// `units` per 1000 units per em, rounded, in the font's units.
fn scaled(app: &FontcraftApp, units: f64) -> f64 {
    (units * f64::from(app.session.font().unwrap().units_per_em) / 1000.0).round()
}

/// A headless window: one egui context kept across frames, drawing the editors as Font Info's
/// Masters tab does for master `master`, and the value field of `row` when there is one.
struct Window {
    ctx: egui::Context,
    master: String,
    row: Option<Row>,
}

fn value_id() -> egui::Id {
    egui::Id::new("test.ttfValueField")
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default(), master: "m01".into(), row: None }
    }

    /// One frame; the commands it queued.
    fn frame(&self, app: &FontcraftApp, events: Vec<Event>) -> Pending {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let font = app.session.font().unwrap();
        let mut queued = Pending::new();
        let mut output = self.ctx.run_ui(input, |ui| {
            queued.clear();
            if let Some(row) = &self.row {
                value_field(ui, value_id(), row);
            }
            show(ui.ctx(), &app.tokens, font, Some(&self.master), &mut queued);
        });
        output.textures_delta.clear();
        queued
    }

    fn settle(&self, app: &FontcraftApp) {
        for _ in 0..3 {
            self.frame(app, Vec::new());
        }
    }

    fn drawn(&self, app: &FontcraftApp, id: egui::Id) -> bool {
        self.settle(app);
        self.ctx.read_response(id).is_some()
    }

    /// Clicks widget `id`; the commands the click queued.
    fn click(&self, app: &FontcraftApp, id: egui::Id) -> Pending {
        self.settle(app);
        let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        let mut queued = self.frame(app, vec![Event::PointerMoved(at)]);
        queued.extend(self.frame(app, vec![button(true)]));
        queued.extend(self.frame(app, vec![button(false)]));
        queued
    }

    /// Focuses field `id`, types `text` over it and presses Return; the commands queued.
    fn enter(&self, app: &FontcraftApp, id: egui::Id, text: &str) -> Pending {
        self.settle(app);
        self.ctx.memory_mut(|m| m.request_focus(id));
        let mut queued = self.frame(app, Vec::new());
        queued.extend(self.frame(app, vec![Event::Text(text.into())]));
        let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        queued.extend(self.frame(app, vec![enter]));
        queued
    }
}

#[test]
fn ttf_zones_value_field_opens_the_editor_on_the_master_font_info_shows() {
    let mut app = FontcraftApp::default();
    let second = second_master(&mut app);
    let mut w = Window::new();
    w.master = second.clone();
    w.row =
        Some(Row { scope: Scope::Font, kind: RowKind::Param, key: "TTFZones".into(), value: openstep::parse_value("()").unwrap(), enabled: true });
    assert!(!w.drawn(&app, value_id()), "the font's TTFZones (family zones) keeps the plain value field");
    w.row = Some(Row {
        scope: Scope::Master,
        kind: RowKind::Param,
        key: "TTFStems".into(),
        value: openstep::parse_value("({name = a; width = 80;})").unwrap(),
        enabled: true,
    });
    w.click(&app, value_id());
    w.settle(&app);
    let editor = open_editor(&w.ctx).expect("the value field opens the editor");
    assert_eq!((editor.kind, editor.master.as_str()), (Kind::Stems, second.as_str()));
    assert!(w.drawn(&app, plus_id(Kind::Stems)));

    // An editor whose master is gone closes instead of drawing.
    open(&w.ctx, Kind::Zones, "no such master");
    w.settle(&app);
    assert_eq!(open_editor(&w.ctx), None);
}

#[test]
fn ttf_zones_plus_minus_get_ps_and_done_run_their_commands() {
    let mut app = FontcraftApp::default();
    let w = Window::new();
    open(&w.ctx, Kind::Zones, "m01");
    let before = list(&app, Kind::Zones, "m01");
    let queued = w.click(&app, plus_id(Kind::Zones));
    assert_eq!(queued, vec![("hint.tt.addZone", json!({"master": "m01", "position": 0.0, "size": scaled(&app, 16.0)}))]);
    run(&mut app, &queued);
    let after = list(&app, Kind::Zones, "m01");
    assert_eq!(after.entries.len(), before.entries.len() + 1);
    let added = after.entries.len() - 1;

    assert!(w.click(&app, minus_id(Kind::Zones)).is_empty(), "minus does nothing without a selected row");
    w.click(&app, row_id(Kind::Zones, added, "select"));
    let queued = w.click(&app, minus_id(Kind::Zones));
    assert_eq!(queued, vec![("hint.tt.removeZone", json!({"master": "m01", "zone": after.entries[added].name}))]);
    run(&mut app, &queued);
    assert_eq!(list(&app, Kind::Zones, "m01").entries, before.entries);
    assert_eq!(open_editor(&w.ctx).unwrap().selected, None, "the removed row's selection goes with it");

    w.click(&app, actions_id(Kind::Zones));
    let queued = w.click(&app, get_ps_id(Kind::Zones));
    assert_eq!(queued, vec![("hint.tt.getPsZones", json!({"master": "m01"}))]);
    run(&mut app, &queued);
    assert!(app.session.font().unwrap().master("m01").unwrap().params.get("TTFZones").is_some(), "Get PS zones writes the parameter");

    w.click(&app, done_id(Kind::Zones));
    w.settle(&app);
    assert_eq!(open_editor(&w.ctx), None);
}

#[test]
fn ttf_zones_name_position_and_align_run_set_zone() {
    let mut app = FontcraftApp::default();
    let w = Window::new();
    open(&w.ctx, Kind::Zones, "m01");
    let zones = list(&app, Kind::Zones, "m01");
    let (first, second) = (zones.entries[0].name.clone(), zones.entries[1].name.clone());

    let queued = w.enter(&app, row_id(Kind::Zones, 0, "position"), "512");
    assert_eq!(queued, vec![("hint.tt.setZone", json!({"master": "m01", "zone": first, "position": 512.0}))]);
    run(&mut app, &queued);
    assert_eq!(list(&app, Kind::Zones, "m01").entries[0].position, 512.0);

    let queued = w.enter(&app, row_id(Kind::Zones, 0, "name"), "top");
    assert_eq!(queued, vec![("hint.tt.setZone", json!({"master": "m01", "zone": first, "name": "top"}))]);
    run(&mut app, &queued);
    assert_eq!(list(&app, Kind::Zones, "m01").entries[0].name, "top");

    // The Align pop-up's choices, as the editor turns them into commands.
    let zones = list(&app, Kind::Zones, "m01");
    let aligned = zones.command(&Control::Align(0, Some(second.clone()))).unwrap();
    assert_eq!(aligned, ("hint.tt.setZone", json!({"master": "m01", "zone": "top", "align": second})));
    run(&mut app, &[aligned]);
    let zones = list(&app, Kind::Zones, "m01");
    assert_eq!(zones.entries[0].align.as_deref(), Some(second.as_str()));
    assert_eq!(zones.command(&Control::Align(0, Some(second.clone()))), None, "the zone it is already aligned to changes nothing");
    let none = zones.command(&Control::Align(0, None)).unwrap();
    assert_eq!(none, ("hint.tt.setZone", json!({"master": "m01", "zone": "top", "align": null})));
    run(&mut app, &[none]);
    assert_eq!(list(&app, Kind::Zones, "m01").entries[0].align, None);
}

#[test]
fn ttf_zones_delta_and_filter_buttons_show_on_the_main_master_only() {
    let mut app = FontcraftApp::default();
    let second = second_master(&mut app);
    let w = Window::new();
    open(&w.ctx, Kind::Zones, "m01");
    assert!(w.drawn(&app, row_id(Kind::Zones, 0, "deltas")));
    assert!(w.drawn(&app, row_id(Kind::Zones, 0, "filter")));
    open(&w.ctx, Kind::Zones, &second);
    assert!(w.drawn(&app, row_id(Kind::Zones, 0, "name")));
    assert!(!w.drawn(&app, row_id(Kind::Zones, 0, "deltas")));
    assert!(!w.drawn(&app, row_id(Kind::Zones, 0, "filter")));
    let click = Click { column: 0, ppm: 12, value: Cell::Up };
    assert_eq!(list(&app, Kind::Zones, &second).command(&Control::Delta(0, click)), None, "no delta outside the main master");

    // Get Hints From Master names the main master.
    let named = second.clone();
    app.session
        .edit("main master", move |font| {
            Arc::make_mut(&mut font.info).params.set("Get Hints From Master", PValue::String(Str::new(named)));
            Ok(())
        })
        .unwrap();
    assert!(w.drawn(&app, row_id(Kind::Zones, 0, "deltas")));
    open(&w.ctx, Kind::Stems, "m01");
    let queued = w.click(&app, plus_id(Kind::Stems));
    run(&mut app, &queued);
    assert!(w.drawn(&app, row_id(Kind::Stems, 0, "name")));
    assert!(!w.drawn(&app, row_id(Kind::Stems, 0, "filter")), "the first master is no longer the main one");
}

#[test]
fn ttf_zones_delta_table_cells_set_the_zone_deltas() {
    let mut app = FontcraftApp::default();
    let w = Window::new();
    open(&w.ctx, Kind::Zones, "m01");
    let name = list(&app, Kind::Zones, "m01").entries[0].name.clone();
    w.click(&app, row_id(Kind::Zones, 0, "deltas"));
    let queued = w.click(&app, cell_id(0, 12));
    assert_eq!(queued, vec![("hint.tt.setZone", json!({"master": "m01", "zone": name, "deltas": {"m01": {"12": "up"}}}))]);
    run(&mut app, &queued);
    let zone = list(&app, Kind::Zones, "m01").entries[0].clone();
    assert_eq!(zone.deltas.get("m01").and_then(|sizes| sizes.get(&12)), Some(&DeltaDir::Up));
    assert!(zone.has_deltas());

    let queued = w.click(&app, cell_id(0, 12));
    assert_eq!(queued.first().map(|(_, p)| p["deltas"].clone()), Some(json!({"m01": {"12": "down"}})), "the table shows the stored cell");
    w.click(&app, table_done_id());
    w.settle(&app);
    let editor = open_editor(&w.ctx).expect("Done of the table leaves the editor open");
    assert_eq!(editor.deltas, None);
}

#[test]
fn ttf_zones_filter_sheet_is_the_rule_editor_and_sets_the_predicate() {
    let mut app = FontcraftApp::default();
    let w = Window::new();
    open(&w.ctx, Kind::Zones, "m01");
    let name = list(&app, Kind::Zones, "m01").entries[0].name.clone();
    let key = filter_key(Kind::Zones);
    w.click(&app, row_id(Kind::Zones, 0, "filter"));
    w.click(&app, rule_tree::control_id(&key, &[], "add"));
    let rules = open_editor(&w.ctx).unwrap().filter.unwrap().rules;
    assert_eq!(rule_tree::group_of(&rules).map(|(_, nodes)| nodes.len()), Some(2), "the rule editor's plus adds a rule");
    let expected = RuleTree::from_json(&rules).unwrap().compile().unwrap().to_string();
    let queued = w.click(&app, filter_button_id(Kind::Zones, "ok"));
    assert_eq!(queued, vec![("hint.tt.setZone", json!({"master": "m01", "zone": name, "filter": expected}))]);
    run(&mut app, &queued);
    assert_eq!(list(&app, Kind::Zones, "m01").entries[0].filter.as_deref(), Some(expected.as_str()));

    // Reopened, the stored predicate is one Custom rule; Clear removes the filter.
    w.click(&app, row_id(Kind::Zones, 0, "filter"));
    let sheet = open_editor(&w.ctx).unwrap().filter.unwrap();
    assert_eq!(sheet.rules, json!({"all": [{"property": "custom", "op": "is", "value": expected}]}));
    assert_eq!(sheet.predicate().unwrap().map(|p| parse_back(&p)), Some(parse_back(&expected)));
    let queued = w.click(&app, filter_button_id(Kind::Zones, "clear"));
    assert_eq!(queued, vec![("hint.tt.setZone", json!({"master": "m01", "zone": name, "filter": null}))]);
    run(&mut app, &queued);
    assert_eq!(list(&app, Kind::Zones, "m01").entries[0].filter, None);
}

/// A predicate as the parser reads it, to compare two spellings of the same one.
fn parse_back(text: &str) -> String {
    fontcraft_glyphquery::parse_predicate(text).unwrap().to_string()
}

#[test]
fn ttf_stems_plus_orientation_and_size_run_their_commands() {
    let mut app = FontcraftApp::default();
    let w = Window::new();
    open(&w.ctx, Kind::Stems, "m01");
    let queued = w.click(&app, plus_id(Kind::Stems));
    assert_eq!(queued, vec![("hint.tt.addStemDefinition", json!({"master": "m01", "orientation": "x", "size": scaled(&app, 80.0)}))]);
    run(&mut app, &queued);
    let stems = list(&app, Kind::Stems, "m01");
    let at = stems.entries.len() - 1;
    let name = stems.entries[at].name.clone();
    assert!(!stems.entries[at].horizontal, "a new stem is an X stem");

    let queued = w.click(&app, row_id(Kind::Stems, at, "orientation"));
    assert_eq!(queued, vec![("hint.tt.setStemDefinition", json!({"master": "m01", "stem": name, "orientation": "y"}))]);
    run(&mut app, &queued);
    assert!(list(&app, Kind::Stems, "m01").entries[at].horizontal);

    let queued = w.enter(&app, row_id(Kind::Stems, at, "size"), "95");
    assert_eq!(queued, vec![("hint.tt.setStemDefinition", json!({"master": "m01", "stem": name, "size": 95.0}))]);
    run(&mut app, &queued);
    assert_eq!(list(&app, Kind::Stems, "m01").entries[at].size, 95.0);
}
