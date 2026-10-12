//! Macros and the Script commands: running, refusing, recording and the Macros folder.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fontcraft_engine::Session;
use fontcraft_testkit::{rect, temp_dir};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn undo_labels(s: &Session) -> Vec<String> {
    s.doc().unwrap().history.undo.iter().map(|h| h.label.to_string()).collect()
}

/// A Macros folder held in memory, as the desktop app hands it over.
fn with_macros(s: &mut Session, files: &[(&str, Value)]) {
    let files = files.iter().map(|(path, m)| ((*path).to_string(), Ok(m.to_string().into_bytes()))).collect();
    s.extensions_mut().sync_macros(files, Default::default());
}

fn widen() -> Value {
    json!({
        "format": "fontcraft.macro", "version": 1, "title": "Widen",
        "args": {"width": {"type": "int", "min": 0, "max": 5000, "default": 640}},
        "steps": [
            {"command": "glyph.newGlyph", "params": {"name": "w.alt"}},
            {"command": "spacing.setWidth", "params": {"glyphs": {"$arg": "glyphs"}, "value": {"$arg": "width"}}},
        ],
    })
}

#[test]
fn script_run_fills_placeholders_from_the_selection_as_one_undo_step() {
    let mut s = Session::with_document();
    let start = s.font().unwrap().clone();
    let out = run(&mut s, "script.run", json!({"macro": widen(), "glyphs": ["A"], "args": {"width": 700}}));
    assert_eq!(out["steps"], 2);
    let font = s.font().unwrap();
    assert!(font.glyph("w.alt").is_some());
    assert_eq!(font.glyph("A").unwrap().layers.first().map(|l| l.width), Some(700.0));
    assert_eq!(undo_labels(&s), ["Widen"]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap(), &start, "one undo takes back the whole macro");

    let dry = run(&mut s, "script.run", json!({"macro": widen(), "glyphs": ["A"], "dryRun": true}));
    assert_eq!(dry["steps"][1]["params"], json!({"glyphs": ["A"], "value": 640}));
    assert_eq!(s.font().unwrap(), &start, "a dry run changes nothing");
}

#[test]
fn script_run_refuses_unknown_commands_and_arguments_before_any_step_runs() {
    let mut s = Session::with_document();
    let start = s.font().unwrap().clone();
    let unknown_command = json!({"steps": [{"command": "glyph.newGlyph", "params": {"name": "x"}}, {"command": "glyph.noSuchThing"}]});
    let err = s.execute("script.run", &json!({"macro": unknown_command})).unwrap_err().to_string();
    assert!(err.contains("step 2") && err.contains("glyph.noSuchThing"), "{err}");
    let unknown_arg = json!({"steps": [{"command": "glyph.newGlyph", "params": {"name": {"$arg": "who"}}}]});
    let err = s.execute("script.run", &json!({"macro": unknown_arg})).unwrap_err().to_string();
    assert!(err.contains("who"), "{err}");
    assert!(s.execute("script.run", &json!({"macro": widen(), "glyphs": ["A"], "args": {"height": 3}})).is_err());
    let needs = json!({"requires": ["com.example.missing"], "steps": [{"command": "glyph.newGlyph", "params": {"name": "x"}}]});
    let err = s.execute("script.run", &json!({"macro": needs})).unwrap_err().to_string();
    assert!(err.contains("com.example.missing"), "{err}");
    assert!(s.execute("script.run", &json!({"macro": {"format": "fontcraft.macro", "version": 2}})).is_err());
    assert_eq!(s.font().unwrap(), &start);
    assert!(undo_labels(&s).is_empty());
    assert!(s.extensions().last_macro.is_none());
}

#[test]
fn script_run_failing_step_changes_nothing_and_is_named() {
    let mut s = Session::with_document();
    let start = s.font().unwrap().clone();
    let failing = json!({"steps": [
        {"command": "glyph.newGlyph", "params": {"name": "b.alt"}},
        {"command": "spacing.setWidth", "params": {"glyphs": ["nope"], "value": 1}},
    ]});
    let err = s.execute("script.run", &json!({"macro": failing})).unwrap_err().to_string();
    assert!(err.contains("step 2") && err.contains("spacing.setWidth"), "{err}");
    assert_eq!(s.font().unwrap(), &start);
    assert!(undo_labels(&s).is_empty());
    assert!(s.extensions().running_macros.is_empty());
}

#[test]
fn script_run_refuses_self_reference_and_deep_nesting() {
    let mut s = Session::with_document();
    let start = s.font().unwrap().clone();
    let mut files =
        vec![("Self.fcmacro", json!({"format": "fontcraft.macro", "version": 1, "steps": [{"command": "script.run", "params": {"macro": "Self"}}]}))];
    let names: Vec<String> = (1..=9).map(|i| format!("M{i}.fcmacro")).collect();
    for i in 1..=9 {
        let step = if i < 9 {
            json!({"command": "script.run", "params": {"macro": format!("M{}", i + 1)}})
        } else {
            json!({"command": "glyph.newGlyph", "params": {"name": "deep"}})
        };
        files.push((names[i - 1].as_str(), json!({"format": "fontcraft.macro", "version": 1, "steps": [step]})));
    }
    with_macros(&mut s, &files);
    let err = s.execute("script.run", &json!({"macro": "Self"})).unwrap_err().to_string();
    assert!(err.contains("runs itself"), "{err}");
    let err = s.execute("script.run", &json!({"macro": "M1"})).unwrap_err().to_string();
    assert!(err.contains("nested"), "{err}");
    assert_eq!(s.font().unwrap(), &start);
    // Eight levels are allowed.
    run(&mut s, "script.run", json!({"macro": "M2"}));
    assert!(s.font().unwrap().glyph("deep").is_some());
    assert_eq!(undo_labels(&s), ["M2"]);
    assert_eq!(s.extensions().last_macro.as_ref().and_then(|l| l.0.as_deref()), Some("M2.fcmacro"));
}

#[test]
fn a_recorded_macro_replays_to_the_same_font() {
    let mut s = Session::with_document();
    run(&mut s, "script.recordMacro", json!({"action": "start"}));
    run(&mut s, "glyph.newGlyph", json!({"name": "a.alt"}));
    run(&mut s, "font.customParameters.list", json!({}));
    run(&mut s, "path.addPath", json!({"glyph": "a.alt", "nodes": rect(50.0, 0.0, 450.0, 500.0)}));
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["a.alt"], "value": 520}));
    let out = run(&mut s, "script.recordMacro", json!({"action": "stop", "title": "Make a.alt"}));
    assert_eq!(out["steps"], 3, "queries are not recorded: {out}");
    assert!(s.extensions().recording.is_none());

    let mut fresh = Session::with_document();
    run(&mut fresh, "script.run", json!({"macro": out["macro"]}));
    assert_eq!(fresh.font().unwrap(), s.font().unwrap());
    assert_eq!(undo_labels(&fresh), ["Make a.alt"]);

    // A re-run while recording is recorded as the macro it ran.
    run(&mut fresh, "edit.undo", json!({}));
    run(&mut fresh, "script.recordMacro", json!({"action": "start"}));
    run(&mut fresh, "script.rerunLast", json!({}));
    let again = run(&mut fresh, "script.recordMacro", json!({"action": "stop"}));
    assert_eq!(again["macro"]["steps"][0]["command"], "script.run");
    assert!(fresh.execute("script.recordMacro", &json!({"action": "stop"})).is_err(), "nothing is being recorded");
}

#[test]
fn rerun_last_runs_the_same_macro_on_a_new_selection() {
    let mut s = Session::with_document();
    assert!(s.execute("script.rerunLast", &json!({})).is_err(), "nothing has run yet");
    let set = json!({"format": "fontcraft.macro", "version": 1, "title": "Set 610",
        "steps": [{"command": "spacing.setWidth", "params": {"glyphs": {"$arg": "glyphs"}, "value": 610}}]});
    with_macros(&mut s, &[("Spacing/Set 610.fcmacro", set)]);
    run(&mut s, "script.run", json!({"macro": "Set 610", "glyphs": ["A"]}));
    run(&mut s, "script.repeatLastAction", json!({"glyphs": ["B"]}));
    let font = s.font().unwrap();
    for name in ["A", "B"] {
        assert_eq!(font.glyph(name).unwrap().layers.first().map(|l| l.width), Some(610.0), "{name}");
    }
    assert_eq!(undo_labels(&s), ["Set 610", "Set 610"]);
}

#[test]
fn the_macros_folder_is_listed_saved_to_and_run_from() {
    let dir = temp_dir("macros-folder").join("Macros");
    let mut s = Session::with_document();
    assert!(s.execute("script.reloadScripts", &json!({})).is_err(), "no Macros folder yet");
    s.extensions_mut().macros_dir = Some(dir.clone());
    let opened = run(&mut s, "script.openScriptsFolder", json!({}));
    assert_eq!(opened["path"], dir.display().to_string());
    std::fs::create_dir_all(dir.join("Tools")).unwrap();
    let write = |path: &str, title: &str| {
        let m = json!({"format": "fontcraft.macro", "version": 1, "title": title, "steps": [{"command": "glyph.newGlyph", "params": {"name": path.replace(['/', '.'], "_")}}]});
        std::fs::write(dir.join(path), m.to_string()).unwrap();
    };
    write("b.fcmacro", "Alpha");
    write("a.fcmacro", "Zulu");
    write("c.fcmacro", "");
    write("Tools/t.fcmacro", "Tool");
    std::fs::write(dir.join("broken.fcmacro"), "{").unwrap();
    std::fs::write(dir.join("notes.txt"), "not a macro").unwrap();
    let out = run(&mut s, "script.reloadScripts", json!({}));
    assert_eq!(out["changed"], true);
    let titles: Vec<&str> = out["macros"].as_array().unwrap().iter().map(|e| e["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Tool", "Alpha", "broken", "c", "Zulu"], "folders first, then titles; an untitled macro shows its file name");
    assert!(out["macros"][2]["error"].is_string());
    assert!(!fontcraft_engine::macros::poll(s.extensions_mut()), "nothing changed since");

    run(&mut s, "script.run", json!({"macro": "Tools/t.fcmacro"}));
    assert!(s.font().unwrap().glyph("Tools_t_fcmacro").is_some());

    let saved = run(&mut s, "script.saveActionSet", json!({"name": "Make x", "steps": [{"command": "glyph.newGlyph", "params": {"name": "x.alt"}}]}));
    assert!(dir.join("Make x.fcmacro").is_file(), "{saved}");
    assert!(s.extensions().find_macro("Make x").is_some());
    assert!(s.execute("script.saveActionSet", &json!({"name": "Make x", "steps": []})).is_err(), "an existing file needs replace");
    assert!(s.execute("script.saveActionSet", &json!({"name": "Bad", "steps": [{"command": "no.such"}]})).is_err());
    assert!(!dir.join("Bad.fcmacro").exists(), "a refused set writes nothing");
    run(&mut s, "script.runActionSet", json!({"set": "Make x"}));
    assert!(s.font().unwrap().glyph("x.alt").is_some());

    run(&mut s, "script.recordMacro", json!({"action": "start"}));
    run(&mut s, "glyph.newGlyph", json!({"name": "rec"}));
    run(&mut s, "script.recordMacro", json!({"action": "stop", "name": "Recorded"}));
    let file = std::fs::read(dir.join("Recorded.fcmacro")).unwrap();
    let m = fontcraft_engine::macros::Macro::parse(&file).unwrap();
    assert_eq!(m.steps.len(), 1);
    assert_eq!(m.title, "Recorded");
}
