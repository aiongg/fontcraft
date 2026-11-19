//! The app layer: the only code that changes the engine.
//!
//! Views post [`Msg`]s into [`FontCraftApp::sent`]. After the frame, [`FontCraftApp::drain`] applies them: engine
//! commands through `Session::execute` (errors go to the status line), shell commands through their module's handler,
//! view messages into [`ViewState`]. Then [`FontCraftApp::project`] refreshes the view models when the engine or the
//! view state moved.

pub mod catalog;
pub mod files;
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
use serde_json::json;

use self::catalog::{Catalog, Kind, TOOL_PREFIX};
use self::files::Picks;
use self::models::{Models, ProjectCx, Stamp};
pub use self::msg::{Msg, ViewMsg};
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
    picks: Picks,
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

    /// The app over `session` with `host`; no window needed (tests, other front ends).
    pub fn new(session: Session, host: Arc<Host>) -> FontCraftApp {
        let mut app = FontCraftApp {
            session,
            host,
            root: RootEnv::new(),
            catalog: Catalog::build(),
            tool_slots: tool_slots(),
            view: ViewState::default(),
            active_tool: Box::new(craft::ui::canvas::NoTool),
            view_revision: 0,
            enabled_at: None,
            models: Models::default(),
            sent: Vec::new(),
            status: String::new(),
            closing: Closing::Idle,
            picks: Picks::default(),
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
            match msg {
                Msg::Run(command) => {
                    if self.picks.ask(&self.host, &command) {
                        continue;
                    }
                    let params = if command.params.is_null() { json!({}) } else { command.params.clone() };
                    if let Err(error) = self.session.execute(&command.id, &params) {
                        failed.push(format!("{}: {error}", command.id));
                    }
                }
                Msg::Shell(command) => {
                    if let Err(error) = crate::shell::commands::run(self, &command) {
                        failed.push(format!("{}: {error}", command.id));
                    }
                }
                Msg::View(view) => self.apply_view(view),
            }
        }
        if let Some(last) = failed.pop() {
            self.status = last;
        }
        if self.closing == Closing::SaveThenClose {
            self.closing = if self.has_unsaved_changes() { Closing::Idle } else { Closing::Confirmed };
        }
        self.project();
    }

    /// Copies what each panel shows into its view model when the engine or the view state moved.
    pub fn project(&mut self) {
        let stamp = Stamp::of(&self.session, self.view_revision);
        let mut cx = ProjectCx { session: &mut self.session, view: &self.view };
        self.models.project(stamp, &mut cx);
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

    fn apply_view(&mut self, msg: ViewMsg) {
        self.view_revision += 1;
        match msg {
            ViewMsg::SelectTool(tool) => self.view.tool = tool,
            ViewMsg::ToolsLastUsed(last_used) => self.view.tools_last_used = last_used,
            ViewMsg::SelectGlyph(glyph) => self.view.glyph = Some(glyph),
            ViewMsg::TogglePanel(panel) => dock::toggle(&mut self.view.workspace, panel),
        }
    }

    /// The message a fired command posts.
    fn route(command: CommandRef) -> Msg {
        match catalog::kind(&command.id) {
            Kind::Tool(tool) => Msg::View(ViewMsg::SelectTool(tool)),
            Kind::Shell => Msg::Shell(command),
            Kind::Engine => Msg::Run(command),
        }
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
                self.sent.push(FontCraftApp::route(fired.command));
            }
        }
    }

    fn logic(&mut self, ctx: &egui::Context) {
        self.drain();
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
