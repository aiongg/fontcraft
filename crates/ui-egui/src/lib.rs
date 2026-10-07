//! FontCraft user interface (L6): a thin egui shell over the engine.
//!
//! - [`app::FontcraftApp`]: the session plus UI state; draws Font View, Edit View, the menu bar,
//!   the Palette and the status bar into any egui context.
//! - [`menus`]: the menu tree built from the command registry, and the shell-only commands.
//! - [`control`]: the JSON control protocol agents drive the running app with.
//! - [`theme`]: design tokens.
//! - [`state`]: serializable UI state.
//!
//! Nothing here depends on a window or a file system: the desktop app, the web app and headless
//! tests all use the same [`app::FontcraftApp::ui`].
#![forbid(unsafe_code)]

pub mod app;
pub mod control;
pub mod menus;
pub mod state;
pub mod theme;

pub use app::FontcraftApp;
pub use menus::UI_COMMANDS;

#[cfg(test)]
mod tests;
