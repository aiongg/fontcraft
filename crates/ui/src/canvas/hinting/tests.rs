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
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    app
}

fn run(app: &mut FontCraftApp, id: &str, params: Value) {
    app.sent.push(Msg::run(id, params));
    app.drain();
    assert!(app.status.is_empty(), "{id}: {}", app.status);
}

/// Runs a PostScript hint command whose hint edges miss every node: the reply's warning reaches the status line, and
/// the hint is still added.
fn run_off_nodes(app: &mut FontCraftApp, id: &str, params: Value) {
    app.sent.push(Msg::run(id, params));
    app.drain();
    assert!(app.status.contains("no node lies exactly"), "{id}: {}", app.status);
    app.status.clear();
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
        run_off_nodes(&mut app, "hint.ps.addHorizontalHint", json!({"glyph": "A", "origin": y, "size": 20}));
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

fn open_menu_at(app: &mut FontCraftApp, at: Pos2) -> crate::app::context_menu::CanvasMenu {
    use crate::app::context_menu::{Space, open_canvas};
    open_canvas(app, fontcraft_geom::Point::new(f64::from(at.x), f64::from(at.y)), Space::Screen).unwrap();
    app.context_menus.canvas.clone().expect("the canvas menu opened")
}

fn pick(app: &mut FontCraftApp, id: &str) {
    crate::app::context_menu::invoke(app, id, |app, msg| app.run_now(msg, false)).unwrap_or_else(|e| panic!("{id}: {e}"));
}

fn ps_badge(app: &FontCraftApp, index: usize) -> Pos2 {
    let marks = app.models.hint_canvas.ps.clone();
    let view = crate::canvas::tools::apply::tools_view(&camera(app));
    let b = marks.badge(&view, &marks.hints[index]).expect("the badge").center();
    Pos2::new(b.x as f32, b.y as f32)
}

/// A right-click on a PostScript hint's badge selects that hint and opens the hint section, whose Make Ghost Hint and
/// Delete Hints act on it as the commands do.
#[test]
fn a_ps_hint_badge_menu_makes_ghosts_and_deletes() {
    use crate::app::context_menu::{CANVAS, HINT};
    let mut app = new_app();
    let mut headless = new_app();
    for a in [&mut app, &mut headless] {
        squares(a);
        run(a, "hint.ps.addHorizontalHint", json!({"glyph": "A", "origin": 100, "size": 100}));
        run_off_nodes(a, "hint.ps.addHorizontalHint", json!({"glyph": "A", "origin": 300, "size": 40}));
    }
    shell(&mut app, "view.showHints", json!({"on": true}));

    let at = ps_badge(&app, 1);
    let menu = open_menu_at(&mut app, at);
    assert_eq!(menu.sections.iter().map(|s| s.context).collect::<Vec<_>>(), vec![HINT, CANVAS]);
    let hint_items: Vec<&str> = menu.sections[0].items.iter().map(|i| i.command).collect();
    assert_eq!(hint_items, vec!["hint.ps.makeGhostHint", "hint.ps.deleteHints"]);
    assert!(menu.sections[0].items.iter().all(|i| i.enabled), "{menu:?}");
    assert_eq!(app.tools.selection.hints.iter().copied().collect::<Vec<_>>(), vec![1], "the clicked hint is selected first");
    pick(&mut app, "hint.ps.makeGhostHint");
    run(&mut headless, "hint.ps.makeGhostHint", json!({"glyph": "A", "hints": [1]}));
    assert_eq!(layer(&app), layer(&headless));

    let at = ps_badge(&app, 0);
    open_menu_at(&mut app, at);
    assert_eq!(app.tools.selection.hints.iter().copied().collect::<Vec<_>>(), vec![0]);
    pick(&mut app, "hint.ps.deleteHints");
    run(&mut headless, "hint.ps.deleteHints", json!({"glyph": "A", "hints": [0]}));
    assert_eq!(layer(&app), layer(&headless));
}

/// With the Instructor active, a right-click on an instruction's badge selects it unless it is selected and opens the
/// instruction section: Reverse wants Stem instructions, Make Triple Hint three vertical ones, and Delete
/// Instructions clears the instruction selection. Each acts as its command does.
#[test]
fn an_instruction_badge_menu_reverses_links_and_deletes() {
    use crate::app::context_menu::{CANVAS, TT_INSTRUCTION, TT_INSTRUCTOR};
    let mut app = new_app();
    let mut headless = new_app();
    for a in [&mut app, &mut headless] {
        squares(a);
        run(a, "hint.tt.snapPoint", json!({"glyph": "A", "nodes": [[0, 0]], "direction": "vertical"}));
        run(a, "hint.tt.addStemHint", json!({"glyph": "A", "nodes": [[0, 1], [0, 2], [1, 1], [1, 2], [2, 1], [2, 2]], "direction": "vertical"}));
    }
    instructor(&mut app);
    let kinds: Vec<BadgeKind> = app.models.hint_canvas.tt.as_ref().unwrap().instructions.iter().map(|m| m.kind).collect();
    assert_eq!(kinds, vec![BadgeKind::Snap, BadgeKind::Stem, BadgeKind::Stem, BadgeKind::Stem]);

    // On the Snap badge: only Delete Instructions applies.
    let at = badge_at(&app, 0);
    let menu = open_menu_at(&mut app, at);
    assert_eq!(menu.sections.iter().map(|s| s.context).collect::<Vec<_>>(), vec![TT_INSTRUCTION, CANVAS, TT_INSTRUCTOR]);
    let items: Vec<&str> = menu.sections[0].items.iter().map(|i| i.command).collect();
    assert_eq!(items, vec!["hint.tt.reverse", "hint.tt.makeTripleHint", "hint.tt.deleteInstructions"]);
    assert_eq!(selected(&app), vec![0], "the clicked instruction is selected first");
    let enabled = |menu: &crate::app::context_menu::CanvasMenu, id: &str| menu.item(id).unwrap().enabled;
    assert!(!enabled(&menu, "hint.tt.reverse") && !enabled(&menu, "hint.tt.makeTripleHint"));
    assert!(enabled(&menu, "hint.tt.deleteInstructions"));
    assert!(crate::app::context_menu::invoke(&mut app, "hint.tt.reverse", |app, msg| app.run_now(msg, false)).is_err(), "a greyed item refuses");

    // On a selected Stem badge the three Stems stay selected: Make Triple Hint, then Reverse.
    shell(&mut app, "hint.tt.selectInstructions", json!({"instructions": [1, 2, 3]}));
    let at = badge_at(&app, 2);
    let menu = open_menu_at(&mut app, at);
    assert_eq!(selected(&app), vec![1, 2, 3]);
    assert!(enabled(&menu, "hint.tt.reverse") && enabled(&menu, "hint.tt.makeTripleHint"));
    pick(&mut app, "hint.tt.makeTripleHint");
    run(&mut headless, "hint.tt.makeTripleHint", json!({"glyph": "A", "instructions": [1, 2, 3]}));
    assert_eq!(layer(&app), layer(&headless));
    shell(&mut app, "hint.tt.selectInstructions", json!({"instructions": [1, 2, 3]}));
    let at = badge_at(&app, 1);
    let menu = open_menu_at(&mut app, at);
    assert_eq!(selected(&app), vec![1, 2, 3]);
    assert!(!enabled(&menu, "hint.tt.makeTripleHint"), "they belong to a triple hint already");
    pick(&mut app, "hint.tt.reverse");
    run(&mut headless, "hint.tt.reverse", json!({"glyph": "A", "instructions": [1, 2, 3]}));
    assert_eq!(layer(&app), layer(&headless));

    // Delete Instructions on the Snap badge deletes it alone and clears the selection.
    let at = badge_at(&app, 0);
    open_menu_at(&mut app, at);
    assert_eq!(selected(&app), vec![0]);
    pick(&mut app, "hint.tt.deleteInstructions");
    run(&mut headless, "hint.tt.deleteInstructions", json!({"glyph": "A", "instructions": [0]}));
    assert_eq!(layer(&app), layer(&headless));
    assert!(selected(&app).is_empty());
}
