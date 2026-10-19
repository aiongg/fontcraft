//! The platform's file and folder choosers, for the dialogs, File › Import and the save paths of
//! panels, and Show in Finder.

use std::path::{Path, PathBuf};
use std::process::Command;

use fontcraft_ui_egui::shell::pickers::Picker;

/// The system choosers. Each call blocks the window until the person answers, as a modal
/// chooser does. Where the platform offers none (no desktop portal on Linux), every call
/// answers as a cancel.
pub struct SystemPicker;

impl Picker for SystemPicker {
    fn folder(&mut self, title: &str) -> Option<PathBuf> {
        rfd::FileDialog::new().set_title(title).pick_folder()
    }

    fn files(&mut self, title: &str, kind: &str, extensions: &[&str]) -> Vec<PathBuf> {
        rfd::FileDialog::new().set_title(title).add_filter(kind, extensions).pick_files().unwrap_or_default()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save(&mut self, title: &str, kind: &str, extensions: &[&str], suggested: &str) -> Option<PathBuf> {
        rfd::FileDialog::new().set_title(title).add_filter(kind, extensions).set_file_name(suggested).save_file()
    }

    /// The browser has no blocking save chooser: the path stays typed.
    #[cfg(target_arch = "wasm32")]
    fn save(&mut self, _: &str, _: &str, _: &[&str], _: &str) -> Option<PathBuf> {
        None
    }
}

/// Shows `path` in the system's file manager (Show in Finder): selected in its folder where the
/// platform can do that, else the folder that holds it.
pub fn reveal(path: &Path) -> Result<(), String> {
    let mut command = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg("-R").arg(path);
        c
    } else if cfg!(target_os = "windows") {
        let mut select = std::ffi::OsString::from("/select,");
        select.push(path);
        let mut c = Command::new("explorer");
        c.arg(select);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(path.parent().unwrap_or(path));
        c
    };
    let mut child = command.spawn().map_err(|e| format!("cannot start the file manager: {e}"))?;
    // Wait on another thread, so the finished process does not linger.
    let _ = std::thread::spawn(move || child.wait());
    Ok(())
}
