//! The app with floating panels in OS windows of their own, over a stand-in for the window system.
//!
//! [`NativeApp`] runs the real app as [`TestApp`](super::TestApp) does, on a context that opens OS viewports (not
//! embedded). The main window reports its place on the desktop, as under X11, so the dock opens floating panels as
//! OS windows. A renderer like eframe's runs each window's pass nested inside the main window's, at the place its
//! builder asks for, and keeps what the window was told: its level, its commands, its cursor and its AccessKit tree.

use std::cell::RefCell;
use std::sync::Arc;

use craft::ui::runtime::env::RootEnv;
use craft::ui::runtime::frame::{FrameInput, set_input, take_output};
use craft::ui::shell::app::ShellApp;
use craft::ui::testing::Frames;
use craft::ui::testing::access::{AccessTree, tree_of};
use egui::{Context, CursorIcon, Event, Pos2, Rect, ViewportCommand, ViewportId, ViewportInfo, WindowLevel};
use fontcraft_engine::Session;
use serde_json::{Value, json};

use crate::app::{APP, FontCraftApp};

/// The main window, at the desktop's origin.
const MAIN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1280.0, 800.0));
/// Seconds between two frames.
const FRAME_SECONDS: f64 = 1.0 / 60.0;

/// An OS window other than the main one, as the stand-in keeps it.
pub struct Window {
    pub id: ViewportId,
    /// Where the window is on the desktop.
    pub place: Rect,
    /// The level its builder asked for last.
    pub level: Option<WindowLevel>,
    /// The cursor its last pass asked for.
    pub cursor: CursorIcon,
    /// The AccessKit tree of its last pass.
    pub tree: Option<AccessTree>,
    /// The events its next pass gets.
    events: Vec<Event>,
    /// Whether it ran a pass in the running frame; a window that ran none has closed.
    ran: bool,
}

#[derive(Default)]
struct System {
    windows: Vec<Window>,
    /// The commands every pass sent to a window since the last [`NativeApp::take_commands`].
    commands: Vec<(ViewportId, ViewportCommand)>,
    time: f64,
}

thread_local! {
    static SYSTEM: RefCell<System> = RefCell::new(System::default());
}

/// Every window the system knows, with its facts, as an integration lists them in each pass's input.
fn viewports() -> egui::ViewportIdMap<ViewportInfo> {
    let mut all = egui::ViewportIdMap::default();
    let main = ViewportInfo {
        inner_rect: Some(MAIN),
        outer_rect: Some(MAIN),
        focused: Some(true),
        native_pixels_per_point: Some(1.0),
        ..ViewportInfo::default()
    };
    all.insert(ViewportId::ROOT, main);
    SYSTEM.with(|system| {
        for window in &system.borrow().windows {
            let info = ViewportInfo {
                parent: Some(ViewportId::ROOT),
                inner_rect: Some(window.place),
                outer_rect: Some(window.place),
                focused: Some(false),
                native_pixels_per_point: Some(1.0),
                ..ViewportInfo::default()
            };
            all.insert(window.id, info);
        }
    });
    all
}

fn keep_commands(output: &egui::FullOutput) {
    SYSTEM.with(|system| {
        let mut system = system.borrow_mut();
        for (id, viewport) in &output.viewport_output {
            system.commands.extend(viewport.commands.iter().map(|command| (*id, command.clone())));
        }
    });
}

/// A renderer like eframe's: the window opens at its builder's place, and its pass runs nested in its parent's.
fn render_window(ctx: &Context, viewport: egui::ImmediateViewport<'_>) {
    let id = viewport.ids.this;
    let builder = viewport.builder;
    let (place, events, time) = SYSTEM.with(|system| {
        let mut system = system.borrow_mut();
        let time = system.time;
        let index = match system.windows.iter().position(|window| window.id == id) {
            Some(index) => index,
            None => {
                let place = Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
                let window = Window { id, place, level: None, cursor: CursorIcon::Default, tree: None, events: Vec::new(), ran: false };
                system.windows.push(window);
                system.windows.len() - 1
            }
        };
        let window = &mut system.windows[index];
        if let Some(at) = builder.position {
            window.place = Rect::from_min_size(at, window.place.size());
        }
        if let Some(size) = builder.inner_size {
            window.place = Rect::from_min_size(window.place.min, size);
        }
        window.level = builder.window_level.or(window.level);
        window.ran = true;
        (window.place, std::mem::take(&mut window.events), time)
    });
    let input = egui::RawInput {
        viewport_id: id,
        viewports: viewports(),
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, place.size())),
        time: Some(time),
        events,
        ..egui::RawInput::default()
    };
    let mut show = viewport.viewport_ui_cb;
    let mut output = ctx.run_ui(input, |ui| show(ui));
    output.textures_delta.clear();
    keep_commands(&output);
    let tree = tree_of(&mut output);
    SYSTEM.with(|system| {
        if let Some(window) = system.borrow_mut().windows.iter_mut().find(|window| window.id == id) {
            window.cursor = output.platform_output.cursor_icon;
            window.tree = Some(tree);
        }
    });
}

/// The app and the context its frames run on.
pub struct NativeApp {
    pub app: FontCraftApp,
    pub ctx: Context,
}

impl Default for NativeApp {
    fn default() -> Self {
        NativeApp::new()
    }
}

impl NativeApp {
    /// The app over a new font with one Edit View tab showing, after its first frames.
    pub fn new() -> NativeApp {
        SYSTEM.with(|system| *system.borrow_mut() = System::default());
        let ctx = Frames::new().ctx;
        ctx.set_embed_viewports(false);
        Context::set_immediate_viewport_renderer(render_window);
        let app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(craft::host::Host::builder(APP).build()));
        let mut native = NativeApp { app, ctx };
        native.frames(3);
        native
    }

    /// One frame as the desktop app runs it: `logic`, the main window's pass (the other windows' passes run inside
    /// it), and the commands the frame fired into `output`.
    pub fn frame(&mut self) {
        let ctx = self.ctx.clone();
        self.app.logic(&ctx);
        set_input(&ctx, FrameInput::new(RootEnv::default()).commands(self.app.catalog.input()).host(Arc::clone(&self.app.host)));
        let time = SYSTEM.with(|system| {
            let mut system = system.borrow_mut();
            system.time += FRAME_SECONDS;
            system.windows.iter_mut().for_each(|window| window.ran = false);
            system.time
        });
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, MAIN.size())),
            time: Some(time),
            focused: true,
            viewports: viewports(),
            ..egui::RawInput::default()
        };
        let app = &mut self.app;
        let mut output = ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
        keep_commands(&output);
        SYSTEM.with(|system| system.borrow_mut().windows.retain(|window| window.ran));
        self.app.output(take_output(&ctx));
    }

    pub fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.frame();
        }
    }

    /// Sends `method` with `params`, runs three frames, and returns the result of a request that must succeed.
    pub fn ok(&mut self, method: &str, params: Value) -> Value {
        let reply = crate::control::handle(&mut self.app, &json!({"id": 1, "method": method, "params": params}));
        assert_eq!(reply["ok"], true, "{method}: {reply}");
        self.frames(3);
        reply["result"].clone()
    }

    /// The OS windows open besides the main one.
    pub fn windows(&self) -> Vec<ViewportId> {
        SYSTEM.with(|system| system.borrow().windows.iter().map(|window| window.id).collect())
    }

    /// What `read` reads of the OS window `id`.
    pub fn window<R>(&self, id: ViewportId, read: impl FnOnce(&Window) -> R) -> R {
        SYSTEM.with(|system| read(system.borrow().windows.iter().find(|window| window.id == id).expect("the window is open")))
    }

    /// Gives `events`, in the window's own points, to the next pass of the OS window `id`, and runs that frame.
    pub fn window_frame(&mut self, id: ViewportId, events: Vec<Event>) {
        SYSTEM.with(|system| {
            if let Some(window) = system.borrow_mut().windows.iter_mut().find(|window| window.id == id) {
                window.events.extend(events);
            }
        });
        self.frame();
    }

    /// The commands every pass sent to a window since the last call.
    pub fn take_commands(&self) -> Vec<(ViewportId, ViewportCommand)> {
        SYSTEM.with(|system| std::mem::take(&mut system.borrow_mut().commands))
    }
}

#[cfg(test)]
mod tests;
