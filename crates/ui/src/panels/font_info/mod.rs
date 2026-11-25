//! Font Info: the tab strip (Font, Masters, Exports, Features, Other Settings, Notes) over the shown tab.
//!
//! | Module | Draws |
//! |---|---|
//! | `mod.rs` | the panel, the tab strip and the shared words |
//! | [`rows`] | the section headings with their plus buttons and the field and parameter rows |
//! | [`param_value`] | the value editor of each row |
//! | [`font_tab`] | General (Family Name, Units per Em, Version, Creation Date, added fields), Axes, Custom Parameters |
//! | [`other_tab`] | Other Settings |
//! | [`notes_tab`] | the font note, its Markdown coloured |
//! | [`instance_preview`] | the Exports tab's preview of the selected instance |
//! | [`exports_tab`] | Exports: the export list and the selected entries' fields and parameters |
//! | [`exported_names`] | the Exports tab's As Exported section |
//! | [`features_tab`] | Features: the sidebar of prefixes, classes and features, and the shown entry |
//! | [`code_editor`] | the Features tab's code field and its diagnostics |
//! | [`masters_tab`] | Masters: the master list and the selected masters' sections |
//!
//! Each tab scrolls in its own pane under the tab strip; the Masters and Exports tabs scroll their list and their
//! fields apart.
//!
//! A tab click posts `font.info.selectTab`; every edit posts an engine command.

pub mod code_editor;
pub mod code_find;
#[cfg(test)]
mod dialogs_tests;
pub mod expanded_code;
pub mod exported_names;
pub mod exports_tab;
pub mod feature_previews;
pub mod features_tab;
pub mod font_tab;
pub mod instance_preview;
pub mod masters_tab;
pub mod notes_tab;
pub mod other_tab;
pub mod param_value;
#[cfg(test)]
mod previews_tests;
pub mod rows;
#[cfg(test)]
mod tabs_tests;
#[cfg(test)]
mod tests;

use std::borrow::Cow;

use craft::behavior::{SelectionChange, keyed};
use craft::ui::contract::{CollectionEvent, Show as _, ShowCollection as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Stack;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::tabs::{Tab, TabKind, TabStrip};
use egui::Ui;
use serde_json::json;

use crate::app::Msg;
use crate::app::models::font_info::FontInfoModel;
use crate::panels::widgets::scrolling;
use crate::shell::font_info::FontInfoTab;

craft::ui::words! {
    /// The words of the Font Info panel and its rows.
    pub struct FontInfoWords {
        /// The tab strip's name.
        tabs: "fontcraftFontInfo.tabs" = "Font Info tabs";
        font: "fontcraftFontInfo.font" = "Font";
        masters: "fontcraftFontInfo.masters" = "Masters";
        exports: "fontcraftFontInfo.exports" = "Exports";
        features: "fontcraftFontInfo.features" = "Features";
        other: "fontcraftFontInfo.other" = "Other Settings";
        notes: "fontcraftFontInfo.notes" = "Notes";
        /// Shown without an open font.
        no_font: "fontcraftFontInfo.noFont" = "No font is open.";
        /// Shown on a tab whose editor is not built yet.
        not_yet: "fontcraftFontInfo.notYet" = "This tab is not available yet.";
        /// A parameter's check box.
        use_parameter: "fontcraftFontInfo.useParameter" = "Use this parameter";
        /// A row's minus button.
        remove_row: "fontcraftFontInfo.removeRow" = "Remove this row";
        /// The field of targets that hold different values.
        multiple: "fontcraftFontInfo.multiple" = "Multiple Values";
        code_line: "fontcraftFontInfo.codeLine" = "Line";
        code_problem: "fontcraftFontInfo.codeProblem" = "Problem in this parameter's feature code";
        /// A menu whose value is none of its entries.
        choose: "fontcraftFontInfo.choose" = "Choose…";
        /// The button of a value edited in its own dialog.
        edit: "fontcraftFontInfo.edit" = "Edit…";
        edit_tip: "fontcraftFontInfo.editTip" = "Edit the value in its dialog";
        /// The tooltip of the mark of a row only some of the selected masters or exports hold.
        some_lack: "fontcraftFontInfo.someLack" = "Not every selected entry has this row. Editing it adds it to the others.";
        /// The language menu of a localizable field.
        language: "fontcraftFontInfo.language" = "Language";
        add_language: "fontcraftFontInfo.addLanguage" = "Add a language";
        remove_language: "fontcraftFontInfo.removeLanguage" = "Remove this language";
    }
}

impl FontInfoTab {
    /// The tab strip's label.
    pub fn label(self, words: &FontInfoWords) -> Label {
        match self {
            FontInfoTab::Font => words.font.clone(),
            FontInfoTab::Masters => words.masters.clone(),
            FontInfoTab::Exports => words.exports.clone(),
            FontInfoTab::Features => words.features.clone(),
            FontInfoTab::Other => words.other.clone(),
            FontInfoTab::Notes => words.notes.clone(),
        }
    }
}

/// What the Font Info panel reads and sends.
pub struct Props<'a> {
    pub model: &'a Tracked<FontInfoModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<FontInfoWords>();
    let model: &FontInfoModel = props.model;
    if !model.open {
        Text::new(words.no_font.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    let sent = &mut *props.sent;
    Stack::new().fill().show(ui, |ui, _| {
        if let Some(tab) = tab_strip(ui, model.tab, &words) {
            sent.push(Msg::Shell(CommandRef::with_params("font.info.selectTab", json!({ "tab": tab.name() }))));
        }
        match model.tab {
            FontInfoTab::Font => scrolling(ui, |ui| font_tab::view(ui, model, sent)),
            FontInfoTab::Other => scrolling(ui, |ui| other_tab::view(ui, &model.other, sent)),
            FontInfoTab::Notes => scrolling(ui, |ui| notes_tab::view(ui, &model.note, sent)),
            FontInfoTab::Exports => exports_tab::view(ui, &model.exports, &model.masters, sent),
            FontInfoTab::Features => features_tab::view(ui, &model.features, sent),
            FontInfoTab::Masters => masters_tab::view(ui, &model.masters_tab, &model.masters, sent),
        }
    });
}

/// The tab strip; the tab the user picked, if another than `shown`.
fn tab_strip(ui: &mut Ui, shown: FontInfoTab, words: &FontInfoWords) -> Option<FontInfoTab> {
    let labels: Vec<(FontInfoTab, Label)> = FontInfoTab::ALL.iter().map(|t| (*t, t.label(words))).collect();
    let source = keyed(&labels, |(tab, _)| *tab).with_text(|(_, label)| Cow::Borrowed(label.as_str()));
    let out = TabStrip::new(words.tabs.clone()).kind(TabKind::Text).shown(shown).show_items(ui, &source, &mut |tab, _| Tab::new(tab.label(words)));
    let picked = out.response.events.iter().rev().find_map(|event| match event {
        CollectionEvent::Selection(SelectionChange::Set(keys)) => keys.last().copied(),
        _ => None,
    });
    picked.or(out.picked).filter(|tab| *tab != shown)
}
