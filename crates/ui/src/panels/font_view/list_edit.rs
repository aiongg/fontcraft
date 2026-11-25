//! Editing in List View (`fontView.list`): which cells of the lead row are fields, the entries of its pop-ups, and the
//! command each edit runs, the same one the inspector's field runs.
//!
//! | Cells | Edited as |
//! |---|---|
//! | Unicode, Production Name, Script, Subcategory, Tags, Sort Name, Note, Components, LSB, RSB, Width, groups | text |
//! | Category, Case, Direction, Color | a pop-up |
//! | Exports, Status | in every row: a check box, a pop-up |
//! | Name | the row's rename |
//!
//! An edit in a selected row changes every selected glyph. Only the lead row holds fields, so a long list lays out one
//! row of them.

use craft::theme::Srgba;
use craft::ui::contract::Entry;
use craft::ui::runtime::env::Label;
use serde_json::json;

use super::list::ColumnWords;
use crate::app::font_view::{CellDraft, Column};
use crate::app::models::font_view::{COLOR_LABEL_NAMES, FontViewModel};
use crate::app::models::glyph_props::{self, Edit, GlyphPropsModel, MetricField, PropField};
use crate::app::{Msg, ViewMsg};
use crate::panels::context_menu::LabelTokens;

/// Longest text a cell takes, in characters.
pub const MAX_CELL_CHARS: usize = 4096;

const CATEGORIES: [&str; 8] = ["Letter", "Mark", "Number", "Punctuation", "Separator", "Symbol", "Space", "Other"];
const CASES: [(&str, &str); 5] =
    [("upper", "Uppercase"), ("lower", "Lowercase"), ("smallCaps", "Small Caps"), ("minor", "Minor"), ("noCase", "No Case")];
const DIRECTIONS: [&str; 5] = ["LTR", "RTL", "BIDI", "VTR", "VTL"];

/// Whether the lead row's cell of `column` is a text field.
pub fn is_text(model: &FontViewModel, row: usize, column: Column) -> bool {
    match column {
        // The cell holds one line; a longer note is edited in Info for Selection.
        Column::Note => model.rows.get(row).is_some_and(|r| !r.long_note),
        Column::Unicode
        | Column::ProductionName
        | Column::Script
        | Column::SubCategory
        | Column::Tags
        | Column::SortName
        | Column::Components
        | Column::Lsb
        | Column::Rsb
        | Column::Width
        | Column::LeftGroup
        | Column::RightGroup => true,
        _ => false,
    }
}

/// Whether the lead row's cell of `column` is a pop-up.
pub fn is_pop_up(column: Column) -> bool {
    matches!(column, Column::Category | Column::Case | Column::Direction | Column::Color)
}

/// The pop-up of `column` in `row`: the value it shows and its entries. An empty value returns the property to the
/// glyph data's, which the entry names.
pub fn pop_up(model: &FontViewModel, row: usize, column: Column, words: &ColumnWords) -> (String, Vec<Entry<String>>) {
    let text = model.rows.get(row).and_then(|r| r.cells.get(column.index())).cloned().unwrap_or_default();
    let mut entries: Vec<Entry<String>> = Vec::new();
    let current = if column == Column::Color {
        entries.push(Entry::new(String::new(), words.no_color.clone()).empty_swatch());
        let unit = |v: u8| f32::from(v) / 255.0;
        for (i, (name, [r, g, b])) in COLOR_LABEL_NAMES.iter().zip(LabelTokens::standard().colors).enumerate() {
            entries.push(Entry::new(i.to_string(), Label::from((*name).to_string())).swatch(Srgba::new(unit(r), unit(g), unit(b), 1.0)));
        }
        COLOR_LABEL_NAMES.iter().position(|n| *n == text).map_or(text, |i| i.to_string())
    } else {
        let derived = model.is_derived(row, column);
        let choices: Vec<(&str, &str)> = match column {
            Column::Case => CASES.to_vec(),
            Column::Direction => DIRECTIONS.iter().map(|d| (*d, *d)).collect(),
            _ => CATEGORIES.iter().map(|c| (*c, *c)).collect(),
        };
        let name = |value: &str| choices.iter().find(|(v, _)| *v == value).map_or(value, |(_, n)| *n).to_string();
        let automatic = words.automatic.as_str();
        let label = if derived && !text.is_empty() { format!("{automatic} ({})", name(&text)) } else { automatic.to_string() };
        entries.push(Entry::new(String::new(), Label::from(label)));
        entries.extend(choices.iter().map(|(v, n)| Entry::new((*v).to_string(), Label::from((*n).to_string()))));
        if derived { String::new() } else { text }
    };
    // A stored value outside the list keeps its own entry.
    if !entries.iter().any(|e| e.value().is_some_and(|v| *v == current)) {
        entries.push(Entry::new(current.clone(), Label::from(current.clone())));
    }
    (current, entries)
}

/// What an edit of `column` in `row` to `text` runs on the glyphs it changes; `None` when there is nothing to set.
pub fn command(model: &FontViewModel, row: usize, column: Column, text: &str) -> Option<Msg> {
    let glyphs = model.edited(row);
    let master = model.master.clone().unwrap_or_default();
    let field = match column {
        Column::Unicode => Edit::Text(PropField::Unicode, text.into()),
        Column::ProductionName => Edit::Text(PropField::ProductionName, text.into()),
        Column::Script => Edit::Text(PropField::Script, text.into()),
        Column::Category => Edit::Text(PropField::Category, text.into()),
        Column::SubCategory => Edit::Text(PropField::SubCategory, text.into()),
        Column::Case => Edit::Text(PropField::Case, text.into()),
        Column::Direction => Edit::Text(PropField::Direction, text.into()),
        Column::SortName => Edit::Text(PropField::SortName, text.into()),
        Column::Note => Edit::Text(PropField::Note, text.into()),
        Column::LeftGroup => Edit::Text(PropField::LeftGroup, text.into()),
        Column::RightGroup => Edit::Text(PropField::RightGroup, text.into()),
        Column::Color => Edit::Text(PropField::Color, text.into()),
        Column::Lsb => Edit::Metric(MetricField::Lsb, text.into()),
        Column::Rsb => Edit::Metric(MetricField::Rsb, text.into()),
        Column::Width => Edit::Metric(MetricField::Width, text.into()),
        Column::Tags => {
            let tags: Vec<&str> = text.split(',').map(str::trim).filter(|t| !t.is_empty()).collect();
            return Some(Msg::run("glyph.setTags", json!({ "glyphs": glyphs, "tags": tags })));
        }
        Column::Components => {
            let params = |glyph: &String| json!({ "glyph": glyph, "layer": master, "components": text.trim() });
            return match glyphs.as_slice() {
                [] => None,
                [glyph] => Some(Msg::run("glyph.setComponents", params(glyph))),
                several => {
                    let steps: Vec<_> = several.iter().map(|g| json!({ "command": "glyph.setComponents", "params": params(g) })).collect();
                    Some(Msg::run("command.batch", json!({ "label": "Set Components", "steps": steps })))
                }
            };
        }
        _ => return None,
    };
    glyph_props::command(&GlyphPropsModel { glyphs, master, ..GlyphPropsModel::default() }, &field)
}

/// What the field of a text cell reported.
pub struct CellEdit {
    pub row: usize,
    pub column: Column,
    /// The text the field showed, and the one it committed.
    pub shown: String,
    pub text: Option<String>,
    /// Return ended the edit.
    pub submitted: bool,
    pub escaped: bool,
    /// Tab (`true`) or Shift+Tab left the field.
    pub tab: Option<bool>,
}

/// The messages for `edit`: the value with the command that sets it, then, after Return with one glyph selected, the
/// same cell one row down, or after Tab the next or previous text cell of the row.
pub fn messages(model: &FontViewModel, edit: &CellEdit) -> Vec<Msg> {
    let mut out = Vec::new();
    if edit.escaped && model.draft.is_some() {
        out.push(Msg::View(ViewMsg::FontViewCell(None)));
    }
    if let Some(text) = edit.text.as_ref().filter(|t| **t != edit.shown)
        && let Some(msg) = command(model, edit.row, edit.column, text)
        && let Some(glyph) = model.name(edit.row)
    {
        let draft = CellDraft { glyph: glyph.to_string(), column: edit.column, text: text.clone(), error: None };
        out.push(Msg::View(ViewMsg::FontViewCell(Some(draft))));
        out.push(msg);
    }
    if let Some(forward) = edit.tab {
        let cells: Vec<Column> = model.columns.iter().copied().filter(|c| is_text(model, edit.row, *c)).collect();
        let at = cells.iter().position(|c| *c == edit.column);
        let to = at.and_then(|at| if forward { at.checked_add(1) } else { at.checked_sub(1) }).and_then(|at| cells.get(at)).copied();
        out.extend(to.map(|column| Msg::View(ViewMsg::FontViewEditCell(Some(column)))));
    }
    if edit.submitted && model.selected.len() <= 1 {
        let next = model.visible.iter().skip_while(|r| **r != edit.row).nth(1).and_then(|&r| model.name(r));
        if let Some(glyph) = next {
            out.push(Msg::View(ViewMsg::FontViewNextCell { glyph: glyph.to_string(), column: edit.column }));
        }
    }
    out
}
