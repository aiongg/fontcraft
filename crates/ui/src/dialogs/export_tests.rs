//! The export dialogs as an agent drives them (`ui.dialog.*`) and as the person does (OK asking the chooser).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use craft::host::{Host, PickKind, Picked, ScriptedFiles};
use fontcraft_engine::Session;
use serde_json::{Value, json};

use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::control;
use crate::dialogs::form;

fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
    control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
}

fn fails(app: &mut FontCraftApp, method: &str, params: Value) -> String {
    control::dispatch(app, method, &params).expect_err(&format!("{method} should fail"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-ui-export-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Every file in `dir` by name, with its bytes.
fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let Ok(entries) = std::fs::read_dir(dir) else { return BTreeMap::new() };
    entries.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap())).collect()
}

/// Glyph A with a square, shown in the glyph panels; the host's chooser answers with `answers`.
fn drawn_with(answers: Vec<Picked>) -> (FontCraftApp, Arc<ScriptedFiles>) {
    let chooser = Arc::new(ScriptedFiles::answering(answers));
    let host = Host::builder(APP).files(chooser.clone()).build();
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(host));
    app.session.execute("glyph.newGlyph", &json!({"name": "A"})).ok();
    app.session.execute("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    (app, chooser)
}

fn drawn() -> FontCraftApp {
    drawn_with(Vec::new()).0
}

fn export(app: &mut FontCraftApp, params: Value) -> Value {
    call(app, "engine.execute", json!({"command": "file.export", "params": params}));
    call(app, "ui.dialog.inspect", json!({}))
}

#[test]
fn the_otf_tab_writes_what_the_command_writes_and_nothing_before_ok() {
    let (by_dialog, direct_dir) = (temp("dialog"), temp("direct"));
    let mut app = drawn();
    let steps = app.session.doc().unwrap().history.undo.len();
    let opened = export(&mut app, json!({"params": {"destination": by_dialog, "timestamp": 0}}));
    assert_eq!(opened["dialog"], "dialog.exportOtf");
    assert_eq!(opened["captions"]["ok"], "Next…");
    assert_eq!(opened["params"]["outlines"], "cff");
    assert_eq!(opened["params"]["autohint"], false, "hinting is optional: off until ticked");
    assert_eq!(opened["params"]["cbdt"], false);

    call(&mut app, "ui.dialog.set", json!({"field": "outlines", "value": "truetype"}));
    assert!(files(&by_dialog).is_empty(), "editing the dialog writes nothing");
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert!(files(&by_dialog).is_empty(), "Cancel writes nothing");

    export(&mut app, json!({"params": {"destination": by_dialog, "timestamp": 0}}));
    call(&mut app, "ui.dialog.set", json!({"field": "outlines", "value": "truetype"}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(done["params"]["removeOverlap"], true);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), steps, "exporting is no undo step");

    let mut direct = drawn();
    direct
        .session
        .execute(
            "file.exportStatic",
            &json!({"outlines": "truetype", "removeOverlap": true, "autohint": false, "destination": direct_dir, "timestamp": 0}),
        )
        .unwrap();
    let (wrote, expected) = (files(&by_dialog), files(&direct_dir));
    assert_eq!(wrote.keys().collect::<Vec<_>>(), ["Untitled-Regular.ttf"]);
    assert_eq!(wrote, expected, "the dialog wrote what the command wrote");

    // A refused option keeps the dialog open with the message.
    export(&mut app, json!({"params": {"destination": by_dialog}}));
    call(&mut app, "ui.dialog.set", json!({"field": "containers", "value": []}));
    fails(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(call(&mut app, "ui.dialog.inspect", json!({}))["open"], true);
    call(&mut app, "ui.dialog.cancel", json!({}));
    let _ = std::fs::remove_dir_all(&by_dialog);
    let _ = std::fs::remove_dir_all(&direct_dir);
}

#[test]
fn next_without_a_folder_asks_the_person_and_is_an_error_for_agents() {
    let dir = temp("prompt");
    let (mut app, chooser) = drawn_with(vec![Picked::Cancelled, Picked::Paths(vec![dir.clone()])]);
    let opened = export(&mut app, json!({"params": {"timestamp": 0}}));
    assert!(opened["params"].get("destination").is_none(), "an empty folder is no parameter");
    call(&mut app, "ui.dialog.set", json!({"field": "containers", "value": ["woff", "plain"]}));

    let e = fails(&mut app, "ui.dialog.confirm", json!({}));
    assert!(e.contains("destination"), "{e}");

    // The person cancels the chooser: nothing runs and the dialog stays.
    form::confirm_by_person(&mut app).unwrap();
    form::poll_chooser(&mut app);
    assert_eq!(chooser.requests().first().map(|r| r.kind), Some(PickKind::Folder));
    assert_eq!(call(&mut app, "ui.dialog.inspect", json!({}))["open"], true);
    assert!(files(&dir).is_empty());

    // The person picks a folder: the export runs into it.
    form::confirm_by_person(&mut app).unwrap();
    form::poll_chooser(&mut app);
    assert_eq!(call(&mut app, "ui.dialog.inspect", json!({}))["open"], false);
    assert_eq!(files(&dir).keys().collect::<Vec<_>>(), ["Untitled-Regular.otf", "Untitled-Regular.woff"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_tabs_keep_their_values_and_the_ufo_tab_writes_production_names() {
    let dir = temp("tabs");
    let mut app = drawn();
    app.session.execute("glyph.newGlyph", &json!({"name": "A-cy"})).ok();
    let opened = export(&mut app, json!({"params": {"destination": dir}}));
    assert_eq!(opened["tabs"][0], json!({"id": "dialog.exportOtf", "tab": "otf", "label": "OTF", "active": true}));
    call(&mut app, "ui.dialog.set", json!({"field": "outlines", "value": "truetype"}));

    let ufo = call(&mut app, "ui.click", json!({"id": "dialog.exportUfo"}));
    assert_eq!(ufo["command"], "file.exportUfo");
    assert_eq!(ufo["params"]["destination"], json!(dir), "the folder comes along to a tab shown first");
    assert_eq!(ufo["params"]["productionNames"], false, "production names are off until ticked");
    assert!(ufo["params"].get("outlines").is_none(), "the OTF tab's fields stay there");

    let otf = call(&mut app, "ui.click", json!({"id": "dialog.exportOtf"}));
    assert_eq!(otf["params"]["outlines"], "truetype", "a tab shown again has its values back");
    call(&mut app, "ui.click", json!({"id": "dialog.exportUfo"}));
    call(&mut app, "ui.click", json!({"id": "productionNames"}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(done["params"]["productionNames"], true);
    let written = done["result"]["files"].as_array().unwrap();
    assert_eq!(written.len(), app.session.font().unwrap().masters.len());

    let direct_dir = temp("tabs-direct");
    let mut direct = drawn();
    direct.session.execute("glyph.newGlyph", &json!({"name": "A-cy"})).ok();
    let params = json!({"destination": direct_dir, "masters": done["params"]["masters"], "productionNames": true, "decomposeSmartStuff": true});
    direct.session.execute("file.exportUfo", &params).unwrap();
    let ufo_dir = |root: &Path| std::fs::read_dir(root).unwrap().flatten().next().unwrap().path().join("glyphs");
    assert_eq!(files(&ufo_dir(&dir)), files(&ufo_dir(&direct_dir)), "the dialog wrote the glyph files the command wrote");
    assert!(files(&ufo_dir(&dir)).keys().any(|name| name.starts_with("uni0410")), "{:?}", files(&ufo_dir(&dir)).keys());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&direct_dir);
}

#[test]
fn window_contents_opens_on_the_edited_glyph_and_asks_where_to_save() {
    let dir = temp("window");
    std::fs::create_dir_all(&dir).unwrap();
    let svg = dir.join("A.svg");
    let (mut app, chooser) = drawn_with(vec![Picked::Paths(vec![svg.clone()])]);

    // The menu item fires with no parameters: the dialog opens on what the canvas shows.
    assert!(crate::dialogs::after::fired(&mut app, "file.exportWindowContents", &Value::Null, &serde_json::Map::new()));
    let opened = call(&mut app, "ui.dialog.inspect", json!({}));
    assert_eq!(opened["dialog"], "dialog.exportWindowContents");
    assert_eq!(opened["captions"]["ok"], "Export…");
    assert_eq!(opened["fields"][0]["kind"], "saveFile");
    assert_eq!(opened["params"]["text"], "/A");
    let master = app.session.font().unwrap().masters[0].id.clone();
    assert_eq!(opened["params"]["master"], json!(master));
    assert!(fails(&mut app, "ui.dialog.confirm", json!({})).contains("path"), "agents give the path");

    form::confirm_by_person(&mut app).unwrap();
    form::poll_chooser(&mut app);
    let asked = chooser.requests();
    assert_eq!(asked.first().map(|r| (r.kind, r.suggested_extension.clone())), Some((PickKind::Save, Some("pdf".into()))));
    assert_eq!(call(&mut app, "ui.dialog.inspect", json!({}))["open"], false, "the export ran");
    let written = std::fs::read(&svg).unwrap();

    let direct = dir.join("direct.svg");
    drawn().session.execute("file.exportWindowContents", &json!({"text": "/A", "format": "svg", "master": master, "path": direct})).unwrap();
    assert_eq!(written, std::fs::read(&direct).unwrap(), "the .svg extension chose SVG");

    // A format given in the command's shape names the extension of a path without one.
    let opened =
        form::open_command(&mut app, &json!({"dialog": "dialog.exportWindowContents", "params": {"path": dir.join("page"), "format": "svg"}}))
            .unwrap();
    assert_eq!(opened["params"]["format"], "svg");
    assert_eq!(opened["params"]["path"], json!(dir.join("page.svg")));
    call(&mut app, "ui.dialog.cancel", json!({}));
    let _ = std::fs::remove_dir_all(&dir);
}
