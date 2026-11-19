//! Glyph Info: the glyph data as a table, searched and sorted by `glyphData.search`, with Add to Font
//! (`glyphInfo.addToFont`).
//!
//! | Input | Does |
//! |---|---|
//! | typing in the search field | searches again (`window.glyphInfo {query}`) |
//! | a header press | sorts by the column (`{sortBy}`) |
//! | a click, Shift+click, Cmd+click on rows | selects rows (`{select}`) |
//! | Add to Font | `glyphInfo.addToFont` with the selected names |
//!
//! The table lays out only the rows in view.

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, ItemSource, Selection};
use craft::ui::buttons::Button;
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnEvent, ColumnSort, ColumnSpec, Props as _, Show as _, ShowCollection as _, ShowContainer as _, Tracked,
};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ListRow;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::table::Table;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Value, json};

use crate::app::Msg;
use crate::app::models::glyph_info::{GlyphInfoModel, InfoRow};
use crate::shell::glyph_info::{Column, MAX_QUERY_CHARS, MAX_ROWS};

craft::ui::words! {
    /// The words of the Glyph Info panel.
    pub struct GlyphInfoWords {
        search: "fontcraftGlyphInfo.search" = "Search name, character or code point";
        table: "fontcraftGlyphInfo.table" = "Glyph data";
        none: "fontcraftGlyphInfo.none" = "No matches";
        name: "fontcraftGlyphInfo.name" = "Name";
        unicode: "fontcraftGlyphInfo.unicode" = "Unicode";
        category: "fontcraftGlyphInfo.category" = "Category";
        sub_category: "fontcraftGlyphInfo.subCategory" = "Subcategory";
        script: "fontcraftGlyphInfo.script" = "Script";
        description: "fontcraftGlyphInfo.description" = "Description";
        /// After the number of matches.
        matches: "fontcraftGlyphInfo.matches" = "matches";
        /// After the number of selected rows.
        selected: "fontcraftGlyphInfo.selected" = "selected";
        /// Before the row limit when the search found more rows than the table lists.
        first: "fontcraftGlyphInfo.first" = "The first";
        add: "fontcraftGlyphInfo.add" = "Add to Font";
        add_tip: "fontcraftGlyphInfo.addTip" = "Add the selected glyphs to the font";
        no_font: "fontcraftGlyphInfo.noFont" = "Open a font first";
    }
}

fn header(words: &GlyphInfoWords, column: Column) -> Label {
    match column {
        Column::Name => words.name.clone(),
        Column::Unicode => words.unicode.clone(),
        Column::Category => words.category.clone(),
        Column::SubCategory => words.sub_category.clone(),
        Column::Script => words.script.clone(),
        Column::Description => words.description.clone(),
    }
}

fn key(column: Column) -> ColumnKey {
    ColumnKey::new(column.id())
}

pub struct Props<'a> {
    pub model: &'a Tracked<GlyphInfoModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn command(params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params("window.glyphInfo", params))
}

/// The rows as the table's item source: the row number is the key.
struct Rows<'a>(&'a [InfoRow]);

impl DisplayOrder<usize> for Rows<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|r| Cow::Borrowed(r.name.as_str()))
    }
}

impl ItemSource<usize> for Rows<'_> {}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<GlyphInfoWords>();
    let model: &GlyphInfoModel = props.model;
    let sent = &mut *props.sent;
    let mut top = Vec::new();
    let mut bottom = Vec::new();
    let mut middle = Vec::new();
    let _ = Stack::new()
        .fill()
        .header(|ui, _| search(ui, &words, model, &mut top))
        .footer(|ui, _| footer(ui, &words, model, &mut bottom))
        .show(ui, |ui, _| table(ui, &words, model, &mut middle));
    sent.extend(top.into_iter().chain(middle).chain(bottom));
}

fn search(ui: &mut Ui, words: &GlyphInfoWords, model: &GlyphInfoModel, sent: &mut Vec<Msg>) {
    let mut text = model.query.clone();
    TextField::search(words.search.clone(), Bind::bound(&mut text)).placeholder(words.search.clone()).limit(MAX_QUERY_CHARS).show(ui);
    if text != model.query {
        sent.push(command(json!({ "query": text })));
    }
}

fn table(ui: &mut Ui, words: &GlyphInfoWords, model: &GlyphInfoModel, sent: &mut Vec<Msg>) {
    let specs: Vec<ColumnSpec> = Column::ALL.iter().map(|&c| ColumnSpec::new(key(c), header(words, c), ColumnWidth::Content).sortable()).collect();
    let sort = Some(ColumnSort::new(key(model.sort_by.unwrap_or_default()), false));
    let lead = model.selected.last().copied();
    let mut selection = Selection::new(model.selected.clone(), lead, lead);
    let out = Table::<usize>::new(words.table.clone(), specs)
        .empty(words.none.clone())
        .sort(sort)
        .striped(true)
        .type_ahead(true)
        .selection(Bind::bound(&mut selection))
        .show_items(ui, &Rows(&model.rows), &mut |at, icx| {
            let row = model.rows.get(*at);
            let text = |c: Column| row.map(|r| r.cell(c).to_string()).unwrap_or_default();
            let mut list_row = ListRow::new(text(Column::Name)).cells(icx.cells()).striped(icx.row.striped);
            for &c in Column::ALL.iter().skip(1) {
                list_row = list_row.cell(key(c), text(c));
            }
            list_row
        });
    if selection.items != model.selected {
        let names: Vec<&str> = selection.items.iter().filter_map(|&i| model.rows.get(i)).map(|r| r.name.as_str()).collect();
        sent.push(command(json!({ "select": names })));
    }
    for event in &out.events {
        match event {
            CollectionEvent::Column(ColumnEvent::Sort { key, .. }) => {
                if let Some(column) = key.as_str().and_then(Column::of_id) {
                    sent.push(command(json!({ "sortBy": column.id() })));
                }
            }
            CollectionEvent::Column(ColumnEvent::Unsorted { .. }) => sent.push(command(json!({ "sortBy": Column::Name.id() }))),
            _ => {}
        }
    }
}

fn footer(ui: &mut Ui, words: &GlyphInfoWords, model: &GlyphInfoModel, sent: &mut Vec<Msg>) {
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        let selected = format!("{} {}", model.selected.len(), words.selected.as_str());
        match &model.error {
            Some(error) => Text::new(error.clone()).show(ui),
            None if model.rows.len() >= MAX_ROWS => {
                Text::new(format!("{} {MAX_ROWS} {}, {selected}", words.first.as_str(), words.matches.as_str())).shade(Shade::Muted).show(ui)
            }
            None => Text::new(format!("{} {}, {selected}", model.rows.len(), words.matches.as_str())).shade(Shade::Muted).show(ui),
        };
        let enabled = model.has_font && !model.selected.is_empty();
        let tip = if model.has_font { words.add_tip.clone() } else { words.no_font.clone() };
        if Button::new(words.add.clone()).tooltip(tip).enabled(enabled).show(ui).clicked() && enabled {
            let names: Vec<&str> = model.selected.iter().filter_map(|&i| model.rows.get(i)).map(|r| r.name.as_str()).collect();
            sent.push(Msg::run("glyphInfo.addToFont", json!({ "names": names })));
        }
    });
}

#[cfg(test)]
mod tests;
