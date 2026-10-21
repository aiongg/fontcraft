#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use egui::{Event, Key, Modifiers, Pos2, Rect};
use serde_json::json;

use super::{OPEN_EXTENSIONS, from_menu, opens_on_drop, take_dropped};
use crate::FontcraftApp;
use crate::shell::pickers::Picker;

/// Each question a chooser was asked: its title and its file extensions (none for a folder).
type Asked = Arc<Mutex<Vec<(String, Vec<String>)>>>;

/// A chooser that answers `files` or `folder` and records what it was asked.
struct FakePicker {
    files: Vec<PathBuf>,
    folder: Option<PathBuf>,
    asked: Asked,
}

impl Picker for FakePicker {
    fn folder(&mut self, title: &str) -> Option<PathBuf> {
        self.asked.lock().unwrap().push((title.to_string(), Vec::new()));
        self.folder.clone()
    }

    fn files(&mut self, title: &str, _: &str, extensions: &[&str]) -> Vec<PathBuf> {
        self.asked.lock().unwrap().push((title.to_string(), extensions.iter().map(|e| e.to_string()).collect()));
        self.files.clone()
    }

    fn save(&mut self, _: &str, _: &str, _: &[&str], _: &str) -> Option<PathBuf> {
        None
    }
}

fn answers(app: &mut FontcraftApp, files: Vec<PathBuf>, folder: Option<PathBuf>) -> Asked {
    let asked = Asked::default();
    app.picker = Some(Box::new(FakePicker { files, folder, asked: asked.clone() }));
    asked
}

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

fn input(events: Vec<Event>, dropped: &[PathBuf]) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
        events,
        dropped_files: dropped.iter().map(|p| -> egui::DroppedFileHandle { Arc::new(Dropped(p.clone())) }).collect(),
        ..Default::default()
    }
}

/// A headless window: one egui context kept across frames.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>, dropped: &[PathBuf]) {
        self.ctx.run_ui(input(events, dropped), |ui| app.ui(ui)).textures_delta.clear();
    }

    /// Presses Cmd+O, as File › Open… runs `file.open` with no parameters.
    fn open_shortcut(&self, app: &mut FontcraftApp) {
        self.frame(app, Vec::new(), &[]);
        let press = |pressed| Event::Key { key: Key::O, physical_key: None, pressed, repeat: false, modifiers: Modifiers::COMMAND };
        self.frame(app, vec![Event::ModifiersChanged(Modifiers::COMMAND), press(true), press(false)], &[]);
        self.frame(app, vec![Event::ModifiersChanged(Modifiers::NONE)], &[]);
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-open-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes a new font to `dir/name`, in the format the extension names.
fn saved(dir: &Path, name: &str) -> PathBuf {
    let mut app = FontcraftApp::default();
    let path = dir.join(name);
    app.run("file.saveAs", &json!({"path": path.to_string_lossy()})).unwrap();
    path
}

fn active_path(app: &FontcraftApp) -> Option<PathBuf> {
    app.session.active().and_then(|d| d.path.clone())
}

#[test]
fn open_from_the_menu_offers_every_readable_extension_and_opens_each_chosen_file() {
    let dir = temp_dir("menu");
    let a = saved(&dir, "A.fontcraft");
    let b = saved(&dir, "B.ufo");
    let w = Window::new();
    let mut app = FontcraftApp::default();

    // Cancel opens nothing.
    let asked = answers(&mut app, Vec::new(), None);
    w.open_shortcut(&mut app);
    let offered: Vec<String> = OPEN_EXTENSIONS.iter().map(|e| e.to_string()).collect();
    assert_eq!(*asked.lock().unwrap(), [("Open".to_string(), offered)]);
    assert_eq!(app.session.documents().len(), 1);

    // Each chosen file opens; the last one shows.
    answers(&mut app, vec![a.clone(), b.clone()], None);
    w.open_shortcut(&mut app);
    assert_eq!(app.session.documents().len(), 3, "{}", app.state.status);
    assert_eq!(active_path(&app), Some(b.clone()));

    // A file already open is shown again, and a failure stays in the status bar.
    let missing = dir.join("missing.glyphs");
    answers(&mut app, vec![missing, a.clone()], None);
    w.open_shortcut(&mut app);
    assert_eq!(app.session.documents().len(), 3);
    assert_eq!(active_path(&app), Some(a));
    assert!(app.state.status.contains("missing.glyphs"), "{}", app.state.status);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_without_a_chooser_says_what_to_run() {
    let w = Window::new();
    let mut app = FontcraftApp::default();
    w.open_shortcut(&mut app);
    assert!(app.state.status.contains("file.open with `path`"), "{}", app.state.status);
    assert_eq!(app.session.documents().len(), 1);
    assert!(from_menu(&mut app, "file.openFolder", &json!({})));
    assert!(app.state.status.contains("file.openFolder with `path`"), "{}", app.state.status);
    // A run that names its path is not the chooser's.
    assert!(!from_menu(&mut app, "file.open", &json!({"path": "x.ufo"})));
    assert!(!from_menu(&mut app, "file.save", &json!({})));
}

#[test]
fn open_folder_asks_for_a_folder_and_opens_the_source_in_it() {
    let dir = temp_dir("folder");
    let ufo = saved(&dir, "C.ufo");
    let file = saved(&dir, "D.fontcraft");
    let mut app = FontcraftApp::default();

    let asked = answers(&mut app, Vec::new(), None);
    assert!(from_menu(&mut app, "file.openFolder", &json!({})));
    assert_eq!(*asked.lock().unwrap(), [("Open Folder".to_string(), Vec::new())]);
    assert_eq!(app.session.documents().len(), 1);

    answers(&mut app, Vec::new(), Some(ufo.clone()));
    assert!(from_menu(&mut app, "file.openFolder", &json!({})));
    assert_eq!(active_path(&app), Some(ufo.clone()), "{}", app.state.status);

    // With a path the command runs headless; a file is refused.
    let again = app.run("file.openFolder", &json!({"path": ufo.to_string_lossy()})).unwrap();
    assert_eq!(again["alreadyOpen"], true);
    let err = app.run("file.openFolder", &json!({"path": file.to_string_lossy()})).unwrap_err();
    assert!(err.contains("not a folder"), "{err}");
    assert_eq!(app.session.documents().len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn font_files_dropped_on_the_window_open_and_images_are_left_for_the_image_drop() {
    let dir = temp_dir("drop");
    let ufo = saved(&dir, "E.ufo");
    let native = saved(&dir, "F.FONTCRAFT");
    let png = dir.join("sketch.png");
    let svg = dir.join("outline.svg");

    // The font files are taken out of the frame's drop; the images stay.
    let ctx = egui::Context::default();
    let mut pending = Vec::new();
    let mut left = Vec::new();
    ctx.run_ui(input(Vec::new(), &[png.clone(), ufo.clone(), svg.clone()]), |ui| {
        take_dropped(ui.ctx(), &mut pending);
        left = ui.ctx().input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect::<Vec<_>>());
    })
    .textures_delta
    .clear();
    assert_eq!(pending, [("file.open", json!({"path": ufo.to_string_lossy()}))]);
    assert_eq!(left, [png.clone(), svg]);

    // Dropped on the window, both fonts open; dropped again, the first one shows.
    let w = Window::new();
    let mut app = FontcraftApp::default();
    w.frame(&mut app, Vec::new(), &[]);
    w.frame(&mut app, Vec::new(), &[ufo.clone(), native.clone(), png]);
    assert_eq!(app.session.documents().len(), 3, "{}", app.state.status);
    assert_eq!(active_path(&app), Some(native));
    w.frame(&mut app, Vec::new(), std::slice::from_ref(&ufo));
    assert_eq!(app.session.documents().len(), 3);
    assert_eq!(active_path(&app), Some(ufo));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_drop_opens_files_named_as_fonts() {
    for name in ["A.ufo", "a.UFO", "x.glyphspackage", "f.sfdir", "w.woff2", "p.fontcraftproject", "x.pcf.gz", "y.BDF.GZ", "cmr10.600pk", "cmr10.gf"] {
        assert!(opens_on_drop(name), "{name}");
    }
    for name in ["outline.svg", "sketch.png", "notes.txt", "archive.tar.gz", "README", "x.pk.png", "cmr10.abcpk", "macro.fcmacro"] {
        assert!(!opens_on_drop(name), "{name}");
    }
}
