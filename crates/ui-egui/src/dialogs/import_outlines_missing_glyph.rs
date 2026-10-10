//! Import Outlines: no matching glyph. Asked after `file.importOutlines` lists files whose
//! glyph does not exist (`unmatched`): OK runs the import again for those files with
//! `onMissingGlyph: "add"`, which creates each glyph and imports into it; Cancel skips them.
//! [`follow_up`] gives the parameters to open it with from the import's request and result.

use serde_json::{Map, Value, json};

use super::form::{Field, Kind};
use super::{Dialog, Fields};

/// Most files the question lists (the engine imports at most this many in one call).
const MAX_FILES: usize = 4096;

pub const DIALOG: Dialog = Dialog {
    id: "dialog.importOutlinesMissingGlyph",
    title: "Import Outlines: no matching glyph",
    command: "file.importOutlines",
    fields: Fields::Fixed(fields),
};

fn fields(_: &fontcraft_engine::Session) -> Vec<Field> {
    vec![Field {
        key: "onMissingGlyph".into(),
        label: "Files with no glyph".into(),
        kind: Kind::Choice { choices: vec!["add".into(), "skip".into()] },
        default: json!("add"),
        doc: "add creates a glyph named after each file and imports into it; skip leaves the files out".into(),
    }]
}

/// The parameters to open the dialog with after `file.importOutlines` ran with `request` and
/// returned `result`: the unmatched files, with the request's layer and scale, and their glyph
/// names when the request gave `glyphs`. `None` when no file was unmatched.
pub fn follow_up(request: &Value, result: &Value) -> Option<Map<String, Value>> {
    let unmatched: Vec<&str> = result.get("unmatched")?.as_array()?.iter().filter_map(Value::as_str).take(MAX_FILES).collect();
    if unmatched.is_empty() {
        return None;
    }
    let mut m = Map::new();
    if let Some(glyphs) = request.get("glyphs").and_then(Value::as_array) {
        // Each file keeps the glyph at its position in the request.
        let files = request.get("files")?.as_array()?;
        let names = unmatched
            .iter()
            .map(|file| files.iter().position(|f| f.as_str() == Some(*file)).and_then(|i| glyphs.get(i)).cloned())
            .collect::<Option<Vec<Value>>>()?;
        m.insert("glyphs".into(), Value::Array(names));
    }
    m.insert("files".into(), json!(unmatched));
    m.insert("onMissingGlyph".into(), json!("add"));
    for key in ["layer", "scale"] {
        if let Some(v) = request.get(key).filter(|v| !v.is_null()) {
            m.insert(key.into(), v.clone());
        }
    }
    Some(m)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};

    use super::follow_up;
    use crate::{FontcraftApp, control};

    fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
        let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["result"].clone()
    }

    fn font(app: &FontcraftApp) -> Arc<fontcraft_model::Font> {
        app.session.doc().unwrap().font.clone()
    }

    /// An app whose font has no glyph `newglyph`, and the result of importing `newglyph.svg`.
    fn imported(file: &str) -> (FontcraftApp, Value, Value) {
        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        let request = json!({"files": [file]});
        let result = app.run("file.importOutlines", &request).unwrap();
        (app, request, result)
    }

    #[test]
    fn ok_adds_the_missing_glyphs_and_cancel_skips_them() {
        let dir = std::env::temp_dir().join(format!("fontcraft-dialog-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("newglyph.svg");
        let drawing = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000"><rect x="0" y="0" width="100" height="800"/></svg>"#;
        std::fs::write(&path, drawing).unwrap();
        let file = path.to_string_lossy().into_owned();

        let (mut app, request, result) = imported(&file);
        assert_eq!(result["unmatched"], json!([file]));
        let before = font(&app);
        let params = follow_up(&request, &result).unwrap();
        assert_eq!(Value::Object(params.clone()), json!({"files": [file], "onMissingGlyph": "add"}));
        let opened = call(
            &mut app,
            "engine.execute",
            json!({"command": "dialog.open", "params": {"dialog": "dialog.importOutlinesMissingGlyph", "params": params}}),
        );
        assert_eq!(opened["fields"][0]["value"], "add");
        assert!(opened["error"].is_null(), "{opened}");
        call(&mut app, "ui.dialog.confirm", json!({}));
        assert!(!font(&app).same_as(&before), "the glyph was added");

        let (mut direct, _, _) = imported(&file);
        direct.run("file.importOutlines", &json!({"files": [file], "onMissingGlyph": "add"})).unwrap();
        assert!(font(&app).same_as(&font(&direct)), "the dialog and the command give the same font");

        let (mut app, _, _) = imported(&file);
        let original = font(&app);
        call(
            &mut app,
            "engine.execute",
            json!({"command": "dialog.open", "params": {"dialog": "dialog.importOutlinesMissingGlyph", "params": params}}),
        );
        assert!(!font(&app).same_as(&original), "the preview shows the added glyph");
        call(&mut app, "ui.dialog.cancel", json!({}));
        assert!(Arc::ptr_eq(&font(&app), &original), "Cancel puts back the very font the dialog opened on");

        // Explicit glyph names follow their files; nothing unmatched asks nothing.
        let request = json!({"files": ["a.svg", "b.svg"], "glyphs": ["x", "y"], "scale": "em"});
        let asked = follow_up(&request, &json!({"unmatched": ["b.svg"]})).unwrap();
        assert_eq!(Value::Object(asked), json!({"files": ["b.svg"], "glyphs": ["y"], "onMissingGlyph": "add", "scale": "em"}));
        assert!(follow_up(&request, &json!({"unmatched": []})).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
