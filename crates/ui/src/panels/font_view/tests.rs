//! Font View as the user drives it: what a click, the keys, the plus and minus buttons and a header press post.

use std::cell::RefCell;
use std::rc::Rc;

use craft::ui::contract::Tracked;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use egui::{Key, Modifiers};
use serde_json::json;

use super::{Props, show};
use crate::app::font_view::{Column, ViewMode};
use crate::app::models::font_view::{FontViewModel, GlyphRow};
use crate::app::models::incompatibility::CellCompat;
use crate::app::{Msg, ViewMsg};

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

/// A cell drawn before its glyph was checked asks for the answer; one whose layers cannot interpolate says so beside its
/// red corner.
#[test]
fn an_unchecked_cell_asks_and_an_incompatible_one_says_so() {
    let mut m = model(ViewMode::Grid, Vec::new());
    let states = [CellCompat::Fine, CellCompat::Unchecked, CellCompat::Differs("Light and Bold differ: 1 shapes against 2".into())];
    for (row, state) in m.rows.iter_mut().zip(states) {
        row.compat = state;
    }
    let (fixture, posted) = open(m);
    assert!(posted.borrow().contains(&Msg::View(ViewMsg::UncheckedCells(vec!["B".into()]))));
    assert!(fixture.by_label("C, incompatible layers").is_some());
    assert!(fixture.by_label("A").is_some(), "a cell that interpolates keeps its name");
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

/// The font's statuses as the model holds them: Draft (code 0, red) and Final (code 1, green).
fn with_statuses(mut model: FontViewModel) -> FontViewModel {
    use crate::app::models::font_view::StatusChoice;
    model.statuses = vec![
        StatusChoice { code: 0, name: "Draft".into(), color: [0xd0, 0x20, 0x20, 0xff] },
        StatusChoice { code: 1, name: "Final".into(), color: [0x20, 0xa0, 0x40, 0xff] },
    ];
    if let Some(row) = model.rows.get_mut(0) {
        row.status = Some(0);
        row.status_color = Some([0xd0, 0x20, 0x20, 0xff]);
    }
    model
}

/// The filled rectangles of one frame, by colour.
fn filled(fixture: &mut Fixture) -> Vec<egui::Color32> {
    fixture
        .frame_shapes()
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Rect(r) => Some(r.fill),
            _ => None,
        })
        .collect()
}

/// A cell whose glyph is in a status draws a bar in the status colour; a cell in none draws no such bar.
#[test]
fn a_cell_with_a_status_draws_its_bar() {
    let red = egui::Color32::from_rgba_unmultiplied(0xd0, 0x20, 0x20, 0xff);
    let (mut fixture, _) = open(with_statuses(model(ViewMode::Grid, Vec::new())));
    assert_eq!(filled(&mut fixture).iter().filter(|c| **c == red).count(), 1, "A's cell has the Draft bar");
    let (mut fixture, _) = open(model(ViewMode::Grid, Vec::new()));
    assert!(!filled(&mut fixture).contains(&red), "no glyph is in a status");
    assert!(super::cells::status_bar(egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(64.0, 64.0))).height() >= 2.0);
}

/// List View's Status cell shows the glyph's status as a pop-up, and its pick runs `glyph.setStatus` on the row's
/// glyph: the same change as the command; No Status clears it.
#[test]
fn the_status_column_edit_equals_set_status() {
    let mut list = with_statuses(model(ViewMode::List, Vec::new()));
    list.columns = vec![Column::Name, Column::Status];
    let (mut fixture, _) = open(list);
    let texts: Vec<String> = fixture
        .frame_shapes()
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
            _ => None,
        })
        .collect();
    assert!(texts.iter().any(|t| t == "Draft"), "A's cell shows its status: {texts:?}");

    assert_eq!(super::list::status_request("A", "1"), Msg::run("glyph.setStatus", json!({"glyphs": ["A"], "status": 1})));
    assert_eq!(super::list::status_request("A", ""), Msg::run("glyph.setStatus", json!({"glyphs": ["A"], "status": null})));
}

/// With a strike shown, a cell with a bitmap draws it as a texture, and a glyph without one still shows its name even
/// below the label size.
#[test]
fn a_strike_cell_draws_its_bitmap_and_an_empty_cell_its_name() {
    use std::sync::Arc;

    use crate::app::models::font_view::{StrikeCell, StrikeShown};

    let strike_model = |bitmap: bool| {
        let mut grid = model(ViewMode::Grid, Vec::new());
        grid.cell_size = 32.0;
        grid.strike = Some(StrikeShown { ppem: 12, depth: 1, ascent: 10, descent: 2, magnification: 1 });
        if bitmap && let Some(row) = grid.rows.get_mut(0) {
            // A 2 × 2 black square, one byte per row at one bit per pixel.
            row.bitmap = Some(Arc::new(StrikeCell { id: 1, width: 2, height: 2, left: 0, bottom: 0, advance: 3, depth: 1, data: vec![0xc0, 0xc0] }));
        }
        grid
    };
    let textured =
        |shapes: &[egui::Shape]| shapes.iter().filter(|s| matches!(s, egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default())).count();
    let (mut fixture, _) = open(strike_model(false));
    let without = textured(&fixture.frame_shapes());
    let (mut fixture, _) = open(strike_model(true));
    let shapes = fixture.frame_shapes();
    assert_eq!(textured(&shapes), without + 1, "A's bitmap is one textured mesh");
    let texts: Vec<String> = shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
            _ => None,
        })
        .collect();
    assert!(texts.iter().any(|t| t == "B"), "B has no bitmap and shows its name: {texts:?}");
}
