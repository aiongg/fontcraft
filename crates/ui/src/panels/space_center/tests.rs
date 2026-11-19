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
use crate::app::models::space_center::{MatrixColumn, PlacedGlyph, SpaceCenterModel, SpaceLine};
use crate::canvas::tokens::Tokens;
use crate::shell::space_center::SpaceCenterState;

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

fn column(glyph: &str, left: f64, width: f64) -> MatrixColumn {
    MatrixColumn { glyph: glyph.into(), layer: "m01".into(), width: Some(width), left: Some(left), right: Some(width - left - 400.0) }
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
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, tokens: &tokens, sent: &mut sent });
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
