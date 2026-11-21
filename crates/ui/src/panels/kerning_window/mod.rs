//! The Kerning window (`window.kerning`): the stored pairs of the current master, left to right, listed by
//! `kerning.listPairs`, with the search options, sorting, editing, deletion and the clean-up actions.
//!
//! | Input | Does |
//! |---|---|
//! | typing in the search field | `kerning.filterPairs {term}` |
//! | Match, Entries, Sides, Only Glyphs Selected in Font View, Reset | the search options (`kerning.filterPairs`, `kerning.resetFilter`) |
//! | header press | `kerning.sortPairs` by the column; a second press reverses the order |
//! | row press (Shift, Cmd as in lists), arrows | selects rows (`window.kerning {select}`) |
//! | Value field | `kerning.setPair` on the current row |
//! | double press on a left group, or F2 | renames it in place; confirming asks `dialog.renameKerningGroup` |
//! | secondary press on a group cell | asks `dialog.renameKerningGroup` for that side's group |
//! | Delete, − button | `kerning.deletePairs` on the selected rows |
//! | Show All Glyphs | `kerning.showAllGlyphs` on the current row: every glyph pair it covers in the edit view |
//! | Clean Up, Compress | `kerning.cleanUp`, `kerning.compress` on the master shown |

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, EditEvent, IconId, ItemSource, Selection};
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnEvent, ColumnSort, ColumnSpec, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _, Tracked,
};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ListRow;
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::table::Table;
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use serde_json::{Map, Value, json};

use crate::app::Msg;
use crate::app::models::kerning_window::{KernRow, KerningWindowModel};
use crate::shell::kerning_window::{Column, MATCHES};

craft::ui::words! {
    /// The words of the Kerning window.
    pub struct KerningWindowWords {
        search: "fontcraftKerningWindow.search" = "Search";
        matching: "fontcraftKerningWindow.match" = "Match";
        contains: "fontcraftKerningWindow.contains" = "Contains";
        is: "fontcraftKerningWindow.is" = "Is Exactly";
        begins_with: "fontcraftKerningWindow.beginsWith" = "Begins With";
        ends_with: "fontcraftKerningWindow.endsWith" = "Ends With";
        does_not_contain: "fontcraftKerningWindow.doesNotContain" = "Does Not Contain";
        entries: "fontcraftKerningWindow.entries" = "Entries";
        any_entry: "fontcraftKerningWindow.anyEntry" = "All";
        group: "fontcraftKerningWindow.group" = "Is a Group";
        not_group: "fontcraftKerningWindow.notGroup" = "Is Not a Group";
        sides: "fontcraftKerningWindow.sides" = "Sides";
        both_sides: "fontcraftKerningWindow.bothSides" = "Both";
        left_side: "fontcraftKerningWindow.leftSide" = "Left Side";
        right_side: "fontcraftKerningWindow.rightSide" = "Right Side";
        selected_only: "fontcraftKerningWindow.selectedOnly" = "Only Glyphs Selected in Font View";
        reset: "fontcraftKerningWindow.reset" = "Reset";
        pairs: "fontcraftKerningWindow.pairs" = "Kerning Pairs";
        left: "fontcraftKerningWindow.left" = "Left";
        value: "fontcraftKerningWindow.value" = "Value";
        pair_value: "fontcraftKerningWindow.pairValue" = "Pair Value";
        right: "fontcraftKerningWindow.right" = "Right";
        empty: "fontcraftKerningWindow.empty" = "No kerning pairs";
        delete: "fontcraftKerningWindow.delete" = "Delete the selected pairs";
        show_all: "fontcraftKerningWindow.showAll" = "Show All Glyphs";
        show_all_tip: "fontcraftKerningWindow.showAllTip" = "Put every glyph pair the current row covers into the edit view";
        clean_up: "fontcraftKerningWindow.cleanUp" = "Clean Up";
        compress: "fontcraftKerningWindow.compress" = "Compress";
        count: "fontcraftKerningWindow.count" = "pairs";
        of: "fontcraftKerningWindow.of" = "of";
        selected: "fontcraftKerningWindow.selected" = "selected";
        linked: "fontcraftKerningWindow.linked" = "the kerning of master";
    }
}

const LEFT: ColumnKey = ColumnKey::new("left");
const VALUE: ColumnKey = ColumnKey::new("value");
const RIGHT: ColumnKey = ColumnKey::new("right");

pub struct Props<'a> {
    pub model: &'a Tracked<KerningWindowModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

/// A kerning value as the list writes it: whole units without decimals.
pub fn format_value(v: f64) -> String {
    if v.fract() == 0.0 { format!("{v:.0}") } else { format!("{}", (v * 100.0).round() / 100.0) }
}

/// The master and direction edits go to.
fn scope(model: &KerningWindowModel) -> Map<String, Value> {
    let mut p = Map::new();
    if let Some(m) = &model.master {
        p.insert("master".into(), json!(m));
    }
    p.insert("direction".into(), json!(if model.direction.is_empty() { "ltr" } else { model.direction.as_str() }));
    p
}

fn pair_json(row: &KernRow) -> Value {
    json!({"left": row.left, "right": row.right})
}

/// The rows in list order: a row's key is its position.
struct Rows<'a>(&'a [KernRow]);

impl DisplayOrder<usize> for Rows<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|r| Cow::Borrowed(r.left.as_str()))
    }
}

impl ItemSource<usize> for Rows<'_> {}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let model: &KerningWindowModel = props.model;
    let sent = &mut *props.sent;
    let words = Env::of(ui).words::<KerningWindowWords>();
    let _ = Stack::new().fill_width().show(ui, |ui, _| {
        search_bar(ui, &words, model, sent);
        pair_table(ui, &words, model, sent);
        footer(ui, &words, model, sent);
    });
}

fn search_bar(ui: &mut Ui, words: &KerningWindowWords, model: &KerningWindowModel, sent: &mut Vec<Msg>) {
    let search = &model.search;
    let mut term = search.term.clone();
    TextField::search(words.search.clone(), Bind::bound(&mut term)).placeholder(words.search.clone()).show(ui);
    if term != search.term {
        sent.push(shell("kerning.filterPairs", json!({ "term": term })));
    }
    let labels = [&words.contains, &words.is, &words.begins_with, &words.ends_with, &words.does_not_contain];
    let matches: Vec<Entry<String>> = MATCHES.iter().zip(labels).map(|(id, label)| Entry::new(id.to_string(), label.clone())).collect();
    let kinds = [
        Entry::new(String::new(), words.any_entry.clone()),
        Entry::new("group".to_string(), words.group.clone()),
        Entry::new("notGroup".to_string(), words.not_group.clone()),
    ];
    let sides = [
        Entry::new(String::new(), words.both_sides.clone()),
        Entry::new("left".to_string(), words.left_side.clone()),
        Entry::new("right".to_string(), words.right_side.clone()),
    ];
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        if let Some(picked) = Select::new(words.matching.clone(), Bind::given(search.matching.clone()), &matches).show(ui).events.committed() {
            sent.push(shell("kerning.filterPairs", json!({ "match": picked })));
        }
        let mut kind = search.kind.clone().unwrap_or_default();
        Segmented::new(words.entries.clone(), Bind::bound(&mut kind), &kinds).show(ui);
        if kind != search.kind.clone().unwrap_or_default() {
            sent.push(shell("kerning.filterPairs", json!({ "kind": (!kind.is_empty()).then_some(kind) })));
        }
        let mut side = search.side.clone().unwrap_or_default();
        Segmented::new(words.sides.clone(), Bind::bound(&mut side), &sides).show(ui);
        if side != search.side.clone().unwrap_or_default() {
            sent.push(shell("kerning.filterPairs", json!({ "side": (!side.is_empty()).then_some(side) })));
        }
    });
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        let only = Checkbox::new(Bind::given(search.selected_glyphs_only)).label(words.selected_only.clone()).show(ui);
        if let Some(on) = only.events.committed() {
            sent.push(shell("kerning.filterPairs", json!({ "selectedGlyphsOnly": on })));
        }
        if Button::new(words.reset.clone()).show(ui).clicked() {
            sent.push(shell("kerning.resetFilter", json!({})));
        }
    });
}

/// The selection as the table holds it: positions, the current row as lead.
fn selection(model: &KerningWindowModel) -> Selection<usize> {
    let lead = model.current.filter(|c| model.selected.contains(c)).or_else(|| model.selected.last().copied());
    Selection::new(model.selected.clone(), lead, lead)
}

fn pair_table(ui: &mut Ui, words: &KerningWindowWords, model: &KerningWindowModel, sent: &mut Vec<Msg>) {
    let columns = vec![
        ColumnSpec::new(LEFT, words.left.clone(), ColumnWidth::Content).sortable().resizable(),
        ColumnSpec::new(VALUE, words.value.clone(), ColumnWidth::Content).sortable().resizable().numeric(),
        ColumnSpec::new(RIGHT, words.right.clone(), ColumnWidth::Content).sortable().resizable(),
    ];
    let key = |c: Column| match c {
        Column::Left => LEFT,
        Column::Value => VALUE,
        Column::Right => RIGHT,
    };
    let sort = model.sort.map(|s| ColumnSort::new(key(s.column), s.descending));
    let before = selection(model);
    let mut after = before.clone();
    let rows = &model.rows;
    let source = Rows(rows);
    let out = Table::<usize>::new(words.pairs.clone(), columns)
        .empty(model.error.clone().map_or_else(|| words.empty.clone(), Into::into))
        .sort(sort)
        .striped(true)
        .deletable(true)
        .rename(true)
        .rename_column(LEFT)
        .can_rename(|at: &usize| rows.get(*at).is_some_and(|r| r.left.starts_with('@')))
        .rename_draft(|at: &usize| rows.get(*at).map(|r| r.left.trim_start_matches('@').to_string()))
        .selection(Bind::bound(&mut after))
        .show_items(ui, &source, &mut |at, icx| {
            let Some(row) = rows.get(*at) else { return ListRow::new("") };
            ListRow::new(row.left.clone())
                .cells(icx.cells())
                .striped(icx.row.striped)
                .cell(VALUE, format_value(row.value))
                .cell(RIGHT, row.right.clone())
        });
    if after.items != before.items || after.lead != before.lead {
        let mut picked: Vec<usize> = after.items.clone();
        // The lead goes last: it is the row the steps go from.
        if let Some(lead) = after.lead
            && let Some(at) = picked.iter().position(|i| *i == lead)
        {
            picked.remove(at);
            picked.push(lead);
        }
        let select: Vec<Value> = picked.iter().filter_map(|&i| rows.get(i)).map(pair_json).collect();
        sent.push(shell("window.kerning", json!({ "select": select })));
    }
    for event in &out.events {
        match event {
            CollectionEvent::Column(ColumnEvent::Sort { key, descending, .. }) => {
                if let Some(column) = key.as_str().and_then(Column::of_id) {
                    sent.push(shell("kerning.sortPairs", json!({ "column": column.id(), "descending": descending })));
                }
            }
            CollectionEvent::Delete(picked) => delete(model, picked, sent),
            CollectionEvent::Rename(EditEvent::Commit { after: rename, .. } | EditEvent::Set { after: rename, .. }) => {
                if let Some(row) = rows.get(rename.key) {
                    ask_rename(model, row, Column::Left, Some(rename.name.trim()), sent);
                }
            }
            CollectionEvent::ContextMenu(request) => {
                let column = request.column.as_ref().and_then(ColumnKey::as_str).and_then(Column::of_id);
                if let (Some(row), Some(column @ (Column::Left | Column::Right))) = (rows.get(request.key), column) {
                    ask_rename(model, row, column, None, sent);
                }
            }
            _ => {}
        }
    }
}

/// Asks the rename question for the group on `column` of `row`; `to` is the new name typed in place.
fn ask_rename(model: &KerningWindowModel, row: &KernRow, column: Column, to: Option<&str>, sent: &mut Vec<Msg>) {
    let side = if column == Column::Left { &row.left } else { &row.right };
    let Some(from) = side.strip_prefix('@') else { return };
    let typed = to.map(|t| t.trim_start_matches('@'));
    if typed.is_some_and(|t| t.is_empty() || t == from) {
        return;
    }
    let to = typed.unwrap_or(from);
    let mut p = scope(model);
    p.insert("side".into(), json!(column.id()));
    p.insert("from".into(), json!(from));
    p.insert("to".into(), json!(to));
    p.insert("pair".into(), pair_json(row));
    sent.push(shell("dialog.renameKerningGroup", Value::Object(p)));
}

fn delete(model: &KerningWindowModel, picked: &[usize], sent: &mut Vec<Msg>) {
    let pairs: Vec<Value> = picked.iter().filter_map(|&i| model.rows.get(i)).map(pair_json).collect();
    if pairs.is_empty() {
        return;
    }
    let mut p = scope(model);
    p.insert("pairs".into(), Value::Array(pairs));
    sent.push(Msg::run("kerning.deletePairs", Value::Object(p)));
}

/// The counts under the list.
pub fn counts_text(words: &KerningWindowWords, model: &KerningWindowModel) -> String {
    let (listed, total) = (model.rows.len(), model.total);
    let mut note = if listed == total {
        format!("{total} {}", words.count.as_str())
    } else {
        format!("{listed} {} {total} {}", words.of.as_str(), words.count.as_str())
    };
    if !model.selected.is_empty() {
        note.push_str(&format!(", {} {}", model.selected.len(), words.selected.as_str()));
    }
    if let (Some(m), Some(s)) = (&model.master, &model.source)
        && m != s
    {
        note.push_str(&format!(" ({} {s})", words.linked.as_str()));
    }
    note
}

fn footer(ui: &mut Ui, words: &KerningWindowWords, model: &KerningWindowModel, sent: &mut Vec<Msg>) {
    let current = model.current.and_then(|i| model.rows.get(i));
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        let minus =
            IconButton::new(IconId::new("minus")).label(words.delete.clone()).tooltip(words.delete.clone()).enabled(!model.selected.is_empty());
        if minus.show(ui).clicked() {
            delete(model, &model.selected, sent);
        }
        let value = NumberField::new(words.pair_value.clone(), Bind::given(current.map(|r| r.value))).enabled(current.is_some()).show(ui);
        if let (Some(Some(v)), Some(row)) = (value.edit.events.committed(), current) {
            let mut p = scope(model);
            p.insert("left".into(), json!(row.left));
            p.insert("right".into(), json!(row.right));
            p.insert("value".into(), json!(v));
            sent.push(Msg::run("kerning.setPair", Value::Object(p)));
        }
        match &model.error {
            Some(error) => Text::new(error.clone()).show(ui),
            None => Text::new(counts_text(words, model)).shade(Shade::Muted).show(ui),
        };
        if Button::new(words.show_all.clone()).tooltip(words.show_all_tip.clone()).enabled(current.is_some()).show(ui).clicked()
            && let Some(row) = current
        {
            sent.push(shell("kerning.showAllGlyphs", pair_json(row)));
        }
        if Button::new(words.clean_up.clone()).enabled(model.has_doc).show(ui).clicked() {
            sent.push(Msg::run("kerning.cleanUp", Value::Object(scope(model))));
        }
        if Button::new(words.compress.clone()).enabled(model.has_doc).show(ui).clicked() {
            sent.push(Msg::run("kerning.compress", Value::Object(scope(model))));
        }
    });
}

#[cfg(test)]
mod tests;
