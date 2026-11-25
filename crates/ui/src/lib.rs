//! FontCraft's user interface on the craft framework (`docs/ui.md`).
//!
//! | Module | Holds |
//! |---|---|
//! | [`app`] | `FontCraftApp`: the engine session, the host, the command catalog, the message queue and the view models |
//! | [`shell`] | the window: title and menu bar, tool rail, options and status bars, the dock and its `PanelId`s |
//! | [`panels`] | one module per panel: its props, its view function and its words |
//! | [`canvas`] | the document area: the camera, the paint layers and the slot for the active tool |
//! | [`dialogs`] | the dialogs the app opens on the window's dialog host |
//! | [`control`] | the control channel: its method table and its listener |
//! | `testing` | (tests) a headless app driven through the control channel |
#![forbid(unsafe_code)]

pub mod app;
pub mod canvas;
pub mod control;
pub mod dialogs;
pub mod icons;
pub mod panels;
pub mod shell;
#[cfg(test)]
pub mod testing;

pub use app::FontCraftApp;
/// The browser and file-manager calls the desktop app hands over in [`Options`].
pub use craft_ext::platform::open::Opener;

/// Builds the app's frame hook once the window's context exists (it wakes the window).
#[cfg(not(target_arch = "wasm32"))]
pub type MakeFrameHook = Box<dyn FnOnce(&egui::Context) -> Box<dyn app::preview_host::FrameHook>>;

/// What the desktop app starts with.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub struct Options {
    /// The control channel's listener (`--control PORT`), bound before the window opens.
    pub control: Option<std::net::TcpListener>,
    /// The folders control requests may read from and write to.
    pub roots: Option<std::sync::Arc<fontcraft_engine::file_access::AutomationRoots>>,
    /// Work done each frame beside the UI (External Preview's server).
    pub frame_hook: Option<MakeFrameHook>,
    /// The web server for the TrueType preview page.
    pub preview_host: Option<Box<dyn app::preview_host::PreviewHost>>,
    /// The system's print service.
    pub print_host: Option<std::sync::Arc<dyn app::print_host::PrintHost>>,
    /// Where the user's settings are kept between runs (the settings folder).
    pub user_store: Option<Box<dyn app::user_store::UserStore>>,
    /// Opens web links and shows files in the file manager (`Opener::system()` on the desktop).
    pub opener: Opener,
    /// Reads the system clipboard's text for Paste outside a text field (`app::clipboard::system_text` on the desktop).
    pub clipboard_reader: Option<app::clipboard::ReadClipboardFn>,
    /// Reads the system clipboard's PDF off the window's thread for a paste on the canvas.
    pub clipboard_flavours: Option<Box<dyn app::clipboard::ClipboardFlavours>>,
    /// Downloads extension indexes, entry files and screenshots over HTTPS for the Extension Manager.
    pub fetch: Option<shell::extension_downloads::FetchFn>,
    /// The settings folder, which holds the window's place, the snippets folder and the extension folders (Modules and
    /// Macros).
    pub settings_dir: Option<std::path::PathBuf>,
    /// Lists the extension modules but runs none (`--safe-mode`).
    pub safe_mode: bool,
    /// The clock behind Last Changed stamps (the system clock on the desktop).
    pub clock: Option<fontcraft_engine::Clock>,
    /// The file or URL named on the command line, opened in place of the Start Window or a new font.
    pub launch: Option<app::startup::Launch>,
}

/// Opens the desktop window where it was last and runs it until it closes.
#[cfg(not(target_arch = "wasm32"))]
pub fn run(options: Options) -> Result<(), String> {
    use std::sync::{Arc, Mutex, PoisonError};

    use craft::behavior::placement::Defaults;
    use craft::ui::shell::app::AppSpec;

    /// The first window's size and its smallest size, in points.
    const WINDOW: [f32; 2] = [1280.0, 800.0];
    const WINDOW_MIN: [f32; 2] = [640.0, 480.0];
    let placement = Arc::new(Mutex::new(window_store(options.settings_dir.as_deref())));
    let spec = AppSpec::new("fontcraft", "FontCraft", Defaults::new(WINDOW, WINDOW_MIN)).placement(Arc::clone(&placement));
    let ran = craft_ext::ui::shell::app::run(spec, move |ctx| open_window(ctx, options)).map_err(|e| e.to_string());
    // The runtime writes the placement once a second; this writes what moved since.
    if let Err(e) = placement.lock().unwrap_or_else(PoisonError::into_inner).save() {
        log::warn!("the window's place was not saved: {e}");
    }
    ran
}

/// Where the window's size, place and maximised state are kept between runs: `window.json` in the settings folder, or
/// memory without a folder. A file that cannot be read is moved aside and the window opens at its first size.
#[cfg(not(target_arch = "wasm32"))]
fn window_store(settings_dir: Option<&std::path::Path>) -> craft::window::restore::Store {
    use craft::window::restore::{FILE_NAME, Store};

    /// Largest window file read.
    const MAX_WINDOW_FILE_BYTES: u64 = 1 << 20;
    let Some(dir) = settings_dir else { return Store::in_memory() };
    let path = dir.join(FILE_NAME);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_WINDOW_FILE_BYTES) {
        log::warn!("{}: larger than {MAX_WINDOW_FILE_BYTES} bytes; the window's place is not kept", path.display());
        return Store::in_memory();
    }
    let store = Store::at(&path);
    if let Some(aside) = store.moved_aside() {
        log::warn!("{}: not a window file; it was kept as {}", path.display(), aside.display());
    }
    store
}

/// The app of the desktop window on `ctx`, set up from `options`: the settings read, the extension folders opened,
/// the first document or the Start Window, and the control channel listening.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_window(ctx: &egui::Context, options: Options) -> FontCraftApp {
    let mut app = FontCraftApp::open(ctx);
    app.session.set_clock(options.clock);
    app.control.roots = options.roots;
    app.frame_hook = options.frame_hook.map(|make| make(ctx));
    app.preview_host = options.preview_host;
    app.print_host = options.print_host;
    app.user_store = options.user_store;
    app::user_store::load(&mut app);
    app::startup::open_extensions(&mut app, options.settings_dir.as_deref(), options.safe_mode);
    app.folder_poll = Some(app::startup::FolderPoll::default());
    app.opener = options.opener;
    app.clipboard_reader = options.clipboard_reader;
    app.clipboard_flavours = options.clipboard_flavours;
    app.view.extensions.downloads.fetch = options.fetch;
    app::startup::launch(&mut app, options.launch.as_ref());
    if let Some(listener) = options.control {
        match app.control.listen(listener, ctx) {
            Ok(addr) => {
                log::info!("control channel listening on {addr}");
                app.session.extensions_mut().surfaces.insert("control".into(), format!("listening on {addr}"));
            }
            Err(e) => {
                log::error!("control channel: {e}");
                app.status = format!("control channel: {e}");
            }
        }
    }
    app
}
