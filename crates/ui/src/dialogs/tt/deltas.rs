//! The Delta table: one column per master or instance, one row per pixel size, one button per cell. A click gives
//! the cell its next value: none, then one pixel up, then one pixel down, then none again. The table changes nothing
//! itself: it hands the click to its owner, which runs the command that stores the cell (`hint.tt.setZone {deltas}`,
//! `hint.tt.setStemDefinition {deltas}`, `hint.tt.setDeltas`), and shows the cells it is given, so the next pass shows
//! the stored value.
//!
//! The owner shows [`view`] in place of its own body while the table is open; Done hands back [`Outcome::Close`].
//!
//! Row: `dialog.ttDeltas` (M10.27).

use std::collections::BTreeMap;

use craft::ui::buttons::Button;
use craft::ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::runtime::env::{Env, Label};
use egui::Ui;
use serde_json::{Map, Value, json};

/// The first and last pixel size the table offers (guess on `dialog.ttDeltas`: the range of the hinting waterfall).
pub const FIRST_PPM: u16 = 8;
pub const LAST_PPM: u16 = 80;
/// Most columns a table shows: the model's limit on delta columns (`fontcraft_hinting::MAX_DELTA_COLUMNS`).
pub const MAX_COLUMNS: usize = 64;

craft::ui::words! {
    /// The words of the Delta table.
    pub struct DeltaWords {
        /// The caption of the pixel-size column.
        ppm: "fontcraftTtDeltas.ppm" = "PPM";
        /// Shown when the table has no column.
        no_columns: "fontcraftTtDeltas.noColumns" = "The font has no master or instance to set deltas for.";
        /// The button that closes the table.
        done: "fontcraftTtDeltas.done" = "Done";
        /// The tooltip of the Done button.
        done_tip: "fontcraftTtDeltas.doneTip" = "Close the Delta table";
        /// An empty cell's tooltip.
        empty_tip: "fontcraftTtDeltas.emptyTip" = "No delta: click for one pixel up";
        /// An up cell's tooltip.
        up_tip: "fontcraftTtDeltas.upTip" = "One pixel up: click for one pixel down";
        /// A down cell's tooltip.
        down_tip: "fontcraftTtDeltas.downTip" = "One pixel down: click to clear";
        /// A mixed cell's tooltip.
        mixed_tip: "fontcraftTtDeltas.mixedTip" = "The selected items differ here: click for one pixel up";
        /// An up cell's mark.
        up: "fontcraftTtDeltas.up" = "↑";
        /// A down cell's mark.
        down: "fontcraftTtDeltas.down" = "↓";
        /// A mixed cell's mark.
        mixed: "fontcraftTtDeltas.mixed" = "–";
        /// An empty cell's mark.
        empty: "fontcraftTtDeltas.empty" = "·";
    }
}

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

    fn mark(self, words: &DeltaWords) -> (Label, Label) {
        match self {
            Cell::Empty => (words.empty.clone(), words.empty_tip.clone()),
            Cell::Up => (words.up.clone(), words.up_tip.clone()),
            Cell::Down => (words.down.clone(), words.down_tip.clone()),
            Cell::Mixed => (words.mixed.clone(), words.mixed_tip.clone()),
        }
    }
}

/// One column: the key the delta commands take (a master id or instance name, or a stored column key) and its
/// caption.
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
    /// At most [`MAX_COLUMNS`] are shown.
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

/// How the person left the table in a pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Click(Click),
    Close,
}

impl DeltaTable {
    pub fn cell(&self, column: usize, ppm: u16) -> Cell {
        self.cells.get(&(column, ppm)).copied().unwrap_or_default()
    }

    /// The `cells` parameter of the delta commands for `click`: `{column key: {ppm: value}}`; `None` when the click
    /// names no column of the table.
    pub fn cells_param(&self, click: &Click) -> Option<Value> {
        let column = self.columns.get(click.column)?;
        let mut sizes = Map::new();
        sizes.insert(click.ppm.to_string(), json!(click.value.param()));
        let mut cells = Map::new();
        cells.insert(column.key.clone(), Value::Object(sizes));
        Some(Value::Object(cells))
    }
}

/// The id of the button of the column at `column` and `ppm`.
pub fn cell_id(column: usize, ppm: u16) -> egui::Id {
    egui::Id::unique(("fontcraftTtDeltas.cell", column, ppm))
}

/// The id of the Done button.
pub fn done_id() -> egui::Id {
    egui::Id::unique("fontcraftTtDeltas.done")
}

/// Shows the table; returns the click of this pass, or that Done closed it.
pub fn view(ui: &mut Ui, table: &DeltaTable) -> Option<Outcome> {
    let words = Env::of(ui).words::<DeltaWords>();
    let columns: Vec<&Column> = table.columns.iter().take(MAX_COLUMNS).collect();
    let mut outcome = None;
    Stack::new().show(ui, |ui, _| {
        Text::new(table.title.clone()).show(ui);
        if columns.is_empty() {
            Text::new(words.no_columns.clone()).shade(Shade::Muted).show(ui);
        }
        Row::new().show(ui, |ui, _| {
            Text::new(words.ppm.clone()).shade(Shade::Muted).show(ui);
            for column in &columns {
                Text::new(column.label.clone()).shade(Shade::Muted).tooltip(column.label.clone()).show(ui);
            }
        });
        Stack::new().scroll(craft::ui::contract::Scroll::vertical()).show(ui, |ui, _| {
            for ppm in FIRST_PPM..=LAST_PPM {
                Row::new().show(ui, |ui, _| {
                    Text::new(ppm.to_string()).shade(Shade::Muted).show(ui);
                    for index in 0..columns.len() {
                        let cell = table.cell(index, ppm);
                        let (mark, tip) = cell.mark(&words);
                        if Button::new(mark).id(cell_id(index, ppm)).tooltip(tip).show(ui).clicked() {
                            outcome = Some(Outcome::Click(Click { column: index, ppm, value: cell.next() }));
                        }
                    }
                });
            }
        });
        if Button::new(words.done.clone()).id(done_id()).tooltip(words.done_tip.clone()).show(ui).clicked() {
            outcome = Some(Outcome::Close);
        }
    });
    outcome
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
