//! The TrueType Instructor's Info box and its Delta table in headless frames: each control queues
//! the command it stands for, with the edited glyph, the active layer and the selected
//! instructions of the kinds that command takes; run, the command changes the font as it does
//! when an agent runs it with the same parameters.

use std::sync::Arc;

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use fontcraft_engine::hinting::tt::delta_column_key;
use fontcraft_engine::hinting::{Deltas, NodeRef, StemRef, TtPoint, set_tt_instructions};
use fontcraft_model::{Instance, Path, Shape};
use serde_json::json;

use super::*;
use crate::dialogs::tt_deltas::cell_id;

fn pt(path: usize, node: usize) -> TtPoint {
    TtPoint::Node(NodeRef::new(path, node))
}

fn snap(node: usize) -> TtInstruction {
    TtInstruction::new(TtKind::Snap { zone: ZoneRef::Auto }, Direction::Vertical, pt(0, node))
}

fn stem(origin: usize, target: usize) -> TtInstruction {
    let mut t = TtInstruction::new(TtKind::Stem { stem: StemRef::Auto, triple: None }, Direction::Vertical, pt(0, target));
    t.origin = Some(pt(0, origin));
    t.rounding = TtInstruction::default_rounding(&t.kind);
    t
}

fn shift(origin: usize, target: usize) -> TtInstruction {
    let mut t = TtInstruction::new(TtKind::Shift, Direction::Vertical, pt(0, target));
    t.origin = Some(pt(0, origin));
    t.rounding = TtInstruction::default_rounding(&t.kind);
    t
}

fn delta(node: usize) -> TtInstruction {
    TtInstruction::new(TtKind::Delta { cells: Deltas::new() }, Direction::Vertical, pt(0, node))
}

/// Glyph `I` drawn as a rectangle with the instructions `tt` on master `m01`, `Bold` at weight
/// 700 as the font's only instance, and a tab editing `I`.
fn app(tt: &[TtInstruction]) -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.session
        .edit("setup", |font| {
            font.instances = Arc::new(vec![Instance { name: "Bold".into(), location: vec![700.0], ..Instance::default() }]);
            let mut g = font.glyph_mut("I")?;
            let layer = g.layer_mut("m01").unwrap();
            layer.shapes = vec![Shape::Path(Path::rect(100.0, 0.0, 200.0, 700.0))];
            set_tt_instructions(layer, tt).unwrap();
            Ok(())
        })
        .unwrap();
    app.run("view.newTab", &json!({"glyphs": ["I"]})).unwrap();
    app
}

fn list(app: &FontcraftApp) -> Vec<TtInstruction> {
    let layer = app.session.font().unwrap().glyph("I").unwrap().layer("m01").unwrap();
    tt_instructions(layer).unwrap().into_iter().map(|(_, t)| t).collect()
}

fn selecting(selected: &[usize]) -> InstructorView {
    InstructorView { selected: selected.iter().copied().collect(), ..InstructorView::default() }
}

/// Runs what the box queued, then the same commands on a fresh app built as `fresh`, and checks
/// that both leave the same instructions.
fn run_and_compare(app: &mut FontcraftApp, queued: &[(&'static str, serde_json::Value)], fresh: &[TtInstruction]) {
    let mut direct = self::app(fresh);
    for (id, params) in queued {
        app.run(id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
        direct.run(id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    assert_eq!(list(app), list(&direct));
}

/// A headless window: one egui context kept across frames.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    /// One frame of the box for `view`; the commands it queued.
    fn frame(&self, app: &mut FontcraftApp, view: &InstructorView, events: Vec<Event>) -> Pending {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut queued = Pending::new();
        let mut output = self.ctx.run_ui(input, |ui| {
            queued.clear();
            show_view(app, ui, &mut queued, view);
        });
        output.textures_delta.clear();
        queued
    }

    fn settle(&self, app: &mut FontcraftApp, view: &InstructorView) {
        for _ in 0..3 {
            self.frame(app, view, Vec::new());
        }
    }

    fn drawn(&self, app: &mut FontcraftApp, view: &InstructorView, id: egui::Id) -> bool {
        self.settle(app, view);
        self.ctx.read_response(id).is_some()
    }

    /// Clicks widget `id`; the commands the click queued.
    fn click(&self, app: &mut FontcraftApp, view: &InstructorView, id: egui::Id) -> Pending {
        self.settle(app, view);
        let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        let mut queued = self.frame(app, view, vec![Event::PointerMoved(at)]);
        queued.extend(self.frame(app, view, vec![button(true)]));
        queued.extend(self.frame(app, view, vec![button(false)]));
        queued
    }

    /// Focuses field `id`, types `text` over it and presses Return; the commands queued.
    fn enter(&self, app: &mut FontcraftApp, view: &InstructorView, id: egui::Id, text: &str) -> Pending {
        self.settle(app, view);
        self.ctx.memory_mut(|m| m.request_focus(id));
        let mut queued = self.frame(app, view, Vec::new());
        queued.extend(self.frame(app, view, vec![Event::Text(text.into())]));
        queued.extend(self.frame(
            app,
            view,
            vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
        ));
        queued
    }
}

#[test]
fn info_box_tt_rounding_buttons_set_the_rounding_of_the_selected_instructions_but_deltas() {
    let tt = [snap(0), snap(1), delta(2)];
    let mut app = app(&tt);
    let view = selecting(&[0, 2]);
    let w = Window::new();
    let queued = w.click(&mut app, &view, rounding_id("up"));
    assert_eq!(queued, vec![("hint.tt.setRounding", json!({"glyph": "I", "layer": "m01", "instructions": [0], "rounding": "up"}))]);
    run_and_compare(&mut app, &queued, &tt);
    let after = list(&app);
    assert_eq!(after.first().map(|t| t.rounding), Some(Rounding::Up));
    assert_eq!(after.get(1).map(|t| t.rounding), Some(Rounding::Round), "an instruction not selected keeps its rounding");
}

#[test]
fn info_box_tt_controls_follow_the_kinds_selected() {
    let tt = [snap(0), delta(1)];
    let mut app = app(&tt);
    let w = Window::new();
    let none = selecting(&[]);
    assert!(w.drawn(&mut app, &none, ppm_id()), "the pixel size is always there");
    assert!(w.drawn(&mut app, &none, direction_id()), "the direction is always there");
    assert!(!w.drawn(&mut app, &none, rounding_id("round")));
    assert!(!w.drawn(&mut app, &none, delta_id()));
    let deltas = selecting(&[1]);
    assert!(!w.drawn(&mut app, &deltas, rounding_id("round")), "a Delta has no rounding");
    assert!(w.drawn(&mut app, &deltas, delta_id()));
    let snaps = selecting(&[0]);
    assert!(w.drawn(&mut app, &snaps, rounding_id("gdiOnly")));
    assert!(!w.drawn(&mut app, &snaps, delta_id()));
}

#[test]
fn info_box_tt_zone_and_stem_pop_ups_name_only_their_own_instructions() {
    let tt = [snap(0), stem(1, 2), shift(0, 3)];
    let mut app = app(&tt);
    let b = TtBox::read(&app, &selecting(&[0, 1, 2])).unwrap();
    assert_eq!(b.zone, Some(Choice::Auto));
    assert_eq!(b.stem, Some(Choice::Auto));
    assert_eq!(b.rounding, None, "Snap rounds, Stem and Shift do not: no shared rounding");
    let zone = b.command(&Control::Zone(Choice::None)).unwrap();
    assert_eq!(zone, ("hint.tt.setSnapZone", json!({"glyph": "I", "layer": "m01", "instructions": [0], "zone": "none"})));
    let stem = b.command(&Control::Stem(Choice::None)).unwrap();
    assert_eq!(stem, ("hint.tt.setStemReference", json!({"glyph": "I", "layer": "m01", "instructions": [1], "stem": "none"})));
    let rounding = b.command(&Control::Rounding(Rounding::Down)).unwrap();
    assert_eq!(rounding, ("hint.tt.setRounding", json!({"glyph": "I", "layer": "m01", "instructions": [0, 1, 2], "rounding": "down"})));
    assert_eq!(b.command(&Control::Delta(Click { column: 0, ppm: 12, value: Cell::Up })), None, "no Delta selected");
    run_and_compare(&mut app, &[zone, stem], &tt);
    let after = list(&app);
    assert_eq!(after.first().map(|t| t.kind.clone()), Some(TtKind::Snap { zone: ZoneRef::NoZone }));
    assert_eq!(after.get(1).map(|t| t.kind.clone()), Some(TtKind::Stem { stem: StemRef::NoStem, triple: None }));
}

#[test]
fn info_box_tt_delta_table_cycles_a_cell_of_the_selected_deltas() {
    let tt = [snap(0), delta(1)];
    let mut app = app(&tt);
    let view = selecting(&[1]);
    let w = Window::new();
    assert_eq!(w.click(&mut app, &view, delta_id()), Vec::new(), "opening the table changes nothing");
    let bold = delta_column_key(&[700.0]);
    let queued = w.click(&mut app, &view, cell_id(0, 12));
    assert_eq!(
        queued,
        vec![("hint.tt.setDeltas", json!({"glyph": "I", "layer": "m01", "instructions": [1], "cells": {bold.clone(): {"12": "up"}}}))]
    );
    run_and_compare(&mut app, &queued, &tt);
    let cell = |app: &FontcraftApp| match list(app).get(1).map(|t| t.kind.clone()) {
        Some(TtKind::Delta { cells }) => cells.get(&bold).and_then(|sizes| sizes.get(&12).copied()),
        _ => None,
    };
    assert_eq!(cell(&app), Some(DeltaDir::Up));
    let queued = w.click(&mut app, &view, cell_id(0, 12));
    assert_eq!(queued.first().and_then(|(_, p)| p.pointer(&format!("/cells/{bold}/12")).cloned()), Some(json!("down")));
    for (id, params) in &queued {
        app.run(id, params).unwrap();
    }
    assert_eq!(cell(&app), Some(DeltaDir::Down));
    w.click(&mut app, &view, tt_deltas::done_id());
    assert!(!w.drawn(&mut app, &view, cell_id(0, 12)), "Done closes the table");
}

#[test]
fn info_box_tt_pixel_size_intent_and_direction_queue_the_view_commands() {
    let mut app = app(&[]);
    let view = selecting(&[]);
    let w = Window::new();
    assert_eq!(w.click(&mut app, &view, larger_id()), vec![("hint.tt.increasePixelSize", json!({}))]);
    assert_eq!(w.click(&mut app, &view, smaller_id()), vec![("hint.tt.decreasePixelSize", json!({}))]);
    assert_eq!(w.click(&mut app, &view, direction_id()), vec![("hint.tt.toggleDirection", json!({"direction": "horizontal"}))]);
    assert_eq!(w.enter(&mut app, &view, ppm_id(), "16"), vec![("hint.tt.setPixelSize", json!({"ppm": 16}))]);
    assert_eq!(w.enter(&mut app, &view, ppm_id(), "500"), vec![("hint.tt.setPixelSize", json!({"ppm": 400}))], "the field keeps to 4 to 400");
    let b = TtBox::read(&app, &view).unwrap();
    assert_eq!(b.command(&Control::Intent(Intent::GdiClearType)), Some(("hint.tt.setRenderingIntent", json!({"intent": "cleartype"}))));
}
