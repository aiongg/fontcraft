//! The Kerning window (`window.kerning`): the stored pairs of the current master in one writing direction, listed by
//! `kerning.listPairs`, with the search options, sorting, editing, deletion and the clean-up actions.
//!
//! | Input | Does |
//! |---|---|
//! | typing in the search field | `kerning.filterPairs {term}` |
//! | Match, Entries, Sides, Only Glyphs Selected in Font View, Reset | the search options (`kerning.filterPairs`, `kerning.resetFilter`) |
//! | Writing Direction | `window.kerning {direction}`: the Edit View tab's, or one chosen here |
//! | header press | `kerning.sortPairs` by the column; a second press reverses the order |
//! | row press (Shift, Cmd as in lists), arrows | selects rows (`window.kerning {select}`) |
//! | Value field | one row selected: `kerning.setPair`; several: `kerning.setPairs`, a number sets them all, `+10`, `-10` or `*0.9` changes each, `=-10` sets a negative one |
//! | double press on a value, Return | edits the value in the row (`window.kerning {edit}`); Up and Down step it by 1, 10 with Shift, 100 with Cmd+Shift; Escape cancels |
//! | double press on a group, F2 (left groups) | renames it in place; confirming asks `dialog.renameKerningGroup` |
//! | secondary press | the row's menu: Show All Glyphs, Rename Left or Right Group…, Make Exception, Copy, Delete, Delete from All Masters |
//! | Delete, − button (with Option: from all masters) | `kerning.deletePairs` on the selected rows |
//! | Copy, Paste, Select All (keys and the Edit menu, while the list has the keyboard) | `kerning.copyPairs` on the selected rows, `dialog.pasteKerningConflict` with the clipboard's text, `window.kerning {select: "all"}` |
//! | Actions › Show All Glyphs | `kerning.showAllGlyphs` on the current row: every glyph pair it covers in the edit view |
//! | Actions › Clean Up, Compress | `kerning.cleanUp`, `kerning.compress` on the master shown |
//!
//! Group names are drawn with their `@` in one colour and glyph names in another (`appearance.color.kerningGroupText`,
//! `appearance.color.kerningGlyphText`); a selected row draws both in the selection's colour.

use std::borrow::Cow;
use std::cell::RefCell;

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, EditEvent, IconId, ItemSource, Mixed, Selection};
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnEvent, ColumnSort, ColumnSpec, EditResponse, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _,
    Tracked, Trigger,
};
use craft::ui::display::{Elide, Shade, Text};
use craft::ui::fonts::FontRef;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ListRow;
use craft::ui::menu::Menu;
use craft::ui::pickers::Select;
use craft::ui::primitives::DocRun;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::runtime::focus::{self, FocusTarget};
use craft::ui::style::DataColor;
use craft::ui::table::Table;
use craft::ui::text::{NumberField, TextField};
use craft_ext::ui::list::{CollectionEdits as _, EditCommand};
use craft_ext::ui::text::{ArrowSteps, StepKeys as _};
use egui::Ui;
use serde_json::{Map, Value, json};

use crate::app::Msg;
use crate::app::models::kerning_window::{KernRow, KernRows, KerningWindowModel};
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
        delete_tip: "fontcraftKerningWindow.deleteTip" = "Delete the selected pairs (Option: from all masters)";
        show_all: "fontcraftKerningWindow.showAll" = "Show All Glyphs";
        actions: "fontcraftKerningWindow.actions" = "Actions";
        row_menu: "fontcraftKerningWindow.rowMenu" = "Kerning pair";
        rename_left: "fontcraftKerningWindow.renameLeft" = "Rename Left Group…";
        rename_right: "fontcraftKerningWindow.renameRight" = "Rename Right Group…";
        exception: "fontcraftKerningWindow.exception" = "Make Exception";
        copy: "fontcraftKerningWindow.copy" = "Copy";
        delete_one: "fontcraftKerningWindow.deleteOne" = "Delete";
        delete_all: "fontcraftKerningWindow.deleteAll" = "Delete from All Masters";
        group_name: "fontcraftKerningWindow.groupName" = "Right group";
        values: "fontcraftKerningWindow.values" = "Values of the Selected Pairs";
        values_tip: "fontcraftKerningWindow.valuesTip" = "A number sets every selected pair; +10, -10 or *0.9 changes each; =-10 sets a negative value";
        clean_up: "fontcraftKerningWindow.cleanUp" = "Clean Up";
        compress: "fontcraftKerningWindow.compress" = "Compress";
        count: "fontcraftKerningWindow.count" = "pairs";
        of: "fontcraftKerningWindow.of" = "of";
        selected: "fontcraftKerningWindow.selected" = "selected";
        linked: "fontcraftKerningWindow.linked" = "the kerning of master";
        direction: "fontcraftKerningWindow.direction" = "Writing Direction";
        tab_direction: "fontcraftKerningWindow.tabDirection" = "As in Edit View";
        ltr: "fontcraftKerningWindow.leftToRight" = "Left to Right";
        rtl: "fontcraftKerningWindow.rightToLeft" = "Right to Left";
        vertical: "fontcraftKerningWindow.topToBottom" = "Top to Bottom";
    }
}

/// The Edit commands the list takes.
const LIST_EDITS: [EditCommand; 3] = [EditCommand::Copy, EditCommand::Paste, EditCommand::SelectAll];

/// Most characters a value field takes.
const MAX_VALUE_CHARS: usize = 32;

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

fn pair_json(row: KernRow<'_>) -> Value {
    json!({"left": row.left, "right": row.right})
}

/// The rows in list order: a row's key is its position.
struct Rows<'a>(&'a KernRows);

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
        self.0.get(at).map(|r| Cow::Borrowed(r.left))
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
    let directions = [
        Entry::new(String::new(), words.tab_direction.clone()),
        Entry::new("ltr".to_string(), words.ltr.clone()),
        Entry::new("rtl".to_string(), words.rtl.clone()),
        Entry::new("vertical".to_string(), words.vertical.clone()),
    ];
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        let own = model.own_direction.clone().unwrap_or_default();
        if let Some(picked) = Select::new(words.direction.clone(), Bind::given(own), &directions).show(ui).events.committed() {
            sent.push(shell("window.kerning", json!({ "direction": (!picked.is_empty()).then_some(picked) })));
        }
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
    Selection::new(model.selected.to_vec(), lead, lead)
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
    let size = Env::of(ui).tokens.typography.body.size;
    let entry = |name: &str| {
        let [r, g, b, a] = if name.starts_with('@') { model.group_color } else { model.glyph_color };
        let color = DataColor::new(egui::Color32::from_rgba_unmultiplied(r, g, b, a));
        Text::spans(&[DocRun::new(name.to_string(), FontRef::proportional(), size, color)]).elide(Elide::End)
    };
    let typing = brings_text(ui);
    let edited: RefCell<Option<EditResponse<String>>> = RefCell::new(None);
    let out = Table::<usize>::new(words.pairs.clone(), columns)
        .empty(words.empty.clone())
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
            let mut shown = ListRow::new(row.left.to_string())
                .cells(icx.cells())
                .state(icx.row.clone())
                .cell(VALUE, format_value(row.value))
                .cell(RIGHT, row.right.to_string());
            let editing = model.editing.filter(|(edited, _)| edited == at).map(|(_, column)| column);
            // The text cells above size the columns; a field or a coloured name stands in their place.
            if !icx.row.selected && !icx.row.renaming {
                shown = shown.cell_field(LEFT, entry(row.left), |_| {});
                if editing != Some(Column::Right) {
                    shown = shown.cell_field(RIGHT, entry(row.right), |_| {});
                }
            }
            let edited = &edited;
            match editing {
                Some(Column::Value) => {
                    let field = TextField::new(words.pair_value.clone(), Bind::given(format_value(row.value)))
                        .select_on_focus()
                        .submits()
                        .limit(MAX_VALUE_CHARS)
                        .id_salt("fontcraftKerningWindow.cellValue");
                    shown.cell_field(VALUE, field, move |out| *edited.borrow_mut() = Some(out))
                }
                Some(Column::Right) => {
                    let field = TextField::new(words.group_name.clone(), Bind::given(row.right.trim_start_matches('@').to_string()))
                        .select_on_focus()
                        .submits()
                        .id_salt("fontcraftKerningWindow.cellRight");
                    shown.cell_field(RIGHT, field, move |out| *edited.borrow_mut() = Some(out))
                }
                _ => shown,
            }
        });
    cell_edit(ui, model, edited.take(), typing, sent);
    if after.items != before.items || after.lead != before.lead {
        let mut picked: Vec<usize> = after.items.clone();
        // The lead goes last: it is the row the steps go from.
        if let Some(lead) = after.lead
            && let Some(at) = picked.iter().position(|i| *i == lead)
        {
            picked.remove(at);
            picked.push(lead);
        }
        if picked.len() > 1 && picked.len() == rows.len() && model.scope.is_some() {
            sent.push(shell("window.kerning", json!({ "select": "all" })));
        } else {
            let select: Vec<Value> = picked.iter().filter_map(|&i| rows.get(i)).map(pair_json).collect();
            sent.push(shell("window.kerning", json!({ "select": select })));
        }
    }
    for edit in out.edit_commands(ui, &LIST_EDITS) {
        match edit {
            EditCommand::Copy if !model.selected.is_empty() => sent.push(copy(model)),
            EditCommand::Paste => sent.push(shell("dialog.pasteKerningConflict", json!({}))),
            EditCommand::SelectAll => sent.push(shell("window.kerning", json!({ "select": "all" }))),
            _ => {}
        }
    }
    let all_masters = ui.input(|i| i.modifiers.alt);
    for event in &out.events {
        match event {
            CollectionEvent::Column(ColumnEvent::Sort { key, descending, .. }) => {
                if let Some(column) = key.as_str().and_then(Column::of_id) {
                    sent.push(shell("kerning.sortPairs", json!({ "column": column.id(), "descending": descending })));
                }
            }
            CollectionEvent::Delete(picked) => delete(model, picked, all_masters, sent),
            CollectionEvent::Rename(EditEvent::Commit { after: rename, .. } | EditEvent::Set { after: rename, .. }) => {
                if let Some(row) = rows.get(rename.key) {
                    ask_rename(model, row, Column::Left, Some(rename.name.trim()), sent);
                }
            }
            CollectionEvent::Activate(at) => {
                let Some(row) = rows.get(*at) else { continue };
                // Return edits the value; a double press edits the cell it lands on.
                let pressed = if out.activation_clicks == 0 {
                    Some(Column::Value)
                } else {
                    let pointer = ui.input(|i| i.pointer.interact_pos());
                    let cells = out.shown.iter().find(|shown| shown.key == *at).map(|shown| shown.cells.as_slice()).unwrap_or_default();
                    let cell = cells.iter().find(|(_, rect)| pointer.is_some_and(|p| rect.x_range().contains(p.x)));
                    cell.and_then(|(key, _)| key.as_str()).and_then(Column::of_id)
                };
                if let Some(column) = pressed.filter(|c| *c == Column::Value || (*c == Column::Right && row.right.starts_with('@'))) {
                    sent.push(shell("window.kerning", json!({ "edit": { "left": row.left, "right": row.right, "column": column.id() } })));
                }
            }
            // The menu acts on the row pressed, with the other selected rows when it is one of them.
            CollectionEvent::ContextMenu(request) if model.current != Some(request.key) => {
                let within = model.selected.contains(&request.key);
                // Every row stays selected as one query, whatever their number.
                if within
                    && model.selected.len() == rows.len()
                    && model.scope.is_some()
                    && let Some(row) = rows.get(request.key)
                {
                    sent.push(shell("window.kerning", json!({ "select": "all", "current": pair_json(row) })));
                    continue;
                }
                let mut picked = if within { model.selected.to_vec() } else { Vec::new() };
                picked.retain(|i| *i != request.key);
                picked.push(request.key);
                sent.push(shell("window.kerning", json!({ "select": pairs_json(model, &picked) })));
            }
            _ => {}
        }
    }
    row_menu(ui, words, model, &out.response, sent);
}

/// The cell edited in place, as the last pass left it.
#[derive(Clone)]
struct EditedCell {
    cell: String,
    /// Its text has had the focus.
    focused: bool,
    /// Its text may differ from the stored value.
    drafted: bool,
}

/// Whether the pass brings something that changes a focused text.
fn brings_text(ui: &Ui) -> bool {
    use egui::{Event, Key};
    ui.input(|i| {
        i.events.iter().any(|e| {
            matches!(
                e,
                Event::Text(_) | Event::Paste(_) | Event::Cut | Event::Ime(_) | Event::Key { key: Key::Backspace | Key::Delete, pressed: true, .. }
            )
        })
    })
}

/// What the cell edited in place did this pass: a new value or group name is sent, and the field closes with Return,
/// Escape, Tab or a press elsewhere.
///
/// `typing`: the pass brings text, which the field may hold as a draft.
fn cell_edit(ui: &Ui, model: &KerningWindowModel, out: Option<EditResponse<String>>, typing: bool, sent: &mut Vec<Msg>) {
    let focused = egui::Id::unique("fontcraftKerningWindow.cellFocused");
    let Some((at, column)) = model.editing else {
        ui.data_mut(|d| d.remove::<EditedCell>(focused));
        return;
    };
    let (Some(out), Some(row)) = (out, model.rows.get(at)) else { return };
    let cell = format!("{}\t{}\t{}", row.left, row.right, column.id());
    let Some(before) = ui.data(|d| d.get_temp::<EditedCell>(focused)).filter(|kept| kept.cell == cell) else {
        let _ = focus::request(ui.ctx(), FocusTarget::Id(out.response.id));
        ui.data_mut(|d| d.insert_temp(focused, EditedCell { cell, focused: false, drafted: false }));
        return;
    };
    // The focus is on the field's text, not on the component.
    let has_focus = craft_ext::ui::text::text_focused(ui.ctx(), out.response.id);
    let left = before.focused && !has_focus;
    let shown = matches!(&out.shown, Some(Mixed::Same(text)) if column == Column::Value && **text != format_value(row.value));
    let drafted = before.drafted || shown || (has_focus && typing);
    ui.data_mut(|d| d.insert_temp(focused, EditedCell { cell, focused: before.focused || has_focus, drafted }));
    let committed = out.events.committed().cloned();
    match (column, &committed) {
        (Column::Value, Some(text)) => {
            let typed = text.trim().parse::<f64>().ok().filter(|v| v.is_finite());
            if let Some(v) = typed.filter(|v| *v != row.value) {
                sent.push(set_pair(model, row, v));
            }
        }
        (Column::Right, Some(text)) => ask_rename(model, row, Column::Right, Some(text.trim()), sent),
        _ => {}
    }
    // The stored value steps, so a typed draft is left to Return, which would store it over the step.
    if column == Column::Value
        && let Some(delta) = out.arrow_steps(ui, ArrowSteps::UNITS)
        && !drafted
    {
        sent.push(set_pair(model, row, row.value + delta));
    }
    if committed.is_some() || out.submitted || out.escaped || out.left_by_tab.is_some() || left {
        sent.push(shell("window.kerning", json!({ "edit": null })));
    }
}

fn set_pair(model: &KerningWindowModel, row: KernRow<'_>, value: f64) -> Msg {
    let mut p = scope(model);
    p.insert("left".into(), json!(row.left));
    p.insert("right".into(), json!(row.right));
    p.insert("value".into(), json!(value));
    Msg::run("kerning.setPair", Value::Object(p))
}

fn copy(model: &KerningWindowModel) -> Msg {
    let mut p = scope(model);
    insert_rows(&mut p, model, &model.selected);
    Msg::run("kerning.copyPairs", Value::Object(p))
}

/// What a typed text does to several rows, as a parameter of `kerning.setPairs`: `+10` and `-10` add, `*0.9` scales, a
/// number or `=-10` sets.
pub fn change_of(text: &str) -> Option<(&'static str, f64)> {
    let text = text.trim();
    let number = |t: &str| t.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    if let Some(rest) = text.strip_prefix('=') {
        return Some(("value", number(rest)?));
    }
    if let Some(rest) = text.strip_prefix('*') {
        return Some(("factor", number(rest)?));
    }
    if text.starts_with(['+', '-']) {
        return Some(("add", number(text)?));
    }
    Some(("value", number(text)?))
}

/// The menu of the pressed row.
fn row_menu(ui: &mut Ui, words: &KerningWindowWords, model: &KerningWindowModel, opener: &egui::Response, sent: &mut Vec<Msg>) {
    let current = model.current.and_then(|i| model.rows.get(i));
    let left = current.is_some_and(|row| row.left.starts_with('@'));
    let right = current.is_some_and(|row| row.right.starts_with('@'));
    let items = [
        Entry::new("showAll", words.show_all.clone()).enabled(current.is_some()),
        Entry::new("renameLeft", words.rename_left.clone()).enabled(left),
        Entry::new("renameRight", words.rename_right.clone()).enabled(right),
        Entry::new("exception", words.exception.clone()).enabled(left || right),
        Entry::new("copy", words.copy.clone()).enabled(!model.selected.is_empty()),
        Entry::new("delete", words.delete_one.clone()).enabled(!model.selected.is_empty()),
        Entry::new("deleteAll", words.delete_all.clone()).enabled(!model.selected.is_empty()),
    ];
    let picked = Menu::context(words.row_menu.clone(), opener).id_salt("fontcraftKerningWindow.rowMenu").pick(ui, &items).picked;
    let (Some(picked), Some(row)) = (picked, current) else { return };
    match picked {
        "showAll" => sent.push(shell("kerning.showAllGlyphs", pair_json(row))),
        "renameLeft" => ask_rename(model, row, Column::Left, None, sent),
        "renameRight" => ask_rename(model, row, Column::Right, None, sent),
        "exception" => sent.push(shell("kerning.makeException", pair_json(row))),
        "copy" => sent.push(copy(model)),
        "delete" => delete(model, &model.selected, false, sent),
        "deleteAll" => delete(model, &model.selected, true, sent),
        _ => {}
    }
}

/// Asks the rename question for the group on `column` of `row`; `to` is the new name typed in place.
fn ask_rename(model: &KerningWindowModel, row: KernRow<'_>, column: Column, to: Option<&str>, sent: &mut Vec<Msg>) {
    let side = if column == Column::Left { row.left } else { row.right };
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

/// Adds the rows at `picked` to a command's parameters: the list's `query` when they are all of it, else `pairs`.
fn insert_rows(p: &mut Map<String, Value>, model: &KerningWindowModel, picked: &[usize]) {
    match model.scope.as_ref().filter(|_| picked.len() > 1 && picked.len() == model.rows.len()) {
        Some(all) => p.insert("query".into(), all.query()),
        None => p.insert("pairs".into(), pairs_json(model, picked)),
    };
}

/// The pairs of the rows at `picked`.
fn pairs_json(model: &KerningWindowModel, picked: &[usize]) -> Value {
    Value::Array(picked.iter().filter_map(|&i| model.rows.get(i)).map(pair_json).collect())
}

fn delete(model: &KerningWindowModel, picked: &[usize], all_masters: bool, sent: &mut Vec<Msg>) {
    if picked.is_empty() {
        return;
    }
    let mut p = scope(model);
    insert_rows(&mut p, model, picked);
    if all_masters {
        p.insert("allMasters".into(), json!(true));
    }
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
            IconButton::new(IconId::new("minus")).label(words.delete.clone()).tooltip(words.delete_tip.clone()).enabled(!model.selected.is_empty());
        if minus.show(ui).clicked() {
            let all_masters = ui.input(|i| i.modifiers.alt);
            delete(model, &model.selected, all_masters, sent);
        }
        if model.selected.len() > 1 {
            let field = TextField::new(words.values.clone(), Bind::given(String::new()))
                .placeholder(words.values.clone())
                .tooltip(words.values_tip.clone())
                .limit(MAX_VALUE_CHARS)
                .id_salt("fontcraftKerningWindow.values")
                .show(ui);
            if let Some((key, v)) = field.events.committed().and_then(|text| change_of(text)) {
                let mut p = scope(model);
                insert_rows(&mut p, model, &model.selected);
                p.insert(key.into(), json!(v));
                sent.push(Msg::run("kerning.setPairs", Value::Object(p)));
            }
        } else {
            let value = NumberField::new(words.pair_value.clone(), Bind::given(current.map(|r| r.value))).enabled(current.is_some()).show(ui);
            if let (Some(Some(v)), Some(row)) = (value.edit.events.committed(), current) {
                sent.push(set_pair(model, row, *v));
            }
        }
        Text::new(counts_text(words, model)).shade(Shade::Muted).show(ui);
        let more = IconButton::new(IconId::new("more")).label(words.actions.clone()).tooltip(words.actions.clone()).show(ui);
        let items = [
            Entry::new("showAll", words.show_all.clone()).enabled(current.is_some()),
            Entry::new("cleanUp", words.clean_up.clone()).enabled(model.has_doc),
            Entry::new("compress", words.compress.clone()).enabled(model.has_doc),
        ];
        match (Menu::new(words.actions.clone()).opened_by(&more.response, Trigger::Click).pick(ui, &items).picked, current) {
            (Some("showAll"), Some(row)) => sent.push(shell("kerning.showAllGlyphs", pair_json(row))),
            (Some("cleanUp"), _) => sent.push(Msg::run("kerning.cleanUp", Value::Object(scope(model)))),
            (Some("compress"), _) => sent.push(Msg::run("kerning.compress", Value::Object(scope(model)))),
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests;
