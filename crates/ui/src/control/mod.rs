//! The control channel (`docs/control-protocol.md`): one JSON request in, one JSON reply out.
//!
//! `{"id": 1, "method": "engine.execute", "params": {"command": "glyph.newGlyph", "params": {}}}` →
//! `{"id": 1, "ok": true, "result": {…}}` or `{"id": 1, "ok": false, "error": "…"}`.
//!
//! Each module lists its methods in a slice and adds one line to [`MODULES`]. A method reads the app directly; a
//! method that changes the engine goes through [`FontCraftApp::run_now`], the path `drain` takes, so it gets the
//! command's result. A method whose package is not built yet is in [`LATER`] and answers with an error naming it.
//!
//! Hosts:
//! - [`handle`] answers one request at once: tests ([`crate::testing`]) and hosts without a window.
//! - The desktop app listens with [`Control::listen`] (`server.rs`). Requests are answered on the UI thread between
//!   frames ([`logic`]); window input (`ui.click`, `ui.drag`, `ui.move`, `ui.wheel`) is replied to once the frames
//!   have carried it, `ui.screenshot` once the window is captured, and `ui.wait` once its frames have passed or the
//!   window is idle ([`after_ui`]). A request that reads the windows' elements (`elements::reads`) is answered once
//!   the frames after it have been drawn.

mod app;
mod context_menu;
mod dialogs;
mod elements;
mod engine;
pub mod events;
mod inspect;
mod menu;
mod pointer;
mod render;
#[cfg(not(target_arch = "wasm32"))]
mod screenshot;
#[cfg(not(target_arch = "wasm32"))]
mod server;
mod text;
mod wait;
mod window_input;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod audit_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod features_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
pub(crate) mod tt_tests;

use std::sync::Arc;

use craft::ui::runtime::windows::WindowHost;
use fontcraft_engine::file_access::{self, AutomationRoots};
use fontcraft_engine::guard;
use serde_json::{Value, json};

use crate::app::FontCraftApp;

pub use inspect::INSPECT;
pub use window_input::{SWITCHES, Switch};

/// Runs one method with its parameters and returns its result, or the error the client gets.
pub type Method = fn(&mut FontCraftApp, &Value) -> Result<Value, String>;

/// Every module's methods.
pub const MODULES: &[&[(&str, Method)]] = &[
    app::METHODS,
    context_menu::METHODS,
    dialogs::METHODS,
    elements::METHODS,
    engine::METHODS,
    events::METHODS,
    inspect::METHODS,
    menu::METHODS,
    pointer::METHODS,
    render::METHODS,
    window_input::METHODS,
    text::METHODS,
];

/// Methods only the desktop app's listener answers: `control.cancel` at once, the others once the window has drawn
/// (`answer` takes them before [`dispatch`]). Headless hosts refuse them.
const WINDOW_METHODS: &[&str] = &["control.cancel", "ui.screenshot", "ui.wait"];

/// Methods of the protocol whose package is not built yet: `(method or prefix ending in `.`, package)`. A package
/// that adds its methods to [`MODULES`] removes its line here.
pub const LATER: &[(&str, &str)] = &[];

/// The control channel's state on the app.
#[derive(Default)]
pub struct Control {
    /// The folders control requests may read from and write to; `None` confines nothing.
    pub roots: Option<Arc<AutomationRoots>>,
    /// The window that window input goes to, as of the last frame; `None` without frames.
    window: Option<WindowHost>,
    /// Window input steps queued by the request being answered.
    queued: usize,
    /// A `ui.set` whose next round runs once frames have been drawn.
    stage: Option<elements::Stage>,
    /// Where the window input requests queued stands; while it is not idle, `drain` runs confined.
    input: Input,
    /// What cancels the command of the request being answered and takes its progress.
    pub work: Option<Work>,
    /// What happened, for `events.list`.
    pub events: events::Events,
    #[cfg(not(target_arch = "wasm32"))]
    server: Option<server::Server>,
}

impl Control {
    /// Answers requests from a listener on `127.0.0.1:port`, bound earlier; `ctx` is woken when one arrives.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn listen(&mut self, listener: std::net::TcpListener, ctx: &egui::Context) -> std::io::Result<std::net::SocketAddr> {
        let server = server::Server::start(listener, ctx.clone())?;
        let addr = server.addr;
        self.server = Some(server);
        Ok(addr)
    }

    /// Whether replies wait for frames (window input or a screenshot).
    pub fn waiting(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(server) = &self.server {
            return server.waiting();
        }
        false
    }
}

/// What cancels the engine command a request names, and takes its progress. Nothing else the request's answer runs is
/// cancelled: what was fired before it is the person's.
pub struct Work {
    pub token: Arc<fontcraft_engine::work::Token>,
    pub report: Option<fontcraft_engine::work::Report>,
}

/// Where control-channel window input stands. The commands it fires run in the `drain` after the pass that
/// carried it, so that drain is confined too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Input {
    #[default]
    Idle,
    /// Steps are queued or in flight.
    InFlight,
    /// The input has run; the next drain applies what it fired.
    LastDrain,
}

/// What a request comes to.
enum Answer {
    /// The reply, now.
    Reply(Result<Value, String>),
    /// A screenshot of the window, written to the path (already checked against the roots) or returned inline. The
    /// web build takes no screenshots.
    #[cfg(not(target_arch = "wasm32"))]
    Screenshot(Option<String>),
    /// A `ui.wait`: the reply goes out once the frames have passed or the window is idle. Only the listener reads it.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Wait(wait::Wait),
    /// A request that reads the windows' elements: it runs ([`run`]) once [`elements::FRAMES`] frames have been drawn,
    /// and again after as many while it is [`unfinished`].
    Elements,
}

/// Answers one request now. Never panics: a bug becomes an error reply and the app keeps running. A screenshot
/// gets an error: this entry point has no window to capture.
pub fn handle(app: &mut FontCraftApp, request: &Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return with_id(reply(Err("a request is a JSON object with a string `method`".into())), &id);
    };
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let answered = match answer(app, method, &params) {
        Answer::Reply(result) => reply(result),
        #[cfg(not(target_arch = "wasm32"))]
        Answer::Screenshot(_) => reply(Err("ui.screenshot: this app has no window to capture (ui.render draws glyphs without one)".into())),
        Answer::Wait(_) => reply(Err("ui.wait: this app answers each request at once and draws no frames to wait for".into())),
        // The host draws its own frames between requests: what they showed is what there is.
        Answer::Elements => reply(run(app, method, &params)),
    };
    with_id(answered, &id)
}

/// `{"ok": true, "result": …}` or `{"ok": false, "error": …}`.
fn reply(result: Result<Value, String>) -> Value {
    match result {
        Ok(result) => json!({"ok": true, "result": result}),
        Err(error) => json!({"ok": false, "error": error}),
    }
}

/// `reply` with the request's `id` first.
fn with_id(reply: Value, id: &Value) -> Value {
    let mut out = serde_json::Map::new();
    out.insert("id".into(), id.clone());
    if let Value::Object(map) = reply {
        out.extend(map);
    }
    Value::Object(out)
}

/// Runs `method` with the automation roots in force. A client of the control channel exports inside the
/// destination it names, whatever the app allows the user.
fn answer(app: &mut FontCraftApp, method: &str, params: &Value) -> Answer {
    app.control.queued = 0;
    let roots = app.control.roots.clone();
    file_access::confine(roots.as_ref(), || {
        #[cfg(not(target_arch = "wasm32"))]
        if method == "ui.screenshot" {
            return match screenshot::path(params) {
                Ok(path) => Answer::Screenshot(path),
                Err(e) => Answer::Reply(Err(format!("ui.screenshot: {e}"))),
            };
        }
        if method == "control.cancel" {
            return Answer::Reply(Err("control.cancel: this app answers one request at a time and has none to cancel".into()));
        }
        if method == "ui.wait" {
            return match wait::Wait::parse(params) {
                Ok(wait) => Answer::Wait(wait),
                Err(e) => Answer::Reply(Err(e)),
            };
        }
        if elements::reads(method, params)
            && let Some(ctx) = &app.window
        {
            elements::activate(ctx);
            return Answer::Elements;
        }
        Answer::Reply(run_confined(app, method, params))
    })
}

/// Frames a host that draws its own draws before [`handle`] answers a request: none, or enough for the windows'
/// elements to be read.
#[cfg(test)]
pub(crate) fn frames_before(app: &FontCraftApp, method: &str, params: &Value) -> u64 {
    match &app.window {
        Some(ctx) if elements::reads(method, params) => {
            elements::activate(ctx);
            elements::FRAMES
        }
        _ => 0,
    }
}

/// Whether the request answered last has another round to run once frames have been drawn. Its reply is the last
/// round's.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn unfinished(app: &FontCraftApp) -> bool {
    app.control.stage.is_some()
}

/// Runs a request [`answer`] put off, with the automation roots in force.
fn run(app: &mut FontCraftApp, method: &str, params: &Value) -> Result<Value, String> {
    app.control.queued = 0;
    let roots = app.control.roots.clone();
    file_access::confine(roots.as_ref(), || run_confined(app, method, params))
}

/// Runs `method`, exporting inside the destination it names.
fn run_confined(app: &mut FontCraftApp, method: &str, params: &Value) -> Result<Value, String> {
    let outside = app.session.export_outside();
    app.session.set_export_outside(false);
    let result = guard::catch_panic(|| dispatch(app, method, params));
    app.session.set_export_outside(outside);
    if app.control.queued > 0 {
        app.control.input = Input::InFlight;
    }
    match result {
        Ok(result) => result,
        Err(panic) => Err(format!("internal error (please report it): `{method}` failed unexpectedly: {panic}")),
    }
}

/// Runs the method `name`.
pub fn dispatch(app: &mut FontCraftApp, name: &str, params: &Value) -> Result<Value, String> {
    let method = MODULES.iter().flat_map(|methods| methods.iter()).find(|(n, _)| *n == name).map(|(_, m)| *m);
    if let Some(method) = method {
        return method(app, params);
    }
    if let Some((_, package)) = LATER.iter().find(|(m, _)| *m == name || (m.ends_with('.') && name.starts_with(m))) {
        return Err(format!("`{name}` is not built yet in this app (it arrives with package {package})"));
    }
    Err(format!("unknown method `{name}` (see control.methods)"))
}

/// Every method name the channel answers: what [`dispatch`] runs, in table order, then the window methods.
pub fn method_names() -> Vec<&'static str> {
    MODULES.iter().flat_map(|methods| methods.iter().map(|(n, _)| *n)).chain(WINDOW_METHODS.iter().copied()).collect()
}

/// The control channel's work before the frame: notes the window for window input and, in the desktop app,
/// answers the requests that arrived.
pub fn logic(app: &mut FontCraftApp, ctx: &egui::Context) {
    app.control.window = Some(WindowHost::of(ctx));
    #[cfg(not(target_arch = "wasm32"))]
    server::logic(app, ctx);
}

/// The control channel's work at the end of a UI pass: screenshots due are issued, and replies waiting on window
/// input go out once the input has run.
pub fn after_ui(app: &mut FontCraftApp, ctx: &egui::Context) {
    if app.control.input == Input::InFlight && WindowHost::of(ctx).injected_idle_at_pass_end() {
        app.control.input = Input::LastDrain;
    }
    #[cfg(not(target_arch = "wasm32"))]
    server::after_ui(app, ctx);
}

/// `FontCraftApp::drain` for the frame. While control-channel window input is in flight, and in the drain after it
/// ran, what it fired runs as a request does: inside the automation roots, exporting inside the destination.
pub fn drain(app: &mut FontCraftApp) {
    if app.control.input == Input::Idle {
        app.drain();
        return;
    }
    if app.control.input == Input::LastDrain {
        app.control.input = Input::Idle;
    }
    let roots = app.control.roots.clone();
    let outside = app.session.export_outside();
    app.session.set_export_outside(false);
    file_access::confine(roots.as_ref(), || app.drain());
    app.session.set_export_outside(outside);
}

/// Sets `key` on a JSON object (and does nothing to anything else).
fn set(value: &mut Value, key: &str, item: Value) {
    if let Some(map) = value.as_object_mut() {
        map.insert(key.to_string(), item);
    }
}

/// `params` as an object: `null` is an empty one.
fn object<'a>(method: &str, params: &'a Value) -> Result<std::borrow::Cow<'a, serde_json::Map<String, Value>>, String> {
    match params {
        Value::Null => Ok(std::borrow::Cow::Owned(serde_json::Map::new())),
        Value::Object(map) => Ok(std::borrow::Cow::Borrowed(map)),
        _ => Err(format!("{method}: params must be a JSON object")),
    }
}
