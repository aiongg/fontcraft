//! The app layer: the only code that changes the engine.
//!
//! Views post [`Msg`]s into [`FontCraftApp::sent`]. After the frame, [`FontCraftApp::drain`] applies them: engine
//! commands through `Session::execute` (errors go to the status line), shell commands through their module's handler,
//! view messages into [`ViewState`]. Then [`FontCraftApp::project`] refreshes the view models when the engine or the
//! view state moved.

pub mod catalog;
pub mod console;
pub mod files;
pub mod font_view;
pub mod menu_alternates;
pub mod models;
mod msg;

use std::sync::Arc;

use craft::behavior::CanvasView;
use craft::behavior::dock::Workspace;
use craft::behavior::kurbo::Point;
use craft::host::{AppInfo, Host};
use craft::ui::rails::{LastUsed, RailTool, ToolSlot};
use craft::ui::runtime::commands::{CommandRef, FirePhase};
use craft::ui::runtime::env::RootEnv;
use craft::ui::runtime::frame::{FrameInput, FrameOutput};
use craft::ui::shell::app::ShellApp;
use craft::ui::shell::embed::CloseDecision;
use fontcraft_engine::Session;
use serde_json::{Value, json};

use self::catalog::{Catalog, Kind, TOOL_PREFIX};
use self::files::Picks;
use self::models::{Models, ProjectCx, Stamp};
pub use self::msg::{Msg, Preview, ViewMsg};
use crate::shell::dock::{self, PanelId};

/// The app's identity for the host: the settings folder name, the display name and the version.
pub const APP: AppInfo = AppInfo { id: "fontcraft", name: "FontCraft", version: env!("CARGO_PKG_VERSION") };

/// View and window state: what no engine command changes.
pub struct ViewState {
    /// The current tool's id (`fontcraft_tools::TOOLS`).
    pub tool: &'static str,
    /// The tool each rail slot showed last.
    pub tools_last_used: LastUsed<&'static str>,
    /// The glyph the glyph panels show; `None` shows the font's first glyph.
    pub glyph: Option<String>,
    /// The dock: which panels show where.
    pub workspace: Workspace<PanelId>,
    /// The document canvas's view.
    pub canvas: CanvasView,
    /// The canvas's display toggles (View menu).
    pub display: crate::canvas::Display,
    /// The canvas edits the background of the layer (`path.editBackground`).
    pub edit_background: bool,
    /// Font View's mode, sort, columns, search and glyph selection.
    pub font_view: font_view::FontViewState,
    /// The Font Info panel's tab and the dialog it asked for.
    pub font_info: crate::shell::font_info::FontInfoView,
    /// The fonts drawn behind the outline (`view.setReferenceFont`).
    pub reference_font: crate::shell::reference_font::ReferenceFontState,
    /// The Pairs & Phrases work lists and the auto-kerning proposal under review.
    pub kerning_review: crate::shell::kerning_review::KerningReviewState,
    /// Glyph Info's search, sort and selection.
    pub glyph_info: crate::shell::glyph_info::GlyphInfoState,
    /// The current master, the active layer and the Layers panel's state.
    pub layers: crate::shell::masters::LayersView,
    /// The Command Console, the command palette and About.
    pub commands: console::CommandSurfaces,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            tool: "select",
            tools_last_used: LastUsed::default(),
            glyph: None,
            workspace: dock::default_workspace(),
            canvas: CanvasView::new(egui::Rect::NOTHING, Point::ZERO, 1.0),
            display: crate::canvas::Display::default(),
            edit_background: false,
            font_view: font_view::FontViewState::default(),
            font_info: Default::default(),
            reference_font: Default::default(),
            kerning_review: Default::default(),
            glyph_info: Default::default(),
            layers: Default::default(),
            commands: console::CommandSurfaces::default(),
        }
    }
}

/// Where closing the window stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Closing {
    #[default]
    Idle,
    /// The window asked to close with unsaved changes: the unsaved-changes dialog opens next.
    Ask,
    /// The dialog is open.
    Asking,
    /// The answer was Save: the window closes once the save leaves nothing unsaved.
    SaveThenClose,
    /// The window may close: the next close request is let through.
    Confirmed,
    /// The close request has been sent.
    Sent,
}

/// The app: the engine session, the host services, the command catalog, view state and view models.
pub struct FontCraftApp {
    pub session: Session,
    /// The host services (language, settings, jobs, files); the same `Arc` goes into every frame.
    pub host: Arc<Host>,
    root: RootEnv,
    pub catalog: Catalog,
    /// The tool rail's slots, one per tool group of `fontcraft_tools::TOOLS`.
    pub tool_slots: Vec<ToolSlot<&'static str>>,
    pub view: ViewState,
    /// The canvas's active-tool slot.
    pub active_tool: Box<dyn crate::canvas::ActiveTool>,
    /// The toolbox, the selection and the running interaction.
    pub tools: crate::canvas::tools::Tools,
    /// Moves with every applied [`ViewMsg`] and every view change a projection reads; part of the projection
    /// [`Stamp`].
    view_revision: u64,
    /// The engine part of the stamp the enabled states were last asked at.
    enabled_at: Option<Stamp>,
    pub models: Models,
    /// What views posted this frame.
    pub sent: Vec<Msg>,
    /// The status line: the last error or notice.
    pub status: String,
    pub closing: Closing,
    /// The hinting dialogs' state (`dialogs::tt`).
    pub tt: crate::dialogs::tt::TtState,
    picks: Picks,
    /// The control channel: its listener, the window its input goes to, the replies waiting on frames.
    pub control: crate::control::Control,
    #[cfg(not(target_arch = "wasm32"))]
    file_panels: Option<craft::dialogs::service::FilePanels>,
}

impl FontCraftApp {
    /// The app for the desktop window of `ctx`: a new font, the system file pickers.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(ctx: &egui::Context) -> FontCraftApp {
        let ctx = ctx.clone();
        let (files, panels) = craft::dialogs::service::FileService::new(craft::host::Wake::new(move || ctx.request_repaint()));
        let host = Host::builder(APP).jobs(craft::host::default_jobs()).files(Arc::new(files)).build();
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(host));
        app.file_panels = Some(panels);
        app
    }

    /// The app over `session` with `host`; no window needed (tests, other front ends). What the user runs here may
    /// export where an Export Folder parameter climbs out of the destination (`Session::set_export_outside`); the
    /// control channel turns that off for its requests.
    pub fn new(mut session: Session, host: Arc<Host>) -> FontCraftApp {
        session.set_export_outside(true);
        let mut app = FontCraftApp {
            session,
            host,
            root: RootEnv::new(),
            catalog: Catalog::build(),
            tool_slots: tool_slots(),
            view: ViewState::default(),
            active_tool: Box::new(crate::canvas::tools::bridge::ToolBridge::default()),
            tools: crate::canvas::tools::Tools::default(),
            view_revision: 0,
            enabled_at: None,
            models: Models::default(),
            sent: Vec::new(),
            status: String::new(),
            closing: Closing::Idle,
            tt: Default::default(),
            picks: Picks::default(),
            control: crate::control::Control::default(),
            #[cfg(not(target_arch = "wasm32"))]
            file_panels: None,
        };
        if let Some(error) = app.catalog.error.clone() {
            app.status = error;
        }
        app.project();
        app
    }

    /// Whether a document has changes not yet saved.
    pub fn has_unsaved_changes(&self) -> bool {
        self.session.documents().iter().any(|d| d.modified())
    }

    /// Applies what was posted: the only place that runs engine commands. A failed command changes nothing and
    /// leaves its message in the status line.
    pub fn drain(&mut self) {
        let mut failed = Vec::new();
        for command in self.picks.poll(&self.host, &mut failed) {
            self.sent.push(Msg::Run(command));
        }
        for msg in std::mem::take(&mut self.sent) {
            if let Err(error) = self.apply_msg(msg) {
                failed.push(error);
            }
        }
        if let Some(last) = failed.pop() {
            self.status = last;
        }
        crate::canvas::tools::apply::sync(self);
        self.tools.refresh();
        if self.closing == Closing::SaveThenClose {
            self.closing = if self.has_unsaved_changes() { Closing::Idle } else { Closing::Confirmed };
        }
        self.project();
    }

    /// Runs `msg` now, as `drain` would after the frame, and returns the command's result: what views posted
    /// before it is applied first. The control channel runs its requests here. With `ask_files`, a command of
    /// [`files::PICKED`] without its path opens the file picker and returns `{"picker": true}`; without, it runs
    /// as given and the engine reports the missing path. A failure also goes to the status line.
    pub fn run_now(&mut self, msg: Msg, ask_files: bool) -> Result<Value, String> {
        self.drain();
        let id = msg.command_id().map(str::to_owned);
        let result = self.apply(msg, ask_files);
        if let Err(error) = &result {
            self.status = with_command_id(id, error.clone());
        }
        self.drain();
        result
    }

    /// Applies one message as `drain` does; the error names the command that failed.
    pub fn apply_msg(&mut self, msg: Msg) -> Result<(), String> {
        let id = msg.command_id().map(str::to_owned);
        self.apply(msg, true).map(|_| ()).map_err(|error| with_command_id(id, error))
    }

    /// Applies one message: the only code that runs `Session::execute`. Tool input runs the toolbox, whose answer
    /// is applied here too, before the next message.
    fn apply(&mut self, msg: Msg, ask_files: bool) -> Result<Value, String> {
        match msg {
            Msg::Run(command) => {
                if ask_files && self.picks.ask(&self.host, &command) {
                    return Ok(json!({ "picker": true }));
                }
                let params = if command.params.is_null() { json!({}) } else { command.params.clone() };
                let reply = self.session.execute(&command.id, &params).map_err(|e| e.to_string())?;
                crate::canvas::tools::apply::adopt(self, &reply);
                if let Some(notice) = self.after_command(&command, &reply) {
                    self.status = notice;
                }
                self.prune_font_view_selection();
                Ok(reply)
            }
            Msg::Shell(command) => {
                // Shell commands change view state, which the projection reads.
                self.view_revision += 1;
                crate::shell::commands::run(self, &command).map(|()| Value::Null)
            }
            Msg::Preview(step) => crate::canvas::tools::apply::preview(self, step).map(|()| Value::Null),
            Msg::Tool(input) => crate::canvas::tools::apply::input(self, input).pop().map_or(Ok(Value::Null), Err),
            Msg::View(view) => self.apply_view(view).map(|()| Value::Null),
        }
    }

    /// Copies what each panel shows into its view model when the engine or the view state moved.
    pub fn project(&mut self) {
        let stamp = Stamp::of(&self.session, self.view_revision);
        let mut cx = ProjectCx { session: &mut self.session, view: &self.view };
        self.models.project(stamp, &mut cx);
        models::console::project(&mut self.models.console, &mut cx);
        let engine = Stamp::of(&self.session, 0);
        if self.enabled_at != Some(engine) {
            self.enabled_at = Some(engine);
            self.catalog.refresh(&self.session);
        }
    }

    /// Records a change to view state that a projection reads, made outside [`ViewMsg`] (a shell command).
    pub fn view_changed(&mut self) {
        self.view_revision += 1;
    }

    fn apply_view(&mut self, msg: ViewMsg) -> Result<(), String> {
        self.view_revision += 1;
        match msg {
            ViewMsg::SelectTool(tool) => {
                self.view.tool = tool;
                return crate::canvas::tools::apply::select_tool(self, tool);
            }
            ViewMsg::ToolsLastUsed(last_used) => self.view.tools_last_used = last_used,
            ViewMsg::SelectGlyph(glyph) => self.view.glyph = Some(glyph),
            ViewMsg::TogglePanel(panel) => dock::toggle(&mut self.view.workspace, panel),
            ViewMsg::LayerMenu(row) => self.view.layers.menu_row = row,
        }
        Ok(())
    }

    /// The message a fired command posts. `tool.select` with a `tool` parameter makes that tool current.
    pub fn route(command: CommandRef) -> Msg {
        match catalog::kind(&command.id) {
            Kind::Tool(tool) => {
                let named = command.params.get("tool").and_then(serde_json::Value::as_str);
                let tool = named.and_then(|id| fontcraft_tools::TOOLS.iter().find(|t| t.id == id)).map_or(tool, |t| t.id);
                Msg::View(ViewMsg::SelectTool(tool))
            }
            Kind::Shell => Msg::Shell(command),
            Kind::Engine => Msg::Run(command),
        }
    }

    /// `command` with its targets filled: from Font View's selection while Font View shows with glyphs selected,
    /// else from the canvas selection.
    pub(crate) fn with_targets(&mut self, command: CommandRef) -> CommandRef {
        if !self.view.font_view.selected.is_empty() && self.font_view_shows() {
            self.with_font_view_targets(command)
        } else {
            crate::canvas::tools::targets::with_targets(self, command)
        }
    }
}

/// `error` with the id of the command that failed in front, when the message ran one.
fn with_command_id(id: Option<String>, error: String) -> String {
    match id {
        Some(id) => format!("{id}: {error}"),
        None => error,
    }
}

/// One rail slot per tool group, in toolbar order; a tool without a group has a slot of its own.
fn tool_slots() -> Vec<ToolSlot<&'static str>> {
    let mut slots: Vec<(&'static str, Vec<RailTool<&'static str>>)> = Vec::new();
    for tool in fontcraft_tools::TOOLS {
        let rail_tool = RailTool::new(tool.id, CommandRef::new(format!("{TOOL_PREFIX}{}", tool.id)));
        match slots.iter_mut().find(|(group, _)| !tool.group.is_empty() && *group == tool.group) {
            Some((_, tools)) => tools.push(rail_tool),
            None => slots.push((tool.group, vec![rail_tool])),
        }
    }
    slots.into_iter().zip(0u32..).map(|((_, tools), id)| ToolSlot::new(id, tools)).collect()
}

impl ShellApp for FontCraftApp {
    fn frame_input(&mut self) -> FrameInput {
        FrameInput::new(self.root.clone()).host(Arc::clone(&self.host)).commands(self.catalog.input())
    }

    fn output(&mut self, output: FrameOutput) {
        for fired in output.fired {
            // A held tool key fires again on release; the tool stays.
            if fired.phase != FirePhase::Release {
                let msg = match FontCraftApp::route(fired.command) {
                    Msg::Run(command) => Msg::Run(self.with_targets(command)),
                    other => other,
                };
                self.sent.push(msg);
            }
        }
    }

    fn logic(&mut self, ctx: &egui::Context) {
        crate::control::logic(self, ctx);
        crate::control::drain(self);
        if self.closing == Closing::Confirmed {
            self.closing = Closing::Sent;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        crate::shell::show(self, ui);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(panels) = self.file_panels.as_mut() {
            panels.pump::<eframe::Frame>(None);
        }
        crate::control::after_ui(self, ui.ctx());
    }

    fn close_requested(&mut self) -> CloseDecision {
        match self.closing {
            Closing::Confirmed | Closing::Sent => CloseDecision::Close,
            _ if !self.has_unsaved_changes() => CloseDecision::Close,
            Closing::Idle => {
                self.closing = Closing::Ask;
                CloseDecision::Hold
            }
            _ => CloseDecision::Hold,
        }
    }
}

#[cfg(test)]
mod tests;
