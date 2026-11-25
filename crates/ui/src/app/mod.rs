//! The app layer: the only code that changes the engine.
//!
//! Views post [`Msg`]s into [`FontCraftApp::sent`]. After the frame, [`FontCraftApp::drain`] applies them: engine
//! commands through `Session::execute` (errors go to the status line), shell commands through their module's handler,
//! view messages into [`ViewState`]. Then [`FontCraftApp::project`] refreshes the view models when the engine or the
//! view state moved.

pub mod appearance;
pub mod catalog;
pub mod clipboard;
pub mod close;
pub mod console;
pub mod context_menu;
pub mod dropped;
pub mod files;
pub mod font_view;
pub mod menu_alternates;
pub mod menu_states;
pub mod menus_dynamic;
pub mod models;
mod msg;
mod notices;
pub mod open;
pub mod preview_host;
#[cfg(not(target_arch = "wasm32"))]
pub mod print_host;
pub mod sidebar;
pub mod startup;
pub mod user_store;
pub mod web_store;

use std::sync::Arc;

use craft::behavior::CanvasView;
use craft::behavior::dock::Workspace;
use craft::behavior::kurbo::Point;
use craft::host::{AppInfo, Host};
use craft::ui::rails::{LastUsed, RailTool, ToolSlot};
use craft::ui::runtime::commands::{CommandRef, FirePhase, FireSource};
use craft::ui::runtime::env::RootEnv;
use craft::ui::runtime::frame::{FrameInput, FrameOutput};
use craft::ui::shell::app::ShellApp;
use craft::ui::shell::embed::CloseDecision;
use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use self::catalog::{Catalog, Kind, TOOL_PREFIX};
use self::files::Picks;
use self::models::{Models, ProjectCx, Stamp};
pub use self::msg::{Msg, Preview, ViewMsg};
use crate::shell::dock::{self, PanelId};

/// The app's identity for the host: the settings folder name, the display name and the version.
pub const APP: AppInfo = AppInfo { id: "fontcraft", name: "FontCraft", version: env!("CARGO_PKG_VERSION") };

/// Who started a run. The question a command's result asks ([`crate::dialogs::spec::After`]) opens only after the
/// person's own run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The person: a menu item, a shortcut, a dialog's OK, a click or key in the window, or one the control channel
    /// plays as a click (`ui.menu.invoke`, `ui.contextMenu.invoke`).
    Person,
    /// A request that names the command: `engine.execute` over the control channel, as an agent sends it.
    Control,
}

/// View and window state: what no engine command changes.
pub struct ViewState {
    /// The current tool's id (`fontcraft_tools::TOOLS`).
    pub tool: &'static str,
    /// The tool each rail slot showed last.
    pub tools_last_used: LastUsed<&'static str>,
    /// The glyph the glyph panels show; `None` shows the font's first glyph. This field and `canvas`, `text`'s text,
    /// `layers`' master and picked layers and `edit_background` belong to the live Edit View tab (`tabs`).
    pub glyph: Option<String>,
    /// The Edit View tabs beside Font View, the showing one and the closed ones (`shell::tabs`).
    pub tabs: crate::shell::tabs::EditTabs,
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
    /// The sidebar's open entries and the glyph inspector's rows.
    pub sidebar: sidebar::SidebarView,
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
    /// The Kerning window's search, sort, selection and the dialogs it asked for.
    pub kerning: crate::shell::kerning_window::KerningView,
    /// The Kerning Pairs Preview's options and the Kerning Groups Editor's side, group, view and proposal.
    pub kerning_tools: crate::shell::kerning_tools::KerningToolsView,
    /// The Info box's reference point and size lock, the Transformations settings and the Fit Curve range.
    pub palette: crate::shell::palette::PaletteView,
    /// The edit view's text, its cursor and layout options (`canvas::text`).
    pub text: crate::canvas::text::EditText,
    /// The Space Center's text, options and selected glyph.
    pub space_center: crate::shell::space_center::SpaceCenterState,
    /// The Start Window.
    pub start: crate::shell::start::StartState,
    /// The Settings window.
    pub settings: crate::shell::settings::SettingsState,
    /// The question asked when a save finds its file changed on disk.
    pub changed_on_disk: Option<crate::dialogs::changed_on_disk::Asked>,
    /// The document the unsaved-changes dialog shows.
    pub unsaved_shown: crate::dialogs::unsaved::Shown,
    /// The Table Viewer's font, table, view, search, tree and what it read.
    pub table_viewer: crate::shell::table_viewer::TableViewerState,
    /// Compare Fonts' sides, collapsed headings and selected row.
    pub compare_fonts: crate::shell::compare_fonts::CompareFontsState,
    /// Compare Family's attribute, filter and reads.
    pub compare_family: crate::shell::compare_family::CompareFamilyState,
    /// Compare Fonts in List's documents, property and Only Differences.
    pub compare_in_list: crate::shell::compare_in_list::CompareInListState,
    /// The Text Preview's text, style, size, layout and find bar.
    pub text_preview: crate::shell::text_preview::TextPreviewState,
    /// Show Hints aside: the TrueType Instructor's state, the badge under a drag, the Review Hints dialog.
    pub hinting: crate::canvas::hinting::HintingView,
    /// The Higher-Order Interpolation tool's axis.
    pub hoi: crate::canvas::hoi::HoiView,
    /// The Actions window's state.
    pub actions: crate::dialogs::actions::ActionsState,
    pub font_audit: crate::shell::font_audit::FontAuditView,
    /// The component glyph picker and the smart settings windows.
    pub components: crate::shell::components::ComponentsView,
    /// The shown reporters and the External Changes dialog.
    pub extensions: crate::shell::extensions::ExtensionsView,
    /// The project the Project panel shows and its selected instances.
    pub project: crate::shell::project::ProjectWindowView,
    /// The Preview Area's and the Preview Panel's settings (`shell::preview`).
    pub preview: crate::shell::preview::PreviewState,
    /// The Overlay Fonts dialog's settings and preview.
    pub overlay_fonts: crate::dialogs::overlay_fonts::OverlayState,
    /// The glyph the Shape Order panel was opened on (`shell::shape_order`).
    pub shape_order: crate::shell::shape_order::ShapeOrderView,
    /// The Bitmap Glyph Editor's glyph, strike, tool, shade and the stroke in progress (`shell::bitmap_editor`).
    pub bitmap_editor: crate::shell::bitmap_editor::BitmapEditorView,
    /// The open Axis Mappings dialog (`dialogs::axis_mappings`).
    pub axis_mappings: crate::dialogs::axis_mappings::AxisMappingsState,
    /// Whether the Cross-Axis Mappings window shows (`dialogs::cross_axis`).
    pub cross_axis: crate::dialogs::cross_axis::CrossAxisState,
    /// Whether the window resolves to the dark appearance: the system's mode, as the theme reads it
    /// ([`FontCraftApp::set_system_dark`]).
    pub system_dark: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            tool: "select",
            tools_last_used: LastUsed::default(),
            glyph: None,
            tabs: Default::default(),
            workspace: dock::default_workspace(),
            canvas: CanvasView::new(egui::Rect::NOTHING, Point::ZERO, 1.0),
            display: crate::canvas::Display::default(),
            edit_background: false,
            font_view: font_view::FontViewState::default(),
            sidebar: sidebar::SidebarView::default(),
            font_info: Default::default(),
            reference_font: Default::default(),
            kerning_review: Default::default(),
            glyph_info: Default::default(),
            layers: Default::default(),
            commands: console::CommandSurfaces::default(),
            kerning: Default::default(),
            kerning_tools: Default::default(),
            palette: Default::default(),
            text: Default::default(),
            space_center: Default::default(),
            start: Default::default(),
            settings: Default::default(),
            changed_on_disk: None,
            unsaved_shown: Default::default(),
            table_viewer: Default::default(),
            compare_fonts: Default::default(),
            compare_family: Default::default(),
            compare_in_list: Default::default(),
            text_preview: Default::default(),
            hinting: Default::default(),
            actions: Default::default(),
            font_audit: Default::default(),
            components: Default::default(),
            extensions: Default::default(),
            project: Default::default(),
            preview: Default::default(),
            overlay_fonts: Default::default(),
            shape_order: Default::default(),
            hoi: Default::default(),
            bitmap_editor: Default::default(),
            axis_mappings: Default::default(),
            cross_axis: Default::default(),
            system_dark: false,
        }
    }
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
    /// The Space Center preview's view and its drag tool.
    pub space_preview: crate::shell::space_center::drag::SpacePreview,
    /// The Bitmap Glyph Editor's pixel canvas: its view and its pointer tool.
    pub bitmap_canvas: crate::shell::bitmap_editor::BitmapCanvas,
    /// The toolbox, the selection and the running interaction.
    pub tools: crate::canvas::tools::Tools,
    /// The gradient knob drag in progress (`canvas::color::knobs`).
    pub knobs: crate::canvas::color::knobs::KnobDrag,
    /// The decoded image files and the image drag in progress (`canvas::images`).
    pub images: crate::canvas::images::Images,
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
    /// The question asked before unsaved work is closed.
    pub closing: close::ClosePrompt,
    /// The hinting dialogs' state (`dialogs::tt`).
    pub tt: crate::dialogs::tt::TtState,
    /// The open form dialog (`dialogs::form`).
    pub dialogs: crate::dialogs::form::DialogState,
    /// The open canvas and glyph context menus.
    pub context_menus: context_menu::ContextMenus,
    /// Text for the system clipboard and a paste waiting for its text.
    pub clipboard: clipboard::Clipboard,
    pub(crate) picks: Picks,
    /// Work the desktop app does each frame beside the UI (External Preview's server).
    pub frame_hook: Option<Box<dyn preview_host::FrameHook>>,
    /// The web server for the TrueType preview page; `None` where no port can be opened.
    pub preview_host: Option<Box<dyn preview_host::PreviewHost>>,
    /// The Print dialog (`dialogs::print`).
    pub printing: crate::dialogs::print::PrintState,
    /// The system's print service; `None` in tests, which print nothing.
    #[cfg(not(target_arch = "wasm32"))]
    pub print_host: Option<Arc<dyn print_host::PrintHost>>,
    /// Where the user's settings are kept between runs; `None` keeps nothing (tests, the browser).
    pub user_store: Option<Box<dyn user_store::UserStore>>,
    /// The window's layout as the store has it; `None` until the store is read.
    pub kept_layout: Option<user_store::KeptLayout>,
    /// Opens web links in the browser and shows files in the file manager; the desktop app supplies the system's,
    /// tests and the browser build have none.
    pub opener: craft_ext::platform::open::Opener,
    /// Reads the system clipboard's text for a paste outside a text field; the desktop app supplies
    /// [`clipboard::system_text`], tests and the browser build have none and wait for the window's paste event.
    pub clipboard_reader: Option<clipboard::ReadClipboardFn>,
    /// Reads the clipboard's PDF off the window's thread for a paste on the canvas; the desktop app supplies the
    /// system's, tests and the browser build have none and paste text only.
    pub clipboard_flavours: Option<Box<dyn clipboard::ClipboardFlavours>>,
    /// When the extension folders were last read ([`startup::poll_folders`]); the desktop app polls them, tests and the
    /// browser build do not.
    pub folder_poll: Option<startup::FolderPoll>,
    /// Where the browser build keeps the modules and macros it installs ([`web_store`]); the desktop app has none.
    pub web_extensions: web_store::WebExtensions,
    /// The graphics adapter the window draws with, for About FontCraft; `None` while the framework does not report it
    /// (and in tests).
    pub graphics: Option<crate::dialogs::about::Graphics>,
    /// The canvas overlays' switches, data and the text note being typed (`canvas::overlays`).
    pub overlays: crate::canvas::overlays::Overlays,
    /// The control channel: its listener, the window its input goes to, the replies waiting on frames.
    pub control: crate::control::Control,
    /// The window's context, kept from the last frame, so an edit command the person fires can reach the list that
    /// holds the keyboard focus ([`clipboard::to_focused_list`]).
    /// The Features tab's checks and compiles off the window's thread.
    pub(crate) feature_work: crate::shell::feature_work::FeatureWork,
    pub(crate) window: Option<egui::Context>,
    #[cfg(not(target_arch = "wasm32"))]
    file_panels: Option<craft::dialogs::service::FilePanels>,
}

impl FontCraftApp {
    /// The app for the desktop window of `ctx`: no document yet ([`startup::launch`] opens the first), the system file
    /// pickers.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(ctx: &egui::Context) -> FontCraftApp {
        let ctx = ctx.clone();
        let (files, panels) = craft::dialogs::service::FileService::new(craft::host::Wake::new(move || ctx.request_repaint()));
        let host = Host::builder(APP).jobs(craft::host::default_jobs()).files(Arc::new(files)).build();
        let mut app = FontCraftApp::new(Session::new(), Arc::new(host));
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
            root: appearance::root(),
            catalog: Catalog::build(),
            tool_slots: tool_slots(),
            view: ViewState::default(),
            active_tool: Box::new(crate::canvas::tools::bridge::ToolBridge::default()),
            space_preview: Default::default(),
            bitmap_canvas: Default::default(),
            tools: crate::canvas::tools::Tools::default(),
            knobs: Default::default(),
            images: Default::default(),
            view_revision: 0,
            enabled_at: None,
            models: Models::default(),
            sent: Vec::new(),
            status: String::new(),
            closing: close::ClosePrompt::default(),
            tt: Default::default(),
            dialogs: crate::dialogs::form::DialogState::default(),
            context_menus: context_menu::ContextMenus::default(),
            clipboard: clipboard::Clipboard::default(),
            overlays: Default::default(),
            picks: Picks::default(),
            frame_hook: None,
            preview_host: None,
            printing: Default::default(),
            #[cfg(not(target_arch = "wasm32"))]
            print_host: None,
            user_store: None,
            kept_layout: None,
            opener: craft_ext::platform::open::Opener::NONE,
            clipboard_reader: None,
            clipboard_flavours: None,
            folder_poll: None,
            web_extensions: web_store::WebExtensions::browser(),
            graphics: None,
            control: crate::control::Control::default(),
            feature_work: Default::default(),
            window: None,
            #[cfg(not(target_arch = "wasm32"))]
            file_panels: None,
        };
        if let Some(error) = app.catalog.error.clone() {
            app.status = error;
        }
        // The window's first tab set is the document it opens with.
        crate::shell::tabs::follow_document(&mut app);
        app.project();
        // In the browser, the modules and macros installed on earlier visits.
        web_store::restore(&mut app);
        app
    }

    /// The app as [`Self::new`] gives it, with one Edit View tab showing the font's first glyph: the start the tests
    /// of the edit view build on.
    #[cfg(test)]
    pub fn with_edit_tab(session: Session, host: Arc<Host>) -> FontCraftApp {
        let mut app = FontCraftApp::new(session, host);
        crate::shell::tabs::open_blank(&mut app);
        app.project();
        app
    }

    /// Whether a document or project has changes not yet saved.
    pub fn has_unsaved_changes(&self) -> bool {
        close::has_unsaved(&self.session)
    }

    /// Applies what was posted: the only place that runs engine commands. A failed command changes nothing and
    /// leaves its message in the status line.
    pub fn drain(&mut self) {
        let mut failed = Vec::new();
        for command in self.picks.poll(&self.host, &mut failed) {
            self.sent.push(FontCraftApp::route(command));
        }
        for error in &failed {
            self.control.events.error(error);
        }
        for msg in std::mem::take(&mut self.sent) {
            if let Err(error) = self.apply_msg(msg) {
                failed.push(error);
            }
        }
        if let Some(last) = failed.pop() {
            self.status = last;
        }
        // A shell command may have switched or closed the active document: its tab set shows before anything reads
        // the tabs, and again after a close the drain answered.
        crate::shell::tabs::follow_document(self);
        crate::canvas::text::sync_layer(self);
        crate::canvas::tools::apply::sync(self);
        crate::canvas::text::sync(self);
        self.tools.refresh();
        close::after_drain(self);
        crate::shell::tabs::follow_document(self);
        crate::canvas::text::sync_layer(self);
        self.project();
        crate::canvas::text::after_project(self);
        crate::control::events::observe(self);
    }

    /// Runs `msg` now, as `drain` would after the frame, and returns the command's result: what views posted
    /// before it is applied first. The control channel runs its requests here. With `ask_files`, a command of
    /// [`files::PICKED`] without its path opens the file picker and returns `{"picker": true}`; without, it runs
    /// as given and the engine reports the missing path. A failure also goes to the status line.
    pub fn run_now(&mut self, msg: Msg, ask_files: bool) -> Result<Value, String> {
        self.run_now_as(msg, ask_files, Origin::Person)
    }

    /// Runs `msg` now as a request that names the command (`engine.execute`): as [`Self::run_now`] without the file
    /// picker, and no follow-up question opens.
    pub fn run_request(&mut self, msg: Msg) -> Result<Value, String> {
        self.run_now_as(msg, false, Origin::Control)
    }

    fn run_now_as(&mut self, msg: Msg, ask_files: bool, origin: Origin) -> Result<Value, String> {
        self.drain();
        let id = msg.command_id().map(str::to_owned);
        let result = self.apply(msg, ask_files, origin);
        if let Err(error) = &result {
            self.status = with_command_id(id, error.clone());
        }
        self.drain();
        result
    }

    /// Runs the engine command `id` now, as `drain` applies it, and returns its result. For shell commands that build
    /// on an engine command's reply.
    pub(crate) fn run_command(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.apply(Msg::run(id, params), false, Origin::Person)
    }

    /// Applies one message as `drain` does; the error names the command that failed.
    pub fn apply_msg(&mut self, msg: Msg) -> Result<(), String> {
        let id = msg.command_id().map(str::to_owned);
        self.apply(msg, true, Origin::Person).map(|_| ()).map_err(|error| with_command_id(id, error))
    }

    /// Applies one message: the only code that runs `Session::execute`. Tool input runs the toolbox, whose answer
    /// is applied here too, before the next message. Each command run, and each other failure, goes into the control
    /// channel's events; queries and shell getters, which change nothing, do not.
    fn apply(&mut self, msg: Msg, ask_files: bool, origin: Origin) -> Result<Value, String> {
        let command = msg.command_id().map(str::to_owned);
        let result = self.apply_now(msg, ask_files, origin);
        match command {
            Some(id) => {
                let query = fontcraft_engine::find_command(&id).is_some_and(|spec| !spec.journal) || crate::shell::getters::find(&id).is_some();
                if !query {
                    let revision = self.session.active().map(|d| d.revision);
                    self.control.events.command(&id, result.as_ref().map(|_| ()).map_err(String::as_str), revision);
                }
            }
            None => {
                if let Err(error) = &result {
                    self.control.events.error(error);
                }
            }
        }
        result
    }

    fn apply_now(&mut self, msg: Msg, ask_files: bool, origin: Origin) -> Result<Value, String> {
        match msg {
            Msg::Run(command) => {
                // File › Open… of a module or macro installs it, as dropping it on the window does. A request that
                // names `file.open` gets the engine's answer.
                if origin == Origin::Person
                    && let Some(install) = dropped::open_as_install(&command)
                {
                    return self.apply_now(install, false, origin);
                }
                if ask_files && self.picks.ask(&self.host, &self.session, &command) {
                    return Ok(json!({ "picker": true }));
                }
                let mut params = if command.params.is_null() { json!({}) } else { command.params.clone() };
                if origin == Origin::Person {
                    params = crate::shell::compatibility::with_matching(self, &command.id, params);
                }
                if matches!(command.id.as_str(), "file.save" | "file.saveAs") {
                    crate::shell::tabs::upkeep::store_display_strings(self);
                }
                let before = self.session.active().map(|d| (d.id, Arc::clone(&d.font)));
                let work = if origin == Origin::Control { self.control.work.take() } else { None };
                let result = match work {
                    Some(work) => fontcraft_engine::work::with(work.token, work.report, || self.session.execute(&command.id, &params)),
                    None => self.session.execute(&command.id, &params),
                };
                let reply = match result {
                    Ok(reply) => reply,
                    Err(e) => {
                        let error = e.to_string();
                        crate::shell::start::after_error(self, &command.id, &error);
                        crate::dialogs::changed_on_disk::after_error(self, &command.id, &params, &error);
                        return Err(error);
                    }
                };
                let switched = crate::shell::tabs::follow_document(self);
                crate::shell::tabs::upkeep::after_command(self, &command.id, &params, &reply, before, switched);
                // A file opened anew brings back its tabs from its display strings.
                let mut opened = if command.id == "file.open" && reply.get("alreadyOpen").is_none() {
                    crate::shell::tabs::upkeep::open_display_strings(self)
                } else {
                    Vec::new()
                };
                crate::shell::start::after_run(self, &command.id);
                crate::canvas::tools::apply::adopt(self, &reply);
                if origin == Origin::Person {
                    crate::dialogs::after::after_run(self, &command.id, &params, &reply);
                    // A request that names a copy gets the text in its reply; the system clipboard stays the person's.
                    clipboard::after_run(self, &command.id, &reply);
                    crate::shell::font_info_tabs::after_run(self, &command.id, &reply);
                }
                crate::shell::project::after_run(self, &command.id, &reply);
                crate::shell::extension_downloads::after_run(self, &command.id, &params, &reply);
                web_store::after_run(self, &command.id, &params, &reply);
                crate::shell::text_preview::note_export(&mut self.view.text_preview, &reply);
                let notice = self.after_command(&command, &reply).or_else(|| notices::notice(&command.id, &reply));
                if let Some(notice) = opened.pop().or(notice) {
                    self.status = notice;
                }
                self.prune_font_view_selection();
                Ok(reply)
            }
            Msg::Shell(command) => {
                if let Some(get) = crate::shell::getters::find(&command.id) {
                    return Ok(get(self));
                }
                if ask_files && self.picks.ask(&self.host, &self.session, &command) {
                    return Ok(json!({ "picker": true }));
                }
                // Shell commands change view state, which the projection reads.
                self.view_revision += 1;
                crate::shell::commands::run(self, &command).map(|()| Value::Null)
            }
            Msg::Preview(step) => crate::canvas::tools::apply::preview(self, step).map(|()| Value::Null),
            Msg::Tool(input) => crate::canvas::tools::apply::input(self, input).pop().map_or(Ok(Value::Null), Err),
            Msg::View(ViewMsg::SelectTool(tool)) => {
                let previous = self.view.tool;
                self.apply_view(ViewMsg::SelectTool(tool))?;
                Ok(json!({ "tool": self.view.tool, "previous": previous }))
            }
            Msg::View(view) => self.apply_view(view).map(|()| Value::Null),
        }
    }

    /// Copies what each panel shows into its view model when the engine or the view state moved.
    pub fn project(&mut self) {
        // First: the Info box reads whether the image is still selected.
        crate::canvas::images::refresh(self);
        let stamp = Stamp::of(&self.session, self.view_revision);
        let mut cx = ProjectCx { session: &mut self.session, view: &self.view };
        self.models.project(stamp, &mut cx);
        self.models.project_sweep(&mut cx);
        models::console::project(&mut self.models.console, &mut cx);
        self.models.project.project(&mut cx);
        models::palette::project(&mut self.models.palette, &mut cx, &self.tools, stamp);
        models::palette_modules::project(&mut self.models.modules, &mut cx);
        models::compare::project_fonts(&mut self.models.compare_fonts, &mut cx);
        models::text_preview::project(&mut self.models.text_preview, &mut cx, &self.host);
        models::color::project(&mut self.models.color, &mut cx, &self.tools, stamp);
        // Last: it takes the whole app, so `cx` must be done.
        crate::canvas::overlays::refresh(self);
        crate::canvas::reporters::refresh(self);
        crate::canvas::reference_font::refresh(self);
        crate::canvas::images::cells::refresh(self);
        crate::canvas::images::cells::space_center(self);
        let engine = Stamp::of(&self.session, 0);
        if self.enabled_at != Some(engine) {
            self.enabled_at = Some(engine);
            self.catalog.refresh(&self.session);
        }
        menus_dynamic::refresh(self);
        crate::shell::window::refresh_menu(self);
        menu_states::refresh(self);
    }

    /// Records whether the window is drawn dark; the canvas colours and the Always use Light Mode scopes follow it.
    pub fn set_system_dark(&mut self, dark: bool) {
        if self.view.system_dark != dark {
            self.view.system_dark = dark;
            self.view_revision += 1;
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
            ViewMsg::TextPreviewArea(area) => self.view.text_preview.area = area,
            ViewMsg::SidebarMenu(path) => self.view.sidebar.menu_row = path,
            ViewMsg::FeatureCompletion(ask) => self.view.font_info.features.completion = ask,
            ViewMsg::FeatureDraft(draft) => self.view.font_info.features.draft = draft,
            ViewMsg::FeatureCaretPlaced => self.view.font_info.features.caret = None,
            ViewMsg::FeaturePreview(ask) => self.view.font_info.features.preview = ask,
            ViewMsg::FeatureFind(find) => self.view.font_info.features.find = find,
            ViewMsg::FontViewSearchFocused => self.view.font_view.focus_search = false,
            ViewMsg::FontViewRevealed => self.view.font_view.reveal = None,
            ViewMsg::Bitmap(input) => return crate::shell::bitmap_editor::input(self, input),
        }
        Ok(())
    }

    /// The message a fired command posts. `tool.select` with a `tool` parameter makes that tool current; one naming no
    /// tool goes to the shell command, which refuses it.
    pub fn route(command: CommandRef) -> Msg {
        match catalog::kind(&command.id) {
            Kind::Tool(own) => match catalog::chosen_tool(own, &command) {
                Ok(tool) => Msg::View(ViewMsg::SelectTool(tool)),
                Err(_) => Msg::Shell(command),
            },
            Kind::Shell => Msg::Shell(command),
            Kind::Engine => Msg::Run(command),
        }
    }

    /// `command` with its targets filled: from Font View's selection while Font View is in front with glyphs
    /// selected, else from the canvas selection.
    /// In text mode a filter takes the glyphs of the text selection, on the layer the canvas edits.
    pub(crate) fn with_targets(&mut self, command: CommandRef) -> CommandRef {
        if self.font_view_targets() {
            return self.with_font_view_targets(command);
        }
        let text_glyphs = crate::canvas::text::filter_glyphs(&self.view, &command);
        let targeted = crate::canvas::tools::targets::with_targets(self, command);
        let Some(glyphs) = text_glyphs else { return targeted };
        let mut m = Map::new();
        if let Some(layer) = targeted.params.get("layer") {
            m.insert("layer".into(), layer.clone());
        }
        m.insert("glyphs".into(), json!(glyphs));
        CommandRef::with_params(targeted.id, Value::Object(m))
    }

    /// Whether the person chose what a fired command acts on: glyphs selected in Font View while it is in front, or a
    /// glyph opened in the edit view while the edit view is in front. With neither, [`Self::fired`] takes no targets
    /// from [`Self::with_targets`], whose canvas part falls back to the font's first glyph: a dialog's command then acts
    /// on every glyph or the dialog asks for a glyph, and any other glyph command refuses ([`select_glyph_first`]).
    pub(crate) fn targets_chosen(&self) -> bool {
        // `view.glyph` keeps the glyph the live tab shows while Font View is in front.
        let glyph_shown = self.view.glyph.is_some() && !self.view.tabs.font_view_shows();
        self.font_view_targets() || glyph_shown
    }

    /// Whether fired commands take Font View's selection: glyphs are selected and Font View is the showing tab.
    fn font_view_targets(&self) -> bool {
        !self.view.font_view.selected.is_empty() && self.view.tabs.font_view_shows()
    }
}

impl FontCraftApp {
    /// A command the person fired (menu item, shortcut): posts what [`Self::fired`] gives, or leaves its failure in
    /// the status line.
    pub(crate) fn fire(&mut self, command: CommandRef) {
        match self.fired(command) {
            Ok(Some(msg)) => self.sent.push(msg),
            Ok(None) => {}
            Err(error) => self.status = error,
        }
    }

    /// What a command the person fired (menu item, shortcut, `ui.menu.invoke`) runs: an engine command takes its
    /// targets from the selection ([`Self::with_targets`]); one that has a dialog and was fired without parameters
    /// opens the dialog on those targets instead of running. `None` when the click was handled here: a question or
    /// dialog opened, or text mode posted its edits.
    pub(crate) fn fired(&mut self, command: CommandRef) -> Result<Option<Msg>, String> {
        // Quit and Close with unsaved work, and New from Glyph Sets from the menu, ask first.
        if close::fired(self, &command) || crate::shell::start::fired(self, &command) {
            return Ok(None);
        }
        let msg = match FontCraftApp::route(command) {
            Msg::Run(fired) => {
                // Text mode's Edit menu acts on the text.
                if let Some(result) = crate::canvas::text::edit::fired(self, &fired) {
                    self.sent.extend(result?);
                    return Ok(None);
                }
                // An extension's menu row fills in the filter or macro it runs.
                let fired = menus_dynamic::fill_targets(self, fired)?;
                let given = !is_empty(&fired.params);
                let targeted = self.with_targets(fired.clone());
                let Some(command) = clipboard::fired(self, targeted) else { return Ok(None) };
                let chosen = self.targets_chosen();
                // Font View's per-glyph batch is no target list for the dialog, which then reads the selection itself.
                let targets = match &command.params {
                    Value::Object(m) if command.id == fired.id && chosen => m.clone(),
                    _ => Map::new(),
                };
                if crate::dialogs::after::fired(self, &fired.id, &fired.params, &targets) {
                    return Ok(None);
                }
                // Targets filled with nothing chosen come from the font's first glyph, which the person never picked.
                if !chosen && !given && command.id == fired.id && !is_empty(&command.params) {
                    return Err(select_glyph_first(&fired.id));
                }
                Msg::Run(command)
            }
            other => other,
        };
        Ok(Some(msg))
    }
}

/// What the status line says when a glyph command is fired with no glyph shown and none selected in Font View.
pub(crate) fn select_glyph_first(id: &str) -> String {
    let label = fontcraft_engine::find_command(id).map_or(id, |spec| spec.label);
    format!("{label}: select a glyph first")
}

fn is_empty(params: &Value) -> bool {
    params.is_null() || params.as_object().is_some_and(Map::is_empty)
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
            // A numbered shortcut fires its catalog definition, whose id holds the parameters.
            let command = craft_ext::ui::runtime::commands::fired_command(fired.command);
            // A key fires only the commands whose shortcut works where the window is (a tool's keys not in Font View).
            if fired.source == FireSource::Key && !catalog::shortcut_works(&command.id, self.font_view_shows(), self.view.tool) {
                continue;
            }
            // A held tool key fires again on release; the tool stays.
            if fired.phase != FirePhase::Release && !clipboard::to_focused_list(self, &command) {
                self.fire(command);
            }
        }
    }

    fn logic(&mut self, ctx: &egui::Context) {
        if self.window.is_none() {
            self.window = Some(ctx.clone());
        }
        self.set_system_dark(appearance::system_dark(&self.root, ctx));
        models::text_preview::logic(&mut self.models.text_preview, ctx);
        crate::shell::feature_work::logic(self, ctx);
        dropped::take(self, ctx);
        crate::shell::external_changes::on_focus(self, ctx);
        crate::canvas::images::logic(self, ctx);
        crate::control::logic(self, ctx);
        open::take_dropped(self, ctx);
        crate::canvas::images::drop::take(self, ctx);
        crate::shell::font_audit::logic(self, ctx);
        crate::shell::axis_sweep::logic(self, ctx);
        startup::poll_folders(self, ctx);
        if clipboard::finish_pdf(self) {
            ctx.request_repaint_after(clipboard::READ_POLL);
        }
        crate::control::drain(self);
        if let Some(mut hook) = self.frame_hook.take() {
            if let Some(after) = hook.frame(self) {
                ctx.request_repaint_after(after);
            }
            self.frame_hook = Some(hook);
        }
        if let Some(again) = user_store::save(self) {
            ctx.request_repaint_after(again);
        }
        if self.closing.take_close_window() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        crate::shell::show(self, ui);
        if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
            self.dialogs.modal = ui.ctx().memory(|memory| memory.top_modal_layer().is_some());
        }
        craft_ext::ui::runtime::windows::keep_modal_on_top(ui.ctx());
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(panels) = self.file_panels.as_mut() {
            panels.pump::<eframe::Frame>(None);
        }
        crate::control::after_ui(self, ui.ctx());
    }

    fn close_requested(&mut self) -> CloseDecision {
        let decision = close::close_requested(self);
        if decision == CloseDecision::Close {
            user_store::flush(self);
        }
        decision
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod active_layer_tests;
