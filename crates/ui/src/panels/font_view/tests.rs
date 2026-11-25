//! Font View as the user drives it: what a click, the keys, the plus and minus buttons and a header press post.

use std::cell::RefCell;
use std::rc::Rc;

use craft::ui::contract::Tracked;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::WordSet as _;
use craft::ui::testing::Fixture;
use egui::{Key, Modifiers};
use serde_json::json;

use super::marks::{Badge, MarkTokens, badge_rect};
use super::{Props, show};
use crate::app::font_view::{Column, ViewMode};
use crate::app::models::cell_marks::{CellMarks, UnicodeMark};
use crate::app::models::font_view::{FontViewModel, GlyphRow};
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
            for column in [Column::Unicode, Column::Char] {
                if let Some(cell) = cells.get_mut(column.index()) {
                    *cell = unicode(i);
                }
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
    let mut m = model_of(&TWELVE, mode, vec![at]);
    // A column no field edits: in the list, a click on the selected row's name could start a rename.
    m.columns = vec![Column::Name, Column::Char];
    let (mut fixture, posted) = open(m);
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

    assert_eq!(super::list::status_request(&["A".to_string()], "1"), Msg::run("glyph.setStatus", json!({"glyphs": ["A"], "status": 1})));
    assert_eq!(super::list::status_request(&["A".to_string()], ""), Msg::run("glyph.setStatus", json!({"glyphs": ["A"], "status": null})));
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

/// The fills a Grid View of one cell paints, its texts and the cell's picture rect.
fn painted_cell(marks: CellMarks, cell_size: f64) -> (Vec<(egui::Color32, egui::Rect)>, Vec<String>) {
    let mut grid = model_of(&["A"], ViewMode::Grid, Vec::new());
    grid.cell_size = cell_size;
    if let Some(row) = grid.rows.get_mut(0) {
        row.marks = marks;
    }
    let (mut fixture, _) = open(grid);
    let (mut fills, mut texts) = (Vec::new(), Vec::new());
    for shape in fixture.frame_shapes() {
        match shape {
            egui::Shape::Rect(r) => fills.push((r.fill, r.rect)),
            egui::Shape::Path(p) => fills.push((p.fill, egui::Rect::from_points(&p.points))),
            egui::Shape::Circle(c) => {
                let shown = if c.fill == egui::Color32::TRANSPARENT { c.stroke.color } else { c.fill };
                fills.push((shown, egui::Rect::from_center_size(c.center, egui::Vec2::splat(c.radius * 2.0))));
            }
            egui::Shape::Text(t) => texts.push(t.galley.job.text.clone()),
            _ => {}
        }
    }
    (fills, texts)
}

/// A glyph colour fills the cell, a layer colour alone fills it but for the cut-out, and with both the layer colour
/// takes the right half.
#[test]
fn colour_labels_fill_the_cell() {
    let t = MarkTokens::standard();
    let (glyph, layer) = ([200, 10, 20], [20, 10, 200]);
    let rect_of = |fills: &[(egui::Color32, egui::Rect)], c| fills.iter().find(|(fill, _)| *fill == t.tint(c)).map(|(_, r)| *r);

    let (fills, _) = painted_cell(CellMarks { color: Some(glyph), ..CellMarks::default() }, 64.0);
    let whole = rect_of(&fills, glyph).expect("the glyph's colour");
    assert!((whole.width() - whole.height()).abs() < 1.0, "{whole:?}");

    let (fills, _) = painted_cell(CellMarks { layer_color: Some(layer), ..CellMarks::default() }, 64.0);
    assert_eq!(rect_of(&fills, layer), Some(whole), "the cut-out is inside the cell's bounds");

    let (fills, _) = painted_cell(CellMarks { color: Some(glyph), layer_color: Some(layer), ..CellMarks::default() }, 64.0);
    let (left, right) = (rect_of(&fills, glyph).expect("the glyph's colour"), rect_of(&fills, layer).expect("the layer's colour"));
    assert!((left.width() - right.width()).abs() < 0.1 && left.right() <= right.left() && right.right() == whole.right());
}

/// Each badge paints in its corner and is said beside the glyph's name; a glyph without one has its plain name, and a
/// glyph that does not export is not veiled.
#[test]
fn badges_paint_in_their_corners_and_are_said() {
    let t = MarkTokens::standard();
    let cases = [
        (CellMarks { annotated: true, ..CellMarks::default() }, t.note, "A, Annotation on this master's layer"),
        (CellMarks { custom: true, ..CellMarks::default() }, t.info, "A, Custom glyph properties"),
        (CellMarks { not_exported: true, ..CellMarks::default() }, t.not_exported, "A, Not exported"),
        (CellMarks { out_of_sync: true, ..CellMarks::default() }, t.out_of_sync, "A, Metrics keys out of sync"),
    ];
    let (plain, _) = painted_cell(CellMarks::default(), 64.0);
    for (marks, color, said) in cases {
        let (fills, _) = painted_cell(marks.clone(), 64.0);
        assert!(fills.iter().any(|(c, _)| *c == color), "{said}");
        assert!(plain.iter().all(|(c, _)| *c != color), "{said} without the state");
        let mut grid = model(ViewMode::Grid, Vec::new());
        if let Some(row) = grid.rows.get_mut(0) {
            row.marks = marks;
        }
        let (fixture, _) = open(grid);
        assert!(fixture.by_label(said).is_some(), "{said}");
        assert!(fixture.by_label("B").is_some(), "a glyph without a badge has its plain name");
    }
    let cell = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::splat(64.0));
    let [note, info, ring, warning] =
        [Badge::Annotation, Badge::CustomInfo, Badge::NotExported, Badge::OutOfSync].map(|b| badge_rect(cell, b, &t).center());
    assert!(note.x < ring.x && ring.x < warning.x && note.y < 32.0 && warning.y < 32.0 && info.x < 32.0 && info.y > 32.0);
}

/// The Unicode mark shows with the name: a marker, or the code point while Display Unicode Value is on; a cell too
/// small for names has neither.
#[test]
fn the_unicode_mark_shows_with_the_name() {
    let t = MarkTokens::standard();
    let marked = |unicode: UnicodeMark, size: f64| painted_cell(CellMarks { unicode, ..CellMarks::default() }, size);
    let has_marker = |fills: &[(egui::Color32, egui::Rect)]| fills.iter().any(|(c, r)| *c == t.marker && r.width() <= t.marker_side + 0.1);
    let (fills, texts) = marked(UnicodeMark::Marker, 64.0);
    assert!(has_marker(&fills) && texts.iter().any(|s| s == "A"), "{texts:?}");
    let (fills, texts) = marked(UnicodeMark::Value("0041".into()), 64.0);
    assert!(!has_marker(&fills) && texts.iter().any(|s| s == "0041"), "{texts:?}");
    let (fills, _) = marked(UnicodeMark::None, 64.0);
    assert!(!has_marker(&fills));
    let (fills, texts) = marked(UnicodeMark::Marker, 40.0);
    assert!(!has_marker(&fills) && !texts.iter().any(|s| s == "A"), "{texts:?}");
}

/// A drag from empty grid space selects the cells it touches; a click there empties the selection.
#[test]
fn a_drag_from_empty_space_selects_the_cells_it_touches() {
    let (mut fixture, posted) = open(model_of(&TWELVE, ViewMode::Grid, vec![4]));
    fixture.room(egui::vec2(420.0, 900.0)).settle();
    let (first, last) = (fixture.by_label("A").expect("the first cell"), fixture.by_label("L").expect("the last cell"));
    let start = last.rect.right_bottom() + egui::vec2(4.0, 40.0);
    posted.borrow_mut().clear();
    fixture.drag_at(start, first.center() - start).settle();
    let swept: Vec<String> = match posted.borrow().last() {
        Some(Msg::Shell(c)) if c.id.as_str() == "fontView.setSelection" => {
            let names = c.params["glyphs"].as_array().cloned().unwrap_or_default();
            names.iter().filter_map(|n| n.as_str().map(str::to_string)).collect()
        }
        other => panic!("{other:?}"),
    };
    assert!(swept.contains(&"A".to_string()) && swept.contains(&"L".to_string()), "{swept:?}");
    posted.borrow_mut().clear();
    fixture.click_at(start).settle();
    assert_eq!(posted.borrow().last(), Some(&shell("fontView.setSelection", json!({"glyphs": []}))));
}

/// A row to scroll to is taken once; hidden selected glyphs are counted beside Show All, which clears what hides them.
#[test]
fn a_reveal_is_taken_and_show_all_clears_the_filters() {
    for mode in [ViewMode::Grid, ViewMode::List] {
        let mut m = model_of(&TWELVE, mode, vec![11]);
        m.reveal = Some(11);
        let (_, posted) = open(m);
        assert!(posted.borrow().contains(&Msg::View(ViewMsg::FontViewRevealed)), "{mode:?}");
    }
    let mut m = model(ViewMode::Grid, vec![1]);
    m.visible = vec![0];
    m.hidden = 1;
    let (mut fixture, posted) = open(m);
    let button = fixture.by_label("Show All").expect("Show All");
    posted.borrow_mut().clear();
    fixture.click(&button).settle();
    assert!(posted.borrow().contains(&shell("fontView.setSearch", json!({"query": ""}))), "{:?}", posted.borrow());
}

/// A List View over A to C with B selected, showing `column` beside the names, and what typing `text` into B's cell of
/// it and pressing `key` posts.
fn typed_into(column: Column, value: &str, text: &str, selected: Vec<usize>) -> Vec<Msg> {
    let lead = selected.last().copied().unwrap_or_default();
    let mut m = model(ViewMode::List, selected);
    m.columns = vec![Column::Name, column];
    m.master = Some("m01".into());
    for row in &mut m.rows {
        if let Some(cell) = row.cells.get_mut(column.index()) {
            *cell = value.to_string();
        }
    }
    let name = m.name(lead).unwrap_or_default().to_string();
    let label = format!("{}, {name}", super::list::header(&super::list::ColumnWords::english(), column).as_str());
    let (mut fixture, posted) = open(m);
    let field = fixture.by_label(&label).unwrap_or_else(|| panic!("{label}"));
    posted.borrow_mut().clear();
    fixture.click(&field).key(Key::A, Modifiers::COMMAND).type_text(text).key(Key::Enter, Modifiers::NONE).settle();
    posted.take()
}

/// Each text cell of the lead row runs the command of its property, then Return goes on to the row below; with several
/// rows selected the edit changes all of them.
#[test]
fn list_cells_run_their_commands() {
    let cases = [
        (Column::Unicode, "0042", "0042 0062", "glyph.setUnicodes", json!({"glyphs": ["B"], "unicodes": ["0042", "0062"]})),
        (Column::ProductionName, "", "uni0042", "glyph.setProductionName", json!({"glyphs": ["B"], "productionName": "uni0042"})),
        (Column::Script, "latin", "greek", "glyph.setScript", json!({"glyphs": ["B"], "script": "greek"})),
        (Column::SubCategory, "", "Uppercase", "glyph.setCategory", json!({"glyphs": ["B"], "subCategory": "Uppercase"})),
        (Column::SortName, "B", "Z", "glyph.setSortName", json!({"glyphs": ["B"], "sortName": "Z"})),
        (Column::Note, "", "check", "glyph.setNote", json!({"glyphs": ["B"], "note": "check"})),
        (Column::Tags, "", "one, two", "glyph.setTags", json!({"glyphs": ["B"], "tags": ["one", "two"]})),
        (Column::Components, "", "A", "glyph.setComponents", json!({"glyph": "B", "layer": "m01", "components": "A"})),
        (Column::Lsb, "40", "55", "spacing.setLeftSidebearing", json!({"glyphs": ["B"], "layer": "m01", "value": 55.0})),
        (Column::Width, "600", "=A", "spacing.setWidth", json!({"glyphs": ["B"], "layer": "m01", "value": "=A"})),
        (Column::LeftGroup, "", "H", "kerning.setLeftGroup", json!({"glyphs": ["B"], "group": "H"})),
    ];
    for (column, value, text, id, params) in cases {
        let posted = typed_into(column, value, text, vec![1]);
        assert!(posted.contains(&Msg::run(id, params)), "{column:?}: {posted:?}");
        let next = ViewMsg::FontViewNextCell { glyph: "C".into(), column };
        assert!(posted.contains(&Msg::View(next)), "{column:?}: {posted:?}");
    }
    let posted = typed_into(Column::Note, "", "check", vec![0, 2, 1]);
    assert!(posted.contains(&Msg::run("glyph.setNote", json!({"glyphs": ["A", "C", "B"], "note": "check"}))), "{posted:?}");
    assert!(!posted.iter().any(|m| matches!(m, Msg::View(ViewMsg::FontViewNextCell { .. }))), "{posted:?}");
    let posted = typed_into(Column::Note, "same", "same", vec![1]);
    assert!(!posted.iter().any(|m| matches!(m, Msg::Run(_))), "an unchanged value sets nothing: {posted:?}");
}

/// Only the lead row holds fields, whatever the length of the list; the Exports check box of any row sets the flag; a
/// value the glyph data gives is offered as Automatic, and a pick sets the property.
#[test]
fn list_fields_are_the_lead_rows() {
    let names: Vec<String> = (0..10_000).map(|i| format!("g{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut m = model_of(&refs, ViewMode::List, vec![1]);
    m.columns = vec![Column::Name, Column::Note, Column::Export];
    let (mut fixture, posted) = open(m);
    assert!(fixture.by_label("Note, g1").is_some() && fixture.by_label("Note, g2").is_none());
    let check = fixture.by_label("Exports, g3").expect("the check box");
    posted.borrow_mut().clear();
    fixture.click(&check).settle();
    assert!(posted.borrow().contains(&Msg::run("glyph.setExport", json!({"glyphs": ["g3"], "export": false}))), "{:?}", posted.borrow());

    let mut m = model(ViewMode::List, vec![1]);
    if let Some(row) = m.rows.get_mut(1) {
        row.derived = 1 << Column::Category.index();
        if let Some(cell) = row.cells.get_mut(Column::Category.index()) {
            *cell = "Letter".into();
        }
        if let Some(cell) = row.cells.get_mut(Column::Color.index()) {
            *cell = "Yellow".into();
        }
    }
    let words = super::list::ColumnWords::english();
    let (current, entries) = super::list_edit::pop_up(&m, 1, Column::Category, &words);
    assert_eq!((current.as_str(), entries.len()), ("", 9));
    assert_eq!(super::list_edit::pop_up(&m, 1, Column::Color, &words).0, "3");
    let set = |column, value| super::list_edit::command(&m, 1, column, value);
    assert_eq!(set(Column::Category, "Symbol"), Some(Msg::run("glyph.setCategory", json!({"glyphs": ["B"], "category": "Symbol"}))));
    assert_eq!(set(Column::Color, ""), Some(Msg::run("glyph.setColorLabel", json!({"glyphs": ["B"], "color": null}))));
    assert_eq!(set(Column::Case, "noCase"), Some(Msg::run("glyph.setCase", json!({"glyphs": ["B"], "case": "noCase"}))));
}
