//! Save: the file changed on disk. Asked when `file.save` refuses because another program
//! changed the document's file since it was read. Overwrite runs the save again with
//! `force: true`, so the person can always save their work; Cancel leaves the file and the
//! document as they are (File › Review External Changes… can then take the other program's
//! changes in). The question names the document it was asked for: when another document is
//! active by the time Overwrite is pressed, the save is refused and the question stays open.

use serde_json::{Map, Value, json};

use super::{Dialog, Fields, Mode};

/// The start of `file.save`'s refusal when the file changed on disk.
const REFUSAL: &str = "changed on disk since it was opened";

pub const DIALOG: Dialog = Dialog {
    mode: Mode::RunOnOk,
    follow_up: true,
    buttons: ["Overwrite", "Cancel"],
    message: "The file changed on disk since it was opened. Overwrite it with this document?",
    ..Dialog::new("dialog.saveChangedOnDisk", "Save: file changed on disk", "file.save", Fields::Fixed(|_| Vec::new()))
};

/// The parameters to open the question with after `file.save` ran with `request` on the
/// document `document` (its session id) and failed with `error`: the request with `force: true`
/// and the document. `None` for any other failure.
pub fn after_error(command: &str, request: &Value, error: &str, document: Option<u64>) -> Option<Map<String, Value>> {
    if command != "file.save" || !error.contains(REFUSAL) {
        return None;
    }
    let mut given = request.as_object().cloned().unwrap_or_default();
    given.insert("force".into(), json!(true));
    if let Some(id) = document {
        given.entry("document").or_insert(json!(id));
    }
    Some(given)
}

#[cfg(test)]
mod tests {
    use fontcraft_engine::Session;
    use serde_json::json;

    use super::DIALOG;
    use crate::{FontcraftApp, dialogs};

    #[test]
    fn overwrite_is_refused_while_another_document_is_active() {
        let dir = std::env::temp_dir().join(format!("fontcraft-save-changed-on-disk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("A.fontcraft");
        let mut app = FontcraftApp::new(Session::with_document());
        app.run("file.saveAs", &json!({"path": path.to_string_lossy()})).unwrap();
        let other = "written by another program";
        std::fs::write(&path, other).unwrap();
        app.run("glyph.newGlyph", &json!({"name": "x.alt"})).unwrap();
        assert!(app.run_from_shell("file.save", &json!({})).is_err());
        assert_eq!(app.dialog.as_ref().map(|d| d.id.as_str()), Some(DIALOG.id), "the question is asked");

        app.run("file.new", &json!({})).unwrap();
        let err = dialogs::confirm(&mut app).unwrap_err();
        assert!(err.contains("not the active document"), "{err}");
        assert!(app.dialog.is_some(), "the question stays open");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), other, "the file is left alone");

        app.run("document.activate", &json!({"index": 0})).unwrap();
        dialogs::confirm(&mut app).unwrap();
        assert_ne!(std::fs::read_to_string(&path).unwrap_or_default(), other, "the asked-about document is written");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
