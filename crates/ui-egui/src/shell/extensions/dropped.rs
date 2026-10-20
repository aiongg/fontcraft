//! Module (`.wasm`) and macro (`.fcmacro`) files dropped on the window: each is installed with
//! `plugin.installFromFile`, wherever on the window it lands. Row: `plugin.installFromFile`.
//!
//! [`take`] runs before anything is drawn and removes these files from the frame's dropped
//! files, so the canvas and the Font View, which take dropped images, never see them.

use egui::DroppedFileHandle;
use fontcraft_engine::{extensions, macros};
use serde_json::{Value, json};

/// Most files one drop installs; the rest are left alone.
const MAX_DROPPED: usize = 64;

/// Whether `file` is a module or a macro, by its file name.
fn is_extension(file: &DroppedFileHandle) -> bool {
    file.path().file_name().and_then(|n| n.to_str()).is_some_and(|name| extensions::is_module_file(name) || macros::is_macro_file(name))
}

/// Takes the dropped module and macro files out of this frame's input and queues an install for
/// each one that has a path on disk. On the web a dropped file has no path, only a name, and its
/// bytes arrive asynchronously, so it is not installed from a drop there.
pub fn take(ctx: &egui::Context, pending: &mut Vec<(&'static str, Value)>) {
    let files: Vec<DroppedFileHandle> = ctx.input_mut(|i| {
        if !i.raw.dropped_files.iter().any(is_extension) {
            return Vec::new();
        }
        let (ours, rest) = std::mem::take(&mut i.raw.dropped_files).into_iter().partition(is_extension);
        i.raw.dropped_files = rest;
        ours
    });
    for file in files.iter().take(MAX_DROPPED) {
        let path = file.path();
        if path.is_absolute() {
            pending.push(("plugin.installFromFile", json!({"path": path.to_string_lossy()})));
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use egui::{Pos2, Rect};
    use fontcraft_engine::extensions as ext;

    use crate::FontcraftApp;

    const MACRO: &[u8] = br#"{"format": "fontcraft.macro", "version": 1, "title": "Tighten", "description": "Tightens spacing.", "steps": []}"#;

    /// A file dropped on the window, as the desktop integration hands it over.
    #[derive(Debug)]
    struct Dropped(PathBuf);

    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &Path {
            &self.0
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|e| e.to_string())
        }
    }

    #[test]
    fn a_macro_dropped_on_the_window_is_installed_as_install_from_file_installs_it() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let data = std::env::temp_dir().join(format!("fontcraft-ui-dropped-{}-{nanos}", std::process::id()));
        let source = data.join("source");
        std::fs::create_dir_all(&source).unwrap();
        let file = source.join("tighten.fcmacro");
        std::fs::write(&file, MACRO).unwrap();
        let mut app = FontcraftApp::default();
        assert!(ext::open_folders(app.session.extensions_mut(), Some(&data), false).is_empty());
        // Dropped on the Font View, where an image would be placed on a glyph cell.
        let ctx = egui::Context::default();
        let dropped: egui::DroppedFileHandle = Arc::new(Dropped(file));
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))),
            dropped_files: vec![dropped],
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| app.ui(ui));
        assert!(data.join("Macros").join("tighten.fcmacro").exists(), "{}", app.state.status);
        let _ = std::fs::remove_dir_all(&data);
    }
}
