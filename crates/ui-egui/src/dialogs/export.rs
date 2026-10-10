//! File › Export…: one tab per output kind, each a dialog of its own command, for the active
//! document. Built: the OTF tab (`file.exportStatic`) and the UFO tab (`file.exportUfo`).
//!
//! Exporting writes files, so nothing runs until OK, which reads Next… as in the handbook. Each
//! tab has an export folder: Choose… picks it, and Next… with no folder asks for one
//! (`export.promptWhenNoDestination`); over the control channel a missing folder is an error.

use fontcraft_engine::Session;
use serde_json::json;

use super::form::{Field, Kind, MAX_CHOICES};
use super::{Dialog, Fields, Mode, Tab};

/// The export dialog: opens on the OTF tab.
pub const DIALOG: Dialog = Dialog::new("dialog.export", "Export", "file.export", Fields::Tabs(&TABS));

/// The tabs, by the id `file.export {tab}` takes.
pub const TABS: [Tab; 2] = [Tab { id: "otf", label: "OTF", dialog: OTF }, Tab { id: "ufo", label: "UFO", dialog: UFO }];

/// Static OpenType fonts.
pub const OTF: Dialog = Dialog {
    mode: Mode::RunOnOk,
    buttons: ["Next…", "Cancel"],
    ..Dialog::new("dialog.exportOtf", "Export", "file.exportStatic", Fields::Fixed(otf_fields))
};

/// One UFO per master.
pub const UFO: Dialog = Dialog {
    mode: Mode::RunOnOk,
    buttons: ["Next…", "Cancel"],
    ..Dialog::new("dialog.exportUfo", "Export", "file.exportUfo", Fields::Fixed(ufo_fields))
};

/// The export folder field of every tab.
fn destination() -> Field {
    Field::folder("destination", "Export Folder", "The folder the files are written to; files of the same name are replaced")
}

fn otf_fields(_: &Session) -> Vec<Field> {
    vec![
        Field::choice(
            "outlines",
            "Outline Flavor",
            &[("cff", "PostScript/CFF"), ("truetype", "TrueType")],
            "cff",
            "cff: PostScript outlines in a .otf file; truetype: quadratic outlines in a .ttf file",
        ),
        Field::multi(
            "containers",
            "File Format",
            &[("plain", "OTF/TTF"), ("woff", "WOFF"), ("woff2", "WOFF2")],
            &["plain"],
            "The containers to write each font in: the plain .otf or .ttf file, WOFF, WOFF2",
        ),
        Field::bool("removeOverlap", "Remove Overlap", true, "Merge overlapping paths and components in every exported glyph"),
        Field::bool("autohint", "Autohint", false, "Add automatic hints to the exported fonts"),
        destination(),
    ]
}

fn ufo_fields(session: &Session) -> Vec<Field> {
    let masters: Vec<(String, String)> =
        session.font().map(|f| f.masters.iter().take(MAX_CHOICES).map(|m| (m.id.clone(), m.name.clone())).collect()).unwrap_or_default();
    let ids: Vec<&String> = masters.iter().map(|(id, _)| id).collect();
    vec![
        Field {
            key: "masters".into(),
            label: "Masters".into(),
            kind: Kind::Multi {
                choices: masters.iter().map(|(id, _)| id.clone()).collect(),
                labels: masters.iter().map(|(_, name)| name.clone()).collect(),
            },
            default: json!(ids),
            doc: "The masters to write, each as its own UFO".into(),
        },
        Field::bool("productionNames", "Convert Glyph Names to Production Names", false, "Rename the glyphs to their production names in the UFOs"),
        Field::bool(
            "decomposeSmartStuff",
            "Decompose Smart Stuff",
            true,
            "Draw smart components as outlines; off keeps them as components with their values in the glyph lib",
        ),
        destination(),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};

    use crate::shell::pickers::Picker;
    use crate::{FontcraftApp, control, dialogs};

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
        assert_eq!(done["params"]["removeOverlap"], true);

        let mut direct = drawn();
        direct
            .run(
                "file.exportStatic",
                &json!({"outlines": "truetype", "removeOverlap": true, "autohint": false, "destination": direct_dir, "timestamp": 0}),
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
    /// OK runs the command once, journaled.
    #[test]
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

    /// A folder chooser that answers `folder` and records the titles it was shown with.
    struct FakePicker {
        folder: Option<PathBuf>,
        asked: Arc<Mutex<Vec<String>>>,
    }

    impl Picker for FakePicker {
        fn folder(&mut self, title: &str) -> Option<PathBuf> {
            self.asked.lock().unwrap().push(title.to_string());
            self.folder.clone()
        }

        fn files(&mut self, _: &str, _: &str, _: &[&str]) -> Vec<PathBuf> {
            Vec::new()
        }
    }

    fn picker(app: &mut FontcraftApp, folder: Option<&Path>) -> Arc<Mutex<Vec<String>>> {
        let asked = Arc::new(Mutex::new(Vec::new()));
        app.picker = Some(Box::new(FakePicker { folder: folder.map(Path::to_path_buf), asked: asked.clone() }));
        asked
    }

    #[test]
    fn next_without_a_folder_asks_the_person_and_is_an_error_for_agents() {
        let dir = temp("prompt");
        let mut app = drawn();
        let opened = call(&mut app, "engine.execute", json!({"command": "file.export", "params": {"params": {"timestamp": 0}}}));
        assert_eq!(opened["dialog"], "dialog.exportOtf");
        assert_eq!(opened["captions"]["ok"], "Next…");
        assert_eq!(opened["canChooseFolder"], false, "no chooser: Choose… is hidden");
        assert!(opened["params"].get("destination").is_none(), "an empty folder is no parameter");

        let shown = call(&mut app, "ui.dialog.set", json!({"field": "containers", "value": ["woff", "plain"]}));
        assert_eq!(shown["params"]["containers"], json!(["plain", "woff"]), "the list's order");
        let r = control::handle(&mut app, &json!({"id": 1, "method": "ui.dialog.set", "params": {"field": "containers", "value": ["zip"]}}));
        assert_eq!(r["ok"], false);

        // Over the control channel, and for a person in a window without a chooser, OK is an error.
        let r = control::handle(&mut app, &json!({"id": 1, "method": "ui.dialog.confirm", "params": {}}));
        assert!(r["error"].as_str().is_some_and(|e| e.contains("destination")), "{r}");
        assert!(dialogs::confirm_by(&mut app, true).is_err());
        assert!(call(&mut app, "ui.dialog.inspect", json!({}))["error"].is_string());

        // The person cancels the chooser: nothing runs and the dialog stays.
        let asked = picker(&mut app, None);
        dialogs::confirm_by(&mut app, true).unwrap();
        assert_eq!(*asked.lock().unwrap(), ["Export Folder"]);
        assert_eq!(call(&mut app, "ui.dialog.inspect", json!({}))["open"], true);
        assert!(files(&dir).is_empty());

        // The person picks a folder: the export runs into it.
        picker(&mut app, Some(&dir));
        let done = dialogs::confirm_by(&mut app, true).unwrap();
        assert_eq!(done["params"]["destination"], json!(dir));
        assert_eq!(files(&dir).keys().collect::<Vec<_>>(), ["Untitled-Regular.otf", "Untitled-Regular.woff"]);
        let direct_dir = temp("prompt-direct");
        drawn()
            .run("file.exportStatic", &json!({"outlines": "cff", "containers": ["plain", "woff"], "destination": direct_dir, "timestamp": 0}))
            .unwrap();
        assert_eq!(files(&dir), files(&direct_dir));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&direct_dir);
    }

    #[test]
    fn the_tabs_keep_their_values_and_the_ufo_tab_exports_each_master() {
        let dir = temp("tabs");
        let mut app = drawn();
        let asked = picker(&mut app, Some(&dir));
        let opened = call(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.export"}}));
        assert_eq!(opened["tabs"][0], json!({"id": "dialog.exportOtf", "tab": "otf", "label": "OTF", "active": true}));
        assert_eq!(opened["tabs"][1]["id"], "dialog.exportUfo");
        assert_eq!(opened["canChooseFolder"], true);

        // Choose… (a click on the folder field) asks the chooser.
        let shown = call(&mut app, "ui.click", json!({"id": "destination"}));
        assert_eq!(shown["params"]["destination"], json!(dir));
        assert_eq!(asked.lock().unwrap().len(), 1);
        call(&mut app, "ui.dialog.set", json!({"field": "outlines", "value": "truetype"}));

        let ufo = call(&mut app, "ui.click", json!({"id": "dialog.exportUfo"}));
        assert_eq!((ufo["dialog"].clone(), ufo["command"].clone()), (json!("dialog.exportUfo"), json!("file.exportUfo")));
        assert_eq!(ufo["tabs"][1]["active"], true);
        assert_eq!(ufo["params"]["destination"], json!(dir), "the folder comes along to a tab shown first");
        let masters: Vec<String> = app.session.font().unwrap().masters.iter().map(|m| m.id.clone()).collect();
        assert_eq!(ufo["params"]["masters"], json!(masters));
        assert!(ufo["params"].get("outlines").is_none(), "the OTF tab's fields stay there");

        let otf = call(&mut app, "ui.click", json!({"id": "dialog.exportOtf"}));
        assert_eq!(otf["params"]["outlines"], "truetype", "a tab shown again has its values back");
        call(&mut app, "ui.click", json!({"id": "dialog.exportUfo"}));
        let done = call(&mut app, "ui.dialog.confirm", json!({}));
        let written = done["result"]["files"].as_array().unwrap();
        assert_eq!(written.len(), masters.len());
        assert!(written.iter().all(|f| Path::new(f["path"].as_str().unwrap()).is_dir()), "{written:?}");

        let tab = call(&mut app, "engine.execute", json!({"command": "file.export", "params": {"tab": "ufo"}}));
        assert_eq!(tab["dialog"], "dialog.exportUfo");
        call(&mut app, "ui.dialog.cancel", json!({}));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
