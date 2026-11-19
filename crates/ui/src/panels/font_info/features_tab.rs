//! The Features tab: the font's prefixes, classes and features in a sidebar of three groups, and the shown entry's
//! code, stylistic-set names and notes beside it.
//!
//! | Control | Posts |
//! |---|---|
//! | plus beside Prefix, Classes | `font.features.addPrefix`, `font.features.addClass` |
//! | plus beside Features | `font.info.openDialog` with the Add Feature picker |
//! | a click, Cmd-click, Shift-click on an entry | `font.info.features.select` |
//! | double-click on an entry, then a new name | `font.features.rename` |
//! | drag an entry | `font.features.reorder` |
//! | minus, Update, Compile under the sidebar | `font.features.remove`, `font.features.update`, `font.features.compile` |
//! | Generate Feature Automatically, Update Entry | `font.features.setAutomatic`, `font.features.updateEntry` |
//! | the stylistic set name rows | `font.features.setLabel`, `font.features.removeLabel` |
//! | the code ([`code_editor`]) | `font.features.setCode` when it loses the keyboard; a Fix button `font.features.applyFix` |
//! | the notes field | `font.features.setNotes` when it loses the keyboard |
//! | Snippets | `font.features.insertSnippet` (at the end of the code), `font.features.showSnippetFolder` |
//!
//! An entry with an error from the last compile shows a mark in the sidebar while the font is unchanged since.

use std::borrow::Cow;

use craft::behavior::{EditEvent, GesturePhase, IconId, Selection, SelectionChange, keyed};
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{
    Bind, CollectionEvent, Entry, Props as _, ReorderRule, Show as _, ShowCollection as _, ShowContainer as _, Trigger, request_rename,
};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, RowInk};
use craft::ui::menu::Menu;
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::style::TypeRole;
use craft::ui::text::TextField;
use egui::Ui;
use fontcraft_engine::model::{DEFAULT_LANGUAGE, LANGUAGES};
use serde_json::{Value, json};

use super::code_editor;
use super::rows;
use crate::app::Msg;
use crate::app::models::font_features::{EntryPane, FeaturesModel, GroupModel, SnippetsModel};
use crate::shell::font_info::{InfoDialog, open_params};
use crate::shell::font_info_tabs::{EntryKey, EntryKind, select_entries};

craft::ui::words! {
    /// The words of the Features tab.
    pub struct FeaturesWords {
        prefix: "fontcraftFeatures.prefix" = "Prefix";
        classes: "fontcraftFeatures.classes" = "Classes";
        features: "fontcraftFeatures.features" = "Features";
        add_prefix: "fontcraftFeatures.addPrefix" = "Add a prefix";
        add_class: "fontcraftFeatures.addClass" = "Add a class";
        add_feature: "fontcraftFeatures.addFeature" = "Add a feature";
        none: "fontcraftFeatures.none" = "None";
        more: "fontcraftFeatures.more" = "more entries are not listed";
        /// The sidebar mark of an entry with an error.
        error_mark: "fontcraftFeatures.errorMark" = "!";
        has_error: "fontcraftFeatures.hasError" = "The last compile found an error here";
        remove: "fontcraftFeatures.remove" = "Delete the selected entries";
        update: "fontcraftFeatures.update" = "Update";
        update_tip: "fontcraftFeatures.updateTip" = "Regenerate every automatic prefix, class and feature, and add the ones the glyph names call for";
        compile: "fontcraftFeatures.compile" = "Compile";
        compile_tip: "fontcraftFeatures.compileTip" = "Compile the feature code to preview it in Edit View";
        empty: "fontcraftFeatures.empty" = "Add a prefix, class or feature with a plus button, or press Update.";
        automatic: "fontcraftFeatures.automatic" = "Generate Feature Automatically";
        update_entry: "fontcraftFeatures.updateEntry" = "Update Entry";
        update_entry_tip: "fontcraftFeatures.updateEntryTip" = "Regenerate this entry's code";
        spec: "fontcraftFeatures.spec" = "Specification";
        language: "fontcraftFeatures.language" = "Language";
        set_name: "fontcraftFeatures.setName" = "Stylistic Set Name";
        set_name_hint: "fontcraftFeatures.setNameHint" = "Name of the stylistic set";
        remove_name: "fontcraftFeatures.removeName" = "Remove this name";
        default_language: "fontcraftFeatures.defaultLanguage" = "Default";
        notes: "fontcraftFeatures.notes" = "Notes";
        notes_hint: "fontcraftFeatures.notesHint" = "Notes about this entry, kept in the source and never exported";
        snippets: "fontcraftFeatures.snippets" = "Snippets";
        show_folder: "fontcraftFeatures.showFolder" = "Show Snippet Folder";
    }
}

/// Lines the notes field shows.
const NOTES_LINES: usize = 4;
/// The name the plus button gives a new class, numbered when taken.
const NEW_CLASS: &str = "NewClass";
/// Most numbers tried for a free class name.
const MAX_CLASS_NUMBER: usize = 10_000;

impl EntryKind {
    fn heading(self, words: &FeaturesWords) -> Label {
        match self {
            EntryKind::Prefix => words.prefix.clone(),
            EntryKind::Class => words.classes.clone(),
            EntryKind::Feature => words.features.clone(),
        }
    }

    fn add(self, words: &FeaturesWords) -> Label {
        match self {
            EntryKind::Prefix => words.add_prefix.clone(),
            EntryKind::Class => words.add_class.clone(),
            EntryKind::Feature => words.add_feature.clone(),
        }
    }
}

/// How an entry shows: a class with its `@`.
fn shown_name(kind: EntryKind, name: &str) -> String {
    if kind == EntryKind::Class { format!("@{name}") } else { name.to_string() }
}

pub fn view(ui: &mut Ui, model: &FeaturesModel, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<FeaturesWords>();
    Row::new().show(ui, |ui, _| {
        Stack::new().id_salt("fontcraftFeatures.sidebar").show(ui, |ui, _| {
            for group in &model.groups {
                sidebar_group(ui, model, group, &words, sent);
            }
            bottom_buttons(ui, model, &words, sent);
        });
        Stack::new().id_salt("fontcraftFeatures.entry").fill_width().show(ui, |ui, _| match &model.entry {
            Some(entry) => entry_pane(ui, entry, &model.snippets, &words, sent),
            None => {
                Text::new(words.empty.clone()).shade(Shade::Muted).show(ui);
            }
        });
    });
}

/// The first free class name of the form `NewClass`, `NewClass1`, `NewClass2`…
fn free_class_name(group: &GroupModel) -> String {
    let taken = |name: &str| group.entries.iter().any(|c| c.name == name);
    (0..MAX_CLASS_NUMBER)
        .map(|n| if n == 0 { NEW_CLASS.to_string() } else { format!("{NEW_CLASS}{n}") })
        .find(|name| !taken(name))
        .unwrap_or_else(|| format!("{NEW_CLASS}{}", group.entries.len()))
}

/// The value of an event that ended: a commit or a set.
fn done<T>(event: &EditEvent<T>) -> Option<&T> {
    match event.phase() {
        Some(GesturePhase::Commit) | None => event.value(),
        _ => None,
    }
}

/// One group: its heading with the plus button, then its entries in a list that selects, renames and reorders.
fn sidebar_group(ui: &mut Ui, model: &FeaturesModel, group: &GroupModel, words: &FeaturesWords, sent: &mut Vec<Msg>) {
    let kind = group.kind;
    let mut posted = Vec::new();
    let plus = rows::section(ui, kind.name(), kind.heading(words), kind.add(words), |ui| {
        if group.entries.is_empty() {
            Text::new(words.none.clone()).shade(Shade::Muted).show(ui);
            return;
        }
        let entries = &group.entries;
        let key_of = |at: usize| entries.get(at).map(|e| EntryKey { kind, name: e.name.clone(), index: at });
        let mine: Vec<usize> = model.selected.iter().filter(|k| k.kind == kind).map(|k| k.index).collect();
        let lead = model.selected.last().filter(|k| k.kind == kind).map(|k| k.index);
        let before = Selection::new(mine, lead, lead);
        let mut after = before.clone();
        let positions: Vec<usize> = (0..entries.len()).collect();
        let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(entries.get(*at).map_or_else(String::new, |e| shown_name(kind, &e.name))));
        let out = List::<usize>::new(kind.heading(words))
            .header(false)
            .natural_height(true)
            .rename(true)
            .rename_draft(|at: &usize| entries.get(*at).map(|e| e.name.clone()))
            .reorder(ReorderRule::new())
            .selection(Bind::bound(&mut after))
            .show_items(ui, &source, &mut |at, icx| {
                let Some(e) = entries.get(*at) else { return ListRow::new(Label::Empty) };
                let mut row = ListRow::new(shown_name(kind, &e.name)).state(icx.row.clone());
                if e.disabled {
                    row = row.ink(RowInk::Dimmed);
                }
                if e.error {
                    row = row.trailing_strong(words.error_mark.clone(), words.has_error.clone());
                }
                row
            });
        let toggled = out.events.iter().any(|e| matches!(e, CollectionEvent::Selection(SelectionChange::Toggle(_))));
        if after.items != before.items || after.lead != before.lead {
            let mut picked: Vec<usize> = after.items.clone();
            if let Some(lead) = after.lead
                && let Some(at) = picked.iter().position(|i| *i == lead)
            {
                picked.remove(at);
                picked.push(lead);
            }
            let mut keys: Vec<EntryKey> = if toggled { model.selected.iter().filter(|k| k.kind != kind).cloned().collect() } else { Vec::new() };
            keys.extend(picked.into_iter().filter_map(key_of));
            posted.push(Msg::Shell(select_entries(&keys)));
        }
        for event in &out.events {
            match event {
                CollectionEvent::Activate(at) => request_rename(ui.ctx(), out.response.id, at),
                CollectionEvent::Rename(edit) => {
                    if let Some(rename) = done(edit)
                        && let Some(key) = key_of(rename.key)
                    {
                        let name = rename.name.trim();
                        if !name.is_empty() && name != key.name {
                            posted.push(Msg::run("font.features.rename", key.with(&[("newName", json!(name))])));
                        }
                    }
                }
                CollectionEvent::Reorder(edit) => {
                    if let Some(moved) = done(edit) {
                        let mut order: Vec<usize> = (0..entries.len()).filter(|i| !moved.keys.contains(i)).collect();
                        let to = moved.to.min(order.len());
                        for (n, key) in moved.keys.iter().enumerate() {
                            order.insert(to.saturating_add(n).min(order.len()), *key);
                        }
                        if order.iter().enumerate().any(|(i, k)| i != *k) {
                            posted.push(Msg::run("font.features.reorder", json!({ "kind": kind.name(), "order": order })));
                        }
                    }
                }
                _ => {}
            }
        }
        if group.hidden > 0 {
            Text::new(format!("{} {}", group.hidden, words.more.as_str())).shade(Shade::Muted).show(ui);
        }
    });
    if plus {
        posted.push(match kind {
            EntryKind::Prefix => Msg::run("font.features.addPrefix", json!({})),
            EntryKind::Class => Msg::run("font.features.addClass", json!({ "name": free_class_name(group) })),
            EntryKind::Feature => Msg::Shell(CommandRef::with_params("font.info.openDialog", open_params(&InfoDialog::FeaturePicker))),
        });
    }
    sent.extend(posted);
}

/// The minus, Update and Compile buttons under the sidebar.
fn bottom_buttons(ui: &mut Ui, model: &FeaturesModel, words: &FeaturesWords, sent: &mut Vec<Msg>) {
    Row::new().show(ui, |ui, _| {
        let minus =
            IconButton::new(IconId::new("minus")).label(words.remove.clone()).tooltip(words.remove.clone()).enabled(!model.selected.is_empty());
        if minus.show(ui).clicked() && !model.selected.is_empty() {
            let entries: Vec<Value> = model.selected.iter().map(|k| Value::Object(k.params())).collect();
            sent.push(Msg::run("font.features.remove", json!({ "entries": entries })));
        }
        if Button::new(words.update.clone()).tooltip(words.update_tip.clone()).show(ui).clicked() {
            sent.push(Msg::run("font.features.update", json!({})));
        }
        if Button::new(words.compile.clone()).tooltip(words.compile_tip.clone()).show(ui).clicked() {
            sent.push(Msg::run("font.features.compile", model.master.as_ref().map_or_else(|| json!({}), |m| json!({ "master": m }))));
        }
    });
}

/// The shown entry: its title and specification link, the automatic check box, the stylistic set names, the code with
/// its diagnostics, the notes and the snippets menu.
fn entry_pane(ui: &mut Ui, entry: &EntryPane, snippets: &SnippetsModel, words: &FeaturesWords, sent: &mut Vec<Msg>) {
    let key = &entry.key;
    Text::new(shown_name(key.kind, &key.name)).role(TypeRole::Heading).show(ui);
    if let Some(url) = &entry.spec_url {
        Text::new(format!("{}: {url}", words.spec.as_str())).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
    Row::new().show(ui, |ui, _| {
        let check = Checkbox::new(Bind::given(entry.automatic)).label(words.automatic.clone()).id_salt(("fontcraftFeatures.automatic", key.index));
        if let Some(on) = check.show(ui).events.committed()
            && *on != entry.automatic
        {
            sent.push(Msg::run("font.features.setAutomatic", key.with(&[("automatic", json!(on))])));
        }
        if entry.automatic && Button::new(words.update_entry.clone()).tooltip(words.update_entry_tip.clone()).show(ui).clicked() {
            sent.push(Msg::run("font.features.updateEntry", Value::Object(key.params())));
        }
    });
    if entry.stylistic_set {
        label_rows(ui, entry, words, sent);
    }
    sent.extend(code_editor::view(ui, entry));
    let notes = TextField::new(words.notes.clone(), Bind::given(entry.notes.clone()))
        .lines(NOTES_LINES)
        .placeholder(words.notes_hint.clone())
        .id_salt(("fontcraftFeatures.notes", key.kind.name(), key.index, key.name.as_str()));
    if let Some(text) = notes.show(ui).events.committed()
        && *text != entry.notes
    {
        sent.push(Msg::run("font.features.setNotes", key.with(&[("notes", json!(text))])));
    }
    snippet_menu(ui, entry, snippets, words, sent);
}

/// The language a name row shows.
fn language_label(tag: &str, words: &FeaturesWords) -> Label {
    if tag == DEFAULT_LANGUAGE { words.default_language.clone() } else { Label::from(tag.to_string()) }
}

/// The languages a name row offers: its own, then those no other name of the set uses.
fn language_entries(current: &str, used: &[&str], words: &FeaturesWords) -> Vec<Entry<String>> {
    std::iter::once(DEFAULT_LANGUAGE)
        .chain(LANGUAGES.iter().map(|(tag, _)| *tag))
        .filter(|l| *l == current || !used.contains(l))
        .map(|tag| Entry::new(tag.to_string(), language_label(tag, words)))
        .collect()
}

/// The name rows of a stylistic set: per name a language menu, the name and a minus button; then a row for a new name
/// in a language the set has no name in yet.
fn label_rows(ui: &mut Ui, entry: &EntryPane, words: &FeaturesWords, sent: &mut Vec<Msg>) {
    let tag = json!(entry.key.name);
    let index = json!(entry.key.index);
    let used: Vec<&str> = entry.labels.iter().map(|(l, _)| l.as_str()).collect();
    let remove = |language: &str| Msg::run("font.features.removeLabel", json!({ "tag": tag, "index": index, "language": language }));
    let set =
        |language: &str, name: &str| Msg::run("font.features.setLabel", json!({ "tag": tag, "index": index, "language": language, "name": name }));
    for (at, (language, name)) in entry.labels.iter().enumerate() {
        Row::new().id_salt(("fontcraftFeatures.label", at)).show(ui, |ui, _| {
            let languages = language_entries(language, &used, words);
            if let Some(to) = Select::new(words.language.clone(), Bind::given(language.clone()), &languages).show(ui).events.committed()
                && to != language
            {
                sent.push(remove(language));
                sent.push(set(to, name));
            }
            if let Some(text) = TextField::new(words.set_name.clone(), Bind::given(name.clone())).show(ui).events.committed()
                && text != name
            {
                sent.push(if text.trim().is_empty() { remove(language) } else { set(language, text) });
            }
            if IconButton::new(IconId::new("minus")).label(words.remove_name.clone()).tooltip(words.remove_name.clone()).show(ui).clicked() {
                sent.push(remove(language));
            }
        });
    }
    let Some(free) = std::iter::once(DEFAULT_LANGUAGE).chain(LANGUAGES.iter().map(|(tag, _)| *tag)).find(|l| !used.contains(l)) else { return };
    Row::new().id_salt(("fontcraftFeatures.newLabel", entry.key.index, used.len())).show(ui, |ui, _| {
        let languages = language_entries(free, &used, words);
        let picked = Select::new(words.language.clone(), Bind::kept().reset(free.to_string()), &languages).show(ui);
        let language = picked.value().cloned().unwrap_or_else(|| free.to_string());
        let field = TextField::new(words.set_name.clone(), Bind::given(String::new())).placeholder(words.set_name_hint.clone());
        if let Some(text) = field.show(ui).events.committed()
            && !text.trim().is_empty()
        {
            sent.push(set(&language, text));
        }
    });
}

/// The Snippets menu: the built-in snippets, one group per user file, and Show Snippet Folder. A snippet goes at the
/// end of the shown entry's code; an automatic entry takes none.
fn snippet_menu(ui: &mut Ui, entry: &EntryPane, snippets: &SnippetsModel, words: &FeaturesWords, sent: &mut Vec<Msg>) {
    const FOLDER: &str = "#folder";
    let button = Button::new(words.snippets.clone()).show(ui);
    let mut entries: Vec<Entry<String>> = Vec::new();
    let take = !entry.automatic;
    for title in &snippets.built_in {
        entries.push(Entry::new(format!("\u{0}{title}"), title.clone()).enabled(take));
    }
    for (group, titles) in &snippets.groups {
        entries.push(Entry::separator());
        entries.push(Entry::heading(group.clone()));
        for title in titles {
            entries.push(Entry::new(format!("{group}\u{0}{title}"), title.clone()).enabled(take));
        }
    }
    entries.push(Entry::separator());
    entries.push(Entry::new(FOLDER.to_string(), words.show_folder.clone()));
    let menu = Menu::new(words.snippets.clone()).opened_by(&button.response, Trigger::Click).pick(ui, &entries);
    let Some(picked) = menu.picked else { return };
    if picked == FOLDER {
        sent.push(Msg::run("font.features.showSnippetFolder", json!({})));
        return;
    }
    let Some((group, title)) = picked.split_once('\u{0}') else { return };
    let mut extra = vec![("snippet", json!(title))];
    if !group.is_empty() {
        extra.push(("group", json!(group)));
    }
    sent.push(Msg::run("font.features.insertSnippet", entry.key.with(&extra)));
}
