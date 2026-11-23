//! The Space Center's view: what its fields, matrix and menu post.

use std::cell::RefCell;
use std::rc::Rc;

use craft::ui::contract::Tracked;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use egui::{Key, Modifiers};
use serde_json::{Value, json};

use super::{Props, show};
use crate::app::Msg;
use crate::app::models::space_center::{GlyphArt, MatrixColumn, PlacedGlyph, SpaceCenterModel, SpaceLine};
use crate::canvas::images::cells::CellImages;
use crate::canvas::tokens::Tokens;
use crate::shell::space_center::drag::SpacePreview;
use crate::shell::space_center::{Display, SpaceCenterState};

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

fn column(glyph: &str, left: f64, width: f64) -> MatrixColumn {
    MatrixColumn {
        glyph: glyph.into(),
        layer: "m01".into(),
        width: Some(width),
        left: Some(left),
        right: Some(width - left - 400.0),
        master_layers: vec!["m01".into(), "m02".into()],
    }
}

/// Typing a text, pressing a matrix row, entering an expression and picking a display option post the window's
/// commands.
#[test]
fn the_window_posts_text_selection_entries_and_display_options() {
    let glyphs = vec![
        PlacedGlyph { glyph: Some("n".into()), offset: 0.0, advance: 500.0, column: Some(0), ..Default::default() },
        PlacedGlyph { glyph: Some("o".into()), offset: 500.0, advance: 520.0, column: Some(1), ..Default::default() },
    ];
    let model = Tracked::new(SpaceCenterModel {
        has_font: true,
        state: SpaceCenterState { text: "no".into(), selected: Some(0), ..Default::default() },
        master: Some("m01".into()),
        upm: 1000.0,
        ascender: 800.0,
        line_height: 1100.0,
        lines: vec![SpaceLine { glyphs, length: 1020.0 }],
        columns: vec![column("n", 50.0, 500.0), column("o", 40.0, 520.0)],
        ..Default::default()
    });
    let tokens = Tokens::default();
    let images = CellImages::default();
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut preview = SpacePreview::default();
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, images: &images, tokens: &tokens, preview: &mut preview, sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.room(egui::vec2(900.0, 700.0)).settle();

    let text = fixture.by_label("Text").expect("the text field");
    fixture.click(&text).key(Key::End, Modifiers::NONE).type_text("e").settle();
    assert!(posted.borrow().contains(&shell("window.spaceCenter", json!({"text": "noe"}))), "{:?}", posted.borrow());

    let o = fixture.by_label("o").expect("the o row");
    fixture.click(&o).settle();
    assert!(posted.borrow().contains(&shell("window.spaceCenter", json!({"select": 1}))), "{:?}", posted.borrow());

    posted.borrow_mut().clear();
    let left = fixture.by_label("Left Margin").expect("the Left Margin field");
    fixture.click(&left).key(Key::A, Modifiers::COMMAND).type_text("=o+5").key(Key::Enter, Modifiers::NONE).settle();
    assert!(posted.borrow().contains(&shell("spaceCenter.setValue", json!({"column": 0, "row": "left", "text": "=o+5"}))), "{:?}", posted.borrow());

    let display = fixture.by_label("Display").expect("the Display button");
    fixture.click(&display).settle();
    let zones = fixture.by_label("Zones").expect("the Zones entry");
    fixture.click(&zones).settle();
    assert!(posted.borrow().contains(&shell("view.setSpaceCenterDisplay", json!({"zones": true}))), "{:?}", posted.borrow());
}

/// A drag in the preview runs one command when it ends: from inside a glyph it moves the outline, from its left edge
/// with Alt held it changes the left margin of every master as one batch; an arrow nudges the selected glyph.
#[test]
fn preview_drags_and_arrows_change_the_selected_glyph() {
    let glyphs = vec![
        PlacedGlyph { glyph: Some("n".into()), offset: 0.0, advance: 500.0, column: Some(0), ..Default::default() },
        PlacedGlyph { glyph: Some("o".into()), offset: 500.0, advance: 520.0, column: Some(1), ..Default::default() },
    ];
    let model = Tracked::new(SpaceCenterModel {
        has_font: true,
        state: SpaceCenterState { text: "no".into(), selected: Some(0), ..Default::default() },
        master: Some("m01".into()),
        upm: 1000.0,
        ascender: 800.0,
        line_height: 1100.0,
        lines: vec![SpaceLine { glyphs, length: 1020.0 }],
        columns: vec![column("n", 50.0, 500.0), column("o", 40.0, 520.0)],
        ..Default::default()
    });
    let tokens = Tokens::default();
    let images = CellImages::default();
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let preview = Rc::new(RefCell::new(SpacePreview::default()));
    let shown = Rc::clone(&preview);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, images: &images, tokens: &tokens, preview: &mut shown.borrow_mut(), sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.room(egui::vec2(900.0, 700.0)).settle();
    let n = preview.borrow().tool.frame().screen_rect(0).expect("the n glyph in the preview");

    // 20 points at 72 points per 1000 units: 278 units.
    fixture.drag_at(n.center(), egui::vec2(20.0, 0.0)).settle();
    assert_eq!(
        *posted.borrow(),
        vec![Msg::run("spacing.shiftOutlineRight", json!({"glyphs": ["n"], "layer": "m01", "amount": 278.0}))],
        "one command for the whole drag"
    );

    posted.borrow_mut().clear();
    fixture.hold(Modifiers::ALT).drag_at(egui::pos2(n.left() + 1.0, n.center().y), egui::vec2(-10.0, 0.0)).hold(Modifiers::NONE).settle();
    let step = |layer: &str| json!({"command": "spacing.increaseLeftSidebearing", "params": {"glyphs": ["n"], "layer": layer, "amount": 139.0}});
    assert_eq!(*posted.borrow(), vec![Msg::run("command.batch", json!({"label": "Increase Left Sidebearing", "steps": [step("m01"), step("m02")]}))]);

    posted.borrow_mut().clear();
    fixture.key(Key::ArrowRight, Modifiers::SHIFT).settle();
    assert_eq!(*posted.borrow(), vec![Msg::run("spacing.increaseRightSidebearing", json!({"glyphs": ["n"], "layer": "m01", "amount": 10.0}))]);
}

/// A model of one glyph, `n`, a square drawn with `art`, in the given display options.
fn one_glyph(display: Display, art: GlyphArt) -> SpaceCenterModel {
    let glyphs = vec![PlacedGlyph { glyph: Some("n".into()), offset: 0.0, advance: 500.0, column: Some(0), ..Default::default() }];
    SpaceCenterModel {
        has_font: true,
        state: SpaceCenterState { text: "n".into(), display, ..Default::default() },
        master: Some("m01".into()),
        upm: 1000.0,
        ascender: 800.0,
        line_height: 1100.0,
        lines: vec![SpaceLine { glyphs, length: 500.0 }],
        art: [("n".to_string(), std::sync::Arc::new(art))].into_iter().collect(),
        columns: vec![column("n", 50.0, 500.0)],
        ..Default::default()
    }
}

fn square() -> fontcraft_geom::BezPath {
    let mut p = fontcraft_geom::BezPath::new();
    p.move_to((50.0, 0.0));
    p.line_to((450.0, 0.0));
    p.line_to((450.0, 400.0));
    p.line_to((50.0, 400.0));
    p.close_path();
    p
}

/// The shapes of one frame of the window showing `model` with `images`.
fn drawn(model: SpaceCenterModel, images: CellImages) -> Vec<egui::Shape> {
    let model = Tracked::new(model);
    let tokens = Tokens::default();
    let mut preview = SpacePreview::default();
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, images: &images, tokens: &tokens, preview: &mut preview, sent: &mut sent });
    });
    fixture.room(egui::vec2(900.0, 700.0)).settle();
    fixture.frame_shapes()
}

fn textured(shapes: &[egui::Shape]) -> usize {
    shapes.iter().filter(|s| matches!(s, egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default())).count()
}

/// Untextured meshes in the glyphs' ink: filled outlines.
fn inked(shapes: &[egui::Shape]) -> usize {
    let ink = Tokens::default().outline;
    shapes
        .iter()
        .filter(|s| matches!(s, egui::Shape::Mesh(m) if m.texture_id == egui::TextureId::default() && m.vertices.iter().any(|v| v.color == ink)))
        .count()
}

/// With Images on, a glyph's placed image is drawn as one textured quad; with it off, not at all.
#[test]
fn the_images_option_draws_placed_images() {
    use std::sync::Arc;

    use fontcraft_engine::imagery::Rgba8;

    use crate::canvas::images::files::Pixels;
    use crate::canvas::images::{Look, PlacedImage};

    let pixels = Pixels(Arc::new(Rgba8 { width: 2, height: 2, pixels: vec![200; 16] }));
    let image = PlacedImage {
        place: fontcraft_geom::Affine::scale(200.0),
        visible: Some(fontcraft_geom::Rect::new(0.0, 0.0, 2.0, 2.0)),
        alpha: 1.0,
        look: Look::Pixels { pixels, size: [2, 2] },
    };
    let images = CellImages { by_glyph: [("n".to_string(), Arc::new(image))].into_iter().collect() };
    let art = || GlyphArt { layer: "m01".into(), outline: square(), ..Default::default() };
    let on = drawn(one_glyph(Display { images: true, ..Display::default() }, art()), images.clone());
    let off = drawn(one_glyph(Display { images: false, ..Display::default() }, art()), images);
    assert_eq!(textured(&on), textured(&off) + 1, "the image is one textured quad");
    assert_eq!(inked(&on), inked(&off), "the outline is drawn either way");
    // The image lies behind the glyph: its quad comes before the filled outline.
    let image_at = on.iter().position(|s| matches!(s, egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default()));
    let ink = Tokens::default().outline;
    let outline_at = on
        .iter()
        .position(|s| matches!(s, egui::Shape::Mesh(m) if m.texture_id == egui::TextureId::default() && m.vertices.iter().any(|v| v.color == ink)));
    assert!(image_at.is_some() && outline_at.is_some() && image_at < outline_at, "{image_at:?} {outline_at:?}");
}

/// With Color on, a colour glyph is drawn from its palette layers (one texture) in place of its filled outline; with
/// it off, its outline is filled and no colour is drawn.
#[test]
fn the_color_option_draws_palette_layers_in_place_of_the_outline() {
    use std::sync::Arc;

    use fontcraft_render::color::artwork::{Drawable, Item, Leaf};

    use crate::canvas::color::{ColorArt, Content};

    let red = Leaf::Solid([1.0, 0.0, 0.0, 1.0]);
    let drawing = Arc::new(ColorArt::new(Content::Items(vec![Item::Fill(Drawable { clips: vec![square()], leaf: red })])));
    let art = || GlyphArt { layer: "m01".into(), outline: square(), color: Some(Arc::clone(&drawing)), ..Default::default() };
    let on = drawn(one_glyph(Display { color: true, ..Display::default() }, art()), CellImages::default());
    let off = drawn(one_glyph(Display { color: false, ..Display::default() }, art()), CellImages::default());
    assert_eq!(textured(&on), textured(&off) + 1, "the palette layers are one texture");
    assert!(inked(&off) >= 1, "with Color off the outline is filled");
    assert_eq!(inked(&on) + 1, inked(&off), "with Color on the outline is not filled");
}
