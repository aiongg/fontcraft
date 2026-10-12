//! The system's file choosers. The desktop app gives [`FontcraftApp::picker`] one; without it (the
//! browser, MCP, tests) folder fields hide Choose…, OK with no folder is an error, and File ›
//! Import › Outlines… says that `file.importOutlines` needs `files`.

use std::path::PathBuf;

use serde_json::{Value, json};

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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::{Picker, import_outlines, wants_files};
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
}
