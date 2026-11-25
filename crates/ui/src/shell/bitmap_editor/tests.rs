//! The Bitmap Glyph Editor's app layer: the window and options commands, and each tool's stroke as one
//! `bitmap.setPixels` undo step.

use std::sync::Arc;

use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::model::strike::StrikeGlyph;
use serde_json::{Value, json};

use super::BitmapInput;
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};

/// A font with a black-and-white strike of 12 and a 2-bit strike of 12, glyph A holding one pixel at (1, 1) in the
/// first.
fn app() -> FontCraftApp {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.session.execute("bitmap.setStrikes", &json!({"add": [{"ppem": 12}, {"ppem": 12, "depth": 2}]})).unwrap();
    app.session
        .execute("bitmap.setPixels", &json!({"glyph": "A", "strike": {"ppem": 12, "depth": 1}, "pixels": [{"x": 1, "y": 1, "value": 1}]}))
        .unwrap();
    ok(&mut app, "window.bitmapEditor", json!({"glyph": "A", "strike": {"ppem": 12}}));
    app
}

fn shell(app: &mut FontCraftApp, id: &str, params: Value) -> Result<Value, String> {
    app.run_now(Msg::Shell(CommandRef::with_params(id, params)), false)
}

fn ok(app: &mut FontCraftApp, id: &str, params: Value) {
    shell(app, id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
}

/// Feeds the canvas inputs as the panel posts them, at pixel centres.
fn gesture(app: &mut FontCraftApp, steps: &[BitmapInput]) {
    for step in steps {
        app.sent.push(Msg::View(ViewMsg::Bitmap(*step)));
    }
    app.drain();
}

fn at(x: i32, y: i32) -> [f64; 2] {
    [f64::from(x) + 0.5, f64::from(y) + 0.5]
}

fn bitmap(app: &FontCraftApp, depth: u8) -> StrikeGlyph {
    let font = app.session.font().unwrap();
    let strike = font.bitmap_strikes.iter().find(|s| s.ppem == 12 && s.depth == depth).unwrap();
    strike.glyph("A").unwrap().clone()
}

/// The level of the pixel `(x, y)` counted from the glyph origin upwards, as the tools count them.
fn level(b: &StrikeGlyph, depth: u8, x: i32, y: i32) -> u8 {
    super::level_at(b, depth, (x, y))
}

fn undo(app: &mut FontCraftApp) {
    app.session.execute("edit.undo", &json!({})).unwrap();
}

/// Pencil: a click toggles one pixel; a drag from a clear pixel sets every pixel it passes over, without gaps; each
/// is one undo step.
#[test]
fn the_pencil_toggles_and_draws_in_one_step() {
    let mut app = app();
    gesture(&mut app, &[BitmapInput::Press { at: at(1, 1), alt: false }, BitmapInput::Release(at(1, 1))]);
    assert_eq!(level(&bitmap(&app, 1), 1, 1, 1), 0, "a click on a set pixel clears it");
    undo(&mut app);
    assert_eq!(level(&bitmap(&app, 1), 1, 1, 1), 1, "one undo step brings it back");

    gesture(&mut app, &[BitmapInput::Press { at: at(0, 3), alt: false }, BitmapInput::Move(at(4, 3)), BitmapInput::Release(at(4, 3))]);
    let b = bitmap(&app, 1);
    assert!((0..=4).all(|x| level(&b, 1, x, 3) == 1), "every pixel passed over is set");
    undo(&mut app);
    let b = bitmap(&app, 1);
    assert!((0..=4).all(|x| level(&b, 1, x, 3) == 0), "the whole drag was one undo step");
    assert_eq!(level(&b, 1, 1, 1), 1);
}

/// Line: the pixels between the press and the release are drawn, and the pixels the pointer passed on the way are
/// not.
#[test]
fn the_line_draws_from_press_to_release() {
    let mut app = app();
    ok(&mut app, "view.setBitmapEditor", json!({"tool": "line"}));
    gesture(
        &mut app,
        &[BitmapInput::Press { at: at(0, 5), alt: false }, BitmapInput::Move(at(0, 9)), BitmapInput::Move(at(3, 5)), BitmapInput::Release(at(3, 5))],
    );
    let b = bitmap(&app, 1);
    assert!((0..=3).all(|x| level(&b, 1, x, 5) == 1), "the run is drawn");
    assert_eq!(level(&b, 1, 0, 8), 0, "the pixels of an earlier pointer position are not");
}

/// Shift: a drag moves the bitmap by whole pixels; a shift back to where it started changes nothing.
#[test]
fn the_shift_moves_the_bitmap() {
    let mut app = app();
    let before = bitmap(&app, 1);
    ok(&mut app, "view.setBitmapEditor", json!({"tool": "shift"}));
    gesture(&mut app, &[BitmapInput::Press { at: at(1, 1), alt: false }, BitmapInput::Move(at(3, 0)), BitmapInput::Release(at(3, 0))]);
    let after = bitmap(&app, 1);
    assert_eq!((after.left, after.bottom), (before.left + 2, before.bottom - 1));
    assert_eq!(after.data, before.data, "the pixels move with it");

    gesture(&mut app, &[BitmapInput::Press { at: at(1, 1), alt: false }, BitmapInput::Move(at(5, 5)), BitmapInput::Release(at(1, 1))]);
    let back = bitmap(&app, 1);
    assert_eq!((back.left, back.bottom), (after.left, after.bottom), "a shift back changes nothing");
    undo(&mut app);
    let undone = bitmap(&app, 1);
    assert_eq!((undone.left, undone.bottom), (before.left, before.bottom), "and leaves no undo step");
}

/// In a grey strike the pencil draws the chosen shade, and Alt with a press picks the level under the pointer.
#[test]
fn a_grey_strike_draws_the_chosen_shade() {
    let mut app = app();
    ok(&mut app, "window.bitmapEditor", json!({"glyph": "A", "strike": {"ppem": 12, "depth": 2}}));
    ok(&mut app, "view.setBitmapEditor", json!({"level": 2}));
    gesture(&mut app, &[BitmapInput::Press { at: at(0, 0), alt: false }, BitmapInput::Release(at(0, 0))]);
    assert_eq!(level(&bitmap(&app, 2), 2, 0, 0), 2);
    ok(&mut app, "view.setBitmapEditor", json!({"level": 3}));
    gesture(&mut app, &[BitmapInput::Press { at: at(0, 0), alt: true }]);
    assert_eq!(app.view.bitmap_editor.level, Some(2), "Alt picks the level under the pointer");
    assert!(app.view.bitmap_editor.stroke.is_none(), "and draws nothing");
}

/// The window refuses a font without strikes, a glyph or strike the font lacks, and an unknown tool.
#[test]
fn the_commands_refuse_what_the_font_lacks() {
    let mut bare = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    assert!(shell(&mut bare, "window.bitmapEditor", json!({})).unwrap_err().contains("no bitmap strikes"));
    let mut app = app();
    assert!(shell(&mut app, "window.bitmapEditor", json!({"glyph": "nope"})).unwrap_err().contains("no glyph"));
    assert!(shell(&mut app, "window.bitmapEditor", json!({"strike": {"ppem": 99}})).unwrap_err().contains("no strike 99"));
    assert!(shell(&mut app, "view.setBitmapEditor", json!({"tool": "eraser"})).is_err());
    assert!(shell(&mut app, "view.setBitmapEditor", json!({"level": 300})).is_err());
    let mut state = serde_json::Map::new();
    super::inspect(&app, &mut state);
    assert_eq!(state["bitmapEditor"]["strike"], json!("12"));
    assert_eq!(state["bitmapEditor"]["bitmap"]["width"], json!(1));
}

/// A shape is one undo step of its pixels; Magnify and Hand draw nothing; the pixel size is set, refused outside its
/// range and fitted again.
#[test]
fn shapes_draw_in_one_step_and_the_view_tools_draw_nothing() {
    let mut app = app();
    ok(&mut app, "view.setBitmapEditor", json!({"tool": "rectangle"}));
    gesture(&mut app, &[BitmapInput::Press { at: at(3, 3), alt: false }, BitmapInput::Move(at(9, 9)), BitmapInput::Release(at(5, 6))]);
    let b = bitmap(&app, 1);
    assert!(level(&b, 1, 3, 3) == 1 && level(&b, 1, 5, 6) == 1 && level(&b, 1, 4, 3) == 1);
    assert!(level(&b, 1, 4, 4) == 0 && level(&b, 1, 9, 9) == 0);
    undo(&mut app);
    assert_eq!(level(&bitmap(&app, 1), 1, 3, 3), 0, "one undo step");
    let before = bitmap(&app, 1);
    for tool in ["hand", "magnify"] {
        ok(&mut app, "view.setBitmapEditor", json!({"tool": tool}));
        gesture(&mut app, &[BitmapInput::Press { at: at(3, 3), alt: false }, BitmapInput::Release(at(3, 3))]);
    }
    assert_eq!(bitmap(&app, 1), before);
    ok(&mut app, "view.setBitmapEditor", json!({"pixelSize": 16}));
    assert_eq!(app.view.bitmap_editor.pixel_size, Some(16));
    assert!(shell(&mut app, "view.setBitmapEditor", json!({"pixelSize": 1})).is_err());
    ok(&mut app, "view.setBitmapEditor", json!({"fit": true}));
    assert_eq!(app.view.bitmap_editor.pixel_size, None);
}
