//! FontCraft's user interface on the craft framework (`docs/ui.md`).
//!
//! | Module | Holds |
//! |---|---|
//! | [`app`] | `FontCraftApp`: the engine session, the host, the command catalog, the message queue and the view models |
//! | [`shell`] | the window: title and menu bar, tool rail, options and status bars, the dock and its `PanelId`s |
//! | [`panels`] | one module per panel: its props, its view function and its words |
//! | [`canvas`] | the document area: the camera, the paint layers and the slot for the active tool |
//! | [`dialogs`] | the dialogs the app opens on the window's dialog host |
//! | [`control`] | the control channel's method table |
#![forbid(unsafe_code)]

pub mod app;
pub mod canvas;
pub mod control;
pub mod dialogs;
pub mod icons;
pub mod panels;
pub mod shell;

pub use app::FontCraftApp;

/// Opens the desktop window and runs it until it closes.
#[cfg(not(target_arch = "wasm32"))]
pub fn run() -> Result<(), String> {
    use craft::behavior::placement::Defaults;
    use craft::ui::shell::app::AppSpec;

    /// The first window's size and its smallest size, in points.
    const WINDOW: [f32; 2] = [1280.0, 800.0];
    const WINDOW_MIN: [f32; 2] = [640.0, 480.0];
    let spec = AppSpec::new("fontcraft", "FontCraft", Defaults::new(WINDOW, WINDOW_MIN));
    craft::ui::shell::app::run(spec, |ctx| FontCraftApp::open(ctx)).map_err(|e| e.to_string())
}
