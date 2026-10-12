//! The application: one [`FontcraftApp`] owns the engine session and the UI state, and draws
//! itself into any egui context (the desktop window, the browser canvas, a headless test frame).
//!
//! The app is thin. Panels read the session and act only through [`FontcraftApp::run`], so
//! anything a click does, the control channel and MCP can do with the same command id. The
//! chrome lives in [`crate::shell`], Edit View's drawing in [`crate::canvas`].

use fontcraft_engine::{EngineError, Session, guard};
use serde_json::{Value, json};

use crate::canvas::{self, fill::FillCache};
use crate::shell::{self, UI_COMMANDS, is_ui_command, menus::MenuCache};
use crate::state::UiState;
use crate::theme::Tokens;
use crate::tools_glue::{self, ToolsState};

/// Commands that report nothing in the status bar when they succeed: wheel and gesture input
/// runs them many times a second.
const QUIET: &[&str] = &["view.scrollTo", "view.setZoom", "fontView.setSearch", "preview.setOffset"];

pub struct FontcraftApp {
    pub session: Session,
    pub state: UiState,
    pub tokens: Tokens,
    /// The active pointer tool shows the edited glyph filled under Fill Preview (the Text, Hand
    /// and Zoom tools). The tools glue sets it.
    pub preview_tool: bool,
    /// The pointer tools and their input.
    pub tools: ToolsState,
    /// The open dialog; while it is open the rest of the app takes no editing input.
    pub dialog: Option<crate::dialogs::OpenDialog>,
    /// The platform's file choosers; `None` where there are none (the browser, MCP, tests).
    pub picker: Option<Box<dyn shell::pickers::Picker>>,
    /// Text for the system clipboard, handed to the platform at the end of the frame.
    pub(crate) clipboard_out: Option<String>,
    /// Where the menu bar was drawn in the last frame.
    pub(crate) menu_bar_rect: egui::Rect,
    pub(crate) menu_cache: MenuCache,
    pub(crate) fill_cache: FillCache,
    /// The tabs' text layouts.
    pub(crate) layouts: canvas::view::LayoutCache,
    /// Font View's caches and the edit or drag in progress.
    pub(crate) font_view: shell::font_view::FontViewUi,
    themed: bool,
}

impl Default for FontcraftApp {
    fn default() -> Self {
        Self::new(Session::with_document())
    }
}

impl FontcraftApp {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            state: UiState::default(),
            tokens: Tokens::dark(),
            preview_tool: false,
            tools: ToolsState::default(),
            dialog: None,
            picker: None,
            clipboard_out: None,
            menu_bar_rect: egui::Rect::NOTHING,
            menu_cache: MenuCache::default(),
            fill_cache: FillCache::default(),
            layouts: Default::default(),
            font_view: Default::default(),
            themed: false,
        }
    }

    /// Runs a command by id: a shell command from [`UI_COMMANDS`] or an engine command. The result
    /// or the error also goes to the status bar. Never panics: a bug in a command is reported as
    /// an internal error and the document is left as it was.
    pub fn run(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        if matches!(id, "file.save" | "file.saveAs") {
            self.follow_document();
            shell::text::store_display_strings(self);
        }
        let before = self.session.active().map(|d| (d.id, d.font.clone()));
        let result = if is_ui_command(id) {
            match guard::catch_panic(|| shell::run(self, id, params)) {
                Ok(r) => r,
                Err(panic) => Err(EngineError::Internal(format!("`{id}` failed unexpectedly: {panic}")).to_string()),
            }
        } else {
            self.session.execute(id, params).map_err(|e| e.to_string())
        };
        if let Ok(r) = &result {
            for (old, new) in renames(id, params, r) {
                self.follow_rename(&old, &new);
            }
            // In Font View the copies Duplicate Glyph makes become the selection.
            if id == "glyph.duplicateGlyph"
                && self.state.tab().is_none()
                && let Some(copies) = r.get("glyphs").and_then(Value::as_array)
            {
                let copies: Vec<String> = copies.iter().filter_map(Value::as_str).map(str::to_string).collect();
                if !copies.is_empty() {
                    self.state.font_view.anchor = copies.last().cloned();
                    self.state.selected_glyphs = copies;
                }
            }
        }
        self.after_command(before);
        let opened = id == "file.open" && result.as_ref().is_ok_and(|r| r.get("alreadyOpen").is_none());
        let warnings = if opened { self.open_display_strings() } else { Vec::new() };
        shell::start_window::after_run(self, id, &result);
        shell::font_view::remember_node_selection(self);
        if result.is_ok() && matches!(id, "audit.run" | "audit.fix") {
            self.state.audit.reread = true;
        }
        match &result {
            Ok(_) if QUIET.contains(&id) => {}
            Ok(_) => self.state.status = format!("{id}: done"),
            Err(e) => self.state.status = e.clone(),
        }
        if let Some(last) = warnings.last() {
            self.state.status = last.clone();
        }
        result
    }

    /// Opens one Edit View tab per display string of the active document, after the open tabs,
    /// up to the tab limit; Font View stays showing. A string over the tab item limit opens cut
    /// to the limit. The warnings (also logged) say which strings were cut or left out.
    pub fn open_display_strings(&mut self) -> Vec<String> {
        let room = crate::state::MAX_TABS.saturating_sub(self.state.tabs.len());
        let (tabs, warnings) = match self.session.font() {
            Ok(font) => shell::text::tabs_from_display_strings(font, &font.display_strings, room),
            Err(_) => return Vec::new(),
        };
        let active = self.state.active_tab;
        for text in tabs {
            let id = self.state.next_tab_id;
            self.state.next_tab_id += 1;
            let master = self.state.font_view.master.clone();
            shell::tabs::push(self, crate::state::Tab { id, text, master, ..Default::default() });
        }
        self.state.active_tab = active;
        for w in &warnings {
            log::warn!("{w}");
        }
        warnings
    }

    /// Runs a command the person started (menu item, shortcut, tool) as [`Self::run`] does, then
    /// opens the follow-up dialog its result asks for (`dialogs::after`). Copy and Cut also put
    /// their text on the system clipboard.
    pub fn run_from_shell(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        let result = match self.run(id, params) {
            Ok(result) => result,
            Err(e) => {
                // A save refused because the file changed on disk asks whether to overwrite it,
                // so the person can always save their work.
                if self.dialog.is_none()
                    && let Some(given) = crate::dialogs::save_changed_on_disk::after_error(id, params, &e)
                    && let Err(open) = crate::dialogs::open(self, &crate::dialogs::save_changed_on_disk::DIALOG, &given)
                {
                    self.state.status = open;
                }
                return Err(e);
            }
        };
        shell::clipboard::after_run(self, id, &result);
        if self.dialog.is_none()
            && let Some((dialog, given)) = crate::dialogs::after(id, params, &result)
            && let Err(e) = crate::dialogs::open(self, &dialog, &given)
        {
            self.state.status = e;
        }
        Ok(result)
    }

    /// What a menu item or shortcut does with its command: one that has a dialog and no
    /// parameters opens the dialog (the result is the dialog's state); any other runs on the
    /// selection's targets through [`Self::run_from_shell`].
    pub(crate) fn run_menu_command(&mut self, id: &str, params: Value) -> Result<Value, String> {
        if let Some(result) = shell::clipboard::kerning_window_copy(self, id) {
            return result;
        }
        let empty = params.is_null() || params.as_object().is_some_and(serde_json::Map::is_empty);
        if empty && let Some(result) = shell::text::menu_command(self, id) {
            return result;
        }
        if empty && let Some(dialog) = crate::dialogs::for_command(id) {
            return crate::dialogs::open(self, &dialog, &serde_json::Map::new());
        }
        if let Some(batch) = shell::font_view::per_glyph_batch(self, id, &params) {
            return self.run_from_shell("command.batch", &batch);
        }
        let params = tools_glue::with_targets(self, id, shell::clipboard::font_view_params(self, id, params));
        self.run_from_shell(id, &params)
    }

    /// Points the UI's references to a renamed glyph at its new name: the Font View selection,
    /// the tabs and the remembered node selections.
    fn follow_rename(&mut self, old: &str, new: &str) {
        if old == new {
            return;
        }
        let rename = |name: &mut String| {
            if name == old {
                *name = new.to_string();
            }
        };
        let state = &mut self.state;
        state.selected_glyphs.iter_mut().for_each(rename);
        state.font_view.anchor.iter_mut().for_each(rename);
        for tab in state.tabs.iter_mut().chain(state.closed_tabs.iter_mut().map(|(_, t)| t)) {
            tab.text.rename_glyph(old, new);
            tab.selection_glyph.iter_mut().for_each(rename);
            if let Some(layer) = tab.layers.remove(old) {
                tab.layers.insert(new.to_string(), layer);
            }
        }
        if let Some(kept) = state.font_view.node_selections.remove(old) {
            state.font_view.node_selections.insert(new.to_string(), kept);
        }
    }

    /// Shows the active document's tabs: the tabs of the document shown until now are kept for
    /// it, or closed with it when it closed. Returns whether another document's tabs show now.
    pub(crate) fn follow_document(&mut self) -> bool {
        let session = &self.session;
        self.state.follow_document(session.active().map(|d| d.id), |id| session.documents().iter().any(|d| d.id == id))
    }

    /// Drops UI references to glyphs a command removed (or to a closed document); `before` is
    /// the active document's id and font before the command. Glyphs a tab names that the font
    /// did not have before (a display string may name one) stay. A tab the command emptied
    /// closes, and a closed tab it emptied is forgotten.
    fn after_command(&mut self, before: Option<(u64, std::sync::Arc<fontcraft_model::Font>)>) {
        let switched = self.follow_document();
        let state = &mut self.state;
        let Some(doc) = self.session.active() else {
            state.selected_glyphs.clear();
            return;
        };
        let font = &doc.font;
        let old = match &before {
            Some((id, old)) if *id == doc.id && !switched => old,
            // Another document: its tabs were put away unchanged while it was not active.
            _ => {
                state.selected_glyphs.retain(|name| font.glyph(name).is_some());
                return;
            }
        };
        if std::sync::Arc::ptr_eq(old, font) || (state.selected_glyphs.is_empty() && state.tabs.is_empty() && state.closed_tabs.is_empty()) {
            return;
        }
        state.selected_glyphs.retain(|name| font.glyph(name).is_some());
        let gone = |name: &str| font.glyph(name).is_none() && old.glyph(name).is_some();
        let active = state.tab().map(|t| t.id);
        let mut emptied = Vec::new();
        for tab in &mut state.tabs {
            if tab.text.remove_glyphs(gone) && tab.text.is_empty() {
                emptied.push(tab.id);
            }
        }
        state.tabs.retain(|t| !emptied.contains(&t.id));
        state.closed_tabs.retain_mut(|(_, tab)| !(tab.text.remove_glyphs(gone) && tab.text.is_empty()));
        state.active_tab = active.and_then(|id| state.tab_index(id));
    }

    /// The master `ui.render` draws in by default: the showing tab's master, else Font View's.
    pub fn render_master(&self) -> Option<String> {
        let font = self.session.font().ok()?;
        match self.state.tab() {
            Some(tab) => crate::canvas::view::tab_master(font, tab).map(|m| m.id.clone()),
            None => shell::font_view::master(self),
        }
    }

    /// The glyphs `ui.render` draws when none are named: the showing tab's line, else the Font
    /// View selection.
    pub fn default_render_glyphs(&self) -> Vec<String> {
        match self.state.tab() {
            Some(tab) => tab.shown_glyphs().map(str::to_string).collect(),
            None => self.state.selected_glyphs.clone(),
        }
    }

    /// Every command the app can run, engine and shell, as the control channel lists them.
    pub fn commands(&self) -> Value {
        let mut list: Vec<Value> =
            fontcraft_engine::command_specs().iter().filter_map(|c| serde_json::to_value(c.info(&self.session)).ok()).collect();
        list.extend(UI_COMMANDS.iter().map(|(id, label, menu, shortcut, params)| {
            json!({"id": id, "label": label, "menu": menu, "shortcut": if shortcut.is_empty() { Value::Null } else { json!(shortcut) }, "params": params, "ui": true, "enabled": true})
        }));
        json!({ "commands": list })
    }

    /// Draws one frame into `ui` (the root of a window or of a headless test context).
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.themed {
            self.tokens.apply(ui.ctx());
            self.themed = true;
        }
        self.follow_document();
        shell::extensions::external_changes::on_focus(self, ui.ctx());
        tools_glue::sync(self);
        crate::dialogs::frame_keys(self, ui.ctx());
        // The tools take their keys first (the Edit View keys have a `when`); the rest go to
        // the menu shortcuts.
        tools_glue::keys(self, ui.ctx());
        let mut pending: Vec<(&'static str, Value)> = Vec::new();
        shell::text_preview::find_shortcut(self, ui.ctx(), &mut pending);
        pending.extend(shell::keymap::pressed(ui.ctx(), self.tools.text_edit_at_end, shell::keymap::focused_panel(self)));
        pending.extend(shell::clipboard::events(self, ui.ctx()));
        self.menu_bar(ui, &mut pending);
        shell::tabs::strip(self, ui, &mut pending);
        shell::status_bar::show(self, ui);
        crate::panels::show(self, ui, &mut pending);
        if self.state.tab().is_some() {
            shell::toolbar::show(self, ui, &mut pending);
            canvas::show(self, ui, &mut pending);
        } else {
            shell::font_view::show(self, ui, &mut pending);
        }
        shell::windows(self, ui, &mut pending);
        crate::dialogs::show(self, ui);
        // Commands run after drawing, so a frame never sees a half-changed document. While a
        // dialog is open only view commands run. A menu item whose command has a dialog opens it.
        for (id, params) in pending {
            if self.dialog.is_some() && !is_ui_command(id) {
                continue;
            }
            // The Kerning window takes the clipboard's text when it arrives.
            if id == "edit.paste" && shell::kerning_window::takes_clipboard(self) {
                shell::clipboard::request_text(ui.ctx(), id);
                continue;
            }
            // Paste Special in text mode is refused before it asks for the clipboard.
            if shell::clipboard::wants_text(id, &params) && !(id == "edit.pasteSpecial" && shell::text::in_text_mode(self)) {
                shell::clipboard::request_text(ui.ctx(), id);
                continue;
            }
            if shell::pickers::wants_files(id, &params) {
                shell::pickers::import_outlines(self);
                continue;
            }
            if shell::start_window::opens_chooser(self, id, &params) {
                continue;
            }
            if let Err(e) = self.run_menu_command(id, params) {
                self.state.status = e;
            }
        }
        if tools_glue::dispatch_queued(self) {
            ui.ctx().request_repaint();
        }
        // Pointer tools change the selection without a command.
        shell::font_view::remember_node_selection(self);
        if let Some(text) = self.clipboard_out.take() {
            ui.ctx().copy_text(text);
        }
        self.tools.text_focus_at_end = ui.ctx().egui_wants_keyboard_input();
        self.tools.text_edit_at_end = ui.ctx().text_edit_focused();
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, pending: &mut Vec<(&'static str, Value)>) {
        let tree = self.menu_cache.tree(&self.session, &self.state);
        let frame = egui::Frame::NONE.fill(self.tokens.app_bar).inner_margin(4.0);
        let bar = egui::Panel::top("menu_bar").frame(frame).show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                for menu in tree {
                    ui.menu_button(menu.title, |ui| {
                        for (submenu, items) in shell::menus::submenus(&menu.items) {
                            match submenu {
                                Some(title) => {
                                    ui.menu_button(title, |ui| menu_items(ui, &items, pending));
                                }
                                None => menu_items(ui, &items, pending),
                            }
                        }
                        shell::menus_dynamic::draw(ui, &menu.dynamic, pending);
                    });
                }
            });
        });
        self.menu_bar_rect = bar.response.rect;
    }
}

/// The glyph renames a successful run of `id` made, as (old, new) pairs.
fn renames(id: &str, params: &Value, result: &Value) -> Vec<(String, String)> {
    match id {
        "glyph.rename" => {
            let (Some(old), Some(new)) = (params.get("glyph").and_then(Value::as_str), result.get("glyph").and_then(Value::as_str)) else {
                return Vec::new();
            };
            vec![(old.to_string(), new.to_string())]
        }
        "edit.findAndReplace" | "glyph.updateGlyphInfo" => result
            .get("renamed")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|r| Some((r.get("from")?.as_str()?.to_string(), r.get("to")?.as_str()?.to_string())))
            .collect(),
        _ => Vec::new(),
    }
}

fn menu_items(ui: &mut egui::Ui, items: &[&shell::menus::MenuItem], pending: &mut Vec<(&'static str, Value)>) {
    for item in items {
        let mut button = egui::Button::new(item.label);
        if !item.shortcut.is_empty() {
            button = button.shortcut_text(item.shortcut);
        }
        let response = ui.add_enabled(item.enabled, button);
        let response = match &item.disabled_reason {
            Some(why) => response.on_disabled_hover_text(why),
            None => response,
        };
        if response.clicked() {
            pending.push((item.command, json!({})));
            ui.close();
        }
    }
}
