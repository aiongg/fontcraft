//! Hinting on the canvas: the Instructor's badges as the pointer and Delete use them, its view state, PostScript hints
//! under the Select tool, menu targets and the Info box variants.

use std::sync::Arc;

use craft::behavior::CanvasView;
use craft::behavior::kurbo::Point as KPoint;
use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use egui::{Pos2, Rect, vec2};
use fontcraft_engine::Session;
use fontcraft_engine::model::Layer;
use fontcraft_tools::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind};
use serde_json::{Value, json};

use super::badges::badges;
use super::tokens::HintTokens;
use super::*;
use crate::app::models::hint_canvas::BadgeKind;
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::canvas::tools::ToolInput;
use crate::canvas::tools::apply::camera;

fn new_app() -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    app
}

fn run(app: &mut FontCraftApp, id: &str, params: Value) {
    app.sent.push(Msg::run(id, params));
    app.drain();
    assert!(app.status.is_empty(), "{id}: {}", app.status);
}

fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
    app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
    app.drain();
    assert!(app.status.is_empty(), "{id}: {}", app.status);
}

/// Glyph A with three squares (paths 0, 1, 2), at one point per unit.
fn squares(app: &mut FontCraftApp) {
    for x in [100, 250, 400] {
        run(app, "path.addRectangle", json!({"glyph": "A", "x": x, "y": 100, "width": 100, "height": 100}));
    }
    app.view.canvas = CanvasView::new(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0)), KPoint::new(300.0, 400.0), 1.0);
}

fn layer(app: &FontCraftApp) -> Layer {
    let font = app.session.font().unwrap();
    font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap().clone()
}

fn pointer(app: &mut FontCraftApp, kind: PointerKind, at: Pos2, mods: Modifiers) {
    let pos = camera(app).to_font(at);
    app.sent.push(Msg::Tool(ToolInput::Pointer(PointerEvent { kind, pos, mods })));
    app.drain();
}

fn click(app: &mut FontCraftApp, at: Pos2, mods: Modifiers) {
    pointer(app, PointerKind::Down, at, mods);
    pointer(app, PointerKind::Up, at, mods);
}

fn badge_at(app: &FontCraftApp, index: usize) -> Pos2 {
    let tt = app.models.hint_canvas.tt.as_ref().expect("the Instructor's picture");
    let marks = badges(&tt.instructions, &camera(app), &HintTokens::standard(), app.view.hinting.instructor.direction);
    marks.iter().find(|b| b.index == index).expect("the badge").at
}

fn selected(app: &FontCraftApp) -> Vec<usize> {
    let l = layer(app);
    app.view.hinting.instructor.selected_in("A", &l.id).into_iter().collect()
}

fn instructor(app: &mut FontCraftApp) {
    app.sent.push(Msg::View(ViewMsg::SelectTool(INSTRUCTOR)));
    app.drain();
}

#[test]
fn instructor_badges_select_move_and_delete_their_instructions() {
    let mut app = new_app();
    squares(&mut app);
    let mut headless = new_app();
    squares(&mut headless);
    for a in [&mut app, &mut headless] {
        run(a, "hint.tt.snapPoint", json!({"glyph": "A", "nodes": [[0, 0], [0, 1]], "direction": "vertical"}));
        run(a, "hint.tt.addStemHint", json!({"glyph": "A", "nodes": [[1, 0], [1, 3]], "direction": "horizontal"}));
    }
    instructor(&mut app);
    let tt = app.models.hint_canvas.tt.clone().expect("the Instructor's picture");
    assert_eq!(tt.instructions.iter().map(|m| m.kind).collect::<Vec<_>>(), vec![BadgeKind::Snap, BadgeKind::Snap, BadgeKind::Stem]);
    assert_eq!(tt.nodes.len(), 12, "every on-curve node is a disc");
    assert!(tt.preview.is_some() || tt.preview_error.is_some(), "the pixel preview was asked for");

    // A click selects, Shift+click toggles; the other direction cannot be clicked.
    let points = app.tools.selection.clone();
    let first = badge_at(&app, 0);
    click(&mut app, first, Modifiers::NONE);
    assert_eq!(selected(&app), vec![0]);
    let second = badge_at(&app, 1);
    click(&mut app, second, Modifiers::SHIFT);
    assert_eq!(selected(&app), vec![0, 1]);
    assert_eq!(app.tools.selection, points, "the node selection stays");
    shell(&mut app, "hint.tt.toggleDirection", json!({}));
    assert!(selected(&app).is_empty(), "switching the direction clears the selection");
    shell(&mut app, "hint.tt.toggleDirection", json!({"direction": "vertical"}));

    // A drag dropped on node 0 of path 2 moves Snap 0 there; a drop on nothing changes nothing.
    let from = badge_at(&app, 0);
    let to = camera(&app).to_screen(KPoint::new(400.0, 100.0));
    pointer(&mut app, PointerKind::Down, from, Modifiers::NONE);
    pointer(&mut app, PointerKind::Drag, from.lerp(to, 0.5), Modifiers::NONE);
    pointer(&mut app, PointerKind::Drag, to, Modifiers::NONE);
    pointer(&mut app, PointerKind::Up, to, Modifiers::NONE);
    assert!(app.status.is_empty(), "{}", app.status);
    run(&mut headless, "hint.tt.moveInstruction", json!({"glyph": "A", "instruction": 0, "point": [2, 0]}));
    assert_eq!(layer(&app), layer(&headless));
    let from = badge_at(&app, 1);
    let empty = Pos2::new(from.x - 150.0, from.y);
    pointer(&mut app, PointerKind::Down, from, Modifiers::NONE);
    pointer(&mut app, PointerKind::Drag, empty, Modifiers::NONE);
    pointer(&mut app, PointerKind::Up, empty, Modifiers::NONE);
    assert_eq!(layer(&app), layer(&headless));

    // Delete deletes the selected instructions and clears the selection.
    shell(&mut app, "hint.tt.selectInstructions", json!({"instructions": [1]}));
    app.sent.push(Msg::Tool(ToolInput::Key(KeyEvent::press(Key::Delete, Modifiers::NONE))));
    app.drain();
    run(&mut headless, "hint.tt.deleteInstructions", json!({"glyph": "A", "instructions": [1]}));
    assert_eq!(layer(&app), layer(&headless));
    assert!(selected(&app).is_empty());
}

#[test]
fn instructor_view_state_keeps_its_limits() {
    let mut app = new_app();
    squares(&mut app);
    instructor(&mut app);
    app.view.hinting.instructor.ppm = MAX_PPM;
    shell(&mut app, "hint.tt.increasePixelSize", json!({}));
    assert_eq!(app.view.hinting.instructor.ppm, MAX_PPM);
    shell(&mut app, "hint.tt.setPixelSize", json!({"ppm": 4}));
    shell(&mut app, "hint.tt.decreasePixelSize", json!({}));
    assert_eq!(app.view.hinting.instructor.ppm, MIN_PPM);
    app.sent.push(Msg::Shell(CommandRef::with_params("hint.tt.setPixelSize", json!({"ppm": 4.5}))));
    app.drain();
    assert!(!app.status.is_empty(), "a fractional size is refused");
    app.status.clear();
    shell(&mut app, "hint.tt.setRenderingIntent", json!({"intent": "cleartype"}));
    assert_eq!(app.view.hinting.instructor.intent, RenderingIntent::ClearType);
    shell(&mut app, "hint.tt.showPointIndexes", json!({}));
    assert_eq!(app.models.hint_canvas.tt.as_ref().map(|t| t.numbers.len()), Some(12), "one number per on-curve node");
    let tt = app.models.palette.info_box.tt.clone().expect("the Instructor's Info box");
    assert_eq!((tt.ppm, tt.intent), (MIN_PPM, RenderingIntent::ClearType));
}

#[test]
fn ps_hints_show_select_step_and_fill_menu_targets() {
    let mut app = new_app();
    squares(&mut app);
    run(&mut app, "hint.ps.addHorizontalHint", json!({"glyph": "A", "origin": 100, "size": 100}));
    run(&mut app, "hint.ps.addVerticalHint", json!({"glyph": "A", "origin": 250, "size": 100}));
    assert!(!app.models.hint_canvas.ps.shown, "hidden until View › Show Hints");
    shell(&mut app, "view.showHints", json!({"on": true}));
    let marks = app.models.hint_canvas.ps.clone();
    assert_eq!(marks.hints.len(), 2);

    // The Select tool selects a hint by its badge, as drawn.
    let view = crate::canvas::tools::apply::tools_view(&camera(&app));
    let badge = marks.badge(&view, &marks.hints[0]).expect("the badge").center();
    click(&mut app, Pos2::new(badge.x as f32, badge.y as f32), Modifiers::NONE);
    assert_eq!(app.tools.selection.hints.iter().copied().collect::<Vec<_>>(), vec![0]);
    let info = app.models.palette.info_box.ps_hint.clone().expect("the hint's Info box");
    assert_eq!((info.index, info.count), (0, 1));

    shell(&mut app, "hint.ps.selectNextHint", json!({}));
    assert_eq!(app.tools.selection.hints.iter().copied().collect::<Vec<_>>(), vec![1]);
    let targeted = app.with_targets(CommandRef::new("hint.ps.deleteHints"));
    let l = layer(&app);
    assert_eq!(targeted.params, json!({"glyph": "A", "layer": l.id, "hints": [1]}));

    // Hidden hints cannot be acted on.
    shell(&mut app, "view.showHints", json!({}));
    assert!(app.tools.selection.hints.is_empty());
}

#[test]
fn review_hints_opens_on_the_first_hint_and_steps() {
    let mut app = new_app();
    squares(&mut app);
    for y in [0, 340] {
        run(&mut app, "hint.ps.addHorizontalHint", json!({"glyph": "A", "origin": y, "size": 20}));
    }
    shell(&mut app, "hint.ps.reviewHints", json!({}));
    let r = app.view.hinting.review.clone();
    assert!(r.open && r.horizontal && app.view.display.hints);
    assert_eq!(r.hint, Some(0));
    shell(&mut app, "hint.ps.reviewHints", json!({"step": "next"}));
    assert_eq!(app.view.hinting.review.hint, Some(1));
    app.sent.push(Msg::Shell(CommandRef::with_params("hint.ps.reviewHints", json!({"step": "next"}))));
    app.drain();
    assert!(app.status.contains("last"), "{}", app.status);
}
