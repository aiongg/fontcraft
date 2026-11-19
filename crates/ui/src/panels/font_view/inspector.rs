//! The glyph inspector (`fontView.inspector`): the properties of the glyphs selected in Font View, at the foot of
//! the sidebar. Its title row shows and hides the fields (`fontView.toggleInspector`).
//!
//! | Row | Shows |
//! |---|---|
//! | Name | the glyph name; read-only with several glyphs |
//! | Metrics | LSB, RSB and width, or TSB, BSB, vertical origin and vertical width ([`super::metrics`]) |
//! | Kerning | the left and right kerning groups |
//! | Exports | a check box, a dash when the glyphs differ |
//! | Unicode | the code points, with the disclosure of the extra rows (`fontView.toggleInspectorDetails`) |
//! | Production Name, Script, Category, Subcategory | the extra rows |
//! | Tags | the tags as chips; a typed tag is added, a chip's cross removes it |
//!
//! Each edit runs the command `app::models::glyph_props::command` names, on every described glyph at once.

use craft::behavior::Mixed;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::controls::disclosure::Disclosure;
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use craft::ui::text::{ChipField, TextField};
use egui::Ui;
use serde_json::json;

use super::metrics;
use crate::app::Msg;
use crate::app::models::glyph_props::{Edit, GlyphPropsModel, MAX_TAG_CHARS, PropField, Shown, command};

craft::ui::words! {
    /// The words of the glyph inspector and the Selection Info panel.
    pub struct InspectorWords {
        /// The inspector's title.
        title: "fontcraftInspector.title" = "Glyph Properties";
        /// The placeholder of a field whose glyphs differ.
        multiple: "fontcraftInspector.multiple" = "Multiple Values";
        name: "fontcraftInspector.name" = "Name";
        kerning: "fontcraftInspector.kerning" = "Kerning";
        left_group: "fontcraftInspector.leftGroup" = "Left Group";
        right_group: "fontcraftInspector.rightGroup" = "Right Group";
        exports: "fontcraftInspector.exports" = "Exports";
        unicode: "fontcraftInspector.unicode" = "Unicode";
        /// The disclosure of the extra rows.
        details: "fontcraftInspector.details" = "Show Extra Glyph Properties";
        production_name: "fontcraftInspector.productionName" = "Production Name";
        script: "fontcraftInspector.script" = "Script";
        category: "fontcraftInspector.category" = "Category";
        sub_category: "fontcraftInspector.subCategory" = "Subcategory";
        tags: "fontcraftInspector.tags" = "Tags";
    }
}

/// Draws the inspector over `model` and posts what its edits run.
pub fn show(ui: &mut Ui, model: &GlyphPropsModel, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<InspectorWords>();
    let mut open = !model.hidden;
    let mut edits = Vec::new();
    let mut shell = Vec::new();
    Section::new(words.title.clone()).open(Bind::bound(&mut open)).show(ui, |ui, _| fields(ui, model, &words, &mut edits, &mut shell));
    if open == model.hidden {
        shell.push(Msg::Shell(CommandRef::with_params("fontView.toggleInspector", json!({ "visible": open }))));
    }
    sent.extend(edits.iter().filter_map(|e| command(model, e)));
    sent.extend(shell);
}

fn fields(ui: &mut Ui, model: &GlyphPropsModel, words: &InspectorWords, edits: &mut Vec<Edit>, shell: &mut Vec<Msg>) {
    let any = !model.glyphs.is_empty();
    let one = model.glyphs.len() == 1;
    text_row(ui, words.name.clone(), model, PropField::Name, one, &words.multiple, edits);
    metrics::show(ui, model, any, &words.multiple, edits, shell);
    Field::new(words.kerning.clone()).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            for (label, field) in [(words.left_group.clone(), PropField::LeftGroup), (words.right_group.clone(), PropField::RightGroup)] {
                if let Some(text) = text_field(ui, label, &model.field(field), any, &words.multiple) {
                    edits.push(Edit::Text(field, text));
                }
            }
        });
    });
    if let Some(on) = export_box(ui, words.exports.clone(), model.export, any) {
        edits.push(Edit::Export(on));
    }
    Field::new(words.unicode.clone()).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            if let Some(text) = text_field(ui, words.unicode.clone(), &model.field(PropField::Unicode), any, &words.multiple) {
                edits.push(Edit::Text(PropField::Unicode, text));
            }
            let out = Disclosure::new(Bind::given(model.details)).label(words.details.clone()).tooltip(words.details.clone()).show(ui);
            if let Some(expanded) = out.events.committed() {
                shell.push(Msg::Shell(CommandRef::with_params("fontView.toggleInspectorDetails", json!({ "expanded": expanded }))));
            }
        });
    });
    if model.details {
        let rows = [
            (words.production_name.clone(), PropField::ProductionName),
            (words.script.clone(), PropField::Script),
            (words.category.clone(), PropField::Category),
            (words.sub_category.clone(), PropField::SubCategory),
        ];
        for (label, field) in rows {
            text_row(ui, label, model, field, any, &words.multiple, edits);
        }
    }
    edits.extend(tags_field(ui, words.tags.clone(), model.tags.as_deref(), any, &words.multiple));
}

/// A text field: *Multiple Values* as its placeholder while the glyphs differ. Returns the committed text.
pub fn text_field(ui: &mut Ui, label: Label, shown: &Shown, enabled: bool, multiple: &Label) -> Option<String> {
    let mut field = TextField::new(label, Bind::given(shown.text.clone())).enabled(enabled);
    if shown.mixed {
        field = field.placeholder(multiple.clone());
    }
    field.show(ui).events.committed().cloned().filter(|_| enabled)
}

/// A labelled text row; pushes the edit it commits.
pub fn text_row(ui: &mut Ui, label: Label, model: &GlyphPropsModel, field: PropField, enabled: bool, multiple: &Label, edits: &mut Vec<Edit>) {
    Field::new(label.clone()).show(ui, |ui, _| {
        if let Some(text) = text_field(ui, label.clone(), &model.field(field), enabled, multiple) {
            edits.push(Edit::Text(field, text));
        }
    });
}

/// The Exports check box: a dash while the glyphs differ. Returns the value a click set.
pub fn export_box(ui: &mut Ui, label: Label, export: Option<bool>, enabled: bool) -> Option<bool> {
    let value = export.map_or(Mixed::Mixed, Mixed::Same);
    let out = Checkbox::new(Bind::given(value)).label(label).enabled(enabled).show(ui);
    out.events.committed().copied().filter(|_| enabled)
}

/// The Tags field (`fontView.inspector.tagsField`): the shared tags as chips, *Multiple Values* when the glyphs'
/// sets differ, and the draft that becomes a tag on a comma, a semicolon or Return. A removed chip removes its tag.
pub fn tags_field(ui: &mut Ui, label: Label, tags: Option<&[String]>, enabled: bool, multiple: &Label) -> Vec<Edit> {
    let before: Vec<String> = tags.map(<[String]>::to_vec).unwrap_or_default();
    let mut field = ChipField::new(label, Bind::given(before.clone())).enabled(enabled);
    if tags.is_none() {
        field = field.placeholder(multiple.clone());
    }
    let out = field.show(ui);
    let Some(after) = out.edit.events.committed().filter(|_| enabled) else { return Vec::new() };
    let mut edits = Vec::new();
    let mut added: Vec<String> = after
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty() && !before.iter().any(|b| b == t))
        .map(|t| t.chars().take(MAX_TAG_CHARS).collect())
        .collect();
    let mut seen = std::collections::HashSet::new();
    added.retain(|t| seen.insert(t.clone()));
    if !added.is_empty() {
        edits.push(Edit::AddTags(added));
    }
    edits.extend(before.iter().filter(|b| !after.contains(b)).map(|b| Edit::RemoveTag(b.clone())));
    edits
}

#[cfg(test)]
#[path = "inspector_tests.rs"]
pub(crate) mod tests;
