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
const QUIET: &[&str] = &["view.scrollTo", "view.setZoom"];

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
    pub(crate) menu_cache: MenuCache,
    pub(crate) fill_cache: FillCache,
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
            menu_cache: MenuCache::default(),
            fill_cache: FillCache::default(),
            font_view: Default::default(),
            themed: false,
        }
    }

    /// Runs a command by id: a shell command from [`UI_COMMANDS`] or an engine command. The result
    /// or the error also goes to the status bar. Never panics: a bug in a command is reported as
    /// an internal error and the document is left as it was.
    pub fn run(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        let result = if is_ui_command(id) {
            match guard::catch_panic(|| shell::run(self, id, params)) {
                Ok(r) => r,
                Err(panic) => Err(EngineError::Internal(format!("`{id}` failed unexpectedly: {panic}")).to_string()),
            }
        } else {
            self.session.execute(id, params).map_err(|e| e.to_string())
        };
        self.after_command();
        shell::font_view::remember_node_selection(self);
        if result.is_ok() && matches!(id, "audit.run" | "audit.fix") {
            self.state.audit.reread = true;
        }
        match &result {
            Ok(_) if QUIET.contains(&id) => {}
            Ok(_) => self.state.status = format!("{id}: done"),
            Err(e) => self.state.status = e.clone(),
        }
        result
    }

    /// Runs a command the person started (menu item, shortcut, tool) as [`Self::run`] does, then
    /// opens the follow-up dialog its result asks for (`dialogs::after`). Copy and Cut also put
    /// their text on the system clipboard.
    pub fn run_from_shell(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        let result = self.run(id, params)?;
        shell::clipboard::after_run(self, id, &result);
        if self.dialog.is_none()
            && let Some((dialog, given)) = crate::dialogs::after(id, params, &result)
            && let Err(e) = crate::dialogs::open(self, &dialog, &given)
        {
            self.state.status = e;
        }
        Ok(result)
    }

    /// Drops UI references to glyphs a command removed (or to a closed document). A tab whose
    /// glyphs are all gone closes.
    fn after_command(&mut self) {
        let state = &mut self.state;
        if state.selected_glyphs.is_empty() && state.tabs.is_empty() {
            return;
        }
        let Ok(font) = self.session.font() else {
            state.selected_glyphs.clear();
            state.tabs.clear();
            state.active_tab = None;
            return;
        };
        let known = |name: &String| font.glyph(name).is_some();
        state.selected_glyphs.retain(known);
        let active = state.tab().map(|t| t.id);
        for tab in &mut state.tabs {
            if tab.glyphs.iter().all(known) {
                continue;
            }
            let edited = tab.edited_glyph().map(str::to_string);
            tab.glyphs.retain(known);
            tab.edited = edited.and_then(|e| tab.glyphs.iter().position(|g| *g == e)).unwrap_or(0);
        }
        state.tabs.retain(|t| !t.glyphs.is_empty());
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
            Some(tab) => tab.glyphs.clone(),
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
        tools_glue::sync(self);
        crate::dialogs::frame_keys(self, ui.ctx());
        // The tools take their keys first (the Edit View keys have a `when`); the rest go to
        // the menu shortcuts.
        tools_glue::keys(self, ui.ctx());
        let mut pending: Vec<(&'static str, Value)> = shell::keymap::pressed(ui.ctx());
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
        crate::dialogs::show(self, ui);
        // Commands run after drawing, so a frame never sees a half-changed document. While a
        // dialog is open only view commands run. A menu item whose command has a dialog opens it.
        for (id, params) in pending {
            if self.dialog.is_some() && !is_ui_command(id) {
                continue;
            }
            if shell::clipboard::wants_text(id, &params) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                continue;
            }
            if shell::pickers::wants_files(id, &params) {
                shell::pickers::import_outlines(self);
                continue;
            }
            let empty = params.is_null() || params.as_object().is_some_and(serde_json::Map::is_empty);
            if empty && let Some(dialog) = crate::dialogs::for_command(id) {
                if let Err(e) = crate::dialogs::open(self, &dialog, &serde_json::Map::new()) {
                    self.state.status = e;
                }
                continue;
            }
            let params = tools_glue::with_targets(self, id, params);
            let _ = self.run_from_shell(id, &params);
        }
        if tools_glue::dispatch_queued(self) {
            ui.ctx().request_repaint();
        }
        // Pointer tools change the selection without a command.
        shell::font_view::remember_node_selection(self);
        if let Some(text) = self.clipboard_out.take() {
            ui.ctx().copy_text(text);
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, pending: &mut Vec<(&'static str, Value)>) {
        let tree = self.menu_cache.tree(&self.session, &self.state);
        let frame = egui::Frame::NONE.fill(self.tokens.app_bar).inner_margin(4.0);
        egui::Panel::top("menu_bar").frame(frame).show(ui, |ui| {
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
                    });
                }
            });
        });
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
