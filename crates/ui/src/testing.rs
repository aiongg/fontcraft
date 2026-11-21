//! A headless app for tests that drive FontCraft through the control channel.
//!
//! [`TestApp`] runs the real app (`ShellApp`'s `logic`, `ui` and `output`) frame by frame on a context with the
//! craft runtime installed, at 1280 × 800 points, with no window. [`TestApp::call`] sends one control request
//! through [`control::handle`](crate::control::handle) and, when it queued window input, runs frames until the
//! input has been carried.
//!
//! ```ignore
//! let mut t = TestApp::new();
//! t.ok("engine.execute", json!({"command": "glyph.newGlyph", "params": {"name": "agent"}}));
//! assert_eq!(t.ok("document.inspect", json!({}))["documents"][0]["undo"], json!(["New Glyph"]));
//! t.ok("ui.click", json!({"x": 640, "y": 400}));
//! ```

use std::sync::Arc;

use craft::host::Host;
use craft::ui::runtime::frame::take_output;
use craft::ui::runtime::windows::WindowHost;
use craft::ui::shell::app::ShellApp;
use craft::ui::testing::Frames;
use fontcraft_engine::Session;
use serde_json::{Value, json};

use crate::app::{APP, FontCraftApp};

/// The window size the frames run at, in points.
pub const SCREEN: [f32; 2] = [1280.0, 800.0];
/// Most frames [`TestApp::call`] runs for one request's window input.
const MAX_INPUT_FRAMES: usize = 64;

/// The app and the frames it runs on.
pub struct TestApp {
    pub app: FontCraftApp,
    pub frames: Frames,
}

impl Default for TestApp {
    fn default() -> Self {
        TestApp::new()
    }
}

impl TestApp {
    /// The app over a new font, after its first frame.
    pub fn new() -> TestApp {
        TestApp::with(Session::with_document())
    }

    /// The app over `session`, after its first frame.
    pub fn with(session: Session) -> TestApp {
        let host = Arc::new(Host::builder(APP).build());
        let app = FontCraftApp::new(session, Arc::clone(&host));
        let mut frames = Frames::new();
        frames.screen(egui::vec2(SCREEN[0], SCREEN[1]));
        frames.host(host);
        let mut test = TestApp { app, frames };
        test.frame();
        test
    }

    /// One frame as the desktop app runs it: `logic` (control requests, then `drain`), the passes of `ui`, and the
    /// commands the frame fired into `output`.
    pub fn frame(&mut self) -> &mut Self {
        self.frame_with(Vec::new())
    }

    /// One frame, as [`Self::frame`], whose input holds `events` (keys from `craft::ui::testing::input`).
    pub fn frame_with(&mut self, events: Vec<egui::Event>) -> &mut Self {
        let ctx = self.frames.ctx.clone();
        self.app.logic(&ctx);
        self.frames.commands(self.app.catalog.input());
        let app = &mut self.app;
        self.frames.frame(events, |ui| app.ui(ui));
        self.app.output(take_output(&ctx));
        self
    }

    /// `n` frames.
    pub fn frames(&mut self, n: usize) -> &mut Self {
        for _ in 0..n {
            self.frame();
        }
        self
    }

    /// Sends `method` with `params` and returns the reply, `{id, ok, result | error}`. Window input the request
    /// queued has run when this returns, and the frame after it too, so its effects are drained.
    pub fn call(&mut self, method: &str, params: Value) -> Value {
        let reply = crate::control::handle(&mut self.app, &json!({"id": 1, "method": method, "params": params}));
        let host = WindowHost::of(&self.frames.ctx);
        if !host.injected_idle() {
            for _ in 0..MAX_INPUT_FRAMES {
                self.frame();
                if host.injected_idle() {
                    break;
                }
            }
            self.frame();
        }
        reply
    }

    /// The result of a request that must succeed.
    pub fn ok(&mut self, method: &str, params: Value) -> Value {
        let reply = self.call(method, params);
        assert_eq!(reply["ok"], true, "{method}: {reply}");
        reply["result"].clone()
    }

    /// The error of a request that must fail.
    pub fn error(&mut self, method: &str, params: Value) -> String {
        let reply = self.call(method, params);
        assert_eq!(reply["ok"], false, "{method} should fail: {reply}");
        reply["error"].as_str().unwrap_or_default().to_string()
    }

    /// Where the pointer is, as the last frame saw it.
    pub fn pointer(&self) -> Option<egui::Pos2> {
        self.frames.ctx.input(|i| i.pointer.latest_pos())
    }
}
