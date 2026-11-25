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
//! | context menu: Disable or Enable, Update Entry, Rename, Copy, Paste, Delete | `font.features.setDisabled`, `font.features.updateEntry`, the rename field, `font.features.copyEntries` (its text goes to the clipboard), `font.info.features.paste` (the clipboard's text to `font.features.pasteEntries`), `font.features.remove` |
//! | Copy, Paste, Select All (Cmd+C, Cmd+V, Cmd+A) while a group's list has the keyboard focus | as the menu's Copy and Paste; Select All `font.info.features.select` with every entry of the group |
//! | the update button of an automatic entry's row | `font.features.updateEntry` |
//! | the error icon of a row | `font.features.jumpToError`, after which the code editor selects the faulty code |
//! | the filter field over the sidebar | `ViewMsg::FeatureFilter`: the groups list the entries whose name or code holds its text, and cannot be reordered while it does |
//! | minus, Update, Compile under the sidebar | `font.features.remove`, `font.features.update`, `font.info.features.compile` (busy until that compile lands) |
//! | Generate Feature Automatically, Update Entry | `font.features.setAutomatic`, `font.features.updateEntry` |
//! | Spec | `font.features.openSpec`, whose address opens in the browser |
//! | the stylistic set name rows | `font.features.setLabel`, `font.features.removeLabel` |
//! | the code ([`code_editor`]) | `font.features.setCode` when it loses the keyboard; a Fix button `font.features.applyFix` |
//! | the notes field | `font.features.setNotes` when it loses the keyboard |
//! | the line over the notes field | a drag sets the field's height (`ViewMsg::FeatureNotesHeight`) |
//! | Snippets | `font.features.insertSnippet` (at the end of the code), `font.features.showSnippetFolder` |
//!
//! An entry with an error from the last compile shows a mark in the sidebar while the font is unchanged since. A
//! disabled entry's name is struck through and dimmed. The context menu acts on the selected entries; a right click on
//! an entry outside the selection selects it first.
//!
//! The sidebar's lists and the entry pane scroll on their own. The code fills the height the rest of the pane leaves and
//! scrolls in it, and so do the notes in theirs; a pane too low for its controls scrolls as a whole.

use std::borrow::Cow;
use std::fmt::Debug;
use std::hash::Hash;

use craft::behavior::{Axis, EditEvent, GesturePhase, IconId, Selection, SelectionChange, keyed};
use craft::theme::variant::Tone;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{
    Bind, CollectionEvent, Entry, Props as _, ReorderRule, RowAlign, Scroll, Show as _, ShowCollection as _, ShowContainer as _, Trigger,
    request_rename,
};
use craft::ui::display::{Shade, Text};
use craft::ui::docking::Splitter;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, RowEvent, RowInk, TrailingFace, TrailingPart};
use craft::ui::menu::Menu;
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::style::TypeRole;
use craft::ui::text::TextField;
use craft_ext::ui::list::{CollectionEdits as _, EditCommand, Strike, StrikeRows as _};
use craft_ext::ui::menu::{MarkedEntry, PickMarked as _};
use craft_ext::ui::text::TextSelection;
use egui::{Rect, Sense, TextStyle, Ui, pos2, vec2};
use fontcraft_engine::language_name;
use fontcraft_engine::model::{DEFAULT_LANGUAGE, LANGUAGES};
use serde_json::{Value, json};

use super::code_editor;
use super::rows;
use crate::app::models::font_features::{EntryPane, EntryRow, FeaturesModel, GroupModel, SnippetsModel};
use crate::app::{Msg, ViewMsg};
use crate::shell::font_info::{InfoDialog, open_params};
use crate::shell::font_info_tabs::{COMPILE, EntryKey, EntryKind, PASTE_ENTRIES, select_entries};

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
        filter: "fontcraftFeatures.filter" = "Filter entries";
        no_match: "fontcraftFeatures.noMatch" = "No entry matches";
        update_row: "fontcraftFeatures.updateRow" = "Regenerate this entry";
        jump_to_error: "fontcraftFeatures.jumpToError" = "Show the error the last compile found here";
        remove: "fontcraftFeatures.remove" = "Delete the selected entries";
        update: "fontcraftFeatures.update" = "Update";
        update_tip: "fontcraftFeatures.updateTip" = "Regenerate every automatic prefix, class and feature, and add the ones the glyph names call for";
        compile: "fontcraftFeatures.compile" = "Compile";
        changed_since: "fontcraftFeatures.changedSince" = "Changed since the last compile";
        more_problems: "fontcraftFeatures.moreProblems" = "more problems outside the entries";
        compile_tip: "fontcraftFeatures.compileTip" = "Compile the feature code to preview it in Edit View";
        empty: "fontcraftFeatures.empty" = "Add a prefix, class or feature with a plus button, or press Update.";
        automatic: "fontcraftFeatures.automatic" = "Generate Feature Automatically";
        update_entry: "fontcraftFeatures.updateEntry" = "Update Entry";
        update_entry_tip: "fontcraftFeatures.updateEntryTip" = "Regenerate this entry's code";
        spec: "fontcraftFeatures.spec" = "Spec";
        spec_tip: "fontcraftFeatures.specTip" = "Open this feature's section of the OpenType specification in the browser";
        language: "fontcraftFeatures.language" = "Language";
        set_name: "fontcraftFeatures.setName" = "Stylistic Set Name";
        set_name_hint: "fontcraftFeatures.setNameHint" = "Name of the stylistic set";
        remove_name: "fontcraftFeatures.removeName" = "Remove this name";
        default_language: "fontcraftFeatures.defaultLanguage" = "Default";
        notes: "fontcraftFeatures.notes" = "Notes";
        notes_divider: "fontcraftFeatures.notesDivider" = "Resize Notes";
        notes_hint: "fontcraftFeatures.notesHint" = "Notes about this entry, kept in the source and never exported";
        snippets: "fontcraftFeatures.snippets" = "Snippets";
        show_folder: "fontcraftFeatures.showFolder" = "Show Snippet Folder";
        /// The sidebar's context menu.
        entry_menu: "fontcraftFeatures.entryMenu" = "Entry Menu";
        disable: "fontcraftFeatures.disable" = "Disable";
        enable: "fontcraftFeatures.enable" = "Enable";
        rename: "fontcraftFeatures.rename" = "Rename";
        copy: "fontcraftFeatures.copy" = "Copy";
        paste: "fontcraftFeatures.paste" = "Paste";
        delete: "fontcraftFeatures.delete" = "Delete";
        not_automatic: "fontcraftFeatures.notAutomatic" = "The entry is not generated automatically";
        one_entry: "fontcraftFeatures.oneEntry" = "Select one entry of this group to rename it";
        nothing_selected: "fontcraftFeatures.nothingSelected" = "Select an entry first";
    }
}

/// Lines the notes field shows until its divider is dragged, and the fewest the divider leaves it.
const NOTES_LINES: f32 = 4.0;
const MIN_NOTES_LINES: f32 = 2.0;
/// The largest part of the entry pane's height the notes field takes.
const MAX_NOTES_PART: f32 = 0.5;
/// The sidebar's part of the tab's width, and its limits in points.
const SIDEBAR_PART: f32 = 0.25;
const SIDEBAR_MIN: f32 = 195.0;
const SIDEBAR_MAX: f32 = 340.0;
/// Most characters of the filter.
const MAX_FILTER_CHARS: usize = 200;
/// The update button and the error icon of a sidebar row.
const UPDATE_PART: TrailingPart = TrailingPart::new(0);
const ERROR_PART: TrailingPart = TrailingPart::new(1);
/// The Edit commands a group's list takes while it has the keyboard focus.
const SIDEBAR_EDITS: [EditCommand; 3] = [EditCommand::Copy, EditCommand::Paste, EditCommand::SelectAll];
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

/// The items of the sidebar's context menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum MenuItem {
    Disable,
    Enable,
    UpdateEntry,
    Rename,
    Copy,
    Paste,
    Delete,
}

/// The sidebar row of `key`.
fn row_of<'a>(model: &'a FeaturesModel, key: &EntryKey) -> Option<&'a EntryRow> {
    model.groups.iter().find(|g| g.kind == key.kind).and_then(|g| g.entries.get(key.index))
}

/// The context menu's items for the selected entries of `model`, opened from the group of `kind`. Disable while any
/// selected entry is enabled, else Enable; Update Entry for an automatic last-selected entry; Rename for one selected
/// entry of this group.
fn menu_entries(model: &FeaturesModel, kind: EntryKind, words: &FeaturesWords) -> Vec<MarkedEntry<MenuItem>> {
    let some = !model.selected.is_empty();
    let any_enabled = model.selected.iter().any(|k| row_of(model, k).is_some_and(|r| !r.disabled));
    let toggle =
        if any_enabled || !some { Entry::new(MenuItem::Disable, words.disable.clone()) } else { Entry::new(MenuItem::Enable, words.enable.clone()) };
    let lead_automatic = model.selected.last().and_then(|k| row_of(model, k)).is_some_and(|r| r.automatic);
    let update = Entry::new(MenuItem::UpdateEntry, words.update_entry.clone());
    let one = model.selected.len() == 1 && model.selected.first().is_some_and(|k| k.kind == kind);
    let rename = Entry::new(MenuItem::Rename, words.rename.clone());
    let with = |entry: Entry<MenuItem>, on: bool, why: &Label| if on { entry } else { entry.disabled_reason(why.clone()) };
    vec![
        with(toggle, some, &words.nothing_selected).into(),
        with(update, lead_automatic, &words.not_automatic).into(),
        with(rename, one, &words.one_entry).into(),
        with(Entry::new(MenuItem::Copy, words.copy.clone()), some, &words.nothing_selected).into(),
        Entry::new(MenuItem::Paste, words.paste.clone()).into(),
        MarkedEntry::new(with(Entry::new(MenuItem::Delete, words.delete.clone()), some, &words.nothing_selected)).danger(true),
    ]
}

/// The selected entries as the `entries` parameter of the entry commands.
fn entries_param(model: &FeaturesModel) -> Value {
    Value::Array(model.selected.iter().map(|k| Value::Object(k.params())).collect())
}

/// How an entry shows: a class with its `@`.
fn shown_name(kind: EntryKind, name: &str) -> String {
    if kind == EntryKind::Class { format!("@{name}") } else { name.to_string() }
}

/// The sidebar and the entry pane beside each other, each scrolling on its own.
pub fn view(ui: &mut Ui, model: &FeaturesModel, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<FeaturesWords>();
    let gap = Env::of(ui).tokens.space.gap.section;
    let (mut top, mut middle, mut bottom, mut entry_sent) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    Stack::new().id_salt("fontcraftFeatures.tab").fill().show(ui, |ui, body| {
        let (side, pane) = columns(body.rect, gap);
        Stack::new()
            .id_salt("fontcraftFeatures.sidebar")
            .scroll(Scroll::vertical())
            .fill()
            .header(|ui, _| filter_field(ui, model, &words, &mut top))
            .footer(|ui, _| bottom_buttons(ui, model, &words, &mut bottom))
            .show_at(ui, side, |ui, _| {
                for group in &model.groups {
                    sidebar_group(ui, model, group, &words, &mut middle);
                }
            });
        Stack::new().id_salt("fontcraftFeatures.entry").scroll(Scroll::vertical()).fill().show_at(ui, pane, |ui, _| match &model.entry {
            Some(entry) => entry_pane(ui, model, entry, pane.height(), &words, &mut entry_sent),
            None => {
                unplaced(ui, model, &words);
                Text::new(words.empty.clone()).shade(Shade::Muted).show(ui);
            }
        });
    });
    for mut part in [top, middle, bottom, entry_sent] {
        sent.append(&mut part);
    }
}

/// The sidebar's rect and the entry pane's, `gap` apart, in the tab's rect.
fn columns(tab: Rect, gap: f32) -> (Rect, Rect) {
    let width = (tab.width() * SIDEBAR_PART).clamp(SIDEBAR_MIN, SIDEBAR_MAX).min(tab.width().max(0.0));
    let edge = tab.left() + width;
    let side = Rect::from_min_max(tab.min, pos2(edge, tab.bottom()));
    let pane = Rect::from_min_max(pos2((edge + gap).min(tab.right()), tab.top()), tab.max);
    (side, pane)
}

/// The filter field over the sidebar.
fn filter_field(ui: &mut Ui, model: &FeaturesModel, words: &FeaturesWords, sent: &mut Vec<Msg>) {
    let field = TextField::search(words.filter.clone(), Bind::given(model.filter.clone()))
        .placeholder(words.filter.clone())
        .limit(MAX_FILTER_CHARS)
        .id_salt("fontcraftFeatures.filter");
    if let Some(text) = field.show(ui).events.value()
        && *text != model.filter
    {
        sent.push(Msg::View(ViewMsg::FeatureFilter(text.clone())));
    }
}

/// A box across the room left in `ui`, `height` points tall, whose content scrolls in it.
pub(super) fn scroll_box(ui: &mut Ui, salt: impl Hash + Debug, height: f32, mut body: impl FnMut(&mut Ui)) {
    let room = ui.available_rect_before_wrap();
    let rect = Rect::from_min_size(room.min, vec2(room.width().max(0.0), height.max(0.0)));
    Stack::new().id_salt(salt).scroll(Scroll::vertical()).fill().show_at_taking_room(ui, rect, |ui, _| body(ui));
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
        let positions: Vec<usize> = (0..entries.len()).filter(|at| entries.get(*at).is_some_and(|e| !e.filtered)).collect();
        if positions.is_empty() {
            Text::new(words.no_match.clone()).shade(Shade::Muted).show(ui);
            return;
        }
        let key_of = |at: usize| entries.get(at).map(|e| EntryKey { kind, name: e.name.clone(), index: at });
        let mine: Vec<usize> = model.selected.iter().filter(|k| k.kind == kind).map(|k| k.index).collect();
        let lead = model.selected.last().filter(|k| k.kind == kind).map(|k| k.index);
        let before = Selection::new(mine, lead, lead);
        let mut after = before.clone();
        let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(entries.get(*at).map_or_else(String::new, |e| shown_name(kind, &e.name))));
        let list = List::<usize>::new(kind.heading(words))
            .header(false)
            .natural_height(true)
            .rename(true)
            .rename_draft(|at: &usize| entries.get(*at).map(|e| e.name.clone()));
        // A place among the entries shown is no place among all of them.
        let list = if model.filter.trim().is_empty() { list.reorder(ReorderRule::new()) } else { list };
        let out = list.selection(Bind::bound(&mut after)).show_items(ui, &source, &mut |at, icx| {
            let Some(e) = entries.get(*at) else { return ListRow::new(Label::Empty) };
            let mut row = ListRow::new(shown_name(kind, &e.name)).state(icx.row.clone());
            if e.disabled {
                row = row.ink(RowInk::Dimmed);
            }
            if e.automatic {
                row = row.trailing_part(UPDATE_PART, TrailingFace::Icon(IconId::new("refresh").into()), words.update_row.clone());
            }
            if e.error {
                row = row.trailing_tone(Tone::Bad).trailing_part(
                    ERROR_PART,
                    TrailingFace::Icon(IconId::new("error").into()),
                    words.jump_to_error.clone(),
                );
            }
            if !e.title.is_empty() {
                row = row.optional(e.title.clone());
            }
            row
        });
        out.strike_rows(ui, |at| {
            let e = entries.get(*at).filter(|e| e.disabled)?;
            Some(Strike::new(shown_name(kind, &e.name)).ink(RowInk::Dimmed).selected(after.items.contains(at)))
        });
        for command in out.edit_commands(ui, &SIDEBAR_EDITS) {
            match command {
                EditCommand::Copy if !model.selected.is_empty() => {
                    posted.push(Msg::run("font.features.copyEntries", json!({ "entries": entries_param(model) })))
                }
                EditCommand::Paste => posted.push(Msg::Shell(CommandRef::new(PASTE_ENTRIES))),
                EditCommand::SelectAll => {
                    let all: Vec<EntryKey> = positions.iter().copied().filter_map(key_of).collect();
                    posted.push(Msg::Shell(select_entries(&all)));
                }
                _ => {}
            }
        }
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
                CollectionEvent::Item(at, RowEvent::Trailing { part, .. }) => {
                    let command = if *part == UPDATE_PART { "font.features.updateEntry" } else { "font.features.jumpToError" };
                    if let Some(key) = key_of(*at) {
                        posted.push(Msg::run(command, Value::Object(key.params())));
                    }
                }
                CollectionEvent::Activate(at) => request_rename(ui.ctx(), out.response.id, at),
                CollectionEvent::ContextMenu(request) => {
                    if !before.items.contains(&request.key)
                        && let Some(key) = key_of(request.key)
                    {
                        posted.push(Msg::Shell(select_entries(&[key])));
                    }
                }
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
        let menu = Menu::context(words.entry_menu.clone(), &out.response).id_salt((kind.name(), "entryMenu"));
        match menu.pick_marked(ui, &menu_entries(model, kind, words)).picked {
            Some(MenuItem::Disable) => {
                posted.push(Msg::run("font.features.setDisabled", json!({ "entries": entries_param(model), "disabled": true })))
            }
            Some(MenuItem::Enable) => {
                posted.push(Msg::run("font.features.setDisabled", json!({ "entries": entries_param(model), "disabled": false })))
            }
            Some(MenuItem::UpdateEntry) => {
                if let Some(key) = model.selected.last() {
                    posted.push(Msg::run("font.features.updateEntry", Value::Object(key.params())));
                }
            }
            Some(MenuItem::Rename) => {
                if let Some(key) = model.selected.first().filter(|k| k.kind == kind) {
                    request_rename(ui.ctx(), out.response.id, &key.index);
                }
            }
            Some(MenuItem::Copy) => posted.push(Msg::run("font.features.copyEntries", json!({ "entries": entries_param(model) }))),
            Some(MenuItem::Paste) => posted.push(Msg::Shell(CommandRef::new(PASTE_ENTRIES))),
            Some(MenuItem::Delete) => posted.push(Msg::run("font.features.remove", json!({ "entries": entries_param(model) }))),
            None => {}
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
        if Button::new(words.compile.clone()).tooltip(words.compile_tip.clone()).loading(model.compiling).show(ui).clicked() {
            sent.push(Msg::Shell(CommandRef::new(COMPILE)));
        }
    });
    if model.stale && !model.compiling {
        Text::new(words.changed_since.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
}

/// Most problems outside the entries listed over the code.
const MAX_UNPLACED: usize = 5;

/// What the last compile found outside the entries, each with its targets, over the code.
fn unplaced(ui: &mut Ui, model: &FeaturesModel, words: &FeaturesWords) {
    for d in model.unplaced.iter().take(MAX_UNPLACED) {
        Text::new(d.message.clone()).tone(if d.error { Tone::Bad } else { Tone::Warn }).show(ui);
    }
    let more = model.unplaced.len().saturating_sub(MAX_UNPLACED);
    if more > 0 {
        Text::new(format!("{more} {}", words.more_problems.as_str())).shade(Shade::Muted).show(ui);
    }
}

/// What the entry pane keeps between passes: the height of what stands under the code, and the notes field's height when
/// a drag of its divider began.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct PaneMemory {
    under: f32,
    dragged_from: f32,
}

/// The shown entry in a pane `height` points tall: its title and Spec button, the automatic check box, the stylistic set
/// names, what the compile found outside the entries, the code with its diagnostics in the height the rest leaves, the notes and the snippets menu.
fn entry_pane(ui: &mut Ui, model: &FeaturesModel, entry: &EntryPane, height: f32, words: &FeaturesWords, sent: &mut Vec<Msg>) {
    let key = &entry.key;
    let top = ui.cursor().min.y;
    let memory_id = egui::Id::unique("fontcraftFeatures.pane");
    let mut memory = ui.ctx().data(|d| d.get_temp::<PaneMemory>(memory_id)).unwrap_or_default();
    let line = line_height(ui);
    Row::new().id_salt("fontcraftFeatures.title").fill_width().align(RowAlign::SpaceBetween).show(ui, |ui, _| {
        Text::new(shown_name(key.kind, &key.name)).role(TypeRole::Heading).show(ui);
        if entry.spec_url.is_some() && Button::new(words.spec.clone()).tooltip(words.spec_tip.clone()).show(ui).clicked() {
            sent.push(Msg::run("font.features.openSpec", json!({ "tag": key.name })));
        }
    });
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
    unplaced(ui, model, words);
    let gap = Env::of(ui).tokens.space.gap.row;
    let under = if memory.under > 0.0 { memory.under } else { (NOTES_LINES + 4.0) * line };
    let code = code_editor::view(ui, entry, code_editor::Room { top, height: height - gap, under });
    let selection = code.selection;
    sent.extend(code.sent);
    let divider_top = ui.cursor().min.y;
    // The field's first line takes a control's height, so a line more than it shows.
    let least = (MIN_NOTES_LINES + 1.0) * line;
    let most = (height * MAX_NOTES_PART).max(least);
    let notes_height = model.notes_height.unwrap_or((NOTES_LINES + 1.0) * line).clamp(least, most);
    notes_divider(ui, notes_height, (least, most), &mut memory, words, sent);
    let mut notes_sent = None;
    scroll_box(ui, ("fontcraftFeatures.notesBox", key.kind.name(), key.index), notes_height, |ui| {
        let lines = ((notes_height / line).floor() - 1.0).max(1.0) as usize;
        let notes = TextField::new(words.notes.clone(), Bind::given(entry.notes.clone()))
            .lines(lines)
            .placeholder(words.notes_hint.clone())
            .id_salt(("fontcraftFeatures.notes", key.kind.name(), key.index, key.name.as_str()));
        if let Some(text) = notes.show(ui).events.committed()
            && *text != entry.notes
        {
            notes_sent = Some(Msg::run("font.features.setNotes", key.with(&[("notes", json!(text))])));
        }
    });
    sent.extend(notes_sent);
    snippet_menu(ui, entry, &model.snippets, selection, words, sent);
    let took = ui.cursor().min.y - divider_top;
    if (took - memory.under).abs() > 1.0 {
        memory.under = took;
        ui.ctx().request_repaint();
    }
    ui.ctx().data_mut(|d| d.insert_temp(memory_id, memory));
}

/// The height of a line of text, the taller of the body's and the code's.
fn line_height(ui: &Ui) -> f32 {
    let (body, mono) = (TextStyle::Body.resolve(ui.style()), TextStyle::Monospace.resolve(ui.style()));
    ui.ctx().fonts_mut(|fonts| fonts.row_height(&body).max(fonts.row_height(&mono))).max(1.0)
}

/// The line over the notes field: dragging it up makes the field, `height` points tall, higher, between `limits`.
fn notes_divider(ui: &mut Ui, height: f32, limits: (f32, f32), memory: &mut PaneMemory, words: &FeaturesWords, sent: &mut Vec<Msg>) {
    let room = ui.available_rect_before_wrap();
    let thick = Env::of(ui).tokens.size.hit.target;
    let (rect, _) = ui.allocate_exact_size(vec2(room.width().max(0.0), thick), Sense::hover());
    let line = Splitter::new(Axis::Y).label(words.notes_divider.clone()).id_salt("fontcraftFeatures.notesLine").show_at(ui, rect);
    let response = ui.interact(rect, line.id.with("drag"), Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if response.drag_started() {
        memory.dragged_from = height;
    }
    if let Some(by) = response.total_drag_delta().filter(|_| response.dragged()) {
        let to = (memory.dragged_from - by.y).clamp(limits.0, limits.1);
        if to != height {
            sent.push(Msg::View(ViewMsg::FeatureNotesHeight(to)));
        }
    }
}

/// The language a name row shows.
fn language_label(tag: &str, words: &FeaturesWords) -> Label {
    if tag == DEFAULT_LANGUAGE { words.default_language.clone() } else { Label::from(language_name(tag).unwrap_or(tag).to_string()) }
}

/// The languages a name row offers, the default one first, then by name: its own and those no other name of the set
/// uses.
fn language_entries(current: &str, used: &[&str], words: &FeaturesWords) -> Vec<Entry<String>> {
    let mut named: Vec<(&str, &str)> = LANGUAGES.iter().map(|(tag, _)| (language_name(tag).unwrap_or(tag), *tag)).collect();
    named.sort_unstable();
    std::iter::once(DEFAULT_LANGUAGE)
        .chain(named.into_iter().map(|(_, tag)| tag))
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
/// The snippets menu: a pick puts the snippet in place of `selection` (the code field's last selection, in characters),
/// or at the end of the code without one.
fn snippet_menu(
    ui: &mut Ui,
    entry: &EntryPane,
    snippets: &SnippetsModel,
    selection: Option<TextSelection>,
    words: &FeaturesWords,
    sent: &mut Vec<Msg>,
) {
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
    if let Some(s) = selection {
        let range = s.range();
        extra.push(("position", json!(range.start)));
        extra.push(("end", json!(range.end)));
    }
    sent.push(Msg::run("font.features.insertSnippet", entry.key.with(&extra)));
}
