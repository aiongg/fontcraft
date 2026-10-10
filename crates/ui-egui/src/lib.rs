//! FontCraft user interface (L6): a thin egui shell over the engine.
//!
//! - [`app::FontcraftApp`]: the session plus UI state; draws itself into any egui context.
//! - [`shell`]: the menu bar (built from the command registry), keymap, tab strip, status bar,
//!   Font View, Palette, and the shell commands, one list per module.
//! - [`canvas`]: Edit View: the camera, metric lines, filled glyphs, outlines, nodes.
//! - [`control`]: the JSON control protocol agents drive the running app with.
//! - [`theme`]: design tokens.
//! - [`state`]: serializable UI state: tabs, their cameras and selections, display toggles.
//!
//! Nothing here depends on a window: the desktop app, the web app and headless tests all use the
//! same [`app::FontcraftApp::ui`]. Files are touched only where a control method names a path
//! (`ui.render`, `ui.screenshot`).
#![forbid(unsafe_code)]

pub mod app;
pub mod canvas;
pub mod control;
pub mod shell;
pub mod state;
pub mod theme;

pub use app::FontcraftApp;
pub use shell::UI_COMMANDS;

#[cfg(test)]
mod tests;
