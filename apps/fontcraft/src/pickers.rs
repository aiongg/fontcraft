//! The platform's file and folder choosers, for the dialogs and File › Import.

use std::path::PathBuf;

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
}
