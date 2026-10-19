//! The system's file choosers. The desktop app gives [`FontcraftApp::picker`] one; without it (the
//! browser, MCP, tests) folder fields and save paths hide Choose… and stay typed, OK with no
//! folder is an error, File › Import › Outlines… says that `file.importOutlines` needs `files`,
//! and File › Save As… says that `file.saveAs` needs a `path`.

use std::path::PathBuf;

use fontcraft_model::SourceFormat;
use serde_json::{Map, Value, json};

use crate::app::FontcraftApp;

/// The command File › Import › Outlines… runs.
const IMPORT_OUTLINES: &str = "file.importOutlines";

/// A chooser the platform shows. Each call waits until the person answers.
pub trait Picker: Send {
    /// The folder the person chose; `None` when they cancelled.
    fn folder(&mut self, title: &str) -> Option<PathBuf>;
    /// The files the person chose (several allowed) among those whose extension is one of
    /// `extensions`, described as `kind`; empty when they cancelled.
    fn files(&mut self, title: &str, kind: &str, extensions: &[&str]) -> Vec<PathBuf>;
    /// The file the person chose to write, offered with the name `suggested` among files whose
    /// extension is one of `extensions`, described as `kind`; `None` when they cancelled. The
    /// chooser asks before it answers with a file that exists.
    fn save(&mut self, title: &str, kind: &str, extensions: &[&str], suggested: &str) -> Option<PathBuf>;
}

/// The chosen `path`, with `.{default}` added when its extension is none of `extensions`: some
/// choosers return the name as typed. `Err` says why nothing should run: the name with the
/// extension added is a file that exists, and the chooser asked only about the name without it.
pub fn with_extension(path: PathBuf, extensions: &[&str], default: &str) -> Result<PathBuf, String> {
    if path.extension().and_then(|e| e.to_str()).is_some_and(|e| extensions.contains(&e)) {
        return Ok(path);
    }
    let mut name = path.into_os_string();
    name.push(".");
    name.push(default);
    let path = PathBuf::from(name);
    if path.exists() {
        return Err(format!("{} exists; choose it in the chooser to replace it", path.display()));
    }
    Ok(path)
}

/// The commands File › Save and File › Save As… run.
const SAVE: &str = "file.save";
const SAVE_AS: &str = "file.saveAs";

/// The extensions the save commands read as a format (the `format` names of `file.saveAs`).
const SAVE_EXTENSIONS: [&str; 5] = ["fontcraft", "glyphs", "glyphspackage", "ufo", "designspace"];

/// The extension of a new document's format, and of the format a document Save cannot write back
/// to is saved in.
const NATIVE_EXTENSION: &str = "fontcraft";

/// The extension of the format a document read from `source` is saved in; `None` for a source
/// Save cannot write back to (a compiled font, a zipped UFO).
fn save_extension(source: SourceFormat) -> Option<&'static str> {
    match source {
        SourceFormat::Native => Some(NATIVE_EXTENSION),
        SourceFormat::Glyphs => Some("glyphs"),
        SourceFormat::GlyphsPackage => Some("glyphspackage"),
        SourceFormat::Ufo => Some("ufo"),
        SourceFormat::Designspace => Some("designspace"),
        SourceFormat::Ufoz | SourceFormat::Binary => None,
    }
}

/// A run of `id` with `params` that asks the save chooser for its file first: Save As… without a
/// `path`, and Save without one of a document that has no file it can write back to (never saved,
/// or opened from a compiled font). Without a chooser the command runs as given and says what it
/// needs.
pub fn wants_save_path(app: &FontcraftApp, id: &str, params: &Value) -> bool {
    if app.picker.is_none() || params.get("path").is_some_and(|p| !p.is_null()) {
        return false;
    }
    let Some(doc) = app.session.active() else { return false };
    match id {
        SAVE_AS => true,
        SAVE => doc.path.is_none() || save_extension(doc.font.source.format).is_none(),
        _ => false,
    }
}

/// File › Save As…, and Save of a document without a file: asks the save chooser for the file,
/// offering the document's title with the extension of its format, and runs `id` with the chosen
/// path as the person's save. A chosen name without a format extension gets the offered one. A
/// cancelled chooser saves nothing.
pub fn save_with_chooser(app: &mut FontcraftApp, id: &str, params: Value) {
    let Some(doc) = app.session.active() else { return };
    let extension = save_extension(doc.font.source.format).unwrap_or(NATIVE_EXTENSION);
    let suggested = format!("{}.{extension}", doc.title);
    let title = if id == SAVE_AS { "Save As" } else { "Save" };
    let Some(picker) = app.picker.as_mut() else { return };
    let Some(path) = picker.save(title, "Font source", &SAVE_EXTENSIONS, &suggested) else { return };
    let path = match with_extension(path, &SAVE_EXTENSIONS, extension) {
        Ok(path) => path,
        Err(why) => {
            app.state.status = format!("{title}: {why}");
            return;
        }
    };
    let mut params = match params {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    params.insert("path".into(), json!(path.to_string_lossy()));
    // A failure is in the status bar already.
    let _ = app.run_from_shell(id, &Value::Object(params));
}

/// A run of `id` with `params` that asks for files first: File › Import › Outlines… names none.
pub fn wants_files(id: &str, params: &Value) -> bool {
    id == IMPORT_OUTLINES && params.get("files").is_none_or(Value::is_null)
}

/// File › Import › Outlines…: asks for SVG files and imports them as the person's run, so the
/// question about files without a glyph of their name can open.
pub fn import_outlines(app: &mut FontcraftApp) {
    let Some(picker) = app.picker.as_mut() else {
        app.state.status = "Import Outlines: this window has no file chooser; run file.importOutlines with `files`".into();
        return;
    };
    let files = picker.files("Import Outlines", "SVG or EPS", &["svg", "eps"]);
    if files.is_empty() {
        return;
    }
    let files: Vec<String> = files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    // A failure is in the status bar already.
    let _ = app.run_from_shell(IMPORT_OUTLINES, &json!({"files": files}));
}

/// A save chooser for tests: answers `path` and records each question.
#[cfg(test)]
pub(crate) mod stub {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use super::Picker;

    /// What a save chooser was asked: its title, its extensions and the name it offered.
    pub type SaveAsked = Arc<Mutex<Vec<(String, Vec<String>, String)>>>;

    pub struct SavePicker {
        pub path: Option<PathBuf>,
        pub asked: SaveAsked,
    }

    impl Picker for SavePicker {
        fn folder(&mut self, _: &str) -> Option<PathBuf> {
            None
        }

        fn files(&mut self, _: &str, _: &str, _: &[&str]) -> Vec<PathBuf> {
            Vec::new()
        }

        fn save(&mut self, title: &str, _: &str, extensions: &[&str], suggested: &str) -> Option<PathBuf> {
            self.asked.lock().unwrap().push((title.into(), extensions.iter().map(|e| e.to_string()).collect(), suggested.into()));
            self.path.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use egui::{Event, Key, Modifiers, PointerButton, Pos2};

    use super::stub::{SaveAsked, SavePicker};
    use super::{Picker, SAVE_EXTENSIONS, import_outlines, wants_files, with_extension};
    use crate::FontcraftApp;

    /// Each question a chooser was asked: its title and its file extensions.
    type Asked = Arc<Mutex<Vec<(String, Vec<String>)>>>;

    /// A chooser that answers `files` and records what it was asked.
    struct FakePicker {
        files: Vec<PathBuf>,
        asked: Asked,
    }

    impl Picker for FakePicker {
        fn folder(&mut self, _: &str) -> Option<PathBuf> {
            None
        }

        fn files(&mut self, title: &str, _: &str, extensions: &[&str]) -> Vec<PathBuf> {
            self.asked.lock().unwrap().push((title.to_string(), extensions.iter().map(|e| e.to_string()).collect()));
            self.files.clone()
        }

        fn save(&mut self, _: &str, _: &str, _: &[&str], _: &str) -> Option<PathBuf> {
            None
        }
    }

    #[test]
    fn import_outlines_from_the_menu_asks_for_svg_files_and_runs_as_the_persons_import() {
        assert!(wants_files("file.importOutlines", &json!({})));
        assert!(!wants_files("file.importOutlines", &json!({"files": ["a.svg"]})));
        assert!(!wants_files("edit.paste", &json!({})));

        let dir = std::env::temp_dir().join(format!("fontcraft-import-picker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("newglyph.svg");
        std::fs::write(
            &file,
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000"><rect x="0" y="0" width="100" height="800"/></svg>"#,
        )
        .unwrap();

        // Without a chooser nothing runs, and the status bar says why.
        let mut app = FontcraftApp::default();
        let before = app.session.font().unwrap().clone();
        import_outlines(&mut app);
        assert!(app.state.status.contains("file.importOutlines"), "{}", app.state.status);
        assert!(app.session.font().unwrap().same_as(&before));

        // A cancelled chooser imports nothing.
        let asked = Arc::new(Mutex::new(Vec::new()));
        app.picker = Some(Box::new(FakePicker { files: Vec::new(), asked: asked.clone() }));
        import_outlines(&mut app);
        assert_eq!(*asked.lock().unwrap(), [("Import Outlines".to_string(), vec!["svg".to_string(), "eps".to_string()])]);
        assert!(app.dialog.is_none());

        // The chosen file has no glyph of its name: the person's import asks about it.
        app.picker = Some(Box::new(FakePicker { files: vec![file.clone()], asked }));
        import_outlines(&mut app);
        let shown = crate::dialogs::inspect(&app);
        assert_eq!(shown["dialog"], "dialog.importOutlinesMissingGlyph");
        assert_eq!(shown["params"]["files"], json!([file.to_string_lossy()]));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.session.font().unwrap().glyph("newglyph").is_some());
        let _ = std::fs::remove_dir_all(Path::new(&dir));
    }

    /// A headless window: one egui context kept across frames.
    struct Window {
        ctx: egui::Context,
    }

    impl Window {
        fn new() -> Self {
            Self { ctx: egui::Context::default() }
        }

        fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) {
            let input =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 1000.0))), events, ..Default::default() };
            let mut out = self.ctx.run_ui(input, |ui| app.ui(ui));
            out.textures_delta.clear();
        }

        /// Presses a menu shortcut, as the menu item runs its command with no parameters.
        fn shortcut(&self, app: &mut FontcraftApp, key: Key, modifiers: Modifiers) {
            self.frame(app, Vec::new());
            let press = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
            self.frame(app, vec![Event::ModifiersChanged(modifiers), press(true), press(false)]);
            self.frame(app, vec![Event::ModifiersChanged(Modifiers::NONE)]);
        }

        fn click(&self, app: &mut FontcraftApp, id: egui::Id) {
            self.frame(app, Vec::new());
            self.frame(app, Vec::new());
            let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
            let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
            self.frame(app, vec![Event::PointerMoved(at)]);
            self.frame(app, vec![button(true)]);
            self.frame(app, vec![button(false)]);
            self.frame(app, Vec::new());
        }
    }

    const SAVE_AS_KEYS: Modifiers = Modifiers { shift: true, ..Modifiers::COMMAND };

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fontcraft-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn asked_by(app: &mut FontcraftApp, path: Option<PathBuf>) -> SaveAsked {
        let asked = SaveAsked::default();
        app.picker = Some(Box::new(SavePicker { path, asked: asked.clone() }));
        asked
    }

    #[test]
    fn file_save_as_from_the_menu_asks_the_save_chooser_and_writes_the_chosen_ufo() {
        // The chooser offers exactly the extensions the engine saves.
        for extension in SAVE_EXTENSIONS {
            assert!(fontcraft_engine::io::SaveFormat::parse(extension).is_some(), "{extension}");
        }
        let dir = temp_dir("save-as-chooser");
        let w = Window::new();
        let mut app = FontcraftApp::default();
        let title = app.session.active().unwrap().title.clone();

        // Cancel writes nothing and leaves the document without a file.
        let asked = asked_by(&mut app, None);
        w.shortcut(&mut app, Key::S, SAVE_AS_KEYS);
        let offered = ("Save As".to_string(), SAVE_EXTENSIONS.map(String::from).to_vec(), format!("{title}.fontcraft"));
        assert_eq!(*asked.lock().unwrap(), [offered]);
        assert!(app.session.active().unwrap().path.is_none());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);

        // The chosen X.ufo is written as a UFO and becomes the document's file.
        let file = dir.join("X.ufo");
        let asked = asked_by(&mut app, Some(file.clone()));
        w.shortcut(&mut app, Key::S, SAVE_AS_KEYS);
        assert_eq!(asked.lock().unwrap().len(), 1);
        assert!(file.join("metainfo.plist").is_file(), "{}", app.state.status);
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some(file.as_path()));

        // Save As again offers the document's new title and format.
        let asked = asked_by(&mut app, None);
        w.shortcut(&mut app, Key::S, SAVE_AS_KEYS);
        assert_eq!(asked.lock().unwrap()[0].2, "X.ufo");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_save_of_an_untitled_document_asks_the_save_chooser_once() {
        let dir = temp_dir("save-untitled-chooser");
        let w = Window::new();
        let mut app = FontcraftApp::default();
        let file = dir.join("Mine.fontcraft");
        let asked = asked_by(&mut app, Some(file.clone()));
        w.shortcut(&mut app, Key::S, Modifiers::COMMAND);
        assert_eq!(asked.lock().unwrap().len(), 1);
        assert_eq!(asked.lock().unwrap()[0].0, "Save");
        assert!(file.is_file(), "{}", app.state.status);
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some(file.as_path()));

        // A document with a file saves to it without asking.
        app.run("glyph.newGlyph", &json!({"name": "A.alt"})).unwrap();
        w.shortcut(&mut app, Key::S, Modifiers::COMMAND);
        assert_eq!(asked.lock().unwrap().len(), 1);
        assert!(!app.session.active().unwrap().modified(), "{}", app.state.status);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_chosen_name_without_an_extension_gets_the_offered_one() {
        let dir = temp_dir("save-no-extension");
        let w = Window::new();
        let mut app = FontcraftApp::default();
        // File › Save of an untitled document, answered with a bare name: a native file.
        let asked = asked_by(&mut app, Some(dir.join("Mine")));
        w.shortcut(&mut app, Key::S, Modifiers::COMMAND);
        assert_eq!(asked.lock().unwrap().len(), 1);
        let file = dir.join("Mine.fontcraft");
        assert!(file.is_file(), "{}", app.state.status);
        assert!(!dir.join("Mine").exists());
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some(file.as_path()));

        // A name that ends in another extension keeps it and gets the offered one after it; a
        // name with an accepted extension stays as it is.
        assert_eq!(with_extension(dir.join("a.v2"), &SAVE_EXTENSIONS, "ufo").unwrap(), dir.join("a.v2.ufo"));
        assert_eq!(with_extension(dir.join("a.glyphs"), &SAVE_EXTENSIONS, "ufo").unwrap(), dir.join("a.glyphs"));
        // The name with the extension added is a file the chooser did not ask about: refused.
        assert!(with_extension(dir.join("Mine"), &SAVE_EXTENSIONS, "fontcraft").unwrap_err().contains("exists"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_save_as_without_a_chooser_keeps_the_commands_error() {
        let w = Window::new();
        let mut app = FontcraftApp::default();
        w.shortcut(&mut app, Key::S, SAVE_AS_KEYS);
        assert!(app.state.status.contains("`path`"), "{}", app.state.status);
        w.shortcut(&mut app, Key::S, Modifiers::COMMAND);
        assert!(app.state.status.contains("never been saved"), "{}", app.state.status);
        assert!(app.session.active().unwrap().path.is_none());
    }

    #[test]
    fn actions_window_save_as_asks_for_a_macro_file_and_saves_the_set_there() {
        let dir = temp_dir("actions-save-as");
        let file = dir.join("tidy.fcmacro");
        let w = Window::new();
        let mut app = FontcraftApp::default();
        app.run("dialog.actions", &json!({"open": true, "command": "path.removeOverlap", "add": true})).unwrap();
        app.run("dialog.actions", &json!({"title": "Tidy"})).unwrap();
        let save_as = crate::panels::widgets::id(("actions", "saveAs"));

        // Without a chooser the set is saved by name or typed path: no Save As….
        w.frame(&mut app, Vec::new());
        w.frame(&mut app, Vec::new());
        assert!(w.ctx.read_response(save_as).is_none());

        let asked = asked_by(&mut app, None);
        w.click(&mut app, save_as);
        assert_eq!(*asked.lock().unwrap(), [("Save Action Set".to_string(), vec!["fcmacro".to_string()], "Tidy.fcmacro".to_string())]);
        assert!(!file.exists());

        let asked = asked_by(&mut app, Some(file.clone()));
        w.click(&mut app, save_as);
        assert_eq!(asked.lock().unwrap().len(), 1);
        assert!(file.is_file(), "{}", app.state.status);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
