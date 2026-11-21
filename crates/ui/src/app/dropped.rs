//! Module (`.wasm`) and macro (`.fcmacro`) files dropped on the window: each is installed with
//! `plugin.installFromFile`, wherever on the window it lands. Row: `plugin.installFromFile`.
//!
//! [`take`] runs in `logic`, before the frame is drawn, and removes these files from the frame's dropped files, so
//! the views that take dropped images never see them.

use std::path::Path;

use fontcraft_engine::{extensions, macros};
use serde_json::json;

use super::{FontCraftApp, Msg};

/// Most files one drop installs; the rest are left alone.
pub const MAX_DROPPED: usize = 64;

/// Whether the file at `path` is a module or a macro, by its file name.
pub fn is_extension(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|name| extensions::is_module_file(name) || macros::is_macro_file(name))
}

/// Takes the dropped module and macro files out of this frame's input and posts an install for each one that has a
/// path on disk. On the web a dropped file has no path, only a name, and its bytes arrive later, so it is not
/// installed from a drop there.
pub fn take(app: &mut FontCraftApp, ctx: &egui::Context) {
    let paths: Vec<std::path::PathBuf> = ctx.input_mut(|i| {
        if !i.raw.dropped_files.iter().any(|f| is_extension(f.path())) {
            return Vec::new();
        }
        let (ours, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut i.raw.dropped_files).into_iter().partition(|f| is_extension(f.path()));
        i.raw.dropped_files = rest;
        ours.iter().map(|f| f.path().to_path_buf()).collect()
    });
    post(app, &paths);
}

/// Posts `plugin.installFromFile` for each of the first [`MAX_DROPPED`] absolute paths.
pub fn post(app: &mut FontCraftApp, paths: &[std::path::PathBuf]) {
    for path in paths.iter().filter(|p| p.is_absolute()).take(MAX_DROPPED) {
        app.sent.push(Msg::run("plugin.installFromFile", json!({"path": path.to_string_lossy()})));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use craft::host::Host;
    use fontcraft_engine::Session;
    use fontcraft_engine::extensions as ext;

    use super::*;

    const MACRO: &[u8] = br#"{"format": "fontcraft.macro", "version": 1, "title": "Tighten", "description": "Tightens spacing.", "steps": []}"#;

    #[test]
    fn a_macro_dropped_on_the_window_is_installed_and_listed_in_the_script_menu() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let data = std::env::temp_dir().join(format!("fontcraft-ui-next-dropped-{}-{nanos}", std::process::id()));
        let source = data.join("source");
        std::fs::create_dir_all(&source).unwrap();
        let file = source.join("tighten.fcmacro");
        std::fs::write(&file, MACRO).unwrap();
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(crate::app::APP).build()));
        assert!(ext::open_folders(app.session.extensions_mut(), Some(&data), false).is_empty());
        // A text file dropped beside it is left for the views.
        post(&mut app, std::slice::from_ref(&file));
        assert!(!is_extension(Path::new("/tmp/notes.txt")));
        app.drain();
        assert!(data.join("Macros").join("tighten.fcmacro").exists(), "{}", app.status);
        let script = app.catalog.menu.menus.iter().find(|m| m.title == "Script").expect("the Script menu");
        let listed = format!("{:?}", script.children);
        assert!(listed.contains("Tighten") && listed.contains("tighten.fcmacro"), "{listed}");
        let _ = std::fs::remove_dir_all(&data);
    }
}
