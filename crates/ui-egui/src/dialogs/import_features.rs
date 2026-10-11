//! File › Import › Features… (M8.22): the dialog of `file.importFeatures`, which adds the
//! entries of a feature file after the font's (`dialog.importFeatures`). Import runs the command
//! once, as one undo step; Cancel changes nothing. The file is typed as a path, as in the other
//! import dialogs.

use super::form::{Field, MAX_PATH_TEXT};
use super::{Dialog, Fields, Mode};

pub const DIALOG: Dialog = Dialog {
    mode: Mode::RunOnOk,
    buttons: ["Import", "Cancel"],
    ..Dialog::new("dialog.importFeatures", "Import Features", "file.importFeatures", Fields::Fixed(fields))
};

fn fields(_: &fontcraft_engine::Session) -> Vec<Field> {
    vec![Field::text("file", "File", MAX_PATH_TEXT, "The path of the feature file (.fea) to import")]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};

    use crate::{FontcraftApp, control};

    fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
        let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["result"].clone()
    }

    #[test]
    fn the_import_features_dialog_equals_file_import_features_and_cancel_changes_nothing() {
        let dir = std::env::temp_dir().join(format!("fontcraft-import-features-dialog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("features.fea");
        std::fs::write(&file, "languagesystem DFLT dflt;\n\n@Upper = [A B];\n\nfeature smcp {\n  sub a by A;\n} smcp;\n").unwrap();

        let mut app = FontcraftApp::default();
        let before = app.session.doc().unwrap().font.clone();
        let opened = call(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"command": "file.importFeatures"}}));
        assert_eq!(opened["dialog"], "dialog.importFeatures", "the menu item opens the dialog");
        assert_eq!(opened["captions"]["ok"], "Import");
        call(&mut app, "ui.dialog.set", json!({"field": "file", "value": file}));
        call(&mut app, "ui.click", json!({"id": "cancel"}));
        assert!(Arc::ptr_eq(&app.session.doc().unwrap().font, &before), "Cancel changes nothing");

        call(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"command": "file.importFeatures"}}));
        call(&mut app, "ui.dialog.set", json!({"field": "file", "value": file}));
        let steps = app.session.doc().unwrap().history.undo.len();
        let done = call(&mut app, "ui.dialog.confirm", json!({}));
        assert_eq!(done["result"]["counts"], json!({"prefixes": 1, "classes": 1, "features": 1}));
        assert_eq!(app.session.doc().unwrap().history.undo.len(), steps + 1, "one undo step");

        let mut direct = FontcraftApp::default();
        direct.run("file.importFeatures", &json!({"file": file})).unwrap();
        assert_eq!(app.session.font().unwrap().features, direct.session.font().unwrap().features, "the dialog gives what the command gives");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
