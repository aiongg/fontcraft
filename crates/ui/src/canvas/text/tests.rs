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
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
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

/// Alt with Left or Right moves by word and Cmd with Up or Down to the start or end of the text; Shift extends the
/// selection with both. None of these keys is a text-mode shortcut.
#[test]
fn alt_arrows_move_by_word_and_cmd_up_down_to_the_ends_of_the_text() {
    let alt = Modifiers { alt: true, ..Modifiers::NONE };
    let alt_shift = Modifiers { alt: true, shift: true, ..Modifiers::NONE };
    let cmd = Modifiers { cmd: true, ..Modifiers::NONE };
    let cmd_shift = Modifiers { cmd: true, shift: true, ..Modifiers::NONE };
    for (k, m) in [(Key::Left, alt), (Key::Right, alt_shift), (Key::Up, cmd), (Key::Down, cmd_shift)] {
        assert_eq!(super::input::text_mode_command(k, m), None, "{k:?} {m:?} is no text-mode shortcut");
    }
    let mut app = text_app();
    try_post(&mut app, Msg::View(ViewMsg::SelectTool(super::TEXT_TOOL))).unwrap();
    shell(&mut app, "editView.setText", json!({"glyphs": ["H", "o", "space", "H", "o", "space", "H", "o"]}));
    shell(&mut app, "editView.setCursor", json!({"index": 0}));
    let at = |app: &FontCraftApp| (get(app)["cursor"].clone(), get(app)["selection"].clone());

    key(&mut app, Key::Right, alt);
    assert_eq!(at(&app), (json!(2), json!([2, 2])), "Alt+Right: to the end of the first word");
    key(&mut app, Key::Right, alt);
    assert_eq!(at(&app), (json!(5), json!([5, 5])), "past the space to the end of the next word");
    key(&mut app, Key::Right, alt_shift);
    assert_eq!(at(&app), (json!(8), json!([5, 8])), "Shift extends the selection by a word");
    key(&mut app, Key::Left, alt);
    assert_eq!(at(&app), (json!(3), json!([3, 3])), "Alt+Left from the selection's start, to the start of its word");
    key(&mut app, Key::Left, alt_shift);
    assert_eq!(at(&app), (json!(0), json!([0, 3])));

    key(&mut app, Key::Down, cmd);
    assert_eq!(at(&app), (json!(8), json!([8, 8])), "Cmd+Down: to the end of the text");
    key(&mut app, Key::Up, cmd_shift);
    assert_eq!(at(&app), (json!(0), json!([0, 8])), "Cmd+Shift+Up selects back to the start");
    key(&mut app, Key::Down, cmd_shift);
    assert_eq!(at(&app), (json!(8), json!([8, 8])), "the anchor stays at the end");
    key(&mut app, Key::Up, cmd);
    assert_eq!(at(&app), (json!(0), json!([0, 0])), "Cmd+Up: to the start of the text");
    assert_eq!(items(&app).as_array().map(Vec::len), Some(8), "no key changed the text");
}

/// The text wraps at `appearance.textViewWidth` thousandths of an em: in a 2,048-unit font the default is 20,480
/// units, and a narrower setting breaks a long line.
#[test]
fn the_text_view_width_is_in_thousandths_of_an_em() {
    use crate::app::models::text::{TEXT_VIEW_WIDTH, line_width};

    let mut font = fontcraft_engine::model::Font::new("Wide");
    font.units_per_em = 2048;
    let mut session = Session::new();
    session.add_document(font, None).unwrap();
    assert_eq!(line_width(session.settings(), 2048), 20_480.0);
    assert_eq!(line_width(session.settings(), 1000), 10_000.0);
    session.execute("settings.set", &json!({"id": TEXT_VIEW_WIDTH, "value": 500})).unwrap();
    assert_eq!(line_width(session.settings(), 2048), 1024.0);
    assert!(session.execute("settings.set", &json!({"id": TEXT_VIEW_WIDTH, "value": 0})).is_err(), "a width of nothing is refused");

    let mut app = FontCraftApp::with_edit_tab(session, Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    shell(&mut app, "editView.setText", json!({"glyphs": ["A", "B", "A", "B", "A", "B", "A", "B"]}));
    let lines = |app: &FontCraftApp| app.models.text.layout.as_ref().map_or(0, |l| l.lines.len());
    assert!(lines(&app) > 1, "eight glyphs do not fit half an em");
    run(&mut app, "settings.set", json!({"id": TEXT_VIEW_WIDTH, "value": 100_000}));
    assert_eq!(lines(&app), 1, "a hundred em holds them on one line");
}

/// Typing past the canvas's edge scrolls it sideways so the cursor stays in view; the zoom stays.
#[test]
fn typing_keeps_the_cursor_in_view() {
    let mut app = text_app();
    try_post(&mut app, Msg::View(ViewMsg::SelectTool(super::TEXT_TOOL))).unwrap();
    let cursor_x = |app: &FontCraftApp| {
        let [[x, _], _] = app.models.text.cursor.expect("a cursor");
        app.view.canvas.to_screen(KPoint::new(x, 0.0)).x
    };
    for _ in 0..16 {
        key(&mut app, Key::Char('h'), SHIFT);
        let x = cursor_x(&app);
        assert!((0.0..=RECT.max.x).contains(&x), "the cursor at {x} after {:?}", items(&app));
    }
    assert_eq!(app.view.canvas.zoom, 0.25);
}

/// Setting the whole text fits it in the canvas.
#[test]
fn set_text_fits_the_new_text() {
    let mut app = text_app();
    shell(&mut app, "editView.setText", json!({"glyphs": ["H", "O", "H", "O", "H", "O", "H", "O", "H", "O", "H", "O"]}));
    let layout = app.models.text.layout.clone().expect("a layout");
    let (x0, x1) = layout.placed.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.x), b.max(p.x + p.advance)));
    let o = app.models.text.origin[0];
    let screen = |x: f64| app.view.canvas.to_screen(KPoint::new(x - o, 0.0)).x;
    assert!(x1 - x0 > 1280.0 / 0.25, "the text is wider than the canvas at the old zoom");
    assert!(0.0 <= screen(x0) && screen(x1) <= RECT.max.x, "{}..{}", screen(x0), screen(x1));
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
    run(&mut app, "path.addRectangle", json!({"glyph": "A", "x": 100, "y": 0, "width": 300, "height": 600}));
    run(&mut app, "kerning.setPair", json!({"left": "A", "right": "V", "value": -100}));
    shell(&mut app, "editView.setText", json!({"glyphs": ["A", "V", "A"]}));
    shell(&mut app, "editView.setCursor", json!({"index": 0}));
    let t = TextTokens::standard();
    let meshes = |shapes: &[Shape]| shapes.iter().filter(|s| matches!(s, Shape::Mesh(_))).count();
    // Fill Preview is on by default: the other glyphs are filled, and the edited one only under the Text tool.
    assert_eq!(meshes(&painted(&app, glyph_shapes)), 2, "V and the second A, filled");
    assert_eq!(meshes(&painted(&app, crate::canvas::paint::outline::shapes)), 0, "the Select tool shows A unfilled");
    shell(&mut app, "view.fillPreview", json!({"on": false}));
    assert_eq!(count(&painted(&app, glyph_shapes), t.glyph), 2, "V and the second A as outlines");
    shell(&mut app, "view.fillPreview", json!({"on": true}));
    assert_eq!(count(&painted(&app, behind_shapes), app.models.canvas_tokens.kerning_negative), 1, "the A V gap");
    run(&mut app, "settings.set", json!({"id": "appearance.color.kerningNegative.light", "value": "#ff00ff"}));
    assert_eq!(count(&painted(&app, behind_shapes), Color32::from_rgb(0xff, 0, 0xff)), 1, "the gap takes the set colour");
    assert!(painted(&app, cursor_shapes).is_empty(), "no cursor outside text mode");

    shell(&mut app, "editView.toggleTextMode", json!({"mode": "text"}));
    assert_eq!(count(&painted(&app, cursor_shapes), t.cursor), 1);
    assert_eq!(meshes(&painted(&app, crate::canvas::paint::outline::shapes)), 1, "the Text tool fills the edited glyph");
    assert!(!super::display(&app.view).nodes, "text mode hides the nodes");
    shell(&mut app, "kerning.setMode", json!({"mode": "none"}));
    assert_eq!(count(&painted(&app, behind_shapes), app.models.canvas_tokens.kerning_negative), 0, "no kerning shown without kerning");
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
fn step_group_member_walks_one_side_of_the_edited_pair_through_its_group() {
    let mut app = grouped_app();
    shell(&mut app, "editView.setText", json!({"glyphs": ["T", "o"]}));
    shell(&mut app, "editView.setCursor", json!({"index": 1}));
    let font = app.session.font().unwrap().clone();
    let lefts: Vec<String> = fontcraft_spacing::kerning::members(&font, GroupSide::Left, "o").into_iter().map(str::to_string).collect();
    let rights: Vec<String> = fontcraft_spacing::kerning::members(&font, GroupSide::Right, "T").into_iter().map(str::to_string).collect();
    let after = |list: &[String], glyph: &str| {
        let at = list.iter().position(|g| g == glyph).unwrap();
        list[(at + 1) % list.len()].clone()
    };
    let o_next = after(&lefts, "o");
    shell(&mut app, "kerning.stepGroupMember", json!({"side": "right"}));
    assert_eq!((items(&app), get(&app)["cursor"].clone()), (json!(["T", o_next]), json!(1)), "the right glyph steps through its left group");
    shell(&mut app, "kerning.stepGroupMember", json!({"side": "right", "direction": "previous"}));
    assert_eq!(items(&app), json!(["T", "o"]), "previous goes back");
    let t_next = after(&rights, "T");
    shell(&mut app, "kerning.stepGroupMember", json!({"side": "left"}));
    assert_eq!(items(&app), json!([t_next, "o"]), "the left glyph steps through its right group");
    assert!(try_shell(&mut app, "kerning.stepGroupMember", json!({})).is_err(), "the side is required");
    shell(&mut app, "editView.setText", json!({"glyphs": ["A", "V"]}));
    shell(&mut app, "editView.setCursor", json!({"index": 1}));
    assert!(try_shell(&mut app, "kerning.stepGroupMember", json!({"side": "left"})).is_err(), "A has no right group");
    assert_eq!(items(&app), json!(["A", "V"]), "a refused step changes nothing");
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

/// A kerning key changes the value the line shows and the Info box reads: in the master both glyphs of the pair are shown
/// in, else in the tab's.
#[test]
fn a_kerning_key_writes_to_the_master_the_pair_is_kerned_in() {
    let mut app = text_app();
    app.session
        .edit("setup", |font| {
            let mut m = font.default_master().unwrap().clone();
            m.id = "m02".into();
            m.name = "Bold".into();
            Arc::make_mut(&mut font.masters).push(m);
            for name in ["A", "V"] {
                font.glyph_mut(name)?.layers.push(fontcraft_engine::model::Layer::new("m02", 600.0));
            }
            Ok(())
        })
        .unwrap();
    let first = app.session.font().unwrap().masters[0].id.clone();
    shell(&mut app, "editView.setText", json!({"glyphs": ["A", "V"]}));
    shell(&mut app, "editView.toggleTextMode", json!({"mode": "text"}));
    assert!(app.view.text.text.set_layer(0, Some("m02")) && app.view.text.text.set_layer(1, Some("m02")));
    shell(&mut app, "editView.setCursor", json!({"index": 1}));
    let pair = |app: &mut FontCraftApp, master: &str| {
        app.session.query("kerning.getPair", &json!({"left": "A", "right": "V", "master": master})).unwrap()["value"].as_f64()
    };
    let shown = |app: &FontCraftApp| app.models.text.layout.as_ref().unwrap().placed[1].kern;
    let field = |app: &FontCraftApp| app.models.palette.info_box.kerning.as_ref().unwrap().edges[0].pair.as_ref().and_then(|p| p.value);
    let before = pair(&mut app, &first);
    key(&mut app, Key::Left, Modifiers { ctrl: true, alt: true, shift: true, ..Modifiers::NONE });
    assert_eq!(pair(&mut app, &first), before, "the tab's master keeps its kerning");
    let bold = pair(&mut app, "m02").unwrap();
    assert_eq!((shown(&app), field(&app)), (bold, Some(bold)));
    assert_ne!(Some(bold), before);

    // V shown in the tab's master: the line kerns the two in it, and so does the key; the Info box has no field.
    assert!(app.view.text.text.set_layer(1, None));
    shell(&mut app, "editView.setCursor", json!({"index": 1}));
    key(&mut app, Key::Right, Modifiers { ctrl: true, alt: true, ..Modifiers::NONE });
    assert_eq!(pair(&mut app, "m02"), Some(bold));
    assert_ne!(pair(&mut app, &first), before);
    assert_eq!(Some(shown(&app)), pair(&mut app, &first));
    assert!(app.models.palette.info_box.kerning.as_ref().unwrap().edges[0].pair.is_none());
}

/// In top-to-bottom text Up and Down with Ctrl and Cmd change the sidebearings above and below, with Alt the kerning
/// above and below; Cmd with an arrow across the column goes to the ends of the text.
#[test]
fn the_spacing_and_kerning_keys_act_along_the_column_in_top_to_bottom_text() {
    let mut app = text_app();
    run(&mut app, "path.addRectangle", json!({"glyph": "V", "x": 100, "y": 100, "width": 200, "height": 200}));
    shell(&mut app, "editView.setText", json!({"glyphs": ["A", "V", "A"]}));
    shell(&mut app, "editView.toggleTextMode", json!({"mode": "text"}));
    shell(&mut app, "editView.setWritingDirection", json!({"direction": "ttb"}));
    shell(&mut app, "editView.setCursor", json!({"index": 1}));
    let sides = |app: &mut FontCraftApp| {
        let m = app.session.query("spacing.getMetrics", &json!({"glyph": "V"})).unwrap();
        let side = |key: &str| m[key].as_f64().unwrap_or_else(|| panic!("{key} in {m}"));
        (side("top"), side("bottom"), side("left"))
    };
    let ctrl = Modifiers { ctrl: true, ..Modifiers::NONE };
    let cmd = Modifiers { cmd: true, ..Modifiers::NONE };
    let start = sides(&mut app);
    for (k, mods, top, bottom) in [
        (Key::Up, ctrl, 1.0, 0.0),
        (Key::Down, ctrl, -1.0, 0.0),
        (Key::Down, cmd, 0.0, 1.0),
        (Key::Up, cmd, 0.0, -1.0),
        (Key::Down, Modifiers { shift: true, ..ctrl }, -10.0, 0.0),
    ] {
        let before = sides(&mut app);
        key(&mut app, k, mods);
        assert_eq!(sides(&mut app), (before.0 + top, before.1 + bottom, before.2), "{k:?} {mods:?}");
    }
    assert_eq!(sides(&mut app).2, start.2, "the horizontal metrics stay");

    let kern = |app: &mut FontCraftApp, left: &str, right: &str| {
        let query = json!({"left": left, "right": right, "direction": "vertical"});
        app.session.query("kerning.getPair", &query).unwrap()["value"].as_f64().unwrap_or(0.0)
    };
    for (k, mods, above, below) in [
        (Key::Up, Modifiers { alt: true, ..ctrl }, 1.0, 0.0),
        (Key::Down, Modifiers { alt: true, ..ctrl }, -1.0, 0.0),
        (Key::Down, Modifiers { alt: true, ..cmd }, 0.0, 1.0),
        (Key::Up, Modifiers { alt: true, ..cmd }, 0.0, -1.0),
    ] {
        let before = (kern(&mut app, "A", "V"), kern(&mut app, "V", "A"));
        key(&mut app, k, mods);
        assert_eq!((kern(&mut app, "A", "V"), kern(&mut app, "V", "A")), (before.0 + above, before.1 + below), "{k:?} {mods:?}");
    }

    key(&mut app, Key::Left, cmd);
    assert_eq!(get(&app)["cursor"], json!(3));
    key(&mut app, Key::Right, cmd);
    assert_eq!(get(&app)["cursor"], json!(0));
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

/// A filter fired in text mode acts on the glyphs the text selection shows, and only those
/// (`filter.target.editViewTextSelection`).
#[test]
fn a_filter_in_text_mode_changes_the_selected_glyphs() {
    let mut app = text_app();
    for g in ["A", "V", "O", "H"] {
        app.session.execute("glyph.newGlyph", &json!({"name": g})).ok();
        app.session.execute("path.addRectangle", &json!({"glyph": g, "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    }
    try_post(&mut app, Msg::View(ViewMsg::SelectTool(super::TEXT_TOOL))).unwrap();
    shell(&mut app, "editView.setText", json!({"glyphs": ["A", "V", "O", "H"]}));
    shell(&mut app, "editView.setCursor", json!({"index": 0, "selectionLength": 3}));
    let targeted = app.with_targets(CommandRef::new("filter.roundCorners"));
    assert_eq!(targeted.params["glyphs"], json!(["A", "V", "O"]), "{}", targeted.params);
    assert!(targeted.params.get("glyph").is_none());

    let opened = crate::dialogs::form::open_command(&mut app, &json!({"dialog": "dialog.filterRoundCorners"})).unwrap();
    assert_eq!(opened["params"]["glyphs"], json!(["A", "V", "O"]));
    crate::dialogs::form::confirm(&mut app).unwrap();
    let nodes = |app: &mut FontCraftApp, g: &str| {
        app.session.query("layer.inspect", &json!({"glyph": g})).unwrap()["shapes"][0]["nodes"].as_array().map_or(0, Vec::len)
    };
    for g in ["A", "V", "O"] {
        assert!(nodes(&mut app, g) > 4, "{g} is rounded");
    }
    assert_eq!(nodes(&mut app, "H"), 4, "H lies outside the selection");
}
