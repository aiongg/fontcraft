#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use craft::host::{Host, Picked, ScriptedFiles};
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_model::{Font, Node, NodeKind, Path as Outline, Shape};
use serde_json::{Value, json};

use super::state;
use crate::app::{APP, FontCraftApp, Msg, close, open_report};
use crate::testing::TestApp;

fn temp(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("fontcraft-ui-{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn lines(closed: bool, points: &[(f64, f64)]) -> Shape {
    Shape::Path(Outline::new(closed, points.iter().map(|&(x, y)| Node::new(x, y, NodeKind::Line)).collect()))
}

/// A new document whose `I` has a corner component.
fn with_corner(files: Vec<Picked>) -> FontCraftApp {
    let mut font = Font::new("Losses");
    font.add_glyph("_corner.one", Vec::new(), 600.0).unwrap();
    font.glyph_mut("_corner.one").unwrap().layer_mut("m01").unwrap().shapes =
        vec![lines(false, &[(0.0, 50.0), (-50.0, 50.0), (-50.0, 0.0), (25.0, 0.0)])];
    font.glyph_mut("I").unwrap().layer_mut("m01").unwrap().shapes = vec![lines(true, &[(100.0, 0.0), (400.0, 0.0), (400.0, 500.0), (100.0, 500.0)])];
    let mut s = Session::new();
    s.add_document(font, None).unwrap();
    s.execute("cornerComponent.add", &json!({"glyph": "I", "points": [[0, 0]], "corner": "_corner.one"})).unwrap();
    let host = Host::builder(APP).files(Arc::new(ScriptedFiles::answering(files))).build();
    FontCraftApp::with_edit_tab(s, Arc::new(host))
}

fn post(app: &mut FontCraftApp, id: &str, params: Value) {
    app.sent.push(Msg::run(id, params));
    app.drain();
}

fn answer(app: &mut FontCraftApp, params: Value) -> Result<Value, String> {
    app.run_now(Msg::Shell(CommandRef::with_params("window.fileQuestion", params)), false)
}

/// A person's Save As to a UFO that cannot hold corner components asks first; Cancel leaves the disk untouched, Save
/// Anyway writes, the next Save asks nothing, and a request that names the command never asks.
#[test]
fn a_lossy_save_asks_before_it_writes() {
    let dir = temp("loss-question");
    let mut app = with_corner(Vec::new());
    let ufo = dir.join("Font.ufo");
    post(&mut app, "file.saveAs", json!({"path": ufo}));
    let asked = state(&app);
    assert_eq!((&asked["kind"], &asked["format"]), (&json!("losses"), &json!("ufo")), "{asked}");
    let corner = asked["losses"].as_array().unwrap().iter().find(|l| l["what"].as_str().unwrap().starts_with("corner components")).unwrap().clone();
    assert_eq!(corner["details"], "I", "{asked}");
    assert!(!ufo.exists(), "nothing is written before the answer");
    answer(&mut app, json!({"answer": "cancel"})).unwrap();
    assert!(!ufo.exists() && app.session.active().unwrap().path.is_none());
    assert!(answer(&mut app, json!({"answer": "proceed"})).is_err(), "the question is gone");

    post(&mut app, "file.saveAs", json!({"path": ufo}));
    answer(&mut app, json!({"answer": "proceed"})).unwrap();
    assert!(ufo.join("metainfo.plist").is_file(), "{}", app.status);
    post(&mut app, "spacing.setWidth", json!({"glyphs": ["I"], "value": 610}));
    post(&mut app, "file.save", json!({}));
    assert_eq!(state(&app)["open"], false, "the UFO already left it out");
    assert!(!app.session.active().unwrap().modified(), "{}", app.status);

    let mut app = with_corner(Vec::new());
    let other = dir.join("Other.ufo");
    let reply = app.run_request(Msg::run("file.saveAs", json!({"path": other}))).unwrap();
    assert!(reply.get("losses").is_some() && other.exists(), "a request runs as given: {reply}");
    assert_eq!(state(&app)["open"], false);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Save in the close question asks too: Cancel keeps the document open and unsaved, Save Anyway saves and closes it.
#[test]
fn saving_from_the_close_question_asks_too() {
    let dir = temp("loss-close");
    let ufo = dir.join("Font.ufo");
    let picked = || Picked::Paths(vec![ufo.clone()]);
    let mut app = with_corner(vec![picked(), picked()]);
    for proceed in [false, true] {
        assert!(close::fired(&mut app, &CommandRef::with_params("file.close", json!({}))));
        app.closing.from_window = true;
        app.sent.push(Msg::Shell(CommandRef::with_params("window.closePrompt", json!({"answer": "save"}))));
        // The first pass asks the save picker; the next runs the save with its answer.
        app.drain();
        app.drain();
        assert_eq!(state(&app)["kind"], "losses", "{}", app.status);
        assert!(app.closing.is_open() && app.closing.saving() && !ufo.exists());
        answer(&mut app, json!({"answer": if proceed { "proceed" } else { "cancel" }})).unwrap();
        assert!(!app.closing.is_open(), "{}", app.status);
        assert_eq!(app.session.documents().len(), usize::from(!proceed), "{}", app.status);
        assert_eq!(ufo.exists(), proceed);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A collection of two fonts in `dir`.
fn collection(dir: &Path) -> PathBuf {
    let mut s = Session::with_document();
    let (ttf, ttc) = (dir.join("One.ttf"), dir.join("Two.ttc"));
    s.execute("file.exportStatic", &json!({"destination": ttf, "timestamp": 0})).unwrap();
    s.execute("collection.pack", &json!({"fonts": [ttf, ttf], "output": ttc})).unwrap();
    ttc
}

/// File › Open… of a collection lists its fonts and opens the chosen ones, each view-only until Save As.
#[test]
fn a_collection_lists_its_fonts_and_opens_the_chosen_one() {
    let dir = temp("collection");
    let ttc = collection(&dir);
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    assert!(app.run_request(Msg::run("file.open", json!({"path": ttc}))).is_err(), "a request without face is refused");
    assert_eq!(state(&app)["open"], false);

    post(&mut app, "file.open", json!({"path": ttc}));
    let asked = state(&app);
    assert_eq!((&asked["kind"], asked["members"].as_array().map(Vec::len)), (&json!("collection"), Some(2)), "{asked}");
    assert!(answer(&mut app, json!({"answer": "proceed", "faces": [7]})).is_err());
    assert_eq!(app.session.documents().len(), 1);
    answer(&mut app, json!({"answer": "proceed", "faces": [1]})).unwrap();
    assert_eq!(app.session.documents().len(), 2, "{}", app.status);
    let doc = app.session.active().unwrap();
    assert_eq!((doc.collection_face(), doc.view_only()), (Some(1), true));
    assert!(open_report::notice(&app).is_some_and(|n| n.view_only));

    let native = dir.join("Second.fontcraft");
    app.run_request(Msg::run("file.saveAs", json!({"path": native}))).unwrap();
    assert!(!app.session.active().unwrap().view_only());
    assert_eq!(open_report::notice(&app), None, "the mark goes after Save As");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The questions and the open warnings' bar and list show in the window; whatever warnings an open gives are listed.
#[test]
fn the_dialogs_and_the_notice_show() {
    let dir = temp("file-dialogs");
    let ttc = collection(&dir);
    let mut t = TestApp::new();
    t.app.sent.push(Msg::run("file.open", json!({"path": ttc})));
    t.frames(3);
    assert_eq!(t.ok("ui.inspect", json!({}))["fileQuestion"]["kind"], "collection");
    answer(&mut t.app, json!({"answer": "proceed", "faces": [0, 1]})).unwrap();
    open_report::after_run(&mut t.app, "file.open", &json!({"viewOnly": true, "warnings": ["first thing", "second thing"]}));
    t.frames(3);
    let report = t.ok("ui.inspect", json!({}))["openReport"].clone();
    assert_eq!((&report["warnings"], &report["notice"]), (&json!(["first thing", "second thing"]), &json!(true)), "{report}");
    t.app.sent.push(Msg::Shell(CommandRef::with_params("file.openReport", json!({}))));
    t.frames(3);
    assert_eq!(t.ok("ui.inspect", json!({}))["openReport"]["open"], true);
    t.app.sent.push(Msg::Shell(CommandRef::with_params("file.openReport", json!({"dismiss": true}))));
    t.frames(3);
    let report = t.ok("ui.inspect", json!({}))["openReport"].clone();
    assert_eq!((&report["open"], &report["notice"]), (&json!(false), &json!(false)), "{report}");
    assert_eq!(t.app.view.file_questions.first(), None);
    let _ = std::fs::remove_dir_all(&dir);
}
