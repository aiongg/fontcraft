//! The Bitmap Glyph Editor's view: what its tools, switches, Shades palette and pixel canvas post.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::ui::contract::Tracked;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use serde_json::{Value, json};

use super::{Props, show};
use crate::app::models::bitmap_editor::BitmapEditorModel;
use crate::app::models::font_view::{StrikeCell, StrikeShown};
use crate::app::{Msg, ViewMsg};
use crate::canvas::tokens::Tokens;
use crate::shell::bitmap_editor::{BitmapCanvas, BitmapInput};

fn set(params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params("view.setBitmapEditor", params))
}

/// A 12-pixel strike at `depth` showing glyph A with a 2 × 2 bitmap at the origin.
fn model(depth: u8, tool: &str) -> BitmapEditorModel {
    let top = fontcraft_engine::model::strike::max_level(depth);
    let data = if depth == 1 { vec![0xc0, 0xc0] } else { vec![0xff; usize::from(depth) / 2 * 2] };
    BitmapEditorModel {
        glyph: Some("A".into()),
        strike: Some(StrikeShown { ppem: 12, depth, ascent: 9, descent: 3, magnification: 1 }),
        strike_label: fontcraft_engine::model::strike::label(12, depth),
        has_strikes: true,
        bitmap: Some(Arc::new(StrikeCell { id: 7, width: 2, height: 2, left: 0, bottom: 0, advance: 6, depth, data })),
        advance: 6,
        tool: tool.into(),
        level: top,
        max_level: top,
        show_outline: true,
        show_grid: true,
        ..Default::default()
    }
}

struct Shown {
    fixture: Fixture,
    posted: Rc<RefCell<Vec<Msg>>>,
    canvas: Rc<RefCell<BitmapCanvas>>,
}

fn shown(m: BitmapEditorModel) -> Shown {
    let model = Tracked::new(m);
    let tokens = Tokens::default();
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let canvas = Rc::new(RefCell::new(BitmapCanvas::default()));
    let held = Rc::clone(&canvas);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, tokens: &tokens, canvas: &mut held.borrow_mut(), sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.room(egui::vec2(900.0, 800.0)).settle();
    Shown { fixture, posted, canvas }
}

/// The pixels the canvas reported, as `(kind, x, y)` with the position floored to its pixel.
fn inputs(posted: &[Msg]) -> Vec<(&'static str, i32, i32)> {
    let cell = |[x, y]: [f64; 2]| (x.floor() as i32, y.floor() as i32);
    posted
        .iter()
        .filter_map(|m| match m {
            Msg::View(ViewMsg::Bitmap(BitmapInput::Press { at, .. })) => Some(("press", cell(*at))),
            Msg::View(ViewMsg::Bitmap(BitmapInput::Move(at))) => Some(("move", cell(*at))),
            Msg::View(ViewMsg::Bitmap(BitmapInput::Release(at))) => Some(("release", cell(*at))),
            _ => None,
        })
        .map(|(kind, (x, y))| (kind, x, y))
        .collect()
}

fn centre(canvas: &Rc<RefCell<BitmapCanvas>>, x: i32, y: i32) -> egui::Pos2 {
    canvas.borrow().tool.frame().and_then(|f| f.pixel_rect(x, y)).expect("the pixel on screen").center()
}

/// Pencil: a click on a pixel reports a press and a release on that pixel, which the shell turns into one toggle.
#[test]
fn a_pencil_click_reports_the_pixel_under_it() {
    let mut s = shown(model(1, "pencil"));
    let at = centre(&s.canvas, 3, 4);
    s.fixture.click_at(at).settle();
    assert_eq!(inputs(&s.posted.borrow()), vec![("press", 3, 4), ("release", 3, 4)]);
}

/// Line: a drag reports its press pixel, the pixels it moves over and the pixel it ends on.
#[test]
fn a_line_drag_reports_press_moves_and_release() {
    let mut s = shown(model(1, "line"));
    let (from, to) = (centre(&s.canvas, 0, 0), centre(&s.canvas, 4, 0));
    s.fixture.drag_at(from, to - from).settle();
    let seen = inputs(&s.posted.borrow());
    assert_eq!(seen.first(), Some(&("press", 0, 0)), "{seen:?}");
    assert_eq!(seen.last(), Some(&("release", 4, 0)), "{seen:?}");
    assert!(seen.iter().any(|(kind, ..)| *kind == "move"), "{seen:?}");
}

/// The tool, the switches and Recalculate post their commands; with Shift a drag reports the same gesture, which the
/// shell turns into a move.
#[test]
fn the_tool_buttons_and_switches_post_the_options() {
    let mut s = shown(model(1, "pencil"));
    let shift = s.fixture.by_label("Shift").expect("the Shift tool");
    s.fixture.click(&shift).settle();
    assert!(s.posted.borrow().contains(&set(json!({"tool": "shift"}))), "{:?}", s.posted.borrow());
    let grid = s.fixture.by_label("Grid").expect("the Grid switch");
    s.fixture.click(&grid).settle();
    assert!(s.posted.borrow().contains(&set(json!({"grid": false}))), "{:?}", s.posted.borrow());
    let recalculate = s.fixture.by_label("Recalculate").expect("the Recalculate button");
    s.fixture.click(&recalculate).settle();
    let regenerate = Msg::run("bitmap.regenerateGlyphs", json!({"glyphs": ["A"], "strikes": [{"ppem": 12, "depth": 1}]}));
    assert!(s.posted.borrow().contains(&regenerate), "{:?}", s.posted.borrow());

    let mut s = shown(model(1, "shift"));
    let (from, to) = (centre(&s.canvas, 1, 1), centre(&s.canvas, 3, 2));
    s.fixture.drag_at(from, to - from).settle();
    let seen = inputs(&s.posted.borrow());
    assert_eq!((seen.first(), seen.last()), (Some(&("press", 1, 1)), Some(&("release", 3, 2))), "{seen:?}");
}

/// The Shades palette shows on a grey strike only, and a click on a swatch picks its level.
#[test]
fn shades_show_only_on_grey_strikes() {
    let mut s = shown(model(1, "pencil"));
    s.fixture.settle();
    assert!(s.fixture.by_label("Shades").is_none(), "no Shades in a black-and-white strike");
    assert!(s.fixture.by_label("Level 1").is_none());

    let mut s = shown(model(2, "pencil"));
    assert!(s.fixture.by_label("Shades").is_some(), "Shades in a grey strike");
    let one = s.fixture.by_label("Level 1").expect("a swatch per level");
    assert!(s.fixture.by_label("Level 3").is_some());
    assert!(s.fixture.by_label("Level 4").is_none(), "a 2-bit strike has levels 0 to 3");
    s.fixture.click(&one).settle();
    assert!(s.posted.borrow().contains(&set(json!({"level": 1}))), "{:?}", s.posted.borrow());
}
