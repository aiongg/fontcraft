//! Installing module (`.wasm`) and macro (`.fcmacro`) files: dropped on the window, or chosen in File › Open… Each
//! is installed with `plugin.installFromFile`, wherever on the window it lands. Row: `plugin.installFromFile`.
//!
//! [`take`] runs in `logic`, before the frame is drawn, and removes these files from the frame's dropped files, so
//! the views that take dropped images never see them. [`open_as_install`] turns a `file.open` of such a file into
//! the same install; [`install_bytes`] is the install of a file the browser read, which has a name but no path.

use std::path::Path;

use base64::Engine as _;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::{extensions, macros};
use serde_json::{Value, json};

use super::{FontCraftApp, Msg};

/// Most files one drop installs; the rest are left alone.
pub const MAX_DROPPED: usize = 64;

/// Whether the file at `path` is a module or a macro, by its file name.
pub fn is_extension(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(is_extension_name)
}

/// Whether a file named `name` is a module or a macro.
pub fn is_extension_name(name: &str) -> bool {
    extensions::is_module_file(name) || macros::is_macro_file(name)
}

/// The install of the module or macro at the absolute `path`.
pub fn install_path(path: &Path) -> Msg {
    Msg::run("plugin.installFromFile", json!({"path": path.to_string_lossy()}))
}

/// The install of a module or macro the browser read: its file `name` and its bytes. `None` for a file over the module
/// size limit, which the install would refuse anyway.
pub fn install_bytes(name: &str, bytes: &[u8]) -> Option<CommandRef> {
    if bytes.len() > fontcraft_engine::plugins::error::MAX_MODULE_BYTES {
        return None;
    }
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(CommandRef::with_params("plugin.installFromFile", json!({"dataBase64": data, "name": name})))
}

/// The install a `file.open` of a module or macro runs in place of opening it as a font, or `None` for any other
/// command or file. File › Open… offers these files beside the fonts.
pub fn open_as_install(command: &CommandRef) -> Option<Msg> {
    if command.id != "file.open" {
        return None;
    }
    let path = Path::new(command.params.get("path").and_then(Value::as_str)?);
    (path.is_absolute() && is_extension(path)).then(|| install_path(path))
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
        app.sent.push(install_path(path));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::sync::Arc;

    use craft::host::{Host, Picked, ScriptedFiles};
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

    /// File › Open… offers modules and macros beside the fonts, and a macro chosen there is installed as a dropped one
    /// is: picked as a path on the desktop, or read by the browser as a name and bytes.
    #[test]
    fn a_macro_chosen_in_file_open_is_installed_as_a_drop_is() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let data = std::env::temp_dir().join(format!("fontcraft-ui-open-macro-{}-{nanos}", std::process::id()));
        let source = data.join("source");
        std::fs::create_dir_all(&source).unwrap();
        let file = source.join("tighten.fcmacro");
        std::fs::write(&file, MACRO).unwrap();
        let answers = [Picked::Paths(vec![file.clone()]), Picked::Contents(vec![("loose.fcmacro".into(), MACRO.to_vec())])];
        let chooser = Arc::new(ScriptedFiles::answering(answers));
        let host = Host::builder(crate::app::APP).files(chooser.clone()).build();
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(host));
        assert!(ext::open_folders(app.session.extensions_mut(), Some(&data), false).is_empty());
        for _ in 0..2 {
            app.sent.push(Msg::run("file.open", serde_json::json!({})));
            // The first pass asks; the next runs File › Open… with the file chosen.
            app.drain();
            app.drain();
        }
        let requests = chooser.requests();
        assert_eq!(requests.len(), 2);
        let offered: Vec<&String> = requests[0].filters.iter().flat_map(|f| &f.extensions).collect();
        assert!(["fcmacro", "wasm", "glyphs"].iter().all(|e| offered.iter().any(|o| o.as_str() == *e)), "{offered:?}");
        assert!(data.join("Macros").join("tighten.fcmacro").exists(), "{}", app.status);
        assert!(data.join("Macros").join("loose.fcmacro").exists(), "{}", app.status);
        assert_eq!(app.session.documents().len(), 1, "no document opened for a macro");
        let _ = std::fs::remove_dir_all(&data);
    }
}
