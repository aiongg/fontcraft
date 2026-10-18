//! The Features tab of Font Info: the font's prefixes, classes and features in a sidebar of three
//! groups, and the selected entry's code, stylistic-set names and notes beside it. Every control
//! acts through a `font.features.*` command; the tab keeps only view state ([`FeaturesState`]).
//!
//! | Control | Command |
//! |---|---|
//! | plus beside Prefix, Classes | `font.features.addPrefix`, `font.features.addClass` |
//! | plus beside Features | the Add Feature picker, then `font.features.addFeature` |
//! | an entry's update button | `font.features.updateEntry` |
//! | an entry's error icon | `font.features.jumpToError` (selects the entry, puts the cursor at the error) |
//! | double-click on an entry, drag an entry | `font.features.rename`, `font.features.reorder` |
//! | minus, Update, Compile (bottom left) | `font.features.remove`, `font.features.update`, `font.features.compile` |
//! | Spec (a feature selected) | `font.features.openSpec`, then the browser |
//! | Generate Feature Automatically | `font.features.setAutomatic` |
//! | stylistic set name rows | `font.features.setLabel`, `font.features.removeLabel` |
//! | the code editor | `font.features.setCode` when it loses the keyboard; `font.features.complete` while typing |
//! | Alt-click in the code | `font.features.previewName`, `font.features.evaluateToken` |
//! | an error's icon under the code, its Fix button | `font.features.showErrorDetails`, `font.features.applyFix` |
//! | the notes field | `font.features.setNotes` when it loses the keyboard |
//! | Snippets (bottom right) | `font.features.listSnippets`, `font.features.insertSnippet`, `font.features.showSnippetFolder` |
//!
//! Typed code and notes are kept while their box has the keyboard and committed as one command
//! when it loses it, or before a button that reads the font (Update, Compile, a snippet, the
//! minus button) runs. Error markers come from the last compile and show only while the font is
//! unchanged since (`font.features.listErrors`).
//!
//! Rows: `fontInfo.features`, `fontInfo.featureAutocomplete`, `fontInfo.featureNamePreview`,
//! `fontInfo.featureTokenPreview`, `fontInfo.featureErrorDetails`, `fontInfo.featureSnippetsMenu`,
//! `fontInfo.features.syntaxColouring`, `.autocomplete`, `.inlineErrors`, `.notesDivider`,
//! `.stylisticSetNames`, `.automaticIndicator`, `.sidebarGroups` (M8.17).

use std::collections::HashSet;
use std::ops::Range;

use egui::{Align, Align2, FontId, Id, Key, Layout, Pos2, Rect, RichText, Sense};
use fontcraft_model::{DEFAULT_LANGUAGE, Features, Font, LANGUAGES, Snippet};
use serde_json::{Map, Value, json};

use super::code_editor::{self, Text};
use super::rows;
use crate::app::FontcraftApp;
use crate::dialogs::feature_picker::{self, FeaturePicker};
use crate::dialogs::field_picker::{Outcome, item};
use crate::icons::{self, Icon};
use crate::panels::{Pending, widgets};
use crate::theme::Tokens;

/// The sidebar's width, in list columns of the theme.
const SIDEBAR_COLUMNS: f32 = 5.0;
/// The height of the sidebar and of the entry pane, in list rows.
const PANE_ROWS: f32 = 24.0;
/// The notes field's height at first, and its least, in list rows.
const NOTES_ROWS: f32 = 4.0;
const MIN_NOTES_ROWS: f32 = 2.0;
/// The code editor's least height, in list rows.
const MIN_EDITOR_ROWS: f32 = 4.0;
/// The height of the divider above the notes field, in points.
const DIVIDER_HEIGHT: f32 = 6.0;
/// Most entries the sidebar draws per group.
const MAX_SHOWN_ENTRIES: usize = 5_000;
/// Most errors listed under the code.
const MAX_SHOWN_ERRORS: usize = 50;
/// Most diagnostics read from the last compile (the compile keeps no more).
const MAX_DIAGNOSTICS: usize = 1_000;
/// Most stylistic-set name rows drawn.
const MAX_SHOWN_LABELS: usize = 100;
/// Most glyph names a class or token popover lists.
const MAX_PREVIEW_GLYPHS: usize = 500;
/// Most suggestions the list shows before it scrolls, and most kept.
const COMPLETION_ROWS: f32 = 8.0;
const MAX_COMPLETIONS: usize = 1_000;
/// Most snippets kept per menu group.
const MAX_MENU_SNIPPETS: usize = 10_000;
/// The name the plus button gives a new class, numbered when taken.
const NEW_CLASS: &str = "NewClass";
/// Most numbers tried for a free class name.
const MAX_CLASS_NUMBER: usize = 10_000;

/// A sidebar group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Prefix,
    Class,
    Feature,
}

impl Kind {
    /// The groups in sidebar order (`fontInfo.features.sidebarGroups`).
    pub const ALL: [Kind; 3] = [Kind::Prefix, Kind::Class, Kind::Feature];

    /// The `kind` parameter of the commands.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Prefix => "prefix",
            Kind::Class => "class",
            Kind::Feature => "feature",
        }
    }

    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }

    fn heading(self) -> &'static str {
        match self {
            Kind::Prefix => "Prefix",
            Kind::Class => "Classes",
            Kind::Feature => "Features",
        }
    }

    fn add_hover(self) -> &'static str {
        match self {
            Kind::Prefix => "Add a prefix",
            Kind::Class => "Add a class",
            Kind::Feature => "Add a feature",
        }
    }

    pub fn group(self, f: &Features) -> &[Snippet] {
        match self {
            Kind::Prefix => &f.prefixes,
            Kind::Class => &f.classes,
            Kind::Feature => &f.features,
        }
    }
}

/// One sidebar entry: its group, its name and its place in the group.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EntryKey {
    pub kind: Kind,
    pub name: String,
    pub index: usize,
}

impl EntryKey {
    /// `{kind, name, index}`, as every entry command takes it.
    pub fn params(&self) -> Map<String, Value> {
        let mut p = Map::new();
        p.insert("kind".into(), json!(self.kind.name()));
        p.insert("name".into(), json!(self.name));
        p.insert("index".into(), json!(self.index));
        p
    }

    /// [`Self::params`] with `extra` added.
    pub fn with(&self, extra: &[(&str, Value)]) -> Value {
        let mut p = self.params();
        for (k, v) in extra {
            p.insert((*k).to_string(), v.clone());
        }
        Value::Object(p)
    }

    /// The entry, when the font has it at this place under this name.
    pub fn snippet<'a>(&self, f: &'a Features) -> Option<&'a Snippet> {
        self.kind.group(f).get(self.index).filter(|e| e.name == self.name)
    }

    /// The key of this entry in the font as it is now: at its place, else the first entry of its
    /// name (it moved), else the entry now at its place (it was renamed). `None` when the group
    /// has neither.
    pub fn resolve(&self, f: &Features) -> Option<EntryKey> {
        let group = self.kind.group(f);
        if self.snippet(f).is_some() {
            return Some(self.clone());
        }
        if let Some(index) = group.iter().position(|e| e.name == self.name) {
            return Some(EntryKey { index, ..self.clone() });
        }
        group.get(self.index).map(|e| EntryKey { name: e.name.clone(), ..self.clone() })
    }

    fn from_json(v: &Value) -> Option<EntryKey> {
        Some(EntryKey {
            kind: Kind::from_name(v.get("kind")?.as_str()?)?,
            name: v.get("name")?.as_str()?.to_string(),
            index: usize::try_from(v.get("index")?.as_u64()?).ok()?,
        })
    }
}

/// Text typed into the code editor or the notes field of one entry, not yet committed.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub key: EntryKey,
    pub text: String,
}

/// One error or warning of the last compile.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub id: u64,
    pub error: bool,
    pub message: String,
    pub entry: Option<EntryKey>,
    /// Bytes of the entry's code.
    pub range: Option<Range<usize>>,
    pub line: u64,
    /// Labels of the suggested fixes (the wrench).
    pub fixes: Vec<String>,
}

/// The last compile's diagnostics, read again when the document or the compile changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ErrorCache {
    /// The document revision and the compile (its revision and input hash) the list was read for.
    pub key: Option<(u64, Option<(u64, u64)>)>,
    /// The font has not changed since the compile.
    pub current: bool,
    pub diagnostics: Vec<Diagnostic>,
}

impl ErrorCache {
    /// The cache made from a `font.features.listErrors` result.
    pub fn from_result(key: Option<(u64, Option<(u64, u64)>)>, result: Option<&Value>) -> ErrorCache {
        let Some(r) = result else { return ErrorCache { key, ..Default::default() } };
        let current = r.get("current").and_then(Value::as_bool).unwrap_or(false);
        let list = r.get("diagnostics").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        let diagnostics = list
            .iter()
            .take(MAX_DIAGNOSTICS)
            .filter_map(|d| {
                let range = d.get("range").and_then(Value::as_array).and_then(|r| {
                    let start = usize::try_from(r.first()?.as_u64()?).ok()?;
                    let end = usize::try_from(r.get(1)?.as_u64()?).ok()?;
                    (start <= end).then_some(start..end)
                });
                Some(Diagnostic {
                    id: d.get("id")?.as_u64()?,
                    error: d.get("severity").and_then(Value::as_str) == Some("error"),
                    message: d.get("message").and_then(Value::as_str).unwrap_or_default().to_string(),
                    entry: d.get("entry").and_then(EntryKey::from_json),
                    range,
                    line: d.get("line").and_then(Value::as_u64).unwrap_or(0),
                    fixes: strings(d.get("fixes")),
                })
            })
            .collect();
        ErrorCache { key, current, diagnostics }
    }

    /// The diagnostics to show: none once the font changed since the compile.
    pub fn shown(&self) -> &[Diagnostic] {
        if self.current { &self.diagnostics } else { &[] }
    }

    /// The entries with errors, as (group, place).
    pub fn entries_with_errors(&self) -> HashSet<(Kind, usize)> {
        self.shown().iter().filter(|d| d.error).filter_map(|d| d.entry.as_ref().map(|e| (e.kind, e.index))).collect()
    }

    fn of(&self, key: &EntryKey) -> impl Iterator<Item = &Diagnostic> {
        self.shown().iter().filter(move |d| d.entry.as_ref() == Some(key))
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// One suggestion of the autocomplete list.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub label: String,
    pub kind: String,
    /// The bytes of the code the label replaces.
    pub replace: Range<usize>,
}

/// The open autocomplete list.
#[derive(Clone, Debug, PartialEq)]
pub struct Completions {
    pub key: EntryKey,
    pub items: Vec<Suggestion>,
    pub highlighted: usize,
    pub at: Pos2,
}

/// What a popover over the code shows.
#[derive(Clone, Debug, PartialEq)]
pub enum PopoverContent {
    /// An Alt-clicked glyph or class name: the `font.features.previewName` result.
    Name(Result<Value, String>),
    /// An Alt-clicked token: the `font.features.evaluateToken` result.
    Token(Result<Value, String>),
    /// A clicked error icon: the error's id and the `font.features.showErrorDetails` result.
    Error(u64, Result<Value, String>),
}

/// A popover over the code.
#[derive(Clone, Debug, PartialEq)]
pub struct Popover {
    pub at: Pos2,
    /// The pass it opened in: the click that opened it does not close it.
    pub opened: u64,
    pub content: PopoverContent,
}

/// The snippets menu's contents.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnippetListing {
    pub built_in: Vec<String>,
    /// User groups: name and snippet titles.
    pub groups: Vec<(String, Vec<String>)>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

/// The open snippets menu.
#[derive(Clone, Debug, PartialEq)]
pub struct SnippetMenu {
    pub listing: Option<SnippetListing>,
    /// The Snippets button: the menu opens above it, and a press on it does not close the menu
    /// (the button's click does).
    pub button: Rect,
    pub opened: u64,
}

/// A sidebar rename in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct Rename {
    pub key: EntryKey,
    /// The field has had the keyboard: losing it now ends the rename.
    pub focused: bool,
}

/// A sidebar drag in progress: the entry dragged and the place it would go.
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    pub key: EntryKey,
    pub over: Option<usize>,
}

/// The tab's view state. None of it is saved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeaturesState {
    /// Selected entries; the last is the one the editor shows.
    pub selected: Vec<EntryKey>,
    pub draft: Option<Draft>,
    pub notes: Option<Draft>,
    /// The notes field's height in points, once the divider was dragged.
    pub notes_height: Option<f32>,
    /// The code editor's selection in characters, with the entry it is in.
    pub selection: Option<(EntryKey, Range<usize>)>,
    /// Where the editor's cursor goes next frame, in characters.
    pub caret: Option<usize>,
    pub renaming: Option<Rename>,
    pub drag: Option<Drag>,
    pub picker: Option<FeaturePicker>,
    pub completions: Option<Completions>,
    pub popover: Option<Popover>,
    pub snippets: Option<SnippetMenu>,
    /// The language of the stylistic-set name row being added.
    pub new_label: Option<String>,
    pub errors: ErrorCache,
}

impl FeaturesState {
    /// The entry the editor shows.
    pub fn primary(&self) -> Option<&EntryKey> {
        self.selected.last()
    }
}

/// A query the tab runs after drawing, when it no longer borrows the font.
enum Act {
    Complete { key: EntryKey, code: String, position: usize, at: Pos2 },
    Preview { name: String, at: Pos2 },
    Token { token: String, at: Pos2 },
    Details { id: u64, at: Pos2 },
    Jump(EntryKey),
    Spec(String),
    ListSnippets,
    ShowFolder,
}

// ── Ids ─────────────────────────────────────────────────────────────────────────────────────────

fn id(salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
    widgets::id(("fontInfo.features", salt))
}

/// The plus button beside a group's heading.
pub fn plus_id(kind: Kind) -> Id {
    id(("plus", kind.name()))
}

/// A sidebar entry.
pub fn entry_id(kind: Kind, index: usize) -> Id {
    id(("entry", kind.name(), index))
}

/// An automatic entry's update button.
pub fn update_id(kind: Kind, index: usize) -> Id {
    id(("update", kind.name(), index))
}

/// The error icon of a sidebar entry.
pub fn sidebar_error_id(kind: Kind, index: usize) -> Id {
    id(("sidebarError", kind.name(), index))
}

pub fn rename_id() -> Id {
    id("rename")
}

pub fn remove_id() -> Id {
    id("remove")
}

pub fn update_all_id() -> Id {
    id("updateAll")
}

pub fn compile_id() -> Id {
    id("compile")
}

pub fn spec_id() -> Id {
    id("spec")
}

pub fn automatic_id() -> Id {
    id("automatic")
}

pub fn editor_id() -> Id {
    id("code")
}

pub fn notes_id() -> Id {
    id("notes")
}

pub fn divider_id() -> Id {
    id("divider")
}

pub fn snippets_id() -> Id {
    id("snippets")
}

/// A snippet of the menu: a built-in one (`group` `None`) or one of a user group.
pub fn snippet_item_id(group: Option<&str>, title: &str) -> Id {
    id(("snippet", group, title))
}

pub fn show_folder_id() -> Id {
    id("showSnippetFolder")
}

/// The icon of error `error` under the code.
pub fn error_id(error: u64) -> Id {
    id(("error", error))
}

/// The Fix button of fix `fix` in the error popover.
pub fn fix_id(fix: usize) -> Id {
    id(("fix", fix))
}

/// The language menu, name field and minus and plus buttons of stylistic-set name row `at`.
pub fn label_language_id(at: usize) -> Id {
    id(("labelLanguage", at))
}

pub fn label_text_id(at: usize) -> Id {
    id(("labelText", at))
}

pub fn label_minus_id(at: usize) -> Id {
    id(("labelMinus", at))
}

pub fn label_plus_id(at: usize) -> Id {
    id(("labelPlus", at))
}

/// The row of a stylistic-set name being added.
pub fn new_label_language_id() -> Id {
    id("newLabelLanguage")
}

pub fn new_label_text_id() -> Id {
    id("newLabelText")
}

pub fn popover_id() -> Id {
    id("popover")
}

pub fn completions_id() -> Id {
    id("completions")
}

pub fn snippet_menu_id() -> Id {
    id("snippetMenu")
}

// ── Drawing ─────────────────────────────────────────────────────────────────────────────────────

/// Draws the tab's contents inside the Font Info window.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    sync(app);
    let master = current_master(app);
    let mut acts = Vec::new();
    {
        let FontcraftApp { session, state, tokens, .. } = &mut *app;
        let Ok(font) = session.font() else { return };
        let st = &mut state.panels.font_info.features;
        draw(ui, tokens, font, st, master.as_deref(), pending, &mut acts);
    }
    let ctx = ui.ctx().clone();
    for act in acts {
        perform(app, &ctx, master.as_deref(), act);
    }
    picker(app, &ctx, pending);
}

/// The master the tab previews, compiles and evaluates tokens for: the active master of the
/// showing Edit View tab, else Font View's.
fn current_master(app: &FontcraftApp) -> Option<String> {
    let font = app.session.font().ok()?;
    match app.state.tab() {
        Some(tab) => crate::canvas::view::active_master(font, tab).map(|m| m.id.clone()),
        None => crate::shell::font_view::master(app),
    }
}

/// Brings the view state up to the font: the error list, the selection (the first entry when
/// none is), drafts of entries that went away, the picker's list.
fn sync(app: &mut FontcraftApp) {
    let key = app.session.doc().ok().map(|d| (d.revision, app.session.feature_state().map(|s| (s.revision, s.source_hash))));
    if app.state.panels.font_info.features.errors.key != key {
        let result = app.run("font.features.listErrors", &json!({})).ok();
        app.state.panels.font_info.features.errors = ErrorCache::from_result(key, result.as_ref());
    }
    if let Some(picker) = &app.state.panels.font_info.features.picker
        && picker.needs_listing()
    {
        let params = picker.query_params();
        let result = app.run("font.features.listAvailable", &params);
        if let Some(picker) = &mut app.state.panels.font_info.features.picker {
            match result {
                Ok(r) => picker.fill(&r),
                Err(e) => picker.fail(e),
            }
        }
    }
    let Ok(font) = app.session.font() else { return };
    let f = &font.features;
    let st = &mut app.state.panels.font_info.features;
    let mut seen = HashSet::new();
    st.selected = st.selected.iter().filter_map(|k| k.resolve(f)).filter(|k| seen.insert(k.clone())).collect();
    if st.selected.is_empty()
        && let Some(first) = Kind::ALL.into_iter().find_map(|kind| kind.group(f).first().map(|e| EntryKey { kind, name: e.name.clone(), index: 0 }))
    {
        st.selected.push(first);
    }
    for draft in [&mut st.draft, &mut st.notes] {
        if let Some(d) = draft {
            match d.key.resolve(f) {
                Some(k) => d.key = k,
                None => *draft = None,
            }
        }
    }
    if st.renaming.as_ref().is_some_and(|r| r.key.snippet(f).is_none()) {
        st.renaming = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(ui: &mut egui::Ui, tokens: &Tokens, font: &Font, st: &mut FeaturesState, master: Option<&str>, pending: &mut Pending, acts: &mut Vec<Act>) {
    let pane = tokens.list_row_height * PANE_ROWS;
    let with_errors = st.errors.entries_with_errors();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(tokens.list_column_unit * SIDEBAR_COLUMNS);
            egui::ScrollArea::vertical().id_salt("fontInfo.features.sidebar").max_height(pane).auto_shrink([false, false]).show(ui, |ui| {
                for kind in Kind::ALL {
                    sidebar_group(ui, tokens, font, st, kind, &with_errors, pending, acts);
                }
            });
            ui.add_space(tokens.dialog_spacing);
            bottom_buttons(ui, font, st, master, pending);
        });
        ui.separator();
        ui.vertical(|ui| entry_pane(ui, tokens, font, st, pane, pending, acts));
    });
    let ctx = ui.ctx().clone();
    completion_list(&ctx, tokens, st);
    popover(&ctx, tokens, font, st, master, pending);
    snippet_menu(&ctx, tokens, font, st, pending, acts);
}

/// Commits the typed code and notes, as their boxes losing the keyboard would.
fn flush(st: &mut FeaturesState, font: &Font, pending: &mut Pending) {
    if let Some(d) = st.draft.take()
        && let Some(e) = d.key.snippet(&font.features)
        && !e.automatic
        && d.text != e.code
    {
        pending.push(("font.features.setCode", d.key.with(&[("code", json!(d.text))])));
    }
    if let Some(d) = st.notes.take()
        && let Some(e) = d.key.snippet(&font.features)
        && d.text != e.notes
    {
        pending.push(("font.features.setNotes", d.key.with(&[("notes", json!(d.text))])));
    }
    st.completions = None;
}

/// A click on `key` in the sidebar: alone it selects only that entry, with `extend` (Cmd) it adds
/// or removes it, with `range` (Shift) it adds the entries of its group from the last selected.
fn select(st: &mut FeaturesState, font: &Font, key: EntryKey, extend: bool, range: bool, pending: &mut Pending) {
    let before = st.primary().cloned();
    if range && let Some(last) = before.as_ref().filter(|l| l.kind == key.kind) {
        let (lo, hi) = (last.index.min(key.index), last.index.max(key.index));
        for (index, e) in key.kind.group(&font.features).iter().enumerate().take(hi.saturating_add(1)).skip(lo) {
            let k = EntryKey { kind: key.kind, name: e.name.clone(), index };
            if !st.selected.contains(&k) {
                st.selected.push(k);
            }
        }
        st.selected.retain(|k| *k != key);
        st.selected.push(key);
    } else if extend {
        match st.selected.iter().position(|k| *k == key) {
            Some(at) => {
                st.selected.remove(at);
            }
            None => st.selected.push(key),
        }
    } else {
        st.selected = vec![key];
    }
    if st.primary() != before.as_ref() {
        show_entry(st, font, pending);
    }
}

/// The editor moves to another entry: what was typed is committed and the entry's popovers close.
fn show_entry(st: &mut FeaturesState, font: &Font, pending: &mut Pending) {
    flush(st, font, pending);
    st.new_label = None;
    st.selection = None;
    st.popover = None;
    st.snippets = None;
}

/// The first free class name of the form `NewClass`, `NewClass1`, `NewClass2`…
fn free_class_name(f: &Features) -> String {
    let taken = |name: &str| f.classes.iter().any(|c| c.name == name);
    (0..MAX_CLASS_NUMBER)
        .map(|n| if n == 0 { NEW_CLASS.to_string() } else { format!("{NEW_CLASS}{n}") })
        .find(|name| !taken(name))
        .unwrap_or_else(|| format!("{NEW_CLASS}{}", f.classes.len()))
}

#[allow(clippy::too_many_arguments)]
fn sidebar_group(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    font: &Font,
    st: &mut FeaturesState,
    kind: Kind,
    with_errors: &HashSet<(Kind, usize)>,
    pending: &mut Pending,
    acts: &mut Vec<Act>,
) {
    if rows::heading(ui, tokens, plus_id(kind), kind.heading(), kind.add_hover()) {
        flush(st, font, pending);
        match kind {
            Kind::Prefix => pending.push(("font.features.addPrefix", json!({}))),
            Kind::Class => pending.push(("font.features.addClass", json!({"name": free_class_name(&font.features)}))),
            Kind::Feature => st.picker = Some(FeaturePicker::new()),
        }
    }
    let entries = kind.group(&font.features);
    if entries.is_empty() {
        ui.label(RichText::new("None").color(tokens.text_dim));
    }
    for (index, e) in entries.iter().enumerate().take(MAX_SHOWN_ENTRIES) {
        let key = EntryKey { kind, name: e.name.clone(), index };
        entry_row(ui, tokens, font, st, key, e, with_errors.contains(&(kind, index)), pending, acts);
    }
    if entries.len() > MAX_SHOWN_ENTRIES {
        ui.label(RichText::new(format!("and {} more", entries.len() - MAX_SHOWN_ENTRIES)).color(tokens.text_dim));
    }
}

/// Paints the error icon: a filled circle with an exclamation mark.
fn paint_error_icon(painter: &egui::Painter, rect: Rect, tokens: &Tokens, error: bool) {
    let r = rect.height().min(rect.width()) * 0.32;
    let fill = if error { tokens.error_text } else { tokens.text_dim };
    painter.circle_filled(rect.center(), r, fill);
    painter.text(rect.center(), Align2::CENTER_CENTER, "!", FontId::proportional(r * 1.6), tokens.panel);
}

#[allow(clippy::too_many_arguments)]
fn entry_row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    font: &Font,
    st: &mut FeaturesState,
    key: EntryKey,
    e: &Snippet,
    has_error: bool,
    pending: &mut Pending,
    acts: &mut Vec<Act>,
) {
    let width = ui.available_width();
    if let Some(rename) = st.renaming.as_mut().filter(|r| r.key == key) {
        let (commit, response) = widgets::line(ui, rename_id(), &e.name, width, widgets::Look::default(), &|_, _| None);
        if !rename.focused {
            response.request_focus();
            rename.focused = true;
        } else if let Some(new) = commit {
            st.renaming = None;
            let new = new.trim().to_string();
            if !new.is_empty() && new != e.name {
                pending.push(("font.features.rename", key.with(&[("newName", json!(new))])));
            }
        } else if !response.has_focus() {
            st.renaming = None;
        }
        return;
    }
    let row = tokens.list_row_height;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, row), Sense::hover());
    let response = ui.interact(rect, entry_id(key.kind, key.index), Sense::click_and_drag());
    let selected = st.selected.contains(&key);
    let dragging_here = st.drag.as_ref().is_some_and(|d| d.key.kind == key.kind) && ui.rect_contains_pointer(rect);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, f32::from(tokens.radius), tokens.row_selected);
        } else if response.hovered() {
            painter.rect_filled(rect, f32::from(tokens.radius), tokens.hover);
        }
        let color = match (selected, e.disabled) {
            (true, _) => tokens.selected_text,
            (false, true) => tokens.text_dim,
            (false, false) => tokens.text,
        };
        let shown = if key.kind == Kind::Class { format!("@{}", e.name) } else { e.name.clone() };
        let font_id = egui::TextStyle::Monospace.resolve(ui.style());
        let at = egui::pos2(rect.left() + tokens.dialog_spacing, rect.center().y);
        painter.with_clip_rect(rect).text(at, Align2::LEFT_CENTER, shown, font_id, color);
        if dragging_here {
            let y = rect.top();
            painter.hline(rect.x_range(), y, egui::Stroke::new(2.0, tokens.accent));
        }
    }
    // The icons sit on top of the row, at its right end: the update button, then the error icon.
    let mut right = rect.right();
    let icon_rect = |right: f32| Rect::from_min_size(egui::pos2(right - row, rect.top()), egui::vec2(row, row));
    if e.automatic {
        let r = icon_rect(right);
        right -= row;
        let button = ui.interact(r, update_id(key.kind, key.index), Sense::click()).on_hover_text("Update this entry");
        let color = if selected { tokens.selected_text } else { tokens.text_dim };
        icons::paint(ui.painter(), r.shrink(row * 0.18), Icon::Update, color);
        if button.clicked() {
            flush(st, font, pending);
            pending.push(("font.features.updateEntry", Value::Object(key.params())));
        }
    }
    if has_error {
        let r = icon_rect(right);
        let button = ui.interact(r, sidebar_error_id(key.kind, key.index), Sense::click()).on_hover_text("Go to the error");
        paint_error_icon(ui.painter(), r, tokens, true);
        if button.clicked() {
            acts.push(Act::Jump(key.clone()));
        }
    }
    if response.drag_started() {
        st.drag = Some(Drag { key: key.clone(), over: None });
    }
    if let Some(drag) = st.drag.as_mut().filter(|d| d.key.kind == key.kind)
        && dragging_here
    {
        drag.over = Some(key.index);
    }
    if response.drag_stopped() {
        if let Some(drag) = st.drag.take()
            && let Some(to) = drag.over.filter(|to| *to != drag.key.index)
        {
            let count = key.kind.group(&font.features).len();
            let mut order: Vec<usize> = (0..count).collect();
            if drag.key.index < count {
                order.retain(|i| *i != drag.key.index);
                order.insert(to.min(order.len()), drag.key.index);
                flush(st, font, pending);
                pending.push(("font.features.reorder", json!({"kind": key.kind.name(), "order": order})));
            }
        }
        return;
    }
    if response.double_clicked() {
        select(st, font, key.clone(), false, false, pending);
        st.renaming = Some(Rename { key, focused: false });
    } else if response.clicked() {
        let (extend, range) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
        select(st, font, key, extend, range, pending);
    }
}

/// The minus, Update and Compile buttons under the sidebar.
fn bottom_buttons(ui: &mut egui::Ui, font: &Font, st: &mut FeaturesState, master: Option<&str>, pending: &mut Pending) {
    ui.horizontal(|ui| {
        if widgets::button(ui, remove_id(), "−", "Delete the selected entries", !st.selected.is_empty(), false) {
            let removed: Vec<EntryKey> = std::mem::take(&mut st.selected);
            for draft in [&mut st.draft, &mut st.notes] {
                if draft.as_ref().is_some_and(|d| removed.contains(&d.key)) {
                    *draft = None;
                }
            }
            flush(st, font, pending);
            let entries: Vec<Value> = removed.iter().map(|k| Value::Object(k.params())).collect();
            pending.push(("font.features.remove", json!({"entries": entries})));
            st.popover = None;
            st.snippets = None;
        }
        if widgets::button(
            ui,
            update_all_id(),
            "Update",
            "Regenerate every automatic prefix, class and feature, and add the ones the glyph names call for",
            true,
            false,
        ) {
            flush(st, font, pending);
            pending.push(("font.features.update", json!({})));
        }
        if widgets::button(ui, compile_id(), "Compile", "Compile the feature code to preview it in Edit View", true, false) {
            flush(st, font, pending);
            pending.push(("font.features.compile", master.map_or_else(|| json!({}), |m| json!({"master": m}))));
        }
    });
}

/// Whether `tag` is one of ss01 to ss20, whose names the tab edits.
fn is_stylistic_set(tag: &str) -> bool {
    tag.strip_prefix("ss").filter(|n| n.len() == 2).and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=20).contains(&n))
}

/// The selected entry's pane: its title and Spec button, the automatic checkbox, the stylistic
/// set names, the code, its errors, the notes and the snippets button.
fn entry_pane(ui: &mut egui::Ui, tokens: &Tokens, font: &Font, st: &mut FeaturesState, pane: f32, pending: &mut Pending, acts: &mut Vec<Act>) {
    let Some(key) = st.primary().cloned() else {
        ui.label(RichText::new("Add a prefix, class or feature with a plus button, or press Update.").color(tokens.text_dim));
        return;
    };
    let Some(e) = key.snippet(&font.features) else { return };
    let top = ui.cursor().top();
    ui.horizontal(|ui| {
        let title = if key.kind == Kind::Class { format!("@{}", e.name) } else { e.name.clone() };
        ui.label(RichText::new(title).strong().size(tokens.section_header_size));
        if key.kind == Kind::Feature {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::button(ui, spec_id(), "Spec", "Open the feature's section of the OpenType specification", true, false) {
                    acts.push(Act::Spec(e.name.clone()));
                }
            });
        }
    });
    automatic_checkbox(ui, font, st, &key, e, pending);
    if key.kind == Kind::Feature && is_stylistic_set(&e.name) {
        label_rows(ui, tokens, st, &key, e, pending);
    }
    let row = tokens.list_row_height;
    let notes_height = st.notes_height.unwrap_or(row * NOTES_ROWS).clamp(row * MIN_NOTES_ROWS, (pane * 0.5).max(row * MIN_NOTES_ROWS));
    let diagnostics: Vec<Diagnostic> = st.errors.of(&key).take(MAX_SHOWN_ERRORS).cloned().collect();
    let used = ui.cursor().top() - top;
    let listed = row * diagnostics.len() as f32;
    let editor_height = (pane - used - listed - notes_height - DIVIDER_HEIGHT - 2.0 * row).max(row * MIN_EDITOR_ROWS);
    code(ui, tokens, font, st, &key, e, &diagnostics, editor_height, pending, acts);
    for d in &diagnostics {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(row, row), Sense::hover());
            let button = ui.interact(r, error_id(d.id), Sense::click()).on_hover_text("Show the whole message");
            paint_error_icon(ui.painter(), r, tokens, d.error);
            if button.clicked() {
                acts.push(Act::Details { id: d.id, at: r.left_bottom() });
            }
            let color = if d.error { tokens.error_text } else { tokens.text_dim };
            ui.label(RichText::new(format!("Line {}: {}", d.line, d.message)).color(color));
            if !d.fixes.is_empty() {
                ui.label(RichText::new("Fix available").color(tokens.text_dim)).on_hover_text("Click the error icon to apply the fix");
            }
        });
    }
    notes(ui, tokens, font, st, &key, e, notes_height, pending);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        let response = widgets::button(ui, snippets_id(), "Snippets", "Insert a code snippet", true, st.snippets.is_some());
        if response {
            if st.snippets.is_some() {
                st.snippets = None;
            } else {
                let rect = ui.ctx().read_response(snippets_id()).map(|r| r.rect).unwrap_or(Rect::NOTHING);
                let opened = ui.ctx().cumulative_pass_nr();
                st.snippets = Some(SnippetMenu { listing: None, button: rect, opened });
                acts.push(Act::ListSnippets);
            }
        }
    });
}

fn automatic_checkbox(ui: &mut egui::Ui, font: &Font, st: &mut FeaturesState, key: &EntryKey, e: &Snippet, pending: &mut Pending) {
    let mut shown = e.automatic;
    let drawn = ui.checkbox(&mut shown, "Generate Feature Automatically");
    // A click goes to this id, on top of the checkbox, so the toggle is a command and the
    // checkbox never changes by itself.
    let response = ui.interact(drawn.rect, automatic_id(), Sense::click());
    if response.clicked() {
        if e.automatic {
            flush(st, font, pending);
        } else {
            st.draft = None;
            st.completions = None;
        }
        pending.push(("font.features.setAutomatic", key.with(&[("automatic", json!(!e.automatic))])));
    }
}

/// The language a stylistic-set name row shows.
fn language_label(tag: &str) -> &str {
    if tag == DEFAULT_LANGUAGE { "Default" } else { tag }
}

/// A language menu showing `current`, offering it and the languages not in `used`. The language
/// picked, when another than `current`.
fn language_menu(ui: &mut egui::Ui, tokens: &Tokens, id: Id, current: &str, used: &[&str]) -> Option<String> {
    let mut chosen = current.to_string();
    egui::ComboBox::from_id_salt(id).selected_text(language_label(current)).width(tokens.list_column_unit * 1.5).show_ui(ui, |ui| {
        let all = std::iter::once(DEFAULT_LANGUAGE).chain(LANGUAGES.iter().map(|(tag, _)| *tag));
        for tag in all.filter(|l| *l == current || !used.contains(l)) {
            ui.selectable_value(&mut chosen, tag.to_string(), language_label(tag));
        }
    });
    (chosen != current).then_some(chosen)
}

/// The name rows of a stylistic set: a language menu, the name and a minus and plus button for
/// each name, then a row for a new name (the only row while the set has no name).
fn label_rows(ui: &mut egui::Ui, tokens: &Tokens, st: &mut FeaturesState, key: &EntryKey, e: &Snippet, pending: &mut Pending) {
    let width = tokens.dialog_field_width;
    let used: Vec<&str> = e.labels.iter().map(|l| l.language.as_str()).collect();
    let next_free = || std::iter::once(DEFAULT_LANGUAGE).chain(LANGUAGES.iter().map(|(tag, _)| *tag)).find(|l| !used.contains(l)).map(str::to_string);
    let tag = json!(e.name);
    let index = json!(key.index);
    for (at, label) in e.labels.iter().enumerate().take(MAX_SHOWN_LABELS) {
        ui.horizontal(|ui| {
            if let Some(to) = language_menu(ui, tokens, label_language_id(at), &label.language, &used) {
                pending.push(("font.features.removeLabel", json!({"tag": tag, "index": index, "language": label.language})));
                pending.push(("font.features.setLabel", json!({"tag": tag, "index": index, "language": to, "name": label.value})));
            }
            if let Some(text) = widgets::text(ui, label_text_id(at), &label.value, width) {
                if text.trim().is_empty() {
                    pending.push(("font.features.removeLabel", json!({"tag": tag, "index": index, "language": label.language})));
                } else {
                    pending.push(("font.features.setLabel", json!({"tag": tag, "index": index, "language": label.language, "name": text})));
                }
            }
            if widgets::flat_button(ui, label_minus_id(at), "−", "Remove this name") {
                pending.push(("font.features.removeLabel", json!({"tag": tag, "index": index, "language": label.language})));
            }
            if st.new_label.is_none() && widgets::flat_button(ui, label_plus_id(at), "+", "Add a name in another language") {
                st.new_label = next_free();
            }
        });
    }
    if !e.labels.is_empty() && st.new_label.is_none() {
        return;
    }
    let language = st.new_label.clone().or_else(next_free).unwrap_or_else(|| DEFAULT_LANGUAGE.to_string());
    ui.horizontal(|ui| {
        if let Some(to) = language_menu(ui, tokens, new_label_language_id(), &language, &used) {
            st.new_label = Some(to);
        }
        if let Some(text) = widgets::line(
            ui,
            new_label_text_id(),
            "",
            width,
            widgets::Look { placeholder: Some(RichText::new("Name of the stylistic set")), ..Default::default() },
            &|_, _| None,
        )
        .0
        .filter(|t| !t.trim().is_empty())
        {
            pending.push(("font.features.setLabel", json!({"tag": tag, "index": index, "language": language, "name": text})));
            st.new_label = None;
        }
    });
}

/// Whether the character before byte `at` of `text` may start or continue a suggestion: a name
/// character, `@` or `$`.
fn wants_completion(text: &str, at: usize) -> bool {
    let at = code_editor::floor_boundary(text, at);
    text.get(..at).and_then(|s| s.chars().next_back()).is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-' | '@' | '$' | '{'))
}

/// The code editor: read-only for an automatic entry; else the typed text is kept as a draft
/// while the editor has the keyboard and committed when it loses it.
#[allow(clippy::too_many_arguments)]
fn code(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    font: &Font,
    st: &mut FeaturesState,
    key: &EntryKey,
    e: &Snippet,
    diagnostics: &[Diagnostic],
    height: f32,
    pending: &mut Pending,
    acts: &mut Vec<Act>,
) {
    let ctx = ui.ctx().clone();
    if let Some(caret) = st.caret.take() {
        code_editor::set_cursor(&ctx, editor_id(), caret);
    }
    let ranges: Vec<Range<usize>> = diagnostics.iter().filter_map(|d| d.range.clone()).collect();
    if e.automatic {
        let out = code_editor::show(ui, tokens, editor_id(), Text::ReadOnly(&e.code), height, &ranges, false);
        if let Some(at) = out.alt_click {
            preview_at(&e.code, at, ui, acts);
        }
        return;
    }
    let drafted = st.draft.as_ref().filter(|d| d.key == *key).map(|d| d.text.clone());
    let editing = drafted.is_some();
    let mut text = drafted.unwrap_or_else(|| e.code.clone());
    let list_open = st.completions.as_ref().is_some_and(|c| c.key == *key && !c.items.is_empty());
    // The underlines belong to the stored code: typed text moves them.
    let ranges = if editing && text != e.code { Vec::new() } else { ranges };
    let out = code_editor::show(ui, tokens, editor_id(), Text::Edit(&mut text), height, &ranges, list_open);
    if let Some(c) = st.completions.as_mut().filter(|_| list_open) {
        let keys = out.list_keys;
        if keys.close {
            st.completions = None;
        } else if keys.accept {
            if let Some(s) = c.items.get(c.highlighted) {
                let start = code_editor::floor_boundary(&text, s.replace.start);
                let end = code_editor::floor_boundary(&text, s.replace.end).max(start);
                text.replace_range(start..end, &s.label);
                st.caret = Some(code_editor::byte_to_char(&text, start + s.label.len()));
            }
            st.completions = None;
        } else if keys.moved != 0 {
            let last = c.items.len().saturating_sub(1);
            let step = usize::try_from(keys.moved.unsigned_abs()).unwrap_or(usize::MAX);
            c.highlighted = if keys.moved < 0 { c.highlighted.saturating_sub(step) } else { c.highlighted.saturating_add(step).min(last) };
        }
    }
    if out.focused {
        if let Some(sel) = out.selection.clone() {
            st.selection = Some((key.clone(), sel));
        }
        if out.changed {
            let cursor = out.selection.as_ref().map_or(text.len(), |s| code_editor::char_to_byte(&text, s.end));
            if wants_completion(&text, cursor) {
                let at = out.cursor_pos.unwrap_or(out.response.rect.left_bottom());
                acts.push(Act::Complete { key: key.clone(), code: text.clone(), position: cursor, at });
            } else {
                st.completions = None;
            }
        }
    }
    if out.focused {
        st.draft = Some(Draft { key: key.clone(), text });
    } else if editing {
        // The editor lost the keyboard: the typed text is committed.
        if let Some(d) = st.draft.as_mut() {
            d.text = text;
        }
        flush(st, font, pending);
    }
    if let Some(at) = out.alt_click {
        let shown = st.draft.as_ref().filter(|d| d.key == *key).map_or(e.code.as_str(), |d| d.text.as_str());
        preview_at(shown, at, ui, acts);
    }
}

/// An Alt-click at byte `at` of `code`: the preview of the glyph name, class name or token there.
fn preview_at(code: &str, at: usize, ui: &egui::Ui, acts: &mut Vec<Act>) {
    let Some((range, kind)) = code_editor::previewable_at(code, at) else { return };
    let Some(word) = code.get(range) else { return };
    let pos = ui.ctx().pointer_latest_pos().unwrap_or_default();
    match kind {
        fontcraft_engine::features::edit::HighlightKind::Token => acts.push(Act::Token { token: word.to_string(), at: pos }),
        _ => acts.push(Act::Preview { name: word.to_string(), at: pos }),
    }
}

/// The divider and the notes field under the code.
#[allow(clippy::too_many_arguments)]
fn notes(ui: &mut egui::Ui, tokens: &Tokens, font: &Font, st: &mut FeaturesState, key: &EntryKey, e: &Snippet, height: f32, pending: &mut Pending) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, DIVIDER_HEIGHT), Sense::hover());
    let divider = ui.interact(rect, divider_id(), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeVertical);
    ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, tokens.border));
    if divider.dragged() {
        // Dragging the divider up makes the notes taller.
        st.notes_height = Some(height - divider.drag_delta().y);
    }
    let drafted = st.notes.as_ref().filter(|d| d.key == *key).map(|d| d.text.clone());
    let editing = drafted.is_some();
    let mut text = drafted.unwrap_or_else(|| e.notes.clone());
    let rows = (height / tokens.list_row_height.max(1.0)).floor().max(1.0) as usize;
    let response = ui.add(
        egui::TextEdit::multiline(&mut text)
            .id(notes_id())
            .desired_width(f32::INFINITY)
            .desired_rows(rows)
            .hint_text("Notes about this entry, kept in the source and never exported"),
    );
    if response.has_focus() {
        st.notes = Some(Draft { key: key.clone(), text });
    } else if editing {
        if let Some(d) = st.notes.as_mut() {
            d.text = text;
        }
        let draft = st.draft.take();
        flush(st, font, pending);
        st.draft = draft;
    }
}

// ── Popovers ────────────────────────────────────────────────────────────────────────────────────

/// Whether a press this frame landed outside `rect`, after the pass `opened`.
fn pressed_outside(ctx: &egui::Context, rect: Rect, opened: u64) -> bool {
    ctx.cumulative_pass_nr() > opened && ctx.input(|i| i.pointer.any_pressed() && i.pointer.press_origin().is_some_and(|p| !rect.contains(p)))
}

/// The autocomplete list under the cursor. It shows the suggestions; the editor's keys move
/// through it and accept one.
fn completion_list(ctx: &egui::Context, tokens: &Tokens, st: &mut FeaturesState) {
    let Some(c) = &st.completions else { return };
    if c.items.is_empty() {
        st.completions = None;
        return;
    }
    egui::Area::new(completions_id()).order(egui::Order::Foreground).fixed_pos(c.at).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_width(tokens.list_column_unit * 4.0);
            egui::ScrollArea::vertical().max_height(tokens.list_row_height * COMPLETION_ROWS).show(ui, |ui| {
                for (i, s) in c.items.iter().enumerate() {
                    let text = format!("{}  {}", s.label, s.kind);
                    let response = item(ui, tokens, completions_id().with(i), &text, i == c.highlighted);
                    if i == c.highlighted {
                        response.scroll_to_me(None);
                    }
                }
            });
        });
    });
}

/// The popover of an Alt-clicked name or token, or of an error icon.
fn popover(ctx: &egui::Context, tokens: &Tokens, font: &Font, st: &mut FeaturesState, master: Option<&str>, pending: &mut Pending) {
    let Some(p) = st.popover.clone() else { return };
    let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
    let mut fix = None;
    let shown = egui::Area::new(popover_id()).order(egui::Order::Foreground).fixed_pos(p.at).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_max_width(tokens.list_column_unit * 8.0);
            match &p.content {
                PopoverContent::Name(Ok(r)) => name_preview(ui, tokens, font, master, r),
                PopoverContent::Token(Ok(r)) => token_preview(ui, tokens, r),
                PopoverContent::Error(id, Ok(r)) => {
                    let details = r.get("details").and_then(Value::as_str).unwrap_or_default();
                    ui.label(RichText::new(details).monospace());
                    for (i, label) in strings(r.get("fixes")).iter().enumerate() {
                        if widgets::button(ui, fix_id(i), &format!("Fix: {label}"), "Make the suggested correction", true, false) {
                            fix = Some((*id, i));
                        }
                    }
                }
                PopoverContent::Name(Err(e)) | PopoverContent::Token(Err(e)) | PopoverContent::Error(_, Err(e)) => {
                    ui.label(RichText::new(e.as_str()).color(tokens.error_text));
                }
            }
        });
    });
    if pressed_outside(ctx, shown.response.rect, p.opened) {
        close = true;
    }
    if let Some((error, at)) = fix {
        pending.push(("font.features.applyFix", json!({"error": error, "fix": at})));
        close = true;
    }
    if close {
        st.popover = None;
    }
}

fn glyph_names(v: Option<&Value>) -> Vec<String> {
    strings(v).into_iter().take(MAX_PREVIEW_GLYPHS).collect()
}

/// A glyph: its drawing, name and code points; a class: its members.
fn name_preview(ui: &mut egui::Ui, tokens: &Tokens, font: &Font, master: Option<&str>, r: &Value) {
    let name = r.get("name").and_then(Value::as_str).unwrap_or_default();
    if r.get("kind").and_then(Value::as_str) == Some("glyph") {
        let side = tokens.cell_size;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), Sense::hover());
        if let Some(master) = master.or_else(|| font.masters.first().map(|m| m.id.as_str())) {
            crate::master_icons::paint_glyph(ui.painter(), rect, font, name, master, tokens.text);
        }
        ui.label(RichText::new(name).monospace().strong());
        let unicodes: Vec<String> = strings(r.get("unicodes")).iter().map(|u| format!("U+{u}")).collect();
        if !unicodes.is_empty() {
            ui.label(unicodes.join(" "));
        }
        if r.get("export").and_then(Value::as_bool) == Some(false) {
            ui.label(RichText::new("Not exported").color(tokens.text_dim));
        }
        return;
    }
    let glyphs = glyph_names(r.get("glyphs"));
    let total = r.get("glyphs").and_then(Value::as_array).map_or(0, Vec::len);
    ui.label(RichText::new(format!("@{name}: {total} glyphs")).strong());
    ui.label(RichText::new(glyphs.join(" ")).monospace());
    let unknown = glyph_names(r.get("unknown"));
    if !unknown.is_empty() {
        ui.label(RichText::new(format!("Not in the font: {}", unknown.join(" "))).color(tokens.error_text));
    }
}

/// A token's value: a number, or the glyphs a predicate matches.
fn token_preview(ui: &mut egui::Ui, tokens: &Tokens, r: &Value) {
    let token = r.get("token").and_then(Value::as_str).unwrap_or_default();
    ui.label(RichText::new(token).monospace().strong());
    if let Some(value) = r.get("value").filter(|_| r.get("kind").and_then(Value::as_str) == Some("number")) {
        ui.label(format!("= {value}"));
        return;
    }
    let glyphs = glyph_names(r.get("glyphs"));
    let total = r.get("glyphs").and_then(Value::as_array).map_or(0, Vec::len);
    ui.label(RichText::new(format!("{total} glyphs")).color(tokens.text_dim));
    ui.label(RichText::new(glyphs.join(" ")).monospace());
}

/// The snippets menu over the Snippets button: the built-in snippets, one group per user file,
/// and Show Snippet Folder.
fn snippet_menu(ctx: &egui::Context, tokens: &Tokens, font: &Font, st: &mut FeaturesState, pending: &mut Pending, acts: &mut Vec<Act>) {
    let Some(menu) = st.snippets.clone() else { return };
    let target = st.primary().cloned().filter(|k| k.snippet(&font.features).is_some_and(|e| !e.automatic));
    let mut chosen: Option<(Option<String>, String)> = None;
    let mut folder = false;
    let shown = egui::Area::new(snippet_menu_id())
        .order(egui::Order::Foreground)
        .pivot(Align2::RIGHT_BOTTOM)
        .fixed_pos(menu.button.right_top())
        .show(ctx, |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_width(tokens.list_column_unit * 5.0);
                let Some(listing) = &menu.listing else {
                    ui.label(RichText::new("Loading…").color(tokens.text_dim));
                    return;
                };
                if let Some(e) = &listing.error {
                    ui.label(RichText::new(e.as_str()).color(tokens.error_text));
                }
                let enabled = target.is_some();
                for title in &listing.built_in {
                    if widgets::button(ui, snippet_item_id(None, title), title, "Insert this snippet at the cursor", enabled, false) {
                        chosen = Some((None, title.clone()));
                    }
                }
                for (group, titles) in &listing.groups {
                    ui.separator();
                    ui.label(RichText::new(group.as_str()).color(tokens.text_dim));
                    for title in titles {
                        if widgets::button(ui, snippet_item_id(Some(group), title), title, "Insert this snippet at the cursor", enabled, false) {
                            chosen = Some((Some(group.clone()), title.clone()));
                        }
                    }
                }
                for w in &listing.warnings {
                    ui.label(RichText::new(w.as_str()).color(tokens.text_dim));
                }
                ui.separator();
                if widgets::button(ui, show_folder_id(), "Show Snippet Folder", "Show the folder user snippet files go in", true, false) {
                    folder = true;
                }
            });
        });
    let mut close = ctx.input(|i| i.key_pressed(Key::Escape)) || pressed_outside(ctx, shown.response.rect.union(menu.button), menu.opened);
    if let (Some((group, title)), Some(key)) = (chosen, target) {
        // The snippet goes over the selection of the code as it is typed, so the typed code is
        // committed first.
        let selection = st.selection.as_ref().filter(|(k, _)| *k == key).map(|(_, s)| s.clone());
        flush(st, font, pending);
        let mut extra = vec![("snippet", json!(title))];
        if let Some(group) = group {
            extra.push(("group", json!(group)));
        }
        if let Some(s) = selection {
            extra.push(("position", json!(s.start)));
            extra.push(("end", json!(s.end)));
        }
        pending.push(("font.features.insertSnippet", key.with(&extra)));
        close = true;
    }
    if folder {
        acts.push(Act::ShowFolder);
        close = true;
    }
    if close {
        st.snippets = None;
    }
}

// ── Queries ─────────────────────────────────────────────────────────────────────────────────────

/// Runs a query the drawing asked for and keeps its result in the view state.
fn perform(app: &mut FontcraftApp, ctx: &egui::Context, master: Option<&str>, act: Act) {
    let opened = ctx.cumulative_pass_nr();
    let with_master = |mut p: Map<String, Value>| {
        if let Some(m) = master {
            p.insert("master".into(), json!(m));
        }
        Value::Object(p)
    };
    match act {
        Act::Complete { key, code, position, at } => {
            let params = key.with(&[("position", json!(position)), ("code", json!(code))]);
            let items: Vec<Suggestion> = app
                .run("font.features.complete", &params)
                .ok()
                .and_then(|r| r.get("completions").and_then(Value::as_array).cloned())
                .unwrap_or_default()
                .iter()
                .take(MAX_COMPLETIONS)
                .filter_map(|c| {
                    let replace = c.get("replace").and_then(Value::as_array)?;
                    let start = usize::try_from(replace.first()?.as_u64()?).ok()?;
                    let end = usize::try_from(replace.get(1)?.as_u64()?).ok()?;
                    Some(Suggestion {
                        label: c.get("label")?.as_str()?.to_string(),
                        kind: c.get("kind").and_then(Value::as_str).unwrap_or_default().to_string(),
                        replace: start..end.max(start),
                    })
                })
                .collect();
            let st = &mut app.state.panels.font_info.features;
            st.completions = (!items.is_empty()).then(|| Completions { key, items, highlighted: 0, at });
        }
        Act::Preview { name, at } => {
            let mut p = Map::new();
            p.insert("name".into(), json!(name));
            let result = app.run("font.features.previewName", &with_master(p));
            app.state.panels.font_info.features.popover = Some(Popover { at, opened, content: PopoverContent::Name(result) });
        }
        Act::Token { token, at } => {
            let mut p = Map::new();
            p.insert("token".into(), json!(token));
            let result = app.run("font.features.evaluateToken", &with_master(p));
            app.state.panels.font_info.features.popover = Some(Popover { at, opened, content: PopoverContent::Token(result) });
        }
        Act::Details { id, at } => {
            let result = app.run("font.features.showErrorDetails", &json!({"error": id}));
            app.state.panels.font_info.features.popover = Some(Popover { at, opened, content: PopoverContent::Error(id, result) });
        }
        Act::Jump(key) => match app.run("font.features.jumpToError", &Value::Object(key.params())) {
            Ok(r) => {
                let start = r.get("range").and_then(Value::as_array).and_then(|a| a.first()?.as_u64()).and_then(|s| usize::try_from(s).ok());
                let code = app.session.font().ok().and_then(|f| key.snippet(&f.features).map(|e| e.code.clone())).unwrap_or_default();
                let st = &mut app.state.panels.font_info.features;
                st.selected = vec![key];
                st.caret = start.map(|s| code_editor::byte_to_char(&code, s));
                ctx.memory_mut(|m| m.request_focus(editor_id()));
            }
            Err(e) => app.state.status = e,
        },
        Act::Spec(tag) => match app.run("font.features.openSpec", &json!({"tag": tag})) {
            Ok(r) => {
                if let Some(url) = r.get("url").and_then(Value::as_str) {
                    ctx.open_url(egui::OpenUrl::new_tab(url));
                    app.state.status = format!("Opened the specification of {tag}: {url}");
                }
            }
            Err(e) => app.state.status = e,
        },
        Act::ListSnippets => {
            let listing = match app.run("font.features.listSnippets", &json!({})) {
                Ok(r) => SnippetListing {
                    built_in: titles(r.get("builtIn")),
                    groups: r
                        .get("groups")
                        .and_then(Value::as_array)
                        .map(|g| {
                            g.iter()
                                .take(MAX_MENU_SNIPPETS)
                                .filter_map(|g| Some((g.get("name")?.as_str()?.to_string(), titles(g.get("snippets")))))
                                .collect()
                        })
                        .unwrap_or_default(),
                    warnings: strings(r.get("warnings")),
                    error: None,
                },
                Err(e) => SnippetListing { error: Some(e), ..Default::default() },
            };
            if let Some(menu) = &mut app.state.panels.font_info.features.snippets {
                menu.listing = Some(listing);
            }
        }
        Act::ShowFolder => match app.run("font.features.showSnippetFolder", &json!({})) {
            Ok(r) => {
                let path = r.get("path").and_then(Value::as_str).unwrap_or_default().to_string();
                let shown = app.state.start.reveal.0.map(|reveal| reveal(std::path::Path::new(&path)));
                app.state.status = match shown {
                    Some(Err(e)) => e,
                    _ => format!("Snippet folder: {path}"),
                };
            }
            Err(e) => app.state.status = e,
        },
    }
}

fn titles(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().take(MAX_MENU_SNIPPETS).filter_map(|s| s.get("title")?.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// Draws the Add Feature picker while it is open; Add queues its command.
fn picker(app: &mut FontcraftApp, ctx: &egui::Context, pending: &mut Pending) {
    let FontcraftApp { state, tokens, .. } = &mut *app;
    let st = &mut state.panels.font_info.features;
    let Some(p) = st.picker.as_mut() else { return };
    match feature_picker::show(ctx, tokens, p) {
        Some(Outcome::Add) => {
            if let Some(command) = p.command() {
                pending.push(command);
            }
            st.picker = None;
        }
        Some(Outcome::Cancel) => st.picker = None,
        None => {}
    }
}

#[cfg(test)]
#[path = "features_tab_tests.rs"]
mod tests;
