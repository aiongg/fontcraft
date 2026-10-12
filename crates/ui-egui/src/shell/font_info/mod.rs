//! The Font Info window: one per document, with the Font, Masters, Exports, Features, Other Settings
//! and Notes tabs ([`Tab`]). Every control acts through `app.run` with an engine command
//! (`font.info.*`, `font.customParameters.*`, `font.master.*`, `font.instance.*`,
//! `font.metrics.*`, `font.stems.*`, `font.settings.*`, `font.note.*`); the window keeps only view
//! state ([`FontInfoState`]).
//!
//! | Module | Package | What it draws |
//! |---|---|---|
//! | `mod.rs` | M7.15 | the window, the tab bar ([`TABS`]), the `font.info` and `font.info.selectTab` commands, Copy, Paste and Cmd+Delete on the selected rows, the open picker |
//! | [`rows`] | M7.15 | the shared row: plus and minus, Alt, selection, Copy and Paste, localizable rows |
//! | [`param_value`] | M7.15 | the value editor of each parameter kind |
//! | [`font_tab`] | M7.16 | Family Name, Units per Em, Version, Date, General fields, Axes, Custom Parameters |
//! | [`masters_tab`] | M7.17 | the master list and its General, Metrics, Stems and Custom Parameters sections |
//! | [`exports_tab`] | M7.18 | the export list, the instance preview, the instance fields |
//! | [`other_tab`] | M7.16 | Other Settings |
//! | [`notes_tab`] | M7.16 | the font note with Markdown highlighting |
//!
//! A tab mounts by naming its `show` in [`TABS`]; it draws its rows with the sections of
//! [`rows`] (General, Custom Parameters, built-in fields), which record what they show for the
//! selection and the pickers. [`FontInfoState::target`] says what the shown tab acts on.
//!
//! While the window has the keyboard (the last press landed in it), Copy puts the selected rows
//! on the clipboard (`font.info.copyRows`, JSON), Paste pastes into the shown tab's targets
//! (`font.info.pasteRows`, replacing rows with the same key), and Cmd+Delete removes the
//! selected rows.
//!
//! Rows: `fontInfo`, `font.info`, `font.info.selectTab`, `fontInfo.fieldPicker`,
//! `fontInfo.customParameterPicker` (M7.15) and the tab rows of each package.

pub mod exports_tab;
pub mod font_tab;
pub mod masters_tab;
pub mod notes_tab;
pub mod other_tab;
pub mod param_value;
pub mod rows;
#[cfg(test)]
mod tests;

use std::collections::HashSet;

use egui::{Align2, Id, RichText};
use fontcraft_engine::fontinfo::Scope;
use fontcraft_engine::fontinfo::rows::RowKind;
use fontcraft_model::Font;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use self::rows::{ShownRow, Target};
use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::dialogs::custom_parameter_picker::{self, ParameterPicker};
use crate::dialogs::field_picker::{self, FieldPicker, Outcome};
use crate::panels::{Pending, widgets};
use crate::theme::Tokens;

/// The window's commands (`font.info`, `font.info.selectTab`).
pub const COMMANDS: &[UiCommand] = &[
    (
        "font.info",
        "Font Info…",
        &["File"],
        "Cmd+I",
        "{tab?: 'font'|'masters'|'exports'|'features'|'other'|'notes'} open the Font Info window of the current font, on `tab` or on the tab it showed last → {open: true, tab}",
    ),
    (
        "font.info.selectTab",
        "Select Font Info Tab",
        &[],
        "",
        "{tab: 'font'|'masters'|'exports'|'features'|'other'|'notes'} show one tab of the Font Info window, opening the window when it is closed → {open: true, tab}",
    ),
];

/// The engine command whose shortcut removes the selected rows while the window has the keyboard.
const REMOVE_FIELD: &str = "font.info.removeField";
/// The window's default size, in list columns and list rows of the theme.
const WINDOW_COLUMNS: f32 = 16.0;
const WINDOW_ROWS: f32 = 26.0;
/// Most feature snippets the read-only Features tab lists per group.
const MAX_SNIPPETS_SHOWN: usize = 512;

/// The tabs, in order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tab {
    #[default]
    Font,
    Masters,
    Exports,
    Features,
    Other,
    Notes,
}

impl Tab {
    pub const ALL: [Tab; 6] = [Tab::Font, Tab::Masters, Tab::Exports, Tab::Features, Tab::Other, Tab::Notes];

    /// The `tab` parameter of `font.info.selectTab`: `font`, `masters`, `exports`, `features`,
    /// `other`, `notes`.
    pub fn name(self) -> &'static str {
        match self {
            Tab::Font => "font",
            Tab::Masters => "masters",
            Tab::Exports => "exports",
            Tab::Features => "features",
            Tab::Other => "other",
            Tab::Notes => "notes",
        }
    }

    pub fn from_name(name: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.name() == name)
    }

    /// The tab bar's label.
    pub fn label(self) -> &'static str {
        match self {
            Tab::Font => "Font",
            Tab::Masters => "Masters",
            Tab::Exports => "Exports",
            Tab::Features => "Features",
            Tab::Other => "Other Settings",
            Tab::Notes => "Notes",
        }
    }
}

/// Draws one tab's contents inside the window.
pub type TabFn = fn(&mut FontcraftApp, &mut egui::Ui, &mut Pending);

/// The function that draws each tab: a tab's package mounts it by naming its `show` here. The
/// Features tab shows the stored feature code read-only until its own widget takes this slot.
pub const TABS: &[(Tab, TabFn)] = &[
    (Tab::Font, font_tab::show),
    (Tab::Masters, masters_tab::show),
    (Tab::Exports, exports_tab::show),
    (Tab::Features, features_read_only),
    (Tab::Other, other_tab::show),
    (Tab::Notes, notes_tab::show),
];

/// The picker open over the window.
#[derive(Clone, Debug, PartialEq)]
pub enum Picker {
    Field(FieldPicker),
    Parameter(ParameterPicker),
}

/// The window's view state. Selections are not saved with the settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FontInfoState {
    pub open: bool,
    pub tab: Tab,
    /// Selected rows of the shown tab, as indices into its row list ([`Self::shown_rows`]).
    #[serde(skip)]
    pub selected_rows: Vec<usize>,
    /// Selected masters of the Masters tab, as indices into the font's masters.
    #[serde(skip)]
    pub selected_masters: Vec<usize>,
    /// Selected export entries of the Exports tab, as indices into the font's instances.
    #[serde(skip)]
    pub selected_instances: Vec<usize>,
    /// The rows the shown tab drew in the last frame, in drawing order.
    #[serde(skip)]
    pub shown_rows: Vec<ShownRow>,
    /// The row a Shift-click selects from: the last row clicked.
    #[serde(skip)]
    pub anchor: Option<usize>,
    #[serde(skip)]
    pub picker: Option<Picker>,
    /// The window has the keyboard: the last press landed in it (or in a menu). Copy, Paste and
    /// Cmd+Delete then act on its rows.
    #[serde(skip)]
    pub keys: bool,
    /// A text field had the keyboard at the end of the last frame.
    #[serde(skip)]
    pub text_focus: bool,
    /// The Masters tab's icon picker or metric editor was open at the end of the last frame.
    #[serde(skip)]
    pub popup: bool,
}

impl FontInfoState {
    /// A click on the shown row `index`: alone it selects only that row, with `extend` (Cmd) it
    /// adds or removes it, with `range` (Shift) it adds the rows from the last row clicked.
    pub fn select(&mut self, index: usize, extend: bool, range: bool) {
        match (range, self.anchor) {
            (true, Some(anchor)) => {
                let last = self.shown_rows.len().saturating_sub(1);
                let (lo, hi) = (anchor.min(index), anchor.max(index).min(last));
                self.selected_rows.extend(lo..=hi);
                self.selected_rows.sort_unstable();
                self.selected_rows.dedup();
            }
            _ if extend => {
                match self.selected_rows.iter().position(|i| *i == index) {
                    Some(at) => {
                        self.selected_rows.remove(at);
                    }
                    None => self.selected_rows.push(index),
                }
                self.anchor = Some(index);
            }
            _ => {
                self.selected_rows = vec![index];
                self.anchor = Some(index);
            }
        }
    }

    /// Shows `tab`; a new tab starts with no row selected and no picker.
    pub fn select_tab(&mut self, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            self.clear_selection();
            self.picker = None;
        }
    }

    pub fn clear_selection(&mut self) {
        self.selected_rows.clear();
        self.anchor = None;
    }

    /// What the shown tab's rows act on: the font; the selected masters or exports (the first
    /// one when none is selected); `None` for the other tabs or when there is no master or
    /// export.
    pub fn target(&self, font: &Font) -> Option<Target> {
        let pick = |selected: &[usize], count: usize, scope: Scope| {
            let mut indices: Vec<usize> = selected.iter().copied().filter(|i| *i < count).collect();
            if indices.is_empty() {
                if count == 0 {
                    return None;
                }
                indices.push(0);
            }
            Some(Target { scope, indices })
        };
        match self.tab {
            Tab::Font => Some(Target::font()),
            Tab::Masters => pick(&self.selected_masters, font.masters.len(), Scope::Master),
            Tab::Exports => pick(&self.selected_instances, font.instances.len(), Scope::Instance),
            Tab::Features | Tab::Other | Tab::Notes => None,
        }
    }

    /// The selected shown rows, in drawing order.
    fn selected(&self) -> Vec<&ShownRow> {
        let mut at = self.selected_rows.clone();
        at.sort_unstable();
        at.dedup();
        at.into_iter().filter_map(|i| self.shown_rows.get(i)).collect()
    }
}

/// Runs one of [`COMMANDS`].
pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    let tab = match args.string("tab")? {
        Some(name) => Some(
            Tab::from_name(name).ok_or_else(|| args.err(format!("unknown tab `{name}`: use font, masters, exports, features, other or notes")))?,
        ),
        None => None,
    };
    let tab = match id {
        "font.info" => tab,
        "font.info.selectTab" => Some(tab.ok_or_else(|| args.err("`tab` is required"))?),
        _ => return Err(args.err(format!("unknown command `{id}`"))),
    };
    app.session.font().map_err(|e| args.err(e))?;
    let fi = &mut app.state.panels.font_info;
    fi.open = true;
    if let Some(tab) = tab {
        fi.select_tab(tab);
    }
    Ok(json!({"open": true, "tab": fi.tab.name()}))
}

/// The id of the window.
pub fn window_id() -> Id {
    widgets::id(("fontInfo", "window"))
}

/// The id of a tab's button in the tab bar.
pub fn tab_id(tab: Tab) -> Id {
    widgets::id(("fontInfo.tab", tab.name()))
}

/// Draws the window while it is open: the tab bar, then the shown tab's `show`, then the open
/// picker over everything.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.font_info.open {
        return;
    }
    if app.session.font().is_err() {
        // The last document closed.
        app.state.panels.font_info.open = false;
        app.state.panels.font_info.picker = None;
        return;
    }
    list_picker(app);
    let tokens = app.tokens.clone();
    let ctx = ui.ctx().clone();
    let tab = app.state.panels.font_info.tab;
    let before = std::mem::take(&mut app.state.panels.font_info.shown_rows);
    let mut open = true;
    let mut picked = None;
    let shown = egui::Window::new("Font Info")
        .id(window_id())
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(egui::vec2(tokens.list_column_unit * WINDOW_COLUMNS, tokens.list_row_height * WINDOW_ROWS))
        .pivot(Align2::CENTER_TOP)
        .default_pos(egui::pos2(ctx.content_rect().center().x, tokens.dialog_top))
        .show(&ctx, |ui| {
            picked = tab_bar(ui, tab);
            ui.separator();
            egui::ScrollArea::vertical().id_salt(("fontInfo.scroll", tab.name())).auto_shrink([false, false]).show(ui, |ui| {
                if let Some((_, draw)) = TABS.iter().find(|(t, _)| *t == tab) {
                    draw(app, ui, pending);
                }
            });
        });
    // Rows that moved or went away take the selection with them.
    let fi = &mut app.state.panels.font_info;
    if fi.shown_rows != before && !before.is_empty() {
        fi.clear_selection();
    }
    if let Some(tab) = picked {
        pending.push(("font.info.selectTab", json!({"tab": tab.name()})));
    }
    keyboard(app, &ctx, shown.map(|r| r.response.rect), pending);
    if !open {
        let fi = &mut app.state.panels.font_info;
        fi.open = false;
        fi.picker = None;
        fi.keys = false;
    }
    draw_picker(app, &ctx, &tokens, pending);
}

/// The tab bar; the tab clicked, if another than `current`.
fn tab_bar(ui: &mut egui::Ui, current: Tab) -> Option<Tab> {
    let mut picked = None;
    ui.horizontal(|ui| {
        for tab in Tab::ALL {
            if widgets::button(ui, tab_id(tab), tab.label(), tab.label(), true, tab == current) && tab != current {
                picked = Some(tab);
            }
        }
    });
    picked
}

/// The Features tab until its own widget mounts: the stored prefixes, classes and features by
/// name, read-only.
fn features_read_only(app: &mut FontcraftApp, ui: &mut egui::Ui, _pending: &mut Pending) {
    let Ok(font) = app.session.font() else { return };
    let tokens = &app.tokens;
    let features = &font.features;
    for (title, list) in [("Prefixes", &features.prefixes), ("Classes", &features.classes), ("Features", &features.features)] {
        ui.add_space(tokens.dialog_spacing);
        ui.label(RichText::new(title).strong().size(tokens.section_header_size));
        if list.is_empty() {
            ui.label(RichText::new("None").color(tokens.text_dim));
        }
        for snippet in list.iter().take(MAX_SNIPPETS_SHOWN) {
            let color = if snippet.disabled { tokens.text_dim } else { tokens.text };
            ui.label(RichText::new(snippet.name.as_str()).monospace().color(color));
        }
        if list.len() > MAX_SNIPPETS_SHOWN {
            ui.label(RichText::new(format!("and {} more", list.len() - MAX_SNIPPETS_SHOWN)).color(tokens.text_dim));
        }
    }
}

/// Whether the window has the keyboard: it is open, the last press landed in it, and no text
/// field or dialog has the keyboard. Copy, Paste and the window's shortcuts are then its own,
/// even while a picker over it is open.
pub fn has_keyboard(app: &FontcraftApp) -> bool {
    let fi = &app.state.panels.font_info;
    fi.open && fi.keys && !fi.text_focus && app.dialog.is_none()
}

/// Whether Copy, Paste and Cmd+Delete act on the window's rows: it has the keyboard and no
/// picker, icon picker or metric editor is open over it.
pub fn takes_clipboard(app: &FontcraftApp) -> bool {
    let fi = &app.state.panels.font_info;
    has_keyboard(app) && fi.picker.is_none() && !fi.popup
}

/// A menu item or shortcut while the window has the keyboard: Edit › Copy copies the selected
/// rows, and `font.info.removeField` (Cmd+Delete, which the keymap gives the window while it has
/// the keyboard) removes them. `None`: the run is not the window's.
pub fn menu_command(app: &mut FontcraftApp, id: &str) -> Option<Result<Value, String>> {
    if !has_keyboard(app) {
        return None;
    }
    if !takes_clipboard(app) {
        // A picker is open over the window: Copy and Cmd+Delete do nothing rather than reach
        // the glyphs behind it.
        return (id == "edit.copy" || id == REMOVE_FIELD).then_some(Ok(Value::Null));
    }
    if id == "edit.copy" {
        return Some(copy(app));
    }
    if id == REMOVE_FIELD {
        return Some(remove_selected(app));
    }
    None
}

/// Who has the keyboard, and this frame's Copy and Paste while the window has it.
fn keyboard(app: &mut FontcraftApp, ctx: &egui::Context, window: Option<egui::Rect>, pending: &mut Pending) {
    let menu_bar = app.menu_bar_rect;
    let press = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.press_origin() } else { None });
    let on_menu = |p: egui::Pos2| menu_bar.contains(p) || ctx.layer_id_at(p).is_some_and(|l| l.order >= egui::Order::Foreground);
    let fi = &mut app.state.panels.font_info;
    if let Some(p) = press {
        if window.is_some_and(|r| r.contains(p)) {
            fi.keys = true;
        } else if !on_menu(p) {
            fi.keys = false;
        }
    }
    fi.text_focus = ctx.text_edit_focused();
    fi.popup = masters_tab::open_editor(ctx).is_some() || masters_tab::open_icon_picker(ctx).is_some();
    if !takes_clipboard(app) {
        return;
    }
    let events: Vec<egui::Event> =
        ctx.input(|i| i.events.iter().filter(|e| matches!(e, egui::Event::Copy | egui::Event::Paste(_))).cloned().collect());
    for event in events {
        match event {
            egui::Event::Copy => {
                if let Err(e) = copy(app) {
                    app.state.status = e;
                }
            }
            egui::Event::Paste(text) => paste(app, text, pending),
            _ => {}
        }
    }
}

/// Puts the selected rows of the first target on the clipboard as `font.info.copyRows` writes
/// them (JSON). A parameter name copies every row of that name.
fn copy(app: &mut FontcraftApp) -> Result<Value, String> {
    let params = {
        let fi = &app.state.panels.font_info;
        let font = app.session.font().map_err(|e| e.to_string())?;
        let Some(target) = fi.target(font) else { return Ok(json!({"rows": 0})) };
        let mut seen = HashSet::new();
        let keys: Vec<String> = fi.selected().into_iter().filter(|r| seen.insert(r.key.as_str())).map(|r| r.key.clone()).collect();
        if keys.is_empty() {
            return Ok(json!({"rows": 0}));
        }
        let mut p = target.each().into_iter().next().unwrap_or_default();
        p.insert("keys".into(), json!(keys));
        Value::Object(p)
    };
    let result = app.run("font.info.copyRows", &params)?;
    if let Some(text) = result.get("json").and_then(Value::as_str) {
        app.clipboard_out = Some(text.to_string());
    }
    let count = result.get("rows").and_then(Value::as_array).map_or(0, Vec::len);
    app.state.status = format!("Copied {count} Font Info rows");
    Ok(json!({"rows": count}))
}

/// Queues `font.info.pasteRows` with the clipboard's text into the shown tab's targets; rows
/// with a key the target has replace it.
fn paste(app: &FontcraftApp, text: String, pending: &mut Pending) {
    let Ok(font) = app.session.font() else { return };
    let Some(target) = app.state.panels.font_info.target(font) else { return };
    let mut p = target.params();
    p.insert("text".into(), json!(text));
    p.insert("mode".into(), json!("replace"));
    pending.push(("font.info.pasteRows", Value::Object(p)));
}

/// Removes the selected rows: the added fields with one `font.info.removeField`, then each
/// parameter with `font.customParameters.remove`, the later rows of a name first so the earlier
/// ones keep their place. Built-in fields stay.
fn remove_selected(app: &mut FontcraftApp) -> Result<Value, String> {
    let (target, fields, mut params) = {
        let fi = &app.state.panels.font_info;
        let font = app.session.font().map_err(|e| e.to_string())?;
        let Some(target) = fi.target(font) else { return Ok(json!({"removed": 0})) };
        let rows: Vec<&ShownRow> = fi.selected().into_iter().filter(|r| !r.built_in).collect();
        let fields: Vec<String> = rows.iter().filter(|r| r.kind == RowKind::Field).map(|r| r.key.clone()).collect();
        let params: Vec<(String, usize)> = rows.iter().filter(|r| r.kind == RowKind::Param).map(|r| (r.key.clone(), r.occurrence)).collect();
        (target, fields, params)
    };
    params.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    let mut removed = 0usize;
    if !fields.is_empty() {
        let mut p = target.params();
        p.insert("keys".into(), json!(fields));
        app.run(REMOVE_FIELD, &Value::Object(p))?;
        removed = fields.len();
    }
    for (name, index) in params {
        let mut p = target.params();
        p.insert("name".into(), json!(name));
        p.insert("index".into(), json!(index));
        app.run("font.customParameters.remove", &Value::Object(p))?;
        removed = removed.saturating_add(1);
    }
    app.state.panels.font_info.clear_selection();
    Ok(json!({"removed": removed}))
}

/// Fills the open picker's list with its query when the list is missing or older than the
/// search text.
fn list_picker(app: &mut FontcraftApp) {
    let (id, params) = match &app.state.panels.font_info.picker {
        Some(Picker::Field(p)) if p.choices.is_none() && p.error.is_none() => ("font.info.listFields", p.query()),
        Some(Picker::Parameter(p)) if p.needs_listing() => ("font.customParameters.describe", p.query_params()),
        _ => return,
    };
    let result = app.run(id, &params);
    match (&mut app.state.panels.font_info.picker, result) {
        (Some(Picker::Field(p)), Ok(r)) => p.fill(&r),
        (Some(Picker::Parameter(p)), Ok(r)) => p.fill(&r),
        (Some(Picker::Field(p)), Err(e)) => p.error = Some(e),
        (Some(Picker::Parameter(p)), Err(e)) => {
            p.error = Some(e);
            p.listed = Some(p.query.clone());
        }
        (None, _) => {}
    }
}

/// Draws the open picker; Add queues its commands, and either button closes it.
fn draw_picker(app: &mut FontcraftApp, ctx: &egui::Context, tokens: &Tokens, pending: &mut Pending) {
    let fi = &mut app.state.panels.font_info;
    let outcome = match &mut fi.picker {
        Some(Picker::Field(p)) => field_picker::show(ctx, tokens, p),
        Some(Picker::Parameter(p)) => custom_parameter_picker::show(ctx, tokens, p),
        None => return,
    };
    match outcome {
        Some(Outcome::Add) => {
            match &fi.picker {
                Some(Picker::Field(p)) => pending.extend(p.commands()),
                Some(Picker::Parameter(p)) => pending.extend(p.commands()),
                None => {}
            }
            fi.picker = None;
        }
        Some(Outcome::Cancel) => fi.picker = None,
        None => {}
    }
}
