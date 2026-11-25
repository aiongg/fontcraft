//! The list of the report windows: what a save leaves out, what an open could not read, what an export reported.

use std::borrow::Cow;

use craft::behavior::{ColumnKey, Selection, keyed};
use craft::ui::contract::{Bind, CollectionEvent, ColumnSpec, SelectMode, ShowCollection as _};
use craft::ui::list::ListRow;
use craft::ui::runtime::env::Label;
use craft::ui::table::Table;
use egui::Ui;

/// Rows a report list shows before it scrolls.
pub const LIST_ROWS: u16 = 12;

/// One row: the text of the first column, which names the row, and of the others.
pub struct Line {
    pub text: String,
    pub cells: Vec<(ColumnKey, String)>,
}

impl Line {
    pub fn new(text: impl Into<String>) -> Line {
        Line { text: text.into(), cells: Vec::new() }
    }

    pub fn cell(mut self, column: ColumnKey, text: impl Into<String>) -> Line {
        self.cells.push((column, text.into()));
        self
    }
}

/// Shows `lines` under `columns`; `selection` holds the chosen row when rows can be chosen. Returns the row that was
/// double-pressed or answered with Enter.
pub fn show(
    ui: &mut Ui,
    name: impl Into<Label>,
    columns: Vec<ColumnSpec>,
    lines: &[Line],
    selection: Option<&mut Selection<usize>>,
) -> Option<usize> {
    let positions: Vec<usize> = (0..lines.len()).collect();
    let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(lines.get(*at).map_or_else(String::new, |l| l.text.clone())));
    let mut table = Table::<usize>::new(name, columns).max_rows(LIST_ROWS);
    if let Some(selection) = selection {
        table = table.select(SelectMode::Single).selection(Bind::bound(selection));
    }
    let out = table.show_items(ui, &source, &mut |at, icx| {
        let Some(line) = lines.get(*at) else { return ListRow::new("") };
        line.cells.iter().fold(ListRow::new(line.text.clone()).cells(icx.cells()), |row, (column, text)| row.cell(column.clone(), text.clone()))
    });
    for event in &out.events {
        if let CollectionEvent::Activate(at) = event {
            return Some(*at);
        }
    }
    None
}
