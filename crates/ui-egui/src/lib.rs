//! FontCraft user interface (L6): a thin egui shell over the engine.
//!
//! - [`app::FontcraftApp`]: the session plus UI state; draws itself into any egui context.
//! - [`shell`]: the menu bar (built from the command registry), keymap, tab strip, status bar,
//!   Font View, and the shell commands, one list per module.
//! - [`panels`]: the Palette sections, the Info box and the Edit View bottom bar.
//! - [`canvas`]: Edit View: the camera, metric lines, filled glyphs, outlines, nodes.
//! - [`control`]: the JSON control protocol agents drive the running app with.
//! - [`tools_glue`]: the pointer tools: canvas and control input in, interactions out.
//! - [`dialogs`]: forms over a command's parameters with a live preview.
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
pub mod dialogs;
pub mod icons;
pub mod panels;
pub mod shell;
pub mod state;
pub mod theme;
pub mod tools_glue;

pub use app::FontcraftApp;
pub use shell::UI_COMMANDS;

#[cfg(test)]
mod audit_markers_tests;
#[cfg(test)]
mod canvas_layers_tests;
#[cfg(test)]
mod context_menu_tests;
#[cfg(test)]
mod dialog_tests;
#[cfg(test)]
mod draw_tests;
#[cfg(test)]
mod guide_edit_tests;
#[cfg(test)]
mod panels_tests;
#[cfg(test)]
mod stroke_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod toolbar_tests;
#[cfg(test)]
mod tools_tests;
#[cfg(test)]
mod transform_tests;
