//! The canvas overlays: what the switches and the engine's answers paint, Lock Outline, and typing a text note.

use std::sync::Arc;

use craft::behavior::CanvasView;
use craft::behavior::kurbo::Point as KPoint;
use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use egui::{LayerId, Painter, Pos2, Rect, vec2};
use fontcraft_engine::Session;
use fontcraft_engine::model::Annotation;
use fontcraft_geom::Point;
use fontcraft_tools::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind};
use serde_json::{Value, json};

use super::tokens::OverlayTokens;
use super::{Drawn, OverlayWords, Painted};
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::canvas::camera::Camera;
use crate::canvas::layers::PaintCx;
use crate::canvas::paint::CanvasWords;
use crate::canvas::tools::ToolInput;

fn run(app: &mut FontCraftApp, id: &str, params: Value) -> Value {
    let r = app.run_now(Msg::run(id, params), false);
    assert!(r.is_ok(), "{id}: {r:?}");
    r.unwrap()
}

fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
    app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
    app.drain();
    assert!(app.status.is_empty(), "{id}: {}", app.status);
}

/// Glyph A with a 200-unit square from (100, 100), shown at one point per unit.
fn edit_app() -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    run(&mut app, "path.addRectangle", json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200}));
    app.view.canvas = CanvasView::new(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0)), KPoint::new(400.0, 400.0), 1.0);
    app.drain();
    app
}

const RECT: Rect = Rect { min: Pos2::ZERO, max: Pos2 { x: 1000.0, y: 800.0 } };

fn drawn(app: &FontCraftApp, layer: fn(&PaintCx<'_>) -> Drawn) -> Drawn {
    let painter = Painter::new(egui::Context::default(), LayerId::background(), RECT);
    let mut view = app.view.canvas;
    view.rect = RECT;
    let top = app.models.canvas.glyph.as_ref().map_or(0.0, |g| g.ascender);
    let (words, overlay_words) = (CanvasWords::default(), OverlayWords::default());
    let cx = PaintCx {
        painter: &painter,
        rect: RECT,
        camera: Camera::new(view, top),
        model: &app.models.canvas,
        tokens: &app.models.canvas_tokens,
        display: app.view.display,
        edit_background: app.view.edit_background,
        words: &words,
        tools: &app.tools.view,
        text: &app.models.text,
        overlays: Painted { data: &app.overlays.data, words: &overlay_words },
        hinting: crate::canvas::hinting::HintPaint { model: &app.models.hint_canvas, view: &app.view.hinting, tokens: &app.models.hint_tokens },
    };
    layer(&cx)
}

fn texts(d: &Drawn) -> Vec<String> {
    d.labels.iter().map(|l| l.text.clone()).collect()
}

fn annotations(app: &FontCraftApp) -> Vec<Annotation> {
    let font = app.session.font().unwrap();
    font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap().info.annotations.clone()
}

fn pointer(app: &mut FontCraftApp, kind: PointerKind, x: f64, y: f64) {
    app.sent.push(Msg::Tool(ToolInput::Pointer(PointerEvent { kind, pos: Point::new(x, y), mods: Modifiers::NONE })));
    app.drain();
}

/// A local guide with its measurement on draws its line and knob and the distances between its crossings with the
/// outline; the guide's name shows at its left end.
#[test]
fn guides_draw_with_their_measurement_crossings() {
    let mut app = edit_app();
    run(&mut app, "guide.add", json!({"glyph": "A", "x": 0, "y": 200, "angle": 0}));
    run(&mut app, "guide.setProperties", json!({"glyph": "A", "guide": {"scope": "local", "index": 0}, "measurement": true, "name": "mid"}));
    let d = drawn(&app, super::guides::guide_shapes);
    let t = OverlayTokens::standard();
    let colored = |c: egui::Color32| d.shapes.iter().filter(|s| matches!(s, egui::Shape::LineSegment { stroke, .. } if stroke.color == c)).count();
    assert_eq!(colored(t.guide_local), 1, "one guide line");
    assert_eq!(app.overlays.data.guide_rulers.len(), 1);
    let labels = texts(&d);
    assert!(labels.contains(&"mid".to_string()), "{labels:?}");
    assert!(labels.contains(&"200".to_string()), "the square is 200 wide at the guide: {labels:?}");
    shell(&mut app, "view.showGuides", json!({"on": false}));
    assert!(drawn(&app, super::guides::guide_shapes).shapes.is_empty(), "Show Guides off hides them");
}

/// The display aids follow their switches: indexes label every node, curve length is off for straight segments,
/// the beam measures the sidebearings and Lock Outline shows its badge.
#[test]
fn display_aids_follow_their_switches() {
    let mut app = edit_app();
    assert!(texts(&drawn(&app, super::aids::aid_shapes)).is_empty());
    shell(&mut app, "view.showIndexes", json!({"points": true}));
    let labels = texts(&drawn(&app, super::aids::aid_shapes));
    assert_eq!(labels, vec!["0", "1", "2", "3"]);
    shell(&mut app, "view.showIndexes", json!({}));
    assert!(texts(&drawn(&app, super::aids::aid_shapes)).is_empty(), "with none given, all turn off");
    shell(&mut app, "view.showMetricsBeam", json!({"y": 150}));
    let labels = texts(&drawn(&app, super::aids::aid_shapes));
    let width = app.models.canvas.glyph.as_ref().unwrap().width;
    assert_eq!(labels, vec!["100".to_string(), super::num(width - 300.0)], "the sidebearings on the beam");
    shell(&mut app, "view.lockOutline", json!({}));
    assert!(texts(&drawn(&app, super::aids::aid_shapes)).contains(&"Outline locked".to_string()));
}

/// Lock Outline drops the tools' outline edits and says why; guide edits and commands run by name still work.
#[test]
fn lock_outline_refuses_pointer_and_key_edits() {
    let mut app = edit_app();
    shell(&mut app, "view.lockOutline", json!({"on": true}));
    shell(&mut app, "selection.set", json!({"points": [[0, 2]]}));
    app.sent.push(Msg::Tool(ToolInput::Key(KeyEvent::press(Key::Right, Modifiers::NONE))));
    app.drain();
    assert_eq!(app.status, super::LOCKED);
    assert!(super::edits_outline("selection.move", &json!({"points": [[0, 1]]}), 0));
    assert!(!super::edits_outline("selection.move", &json!({"anchors": ["top"]}), 0));
    let batch = json!({"steps": [{"command": "guide.add", "params": {}}]});
    assert!(!super::edits_outline("command.batch", &batch, 0));
    app.status.clear();
    run(&mut app, "path.addRectangle", json!({"glyph": "A", "x": 400, "y": 100, "width": 50, "height": 50}));
}

/// An arrow move of a component that cannot move reaches the status line with the engine's reason.
#[test]
fn a_refused_component_move_says_why_in_the_status_line() {
    let mut app = edit_app();
    run(&mut app, "glyph.newGlyph", json!({"name": "Acomp"}));
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("Acomp".into())));
    app.drain();
    run(&mut app, "glyph.addComponent", json!({"glyph": "Acomp", "component": "A"}));
    // Locks it unless it is aligned, which holds it as well.
    app.sent.push(Msg::run("component.lock", json!({"glyph": "Acomp"})));
    app.drain();
    app.status.clear();
    shell(&mut app, "selection.set", json!({"shapes": [0]}));
    app.sent.push(Msg::Tool(ToolInput::Key(KeyEvent::press(Key::Right, Modifiers::NONE))));
    app.drain();
    assert!(app.status.starts_with("selection.move"), "{}", app.status);
}

/// The Text Annotation tool adds a note and opens its field; committing typed text is one undo step, Escape on a new
/// note leaves the font as it was, and a double-click edits an existing note's text.
#[test]
fn text_notes_are_typed_on_the_canvas() {
    let mut app = edit_app();
    shell(&mut app, "annotation.setTool", json!({"tool": "text"}));
    let undo = |app: &FontCraftApp| app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len);
    let before = undo(&app);
    pointer(&mut app, PointerKind::Down, 500.0, 500.0);
    pointer(&mut app, PointerKind::Up, 500.0, 500.0);
    assert_eq!(annotations(&app).len(), 1);
    let note = app.overlays.note.as_mut().expect("the field opens on the new note");
    assert!(note.new);
    note.text = "Kern this\nand that".into();
    note.done = Some(true);
    app.drain();
    let list = annotations(&app);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].text.as_deref(), Some("Kern this\nand that"));
    assert_eq!(undo(&app), before + 1, "adding and typing are one undo step");
    assert!(app.overlays.note.is_none());

    pointer(&mut app, PointerKind::Down, 900.0, 300.0);
    pointer(&mut app, PointerKind::Up, 900.0, 300.0);
    assert_eq!(annotations(&app).len(), 2);
    let note = app.overlays.note.as_mut().unwrap();
    note.text = "dropped".into();
    note.done = Some(false);
    app.drain();
    assert_eq!(annotations(&app).len(), 1, "Escape removes the new note");

    pointer(&mut app, PointerKind::Down, 900.0, 300.0);
    pointer(&mut app, PointerKind::Up, 900.0, 300.0);
    app.overlays.note.as_mut().unwrap().done = Some(true);
    app.drain();
    assert_eq!(annotations(&app).len(), 1, "a new note left empty is removed");

    pointer(&mut app, PointerKind::DoubleClick, 505.0, 495.0);
    let note = app.overlays.note.as_mut().expect("a double-click opens the note's field");
    assert!(!note.new);
    assert_eq!(note.index, 0);
    note.text = "Kern".into();
    note.done = Some(true);
    app.drain();
    assert_eq!(annotations(&app)[0].text.as_deref(), Some("Kern"));
}

/// Stems, the quick measurement and audit switches reach the layers.
#[test]
fn autometer_and_audit_switches_reach_the_layers() {
    let mut app = edit_app();
    shell(&mut app, "view.showQuickMeasurement", json!({"on": true}));
    pointer(&mut app, PointerKind::Move, 200.0, 150.0);
    let quick = app.overlays.data.quick.expect("a quick measurement under the pointer");
    assert!((quick.2 - 200.0).abs() < 1e-6, "{quick:?}");
    assert!(texts(&drawn(&app, super::autometer::meter_shapes)).contains(&"200".to_string()));
    shell(&mut app, "audit.toggleLiveAudit", json!({"on": true}));
    assert!(app.overlays.audit_runs >= 1);
}
