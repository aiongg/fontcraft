//! Font View as the user drives it: what a click, the keys, the plus and minus buttons and a header press post.

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
    model_of(&["A", "B", "C"], mode, selected)
}

/// A Font View over the glyphs `names`, with the rows `selected` selected and the last of them the anchor.
fn model_of(names: &[&str], mode: ViewMode, selected: Vec<usize>) -> FontViewModel {
    let rows: Vec<GlyphRow> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let mut cells = vec![String::new(); Column::ALL.len()];
            if let Some(cell) = cells.get_mut(Column::Name.index()) {
                *cell = (*name).to_string();
            }
            if let Some(cell) = cells.get_mut(Column::Unicode.index()) {
                *cell = unicode(i);
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

/// The Unicode column's text of row `i`.
fn unicode(i: usize) -> String {
    format!("{:04X}", 0x41 + i)
}

/// A fixture over `model` and the messages it posted.
fn open(model: FontViewModel) -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
    let model = Tracked::new(model);
    let images = Tracked::new(crate::canvas::images::cells::CellImages::default());
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, images: &images, sent: &mut sent });
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
    assert_eq!(*posted.borrow(), vec![remove_dialog(&["A", "C"])], "minus asks first, in Remove Glyph's dialog");
}

fn remove_dialog(glyphs: &[&str]) -> Msg {
    shell("dialog.open", json!({"command": "glyph.removeGlyph", "params": {"glyphs": glyphs}}))
}

/// Twelve glyphs, A to L: more than one grid row at the fixture's width.
const TWELVE: [&str; 12] = ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L"];

/// What one key press posts in a Font View over [`TWELVE`] in `mode` with the glyph `at` selected and the keyboard
/// (a click on its cell gives it the keyboard and changes nothing).
fn after_key(mode: ViewMode, at: usize, key: Key, mods: Modifiers) -> Vec<Msg> {
    let (mut fixture, posted) = open(model_of(&TWELVE, mode, vec![at]));
    // In the list, a click on the selected row's name could start a rename: the Unicode cell is clicked instead.
    let label = match mode {
        ViewMode::List => unicode(at),
        _ => TWELVE.get(at).copied().unwrap_or_default().to_string(),
    };
    let cell = fixture.by_label(&label).expect("the selected glyph's cell");
    fixture.click(&cell).settle();
    posted.borrow_mut().clear();
    fixture.key(key, mods).settle();
    posted.take()
}

fn selects(glyphs: &[&str]) -> Vec<Msg> {
    vec![shell("fontView.setSelection", json!({ "glyphs": glyphs }))]
}

/// The grid's arrows move by one cell, Up and Down by a row, and Right at a row's end goes on to the next row; at the
/// first and last glyph the selection stays.
#[test]
fn grid_arrows_move_by_a_cell_and_a_row() {
    let grid = ViewMode::Grid;
    assert_eq!(after_key(grid, 1, Key::ArrowRight, Modifiers::NONE), selects(&["C"]));
    assert_eq!(after_key(grid, 1, Key::ArrowLeft, Modifiers::NONE), selects(&["A"]));
    assert!(after_key(grid, 0, Key::ArrowLeft, Modifiers::NONE).is_empty(), "nothing comes before the first glyph");
    assert!(after_key(grid, 11, Key::ArrowRight, Modifiers::NONE).is_empty(), "nothing comes after the last glyph");
    assert!(after_key(grid, 0, Key::ArrowUp, Modifiers::NONE).is_empty(), "nothing is above the first row");

    let down = after_key(grid, 0, Key::ArrowDown, Modifiers::NONE);
    let per_row = TWELVE.iter().position(|g| down == selects(&[*g])).expect("Down selects one glyph");
    assert!(per_row > 1 && per_row < TWELVE.len(), "Down moves by a whole row of more than one cell: {per_row}");
    let below = TWELVE.get(per_row).copied().unwrap_or_default();
    assert_eq!(after_key(grid, per_row, Key::ArrowUp, Modifiers::NONE), selects(&["A"]), "Up comes back a row");
    assert_eq!(after_key(grid, per_row - 1, Key::ArrowRight, Modifiers::NONE), selects(&[below]), "a row's end goes on");
}

/// In the list, Up and Down move by one row; in both views Shift with an arrow extends from the anchor, Home and End
/// go to the first and last glyph, and Delete asks to remove the selection.
#[test]
fn list_arrows_shift_home_end_and_delete() {
    let list = ViewMode::List;
    assert_eq!(after_key(list, 1, Key::ArrowDown, Modifiers::NONE), selects(&["C"]));
    assert_eq!(after_key(list, 1, Key::ArrowUp, Modifiers::NONE), selects(&["A"]));
    assert!(after_key(list, 0, Key::ArrowUp, Modifiers::NONE).is_empty(), "nothing is above the first row");
    assert!(after_key(list, 11, Key::ArrowDown, Modifiers::NONE).is_empty(), "nothing is below the last row");
    for mode in [ViewMode::Grid, list] {
        assert_eq!(after_key(mode, 3, Key::Home, Modifiers::NONE), selects(&["A"]), "{mode:?}: Home");
        assert_eq!(after_key(mode, 3, Key::End, Modifiers::NONE), selects(&["L"]), "{mode:?}: End");
        assert_eq!(after_key(mode, 3, Key::Delete, Modifiers::NONE), vec![remove_dialog(&["D"])], "{mode:?}: Delete");
        assert_eq!(after_key(mode, 3, Key::Enter, Modifiers::NONE), vec![shell("view.editSelectedGlyphs", json!({"glyphs": ["D"]}))]);
    }
    // The lead goes last in the posted list: it is where the next Shift press extends from.
    assert_eq!(after_key(ViewMode::Grid, 1, Key::ArrowRight, Modifiers::SHIFT), selects(&["B", "C"]));
    assert_eq!(after_key(ViewMode::Grid, 1, Key::ArrowLeft, Modifiers::SHIFT), selects(&["B", "A"]));
    assert_eq!(after_key(list, 1, Key::ArrowDown, Modifiers::SHIFT), selects(&["B", "C"]));
}

/// End in a font of many glyphs scrolls the grid to the last glyph.
#[test]
fn end_scrolls_the_last_glyph_into_view() {
    let names: Vec<String> = (0..400).map(|i| format!("g{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (mut fixture, posted) = open(model_of(&refs, ViewMode::Grid, vec![0]));
    let first = fixture.by_label("g0").expect("the first cell");
    fixture.click(&first).settle();
    assert!(fixture.by_label("g399").and_then(|t| fixture.visible(&t)).is_none(), "the last glyph starts out of view");
    fixture.key(Key::End, Modifiers::NONE).settle();
    assert_eq!(posted.borrow().last(), selects(&["g399"]).first());
    assert!(fixture.by_label("g399").and_then(|t| fixture.visible(&t)).is_some(), "the last glyph is in view");
}

/// A press on a List View header sorts by its column.
#[test]
fn a_header_press_sorts_the_list() {
    let (mut fixture, posted) = open(model(ViewMode::List, Vec::new()));
    let header = fixture.by_label("Unicode").expect("the Unicode header");
    fixture.click(&header).settle();
    assert_eq!(*posted.borrow(), vec![shell("fontView.sortByColumn", json!({"column": "unicode", "ascending": true}))]);
}
