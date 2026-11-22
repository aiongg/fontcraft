//! List View (`fontView.list`): a row per shown glyph and a column per glyph property. A header press sorts
//! (`fontView.sortByColumn`); the header menu shows and hides columns and a header drag moves one
//! (`fontView.setColumns`); the Name cell renames the glyph, and the Status cell is a pop-up of the font's statuses
//! that runs `glyph.setStatus` on the row's glyph (No Status clears the stored one). The table lays out only the rows
//! in view.

use std::cell::RefCell;

use craft::behavior::{ColumnKey, ColumnWidth};
use craft::ui::contract::{Bind, CollectionEvent, ColumnEvent, ColumnSort, ColumnSpec, Entry, Props as _, ShowCollection as _};
use craft::ui::list::ListRow;
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::table::Table;
use egui::Ui;
use serde_json::json;

use super::grid::Shown;
use super::{FontViewWords, keys};
use crate::app::Msg;
use crate::app::font_view::Column;
use crate::app::models::font_view::FontViewModel;

craft::ui::words! {
    /// List View's column headers.
    pub struct ColumnWords {
        id: "fontcraftFontViewColumn.id" = "ID";
        char: "fontcraftFontViewColumn.char" = "Char";
        name: "fontcraftFontViewColumn.name" = "Name";
        unicode: "fontcraftFontViewColumn.unicode" = "Unicode";
        production_name: "fontcraftFontViewColumn.productionName" = "Production Name";
        script: "fontcraftFontViewColumn.script" = "Script";
        category: "fontcraftFontViewColumn.category" = "Category";
        sub_category: "fontcraftFontViewColumn.subCategory" = "Subcategory";
        case: "fontcraftFontViewColumn.case" = "Case";
        direction: "fontcraftFontViewColumn.direction" = "Direction";
        export: "fontcraftFontViewColumn.export" = "Exports";
        color: "fontcraftFontViewColumn.color" = "Color";
        tags: "fontcraftFontViewColumn.tags" = "Tags";
        sort_name: "fontcraftFontViewColumn.sortName" = "Sort Name";
        note: "fontcraftFontViewColumn.note" = "Note";
        components: "fontcraftFontViewColumn.components" = "Components";
        last_changed: "fontcraftFontViewColumn.lastChanged" = "Last Changed";
        lsb: "fontcraftFontViewColumn.lsb" = "LSB";
        rsb: "fontcraftFontViewColumn.rsb" = "RSB";
        width: "fontcraftFontViewColumn.width" = "Width";
        left_group: "fontcraftFontViewColumn.leftGroup" = "Left Group";
        right_group: "fontcraftFontViewColumn.rightGroup" = "Right Group";
        status: "fontcraftFontViewColumn.status" = "Status";
        /// The Status pop-up's entry that clears the stored status.
        no_status: "fontcraftFontViewColumn.noStatus" = "No Status";
    }
}

/// The header of `column`.
pub fn header(words: &ColumnWords, column: Column) -> Label {
    let w = words;
    match column {
        Column::Id => w.id.clone(),
        Column::Char => w.char.clone(),
        Column::Name => w.name.clone(),
        Column::Unicode => w.unicode.clone(),
        Column::ProductionName => w.production_name.clone(),
        Column::Script => w.script.clone(),
        Column::Category => w.category.clone(),
        Column::SubCategory => w.sub_category.clone(),
        Column::Case => w.case.clone(),
        Column::Direction => w.direction.clone(),
        Column::Export => w.export.clone(),
        Column::Color => w.color.clone(),
        Column::Tags => w.tags.clone(),
        Column::SortName => w.sort_name.clone(),
        Column::Note => w.note.clone(),
        Column::Components => w.components.clone(),
        Column::LastChanged => w.last_changed.clone(),
        Column::Lsb => w.lsb.clone(),
        Column::Rsb => w.rsb.clone(),
        Column::Width => w.width.clone(),
        Column::LeftGroup => w.left_group.clone(),
        Column::RightGroup => w.right_group.clone(),
        Column::Status => w.status.clone(),
    }
}

fn key(column: Column) -> ColumnKey {
    ColumnKey::new(column.id())
}

fn column_of(key: &ColumnKey) -> Option<Column> {
    key.as_str().and_then(Column::parse)
}

pub fn show(ui: &mut Ui, model: &FontViewModel, sent: &mut Vec<Msg>) {
    let env = Env::of(ui);
    let words = env.words::<FontViewWords>();
    let headers = env.words::<ColumnWords>();
    let first = model.columns.first().copied().unwrap_or(Column::Name);
    let specs: Vec<ColumnSpec> = model
        .columns
        .iter()
        .map(|&c| {
            let spec = ColumnSpec::new(key(c), header(&headers, c), ColumnWidth::Content).sortable().resizable().movable();
            let spec = if c == Column::Name { spec } else { spec.hideable() };
            // The first column's text is the row's own, which starts at the leading edge; its header does too.
            if c.is_numeric() && c != first { spec.numeric() } else { spec }
        })
        .collect();
    let sort = model.sort.map(|s| ColumnSort::new(key(s.column), !s.ascending));
    let mut selection = keys::selection(model);
    let source = Shown::new(model);
    let statuses = status_entries(model, &headers);
    // The Status pop-up a row picked from: the glyph and the entry's value.
    let picked: RefCell<Option<(String, String)>> = RefCell::new(None);
    let out = Table::<usize>::new(words.glyphs.clone(), specs)
        .empty(words.no_glyphs.clone())
        .sort(sort)
        .striped(true)
        .type_ahead(true)
        .deletable(true)
        .rename_column(key(Column::Name))
        .selection(Bind::bound(&mut selection))
        .show_items(ui, &source, &mut |row, icx| {
            let cells = model.rows.get(*row).map(|r| r.cells.as_slice()).unwrap_or_default();
            let text = |c: Column| cells.get(c.index()).cloned().unwrap_or_default();
            let mut list_row = ListRow::new(text(first)).cells(icx.cells()).striped(icx.row.striped);
            for &c in model.columns.iter().skip(1) {
                if c == Column::Status
                    && !statuses.is_empty()
                    && let Some(glyph) = model.rows.get(*row)
                {
                    let current = glyph.status.map(|code| code.to_string()).unwrap_or_default();
                    let name = glyph.name.clone();
                    let picked = &picked;
                    let field = Select::new(headers.status.clone(), Bind::given(current.clone()), &statuses)
                        .id_salt(("fontcraftFontView.status", glyph.name.as_str()));
                    list_row = list_row.cell_field(key(c), field, move |out| {
                        if let Some(value) = out.events.committed()
                            && *value != current
                        {
                            *picked.borrow_mut() = Some((name, value.clone()));
                        }
                    });
                    continue;
                }
                list_row = list_row.cell(key(c), text(c));
            }
            list_row
        });
    if let Some((glyph, value)) = picked.into_inner() {
        sent.push(status_request(&glyph, &value));
    }
    keys::requests(ui.ctx(), model, &selection, &out.events, sent);
    for event in &out.events {
        if let CollectionEvent::Column(change) = event {
            column_request(model, change, sent);
        }
    }
}

/// The Status pop-up's entries: No Status, then each status by its code; none when the font defines no status.
fn status_entries(model: &FontViewModel, words: &ColumnWords) -> Vec<Entry<String>> {
    if model.statuses.is_empty() {
        return Vec::new();
    }
    let mut out = vec![Entry::new(String::new(), words.no_status.clone())];
    out.extend(model.statuses.iter().map(|s| Entry::new(s.code.to_string(), Label::from(s.name.clone()))));
    out
}

/// `glyph.setStatus` for a Status pop-up's pick: a code, or empty for No Status.
pub fn status_request(glyph: &str, value: &str) -> Msg {
    let status = value.parse::<i64>().map_or(serde_json::Value::Null, |code| json!(code));
    Msg::run("glyph.setStatus", json!({"glyphs": [glyph], "status": status}))
}

/// The shell command for a change the header made.
fn column_request(model: &FontViewModel, change: &ColumnEvent, sent: &mut Vec<Msg>) {
    let shell = |id: &str, params| Msg::Shell(CommandRef::with_params(id, params));
    let ids = |columns: &[Column]| columns.iter().map(|c| c.id()).collect::<Vec<_>>();
    match change {
        ColumnEvent::Sort { key, descending, .. } => {
            if let Some(column) = column_of(key) {
                sent.push(shell("fontView.sortByColumn", json!({ "column": column.id(), "ascending": !descending })));
            }
        }
        ColumnEvent::Unsorted => sent.push(shell("fontView.sortByColumn", json!({ "column": "id", "ascending": true }))),
        ColumnEvent::Shown { key, shown, .. } => {
            let Some(column) = column_of(key) else { return };
            let mut visible = model.columns.clone();
            if *shown && !visible.contains(&column) {
                visible.push(column);
            } else if !*shown {
                visible.retain(|c| *c != column);
            }
            if !visible.is_empty() {
                sent.push(shell("fontView.setColumns", json!({ "visible": ids(&visible) })));
            }
        }
        ColumnEvent::Moved { key, to, .. } => {
            let Some(column) = column_of(key) else { return };
            let mut order = model.columns.clone();
            order.retain(|c| *c != column);
            order.insert((*to).min(order.len()), column);
            sent.push(shell("fontView.setColumns", json!({ "order": ids(&order) })));
        }
        ColumnEvent::Reset => sent.push(shell("fontView.setColumns", json!({ "visible": ids(&Column::ALL), "order": ids(&Column::ALL) }))),
        _ => {}
    }
}
