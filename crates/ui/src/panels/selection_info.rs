//! The Selection Info panel (`window.selectionInfo`, Edit › Info for Selection): every Glyph Properties field of the
//! selected glyphs, including the ones only it edits (colours, sort name, case, writing direction, note, metrics
//! keys). With Font View showing it describes the Font View selection, else the glyph the glyph panels show.
//!
//! The fields follow the inspector's rules ([`super::font_view::inspector`]): *Multiple Values* where the glyphs
//! differ, one command for every glyph per edit. The sort name check box shows whether the glyphs store their own
//! sort name; turning it off returns them to the default sort order (`glyph.setSortName` with `null`).

use craft::behavior::Mixed;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::TextField;
use egui::Ui;

use super::font_view::inspector::{InspectorWords, export_box, tags_field, text_field, text_row};
use crate::app::Msg;
use crate::app::models::glyph_props::{Edit, GlyphPropsModel, PropField, Shown, command};

/// Lines the note field shows.
const NOTE_LINES: usize = 3;

craft::ui::words! {
    /// The words of the Selection Info panel besides the inspector's.
    pub struct SelectionInfoWords {
        no_selection: "fontcraftSelectionInfo.noSelection" = "No glyph is selected";
        color: "fontcraftSelectionInfo.color" = "Color";
        layer_color: "fontcraftSelectionInfo.layerColor" = "Layer Color";
        case: "fontcraftSelectionInfo.case" = "Case";
        direction: "fontcraftSelectionInfo.direction" = "Direction";
        sort_name: "fontcraftSelectionInfo.sortName" = "Sort Name";
        /// The sort name's check box.
        custom_sort_name: "fontcraftSelectionInfo.customSortName" = "Use a custom sort name";
        note: "fontcraftSelectionInfo.note" = "Note";
        left_key: "fontcraftSelectionInfo.leftKey" = "Left Metrics Key";
        right_key: "fontcraftSelectionInfo.rightKey" = "Right Metrics Key";
        width_key: "fontcraftSelectionInfo.widthKey" = "Width Metrics Key";
        /// No colour label.
        no_color: "fontcraftSelectionInfo.noColor" = "None";
        /// The twelve colour labels, in index order.
        red: "fontcraftSelectionInfo.red" = "Red";
        orange: "fontcraftSelectionInfo.orange" = "Orange";
        brown: "fontcraftSelectionInfo.brown" = "Brown";
        yellow: "fontcraftSelectionInfo.yellow" = "Yellow";
        light_green: "fontcraftSelectionInfo.lightGreen" = "Light Green";
        dark_green: "fontcraftSelectionInfo.darkGreen" = "Dark Green";
        cyan: "fontcraftSelectionInfo.cyan" = "Cyan";
        blue: "fontcraftSelectionInfo.blue" = "Blue";
        purple: "fontcraftSelectionInfo.purple" = "Purple";
        pink: "fontcraftSelectionInfo.pink" = "Pink";
        light_gray: "fontcraftSelectionInfo.lightGray" = "Light Gray";
        dark_gray: "fontcraftSelectionInfo.darkGray" = "Dark Gray";
    }
}

/// What the panel reads and sends.
pub struct Props<'a> {
    pub model: &'a Tracked<GlyphPropsModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let env = Env::of(ui);
    let words = env.words::<SelectionInfoWords>();
    let shared = env.words::<InspectorWords>();
    let model: &GlyphPropsModel = props.model;
    let mut edits = Vec::new();
    Stack::new().show(ui, |ui, _| {
        if model.glyphs.is_empty() {
            Text::new(words.no_selection.clone()).show(ui);
        }
        fields(ui, model, &words, &shared, &mut edits);
    });
    props.sent.extend(edits.iter().filter_map(|e| command(model, e)));
}

fn fields(ui: &mut Ui, model: &GlyphPropsModel, words: &SelectionInfoWords, shared: &InspectorWords, edits: &mut Vec<Edit>) {
    let any = !model.glyphs.is_empty();
    let one = model.glyphs.len() == 1;
    let multiple = &shared.multiple;
    text_row(ui, shared.name.clone(), model, PropField::Name, one, multiple, edits);
    if let Some(on) = export_box(ui, shared.exports.clone(), model.export, any) {
        edits.push(Edit::Export(on));
    }
    for (label, field) in [(words.color.clone(), PropField::Color), (words.layer_color.clone(), PropField::LayerColor)] {
        if let Some(value) = color_field(ui, label, &model.field(field), any, words) {
            edits.push(Edit::Text(field, value));
        }
    }
    edits.extend(tags_field(ui, shared.tags.clone(), model.tags.as_deref(), any, multiple));
    let texts = [
        (shared.unicode.clone(), PropField::Unicode),
        (shared.production_name.clone(), PropField::ProductionName),
        (shared.script.clone(), PropField::Script),
        (shared.category.clone(), PropField::Category),
        (shared.sub_category.clone(), PropField::SubCategory),
        (words.case.clone(), PropField::Case),
        (words.direction.clone(), PropField::Direction),
    ];
    for (label, field) in texts {
        text_row(ui, label, model, field, any, multiple, edits);
    }
    Field::new(words.sort_name.clone()).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            let stored = model.sort_name_stored.map_or(Mixed::Mixed, Mixed::Same);
            let out = Checkbox::new(Bind::given(stored)).label(words.custom_sort_name.clone()).enabled(any).show(ui);
            if out.events.committed() == Some(&false) && model.sort_name_stored != Some(false) {
                edits.push(Edit::ClearSortName);
            }
            if let Some(text) = text_field(ui, words.sort_name.clone(), &model.field(PropField::SortName), any, multiple) {
                edits.push(Edit::Text(PropField::SortName, text));
            }
        });
    });
    Field::new(words.note.clone()).show(ui, |ui, _| {
        let shown = model.field(PropField::Note);
        let mut field = TextField::new(words.note.clone(), Bind::given(shown.text.clone())).lines(NOTE_LINES).enabled(any);
        if shown.mixed {
            field = field.placeholder(multiple.clone());
        }
        if let Some(text) = field.show(ui).events.committed().filter(|_| any) {
            edits.push(Edit::Text(PropField::Note, text.clone()));
        }
    });
    let keys = [
        (words.left_key.clone(), PropField::LeftKey),
        (words.right_key.clone(), PropField::RightKey),
        (words.width_key.clone(), PropField::WidthKey),
        (shared.left_group.clone(), PropField::LeftGroup),
        (shared.right_group.clone(), PropField::RightGroup),
    ];
    for (label, field) in keys {
        text_row(ui, label, model, field, any, multiple, edits);
    }
}

/// The colour label names, none first, with their field values.
fn color_entries(words: &SelectionInfoWords, shown: &Shown) -> Vec<Entry<String>> {
    let named = [
        &words.red,
        &words.orange,
        &words.brown,
        &words.yellow,
        &words.light_green,
        &words.dark_green,
        &words.cyan,
        &words.blue,
        &words.purple,
        &words.pink,
        &words.light_gray,
        &words.dark_gray,
    ];
    let mut out = vec![Entry::new(String::new(), words.no_color.clone())];
    out.extend(named.iter().enumerate().map(|(i, label)| Entry::new(i.to_string(), (*label).clone())));
    // A custom colour is listed by its `#RRGGBBAA`, so the field can show it.
    if shown.text.starts_with('#') {
        out.push(Entry::new(shown.text.clone(), Label::from(shown.text.clone())));
    }
    out
}

/// A colour label menu; returns the value picked.
fn color_field(ui: &mut Ui, label: Label, shown: &Shown, enabled: bool, words: &SelectionInfoWords) -> Option<String> {
    let entries = color_entries(words, shown);
    let value = if shown.mixed { Mixed::Mixed } else { Mixed::Same(shown.text.clone()) };
    let mut picked = None;
    Field::new(label.clone()).show(ui, |ui, _| {
        let out = Select::new(label.clone(), Bind::given(value.clone()), &entries).enabled(enabled).show(ui);
        picked = out.events.committed().cloned().filter(|v| enabled && (shown.mixed || *v != shown.text));
    });
    picked
}

#[cfg(test)]
#[path = "selection_info_tests.rs"]
mod tests;
