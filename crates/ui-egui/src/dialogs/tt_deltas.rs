//! The Delta table: one column per master or instance, one row per pixel size, one button per
//! cell. A click on a button gives that cell its next value: none, then one pixel up, then one
//! pixel down, then none again. The table changes nothing itself: it hands the click to its
//! owner, which runs the command that stores the cell (`hint.tt.setDeltas` for the selected
//! Delta instructions of the TrueType Instructor's Info box), and shows the cells it is given, so
//! after the command runs the next frame shows the stored value.
//!
//! The same table serves the delta buttons of TT zones and TT stems: their owner builds the
//! columns (the masters) and turns a [`Click`] into its own command with [`DeltaTable::cells_param`].
//!
//! Done, Escape or a click outside closes the table.
//!
//! Row: `dialog.ttDeltas` (M10.27).

use std::collections::BTreeMap;

use egui::{Align2, FontId, Id, RichText, Sense};
use serde_json::{Map, Value, json};

use crate::panels::widgets;
use crate::theme::Tokens;

/// The first and last pixel size the table offers (guess on `dialog.ttDeltas`: the range of the
/// hinting waterfall).
pub const FIRST_PPM: u16 = 8;
pub const LAST_PPM: u16 = 80;
/// Most columns a table shows: the model's limit on delta columns
/// (`fontcraft_hinting::MAX_DELTA_COLUMNS`).
pub const MAX_COLUMNS: usize = 64;
/// Rows visible before the table scrolls.
const VISIBLE_ROWS: f32 = 16.0;

/// What one cell holds, as the table shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cell {
    #[default]
    Empty,
    /// One pixel up (or right).
    Up,
    /// One pixel down (or left).
    Down,
    /// The table stands for several items whose cells differ.
    Mixed,
}

impl Cell {
    /// The value a click gives: none, up, down, none again. A mixed cell starts at up.
    pub fn next(self) -> Cell {
        match self {
            Cell::Empty | Cell::Mixed => Cell::Up,
            Cell::Up => Cell::Down,
            Cell::Down => Cell::Empty,
        }
    }

    /// The value as the delta commands take it.
    pub fn param(self) -> &'static str {
        match self {
            Cell::Up => "up",
            Cell::Down => "down",
            Cell::Empty | Cell::Mixed => "none",
        }
    }

    fn text(self) -> &'static str {
        match self {
            Cell::Empty => "",
            Cell::Up => "↑",
            Cell::Down => "↓",
            Cell::Mixed => "–",
        }
    }

    fn hover(self) -> &'static str {
        match self {
            Cell::Empty => "No delta: click for one pixel up",
            Cell::Up => "One pixel up: click for one pixel down",
            Cell::Down => "One pixel down: click to clear",
            Cell::Mixed => "The selected items differ here: click for one pixel up",
        }
    }
}

/// One column: the key the delta commands take (a master or instance name, or a stored column
/// key in braces) and its caption.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub key: String,
    pub label: String,
}

/// The table's contents.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeltaTable {
    /// Shown above the grid.
    pub title: String,
    /// At most [`MAX_COLUMNS`] are drawn.
    pub columns: Vec<Column>,
    /// The cells that are not empty, by (column index, PPM).
    pub cells: BTreeMap<(usize, u16), Cell>,
}

/// A click on one cell, with the value the cell takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Click {
    pub column: usize,
    pub ppm: u16,
    pub value: Cell,
}

/// How the person left the table this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Click(Click),
    Close,
}

impl DeltaTable {
    pub fn cell(&self, column: usize, ppm: u16) -> Cell {
        self.cells.get(&(column, ppm)).copied().unwrap_or_default()
    }

    /// The `cells` parameter of the delta commands for `click`: `{column key: {ppm: value}}`;
    /// `None` when the click names no column of the table.
    pub fn cells_param(&self, click: &Click) -> Option<Value> {
        let column = self.columns.get(click.column)?;
        let mut sizes = Map::new();
        sizes.insert(click.ppm.to_string(), json!(click.value.param()));
        let mut cells = Map::new();
        cells.insert(column.key.clone(), Value::Object(sizes));
        Some(Value::Object(cells))
    }
}

/// The id of the table's modal area.
pub fn modal_id() -> Id {
    widgets::id(("dialog.ttDeltas", "modal"))
}

/// The id of the button of `column` at `ppm`.
pub fn cell_id(column: usize, ppm: u16) -> Id {
    widgets::id(("dialog.ttDeltas", "cell", column, ppm))
}

/// The id of the Done button.
pub fn done_id() -> Id {
    widgets::id(("dialog.ttDeltas", "done"))
}

/// Draws the table over everything else. Returns the click of this frame, or that the table
/// closed.
pub fn show(ctx: &egui::Context, tokens: &Tokens, table: &DeltaTable) -> Option<Outcome> {
    let mut outcome = None;
    let width = tokens.field_width;
    let height = tokens.list_row_height;
    let columns: Vec<&Column> = table.columns.iter().take(MAX_COLUMNS).collect();
    let modal = egui::Modal::new(modal_id()).show(ctx, |ui| {
        ui.label(RichText::new(&table.title).strong().size(tokens.dialog_title_size));
        ui.add_space(tokens.dialog_spacing);
        if columns.is_empty() {
            ui.label(RichText::new("The font has no master or instance to set deltas for.").color(tokens.text_dim));
        }
        ui.horizontal(|ui| {
            ui.add_sized([width, height], egui::Label::new(RichText::new("PPM").color(tokens.text_dim)));
            for column in &columns {
                ui.add_sized([width, height], egui::Label::new(RichText::new(&column.label).color(tokens.text_dim)).truncate())
                    .on_hover_text(&column.label);
            }
        });
        egui::ScrollArea::vertical().id_salt(("dialog.ttDeltas", "rows")).max_height(height * VISIBLE_ROWS).show(ui, |ui| {
            for ppm in FIRST_PPM..=LAST_PPM {
                ui.horizontal(|ui| {
                    ui.add_sized([width, height], egui::Label::new(RichText::new(ppm.to_string()).color(tokens.text_dim)));
                    for (index, _) in columns.iter().enumerate() {
                        let cell = table.cell(index, ppm);
                        if cell_button(ui, tokens, cell_id(index, ppm), cell, egui::vec2(width, height)) {
                            outcome = Some(Outcome::Click(Click { column: index, ppm, value: cell.next() }));
                        }
                    }
                });
            }
        });
        ui.add_space(tokens.dialog_spacing);
        if widgets::button(ui, done_id(), "Done", "Close the Delta table", true, false) {
            outcome = Some(Outcome::Close);
        }
    });
    if outcome.is_none() && modal.should_close() {
        outcome = Some(Outcome::Close);
    }
    outcome
}

/// One cell: filled when it holds a delta, its arrow in the middle.
fn cell_button(ui: &mut egui::Ui, tokens: &Tokens, id: Id, cell: Cell, size: egui::Vec2) -> bool {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = f32::from(tokens.radius);
        let fill = match cell {
            Cell::Empty if response.hovered() => Some(ui.visuals().widgets.hovered.weak_bg_fill),
            Cell::Empty => None,
            Cell::Up | Cell::Down | Cell::Mixed => Some(tokens.row_selected),
        };
        if let Some(fill) = fill {
            painter.rect_filled(rect.shrink(1.0), radius, fill);
        }
        painter.rect_stroke(rect.shrink(1.0), radius, egui::Stroke::new(1.0, tokens.border), egui::StrokeKind::Inside);
        let color = if cell == Cell::Empty { tokens.text } else { tokens.selected_text };
        painter.text(rect.center(), Align2::CENTER_CENTER, cell.text(), FontId::proportional(rect.height() * 0.7), color);
    }
    response.on_hover_text(cell.hover()).clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_cycles_none_up_down() {
        assert_eq!(Cell::Empty.next(), Cell::Up);
        assert_eq!(Cell::Up.next(), Cell::Down);
        assert_eq!(Cell::Down.next(), Cell::Empty);
        assert_eq!(Cell::Mixed.next(), Cell::Up);
    }

    #[test]
    fn the_cells_parameter_names_the_column_key_and_the_size() {
        let table = DeltaTable {
            title: "Deltas".into(),
            columns: vec![Column { key: "{700, 0, 0, 0, 0, 0}".into(), label: "Bold".into() }],
            cells: BTreeMap::new(),
        };
        let click = Click { column: 0, ppm: 12, value: Cell::Down };
        assert_eq!(table.cells_param(&click), Some(json!({"{700, 0, 0, 0, 0, 0}": {"12": "down"}})));
        assert_eq!(table.cells_param(&Click { column: 1, ppm: 12, value: Cell::Up }), None);
    }
}
