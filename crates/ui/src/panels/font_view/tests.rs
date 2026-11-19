//! Font View as the user drives it: what a click, Return, the plus and minus buttons and a header press post.

use std::cell::RefCell;
use std::rc::Rc;

use craft::ui::contract::Tracked;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use egui::{Key, Modifiers};
use serde_json::json;

use super::{Props, show};
use crate::app::Msg;
use crate::app::font_view::{Column, ViewMode};
use crate::app::models::font_view::{FontViewModel, GlyphRow};

fn model(mode: ViewMode, selected: Vec<usize>) -> FontViewModel {
    let rows: Vec<GlyphRow> = ["A", "B", "C"]
        .iter()
        .map(|name| {
            let mut cells = vec![String::new(); Column::ALL.len()];
            if let Some(cell) = cells.get_mut(Column::Name.index()) {
                *cell = (*name).to_string();
            }
            GlyphRow { name: (*name).to_string(), export: true, cells, ..GlyphRow::default() }
        })
        .collect();
    FontViewModel {
        has_font: true,
        mode,
        cell_size: 64.0,
        visible: (0..rows.len()).collect(),
        anchor: selected.last().copied(),
        selected,
        columns: vec![Column::Name, Column::Unicode],
        ascender: 800.0,
        descender: -200.0,
        rows,
        ..FontViewModel::default()
    }
}

/// A fixture over `model` and the messages it posted.
fn open(model: FontViewModel) -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
    let model = Tracked::new(model);
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.settle();
    (fixture, posted)
}

fn shell(id: &str, params: serde_json::Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

/// A click on a cell selects its glyph; Return on the selection opens it in Edit View.
#[test]
fn a_click_selects_and_return_opens_the_glyph() {
    let (mut fixture, posted) = open(model(ViewMode::Grid, Vec::new()));
    let b = fixture.by_label("B").expect("the B cell");
    fixture.click(&b).settle();
    assert_eq!(*posted.borrow(), vec![shell("fontView.setSelection", json!({"glyphs": ["B"]}))]);

    let (mut fixture, posted) = open(model(ViewMode::Grid, vec![1]));
    let b = fixture.by_label("B").expect("the B cell");
    fixture.click(&b).settle();
    posted.borrow_mut().clear();
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*posted.borrow(), vec![shell("view.editSelectedGlyphs", json!({"glyphs": ["B"]}))]);
}

/// Plus adds a glyph; minus removes the selection and does nothing without one.
#[test]
fn plus_and_minus_post_their_commands() {
    let (mut fixture, posted) = open(model(ViewMode::Grid, Vec::new()));
    let minus = fixture.by_label("Remove Glyph").expect("the minus button");
    fixture.click(&minus).settle();
    assert!(posted.borrow().is_empty(), "{:?}", posted.borrow());
    let plus = fixture.by_label("New Glyph").expect("the plus button");
    fixture.click(&plus).settle();
    assert_eq!(*posted.borrow(), vec![Msg::run("glyph.newGlyph", json!({}))]);

    let (mut fixture, posted) = open(model(ViewMode::Grid, vec![0, 2]));
    let minus = fixture.by_label("Remove Glyph").expect("the minus button");
    fixture.click(&minus).settle();
    assert_eq!(*posted.borrow(), vec![Msg::run("glyph.removeGlyph", json!({"glyphs": ["A", "C"]}))]);
}

/// A press on a List View header sorts by its column.
#[test]
fn a_header_press_sorts_the_list() {
    let (mut fixture, posted) = open(model(ViewMode::List, Vec::new()));
    let header = fixture.by_label("Unicode").expect("the Unicode header");
    fixture.click(&header).settle();
    assert_eq!(*posted.borrow(), vec![shell("fontView.sortByColumn", json!({"column": "unicode", "ascending": true}))]);
}
