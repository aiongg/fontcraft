//! The edit view's text: the text commands, the Text tool's keys and clicks, the text layers and the kerning
//! panels' requests.

use std::sync::Arc;

use craft::behavior::CanvasView;
use craft::behavior::kurbo::Point as KPoint;
use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use egui::epaint::ColorMode;
use egui::{Color32, LayerId, Painter, Pos2, Rect, Shape};
use fontcraft_engine::Session;
use fontcraft_spacing::kerning::GroupSide;
use fontcraft_tools::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind};
use serde_json::{Value, json};

use super::paint::{behind_shapes, cursor_shapes, glyph_shapes};
use super::tokens::TextTokens;
use crate::app::models::text::members_shown;
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::canvas::camera::Camera;
use crate::canvas::layers::PaintCx;
use crate::canvas::paint::CanvasWords;
use crate::canvas::tools::ToolInput;

const RECT: Rect = Rect { min: Pos2::ZERO, max: Pos2 { x: 1280.0, y: 800.0 } };

/// Glyph A shown, the canvas at 0.25 points per unit.
fn text_app() -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    app.view.canvas = CanvasView::new(RECT, KPoint::new(300.0, 400.0), 0.25);
    app
}

fn try_post(app: &mut FontCraftApp, msg: Msg) -> Result<(), String> {
    app.status.clear();
    app.sent.push(msg);
    app.drain();
    if app.status.is_empty() { Ok(()) } else { Err(app.status.clone()) }
}

fn try_shell(app: &mut FontCraftApp, id: &str, params: Value) -> Result<(), String> {
    try_post(app, Msg::Shell(CommandRef::with_params(id, params)))
}

fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
    try_shell(app, id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
}

fn run(app: &mut FontCraftApp, id: &str, params: Value) {
    try_post(app, Msg::run(id, params)).unwrap_or_else(|e| panic!("{id}: {e}"));
}

fn key(app: &mut FontCraftApp, key: Key, mods: Modifiers) {
    try_post(app, Msg::Tool(ToolInput::Key(KeyEvent::press(key, mods)))).unwrap();
}

fn get(app: &FontCraftApp) -> Value {
    super::reply(app)
}

fn items(app: &FontCraftApp) -> Value {
    get(app)["items"].clone()
}

const SHIFT: Modifiers = Modifiers { shift: true, alt: false, cmd: false, ctrl: false };

#[test]
fn text_commands_edit_the_text_and_the_shown_glyph_follows_the_cursor() {
    let mut app = text_app();
    shell(&mut app, "editView.setText", json!({"text": "AV/a"}));
    let t = get(&app);
    assert_eq!((t["items"].clone(), t["cursor"].clone(), t["current"].clone()), (json!(["A", "V", "a"]), json!(3), json!("a")));
    assert_eq!(app.view.glyph.as_deref(), Some("a"), "the glyph before the cursor at the end of the text is edited");

    shell(&mut app, "editView.insertText", json!({"glyphs": ["B"], "at": 1}));
    assert_eq!((items(&app), get(&app)["cursor"].clone(), app.view.glyph.clone()), (json!(["A", "B", "V", "a"]), json!(2), Some("V".into())));
    shell(&mut app, "editView.insertText", json!({"chars": "xy", "replace": [0, 2]}));
    assert_eq!(items(&app), json!(["x", "y", "V", "a"]));
    shell(&mut app, "editView.setCursor", json!({"index": 1, "selectionLength": 2}));
    assert_eq!((get(&app)["selection"].clone(), get(&app)["current"].clone()), (json!([1, 3]), json!("y")));
    shell(&mut app, "editView.setCursor", json!({"index": 3, "anchor": 1}));
    assert_eq!((get(&app)["selection"].clone(), get(&app)["cursor"].clone()), (json!([1, 3]), json!(3)));

    // A command that fails changes nothing.
    let before = get(&app);
    for (id, params) in [
        ("editView.setCursor", json!({"index": 9})),
        ("editView.setCursor", json!({"index": 1, "selectionLength": 9})),
        ("editView.insertText", json!({"chars": "a", "at": 1, "replace": [0, 1]})),
        ("editView.insertText", json!({"glyphs": ["nosuch"]})),
        ("editView.setText", json!({"text": "A", "chars": "A"})),
        ("editView.setWritingDirection", json!({"direction": "up"})),
        ("kerning.setMode", json!({"mode": "half"})),
    ] {
        assert!(try_shell(&mut app, id, params.clone()).is_err(), "{id} {params}");
        assert_eq!(get(&app), before, "{id} {params} changed the text");
    }

    shell(&mut app, "edit.addPlaceholder", json!({"at": 0}));
    assert_eq!(items(&app), json!([{"placeholder": true}, "x", "y", "V", "a"]));
    shell(&mut app, "editView.setCursor", json!({"index": 1}));
    shell(&mut app, "editView.nextGlyph", json!({}));
    assert_eq!((items(&app), get(&app)["current"].clone()), (json!([{"placeholder": true}, "y", "y", "V", "a"]), json!("y")));
    shell(&mut app, "editView.previousGlyph", json!({}));
    assert_eq!(items(&app)[1], json!("x"));

    shell(&mut app, "editView.setWritingDirection", json!({"direction": "rtl"}));
    assert_eq!(get(&app)["direction"], "rtl");
    let modes: Vec<Value> = (0..3)
        .map(|_| {
            shell(&mut app, "kerning.setMode", json!({}));
            get(&app)["kerning"].clone()
        })
        .collect();
    assert_eq!(modes, [json!("locked"), json!("none"), json!("kerning")], "omitted: the next of the three");

    // Another glyph picked elsewhere: the text stays, and the canvas shows that glyph alone.
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("Q".into())));
    app.drain();
    assert_eq!((get(&app)["shown"].clone(), app.models.text.shown), (json!(false), false));
    assert_eq!(items(&app)[1], json!("x"));
}

#[test]
fn the_text_tool_types_deletes_moves_the_cursor_and_escape_leaves_text_mode() {
    let mut app = text_app();
    try_post(&mut app, Msg::View(ViewMsg::SelectTool(super::TEXT_TOOL))).unwrap();
    let t = get(&app);
    assert_eq!((t["textMode"].clone(), t["items"].clone(), t["cursor"].clone()), (json!(true), json!(["A"]), json!(0)));
    assert!(app.active_tool.claims().keyboard, "the Text tool takes the plain keys, so letters type");

    key(&mut app, Key::Char('h'), SHIFT);
    key(&mut app, Key::Char('o'), Modifiers::NONE);
    key(&mut app, Key::Space, Modifiers::NONE);
    assert_eq!((items(&app), get(&app)["cursor"].clone()), (json!(["H", "o", "space", "A"]), json!(3)));
    key(&mut app, Key::Backspace, Modifiers::NONE);
    key(&mut app, Key::Right, SHIFT);
    assert_eq!(get(&app)["selection"], json!([2, 3]));
    key(&mut app, Key::Delete, Modifiers::NONE);
    assert_eq!(items(&app), json!(["H", "o"]));
    key(&mut app, Key::Left, Modifiers::NONE);
    key(&mut app, Key::Left, Modifiers::NONE);
    assert_eq!((get(&app)["cursor"].clone(), app.view.glyph.as_deref()), (json!(0), Some("H")));
    key(&mut app, Key::End, Modifiers::NONE);
    assert_eq!(items(&app), json!(["I", "o"]), "End: the next glyph in the font's order in place of the current one");
    key(&mut app, Key::Char('a'), Modifiers { cmd: true, ..Modifiers::NONE });
    assert_eq!(get(&app)["selection"], json!([0, 2]), "Cmd+A selects the whole text");

    key(&mut app, Key::Escape, Modifiers::NONE);
    assert_eq!((app.view.tool, get(&app)["textMode"].clone()), ("select", json!(false)));
    assert!(!app.active_tool.claims().keyboard, "the tool bridge is back in the slot");
    shell(&mut app, "editView.toggleTextMode", json!({}));
    assert_eq!(app.view.tool, super::TEXT_TOOL);
}

/// A tool event at the text point (x, y), as the canvas posts it: in the edited glyph's units.
fn pointer(app: &mut FontCraftApp, kind: PointerKind, x: f64, y: f64, mods: Modifiers) {
    let [ox, oy] = app.models.text.origin;
    try_post(app, Msg::Tool(ToolInput::Pointer(PointerEvent::new(kind, x - ox, y - oy, mods)))).unwrap();
}

#[test]
fn clicks_place_the_cursor_drags_select_and_a_double_click_edits() {
    let mut app = text_app();
    shell(&mut app, "editView.setText", json!({"glyphs": ["A", "V", "A"]}));
    shell(&mut app, "editView.toggleTextMode", json!({"mode": "text"}));
    let center = app.view.canvas.center;
    shell(&mut app, "editView.setCursor", json!({"index": 0}));
    let moved = app.view.canvas.center;
    assert_eq!((moved.x - center.x, moved.y - center.y), (1200.0, 0.0), "the view follows the edited glyph, so the text stays put");

    pointer(&mut app, PointerKind::Down, 650.0, 100.0, Modifiers::NONE);
    pointer(&mut app, PointerKind::Up, 650.0, 100.0, Modifiers::NONE);
    assert_eq!((get(&app)["cursor"].clone(), get(&app)["selection"].clone()), (json!(1), json!([1, 1])));
    pointer(&mut app, PointerKind::Down, 100.0, 100.0, Modifiers::NONE);
    pointer(&mut app, PointerKind::Drag, 1500.0, 100.0, Modifiers::NONE);
    pointer(&mut app, PointerKind::Up, 1500.0, 100.0, Modifiers::NONE);
    assert_eq!((get(&app)["cursor"].clone(), get(&app)["selection"].clone()), (json!(3), json!([0, 3])));
    pointer(&mut app, PointerKind::Down, 650.0, 100.0, SHIFT);
    pointer(&mut app, PointerKind::Up, 650.0, 100.0, SHIFT);
    assert_eq!(get(&app)["selection"], json!([0, 1]), "Shift keeps the far end of the selection");

    pointer(&mut app, PointerKind::DoubleClick, 650.0, 100.0, Modifiers::NONE);
    assert_eq!((app.view.tool, app.view.glyph.as_deref()), ("select", Some("V")), "a double-click edits the glyph");
    assert!(app.models.text.shown && !app.models.text.text_mode);
}

/// The shapes `layer` paints for `app`'s canvas.
fn painted(app: &FontCraftApp, layer: fn(&PaintCx<'_>) -> Vec<Shape>) -> Vec<Shape> {
    let painter = Painter::new(egui::Context::default(), LayerId::background(), RECT);
    let top = app.models.canvas.glyph.as_ref().map_or(0.0, |g| g.ascender);
    let words = CanvasWords::default();
    let overlay_words = crate::canvas::overlays::OverlayWords::default();
    let cx = PaintCx {
        painter: &painter,
        rect: RECT,
        camera: Camera::new(app.view.canvas, top),
        model: &app.models.canvas,
        tokens: &app.models.canvas_tokens,
        display: super::display(&app.view),
        edit_background: app.view.edit_background,
        words: &words,
        tools: &app.tools.view,
        text: &app.models.text,
        overlays: crate::canvas::overlays::Painted { data: &app.overlays.data, words: &overlay_words },
        hinting: crate::canvas::hinting::HintPaint { model: &app.models.hint_canvas, view: &app.view.hinting, tokens: &app.models.hint_tokens },
        compatibility: &app.models.compatibility,
    };
    layer(&cx)
}

fn count(shapes: &[Shape], color: Color32) -> usize {
    shapes
        .iter()
        .filter(|s| match s {
            Shape::Path(p) => p.stroke.color == ColorMode::Solid(color),
            Shape::Rect(r) => r.fill == color,
            Shape::LineSegment { stroke, .. } => stroke.color == color,
            _ => false,
        })
        .count()
}

#[test]
fn the_text_layers_draw_the_other_glyphs_the_kerned_gaps_and_the_cursor() {
    let mut app = text_app();
    run(&mut app, "path.addRectangle", json!({"glyph": "V", "x": 100, "y": 0, "width": 400, "height": 500}));
    run(&mut app, "kerning.setPair", json!({"left": "A", "right": "V", "value": -100}));
    shell(&mut app, "editView.setText", json!({"glyphs": ["A", "V", "A"]}));
    shell(&mut app, "editView.setCursor", json!({"index": 0}));
    let t = TextTokens::standard();
    assert_eq!(count(&painted(&app, glyph_shapes), t.glyph), 1, "V, the one other glyph with an outline");
    assert_eq!(count(&painted(&app, behind_shapes), t.kerning_negative), 1, "the A V gap");
    assert!(painted(&app, cursor_shapes).is_empty(), "no cursor outside text mode");

    shell(&mut app, "editView.toggleTextMode", json!({"mode": "text"}));
    assert_eq!(count(&painted(&app, cursor_shapes), t.cursor), 1);
    assert!(!super::display(&app.view).nodes, "text mode hides the nodes");
    shell(&mut app, "kerning.setMode", json!({"mode": "none"}));
    assert_eq!(count(&painted(&app, behind_shapes), t.kerning_negative), 0, "no kerning shown without kerning");
    shell(&mut app, "editView.setCursor", json!({"index": 0, "selectionLength": 2}));
    assert_eq!((count(&painted(&app, behind_shapes), t.selection), painted(&app, cursor_shapes).len()), (2, 0));
}

/// T and F in the right group T; o, c and e in the left group o.
fn grouped_app() -> FontCraftApp {
    let mut app = text_app();
    run(&mut app, "kerning.setRightGroup", json!({"glyphs": ["T", "F"], "group": "T"}));
    run(&mut app, "kerning.setLeftGroup", json!({"glyphs": ["o", "c", "e"], "group": "o"}));
    app
}

#[test]
fn show_pair_in_edit_view_replaces_the_pair_at_the_current_glyph() {
    let mut app = grouped_app();
    shell(&mut app, "kerning.showPairInEditView", json!({"left": "@T", "right": "@o"}));
    assert_eq!((items(&app), get(&app)["cursor"].clone()), (json!(["F", "c", "A"]), json!(1)), "groups show their first member");
    shell(&mut app, "kerning.showPairInEditView", json!({"left": "T", "right": "o"}));
    assert_eq!((items(&app), app.view.glyph.as_deref()), (json!(["T", "o", "A"]), Some("o")));
    assert!(try_shell(&mut app, "kerning.showPairInEditView", json!({"left": "@none", "right": "o"})).is_err());
    assert_eq!(items(&app), json!(["T", "o", "A"]));

    // The Kerning window shows the pair it selects.
    shell(&mut app, "window.kerning", json!({"select": [{"left": "A", "right": "V"}]}));
    assert_eq!(items(&app), json!(["A", "V", "A"]));
}

#[test]
fn view_show_group_members_draws_the_facing_groups_of_the_edited_pair() {
    let mut app = grouped_app();
    shell(&mut app, "editView.setText", json!({"glyphs": ["T", "o"]}));
    shell(&mut app, "editView.setCursor", json!({"index": 1}));
    assert!(app.models.text.members.is_empty(), "off at first");
    shell(&mut app, "view.showGroupMembers", json!({}));
    assert_eq!(get(&app)["groupMembers"], true, "omitted: toggle");

    let font = app.session.font().unwrap().clone();
    let layout = app.models.text.layout.clone().unwrap();
    let master = font.masters[0].id.clone();
    let shown: Vec<(String, usize, GroupSide, f64)> =
        members_shown(&font, &app.view.text, &layout, &master).into_iter().map(|m| (m.glyph, m.item, m.side, m.x)).collect();
    let (t, o) = (&layout.placed[0], &layout.placed[1]);
    assert_eq!(
        shown,
        vec![
            ("F".to_string(), 0, GroupSide::Right, t.x + t.advance - 600.0),
            ("c".to_string(), 1, GroupSide::Left, o.x),
            ("e".to_string(), 1, GroupSide::Left, o.x),
        ],
        "each member's facing side lines up with the glyph it stands in for"
    );
    assert_eq!(app.models.text.members.len(), 3);

    // Right to left, o faces T with its right side, which has no group here.
    shell(&mut app, "editView.setWritingDirection", json!({"direction": "rtl"}));
    let layout = app.models.text.layout.clone().unwrap();
    let names: Vec<String> = members_shown(&font, &app.view.text, &layout, &master).into_iter().map(|m| m.glyph).collect();
    assert!(names.is_empty(), "{names:?}");
    // The first glyph of a line has nothing before it.
    shell(&mut app, "editView.setWritingDirection", json!({"direction": "ltr"}));
    shell(&mut app, "editView.setCursor", json!({"index": 0}));
    let layout = app.models.text.layout.clone().unwrap();
    assert!(members_shown(&font, &app.view.text, &layout, &master).is_empty());
    shell(&mut app, "view.showGroupMembers", json!({"on": false}));
    assert!(app.models.text.members.is_empty());
}

#[test]
fn a_kerning_key_in_text_mode_kerns_the_pair_at_the_cursor() {
    let mut app = text_app();
    shell(&mut app, "editView.setText", json!({"glyphs": ["A", "V"]}));
    shell(&mut app, "editView.toggleTextMode", json!({"mode": "text"}));
    shell(&mut app, "editView.setCursor", json!({"index": 1}));
    let mods = Modifiers { ctrl: true, alt: true, ..Modifiers::NONE };
    assert_eq!(super::input::text_mode_command(Key::Left, mods), Some("kerning.increaseLeft"));
    let revision = |app: &FontCraftApp| app.session.active().unwrap().revision;
    let before = revision(&app);
    key(&mut app, Key::Left, mods);
    assert_ne!(revision(&app), before, "the pair before the cursor is kerned");
    shell(&mut app, "kerning.setMode", json!({"mode": "none"}));
    assert!(try_post(&mut app, Msg::Tool(ToolInput::Key(KeyEvent::press(Key::Left, mods)))).is_err(), "off without kerning");
}

#[test]
fn the_measurement_line_measures_each_glyph_in_text_mode_and_moves_with_cmd_ctrl_alt() {
    let mut app = text_app();
    run(&mut app, "path.addRectangle", json!({"glyph": "V", "x": 100, "y": 0, "width": 400, "height": 500}));
    shell(&mut app, "editView.setText", json!({"glyphs": ["A", "V", "A"]}));
    shell(&mut app, "view.showMeasurementLine", json!({}));
    assert!(app.models.text.measurement.is_none(), "only while the Text tool is active");
    shell(&mut app, "editView.toggleTextMode", json!({"mode": "text"}));
    let line = app.models.text.measurement.clone().expect("the line in text mode");
    let sides: Vec<Option<(f64, f64)>> = line.glyphs.iter().map(|g| g.sides).collect();
    assert_eq!(sides, [None, Some((100.0, 100.0)), None], "A has no outline; V is 100 from each side at {}", line.height);
    let t = TextTokens::standard();
    let shapes = painted(&app, super::paint::measurement_shapes);
    assert_eq!(count(&shapes, t.measurement), 1, "one rule per line of text");
    assert_eq!(count(&shapes, t.measurement_advance), 6, "both edges of every advance");
    let painter = Painter::new(egui::Context::default(), LayerId::background(), RECT);
    let words = CanvasWords::default();
    let overlay_words = crate::canvas::overlays::OverlayWords::default();
    let top = app.models.canvas.glyph.as_ref().map_or(0.0, |g| g.ascender);
    let cx = PaintCx {
        painter: &painter,
        rect: RECT,
        camera: Camera::new(app.view.canvas, top),
        model: &app.models.canvas,
        tokens: &app.models.canvas_tokens,
        display: super::display(&app.view),
        edit_background: app.view.edit_background,
        words: &words,
        tools: &app.tools.view,
        text: &app.models.text,
        overlays: crate::canvas::overlays::Painted { data: &app.overlays.data, words: &overlay_words },
        hinting: crate::canvas::hinting::HintPaint { model: &app.models.hint_canvas, view: &app.view.hinting, tokens: &app.models.hint_tokens },
        compatibility: &app.models.compatibility,
    };
    let labels: Vec<f64> = super::paint::measurement_labels(&cx).into_iter().map(|(_, v)| v).collect();
    assert_eq!(labels, [100.0, 100.0], "V's two sidebearings are printed");

    let mods = Modifiers { cmd: true, ctrl: true, alt: true, shift: false };
    pointer(&mut app, PointerKind::Down, 650.0, 120.4, mods);
    pointer(&mut app, PointerKind::Up, 650.0, 120.4, mods);
    assert_eq!(
        (get(&app)["measurementLineHeight"].clone(), get(&app)["cursor"].clone()),
        (json!(120.0), json!(3)),
        "the click moves the line, not the cursor"
    );
    assert!(try_shell(&mut app, "view.setMeasurementLineHeight", json!({"y": 2.0e7})).is_err());

    shell(&mut app, "editView.setWritingDirection", json!({"direction": "ttb"}));
    assert!(app.models.text.measurement.is_none(), "horizontal text only");
}
