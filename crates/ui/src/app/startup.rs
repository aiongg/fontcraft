//! What the desktop window sets up when it opens and keeps up while it runs, beside the UI.
//!
//! - [`open_extensions`]: the snippets folder and the extension folders (Modules and Macros) inside the settings
//!   folder, the module switches, and when the session started.
//!
//! The desktop app hands these facts over in `fontcraft_ui::Options`; tests and the browser build have none of them.

use super::FontCraftApp;

/// Where the snippets are kept inside the settings folder.
pub const SNIPPETS_FOLDER: &str = "snippets";

/// Sets the snippets folder and the extension folders inside the settings folder `dir`, notes when the session
/// started, and reads the module switches; `safe_mode` lists the modules but runs none. The folders are read by the
/// first poll. Every problem goes to the log, the first to the status line.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_extensions(app: &mut FontCraftApp, dir: Option<&std::path::Path>, safe_mode: bool) {
    use std::time::{SystemTime, UNIX_EPOCH};

    app.session.set_snippets_dir(dir.map(|d| d.join(SNIPPETS_FOLDER)));
    let started = SystemTime::now().duration_since(UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok());
    app.session.extensions_mut().started_at = started;
    let problems = fontcraft_engine::extensions::open_folders(app.session.extensions_mut(), dir, safe_mode);
    report(app, &problems);
}

/// Logs every problem and puts the first in the status line.
fn report(app: &mut FontCraftApp, problems: &[String]) {
    for problem in problems {
        log::warn!("{problem}");
    }
    if let Some(first) = problems.first() {
        app.status = first.clone();
    }
}
