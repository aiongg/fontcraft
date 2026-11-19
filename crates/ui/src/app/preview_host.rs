//! What the desktop app plugs into the UI for the web previews: a hook that runs each frame beside the UI (External
//! Preview's server, `apps/fontcraft-next`), and the web server the TrueType preview page needs.
//!
//! The UI crate opens no network ports and starts no threads; the app supplies these.

use std::time::Duration;

use super::FontCraftApp;

/// Work the app does once per frame, before the UI pass (after the control channel's requests ran).
pub trait FrameHook: Send {
    /// Runs the hook; returns how long until it needs the next frame, when it waits for something.
    fn frame(&mut self, app: &mut FontCraftApp) -> Option<Duration>;
}

/// The file name the preview page loads its font from, beside the page.
pub const FONT_FILE: &str = "font.ttf";

/// Who may open a preview page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// Every device on the local network (all of this computer's network interfaces).
    #[default]
    LocalNetwork,
    /// This computer only (the loopback interface).
    ThisComputer,
}

impl Scope {
    pub fn name(self) -> &'static str {
        match self {
            Scope::LocalNetwork => "localNetwork",
            Scope::ThisComputer => "thisComputer",
        }
    }
}

/// What a preview server hands out: the page, and the font it loads from [`FONT_FILE`] beside it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Site {
    pub page: String,
    pub font: Vec<u8>,
}

/// A web server for the preview page, supplied by the app that can open network ports.
pub trait PreviewHost: Send {
    /// Serves `site` to `scope` and returns the page's address. While a server runs for the same scope, it keeps its
    /// address and serves `site` from the next request on. For another scope a new server starts first and the old
    /// one stops after, so a failure leaves the running server as it was.
    fn serve(&mut self, site: Site, scope: Scope) -> Result<String, String>;
    /// Stops the server. Its port is closed when this returns.
    fn stop(&mut self);
}
