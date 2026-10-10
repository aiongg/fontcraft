//! File › Export…, OTF tab: the options of `file.exportStatic` for the active document.
//!
//! The fields are the outline flavour, Remove Overlap and Autohint. The destination folder is
//! given with `dialog.open`'s `params` (`{destination: "…"}`) and travels as a target; the
//! containers keep the command's default (plain .otf or .ttf).

use fontcraft_engine::Session;
use serde_json::json;

use super::form::{Field, Kind};
use super::{Dialog, Fields};

pub const DIALOG: Dialog = Dialog { id: "dialog.exportOtf", title: "Export", command: "file.exportStatic", fields: Fields::Fixed(fields) };

fn fields(_: &Session) -> Vec<Field> {
    vec![
        Field {
            key: "outlines".into(),
            label: "Outline Flavor".into(),
            kind: Kind::Choice { choices: vec!["cff".into(), "truetype".into()] },
            default: json!("cff"),
            doc: "cff: PostScript outlines in a .otf file; truetype: quadratic outlines in a .ttf file".into(),
        },
        Field::bool("removeOverlap", "Remove Overlap", false, "Merge overlapping paths and components in every exported glyph"),
        Field::bool("autohint", "Autohint", false, "Add automatic hints to the exported fonts"),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use serde_json::{Value, json};

    use crate::{FontcraftApp, control};

    fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
        let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["result"].clone()
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fontcraft-export-dialog-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Every file in `dir` by name, with its bytes.
    fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        let Ok(entries) = std::fs::read_dir(dir) else { return BTreeMap::new() };
        entries.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap())).collect()
    }

    fn drawn() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        app
    }

    fn open(app: &mut FontcraftApp, destination: &Path) -> Value {
        call(
            app,
            "engine.execute",
            json!({"command": "dialog.open", "params": {"dialog": "dialog.exportOtf", "params": {"destination": destination, "timestamp": 0}}}),
        )
    }

    #[test]
    fn the_export_dialog_writes_what_the_command_writes() {
        let (by_dialog, direct_dir) = (temp("dialog"), temp("direct"));
        let mut app = drawn();
        let opened = open(&mut app, &by_dialog);
        assert_eq!(opened["dialog"], "dialog.exportOtf");
        assert_eq!(opened["fields"][0]["id"], "outlines");
        assert_eq!(opened["fields"][0]["value"], "cff");
        assert_eq!(opened["params"]["destination"], json!(by_dialog));

        call(&mut app, "ui.dialog.set", json!({"field": "outlines", "value": "truetype"}));
        let done = call(&mut app, "ui.dialog.confirm", json!({}));
        assert_eq!(done["params"]["outlines"], "truetype");
        assert_eq!(done["params"]["removeOverlap"], false);

        let mut direct = drawn();
        direct
            .run(
                "file.exportStatic",
                &json!({"outlines": "truetype", "removeOverlap": false, "autohint": false, "destination": direct_dir, "timestamp": 0}),
            )
            .unwrap();
        let (wrote, expected) = (files(&by_dialog), files(&direct_dir));
        assert!(expected.contains_key("Untitled-Regular.ttf"), "{:?}", expected.keys());
        for (name, bytes) in &expected {
            assert_eq!(wrote.get(name), Some(bytes), "the dialog wrote {name} as the command does");
        }

        // Cancel leaves the very font the dialog opened on.
        let original = app.session.doc().unwrap().font.clone();
        open(&mut app, &by_dialog);
        call(&mut app, "ui.click", json!({"id": "cancel"}));
        assert!(Arc::ptr_eq(&app.session.doc().unwrap().font, &original));
        let _ = std::fs::remove_dir_all(&by_dialog);
        let _ = std::fs::remove_dir_all(&direct_dir);
    }

    /// Exporting writes files, so the dialog must not preview: nothing is written until OK, and
    /// OK runs the command once, journaled. Needs the run-on-OK dialog mode.
    #[test]
    #[ignore = "needs the run-on-OK dialog mode: the preview writes files"]
    fn the_export_dialog_writes_nothing_before_ok() {
        let dir = temp("ok");
        let mut app = drawn();
        let steps = app.session.doc().unwrap().history.undo.len();
        open(&mut app, &dir);
        call(&mut app, "ui.dialog.set", json!({"field": "outlines", "value": "truetype"}));
        assert!(files(&dir).is_empty(), "opening and editing the dialog writes nothing");
        call(&mut app, "ui.dialog.cancel", json!({}));
        assert!(files(&dir).is_empty(), "Cancel writes nothing");

        open(&mut app, &dir);
        call(&mut app, "ui.dialog.set", json!({"field": "outlines", "value": "truetype"}));
        call(&mut app, "ui.dialog.confirm", json!({}));
        assert_eq!(files(&dir).keys().collect::<Vec<_>>(), ["Untitled-Regular.ttf"], "OK writes the chosen flavour only");
        assert_eq!(app.session.journal().last().map(|(id, _)| id.as_str()), Some("file.exportStatic"));
        assert_eq!(app.session.doc().unwrap().history.undo.len(), steps, "exporting is no undo step");

        // A refused option keeps the dialog open with the message.
        open(&mut app, &dir);
        call(&mut app, "ui.click", json!({"id": "autohint"}));
        let r = control::handle(&mut app, &json!({"id": 1, "method": "ui.dialog.confirm", "params": {}}));
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(call(&mut app, "ui.dialog.inspect", json!({}))["open"], true);
        call(&mut app, "ui.dialog.cancel", json!({}));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
